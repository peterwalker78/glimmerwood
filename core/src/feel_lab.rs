//! The feel lab: a scripted day played fast, to judge how the wisp moves
//! without living through a day of browsing.
//!
//! Pure: it maps real time to lab time and says what the script has the user
//! doing. The companion feeds that to the engine in place of real attention.

use crate::dose::{Activity, Moment, Place};

pub const DAY: &str = include_str!("../data/feel-lab.day");

#[derive(Clone, Debug, PartialEq)]
enum Doing {
    Activity(Option<Place>),
    End,
}

#[derive(Clone, Debug)]
struct Line {
    minute: i64,
    doing: Doing,
}

pub struct Lab {
    lines: Vec<Line>,
    /// Real moment the lab started, and the lab moment it started at.
    real_start: Moment,
    lab_start: Moment,
    speed: f64,
    /// Local midnight of the lab's day, in Unix ms.
    midnight_ms: i64,
    applied: usize,
    /// Hold the hover caption open, for screenshots.
    pub show_caption: bool,
}

/// One scripted change, ready for the engine.
pub struct Step {
    pub at: Moment,
    pub activity: Activity,
}

impl Lab {
    /// `from` is "HH:MM" to jump to, or the script's first line.
    pub fn new(
        script: &str,
        real_now: Moment,
        speed: f64,
        from: Option<&str>,
    ) -> Result<Lab, String> {
        let lines = parse(script)?;
        let first = lines.first().ok_or("the feel lab script is empty")?.minute;
        let start_minute = match from {
            Some(hhmm) => minute_of(hhmm).ok_or_else(|| format!("{hhmm:?} is not HH:MM"))?,
            None => first,
        };
        let local_ms = real_now.ms + i64::from(real_now.utc_offset_s) * 1000;
        let midnight_ms =
            local_ms - local_ms.rem_euclid(86_400_000) - i64::from(real_now.utc_offset_s) * 1000;
        let lab = Lab {
            lines,
            real_start: real_now,
            lab_start: Moment {
                ms: midnight_ms + start_minute * 60_000,
                utc_offset_s: real_now.utc_offset_s,
            },
            speed,
            midnight_ms,
            applied: 0,
            show_caption: false,
        };
        Ok(lab)
    }

    /// When the scripted day begins; the engine starts here.
    pub fn day_start(&self) -> Moment {
        self.moment(self.lines[0].minute)
    }

    pub fn clock(&self, real_now: Moment) -> Moment {
        let elapsed = (real_now.ms - self.real_start.ms) as f64 * self.speed;
        let end = self
            .lines
            .iter()
            .find(|l| l.doing == Doing::End)
            .map(|l| self.moment(l.minute).ms)
            .unwrap_or(i64::MAX);
        Moment {
            ms: (self.lab_start.ms + elapsed as i64).min(end),
            utc_offset_s: self.lab_start.utc_offset_s,
        }
    }

    /// Real milliseconds until lab time `lab_ms`.
    pub fn real_delay(&self, lab_now: Moment, lab_ms: i64) -> i64 {
        if self.speed <= 0.0 {
            return i64::MAX;
        }
        ((lab_ms - lab_now.ms) as f64 / self.speed).ceil() as i64
    }

    /// The lab time of the next scripted line not yet applied.
    pub fn next_line(&self) -> Option<Moment> {
        self.lines.get(self.applied).map(|l| self.moment(l.minute))
    }

    /// Scripted changes up to `lab_now`, each with its presence leased until
    /// the next change of activity.
    pub fn due(&mut self, lab_now: Moment) -> Vec<Step> {
        let mut steps = Vec::new();
        while let Some(line) = self.lines.get(self.applied) {
            let at = self.moment(line.minute);
            if at.ms > lab_now.ms {
                break;
            }
            let until = self
                .lines
                .get(self.applied + 1)
                .map(|l| self.moment(l.minute))
                .unwrap_or(Moment {
                    ms: at.ms + 86_400_000,
                    utc_offset_s: at.utc_offset_s,
                });
            let activity = match &line.doing {
                Doing::Activity(Some(place)) => Activity::Present {
                    place: place.clone(),
                    until,
                },
                Doing::Activity(None) | Doing::End => Activity::Away,
            };
            steps.push(Step { at, activity });
            self.applied += 1;
        }
        steps
    }

    fn moment(&self, minute: i64) -> Moment {
        Moment {
            ms: self.midnight_ms + minute * 60_000,
            utc_offset_s: self.lab_start.utc_offset_s,
        }
    }
}

fn minute_of(hhmm: &str) -> Option<i64> {
    let (h, m) = hhmm.split_once(':')?;
    let (h, m): (i64, i64) = (h.parse().ok()?, m.parse().ok()?);
    ((0..24).contains(&h) && (0..60).contains(&m)).then_some(h * 60 + m)
}

fn parse(script: &str) -> Result<Vec<Line>, String> {
    let mut lines = Vec::new();
    for (n, raw) in script.lines().enumerate() {
        let text = raw.trim();
        if text.is_empty() || text.starts_with('#') {
            continue;
        }
        let bad = |why: &str| format!("feel lab line {}: {why}: {raw:?}", n + 1);
        let words: Vec<&str> = text.split_whitespace().collect();
        let minute = words
            .first()
            .and_then(|w| minute_of(w))
            .ok_or_else(|| bad("no HH:MM"))?;
        let weight = |w: &str| w.parse::<f64>().map_err(|_| bad("weight is not a number"));
        let doing = match &words[1..] {
            ["draining", w, entry] => Doing::Activity(Some(Place::Listed {
                entry: (*entry).into(),
                weight: -weight(w)?.abs(),
                news: false,
            })),
            ["news", entry] => Doing::Activity(Some(Place::Listed {
                entry: (*entry).into(),
                weight: -0.4,
                news: true,
            })),
            ["nourishing", w, entry] => Doing::Activity(Some(Place::Listed {
                entry: (*entry).into(),
                weight: weight(w)?.abs(),
                news: false,
            })),
            ["ordinary", entry] => Doing::Activity(Some(Place::Listed {
                entry: (*entry).into(),
                weight: 0.0,
                news: false,
            })),
            ["unlisted"] => Doing::Activity(Some(Place::Unlisted)),
            ["private"] => Doing::Activity(Some(Place::Private)),
            ["away"] => Doing::Activity(None),
            ["end"] => Doing::End,
            _ => return Err(bad("unknown activity")),
        };
        if lines.last().is_some_and(|l: &Line| l.minute > minute) {
            return Err(bad("lines must be in time order"));
        }
        lines.push(Line { minute, doing });
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-09-15 10:00 BST, as the real clock.
    fn real(offset_ms: i64) -> Moment {
        Moment {
            ms: 1_789_462_800_000 + offset_ms,
            utc_offset_s: 3600,
        }
    }

    #[test]
    fn the_bundled_day_parses() {
        assert!(parse(DAY).is_ok());
    }

    #[test]
    fn the_clock_runs_at_the_chosen_speed_from_the_chosen_time() {
        let lab = Lab::new(DAY, real(0), 60.0, Some("12:00")).unwrap();
        // One real second is one lab minute.
        let lab_now = lab.clock(real(1000));
        assert_eq!(lab_now.ms - lab.clock(real(0)).ms, 60_000);
        assert_eq!(lab.real_delay(lab_now, lab_now.ms + 120_000), 2000);
    }

    #[test]
    fn jumping_ahead_replays_every_earlier_line_in_order() {
        let mut lab = Lab::new(DAY, real(0), 60.0, Some("12:30")).unwrap();
        let steps = lab.due(lab.clock(real(0)));
        // 07:50, 08:10, 09:00, 11:40, 12:10.
        assert_eq!(steps.len(), 5);
        assert!(matches!(
            &steps[4].activity,
            Activity::Present { place: Place::Listed { entry, .. }, .. } if entry == "tiktok.com"
        ));
    }

    #[test]
    fn the_day_stops_at_its_end() {
        let lab = Lab::new(DAY, real(0), 60.0, Some("23:39")).unwrap();
        let later = lab.clock(real(3_600_000));
        assert_eq!(later.ms, lab.moment(23 * 60 + 40).ms);
    }

    #[test]
    fn scripts_with_mistakes_are_rejected() {
        assert!(parse("25:00 away").is_err());
        assert!(parse("09:00 dancing").is_err());
        assert!(parse("10:00 away\n09:00 away").is_err());
    }
}

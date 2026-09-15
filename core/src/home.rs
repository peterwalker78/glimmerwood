//! Home: what the local home page says, as pure
//! functions of what Glimmerwood already knows. The GTK side gathers the facts; the
//! words come from `core/data/home.toml`.

use std::collections::{BTreeMap, HashSet};

use serde::Deserialize;

use crate::dose::{self, Mode, Moment, Rates};
use crate::sun;

#[derive(Clone, Debug, Deserialize)]
pub struct Words {
    moments: ByPart<u32>,
    sky: Sky,
    /// Where the sun is read from, found once at start-up rather than set by
    /// the user: `None` where the time zone doesn't say.
    #[serde(skip)]
    place: Option<sun::Coords>,
    welcome: ByPart<String>,
    quiet: ByPart<Vec<String>>,
    explain: Explanations,
    pub about: About,
    praise: Praise,
    pub thresholds: Thresholds,
}

/// Where the sun has to stand for the garden to change its light.
#[derive(Clone, Debug, Deserialize)]
struct Sky {
    night_below: f64,
    low_sun: f64,
}

#[derive(Clone, Debug, Deserialize)]
struct ByPart<T> {
    morning: T,
    day: T,
    evening: T,
    night: T,
}

impl<T> ByPart<T> {
    fn get(&self, part: PartOfDay) -> &T {
        match part {
            PartOfDay::Morning => &self.morning,
            PartOfDay::Day => &self.day,
            PartOfDay::Evening => &self.evening,
            PartOfDay::Night => &self.night,
        }
    }
}

/// The wisp introducing itself and what Glimmerwood is for, until dismissed.
#[derive(Clone, Debug, Deserialize)]
pub struct About {
    pub title: String,
    pub paragraphs: Vec<String>,
    pub done: String,
}

#[derive(Clone, Debug, Deserialize)]
struct Explanations {
    draining: String,
    night: String,
    privacy: String,
}

#[derive(Clone, Debug, Deserialize)]
struct Praise {
    garden: Vec<String>,
    #[serde(rename = "break")]
    took_a_break: Vec<String>,
    restoring: Vec<String>,
    gentle: Vec<String>,
    calm_night: Vec<String>,
    rested: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Thresholds {
    pub break_minutes: f64,
    pub restoring_minutes: f64,
    pub gentle_present_minutes: f64,
    pub gentle_load: f64,
    pub rested_away_minutes: f64,
    pub own_place_minutes: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PartOfDay {
    Morning,
    Day,
    Evening,
    Night,
}

/// Something Home explains once. `About` is the wisp's introduction; the
/// others are explained once the user has met them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Topic {
    About,
    Draining,
    Night,
    Privacy,
}

impl Topic {
    pub const ALL: [Topic; 4] = [Topic::About, Topic::Draining, Topic::Night, Topic::Privacy];

    pub fn key(self) -> &'static str {
        match self {
            Topic::About => "about",
            Topic::Draining => "draining",
            Topic::Night => "night",
            Topic::Privacy => "privacy",
        }
    }

    pub fn from_key(key: &str) -> Option<Topic> {
        Topic::ALL.into_iter().find(|t| t.key() == key)
    }
}

impl Words {
    pub fn bundled() -> Words {
        let mut words: Words = toml::from_str(include_str!("../data/home.toml"))
            .expect("core/data/home.toml is valid");
        words.place = sun::here();
        words
    }

    /// Where the light is: by the sun itself where the time zone says where
    /// we are, and by the clock where it doesn't.
    pub fn part_of_day(&self, at: Moment) -> PartOfDay {
        match self.place {
            Some(coords) => {
                let sun = sun::position(coords, at.ms);
                let s = &self.sky;
                if sun.altitude < s.night_below {
                    PartOfDay::Night
                } else if sun.altitude >= s.low_sun {
                    PartOfDay::Day
                } else if sun.climbing {
                    PartOfDay::Morning
                } else {
                    PartOfDay::Evening
                }
            }
            None => self.by_the_clock(at),
        }
    }

    fn by_the_clock(&self, at: Moment) -> PartOfDay {
        let hour = (at.ms / 1000 + i64::from(at.utc_offset_s)).rem_euclid(86_400) / 3600;
        let hour = hour as u32;
        let m = &self.moments;
        if hour >= m.night || hour < m.morning {
            PartOfDay::Night
        } else if hour >= m.evening {
            PartOfDay::Evening
        } else if hour >= m.day {
            PartOfDay::Day
        } else {
            PartOfDay::Morning
        }
    }

    fn explanation(&self, topic: Topic) -> &str {
        let e = &self.explain;
        match topic {
            Topic::About => &self.about.title,
            Topic::Draining => &e.draining,
            Topic::Night => &e.night,
            Topic::Privacy => &e.privacy,
        }
    }
}

// --- What happened, day by day ------------------------------------------------

/// One recorded minute, as Home needs it: no sites, only kinds of time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Minute {
    pub at: Moment,
    pub mode: Mode,
    /// The list weight of where attention was, if it was on a listed site.
    pub weight: Option<f64>,
}

/// A day (05:00 to 05:00) summed up. Minutes are counted from the history,
/// where a missing minute is time away.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Day {
    pub present: f64,
    /// Weighted draining minutes, as the dose model counts load.
    pub load: f64,
    pub draining: f64,
    pub nourishing: f64,
    /// Draining load during the night window.
    pub night_load: f64,
    /// The longest time away between two present minutes.
    pub longest_break: f64,
}

/// Sum minutes up by day number (`dose::day_of`).
pub fn days(rates: &Rates, minutes: &[Minute]) -> BTreeMap<i64, Day> {
    let mut days: BTreeMap<i64, Day> = BTreeMap::new();
    let mut last_present: BTreeMap<i64, i64> = BTreeMap::new();
    let mut sorted = minutes.to_vec();
    sorted.sort_by_key(|m| m.at.ms);
    for m in sorted {
        let number = dose::day_of(rates, m.at);
        let day = days.entry(number).or_default();
        if m.mode == Mode::Away {
            continue;
        }
        let minute = m.at.ms.div_euclid(60_000);
        if let Some(previous) = last_present.insert(number, minute) {
            day.longest_break = day.longest_break.max((minute - previous - 1) as f64);
        }
        day.present += 1.0;
        match m.weight {
            Some(w) if w < 0.0 => {
                day.load += w.abs();
                day.draining += 1.0;
                if rates.is_night(m.at) {
                    day.night_load += w.abs();
                }
            }
            Some(w) if w > 0.0 => day.nourishing += 1.0,
            _ => {}
        }
    }
    days
}

// --- The garden ----------------------------------------------------------------

/// What a day left in the garden.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plant {
    /// A quiet day, mostly away from the screen: rain and moss.
    Moss,
    /// More nourishing time than draining: new leaves and ferns.
    Fern,
    /// A gentle day: flowers.
    Flower,
}

impl Plant {
    pub fn key(self) -> &'static str {
        match self {
            Plant::Moss => "moss",
            Plant::Fern => "fern",
            Plant::Flower => "flower",
        }
    }

    pub fn from_key(key: &str) -> Option<Plant> {
        [Plant::Moss, Plant::Fern, Plant::Flower]
            .into_iter()
            .find(|p| p.key() == key)
    }
}

/// Whether a finished day grew the garden, and with what. `None` for the
/// day means nothing was recorded: a day away, which is a quiet day. Heavy
/// days grow nothing and take nothing away.
pub fn growth(thresholds: &Thresholds, day: Option<&Day>) -> Option<Plant> {
    let Some(day) = day else {
        return Some(Plant::Moss);
    };
    if day.present < thresholds.gentle_present_minutes {
        Some(Plant::Moss)
    } else if day.nourishing > day.draining && day.nourishing >= 20.0 {
        Some(Plant::Fern)
    } else if day.load < thresholds.gentle_load {
        Some(Plant::Flower)
    } else {
        None
    }
}

/// A day left fireflies: time here, none of it draining after dark.
pub fn fireflies(day: Option<&Day>) -> bool {
    day.is_some_and(|d| d.present > 0.0 && d.night_load == 0.0)
}

// --- The greeting --------------------------------------------------------------

/// Everything the greeting depends on.
#[derive(Clone, Debug)]
pub struct Facts {
    pub now: Moment,
    pub dose: f64,
    /// Topics the user has met, and topics they've said "Got it" to.
    pub met: HashSet<Topic>,
    pub explained: HashSet<Topic>,
    pub today: Option<Day>,
    pub yesterday: Option<Day>,
    /// Minutes on nourishing sites over the last seven days.
    pub week_nourishing: f64,
    /// Minutes since the user was last present before coming here.
    pub away_minutes: f64,
    pub garden_grew_yesterday: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Greeting {
    pub title: String,
    pub line: String,
    pub explain: Option<(Topic, String)>,
    /// The wisp introduces itself, until the user dismisses it.
    pub about: bool,
}

pub fn greet(words: &Words, rates: &Rates, facts: &Facts) -> Greeting {
    let part = words.part_of_day(facts.now);
    let day = dose::day_of(rates, facts.now);
    let pick = |lines: &[String]| {
        lines
            .get(day.rem_euclid(lines.len().max(1) as i64) as usize)
            .cloned()
            .unwrap_or_default()
    };

    let t = &words.thresholds;
    // Other explanations wait until the introduction has been read.
    let about = !facts.explained.contains(&Topic::About);
    let explain = [Topic::Draining, Topic::Night, Topic::Privacy]
        .into_iter()
        .filter(|_| !about)
        .find(|topic| facts.met.contains(topic) && !facts.explained.contains(topic))
        .map(|topic| (topic, words.explanation(topic).to_owned()));

    let p = &words.praise;
    let today = facts.today.clone().unwrap_or_default();
    let been_here_today = today.present > 0.0;
    let praise = if facts.garden_grew_yesterday && part == PartOfDay::Morning {
        Some(&p.garden)
    } else if been_here_today
        && today.longest_break.max(facts.away_minutes) >= t.break_minutes
        && facts.away_minutes < t.rested_away_minutes
    {
        Some(&p.took_a_break)
    } else if facts.away_minutes >= t.rested_away_minutes && facts.dose < rates.levels.engaged {
        Some(&p.rested)
    } else if facts.week_nourishing >= t.restoring_minutes {
        Some(&p.restoring)
    } else if today.present >= t.gentle_present_minutes && today.load < t.gentle_load {
        Some(&p.gentle)
    } else if part == PartOfDay::Morning
        && facts
            .yesterday
            .as_ref()
            .is_some_and(|y| y.present >= t.gentle_present_minutes && y.night_load == 0.0)
    {
        Some(&p.calm_night)
    } else {
        None
    };

    Greeting {
        title: words.welcome.get(part).clone(),
        line: pick(praise.unwrap_or(words.quiet.get(part))),
        explain,
        about,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BST: i32 = 3600;
    /// 2026-09-15 00:00 BST.
    const MIDNIGHT_MS: i64 = 1_789_426_800_000;

    fn at(hour: i64, minute: i64) -> Moment {
        Moment {
            ms: MIDNIGHT_MS + (hour * 60 + minute) * 60_000,
            utc_offset_s: BST,
        }
    }

    fn facts(now: Moment) -> Facts {
        Facts {
            now,
            dose: 0.3,
            met: HashSet::new(),
            explained: HashSet::from([Topic::About]),
            today: None,
            yesterday: None,
            week_nourishing: 0.0,
            away_minutes: 0.0,
            garden_grew_yesterday: false,
        }
    }

    fn day(present: f64, load: f64, nourishing: f64) -> Day {
        Day {
            present,
            load,
            draining: load,
            nourishing,
            night_load: 0.0,
            longest_break: 0.0,
        }
    }

    fn greet_at(f: &Facts) -> Greeting {
        greet(&Words::bundled(), &Rates::bundled(), f)
    }

    #[test]
    fn every_line_the_data_file_needs_is_there() {
        let words = Words::bundled();
        for part in [
            PartOfDay::Morning,
            PartOfDay::Day,
            PartOfDay::Evening,
            PartOfDay::Night,
        ] {
            assert!(!words.quiet.get(part).is_empty());
            assert!(!words.welcome.get(part).is_empty());
        }
        assert!(!words.about.title.is_empty() && words.about.paragraphs.len() >= 2);
        for topic in Topic::ALL {
            assert!(!words.explanation(topic).is_empty());
            assert_eq!(Topic::from_key(topic.key()), Some(topic));
        }
    }

    #[test]
    fn the_wisp_introduces_itself_until_dismissed() {
        let mut f = facts(at(10, 0));
        f.explained.clear();
        f.met.insert(Topic::Draining);
        let g = greet_at(&f);
        assert!(g.about);
        // One thing to read at a time.
        assert_eq!(g.explain, None);
        f.explained.insert(Topic::About);
        let g = greet_at(&f);
        assert!(!g.about);
        assert_eq!(g.explain.map(|e| e.0), Some(Topic::Draining));
    }

    #[test]
    fn a_thing_met_is_explained_once_until_got() {
        let mut f = facts(at(10, 0));
        f.met.insert(Topic::Night);
        f.met.insert(Topic::Draining);
        assert_eq!(greet_at(&f).explain.map(|e| e.0), Some(Topic::Draining));
        f.explained.insert(Topic::Draining);
        assert_eq!(greet_at(&f).explain.map(|e| e.0), Some(Topic::Night));
        f.explained.insert(Topic::Night);
        assert_eq!(greet_at(&f).explain, None);
    }

    #[test]
    fn welcomes_follow_the_time_of_day() {
        let words = Words::bundled();
        assert_eq!(greet_at(&facts(at(7, 0))).title, words.welcome.morning);
        assert_eq!(greet_at(&facts(at(13, 0))).title, words.welcome.day);
        assert_eq!(greet_at(&facts(at(19, 0))).title, words.welcome.evening);
        assert_eq!(greet_at(&facts(at(23, 30))).title, words.welcome.night);
        assert_eq!(words.part_of_day(at(4, 59)), PartOfDay::Night);
    }

    #[test]
    fn praise_is_only_ever_true() {
        let words = Words::bundled();
        let quiet_day = &words.quiet.day;

        // Nothing notable: a plain line for the time of day.
        let f = facts(at(14, 0));
        assert!(quiet_day.contains(&greet_at(&f).line));

        // A heavy day earns nothing, and says nothing about it.
        let mut heavy = facts(at(14, 0));
        heavy.today = Some(day(120.0, 90.0, 0.0));
        assert!(quiet_day.contains(&greet_at(&heavy).line));

        let mut gentle = facts(at(14, 0));
        gentle.today = Some(day(60.0, 5.0, 0.0));
        assert!(words.praise.gentle.contains(&greet_at(&gentle).line));

        let mut restoring = facts(at(14, 0));
        restoring.week_nourishing = 90.0;
        assert!(words.praise.restoring.contains(&greet_at(&restoring).line));

        let mut rested = facts(at(14, 0));
        rested.away_minutes = 180.0;
        rested.dose = 0.05;
        assert!(words.praise.rested.contains(&greet_at(&rested).line));
    }

    #[test]
    fn a_break_taken_today_is_noticed() {
        let words = Words::bundled();
        let mut f = facts(at(15, 0));
        let mut today = day(40.0, 40.0, 0.0);
        today.longest_break = 45.0;
        f.today = Some(today);
        assert!(words.praise.took_a_break.contains(&greet_at(&f).line));
    }

    #[test]
    fn mornings_notice_a_calm_night_and_a_garden_that_grew() {
        let words = Words::bundled();
        let mut f = facts(at(8, 0));
        f.yesterday = Some(day(60.0, 50.0, 0.0));
        assert!(words.praise.calm_night.contains(&greet_at(&f).line));
        f.garden_grew_yesterday = true;
        assert!(words.praise.garden.contains(&greet_at(&f).line));
        // Not in the evening.
        let mut evening = f.clone();
        evening.now = at(20, 0);
        evening.yesterday = None;
        assert!(words.quiet.evening.contains(&greet_at(&evening).line));
    }

    #[test]
    fn days_sum_minutes_by_kind_and_find_the_longest_break() {
        let rates = Rates::bundled();
        let minute = |h, m, mode, weight| Minute {
            at: at(h, m),
            mode,
            weight,
        };
        let mut minutes = vec![];
        for m in 0..10 {
            minutes.push(minute(9, m, Mode::Draining, Some(-1.0)));
        }
        for m in 0..5 {
            minutes.push(minute(10, m, Mode::Nourishing, Some(0.5)));
        }
        minutes.push(minute(10, 30, Mode::Away, None));
        minutes.push(minute(23, 30, Mode::Draining, Some(-0.4)));
        let days = days(&rates, &minutes);
        let d = &days[&dose::day_of(&rates, at(9, 0))];
        assert_eq!(d.present, 16.0);
        assert_eq!(d.draining, 11.0);
        assert_eq!(d.nourishing, 5.0);
        assert!((d.load - 10.4).abs() < 1e-9);
        assert!((d.night_load - 0.4).abs() < 1e-9);
        // 10:04 to 23:30.
        assert_eq!(d.longest_break, 805.0);
    }

    #[test]
    fn gardens_grow_on_good_and_quiet_days_and_never_shrink() {
        let t = Words::bundled().thresholds;
        assert_eq!(growth(&t, None), Some(Plant::Moss));
        assert_eq!(growth(&t, Some(&day(10.0, 5.0, 0.0))), Some(Plant::Moss));
        assert_eq!(growth(&t, Some(&day(90.0, 10.0, 40.0))), Some(Plant::Fern));
        assert_eq!(growth(&t, Some(&day(90.0, 10.0, 0.0))), Some(Plant::Flower));
        assert_eq!(growth(&t, Some(&day(180.0, 120.0, 10.0))), None);
        assert!(fireflies(Some(&day(90.0, 10.0, 0.0))));
        let mut late = day(90.0, 10.0, 0.0);
        late.night_load = 3.0;
        assert!(!fireflies(Some(&late)));
    }
}

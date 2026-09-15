//! The dose engine.
//!
//! Pure and deterministic: it only ever learns the time from the moments it is
//! given. Time is cut into stretches over which nothing changes but the
//! clock: a new stretch starts whenever the activity, a session threshold, the
//! news allowance, the night or the day changes. Within a stretch the dose
//! follows a formula (a straight climb, an exponential fall, or holding), so
//! the engine can say exactly when the next level will be crossed and the app
//! can sleep until then instead of polling.

use std::collections::VecDeque;

use serde::Deserialize;

const MINUTE_MS: f64 = 60_000.0;
const DAY_S: i64 = 86_400;

/// A point in time: Unix milliseconds plus the local UTC offset in effect, so
/// the engine can find 05:00 without reading the system clock or time zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Moment {
    pub ms: i64,
    pub utc_offset_s: i32,
}

impl Moment {
    fn local_s(self) -> i64 {
        self.ms.div_euclid(1000) + i64::from(self.utc_offset_s)
    }

    fn plus_minutes(self, minutes: f64) -> Moment {
        Moment {
            ms: self.ms + (minutes * MINUTE_MS).ceil() as i64,
            utc_offset_s: self.utc_offset_s,
        }
    }
}

/// The tunable rates, read from `core/data/dose.toml`.
#[derive(Clone, Debug, Deserialize)]
pub struct Rates {
    pub levels: Levels,
    pub draining: Draining,
    pub session: Session,
    pub news: News,
    pub rest: Rest,
    pub night: Night,
    pub clutter: Clutter,
    pub day: Day,
    pub presence: Presence,
    pub heard: Heard,
    pub caption: Caption,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Levels {
    pub engaged: f64,
    pub clouded: f64,
    pub drained: f64,
}

impl Levels {
    pub fn phase(&self, dose: f64) -> Phase {
        match dose {
            d if d >= self.drained => Phase::Drained,
            d if d >= self.clouded => Phase::Clouded,
            d if d >= self.engaged => Phase::Engaged,
            _ => Phase::Rested,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Draining {
    pub minutes_to_drained: f64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Session {
    pub short_minutes: f64,
    pub short_factor: f64,
    pub long_minutes: f64,
    pub long_factor: f64,
    pub break_minutes: f64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct News {
    pub allowance_minutes: f64,
    pub allowance_factor: f64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Rest {
    pub away_half_life: f64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Night {
    pub starts_at: String,
    pub ends_at: String,
    pub draining_factor: f64,
    pub carry_factor: f64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Clutter {
    pub untouched_hours: f64,
    pub tabs_for_full_effect: u32,
    pub slowest_recovery: f64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Day {
    pub starts_at: String,
    pub carry_max: f64,
    pub carry_full_load: f64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Presence {
    pub input_seconds: u32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Heard {
    pub factor: f64,
    pub listening_minutes: f64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Caption {
    pub window: f64,
}

/// Seconds after local midnight for "HH:MM".
fn clock_s(text: &str, name: &str) -> Result<i64, String> {
    let bad = || format!("{name} {text:?} is not HH:MM");
    let (h, m) = text.split_once(':').ok_or_else(bad)?;
    let (h, m): (i64, i64) = (h.parse().map_err(|_| bad())?, m.parse().map_err(|_| bad())?);
    if !(0..24).contains(&h) || !(0..60).contains(&m) {
        return Err(bad());
    }
    Ok(h * 3600 + m * 60)
}

impl Rates {
    pub fn bundled() -> Rates {
        Rates::parse(include_str!("../data/dose.toml")).expect("core/data/dose.toml is valid")
    }

    pub fn parse(text: &str) -> Result<Rates, String> {
        let rates: Rates = toml::from_str(text).map_err(|e| e.to_string())?;
        clock_s(&rates.day.starts_at, "day.starts_at")?;
        clock_s(&rates.night.starts_at, "night.starts_at")?;
        clock_s(&rates.night.ends_at, "night.ends_at")?;
        let l = &rates.levels;
        if !(0.0 < l.engaged && l.engaged < l.clouded && l.clouded < l.drained && l.drained <= 1.0)
        {
            return Err("levels must rise strictly between 0 and 1".into());
        }
        let s = &rates.session;
        if !(0.0 < s.short_minutes && s.short_minutes < s.long_minutes) {
            return Err("session thresholds must rise".into());
        }
        Ok(rates)
    }

    /// Move the night window. Both times are "HH:MM" and must differ.
    pub fn set_night(&mut self, starts_at: &str, ends_at: &str) -> Result<(), String> {
        if clock_s(starts_at, "the night's start")? == clock_s(ends_at, "the night's end")? {
            return Err("the night can't start and end at the same time".into());
        }
        self.night.starts_at = starts_at.to_owned();
        self.night.ends_at = ends_at.to_owned();
        Ok(())
    }

    fn day_start_s(&self) -> i64 {
        clock_s(&self.day.starts_at, "").expect("validated when parsed")
    }

    /// When a day begins, in minutes after midnight.
    pub fn day_start_minute(&self) -> u32 {
        (self.day_start_s() / 60) as u32
    }

    /// Minutes from the start of `at`'s day (not midnight) to `at`.
    pub fn minute_of_day(&self, at: Moment) -> u32 {
        ((at.local_s() - self.day_start_s()).rem_euclid(DAY_S) / 60) as u32
    }

    /// The night window as minutes from the start of the day: where it
    /// starts, and where it ends (after the start, and possibly past the
    /// day's end).
    pub fn night_minutes(&self) -> (u32, u32) {
        let start = clock_s(&self.night.starts_at, "").expect("validated");
        let end = clock_s(&self.night.ends_at, "").expect("validated");
        let from = (start - self.day_start_s()).rem_euclid(DAY_S) / 60;
        let mut until = (end - self.day_start_s()).rem_euclid(DAY_S) / 60;
        if until <= from {
            until += DAY_S / 60;
        }
        (from as u32, until as u32)
    }

    /// Dose added per minute on a site of weight -1, before any factors.
    fn climb(&self) -> f64 {
        self.levels.drained / self.draining.minutes_to_drained
    }

    /// Fraction of usual recovery speed with `tabs` untouched tabs.
    pub fn recovery_speed(&self, tabs: u32) -> f64 {
        let full = self.clutter.tabs_for_full_effect.max(1);
        let share = f64::from(tabs.min(full)) / f64::from(full);
        1.0 - share * (1.0 - self.clutter.slowest_recovery)
    }

    fn session_factor(&self, minutes: f64) -> f64 {
        let s = &self.session;
        if minutes < s.short_minutes {
            s.short_factor
        } else if minutes < s.long_minutes {
            1.0
        } else {
            s.long_factor
        }
    }

    /// Whether `at` falls in the night window.
    pub fn is_night(&self, at: Moment) -> bool {
        let start = clock_s(&self.night.starts_at, "").expect("validated");
        let end = clock_s(&self.night.ends_at, "").expect("validated");
        let t = at.local_s().rem_euclid(DAY_S);
        if start <= end {
            (start..end).contains(&t)
        } else {
            t >= start || t < end
        }
    }

    /// The next moment the night starts or ends, after `at`.
    fn next_night_change(&self, at: Moment) -> Moment {
        let start = clock_s(&self.night.starts_at, "").expect("validated");
        let end = clock_s(&self.night.ends_at, "").expect("validated");
        let local = at.local_s();
        let midnight = local - local.rem_euclid(DAY_S);
        let next = [start, end]
            .into_iter()
            .flat_map(|t| [midnight + t, midnight + t + DAY_S])
            .filter(|&t| t > local)
            .min()
            .expect("tomorrow is always later");
        Moment {
            ms: (next - i64::from(at.utc_offset_s)) * 1000,
            utc_offset_s: at.utc_offset_s,
        }
    }
}

/// Where the user's attention is, as the dose engine needs to know it.
#[derive(Clone, Debug, PartialEq)]
pub enum Place {
    /// A site on one of the weighted lists. `entry` is the list entry that
    /// matched (`reddit.com/r/diy`), never the address visited.
    Listed {
        entry: String,
        weight: f64,
        /// On the news list, with its daily allowance.
        news: bool,
    },
    /// A site on no list, a carve-out, or a local page: holds steady.
    Unlisted,
    /// An adult site: holds steady and leaves no trace.
    Private,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Activity {
    Away,
    /// Present on `place` until `until`, unless renewed. The lease is what
    /// makes a laptop lid closed mid-scroll count as away: no renewal arrives
    /// while suspended, so the time after the lease ran out is rest.
    ///
    /// `heard` is a tab playing sound that isn't on screen.
    Present {
        place: Place,
        heard: Option<Place>,
        until: Moment,
    },
    /// Not present, but a tab is playing sound until `until`. Only a draining
    /// site counts; anything else heard is the same as being away.
    Listening {
        heard: Place,
        until: Moment,
    },
}

impl Activity {
    fn until(&self) -> Option<Moment> {
        match self {
            Activity::Away => None,
            Activity::Present { until, .. } | Activity::Listening { until, .. } => Some(*until),
        }
    }
}

/// The draining site that counts right now, if any: the page on screen, or a
/// heard site under a page that isn't draining.
struct Drain<'a> {
    entry: &'a str,
    weight: f64,
    news: bool,
    /// ×1 for the page on screen, less for a site only heard.
    factor: f64,
    heard: bool,
}

fn draining_listed(place: &Place) -> Option<(&str, f64, bool)> {
    match place {
        Place::Listed {
            entry,
            weight,
            news,
        } if *weight < 0.0 => Some((entry, *weight, *news)),
        _ => None,
    }
}

fn drain<'a>(rates: &Rates, activity: &'a Activity) -> Option<Drain<'a>> {
    let heard = |place: &'a Place| {
        draining_listed(place).map(|(entry, weight, news)| Drain {
            entry,
            weight,
            news,
            factor: rates.heard.factor,
            heard: true,
        })
    };
    match activity {
        Activity::Away => None,
        Activity::Present {
            place, heard: h, ..
        } => match draining_listed(place) {
            Some((entry, weight, news)) => Some(Drain {
                entry,
                weight,
                news,
                factor: 1.0,
                heard: false,
            }),
            None => h.as_ref().and_then(heard),
        },
        Activity::Listening { heard: h, .. } => heard(h),
    }
}

/// A heard nourishing site that rests the dose under an ordinary or unlisted
/// page: its entry and the weight the page counts as.
fn heard_rest<'a>(rates: &Rates, activity: &'a Activity) -> Option<(&'a str, f64)> {
    let Activity::Present {
        place,
        heard: Some(Place::Listed { entry, weight, .. }),
        ..
    } = activity
    else {
        return None;
    };
    let page_rests_plainly = match place {
        Place::Unlisted => true,
        Place::Listed { weight, .. } => *weight == 0.0,
        Place::Private => false,
    };
    (page_rests_plainly && *weight > 0.0).then(|| (entry.as_str(), weight * rates.heard.factor))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Rested,
    Engaged,
    Clouded,
    Drained,
}

/// What the dose is doing right now, by the kind of place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Away,
    Draining,
    Resting,
    Nourishing,
    Holding,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trend {
    Rising,
    Falling,
    Steady,
}

/// One line of the hover caption.
#[derive(Clone, Debug, PartialEq)]
pub struct Factor {
    pub what: FactorKind,
    /// 1 to 3, by share of the movement; 0 for clutter, which moves nothing
    /// itself.
    pub bars: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FactorKind {
    /// `heard`: a tab playing sound, not the page on screen.
    Wearing {
        entry: String,
        heard: bool,
    },
    Restoring {
        entry: String,
        heard: bool,
    },
    OrdinarySites,
    Away,
    /// Untouched tabs slowed the recovery above.
    Clutter {
        tabs: u32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Effect {
    /// Dose added per minute.
    Climb(f64),
    /// Minutes for the dose to halve.
    Halve(f64),
    Hold,
}

impl Effect {
    fn apply(self, dose: f64, span: f64) -> f64 {
        match self {
            Effect::Climb(per_minute) => (dose + per_minute * span).min(1.0),
            Effect::Halve(half_life) => dose * 0.5_f64.powf(span / half_life),
            Effect::Hold => dose,
        }
    }
}

/// A stretch of time over which nothing changed but the clock.
#[derive(Clone, Debug)]
struct Segment {
    start: i64,
    end: i64,
    dose_at_start: f64,
    activity: Activity,
    effect: Effect,
    /// The same stretch without clutter, to say how much clutter slowed it.
    unhindered: Effect,
    clutter: u32,
}

pub struct Engine {
    rates: Rates,
    dose: f64,
    at: Moment,
    /// Weighted draining minutes so far today, night minutes counted extra.
    day_load: f64,
    activity: Activity,
    clutter: u32,
    /// Minutes of the current draining session, and minutes since draining
    /// stopped (a long enough break starts a new session).
    session: f64,
    off_draining: f64,
    /// Minutes of news so far today.
    news_today: f64,
    recent: VecDeque<Segment>,
}

/// Enough to pick up where the engine left off after a restart.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub dose: f64,
    pub at: Moment,
    pub day_load: f64,
}

impl Engine {
    pub fn new(rates: Rates, at: Moment) -> Engine {
        Engine::resume(
            rates,
            Snapshot {
                dose: 0.0,
                at,
                day_load: 0.0,
            },
        )
    }

    /// Continue from a snapshot. Time since it was taken counts as away.
    pub fn resume(rates: Rates, snapshot: Snapshot) -> Engine {
        let break_minutes = rates.session.break_minutes;
        Engine {
            rates,
            dose: snapshot.dose.clamp(0.0, 1.0),
            at: snapshot.at,
            day_load: snapshot.day_load.max(0.0),
            activity: Activity::Away,
            clutter: 0,
            session: 0.0,
            off_draining: break_minutes,
            news_today: 0.0,
            recent: VecDeque::new(),
        }
    }

    pub fn rates(&self) -> &Rates {
        &self.rates
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            dose: self.dose,
            at: self.at,
            day_load: self.day_load,
        }
    }

    pub fn dose(&self) -> f64 {
        self.dose
    }

    pub fn activity(&self) -> &Activity {
        &self.activity
    }

    pub fn phase(&self) -> Phase {
        self.rates.levels.phase(self.dose)
    }

    /// By the page on screen: a site only heard doesn't change how the wisp
    /// looks at the page, though it moves the dose.
    pub fn mode(&self) -> Mode {
        match &self.activity {
            Activity::Away => Mode::Away,
            Activity::Listening { .. } if drain(&self.rates, &self.activity).is_some() => {
                Mode::Draining
            }
            Activity::Listening { .. } => Mode::Away,
            Activity::Present { place, .. } => match place {
                Place::Unlisted | Place::Private => Mode::Holding,
                Place::Listed { weight, .. } if *weight < 0.0 => Mode::Draining,
                Place::Listed { weight, .. } if *weight > 0.0 => Mode::Nourishing,
                Place::Listed { .. } => Mode::Resting,
            },
        }
    }

    pub fn trend(&self) -> Trend {
        match self.effect(self.at) {
            Effect::Climb(_) if self.dose < 1.0 => Trend::Rising,
            Effect::Halve(_) if self.dose > 0.001 => Trend::Falling,
            _ => Trend::Steady,
        }
    }

    /// Whether a site that's only heard is moving the dose right now.
    pub fn heard_counts(&self) -> bool {
        self.draining().is_some_and(|d| d.heard)
            || heard_rest(&self.rates, &self.activity).is_some()
    }

    /// Bring the dose up to `now`.
    pub fn advance(&mut self, now: Moment) {
        if now.ms < self.at.ms {
            // The clock went backwards (a manual change, a bad NTP step).
            // Nothing happened in "negative time"; re-anchor and carry on.
            self.at = now;
            self.recent.clear();
            return;
        }
        while self.at.ms < now.ms {
            if self
                .activity
                .until()
                .is_some_and(|until| until.ms <= self.at.ms)
            {
                self.activity = Activity::Away;
            }
            let day = next_day_start(&self.rates, self.at);
            let end = [now.ms, day.ms, self.next_split().ms]
                .into_iter()
                .min()
                .expect("now is always a candidate")
                .max(self.at.ms + 1);
            self.run(end);
            self.at = Moment {
                ms: end,
                utc_offset_s: now.utc_offset_s,
            };
            if end == day.ms {
                self.start_day();
            }
        }
        self.forget_before(now.ms - self.caption_window_ms());
    }

    pub fn set_activity(&mut self, now: Moment, activity: Activity) {
        self.advance(now);
        self.activity = activity;
    }

    /// The user moved the night: everything up to `now` stays as it was.
    pub fn set_night(&mut self, now: Moment, starts_at: &str, ends_at: &str) -> Result<(), String> {
        self.advance(now);
        self.rates.set_night(starts_at, ends_at)
    }

    pub fn set_clutter(&mut self, now: Moment, tabs: u32) {
        self.advance(now);
        self.clutter = tabs;
    }

    /// The next moment anything visible changes on its own: a level crossed,
    /// a rule's rate changing (session, allowance, night), the presence lease
    /// running out, or a new day.
    pub fn next_change(&self) -> Moment {
        let levels = {
            let l = &self.rates.levels;
            [l.engaged, l.clouded, l.drained]
        };
        let mut next = self
            .next_split()
            .ms
            .min(next_day_start(&self.rates, self.at).ms);
        let minutes = match self.effect(self.at) {
            Effect::Climb(per_minute) => levels
                .iter()
                .find(|&&level| level > self.dose + 1e-9)
                .map(|level| (level - self.dose) / per_minute),
            Effect::Halve(half_life) => levels
                .iter()
                .rev()
                .find(|&&level| level < self.dose - 1e-9)
                .map(|level| half_life * (self.dose / level).log2()),
            Effect::Hold => None,
        };
        if let Some(minutes) = minutes {
            // Land just past the crossing so the phase has changed on arrival.
            next = next.min(self.at.ms + (minutes * MINUTE_MS).ceil() as i64 + 1);
        }
        Moment {
            ms: next,
            utc_offset_s: self.at.utc_offset_s,
        }
    }

    /// What moved the dose over the caption window before `now`, strongest
    /// first. Empty means steady.
    pub fn caption(&self, now: Moment) -> Vec<Factor> {
        let from = now.ms - self.caption_window_ms();
        let current = Segment {
            start: self.at.ms,
            end: now.ms.max(self.at.ms),
            dose_at_start: self.dose,
            activity: self.activity.clone(),
            effect: self.effect(self.at),
            unhindered: self.effect_with_clutter(self.at, 0),
            clutter: self.clutter,
        };
        let mut moved: Vec<(FactorKind, f64)> = Vec::new();
        let mut slowed = 0.0;
        let mut slowing_tabs = 0;
        for seg in self.recent.iter().chain(std::iter::once(&current)) {
            let (start, end) = (seg.start.max(from), seg.end.min(now.ms));
            if end <= start {
                continue;
            }
            let d0 = seg
                .effect
                .apply(seg.dose_at_start, minutes(start - seg.start));
            let span = minutes(end - start);
            let delta = seg.effect.apply(d0, span) - d0;
            if delta.abs() < 1e-9 {
                continue;
            }
            if delta < 0.0 && seg.clutter > 0 {
                slowed += delta - (seg.unhindered.apply(d0, span) - d0);
                slowing_tabs = seg.clutter;
            }
            let kind = if let Some(d) = drain(&self.rates, &seg.activity) {
                FactorKind::Wearing {
                    entry: d.entry.to_string(),
                    heard: d.heard,
                }
            } else if let Some((entry, _)) = heard_rest(&self.rates, &seg.activity) {
                FactorKind::Restoring {
                    entry: entry.to_string(),
                    heard: true,
                }
            } else {
                match &seg.activity {
                    Activity::Away | Activity::Listening { .. } => FactorKind::Away,
                    Activity::Present {
                        place: Place::Listed { entry, weight, .. },
                        ..
                    } if *weight > 0.0 => FactorKind::Restoring {
                        entry: entry.clone(),
                        heard: false,
                    },
                    Activity::Present { .. } => FactorKind::OrdinarySites,
                }
            };
            match moved.iter_mut().find(|(k, _)| *k == kind) {
                Some((_, total)) => *total += delta.abs(),
                None => moved.push((kind, delta.abs())),
            }
        }
        let total: f64 = moved.iter().map(|(_, d)| d).sum();
        if total < 0.002 {
            return Vec::new();
        }
        moved.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut factors: Vec<Factor> = moved
            .into_iter()
            .filter(|(_, d)| d / total >= 0.05)
            .map(|(what, d)| Factor {
                what,
                bars: match d / total {
                    s if s >= 0.5 => 3,
                    s if s >= 0.2 => 2,
                    _ => 1,
                },
            })
            .collect();
        if slowed > 0.001 {
            factors.push(Factor {
                what: FactorKind::Clutter { tabs: slowing_tabs },
                bars: 0,
            });
        }
        factors
    }

    fn caption_window_ms(&self) -> i64 {
        (self.rates.caption.window * MINUTE_MS) as i64
    }

    fn draining(&self) -> Option<Drain<'_>> {
        drain(&self.rates, &self.activity)
    }

    fn effect(&self, at: Moment) -> Effect {
        self.effect_with_clutter(at, self.clutter)
    }

    fn effect_with_clutter(&self, at: Moment, clutter: u32) -> Effect {
        let r = &self.rates;
        let night = r.is_night(at);
        let speed = r.recovery_speed(clutter);
        let away = r.rest.away_half_life;
        if let Some(d) = self.draining() {
            // A session that has been broken for long enough starts over when
            // draining resumes.
            let session = if self.off_draining >= r.session.break_minutes {
                0.0
            } else {
                self.session
            };
            let mut rate = r.climb() * d.weight.abs() * r.session_factor(session) * d.factor;
            if night {
                rate *= r.night.draining_factor;
            }
            if d.news && self.news_today < r.news.allowance_minutes {
                rate *= r.news.allowance_factor;
            }
            return Effect::Climb(rate);
        }
        let weight = match &self.activity {
            Activity::Away | Activity::Listening { .. } => return Effect::Halve(away / speed),
            Activity::Present { place, .. } => match (place, heard_rest(r, &self.activity)) {
                (Place::Private, _) => return Effect::Hold,
                (_, Some((_, weight))) => weight,
                (Place::Unlisted, None) => return Effect::Hold,
                (Place::Listed { weight, .. }, None) => *weight,
            },
        };
        if night && weight == 0.0 {
            Effect::Hold
        } else if night {
            Effect::Halve(away * 2.0 / speed)
        } else {
            Effect::Halve(away * (2.0 - weight) / speed)
        }
    }

    /// The next moment a rule's rate changes on its own, not counting the
    /// start of a new day: the presence lease ends, a session threshold or
    /// the news allowance is reached, the night starts or ends.
    fn next_split(&self) -> Moment {
        let r = &self.rates;
        let mut next = r.next_night_change(self.at);
        if let Some(until) = self.activity.until()
            && until.ms > self.at.ms
        {
            next = Moment {
                ms: next.ms.min(until.ms),
                ..next
            };
        }
        if let Some(Drain { news, .. }) = self.draining() {
            let session = if self.off_draining >= r.session.break_minutes {
                0.0
            } else {
                self.session
            };
            let mut minutes: Vec<f64> = [r.session.short_minutes, r.session.long_minutes]
                .into_iter()
                .filter(|&t| t > session)
                .map(|t| t - session)
                .collect();
            if news && self.news_today < r.news.allowance_minutes {
                minutes.push(r.news.allowance_minutes - self.news_today);
            }
            if let Some(m) = minutes.into_iter().reduce(f64::min) {
                next = Moment {
                    ms: next.ms.min(self.at.plus_minutes(m).ms),
                    ..next
                };
            }
        }
        next
    }

    /// Move the dose from `self.at` to `end`, over which nothing changes.
    fn run(&mut self, end: i64) {
        let span = minutes(end - self.at.ms);
        if span <= 0.0 {
            return;
        }
        let effect = self.effect(self.at);
        self.recent.push_back(Segment {
            start: self.at.ms,
            end,
            dose_at_start: self.dose,
            activity: self.activity.clone(),
            effect,
            unhindered: self.effect_with_clutter(self.at, 0),
            clutter: self.clutter,
        });
        self.dose = effect.apply(self.dose, span);

        match self.draining().map(|d| (d.weight * d.factor, d.news)) {
            Some((weight, news)) => {
                if self.off_draining >= self.rates.session.break_minutes {
                    self.session = 0.0;
                }
                self.off_draining = 0.0;
                self.session += span;
                if news {
                    self.news_today += span;
                }
                let night = if self.rates.is_night(self.at) {
                    self.rates.night.carry_factor
                } else {
                    1.0
                };
                self.day_load += weight.abs() * span * night;
            }
            None => self.off_draining += span,
        }
    }

    fn start_day(&mut self) {
        let carry =
            self.rates.day.carry_max * (self.day_load / self.rates.day.carry_full_load).min(1.0);
        self.dose = carry;
        self.day_load = 0.0;
        self.news_today = 0.0;
    }

    fn forget_before(&mut self, ms: i64) {
        while self.recent.front().is_some_and(|seg| seg.end <= ms) {
            self.recent.pop_front();
        }
    }
}

fn minutes(ms: i64) -> f64 {
    ms as f64 / MINUTE_MS
}

/// Days are numbered from the local day start, not midnight.
pub fn day_of(rates: &Rates, at: Moment) -> i64 {
    (at.local_s() - rates.day_start_s()).div_euclid(DAY_S)
}

fn next_day_start(rates: &Rates, at: Moment) -> Moment {
    let local_boundary = (day_of(rates, at) + 1) * DAY_S + rates.day_start_s();
    Moment {
        ms: (local_boundary - i64::from(at.utc_offset_s)) * 1000,
        utc_offset_s: at.utc_offset_s,
    }
}

#[cfg(test)]
mod tests {
    //! Simulated days. Each script is a short timeline in local time from
    //! 15 September 2026 (BST, UTC+1); see `run` for the vocabulary.

    use super::*;

    const BST: i32 = 3600;
    /// 2026-09-15 00:00 BST.
    const MIDNIGHT_MS: i64 = 1_789_426_800_000;

    /// "HH:MM" on 15 September, or "N/HH:MM" N days later.
    fn at(time: &str) -> Moment {
        let (day, hhmm) = match time.split_once('/') {
            Some((d, t)) => (d.parse::<i64>().expect("day"), t),
            None => (0, time),
        };
        let (h, m) = hhmm.split_once(':').expect("HH:MM");
        let (h, m): (i64, i64) = (h.parse().expect("hour"), m.parse().expect("minute"));
        Moment {
            ms: MIDNIGHT_MS + (day * 86_400 + h * 3600 + m * 60) * 1000,
            utc_offset_s: BST,
        }
    }

    fn listed(entry: &str, weight: f64) -> Place {
        Place::Listed {
            entry: entry.into(),
            weight,
            news: false,
        }
    }

    fn site(entry: &str, weight: f64, until: Moment) -> Activity {
        Activity::Present {
            place: listed(entry, weight),
            heard: None,
            until,
        }
    }

    /// `draining W ENTRY`, `news ENTRY`, `nourishing W ENTRY`,
    /// `ordinary ENTRY`, `unlisted` or `private`.
    fn place_of(words: &[&str]) -> Option<Place> {
        let number = |i: usize| words[i].parse::<f64>().expect("number");
        Some(match words {
            ["draining", _, entry] => listed(entry, -number(1).abs()),
            ["news", entry] => Place::Listed {
                entry: (*entry).into(),
                weight: -0.4,
                news: true,
            },
            ["nourishing", _, entry] => listed(entry, number(1)),
            ["ordinary", entry] => listed(entry, 0.0),
            ["unlisted"] => Place::Unlisted,
            ["private"] => Place::Private,
            _ => return None,
        })
    }

    fn engine_at(time: &str, dose: f64) -> Engine {
        Engine::resume(
            Rates::bundled(),
            Snapshot {
                dose,
                at: at(time),
                day_load: 0.0,
            },
        )
    }

    fn assert_near(actual: f64, expected: f64, tolerance: f64) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "expected {expected} ± {tolerance}, got {actual}"
        );
    }

    /// A simulated day. Lines are `TIME what`, applied in order, where TIME is
    /// `HH:MM` or `N/HH:MM`:
    ///   `draining W ENTRY`, `news ENTRY` (weight -0.4), `nourishing W ENTRY`,
    ///   `ordinary ENTRY`, `unlisted`, `private`, `away`: what the user is
    ///   doing from then on (presence is leased to the next such line),
    ///   optionally followed by `+ heard PLACE` for a tab playing sound;
    ///   `listening PLACE`: away from the window with PLACE playing;
    ///   `clutter N`: untouched tabs from then on;
    ///   `expect dose X ± T`, `expect phase NAME`: checks at that time.
    fn run(script: &str) -> Engine {
        let lines: Vec<(Moment, Vec<&str>)> = script
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| {
                let mut words = l.split_whitespace();
                let time = at(words.next().expect("time"));
                (time, words.collect())
            })
            .collect();
        let mut engine = Engine::new(Rates::bundled(), lines[0].0);
        for (i, (time, words)) in lines.iter().enumerate() {
            // Presence lasts until the next thing the user does, not the next
            // check or clutter change.
            let lease = lines
                .iter()
                .skip(i + 1)
                .filter(|(_, w)| !matches!(w.first(), Some(&"expect" | &"clutter")))
                .map(|(t, _)| *t)
                .find(|t| t.ms > time.ms)
                .unwrap_or(Moment {
                    ms: time.ms + 2 * 86_400_000,
                    utc_offset_s: BST,
                });
            let number = |i: usize| words[i].parse::<f64>().expect("number");
            let (doing, heard) = match words.iter().position(|w| *w == "+") {
                Some(i) => {
                    assert_eq!(words.get(i + 1), Some(&"heard"), "`+ heard PLACE`");
                    (&words[..i], place_of(&words[i + 2..]))
                }
                None => (&words[..], None),
            };
            if let Some(place) = place_of(doing) {
                engine.set_activity(
                    *time,
                    Activity::Present {
                        place,
                        heard,
                        until: lease,
                    },
                );
                continue;
            }
            if let ["listening", rest @ ..] = doing {
                let heard = place_of(rest).expect("listening PLACE");
                engine.set_activity(
                    *time,
                    Activity::Listening {
                        heard,
                        until: lease,
                    },
                );
                continue;
            }
            match words.as_slice() {
                ["away"] => engine.set_activity(*time, Activity::Away),
                ["clutter", _] => engine.set_clutter(*time, number(1) as u32),
                ["expect", "dose", _, "±", _] => {
                    engine.advance(*time);
                    assert_near(engine.dose(), number(2), number(4));
                }
                ["expect", "phase", name] => {
                    engine.advance(*time);
                    assert_eq!(format!("{:?}", engine.phase()).to_lowercase(), *name);
                }
                other => panic!("unknown script line {other:?}"),
            }
        }
        engine
    }

    #[test]
    fn bundled_rates_are_the_designed_defaults() {
        let r = Rates::bundled();
        assert_eq!(
            (r.levels.engaged, r.levels.clouded, r.levels.drained),
            (0.2, 0.5, 0.75)
        );
        assert_eq!(r.day.starts_at, "05:00");
        assert_eq!(r.day.carry_max, 0.1);
        assert_eq!(
            (r.night.starts_at.as_str(), r.night.ends_at.as_str()),
            ("23:00", "05:00")
        );
        assert_eq!(r.presence.input_seconds, 240);
        assert_eq!(r.rest.away_half_life, 25.0);
    }

    // --- Draining ------------------------------------------------------------

    #[test]
    fn a_continuous_session_on_a_fully_draining_site_reaches_drained_in_about_fifty_minutes() {
        // x0.5 for 10 minutes, x1.0 to 30, x1.4 beyond.
        run("
            09:00 draining 1 tiktok.com
            09:10 expect dose 0.075 ± 0.0001
            09:30 expect dose 0.375 ± 0.0001
            09:47 expect phase clouded
            09:48 expect phase drained
        ");
    }

    #[test]
    fn milder_sites_climb_in_proportion_and_the_dose_tops_out_at_one() {
        run("
            09:00 draining 0.4 dailymail.co.uk
            09:30 expect dose 0.15 ± 0.0001
            09:50 expect dose 0.318 ± 0.0001
        ");
        run("
            09:00 draining 1 tiktok.com
            11:00 expect dose 1 ± 0
        ");
    }

    #[test]
    fn short_visits_cost_less_than_one_long_session() {
        let visits = run("
            09:00 draining 1 tiktok.com
            09:10 away
            10:00 draining 1 tiktok.com
            10:10 away
            11:00 draining 1 tiktok.com
            11:10 away
            11:10 expect dose 0.0 ± 1
        ");
        let session = run("
            11:00 draining 1 tiktok.com
            11:30 away
            11:30 expect dose 0.375 ± 0.0001
        ");
        assert!(
            visits.dose() < session.dose() / 2.0,
            "{} vs {}",
            visits.dose(),
            session.dose()
        );
    }

    #[test]
    fn a_five_minute_break_starts_a_new_session_and_a_shorter_one_doesnt() {
        run("
            09:00 draining 1 tiktok.com
            09:10 away
            09:14 draining 1 tiktok.com
            09:24 expect dose 0.2171 ± 0.0005
        ");
        run("
            09:00 draining 1 tiktok.com
            09:10 away
            09:15 draining 1 tiktok.com
            09:25 expect dose 0.1403 ± 0.0005
        ");
    }

    #[test]
    fn the_first_fifteen_minutes_of_news_each_day_count_half() {
        let fresh = run("
            08:00 news theguardian.com
            08:10 away
            08:10 expect dose 0.015 ± 0.0001
        ");
        assert_near(fresh.dose(), 0.015, 1e-4);
        // Allowance used earlier; a break resets the session but not the
        // allowance, so the same ten minutes count twice as much.
        let mut spent = run("
            07:00 news theguardian.com
            07:15 away
            08:00 news theguardian.com
        ");
        let before = spent.dose();
        spent.advance(at("08:10"));
        assert_near(spent.dose() - before, 0.03, 1e-4);
    }

    #[test]
    fn the_news_allowance_renews_each_day() {
        let mut engine = run("
            07:00 news theguardian.com
            08:00 away
        ");
        engine.set_activity(
            at("1/08:00"),
            Activity::Present {
                place: Place::Listed {
                    entry: "theguardian.com".into(),
                    weight: -0.4,
                    news: true,
                },
                heard: None,
                until: at("1/08:10"),
            },
        );
        let before = engine.dose();
        engine.advance(at("1/08:10"));
        assert_near(engine.dose() - before, 0.015, 1e-4);
    }

    // --- Rest ----------------------------------------------------------------

    #[test]
    fn time_away_is_the_best_rest() {
        let mut away = engine_at("12:00", 0.8);
        away.set_activity(at("12:00"), Activity::Away);
        away.advance(at("12:25"));
        assert_near(away.dose(), 0.4, 1e-9);

        for (weight, expected) in [
            (1.0, 0.4),
            (0.5, 0.8 * 0.5_f64.powf(25.0 / 37.5)),
            (0.0, 0.8 * 0.5_f64.sqrt()),
        ] {
            let mut present = engine_at("12:00", 0.8);
            present.set_activity(at("12:00"), site("example.org", weight, at("13:00")));
            present.advance(at("12:25"));
            assert_near(present.dose(), expected, 1e-9);
            assert!(
                present.dose() >= away.dose() - 1e-9,
                "weight {weight} rested faster than away"
            );
        }
    }

    #[test]
    fn ordinary_sites_rest_at_half_the_speed_of_being_away() {
        run("
            09:00 draining 1 tiktok.com
            09:40 ordinary gmail.com
            10:30 expect dose 0.2925 ± 0.0005
        ");
    }

    #[test]
    fn unlisted_and_private_sites_hold_the_dose_steady() {
        run("
            09:00 draining 1 tiktok.com
            09:40 unlisted
            11:40 expect dose 0.585 ± 0.0005
        ");
        run("
            09:00 draining 1 tiktok.com
            09:40 private
            11:40 expect dose 0.585 ± 0.0005
        ");
    }

    #[test]
    fn only_presence_counts_background_time_is_away() {
        // The adapter reports a tab in the background, or the window behind
        // another, as away; the engine treats away as rest.
        run("
            09:00 draining 1 tiktok.com
            09:40 away
            10:05 expect dose 0.2925 ± 0.0005
        ");
    }

    #[test]
    fn an_expired_presence_lease_turns_into_rest() {
        // Lid closed at 09:40 mid-scroll: the lease ran out at 09:42 and no
        // renewal came while suspended.
        let mut engine = engine_at("09:00", 0.0);
        engine.set_activity(at("09:00"), site("tiktok.com", -1.0, at("09:42")));
        engine.advance(at("10:07"));
        let at_lease_end = 0.075 + 0.3 + 12.0 * 0.021;
        assert_near(engine.dose(), at_lease_end * 0.5, 1e-6);
        assert_eq!(engine.mode(), Mode::Away);
    }

    #[test]
    fn clutter_slows_recovery_and_never_raises_the_dose() {
        // With eight untouched tabs, rest takes twice as long to halve.
        run("
            09:00 draining 1 tiktok.com
            09:40 clutter 8
            09:40 away
            10:30 expect dose 0.2925 ± 0.0005
        ");
        run("
            09:00 draining 1 tiktok.com
            09:40 clutter 20
            09:40 unlisted
            12:00 expect dose 0.585 ± 0.0005
        ");
        let rates = Rates::bundled();
        assert_eq!(rates.recovery_speed(0), 1.0);
        assert_eq!(rates.recovery_speed(4), 0.75);
        assert_eq!(rates.recovery_speed(8), 0.5);
        assert_eq!(rates.recovery_speed(30), 0.5);
    }

    // --- Night ---------------------------------------------------------------

    #[test]
    fn draining_counts_half_as_much_again_at_night() {
        let day = run("
            15:00 draining 1 tiktok.com
            15:30 away
            15:30 expect dose 0.375 ± 0.0001
        ");
        let night = run("
            1/00:30 draining 1 tiktok.com
            1/01:00 away
            1/01:00 expect dose 0.5625 ± 0.0001
        ");
        assert!(night.dose() > day.dose());
    }

    #[test]
    fn at_night_ordinary_sites_hold_and_nourishing_ones_rest_slowly() {
        // 22:40-23:00 rests at a 50-minute half-life; from 23:00 it holds.
        run("
            22:00 draining 1 tiktok.com
            22:40 ordinary gmail.com
            1/00:30 expect dose 0.4432 ± 0.001
        ");
        let mut engine = engine_at("23:30", 0.8);
        engine.set_activity(at("23:30"), site("khanacademy.org", 1.0, at("1/01:00")));
        engine.advance(at("1/00:20"));
        assert_near(engine.dose(), 0.4, 1e-9);
    }

    #[test]
    fn the_user_can_move_the_night_and_the_past_stays_put() {
        let mut engine = Engine::new(Rates::bundled(), at("21:00"));
        engine.set_activity(at("21:00"), site("feed.example", -1.0, at("23:00")));
        engine.advance(at("22:00"));
        let before = engine.dose();
        engine
            .set_night(at("22:00"), "22:00", "06:00")
            .expect("a valid window");
        assert_eq!(engine.dose(), before);
        assert!(engine.rates().is_night(at("22:30")));
        assert!(engine.rates().is_night(at("1/05:30")));
        assert!(!engine.rates().is_night(at("1/06:00")));
        assert_eq!(engine.rates().night_minutes(), (1020, 1500));
        let mut rates = Rates::bundled();
        assert!(rates.set_night("23:00", "23:00").is_err());
        assert!(rates.set_night("25:00", "05:00").is_err());
        assert!(rates.is_night(at("23:30")));
    }

    #[test]
    fn night_follows_the_window_across_midnight() {
        let rates = Rates::bundled();
        assert!(!rates.is_night(at("22:59")));
        assert!(rates.is_night(at("23:00")));
        assert!(rates.is_night(at("1/02:00")));
        assert!(!rates.is_night(at("1/05:00")));
        assert_eq!(rates.next_night_change(at("12:00")), at("23:00"));
        assert_eq!(rates.next_night_change(at("23:30")), at("1/05:00"));
    }

    // --- Days ----------------------------------------------------------------

    #[test]
    fn a_new_day_starts_at_five_with_a_small_carry_over() {
        // 3 hours fully draining in the evening: full carry of 0.1.
        let mut engine = Engine::new(Rates::bundled(), at("18:00"));
        engine.set_activity(at("18:00"), site("tiktok.com", -1.0, at("21:00")));
        engine.set_activity(at("21:00"), Activity::Away);
        engine.advance(at("1/04:59"));
        assert!(engine.dose() < 0.01, "rest overnight: {}", engine.dose());
        engine.advance(at("1/05:00"));
        assert_near(engine.dose(), 0.1, 1e-9);
    }

    #[test]
    fn carry_over_scales_with_yesterdays_load_and_night_counts_double() {
        // 90 weighted minutes in the evening: half the maximum carry.
        let mut evening = Engine::new(Rates::bundled(), at("18:00"));
        evening.set_activity(at("18:00"), site("tiktok.com", -1.0, at("19:30")));
        evening.set_activity(at("19:30"), Activity::Away);
        evening.advance(at("1/05:00"));
        assert_near(evening.dose(), 0.05, 1e-9);
        // The same 90 minutes late at night: the full carry.
        let mut late = Engine::new(Rates::bundled(), at("23:00"));
        late.set_activity(at("23:00"), site("tiktok.com", -1.0, at("1/00:30")));
        late.set_activity(at("1/00:30"), Activity::Away);
        late.advance(at("1/05:00"));
        assert_near(late.dose(), 0.1, 1e-9);
    }

    #[test]
    fn a_late_night_belongs_to_the_day_it_started_in() {
        let mut engine = Engine::new(Rates::bundled(), at("23:00"));
        engine.set_activity(at("23:00"), site("tiktok.com", -1.0, at("1/01:00")));
        engine.advance(at("1/04:00"));
        assert_near(engine.snapshot().day_load, 240.0, 1e-6);
        engine.advance(at("1/05:01"));
        assert_near(engine.snapshot().day_load, 0.0, 1e-9);
    }

    #[test]
    fn a_day_with_no_draining_starts_the_next_fresh() {
        let mut engine = engine_at("12:00", 0.9);
        engine.advance(at("1/05:00"));
        assert_eq!(engine.dose(), 0.0);
    }

    #[test]
    fn the_clock_going_backwards_changes_nothing() {
        let mut engine = engine_at("12:00", 0.5);
        engine.set_activity(at("12:00"), site("tiktok.com", -1.0, at("13:00")));
        engine.advance(at("12:10"));
        let before = engine.dose();
        engine.advance(at("11:00"));
        assert_eq!(engine.dose(), before);
        engine.advance(at("11:10"));
        assert_near(engine.dose(), (before + 0.15).min(1.0), 1e-9);
    }

    #[test]
    fn resuming_after_a_restart_counts_the_gap_as_rest() {
        let snapshot = Snapshot {
            dose: 0.6,
            at: at("12:00"),
            day_load: 40.0,
        };
        let mut engine = Engine::resume(Rates::bundled(), snapshot);
        engine.advance(at("12:50"));
        assert_near(engine.dose(), 0.15, 1e-9);
        assert_near(engine.snapshot().day_load, 40.0, 1e-9);
    }

    // --- Waking up on time ---------------------------------------------------

    #[test]
    fn next_change_lands_on_rule_changes_and_crossings() {
        let mut engine = engine_at("09:00", 0.0);
        engine.set_activity(at("09:00"), site("tiktok.com", -1.0, at("12:00")));
        // The session factor changes at 10 minutes, before Engaged (0.2).
        assert_eq!(engine.next_change(), at("09:10"));
        engine.advance(at("09:10"));
        // Then 0.075 -> 0.2 at 0.015 a minute: 8 min 20 s.
        let next = engine.next_change();
        assert!(
            (next.ms - at("09:10").ms - 500_000).abs() <= 2,
            "{}",
            next.ms
        );
        engine.advance(next);
        assert_eq!(engine.phase(), Phase::Engaged);

        let mut resting = engine_at("12:00", 0.7);
        resting.set_activity(at("12:00"), Activity::Away);
        let expected = (0.7_f64 / 0.5).log2() * 25.0 * MINUTE_MS;
        let next = resting.next_change();
        assert!((next.ms - at("12:00").ms - expected.ceil() as i64).abs() <= 1);
        resting.advance(next);
        assert_eq!(resting.phase(), Phase::Engaged);
    }

    #[test]
    fn next_change_is_the_lease_the_night_or_the_day_when_nothing_crosses() {
        let mut engine = engine_at("09:00", 0.3);
        engine.set_activity(
            at("09:00"),
            Activity::Present {
                place: Place::Unlisted,
                heard: None,
                until: at("09:02"),
            },
        );
        assert_eq!(engine.next_change(), at("09:02"));
        let mut idle = engine_at("09:00", 0.0);
        idle.set_activity(at("09:00"), Activity::Away);
        assert_eq!(idle.next_change(), at("23:00"));
        let mut late = engine_at("23:30", 0.0);
        late.set_activity(at("23:30"), Activity::Away);
        assert_eq!(late.next_change(), at("1/05:00"));
    }

    // --- The caption ---------------------------------------------------------

    #[test]
    fn caption_names_what_moved_the_dose_strongest_first() {
        let engine = run("
            09:00 draining 1 tiktok.com
            09:40 away
            09:44 draining 1 tiktok.com
            09:55 expect dose 0.755 ± 0.01
        ");
        let caption = engine.caption(at("09:55"));
        assert_eq!(caption.len(), 2, "{caption:?}");
        assert_eq!(
            caption[0].what,
            FactorKind::Wearing {
                entry: "tiktok.com".into(),
                heard: false,
            }
        );
        assert_eq!(caption[0].bars, 3);
        assert_eq!(caption[1].what, FactorKind::Away);
    }

    #[test]
    fn caption_mentions_clutter_only_when_it_slowed_recovery() {
        let engine = run("
            09:00 draining 1 tiktok.com
            09:40 clutter 11
            09:40 ordinary gmail.com
            09:55 expect phase clouded
        ");
        let caption = engine.caption(at("09:55"));
        assert_eq!(caption[0].what, FactorKind::OrdinarySites);
        assert_eq!(
            caption.last().map(|f| &f.what),
            Some(&FactorKind::Clutter { tabs: 11 })
        );

        let climbing = run("
            09:00 clutter 11
            09:00 draining 1 tiktok.com
            09:20 expect phase engaged
        ");
        assert!(
            !climbing
                .caption(at("09:20"))
                .iter()
                .any(|f| matches!(f.what, FactorKind::Clutter { .. }))
        );
    }

    #[test]
    fn caption_is_empty_when_nothing_moved() {
        let engine = run("
            09:00 unlisted
            09:30 expect dose 0 ± 0
        ");
        assert!(engine.caption(at("09:30")).is_empty());
        let rested = run("
            09:00 away
            10:00 expect dose 0 ± 0
        ");
        assert!(rested.caption(at("10:00")).is_empty());
    }

    #[test]
    fn caption_only_looks_back_fifteen_minutes() {
        let engine = run("
            09:00 draining 1 tiktok.com
            09:30 nourishing 1 khanacademy.org
            10:00 expect phase rested
        ");
        let caption = engine.caption(at("10:00"));
        assert_eq!(caption.len(), 1, "{caption:?}");
        assert_eq!(
            caption[0].what,
            FactorKind::Restoring {
                entry: "khanacademy.org".into(),
                heard: false,
            }
        );
    }

    #[test]
    fn a_heard_draining_site_counts_half_under_a_page_that_isnt_draining() {
        // 40 minutes of a fully draining site on screen reach 0.585 (see the
        // session test); heard under a page that doesn't drain, half that,
        // with the same session ramp.
        run("
            09:00 unlisted + heard draining 1 youtube.com
            09:40 expect dose 0.2925 ± 0.0005
        ");
        run("
            09:00 nourishing 1 khanacademy.org + heard draining 1 youtube.com
            09:40 expect dose 0.2925 ± 0.0005
        ");
    }

    #[test]
    fn heard_sound_never_adds_to_a_draining_page() {
        run("
            09:00 draining 1 tiktok.com + heard draining 1 youtube.com
            09:40 expect dose 0.585 ± 0.0005
        ");
    }

    #[test]
    fn heard_nourishing_sound_turns_holding_into_gentle_rest() {
        // Unlisted holds; with a strongly nourishing site heard it rests as if
        // its weight were 0.5, halving every 37.5 minutes: two halvings.
        let mut engine = engine_at("09:00", 0.5);
        engine.set_activity(
            at("09:00"),
            Activity::Present {
                place: Place::Unlisted,
                heard: Some(listed("calm.com", 1.0)),
                until: at("12:00"),
            },
        );
        engine.advance(at("10:15"));
        assert_near(engine.dose(), 0.125, 1e-6);

        // A private page stays exactly as it was.
        let mut private = engine_at("09:00", 0.5);
        private.set_activity(
            at("09:00"),
            Activity::Present {
                place: Place::Private,
                heard: Some(listed("calm.com", 1.0)),
                until: at("12:00"),
            },
        );
        private.advance(at("10:15"));
        assert_near(private.dose(), 0.5, 1e-9);
    }

    #[test]
    fn heard_sound_that_isnt_draining_or_nourishing_changes_nothing() {
        for heard in [
            listed("bbc.co.uk/sounds", 0.0),
            Place::Unlisted,
            Place::Private,
        ] {
            let mut engine = engine_at("09:00", 0.5);
            engine.set_activity(
                at("09:00"),
                Activity::Present {
                    place: Place::Unlisted,
                    heard: Some(heard),
                    until: at("12:00"),
                },
            );
            engine.advance(at("10:00"));
            assert_near(engine.dose(), 0.5, 1e-9);
            assert!(engine.caption(at("10:00")).is_empty());
        }
    }

    #[test]
    fn listening_away_from_the_window_counts_half_until_it_lapses() {
        // 10 minutes ×0.5 session ×0.5 heard, 20 minutes ×0.5 heard, then
        // away from 09:30: ten minutes of rest.
        let engine = run("
            09:00 listening draining 1 youtube.com
            09:30 away
            09:30 expect dose 0.1875 ± 0.0005
            09:40 expect dose 0.1421 ± 0.0005
        ");
        assert_eq!(engine.mode(), Mode::Away);
    }

    #[test]
    fn music_never_makes_being_away_less_restful() {
        let mut engine = engine_at("09:00", 0.5);
        engine.set_activity(
            at("09:00"),
            Activity::Listening {
                heard: listed("calm.com", 1.0),
                until: at("12:00"),
            },
        );
        engine.advance(at("09:25"));
        assert_near(engine.dose(), 0.25, 1e-6);
    }

    #[test]
    fn caption_says_when_a_site_is_only_heard() {
        let engine = run("
            09:00 unlisted + heard draining 1 youtube.com
            09:14 expect phase rested
        ");
        assert_eq!(
            engine.caption(at("09:14"))[0].what,
            FactorKind::Wearing {
                entry: "youtube.com".into(),
                heard: true,
            }
        );
        let resting = run("
            09:00 draining 1 tiktok.com
            09:30 ordinary gmail.com + heard nourishing 1 calm.com
            09:50 expect phase engaged
        ");
        assert_eq!(
            resting.caption(at("09:50"))[0].what,
            FactorKind::Restoring {
                entry: "calm.com".into(),
                heard: true,
            }
        );
    }

    #[test]
    fn day_boundaries_follow_the_local_offset() {
        // 05:00 BST is 04:00 UTC.
        let rates = Rates::bundled();
        let boundary = next_day_start(&rates, at("12:00"));
        assert_eq!(boundary, at("1/05:00"));
        assert_eq!((boundary.ms / 1000).rem_euclid(86_400), 4 * 3600);
    }

    #[test]
    fn bad_rates_are_rejected() {
        let good = include_str!("../data/dose.toml");
        assert!(
            Rates::parse(&good.replace("starts_at = \"05:00\"", "starts_at = \"5am\"")).is_err()
        );
        assert!(
            Rates::parse(&good.replace("starts_at = \"23:00\"", "starts_at = \"late\"")).is_err()
        );
        assert!(Rates::parse(&good.replace("clouded = 0.5", "clouded = 0.1")).is_err());
        assert!(Rates::parse(&good.replace("long_minutes = 30.0", "long_minutes = 5.0")).is_err());
    }
}

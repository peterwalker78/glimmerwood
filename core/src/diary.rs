//! The wisp's history on Home: today's line, the last seven days,
//! the weeks before them, and the list entries that moved it this week.
//!
//! Pure: built from recorded minutes, the garden and the clock. Sizes are only
//! ever compared with each other, so nothing here becomes minutes on screen.

use std::collections::BTreeMap;

use crate::dose::{Mode, Moment, Phase, Place, Rates, day_of};
use crate::home::Plant;
use crate::protocol::{
    DiaryDay, DiarySite, DiaryWeek, DosePoint, HomeWisp, PlantKind, TimeKind, TimeShare, WispPhase,
};
use crate::store::{GardenDay, Sample};

/// Today's line keeps one point for every this many minutes.
const STEP_MINUTES: i64 = 5;
/// This long with nothing recorded, and the browser was closed: the line
/// breaks.
const GAP_MINUTES: i64 = 10;
/// Weeks shown before the last seven days.
pub const WEEKS_BEFORE: i64 = 4;
/// Entries named for each kind, and the time a week needs to be named.
const SITES_EACH: usize = 3;
const SITE_MINUTES: f64 = 5.0;

/// How many days of history `build` looks at, counting today.
pub const DAYS: i64 = 7 * (WEEKS_BEFORE + 1);

/// Time here on one day, by kind, and how heavy the wisp got.
#[derive(Clone, Debug, Default)]
struct Tally {
    restoring: f64,
    everyday: f64,
    wearing: f64,
    peak: Option<f64>,
}

impl Tally {
    fn total(&self) -> f64 {
        self.restoring + self.everyday + self.wearing
    }

    fn add(&mut self, other: &Tally) {
        self.restoring += other.restoring;
        self.everyday += other.everyday;
        self.wearing += other.wearing;
    }

    fn shares(&self) -> Vec<TimeShare> {
        let total = self.total();
        if total <= 0.0 {
            return Vec::new();
        }
        [
            (TimeKind::Restoring, self.restoring),
            (TimeKind::Everyday, self.everyday),
            (TimeKind::Wearing, self.wearing),
        ]
        .into_iter()
        .filter(|(_, minutes)| *minutes > 0.0)
        .map(|(kind, minutes)| TimeShare {
            kind,
            share: minutes / total,
        })
        .collect()
    }
}

/// Time away isn't counted. A tab only heard while away counts where it
/// moved the dose, as the engine's mode says.
fn kind(mode: Mode) -> Option<TimeKind> {
    match mode {
        Mode::Away => None,
        Mode::Draining => Some(TimeKind::Wearing),
        Mode::Nourishing => Some(TimeKind::Restoring),
        Mode::Resting | Mode::Holding => Some(TimeKind::Everyday),
    }
}

/// `samples` are the recorded minutes of the last `DAYS` days, oldest first;
/// `dose` is the dose right now, which may not be recorded yet.
pub fn build(
    rates: &Rates,
    samples: &[Sample],
    garden: &[GardenDay],
    now: Moment,
    dose: f64,
) -> HomeWisp {
    let today = day_of(rates, now);
    let first_day = samples.first().map_or(today, |s| day_of(rates, s.at));

    let mut days: BTreeMap<i64, Tally> = BTreeMap::new();
    for sample in samples {
        let tally = days.entry(day_of(rates, sample.at)).or_default();
        tally.peak = Some(tally.peak.map_or(sample.dose, |p| p.max(sample.dose)));
        match kind(sample.mode) {
            Some(TimeKind::Restoring) => tally.restoring += 1.0,
            Some(TimeKind::Everyday) => tally.everyday += 1.0,
            Some(TimeKind::Wearing) => tally.wearing += 1.0,
            None => {}
        }
    }
    let now_tally = days.entry(today).or_default();
    now_tally.peak = Some(now_tally.peak.map_or(dose, |p| p.max(dose)));

    let plants: BTreeMap<i64, PlantKind> = garden
        .iter()
        .filter_map(|g| Some((g.day, plant_kind(g.plant?))))
        .collect();
    let levels = &rates.levels;

    let week_days = today - 6..=today;
    let busiest_day = week_days
        .clone()
        .filter_map(|d| days.get(&d))
        .map(Tally::total)
        .fold(0.0, f64::max);
    let week = week_days
        .map(|d| {
            let tally = days.get(&d).cloned().unwrap_or_default();
            DiaryDay {
                label: if d == today {
                    "Today".into()
                } else {
                    weekday(d).into()
                },
                known: d >= first_day,
                size: ratio(tally.total(), busiest_day),
                shares: tally.shares(),
                peak: tally.peak.map(|p| wisp_phase(levels.phase(p))),
                plant: plants.get(&d).copied(),
            }
        })
        .collect();

    let week_tally = |from: i64| {
        let mut sum = Tally::default();
        for d in from..from + 7 {
            if let Some(t) = days.get(&d) {
                sum.add(t);
            }
        }
        sum
    };
    let shown: Vec<(i64, Tally)> = (1..=WEEKS_BEFORE)
        .map(|w| today - 6 - 7 * w)
        .filter(|&from| from + 6 >= first_day)
        .map(|from| (from, week_tally(from)))
        .collect();
    let busiest_week = shown
        .iter()
        .map(|(_, t)| t.total())
        .fold(week_tally(today - 6).total(), f64::max);
    let earlier = shown
        .into_iter()
        .map(|(from, tally)| DiaryWeek {
            label: date_range(from, from + 6),
            size: ratio(tally.total(), busiest_week),
            shares: tally.shares(),
            plants: (from..from + 7)
                .filter_map(|d| plants.get(&d).copied())
                .collect(),
        })
        .collect();

    let (night_from, night_until) = rates.night_minutes();
    HomeWisp {
        today: today_line(rates, samples, today, now, dose),
        now_minute: rates.minute_of_day(now),
        day_start_minute: rates.day_start_minute(),
        night_from,
        night_until,
        engaged: levels.engaged,
        clouded: levels.clouded,
        drained: levels.drained,
        week,
        earlier,
        sites: sites(rates, samples, today),
    }
}

/// The dose through today, a point every `STEP_MINUTES` (the last sample in
/// each step), ending with the dose now.
fn today_line(
    rates: &Rates,
    samples: &[Sample],
    today: i64,
    now: Moment,
    dose: f64,
) -> Vec<DosePoint> {
    let mut points: Vec<DosePoint> = Vec::new();
    let mut last: Option<i64> = None;
    let mut add = |at: Moment, dose: f64, points: &mut Vec<DosePoint>| {
        let absolute = at.ms.div_euclid(60_000);
        let minute = rates.minute_of_day(at);
        let gap = last.is_some_and(|l| absolute - l > GAP_MINUTES);
        let same_step = points.last().is_some_and(|p| {
            !gap && i64::from(p.minute) / STEP_MINUTES == i64::from(minute) / STEP_MINUTES
        });
        let point = DosePoint { minute, dose, gap };
        match points.last_mut() {
            Some(previous) if same_step => {
                previous.minute = minute;
                previous.dose = dose;
            }
            _ => points.push(point),
        }
        last = Some(absolute);
    };
    for sample in samples.iter().filter(|s| day_of(rates, s.at) == today) {
        add(sample.at, sample.dose, &mut points);
    }
    add(now, dose, &mut points);
    for p in &mut points {
        p.dose = (p.dose * 1000.0).round() / 1000.0;
    }
    points
}

/// Up to `SITES_EACH` restoring and wearing list entries over the last seven
/// days, most time first. Ordinary entries and unrecorded places (unlisted,
/// private) are never named.
fn sites(rates: &Rates, samples: &[Sample], today: i64) -> Vec<DiarySite> {
    let mut minutes: BTreeMap<(bool, &str), f64> = BTreeMap::new();
    for sample in samples {
        if sample.mode == Mode::Away || day_of(rates, sample.at) < today - 6 {
            continue;
        }
        if let Some(Place::Listed { entry, weight, .. }) = &sample.place
            && *weight != 0.0
        {
            *minutes.entry((*weight > 0.0, entry.as_str())).or_default() += 1.0;
        }
    }
    let most = minutes.values().copied().fold(0.0, f64::max);
    let mut out = Vec::new();
    for restoring in [true, false] {
        let mut these: Vec<(&str, f64)> = minutes
            .iter()
            .filter(|((r, _), m)| *r == restoring && **m >= SITE_MINUTES)
            .map(|((_, entry), m)| (*entry, *m))
            .collect();
        these.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        out.extend(these.into_iter().take(SITES_EACH).map(|(entry, m)| {
            let share = m / most;
            DiarySite {
                label: entry.to_owned(),
                kind: if restoring {
                    TimeKind::Restoring
                } else {
                    TimeKind::Wearing
                },
                bars: if share >= 0.5 {
                    3
                } else if share >= 0.2 {
                    2
                } else {
                    1
                },
            }
        }));
    }
    out
}

fn ratio(part: f64, whole: f64) -> f64 {
    if whole > 0.0 {
        ((part / whole) * 1000.0).round() / 1000.0
    } else {
        0.0
    }
}

fn plant_kind(plant: Plant) -> PlantKind {
    match plant {
        Plant::Moss => PlantKind::Moss,
        Plant::Fern => PlantKind::Fern,
        Plant::Flower => PlantKind::Flower,
    }
}

fn wisp_phase(phase: Phase) -> WispPhase {
    match phase {
        Phase::Rested => WispPhase::Rested,
        Phase::Engaged => WispPhase::Engaged,
        Phase::Clouded => WispPhase::Clouded,
        Phase::Drained => WispPhase::Drained,
    }
}

/// A day number (`day_of`) counts local days from 1 January 1970, a
/// Thursday.
fn weekday(day: i64) -> &'static str {
    ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][day.rem_euclid(7) as usize]
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// "7–13 Sep", or "31 Aug – 6 Sep" across a month's end.
fn date_range(from: i64, to: i64) -> String {
    let (_, m1, d1) = civil(from);
    let (_, m2, d2) = civil(to);
    let month = |m: u32| MONTHS[m as usize - 1];
    if m1 == m2 {
        format!("{d1}–{d2} {}", month(m2))
    } else {
        format!("{d1} {} – {d2} {}", month(m1), month(m2))
    }
}

/// Year, month and day of a count of days since 1970-01-01 (Howard
/// Hinnant's `civil_from_days`).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 05:00 BST on Tuesday 15 September 2026, the start of that day.
    const DAY_START_MS: i64 = 1_789_444_800_000;

    fn at(day: i64, minute: i64) -> Moment {
        Moment {
            ms: DAY_START_MS + day * 86_400_000 + minute * 60_000,
            utc_offset_s: 3600,
        }
    }

    fn sample(day: i64, minute: i64, dose: f64, mode: Mode, entry: Option<(&str, f64)>) -> Sample {
        Sample {
            at: at(day, minute),
            dose,
            day_load: 0.0,
            mode,
            place: entry.map(|(entry, weight)| Place::Listed {
                entry: entry.into(),
                weight,
                news: false,
            }),
        }
    }

    fn run(
        mode: Mode,
        entry: Option<(&str, f64)>,
        day: i64,
        from: i64,
        minutes: i64,
    ) -> Vec<Sample> {
        (from..from + minutes)
            .map(|m| sample(day, m, 0.3, mode, entry))
            .collect()
    }

    #[test]
    fn calendar_arithmetic() {
        let rates = Rates::bundled();
        let today = day_of(&rates, at(0, 0));
        assert_eq!(today, 20_711);
        assert_eq!(weekday(today), "Tue");
        assert_eq!(civil(today), (2026, 9, 15));
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(11_016), (2000, 2, 29));
        assert_eq!(date_range(today - 6, today), "9–15 Sep");
        assert_eq!(date_range(today - 20, today - 14), "26 Aug – 1 Sep");
        // Four in the morning still belongs to yesterday.
        assert_eq!(day_of(&rates, at(0, -60)), today - 1);
        assert_eq!(rates.minute_of_day(at(0, -60)), 1380);
        assert_eq!(rates.night_minutes(), (1080, 1440));
    }

    #[test]
    fn today_is_a_line_every_five_minutes_broken_where_nothing_was_recorded() {
        let rates = Rates::bundled();
        let mut samples = Vec::new();
        // Yesterday evening doesn't belong on today's line.
        samples.push(sample(-1, 900, 0.9, Mode::Draining, None));
        for m in 0..12 {
            samples.push(sample(
                0,
                60 + m,
                0.1 + m as f64 * 0.01,
                Mode::Holding,
                None,
            ));
        }
        // Closed for an hour, then back.
        samples.push(sample(0, 132, 0.05, Mode::Holding, None));
        let wisp = build(&rates, &samples, &[], at(0, 133), 0.04);
        let line: Vec<(u32, f64, bool)> = wisp
            .today
            .iter()
            .map(|p| (p.minute, p.dose, p.gap))
            .collect();
        assert_eq!(
            line,
            [
                (64, 0.14, false),
                (69, 0.19, false),
                (71, 0.21, false),
                (133, 0.04, true),
            ]
        );
        assert_eq!(wisp.now_minute, 133);
    }

    #[test]
    fn the_week_compares_days_with_each_other_and_shows_what_grew() {
        let rates = Rates::bundled();
        let mut samples = Vec::new();
        // First recorded four days ago.
        samples.extend(run(Mode::Draining, Some(("tiktok.com", -1.0)), -4, 600, 30));
        samples.extend(run(Mode::Holding, None, -4, 700, 30));
        samples.extend(run(
            Mode::Nourishing,
            Some(("gutenberg.org", 1.0)),
            -2,
            600,
            15,
        ));
        samples.extend(run(Mode::Away, None, -1, 600, 90));
        samples.push(sample(-4, 800, 0.8, Mode::Away, None));
        samples.sort_by_key(|s| s.at.ms);
        let today = day_of(&rates, at(0, 0));
        let garden = [
            GardenDay {
                day: today - 2,
                plant: Some(Plant::Fern),
                fireflies: false,
            },
            GardenDay {
                day: today - 1,
                plant: Some(Plant::Moss),
                fireflies: true,
            },
        ];
        let wisp = build(&rates, &samples, &garden, at(0, 10), 0.0);
        let week = &wisp.week;
        assert_eq!(week.len(), 7);
        assert_eq!(
            week.iter().map(|d| d.label.as_str()).collect::<Vec<_>>(),
            ["Wed", "Thu", "Fri", "Sat", "Sun", "Mon", "Today"]
        );
        assert_eq!(
            week.iter().map(|d| d.known).collect::<Vec<_>>(),
            [false, false, true, true, true, true, true]
        );
        let busiest = &week[2];
        assert_eq!(busiest.size, 1.0);
        assert_eq!(
            busiest.shares,
            [
                TimeShare {
                    kind: TimeKind::Everyday,
                    share: 0.5
                },
                TimeShare {
                    kind: TimeKind::Wearing,
                    share: 0.5
                },
            ]
        );
        assert_eq!(busiest.peak, Some(WispPhase::Drained));
        assert_eq!(week[4].size, 0.25);
        assert_eq!(week[4].plant, Some(PlantKind::Fern));
        // A day spent away weighs nothing, and still grew moss.
        assert_eq!(week[5].size, 0.0);
        assert!(week[5].shares.is_empty());
        assert_eq!(week[5].plant, Some(PlantKind::Moss));
        // A day with nothing recorded has no weather at all.
        assert_eq!(week[3].peak, None);
        assert_eq!(week[6].peak, Some(WispPhase::Rested));
        // History doesn't reach the week before.
        assert!(wisp.earlier.is_empty());
    }

    #[test]
    fn earlier_weeks_reach_back_only_as_far_as_the_history() {
        let rates = Rates::bundled();
        let mut samples = run(Mode::Holding, None, -17, 600, 40);
        samples.extend(run(Mode::Holding, None, -3, 600, 10));
        let today = day_of(&rates, at(0, 0));
        let garden = [GardenDay {
            day: today - 16,
            plant: Some(Plant::Flower),
            fireflies: false,
        }];
        let wisp = build(&rates, &samples, &garden, at(0, 0), 0.0);
        let labels: Vec<&str> = wisp.earlier.iter().map(|w| w.label.as_str()).collect();
        assert_eq!(labels, ["2–8 Sep", "26 Aug – 1 Sep"]);
        assert_eq!(wisp.earlier[0].size, 0.0);
        assert_eq!(wisp.earlier[1].size, 1.0);
        assert_eq!(wisp.earlier[1].plants, [PlantKind::Flower]);
    }

    #[test]
    fn sites_are_named_by_list_entry_and_only_when_they_moved_it() {
        let rates = Rates::bundled();
        let mut samples = Vec::new();
        samples.extend(run(Mode::Draining, Some(("reddit.com", -0.4)), -1, 0, 60));
        samples.extend(run(Mode::Draining, Some(("tiktok.com", -1.0)), -2, 0, 20));
        samples.extend(run(Mode::Draining, Some(("x.com", -1.0)), -2, 100, 8));
        samples.extend(run(Mode::Draining, Some(("imgur.com", -0.4)), -3, 0, 7));
        samples.extend(run(
            Mode::Nourishing,
            Some(("wikipedia.org", 0.5)),
            0,
            0,
            25,
        ));
        // Too little to name, ordinary, private (never recorded by entry),
        // before the week, and a heard tab while away.
        samples.extend(run(Mode::Nourishing, Some(("nhs.uk", 1.0)), 0, 30, 4));
        samples.extend(run(Mode::Resting, Some(("gmail.com", 0.0)), 0, 40, 90));
        samples.extend(run(Mode::Holding, None, 0, 140, 90));
        samples.extend(run(Mode::Draining, Some(("twitch.tv", -0.4)), -8, 0, 90));
        samples.extend(run(Mode::Away, Some(("youtube.com", -0.4)), 0, 300, 90));
        samples.sort_by_key(|s| s.at.ms);
        let wisp = build(&rates, &samples, &[], at(0, 400), 0.2);
        let named: Vec<(&str, TimeKind, u32)> = wisp
            .sites
            .iter()
            .map(|s| (s.label.as_str(), s.kind, s.bars))
            .collect();
        assert_eq!(
            named,
            [
                ("wikipedia.org", TimeKind::Restoring, 2),
                ("reddit.com", TimeKind::Wearing, 3),
                ("tiktok.com", TimeKind::Wearing, 2),
                ("x.com", TimeKind::Wearing, 1),
            ]
        );
    }

    #[test]
    fn nothing_recorded_yet_is_just_now() {
        let rates = Rates::bundled();
        let wisp = build(&rates, &[], &[], at(0, 90), 0.0);
        assert_eq!(wisp.today.len(), 1);
        assert!(wisp.week[..6].iter().all(|d| !d.known));
        assert!(wisp.week[6].known && wisp.week[6].shares.is_empty());
        assert!(wisp.earlier.is_empty() && wisp.sites.is_empty());
    }
}

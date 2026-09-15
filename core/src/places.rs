//! Good places: which sites Home offers, and when.
//!
//! Pure. The pool lives in `core/data/places.toml`. Home offers the user's
//! own nourishing places first, then suggestions picked afresh for each
//! stretch of the day: fitting the part of the day, the season and where the
//! user is, leaning toward stillness and nature when the wisp is heavy, never
//! repeating what was offered in the last day or so, and mixing kinds. Within
//! a stretch the picks stay put, so Home is never a slot machine to reload.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;

use crate::dose::{self, Phase};
use crate::home::PartOfDay;
use crate::reputation::Lists;

/// How many good places Home offers, and how many of them can be the user's
/// own.
pub const PLACES: usize = 6;
pub const OWN_PLACES: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Nature,
    Sky,
    Awe,
    Stillness,
    Reading,
    Poetry,
    Learning,
    Art,
    Making,
    Music,
    Play,
    Connection,
    Giving,
    Movement,
    Kindness,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Region {
    Uk,
    Intl,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Suggestion {
    /// The reputation-list entry the page falls under.
    pub entry: String,
    pub url: String,
    pub name: String,
    pub line: String,
    /// What kind of good it does, the main one first.
    pub kinds: Vec<Kind>,
    pub moments: Vec<PartOfDay>,
    /// Months it suits (1–12); empty for all year.
    #[serde(default)]
    pub seasons: Vec<u32>,
    /// Its content renews every day or week, so it can come round sooner.
    #[serde(default)]
    pub fresh: bool,
    pub region: Region,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pool {
    /// Hours in each stretch of the day between new picks.
    pub rotate_hours: u32,
    pub places: Vec<Suggestion>,
}

impl Pool {
    pub fn bundled() -> Pool {
        toml::from_str(include_str!("../data/places.toml")).expect("core/data/places.toml is valid")
    }

    /// The stretch of the day `local_s` (seconds since the epoch, local
    /// time) falls in. Picks change when it does.
    pub fn stretch(&self, local_s: i64) -> i64 {
        local_s.div_euclid(i64::from(self.rotate_hours.max(1)) * 3600)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlaceCard {
    pub entry: String,
    pub name: String,
    pub line: String,
    pub url: String,
    /// One the user already returns to, rather than a suggestion.
    pub yours: bool,
}

/// What the picks depend on.
#[derive(Clone, Copy, Debug)]
pub struct Moment<'a> {
    pub part: PartOfDay,
    /// Local calendar month, 1–12.
    pub month: u32,
    pub stretch: i64,
    pub phase: Phase,
    /// The two-letter country from the user's locale (`GB`), if known.
    /// Places for one country are only offered there, or when it's unknown.
    pub country: Option<&'a str>,
    /// Fixed for this install, so every install rotates differently.
    pub seed: u32,
}

/// Up to `PLACES` good places: the user's own nourishing places first (list
/// entries with `own_minutes` or more this month, most time first), then
/// suggestions. `offered` holds the stretch in which each suggestion was last
/// offered. Nothing that isn't on a nourishing list.
pub fn pick(
    pool: &Pool,
    lists: &Lists,
    at: Moment,
    own: &[(String, f64)],
    own_minutes: f64,
    offered: &HashMap<String, i64>,
) -> Vec<PlaceCard> {
    let nourishing = |url: &str| {
        matches!(
            lists.place(url),
            dose::Place::Listed { weight, .. } if weight > 0.0
        )
    };
    let mut cards: Vec<PlaceCard> = Vec::new();
    let mut shown: HashSet<String> = HashSet::new();

    let mut own: Vec<&(String, f64)> = own
        .iter()
        .filter(|(entry, minutes)| {
            *minutes >= own_minutes && nourishing(&format!("https://{entry}"))
        })
        .collect();
    own.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    for (entry, _) in own.into_iter().take(OWN_PLACES) {
        let known = pool.places.iter().find(|s| s.entry == *entry);
        cards.push(PlaceCard {
            entry: entry.clone(),
            name: known.map_or_else(|| entry.clone(), |s| s.name.clone()),
            line: known.map_or_else(|| "One of your good places.".into(), |s| s.line.clone()),
            url: known.map_or_else(|| format!("https://{entry}"), |s| s.url.clone()),
            yours: true,
        });
        shown.insert(entry.clone());
    }

    let mut scored: Vec<(f64, &Suggestion)> = pool
        .places
        .iter()
        .filter(|s| {
            s.moments.contains(&at.part)
                && (s.seasons.is_empty() || s.seasons.contains(&at.month))
                && fits_region(s.region, at.country)
                && !shown.contains(&s.entry)
                && nourishing(&s.url)
        })
        .map(|s| (score(s, at, offered), s))
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.url.cmp(&b.1.url)));

    // Mix kinds: one of each main kind first, then whatever scores best.
    let mut kinds: HashSet<Kind> = HashSet::new();
    let mut picked: Vec<&Suggestion> = Vec::new();
    for pass in 0..2 {
        for (_, s) in &scored {
            if cards.len() + picked.len() >= PLACES {
                break;
            }
            if shown.contains(&s.entry) {
                continue;
            }
            let main = s.kinds.first().copied();
            if pass == 0 && main.is_some_and(|k| kinds.contains(&k)) {
                continue;
            }
            kinds.extend(main);
            shown.insert(s.entry.clone());
            picked.push(s);
        }
    }
    cards.extend(picked.into_iter().map(|s| PlaceCard {
        entry: s.entry.clone(),
        name: s.name.clone(),
        line: s.line.clone(),
        url: s.url.clone(),
        yours: false,
    }));
    cards
}

fn fits_region(region: Region, country: Option<&str>) -> bool {
    match (region, country) {
        (Region::Intl, _) | (Region::Uk, None) => true,
        (Region::Uk, Some(country)) => matches!(country, "GB" | "UK" | "IM" | "JE" | "GG"),
    }
}

/// Higher is offered first: a steady shuffle for the stretch, less for
/// anything offered lately, more for what suits how the wisp is.
fn score(s: &Suggestion, at: Moment, offered: &HashMap<String, i64>) -> f64 {
    let shuffle = unit(at.seed, at.stretch, &s.entry);
    let recent = match offered.get(&s.entry).map(|&stretch| at.stretch - stretch) {
        // Offered in this stretch: keep it, so Home doesn't change under the user.
        Some(0) => 1.0,
        // Something new every day or week can come round sooner.
        Some(ago)
            if ago > 0
                && ago
                    <= if s.fresh {
                        RECENT_STRETCHES / 2
                    } else {
                        RECENT_STRETCHES
                    } =>
        {
            -1.0
        }
        Some(ago) if ago > 0 && ago <= WEEK_STRETCHES && !s.fresh => -0.3,
        _ => 0.0,
    };
    let suits = |wanted: &[Kind]| s.kinds.iter().any(|k| wanted.contains(k));
    let fit = match (at.phase, at.part) {
        (_, PartOfDay::Night)
            if suits(&[Kind::Stillness, Kind::Poetry, Kind::Reading, Kind::Sky]) =>
        {
            0.4
        }
        (Phase::Clouded | Phase::Drained, _)
            if suits(&[
                Kind::Nature,
                Kind::Stillness,
                Kind::Movement,
                Kind::Kindness,
                Kind::Music,
            ]) =>
        {
            0.6
        }
        (Phase::Rested | Phase::Engaged, PartOfDay::Morning | PartOfDay::Day)
            if suits(&[
                Kind::Learning,
                Kind::Making,
                Kind::Play,
                Kind::Art,
                Kind::Giving,
            ]) =>
        {
            0.3
        }
        _ => 0.0,
    };
    shuffle + recent + fit + if s.fresh { 0.1 } else { 0.0 }
}

/// Stretches of about a day, and of about a week, at two hours each; kept
/// in stretches so a different `rotate_hours` scales them roughly.
const RECENT_STRETCHES: i64 = 12;
const WEEK_STRETCHES: i64 = 84;

/// A number in [0, 1) that is the same for the same seed, stretch and entry.
fn unit(seed: u32, stretch: i64, entry: &str) -> f64 {
    // FNV-1a over the entry, then SplitMix64 over everything.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in entry.bytes() {
        h ^= u64::from(byte);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let mut x = h ^ (u64::from(seed) << 32) ^ (stretch as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

/// The local calendar month (1–12) of a local time in seconds since the epoch.
pub fn month_of(local_s: i64) -> u32 {
    // Days to civil date, after Howard Hinnant's algorithm.
    let z = local_s.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    (if mp < 10 { mp + 3 } else { mp - 9 }) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(part: PartOfDay, stretch: i64, phase: Phase) -> Moment<'static> {
        Moment {
            part,
            month: 9,
            stretch,
            phase,
            country: Some("GB"),
            seed: 7,
        }
    }

    fn all_parts() -> [PartOfDay; 4] {
        [
            PartOfDay::Morning,
            PartOfDay::Day,
            PartOfDay::Evening,
            PartOfDay::Night,
        ]
    }

    #[test]
    fn every_place_is_on_a_nourishing_list_and_well_formed() {
        let lists = Lists::bundled();
        let pool = Pool::bundled();
        let mut urls = HashSet::new();
        for s in &pool.places {
            match lists.lookup(&s.url) {
                Some((_, list)) => assert!(
                    lists.weight(list) > 0.0,
                    "{} ({}) isn't on a nourishing list",
                    s.url,
                    s.entry
                ),
                None => panic!("{} is on no list", s.url),
            }
            assert_eq!(
                lists.lookup(&s.url).map(|(e, _)| e),
                Some(s.entry.as_str()),
                "{} doesn't fall under {}",
                s.url,
                s.entry
            );
            assert!(s.url.starts_with("https://"), "{}", s.url);
            assert!(urls.insert(s.url.clone()), "{} is in the pool twice", s.url);
            assert!(!s.kinds.is_empty() && !s.moments.is_empty(), "{}", s.url);
            assert!(s.seasons.iter().all(|m| (1..=12).contains(m)), "{}", s.url);
            assert!(
                !s.name.is_empty() && s.line.chars().count() <= 48,
                "{}: {:?}",
                s.url,
                s.line
            );
            assert!(!s.line.contains('!'), "{}", s.url);
        }
    }

    #[test]
    fn there_is_plenty_to_offer_at_every_part_of_the_day_everywhere() {
        let pool = Pool::bundled();
        for part in all_parts() {
            for month in 1..=12 {
                for country in [Some("GB"), Some("US"), None] {
                    let count = pool
                        .places
                        .iter()
                        .filter(|s| {
                            s.moments.contains(&part)
                                && (s.seasons.is_empty() || s.seasons.contains(&month))
                                && fits_region(s.region, country)
                        })
                        .count();
                    assert!(
                        count >= 4 * PLACES,
                        "only {count} places for {part:?} in month {month} ({country:?})"
                    );
                }
            }
        }
    }

    #[test]
    fn the_users_own_places_come_first_and_nothing_draining_is_offered() {
        let pool = Pool::bundled();
        let lists = Lists::bundled();
        let own = vec![
            ("khanacademy.org".to_string(), 90.0),
            ("tiktok.com".to_string(), 500.0),
            ("rhs.org.uk".to_string(), 5.0),
        ];
        let cards = pick(
            &pool,
            &lists,
            at(PartOfDay::Evening, 100, Phase::Engaged),
            &own,
            20.0,
            &HashMap::new(),
        );
        assert_eq!(cards.len(), PLACES);
        assert_eq!(cards[0].entry, "khanacademy.org");
        assert!(cards[0].yours && cards[1..].iter().all(|c| !c.yours));
        assert!(!cards.iter().any(|c| c.url.contains("tiktok")));
        let entries: HashSet<&str> = cards.iter().map(|c| c.entry.as_str()).collect();
        assert_eq!(entries.len(), cards.len());
    }

    #[test]
    fn picks_hold_within_a_stretch_and_change_between_them() {
        let pool = Pool::bundled();
        let lists = Lists::bundled();
        let now = at(PartOfDay::Day, 200, Phase::Engaged);
        let first = pick(&pool, &lists, now, &[], 20.0, &HashMap::new());
        let offered: HashMap<String, i64> = first.iter().map(|c| (c.entry.clone(), 200)).collect();
        // Seen again in the same stretch, even after being recorded: the same.
        assert_eq!(pick(&pool, &lists, now, &[], 20.0, &offered), first);
        // The next stretch offers none of them again.
        let next = pick(
            &pool,
            &lists,
            at(PartOfDay::Day, 201, Phase::Engaged),
            &[],
            20.0,
            &offered,
        );
        assert_eq!(next.len(), PLACES);
        assert!(
            next.iter().all(|c| !offered.contains_key(&c.entry)),
            "{next:?}"
        );
    }

    #[test]
    fn a_day_of_stretches_offers_many_different_places_and_mixes_kinds() {
        let pool = Pool::bundled();
        let lists = Lists::bundled();
        let mut offered = HashMap::new();
        let mut seen = HashSet::new();
        for stretch in 300..312 {
            let part = all_parts()[(stretch % 4) as usize];
            let cards = pick(
                &pool,
                &lists,
                at(part, stretch, Phase::Engaged),
                &[],
                20.0,
                &offered,
            );
            let kinds: HashSet<Kind> = cards
                .iter()
                .filter_map(|c| {
                    pool.places
                        .iter()
                        .find(|s| s.url == c.url)?
                        .kinds
                        .first()
                        .copied()
                })
                .collect();
            assert!(kinds.len() >= 4, "too alike at {stretch}: {cards:?}");
            for card in cards {
                offered.insert(card.entry.clone(), stretch);
                seen.insert(card.entry);
            }
        }
        assert!(
            seen.len() >= 60,
            "only {} different places in a day",
            seen.len()
        );
    }

    #[test]
    fn a_heavy_wisp_is_offered_stillness_and_nature() {
        let pool = Pool::bundled();
        let lists = Lists::bundled();
        let calming = [
            Kind::Nature,
            Kind::Stillness,
            Kind::Movement,
            Kind::Kindness,
            Kind::Music,
        ];
        let calm_share = |phase| {
            let mut calm = 0;
            for stretch in 0..40 {
                for card in pick(
                    &pool,
                    &lists,
                    at(PartOfDay::Day, stretch, phase),
                    &[],
                    20.0,
                    &HashMap::new(),
                ) {
                    let s = pool
                        .places
                        .iter()
                        .find(|s| s.url == card.url)
                        .expect("from the pool");
                    calm += usize::from(s.kinds.iter().any(|k| calming.contains(k)));
                }
            }
            calm
        };
        assert!(calm_share(Phase::Drained) > calm_share(Phase::Rested));
    }

    #[test]
    fn places_for_one_country_stay_there() {
        let pool = Pool::bundled();
        let lists = Lists::bundled();
        let us = Moment {
            country: Some("US"),
            ..at(PartOfDay::Day, 5, Phase::Engaged)
        };
        for stretch in 0..30 {
            for card in pick(
                &pool,
                &lists,
                Moment { stretch, ..us },
                &[],
                20.0,
                &HashMap::new(),
            ) {
                let s = pool
                    .places
                    .iter()
                    .find(|s| s.url == card.url)
                    .expect("from the pool");
                assert_eq!(s.region, Region::Intl, "{}", s.url);
            }
        }
    }

    #[test]
    fn months_come_from_local_time() {
        // 2026-09-15 12:00 and 2026-01-01 00:00, 2024-02-29 in local seconds.
        assert_eq!(month_of(1_789_473_600), 9);
        assert_eq!(month_of(1_767_225_600), 1);
        assert_eq!(month_of(1_709_164_800), 2);
        assert_eq!(month_of(1_767_225_599), 12);
    }
}

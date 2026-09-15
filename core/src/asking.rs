//! When the wisp asks how a site it hasn't met leaves the user.
//!
//! Pure: it only learns the time from the moments it is given. The companion
//! tells it, at every refresh, which unrated site is on screen while the user
//! is present, and it says whether a question is open and about which site.
//!
//! Which sites were asked about, and for how long each was visited, is kept
//! in memory only: sites on no list are never written down.

use std::collections::{HashMap, HashSet};

use crate::dose::Moment;

/// Time present on a site, over the day, before the wisp asks about it.
pub const ASK_AFTER_MS: i64 = 3 * 60_000;
/// Never within this long of a key press.
pub const QUIET_AFTER_TYPING_MS: i64 = 10_000;
/// Questions a day, at most.
pub const PER_DAY: u32 = 3;

/// What the companion sees at a refresh.
#[derive(Clone, Copy, Debug)]
pub struct Seen<'a> {
    pub now: Moment,
    /// The day `now` falls in (days start at the day boundary, not midnight).
    pub day: i64,
    /// The rateable site on screen while the user is present: on no list,
    /// not a carve-out, not private. `None` when away or anywhere else.
    pub site: Option<&'a str>,
    /// The setting is on and it isn't night.
    pub allowed: bool,
    pub last_typed: Option<Moment>,
}

#[derive(Debug, Default)]
pub struct Asker {
    day: i64,
    asked_today: u32,
    asked: HashSet<String>,
    time_on: HashMap<String, i64>,
    /// The site being visited at the last refresh, and when that was.
    visiting: Option<(String, Moment)>,
    open: Option<String>,
    /// When a question can next open, as of the last refresh.
    due: Option<Moment>,
}

impl Asker {
    /// Take in a refresh. Returns whether the open question changed.
    pub fn observe(&mut self, seen: Seen) -> bool {
        let before = self.open.clone();
        if seen.day != self.day {
            self.day = seen.day;
            self.asked_today = 0;
            self.asked.clear();
            self.time_on.clear();
            self.visiting = None;
        }
        if let Some((site, since)) = self.visiting.take() {
            *self.time_on.entry(site).or_default() += (seen.now.ms - since.ms).max(0);
        }
        self.visiting = seen.site.map(|site| (site.to_owned(), seen.now));

        if !seen.allowed || self.open.as_deref() != seen.site {
            self.open = None;
        }
        self.due = None;
        if let Some(site) = seen.site
            && self.open.is_none()
            && seen.allowed
            && self.asked_today < PER_DAY
            && !self.asked.contains(site)
        {
            let visited = self.time_on.get(site).copied().unwrap_or(0);
            let quiet_from = seen
                .last_typed
                .map_or(i64::MIN, |t| t.ms + QUIET_AFTER_TYPING_MS);
            let due = (seen.now.ms - visited + ASK_AFTER_MS).max(quiet_from);
            if due <= seen.now.ms {
                self.asked.insert(site.to_owned());
                self.asked_today += 1;
                self.open = Some(site.to_owned());
            } else {
                self.due = Some(Moment {
                    ms: due,
                    utc_offset_s: seen.now.utc_offset_s,
                });
            }
        }
        self.open != before
    }

    /// The site the open question is about.
    pub fn question(&self) -> Option<&str> {
        self.open.as_deref()
    }

    /// When a question could open if nothing else changes, so the companion
    /// can wake then.
    pub fn due(&self) -> Option<Moment> {
        self.due
    }

    /// The user answered, said not now, or let the question fade. Returns
    /// whether it was the open question.
    pub fn close(&mut self, site: &str) -> bool {
        if self.open.as_deref() == Some(site) {
            self.open = None;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-09-15 12:00 BST plus `minutes`, with `seconds` more.
    fn at(minutes: i64, seconds: i64) -> Moment {
        Moment {
            ms: 1_789_470_000_000 + (minutes * 60 + seconds) * 1000,
            utc_offset_s: 3600,
        }
    }

    fn seen(now: Moment, site: Option<&str>) -> Seen<'_> {
        Seen {
            now,
            day: 1,
            site,
            allowed: true,
            last_typed: None,
        }
    }

    #[test]
    fn it_asks_after_three_minutes_on_a_site_and_only_once_that_day() {
        let mut asker = Asker::default();
        asker.observe(seen(at(0, 0), Some("moss.example")));
        assert!(!asker.observe(seen(at(2, 0), Some("moss.example"))));
        assert_eq!(asker.due(), Some(at(3, 0)));
        assert!(asker.observe(seen(at(3, 0), Some("moss.example"))));
        assert_eq!(asker.question(), Some("moss.example"));
        assert!(asker.close("moss.example"));
        asker.observe(seen(at(30, 0), Some("moss.example")));
        assert_eq!(asker.question(), None);
        assert_eq!(asker.due(), None);
    }

    #[test]
    fn time_on_a_site_adds_up_across_visits_but_not_time_elsewhere() {
        let mut asker = Asker::default();
        asker.observe(seen(at(0, 0), Some("moss.example")));
        asker.observe(seen(at(2, 0), None));
        asker.observe(seen(at(50, 0), Some("fern.example")));
        asker.observe(seen(at(51, 0), Some("moss.example")));
        assert_eq!(asker.due(), Some(at(52, 0)));
        assert!(asker.observe(seen(at(52, 0), Some("moss.example"))));
    }

    #[test]
    fn it_waits_for_a_pause_in_typing() {
        let mut asker = Asker::default();
        asker.observe(seen(at(0, 0), Some("moss.example")));
        let typing = Seen {
            last_typed: Some(at(2, 58)),
            ..seen(at(3, 0), Some("moss.example"))
        };
        assert!(!asker.observe(typing));
        assert_eq!(asker.due(), Some(at(3, 8)));
        let paused = Seen {
            last_typed: Some(at(2, 58)),
            ..seen(at(3, 8), Some("moss.example"))
        };
        assert!(asker.observe(paused));
    }

    #[test]
    fn leaving_the_site_or_the_setting_going_off_closes_the_question() {
        let mut asker = Asker::default();
        asker.observe(seen(at(0, 0), Some("moss.example")));
        asker.observe(seen(at(3, 0), Some("moss.example")));
        assert!(asker.observe(seen(at(4, 0), Some("fern.example"))));
        assert_eq!(asker.question(), None);

        asker.observe(seen(at(10, 0), Some("fern.example")));
        assert_eq!(asker.question(), Some("fern.example"));
        let night = Seen {
            allowed: false,
            ..seen(at(11, 0), Some("fern.example"))
        };
        assert!(asker.observe(night));
        assert_eq!(asker.question(), None);
    }

    #[test]
    fn never_when_not_allowed_and_never_more_than_three_a_day() {
        let mut asker = Asker::default();
        let off = |now, site| Seen {
            allowed: false,
            ..seen(now, site)
        };
        asker.observe(off(at(0, 0), Some("a.example")));
        asker.observe(off(at(10, 0), Some("a.example")));
        assert_eq!(asker.question(), None);
        assert_eq!(asker.due(), None);

        for (i, site) in ["b.example", "c.example", "d.example", "e.example"]
            .into_iter()
            .enumerate()
        {
            let start = 20 + 10 * i as i64;
            asker.observe(seen(at(start, 0), Some(site)));
            asker.observe(seen(at(start + 3, 0), Some(site)));
            assert_eq!(asker.question().is_some(), i < 3, "{site}");
            asker.close(site);
        }

        // A new day starts afresh.
        let tomorrow = |now, site| Seen {
            day: 2,
            ..seen(now, site)
        };
        asker.observe(tomorrow(at(1440, 0), Some("e.example")));
        assert!(asker.observe(tomorrow(at(1443, 0), Some("e.example"))));
    }
}

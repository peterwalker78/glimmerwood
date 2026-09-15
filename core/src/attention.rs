//! Where the user's attention is: the rules for presence and tab clutter,
//! and the app's only view of the clock.
//!
//! The rules are pure functions of what the window reports; the GTK side lives
//! in `companion.rs`.

use crate::dose::{Moment, Rates};

/// While sound plays with no input, presence is leased this far ahead (or
/// until sound stops counting) and renewed well before it runs out.
pub const SOUND_LEASE_MS: i64 = 60_000;

/// The one place Glimmerwood reads the wall clock. Everything else is handed the
/// moment it concerns.
#[allow(clippy::disallowed_methods)]
pub fn now() -> Moment {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    let utc_offset_s = gtk::glib::DateTime::now_local()
        .map(|t| (t.utc_offset().as_seconds()) as i32)
        .unwrap_or(0);
    Moment { ms, utc_offset_s }
}

/// What a window can tell us about the user.
#[derive(Clone, Copy, Debug, Default)]
pub struct Signals {
    /// The window is the active one and not hidden or minimised.
    pub in_front: bool,
    pub last_input: Option<Moment>,
    /// The visible tab is playing sound (not muted). Silent video doesn't
    /// count.
    pub sound_on_screen: bool,
}

/// Until when the user counts as present, or `None` if they're away.
pub fn presence_until(rates: &Rates, signals: Signals, now: Moment) -> Option<Moment> {
    if !signals.in_front {
        return None;
    }
    let by_input = signals
        .last_input
        .map(|t| t.ms + i64::from(rates.presence.input_seconds) * 1000)
        .filter(|&until| until > now.ms);
    let by_sound = signals
        .sound_on_screen
        .then(|| sound_until(rates, signals.last_input, now))
        .flatten()
        .map(|until| until.min(now.ms + SOUND_LEASE_MS));
    by_input.max(by_sound).map(|ms| Moment {
        ms,
        utc_offset_s: now.utc_offset_s,
    })
}

/// Until when a heard draining site keeps counting with nobody present: the
/// sound window after the last input in Glimmerwood, or `None` once it has
/// passed.
pub fn listening_until(rates: &Rates, last_input: Option<Moment>, now: Moment) -> Option<Moment> {
    sound_until(rates, last_input, now).map(|ms| Moment {
        ms,
        utc_offset_s: now.utc_offset_s,
    })
}

/// Until when sound still counts without input, or `None` once it doesn't.
/// The window is shorter at night, so falling asleep to something is rest.
fn sound_until(rates: &Rates, last_input: Option<Moment>, now: Moment) -> Option<i64> {
    let p = &rates.presence;
    let minutes = if rates.is_night(now) {
        p.night_sound_minutes
    } else {
        p.sound_minutes
    };
    let until = last_input?.ms + (minutes * 60_000.0) as i64;
    (until > now.ms).then_some(until)
}

/// The tab heard right now: of the tabs playing sound that aren't the one on
/// screen, the one that started most recently. Items are (started, tab).
pub fn heard<T>(playing: impl IntoIterator<Item = (Moment, T)>) -> Option<T> {
    playing
        .into_iter()
        .max_by_key(|(started, _)| started.ms)
        .map(|(_, tab)| tab)
}

/// How many tabs count as clutter: not looked at for longer than the rates
/// allow. `last_seen` is when each tab was last visible; a tab visible right
/// now should be given `now`.
pub fn untouched_tabs(rates: &Rates, last_seen: &[Moment], now: Moment) -> u32 {
    let limit = untouched_ms(rates);
    last_seen.iter().filter(|t| now.ms - t.ms > limit).count() as u32
}

/// When the next tab will become clutter, if any will.
pub fn next_untouched(rates: &Rates, last_seen: &[Moment], now: Moment) -> Option<Moment> {
    let limit = untouched_ms(rates);
    last_seen
        .iter()
        .map(|t| t.ms + limit + 1)
        .filter(|&ms| ms > now.ms)
        .min()
        .map(|ms| Moment {
            ms,
            utc_offset_s: now.utc_offset_s,
        })
}

fn untouched_ms(rates: &Rates) -> i64 {
    (rates.clutter.untouched_hours * 3_600_000.0) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(minutes: i64) -> Moment {
        Moment {
            ms: 1_789_426_800_000 + minutes * 60_000,
            utc_offset_s: 3600,
        }
    }

    /// Minutes after noon and after midnight, local time.
    fn noon(minutes: i64) -> Moment {
        local(12 * 60, minutes)
    }

    fn midnight(minutes: i64) -> Moment {
        local(0, minutes)
    }

    fn local(at_minute: i64, minutes: i64) -> Moment {
        let start = t(0);
        let into_day = (start.ms / 1000 + i64::from(start.utc_offset_s)).rem_euclid(86_400) / 60;
        Moment {
            ms: start.ms + (at_minute - into_day + minutes) * 60_000,
            utc_offset_s: start.utc_offset_s,
        }
    }

    #[test]
    fn present_means_in_front_with_recent_input() {
        let rates = Rates::bundled();
        let signals = Signals {
            in_front: true,
            last_input: Some(t(0)),
            sound_on_screen: false,
        };
        assert_eq!(presence_until(&rates, signals, t(1)), Some(t(4)));
        assert_eq!(presence_until(&rates, signals, t(4)), None);
        let behind = Signals {
            in_front: false,
            ..signals
        };
        assert_eq!(presence_until(&rates, behind, t(1)), None);
    }

    #[test]
    fn sound_on_screen_keeps_you_present_for_an_hour_without_input() {
        let rates = Rates::bundled();
        let watching = Signals {
            in_front: true,
            last_input: Some(noon(0)),
            sound_on_screen: true,
        };
        assert_eq!(presence_until(&rates, watching, noon(30)), Some(noon(31)));
        assert_eq!(presence_until(&rates, watching, noon(59)), Some(noon(60)));
        assert_eq!(presence_until(&rates, watching, noon(60)), None);
        // ...but not with the window behind another.
        let behind = Signals {
            in_front: false,
            ..watching
        };
        assert_eq!(presence_until(&rates, behind, noon(30)), None);
        // ...and not without any input at all.
        let untouched = Signals {
            last_input: None,
            ..watching
        };
        assert_eq!(presence_until(&rates, untouched, noon(1)), None);
    }

    #[test]
    fn falling_asleep_to_sound_at_night_is_being_away() {
        let rates = Rates::bundled();
        let in_bed = Signals {
            in_front: true,
            last_input: Some(midnight(0)),
            sound_on_screen: true,
        };
        assert_eq!(
            presence_until(&rates, in_bed, midnight(10)),
            Some(midnight(11))
        );
        assert_eq!(presence_until(&rates, in_bed, midnight(20)), None);
        assert_eq!(
            listening_until(&rates, Some(midnight(0)), midnight(19)),
            Some(midnight(20))
        );
        assert_eq!(
            listening_until(&rates, Some(midnight(0)), midnight(20)),
            None
        );
    }

    #[test]
    fn heard_sound_counts_for_an_hour_after_the_last_input_by_day() {
        let rates = Rates::bundled();
        assert_eq!(
            listening_until(&rates, Some(noon(0)), noon(10)),
            Some(noon(60))
        );
        assert_eq!(listening_until(&rates, Some(noon(0)), noon(60)), None);
        assert_eq!(listening_until(&rates, None, noon(1)), None);
    }

    #[test]
    fn the_most_recently_started_sound_is_the_one_heard() {
        assert_eq!(
            heard([(t(5), "music"), (t(9), "news"), (t(2), "old")]),
            Some("news")
        );
        assert_eq!(heard(Vec::<(Moment, &str)>::new()), None);
    }

    #[test]
    fn tabs_become_clutter_after_a_day_untouched() {
        let rates = Rates::bundled();
        let seen = [t(0), t(60), t(24 * 60 + 30)];
        assert_eq!(untouched_tabs(&rates, &seen, t(24 * 60)), 0);
        assert_eq!(untouched_tabs(&rates, &seen, t(24 * 60 + 1)), 1);
        assert_eq!(untouched_tabs(&rates, &seen, t(25 * 60 + 1)), 2);
        assert_eq!(
            next_untouched(&rates, &seen, t(24 * 60 + 1)).map(|m| m.ms),
            Some(t(25 * 60).ms + 1)
        );
    }
}

//! Where the user's attention is: the rule for presence, and the app's only
//! view of the clock.
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
    // Sound keeps the user present only so long after they last touched
    // anything: past that they've stepped away or fallen asleep to it.
    let by_sound = signals
        .last_input
        .filter(|_| signals.sound_on_screen)
        .map(|t| t.ms + (rates.presence.sound_minutes * 60_000.0) as i64)
        .filter(|&until| until > now.ms)
        .map(|until| until.min(now.ms + SOUND_LEASE_MS));
    by_input.max(by_sound).map(|ms| Moment {
        ms,
        utc_offset_s: now.utc_offset_s,
    })
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
    fn sound_on_screen_keeps_you_present_for_half_an_hour_without_input() {
        let rates = Rates::bundled();
        let watching = Signals {
            in_front: true,
            last_input: Some(t(0)),
            sound_on_screen: true,
        };
        assert_eq!(presence_until(&rates, watching, t(10)), Some(t(11)));
        assert_eq!(presence_until(&rates, watching, t(29)), Some(t(30)));
        // Past that, falling asleep to rain sounds is being away.
        assert_eq!(presence_until(&rates, watching, t(30)), None);
        // ...and never with the window behind another.
        let behind = Signals {
            in_front: false,
            ..watching
        };
        assert_eq!(presence_until(&rates, behind, t(10)), None);
        // ...or without any input at all.
        let untouched = Signals {
            last_input: None,
            ..watching
        };
        assert_eq!(presence_until(&rates, untouched, t(1)), None);
    }
}

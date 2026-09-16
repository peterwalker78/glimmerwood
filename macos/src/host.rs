//! What the companion is given to work with here: the window on screen,
//! macOS's idea of where files live, and a timer.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use glimmerwood_core::companion;
use glimmerwood_core::dose::Moment;
use glimmerwood_core::host;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol};
use objc2::{AllocAnyThread, define_class, msg_send, sel};
use objc2_foundation::{NSTimeZone, NSTimer};

use crate::shell::Shell;

thread_local! {
    /// The wake-up the companion asked for, so a new one replaces it.
    static WAKE: RefCell<Option<Retained<NSTimer>>> = const { RefCell::new(None) };
}

impl host::Host for Shell {
    fn windows(&self) -> Vec<Rc<dyn host::Window>> {
        match crate::shell::held() {
            Some(shell) => vec![shell as Rc<dyn host::Window>],
            None => Vec::new(),
        }
    }

    /// The one place this build reads the wall clock, which is what a host is
    /// for; everything above it is handed the moment it concerns.
    #[allow(clippy::disallowed_methods)]
    fn now(&self) -> Moment {
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_millis() as i64);
        let utc_offset_s = NSTimeZone::localTimeZone().secondsFromGMT() as i32;
        Moment { ms, utc_offset_s }
    }

    /// macOS keeps an application's own files in one place, and makes no
    /// distinction between what is settings and what is data.
    fn data_dir(&self) -> PathBuf {
        self.home_dir().join("Library/Application Support")
    }

    fn config_dir(&self) -> PathBuf {
        self.data_dir()
    }

    fn home_dir(&self) -> PathBuf {
        std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from)
    }

    fn locale_country(&self) -> Option<String> {
        let locale = std::env::var("LANG").ok()?;
        companion::country_from_locales(&[locale])
    }

    fn wake_in(&self, ms: u64) {
        WAKE.with_borrow_mut(|held| {
            if let Some(timer) = held.take() {
                timer.invalidate();
            }
            let waker: Retained<Waker> = unsafe { msg_send![Waker::alloc(), init] };
            let timer = unsafe {
                NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                    ms.max(1) as f64 / 1000.0,
                    &waker,
                    sel!(fire:),
                    None,
                    false,
                )
            };
            *held = Some(timer);
        });
    }

    #[allow(clippy::disallowed_methods)]
    fn noise(&self) -> u32 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos() as u64);
        (nanos ^ (nanos >> 32)) as u32
    }
}

define_class!(
    /// What a timer talks to when it goes off.
    #[unsafe(super(NSObject))]
    #[name = "GlimmerwoodWaker"]
    struct Waker;

    unsafe impl NSObjectProtocol for Waker {}

    impl Waker {
        #[unsafe(method(fire:))]
        fn fire(&self, _timer: &NSTimer) {
            WAKE.with_borrow_mut(|held| *held = None);
            if let Some(companion) = crate::shell::companion() {
                companion.refresh();
            }
        }
    }
);

//! What the companion needs from whatever is running it.
//!
//! The companion decides how someone's time online is going: it owns the dose
//! engine, the lists, the history and the garden. None of that is about
//! windows or widgets. But it does need to look at what is on screen, and to
//! say things back to the chrome, and it needs a clock and somewhere to keep
//! its files — and those are the platform's to provide.
//!
//! So the platform hands it a [`Host`], and the companion works the same
//! wherever it runs.

use std::path::PathBuf;
use std::rc::Rc;

use crate::dose::Moment;
use crate::protocol::ToChrome;

/// One of Glimmerwood's windows, as the companion sees it.
pub trait Window {
    /// The window is the active one and not hidden or minimised.
    fn in_front(&self) -> bool;
    /// The address of the tab the user is looking at.
    fn attended_uri(&self) -> String;
    /// The addresses of every other tab that is open. `in_front` says
    /// whether this window is the one being looked at.
    fn other_tabs(&self, in_front: bool) -> Vec<String>;
    /// The tab in front is playing sound, and is not muted.
    fn sound_on_screen(&self) -> bool;
    fn send_to_chrome(&self, message: &ToChrome);
    /// Home and Settings are showing something that has changed underneath.
    fn refresh_pages(&self);
    /// Ask about a site the wisp hasn't met, or take the question away.
    fn ask(&self, site: Option<String>);
    /// Offer someone to talk to, or take the offer away.
    fn care(&self, open: bool, samaritans: bool);
    fn set_title(&self, title: &str);
}

/// The machine the companion is running on.
pub trait Host {
    /// Every window still open, in the order they were opened.
    fn windows(&self) -> Vec<Rc<dyn Window>>;
    /// The moment it is now, including the offset from UTC.
    fn now(&self) -> Moment;
    /// Where history, bookmarks and the garden are kept.
    fn data_dir(&self) -> PathBuf;
    /// Where settings and the user's own lists are kept.
    fn config_dir(&self) -> PathBuf;
    /// The user's home, only so that paths can be shown with a `~`.
    fn home_dir(&self) -> PathBuf;
    /// The two-letter country from the user's locale, for places offered
    /// only there.
    fn locale_country(&self) -> Option<String>;
    /// Wake the companion in `ms`, replacing whatever wake was pending.
    fn wake_in(&self, ms: u64);
    /// Something unpredictable, to start a seed from.
    fn noise(&self) -> u32;
}

//! What the tab shows when a page can't be opened.
//!
//! WebKit's default is the raw error string, which can mislead: a server that
//! never answers is reported as "Operation was cancelled". The words and the
//! page itself are the core's, shared with the other shells; what is GTK's is
//! reading the engine's error.

use gtk::{gio, glib};

pub use glimmerwood_core::failure::Reason;

const TEMPLATE: &str = "/io/github/peterwalker78/Glimmerwood/pages/failed.html";

/// Which reason a GLib error is.
pub fn reason_of(error: &glib::Error) -> Reason {
    use gio::IOErrorEnum as Io;
    match error.kind::<Io>() {
        // The network layer reports its own timeouts as cancellation. A
        // user's Stop arrives as WebKit's NetworkError::Cancelled instead,
        // and never gets this page.
        Some(Io::Cancelled | Io::TimedOut) => Reason::NoAnswer,
        // PartialInput is libsoup's "Connection terminated unexpectedly".
        Some(Io::ConnectionRefused | Io::PartialInput | Io::BrokenPipe) => Reason::Refused,
        Some(Io::HostNotFound) => Reason::NotFound,
        Some(Io::NetworkUnreachable | Io::HostUnreachable) => Reason::Offline,
        _ if error.kind::<gio::ResolverError>().is_some() => Reason::NotFound,
        _ => Reason::Other(error.message().to_owned()),
    }
}

/// The page for `uri`, built from the bundled template.
pub fn page(uri: &str, reason: &Reason) -> String {
    let template = gio::resources_lookup_data(TEMPLATE, gio::ResourceLookupFlags::NONE)
        .expect("the failure page is bundled");
    glimmerwood_core::failure::fill(
        std::str::from_utf8(&template).expect("the failure page is UTF-8"),
        uri,
        reason,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_timeouts_read_as_no_answer_not_cancelled() {
        let error = glib::Error::new(gio::IOErrorEnum::Cancelled, "Operation was cancelled");
        assert_eq!(reason_of(&error), Reason::NoAnswer);
    }

    #[test]
    fn other_errors_keep_the_engines_words() {
        let error = glib::Error::new(gio::IOErrorEnum::NotSupported, "Unsupported thing");
        assert_eq!(reason_of(&error), Reason::Other("Unsupported thing".into()));
    }
}

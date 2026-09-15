//! What the tab shows when a page can't be opened.
//!
//! WebKit's default is the raw error string, which can mislead: a server that
//! never answers is reported as "Operation was cancelled". This page says what
//! happened in plain words, quietly, with a way to try again.
//!
//! Pure: the caller classifies the error.

use gtk::{gio, glib};

const TEMPLATE: &str = "/io/github/peterwalker78/Glimmerwood/pages/failed.html";

#[derive(Debug, PartialEq, Eq)]
pub enum Reason {
    /// The server accepted nothing or said nothing before the network gave up.
    NoAnswer,
    /// The connection was refused or dropped.
    Refused,
    /// The name doesn't resolve.
    NotFound,
    /// This machine has no route to the network.
    Offline,
    /// Anything else, with the engine's own words.
    Other(String),
}

impl Reason {
    pub fn of(error: &glib::Error) -> Reason {
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

    fn words(&self, host: &str) -> (String, String) {
        match self {
            Reason::NoAnswer => (
                format!("{host} didn’t answer"),
                "It may be busy, or not reachable from this network.".into(),
            ),
            Reason::Refused => (
                format!("{host} closed the connection"),
                "The site may be down for now.".into(),
            ),
            Reason::NotFound => (
                format!("Couldn’t find {host}"),
                "The address may be mistyped, or the name no longer in use.".into(),
            ),
            Reason::Offline => (
                "Not connected".into(),
                "Glimmerwood can’t reach the network at the moment.".into(),
            ),
            Reason::Other(message) => (format!("Couldn’t open {host}"), message.clone()),
        }
    }
}

/// The page for `uri`, built from the bundled template.
pub fn page(uri: &str, reason: &Reason) -> String {
    let template = gio::resources_lookup_data(TEMPLATE, gio::ResourceLookupFlags::NONE)
        .expect("the failure page is bundled");
    fill(
        std::str::from_utf8(&template).expect("the failure page is UTF-8"),
        uri,
        reason,
    )
}

fn fill(template: &str, uri: &str, reason: &Reason) -> String {
    let (title, detail) = reason.words(&host(uri));
    template
        .replace("{{title}}", &escape(&title))
        .replace("{{detail}}", &escape(&detail))
        .replace("{{uri}}", &escape(uri))
}

fn host(uri: &str) -> String {
    let rest = uri.split_once("://").map_or(uri, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    if host.is_empty() {
        "This page".into()
    } else {
        host.to_owned()
    }
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEMPLATE: &str = "<h1>{{title}}</h1><p>{{detail}}</p><a href=\"{{uri}}\">Try again</a>";

    #[test]
    fn network_timeouts_read_as_no_answer_not_cancelled() {
        let error = glib::Error::new(gio::IOErrorEnum::Cancelled, "Operation was cancelled");
        assert_eq!(Reason::of(&error), Reason::NoAnswer);
        let page = fill(
            TEMPLATE,
            "http://calm.neverssl.com/online",
            &Reason::of(&error),
        );
        assert!(page.contains("calm.neverssl.com didn’t answer"), "{page}");
        assert!(!page.to_lowercase().contains("cancel"), "{page}");
    }

    #[test]
    fn other_errors_keep_the_engines_words() {
        let error = glib::Error::new(gio::IOErrorEnum::NotSupported, "Unsupported thing");
        assert_eq!(
            Reason::of(&error),
            Reason::Other("Unsupported thing".into())
        );
    }

    #[test]
    fn hosts_come_from_the_authority() {
        assert_eq!(
            host("https://user@example.org:8080/a?b#c"),
            "example.org:8080"
        );
        assert_eq!(host("http://example.org"), "example.org");
        assert_eq!(host(""), "This page");
    }

    #[test]
    fn everything_from_the_address_is_escaped() {
        let page = fill(
            TEMPLATE,
            "http://x/\"><script>alert(1)</script>",
            &Reason::NoAnswer,
        );
        assert!(!page.contains("<script>"), "{page}");
        assert!(page.contains("&quot;&gt;&lt;script&gt;"), "{page}");
    }
}

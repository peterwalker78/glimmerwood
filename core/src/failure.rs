//! What a tab shows when a page can't be opened.
//!
//! An engine's own words can mislead: a server that never answers is reported
//! by one as "Operation was cancelled" and by another as a bare error number.
//! This says what happened in plain words, quietly, with a way to try again.
//!
//! Pure: the shell classifies its engine's error into a `Reason`, and hands
//! the template in, since each platform carries its files differently.

/// Why a page didn't open, in the terms a reader cares about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    /// The server accepted nothing or said nothing before the network gave up.
    NoAnswer,
    /// The connection was refused or dropped.
    Refused,
    /// The name doesn't resolve.
    NotFound,
    /// This machine has no route to the network.
    Offline,
    /// The connection couldn't be trusted: a certificate that doesn't match,
    /// has expired, or isn't signed by anyone known.
    Untrusted,
    /// Anything else, with the engine's own words.
    Other(String),
}

impl Reason {
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
            Reason::Untrusted => (
                format!("Can’t make a private connection to {host}"),
                "The certificate it offered doesn’t check out, so what you send \
                 could be read by someone else."
                    .into(),
            ),
            Reason::Other(message) => (format!("Couldn’t open {host}"), message.clone()),
        }
    }
}

/// The page for `uri`, built from the bundled template.
pub fn fill(template: &str, uri: &str, reason: &Reason) -> String {
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
    fn a_silent_server_is_named_rather_than_blamed_on_a_cancellation() {
        let page = fill(
            TEMPLATE,
            "http://calm.neverssl.com/online",
            &Reason::NoAnswer,
        );
        assert!(page.contains("calm.neverssl.com didn’t answer"), "{page}");
        assert!(!page.to_lowercase().contains("cancel"), "{page}");
    }

    #[test]
    fn an_untrusted_certificate_says_what_is_at_stake() {
        let page = fill(TEMPLATE, "https://expired.example.org/", &Reason::Untrusted);
        assert!(page.contains("private connection"), "{page}");
        assert!(page.contains("read by someone else"), "{page}");
    }

    #[test]
    fn other_errors_keep_the_engines_words() {
        let page = fill(TEMPLATE, "https://x/", &Reason::Other("Odd thing".into()));
        assert!(page.contains("Odd thing"), "{page}");
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

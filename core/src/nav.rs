//! Turning what was typed into the address field into somewhere to go.
//!
//! Pure: no GTK, no WebKit, no clock.

use crate::protocol::Security;

/// Default search: a privacy-respecting engine.
pub const SEARCH: &str = "https://duckduckgo.com/?q=";

#[derive(Debug, PartialEq, Eq)]
pub struct Target {
    /// What to load first.
    pub uri: String,
    /// Plain HTTP to fall back to if `uri` was upgraded to HTTPS and the
    /// connection fails. Never used for certificate errors.
    pub fallback: Option<String>,
}

pub fn resolve(input: &str) -> Option<Target> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }

    if let Some((scheme, rest)) = input.split_once(':')
        && is_scheme(scheme)
    {
        match scheme.to_ascii_lowercase().as_str() {
            "https" if rest.starts_with("//") => return Some(direct(input)),
            "http" if rest.starts_with("//") => {
                return Some(Target {
                    uri: format!("https:{rest}"),
                    fallback: Some(input.to_owned()),
                });
            }
            "about" if rest.eq_ignore_ascii_case("blank") => return Some(direct("about:blank")),
            // Anything else with a colon is either `host:port` or a scheme we
            // don't open from the address field (javascript:, file:, wisp:).
            _ => {}
        }
    }

    if looks_like_address(input) {
        return Some(Target {
            uri: format!("https://{input}"),
            fallback: Some(format!("http://{input}")),
        });
    }

    Some(direct(&format!("{SEARCH}{}", encode_query(input))))
}

/// Whether two addresses name the same page, allowing for WebKit adding the
/// trailing slash to an empty path (`https://example.org` → `https://example.org/`).
pub fn same_address(a: &str, b: &str) -> bool {
    let trim = |uri: &str| -> String {
        match uri.split_once("://") {
            Some((scheme, rest)) if !rest.contains('/') => format!("{scheme}://{rest}/"),
            _ => uri.to_owned(),
        }
    };
    trim(a).eq_ignore_ascii_case(&trim(b))
}

/// `https://www.example.org/a` → `example.org`; empty for local pages.
pub fn host_of(uri: &str) -> String {
    let Some((scheme, rest)) = uri.split_once("://") else {
        return String::new();
    };
    if !matches!(scheme, "http" | "https") {
        return String::new();
    }
    let host = rest.split(['/', '?', '#', ':']).next().unwrap_or_default();
    host.strip_prefix("www.")
        .unwrap_or(host)
        .to_ascii_lowercase()
}

pub fn security(uri: &str) -> Security {
    let lower = uri.get(..8).unwrap_or(uri).to_ascii_lowercase();
    if lower.starts_with("https://") {
        Security::Secure
    } else if lower.starts_with("http://") {
        Security::NotSecure
    } else {
        Security::Local
    }
}

fn direct(uri: &str) -> Target {
    Target {
        uri: uri.to_owned(),
        fallback: None,
    }
}

fn is_scheme(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

fn looks_like_address(input: &str) -> bool {
    if input.chars().any(char::is_whitespace) {
        return false;
    }
    let authority = input.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.starts_with('[') {
        return authority.contains(']'); // IPv6 literal
    }
    let host = match authority.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => host,
        Some(_) => return false,
        None => authority,
    };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 || labels.iter().any(|l| l.is_empty()) {
        return false;
    }
    let valid = |l: &&str| l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !labels.iter().all(valid) {
        return false;
    }
    let numeric = |l: &&str| l.chars().all(|c| c.is_ascii_digit());
    if labels.iter().all(numeric) {
        return labels.len() == 4; // IPv4, not "3.14"
    }
    let tld = labels[labels.len() - 1];
    tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic())
}

fn encode_query(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b' ' => out.push('+'),
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Percent-decoding, for text that arrived inside an address.
pub fn unescape(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let digit = |b: u8| (b as char).to_digit(16);
        match (bytes[i], bytes.get(i + 1), bytes.get(i + 2)) {
            (b'%', Some(&high), Some(&low)) => match (digit(high), digit(low)) {
                (Some(high), Some(low)) => {
                    out.push((high * 16 + low) as u8);
                    i += 3;
                }
                _ => {
                    out.push(bytes[i]);
                    i += 1;
                }
            },
            _ => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uri(input: &str) -> String {
        resolve(input).expect("resolves").uri
    }

    #[test]
    fn empty_input_goes_nowhere() {
        assert_eq!(resolve("   "), None);
    }

    #[test]
    fn bare_domains_try_https_first() {
        assert_eq!(
            resolve("wikipedia.org"),
            Some(Target {
                uri: "https://wikipedia.org".into(),
                fallback: Some("http://wikipedia.org".into()),
            })
        );
        assert_eq!(
            uri("en.wikipedia.org/wiki/Moss"),
            "https://en.wikipedia.org/wiki/Moss"
        );
        assert_eq!(uri("localhost:8080"), "https://localhost:8080");
        assert_eq!(uri("192.168.1.1"), "https://192.168.1.1");
    }

    #[test]
    fn typed_http_is_upgraded_but_can_fall_back() {
        assert_eq!(
            resolve("http://example.org/a"),
            Some(Target {
                uri: "https://example.org/a".into(),
                fallback: Some("http://example.org/a".into()),
            })
        );
    }

    #[test]
    fn typed_https_is_used_as_is() {
        assert_eq!(
            resolve("https://example.org"),
            Some(Target {
                uri: "https://example.org".into(),
                fallback: None,
            })
        );
    }

    #[test]
    fn everything_else_is_a_search() {
        assert_eq!(uri("moss gardens"), format!("{SEARCH}moss+gardens"));
        assert_eq!(uri("3.14"), format!("{SEARCH}3.14"));
        assert_eq!(uri("what is c++?"), format!("{SEARCH}what+is+c%2B%2B%3F"));
        assert_eq!(uri("café"), format!("{SEARCH}caf%C3%A9"));
        assert_eq!(uri("rust"), format!("{SEARCH}rust"));
    }

    #[test]
    fn schemes_that_could_run_code_are_searched_not_opened() {
        assert!(uri("javascript:alert(1)").starts_with(SEARCH));
        assert!(uri("glimmerwood://chrome/index.html").starts_with(SEARCH));
        assert!(uri("file:///etc/passwd").starts_with(SEARCH));
    }

    #[test]
    fn same_address_allows_for_the_added_slash() {
        assert!(same_address("https://example.org", "https://example.org/"));
        assert!(same_address("https://Example.org/", "https://example.org"));
        assert!(!same_address(
            "https://example.org/a",
            "https://example.org/"
        ));
        assert!(!same_address("https://example.org", "http://example.org/"));
    }

    #[test]
    fn security_follows_the_scheme() {
        assert_eq!(security("https://example.org"), Security::Secure);
        assert_eq!(security("HTTP://example.org"), Security::NotSecure);
        assert_eq!(security("about:blank"), Security::Local);
        assert_eq!(security(""), Security::Local);
    }

    #[test]
    fn percent_signs_come_back_as_what_they_stood_for() {
        assert_eq!(unescape("a%20b"), "a b");
        assert_eq!(unescape("caf%C3%A9"), "café");
        assert_eq!(unescape("100%"), "100%");
        assert_eq!(unescape("%zz"), "%zz");
        assert_eq!(unescape("plain"), "plain");
    }
}

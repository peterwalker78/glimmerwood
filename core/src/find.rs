//! Find in page: how a search went, in words for the find bar.

/// WebKit stops counting here and reports "more than this".
pub const MAX_MATCHES: u32 = 999;

/// `matches` is `None` when nothing was found. WebKit reports `u32::MAX` once
/// the count passes `MAX_MATCHES`.
pub fn summary(matches: Option<u32>) -> String {
    match matches {
        None | Some(0) => "No matches".into(),
        Some(1) => "One match".into(),
        Some(n) if n > MAX_MATCHES => format!("Over {MAX_MATCHES} matches"),
        Some(n) => format!("{n} matches"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_read_as_words() {
        assert_eq!(summary(None), "No matches");
        assert_eq!(summary(Some(0)), "No matches");
        assert_eq!(summary(Some(1)), "One match");
        assert_eq!(summary(Some(12)), "12 matches");
        assert_eq!(summary(Some(MAX_MATCHES)), "999 matches");
        assert_eq!(summary(Some(u32::MAX)), "Over 999 matches");
    }
}

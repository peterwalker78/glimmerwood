//! Good news for Newsboat, the feed reader that runs in a terminal: which
//! feeds, where Newsboat keeps its list of them, and adding them to that list
//! or taking them out again.
//!
//! Pure apart from `urls_file`'s question about the disk. Lines are only ever
//! added or removed whole, and only lines carrying the `glimmerwood` tag are
//! ever removed, so the user's own feeds and comments are left as they were.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::protocol::NewsKind;

/// The tag on every line Glimmerwood writes, and the only lines it removes.
pub const TAG: &str = "glimmerwood";

/// The comment above the feeds Glimmerwood adds.
const HEADER: &str = "# Good news from Glimmerwood. Its Settings can take these out again.";

/// The comment above the settings Glimmerwood adds to Newsboat's config.
const CONFIG_HEADER: &str =
    "# Added by Glimmerwood with its good news. Its Settings can take these out again.";

/// What Glimmerwood sets in Newsboat's config, each only when the config
/// doesn't already say: fetch every feed as it starts, since otherwise a
/// fresh list opens empty, and open links in the desktop's default browser,
/// since Newsboat's own default is lynx, which few have installed.
pub const REFRESH: &str = "refresh-on-startup";
pub const BROWSER: &str = "browser";
const SETTINGS: [(&str, &str); 2] = [(REFRESH, "yes"), (BROWSER, "xdg-open")];

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Feed {
    /// The reputation-list entry its stories open under.
    pub site: String,
    pub url: String,
    pub name: String,
    pub kind: NewsKind,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoodNews {
    pub feeds: Vec<Feed>,
}

impl GoodNews {
    pub fn bundled() -> GoodNews {
        toml::from_str(include_str!("../data/good-news.toml"))
            .expect("core/data/good-news.toml is valid")
    }
}

/// Where Newsboat reads its feeds from, if it has been run here. Newsboat
/// uses `~/.newsboat` when it exists and the XDG directory otherwise, and
/// makes one of them the first time it runs.
pub fn urls_file(home: &Path, is_dir: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    [
        home.join(".newsboat"),
        home.join(".config").join("newsboat"),
    ]
    .into_iter()
    .find(|dir| is_dir(dir))
    .map(|dir| dir.join("urls"))
}

/// Newsboat's config, which sits beside its list.
pub fn config_file(urls: &Path) -> PathBuf {
    urls.with_file_name("config")
}

/// What the config sets `name` to, if anything. The last line wins.
fn setting<'a>(config: &'a str, name: &str) -> Option<&'a str> {
    config
        .lines()
        .filter_map(|line| {
            let mut words = words(line);
            (words.next()? == name).then(|| words.next().unwrap_or(""))
        })
        .next_back()
}

/// Whether Newsboat fetches every feed as it starts.
pub fn fetches_on_start(config: &str) -> bool {
    matches!(setting(config, REFRESH), Some("yes" | "true"))
}

/// Whether the config already says everything Glimmerwood would set.
pub fn configured(config: &str) -> bool {
    SETTINGS
        .iter()
        .all(|(name, _)| setting(config, name).is_some())
}

/// `config` with Glimmerwood's settings added under a comment, and the names
/// of those it added; `None` when the config already says all of them,
/// because those choices are the user's.
pub fn configure(config: &str) -> Option<(String, Vec<&'static str>)> {
    let missing: Vec<(&str, &str)> = SETTINGS
        .into_iter()
        .filter(|(name, _)| setting(config, name).is_none())
        .collect();
    if missing.is_empty() {
        return None;
    }
    let mut out = config.to_owned();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if !out.trim().is_empty() && !out.ends_with("\n\n") {
        out.push('\n');
    }
    out.push_str(CONFIG_HEADER);
    out.push('\n');
    for (name, value) in &missing {
        out.push_str(&format!("{name} {value}\n"));
    }
    Some((out, missing.into_iter().map(|(name, _)| name).collect()))
}

/// `config` without the settings Glimmerwood added, or `None` if it has
/// none: its comment and the lines right after it that are its own.
pub fn unconfigure(config: &str) -> Option<String> {
    let lines: Vec<&str> = config.lines().collect();
    let at = lines.iter().position(|&line| line == CONFIG_HEADER)?;
    let ours: Vec<String> = SETTINGS
        .iter()
        .map(|(name, value)| format!("{name} {value}"))
        .collect();
    let after = at
        + 1
        + lines[at + 1..]
            .iter()
            .take_while(|line| ours.iter().any(|our| our == *line))
            .count();
    let mut kept = lines[..at].to_vec();
    kept.extend(&lines[after..]);
    Some(tidy_end(&kept))
}

/// Lines joined, without blank lines at the end.
fn tidy_end(lines: &[&str]) -> String {
    let end = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .map_or(0, |last| last + 1);
    let mut out = lines[..end].join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// What adding the feeds did.
#[derive(Debug, PartialEq)]
pub struct Added {
    pub text: String,
    pub added: usize,
    /// Feeds that were in the list already, whoever put them there.
    pub already: usize,
}

/// `text` with every feed it hasn't got added at the end, under a comment.
pub fn add(text: &str, feeds: &[Feed]) -> Added {
    let present: HashSet<&str> = text.lines().filter_map(url_of).collect();
    let missing: Vec<&Feed> = feeds
        .iter()
        .filter(|feed| !present.contains(feed.url.as_str()))
        .collect();
    let mut out = text.to_owned();
    if !missing.is_empty() {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        if !out.trim().is_empty() && !out.ends_with("\n\n") {
            out.push('\n');
        }
        if !text.lines().any(|line| line == HEADER) {
            out.push_str(HEADER);
            out.push('\n');
        }
        for feed in &missing {
            out.push_str(&line_for(feed));
            out.push('\n');
        }
    }
    Added {
        text: out,
        added: missing.len(),
        already: feeds.len() - missing.len(),
    }
}

/// `text` without the lines Glimmerwood added, and how many feeds went.
pub fn remove(text: &str, feeds: &[Feed]) -> (String, usize) {
    let ours: HashSet<&str> = feeds.iter().map(|feed| feed.url.as_str()).collect();
    let mut removed = 0;
    let kept: Vec<&str> = text
        .lines()
        .filter(|&line| {
            let drop = url_of(line).is_some_and(|url| ours.contains(url))
                && words(line).skip(1).any(|word| word == TAG);
            removed += usize::from(drop);
            !drop && line != HEADER
        })
        .collect();
    if removed == 0 {
        return (text.to_owned(), 0);
    }
    (tidy_end(&kept), removed)
}

/// Which of `feeds` the list has, in order, whoever added them.
pub fn present(text: &str, feeds: &[Feed]) -> Vec<bool> {
    let present: HashSet<&str> = text.lines().filter_map(url_of).collect();
    feeds
        .iter()
        .map(|feed| present.contains(feed.url.as_str()))
        .collect()
}

/// One feed as a line of Newsboat's list: the address, `~` and the name it
/// shows, then tags.
fn line_for(feed: &Feed) -> String {
    let kind = match feed.kind {
        NewsKind::Working => "\"what's working\"",
        NewsKind::Light => "light",
        NewsKind::Awe => "awe",
    };
    format!("{} \"~{}\" {TAG} {kind}", feed.url, feed.name)
}

/// The feed a line of the list is for; none for blanks and comments.
fn url_of(line: &str) -> Option<&str> {
    let first = words(line).next()?;
    (!first.starts_with('#')).then_some(first)
}

/// A line's words, where a word in double quotes may hold spaces.
fn words(line: &str) -> impl Iterator<Item = &str> {
    let mut rest = line.trim_start();
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let (word, after) = match rest.strip_prefix('"') {
            Some(quoted) => match quoted.find('"') {
                Some(end) => (&quoted[..end], &quoted[end + 1..]),
                None => (quoted, ""),
            },
            None => match rest.find(char::is_whitespace) {
                Some(end) => (&rest[..end], &rest[end..]),
                None => (rest, ""),
            },
        };
        rest = after.trim_start();
        Some(word)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reputation::Lists;

    fn feed(url: &str, name: &str, kind: NewsKind) -> Feed {
        Feed {
            site: String::new(),
            url: url.into(),
            name: name.into(),
            kind,
        }
    }

    fn two() -> Vec<Feed> {
        vec![
            feed("https://a.example/feed", "A", NewsKind::Working),
            feed("https://b.example/rss", "B b", NewsKind::Awe),
        ]
    }

    #[test]
    fn every_feed_opens_somewhere_nourishing() {
        let lists = Lists::bundled();
        let mut urls = HashSet::new();
        for feed in GoodNews::bundled().feeds {
            match lists.lookup_site(&feed.site) {
                Some((entry, list)) => {
                    assert_eq!(entry, feed.site, "{} isn't an entry", feed.site);
                    assert!(lists.weight(list) > 0.0, "{} isn't nourishing", feed.site);
                }
                None => panic!("{} is on no list", feed.site),
            }
            assert!(feed.url.starts_with("https://"), "{}", feed.url);
            assert!(!feed.name.contains('"'), "{}", feed.name);
            assert!(urls.insert(feed.url.clone()), "{} is there twice", feed.url);
        }
    }

    #[test]
    fn newsboat_s_own_directory_comes_first() {
        let home = Path::new("/home/someone");
        let both = |_: &Path| true;
        assert_eq!(urls_file(home, both), Some(home.join(".newsboat/urls")));
        let xdg = |dir: &Path| dir.ends_with(".config/newsboat");
        assert_eq!(
            urls_file(home, xdg),
            Some(home.join(".config/newsboat/urls"))
        );
        assert_eq!(urls_file(home, |_: &Path| false), None);
    }

    #[test]
    fn adds_to_an_empty_list() {
        let added = add("", &two());
        assert_eq!(
            added.text,
            format!(
                "{HEADER}\n\
                 https://a.example/feed \"~A\" glimmerwood \"what's working\"\n\
                 https://b.example/rss \"~B b\" glimmerwood awe\n"
            )
        );
        assert_eq!((added.added, added.already), (2, 0));
    }

    #[test]
    fn leaves_the_user_s_own_lines_alone() {
        let mine = "# mine\nhttps://mine.example/feed tech\nhttps://b.example/rss";
        let added = add(mine, &two());
        assert!(added.text.starts_with(&format!("{mine}\n\n{HEADER}\n")));
        assert_eq!((added.added, added.already), (1, 1));
        assert_eq!(added.text.matches("b.example").count(), 1);

        // Taking ours out leaves theirs, even the feed they had already.
        let (back, removed) = remove(&added.text, &two());
        assert_eq!(removed, 1);
        assert_eq!(back, format!("{mine}\n"));
    }

    #[test]
    fn adding_twice_changes_nothing() {
        let once = add("", &two()).text;
        let twice = add(&once, &two());
        assert_eq!(twice.text, once);
        assert_eq!((twice.added, twice.already), (0, 2));
    }

    #[test]
    fn removes_everything_it_added() {
        let (text, removed) = remove(&add("", &two()).text, &two());
        assert_eq!((text.as_str(), removed), ("", 2));
        assert_eq!(remove("x y\n", &two()), ("x y\n".to_owned(), 0));
    }

    #[test]
    fn knows_which_feeds_are_there() {
        let text = "  https://b.example/rss\n#https://a.example/feed\n";
        assert_eq!(present(text, &two()), vec![false, true]);
    }

    #[test]
    fn sets_only_what_the_config_leaves_unsaid() {
        let (set, added) = configure("").expect("an empty config says nothing");
        assert_eq!(
            set,
            format!("{CONFIG_HEADER}\nrefresh-on-startup yes\nbrowser xdg-open\n")
        );
        assert_eq!(added, vec![REFRESH, BROWSER]);
        assert!(fetches_on_start(&set));
        assert_eq!(configure(&set), None);
        assert!(configured(&set) && !configured(""));

        let theirs = "browser firefox\nrefresh-on-startup no\n";
        assert!(!fetches_on_start(theirs));
        assert_eq!(configure(theirs), None);

        let (set, added) = configure("browser \"firefox %u\"").expect("fetching unsaid");
        assert_eq!(added, vec![REFRESH]);
        assert!(set.ends_with(&format!("{CONFIG_HEADER}\nrefresh-on-startup yes\n")));
        assert!(!fetches_on_start("# refresh-on-startup yes"));
    }

    #[test]
    fn takes_back_only_its_own_settings() {
        let mine = "color background white black\n";
        let (set, _) = configure(mine).expect("nothing said yet");
        let later = format!("{set}bind-key j down\n");
        assert_eq!(
            unconfigure(&later),
            Some("color background white black\n\nbind-key j down\n".to_owned())
        );
        assert_eq!(unconfigure(&set), Some(mine.to_owned()));
        assert_eq!(unconfigure("refresh-on-startup yes\n"), None);
        assert_eq!(
            config_file(Path::new("/h/.newsboat/urls")),
            Path::new("/h/.newsboat/config")
        );
    }

    #[test]
    fn reads_quoted_words() {
        let line = r#"  https://x.example/  "~Two words"   tag "#;
        assert_eq!(
            words(line).collect::<Vec<_>>(),
            vec!["https://x.example/", "~Two words", "tag"]
        );
        assert_eq!(url_of("   "), None);
        assert_eq!(url_of("# note"), None);
    }
}

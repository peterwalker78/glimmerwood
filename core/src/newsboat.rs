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
    let end = kept
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .map_or(0, |last| last + 1);
    let mut out = kept[..end].join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    (out, removed)
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

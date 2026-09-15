//! The reputation lists: which list a site is on.
//!
//! Pure: parsing and matching only. Reading the user's file from disk and
//! noticing when it changes belongs to the caller.

use std::collections::HashMap;

use serde::Deserialize;

use crate::dose::Place;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum List {
    DrainingStrong,
    DrainingMild,
    News,
    Ordinary,
    NourishingMild,
    NourishingStrong,
    /// Adult sites: the dose holds, nothing is recorded, the wisp gives the
    /// user privacy.
    Private,
    /// Parts of listed sites carved back out to hold steady.
    Unlisted,
}

impl List {
    #[cfg(test)]
    pub const ALL: [List; 8] = [
        List::DrainingStrong,
        List::DrainingMild,
        List::News,
        List::Ordinary,
        List::NourishingMild,
        List::NourishingStrong,
        List::Private,
        List::Unlisted,
    ];

    /// The lists that carry a weight.
    const WEIGHTED: usize = 6;

    pub fn key(self) -> &'static str {
        match self {
            List::DrainingStrong => "draining-strong",
            List::DrainingMild => "draining-mild",
            List::News => "news",
            List::Ordinary => "ordinary",
            List::NourishingMild => "nourishing-mild",
            List::NourishingStrong => "nourishing-strong",
            List::Private => "private",
            List::Unlisted => "unlisted",
        }
    }

    fn index(self) -> usize {
        self as usize
    }

    fn weighted(self) -> bool {
        self.index() < List::WEIGHTED
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(rename = "draining-strong")]
    draining_strong: Option<Table>,
    #[serde(rename = "draining-mild")]
    draining_mild: Option<Table>,
    news: Option<Table>,
    ordinary: Option<Table>,
    #[serde(rename = "nourishing-mild")]
    nourishing_mild: Option<Table>,
    #[serde(rename = "nourishing-strong")]
    nourishing_strong: Option<Table>,
    private: Option<Table>,
    unlisted: Option<Table>,
    removed: Option<Removed>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Table {
    weight: Option<f64>,
    #[serde(default)]
    sites: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Removed {
    #[serde(default)]
    sites: Vec<String>,
}

impl File {
    fn tables(&self) -> [(List, Option<&Table>); 8] {
        [
            (List::DrainingStrong, self.draining_strong.as_ref()),
            (List::DrainingMild, self.draining_mild.as_ref()),
            (List::News, self.news.as_ref()),
            (List::Ordinary, self.ordinary.as_ref()),
            (List::NourishingMild, self.nourishing_mild.as_ref()),
            (List::NourishingStrong, self.nourishing_strong.as_ref()),
            (List::Private, self.private.as_ref()),
            (List::Unlisted, self.unlisted.as_ref()),
        ]
    }
}

#[derive(Clone, Debug)]
pub struct Lists {
    weights: [f64; List::WEIGHTED],
    entries: HashMap<String, List>,
}

impl Lists {
    pub fn bundled() -> Lists {
        Lists::seed(include_str!("../data/reputation.toml"))
            .expect("core/data/reputation.toml is valid")
    }

    /// The seed list: every table present with a weight, no entry twice.
    pub fn seed(text: &str) -> Result<Lists, String> {
        let file = parse(text)?;
        if file.removed.is_some() {
            return Err("the seed list has no [removed] table".into());
        }
        let mut weights = [0.0; List::WEIGHTED];
        let mut entries = HashMap::new();
        for (list, table) in file.tables() {
            let table = table.ok_or_else(|| format!("missing [{}]", list.key()))?;
            match (list.weighted(), table.weight) {
                (true, Some(weight)) => weights[list.index()] = weight,
                (true, None) => return Err(format!("[{}] has no weight", list.key())),
                (false, Some(_)) => return Err(format!("[{}] takes no weight", list.key())),
                (false, None) => {}
            }
            for site in &table.sites {
                if let Some(previous) = entries.insert(site.clone(), list) {
                    return Err(format!(
                        "{site} is on both [{}] and [{}]",
                        previous.key(),
                        list.key()
                    ));
                }
            }
        }
        check_weights(&weights)?;
        Ok(Lists { weights, entries })
    }

    /// Apply the user's own file on top: sites it lists move to (or join) that
    /// list, `[removed]` sites become unlisted, and a table's weight, if given,
    /// replaces the seed's.
    pub fn with_user(&self, text: &str) -> Result<Lists, String> {
        let file = parse(text)?;
        let mut lists = self.clone();
        let mut seen = HashMap::new();
        for (list, table) in file.tables() {
            let Some(table) = table else { continue };
            if let Some(weight) = table.weight {
                if !list.weighted() {
                    return Err(format!("[{}] takes no weight", list.key()));
                }
                lists.weights[list.index()] = weight;
            }
            for site in &table.sites {
                if let Some(previous) = seen.insert(site.clone(), list) {
                    return Err(format!(
                        "{site} is on both [{}] and [{}] in your list",
                        previous.key(),
                        list.key()
                    ));
                }
                lists.entries.insert(site.clone(), list);
            }
        }
        for site in file.removed.iter().flat_map(|r| &r.sites) {
            if seen.contains_key(site) {
                return Err(format!("{site} is both on a list and removed in your list"));
            }
            lists.entries.remove(site);
        }
        check_weights(&lists.weights)?;
        Ok(lists)
    }

    #[cfg(test)]
    pub fn count(&self, list: List) -> usize {
        self.entries.values().filter(|&&l| l == list).count()
    }

    /// The list's weight; private and unlisted sites carry none.
    pub fn weight(&self, list: List) -> f64 {
        if list.weighted() {
            self.weights[list.index()]
        } else {
            0.0
        }
    }

    /// Which entry, on which list, an address falls under.
    pub fn lookup(&self, uri: &str) -> Option<(&str, List)> {
        let (host, path) = split_address(uri)?;
        let labels: Vec<&str> = host.split('.').collect();
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let mut best: Option<(&str, List)> = None;
        // Every parent domain (not the bare TLD), with every path prefix.
        for start in 0..labels.len().saturating_sub(1) {
            let domain = labels[start..].join(".");
            for depth in 0..=segments.len() {
                let key = if depth == 0 {
                    domain.clone()
                } else {
                    format!("{domain}/{}", segments[..depth].join("/"))
                };
                if let Some((entry, &list)) = self.entries.get_key_value(key.as_str())
                    && best.is_none_or(|(b, _)| entry.len() > b.len())
                {
                    best = Some((entry.as_str(), list));
                }
            }
        }
        best
    }

    /// What the dose engine needs to know about an address.
    pub fn place(&self, uri: &str) -> Place {
        match self.lookup(uri) {
            Some((_, List::Private)) => Place::Private,
            Some((_, List::Unlisted)) | None => Place::Unlisted,
            Some((entry, list)) => Place::Listed {
                entry: entry.to_owned(),
                weight: self.weight(list),
                news: list == List::News,
            },
        }
    }
}

fn parse(text: &str) -> Result<File, String> {
    let file: File = toml::from_str(text).map_err(|e| e.to_string())?;
    for (list, table) in file.tables() {
        for site in table.iter().flat_map(|t| &t.sites) {
            check_entry(site).map_err(|why| format!("[{}] {site:?}: {why}", list.key()))?;
        }
    }
    for site in file.removed.iter().flat_map(|r| &r.sites) {
        check_entry(site).map_err(|why| format!("[removed] {site:?}: {why}"))?;
    }
    Ok(file)
}

fn check_entry(site: &str) -> Result<(), &'static str> {
    if site.is_empty() {
        return Err("empty");
    }
    if site.contains("://") {
        return Err("leave out the scheme");
    }
    if site.starts_with("www.") {
        return Err("leave out www. (subdomains match their parent)");
    }
    if site.ends_with('/') {
        return Err("no trailing slash");
    }
    if site
        .chars()
        .any(|c| c.is_ascii_uppercase() || c.is_whitespace())
    {
        return Err("lowercase, no spaces");
    }
    let host = site.split('/').next().unwrap_or_default();
    if !host.contains('.') || host.starts_with('.') || host.ends_with('.') {
        return Err("not a domain");
    }
    Ok(())
}

fn check_weights(weights: &[f64; List::WEIGHTED]) -> Result<(), String> {
    let w = |list: List| weights[list.index()];
    let in_range = weights.iter().all(|w| (-1.0..=1.0).contains(w));
    let ordered = w(List::DrainingStrong) <= w(List::DrainingMild)
        && w(List::DrainingMild) < 0.0
        && w(List::News) < 0.0
        && w(List::Ordinary) == 0.0
        && 0.0 < w(List::NourishingMild)
        && w(List::NourishingMild) <= w(List::NourishingStrong);
    if ordered && in_range {
        Ok(())
    } else {
        Err("weights must run from draining (below 0) through ordinary (0) to nourishing (above 0), within -1..1".into())
    }
}

/// `https://M.YouTube.com:443/Shorts/x?y` → (`m.youtube.com`, `/shorts/x`).
/// Only web addresses have a place; anything else is unlisted.
fn split_address(uri: &str) -> Option<(String, String)> {
    let (scheme, rest) = uri.split_once("://")?;
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
        return None;
    }
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = match host.rsplit_once(':') {
        Some((h, port)) if port.chars().all(|c| c.is_ascii_digit()) => h,
        _ => host,
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    let path = rest[end..]
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    Some((host, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: &str = r#"
        [draining-strong]
        weight = -1.0
        sites = ["tiktok.com", "youtube.com/shorts"]

        [draining-mild]
        weight = -0.4
        sites = ["youtube.com", "reddit.com", "bbc.co.uk/news"]

        [ordinary]
        weight = 0.0
        sites = ["gmail.com", "gov.uk"]

        [nourishing-mild]
        weight = 0.5
        sites = ["wikipedia.org", "reddit.com/r/diy"]

        [nourishing-strong]
        weight = 1.0
        sites = ["khanacademy.org", "bbc.co.uk/bitesize"]

        [news]
        weight = -0.4
        sites = ["theguardian.com"]

        [private]
        sites = ["adult.example"]

        [unlisted]
        sites = ["mail.gmail.com", "reddit.com/r/diy/wiki"]
    "#;

    fn lists() -> Lists {
        Lists::seed(SEED).expect("fixture parses")
    }

    fn entry(lists: &Lists, uri: &str) -> Option<(String, List)> {
        lists.lookup(uri).map(|(e, l)| (e.to_owned(), l))
    }

    #[test]
    fn subdomains_match_their_parent() {
        let lists = lists();
        assert_eq!(
            entry(&lists, "https://m.youtube.com/watch?v=1"),
            Some(("youtube.com".into(), List::DrainingMild))
        );
        assert_eq!(
            entry(&lists, "https://en.wikipedia.org/wiki/Moss"),
            Some(("wikipedia.org".into(), List::NourishingMild))
        );
        assert_eq!(
            entry(&lists, "https://www.gov.uk/browse"),
            Some(("gov.uk".into(), List::Ordinary))
        );
    }

    #[test]
    fn the_longest_matching_entry_wins() {
        let lists = lists();
        assert_eq!(
            entry(&lists, "https://www.youtube.com/shorts/abc"),
            Some(("youtube.com/shorts".into(), List::DrainingStrong))
        );
        assert_eq!(
            entry(&lists, "https://old.reddit.com/r/DIY/comments/x"),
            Some(("reddit.com/r/diy".into(), List::NourishingMild))
        );
        assert_eq!(
            entry(&lists, "https://www.bbc.co.uk/bitesize/subjects"),
            Some(("bbc.co.uk/bitesize".into(), List::NourishingStrong))
        );
    }

    #[test]
    fn paths_match_whole_segments_only() {
        let lists = lists();
        assert_eq!(
            entry(&lists, "https://reddit.com/r/diyaudio"),
            Some(("reddit.com".into(), List::DrainingMild))
        );
        assert_eq!(entry(&lists, "https://www.bbc.co.uk/sport"), None);
    }

    #[test]
    fn lookalike_domains_and_other_schemes_are_unlisted() {
        let lists = lists();
        assert_eq!(entry(&lists, "https://nottiktok.com/"), None);
        assert_eq!(entry(&lists, "https://tiktok.com.example.net/"), None);
        assert_eq!(entry(&lists, "about:blank"), None);
        assert_eq!(entry(&lists, "glimmerwood://chrome/index.html"), None);
        assert_eq!(entry(&lists, "file:///home/tiktok.com"), None);
        assert_eq!(
            entry(&lists, "https://user@TikTok.com:443/@someone"),
            Some(("tiktok.com".into(), List::DrainingStrong))
        );
    }

    #[test]
    fn places_carry_the_entry_and_the_lists_weight_never_the_address() {
        let lists = lists();
        assert_eq!(
            lists.place("https://www.tiktok.com/@someone/video/123?secret=1"),
            Place::Listed {
                entry: "tiktok.com".into(),
                weight: -1.0,
                news: false,
            }
        );
        assert_eq!(lists.place("https://example.org/"), Place::Unlisted);
        assert_eq!(
            lists.place("https://www.theguardian.com/world"),
            Place::Listed {
                entry: "theguardian.com".into(),
                weight: -0.4,
                news: true,
            }
        );
    }

    #[test]
    fn private_sites_and_carve_outs_carry_no_name_or_weight() {
        let lists = lists();
        assert_eq!(
            lists.place("https://www.adult.example/video/1"),
            Place::Private
        );
        assert_eq!(lists.place("https://mail.gmail.com/inbox"), Place::Unlisted);
        assert_eq!(
            lists.place("https://reddit.com/r/diy/wiki/tools"),
            Place::Unlisted
        );
        assert!(Lists::seed(&SEED.replace("[private]\n", "[private]\nweight = 0.0\n")).is_err());
    }

    #[test]
    fn the_user_can_add_move_and_remove_sites() {
        let user = r#"
            [removed]
            sites = ["gmail.com"]

            [draining-mild]
            sites = ["wikipedia.org"]

            [nourishing-strong]
            sites = ["lichess.org"]
        "#;
        let lists = lists().with_user(user).expect("user file applies");
        assert_eq!(entry(&lists, "https://gmail.com/"), None);
        assert_eq!(
            entry(&lists, "https://en.wikipedia.org"),
            Some(("wikipedia.org".into(), List::DrainingMild))
        );
        assert_eq!(
            entry(&lists, "https://lichess.org"),
            Some(("lichess.org".into(), List::NourishingStrong))
        );
        // Everything else is untouched.
        assert_eq!(
            entry(&lists, "https://tiktok.com"),
            Some(("tiktok.com".into(), List::DrainingStrong))
        );
    }

    #[test]
    fn the_user_can_retune_a_lists_weight() {
        let lists = lists()
            .with_user("[draining-mild]\nweight = -0.6")
            .expect("applies");
        assert_eq!(lists.weight(List::DrainingMild), -0.6);
    }

    #[test]
    fn mistakes_in_a_list_are_reported_not_guessed_at() {
        let dup = SEED.replace(
            r#"sites = ["gmail.com", "gov.uk"]"#,
            r#"sites = ["gmail.com", "tiktok.com"]"#,
        );
        assert!(Lists::seed(&dup).unwrap_err().contains("tiktok.com"));
        let scheme = SEED.replace("\"gov.uk\"", "\"https://gov.uk\"");
        assert!(Lists::seed(&scheme).is_err());
        let www = SEED.replace("\"gov.uk\"", "\"www.gov.uk\"");
        assert!(Lists::seed(&www).is_err());
        assert!(
            lists()
                .with_user("[removed]\nsites = [\"a.com\"]\n[ordinary]\nsites = [\"a.com\"]")
                .is_err()
        );
        assert!(lists().with_user("[ordinary]\nweight = 0.3").is_err());
        assert!(lists().with_user("[unlisted]\nweight = 0.3").is_err());
        assert!(lists().with_user("[shiny]\nsites = []").is_err());
    }

    #[test]
    fn the_bundled_seed_list_is_comprehensive() {
        // At least 1,000 sites on the weighted lists, every list populated.
        let lists = Lists::bundled();
        let weighted: usize = List::ALL
            .iter()
            .filter(|l| l.weighted())
            .map(|&l| lists.count(l))
            .sum();
        assert!(weighted >= 1000, "only {weighted} weighted entries");
        for list in List::ALL {
            assert!(lists.count(list) > 0, "[{}] is empty", list.key());
        }
        assert_eq!(
            List::ALL.map(|l| lists.weight(l)),
            [-1.0, -0.4, -0.4, 0.0, 0.5, 1.0, 0.0, 0.0]
        );
    }
}

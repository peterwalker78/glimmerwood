//! Ratings as the user sees them, and what the Settings page's links ask for.
//!
//! Pure: the companion reads and writes the files and applies the changes.

use std::collections::HashMap;

use crate::protocol::{Rating, SiteRating};
use crate::reputation::{List, Lists};

/// What rating a list is, or unrated for no list (or `[removed]`).
pub fn rating_of(list: Option<List>) -> Rating {
    match list {
        Some(List::DrainingStrong) => Rating::DrainsALot,
        Some(List::DrainingMild) => Rating::DrainsALittle,
        Some(List::Ordinary) => Rating::Neither,
        Some(List::NourishingMild) => Rating::RestoresALittle,
        Some(List::NourishingStrong) => Rating::RestoresALot,
        Some(List::News) => Rating::News,
        Some(List::Private) => Rating::Private,
        Some(List::Unlisted) | None => Rating::Unrated,
        Some(List::Care) => Rating::Care,
    }
}

/// The list a rating puts a site on. Unrated is the user's `[unlisted]`, so
/// the site holds steady even if part of a rated site.
pub fn list_of(rating: Rating) -> List {
    match rating {
        Rating::DrainsALot => List::DrainingStrong,
        Rating::DrainsALittle => List::DrainingMild,
        Rating::Neither => List::Ordinary,
        Rating::RestoresALittle => List::NourishingMild,
        Rating::RestoresALot => List::NourishingStrong,
        Rating::News => List::News,
        Rating::Private => List::Private,
        Rating::Unrated => List::Unlisted,
        Rating::Care => List::Care,
    }
}

/// A site as Settings shows it. `lists` has the user's file applied; `yours`
/// is that file's entries (`None` for one under `[removed]`).
pub fn site_rating(
    lists: &Lists,
    seed: &Lists,
    yours: &HashMap<String, Option<List>>,
    site: &str,
) -> SiteRating {
    let (matched, rating) = match lists.lookup_site(site) {
        Some((entry, list)) => (entry.to_owned(), rating_of(Some(list))),
        None => (String::new(), Rating::Unrated),
    };
    SiteRating {
        site: site.to_owned(),
        rating,
        matched,
        yours: yours.get(site).map(|&list| rating_of(list)),
        seed: seed.exact(site).map(|list| rating_of(Some(list))),
    }
}

/// A link followed on the Settings page: `glimmerwood://settings/do/` and
/// then one of these, with the parts percent-encoded.
#[derive(Debug, PartialEq)]
pub enum Action {
    /// `rate/RATING/SITE`, where RATING is a rating's name or `glimmerwood`
    /// to go back to Glimmerwood's own (`None`).
    Rate {
        site: String,
        rating: Option<Rating>,
    },
    /// `look-up/TEXT`; empty clears the look-up.
    LookUp(String),
    /// `ask/on` or `ask/off`.
    Ask(bool),
    /// `night/HH:MM/HH:MM`.
    Night { starts: String, ends: String },
    /// `newsboat/add` or `newsboat/remove`: good news into Newsboat's list
    /// of feeds, or out of it again.
    Newsboat { add: bool },
}

impl Action {
    pub fn parse(action: &str, unescape: impl Fn(&str) -> Option<String>) -> Option<Action> {
        let (verb, rest) = action.split_once('/')?;
        match verb {
            "rate" => {
                let (rating, site) = rest.split_once('/')?;
                let rating = match rating {
                    "glimmerwood" => None,
                    key => Some(rating_named(key)?),
                };
                let site = unescape(site)?;
                (!site.is_empty()).then_some(Action::Rate { site, rating })
            }
            "look-up" => Some(Action::LookUp(unescape(rest)?)),
            "ask" => match rest {
                "on" => Some(Action::Ask(true)),
                "off" => Some(Action::Ask(false)),
                _ => None,
            },
            "night" => {
                let (starts, ends) = rest.split_once('/')?;
                Some(Action::Night {
                    starts: unescape(starts)?,
                    ends: unescape(ends)?,
                })
            }
            "newsboat" => match rest {
                "add" => Some(Action::Newsboat { add: true }),
                "remove" => Some(Action::Newsboat { add: false }),
                _ => None,
            },
            _ => None,
        }
    }
}

/// A rating by its name on the wire (`drains_a_little`).
pub fn rating_named(key: &str) -> Option<Rating> {
    serde_json::from_value(serde_json::Value::String(key.to_owned())).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: &str = r#"
        [draining-strong]
        weight = -1.0
        sites = ["feed.example"]
        [draining-mild]
        weight = -0.4
        sites = ["bbc.co.uk"]
        [news]
        weight = -0.4
        sites = ["paper.example"]
        [ordinary]
        weight = 0.0
        sites = ["bank.example"]
        [nourishing-mild]
        weight = 0.5
        sites = ["wiki.example"]
        [nourishing-strong]
        weight = 1.0
        sites = ["books.example"]
        [private]
        sites = ["adult.example"]
        [unlisted]
        sites = ["mail.bank.example"]
        [care]
        sites = ["crisis.example"]
    "#;

    /// Percent-decoding for the tests; the app uses GLib's.
    fn unescape(text: &str) -> Option<String> {
        Some(text.replace("%2F", "/").replace("%3A", ":"))
    }

    #[test]
    fn every_rating_is_a_list_and_back() {
        for list in List::ALL {
            assert_eq!(list_of(rating_of(Some(list))), list);
        }
        assert_eq!(rating_of(None), Rating::Unrated);
        assert_eq!(rating_named("restores_a_lot"), Some(Rating::RestoresALot));
        assert_eq!(rating_named("shiny"), None);
    }

    #[test]
    fn settings_show_how_a_site_counts_and_whose_rating_that_is() {
        let seed = Lists::seed(SEED).expect("fixture");
        let user =
            "[nourishing-strong]\nsites = [\"bbc.co.uk\"]\n[removed]\nsites = [\"feed.example\"]";
        let lists = seed.with_user(user).expect("applies");
        let yours: HashMap<String, Option<List>> = crate::reputation::user_entries(user)
            .expect("parses")
            .into_iter()
            .collect();

        let bbc = site_rating(&lists, &seed, &yours, "bbc.co.uk");
        assert_eq!(bbc.rating, Rating::RestoresALot);
        assert_eq!(bbc.matched, "bbc.co.uk");
        assert_eq!(bbc.yours, Some(Rating::RestoresALot));
        assert_eq!(bbc.seed, Some(Rating::DrainsALittle));

        let part = site_rating(&lists, &seed, &yours, "news.bbc.co.uk");
        assert_eq!(
            (part.rating, part.matched.as_str()),
            (Rating::RestoresALot, "bbc.co.uk")
        );
        assert_eq!((part.yours, part.seed), (None, None));

        let removed = site_rating(&lists, &seed, &yours, "feed.example");
        assert_eq!(removed.rating, Rating::Unrated);
        assert_eq!(removed.matched, "");
        assert_eq!(removed.yours, Some(Rating::Unrated));
        assert_eq!(removed.seed, Some(Rating::DrainsALot));

        let carve_out = site_rating(&lists, &seed, &yours, "mail.bank.example");
        assert_eq!(
            (carve_out.rating, carve_out.seed),
            (Rating::Unrated, Some(Rating::Unrated))
        );
    }

    #[test]
    fn settings_links_say_what_they_do() {
        assert_eq!(
            Action::parse("rate/drains_a_little/reddit.com%2Fr%2Fdiy", unescape),
            Some(Action::Rate {
                site: "reddit.com/r/diy".into(),
                rating: Some(Rating::DrainsALittle),
            })
        );
        assert_eq!(
            Action::parse("rate/glimmerwood/bbc.co.uk", unescape),
            Some(Action::Rate {
                site: "bbc.co.uk".into(),
                rating: None,
            })
        );
        assert_eq!(
            Action::parse("look-up/", unescape),
            Some(Action::LookUp(String::new()))
        );
        assert_eq!(Action::parse("ask/off", unescape), Some(Action::Ask(false)));
        assert_eq!(
            Action::parse("night/22%3A30/06%3A00", unescape),
            Some(Action::Night {
                starts: "22:30".into(),
                ends: "06:00".into(),
            })
        );
        assert_eq!(
            Action::parse("newsboat/remove", unescape),
            Some(Action::Newsboat { add: false })
        );
        for bad in [
            "rate/shiny/a.com",
            "rate/neither/",
            "ask/maybe",
            "format/disk",
            "night/22:00",
            "newsboat/",
        ] {
            assert_eq!(Action::parse(bad, unescape), None, "{bad}");
        }
    }
}

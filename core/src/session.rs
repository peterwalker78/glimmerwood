//! The tabs that were open, so a restart doesn't cost you your place.
//!
//! What comes back is a list of addresses and the titles they had, not the
//! pages themselves: a restored tab sleeps until it is asked for. A browser
//! that reloads thirty tabs on start has decided for you that you are going
//! back to all of them.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// A tab as it was left.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sleeper {
    pub url: String,
    pub title: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub tabs: Vec<Sleeper>,
    /// Which of them was in front, by position.
    #[serde(default)]
    pub selected: usize,
}

impl Session {
    pub fn load(path: &Path) -> Session {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Session::default();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        let text = serde_json::to_string(self).map_err(|err| err.to_string())?;
        std::fs::write(path, text).map_err(|err| err.to_string())
    }

    /// What to restore. Home tabs are dropped: a window always opens with
    /// Home in front anyway, and restoring five of them is clutter, not a
    /// place you were. Anything that isn't the web is dropped with them.
    pub fn worth_restoring(&self) -> Session {
        let keep: Vec<Sleeper> = self
            .tabs
            .iter()
            .filter(|tab| worth_keeping(&tab.url))
            .cloned()
            .collect();
        // Which tab was in front, once the dropped ones are out of the way.
        let selected = self
            .tabs
            .get(self.selected)
            .and_then(|was| keep.iter().position(|tab| tab.url == was.url))
            .unwrap_or(0);
        Session {
            tabs: keep,
            selected,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }
}

fn worth_keeping(url: &str) -> bool {
    !url.is_empty() && !crate::pages::is_local_page(url) && !crate::nav::host_of(url).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(url: &str) -> Sleeper {
        Sleeper {
            url: url.into(),
            title: url.into(),
        }
    }

    #[test]
    fn what_was_open_comes_back() {
        let session = Session {
            tabs: vec![tab("https://one.example/"), tab("https://two.example/")],
            selected: 1,
        };
        let back = session.worth_restoring();
        assert_eq!(back.tabs.len(), 2);
        assert_eq!(back.selected, 1);
    }

    #[test]
    fn home_tabs_are_not_a_place_you_were() {
        let session = Session {
            tabs: vec![
                tab("glimmerwood://home/"),
                tab("https://one.example/"),
                tab("glimmerwood://settings/"),
            ],
            selected: 1,
        };
        let back = session.worth_restoring();
        assert_eq!(back.tabs.len(), 1);
        assert_eq!(back.selected, 0);
    }

    #[test]
    fn the_tab_in_front_is_still_in_front_after_the_others_are_dropped() {
        let session = Session {
            tabs: vec![
                tab("glimmerwood://home/"),
                tab("https://one.example/"),
                tab("https://two.example/"),
            ],
            selected: 2,
        };
        assert_eq!(back_url(&session.worth_restoring()), "https://two.example/");
    }

    fn back_url(session: &Session) -> String {
        session.tabs[session.selected].url.clone()
    }

    #[test]
    fn a_window_of_nothing_but_home_restores_nothing() {
        let session = Session {
            tabs: vec![tab("glimmerwood://home/")],
            selected: 0,
        };
        assert!(session.worth_restoring().is_empty());
    }

    #[test]
    fn a_missing_or_broken_file_is_simply_no_session() {
        let dir = std::env::temp_dir().join("glimmerwood-session-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("nothing.json");
        let _ = std::fs::remove_file(&path);
        assert!(Session::load(&path).is_empty());
        std::fs::write(&path, "{ not json").unwrap();
        assert!(Session::load(&path).is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn what_is_written_reads_back_the_same() {
        let dir = std::env::temp_dir().join("glimmerwood-session-test");
        let path = dir.join("session.json");
        let session = Session {
            tabs: vec![tab("https://one.example/"), tab("https://two.example/")],
            selected: 1,
        };
        session.save(&path).unwrap();
        assert_eq!(Session::load(&path), session);
        let _ = std::fs::remove_file(&path);
    }
}

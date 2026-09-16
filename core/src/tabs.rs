//! The tabs a window holds, and which of them is in front.
//!
//! Nothing here knows what a webview is. A shell keeps its own engine beside
//! each tab's id, tells this what the engine reports, and asks it what the
//! tab column should show. The order, the ids, which tab takes over when one
//! closes and what the chrome is told are then the same on every platform.

use crate::nav;
use crate::protocol::{TabInfo, TabSound};

/// What a shell's engine reports about one tab.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub title: String,
    pub uri: String,
    pub loading: bool,
    pub playing: bool,
    pub muted: bool,
    /// The site icon as a `data:` URL, sent to the chrome on its own.
    pub icon: Option<String>,
    /// Restored from the last run and not loaded yet. It holds its place in
    /// the column with the title it had, and wakes when it is asked for.
    pub asleep: bool,
}

impl Facts {
    fn sound(&self) -> TabSound {
        match (self.playing, self.muted) {
            (_, true) => TabSound::Muted,
            (true, false) => TabSound::Playing,
            (false, false) => TabSound::Silent,
        }
    }

    /// What the column shows on the tab: its title, or failing that where it
    /// is, or failing that nothing at all.
    pub(crate) fn label(&self) -> String {
        if self.title.is_empty() {
            self.uri.clone()
        } else {
            self.title.clone()
        }
    }
}

#[derive(Clone, Debug)]
pub struct Tab {
    pub id: u32,
    pub facts: Facts,
}

/// The tabs, in the order they are shown.
#[derive(Debug, Default)]
pub struct Tabs {
    tabs: Vec<Tab>,
    selected: Option<u32>,
    next_id: u32,
}

impl Tabs {
    pub fn new() -> Self {
        Self::default()
    }

    /// A new tab at the end, with an id nothing else has had. `select` is
    /// false for the ones a page opens behind what someone is reading.
    pub fn open(&mut self, select: bool) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(Tab {
            id,
            facts: Facts::default(),
        });
        if select || self.selected.is_none() {
            self.selected = Some(id);
        }
        id
    }

    /// A tab restored from the last run: it holds a place in the column with
    /// what it was, and nothing is loaded until it is selected.
    pub fn open_asleep(&mut self, url: &str, title: &str) -> u32 {
        let id = self.open(false);
        self.update(id, |facts| {
            facts.uri = url.to_owned();
            facts.title = title.to_owned();
            facts.asleep = true;
        });
        id
    }

    /// True if this tab was asleep and is now awake, which is the shell's
    /// cue to load the address it has been holding.
    pub fn wake(&mut self, id: u32) -> bool {
        let asleep = self.facts(id).is_some_and(|facts| facts.asleep);
        if asleep {
            self.update(id, |facts| facts.asleep = false);
        }
        asleep
    }

    /// What to write down for next time: everything that is a page, and
    /// which one was in front.
    pub fn session(&self) -> crate::session::Session {
        let tabs: Vec<crate::session::Sleeper> = self
            .tabs
            .iter()
            .map(|tab| crate::session::Sleeper {
                url: tab.facts.uri.clone(),
                title: tab.facts.label(),
            })
            .collect();
        let selected = self
            .selected
            .and_then(|id| self.index_of(id))
            .unwrap_or(0);
        crate::session::Session { tabs, selected }.worth_restoring()
    }

    /// Closes a tab, and says which one is in front afterwards: the one that
    /// took its place, or the last if it was the last. `None` once the
    /// window holds no tabs at all.
    pub fn close(&mut self, id: u32) -> Option<u32> {
        let Some(index) = self.index_of(id) else {
            return self.selected;
        };
        self.tabs.remove(index);
        if self.selected == Some(id) {
            self.selected = self
                .tabs
                .get(index)
                .or_else(|| self.tabs.last())
                .map(|tab| tab.id);
        }
        self.selected
    }

    /// True if this changed which tab is in front.
    pub fn select(&mut self, id: u32) -> bool {
        if self.selected == Some(id) || self.index_of(id).is_none() {
            return false;
        }
        self.selected = Some(id);
        true
    }

    /// The tab `by` places along from the one in front, wrapping at both
    /// ends, for Ctrl+Tab and its opposite.
    pub fn step(&self, by: isize) -> Option<u32> {
        let index = self.selected.and_then(|id| self.index_of(id))?;
        let len = self.tabs.len() as isize;
        let next = (index as isize + by).rem_euclid(len);
        self.tabs.get(next as usize).map(|tab| tab.id)
    }

    /// The nth tab, for Alt+1 to Alt+8. There is no tab for a number past
    /// the end.
    pub fn nth(&self, n: usize) -> Option<u32> {
        self.tabs.get(n).map(|tab| tab.id)
    }

    pub fn last(&self) -> Option<u32> {
        self.tabs.last().map(|tab| tab.id)
    }

    pub fn selected(&self) -> Option<u32> {
        self.selected
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn ids(&self) -> Vec<u32> {
        self.tabs.iter().map(|tab| tab.id).collect()
    }

    pub fn facts(&self, id: u32) -> Option<&Facts> {
        self.tabs.iter().find(|tab| tab.id == id).map(|t| &t.facts)
    }

    pub fn selected_facts(&self) -> Option<&Facts> {
        self.selected.and_then(|id| self.facts(id))
    }

    /// Hands a tab's facts over to be changed, and says whether anything
    /// about it actually did. A shell that hears the same title twice
    /// shouldn't make the chrome redraw for it.
    pub fn update(&mut self, id: u32, change: impl FnOnce(&mut Facts)) -> bool {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) else {
            return false;
        };
        let before = tab.facts.clone();
        change(&mut tab.facts);
        before != tab.facts
    }

    /// The addresses of every tab but the one named, which is what the dose
    /// engine asks for: tabs nobody is looking at weigh nothing, but it
    /// still wants to know they are there.
    pub fn other_uris(&self, than: Option<u32>) -> Vec<String> {
        self.tabs
            .iter()
            .filter(|tab| Some(tab.id) != than)
            .map(|tab| tab.facts.uri.clone())
            .filter(|uri| !uri.is_empty())
            .collect()
    }

    /// What the tab column shows.
    pub fn info(&self) -> Vec<TabInfo> {
        self.tabs
            .iter()
            .map(|tab| TabInfo {
                id: tab.id,
                title: tab.facts.label(),
                host: nav::host_of(&tab.facts.uri),
                loading: tab.facts.loading,
                sound: tab.facts.sound(),
                asleep: tab.facts.asleep,
            })
            .collect()
    }

    fn index_of(&self, id: u32) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(n: usize) -> Tabs {
        let mut tabs = Tabs::new();
        for _ in 0..n {
            tabs.open(true);
        }
        tabs
    }

    #[test]
    fn the_first_tab_is_in_front_even_when_opened_behind() {
        let mut tabs = Tabs::new();
        let id = tabs.open(false);
        assert_eq!(tabs.selected(), Some(id));
    }

    #[test]
    fn a_tab_opened_behind_leaves_the_front_alone() {
        let mut tabs = with(1);
        let first = tabs.selected();
        tabs.open(false);
        assert_eq!(tabs.selected(), first);
    }

    #[test]
    fn ids_are_never_reused() {
        let mut tabs = with(3);
        tabs.close(1);
        let fresh = tabs.open(true);
        assert_eq!(fresh, 3);
        assert_eq!(tabs.ids(), vec![0, 2, 3]);
    }

    #[test]
    fn closing_the_front_tab_moves_to_the_one_that_takes_its_place() {
        let mut tabs = with(3);
        tabs.select(1);
        assert_eq!(tabs.close(1), Some(2));
    }

    #[test]
    fn closing_the_last_tab_moves_to_the_new_last() {
        let mut tabs = with(3);
        tabs.select(2);
        assert_eq!(tabs.close(2), Some(1));
    }

    #[test]
    fn closing_a_tab_behind_leaves_the_front_where_it_was() {
        let mut tabs = with(3);
        tabs.select(2);
        assert_eq!(tabs.close(0), Some(2));
    }

    #[test]
    fn closing_the_only_tab_leaves_nothing_in_front() {
        let mut tabs = with(1);
        assert_eq!(tabs.close(0), None);
        assert!(tabs.is_empty());
    }

    #[test]
    fn closing_a_tab_that_isnt_there_changes_nothing() {
        let mut tabs = with(2);
        assert_eq!(tabs.close(9), tabs.selected());
        assert_eq!(tabs.len(), 2);
    }

    #[test]
    fn stepping_wraps_at_both_ends() {
        let mut tabs = with(3);
        tabs.select(2);
        assert_eq!(tabs.step(1), Some(0));
        tabs.select(0);
        assert_eq!(tabs.step(-1), Some(2));
    }

    #[test]
    fn there_is_no_nth_tab_past_the_end() {
        let tabs = with(2);
        assert_eq!(tabs.nth(1), Some(1));
        assert_eq!(tabs.nth(7), None);
    }

    #[test]
    fn selecting_the_tab_already_in_front_is_not_a_change() {
        let mut tabs = with(2);
        tabs.select(1);
        assert!(!tabs.select(1));
        assert!(tabs.select(0));
    }

    #[test]
    fn an_update_that_changes_nothing_says_so() {
        let mut tabs = with(1);
        assert!(tabs.update(0, |facts| facts.title = "Home".into()));
        assert!(!tabs.update(0, |facts| facts.title = "Home".into()));
    }

    #[test]
    fn the_column_shows_where_a_tab_is_until_it_has_a_title() {
        let mut tabs = with(1);
        tabs.update(0, |facts| {
            facts.uri = "https://www.example.org/a".into();
        });
        let info = tabs.info();
        assert_eq!(info[0].title, "https://www.example.org/a");
        assert_eq!(info[0].host, "example.org");

        tabs.update(0, |facts| facts.title = "An example".into());
        assert_eq!(tabs.info()[0].title, "An example");
    }

    #[test]
    fn a_restored_tab_sleeps_until_it_is_asked_for() {
        let mut tabs = Tabs::new();
        let id = tabs.open_asleep("https://example.org/a", "An example");
        assert!(tabs.facts(id).unwrap().asleep);
        assert!(tabs.info()[0].asleep);
        assert!(tabs.wake(id));
        assert!(!tabs.wake(id), "waking twice is not a second wake");
        assert!(!tabs.facts(id).unwrap().asleep);
    }

    #[test]
    fn the_session_keeps_the_pages_and_which_was_in_front() {
        let mut tabs = Tabs::new();
        let home = tabs.open(true);
        tabs.update(home, |facts| facts.uri = "glimmerwood://home/".into());
        let one = tabs.open(true);
        tabs.update(one, |facts| {
            facts.uri = "https://one.example/".into();
            facts.title = "One".into();
        });
        let session = tabs.session();
        assert_eq!(session.tabs.len(), 1, "Home is not a place you were");
        assert_eq!(session.tabs[0].title, "One");
        assert_eq!(session.selected, 0);
    }

    #[test]
    fn a_muted_tab_reads_as_muted_whether_or_not_it_is_playing() {
        let mut tabs = with(1);
        tabs.update(0, |facts| {
            facts.playing = true;
            facts.muted = true;
        });
        assert_eq!(tabs.info()[0].sound, TabSound::Muted);
    }

    #[test]
    fn the_other_tabs_are_every_tab_but_the_one_named() {
        let mut tabs = with(3);
        for id in 0..3 {
            tabs.update(id, |facts| facts.uri = format!("https://{id}.example/"));
        }
        assert_eq!(
            tabs.other_uris(Some(1)),
            vec!["https://0.example/", "https://2.example/"]
        );
    }

    #[test]
    fn a_tab_with_nowhere_to_be_is_not_one_of_the_others() {
        let mut tabs = with(2);
        tabs.update(0, |facts| facts.uri = "https://example.org/".into());
        assert_eq!(tabs.other_uris(None), vec!["https://example.org/"]);
    }
}

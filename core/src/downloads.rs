//! What is being saved, and what was saved a moment ago.
//!
//! Deliberately thin. A download is a file arriving on your machine, not a
//! second inbox: the toolbar's button moves while one is running and carries
//! a dot until it has been looked at, and Home lists the latest few for the
//! rest of the run. Nothing is kept between runs, because a list of
//! everything ever downloaded is a place to browse rather than a thing to use.

pub use crate::protocol::{Download, Progress};

/// A download and whether the user has looked at it since it finished,
/// which is ours to know and not something the pages need told.
#[derive(Clone, Debug, PartialEq)]
struct Kept {
    download: Download,
    seen: bool,
}

#[derive(Debug, Default)]
pub struct Downloads {
    kept: Vec<Kept>,
    next_id: u32,
}

impl Downloads {
    pub fn new() -> Self {
        Self::default()
    }

    /// A download has started. The shell keeps the id to report progress.
    pub fn started(&mut self, name: &str, path: &str) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.kept.push(Kept {
            download: Download {
                id,
                name: name.to_owned(),
                path: path.to_owned(),
                progress: Progress::Running,
                fraction: None,
            },
            seen: false,
        });
        id
    }

    /// True when this changed anything the chrome or Home would show.
    pub fn progressed(&mut self, id: u32, fraction: Option<f64>) -> bool {
        let Some(download) = self.find(id) else {
            return false;
        };
        // A bar that moves a hundred times a second is a bar nobody reads.
        let coarse = fraction.map(|f| (f.clamp(0.0, 1.0) * 20.0).round() / 20.0);
        if download.fraction == coarse {
            return false;
        }
        download.fraction = coarse;
        true
    }

    pub fn finished(&mut self, id: u32, progress: Progress, path: Option<&str>) -> bool {
        let Some(download) = self.find(id) else {
            return false;
        };
        download.progress = progress;
        if progress == Progress::Saved {
            download.fraction = Some(1.0);
        }
        if let Some(path) = path {
            download.path = path.to_owned();
        }
        true
    }

    /// The user asked to see it: where it is. It stays on Home, so the list
    /// doesn't move under them.
    pub fn opened(&mut self, id: u32) -> Option<String> {
        self.find_kept(id).map(|kept| {
            kept.seen = true;
            kept.download.path.clone()
        })
    }

    /// The user went to look at the downloads: none of the finished ones are
    /// new any more.
    pub fn seen(&mut self) {
        for kept in &mut self.kept {
            if kept.download.progress != Progress::Running {
                kept.seen = true;
            }
        }
    }

    /// How many have finished since the user last looked: the button's dot.
    pub fn waiting(&self) -> usize {
        self.kept
            .iter()
            .filter(|kept| !kept.seen && kept.download.progress != Progress::Running)
            .count()
    }

    /// How many are still arriving: the toolbar's mark.
    pub fn running(&self) -> usize {
        self.kept
            .iter()
            .filter(|kept| kept.download.progress == Progress::Running)
            .count()
    }

    /// What Home shows: newest first, and never a long list.
    pub fn showing(&self) -> Vec<Download> {
        let mut showing: Vec<Download> =
            self.kept.iter().map(|kept| kept.download.clone()).collect();
        showing.reverse();
        showing.truncate(SHOWN);
        showing
    }

    /// Forget the ones that have finished, leaving anything still arriving.
    pub fn forget_finished(&mut self) {
        self.kept
            .retain(|kept| kept.download.progress == Progress::Running);
    }

    fn find(&mut self, id: u32) -> Option<&mut Download> {
        self.find_kept(id).map(|kept| &mut kept.download)
    }

    fn find_kept(&mut self, id: u32) -> Option<&mut Kept> {
        self.kept.iter_mut().find(|kept| kept.download.id == id)
    }
}

/// Home shows this many at most. More than a handful is a manager, which is
/// what this is not.
const SHOWN: usize = 4;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_download_is_running_until_it_is_not() {
        let mut downloads = Downloads::new();
        let id = downloads.started("moss.pdf", "/home/x/Downloads/moss.pdf");
        assert_eq!(downloads.running(), 1);
        downloads.finished(id, Progress::Saved, None);
        assert_eq!(downloads.running(), 0);
        assert_eq!(downloads.showing().len(), 1);
    }

    #[test]
    fn showing_it_keeps_it_on_home_but_clears_the_dot() {
        let mut downloads = Downloads::new();
        let id = downloads.started("moss.pdf", "/tmp/moss.pdf");
        assert_eq!(downloads.waiting(), 0, "still arriving");
        downloads.finished(id, Progress::Saved, None);
        assert_eq!(downloads.waiting(), 1);
        assert_eq!(downloads.opened(id).as_deref(), Some("/tmp/moss.pdf"));
        assert_eq!(downloads.showing().len(), 1);
        assert_eq!(downloads.waiting(), 0);
    }

    #[test]
    fn looking_at_the_downloads_clears_the_dot_but_not_what_is_arriving() {
        let mut downloads = Downloads::new();
        let done = downloads.started("moss.pdf", "/tmp/moss.pdf");
        downloads.finished(done, Progress::Failed, None);
        let running = downloads.started("fern.pdf", "/tmp/fern.pdf");
        downloads.seen();
        assert_eq!(downloads.waiting(), 0);
        downloads.finished(running, Progress::Saved, None);
        assert_eq!(downloads.waiting(), 1);
    }

    #[test]
    fn progress_only_reports_when_it_has_visibly_moved() {
        let mut downloads = Downloads::new();
        let id = downloads.started("big.iso", "/tmp/big.iso");
        assert!(downloads.progressed(id, Some(0.10)));
        assert!(!downloads.progressed(id, Some(0.11)));
        assert!(downloads.progressed(id, Some(0.20)));
    }

    #[test]
    fn home_shows_the_newest_few_newest_first() {
        let mut downloads = Downloads::new();
        for n in 0..6 {
            let id = downloads.started(&format!("{n}.txt"), "/tmp");
            downloads.finished(id, Progress::Saved, None);
        }
        let showing = downloads.showing();
        assert_eq!(showing.len(), 4);
        assert_eq!(showing[0].name, "5.txt");
    }

    #[test]
    fn a_failure_is_still_worth_saying() {
        let mut downloads = Downloads::new();
        let id = downloads.started("gone.zip", "/tmp/gone.zip");
        downloads.finished(id, Progress::Failed, None);
        assert_eq!(downloads.showing()[0].progress, Progress::Failed);
        assert_eq!(downloads.running(), 0);
    }

    #[test]
    fn forgetting_the_finished_leaves_what_is_still_arriving() {
        let mut downloads = Downloads::new();
        let done = downloads.started("done.txt", "/tmp");
        let going = downloads.started("going.txt", "/tmp");
        downloads.finished(done, Progress::Saved, None);
        downloads.forget_finished();
        let showing = downloads.showing();
        assert_eq!(showing.len(), 1);
        assert_eq!(showing[0].id, going);
    }

    #[test]
    fn nothing_is_reported_for_a_download_that_isnt_there() {
        let mut downloads = Downloads::new();
        assert!(!downloads.progressed(9, Some(0.5)));
        assert!(!downloads.finished(9, Progress::Saved, None));
        assert_eq!(downloads.opened(9), None);
    }
}

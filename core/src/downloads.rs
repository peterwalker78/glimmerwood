//! What is being saved, and what was saved a moment ago.
//!
//! Deliberately thin. A download is a file arriving on your machine, not a
//! second inbox: there is a quiet mark in the toolbar while one is running
//! and a line on Home until it has been opened, and then it is gone from
//! here. Nothing is kept between runs, because a list of everything ever
//! downloaded is a place to browse rather than a thing to use.

pub use crate::protocol::{Download, Progress};

/// A download and whether it has been opened, which is ours to know and not
/// something the pages need told.
#[derive(Clone, Debug, PartialEq)]
struct Kept {
    download: Download,
    opened: bool,
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
            opened: false,
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

    /// The user opened it, so Home has nothing left to say about it.
    pub fn opened(&mut self, id: u32) -> Option<String> {
        let path = self.find_kept(id).map(|kept| {
            kept.opened = true;
            kept.download.path.clone()
        })?;
        self.kept.retain(|kept| !kept.opened);
        Some(path)
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
        let mut showing: Vec<Download> = self
            .kept
            .iter()
            .filter(|kept| !kept.opened)
            .map(|kept| kept.download.clone())
            .collect();
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
    fn opening_it_takes_it_off_home() {
        let mut downloads = Downloads::new();
        let id = downloads.started("moss.pdf", "/tmp/moss.pdf");
        downloads.finished(id, Progress::Saved, None);
        assert_eq!(downloads.opened(id).as_deref(), Some("/tmp/moss.pdf"));
        assert!(downloads.showing().is_empty());
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

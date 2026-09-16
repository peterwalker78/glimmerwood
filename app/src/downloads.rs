//! Files arriving from the web.
//!
//! A download is a file landing in the user's Downloads folder, not a second
//! inbox: while one is arriving there is a mark in the toolbar, and a line on
//! Home until it has been opened. Both of those are the companion's to show.
//! All that happens here is saying what WebKit is doing with the file.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk::glib;

use glimmerwood_core::companion::Companion;
use glimmerwood_core::downloads::Progress;

/// Follow every file the pages ask to save.
pub fn watch(companion: &Rc<Companion>) {
    let Some(session) = webkit::NetworkSession::default() else {
        eprintln!("glimmerwood: no network session, so nothing can be saved");
        return;
    };
    let weak = Rc::downgrade(companion);
    session.connect_download_started(move |_, download| {
        if let Some(companion) = weak.upgrade() {
            follow(&companion, download);
        }
    });
}

/// Give the file somewhere to land and report it on its way.
fn follow(companion: &Rc<Companion>, download: &webkit::Download) {
    // What the companion calls this file, once it has a destination.
    let id: Rc<Cell<Option<u32>>> = Rc::default();
    // A failure is followed by "finished", which then has nothing to add.
    let ended = Rc::new(Cell::new(false));

    let weak = Rc::downgrade(companion);
    let deciding = Rc::clone(&id);
    download.connect_decide_destination(move |download, suggested| {
        let Some(companion) = weak.upgrade() else {
            return false;
        };
        if deciding.get().is_some() {
            return false;
        }
        let dir = downloads_dir();
        if let Err(err) = std::fs::create_dir_all(&dir) {
            eprintln!("glimmerwood: couldn't make {}: {err}", dir.display());
            return false;
        }
        let path = free_path(&dir, suggested);
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let path = path.to_string_lossy().into_owned();
        download.set_destination(&path);
        deciding.set(Some(companion.download_started(&name, &path)));
        true
    });

    let weak = Rc::downgrade(companion);
    let arriving = Rc::clone(&id);
    download.connect_received_data(move |download, _| {
        if let (Some(companion), Some(id)) = (weak.upgrade(), arriving.get()) {
            companion.download_progressed(id, fraction(download));
        }
    });

    let weak = Rc::downgrade(companion);
    let saved = Rc::clone(&id);
    let saved_ended = Rc::clone(&ended);
    download.connect_finished(move |download| {
        if saved_ended.replace(true) {
            return;
        }
        if let (Some(companion), Some(id)) = (weak.upgrade(), saved.get()) {
            let path = download.destination();
            companion.download_finished(id, Progress::Saved, path.as_deref());
        }
    });

    let weak = Rc::downgrade(companion);
    let lost = Rc::clone(&id);
    let lost_ended = Rc::clone(&ended);
    download.connect_failed(move |_, error| {
        if lost_ended.replace(true) {
            return;
        }
        // Stopped is the user's own doing; anything else went wrong.
        let progress = if error.matches(webkit::DownloadError::CancelledByUser) {
            Progress::Stopped
        } else {
            eprintln!("glimmerwood: a file didn't arrive: {error}");
            Progress::Failed
        };
        if let (Some(companion), Some(id)) = (weak.upgrade(), lost.get()) {
            companion.download_finished(id, progress, None);
        }
    });
}

/// How far along, when the size is known. Nobody can draw a bar for a file
/// whose length was never declared.
fn fraction(download: &webkit::Download) -> Option<f64> {
    download
        .response()
        .filter(|response| response.content_length() > 0)
        .map(|_| download.estimated_progress())
}

/// Where files land, falling back to the home folder on a machine with no
/// Downloads folder at all.
fn downloads_dir() -> PathBuf {
    glib::user_special_dir(glib::UserDirectory::Downloads).unwrap_or_else(glib::home_dir)
}

/// A name nothing is using yet: `moss.pdf`, then `moss (2).pdf`, and so on.
/// A file already there is somebody's, and is never written over.
fn free_path(dir: &Path, suggested: &str) -> PathBuf {
    let name = file_name(suggested);
    let path = dir.join(&name);
    if !path.exists() {
        return path;
    }
    let name = Path::new(&name);
    let stem = name
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = name
        .extension()
        .map(|ext| format!(".{}", ext.to_string_lossy()))
        .unwrap_or_default();
    (2..100)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|path| !path.exists())
        .unwrap_or_else(|| dir.join(format!("{stem} ({}){ext}", glib::random_int())))
}

/// The name a page suggested, as a name and nothing else: what it suggests
/// doesn't get to choose the folder.
fn file_name(suggested: &str) -> String {
    let name: String = suggested
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .trim()
        .chars()
        .filter(|c| *c != '\0')
        .collect();
    if name.is_empty() || name == "." || name == ".." {
        "download".to_owned()
    } else {
        name
    }
}

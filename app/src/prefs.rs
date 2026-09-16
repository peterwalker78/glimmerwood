//! Small things about the window the user set by hand and expects kept: for
//! now, how wide they dragged the tab column.
//!
//! Stored as a key file beside the reputation overrides. Nothing here says
//! anything about browsing.

use std::path::PathBuf;

use gtk::glib;

const GROUP: &str = "window";
const TAB_COLUMN_WIDTH: &str = "tab-column-width";

// The width the column starts at and may be dragged between is the core's,
// so every platform agrees about it.
pub use glimmerwood_core::tabs::{
    COLUMN_DEFAULT as DEFAULT_TAB_COLUMN_WIDTH, COLUMN_MAX as MAX_TAB_COLUMN_WIDTH,
    COLUMN_MIN as MIN_TAB_COLUMN_WIDTH,
};

fn path() -> PathBuf {
    glib::user_config_dir()
        .join("glimmerwood")
        .join("window.ini")
}

/// Where the per-site drawing sizes are kept: beside the reputation
/// overrides, in the same plain form, so it can be edited by hand.
pub fn zooms_path() -> PathBuf {
    glib::user_config_dir()
        .join("glimmerwood")
        .join("zoom.toml")
}

pub fn tab_column_width() -> i32 {
    let file = glib::KeyFile::new();
    file.load_from_file(path(), glib::KeyFileFlags::NONE)
        .and_then(|()| file.integer(GROUP, TAB_COLUMN_WIDTH))
        .unwrap_or(DEFAULT_TAB_COLUMN_WIDTH)
        .clamp(MIN_TAB_COLUMN_WIDTH, MAX_TAB_COLUMN_WIDTH)
}

pub fn set_tab_column_width(width: i32) {
    let path = path();
    let file = glib::KeyFile::new();
    // Keep anything else a later version put there.
    let _ = file.load_from_file(&path, glib::KeyFileFlags::KEEP_COMMENTS);
    file.set_integer(GROUP, TAB_COLUMN_WIDTH, width);
    let saved = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .map_err(|err| err.to_string())
        .and_then(|()| file.save_to_file(&path).map_err(|err| err.to_string()));
    if let Err(err) = saved {
        eprintln!("glimmerwood: couldn't remember the tab column width: {err}");
    }
}

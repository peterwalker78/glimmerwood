//! The part of Glimmerwood that has no platform: the dose model, the lists,
//! the garden, the diary, the protocol the chrome speaks, and the files they
//! live in. No window, no widgets, no web engine, and no dependency that
//! isn't portable — so the same core runs behind GTK on Linux, WKWebView on
//! macOS and WebView2 on Windows.
//!
//! What it can't know, it is handed: the clock's offset from UTC, and the
//! paths its files live at.

pub mod asking;
pub mod attention;
pub mod bookmarks;
pub mod canvas;
pub mod companion;
pub mod diary;
pub mod dose;
pub mod downloads;
pub mod failure;
pub mod feel_lab;
pub mod find;
pub mod history;
pub mod home;
pub mod host;
pub mod look;
pub mod nav;
pub mod oklab;
pub mod pages;
pub mod places;
pub mod protocol;
pub mod ratings;
pub mod reputation;
pub mod session;
pub mod settings;
pub mod store;
pub mod sun;
pub mod svg;
pub mod tabs;
pub mod wisp;
pub mod zoom;

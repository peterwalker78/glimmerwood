//! Glimmerwood on Windows.
//!
//! The same core, the same chrome and the same wisp; a different window
//! around them. Windows has no WebKit of any kind, so the engine here is
//! WebView2 — the Edge runtime the system already has. Nothing is bundled:
//! a machine without it is told so rather than shipped one.

// A browser is not a console program. Without this the binary is one, and
// opening it hangs a console window beside the browser for as long as it
// runs — and swallows anything it had to say when it doesn't.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod files {
    include!(concat!(env!("OUT_DIR"), "/files.rs"));
}

#[cfg(windows)]
mod finding;

#[cfg(windows)]
mod host;

#[cfg(windows)]
mod nook;

#[cfg(windows)]
mod shell;

#[cfg(windows)]
fn main() {
    // Nothing here is printed anywhere anyone can read, so whatever ends the
    // program says so on screen: a failure to start and a panic alike would
    // otherwise be a window that closes before it has drawn anything.
    std::panic::set_hook(Box::new(|panic| {
        shell::complain(None, &format!("Glimmerwood has stopped: {panic}"));
    }));
    if let Err(why) = shell::run() {
        shell::complain(None, &format!("Glimmerwood couldn't start: {why}"));
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!(
        "glimmerwood-windows is the Windows shell. On this machine, \
         build the GTK app in app/ instead."
    );
}

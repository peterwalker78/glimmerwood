//! Glimmerwood on Windows.
//!
//! The same core, the same chrome and the same wisp; a different window
//! around them. Windows has no WebKit of any kind, so the engine here is
//! WebView2 — the Edge runtime the system already has. Nothing is bundled:
//! a machine without it is told so rather than shipped one.

#[cfg(windows)]
mod files {
    include!(concat!(env!("OUT_DIR"), "/files.rs"));
}

#[cfg(windows)]
mod shell;

#[cfg(windows)]
fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    shell::run()
}

#[cfg(not(windows))]
fn main() {
    eprintln!(
        "glimmerwood-windows is the Windows shell. On this machine, \
         build the GTK app in app/ instead."
    );
}

//! Glimmerwood on macOS.
//!
//! The same core, the same chrome and the same wisp. The engine here is
//! WebKit, which is the same family as the WebKitGTK the Linux build uses, so
//! pages are rendered by the same lineage on both.

#[cfg(target_os = "macos")]
mod files {
    include!(concat!(env!("OUT_DIR"), "/files.rs"));
}

#[cfg(target_os = "macos")]
mod shell;

#[cfg(target_os = "macos")]
fn main() {
    shell::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!(
        "glimmerwood-macos is the macOS shell. On this machine, \
         build the GTK app in app/ instead."
    );
}

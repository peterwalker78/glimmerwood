//! Where this build reads the wall clock's offset from UTC. GLib knows the
//! zone the user is in; every platform has its own way of saying so.

use glimmerwood_core::attention;
use glimmerwood_core::dose::Moment;
use gtk::glib;

pub fn now() -> Moment {
    let offset = glib::DateTime::now_local()
        .map(|t| t.utc_offset().as_seconds() as i32)
        .unwrap_or(0);
    attention::now(offset)
}

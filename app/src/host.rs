//! What the companion is given to work with here: GTK's windows, GLib's idea
//! of where files live, and a timer.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::{gio, glib, prelude::*};

use glimmerwood_core::companion::{self, Companion};
use glimmerwood_core::dose::Moment;
use glimmerwood_core::host;
use glimmerwood_core::protocol::ToChrome;

use crate::clock;
use crate::window::Window;

pub struct GtkHost {
    windows: RefCell<Vec<Weak<Window>>>,
    companion: RefCell<Weak<Companion>>,
    wake: RefCell<Option<glib::SourceId>>,
    lists: RefCell<Option<gio::FileMonitor>>,
}

impl GtkHost {
    pub fn new() -> Rc<GtkHost> {
        Rc::new(GtkHost {
            windows: RefCell::new(Vec::new()),
            companion: RefCell::new(Weak::new()),
            wake: RefCell::new(None),
            lists: RefCell::new(None),
        })
    }

    /// The companion is held weakly: it holds the host, and neither should
    /// keep the other alive.
    pub fn attach(self: &Rc<Self>, companion: &Rc<Companion>) {
        *self.companion.borrow_mut() = Rc::downgrade(companion);
        self.watch_user_lists();
    }

    /// The user edits their own reputation list by hand, so changes to it are
    /// picked up without a restart.
    fn watch_user_lists(self: &Rc<Self>) {
        let path = glib::user_config_dir()
            .join("glimmerwood")
            .join("reputation.toml");
        let file = gio::File::for_path(path);
        let monitor = match file.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE) {
            Ok(monitor) => monitor,
            Err(err) => {
                eprintln!("glimmerwood: can't watch your reputation list for changes: {err}");
                return;
            }
        };
        let weak = Rc::downgrade(self);
        monitor.connect_changed(move |_, _, _, event| {
            use gio::FileMonitorEvent as E;
            if !matches!(
                event,
                E::ChangesDoneHint | E::Created | E::Deleted | E::MovedIn
            ) {
                return;
            }
            if let Some(this) = weak.upgrade() {
                this.lists_changed();
            }
        });
        *self.lists.borrow_mut() = Some(monitor);
    }

    pub fn add_window(&self, window: &Rc<Window>) {
        self.windows.borrow_mut().push(Rc::downgrade(window));
        if let Some(companion) = self.companion.borrow().upgrade() {
            companion.windows_changed();
        }
    }

    /// The user's own list file changed on disk.
    pub fn lists_changed(&self) {
        if let Some(companion) = self.companion.borrow().upgrade() {
            companion.user_lists_changed();
        }
    }
}

impl host::Host for GtkHost {
    fn windows(&self) -> Vec<Rc<dyn host::Window>> {
        let mut live = self.windows.borrow_mut();
        live.retain(|w| w.strong_count() > 0);
        live.iter()
            .filter_map(Weak::upgrade)
            .map(|w| w as Rc<dyn host::Window>)
            .collect()
    }

    fn now(&self) -> Moment {
        clock::now()
    }

    fn data_dir(&self) -> PathBuf {
        glib::user_data_dir()
    }

    fn config_dir(&self) -> PathBuf {
        glib::user_config_dir()
    }

    fn home_dir(&self) -> PathBuf {
        glib::home_dir()
    }

    fn locale_country(&self) -> Option<String> {
        let names: Vec<String> = glib::language_names()
            .iter()
            .map(|name| name.to_string())
            .collect();
        companion::country_from_locales(&names)
    }

    fn wake_in(&self, ms: u64) {
        if let Some(id) = self.wake.borrow_mut().take() {
            id.remove();
        }
        let companion = self.companion.borrow().clone();
        let id = glib::timeout_add_local_once(Duration::from_millis(ms.max(1)), move || {
            if let Some(companion) = companion.upgrade() {
                companion.refresh();
            }
        });
        *self.wake.borrow_mut() = Some(id);
    }

    fn noise(&self) -> u32 {
        glib::random_int()
    }
}

impl host::Window for Window {
    fn in_front(&self) -> bool {
        Window::in_front(self)
    }
    fn attended_uri(&self) -> String {
        Window::attended_uri(self)
    }
    fn other_tabs(&self, in_front: bool) -> Vec<String> {
        Window::other_tabs(self, in_front)
    }
    fn sound_on_screen(&self) -> bool {
        Window::sound_on_screen(self)
    }
    fn send_to_chrome(&self, message: &ToChrome) {
        Window::send_to_chrome(self, message);
    }
    fn refresh_pages(&self) {
        Window::refresh_pages(self);
    }
    fn ask(&self, site: Option<String>) {
        Window::ask(self, site);
    }
    fn care(&self, open: bool, samaritans: bool) {
        Window::care(self, open, samaritans);
    }
    fn set_title(&self, title: &str) {
        Window::set_title(self, title);
    }
}

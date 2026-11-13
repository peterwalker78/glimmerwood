//! Sites asking to use the camera, the microphone, the screen or the
//! person's location.
//!
//! The answer is the person's, asked in a quiet line in the bar that never
//! covers the page or takes the caret. A yes is remembered for the site;
//! "not now" lasts only as long as the request. Notifications are refused
//! without a word: they exist to call people back.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Weak;

use gtk::{glib, prelude::*};
use webkit::prelude::*;

use glimmerwood_core::nav;
use glimmerwood_core::permissions::Allowed;
use glimmerwood_core::protocol::{ToChrome, Want};

use crate::window::Window;

/// A window's open question: one at a time, the newest.
#[derive(Default)]
pub struct Asking {
    pending: RefCell<Option<Pending>>,
    next_id: Cell<u32>,
}

struct Pending {
    id: u32,
    uri: String,
    wants: Vec<Want>,
    request: webkit::PermissionRequest,
    view: glib::WeakRef<webkit::WebView>,
}

/// Kept beside the other per-site choices, in a form that can be edited by
/// hand.
fn path() -> PathBuf {
    glib::user_config_dir()
        .join("glimmerwood")
        .join("permissions.toml")
}

/// Answer, or ask about, everything a tab's pages request.
pub fn watch(view: &webkit::WebView, window: Weak<Window>) {
    let asking = window.clone();
    view.connect_permission_request(move |view, request| {
        if request.is::<webkit::NotificationPermissionRequest>() {
            request.deny();
            return true;
        }
        let uri = view.uri().map(|uri| uri.to_string()).unwrap_or_default();
        if request.is::<webkit::DeviceInfoPermissionRequest>() {
            // The names of the cameras and microphones, for choosing between
            // them, go only to a site already let use one.
            if Allowed::load(&path()).allows_any(&uri, &[Want::Camera, Want::Microphone]) {
                request.allow();
            } else {
                request.deny();
            }
            return true;
        }
        let Some(wants) = wants_of(request) else {
            // Anything else gets WebKit's own answer.
            return false;
        };
        match asking.upgrade() {
            Some(window) => window.asking.ask(&window, view, request, uri, wants),
            None => request.deny(),
        }
        true
    });

    // Leaving the page, or closing its tab, takes the question with it.
    let leaving = window.clone();
    view.connect_load_changed(move |view, event| {
        if event == webkit::LoadEvent::Committed
            && let Some(window) = leaving.upgrade()
        {
            window.asking.drop_for(&window, view);
        }
    });
    view.connect_destroy(move |view| {
        if let Some(window) = window.upgrade() {
            window.asking.drop_for(&window, view);
        }
    });
}

/// What a request is for, if it is one the person answers.
fn wants_of(request: &webkit::PermissionRequest) -> Option<Vec<Want>> {
    if request.is::<webkit::GeolocationPermissionRequest>() {
        return Some(vec![Want::Location]);
    }
    let media = request.downcast_ref::<webkit::UserMediaPermissionRequest>()?;
    let wants: Vec<Want> = [
        (media.is_for_video_device(), Want::Camera),
        (media.is_for_audio_device(), Want::Microphone),
        (
            webkit::functions::user_media_permission_is_for_display_device(media),
            Want::Screen,
        ),
    ]
    .into_iter()
    .filter_map(|(asked, want)| asked.then_some(want))
    .collect();
    (!wants.is_empty()).then_some(wants)
}

impl Asking {
    fn ask(
        &self,
        window: &Window,
        view: &webkit::WebView,
        request: &webkit::PermissionRequest,
        uri: String,
        wants: Vec<Want>,
    ) {
        let site = nav::host_of(&uri);
        // A page with no site of its own has nobody to name in the question.
        if site.is_empty() {
            request.deny();
            return;
        }
        if Allowed::load(&path()).allows(&uri, &wants) {
            request.allow();
            return;
        }
        let id = self.next_id.get() + 1;
        self.next_id.set(id);
        let asked = Pending {
            id,
            uri,
            wants: wants.clone(),
            request: request.clone(),
            view: view.downgrade(),
        };
        // A newer question takes the place of an older one, which is a no.
        if let Some(older) = self.pending.replace(Some(asked)) {
            older.request.deny();
        }
        window.send_to_chrome(&ToChrome::Permission {
            id,
            site: Some(site),
            wants,
        });
    }

    /// The person chose Allow or Not now.
    pub fn answer(&self, window: &Window, id: u32, allow: bool) {
        if self.pending.borrow().as_ref().is_none_or(|p| p.id != id) {
            return;
        }
        let Some(pending) = self.pending.take() else {
            return;
        };
        if allow {
            let mut allowed = Allowed::load(&path());
            allowed.allow(&pending.uri, &pending.wants);
            if let Err(err) = allowed.save(&path()) {
                eprintln!("glimmerwood: couldn't remember what a site may use: {err}");
            }
            pending.request.allow();
        } else {
            pending.request.deny();
        }
        close(window, id);
    }

    /// The page that asked has gone: its question goes too.
    fn drop_for(&self, window: &Window, view: &webkit::WebView) {
        let gone = self
            .pending
            .borrow()
            .as_ref()
            .is_some_and(|p| p.view.upgrade().is_none_or(|asker| &asker == view));
        if gone && let Some(pending) = self.pending.take() {
            pending.request.deny();
            close(window, pending.id);
        }
    }
}

fn close(window: &Window, id: u32) {
    window.send_to_chrome(&ToChrome::Permission {
        id,
        site: None,
        wants: Vec::new(),
    });
}

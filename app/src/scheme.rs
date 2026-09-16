//! The `glimmerwood://` scheme, which serves the chrome from bundled resources.
//!
//! The chrome is only served to webviews marked with [`trust`]. Tabs get Home
//! (`glimmerwood://home/`) and Settings (`glimmerwood://settings/`) and nothing
//! else. WebKit is told the scheme is
//! display-isolated, so a web page can't navigate to or embed any of it.

use std::cell::RefCell;

use gtk::{gio, glib};
use webkit::prelude::*;

use glimmerwood_core::pages::{self, is_local_page, mime_type};

const SCHEME: &str = "glimmerwood";
const RESOURCE_ROOT: &str = "/io/github/peterwalker78/Glimmerwood/";

thread_local! {
    static TRUSTED: RefCell<Vec<glib::WeakRef<webkit::WebView>>> = const { RefCell::new(Vec::new()) };
}

pub fn register() {
    let context = webkit::WebContext::default().expect("WebKit has a default context");
    context.register_uri_scheme(SCHEME, serve);
    if let Some(security) = context.security_manager() {
        security.register_uri_scheme_as_secure(SCHEME);
        security.register_uri_scheme_as_display_isolated(SCHEME);
    }
}

pub fn trust(view: &webkit::WebView) {
    TRUSTED.with_borrow_mut(|trusted| {
        trusted.retain(|weak| weak.upgrade().is_some());
        trusted.push(view.downgrade());
    });
}

fn is_trusted(view: &webkit::WebView) -> bool {
    TRUSTED.with_borrow(|trusted| {
        trusted
            .iter()
            .any(|weak| weak.upgrade().as_ref() == Some(view))
    })
}

fn serve(request: &webkit::URISchemeRequest) {
    let uri = request.uri().map(|u| u.to_string()).unwrap_or_default();
    if !is_local_page(&uri) && !request.web_view().is_some_and(|view| is_trusted(&view)) {
        refuse(
            request,
            "glimmerwood:// is only served to Glimmerwood's own interface",
        );
        return;
    }
    let Some(path) = pages::path_of(&uri).map(|path| format!("{RESOURCE_ROOT}{path}")) else {
        refuse(request, "not a glimmerwood:// resource");
        return;
    };
    match gio::resources_lookup_data(&path, gio::ResourceLookupFlags::NONE) {
        Ok(bytes) => {
            let len = bytes.len() as i64;
            let stream = gio::MemoryInputStream::from_bytes(&bytes);
            request.finish(&stream, len, Some(mime_type(&path)));
        }
        Err(_) => refuse(request, "no such glimmerwood:// resource"),
    }
}

fn refuse(request: &webkit::URISchemeRequest, why: &str) {
    let mut error = glib::Error::new(gio::IOErrorEnum::PermissionDenied, why);
    request.finish_error(&mut error);
}

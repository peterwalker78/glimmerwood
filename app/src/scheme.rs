//! The `glimmerwood://` scheme, which serves the chrome from bundled resources.
//!
//! The chrome is only served to webviews marked with [`trust`]. Tabs get Home
//! (`glimmerwood://home/`) and Settings (`glimmerwood://settings/`) and nothing
//! else. WebKit is told the scheme is
//! display-isolated, so a web page can't navigate to or embed any of it.

use std::cell::RefCell;

use gtk::{gio, glib};
use webkit::prelude::*;

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
    let Some(path) = resource_path(&uri) else {
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

/// Home's own files, which any tab may load.
pub fn is_home(uri: &str) -> bool {
    is_page(uri, "home")
}

/// The Settings page's own files, which any tab may load.
pub fn is_settings(uri: &str) -> bool {
    is_page(uri, "settings")
}

pub fn is_local_page(uri: &str) -> bool {
    is_home(uri) || is_settings(uri)
}

fn is_page(uri: &str, page: &str) -> bool {
    uri.strip_prefix("glimmerwood://")
        .and_then(|rest| rest.strip_prefix(page))
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(['/', '?', '#']))
}

/// `glimmerwood://chrome/index.html` → `/io/github/peterwalker78/Glimmerwood/chrome/index.html`
fn resource_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("glimmerwood://")?;
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let rest = match rest {
        "home" | "home/" => "home/index.html",
        "settings" | "settings/" => "settings/index.html",
        _ => rest,
    };
    let clean = rest
        .split('/')
        .all(|segment| !segment.is_empty() && segment != "." && segment != "..");
    clean.then(|| format!("{RESOURCE_ROOT}{rest}"))
}

fn mime_type(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, ext)| ext) {
        Some("html") => "text/html",
        Some("js") => "text/javascript",
        Some("css") => "text/css",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_paths_stay_inside_the_bundle() {
        assert_eq!(
            resource_path("glimmerwood://chrome/index.html").as_deref(),
            Some("/io/github/peterwalker78/Glimmerwood/chrome/index.html")
        );
        assert_eq!(
            resource_path("glimmerwood://chrome/chrome.js?v=1").as_deref(),
            Some("/io/github/peterwalker78/Glimmerwood/chrome/chrome.js")
        );
        assert_eq!(resource_path("glimmerwood://chrome/../../etc"), None);
        assert_eq!(resource_path("glimmerwood://chrome//index.html"), None);
        assert_eq!(resource_path("https://chrome/index.html"), None);
        assert_eq!(
            resource_path("glimmerwood://home/").as_deref(),
            Some("/io/github/peterwalker78/Glimmerwood/home/index.html")
        );
        assert!(is_home("glimmerwood://home/") && is_home("glimmerwood://home/home.js"));
        assert!(
            !is_home("glimmerwood://homeless/x") && !is_home("glimmerwood://chrome/toolbar.html")
        );
        assert_eq!(
            resource_path("glimmerwood://settings/#ratings").as_deref(),
            Some("/io/github/peterwalker78/Glimmerwood/settings/index.html")
        );
        assert!(
            is_settings("glimmerwood://settings/settings.js")
                && is_local_page("glimmerwood://settings")
        );
        assert!(
            !is_local_page("glimmerwood://settingsx/") && !is_local_page("glimmerwood://chrome/")
        );
    }
}

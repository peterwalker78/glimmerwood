//! What `glimmerwood://` means.
//!
//! Glimmerwood's own interface — the toolbar, the tab column, Home and
//! Settings — is served from files carried inside the app rather than off a
//! disk or a network. Which address means which file, and what each file is,
//! is the same wherever it runs; how those files are carried and handed to a
//! webview is the platform's business.

pub const SCHEME: &str = "glimmerwood://";

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
    uri.strip_prefix(SCHEME)
        .and_then(|rest| rest.strip_prefix(page))
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(['/', '?', '#']))
}

/// `glimmerwood://chrome/index.html` → `chrome/index.html`, and `None` for
/// anything trying to climb out of the bundle.
pub fn path_of(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix(SCHEME)?;
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let rest = match rest {
        "home" | "home/" => "home/index.html",
        "settings" | "settings/" => "settings/index.html",
        _ => rest,
    };
    let clean = rest
        .split('/')
        .all(|segment| !segment.is_empty() && segment != "." && segment != "..");
    clean.then(|| rest.to_string())
}

/// One of Glimmerwood's own pages, which any tab may open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Local {
    Home,
    Settings,
}

impl Local {
    /// Which page an address is, if it is one of ours.
    pub fn of(uri: &str) -> Option<Local> {
        if is_home(uri) {
            Some(Local::Home)
        } else if is_settings(uri) {
            Some(Local::Settings)
        } else {
            None
        }
    }

    fn name(self) -> &'static str {
        match self {
            Local::Home => "home",
            Local::Settings => "settings",
        }
    }

    fn global(self) -> &'static str {
        match self {
            Local::Home => "wispHome",
            Local::Settings => "wispSettings",
        }
    }

    /// The script that hands the page what it shows. It checks where it has
    /// landed first, because a page can go somewhere else between being asked
    /// for and answering.
    pub fn hand_over(self, json: &str) -> String {
        let (page, global) = (self.name(), self.global());
        format!(
            "if (location.href.startsWith('glimmerwood://{page}/')) \
             {{ window.{global}Data = {json}; window.{global}?.show(window.{global}Data); }}"
        )
    }
}

pub fn mime_type(path: &str) -> &'static str {
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
    fn addresses_stay_inside_the_bundle() {
        assert_eq!(
            path_of("glimmerwood://chrome/index.html").as_deref(),
            Some("chrome/index.html")
        );
        assert_eq!(
            path_of("glimmerwood://chrome/chrome.js?v=1").as_deref(),
            Some("chrome/chrome.js")
        );
        assert_eq!(path_of("glimmerwood://chrome/../../etc"), None);
        assert_eq!(path_of("glimmerwood://chrome//index.html"), None);
        assert_eq!(path_of("https://chrome/index.html"), None);
        assert_eq!(
            path_of("glimmerwood://home/").as_deref(),
            Some("home/index.html")
        );
        assert_eq!(
            path_of("glimmerwood://settings/#ratings").as_deref(),
            Some("settings/index.html")
        );
    }

    #[test]
    fn a_tab_may_load_home_and_settings_and_nothing_else() {
        assert!(is_home("glimmerwood://home/") && is_home("glimmerwood://home/home.js"));
        assert!(
            !is_home("glimmerwood://homeless/x") && !is_home("glimmerwood://chrome/toolbar.html")
        );
        assert!(
            is_settings("glimmerwood://settings/settings.js")
                && is_local_page("glimmerwood://settings")
        );
        assert!(
            !is_local_page("glimmerwood://settingsx/") && !is_local_page("glimmerwood://chrome/")
        );
    }

    #[test]
    fn files_say_what_they_are() {
        assert_eq!(mime_type("home/index.html"), "text/html");
        assert_eq!(mime_type("home/home.js"), "text/javascript");
        assert_eq!(mime_type("home/home.css"), "text/css");
        assert_eq!(mime_type("home/wisp.svg"), "image/svg+xml");
        assert_eq!(mime_type("home/mystery"), "application/octet-stream");
    }
}

//! Tab webviews.
//!
//! Each tab gets its own content manager with nothing registered in it: no
//! bridge to the core at all. What the core needs to know about a tab
//! (address, title, loading, whether it's playing sound) WebKit reports
//! itself.

use gtk::gdk;
use webkit::prelude::*;

use glimmerwood_core::pages;

thread_local! {
    /// One set of settings for every tab, so they all behave alike.
    static SETTINGS: webkit::Settings = shared_settings();
}

fn shared_settings() -> webkit::Settings {
    let settings = webkit::Settings::new();
    // No reporting back to trackers when a link is followed (`<a ping>`).
    settings.set_enable_hyperlink_auditing(false);
    // Calls in the browser: the camera and microphone are still only used
    // once the person says so.
    settings.set_enable_webrtc(true);
    settings.set_enable_media_stream(true);
    // Two fingers swiped sideways go back and forward.
    settings.set_property("enable-back-forward-navigation-gestures", true);
    settings
}

/// A tab's webview. `related` is the view that asked for it with
/// `window.open`, which must share its web process for the opener to work.
pub fn new_view(related: Option<&webkit::WebView>) -> webkit::WebView {
    let content = webkit::UserContentManager::new();
    let builder = SETTINGS.with(|settings| {
        webkit::WebView::builder()
            .user_content_manager(&content)
            .settings(settings)
    });
    let builder = match related {
        Some(related) => builder.related_view(related),
        None => builder,
    };
    let view = builder.build();
    view.set_vexpand(true);
    view.set_hexpand(true);
    match_background(&view);
    view
}

/// WebKit paints white behind a page until it draws, which flashes on Home
/// and Settings in the dark. Glimmerwood's own pages get the chrome's paper
/// behind them instead; every website still gets white.
fn match_background(view: &webkit::WebView) {
    // A new tab is nearly always Home.
    view.set_background_color(&paper());
    view.connect_load_changed(|view, event| {
        let own = view.uri().is_some_and(|uri| uri.starts_with(pages::SCHEME));
        match event {
            webkit::LoadEvent::Started if own => view.set_background_color(&paper()),
            webkit::LoadEvent::Committed => {
                view.set_background_color(&if own { paper() } else { gdk::RGBA::WHITE })
            }
            _ => {}
        }
    });
}

fn paper() -> gdk::RGBA {
    paper_for(dark())
}

/// The chrome's paper, light or dark: `--paper` in `ui/chrome/chrome.css`.
pub fn paper_for(dark: bool) -> gdk::RGBA {
    let (r, g, b) = if dark {
        (0x1d, 0x1e, 0x1c)
    } else {
        (0xec, 0xec, 0xe8)
    };
    gdk::RGBA::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0)
}

/// Whether pages are being drawn dark: WebKit takes this setting for
/// prefers-color-scheme.
pub fn dark() -> bool {
    // Deprecated in GTK 4.20, but still what WebKitGTK 2.52 follows.
    #[allow(deprecated)]
    gtk::Settings::default().is_some_and(|settings| settings.is_gtk_application_prefer_dark_theme())
}

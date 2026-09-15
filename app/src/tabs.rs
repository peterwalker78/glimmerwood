//! Tab webviews.
//!
//! Each tab gets its own content manager with nothing registered in it: no
//! bridge to the core at all. What the core needs to know about a tab
//! (address, title, loading, whether it's playing sound) WebKit reports
//! itself.

use webkit::prelude::*;

/// A tab's webview. `related` is the view that asked for it with
/// `window.open`, which must share its web process for the opener to work.
pub fn new_view(related: Option<&webkit::WebView>) -> webkit::WebView {
    let content = webkit::UserContentManager::new();
    let builder = webkit::WebView::builder().user_content_manager(&content);
    let builder = match related {
        Some(related) => builder.related_view(related),
        None => builder,
    };
    let view = builder.build();
    view.set_vexpand(true);
    view.set_hexpand(true);
    view
}

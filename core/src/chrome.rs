//! The chrome: the trusted webviews that draw the toolbar and the tab column.
//!
//! They are the only webviews with a script message handler. Tab webviews get
//! their own content managers, so `window.webkit.messageHandlers.wisp` never
//! exists for a web page.

use gtk::{gdk, gio};
use webkit::prelude::*;

use crate::protocol::{ToChrome, ToCore};
use crate::scheme;

const HANDLER: &str = "wisp";

/// The toolbar and the tab column. They share one web process, one content
/// manager and one message handler: each message says what it's about.
pub fn new_pair(on_message: impl Fn(ToCore) + 'static) -> (webkit::WebView, webkit::WebView) {
    let content = webkit::UserContentManager::new();
    content.register_script_message_handler(HANDLER, None);
    content.connect_script_message_received(Some(HANDLER), move |_, value| {
        let Some(json) = value.to_json(0) else {
            return;
        };
        match serde_json::from_str::<ToCore>(&json) {
            Ok(message) => on_message(message),
            Err(err) => eprintln!("wisp: ignoring a chrome message the core doesn't know: {err}"),
        }
    });

    let settings = webkit::Settings::new();
    settings.set_enable_developer_extras(cfg!(debug_assertions));
    settings.set_enable_write_console_messages_to_stdout(cfg!(debug_assertions));

    let toolbar = webkit::WebView::builder()
        .user_content_manager(&content)
        .settings(&settings)
        .build();
    let sidebar = webkit::WebView::builder()
        .related_view(&toolbar)
        .user_content_manager(&content)
        .settings(&settings)
        .build();
    for (view, page) in [(&toolbar, "toolbar"), (&sidebar, "sidebar")] {
        prepare(view);
        view.load_uri(&format!("wisp://chrome/{page}.html"));
    }
    (toolbar, sidebar)
}

fn prepare(view: &webkit::WebView) {
    view.set_background_color(&gdk::RGBA::TRANSPARENT);
    scheme::trust(view);

    // The chrome never leaves its own pages.
    view.connect_decide_policy(|_, decision, kind| {
        if kind != webkit::PolicyDecisionType::NavigationAction {
            return false;
        }
        let uri = decision
            .downcast_ref::<webkit::NavigationPolicyDecision>()
            .and_then(|d| d.navigation_action())
            .and_then(|action| action.request())
            .and_then(|request| request.uri());
        if uri.is_some_and(|uri| uri.starts_with("wisp://chrome/")) {
            decision.use_();
        } else {
            decision.ignore();
        }
        true
    });

    // If its process dies, start the page again; it asks for everything it
    // needs once it's ready.
    view.connect_web_process_terminated(|view, _| view.reload());
}

pub fn send(view: &webkit::WebView, message: &ToChrome) {
    let json = serde_json::to_string(message).expect("chrome messages always serialise");
    view.evaluate_javascript(
        &format!("window.wispChrome?.receive?.({json})"),
        None,
        None,
        None::<&gio::Cancellable>,
        |result| {
            if let Err(err) = result {
                eprintln!("wisp: chrome didn't take a message: {err}");
            }
        },
    );
}

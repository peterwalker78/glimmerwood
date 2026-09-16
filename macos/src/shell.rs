//! The window, and the engines inside it.
//!
//! The toolbar runs across the top and the page fills the rest, each its own
//! WKWebView. The toolbar is Glimmerwood's own chrome and is the only one
//! given a handler for `glimmerwood://`; the page below it is the web.

use std::cell::RefCell;

use glimmerwood_core::nav;
use glimmerwood_core::pages;
use glimmerwood_core::protocol::{Security, ToChrome, ToCore};
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSData, NSJSONSerialization, NSJSONWritingOptions, NSPoint, NSRect, NSSize,
    NSString, NSURL, NSURLResponse,
};
use objc2_web_kit::{
    WKScriptMessage, WKScriptMessageHandler, WKURLSchemeHandler, WKURLSchemeTask,
    WKUserContentController, WKWebView, WKWebViewConfiguration,
};

/// Before the chrome says otherwise.
const INITIAL_WIDTH: f64 = 1100.0;
const INITIAL_HEIGHT: f64 = 760.0;
/// The toolbar's height until it reports its own.
const TOOLBAR_HEIGHT: f64 = 56.0;

thread_local! {
    static SHELL: RefCell<Option<Shell>> = const { RefCell::new(None) };
}

struct Shell {
    window: Retained<NSWindow>,
    toolbar: Retained<WKWebView>,
    page: Retained<WKWebView>,
    toolbar_height: f64,
}

pub fn run() {
    let mtm = MainThreadMarker::new().expect("the shell starts on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);

    let frame = NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(INITIAL_WIDTH, INITIAL_HEIGHT),
    );
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame,
            NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable
                | NSWindowStyleMask::Resizable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    window.setTitle(&NSString::from_str("Glimmerwood"));

    let toolbar = make_webview(mtm, true);
    let page = make_webview(mtm, false);

    if let Some(content) = window.contentView() {
        content.addSubview(&toolbar);
        content.addSubview(&page);
    }

    load(&toolbar, "glimmerwood://chrome/toolbar.html");
    load(&page, "glimmerwood://home/");

    SHELL.with_borrow_mut(|held| {
        *held = Some(Shell {
            window: window.clone(),
            toolbar,
            page,
            toolbar_height: TOOLBAR_HEIGHT,
        });
    });
    lay_out();

    window.center();
    window.makeKeyAndOrderFront(None);
    app.activate();
    app.run();
}

/// A webview. Only the chrome is given a way to ask for Glimmerwood's own
/// files; the page below it is the web, and has no handler for the scheme.
fn make_webview(mtm: MainThreadMarker, is_chrome: bool) -> Retained<WKWebView> {
    let configuration = unsafe { WKWebViewConfiguration::new(mtm) };
    if is_chrome {
        let scheme: Retained<Scheme> = unsafe { msg_send![Scheme::alloc(mtm), init] };
        let scheme = ProtocolObject::from_retained(scheme);
        unsafe {
            configuration.setURLSchemeHandler_forURLScheme(
                Some(&scheme),
                &NSString::from_str("glimmerwood"),
            );
        }
        let controller = unsafe { WKUserContentController::new(mtm) };
        let listener: Retained<Listener> = unsafe { msg_send![Listener::alloc(mtm), init] };
        let listener = ProtocolObject::from_retained(listener);
        unsafe {
            controller.addScriptMessageHandler_name(&listener, &NSString::from_str("wisp"));
            configuration.setUserContentController(&controller);
        }
    }
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0));
    unsafe { WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), frame, &configuration) }
}

fn load(view: &WKWebView, uri: &str) {
    let url = NSURL::URLWithString(&NSString::from_str(uri));
    let Some(url) = url else { return };
    let request = objc2_foundation::NSURLRequest::requestWithURL(&url);
    unsafe {
        let _ = view.loadRequest(&request);
    }
}

/// The toolbar across the top, the page filling what is left. Cocoa measures
/// from the bottom left, so the toolbar's frame sits at the top of the view.
fn lay_out() {
    SHELL.with_borrow(|held| {
        let Some(shell) = held.as_ref() else { return };
        let Some(content) = shell.window.contentView() else {
            return;
        };
        let whole = content.frame();
        let split = shell.toolbar_height.min(whole.size.height);
        shell.toolbar.setFrame(NSRect::new(
            NSPoint::new(0.0, whole.size.height - split),
            NSSize::new(whole.size.width, split),
        ));
        shell.page.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(whole.size.width, whole.size.height - split),
        ));
    });
}

fn tell_the_chrome(message: &ToChrome) {
    let Ok(json) = serde_json::to_string(message) else {
        return;
    };
    SHELL.with_borrow(|held| {
        let Some(shell) = held.as_ref() else { return };
        let script = NSString::from_str(&format!("window.wispChrome?.receive?.({json})"));
        unsafe {
            shell
                .toolbar
                .evaluateJavaScript_completionHandler(&script, None);
        }
    });
}

/// What the page is showing, for the address field and the buttons.
fn push_state() {
    SHELL.with_borrow(|held| {
        let Some(shell) = held.as_ref() else { return };
        let uri = unsafe { shell.page.URL() }
            .and_then(|url| url.absoluteString())
            .map(|s| s.to_string())
            .unwrap_or_default();
        let title = unsafe { shell.page.title() }
            .map(|s| s.to_string())
            .unwrap_or_default();
        let local = pages::is_local_page(&uri);
        tell_the_chrome(&ToChrome::State {
            security: if local {
                Security::Local
            } else {
                nav::security(&uri)
            },
            uri: uri.clone(),
            title,
            loading: unsafe { shell.page.isLoading() },
            progress: 1.0,
            can_go_back: unsafe { shell.page.canGoBack() },
            can_go_forward: unsafe { shell.page.canGoForward() },
            can_bookmark: !local && !uri.is_empty(),
            bookmarked: false,
        });
    });
}

fn heard(message: ToCore) {
    SHELL.with_borrow_mut(|held| {
        let Some(shell) = held.as_mut() else { return };
        match message {
            ToCore::Navigate { input } => {
                if let Some(target) = nav::resolve(&input) {
                    load(&shell.page, &target.uri);
                }
            }
            ToCore::Back => unsafe {
                let _ = shell.page.goBack();
            },
            ToCore::Forward => unsafe {
                let _ = shell.page.goForward();
            },
            ToCore::Reload => unsafe {
                let _ = shell.page.reload();
            },
            ToCore::Stop => unsafe {
                shell.page.stopLoading();
            },
            ToCore::GoHome => load(&shell.page, "glimmerwood://home/"),
            ToCore::ToolbarLayout { height, .. } => {
                shell.toolbar_height = f64::from(height);
            }
            ToCore::Minimize => shell.window.miniaturize(None),
            ToCore::CloseWindow => shell.window.close(),
            // The rest belong to tabs, the wisp and the lists, which this
            // shell has yet to grow.
            _ => {}
        }
    });
    // Settled after the borrow above has ended.
    lay_out();
    push_state();
}

define_class!(
    /// Answers `glimmerwood://` out of the files carried in the binary.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GlimmerwoodScheme"]
    struct Scheme;

    unsafe impl NSObjectProtocol for Scheme {}

    unsafe impl WKURLSchemeHandler for Scheme {
        #[unsafe(method(webView:startURLSchemeTask:))]
        fn start(&self, _view: &WKWebView, task: &ProtocolObject<dyn WKURLSchemeTask>) {
            let uri = unsafe { task.request() }
                .URL()
                .and_then(|url| url.absoluteString())
                .map(|s| s.to_string())
                .unwrap_or_default();
            let Some(path) = pages::path_of(&uri) else {
                return;
            };
            let Some(bytes) = file(&path) else {
                return;
            };
            let url = NSURL::URLWithString(&NSString::from_str(&uri));
            let Some(url) = url else { return };
            let response =
                NSURLResponse::initWithURL_MIMEType_expectedContentLength_textEncodingName(
                    NSURLResponse::alloc(),
                    &url,
                    Some(&NSString::from_str(pages::mime_type(&path))),
                    bytes.len() as isize,
                    None,
                );
            let data = NSData::with_bytes(bytes);
            unsafe {
                task.didReceiveResponse(&response);
                task.didReceiveData(&data);
                task.didFinish();
            }
        }

        #[unsafe(method(webView:stopURLSchemeTask:))]
        fn stop(&self, _view: &WKWebView, _task: &ProtocolObject<dyn WKURLSchemeTask>) {}
    }
);

define_class!(
    /// Everything the chrome says arrives here as the protocol the core knows.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GlimmerwoodListener"]
    struct Listener;

    unsafe impl NSObjectProtocol for Listener {}

    unsafe impl WKScriptMessageHandler for Listener {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        fn received(&self, _controller: &WKUserContentController, message: &WKScriptMessage) {
            let body = unsafe { message.body() };
            let data = unsafe {
                NSJSONSerialization::dataWithJSONObject_options_error(
                    &body,
                    NSJSONWritingOptions::empty(),
                )
            };
            let Ok(data) = data else { return };
            let json = String::from_utf8_lossy(unsafe { data.as_bytes_unchecked() }).into_owned();
            match serde_json::from_str::<ToCore>(&json) {
                Ok(message) => heard(message),
                Err(err) => {
                    eprintln!("glimmerwood: ignoring a chrome message the core doesn't know: {err}")
                }
            }
        }
    }
);

fn file(path: &str) -> Option<&'static [u8]> {
    crate::files::FILES
        .iter()
        .find(|(name, _)| *name == path)
        .map(|(_, bytes)| *bytes)
}

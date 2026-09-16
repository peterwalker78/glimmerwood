//! The window, and the engines inside it.
//!
//! The toolbar runs across the top and the page fills the rest, each its own
//! WKWebView. The toolbar is Glimmerwood's own chrome and is the only one
//! given a handler for `glimmerwood://`; the page below it is the web.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use glimmerwood_core::companion::Companion;
use glimmerwood_core::host;
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
    static SHELL: RefCell<Option<Rc<Shell>>> = const { RefCell::new(None) };
    static COMPANION: RefCell<Option<Rc<Companion>>> = const { RefCell::new(None) };
}

/// The shell, for the few places that need it from outside.
pub fn held() -> Option<Rc<Shell>> {
    SHELL.with_borrow(|held| held.clone())
}

pub fn companion() -> Option<Rc<Companion>> {
    COMPANION.with_borrow(|held| held.clone())
}

pub struct Shell {
    window: Retained<NSWindow>,
    pub(crate) toolbar: Retained<WKWebView>,
    pub(crate) page: Retained<WKWebView>,
    toolbar_height: Cell<f64>,
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

    let shell = Rc::new(Shell {
        window: window.clone(),
        toolbar,
        page,
        toolbar_height: Cell::new(TOOLBAR_HEIGHT),
    });
    SHELL.with_borrow_mut(|held| *held = Some(shell.clone()));
    lay_out();

    // The companion decides how the time is going. It is the same one the
    // Linux build uses; this only tells it what is on screen.
    let started = Companion::new(shell as Rc<dyn host::Host>, None);
    started.windows_changed();
    COMPANION.with_borrow_mut(|held| *held = Some(started));

    window.center();
    window.makeKeyAndOrderFront(None);
    app.activate();
    app.run();
}

/// A webview. Only the chrome is given a way to ask for Glimmerwood's own
/// files; the page below it is the web, and has no handler for the scheme.
fn make_webview(mtm: MainThreadMarker, is_chrome: bool) -> Retained<WKWebView> {
    let configuration = unsafe { WKWebViewConfiguration::new(mtm) };
    {
        let scheme: Retained<Scheme> = unsafe { msg_send![Scheme::alloc(mtm), init] };
        let scheme = ProtocolObject::from_retained(scheme);
        unsafe {
            configuration.setURLSchemeHandler_forURLScheme(
                Some(&scheme),
                &NSString::from_str("glimmerwood"),
            );
        }
    }
    if is_chrome {
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
        let split = shell.toolbar_height.get().min(whole.size.height);
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

/// Hand Home or Settings what it shows, if that is what is on the page.
fn push_page() {
    let Some(shell) = held() else { return };
    let Some(companion) = companion() else { return };
    let uri = unsafe { shell.page.URL() }
        .and_then(|url| url.absoluteString())
        .map(|text| text.to_string())
        .unwrap_or_default();
    let Some(page) = pages::Local::of(&uri) else {
        return;
    };
    let json = match page {
        pages::Local::Home => serde_json::to_string(&companion.home_data(companion.now()))
            .expect("home data serialises"),
        pages::Local::Settings => {
            serde_json::to_string(&companion.settings_data()).expect("settings serialises")
        }
    };
    let script = NSString::from_str(&page.hand_over(&json));
    unsafe {
        shell
            .page
            .evaluateJavaScript_completionHandler(&script, None);
    }
}

/// What the page is showing, for the address field and the buttons.
fn push_state() {
    push_page();
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
    SHELL.with_borrow(|held| {
        let Some(shell) = held.as_ref() else { return };
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
                shell.toolbar_height.set(f64::from(height));
            }
            ToCore::Minimize => shell.window.miniaturize(None),
            ToCore::CloseWindow => shell.window.close(),
            ToCore::RateSite { site, rating } => {
                if let Some(companion) = companion() {
                    companion.rate_site(&site, Some(rating));
                }
            }
            ToCore::NotNow { site } => {
                if let Some(companion) = companion() {
                    companion.not_now(&site);
                }
            }
            ToCore::CloseCare => {
                if let Some(companion) = companion() {
                    companion.close_care();
                }
            }
            ToCore::Ready { .. } => {
                if let Some(companion) = companion() {
                    companion.chrome_ready();
                }
            }
            // The rest belong to tabs and the lists, which this shell has yet
            // to grow.
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
            let ours = held().is_some_and(|shell| &*shell.toolbar == _view);
            if !ours && !pages::is_local_page(&uri) {
                return;
            }
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

impl host::Window for Shell {
    fn in_front(&self) -> bool {
        self.window.isKeyWindow() && self.window.isVisible()
    }

    fn attended_uri(&self) -> String {
        unsafe { self.page.URL() }
            .and_then(|url| url.absoluteString())
            .map(|text| text.to_string())
            .unwrap_or_default()
    }

    fn other_tabs(&self, _in_front: bool) -> Vec<String> {
        // One page at a time, until this shell grows tabs.
        Vec::new()
    }

    fn sound_on_screen(&self) -> bool {
        // WebKit tells an app this only through a private property, so until
        // there is a supported way to ask, sound is not counted here.
        false
    }

    fn send_to_chrome(&self, message: &ToChrome) {
        tell_the_chrome(message);
    }

    fn refresh_pages(&self) {
        push_page();
    }

    fn ask(&self, site: Option<String>) {
        tell_the_chrome(&ToChrome::Ask { site });
    }

    fn care(&self, open: bool, samaritans: bool) {
        tell_the_chrome(&ToChrome::Care { open, samaritans });
    }

    fn set_title(&self, title: &str) {
        self.window.setTitle(&NSString::from_str(title));
    }
}

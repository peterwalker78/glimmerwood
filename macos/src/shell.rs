//! The window, and the engines inside it.
//!
//! The toolbar runs across the top, the tab column down the left and the tab
//! in front fills the rest, each its own WKWebView. The toolbar and the
//! column are Glimmerwood's own chrome and the only ones given a handler for
//! `glimmerwood://`; the tabs are the web, and each keeps its own engine
//! whether or not it is the one being looked at.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use glimmerwood_core::companion::Companion;
use glimmerwood_core::host;
use glimmerwood_core::nav;
use glimmerwood_core::pages;
use glimmerwood_core::protocol::{ChromeView, Security, ToChrome, ToCore};
use glimmerwood_core::tabs::Tabs;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSData, NSJSONSerialization, NSJSONWritingOptions, NSPoint, NSRect, NSSize,
    NSString, NSTimer, NSURL, NSURLResponse,
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
/// The tab column's width. The GTK build lets it be dragged and remembers
/// where; here it is the width that shows a tab's mark and nothing else.
const COLUMN_WIDTH: f64 = 48.0;
/// How often the engines are asked what they are showing. WebKit reports a
/// title or a finished load through KVO, which this stands in for.
const WATCH_SECONDS: f64 = 0.4;

/// Where a new tab starts, and where the chrome's buttons point.
const HOME: &str = "glimmerwood://home/";
const HOME_WISP: &str = "glimmerwood://home/#wisp";
const SETTINGS: &str = "glimmerwood://settings/";
const HELPLINES: &str = "https://findahelpline.com/";
const SAMARITANS: &str = "https://www.samaritans.org/how-we-can-help/contact-samaritan/";

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

/// One tab's engine. A tab has no script message handler, so a page has no
/// way to speak to the core.
struct Engine {
    id: u32,
    view: Retained<WKWebView>,
}

pub struct Shell {
    window: Retained<NSWindow>,
    pub(crate) toolbar: Retained<WKWebView>,
    pub(crate) sidebar: Retained<WKWebView>,
    /// The engines, in the order the column shows them.
    engines: RefCell<Vec<Engine>>,
    /// Which tabs there are and which one is in front. The bookkeeping is the
    /// core's, so it is the same here as it is anywhere else.
    tabs: RefCell<Tabs>,
    toolbar_height: Cell<f64>,
    mtm: MainThreadMarker,
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
    let sidebar = make_webview(mtm, true);

    if let Some(content) = window.contentView() {
        content.addSubview(&toolbar);
        content.addSubview(&sidebar);
    }

    load(&toolbar, "glimmerwood://chrome/toolbar.html");
    load(&sidebar, "glimmerwood://chrome/sidebar.html");

    let shell = Rc::new(Shell {
        window: window.clone(),
        toolbar,
        sidebar,
        engines: RefCell::new(Vec::new()),
        tabs: RefCell::new(Tabs::new()),
        toolbar_height: Cell::new(TOOLBAR_HEIGHT),
        mtm,
    });
    SHELL.with_borrow_mut(|held| *held = Some(shell.clone()));
    open_tab(HOME, true);
    watch_the_engines();
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

/// The toolbar across the top, the column down the left, the tab in front
/// filling what is left. Cocoa measures from the bottom left, so the
/// toolbar's frame sits at the top of the view. A tab that isn't in front is
/// hidden rather than removed: it goes on loading, and goes on making sound.
fn lay_out() {
    SHELL.with_borrow(|held| {
        let Some(shell) = held.as_ref() else { return };
        let Some(content) = shell.window.contentView() else {
            return;
        };
        let whole = content.frame();
        let split = shell.toolbar_height.get().min(whole.size.height);
        let column = COLUMN_WIDTH.min(whole.size.width);
        let below = whole.size.height - split;
        shell.toolbar.setFrame(NSRect::new(
            NSPoint::new(0.0, whole.size.height - split),
            NSSize::new(whole.size.width, split),
        ));
        shell.sidebar.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(column, below),
        ));
        let selected = shell.tabs.borrow().selected();
        let page = NSRect::new(
            NSPoint::new(column, 0.0),
            NSSize::new(whole.size.width - column, below),
        );
        for engine in shell.engines.borrow().iter() {
            engine.view.setFrame(page);
            engine.view.setHidden(Some(engine.id) != selected);
        }
    });
}

// --- Tabs --------------------------------------------------------------------

/// The engine behind a tab, cloned out so nothing stays borrowed while it is
/// used.
fn engine(id: u32) -> Option<Retained<WKWebView>> {
    let shell = held()?;
    shell
        .engines
        .borrow()
        .iter()
        .find(|engine| engine.id == id)
        .map(|engine| engine.view.clone())
}

fn selected_view() -> Option<Retained<WKWebView>> {
    let id = held()?.tabs.borrow().selected()?;
    engine(id)
}

/// A new tab showing `uri`, with an engine of its own.
fn open_tab(uri: &str, select: bool) -> Option<u32> {
    let shell = held()?;
    let view = make_webview(shell.mtm, false);
    if let Some(content) = shell.window.contentView() {
        content.addSubview(&view);
    }
    let id = shell.tabs.borrow_mut().open(select);
    shell.engines.borrow_mut().push(Engine {
        id,
        view: view.clone(),
    });
    load(&view, uri);
    lay_out();
    push_tabs();
    if select {
        push_state();
    }
    Some(id)
}

fn select_tab(id: u32) {
    let Some(shell) = held() else { return };
    if !shell.tabs.borrow_mut().select(id) {
        return;
    }
    lay_out();
    push_tabs();
    push_state();
    focus_tab(id);
    if let Some(companion) = companion() {
        companion.refresh();
    }
}

/// The keyboard belongs to the page once a tab is in front, unless it is one
/// of Glimmerwood's own pages, where the address field is the more useful
/// place to be.
fn focus_tab(id: u32) {
    let local = held().is_some_and(|shell| {
        shell
            .tabs
            .borrow()
            .facts(id)
            .is_some_and(|facts| facts.uri.is_empty() || pages::is_home(&facts.uri))
    });
    if local {
        tell_the_chrome(&ToChrome::FocusAddress);
    } else if let Some(view) = engine(id) {
        view.window().inspect(|window| {
            window.makeFirstResponder(Some(&view));
        });
    }
}

/// Closes a tab and hands the window to whichever takes its place. The last
/// tab closing closes the window, as it does everywhere else.
fn close_tab(id: u32) {
    let Some(shell) = held() else { return };
    let index = {
        let engines = shell.engines.borrow();
        engines.iter().position(|engine| engine.id == id)
    };
    let Some(index) = index else { return };
    let engine = shell.engines.borrow_mut().remove(index);
    unsafe { engine.view.stopLoading() };
    engine.view.removeFromSuperview();
    let was_selected = shell.tabs.borrow().selected() == Some(id);
    let next = shell.tabs.borrow_mut().close(id);
    match next {
        None => shell.window.close(),
        Some(_) => {
            lay_out();
            push_tabs();
            if was_selected {
                push_state();
            }
            if let Some(companion) = companion() {
                companion.refresh();
            }
        }
    }
}

/// Reads what the engines are showing into the bookkeeping, and tells the
/// chrome only about what changed. WebKit reports a title or a finished load
/// through KVO; this asks instead, on the watch timer.
fn refresh_tabs() {
    let Some(shell) = held() else { return };
    let engines: Vec<(u32, Retained<WKWebView>)> = shell
        .engines
        .borrow()
        .iter()
        .map(|engine| (engine.id, engine.view.clone()))
        .collect();
    let mut changed = false;
    let mut front_changed = false;
    let selected = shell.tabs.borrow().selected();
    for (id, view) in engines {
        let uri = unsafe { view.URL() }
            .and_then(|url| url.absoluteString())
            .map(|text| text.to_string())
            .unwrap_or_default();
        let title = unsafe { view.title() }
            .map(|text| text.to_string())
            .unwrap_or_default();
        let loading = unsafe { view.isLoading() };
        if shell.tabs.borrow_mut().update(id, |facts| {
            facts.uri = uri;
            facts.title = title;
            facts.loading = loading;
        }) {
            changed = true;
            front_changed |= Some(id) == selected;
        }
    }
    if changed {
        push_tabs();
    }
    if front_changed {
        push_state();
    }
}

fn push_tabs() {
    let Some(shell) = held() else { return };
    let (tabs, selected) = {
        let held = shell.tabs.borrow();
        (held.info(), held.selected().unwrap_or_default())
    };
    tell_the_column(&ToChrome::Tabs { tabs, selected });
}

/// The wisp's history on Home: the tab in front if that is where it is, else
/// the first Home tab this window has, else a new one.
fn show_wisp() {
    let Some(shell) = held() else { return };
    let home = {
        let tabs = shell.tabs.borrow();
        let front = tabs
            .selected()
            .filter(|id| tabs.facts(*id).is_some_and(|f| pages::is_home(&f.uri)));
        front.or_else(|| {
            tabs.ids()
                .into_iter()
                .find(|id| tabs.facts(*id).is_some_and(|f| pages::is_home(&f.uri)))
        })
    };
    match home {
        Some(id) => {
            select_tab(id);
            if let Some(view) = engine(id) {
                let script = NSString::from_str(
                    "if (location.href.startsWith('glimmerwood://home/')) \
                     window.wispHome?.reveal()",
                );
                unsafe { view.evaluateJavaScript_completionHandler(&script, None) };
            }
        }
        None => {
            open_tab(HOME_WISP, true);
        }
    }
}

/// Settings: the window's Settings tab if it has one, else a new one.
fn open_settings() {
    let Some(shell) = held() else { return };
    let existing = {
        let tabs = shell.tabs.borrow();
        tabs.ids()
            .into_iter()
            .find(|id| tabs.facts(*id).is_some_and(|f| pages::is_settings(&f.uri)))
    };
    match existing {
        Some(id) => select_tab(id),
        None => {
            if let Some(companion) = companion() {
                companion.forget_lookup();
            }
            open_tab(SETTINGS, true);
        }
    }
}

/// Star the page in front, or unstar it.
fn toggle_bookmark() {
    let Some(shell) = held() else { return };
    let Some(companion) = companion() else { return };
    let Some(facts) = shell
        .tabs
        .borrow()
        .selected_facts()
        .filter(|facts| !facts.uri.is_empty() && !pages::is_local_page(&facts.uri))
        .cloned()
    else {
        return;
    };
    let title = if facts.title.is_empty() {
        facts.uri.clone()
    } else {
        facts.title.clone()
    };
    companion.toggle_bookmark(&facts.uri, &title);
    push_state();
    companion.refresh_pages();
}

fn tell_the_chrome(message: &ToChrome) {
    if let Some(shell) = held() {
        say(&shell.toolbar, message);
    }
}

/// The tab column hears about the tabs; the toolbar hears everything else.
/// Each chrome page is only sent what it draws.
fn tell_the_column(message: &ToChrome) {
    if let Some(shell) = held() {
        say(&shell.sidebar, message);
    }
}

fn say(view: &WKWebView, message: &ToChrome) {
    let Ok(json) = serde_json::to_string(message) else {
        return;
    };
    let script = NSString::from_str(&format!("window.wispChrome?.receive?.({json})"));
    unsafe { view.evaluateJavaScript_completionHandler(&script, None) };
}

/// Hand Home or Settings what it shows, if that is what a tab holds.
fn push_page(id: u32) {
    let Some(companion) = companion() else { return };
    let Some(view) = engine(id) else { return };
    let uri = unsafe { view.URL() }
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
    unsafe { view.evaluateJavaScript_completionHandler(&script, None) };
}

/// What the tab in front is showing, for the address field and the buttons
/// beside it.
fn push_state() {
    let Some(shell) = held() else { return };
    let Some(id) = shell.tabs.borrow().selected() else {
        return;
    };
    let Some(view) = engine(id) else { return };
    push_page(id);
    let uri = unsafe { view.URL() }
        .and_then(|url| url.absoluteString())
        .map(|s| s.to_string())
        .unwrap_or_default();
    let title = unsafe { view.title() }
        .map(|s| s.to_string())
        .unwrap_or_default();
    let loading = unsafe { view.isLoading() };
    let local = pages::is_local_page(&uri);
    let bookmarked = companion().is_some_and(|companion| companion.is_bookmarked(&uri));
    shell
        .window
        .setTitle(&NSString::from_str(if title.is_empty() {
            "Glimmerwood"
        } else {
            &title
        }));
    tell_the_chrome(&ToChrome::State {
        security: if local {
            Security::Local
        } else {
            nav::security(&uri)
        },
        uri: uri.clone(),
        title,
        loading,
        progress: if loading { 0.5 } else { 1.0 },
        can_go_back: unsafe { view.canGoBack() },
        can_go_forward: unsafe { view.canGoForward() },
        can_bookmark: !local && !uri.is_empty(),
        bookmarked,
    });
}

/// Ask the engines what they are showing, on a timer, because WebKit reports
/// it through KVO and this shell has no observers.
fn watch_the_engines() {
    let watcher: Retained<Watcher> = unsafe { msg_send![Watcher::alloc(), init] };
    unsafe {
        NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
            WATCH_SECONDS,
            &watcher,
            sel!(fire:),
            None,
            true,
        );
    }
}

fn heard(message: ToCore) {
    let Some(shell) = held() else { return };
    let view = selected_view();
    match message {
        ToCore::Navigate { input } => {
            if let (Some(view), Some(target)) = (view, nav::resolve(&input)) {
                load(&view, &target.uri);
            }
        }
        ToCore::Back => {
            if let Some(view) = view {
                unsafe {
                    let _ = view.goBack();
                }
            }
        }
        ToCore::Forward => {
            if let Some(view) = view {
                unsafe {
                    let _ = view.goForward();
                }
            }
        }
        ToCore::Reload => {
            if let Some(view) = view {
                unsafe {
                    let _ = view.reload();
                }
            }
        }
        ToCore::Stop => {
            if let Some(view) = view {
                unsafe { view.stopLoading() };
            }
        }
        ToCore::GoHome => {
            if let Some(view) = view {
                load(&view, HOME);
            }
        }
        ToCore::FocusPage => {
            if let Some(id) = shell.tabs.borrow().selected() {
                focus_tab(id);
            }
        }
        ToCore::NewTab => {
            open_tab(HOME, true);
        }
        ToCore::SelectTab { id } => select_tab(id),
        ToCore::CloseTab { id } => close_tab(id),
        ToCore::ToggleMute { .. } => {
            // Muting a tab is behind the same private API that says whether
            // one is making a sound, so the column doesn't offer it here.
        }
        ToCore::ToggleBookmark => toggle_bookmark(),
        ToCore::ShowWisp => show_wisp(),
        ToCore::OpenSettings => open_settings(),
        ToCore::FindSupport { samaritans } => {
            open_tab(if samaritans { SAMARITANS } else { HELPLINES }, true);
        }
        ToCore::ToolbarLayout { height, .. } => {
            shell.toolbar_height.set(f64::from(height));
            lay_out();
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
        ToCore::Ready {
            view: ChromeView::Toolbar,
        } => {
            if let Some(companion) = companion() {
                companion.chrome_ready();
            }
            push_state();
        }
        ToCore::Ready {
            view: ChromeView::Sidebar,
        } => push_tabs(),
        // Find in page is the one thing still to come here; everything else
        // the chrome can ask for is answered.
        _ => {}
    }
    refresh_tabs();
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

define_class!(
    /// Asks the engines what they are showing, on a repeating timer.
    #[unsafe(super(NSObject))]
    #[name = "GlimmerwoodWatcher"]
    struct Watcher;

    unsafe impl NSObjectProtocol for Watcher {}

    impl Watcher {
        #[unsafe(method(fire:))]
        fn fire(&self, _timer: &NSTimer) {
            refresh_tabs();
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
        self.tabs
            .borrow()
            .selected_facts()
            .map(|facts| facts.uri.clone())
            .unwrap_or_default()
    }

    /// Every tab but the one being looked at. They weigh nothing, but the
    /// dose engine still wants to know they are open.
    fn other_tabs(&self, in_front: bool) -> Vec<String> {
        let tabs = self.tabs.borrow();
        let attended = if in_front { tabs.selected() } else { None };
        tabs.other_uris(attended)
    }

    fn sound_on_screen(&self) -> bool {
        // WebKit tells an app this only through a private property, so until
        // there is a supported way to ask, sound is not counted here.
        false
    }

    fn send_to_chrome(&self, message: &ToChrome) {
        match message {
            ToChrome::Tabs { .. } | ToChrome::TabIcon { .. } => tell_the_column(message),
            _ => tell_the_chrome(message),
        }
    }

    fn refresh_pages(&self) {
        for id in self.tabs.borrow().ids() {
            push_page(id);
        }
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

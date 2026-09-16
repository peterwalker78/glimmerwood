//! The window, and the engines inside it.
//!
//! The toolbar runs across the top, the tab column down the left and the tab
//! in front fills the rest, each its own WKWebView. The toolbar and the
//! column are Glimmerwood's own chrome and the only ones given a handler for
//! `glimmerwood://`; the tabs are the web, and each keeps its own engine
//! whether or not it is the one being looked at.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use block2::DynBlock;
use glimmerwood_core::companion::Companion;
use glimmerwood_core::failure::{self, Reason};
use glimmerwood_core::host;
use glimmerwood_core::nav;
use glimmerwood_core::pages;
use glimmerwood_core::protocol::{ChromeView, Security, ToChrome, ToCore};
use glimmerwood_core::tabs::Tabs;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{AllocAnyThread, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSEventModifierFlags, NSMenu,
    NSMenuItem, NSWindow, NSWindowDelegate, NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSData, NSError, NSInteger, NSJSONSerialization, NSJSONWritingOptions,
    NSNotification, NSPoint, NSRect, NSSize, NSString, NSTimer, NSURL,
    NSURLAuthenticationChallenge, NSURLAuthenticationMethodServerTrust, NSURLCredential,
    NSURLErrorCancelled, NSURLErrorCannotConnectToHost, NSURLErrorCannotFindHost,
    NSURLErrorDNSLookupFailed, NSURLErrorFailingURLErrorKey, NSURLErrorInternationalRoamingOff,
    NSURLErrorNetworkConnectionLost, NSURLErrorNotConnectedToInternet,
    NSURLErrorSecureConnectionFailed, NSURLErrorServerCertificateHasBadDate,
    NSURLErrorServerCertificateHasUnknownRoot, NSURLErrorServerCertificateNotYetValid,
    NSURLErrorServerCertificateUntrusted, NSURLErrorTimedOut, NSURLResponse,
    NSURLSessionAuthChallengeDisposition,
};
use objc2_web_kit::{
    WKNavigation, WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate,
    WKScriptMessage, WKScriptMessageHandler, WKURLSchemeHandler, WKURLSchemeTask,
    WKUserContentController, WKWebView, WKWebViewConfiguration,
};

/// The size the window opens at before it has been left anywhere to return
/// to, and before the chrome says otherwise.
const INITIAL_WIDTH: f64 = 1100.0;
const INITIAL_HEIGHT: f64 = 760.0;
/// The toolbar's height until it reports its own.
const TOOLBAR_HEIGHT: f64 = 56.0;
/// The tab column's width. The GTK build lets it be dragged and remembers
/// where; here it is the width that shows a tab's mark and nothing else.
const COLUMN_WIDTH: f64 = 48.0;
/// How often the engines are asked what they are showing. The navigation
/// delegate reports a load starting, committing, finishing or failing as it
/// happens; this is left for the two things it never hears about — a page
/// that changes its own title after it has loaded, and a move within a page
/// that never becomes a navigation.
const WATCH_SECONDS: f64 = 2.0;

/// Where a new tab starts, and where the chrome's buttons point.
const HOME: &str = "glimmerwood://home/";
const HOME_WISP: &str = "glimmerwood://home/#wisp";
const SETTINGS: &str = "glimmerwood://settings/";
const HELPLINES: &str = "https://findahelpline.com/";
const SAMARITANS: &str = "https://www.samaritans.org/how-we-can-help/contact-samaritan/";

thread_local! {
    static SHELL: RefCell<Option<Rc<Shell>>> = const { RefCell::new(None) };
    static COMPANION: RefCell<Option<Rc<Companion>>> = const { RefCell::new(None) };
    /// A window points at its delegate weakly, so this one is kept here.
    static KEEPER: RefCell<Option<Retained<Keeper>>> = const { RefCell::new(None) };
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
    /// While an address optimistically upgraded to HTTPS is in flight: what
    /// it tried, and the plain HTTP form to use instead if that exact attempt
    /// can't connect.
    fallback: RefCell<Option<nav::Target>>,
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
    // The window goes back where it was left before anything is laid out
    // inside it.
    match saved_frame() {
        Some(frame) => window.setFrame_display(frame, false),
        None => window.center(),
    }
    let keeper: Retained<Keeper> = unsafe { msg_send![Keeper::alloc(mtm), init] };
    window.setDelegate(Some(&ProtocolObject::from_retained(keeper.clone())));
    KEEPER.with_borrow_mut(|held| *held = Some(keeper));
    build_the_menu(&app, mtm);
    open_tab(HOME, true);
    watch_the_engines();
    lay_out();

    // The companion decides how the time is going. It is the same one the
    // Linux build uses; this only tells it what is on screen.
    let started = Companion::new(shell as Rc<dyn host::Host>, None);
    started.windows_changed();
    COMPANION.with_borrow_mut(|held| *held = Some(started));

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
    let view = unsafe {
        WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), frame, &configuration)
    };
    if !is_chrome {
        let navigator = ProtocolObject::from_retained(navigator(mtm));
        unsafe { view.setNavigationDelegate(Some(&navigator)) };
    }
    view
}

/// What a webview says it is showing.
fn uri_of(view: &WKWebView) -> String {
    unsafe { view.URL() }
        .and_then(|url| url.absoluteString())
        .map(|text| text.to_string())
        .unwrap_or_default()
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

/// Which tab an engine belongs to.
fn tab_of(view: &WKWebView) -> Option<u32> {
    let shell = held()?;
    shell
        .engines
        .borrow()
        .iter()
        .find(|engine| &*engine.view == view)
        .map(|engine| engine.id)
}

/// Sends a tab to `input`, keeping the plain HTTP form of an address that was
/// optimistically upgraded in case the secure attempt can't connect.
fn navigate(id: u32, input: &str) {
    let Some(shell) = held() else { return };
    let Some(view) = engine(id) else { return };
    let Some(target) = nav::resolve(input) else {
        return;
    };
    load(&view, &target.uri);
    if let Some(engine) = shell.engines.borrow().iter().find(|engine| engine.id == id) {
        engine
            .fallback
            .replace(target.fallback.is_some().then_some(target));
    }
}

/// The plain HTTP address to try instead, if this failure is the upgraded
/// attempt's own. A fallback belongs to one attempt: the failure of a page
/// this navigation replaced may arrive after it, and must leave it be.
fn fallback_for(id: u32, failing: &str) -> Option<String> {
    let shell = held()?;
    let engines = shell.engines.borrow();
    let engine = engines.iter().find(|engine| engine.id == id)?;
    let mut slot = engine.fallback.borrow_mut();
    match slot.as_ref() {
        Some(target) if nav::same_address(&target.uri, failing) => {
            slot.take().and_then(|target| target.fallback)
        }
        _ => None,
    }
}

/// Forgets a tab's fallback: the attempt got far enough that trying the same
/// address again without encryption would be the wrong answer.
fn forget_fallback(id: u32) {
    if let Some(shell) = held()
        && let Some(engine) = shell.engines.borrow().iter().find(|engine| engine.id == id)
    {
        engine.fallback.take();
    }
}

/// The tab `by` places along, wrapping at both ends.
fn step_tab(by: isize) {
    let next = held().and_then(|shell| shell.tabs.borrow().step(by));
    if let Some(id) = next {
        select_tab(id);
    }
}

/// The nth tab, counting from zero. There is nothing to select past the end.
fn select_nth(n: usize) {
    let id = held().and_then(|shell| shell.tabs.borrow().nth(n));
    if let Some(id) = id {
        select_tab(id);
    }
}

/// The tab at the end of the column.
fn select_last() {
    let id = held().and_then(|shell| shell.tabs.borrow().last());
    if let Some(id) = id {
        select_tab(id);
    }
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
        fallback: RefCell::new(None),
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
/// chrome only about what changed. The navigation delegate calls this as a
/// load moves on; the watch timer calls it for what a delegate never hears.
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
        let uri = uri_of(&view);
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
    let uri = uri_of(&view);
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
    let uri = uri_of(&view);
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

/// Ask the engines what they are showing, on a slow timer. The navigation
/// delegate covers a load's own edges; what is left is a page that changes
/// its title once it is up and a move within a page that never becomes a
/// navigation, and WebKit reports both only through KVO, which this shell has
/// no observers for.
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
            let selected = shell.tabs.borrow().selected();
            if let Some(id) = selected {
                navigate(id, &input);
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

// --- The menu bar ------------------------------------------------------------

/// The arrow keys, as a menu item spells them.
const LEFT_ARROW: &str = "\u{f702}";
const RIGHT_ARROW: &str = "\u{f703}";
/// Tabs reachable by number; the ninth key is always the last tab.
const NUMBERED_TABS: usize = 8;

thread_local! {
    /// A menu item points at its target weakly, so the one every item shares
    /// is kept here.
    static MENUS: RefCell<Option<Retained<Menus>>> = const { RefCell::new(None) };
}

/// The menu bar. Without one, none of the standard keys — copy, paste, quit —
/// reach anything, so neither the address field nor the page can be edited.
fn build_the_menu(app: &NSApplication, mtm: MainThreadMarker) {
    let menus: Retained<Menus> = unsafe { msg_send![Menus::alloc(mtm), init] };
    let bar = NSMenu::new(mtm);
    let command = NSEventModifierFlags::Command;
    let shifted = NSEventModifierFlags::Command | NSEventModifierFlags::Shift;
    let alt = NSEventModifierFlags::Command | NSEventModifierFlags::Option;
    let target = Some(&*menus as &AnyObject);

    // The first submenu is the application's own, whatever it is called.
    let application = submenu(&bar, mtm, "Glimmerwood");
    application.addItem(&item(
        mtm,
        "About Glimmerwood",
        sel!(orderFrontStandardAboutPanel:),
        "",
        command,
        None,
    ));
    application.addItem(&NSMenuItem::separatorItem(mtm));
    application.addItem(&item(
        mtm,
        "Settings…",
        sel!(openSettings:),
        ",",
        command,
        target,
    ));
    application.addItem(&NSMenuItem::separatorItem(mtm));
    application.addItem(&item(
        mtm,
        "Hide Glimmerwood",
        sel!(hide:),
        "h",
        command,
        None,
    ));
    application.addItem(&NSMenuItem::separatorItem(mtm));
    application.addItem(&item(
        mtm,
        "Quit Glimmerwood",
        sel!(quitGlimmerwood:),
        "q",
        command,
        target,
    ));

    // Nothing targets these: they travel up the responder chain to whatever
    // is being edited, which is the only thing that knows what to do.
    let edit = submenu(&bar, mtm, "Edit");
    edit.addItem(&item(mtm, "Undo", sel!(undo:), "z", command, None));
    edit.addItem(&item(mtm, "Redo", sel!(redo:), "z", shifted, None));
    edit.addItem(&NSMenuItem::separatorItem(mtm));
    edit.addItem(&item(mtm, "Cut", sel!(cut:), "x", command, None));
    edit.addItem(&item(mtm, "Copy", sel!(copy:), "c", command, None));
    edit.addItem(&item(mtm, "Paste", sel!(paste:), "v", command, None));
    edit.addItem(&NSMenuItem::separatorItem(mtm));
    edit.addItem(&item(
        mtm,
        "Select All",
        sel!(selectAll:),
        "a",
        command,
        None,
    ));

    let file = submenu(&bar, mtm, "File");
    file.addItem(&item(
        mtm,
        "New Tab",
        sel!(openNewTab:),
        "t",
        command,
        target,
    ));
    file.addItem(&item(
        mtm,
        "Close Tab",
        sel!(closeCurrentTab:),
        "w",
        command,
        target,
    ));
    file.addItem(&NSMenuItem::separatorItem(mtm));
    file.addItem(&item(
        mtm,
        "Open Location…",
        sel!(openLocation:),
        "l",
        command,
        target,
    ));

    let view = submenu(&bar, mtm, "View");
    view.addItem(&item(
        mtm,
        "Reload",
        sel!(reloadPage:),
        "r",
        command,
        target,
    ));
    view.addItem(&NSMenuItem::separatorItem(mtm));
    view.addItem(&item(
        mtm,
        "Next Tab",
        sel!(showNextTab:),
        RIGHT_ARROW,
        alt,
        target,
    ));
    view.addItem(&item(
        mtm,
        "Previous Tab",
        sel!(showPreviousTab:),
        LEFT_ARROW,
        alt,
        target,
    ));
    spare_key(
        &view,
        mtm,
        sel!(showNextTab:),
        "\t",
        NSEventModifierFlags::Control,
        target,
    );
    spare_key(
        &view,
        mtm,
        sel!(showPreviousTab:),
        "\t",
        NSEventModifierFlags::Control | NSEventModifierFlags::Shift,
        target,
    );
    // The numbers reach a tab without taking up a line of the menu each.
    for n in 1..=NUMBERED_TABS {
        let numbered = item(
            mtm,
            &format!("Tab {n}"),
            sel!(showNumberedTab:),
            &n.to_string(),
            command,
            target,
        );
        numbered.setTag(n as NSInteger);
        hide(&numbered);
        view.addItem(&numbered);
    }
    let last = item(mtm, "Last Tab", sel!(showLastTab:), "9", command, target);
    hide(&last);
    view.addItem(&last);

    let history = submenu(&bar, mtm, "History");
    history.addItem(&item(mtm, "Back", sel!(goBack:), "[", command, target));
    history.addItem(&item(
        mtm,
        "Forward",
        sel!(goForward:),
        "]",
        command,
        target,
    ));
    spare_key(&history, mtm, sel!(goBack:), LEFT_ARROW, command, target);
    spare_key(
        &history,
        mtm,
        sel!(goForward:),
        RIGHT_ARROW,
        command,
        target,
    );
    history.addItem(&NSMenuItem::separatorItem(mtm));
    history.addItem(&item(mtm, "Home", sel!(goHome:), "h", shifted, target));

    app.setMainMenu(Some(&bar));
    MENUS.with_borrow_mut(|held| *held = Some(menus));
}

/// A menu of `title` hung off the bar.
fn submenu(bar: &NSMenu, mtm: MainThreadMarker, title: &str) -> Retained<NSMenu> {
    let holder = NSMenuItem::new(mtm);
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title));
    holder.setSubmenu(Some(&menu));
    bar.addItem(&holder);
    menu
}

/// One item. No target sends the action up the responder chain instead.
fn item(
    mtm: MainThreadMarker,
    title: &str,
    action: Sel,
    key: &str,
    modifiers: NSEventModifierFlags,
    target: Option<&AnyObject>,
) -> Retained<NSMenuItem> {
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            Some(action),
            &NSString::from_str(key),
        )
    };
    item.setKeyEquivalentModifierMask(modifiers);
    unsafe { item.setTarget(target) };
    item
}

/// An item that is only there for its key. macOS gives one item one key, so a
/// second way to reach the same thing needs an item of its own.
fn spare_key(
    menu: &NSMenu,
    mtm: MainThreadMarker,
    action: Sel,
    key: &str,
    modifiers: NSEventModifierFlags,
    target: Option<&AnyObject>,
) {
    let spare = item(mtm, "", action, key, modifiers, target);
    hide(&spare);
    menu.addItem(&spare);
}

/// Takes an item out of the menu but leaves its key working, which is how a
/// row of numbered tabs stays reachable without filling the menu.
fn hide(item: &NSMenuItem) {
    item.setHidden(true);
    item.setAllowsKeyEquivalentWhenHidden(true);
}

define_class!(
    /// What the menu bar's own items act on. Each does what the same button
    /// in the chrome does, by the same route.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GlimmerwoodMenus"]
    struct Menus;

    unsafe impl NSObjectProtocol for Menus {}

    impl Menus {
        #[unsafe(method(openNewTab:))]
        fn open_new_tab(&self, _sender: Option<&AnyObject>) {
            open_tab(HOME, true);
        }

        #[unsafe(method(closeCurrentTab:))]
        fn close_current_tab(&self, _sender: Option<&AnyObject>) {
            let selected = held().and_then(|shell| shell.tabs.borrow().selected());
            if let Some(id) = selected {
                close_tab(id);
            }
        }

        #[unsafe(method(openLocation:))]
        fn open_location(&self, _sender: Option<&AnyObject>) {
            tell_the_chrome(&ToChrome::FocusAddress);
        }

        #[unsafe(method(reloadPage:))]
        fn reload_page(&self, _sender: Option<&AnyObject>) {
            if let Some(view) = selected_view() {
                unsafe {
                    let _ = view.reload();
                }
            }
        }

        #[unsafe(method(goBack:))]
        fn go_back(&self, _sender: Option<&AnyObject>) {
            if let Some(view) = selected_view() {
                unsafe {
                    let _ = view.goBack();
                }
            }
        }

        #[unsafe(method(goForward:))]
        fn go_forward(&self, _sender: Option<&AnyObject>) {
            if let Some(view) = selected_view() {
                unsafe {
                    let _ = view.goForward();
                }
            }
        }

        #[unsafe(method(goHome:))]
        fn go_home(&self, _sender: Option<&AnyObject>) {
            if let Some(view) = selected_view() {
                load(&view, HOME);
            }
        }

        #[unsafe(method(openSettings:))]
        fn settings(&self, _sender: Option<&AnyObject>) {
            open_settings();
        }

        #[unsafe(method(showNextTab:))]
        fn show_next_tab(&self, _sender: Option<&AnyObject>) {
            step_tab(1);
        }

        #[unsafe(method(showPreviousTab:))]
        fn show_previous_tab(&self, _sender: Option<&AnyObject>) {
            step_tab(-1);
        }

        #[unsafe(method(showNumberedTab:))]
        fn show_numbered_tab(&self, sender: &NSMenuItem) {
            let n = sender.tag();
            if n >= 1 {
                select_nth(n as usize - 1);
            }
        }

        #[unsafe(method(showLastTab:))]
        fn show_last_tab(&self, _sender: Option<&AnyObject>) {
            select_last();
        }

        /// Quitting doesn't close the window, so the window's size and place
        /// are written down on the way out.
        #[unsafe(method(quitGlimmerwood:))]
        fn quit(&self, _sender: Option<&AnyObject>) {
            remember_frame();
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }
    }
);

// --- Where the window was last left ------------------------------------------

/// Where the window's size and place are kept between runs.
fn frame_path() -> Option<PathBuf> {
    let shell = held()?;
    Some(
        host::Host::config_dir(&*shell)
            .join("glimmerwood")
            .join("window.ini"),
    )
}

/// The window as it was left, if it was written down. Anything unreadable is
/// simply a window that hasn't been opened before.
fn saved_frame() -> Option<NSRect> {
    let text = std::fs::read_to_string(frame_path()?).ok()?;
    let numbers: Vec<f64> = text
        .split_whitespace()
        .filter_map(|word| word.parse().ok())
        .collect();
    let [x, y, width, height] = numbers[..] else {
        return None;
    };
    (width > 0.0 && height > 0.0)
        .then(|| NSRect::new(NSPoint::new(x, y), NSSize::new(width, height)))
}

/// Writes the window's size and place down for next time.
fn remember_frame() {
    let Some(shell) = held() else { return };
    let Some(path) = frame_path() else { return };
    let frame = shell.window.frame();
    let line = format!(
        "frame = {} {} {} {}\n",
        frame.origin.x, frame.origin.y, frame.size.width, frame.size.height
    );
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&path, line));
    if let Err(err) = written {
        eprintln!("glimmerwood: couldn't remember the window's size: {err}");
    }
}

define_class!(
    /// Watches the window, so its size and place outlive it.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GlimmerwoodKeeper"]
    struct Keeper;

    unsafe impl NSObjectProtocol for Keeper {}

    unsafe impl NSWindowDelegate for Keeper {
        #[unsafe(method(windowWillClose:))]
        fn will_close(&self, _notification: &NSNotification) {
            remember_frame();
        }
    }
);

// --- What the engines report -------------------------------------------------

/// The callbacks a WebKit delegate method is handed to answer with.
type Decision = DynBlock<dyn Fn(WKNavigationActionPolicy)>;
type Trust = DynBlock<dyn Fn(NSURLSessionAuthChallengeDisposition, *const NSURLCredential)>;

/// Home and Settings trigger things by asking for an address rather than by
/// speaking to the core, since neither page is given a way to.
const HOME_ACTIONS: &str = "glimmerwood://home/do/";
const SETTINGS_ACTIONS: &str = "glimmerwood://settings/do/";

thread_local! {
    /// A webview points at its navigation delegate weakly, so the one every
    /// tab shares is kept here.
    static NAVIGATOR: RefCell<Option<Retained<Navigator>>> = const { RefCell::new(None) };
}

fn navigator(mtm: MainThreadMarker) -> Retained<Navigator> {
    NAVIGATOR.with_borrow_mut(|held| {
        held.get_or_insert_with(|| unsafe { msg_send![Navigator::alloc(mtm), init] })
            .clone()
    })
}

/// Which reason a Cocoa error is. `None` for a load the user stopped or the
/// shell itself turned away, neither of which is a failure: the tab stays as
/// it is.
fn reason_of(error: &NSError) -> Option<Reason> {
    /// WebKit's own number for a navigation that was answered with Cancel.
    /// The rest below are the URL loading system's, which are all negative.
    const FRAME_INTERRUPTED: NSInteger = 102;
    Some(match error.code() {
        code if code == NSURLErrorCancelled || code == FRAME_INTERRUPTED => return None,
        code if code == NSURLErrorCannotFindHost || code == NSURLErrorDNSLookupFailed => {
            Reason::NotFound
        }
        code if code == NSURLErrorCannotConnectToHost
            || code == NSURLErrorNetworkConnectionLost =>
        {
            Reason::Refused
        }
        code if code == NSURLErrorTimedOut => Reason::NoAnswer,
        code if code == NSURLErrorNotConnectedToInternet
            || code == NSURLErrorInternationalRoamingOff =>
        {
            Reason::Offline
        }
        code if code == NSURLErrorSecureConnectionFailed
            || code == NSURLErrorServerCertificateHasBadDate
            || code == NSURLErrorServerCertificateUntrusted
            || code == NSURLErrorServerCertificateHasUnknownRoot
            || code == NSURLErrorServerCertificateNotYetValid =>
        {
            Reason::Untrusted
        }
        _ => Reason::Other(error.localizedDescription().to_string()),
    })
}

/// Which address the engine was trying. The error carries it; the view's own
/// address is what is left if it doesn't.
fn failing_uri(view: &WKWebView, error: &NSError) -> String {
    error
        .userInfo()
        .objectForKey(unsafe { NSURLErrorFailingURLErrorKey })
        .and_then(|value| value.downcast::<NSURL>().ok())
        .and_then(|url| url.absoluteString())
        .map(|text| text.to_string())
        .unwrap_or_else(|| uri_of(view))
}

/// The page that says why an address didn't open, in place of the address.
fn show_failure(view: &WKWebView, uri: &str, reason: &Reason) {
    let Some(template) = file("pages/failed.html").and_then(|bytes| str::from_utf8(bytes).ok())
    else {
        return;
    };
    let html = NSString::from_str(&failure::fill(template, uri, reason));
    let base = NSURL::URLWithString(&NSString::from_str(uri));
    unsafe {
        let _ = view.loadHTMLString_baseURL(&html, base.as_deref());
    }
}

/// A load that didn't finish: the plain HTTP form if this was an upgraded
/// attempt that couldn't connect, and otherwise the page saying what happened.
fn load_failed(view: &WKWebView, error: &NSError) {
    let Some(reason) = reason_of(error) else {
        return;
    };
    let uri = failing_uri(view, error);
    // A certificate problem is never a reason to try the same address again
    // without encryption.
    if reason != Reason::Untrusted
        && let Some(id) = tab_of(view)
        && let Some(http) = fallback_for(id, &uri)
    {
        load(view, &http);
        return;
    }
    show_failure(view, &uri, &reason);
    refresh_tabs();
}

define_class!(
    /// What the engines report as a page loads: the state the address field
    /// and the tab column draw, the buttons on Home and Settings, and the
    /// page shown when an address can't be opened.
    ///
    /// The methods that answer through a block are written out by hand, so
    /// the conformance above them declares none of its own.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GlimmerwoodNavigator"]
    struct Navigator;

    unsafe impl NSObjectProtocol for Navigator {}

    unsafe impl WKNavigationDelegate for Navigator {}

    impl Navigator {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn decide(&self, view: &WKWebView, action: &WKNavigationAction, answer: &Decision) {
            let wanted = unsafe { action.request() }
                .URL()
                .and_then(|url| url.absoluteString())
                .map(|text| text.to_string())
                .unwrap_or_default();
            let showing = uri_of(view);
            let mut policy = WKNavigationActionPolicy::Allow;
            // An action is only honoured while the tab is still showing the
            // page whose button it belongs to.
            if let Some(deed) = wanted.strip_prefix(HOME_ACTIONS) {
                policy = WKNavigationActionPolicy::Cancel;
                if let Some(companion) = companion()
                    && pages::is_home(&showing)
                    && companion.home_action(deed)
                {
                    companion.refresh_pages();
                }
            } else if let Some(deed) = wanted.strip_prefix(SETTINGS_ACTIONS) {
                policy = WKNavigationActionPolicy::Cancel;
                if let Some(companion) = companion()
                    && pages::is_settings(&showing)
                {
                    companion.settings_action(deed);
                }
            }
            answer.call((policy,));
        }

        #[unsafe(method(webView:didStartProvisionalNavigation:))]
        fn started(&self, _view: &WKWebView, _navigation: Option<&WKNavigation>) {
            refresh_tabs();
        }

        #[unsafe(method(webView:didCommitNavigation:))]
        fn committed(&self, view: &WKWebView, _navigation: Option<&WKNavigation>) {
            if let Some(id) = tab_of(view) {
                forget_fallback(id);
            }
            refresh_tabs();
        }

        #[unsafe(method(webView:didFinishNavigation:))]
        fn finished(&self, view: &WKWebView, _navigation: Option<&WKNavigation>) {
            if let Some(id) = tab_of(view) {
                push_page(id);
            }
            refresh_tabs();
        }

        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn failed_early(
            &self,
            view: &WKWebView,
            _navigation: Option<&WKNavigation>,
            error: &NSError,
        ) {
            load_failed(view, error);
        }

        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn failed_late(
            &self,
            view: &WKWebView,
            _navigation: Option<&WKNavigation>,
            error: &NSError,
        ) {
            load_failed(view, error);
        }

        #[unsafe(method(webView:didReceiveAuthenticationChallenge:completionHandler:))]
        fn challenged(
            &self,
            view: &WKWebView,
            challenge: &NSURLAuthenticationChallenge,
            answer: &Trust,
        ) {
            let method = challenge.protectionSpace().authenticationMethod();
            if &*method == unsafe { NSURLAuthenticationMethodServerTrust }
                && let Some(id) = tab_of(view)
            {
                // Whatever the certificate turns out to be, the address has
                // answered over TLS, so there is nothing to retry in the open.
                forget_fallback(id);
            }
            // The system's own check decides. A certificate it turns down
            // comes back as a failure, and there is no way here to wave one
            // through.
            answer.call((
                NSURLSessionAuthChallengeDisposition::PerformDefaultHandling,
                std::ptr::null(),
            ));
        }
    }
);

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
                .map(|text| text.to_string())
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

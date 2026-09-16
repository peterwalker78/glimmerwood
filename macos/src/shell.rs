//! The window, and the engines inside it.
//!
//! The toolbar runs across the top, the tab column down the left and the tab
//! in front fills the rest, each its own WKWebView. The toolbar and the
//! column are Glimmerwood's own chrome and the only ones given a handler for
//! `glimmerwood://`; the tabs are the web, and each keeps its own engine
//! whether or not it is the one being looked at.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::rc::Rc;

use block2::{DynBlock, RcBlock};
use glimmerwood_core::companion::Companion;
use glimmerwood_core::dose::{Mode, Trend};
use glimmerwood_core::downloads::Progress;
use glimmerwood_core::failure::{self, Reason};
use glimmerwood_core::find;
use glimmerwood_core::host;
use glimmerwood_core::nav;
use glimmerwood_core::pages;
use glimmerwood_core::protocol::{ChromeView, Security, ToChrome, ToCore};
use glimmerwood_core::tabs::{COLUMN_DEFAULT, Tabs, column_width};
use glimmerwood_core::zoom;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{AllocAnyThread, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSCursor, NSEvent,
    NSEventMask, NSEventModifierFlags, NSEventType, NSMenu, NSMenuItem, NSMenuItemValidation,
    NSResponder, NSView, NSWindow, NSWindowDelegate, NSWindowOrderingMode, NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSData, NSError, NSFileManager, NSInteger, NSJSONSerialization,
    NSJSONWritingOptions, NSNotification, NSPoint, NSProgressReporting, NSRect,
    NSSearchPathDirectory, NSSearchPathDomainMask, NSSize, NSString, NSTimer, NSURL,
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
    WKDownload, WKDownloadDelegate, WKFindConfiguration, WKFindResult, WKNavigation,
    WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate, WKNavigationResponse,
    WKNavigationResponsePolicy, WKScriptMessage, WKScriptMessageHandler, WKURLSchemeHandler,
    WKURLSchemeTask, WKUserContentController, WKWebView, WKWebViewConfiguration,
};

use crate::nook;

/// The size the window opens at before it has been left anywhere to return
/// to, and before the chrome says otherwise.
const INITIAL_WIDTH: f64 = 1100.0;
const INITIAL_HEIGHT: f64 = 760.0;
/// The toolbar's height until it reports its own.
const TOOLBAR_HEIGHT: f64 = 56.0;
/// The grab area along the column's edge: a hairline to look at, wide
/// enough to take hold of. How wide the column itself may be is the core's
/// word, and the width it is left at is kept between runs.
const GRIP_WIDTH: f64 = 6.0;
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
    /// The event monitor that notices a person is here, held for as long as
    /// the app runs.
    static WATCHING: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
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
    /// The page saying an address couldn't be opened is on its way in, and
    /// then is what the tab is showing. Where a tab got to is written down;
    /// where it didn't get to is not.
    failing: Cell<bool>,
    failed: Cell<bool>,
}

/// Where the toolbar says the wisp's nook belongs, measured from the top
/// right of the window, and how big it is.
#[derive(Clone, Copy)]
struct NookAt {
    right: f64,
    top: f64,
    width: f64,
    height: f64,
}

/// A hold on the column's edge: how far the pointer was from the edge when
/// it took hold, so the edge doesn't jump under it, and how wide the column
/// was then, so a press that lets go without moving changes nothing.
#[derive(Clone, Copy)]
struct Grab {
    offset: f64,
    was: f64,
}

/// The nook's corner until the toolbar reports its own.
const INITIAL_NOOK: NookAt = NookAt {
    right: 0.0,
    top: 0.0,
    width: 152.0,
    height: TOOLBAR_HEIGHT,
};

pub struct Shell {
    window: Retained<NSWindow>,
    pub(crate) toolbar: Retained<WKWebView>,
    pub(crate) sidebar: Retained<WKWebView>,
    /// The engines, in the order the column shows them. A tab restored from
    /// the last run has none until it is asked for.
    engines: RefCell<Vec<Engine>>,
    /// Which tabs there are and which one is in front. The bookkeeping is the
    /// core's, so it is the same here as it is anywhere else.
    tabs: RefCell<Tabs>,
    toolbar_height: Cell<f64>,
    /// How wide the tab column is, the view over its edge that sets it, and
    /// the hold the pointer has on that edge while one is under way.
    column: Cell<f64>,
    grip: Retained<Grip>,
    grab: Cell<Option<Grab>>,
    nook_at: Cell<NookAt>,
    /// Whether the find bar is open, and what was last typed into it.
    finding: Cell<bool>,
    find_query: RefCell<String>,
    /// How large each site is drawn. The level belongs to the site, so it is
    /// the window's rather than a tab's.
    zooms: RefCell<zoom::Zooms>,
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
    let grip: Retained<Grip> = unsafe {
        msg_send![
            Grip::alloc(mtm),
            initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(GRIP_WIDTH, 1.0))
        ]
    };

    if let Some(content) = window.contentView() {
        content.addSubview(&toolbar);
        content.addSubview(&sidebar);
        // The column's edge goes over everything else in the window, so it
        // is what the pointer finds along the join.
        content.addSubview_positioned_relativeTo(&grip, NSWindowOrderingMode::Above, None);
        // The wisp is drawn over the toolbar's own corner, so its view goes
        // in front of the toolbar's.
        nook::open(
            &content,
            &toolbar,
            NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(INITIAL_NOOK.width, INITIAL_NOOK.height),
            ),
            mtm,
        );
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
        column: Cell::new(f64::from(COLUMN_DEFAULT)),
        grip,
        grab: Cell::new(None),
        nook_at: Cell::new(INITIAL_NOOK),
        finding: Cell::new(false),
        find_query: RefCell::new(String::new()),
        zooms: RefCell::new(zoom::Zooms::new()),
        mtm,
    });
    SHELL.with_borrow_mut(|held| *held = Some(shell.clone()));
    if let Some(path) = zooms_path() {
        shell.zooms.replace(zoom::Zooms::load(&path));
    }
    // The window goes back where it was left, and the column to the width
    // it was dragged to, before anything is laid out inside it. Where that
    // is written down is only known once the shell is held, since it is the
    // shell that says where this machine keeps such things.
    let saved = saved_window();
    if let Some(column) = saved.column {
        shell.column.set(column);
    }
    match saved.frame {
        Some(frame) => window.setFrame_display(frame, false),
        None => window.center(),
    }
    let keeper: Retained<Keeper> = unsafe { msg_send![Keeper::alloc(mtm), init] };
    window.setDelegate(Some(&ProtocolObject::from_retained(keeper.clone())));
    KEEPER.with_borrow_mut(|held| *held = Some(keeper));
    build_the_menu(&app, mtm);

    // The companion decides how the time is going. It is the same one the
    // Linux build uses; this only tells it what is on screen. It is started
    // before the first tab because it is what remembers which tabs there
    // were.
    let started = Companion::new(shell as Rc<dyn host::Host>, None);
    COMPANION.with_borrow_mut(|held| *held = Some(started.clone()));
    // What was open goes back in the column, asleep, and Home opens in front
    // of it as it does on any other run.
    restore_the_session(&started);
    open_tab(HOME, true);
    watch_the_engines();
    watch_for_a_person();
    lay_out();
    started.windows_changed();

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
        let column = shell.column.get().min(whole.size.width);
        let below = (whole.size.height - split).max(0.0);
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
        // The edge between the two, over a few points of each.
        let edge = NSRect::new(
            NSPoint::new(column - GRIP_WIDTH / 2.0, 0.0),
            NSSize::new(GRIP_WIDTH, below.max(1.0)),
        );
        if shell.grip.frame() != edge {
            shell.grip.setFrame(edge);
            // What the pointer is told is in the window's own points, so it
            // is stale as soon as the edge moves.
            if let Some(window) = shell.grip.window() {
                window.invalidateCursorRectsForView(&shell.grip);
            }
        }
        // The nook is measured from the top right, so it stays in its corner
        // as the window resizes.
        let at = shell.nook_at.get();
        nook::move_to(NSRect::new(
            NSPoint::new(
                (whole.size.width - at.right - at.width).max(0.0),
                (whole.size.height - at.top - at.height).max(0.0),
            ),
            NSSize::new(at.width.max(1.0), at.height.max(1.0)),
        ));
    });
}

// --- The column's edge -------------------------------------------------------

/// Where the pointer is across the window, in the content view's own points,
/// which is what the column's width is measured in.
fn pointer_x(grip: &Grip, event: &NSEvent) -> Option<f64> {
    let content = unsafe { grip.superview() }?;
    Some(
        content
            .convertPoint_fromView(event.locationInWindow(), None)
            .x,
    )
}

/// The pointer has taken hold of the edge.
fn take_hold(grip: &Grip, event: &NSEvent) {
    let Some(shell) = held() else { return };
    let Some(x) = pointer_x(grip, event) else {
        return;
    };
    let was = shell.column.get();
    shell.grab.set(Some(Grab {
        offset: was - x,
        was,
    }));
}

/// The column follows the pointer while it is held, within the limits the
/// core sets. The window is laid out at each step, so what the user sees
/// while dragging is where they are putting the edge.
fn drag_the_edge(grip: &Grip, event: &NSEvent) {
    let Some(shell) = held() else { return };
    let Some(grab) = shell.grab.get() else { return };
    let Some(x) = pointer_x(grip, event) else {
        return;
    };
    let wanted = f64::from(column_width((x + grab.offset).round() as i32));
    if shell.column.get() != wanted {
        shell.column.set(wanted);
        lay_out();
    }
}

/// The pointer has let go. A width somebody chose is written down; a press
/// that never moved the edge chose nothing, and leaves the file as it was.
fn let_go() {
    let Some(shell) = held() else { return };
    let Some(grab) = shell.grab.replace(None) else {
        return;
    };
    if shell.column.get() != grab.was {
        remember_window();
    }
}

define_class!(
    /// The edge between the tab column and the page: nothing is drawn into
    /// it, and dragging it says how much of the window the column takes.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GlimmerwoodGrip"]
    struct Grip;

    unsafe impl NSObjectProtocol for Grip {}

    impl Grip {
        /// The column and the page are drawn right up to the join; this only
        /// answers the pointer there.
        #[unsafe(method(isOpaque))]
        fn is_opaque(&self) -> bool {
            false
        }

        /// The pointer says what it can do here before anything is pressed.
        #[unsafe(method(resetCursorRects))]
        fn reset_cursor_rects(&self) {
            // What replaced it wants macOS 15; this one is understood as far
            // back as the rest of the shell runs.
            #[allow(deprecated)]
            let cursor = NSCursor::resizeLeftRightCursor();
            self.addCursorRect_cursor(self.bounds(), &cursor);
        }

        /// A window that wasn't in front can be dragged straight away,
        /// rather than the first press only bringing it forward.
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            take_hold(self, event);
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            drag_the_edge(self, event);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, _event: &NSEvent) {
            let_go();
        }
    }
);

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
    send(id, nav::resolve(input));
}

/// The same, for the address field submitted with Cmd held: a bare word is
/// the `.com` of that name.
fn navigate_dot_com(id: u32, input: &str) {
    send(id, nav::dot_com(input));
}

fn send(id: u32, target: Option<nav::Target>) {
    let Some(shell) = held() else { return };
    let Some(view) = engine(id) else { return };
    let Some(target) = target else { return };
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

/// A page saying an address couldn't be opened is about to be loaded into
/// this tab.
fn expect_failure(id: u32) {
    if let Some(shell) = held()
        && let Some(engine) = shell.engines.borrow().iter().find(|engine| engine.id == id)
    {
        engine.failing.set(true);
    }
}

/// Whether the page that just committed is one of those, and remembers the
/// answer for as long as the tab is showing it.
fn committing_failure(id: u32) -> bool {
    let Some(shell) = held() else { return false };
    let engines = shell.engines.borrow();
    let Some(engine) = engines.iter().find(|engine| engine.id == id) else {
        return false;
    };
    let failure = engine.failing.replace(false);
    engine.failed.set(failure);
    failure
}

fn showing_failure(id: u32) -> bool {
    held().is_some_and(|shell| {
        shell
            .engines
            .borrow()
            .iter()
            .any(|engine| engine.id == id && engine.failed.get())
    })
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
    let id = shell.tabs.borrow_mut().open(select);
    let view = give_an_engine(&shell, id);
    load(&view, uri);
    lay_out();
    push_tabs();
    if select {
        push_state();
    }
    keep_the_session();
    Some(id)
}

/// A tab restored from the last run is a row in the column and nothing more
/// until it is asked for. This is the moment it becomes a page: an engine of
/// its own, pointed at the address it has been holding.
fn wake_tab(id: u32) {
    let Some(shell) = held() else { return };
    if !shell.tabs.borrow_mut().wake(id) {
        return;
    }
    let uri = shell
        .tabs
        .borrow()
        .facts(id)
        .map(|facts| facts.uri.clone())
        .unwrap_or_default();
    let view = give_an_engine(&shell, id);
    draw_at_remembered_size(&view);
    load(&view, &uri);
}

/// Builds a tab's engine and puts it in beside the tab, so the engines stay
/// in the order the column shows them.
fn give_an_engine(shell: &Rc<Shell>, id: u32) -> Retained<WKWebView> {
    let view = make_webview(shell.mtm, false);
    if let Some(content) = shell.window.contentView() {
        // Under the column's edge, which goes on answering the pointer over
        // the page as tabs come and go.
        content.addSubview_positioned_relativeTo(
            &view,
            NSWindowOrderingMode::Below,
            Some(&shell.grip),
        );
    }
    let engine = Engine {
        id,
        view: view.clone(),
        fallback: RefCell::new(None),
        failing: Cell::new(false),
        failed: Cell::new(false),
    };
    let order = shell.tabs.borrow().ids();
    let place = |id: u32| order.iter().position(|other| *other == id);
    let mut engines = shell.engines.borrow_mut();
    let at = engines
        .iter()
        .position(|other| place(other.id) > place(id))
        .unwrap_or(engines.len());
    engines.insert(at, engine);
    view
}

fn select_tab(id: u32) {
    let Some(shell) = held() else { return };
    if shell.tabs.borrow().selected() != Some(id) {
        close_find(false);
    }
    if !shell.tabs.borrow_mut().select(id) {
        return;
    }
    wake_tab(id);
    lay_out();
    push_tabs();
    push_state();
    focus_tab(id);
    keep_the_session();
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
        Some(next) => {
            // The tab that takes its place may be one still asleep.
            wake_tab(next);
            lay_out();
            push_tabs();
            if was_selected {
                push_state();
            }
            keep_the_session();
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
    let mut moved = false;
    let selected = shell.tabs.borrow().selected();
    for (id, view) in engines {
        let uri = uri_of(&view);
        let title = unsafe { view.title() }
            .map(|text| text.to_string())
            .unwrap_or_default();
        let loading = unsafe { view.isLoading() };
        let (was_uri, was_title) = shell
            .tabs
            .borrow()
            .facts(id)
            .map(|facts| (facts.uri.clone(), facts.title.clone()))
            .unwrap_or_default();
        // A tab just woken holds the address it is on its way to, which the
        // engine has nothing to say about until the load starts.
        let holding = uri.is_empty() && !was_uri.is_empty();
        let elsewhere = !holding && (uri != was_uri || title != was_title);
        if shell.tabs.borrow_mut().update(id, |facts| {
            if !holding {
                facts.uri = uri.clone();
                facts.title = title.clone();
            }
            facts.loading = loading;
        }) {
            changed = true;
            front_changed |= Some(id) == selected;
            if elsewhere {
                // A page that names itself once it is up, or a move within
                // one that never became a navigation: either way the tab is
                // somewhere the delegate never heard about.
                note_visit(id, &uri, &title);
                moved = true;
            }
        }
        // Glimmerwood's own pages wear the wisp; anything else keeps the
        // letter the column draws from its site.
        let mark = pages::is_local_page(&uri).then(wisp_icon);
        if shell
            .tabs
            .borrow()
            .facts(id)
            .map(|facts| facts.icon.clone())
            != Some(mark.clone())
        {
            set_icon(id, mark);
        }
    }
    if changed {
        push_tabs();
    }
    if moved {
        keep_the_session();
    }
    if front_changed {
        push_state();
    }
}

/// Write down where a tab has got to. The core keeps its own counsel about
/// what is worth remembering and what is nobody's business; what is held
/// back here is the page that says an address couldn't be opened, which is
/// not somewhere anyone went.
fn note_visit(id: u32, uri: &str, title: &str) {
    if uri.is_empty() || showing_failure(id) {
        return;
    }
    if let Some(companion) = companion() {
        companion.visited(uri, title);
    }
    // The session keeps titles, and a page often names itself after it has
    // loaded; a restored tab should come back as its title.
    keep_the_session();
}

/// The tabs as they stand, for next time. Written whenever they change and
/// on the way out, so a crash costs at most the last move.
fn keep_the_session() {
    let Some(shell) = held() else { return };
    let Some(companion) = companion() else { return };
    let session = shell.tabs.borrow().session();
    companion.keep_session(&session);
}

/// What was open last time, as rows in the column and nothing more. Home is
/// opened in front of them afterwards, as it is on any other run.
fn restore_the_session(companion: &Companion) {
    let Some(shell) = held() else { return };
    for sleeper in companion.last_session().tabs {
        shell
            .tabs
            .borrow_mut()
            .open_asleep(&sleeper.url, &sleeper.title);
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

/// The pointer entered or left the wisp, which the toolbar shows a caption
/// for. The wisp is drawn over the toolbar, so the hover is heard here.
pub(crate) fn hovering_wisp(open: bool) {
    tell_the_chrome(&ToChrome::Caption { open });
}

/// The wisp was clicked.
pub(crate) fn wisp_clicked() {
    show_wisp();
}

/// Glimmerwood's own mark, for the tabs showing Home or Settings. WebKit
/// offers no way to ask a site for its icon, so every other tab keeps the
/// letter the column draws for it.
fn wisp_icon() -> String {
    thread_local! {
        static ICON: String = match file("home/wisp.svg") {
            Some(svg) => format!(
                "data:image/svg+xml;base64,{}",
                base64_of(svg),
            ),
            None => String::new(),
        };
    }
    ICON.with(Clone::clone)
}

/// Enough base64 for one small file.
fn base64_of(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let mut three = [0u8; 3];
        three[..chunk.len()].copy_from_slice(chunk);
        let bits = u32::from_be_bytes([0, three[0], three[1], three[2]]);
        for place in 0..4 {
            if place <= chunk.len() {
                out.push(ALPHABET[(bits >> (18 - 6 * place) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Every mark again, for a column that has just come back.
fn send_the_icons() {
    let Some(shell) = held() else { return };
    let marks: Vec<(u32, Option<String>)> = shell
        .tabs
        .borrow()
        .ids()
        .into_iter()
        .map(|id| {
            let icon = shell
                .tabs
                .borrow()
                .facts(id)
                .and_then(|facts| facts.icon.clone());
            (id, icon)
        })
        .collect();
    for (id, icon) in marks {
        tell_the_column(&ToChrome::TabIcon { id, icon });
    }
}

/// The mark a tab wears, told to the column when it changes.
fn set_icon(id: u32, icon: Option<String>) {
    let Some(shell) = held() else { return };
    let changed = shell
        .tabs
        .borrow_mut()
        .update(id, |facts| facts.icon = icon.clone());
    if changed {
        tell_the_column(&ToChrome::TabIcon { id, icon });
    }
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

/// The companion needs to know someone is there. What it is told is that
/// there was input, never what the input was: a monitor that sees every
/// event in this app, and passes every one of them straight on.
fn watch_for_a_person() {
    let mask = NSEventMask::KeyDown
        | NSEventMask::LeftMouseDown
        | NSEventMask::RightMouseDown
        | NSEventMask::OtherMouseDown
        | NSEventMask::ScrollWheel
        | NSEventMask::MouseMoved;
    let noticed = RcBlock::new(|event: NonNull<NSEvent>| -> *mut NSEvent {
        if let Some(companion) = companion() {
            companion.input();
            // A key press also holds the wisp's question back: it never asks
            // anything mid-sentence.
            let kind = unsafe { event.as_ref().r#type() };
            if kind == NSEventType::KeyDown {
                companion.typed();
            }
        }
        event.as_ptr()
    });
    let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &noticed) };
    WATCHING.with_borrow_mut(|held| *held = monitor);
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
        ToCore::NavigateDotCom { input } => {
            let selected = shell.tabs.borrow().selected();
            if let Some(id) = selected {
                navigate_dot_com(id, &input);
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
        ToCore::OpenDownload { id } => open_download(id),
        ToCore::ShowWisp => show_wisp(),
        ToCore::OpenSettings => open_settings(),
        ToCore::FindSupport { samaritans } => {
            open_tab(if samaritans { SAMARITANS } else { HELPLINES }, true);
        }
        ToCore::Find { query } => find(query),
        ToCore::FindNext { backwards } => find_next(backwards),
        ToCore::CloseFind => close_find(true),
        ToCore::ToolbarLayout {
            height,
            nook_right,
            nook_top,
            nook_width,
            nook_height,
            ..
        } => {
            shell.toolbar_height.set(f64::from(height));
            shell.nook_at.set(NookAt {
                right: f64::from(nook_right),
                top: f64::from(nook_top),
                width: f64::from(nook_width),
                height: f64::from(nook_height),
            });
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
        } => {
            push_tabs();
            send_the_icons();
        }
        // What is left belongs to a window that floats, which this shell
        // doesn't have: the title bar is the system's.
        _ => {}
    }
    refresh_tabs();
}

// --- How large a site is drawn -----------------------------------------------

/// A step larger, a step smaller, or back to plain. The level belongs to the
/// site, so every tab showing it follows.
fn zoom(by: isize) {
    let Some(shell) = held() else { return };
    let Some(view) = selected_view() else { return };
    let uri = uri_of(&view);
    {
        let mut zooms = shell.zooms.borrow_mut();
        if by == 0 {
            zooms.reset(&uri);
        } else {
            zooms.step(&uri, by);
        }
    }
    let host = nav::host_of(&uri);
    let views: Vec<Retained<WKWebView>> = shell
        .engines
        .borrow()
        .iter()
        .map(|engine| engine.view.clone())
        .collect();
    for view in views {
        if nav::host_of(&uri_of(&view)) == host {
            draw_at_remembered_size(&view);
        }
    }
    let saved = zooms_path().map_or(Ok(()), |path| {
        path.parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .map_err(|err| err.to_string())
            .and_then(|()| shell.zooms.borrow().save(&path))
    });
    if let Err(err) = saved {
        eprintln!("glimmerwood: couldn't remember how large {host} is drawn: {err}");
    }
}

/// Draw a tab at the level its site is remembered at.
fn draw_at_remembered_size(view: &WKWebView) {
    let Some(shell) = held() else { return };
    let level = shell.zooms.borrow().of(&uri_of(view));
    unsafe { view.setPageZoom(level) };
}

// --- Find in page ------------------------------------------------------------

/// Open the find bar, with the keyboard in it.
fn open_find() {
    let Some(shell) = held() else { return };
    if selected_view().is_none() {
        return;
    }
    shell.finding.set(true);
    shell.window.makeFirstResponder(Some(&shell.toolbar));
    tell_the_chrome(&ToChrome::Find { open: true });
}

/// Search the page on screen from the top, ignoring case and wrapping round
/// at the end. An empty query clears what was found.
fn find(query: String) {
    let Some(shell) = held() else { return };
    let Some(view) = selected_view() else { return };
    shell.find_query.replace(query.clone());
    if query.is_empty() {
        clear_matches(&view);
        tell_the_chrome(&ToChrome::Found {
            query,
            summary: String::new(),
        });
        return;
    }
    search(&view, &query, false);
}

/// The next match, or the one before it. With the bar closed, it opens.
fn find_next(backwards: bool) {
    let Some(shell) = held() else { return };
    let Some(view) = selected_view() else { return };
    let query = shell.find_query.borrow().clone();
    if !shell.finding.get() || query.is_empty() {
        open_find();
        return;
    }
    search(&view, &query, backwards);
}

/// Clear what was found and close the bar. `to_page`: the user closed it, so
/// the keyboard goes back to the page.
fn close_find(to_page: bool) {
    let Some(shell) = held() else { return };
    if !shell.finding.replace(false) {
        return;
    }
    shell.find_query.borrow_mut().clear();
    if let Some(view) = selected_view() {
        clear_matches(&view);
        if to_page {
            shell.window.makeFirstResponder(Some(&view));
        }
    }
    tell_the_chrome(&ToChrome::Find { open: false });
}

/// Ask the engine for `query`. WebKit searches from wherever the last match
/// was, so asking again is what moves on to the next one.
fn search(view: &WKWebView, query: &str, backwards: bool) {
    let Some(shell) = held() else { return };
    let configuration = unsafe { WKFindConfiguration::new(shell.mtm) };
    unsafe {
        configuration.setBackwards(backwards);
        configuration.setCaseSensitive(false);
        configuration.setWraps(true);
    }
    let asked = query.to_owned();
    let answer = RcBlock::new(move |result: NonNull<WKFindResult>| {
        // Safety: WebKit hands the result to this block and keeps it alive
        // for the call.
        let found = unsafe { result.as_ref().matchFound() };
        report(&asked, found);
    });
    unsafe {
        view.findString_withConfiguration_completionHandler(
            &NSString::from_str(query),
            Some(&configuration),
            &answer,
        );
    }
}

/// What the bar is told about a search. WebKit says whether it found
/// anything and not how much, so a search that found something is reported
/// with nothing to say rather than with a count it hasn't counted. A late
/// answer to a search the user has already typed past is dropped.
fn report(query: &str, found: bool) {
    let Some(shell) = held() else { return };
    if !shell.finding.get() || *shell.find_query.borrow() != query {
        return;
    }
    tell_the_chrome(&ToChrome::Found {
        query: query.to_owned(),
        summary: if found {
            String::new()
        } else {
            find::summary(None)
        },
    });
}

/// Take what was found off the page. WebKit's find leaves the match selected
/// and offers no way to undo that, so the selection is what goes.
fn clear_matches(view: &WKWebView) {
    let script = NSString::from_str("window.getSelection()?.removeAllRanges()");
    unsafe { view.evaluateJavaScript_completionHandler(&script, None) };
}

// --- Files arriving ----------------------------------------------------------

/// A file on its way in: the number the companion knows it by, the download
/// itself so its progress can be read, and where it is being written.
struct Arriving {
    id: u32,
    download: Retained<WKDownload>,
    path: String,
}

thread_local! {
    /// A download points at its delegate weakly, so the one they all share is
    /// kept here.
    static SAVER: RefCell<Option<Retained<Saver>>> = const { RefCell::new(None) };
    /// The files still arriving.
    static ARRIVING: RefCell<Vec<Arriving>> = const { RefCell::new(Vec::new()) };
}

fn saver(mtm: MainThreadMarker) -> Retained<Saver> {
    SAVER.with_borrow_mut(|held| {
        held.get_or_insert_with(|| unsafe { msg_send![Saver::alloc(mtm), init] })
            .clone()
    })
}

/// A navigation turned out to be a file. From here it answers to the saver
/// rather than to the page it came from.
fn take_over(download: &WKDownload, mtm: MainThreadMarker) {
    let saver = ProtocolObject::from_retained(saver(mtm));
    unsafe { download.setDelegate(Some(&saver)) };
}

/// Where files are saved. macOS keeps a folder for this and will say where
/// it is even if it has been moved.
fn downloads_dir() -> PathBuf {
    NSFileManager::defaultManager()
        .URLsForDirectory_inDomains(
            NSSearchPathDirectory::DownloadsDirectory,
            NSSearchPathDomainMask::UserDomainMask,
        )
        .firstObject()
        .and_then(|url| url.path())
        .map(|path| PathBuf::from(path.to_string()))
        .unwrap_or_else(|| match held() {
            Some(shell) => host::Host::home_dir(&*shell).join("Downloads"),
            None => PathBuf::from("."),
        })
}

/// The name to save under: what the site suggested, with anything that would
/// take the file somewhere other than the folder it is meant for taken out.
fn file_name(suggested: &str) -> String {
    let name: String = suggested
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .chars()
        .filter(|c| *c != ':' && !c.is_control())
        .collect();
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." {
        "download".to_owned()
    } else {
        name.to_owned()
    }
}

/// Where the file goes, never over one that is already there: `moss.pdf`,
/// then `moss (2).pdf`, and so on up to the point where it is plainly not
/// working.
fn free_path(dir: &Path, name: &str) -> Option<PathBuf> {
    let first = dir.join(name);
    if !first.exists() {
        return Some(first);
    }
    let (stem, extension) = match name.rfind('.') {
        Some(at) if at > 0 => (&name[..at], &name[at..]),
        _ => (name, ""),
    };
    (2..100)
        .map(|n| dir.join(format!("{stem} ({n}){extension}")))
        .find(|path| !path.exists())
}

/// The number a download is known by, and where it is being written, taken
/// off the list now that it has stopped arriving.
fn arrived(download: &WKDownload) -> Option<(u32, String)> {
    ARRIVING.with_borrow_mut(|list| {
        let at = list
            .iter()
            .position(|arriving| &*arriving.download == download)?;
        let arriving = list.remove(at);
        Some((arriving.id, arriving.path))
    })
}

/// How far along each file is. WebKit keeps a progress object on a download
/// and changes it as bytes land; this reads it on the watch timer rather
/// than observing it, which is often enough for a bar that moves in steps.
fn report_progress() {
    let Some(companion) = companion() else { return };
    let along: Vec<(u32, Option<f64>)> = ARRIVING.with_borrow(|list| {
        list.iter()
            .map(|arriving| {
                let progress = arriving.download.progress();
                let known = progress.totalUnitCount() > 0 && !progress.isIndeterminate();
                (arriving.id, known.then(|| progress.fractionCompleted()))
            })
            .collect()
    });
    for (id, fraction) in along {
        companion.download_progressed(id, fraction);
    }
}

/// Hand a file that has finished arriving to the system, which takes it off
/// Home: it has been dealt with.
fn open_download(id: u32) {
    let Some(shell) = held() else { return };
    let Some(companion) = companion() else { return };
    if let Some(path) = companion.open_download(id)
        && !host::Host::open_file(&*shell, &path)
    {
        eprintln!("glimmerwood: nothing here opens {path}");
    }
}

/// How WebKit is told where to put a file: a place for it, or nothing,
/// which turns the download down.
type Destination = DynBlock<dyn Fn(*mut NSURL)>;

define_class!(
    /// What becomes of a file being saved. There is no window for this: a
    /// download is a file arriving on the machine, so it goes where files
    /// go, and the mark in the toolbar and the line on Home are all that is
    /// said about it.
    ///
    /// The method that answers through a block is written out by hand, so the
    /// conformance below declares none of its own.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GlimmerwoodSaver"]
    struct Saver;

    unsafe impl NSObjectProtocol for Saver {}

    unsafe impl WKDownloadDelegate for Saver {}

    impl Saver {
        #[unsafe(method(download:decideDestinationUsingResponse:suggestedFilename:completionHandler:))]
        fn decide_destination(
            &self,
            download: &WKDownload,
            _response: &NSURLResponse,
            suggested: &NSString,
            answer: &Destination,
        ) {
            let dir = downloads_dir();
            let name = file_name(&suggested.to_string());
            let where_to = std::fs::create_dir_all(&dir)
                .ok()
                .and_then(|()| free_path(&dir, &name));
            // Answering with nowhere is how a download is turned down, and
            // nowhere to put it is the only reason to turn one down here.
            let (Some(where_to), Some(companion)) = (where_to, companion()) else {
                answer.call((std::ptr::null_mut(),));
                return;
            };
            let shown = where_to
                .file_name()
                .map_or(name, |name| name.to_string_lossy().into_owned());
            let path = where_to.to_string_lossy().into_owned();
            let id = companion.download_started(&shown, &path);
            ARRIVING.with_borrow_mut(|list| {
                list.push(Arriving {
                    id,
                    download: download.retain(),
                    path: path.clone(),
                });
            });
            let url = NSURL::fileURLWithPath(&NSString::from_str(&path));
            answer.call((Retained::as_ptr(&url).cast_mut(),));
        }

        #[unsafe(method(downloadDidFinish:))]
        fn finished(&self, download: &WKDownload) {
            if let Some((id, path)) = arrived(download)
                && let Some(companion) = companion()
            {
                companion.download_finished(id, Progress::Saved, Some(&path));
            }
        }

        #[unsafe(method(download:didFailWithError:resumeData:))]
        fn failed(&self, download: &WKDownload, error: &NSError, _resume: Option<&NSData>) {
            let stopped = error.code() == NSURLErrorCancelled;
            if let Some((id, _)) = arrived(download)
                && let Some(companion) = companion()
            {
                let how = if stopped {
                    Progress::Stopped
                } else {
                    Progress::Failed
                };
                companion.download_finished(id, how, None);
            }
        }
    }
);

// --- The menu bar ------------------------------------------------------------

/// The arrow keys and Escape, as a menu item spells them.
const LEFT_ARROW: &str = "\u{f702}";
const RIGHT_ARROW: &str = "\u{f703}";
const ESCAPE: &str = "\u{1b}";
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
    edit.addItem(&NSMenuItem::separatorItem(mtm));
    let finding = submenu(&edit, mtm, "Find");
    finding.addItem(&item(mtm, "Find…", sel!(openFind:), "f", command, target));
    finding.addItem(&item(
        mtm,
        "Find Next",
        sel!(findNext:),
        "g",
        command,
        target,
    ));
    finding.addItem(&item(
        mtm,
        "Find Previous",
        sel!(findPrevious:),
        "g",
        shifted,
        target,
    ));
    // Escape closes the bar, and only while it is open: the rest of the time
    // the key belongs to the page or to the address field.
    spare_key(
        &finding,
        mtm,
        sel!(closeFind:),
        ESCAPE,
        NSEventModifierFlags::empty(),
        target,
    );

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
    view.addItem(&item(mtm, "Zoom In", sel!(zoomIn:), "+", command, target));
    // The key beside the minus, which is what a hand reaches for and what
    // the plus above needs Shift for.
    spare_key(&view, mtm, sel!(zoomIn:), "=", command, target);
    view.addItem(&item(mtm, "Zoom Out", sel!(zoomOut:), "-", command, target));
    view.addItem(&item(
        mtm,
        "Actual Size",
        sel!(zoomPlain:),
        "0",
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

/// A menu of `title` hung off `parent`: off the bar, or off another menu.
fn submenu(parent: &NSMenu, mtm: MainThreadMarker, title: &str) -> Retained<NSMenu> {
    let holder = NSMenuItem::new(mtm);
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title));
    holder.setSubmenu(Some(&menu));
    parent.addItem(&holder);
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

        #[unsafe(method(openFind:))]
        fn open_the_find_bar(&self, _sender: Option<&AnyObject>) {
            open_find();
        }

        #[unsafe(method(findNext:))]
        fn find_the_next(&self, _sender: Option<&AnyObject>) {
            find_next(false);
        }

        #[unsafe(method(findPrevious:))]
        fn find_the_previous(&self, _sender: Option<&AnyObject>) {
            find_next(true);
        }

        #[unsafe(method(closeFind:))]
        fn close_the_find_bar(&self, _sender: Option<&AnyObject>) {
            close_find(true);
        }

        #[unsafe(method(zoomIn:))]
        fn zoom_in(&self, _sender: Option<&AnyObject>) {
            zoom(1);
        }

        #[unsafe(method(zoomOut:))]
        fn zoom_out(&self, _sender: Option<&AnyObject>) {
            zoom(-1);
        }

        #[unsafe(method(zoomPlain:))]
        fn zoom_plain(&self, _sender: Option<&AnyObject>) {
            zoom(0);
        }

        /// Quitting doesn't close the window, so its size and place and the
        /// tabs it holds are written down on the way out.
        #[unsafe(method(quitGlimmerwood:))]
        fn quit(&self, _sender: Option<&AnyObject>) {
            remember_window();
            keep_the_session();
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }
    }

    /// Escape belongs to the find bar only while it is open. An item its
    /// target turns down doesn't take the key, so it goes on to the page or
    /// the address field as it did before.
    unsafe impl NSMenuItemValidation for Menus {
        #[unsafe(method(validateMenuItem:))]
        fn validate(&self, item: &NSMenuItem) -> bool {
            item.action() != Some(sel!(closeFind:))
                || held().is_some_and(|shell| shell.finding.get())
        }
    }
);

// --- Where the window was last left ------------------------------------------

/// Where what the user set by hand is kept between runs.
fn config_dir() -> Option<PathBuf> {
    let shell = held()?;
    Some(host::Host::config_dir(&*shell).join("glimmerwood"))
}

/// Where the window's size and place, and how wide the tab column was left,
/// are kept.
fn window_path() -> Option<PathBuf> {
    Some(config_dir()?.join("window.ini"))
}

/// Where the per-site drawing sizes are kept, in the same plain form as the
/// Linux build's, so it can be edited by hand.
fn zooms_path() -> Option<PathBuf> {
    Some(config_dir()?.join("zoom.toml"))
}

/// How the window was left, as far as it was written down.
#[derive(Default)]
struct Saved {
    frame: Option<NSRect>,
    column: Option<f64>,
}

/// What the file says, a `key = value` to a line. Anything missing or
/// unreadable is simply something nobody has chosen yet, so a file from
/// before the column could be dragged gives the window back and leaves the
/// column at its default.
fn saved_window() -> Saved {
    let Some(text) = window_path().and_then(|path| std::fs::read_to_string(path).ok()) else {
        return Saved::default();
    };
    let mut saved = Saved::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let numbers: Vec<f64> = value
            .split_whitespace()
            .filter_map(|word| word.parse().ok())
            .collect();
        match (key.trim(), &numbers[..]) {
            ("frame", &[x, y, width, height]) if width > 0.0 && height > 0.0 => {
                saved.frame = Some(NSRect::new(NSPoint::new(x, y), NSSize::new(width, height)));
            }
            ("column", &[width]) => saved.column = Some(f64::from(column_width(width as i32))),
            _ => {}
        }
    }
    saved
}

/// Writes the window's size and place, and the width the column was left at,
/// down for next time.
fn remember_window() {
    let Some(shell) = held() else { return };
    let Some(path) = window_path() else { return };
    let frame = shell.window.frame();
    let lines = format!(
        "frame = {} {} {} {}\ncolumn = {}\n",
        frame.origin.x,
        frame.origin.y,
        frame.size.width,
        frame.size.height,
        column_width(shell.column.get().round() as i32),
    );
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&path, lines));
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
            remember_window();
            keep_the_session();
        }

        /// Nothing inside the window sizes itself, so a resize is laid out
        /// from here: the toolbar across the new width, the column and the
        /// tab below it, and the wisp's nook back in its corner.
        #[unsafe(method(windowDidResize:))]
        fn did_resize(&self, _notification: &NSNotification) {
            lay_out();
        }
    }
);

// --- What the engines report -------------------------------------------------

/// The callbacks a WebKit delegate method is handed to answer with.
type Decision = DynBlock<dyn Fn(WKNavigationActionPolicy)>;
type Answer = DynBlock<dyn Fn(WKNavigationResponsePolicy)>;
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
/// False if there was no page to show, in which case the tab is left with
/// whatever it had.
fn show_failure(view: &WKWebView, uri: &str, reason: &Reason) -> bool {
    let Some(template) = file("pages/failed.html").and_then(|bytes| str::from_utf8(bytes).ok())
    else {
        return false;
    };
    let html = NSString::from_str(&failure::fill(template, uri, reason));
    let base = NSURL::URLWithString(&NSString::from_str(uri));
    unsafe {
        let _ = view.loadHTMLString_baseURL(&html, base.as_deref());
    }
    true
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
    if show_failure(view, &uri, &reason)
        && let Some(id) = tab_of(view)
    {
        expect_failure(id);
    }
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
            // A link that asks to be saved rather than opened is a file
            // arriving, and WebKit hands it over once it is told so.
            if unsafe { action.shouldPerformDownload() } {
                answer.call((WKNavigationActionPolicy::Download,));
                return;
            }
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

        /// What comes back from an address WebKit has no way to draw is a
        /// file, and a file is something to save rather than a page that
        /// wouldn't open.
        #[unsafe(method(webView:decidePolicyForNavigationResponse:decisionHandler:))]
        fn decide_response(
            &self,
            _view: &WKWebView,
            response: &WKNavigationResponse,
            answer: &Answer,
        ) {
            let showable =
                unsafe { response.canShowMIMEType() || !response.isForMainFrame() };
            let policy = if showable {
                WKNavigationResponsePolicy::Allow
            } else {
                WKNavigationResponsePolicy::Download
            };
            answer.call((policy,));
        }

        #[unsafe(method(webView:navigationAction:didBecomeDownload:))]
        fn action_became_download(
            &self,
            _view: &WKWebView,
            _action: &WKNavigationAction,
            download: &WKDownload,
        ) {
            take_over(download, self.mtm());
        }

        #[unsafe(method(webView:navigationResponse:didBecomeDownload:))]
        fn response_became_download(
            &self,
            _view: &WKWebView,
            _response: &WKNavigationResponse,
            download: &WKDownload,
        ) {
            take_over(download, self.mtm());
        }

        #[unsafe(method(webView:didStartProvisionalNavigation:))]
        fn started(&self, view: &WKWebView, _navigation: Option<&WKNavigation>) {
            // The page the find bar was searching is on its way out.
            if tab_of(view).is_some_and(|id| {
                held().is_some_and(|shell| shell.tabs.borrow().selected() == Some(id))
            }) {
                close_find(false);
            }
            refresh_tabs();
        }

        #[unsafe(method(webView:didCommitNavigation:))]
        fn committed(&self, view: &WKWebView, _navigation: Option<&WKNavigation>) {
            if let Some(id) = tab_of(view) {
                forget_fallback(id);
                if !committing_failure(id) {
                    let title = unsafe { view.title() }
                        .map(|text| text.to_string())
                        .unwrap_or_default();
                    note_visit(id, &uri_of(view), &title);
                }
            }
            // As early as the level can be set, so the page is laid out at
            // the size its site is remembered at rather than resized once it
            // is up.
            draw_at_remembered_size(view);
            refresh_tabs();
        }

        #[unsafe(method(webView:didFinishNavigation:))]
        fn finished(&self, view: &WKWebView, _navigation: Option<&WKNavigation>) {
            if let Some(id) = tab_of(view) {
                push_page(id);
            }
            draw_at_remembered_size(view);
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
            // Both of the chrome's own views may ask for anything carried in
            // the binary — the toolbar and the tab column are each one of
            // Glimmerwood's pages. A tab is the web, and may ask only for the
            // two pages any tab is allowed to open.
            let ours =
                held().is_some_and(|shell| &*shell.toolbar == _view || &*shell.sidebar == _view);
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
            report_progress();
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
            .filter(|facts| !facts.asleep)
            .map(|facts| facts.uri.clone())
            .unwrap_or_default()
    }

    /// Every tab but the one being looked at. They weigh nothing, but the
    /// dose engine still wants to know they are open — and a tab restored
    /// from the last run has not been opened at all. It is a name in the
    /// column until someone asks for it, and nowhere anybody is.
    fn other_tabs(&self, in_front: bool) -> Vec<String> {
        let tabs = self.tabs.borrow();
        let attended = if in_front { tabs.selected() } else { None };
        tabs.ids()
            .into_iter()
            .filter(|id| Some(*id) != attended)
            .filter_map(|id| tabs.facts(id))
            .filter(|facts| !facts.asleep && !facts.uri.is_empty())
            .map(|facts| facts.uri.clone())
            .collect()
    }

    fn sound_on_screen(&self) -> bool {
        // WebKit tells an app this only through a private property, so until
        // there is a supported way to ask, sound is not counted here.
        false
    }

    fn send_to_chrome(&self, message: &ToChrome) {
        // The wisp is drawn here, not in the chrome, so it takes every change
        // of dose; the chrome only shows words.
        if let ToChrome::Wisp {
            dose,
            mode,
            trend,
            night,
            private,
            welcome,
            ..
        } = message
        {
            nook::update(
                *dose,
                Mode::from(*mode),
                Trend::from(*trend),
                *private,
                *night,
                *welcome,
            );
        }
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

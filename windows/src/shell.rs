//! The window, and the engines inside it.
//!
//! The toolbar runs across the top, the tab column down the left and the tab
//! in front fills the rest, each its own WebView2. The toolbar and the column
//! are Glimmerwood's own chrome and the only ones handed `glimmerwood://`;
//! the tabs are the web, and there is one engine per tab whether or not it
//! is the one on screen.

use std::cell::RefCell;
use std::collections::HashMap;
use std::error::Error;
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use crate::nook;
use glimmerwood_core::companion::Companion;
use glimmerwood_core::dose::{Mode, Trend};
use glimmerwood_core::failure::{self, Reason};
use glimmerwood_core::host;
use glimmerwood_core::nav;
use glimmerwood_core::pages;
use glimmerwood_core::protocol::{ChromeView, Security, ToChrome, ToCore};
use glimmerwood_core::tabs::Tabs;
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{
    AcceleratorKeyPressedEventHandler, CreateCoreWebView2ControllerCompletedHandler,
    CreateCoreWebView2EnvironmentCompletedHandler, DocumentTitleChangedEventHandler,
    FaviconChangedEventHandler, GetFaviconCompletedHandler,
    IsDocumentPlayingAudioChangedEventHandler, IsMutedChangedEventHandler,
    NavigationCompletedEventHandler, NavigationStartingEventHandler,
    NewWindowRequestedEventHandler, SourceChangedEventHandler, WebMessageReceivedEventHandler,
    WebResourceRequestedEventHandler,
};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, IStream,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VIRTUAL_KEY, VK_1, VK_9, VK_CONTROL, VK_D, VK_F4, VK_F5, VK_F6, VK_HOME, VK_L,
    VK_LEFT, VK_MENU, VK_NEXT, VK_OEM_COMMA, VK_PRIOR, VK_R, VK_RIGHT, VK_SHIFT, VK_T, VK_TAB,
    VK_W,
};
use windows::Win32::UI::Shell::SHCreateMemStream;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::BOOL;
use windows::core::{HSTRING, Interface, PCWSTR, PWSTR, w};

type Fallible<T> = std::result::Result<T, Box<dyn Error>>;

/// The window's size the first time it opens, before there is one to
/// remember, and the smallest it is ever restored to.
const INITIAL_WIDTH: i32 = 1100;
const INITIAL_HEIGHT: i32 = 760;
const LEAST_WIDTH: i32 = 480;
const LEAST_HEIGHT: i32 = 360;
/// The toolbar's height until it reports its own.
const TOOLBAR_HEIGHT: i32 = 56;
/// The tab column's width. The GTK build lets it be dragged and remembers
/// where; here it is the width that shows a tab's mark and nothing else.
const COLUMN_WIDTH: i32 = 48;

/// Where a new tab starts, and where the chrome's buttons point.
const HOME: &str = "glimmerwood://home/";
const HOME_WISP: &str = "glimmerwood://home/#wisp";
const SETTINGS: &str = "glimmerwood://settings/";
/// What Home's and Settings' own buttons are links to. Following one is
/// never a navigation: it is how the page asks for something to be done.
const HOME_ACTIONS: &str = "glimmerwood://home/do/";
const SETTINGS_ACTIONS: &str = "glimmerwood://settings/do/";
const HELPLINES: &str = "https://findahelpline.com/";
const SAMARITANS: &str = "https://www.samaritans.org/how-we-can-help/contact-samaritan/";

/// Tabs reachable with Ctrl+number; Ctrl+9 is always the last tab.
const NUMBERED_TABS: u16 = 8;

thread_local! {
    static SHELL: RefCell<Option<Rc<Shell>>> = const { RefCell::new(None) };
    static COMPANION: RefCell<Option<Rc<Companion>>> = const { RefCell::new(None) };
}

/// The shell, for the few places that need it from outside.
pub fn held() -> Option<Rc<Shell>> {
    SHELL.with_borrow(|held| held.clone())
}

fn companion() -> Option<Rc<Companion>> {
    COMPANION.with_borrow(|held| held.clone())
}

/// One tab's engine. Nothing here is the chrome's: a tab has no message
/// handler, so a page has no way to speak to the core.
struct Engine {
    id: u32,
    controller: ICoreWebView2Controller,
    view: ICoreWebView2,
}

/// What a tab last set out to load.
struct Attempt {
    /// The address it is trying to reach.
    uri: String,
    /// The plain HTTP form of it, when it was optimistically upgraded to
    /// HTTPS and the plain one is what was meant if that can't be reached.
    plain: Option<String>,
}

/// A failure page standing in for a page a tab couldn't open.
struct Standing {
    /// The address it stands in for, which is still the tab's address as
    /// far as the chrome and the wisp are concerned.
    uri: String,
    /// Its own load hasn't started yet.
    pending: bool,
}

pub struct Shell {
    window: HWND,
    environment: ICoreWebView2Environment,
    toolbar: ICoreWebView2Controller,
    sidebar: ICoreWebView2Controller,
    /// The engines, in the order the column shows them.
    engines: RefCell<Vec<Engine>>,
    /// Which tabs there are and which one is in front. The bookkeeping is the
    /// core's, so it is the same here as it is anywhere else.
    tabs: RefCell<Tabs>,
    /// What the toolbar last said it needed, in the window's own pixels.
    toolbar_height: RefCell<i32>,
    /// What each tab is in the middle of loading, while it is.
    attempts: RefCell<HashMap<u32, Attempt>>,
    /// The tabs showing a failure page rather than what was asked for.
    failures: RefCell<HashMap<u32, Standing>>,
}

pub fn run() -> Fallible<()> {
    unsafe {
        // Before any window exists, so the chrome is laid out at the real
        // scale rather than stretched up to it.
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
    }

    let shape = remembered_shape();
    let window = make_window(&shape)?;
    // Shown before the engine is asked for, so that a machine which cannot
    // start one has a window to be told so in rather than nothing at all.
    unsafe {
        let _ = ShowWindow(
            window,
            if shape.maximised {
                SW_SHOWMAXIMIZED
            } else {
                SW_SHOW
            },
        );
    }
    if let Err(why) = runtime_version() {
        complain(window, &why);
        return Ok(());
    }
    let environment = make_environment()?;
    let toolbar = make_controller(&environment, window)?;
    let sidebar = make_controller(&environment, window)?;

    let shell = Rc::new(Shell {
        window,
        environment: environment.clone(),
        toolbar,
        sidebar,
        engines: RefCell::new(Vec::new()),
        tabs: RefCell::new(Tabs::new()),
        toolbar_height: RefCell::new(TOOLBAR_HEIGHT),
        attempts: RefCell::new(HashMap::new()),
        failures: RefCell::new(HashMap::new()),
    });

    let toolbar_view = unsafe { shell.toolbar.CoreWebView2()? };
    let sidebar_view = unsafe { shell.sidebar.CoreWebView2()? };

    // Only the chrome is served Glimmerwood's own files and heard from. The
    // tabs are the web, and have no way to ask for either.
    for view in [&toolbar_view, &sidebar_view] {
        serve_our_own_files(view, &environment, true)?;
        listen_to_the_chrome(view, &shell)?;
    }
    // The window's shortcuts work wherever the keyboard is, which includes
    // the chrome's own two engines.
    watch_the_keys(&shell.toolbar, &shell)?;
    watch_the_keys(&shell.sidebar, &shell)?;

    // The wisp's own window, over the toolbar's corner. It is created after
    // the engines so that it sits above them.
    nook::open(window)?;

    shell.open_tab(HOME, true)?;
    shell.lay_out();
    unsafe {
        toolbar_view.Navigate(w!("glimmerwood://chrome/toolbar.html"))?;
        sidebar_view.Navigate(w!("glimmerwood://chrome/sidebar.html"))?;
    }
    SHELL.with_borrow_mut(|held| *held = Some(shell.clone()));

    // The companion decides how the time is going. It is the same one the
    // Linux build uses; this only tells it what is on screen.
    let started = Companion::new(shell as Rc<dyn host::Host>, None);
    started.windows_changed();
    COMPANION.with_borrow_mut(|held| *held = Some(started));

    pump();
    Ok(())
}

impl Shell {
    /// The toolbar across the top, the column down the left, the tab in front
    /// filling what is left. The tabs behind it keep their size but are not
    /// shown: a hidden engine still plays sound, which is the point.
    fn lay_out(&self) {
        let mut whole = RECT::default();
        let _ = unsafe { GetClientRect(self.window, &mut whole) };
        let split = (*self.toolbar_height.borrow()).min(whole.bottom);
        let column = COLUMN_WIDTH.min(whole.right);
        let _ = unsafe {
            self.toolbar.SetBounds(RECT {
                bottom: split,
                ..whole
            })
        };
        let _ = unsafe {
            self.sidebar.SetBounds(RECT {
                top: split,
                right: column,
                ..whole
            })
        };
        let selected = self.tabs.borrow().selected();
        let page = RECT {
            top: split,
            left: column,
            ..whole
        };
        for engine in self.engines.borrow().iter() {
            let showing = Some(engine.id) == selected;
            unsafe {
                let _ = engine.controller.SetBounds(page);
                let _ = engine.controller.SetIsVisible(showing);
            }
        }
    }

    // --- Tabs ------------------------------------------------------------------

    /// The engine behind a tab, taken out of the list so nothing is borrowed
    /// while it is used.
    fn engine(&self, id: u32) -> Option<ICoreWebView2> {
        self.engines
            .borrow()
            .iter()
            .find(|engine| engine.id == id)
            .map(|engine| engine.view.clone())
    }

    fn selected_view(&self) -> Option<ICoreWebView2> {
        let id = self.tabs.borrow().selected()?;
        self.engine(id)
    }

    /// Where a tab is, as everything outside the engine sees it. While a
    /// failure page stands in for a page, it is the address that wouldn't
    /// open rather than the page put in its place.
    fn uri_of(&self, id: u32) -> String {
        if let Some(standing) = self.failures.borrow().get(&id) {
            return standing.uri.clone();
        }
        match self.engine(id) {
            Some(view) => unsafe { taken_string(|out| view.Source(out)) }.unwrap_or_default(),
            None => String::new(),
        }
    }

    /// Whether a failure page is what a tab is showing. Time on one is time
    /// on no site at all, and there is nothing there to bookmark.
    fn failed(&self, id: u32) -> bool {
        self.failures.borrow().contains_key(&id)
    }

    /// A new tab showing `uri`, with an engine of its own.
    fn open_tab(self: &Rc<Self>, uri: &str, select: bool) -> Fallible<u32> {
        let controller = make_controller(&self.environment, self.window)?;
        let view = unsafe { controller.CoreWebView2()? };
        let id = self.tabs.borrow_mut().open(select);
        watch_a_tab(&view, self, id)?;
        watch_the_keys(&controller, self)?;
        self.engines.borrow_mut().push(Engine {
            id,
            controller,
            view: view.clone(),
        });
        let target = HSTRING::from(uri);
        let _ = unsafe { view.Navigate(PCWSTR(target.as_ptr())) };
        self.lay_out();
        self.push_tabs();
        if select {
            self.push_state();
        }
        Ok(id)
    }

    fn select_tab(self: &Rc<Self>, id: u32) {
        if !self.tabs.borrow_mut().select(id) {
            return;
        }
        self.lay_out();
        self.push_tabs();
        self.push_state();
        self.focus_tab(id);
        if let Some(companion) = companion() {
            companion.refresh();
        }
    }

    /// Closes a tab and hands the window to whichever takes its place. The
    /// last tab closing closes the window, as it does everywhere else.
    fn close_tab(self: &Rc<Self>, id: u32) {
        let index = {
            let engines = self.engines.borrow();
            engines.iter().position(|engine| engine.id == id)
        };
        let Some(index) = index else { return };
        let engine = self.engines.borrow_mut().remove(index);
        unsafe {
            let _ = engine.controller.SetIsVisible(false);
            let _ = engine.controller.Close();
        }
        self.attempts.borrow_mut().remove(&id);
        self.failures.borrow_mut().remove(&id);
        let was_selected = self.tabs.borrow().selected() == Some(id);
        let next = self.tabs.borrow_mut().close(id);
        match next {
            None => unsafe {
                let _ = PostMessageW(Some(self.window), WM_CLOSE, WPARAM(0), LPARAM(0));
            },
            Some(_) if was_selected => {
                self.lay_out();
                self.push_tabs();
                self.push_state();
                if let Some(companion) = companion() {
                    companion.refresh();
                }
            }
            Some(_) => {
                self.push_tabs();
                if let Some(companion) = companion() {
                    companion.refresh();
                }
            }
        }
    }

    /// Reads what the engine says about a tab into the bookkeeping, and tells
    /// the chrome only if something it shows actually changed.
    fn refresh_tab(&self, id: u32, loading: Option<bool>) {
        let Some(view) = self.engine(id) else { return };
        let uri = self.uri_of(id);
        let title = unsafe { taken_string(|out| view.DocumentTitle(out)) }.unwrap_or_default();
        let (playing, muted) = match view.cast::<ICoreWebView2_8>() {
            Ok(audio) => unsafe {
                (
                    taken_bool(|out| audio.IsDocumentPlayingAudio(out)),
                    taken_bool(|out| audio.IsMuted(out)),
                )
            },
            Err(_) => (false, false),
        };
        let changed = self.tabs.borrow_mut().update(id, |facts| {
            facts.uri = uri;
            facts.title = title;
            facts.playing = playing;
            facts.muted = muted;
            if let Some(loading) = loading {
                facts.loading = loading;
            }
        });
        if changed {
            self.push_tabs();
        }
        if self.tabs.borrow().selected() == Some(id) {
            self.push_state();
        }
    }

    /// The keyboard belongs to the page once a tab is in front, unless it is
    /// one of Glimmerwood's own pages, where the address field is the more
    /// useful place to be.
    fn focus_tab(&self, id: u32) {
        let local = self
            .tabs
            .borrow()
            .facts(id)
            .is_some_and(|facts| facts.uri.is_empty() || pages::is_home(&facts.uri));
        if local {
            self.tell_the_chrome(&ToChrome::FocusAddress);
            return;
        }
        let controller = self
            .engines
            .borrow()
            .iter()
            .find(|engine| engine.id == id)
            .map(|engine| engine.controller.clone());
        if let Some(controller) = controller {
            let _ = unsafe { controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC) };
        }
    }

    /// Every tab's icon, for a column that has only just been drawn.
    fn push_icons(&self) {
        let icons: Vec<(u32, Option<String>)> = {
            let tabs = self.tabs.borrow();
            tabs.ids()
                .into_iter()
                .map(|id| (id, tabs.facts(id).and_then(|facts| facts.icon.clone())))
                .collect()
        };
        for (id, icon) in icons {
            self.tell_the_column(&ToChrome::TabIcon { id, icon });
        }
    }

    fn push_tabs(&self) {
        let (tabs, selected) = {
            let held = self.tabs.borrow();
            (held.info(), held.selected().unwrap_or_default())
        };
        self.tell_the_column(&ToChrome::Tabs { tabs, selected });
    }

    /// The wisp's history on Home: the tab in front if that is where it is,
    /// else the first Home tab this window has, else a new one.
    fn show_wisp(self: &Rc<Self>) {
        let home = {
            let tabs = self.tabs.borrow();
            let selected = tabs
                .selected()
                .filter(|id| tabs.facts(*id).is_some_and(|f| pages::is_home(&f.uri)));
            selected.or_else(|| {
                tabs.ids()
                    .into_iter()
                    .find(|id| tabs.facts(*id).is_some_and(|f| pages::is_home(&f.uri)))
            })
        };
        match home {
            Some(id) => {
                self.select_tab(id);
                if let Some(view) = self.engine(id) {
                    let script = HSTRING::from(
                        "if (location.href.startsWith('glimmerwood://home/')) \
                         window.wispHome?.reveal()",
                    );
                    let _ = unsafe { view.ExecuteScript(PCWSTR(script.as_ptr()), None) };
                }
            }
            None => {
                let _ = self.open_tab(HOME_WISP, true);
            }
        }
    }

    /// Settings: the window's Settings tab if it has one, else a new one.
    fn open_settings(self: &Rc<Self>) {
        let existing = {
            let tabs = self.tabs.borrow();
            tabs.ids()
                .into_iter()
                .find(|id| tabs.facts(*id).is_some_and(|f| pages::is_settings(&f.uri)))
        };
        match existing {
            Some(id) => self.select_tab(id),
            None => {
                if let Some(companion) = companion() {
                    companion.forget_lookup();
                }
                let _ = self.open_tab(SETTINGS, true);
            }
        }
    }

    /// Star the page in front, or unstar it.
    fn toggle_bookmark(&self) {
        let Some(companion) = companion() else { return };
        if self
            .tabs
            .borrow()
            .selected()
            .is_some_and(|id| self.failed(id))
        {
            return;
        }
        let Some(facts) = self
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
        self.push_state();
        companion.refresh_pages();
    }

    /// Hand Home or Settings what it shows, if that is what a tab holds.
    fn push_page(&self, id: u32) {
        let Some(companion) = companion() else { return };
        let Some(view) = self.engine(id) else { return };
        let uri = unsafe { taken_string(|out| view.Source(out)) }.unwrap_or_default();
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
        let script = HSTRING::from(page.hand_over(&json));
        let _ = unsafe { view.ExecuteScript(PCWSTR(script.as_ptr()), None) };
    }

    fn tell_the_chrome(&self, message: &ToChrome) {
        say(&self.toolbar, message);
    }

    /// The tab column hears about the tabs; the toolbar hears everything
    /// else. Each chrome page is only sent what it draws.
    fn tell_the_column(&self, message: &ToChrome) {
        say(&self.sidebar, message);
    }

    /// What the tab in front is showing, for the address field and the
    /// buttons beside it.
    fn push_state(&self) {
        let Some(id) = self.tabs.borrow().selected() else {
            return;
        };
        let Some(view) = self.engine(id) else { return };
        let uri = self.uri_of(id);
        let title = unsafe { taken_string(|out| view.DocumentTitle(out)) }.unwrap_or_default();
        let can_go_back = unsafe { taken_bool(|out| view.CanGoBack(out)) };
        let can_go_forward = unsafe { taken_bool(|out| view.CanGoForward(out)) };
        let loading = self
            .tabs
            .borrow()
            .facts(id)
            .is_some_and(|facts| facts.loading);
        let local = pages::is_local_page(&uri);
        let bookmarked = companion().is_some_and(|companion| companion.is_bookmarked(&uri));
        self.push_page(id);
        self.set_window_title(&title);
        self.tell_the_chrome(&ToChrome::State {
            security: if local {
                Security::Local
            } else {
                nav::security(&uri)
            },
            uri: uri.clone(),
            title,
            loading,
            progress: if loading { 0.5 } else { 1.0 },
            can_go_back,
            can_go_forward,
            can_bookmark: !local && !uri.is_empty() && !self.failed(id),
            bookmarked,
        });
    }

    fn set_window_title(&self, title: &str) {
        let shown = if title.is_empty() {
            "Glimmerwood"
        } else {
            title
        };
        let text = HSTRING::from(shown);
        let _ = unsafe { SetWindowTextW(self.window, PCWSTR(text.as_ptr())) };
    }

    fn navigate(&self, input: &str) {
        let Some(id) = self.tabs.borrow().selected() else {
            return;
        };
        let Some(view) = self.engine(id) else {
            return;
        };
        let Some(target) = nav::resolve(input) else {
            return;
        };
        let uri = HSTRING::from(target.uri.clone());
        let _ = unsafe { view.Navigate(PCWSTR(uri.as_ptr())) };
        // An address typed without a scheme is tried over HTTPS first, and
        // the plain form is what was meant if that can't be reached.
        self.attempts.borrow_mut().insert(
            id,
            Attempt {
                uri: target.uri,
                plain: target.fallback,
            },
        );
    }

    /// Put the caret in the address field, wherever the keyboard was.
    fn focus_address(&self) {
        let _ = unsafe {
            self.toolbar
                .MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC)
        };
        self.tell_the_chrome(&ToChrome::FocusAddress);
    }

    /// The tab `by` places along the column from the one in front, wrapping
    /// round at either end.
    fn step_tab(self: &Rc<Self>, by: isize) {
        let next = self.tabs.borrow().step(by);
        if let Some(id) = next {
            self.select_tab(id);
        }
    }

    // --- Navigation ------------------------------------------------------------

    /// A navigation a tab is about to make. Home's and Settings' buttons are
    /// links that never go anywhere: they are caught here and done instead.
    /// Anything else is a real load, and ends whatever the last one left.
    fn navigation_starting(&self, id: u32, args: &ICoreWebView2NavigationStartingEventArgs) {
        let target = unsafe { taken_string(|out| args.Uri(out)) }.unwrap_or_default();
        if self.page_action(id, &target) {
            let _ = unsafe { args.SetCancel(true) };
            return;
        }
        // A failure page's own load is the one load that leaves the failure
        // standing; any other is a fresh attempt at something.
        {
            let mut failures = self.failures.borrow_mut();
            match failures.get_mut(&id) {
                Some(standing) if standing.pending => standing.pending = false,
                _ => {
                    failures.remove(&id);
                }
            }
        }
        // A fallback belongs to one attempt at one address. Anywhere else,
        // a redirect included, is not that attempt any more.
        let mut attempts = self.attempts.borrow_mut();
        let carried = attempts
            .get(&id)
            .filter(|attempt| nav::same_address(&attempt.uri, &target))
            .and_then(|attempt| attempt.plain.clone());
        attempts.insert(
            id,
            Attempt {
                uri: target,
                plain: carried,
            },
        );
        drop(attempts);
        self.refresh_tab(id, Some(true));
    }

    /// The buttons on Home and Settings, which are links the page follows
    /// rather than messages: honoured only while the tab is actually showing
    /// the page that asked. Says whether the address was one of them.
    fn page_action(&self, id: u32, target: &str) -> bool {
        let showing = self.uri_of(id);
        if let Some(action) = target.strip_prefix(HOME_ACTIONS) {
            if let Some(companion) = companion()
                && pages::is_home(&showing)
                && companion.home_action(action)
            {
                companion.refresh_pages();
            }
            return true;
        }
        if let Some(action) = target.strip_prefix(SETTINGS_ACTIONS) {
            if let Some(companion) = companion()
                && pages::is_settings(&showing)
            {
                companion.settings_action(action);
            }
            return true;
        }
        false
    }

    /// What a navigation came to. One that worked needs nothing; one that
    /// didn't is either the plain-HTTP address an upgrade left behind, or
    /// Glimmerwood's own page saying what happened.
    fn navigation_ended(&self, id: u32, args: &ICoreWebView2NavigationCompletedEventArgs) {
        if unsafe { taken_bool(|out| args.IsSuccess(out)) } {
            self.attempts.borrow_mut().remove(&id);
            return;
        }
        let mut status = COREWEBVIEW2_WEB_ERROR_STATUS::default();
        if unsafe { args.WebErrorStatus(&mut status) }.is_err() {
            return;
        }
        // The user's own Stop, and a load a newer one replaced.
        if status == COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED {
            return;
        }
        // The failure page itself wouldn't load. Another would fare no
        // better, and asking for one would never stop.
        if self.failed(id) {
            return;
        }
        let Some(view) = self.engine(id) else { return };
        let attempt = self.attempts.borrow_mut().remove(&id);
        let failing = match &attempt {
            Some(attempt) => attempt.uri.clone(),
            None => unsafe { taken_string(|out| view.Source(out)) }.unwrap_or_default(),
        };
        let reason = reason_of(status);
        // A certificate is never a reason to try the same site again
        // without encryption.
        let fallback = attempt
            .and_then(|attempt| attempt.plain)
            .filter(|_| reason != Reason::Untrusted);
        if let Some(plain) = fallback {
            let uri = HSTRING::from(plain);
            let _ = unsafe { view.Navigate(PCWSTR(uri.as_ptr())) };
            return;
        }
        self.show_failure(id, &view, &failing, &reason);
    }

    /// Say in Glimmerwood's words why a page didn't open, in place of the
    /// engine's own page about it.
    fn show_failure(&self, id: u32, view: &ICoreWebView2, uri: &str, reason: &Reason) {
        let Some(template) = file("pages/failed.html") else {
            return;
        };
        let Ok(template) = std::str::from_utf8(template) else {
            return;
        };
        let page = HSTRING::from(failure::fill(template, uri, reason));
        if unsafe { view.NavigateToString(PCWSTR(page.as_ptr())) }.is_ok() {
            self.failures.borrow_mut().insert(
                id,
                Standing {
                    uri: uri.to_owned(),
                    pending: true,
                },
            );
        }
    }

    // --- Site icons ------------------------------------------------------------

    /// Ask the engine for the icon of the site a tab is on. Glimmerwood's
    /// own pages have no favicon; they wear the wisp.
    fn fetch_icon(self: &Rc<Self>, id: u32, icons: &ICoreWebView2_15) {
        if pages::is_local_page(&self.uri_of(id)) {
            self.set_icon(id, Some(wisp_icon()));
            return;
        }
        let weak: Weak<Shell> = Rc::downgrade(self);
        let got = GetFaviconCompletedHandler::create(Box::new(move |code, stream| {
            let Some(shell) = weak.upgrade() else {
                return Ok(());
            };
            let icon = code
                .ok()
                .and(stream)
                .map(|stream| everything_in(&stream))
                .filter(|png| !png.is_empty())
                .map(|png| format!("data:image/png;base64,{}", base64(&png)));
            shell.set_icon(id, icon);
            Ok(())
        }));
        let _ = unsafe { icons.GetFavicon(COREWEBVIEW2_FAVICON_IMAGE_FORMAT_PNG, &got) };
    }

    /// Hand the column a tab's icon, if it isn't the one it already has.
    fn set_icon(&self, id: u32, icon: Option<String>) {
        let changed = self
            .tabs
            .borrow_mut()
            .update(id, |facts| facts.icon = icon.clone());
        if changed {
            self.tell_the_column(&ToChrome::TabIcon { id, icon });
        }
    }

    // --- The keyboard ----------------------------------------------------------

    /// The window's shortcuts, the same set the Linux build has. Says
    /// whether the key was one of them, since one that was must not reach
    /// the page as well.
    fn shortcut(self: &Rc<Self>, key: VIRTUAL_KEY, ctrl: bool, shift: bool, alt: bool) -> bool {
        // Ctrl+1 to Ctrl+8 pick a tab by where it sits in the column, and
        // Ctrl+9 the last one however many there are.
        if ctrl && !shift && !alt && (VK_1.0..=VK_9.0).contains(&key.0) {
            let nth = key.0 - VK_1.0;
            let tabs = self.tabs.borrow();
            let id = if nth < NUMBERED_TABS {
                tabs.nth(nth as usize)
            } else {
                tabs.last()
            };
            drop(tabs);
            if let Some(id) = id {
                self.select_tab(id);
            }
            return true;
        }
        match (key, ctrl, shift, alt) {
            (VK_L, true, false, false)
            | (VK_D, false, false, true)
            | (VK_F6, false, false, false) => self.focus_address(),
            (VK_LEFT, false, false, true) => self.heard(ToCore::Back),
            (VK_RIGHT, false, false, true) => self.heard(ToCore::Forward),
            (VK_R, true, false, false) | (VK_F5, false, false, false) => self.heard(ToCore::Reload),
            (VK_T, true, false, false) => self.heard(ToCore::NewTab),
            (VK_W, true, false, false) | (VK_F4, true, false, false) => {
                let in_front = self.tabs.borrow().selected();
                if let Some(id) = in_front {
                    self.close_tab(id);
                }
            }
            (VK_D, true, false, false) => self.heard(ToCore::ToggleBookmark),
            (VK_HOME, false, false, true) => self.heard(ToCore::GoHome),
            (VK_OEM_COMMA, true, false, false) => self.heard(ToCore::OpenSettings),
            (VK_TAB, true, false, false) | (VK_NEXT, true, false, false) => self.step_tab(1),
            (VK_TAB, true, true, false) | (VK_PRIOR, true, false, false) => self.step_tab(-1),
            _ => return false,
        }
        true
    }

    fn heard(self: &Rc<Self>, message: ToCore) {
        let view = self.selected_view().ok_or(());
        match message {
            ToCore::Navigate { input } => self.navigate(&input),
            ToCore::Back => {
                if let Ok(view) = view {
                    let _ = unsafe { view.GoBack() };
                }
            }
            ToCore::Forward => {
                if let Ok(view) = view {
                    let _ = unsafe { view.GoForward() };
                }
            }
            ToCore::Reload => {
                if let Ok(view) = view {
                    let _ = unsafe { view.Reload() };
                }
            }
            ToCore::Stop => {
                if let Ok(view) = view {
                    let _ = unsafe { view.Stop() };
                }
            }
            ToCore::GoHome => {
                if let Ok(view) = view {
                    let _ = unsafe { view.Navigate(w!("glimmerwood://home/")) };
                }
            }
            ToCore::NewTab => {
                let _ = self.open_tab(HOME, true);
            }
            ToCore::SelectTab { id } => self.select_tab(id),
            ToCore::CloseTab { id } => self.close_tab(id),
            ToCore::ToggleMute { id } => {
                if let Some(view) = self.engine(id)
                    && let Ok(audio) = view.cast::<ICoreWebView2_8>()
                {
                    let muted = unsafe { taken_bool(|out| audio.IsMuted(out)) };
                    let _ = unsafe { audio.SetIsMuted(!muted) };
                    self.refresh_tab(id, None);
                }
            }
            ToCore::ToggleBookmark => self.toggle_bookmark(),
            ToCore::ShowWisp => self.show_wisp(),
            ToCore::OpenSettings => self.open_settings(),
            ToCore::FindSupport { samaritans } => {
                let _ = self.open_tab(if samaritans { SAMARITANS } else { HELPLINES }, true);
            }
            ToCore::FocusPage => {
                if let Some(id) = self.tabs.borrow().selected() {
                    self.focus_tab(id);
                }
            }
            ToCore::ToolbarLayout {
                height,
                nook_right,
                nook_top,
                nook_width,
                nook_height,
                ..
            } => {
                *self.toolbar_height.borrow_mut() = height as i32;
                self.lay_out();
                let mut whole = RECT::default();
                let _ = unsafe { GetClientRect(self.window, &mut whole) };
                // The toolbar gives the nook's inset from its right edge.
                let width = nook_width.max(1) as i32;
                let height = nook_height.max(1) as i32;
                let left = whole.right - nook_right as i32 - width;
                let top = nook_top as i32;
                nook::move_to(RECT {
                    left,
                    top,
                    right: left + width,
                    bottom: top + height,
                });
            }
            ToCore::Minimize => unsafe {
                let _ = ShowWindow(self.window, SW_MINIMIZE);
            },
            ToCore::CloseWindow => unsafe {
                let _ = PostMessageW(Some(self.window), WM_CLOSE, WPARAM(0), LPARAM(0));
            },
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
                self.push_state();
                if let Some(companion) = companion() {
                    companion.chrome_ready();
                }
            }
            ToCore::Ready {
                view: ChromeView::Sidebar,
            } => {
                self.push_tabs();
                self.push_icons();
            }
            // Find in page is the one thing WebView2 has no API for, so it
            // is not here yet; everything else the chrome can ask for is.
            _ => {}
        }
    }
}

/// How the window was left last time, which is how it opens this time.
struct Shape {
    width: i32,
    height: i32,
    maximised: bool,
}

impl Default for Shape {
    fn default() -> Self {
        Self {
            width: INITIAL_WIDTH,
            height: INITIAL_HEIGHT,
            maximised: false,
        }
    }
}

/// Beside the settings, and about the window and nothing else.
fn shape_file() -> PathBuf {
    crate::host::config_dir()
        .join("glimmerwood")
        .join("window.ini")
}

/// The size the window was last left at. No file, or one that says nothing
/// this version understands, just means the usual size.
fn remembered_shape() -> Shape {
    let mut shape = Shape::default();
    let Ok(text) = std::fs::read_to_string(shape_file()) else {
        return shape;
    };
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "width" => shape.width = value.parse().unwrap_or(shape.width),
            "height" => shape.height = value.parse().unwrap_or(shape.height),
            "maximised" => shape.maximised = value == "true",
            _ => {}
        }
    }
    // A saved size from a screen that is no longer here shouldn't leave the
    // window too small to use.
    shape.width = shape.width.max(LEAST_WIDTH);
    shape.height = shape.height.max(LEAST_HEIGHT);
    shape
}

/// Keep the window's size for next time. A window closed maximised opens
/// maximised, at the size it had before it was.
fn remember_shape(window: HWND) {
    let mut placement = WINDOWPLACEMENT {
        length: size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    if unsafe { GetWindowPlacement(window, &mut placement) }.is_err() {
        return;
    }
    let normal = placement.rcNormalPosition;
    let text = format!(
        "[window]\nwidth={}\nheight={}\nmaximised={}\n",
        normal.right - normal.left,
        normal.bottom - normal.top,
        placement.showCmd == SW_SHOWMAXIMIZED.0 as u32,
    );
    let path = shape_file();
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&path, text));
    if let Err(err) = written {
        eprintln!("glimmerwood: couldn't remember the window's size: {err}");
    }
}

fn make_window(shape: &Shape) -> Fallible<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance.into(),
            lpszClassName: w!("Glimmerwood"),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        RegisterClassW(&class);
        Ok(CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("Glimmerwood"),
            w!("Glimmerwood"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            shape.width,
            shape.height,
            None,
            None,
            Some(instance.into()),
            None,
        )?)
    }
}

/// Which WebView2 the machine has, if it has one. Asked before anything else,
/// because creating an environment without a runtime waits for an answer that
/// never comes.
fn runtime_version() -> std::result::Result<String, String> {
    let mut raw = PWSTR::null();
    let found = unsafe { GetAvailableCoreWebView2BrowserVersionString(PCWSTR::null(), &mut raw) };
    if found.is_err() || raw.is_null() {
        return Err(
            "Glimmerwood needs the Microsoft Edge WebView2 runtime, which this \
             machine doesn't have. Most copies of Windows come with it; it can \
             also be installed from Microsoft."
                .into(),
        );
    }
    let version = unsafe { raw.to_string() }.unwrap_or_default();
    unsafe { CoTaskMemFree(Some(raw.as_ptr().cast())) };
    Ok(version)
}

/// Say what is wrong, in the window, rather than exiting without a word.
fn complain(window: HWND, why: &str) {
    let text = HSTRING::from(why);
    unsafe {
        MessageBoxW(
            Some(window),
            PCWSTR(text.as_ptr()),
            w!("Glimmerwood"),
            MB_OK | MB_ICONINFORMATION,
        );
        let _ = DestroyWindow(window);
    }
}

fn make_environment() -> Fallible<ICoreWebView2Environment> {
    let held: Rc<RefCell<Option<ICoreWebView2Environment>>> = Rc::new(RefCell::new(None));
    let out = held.clone();
    CreateCoreWebView2EnvironmentCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| unsafe {
            CreateCoreWebView2EnvironmentWithOptions(PCWSTR::null(), PCWSTR::null(), None, &handler)
                .map_err(Into::into)
        }),
        Box::new(move |code, environment| {
            code?;
            *out.borrow_mut() = environment;
            Ok(())
        }),
    )?;
    let taken = held.borrow_mut().take();
    taken.ok_or_else(|| missing("WebView2 is not installed on this machine"))
}

fn make_controller(
    environment: &ICoreWebView2Environment,
    window: HWND,
) -> Fallible<ICoreWebView2Controller> {
    let held: Rc<RefCell<Option<ICoreWebView2Controller>>> = Rc::new(RefCell::new(None));
    let out = held.clone();
    let environment = environment.clone();
    CreateCoreWebView2ControllerCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| unsafe {
            environment
                .CreateCoreWebView2Controller(window, &handler)
                .map_err(Into::into)
        }),
        Box::new(move |code, controller| {
            code?;
            *out.borrow_mut() = controller;
            Ok(())
        }),
    )?;
    let taken = held.borrow_mut().take();
    taken.ok_or_else(|| missing("WebView2 would not start"))
}

fn missing(why: &str) -> Box<dyn Error> {
    Box::<dyn Error>::from(why.to_string())
}

/// Answer `glimmerwood://` out of the files carried in the binary. Anything
/// not carried is left unanswered, which the page sees as a failed load.
fn serve_our_own_files(
    webview: &ICoreWebView2,
    environment: &ICoreWebView2Environment,
    ours_only: bool,
) -> Fallible<()> {
    unsafe {
        webview.AddWebResourceRequestedFilter(
            w!("glimmerwood://*"),
            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
        )?;
    }
    let environment = environment.clone();
    let handler = WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else { return Ok(()) };
        let request = unsafe { args.Request()? };
        let uri = unsafe { taken_string(|out| request.Uri(out))? };
        if !ours_only && !pages::is_local_page(&uri) {
            return Ok(());
        }
        let Some(path) = pages::path_of(&uri) else {
            return Ok(());
        };
        let Some(bytes) = file(&path) else {
            return Ok(());
        };
        let stream = unsafe { SHCreateMemStream(Some(bytes)) };
        let headers = HSTRING::from(format!("Content-Type: {}", pages::mime_type(&path)));
        let response = unsafe {
            environment.CreateWebResourceResponse(
                stream.as_ref(),
                200,
                w!("OK"),
                PCWSTR(headers.as_ptr()),
            )?
        };
        unsafe { args.SetResponse(&response)? };
        Ok(())
    }));
    let mut token = 0i64;
    unsafe { webview.add_WebResourceRequested(&handler, &mut token)? };
    Ok(())
}

/// Everything the chrome says arrives here as the protocol the core knows.
fn listen_to_the_chrome(webview: &ICoreWebView2, shell: &Rc<Shell>) -> Fallible<()> {
    let weak: Weak<Shell> = Rc::downgrade(shell);
    let handler = WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else { return Ok(()) };
        let json = unsafe { taken_string(|out| args.WebMessageAsJson(out))? };
        match serde_json::from_str::<ToCore>(&json) {
            Ok(message) => {
                if let Some(shell) = weak.upgrade() {
                    shell.heard(message);
                }
            }
            Err(err) => {
                eprintln!("glimmerwood: ignoring a chrome message the core doesn't know: {err}")
            }
        }
        Ok(())
    }));
    let mut token = 0i64;
    unsafe { webview.add_WebMessageReceived(&handler, &mut token)? };
    Ok(())
}

/// Everything one tab's engine reports, with the tab it belongs to carried
/// along: a tab nobody is looking at still changes title, still finishes
/// loading and still starts playing sound, and the column shows all three.
fn watch_a_tab(webview: &ICoreWebView2, shell: &Rc<Shell>, id: u32) -> Fallible<()> {
    let mut token = 0i64;

    let told = |shell: &Rc<Shell>| {
        let weak: Weak<Shell> = Rc::downgrade(shell);
        move || {
            if let Some(shell) = weak.upgrade() {
                shell.refresh_tab(id, None);
            }
        }
    };

    let moved = told(shell);
    let changed = SourceChangedEventHandler::create(Box::new(move |_, _| {
        moved();
        Ok(())
    }));
    unsafe { webview.add_SourceChanged(&changed, &mut token)? };

    let named = told(shell);
    let titled = DocumentTitleChangedEventHandler::create(Box::new(move |_, _| {
        named();
        Ok(())
    }));
    unsafe { webview.add_DocumentTitleChanged(&titled, &mut token)? };

    let weak: Weak<Shell> = Rc::downgrade(shell);
    let starting = NavigationStartingEventHandler::create(Box::new(move |_, args| {
        if let Some(shell) = weak.upgrade()
            && let Some(args) = args
        {
            shell.navigation_starting(id, &args);
        }
        Ok(())
    }));
    unsafe { webview.add_NavigationStarting(&starting, &mut token)? };

    // The site icon the column shows. It is asked for again at the end of
    // every load, so that a page with none of its own loses the last one's.
    let icons = webview.cast::<ICoreWebView2_15>().ok();

    let weak: Weak<Shell> = Rc::downgrade(shell);
    let after = icons.clone();
    let done = NavigationCompletedEventHandler::create(Box::new(move |_, args| {
        let Some(shell) = weak.upgrade() else {
            return Ok(());
        };
        shell.refresh_tab(id, Some(false));
        if let Some(args) = args {
            shell.navigation_ended(id, &args);
        }
        if let Some(icons) = after.as_ref() {
            shell.fetch_icon(id, icons);
        }
        Ok(())
    }));
    unsafe { webview.add_NavigationCompleted(&done, &mut token)? };

    if let Some(icons) = icons {
        let weak: Weak<Shell> = Rc::downgrade(shell);
        let mine = icons.clone();
        let drawn = FaviconChangedEventHandler::create(Box::new(move |_, _| {
            if let Some(shell) = weak.upgrade() {
                shell.fetch_icon(id, &mine);
            }
            Ok(())
        }));
        unsafe { icons.add_FaviconChanged(&drawn, &mut token)? };
    }

    // Sound, which the column marks and the dose engine counts.
    if let Ok(audio) = webview.cast::<ICoreWebView2_8>() {
        let heard = told(shell);
        let playing = IsDocumentPlayingAudioChangedEventHandler::create(Box::new(move |_, _| {
            heard();
            Ok(())
        }));
        unsafe { audio.add_IsDocumentPlayingAudioChanged(&playing, &mut token)? };

        let hushed = told(shell);
        let muted = IsMutedChangedEventHandler::create(Box::new(move |_, _| {
            hushed();
            Ok(())
        }));
        unsafe { audio.add_IsMutedChanged(&muted, &mut token)? };
    }

    // A page asking for a window of its own gets a tab, in the background,
    // the way the Linux build has always handled it.
    let weak: Weak<Shell> = Rc::downgrade(shell);
    let asked = NewWindowRequestedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else { return Ok(()) };
        let uri = unsafe { taken_string(|out| args.Uri(out)) }.unwrap_or_default();
        unsafe { args.SetHandled(true)? };
        if let Some(shell) = weak.upgrade()
            && !uri.is_empty()
        {
            let _ = shell.open_tab(&uri, false);
        }
        Ok(())
    }));
    unsafe { webview.add_NewWindowRequested(&asked, &mut token)? };
    Ok(())
}

/// The window's shortcuts on one engine. WebView2 hands the page every key
/// it is sent, so the ones the window answers have to be taken out of the
/// stream before the page sees them.
fn watch_the_keys(controller: &ICoreWebView2Controller, shell: &Rc<Shell>) -> Fallible<()> {
    let weak: Weak<Shell> = Rc::downgrade(shell);
    let pressed = AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else { return Ok(()) };
        let mut kind = COREWEBVIEW2_KEY_EVENT_KIND::default();
        unsafe { args.KeyEventKind(&mut kind)? };
        // Presses only, and Alt combinations arrive as system keys.
        if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN
            && kind != COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN
        {
            return Ok(());
        }
        let mut key = 0u32;
        unsafe { args.VirtualKey(&mut key)? };
        let Some(shell) = weak.upgrade() else {
            return Ok(());
        };
        let taken = shell.shortcut(
            VIRTUAL_KEY(key as u16),
            down(VK_CONTROL),
            down(VK_SHIFT),
            down(VK_MENU),
        );
        if taken {
            unsafe { args.SetHandled(true)? };
        }
        Ok(())
    }));
    let mut token = 0i64;
    unsafe { controller.add_AcceleratorKeyPressed(&pressed, &mut token)? };
    Ok(())
}

/// Whether a modifier key is down at this moment.
fn down(key: VIRTUAL_KEY) -> bool {
    unsafe { GetKeyState(key.0 as i32) < 0 }
}

/// What the engine's error status means in the words a reader cares about.
fn reason_of(status: COREWEBVIEW2_WEB_ERROR_STATUS) -> Reason {
    match status {
        COREWEBVIEW2_WEB_ERROR_STATUS_HOST_NAME_NOT_RESOLVED => Reason::NotFound,
        COREWEBVIEW2_WEB_ERROR_STATUS_CANNOT_CONNECT
        | COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_ABORTED
        | COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_RESET => Reason::Refused,
        COREWEBVIEW2_WEB_ERROR_STATUS_TIMEOUT
        | COREWEBVIEW2_WEB_ERROR_STATUS_SERVER_UNREACHABLE
        | COREWEBVIEW2_WEB_ERROR_STATUS_ERROR_HTTP_INVALID_SERVER_RESPONSE => Reason::NoAnswer,
        COREWEBVIEW2_WEB_ERROR_STATUS_DISCONNECTED => Reason::Offline,
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_COMMON_NAME_IS_INCORRECT
        | COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_EXPIRED
        | COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_IS_INVALID
        | COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_REVOKED
        | COREWEBVIEW2_WEB_ERROR_STATUS_CLIENT_CERTIFICATE_CONTAINS_ERRORS => Reason::Untrusted,
        COREWEBVIEW2_WEB_ERROR_STATUS_REDIRECT_FAILED => {
            Reason::Other("The site sent the page round in circles.".into())
        }
        COREWEBVIEW2_WEB_ERROR_STATUS_VALID_AUTHENTICATION_CREDENTIALS_REQUIRED => {
            Reason::Other("The site wants a name and password this address doesn't carry.".into())
        }
        COREWEBVIEW2_WEB_ERROR_STATUS_VALID_PROXY_AUTHENTICATION_REQUIRED => {
            Reason::Other("The proxy for this network wants a name and password.".into())
        }
        _ => Reason::Other("The page couldn't be opened.".into()),
    }
}

/// Glimmerwood's own mark, for the tabs showing Home or Settings.
fn wisp_icon() -> String {
    thread_local! {
        static ICON: String = match file("home/wisp.svg") {
            Some(svg) => format!("data:image/svg+xml;base64,{}", base64(svg)),
            None => String::new(),
        };
    }
    ICON.with(Clone::clone)
}

/// Everything a stream holds. COM hands a site icon over as one.
fn everything_in(stream: &IStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        let mut read = 0u32;
        let code = unsafe {
            stream.Read(
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                Some(&mut read),
            )
        };
        if code.is_err() || read == 0 {
            return bytes;
        }
        bytes.extend_from_slice(&buffer[..read as usize]);
    }
}

/// Base64, for the `data:` URL an icon is handed to the chrome as.
fn base64(bytes: &[u8]) -> String {
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

/// Hand a chrome page a message. Nothing else is ever spoken to.
fn say(controller: &ICoreWebView2Controller, message: &ToChrome) {
    let Ok(json) = serde_json::to_string(message) else {
        return;
    };
    let Ok(view) = (unsafe { controller.CoreWebView2() }) else {
        return;
    };
    let script = HSTRING::from(format!("window.wispChrome?.receive?.({json})"));
    let _ = unsafe { view.ExecuteScript(PCWSTR(script.as_ptr()), None) };
}

/// A flag COM hands back in an out-parameter.
unsafe fn taken_bool(from: impl FnOnce(*mut BOOL) -> windows::core::Result<()>) -> bool {
    let mut flag = BOOL::default();
    from(&mut flag).is_ok() && flag.as_bool()
}

/// A string COM hands back in an out-parameter, and expects to be freed.
unsafe fn taken_string(
    from: impl FnOnce(*mut PWSTR) -> windows::core::Result<()>,
) -> windows::core::Result<String> {
    let mut raw = PWSTR::null();
    from(&mut raw)?;
    let text = unsafe { raw.to_string() }.unwrap_or_default();
    unsafe { CoTaskMemFree(Some(raw.as_ptr().cast())) };
    Ok(text)
}

fn file(path: &str) -> Option<&'static [u8]> {
    crate::files::FILES
        .iter()
        .find(|(name, _)| *name == path)
        .map(|(_, bytes)| *bytes)
}

fn pump() {
    unsafe {
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

extern "system" fn procedure(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match message {
        WM_SIZE => {
            SHELL.with_borrow(|held| {
                if let Some(shell) = held.as_ref() {
                    shell.lay_out();
                }
            });
            LRESULT(0)
        }
        WM_TIMER => {
            if w.0 == crate::host::WAKE
                && let Some(companion) = companion()
            {
                companion.refresh();
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            remember_shape(window);
            COMPANION.with_borrow_mut(|held| *held = None);
            SHELL.with_borrow_mut(|held| *held = None);
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(window, message, w, l) },
    }
}

impl Shell {
    pub fn window(&self) -> HWND {
        self.window
    }
}

impl host::Window for Shell {
    fn in_front(&self) -> bool {
        unsafe { GetForegroundWindow() == self.window && IsWindowVisible(self.window).as_bool() }
    }

    /// Nothing while a failure page stands in for the page in front: time
    /// spent on one is time spent on no site at all.
    fn attended_uri(&self) -> String {
        let tabs = self.tabs.borrow();
        match tabs.selected() {
            Some(id) if !self.failed(id) => tabs
                .facts(id)
                .map(|facts| facts.uri.clone())
                .unwrap_or_default(),
            _ => String::new(),
        }
    }

    /// Every tab but the one being looked at. They weigh nothing, but the
    /// dose engine still wants to know they are open.
    fn other_tabs(&self, in_front: bool) -> Vec<String> {
        let tabs = self.tabs.borrow();
        let attended = if in_front { tabs.selected() } else { None };
        tabs.other_uris(attended)
    }

    fn sound_on_screen(&self) -> bool {
        self.tabs
            .borrow()
            .selected_facts()
            .is_some_and(|facts| facts.playing && !facts.muted)
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
            ToChrome::Tabs { .. } | ToChrome::TabIcon { .. } => self.tell_the_column(message),
            _ => self.tell_the_chrome(message),
        }
    }

    fn refresh_pages(&self) {
        for id in self.tabs.borrow().ids() {
            self.push_page(id);
        }
    }

    fn ask(&self, site: Option<String>) {
        self.tell_the_chrome(&ToChrome::Ask { site });
    }

    fn care(&self, open: bool, samaritans: bool) {
        self.tell_the_chrome(&ToChrome::Care { open, samaritans });
    }

    fn set_title(&self, title: &str) {
        self.set_window_title(title);
    }
}

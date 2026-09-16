//! The window, and the engines inside it.
//!
//! The toolbar runs across the top, the tab column down the left and the tab
//! in front fills the rest, each its own WebView2. The toolbar and the column
//! are Glimmerwood's own chrome and the only ones handed `glimmerwood://`;
//! the tabs are the web, and there is one engine per tab whether or not it
//! is the one on screen.

use std::cell::RefCell;
use std::error::Error;
use std::rc::{Rc, Weak};

use crate::nook;
use glimmerwood_core::companion::Companion;
use glimmerwood_core::dose::{Mode, Trend};
use glimmerwood_core::host;
use glimmerwood_core::nav;
use glimmerwood_core::pages;
use glimmerwood_core::protocol::{ChromeView, Security, ToChrome, ToCore};
use glimmerwood_core::tabs::Tabs;
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{
    CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
    DocumentTitleChangedEventHandler, IsDocumentPlayingAudioChangedEventHandler,
    IsMutedChangedEventHandler, NavigationCompletedEventHandler, NavigationStartingEventHandler,
    NewWindowRequestedEventHandler, SourceChangedEventHandler, WebMessageReceivedEventHandler,
    WebResourceRequestedEventHandler,
};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Shell::SHCreateMemStream;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::BOOL;
use windows::core::{HSTRING, Interface, PCWSTR, PWSTR, w};

type Fallible<T> = std::result::Result<T, Box<dyn Error>>;

/// Before the chrome says otherwise.
const INITIAL_WIDTH: i32 = 1100;
const INITIAL_HEIGHT: i32 = 760;
/// The toolbar's height until it reports its own.
const TOOLBAR_HEIGHT: i32 = 56;
/// The tab column's width. The GTK build lets it be dragged and remembers
/// where; here it is the width that shows a tab's mark and nothing else.
const COLUMN_WIDTH: i32 = 48;

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
}

pub fn run() -> Fallible<()> {
    unsafe {
        // Before any window exists, so the chrome is laid out at the real
        // scale rather than stretched up to it.
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
    }

    let window = make_window()?;
    // Shown before the engine is asked for, so that a machine which cannot
    // start one has a window to be told so in rather than nothing at all.
    unsafe {
        let _ = ShowWindow(window, SW_SHOW);
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
    });

    let toolbar_view = unsafe { shell.toolbar.CoreWebView2()? };
    let sidebar_view = unsafe { shell.sidebar.CoreWebView2()? };

    // Only the chrome is served Glimmerwood's own files and heard from. The
    // tabs are the web, and have no way to ask for either.
    for view in [&toolbar_view, &sidebar_view] {
        serve_our_own_files(view, &environment, true)?;
        listen_to_the_chrome(view, &shell)?;
    }

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

    /// A new tab showing `uri`, with an engine of its own.
    fn open_tab(self: &Rc<Self>, uri: &str, select: bool) -> Fallible<u32> {
        let controller = make_controller(&self.environment, self.window)?;
        let view = unsafe { controller.CoreWebView2()? };
        let id = self.tabs.borrow_mut().open(select);
        watch_a_tab(&view, self, id)?;
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
        let uri = unsafe { taken_string(|out| view.Source(out)) }.unwrap_or_default();
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
        let uri = unsafe { taken_string(|out| view.Source(out)) }.unwrap_or_default();
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
            can_bookmark: !local && !uri.is_empty(),
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
        let Some(view) = self.selected_view() else {
            return;
        };
        let Some(target) = nav::resolve(input) else {
            return;
        };
        let uri = HSTRING::from(target.uri);
        let _ = unsafe { view.Navigate(PCWSTR(uri.as_ptr())) };
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
            } => self.push_tabs(),
            // Find in page is the one thing WebView2 has no API for, so it
            // is not here yet; everything else the chrome can ask for is.
            _ => {}
        }
    }
}

fn make_window() -> Fallible<HWND> {
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
            INITIAL_WIDTH,
            INITIAL_HEIGHT,
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

    let told = |shell: &Rc<Shell>, loading: Option<bool>| {
        let weak: Weak<Shell> = Rc::downgrade(shell);
        move || {
            if let Some(shell) = weak.upgrade() {
                shell.refresh_tab(id, loading);
            }
        }
    };

    let moved = told(shell, None);
    let changed = SourceChangedEventHandler::create(Box::new(move |_, _| {
        moved();
        Ok(())
    }));
    unsafe { webview.add_SourceChanged(&changed, &mut token)? };

    let named = told(shell, None);
    let titled = DocumentTitleChangedEventHandler::create(Box::new(move |_, _| {
        named();
        Ok(())
    }));
    unsafe { webview.add_DocumentTitleChanged(&titled, &mut token)? };

    let began = told(shell, Some(true));
    let starting = NavigationStartingEventHandler::create(Box::new(move |_, _| {
        began();
        Ok(())
    }));
    unsafe { webview.add_NavigationStarting(&starting, &mut token)? };

    let ended = told(shell, Some(false));
    let done = NavigationCompletedEventHandler::create(Box::new(move |_, _| {
        ended();
        Ok(())
    }));
    unsafe { webview.add_NavigationCompleted(&done, &mut token)? };

    // Sound, which the column marks and the dose engine counts.
    if let Ok(audio) = webview.cast::<ICoreWebView2_8>() {
        let heard = told(shell, None);
        let playing = IsDocumentPlayingAudioChangedEventHandler::create(Box::new(move |_, _| {
            heard();
            Ok(())
        }));
        unsafe { audio.add_IsDocumentPlayingAudioChanged(&playing, &mut token)? };

        let hushed = told(shell, None);
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

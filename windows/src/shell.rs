//! The window, and the engines inside it.
//!
//! The toolbar runs across the top and the page fills the rest, each its own
//! WebView2. The toolbar is Glimmerwood's own chrome and is the only one
//! handed `glimmerwood://`; the page below it is the web.

use std::cell::RefCell;
use std::error::Error;
use std::rc::{Rc, Weak};

use crate::nook;
use glimmerwood_core::companion::Companion;
use glimmerwood_core::dose::{Mode, Trend};
use glimmerwood_core::host;
use glimmerwood_core::nav;
use glimmerwood_core::pages;
use glimmerwood_core::protocol::{Security, ToChrome, ToCore};
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{
    CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
    DocumentTitleChangedEventHandler, NavigationCompletedEventHandler,
    NavigationStartingEventHandler, SourceChangedEventHandler, WebMessageReceivedEventHandler,
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

pub struct Shell {
    window: HWND,
    toolbar: ICoreWebView2Controller,
    page: ICoreWebView2Controller,
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
    let page = make_controller(&environment, window)?;

    let shell = Rc::new(Shell {
        window,
        toolbar,
        page,
        toolbar_height: RefCell::new(TOOLBAR_HEIGHT),
    });

    let toolbar_view = unsafe { shell.toolbar.CoreWebView2()? };
    let page_view = unsafe { shell.page.CoreWebView2()? };

    // Only the chrome is served Glimmerwood's own files. The page below is
    // the web, and has no way to ask for them.
    serve_our_own_files(&toolbar_view, &environment)?;
    listen_to_the_chrome(&toolbar_view, &shell)?;
    watch_the_page(&page_view, &shell)?;

    // The wisp's own window, over the toolbar's corner. It is created after
    // the engines so that it sits above them.
    nook::open(window)?;

    shell.lay_out();
    unsafe {
        toolbar_view.Navigate(w!("glimmerwood://chrome/toolbar.html"))?;
        page_view.Navigate(w!("glimmerwood://home/"))?;
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
    /// The toolbar across the top, the page filling what is left.
    fn lay_out(&self) {
        let mut whole = RECT::default();
        let _ = unsafe { GetClientRect(self.window, &mut whole) };
        let split = (*self.toolbar_height.borrow()).min(whole.bottom);
        let _ = unsafe {
            self.toolbar.SetBounds(RECT {
                bottom: split,
                ..whole
            })
        };
        let _ = unsafe {
            self.page.SetBounds(RECT {
                top: split,
                ..whole
            })
        };
    }

    fn tell_the_chrome(&self, message: &ToChrome) {
        let Ok(json) = serde_json::to_string(message) else {
            return;
        };
        let Ok(view) = (unsafe { self.toolbar.CoreWebView2() }) else {
            return;
        };
        let script = HSTRING::from(format!("window.wispChrome?.receive?.({json})"));
        let _ = unsafe { view.ExecuteScript(PCWSTR(script.as_ptr()), None) };
    }

    /// What the page is showing, for the address field and the buttons.
    fn push_state(&self) {
        let Ok(view) = (unsafe { self.page.CoreWebView2() }) else {
            return;
        };
        let uri = unsafe { taken_string(|out| view.Source(out)) }.unwrap_or_default();
        let title = unsafe { taken_string(|out| view.DocumentTitle(out)) }.unwrap_or_default();
        let can_go_back = unsafe { taken_bool(|out| view.CanGoBack(out)) };
        let can_go_forward = unsafe { taken_bool(|out| view.CanGoForward(out)) };
        let local = pages::is_local_page(&uri);
        self.tell_the_chrome(&ToChrome::State {
            security: if local {
                Security::Local
            } else {
                nav::security(&uri)
            },
            uri: uri.clone(),
            title,
            loading: false,
            progress: 1.0,
            can_go_back,
            can_go_forward,
            can_bookmark: !local && !uri.is_empty(),
            bookmarked: false,
        });
    }

    fn navigate(&self, input: &str) {
        let Ok(view) = (unsafe { self.page.CoreWebView2() }) else {
            return;
        };
        let Some(target) = nav::resolve(input) else {
            return;
        };
        let uri = HSTRING::from(target.uri);
        let _ = unsafe { view.Navigate(PCWSTR(uri.as_ptr())) };
    }

    fn heard(&self, message: ToCore) {
        let view = unsafe { self.page.CoreWebView2() };
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
            ToCore::FocusPage => {
                let _ = unsafe {
                    self.page
                        .MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC)
                };
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
            ToCore::Ready { .. } => {
                self.push_state();
                if let Some(companion) = companion() {
                    companion.chrome_ready();
                }
            }
            // The rest belong to tabs and the lists, which this shell has yet
            // to grow.
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

/// Tell the chrome whenever the page moves.
fn watch_the_page(webview: &ICoreWebView2, shell: &Rc<Shell>) -> Fallible<()> {
    let mut token = 0i64;

    let weak: Weak<Shell> = Rc::downgrade(shell);
    let changed = SourceChangedEventHandler::create(Box::new(move |_, _| {
        if let Some(shell) = weak.upgrade() {
            shell.push_state();
        }
        Ok(())
    }));
    unsafe { webview.add_SourceChanged(&changed, &mut token)? };

    let weak: Weak<Shell> = Rc::downgrade(shell);
    let titled = DocumentTitleChangedEventHandler::create(Box::new(move |_, _| {
        if let Some(shell) = weak.upgrade() {
            shell.push_state();
        }
        Ok(())
    }));
    unsafe { webview.add_DocumentTitleChanged(&titled, &mut token)? };

    let weak: Weak<Shell> = Rc::downgrade(shell);
    let starting = NavigationStartingEventHandler::create(Box::new(move |_, _| {
        if let Some(shell) = weak.upgrade() {
            shell.push_state();
        }
        Ok(())
    }));
    unsafe { webview.add_NavigationStarting(&starting, &mut token)? };

    let weak: Weak<Shell> = Rc::downgrade(shell);
    let done = NavigationCompletedEventHandler::create(Box::new(move |_, _| {
        if let Some(shell) = weak.upgrade() {
            shell.push_state();
        }
        Ok(())
    }));
    unsafe { webview.add_NavigationCompleted(&done, &mut token)? };
    Ok(())
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
        let Ok(view) = (unsafe { self.page.CoreWebView2() }) else {
            return String::new();
        };
        unsafe { taken_string(|out| view.Source(out)) }.unwrap_or_default()
    }

    fn other_tabs(&self, _in_front: bool) -> Vec<String> {
        // One page at a time, until this shell grows tabs.
        Vec::new()
    }

    fn sound_on_screen(&self) -> bool {
        let Ok(view) = (unsafe { self.page.CoreWebView2() }) else {
            return false;
        };
        // Whether a page is making a noise arrived in a later WebView2.
        let Ok(view) = view.cast::<ICoreWebView2_8>() else {
            return false;
        };
        let playing = unsafe { taken_bool(|out| view.IsDocumentPlayingAudio(out)) };
        let muted = unsafe { taken_bool(|out| view.IsMuted(out)) };
        playing && !muted
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
        self.tell_the_chrome(message);
    }

    fn refresh_pages(&self) {
        // Home and Settings are not served here yet, so there is nothing
        // showing that could have gone stale.
    }

    fn ask(&self, site: Option<String>) {
        self.tell_the_chrome(&ToChrome::Ask { site });
    }

    fn care(&self, open: bool, samaritans: bool) {
        self.tell_the_chrome(&ToChrome::Care { open, samaritans });
    }

    fn set_title(&self, title: &str) {
        let text = HSTRING::from(title);
        let _ = unsafe { SetWindowTextW(self.window, PCWSTR(text.as_ptr())) };
    }
}

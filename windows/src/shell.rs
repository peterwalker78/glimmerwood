//! The window, and the engine inside it.

use std::cell::RefCell;
use std::error::Error;
use std::rc::Rc;

use glimmerwood_core::pages;
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{
    CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
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
use windows::core::{HSTRING, PCWSTR, PWSTR, w};

type Fallible<T> = std::result::Result<T, Box<dyn Error>>;

/// Before the chrome says otherwise.
const INITIAL_WIDTH: i32 = 1100;
const INITIAL_HEIGHT: i32 = 760;

thread_local! {
    /// The controller, so the window can resize what is inside it.
    static CONTROLLER: RefCell<Option<ICoreWebView2Controller>> = const { RefCell::new(None) };
}

pub fn run() -> Fallible<()> {
    unsafe {
        // Before any window exists, so the chrome is laid out at the real
        // scale rather than stretched up to it.
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
    }

    let window = make_window()?;
    let environment = make_environment()?;
    let controller = make_controller(&environment, window)?;
    let webview = unsafe { controller.CoreWebView2()? };

    serve_our_own_files(&webview, &environment)?;
    unsafe {
        controller.SetBounds(client_rect(window))?;
        webview.Navigate(w!("glimmerwood://chrome/toolbar.html"))?;
    }
    CONTROLLER.with_borrow_mut(|held| *held = Some(controller));

    unsafe {
        let _ = ShowWindow(window, SW_SHOW);
    }
    pump();
    Ok(())
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
    // The handler stays for the life of the webview, so its token is of no
    // further use to anyone.
    let mut token = 0i64;
    unsafe { webview.add_WebResourceRequested(&handler, &mut token)? };
    Ok(())
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

fn client_rect(window: HWND) -> RECT {
    let mut rect = RECT::default();
    let _ = unsafe { GetClientRect(window, &mut rect) };
    rect
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
            CONTROLLER.with_borrow(|held| {
                if let Some(controller) = held.as_ref() {
                    let _ = unsafe { controller.SetBounds(client_rect(window)) };
                }
            });
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(window, message, w, l) },
    }
}

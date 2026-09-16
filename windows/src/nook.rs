//! The wisp's nook on Windows.
//!
//! WebView2 draws into a window of its own, and nothing can be painted on top
//! of it. So the wisp gets a small layered window of its own, sitting over the
//! corner of the toolbar where the nook is, painted from the same drawing the
//! GTK build uses — through the rasteriser rather than cairo.

use std::cell::RefCell;

use glimmerwood_core::dose::{Mode, Trend};
use glimmerwood_core::wisp::Wisp;
use glimmerwood_raster::Raster;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, HBITMAP,
    ReleaseDC, SelectObject,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

/// The nook, until the toolbar reports where it put it.
const INITIAL: RECT = RECT {
    left: 0,
    top: 0,
    right: 152,
    bottom: 56,
};

thread_local! {
    static NOOK: RefCell<Option<Nook>> = const { RefCell::new(None) };
}

struct Nook {
    window: HWND,
    wisp: Wisp,
    at: RECT,
}

/// A window for the wisp, over the toolbar's corner.
pub fn open(parent: HWND) -> windows::core::Result<HWND> {
    let instance = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None)? };
    let class = WNDCLASSW {
        lpfnWndProc: Some(procedure),
        hInstance: instance.into(),
        lpszClassName: w!("GlimmerwoodNook"),
        ..Default::default()
    };
    unsafe { RegisterClassW(&class) };
    let window = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
            w!("GlimmerwoodNook"),
            None,
            WS_CHILD | WS_VISIBLE,
            INITIAL.left,
            INITIAL.top,
            INITIAL.right,
            INITIAL.bottom,
            Some(parent),
            None,
            Some(instance.into()),
            None,
        )?
    };
    let width = f64::from(INITIAL.right);
    let height = f64::from(INITIAL.bottom);
    NOOK.with_borrow_mut(|held| {
        *held = Some(Nook {
            window,
            wisp: Wisp::new(width, height),
            at: INITIAL,
        });
    });
    draw();
    Ok(window)
}

/// Where the toolbar has put its nook, in the window's own pixels.
pub fn move_to(at: RECT) {
    NOOK.with_borrow_mut(|held| {
        let Some(nook) = held.as_mut() else { return };
        nook.at = at;
        nook.wisp
            .resize(f64::from(at.right - at.left), f64::from(at.bottom - at.top));
        let _ = unsafe {
            SetWindowPos(
                nook.window,
                Some(HWND_TOP),
                at.left,
                at.top,
                at.right - at.left,
                at.bottom - at.top,
                SWP_NOACTIVATE,
            )
        };
    });
    draw();
}

/// How the wisp is doing. Nothing calls this yet: the companion that works
/// out the dose still lives in the GTK app, and until it moves the wisp here
/// only dozes.
#[expect(dead_code, reason = "waiting on the companion to be shared")]
pub fn update(dose: f64, mode: Mode, trend: Trend, private: bool, night: bool, welcome: bool) {
    let moved = NOOK.with_borrow_mut(|held| {
        held.as_mut()
            .is_some_and(|nook| nook.wisp.update(dose, mode, trend, private, night, welcome))
    });
    if moved {
        draw();
    }
}

/// Paint one frame, and ask to be woken for the next if one is wanted.
pub fn draw() {
    let next = NOOK.with_borrow_mut(|held| {
        let nook = held.as_mut()?;
        let width = (nook.at.right - nook.at.left).max(1) as u32;
        let height = (nook.at.bottom - nook.at.top).max(1) as u32;
        let mut raster = Raster::new(width, height)?;
        // The wisp's own clock: milliseconds since the program started.
        let now = unsafe { windows::Win32::System::SystemInformation::GetTickCount64() } as f64;
        let next = nook.wisp.draw(now, true, dark(), &mut raster);
        blit(nook.window, &raster, width, height);
        next
    });
    NOOK.with_borrow(|held| {
        let Some(nook) = held.as_ref() else { return };
        match next {
            Some(delay) => unsafe {
                SetTimer(Some(nook.window), 1, (delay.max(1.0) as u32).max(1), None);
            },
            None => unsafe {
                let _ = KillTimer(Some(nook.window), 1);
            },
        }
    });
}

/// Windows has no one answer for this yet in Glimmerwood; the chrome follows
/// the system and will tell us, the way the toolbar tells us its layout.
fn dark() -> bool {
    false
}

/// Put the pixels on the screen. A layered window takes the whole image at
/// once, alpha and all, which is what lets the wisp sit over the toolbar
/// without a rectangle around it.
fn blit(window: HWND, raster: &Raster, width: u32, height: u32) {
    let pixels = raster.data();
    let screen = unsafe { GetDC(None) };
    let memory = unsafe { CreateCompatibleDC(Some(screen)) };

    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            // Negative, so the first row in the buffer is the top one.
            biHeight: -(height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    let bitmap: HBITMAP = match unsafe {
        CreateDIBSection(Some(memory), &info, DIB_RGB_COLORS, &mut bits, None, 0)
    } {
        Ok(bitmap) if !bits.is_null() => bitmap,
        _ => {
            unsafe {
                let _ = DeleteDC(memory);
                ReleaseDC(None, screen);
            }
            return;
        }
    };

    // The rasteriser gives premultiplied RGBA; Windows wants it the other way
    // round.
    let count = (width as usize) * (height as usize);
    let out = unsafe { std::slice::from_raw_parts_mut(bits.cast::<u8>(), count * 4) };
    for pixel in 0..count.min(pixels.len() / 4) {
        let (to, from) = (pixel * 4, pixel * 4);
        out[to] = pixels[from + 2];
        out[to + 1] = pixels[from + 1];
        out[to + 2] = pixels[from];
        out[to + 3] = pixels[from + 3];
    }

    let old = unsafe { SelectObject(memory, bitmap.into()) };
    let where_ = POINT::default();
    let size = windows::Win32::Foundation::SIZE {
        cx: width as i32,
        cy: height as i32,
    };
    let from = POINT { x: 0, y: 0 };
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    let _ = unsafe {
        UpdateLayeredWindow(
            window,
            Some(screen),
            Some(&where_),
            Some(&size),
            Some(memory),
            Some(&from),
            windows::Win32::Foundation::COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        )
    };

    unsafe {
        SelectObject(memory, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
    }
}

extern "system" fn procedure(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match message {
        WM_TIMER => {
            draw();
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(window, message, w, l) },
    }
}

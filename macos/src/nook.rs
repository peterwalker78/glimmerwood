//! The wisp's nook.
//!
//! The toolbar is a web page, and nothing can be drawn into it from here, so
//! the wisp gets a small view of its own sitting over the corner of it where
//! the chrome leaves a gap. What the wisp *is* lives in the core; this lends
//! it a surface, a clock and a timer, and passes on a hover and a click.
//!
//! The pixels come from the rasteriser rather than from cairo, so it is the
//! same drawing here as on Linux.

use std::cell::RefCell;

use glimmerwood_core::canvas::Canvas;
use glimmerwood_core::dose::{Mode, Trend};
use glimmerwood_core::wisp::Wisp;
use glimmerwood_raster::Raster;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol};
use objc2::{AllocAnyThread, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSBitmapFormat,
    NSBitmapImageRep, NSDeviceRGBColorSpace, NSEvent, NSImage, NSResponder, NSTrackingArea,
    NSTrackingAreaOptions, NSView, NSWindowOrderingMode, NSWorkspace,
};
use objc2_foundation::{MainThreadMarker, NSArray, NSInteger, NSProcessInfo, NSRect, NSTimer};

/// Eight bits a channel, four of them, premultiplied and in that order, which
/// is what the rasteriser hands over and what a bitmap here takes by default.
const BITS_PER_SAMPLE: NSInteger = 8;
const SAMPLES_PER_PIXEL: NSInteger = 4;
const BITS_PER_PIXEL: NSInteger = 32;

thread_local! {
    static NOOK: RefCell<Option<Nook>> = const { RefCell::new(None) };
}

struct Nook {
    view: Retained<NookView>,
    wisp: Wisp,
    /// The wake-up for the next frame, if one is wanted.
    wake: Option<Retained<NSTimer>>,
}

/// A view for the wisp, over the toolbar's corner.
pub fn open(content: &NSView, above: &NSView, at: NSRect, mtm: MainThreadMarker) {
    let view: Retained<NookView> = unsafe { msg_send![NookView::alloc(mtm), initWithFrame: at] };
    // Its own layer, so it sits over the toolbar's without either of them
    // being drawn again for the other's sake. Nothing but the wisp is painted
    // into it, so the toolbar shows through around it.
    view.setWantsLayer(true);
    let tracking = unsafe {
        NSTrackingArea::initWithRect_options_owner_userInfo(
            NSTrackingArea::alloc(),
            at,
            NSTrackingAreaOptions::MouseEnteredAndExited
                | NSTrackingAreaOptions::ActiveInKeyWindow
                // The area follows the view, so moving the nook leaves it be.
                | NSTrackingAreaOptions::InVisibleRect,
            Some(&view),
            None,
        )
    };
    view.addTrackingArea(&tracking);
    content.addSubview_positioned_relativeTo(&view, NSWindowOrderingMode::Above, Some(above));
    NOOK.with_borrow_mut(|held| {
        *held = Some(Nook {
            view: view.clone(),
            wisp: Wisp::new(at.size.width, at.size.height),
            wake: None,
        });
    });
    view.setNeedsDisplay(true);
}

/// Where the toolbar has put its nook, in the window's own points.
pub fn move_to(at: NSRect) {
    let view = NOOK.with_borrow(|held| held.as_ref().map(|nook| nook.view.clone()));
    let Some(view) = view else { return };
    if view.frame() != at {
        view.setFrame(at);
    }
    view.setNeedsDisplay(true);
}

/// How the wisp is doing, as the companion sees it.
pub fn update(dose: f64, mode: Mode, trend: Trend, private: bool, night: bool, welcome: bool) {
    let moved = NOOK.with_borrow_mut(|held| {
        held.as_mut()
            .is_some_and(|nook| nook.wisp.update(dose, mode, trend, private, night, welcome))
    });
    if moved {
        NOOK.with_borrow(|held| {
            if let Some(nook) = held.as_ref() {
                nook.view.setNeedsDisplay(true);
            }
        });
    }
}

/// Paint one frame, and ask to be woken for the next if one is wanted.
fn paint(view: &NookView) {
    let bounds = view.bounds();
    // The wisp is drawn in points and rasterised at whatever the screen makes
    // of them, which is what cairo does for the GTK build.
    let scale = view
        .window()
        .map_or(1.0, |window| window.backingScaleFactor())
        .max(1.0);
    let width = (bounds.size.width * scale).round().max(1.0) as u32;
    let height = (bounds.size.height * scale).round().max(1.0) as u32;
    let dark = dark(view);
    let next = NOOK.with_borrow_mut(|held| {
        let nook = held.as_mut()?;
        let mut raster = Raster::new(width, height)?;
        raster.scale(scale, scale);
        nook.wisp.resize(bounds.size.width, bounds.size.height);
        let next = nook.wisp.draw(now_ms(), moving(), dark, &mut raster);
        show(&raster, width, height, bounds);
        next
    });
    wake_for(view, next);
}

/// Put the pixels on the screen. The bitmap is in pixels and the view in
/// points, so the image is given the view's size and lands one pixel to one
/// pixel on any screen.
fn show(raster: &Raster, width: u32, height: u32, into: NSRect) {
    let pixels = raster.data();
    let row = width as usize * 4;
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bitmapFormat_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            width as NSInteger,
            height as NSInteger,
            BITS_PER_SAMPLE,
            SAMPLES_PER_PIXEL,
            true,
            false,
            NSDeviceRGBColorSpace,
            NSBitmapFormat::empty(),
            row as NSInteger,
            BITS_PER_PIXEL,
        )
    };
    let Some(rep) = rep else { return };
    let into_bitmap = rep.bitmapData();
    if into_bitmap.is_null() {
        return;
    }
    let wanted = row * height as usize;
    if pixels.len() < wanted {
        return;
    }
    // Safety: the bitmap was asked for exactly this many rows of this width,
    // and the rasteriser was given the same size.
    unsafe { std::ptr::copy_nonoverlapping(pixels.as_ptr(), into_bitmap, wanted) };
    let image = NSImage::initWithSize(NSImage::alloc(), into.size);
    image.addRepresentation(&rep);
    image.drawInRect(into);
}

/// Wake for the next frame after `delay_ms`, unless nothing needs one.
fn wake_for(view: &NookView, delay_ms: Option<f64>) {
    NOOK.with_borrow_mut(|held| {
        let Some(nook) = held.as_mut() else { return };
        if let Some(timer) = nook.wake.take() {
            timer.invalidate();
        }
        let Some(delay) = delay_ms else { return };
        nook.wake = Some(unsafe {
            NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                delay.max(1.0) / 1000.0,
                view,
                sel!(tick:),
                None,
                false,
            )
        });
    });
}

/// The wisp's clock, in milliseconds. It counts from when the machine came
/// up, so it goes on moving forward whatever the wall clock does.
fn now_ms() -> f64 {
    NSProcessInfo::processInfo().systemUptime() * 1000.0
}

/// Whether the wisp may move. With Reduce Motion on, it holds still and is
/// redrawn only when the dose moves, which is what the core's own drawing
/// expects of a platform that says no.
fn moving() -> bool {
    !NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

/// Whether the chrome around the nook is dark. The system says so here, the
/// same answer the toolbar's own page gets from `prefers-color-scheme`.
fn dark(view: &NookView) -> bool {
    let light = unsafe { NSAppearanceNameAqua };
    let dark = unsafe { NSAppearanceNameDarkAqua };
    let names = NSArray::from_slice(&[light, dark]);
    view.effectiveAppearance()
        .bestMatchFromAppearancesWithNames(&names)
        .is_some_and(|name| &*name == dark)
}

define_class!(
    /// The nook itself: a view that draws the wisp and nothing else.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GlimmerwoodNook"]
    struct NookView;

    unsafe impl NSObjectProtocol for NookView {}

    impl NookView {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            paint(self);
        }

        /// The wisp is drawn from the top left down, as it is everywhere
        /// else.
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        /// Only the wisp is painted into it; the toolbar behind shows
        /// through everywhere else.
        #[unsafe(method(isOpaque))]
        fn is_opaque(&self) -> bool {
            false
        }

        #[unsafe(method(tick:))]
        fn tick(&self, _timer: &NSTimer) {
            NOOK.with_borrow_mut(|held| {
                if let Some(nook) = held.as_mut() {
                    nook.wake = None;
                }
            });
            self.setNeedsDisplay(true);
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _event: &NSEvent) {
            crate::shell::hovering_wisp(true);
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            crate::shell::hovering_wisp(false);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, _event: &NSEvent) {
            crate::shell::wisp_clicked();
        }
    }
);

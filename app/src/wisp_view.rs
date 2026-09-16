//! The wisp's nook: a GTK drawing area laid over the nook in the chrome,
//! with cairo behind it. What the wisp *is* lives in the core; this lends it
//! a surface, a clock and a timer, and passes on a hover and a click.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk::{cairo, gdk, glib, prelude::*};

use glimmerwood_core::canvas::{Canvas, Paint, Stop};
use glimmerwood_core::dose::{Mode, Trend};
use glimmerwood_core::wisp::Wisp;

/// The drawing area covers the nook; its size comes from the toolbar's
/// layout, these are only the size before that arrives.
const INITIAL_WIDTH: i32 = 152;
const INITIAL_HEIGHT: i32 = 56;

pub struct WispView {
    area: gtk::DrawingArea,
    wisp: Rc<RefCell<Wisp>>,
}

/// The pending wake-up for the next frame, if one is wanted.
type Wake = Rc<RefCell<Option<glib::SourceId>>>;

impl WispView {
    /// `on_hover`: the pointer entered or left it. `on_click`: it was clicked.
    pub fn new(on_hover: impl Fn(bool) + 'static, on_click: impl Fn() + 'static) -> WispView {
        let area = gtk::DrawingArea::new();
        area.set_content_width(INITIAL_WIDTH);
        area.set_content_height(INITIAL_HEIGHT);
        area.set_halign(gtk::Align::End);
        area.set_valign(gtk::Align::Start);
        let wisp = Rc::new(RefCell::new(Wisp::new(
            f64::from(INITIAL_WIDTH),
            f64::from(INITIAL_HEIGHT),
        )));
        let wake: Wake = Rc::new(RefCell::new(None));

        let weak = Rc::downgrade(&wisp);
        let waking = wake.clone();
        area.set_draw_func(move |area, cr, width, height| {
            if let Some(wisp) = weak.upgrade() {
                // GTK's frame clock is the wisp's clock, in milliseconds.
                let now = area
                    .frame_clock()
                    .map_or(0.0, |clock| clock.frame_time() as f64 / 1000.0);
                let next = {
                    let mut wisp = wisp.borrow_mut();
                    wisp.resize(f64::from(width), f64::from(height));
                    wisp.draw(now, animations_enabled(), dark(), &mut CairoCanvas { cr })
                };
                schedule(&waking, area, next);
            }
        });

        let hover = Rc::new(on_hover);
        let motion = gtk::EventControllerMotion::new();
        motion.connect_enter(glib::clone!(
            #[strong]
            hover,
            move |_, _, _| hover(true)
        ));
        motion.connect_leave(move |_| hover(false));
        area.add_controller(motion);

        let click = gtk::GestureClick::new();
        click.set_button(gdk::BUTTON_PRIMARY);
        click.connect_released(move |_, _, _, _| on_click());
        area.add_controller(click);

        WispView { area, wisp }
    }

    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.area
    }

    /// `private`: the visible tab is on the privacy list, so the
    /// wisp slips out of sight. `night`: it winds down. `welcome`: the user
    /// has just come back after a long time away.
    pub fn update(
        &self,
        dose: f64,
        mode: Mode,
        trend: Trend,
        private: bool,
        night: bool,
        welcome: bool,
    ) {
        let moved = self
            .wisp
            .borrow_mut()
            .update(dose, mode, trend, private, night, welcome);
        if moved {
            self.area.queue_draw();
        }
    }
}

fn animations_enabled() -> bool {
    gtk::Settings::default().is_none_or(|s| s.is_gtk_enable_animations())
}

fn dark() -> bool {
    #[allow(deprecated, reason = "kept in step with the portal by main.rs")]
    gtk::Settings::default().is_some_and(|s| s.is_gtk_application_prefer_dark_theme())
}

/// Wake for the next frame after `delay_ms`, unless nothing needs one.
fn schedule(wake: &Wake, area: &gtk::DrawingArea, delay_ms: Option<f64>) {
    if let Some(id) = wake.borrow_mut().take() {
        id.remove();
    }
    let Some(delay) = delay_ms else { return };
    let weak_wake = Rc::downgrade(wake);
    let weak_area = area.downgrade();
    let id =
        glib::timeout_add_local_once(Duration::from_millis(delay.max(1.0) as u64), move || {
            if let Some(wake) = weak_wake.upgrade() {
                *wake.borrow_mut() = None;
            }
            if let Some(area) = weak_area.upgrade() {
                area.queue_draw();
            }
        });
    *wake.borrow_mut() = Some(id);
}
/// cairo, lending the wisp a surface to be drawn on.
struct CairoCanvas<'a> {
    cr: &'a cairo::Context,
}

impl CairoCanvas<'_> {
    fn stops(gradient: &cairo::Gradient, stops: &[Stop]) {
        for s in stops {
            gradient.add_color_stop_rgba(
                s.at,
                s.colour.0,
                s.colour.1,
                s.colour.2,
                s.alpha.clamp(0.0, 1.0),
            );
        }
    }
}

impl Canvas for CairoCanvas<'_> {
    fn save(&mut self) {
        let _ = self.cr.save();
    }
    fn restore(&mut self) {
        let _ = self.cr.restore();
    }
    fn translate(&mut self, dx: f64, dy: f64) {
        self.cr.translate(dx, dy);
    }
    fn scale(&mut self, sx: f64, sy: f64) {
        self.cr.scale(sx, sy);
    }

    fn move_to(&mut self, x: f64, y: f64) {
        self.cr.move_to(x, y);
    }
    fn line_to(&mut self, x: f64, y: f64) {
        self.cr.line_to(x, y);
    }
    fn curve_to(&mut self, c1x: f64, c1y: f64, c2x: f64, c2y: f64, x: f64, y: f64) {
        self.cr.curve_to(c1x, c1y, c2x, c2y, x, y);
    }
    fn arc(&mut self, x: f64, y: f64, radius: f64, from: f64, to: f64) {
        self.cr.arc(x, y, radius, from, to);
    }
    fn rectangle(&mut self, x: f64, y: f64, width: f64, height: f64) {
        self.cr.rectangle(x, y, width, height);
    }
    fn close_path(&mut self) {
        self.cr.close_path();
    }
    fn new_path(&mut self) {
        self.cr.new_path();
    }

    fn set_paint(&mut self, paint: Paint<'_>) {
        match paint {
            Paint::Solid { colour, alpha } => {
                self.cr
                    .set_source_rgba(colour.0, colour.1, colour.2, alpha.clamp(0.0, 1.0));
            }
            Paint::Radial {
                inner,
                outer,
                stops,
            } => {
                let gradient = cairo::RadialGradient::new(
                    inner.0, inner.1, inner.2, outer.0, outer.1, outer.2,
                );
                Self::stops(&gradient, stops);
                let _ = self.cr.set_source(&gradient);
            }
            Paint::Linear { from, to, stops } => {
                let gradient = cairo::LinearGradient::new(from.0, from.1, to.0, to.1);
                Self::stops(&gradient, stops);
                let _ = self.cr.set_source(&gradient);
            }
        }
    }
    fn set_line_width(&mut self, width: f64) {
        self.cr.set_line_width(width);
    }
    fn set_round_ends(&mut self, round: bool) {
        let (cap, join) = if round {
            (cairo::LineCap::Round, cairo::LineJoin::Round)
        } else {
            (cairo::LineCap::Butt, cairo::LineJoin::Miter)
        };
        self.cr.set_line_cap(cap);
        self.cr.set_line_join(join);
    }
    fn set_adding(&mut self, adding: bool) {
        self.cr.set_operator(if adding {
            cairo::Operator::Add
        } else {
            cairo::Operator::Over
        });
    }

    fn fill(&mut self) {
        let _ = self.cr.fill();
    }
    fn stroke(&mut self) {
        let _ = self.cr.stroke();
    }
    fn clip(&mut self) {
        self.cr.clip();
    }
}

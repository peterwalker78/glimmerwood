//! The wisp: a flame-shaped sprite with a friendly face, resting on a tuft of
//! moss in its nook, drawn natively in a GTK drawing area laid over
//! the nook in the chrome.
//!
//! Its colours along the dose come from `core/data/wisp.toml`; its face,
//! motion and moss are set by the constants below. It redraws only as often
//! as what it is doing needs, and a still wisp (reduced motion) only when the
//! dose moves.

use std::cell::RefCell;
use std::f64::consts::{PI, TAU};
use std::rc::Rc;
use std::time::Duration;

use gtk::{cairo, gdk, glib, prelude::*};

use crate::dose::Mode;
use crate::look::{Look, Stops};
use crate::oklab::Rgb;

/// The drawing area covers the nook; its size comes from the toolbar's
/// layout, these are only the size before that arrives.
const INITIAL_WIDTH: i32 = 152;
const INITIAL_HEIGHT: i32 = 56;

// Feel constants. Alternatives are noted beside some of them.
/// Body radius at rest, in CSS pixels: about 27 px across, which fits a
/// 56 px nook with its glow.
const BODY_RADIUS: f64 = 13.5;
/// A slow breath of about six seconds at rest.
const BREATH_MS: f64 = 6000.0;
/// How much the body swells with each breath: 0.1 made the face bounce.
const BREATH_DEPTH: f64 = 0.05;
/// Time for the shown dose to catch up with a changed one.
const EASE_MS: f64 = 1500.0;
/// A blink lasts this long, and comes every 2.5-7 s.
const BLINK_MS: f64 = 170.0;
/// Giving privacy: roughly the time to slip out of sight, or come back.
const PRIVACY_MS: f64 = 1200.0;
/// Noticing a change of site: a glance toward the page and a small hop, so
/// the user sees at once that the wisp registered where they went.
const NOTICE_MS: f64 = 1100.0;
const NOTICE_HOP: f64 = 3.0;
/// How strongly the kind of site shows on its face, independent of the
/// dose: wary on draining sites (lids a little lower, the smile flattened,
/// the glow a touch dimmer; never sad), calm on ordinary ones, curious on
/// unlisted ones.
const WARY_LIDS: f64 = 0.22;
const WARY_DIM: f64 = 0.08;
/// At night the glow warms by this much and the breath slows by
/// this factor, whatever the dose.
const NIGHT_WARMTH: f64 = 0.35;
const NIGHT_BREATH: f64 = 1.4;
const NIGHT_GLOW: Rgb = Rgb(0.96, 0.66, 0.36);
/// Coming back after a long time away: brighter and a little bigger.
const WELCOME_BRIGHTER: f64 = 0.3;
const WELCOME_BIGGER: f64 = 0.08;
/// Frames per second by what the wisp is doing. Each frame per second costs
/// about 0.11% of a core. A dozing wisp breathes at 2; resting, 4;
/// drifting, smoking or releasing motes, 6. Blinks, slipping away and catching
/// up with a changed dose get short bursts of 20.
const DOZING_FPS: f64 = 2.0;
const RESTING_FPS: f64 = 4.0;
const LIVELY_FPS: f64 = 6.0;
const BURST_FPS: f64 = 20.0;
/// Nourishing: a mote now and then, not a fountain.
const MOTE_EVERY_MS: f64 = 1400.0;
/// Clouded and drained: faint smoke.
const SMOKE_EVERY_MS: f64 = 2200.0;
/// Dozing: a small z drifts up now and then.
const Z_EVERY_MS: f64 = 3200.0;
/// Drained: out toward the window edge, pause, back, rest.
const DRIFT_CYCLE_MS: f64 = 14000.0;
/// When drained it drifts toward the window's edge, to the right: to this
/// far from the nook's right side.
const EDGE_MARGIN: f64 = 34.0;
/// How far an engaged wisp wanders around its nook.
const ROAM_X: f64 = 18.0;
const ROAM_Y: f64 = 3.0;
/// The moss: the share of the nook's width its mound spans, and its floor
/// and height.
const MOSS_SPAN: f64 = 0.74;
const MOSS_FLOOR: f64 = 7.0;
const MOSS_HEIGHT: f64 = 8.0;
/// Moss colours along the dose: fresh, tired olive, dry tan. It browns and
/// its tufts droop as the dose rises, and greens again as it falls. It never
/// goes bare: the garden never dies, and neither does this.
const MOSS_FRESH: Rgb = Rgb(0.49, 0.62, 0.34);
const MOSS_TIRED: Rgb = Rgb(0.55, 0.56, 0.34);
const MOSS_DRY: Rgb = Rgb(0.62, 0.52, 0.33);
/// Light themes only: the soft shadow the wisp glows in.
const WELL_DEPTH: f64 = 0.07;
/// The face's dark ink.
const FACE_INK: Rgb = Rgb(0.17, 0.13, 0.10);
/// Warm cheeks: amber, never pink or red.
const BLUSH: Rgb = Rgb(0.95, 0.62, 0.35);

struct Particle {
    born: f64,
    life: f64,
    x: f64,
    y: f64,
    dx: f64,
    dy: f64,
    size: f64,
    kind: ParticleKind,
}

#[derive(Clone, Copy, PartialEq)]
enum ParticleKind {
    Mote,
    Smoke,
    Z,
}

struct Anim {
    stops: Stops,
    target: f64,
    shown: f64,
    mode: Mode,
    private: bool,
    night: bool,
    welcome: bool,
    /// 0-1, eased toward `night` and `welcome`.
    night_mix: f64,
    welcome_mix: f64,
    /// 0 = here, 1 = out of sight.
    privacy: f64,
    /// 0 = normal, 1 = happily closed eyes; eased.
    happy: f64,
    /// 0 = awake, 1 = asleep; eased.
    sleep: f64,
    /// The mode the wisp last reacted to, and when it noticed the change.
    noticed: Mode,
    notice_start: f64,
    /// 0-1, eased: how wary, calm or curious it looks.
    wary: f64,
    calm: f64,
    curious: f64,
    glance: f64,
    glance_target: f64,
    next_glance: f64,
    next_blink: f64,
    particles: Vec<Particle>,
    last_tick: Option<f64>,
    last_mote: f64,
    last_smoke: f64,
    last_z: f64,
    seed: u64,
    width: f64,
    height: f64,
    wake: Option<glib::SourceId>,
}

pub struct WispView {
    area: gtk::DrawingArea,
    anim: Rc<RefCell<Anim>>,
}

impl WispView {
    /// `on_hover`: the pointer entered or left it. `on_click`: it was clicked.
    pub fn new(on_hover: impl Fn(bool) + 'static, on_click: impl Fn() + 'static) -> WispView {
        let area = gtk::DrawingArea::new();
        area.set_content_width(INITIAL_WIDTH);
        area.set_content_height(INITIAL_HEIGHT);
        area.set_halign(gtk::Align::End);
        area.set_valign(gtk::Align::Start);
        let anim = Rc::new(RefCell::new(Anim {
            stops: Stops::bundled(),
            target: 0.0,
            shown: 0.0,
            mode: Mode::Away,
            private: false,
            night: false,
            welcome: false,
            night_mix: 0.0,
            welcome_mix: 0.0,
            privacy: 0.0,
            happy: 0.0,
            sleep: 1.0,
            noticed: Mode::Away,
            notice_start: f64::NEG_INFINITY,
            wary: 0.0,
            calm: 0.0,
            curious: 0.0,
            glance: 0.0,
            glance_target: 0.0,
            next_glance: 0.0,
            next_blink: 0.0,
            particles: Vec::new(),
            last_tick: None,
            last_mote: 0.0,
            last_smoke: 0.0,
            last_z: 0.0,
            seed: 0x9e37_79b9_7f4a_7c15,
            width: f64::from(INITIAL_WIDTH),
            height: f64::from(INITIAL_HEIGHT),
            wake: None,
        }));

        let weak = Rc::downgrade(&anim);
        area.set_draw_func(move |area, cr, width, height| {
            if let Some(anim) = weak.upgrade() {
                let next = {
                    let mut anim = anim.borrow_mut();
                    anim.width = f64::from(width);
                    anim.height = f64::from(height);
                    draw(&mut anim, area, cr)
                };
                schedule(&anim, area, next);
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

        WispView { area, anim }
    }

    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.area
    }

    /// `private`: the visible tab is on the privacy list, so the
    /// wisp slips out of sight. `night`: it winds down. `welcome`: the user
    /// has just come back after a long time away.
    pub fn update(&self, dose: f64, mode: Mode, private: bool, night: bool, welcome: bool) {
        {
            let mut anim = self.anim.borrow_mut();
            if (anim.target - dose).abs() < 1e-6
                && anim.mode == mode
                && anim.private == private
                && anim.night == night
                && anim.welcome == welcome
            {
                return;
            }
            anim.target = dose;
            anim.mode = mode;
            anim.private = private;
            anim.night = night;
            anim.welcome = welcome;
        }
        self.area.queue_draw();
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
fn schedule(anim: &Rc<RefCell<Anim>>, area: &gtk::DrawingArea, delay_ms: Option<f64>) {
    if let Some(id) = anim.borrow_mut().wake.take() {
        id.remove();
    }
    let Some(delay) = delay_ms else { return };
    let weak_anim = Rc::downgrade(anim);
    let weak_area = area.downgrade();
    let id =
        glib::timeout_add_local_once(Duration::from_millis(delay.max(1.0) as u64), move || {
            if let Some(anim) = weak_anim.upgrade() {
                anim.borrow_mut().wake = None;
            }
            if let Some(area) = weak_area.upgrade() {
                area.queue_draw();
            }
        });
    anim.borrow_mut().wake = Some(id);
}

/// Cheap smooth noise: a few incommensurate sines. Deterministic, so the
/// same wisp wanders the same way.
fn wander(t: f64, seed: f64) -> f64 {
    (t * 0.00031 + seed).sin() * 0.5
        + (t * 0.00073 + seed * 2.1).sin() * 0.3
        + (t * 0.00137 + seed * 3.7).sin() * 0.2
}

/// A small xorshift, for blinks, glances and particles.
fn random(anim: &mut Anim) -> f64 {
    let mut x = anim.seed;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    anim.seed = x;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

fn ease_toward(value: f64, target: f64, dt: f64, time_ms: f64) -> f64 {
    value + (target - value) * (1.0 - (-dt / time_ms).exp())
}

/// Linear interpolation of `points` (dose, value), clamped at the ends.
fn along(points: &[(f64, f64)], dose: f64) -> f64 {
    let first = points[0];
    if dose <= first.0 {
        return first.1;
    }
    for pair in points.windows(2) {
        let ((d0, v0), (d1, v1)) = (pair[0], pair[1]);
        if dose <= d1 {
            return v0 + (v1 - v0) * (dose - d0) / (d1 - d0);
        }
    }
    points[points.len() - 1].1
}

fn source(cr: &cairo::Context, colour: Rgb, alpha: f64) {
    cr.set_source_rgba(colour.0, colour.1, colour.2, alpha.clamp(0.0, 1.0));
}

fn radial(cx: f64, cy: f64, radius: f64, stops: &[(f64, Rgb, f64)]) -> cairo::RadialGradient {
    let gradient = cairo::RadialGradient::new(cx, cy, 0.0, cx, cy, radius);
    for &(offset, colour, alpha) in stops {
        gradient.add_color_stop_rgba(offset, colour.0, colour.1, colour.2, alpha.clamp(0.0, 1.0));
    }
    gradient
}

fn disc(cr: &cairo::Context, x: f64, y: f64, radius: f64) {
    cr.arc(x, y, radius, 0.0, TAU);
    let _ = cr.fill();
}

/// Draw one frame. Returns how long until the next one is wanted.
fn draw(anim: &mut Anim, area: &gtk::DrawingArea, cr: &cairo::Context) -> Option<f64> {
    let now = area
        .frame_clock()
        .map_or(0.0, |clock| clock.frame_time() as f64 / 1000.0);
    let moving = animations_enabled();
    let dt = anim
        .last_tick
        .map_or(16.0, |last| (now - last).clamp(0.0, 250.0));
    anim.last_tick = Some(now);

    let asleep = anim.mode == Mode::Away;
    let nourishing = anim.mode == Mode::Nourishing;
    let as_f64 = |b: bool| f64::from(u8::from(b));

    // A change of site (or coming back): notice it straight away.
    let mut just_noticed = false;
    if anim.mode != anim.noticed {
        let woke = anim.noticed == Mode::Away;
        anim.noticed = anim.mode;
        if !asleep {
            anim.notice_start = now;
            just_noticed = true;
            // Look toward the page, where the user went.
            anim.glance_target = if woke { 0.0 } else { -1.0 };
            anim.next_glance = now + NOTICE_MS;
        }
    }

    let targets = (
        as_f64(anim.mode == Mode::Draining),
        as_f64(anim.mode == Mode::Resting),
        as_f64(anim.mode == Mode::Holding),
    );
    if moving {
        anim.shown = ease_toward(anim.shown, anim.target, dt, EASE_MS);
        anim.privacy = ease_toward(anim.privacy, as_f64(anim.private), dt, PRIVACY_MS / 3.0);
        anim.happy = ease_toward(anim.happy, as_f64(nourishing), dt, 400.0);
        anim.sleep = ease_toward(anim.sleep, as_f64(asleep), dt, 900.0);
        anim.wary = ease_toward(anim.wary, targets.0, dt, 450.0);
        anim.calm = ease_toward(anim.calm, targets.1, dt, 450.0);
        anim.curious = ease_toward(anim.curious, targets.2, dt, 450.0);
        anim.night_mix = ease_toward(anim.night_mix, as_f64(anim.night), dt, 4000.0);
        anim.welcome_mix = ease_toward(anim.welcome_mix, as_f64(anim.welcome), dt, 1500.0);
    } else {
        anim.shown = anim.target;
        anim.privacy = as_f64(anim.private);
        anim.happy = as_f64(nourishing);
        anim.sleep = as_f64(asleep);
        (anim.wary, anim.calm, anim.curious) = targets;
        anim.night_mix = as_f64(anim.night);
        anim.welcome_mix = as_f64(anim.welcome);
    }

    let dose = anim.shown;
    let mut look: Look = anim.stops.at(dose);
    look.glow = look.glow.mix(NIGHT_GLOW, NIGHT_WARMTH * anim.night_mix);
    look.core = look
        .core
        .mix(NIGHT_GLOW, NIGHT_WARMTH * 0.3 * anim.night_mix);
    let dark = dark();
    let height = anim.height;
    let rested = (1.0 - dose / 0.5).max(0.0);
    let engaged = if (0.15..0.6).contains(&dose) {
        1.0 - ((dose - 0.35) / 0.25).abs().min(1.0)
    } else {
        0.0
    };
    let clouded = ((dose - 0.45) / 0.3).clamp(0.0, 1.0);
    let drained = ((dose - 0.72) / 0.28).clamp(0.0, 1.0);
    let awake = 1.0 - anim.sleep;

    // The moss comes first: it's behind everything, and the wisp rests on it.
    let width = anim.width;
    let home_x = width / 2.0;
    draw_moss(cr, width, height, dose, anim.happy, dark, now, moving);

    // --- Where and how big ---------------------------------------------------
    let mut radius =
        BODY_RADIUS * along(&[(0.0, 1.0), (0.5, 0.88), (0.75, 0.78), (1.0, 0.7)], dose);
    let mut x = home_x;
    let mut y = height - MOSS_FLOOR - MOSS_HEIGHT - radius * 1.15;
    let mut brightness = (look.brightness * (1.0 + WELCOME_BRIGHTER * anim.welcome_mix)).min(1.0);
    let mut sway = 0.0;
    let mut blink = 0.0;

    let since_notice = now - anim.notice_start;
    let noticing = moving && (0.0..NOTICE_MS).contains(&since_notice);
    if noticing {
        y -= (since_notice / NOTICE_MS * PI).sin() * NOTICE_HOP;
    }
    brightness *= 1.0 - WARY_DIM * anim.wary;

    if moving {
        let slow = (1.0 + anim.sleep * 0.4) * (1.0 + (NIGHT_BREATH - 1.0) * anim.night_mix);
        let breath = (now / (BREATH_MS * slow) * TAU).sin();
        radius *= (1.0 + breath * BREATH_DEPTH) * (1.0 + WELCOME_BIGGER * anim.welcome_mix);
        y -= ((now / 2400.0 * TAU).sin() * 0.5 + 0.5) * 2.0 * rested * awake;
        x += wander(now, 1.3) * ROAM_X * engaged * awake;
        y -= wander(now, 4.2).abs() * ROAM_Y * engaged * awake;
        sway = wander(now * 1.7, 5.1) * radius * 0.22;

        // Clouded: an occasional flicker.
        let flick = (wander(now * 3.1, 7.7) - 0.55).max(0.0) * 1.6;
        brightness *= 1.0 - flick * clouded * 0.45 * awake;
        // Drained: unsteady, drifting toward the edge as if ready to leave.
        if drained > 0.0 {
            brightness *= 1.0 - wander(now * 5.0, 2.9).abs() * 0.25 * drained;
            let phase = (now % DRIFT_CYCLE_MS) / DRIFT_CYCLE_MS;
            let out = if phase < 0.3 {
                phase / 0.3
            } else if phase < 0.5 {
                1.0
            } else if phase < 0.8 {
                1.0 - (phase - 0.5) / 0.3
            } else {
                0.0
            };
            x += (width - EDGE_MARGIN - home_x) * out * out * (3.0 - 2.0 * out) * drained * awake;
        }

        // Blinks and glances.
        if now >= anim.next_blink {
            anim.next_blink = now + 2500.0 + random(anim) * 4500.0;
        }
        let until_blink = anim.next_blink - now;
        if until_blink < BLINK_MS {
            blink = 1.0 - ((until_blink / BLINK_MS) * 2.0 - 1.0).abs();
        }
        if now >= anim.next_glance {
            anim.next_glance = now + 2000.0 + random(anim) * 3000.0;
            anim.glance_target = (random(anim) * 2.0 - 1.0) * engaged.max(anim.curious * 0.8);
        }
        if just_noticed && nourishing {
            for _ in 0..3 {
                spawn(anim, now, x, y - radius, ParticleKind::Mote);
            }
            anim.last_mote = now;
        }
        anim.glance = ease_toward(anim.glance, anim.glance_target, dt, 250.0);

        // Particles.
        if anim.happy > 0.5 && now - anim.last_mote > MOTE_EVERY_MS * (0.6 + random(anim) * 0.8) {
            anim.last_mote = now;
            spawn(anim, now, x, y - radius, ParticleKind::Mote);
        }
        if clouded > 0.3
            && awake > 0.5
            && now - anim.last_smoke > SMOKE_EVERY_MS * (0.7 + random(anim) * 0.6)
        {
            anim.last_smoke = now;
            spawn(anim, now, x, y - radius * 1.4, ParticleKind::Smoke);
        }
        if anim.sleep > 0.8 && anim.privacy < 0.1 && now - anim.last_z > Z_EVERY_MS {
            anim.last_z = now;
            spawn(
                anim,
                now,
                x + radius * 0.9,
                y - radius * 0.9,
                ParticleKind::Z,
            );
        }
    }

    // Giving privacy: turn, slip toward the edge, fade.
    let privacy = anim.privacy;
    x += privacy * privacy * home_x;
    let presence = (1.0 - privacy * 1.4).clamp(0.0, 1.0);
    if nourishing {
        brightness = (brightness * 1.15).min(1.0);
    }

    if presence > 0.0 {
        draw_particles(anim, cr, now, look, dark, presence);
        draw_sprite(
            cr,
            Sprite {
                x,
                y,
                radius,
                sway,
                droop: drained,
                look,
                brightness,
                dark,
                presence,
            },
        );
        let openness = along(
            &[
                (0.0, 1.0),
                (0.35, 0.9),
                (0.6, 0.55),
                (0.85, 0.28),
                (1.0, 0.2),
            ],
            dose,
        ) * (1.0 - blink)
            * (1.0 - WARY_LIDS * anim.wary)
            * (1.0 - anim.sleep)
            * (1.0 - (privacy * 3.0).min(1.0));
        draw_face(
            cr,
            Face {
                x,
                y,
                radius,
                openness,
                happy: anim.happy * awake,
                glance: anim.glance,
                smile: (along(&[(0.0, 1.0), (0.3, 0.6), (0.55, 0.0)], dose)
                    * (1.0 - 0.7 * anim.wary))
                    .max(0.8 * anim.calm * awake)
                    * awake.max(0.4),
                blush: along(&[(0.0, 0.9), (0.5, 0.3), (1.0, 0.15)], dose),
                ink: if drained > 0.5 { 0.72 } else { 0.9 },
                presence,
            },
        );
    }

    if !moving {
        return None;
    }
    let settling = |v: f64| v > 0.02 && v < 0.98;
    let bursting = blink > 0.0
        || noticing
        || settling(anim.wary)
        || settling(anim.calm)
        || settling(anim.curious)
        || settling(anim.welcome_mix)
        || (anim.target - anim.shown).abs() > 0.002
        || settling(privacy)
        || settling(anim.sleep)
        || settling(anim.happy);
    let lively = anim.particles.iter().any(|p| p.kind != ParticleKind::Z)
        || nourishing
        || dose >= 0.45
        || engaged > 0.2;
    let fps = if bursting {
        BURST_FPS
    } else if anim.sleep > 0.9 || privacy > 0.99 {
        DOZING_FPS
    } else if lively {
        LIVELY_FPS
    } else {
        RESTING_FPS
    };
    let mut delay = 1000.0 / fps;
    // Wake in time for the next blink rather than stepping over it.
    if awake > 0.9 && privacy < 0.01 {
        let to_blink = anim.next_blink - BLINK_MS - now;
        if to_blink > 0.0 {
            delay = delay.min(to_blink);
        }
    }
    Some(delay)
}

/// A soft mound of moss along the nook's floor, with a few tufts.
#[allow(clippy::too_many_arguments)]
fn draw_moss(
    cr: &cairo::Context,
    width: f64,
    height: f64,
    dose: f64,
    happy: f64,
    dark: bool,
    now: f64,
    moving: bool,
) {
    let moss_left = width * (1.0 - MOSS_SPAN) / 2.0;
    let moss_right = width - moss_left;
    let tired = ((dose - 0.3) / 0.4).clamp(0.0, 1.0);
    let dry = ((dose - 0.7) / 0.3).clamp(0.0, 1.0);
    let base = MOSS_FRESH.mix(MOSS_TIRED, tired).mix(MOSS_DRY, dry);
    // A dark chrome wants deeper moss; a nourishing site perks it up.
    let colour = if dark {
        base.mix(Rgb(0.1, 0.12, 0.08), 0.35)
    } else {
        base
    };
    let colour = colour.mix(MOSS_FRESH, happy * 0.3);
    let floor = height - MOSS_FLOOR;
    let top = floor - MOSS_HEIGHT * (1.0 - dry * 0.25);

    source(cr, colour, 0.8);
    cr.move_to(moss_left, floor);
    cr.curve_to(
        moss_left + 16.0,
        top,
        moss_right - 16.0,
        top,
        moss_right,
        floor,
    );
    cr.close_path();
    let _ = cr.fill();

    // Cushions along the mound's top: plump and lighter when fresh, flatter
    // as they dry.
    let plump = 1.0 - dry * 0.4;
    let cushion = colour.mix(Rgb(0.85, 0.92, 0.6), 0.18 * (1.0 - dry));
    source(cr, cushion, 0.85);
    for (at, size) in [
        (0.14, 3.2),
        (0.24, 4.2),
        (0.35, 3.6),
        (0.47, 4.6),
        (0.58, 3.8),
        (0.69, 4.3),
        (0.8, 3.4),
        (0.89, 2.8),
    ] {
        let cx = moss_left + (moss_right - moss_left) * at;
        // Sit each cushion on the curve of the mound.
        let t = at;
        let ground = floor - (top - floor).abs() * 0.75 * (4.0 * t * (1.0 - t));
        let _ = cr.save();
        cr.translate(cx, ground);
        cr.scale(size * 1.15, size * plump);
        cr.arc(0.0, 0.0, 1.0, PI, TAU);
        let _ = cr.restore();
        let _ = cr.fill();
    }

    // A few short sprigs stand up when fresh and bow over as they dry.
    let lift = 1.0 - dry * 0.5;
    let stir = if moving {
        (now / 3100.0).sin() * 0.5
    } else {
        0.0
    };
    source(
        cr,
        cushion.mix(Rgb(0.9, 0.95, 0.7), 0.15 * (1.0 - dry)),
        0.9,
    );
    cr.set_line_width(1.2);
    cr.set_line_cap(cairo::LineCap::Round);
    for (i, (at, tall)) in [(0.3, 4.0), (0.52, 5.0), (0.74, 3.5)]
        .into_iter()
        .enumerate()
    {
        let bx = moss_left + (moss_right - moss_left) * at;
        let by = floor - MOSS_HEIGHT * 0.9;
        let rise = tall * lift;
        let lean = (dry * 2.0 + stir) * if i % 2 == 0 { 1.0 } else { -1.0 };
        cr.move_to(bx, by);
        cr.curve_to(
            bx,
            by - rise * 0.6,
            bx + lean * 0.4,
            by - rise * 0.9,
            bx + lean,
            by - rise,
        );
        let _ = cr.stroke();
        disc(cr, bx + lean, by - rise, 0.9 * lift);
    }
}

struct Sprite {
    x: f64,
    y: f64,
    radius: f64,
    sway: f64,
    droop: f64,
    look: Look,
    brightness: f64,
    dark: bool,
    presence: f64,
}

/// The flame-topped teardrop body and its glow.
fn draw_sprite(cr: &cairo::Context, s: Sprite) {
    let Sprite {
        x,
        y,
        radius: r,
        sway,
        droop,
        look,
        brightness,
        dark,
        presence,
    } = s;

    if !dark {
        let ink = Rgb(0.15, 0.16, 0.14);
        let well = r * 2.8;
        let _ = cr.set_source(radial(
            x,
            y,
            well,
            &[
                (0.0, ink, WELL_DEPTH * presence),
                (0.6, ink, WELL_DEPTH * 0.4 * presence),
                (1.0, ink, 0.0),
            ],
        ));
        disc(cr, x, y, well);
    }
    cr.set_operator(if dark {
        cairo::Operator::Add
    } else {
        cairo::Operator::Over
    });
    let halo = r * 2.4;
    let _ = cr.set_source(radial(
        x,
        y,
        halo,
        &[
            (0.0, look.glow, 0.55 * brightness * presence),
            (0.45, look.glow, 0.24 * brightness * presence),
            (1.0, look.glow, 0.0),
        ],
    ));
    disc(cr, x, y, halo);
    cr.set_operator(cairo::Operator::Over);

    // A dim wisp is smaller and softer, never muddy: the body stays lit.
    let lit = (0.55 + 0.45 * brightness) * presence;
    let warm = look.core.mix(look.glow, 0.3);
    let tip_lean = sway - droop * r * 0.55;
    cr.move_to(x - r, y);
    cr.curve_to(x - r, y + r * 1.05, x + r, y + r * 1.05, x + r, y);
    cr.curve_to(
        x + r,
        y - r * 0.75,
        x + r * 0.35 + tip_lean,
        y - r * 1.15,
        x + r * 0.15 + tip_lean * 1.6,
        y - r * (1.75 - droop * 0.25),
    );
    cr.curve_to(
        x - r * 0.05 + tip_lean,
        y - r * 1.2,
        x - r,
        y - r * 0.8,
        x - r,
        y,
    );
    cr.close_path();
    let body = cairo::RadialGradient::new(x - r * 0.25, y - r * 0.3, r * 0.1, x, y, r * 1.35);
    body.add_color_stop_rgba(0.0, look.core.0, look.core.1, look.core.2, lit);
    body.add_color_stop_rgba(0.7, warm.0, warm.1, warm.2, lit);
    body.add_color_stop_rgba(1.0, warm.0, warm.1, warm.2, lit * 0.9);
    let _ = cr.set_source(body);
    let _ = cr.fill();
}

struct Face {
    x: f64,
    y: f64,
    radius: f64,
    /// 1 = wide open, 0 = closed.
    openness: f64,
    /// 1 = happily closed (^ ^).
    happy: f64,
    /// -1 to 1, where the eyes look.
    glance: f64,
    /// 1 = a small smile, 0 = a flat line.
    smile: f64,
    blush: f64,
    ink: f64,
    presence: f64,
}

fn draw_face(cr: &cairo::Context, f: Face) {
    let r = f.radius;
    let alpha = f.presence * f.ink;
    let eye_y = f.y - r * 0.08;
    let (eye_w, eye_h) = (r * 0.2, r * 0.34);
    cr.set_line_cap(cairo::LineCap::Round);

    for side in [-1.0, 1.0] {
        // Cheeks first, under the eyes.
        let cheek_x = f.x + side * r * 0.62;
        let cheek_y = f.y + r * 0.2;
        let _ = cr.set_source(radial(
            cheek_x,
            cheek_y,
            r * 0.22,
            &[(0.0, BLUSH, 0.45 * f.blush * f.presence), (1.0, BLUSH, 0.0)],
        ));
        disc(cr, cheek_x, cheek_y, r * 0.22);

        let eye_x = f.x + side * r * 0.38 + f.glance * r * 0.09;
        if f.happy > 0.5 {
            // ^ ^
            source(cr, FACE_INK, alpha);
            cr.set_line_width(r * 0.09);
            cr.arc(
                eye_x,
                eye_y + eye_h * 0.2,
                eye_w * 0.62,
                PI * 1.15,
                PI * 1.85,
            );
            let _ = cr.stroke();
        } else if f.openness < 0.12 {
            // Closed and gently curved: asleep, or mid-blink.
            source(cr, FACE_INK, alpha);
            cr.set_line_width(r * 0.08);
            cr.arc(
                eye_x,
                eye_y - eye_h * 0.05,
                eye_w * 0.55,
                PI * 0.15,
                PI * 0.85,
            );
            let _ = cr.stroke();
        } else {
            // An oval with the lid lowered from the top.
            let _ = cr.save();
            let lid = eye_y - eye_h / 2.0 + eye_h * (1.0 - f.openness);
            cr.rectangle(eye_x - eye_w, lid, eye_w * 2.0, eye_h * 1.2);
            cr.clip();
            let _ = cr.save();
            cr.translate(eye_x, eye_y);
            cr.scale(eye_w / 2.0, eye_h / 2.0);
            cr.arc(0.0, 0.0, 1.0, 0.0, TAU);
            let _ = cr.restore();
            source(cr, FACE_INK, alpha);
            let _ = cr.fill();
            if f.openness > 0.5 {
                source(cr, Rgb(1.0, 1.0, 1.0), 0.9 * f.presence);
                disc(cr, eye_x - eye_w * 0.18, eye_y - eye_h * 0.2, eye_w * 0.2);
            }
            let _ = cr.restore();
        }
    }

    // The mouth: a small smile that flattens as the dose rises.
    source(cr, FACE_INK, alpha * 0.9);
    let mouth_y = f.y + r * 0.32;
    if f.happy > 0.5 {
        cr.arc(f.x, mouth_y - r * 0.12, r * 0.2, PI * 0.1, PI * 0.9);
        cr.close_path();
        let _ = cr.fill();
    } else {
        cr.set_line_width(r * 0.08);
        let width = r * 0.13;
        let curve = r * 0.1 * f.smile;
        cr.move_to(f.x - width, mouth_y);
        cr.curve_to(
            f.x - width * 0.4,
            mouth_y + curve,
            f.x + width * 0.4,
            mouth_y + curve,
            f.x + width,
            mouth_y,
        );
        let _ = cr.stroke();
    }
}

fn spawn(anim: &mut Anim, now: f64, x: f64, y: f64, kind: ParticleKind) {
    let jitter = random(anim) - 0.5;
    let spread = random(anim) - 0.5;
    let (life, dx, dy, size) = match kind {
        ParticleKind::Mote => (900.0, spread * 8.0, -16.0, 1.4),
        ParticleKind::Smoke => (2600.0, spread * 4.0, -12.0, 5.0),
        ParticleKind::Z => (2600.0, 6.0, -14.0, 3.2),
    };
    anim.particles.push(Particle {
        born: now,
        life,
        x: x + jitter * 4.0,
        y,
        dx,
        dy,
        size,
        kind,
    });
}

fn draw_particles(
    anim: &mut Anim,
    cr: &cairo::Context,
    now: f64,
    look: Look,
    dark: bool,
    presence: f64,
) {
    anim.particles.retain(|p| now - p.born < p.life);
    for p in &anim.particles {
        let age = (now - p.born) / p.life;
        let px = p.x + p.dx * age;
        let py = p.y + p.dy * age;
        let fade = (1.0 - age) * presence;
        match p.kind {
            ParticleKind::Mote => {
                source(cr, look.core, 0.85 * fade);
                disc(cr, px, py, p.size * (1.0 - age * 0.5));
            }
            ParticleKind::Smoke => {
                let size = p.size * (1.0 + age * 1.6);
                let grey = look.glow.mix(Rgb(0.6, 0.62, 0.66), 0.6);
                let _ = cr.set_source(radial(
                    px,
                    py,
                    size,
                    &[
                        (0.0, grey, 0.1 * fade),
                        (0.6, grey, 0.04 * fade),
                        (1.0, grey, 0.0),
                    ],
                ));
                disc(cr, px, py, size);
            }
            ParticleKind::Z => {
                let ink = if dark {
                    Rgb(0.85, 0.86, 0.83)
                } else {
                    Rgb(0.17, 0.18, 0.16)
                };
                source(cr, ink, 0.45 * fade);
                cr.set_line_width(1.1);
                cr.set_line_cap(cairo::LineCap::Round);
                cr.set_line_join(cairo::LineJoin::Round);
                let s = p.size * (1.0 + age * 0.5);
                cr.move_to(px, py);
                cr.line_to(px + s, py);
                cr.line_to(px, py + s);
                cr.line_to(px + s, py + s);
                let _ = cr.stroke();
            }
        }
    }
}

//! The wisp: a flame-shaped sprite with a friendly face, resting on a tuft of
//! moss in its nook.
//!
//! This is the whole of it — its moods, its motion, its face and its moss —
//! drawn on a `Canvas` and so the same wherever Glimmerwood runs. What it
//! cannot know it is told: the frame's time, whether the platform wants
//! animation, and whether the chrome around it is dark.
//!
//! Its colours along the dose come from `core/data/wisp.toml`; its face,
//! motion and moss are set by the constants below. It asks to be redrawn
//! only as often as what it is doing needs, and a still wisp (reduced
//! motion) only when the dose moves.

use std::f64::consts::{PI, TAU};

use crate::canvas::{Canvas, Paint, stop};
use crate::dose::{Mode, Trend};
use crate::look::{Look, Stops};
use crate::oklab::Rgb;

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
const WARY_LIDS: f64 = 0.34;
const WARY_DIM: f64 = 0.1;
/// Eyebrows: how far above the eyes they sit, and how much a mood lifts or
/// lowers them. They carry most of what the face says, so a mood reads from
/// across the room rather than only close up.
const BROW_ABOVE: f64 = 0.36;
const BROW_TRAVEL: f64 = 0.16;
/// Brows stay level and only move up or down: tilting either end turns the
/// face sad (inner end up) or stern (inner end down), and it is never either.
const BROW_ARCH: f64 = 0.06;
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
/// The sky in the nook: a wash behind the wisp saying which way the dose is
/// going, so recovering and wearing read at a glance rather than only from
/// the wisp's colour. Fresh green-blue while it recovers, a dusky haze while
/// it wears (never red, never alarm), nothing at all when it holds steady.
const SKY_RECOVERING: Rgb = Rgb(0.40, 0.75, 0.65);
const SKY_WEARING: Rgb = Rgb(0.37, 0.28, 0.54);
/// How strong that wash gets, and how long it takes to arrive: slow enough
/// that it reads as weather rather than a status light.
const SKY_ALPHA: f64 = 0.52;
const SKY_MS: f64 = 2500.0;
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

/// Everything the wisp is doing, and everything it is about to do.
pub struct Wisp {
    stops: Stops,
    target: f64,
    shown: f64,
    mode: Mode,
    private: bool,
    night: bool,
    welcome: bool,
    trend: Trend,
    /// 0-1, eased: how much the sky shows the dose falling or rising.
    recovering: f64,
    wearing: f64,
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
}

impl Wisp {
    /// A wisp asleep in a nook of this size, before the chrome says otherwise.
    pub fn new(width: f64, height: f64) -> Wisp {
        Wisp {
            stops: Stops::bundled(),
            target: 0.0,
            shown: 0.0,
            mode: Mode::Away,
            trend: Trend::Steady,
            recovering: 0.0,
            wearing: 0.0,
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
            width,
            height,
        }
    }

    /// The nook has been given a new size.
    pub fn resize(&mut self, width: f64, height: f64) {
        self.width = width;
        self.height = height;
    }

    /// `private`: the visible tab is on the privacy list, so the wisp slips
    /// out of sight. `night`: it winds down. `welcome`: the user has just
    /// come back after a long time away. Says whether anything moved, so a
    /// platform knows whether it is worth a redraw.
    pub fn update(
        &mut self,
        dose: f64,
        mode: Mode,
        trend: Trend,
        private: bool,
        night: bool,
        welcome: bool,
    ) -> bool {
        if (self.target - dose).abs() < 1e-6
            && self.mode == mode
            && self.trend == trend
            && self.private == private
            && self.night == night
            && self.welcome == welcome
        {
            return false;
        }
        self.target = dose;
        self.mode = mode;
        self.trend = trend;
        self.private = private;
        self.night = night;
        self.welcome = welcome;
        true
    }

    /// Draw one frame, at `now` milliseconds. `moving` is whether the
    /// platform allows animation, `dark` whether the chrome is dark. Returns
    /// how long until the next frame is wanted, or `None` to rest.
    pub fn draw(
        &mut self,
        now: f64,
        moving: bool,
        dark: bool,
        canvas: &mut dyn Canvas,
    ) -> Option<f64> {
        draw(self, now, moving, dark, canvas)
    }
}

fn wander(t: f64, seed: f64) -> f64 {
    (t * 0.00031 + seed).sin() * 0.5
        + (t * 0.00073 + seed * 2.1).sin() * 0.3
        + (t * 0.00137 + seed * 3.7).sin() * 0.2
}

/// A small xorshift, for blinks, glances and particles.
fn random(wisp: &mut Wisp) -> f64 {
    let mut x = wisp.seed;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    wisp.seed = x;
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

/// Draw one frame. Returns how long until the next one is wanted.
fn draw(wisp: &mut Wisp, now: f64, moving: bool, dark: bool, cr: &mut dyn Canvas) -> Option<f64> {
    let dt = wisp
        .last_tick
        .map_or(16.0, |last| (now - last).clamp(0.0, 250.0));
    wisp.last_tick = Some(now);

    let asleep = wisp.mode == Mode::Away;
    let nourishing = wisp.mode == Mode::Nourishing;
    let as_f64 = |b: bool| f64::from(u8::from(b));

    // A change of site (or coming back): notice it straight away.
    let mut just_noticed = false;
    if wisp.mode != wisp.noticed {
        let woke = wisp.noticed == Mode::Away;
        wisp.noticed = wisp.mode;
        if !asleep {
            wisp.notice_start = now;
            just_noticed = true;
            // Look toward the page, where the user went.
            wisp.glance_target = if woke { 0.0 } else { -1.0 };
            wisp.next_glance = now + NOTICE_MS;
        }
    }

    let targets = (
        as_f64(wisp.mode == Mode::Draining),
        as_f64(wisp.mode == Mode::Resting),
        as_f64(wisp.mode == Mode::Holding),
    );
    if moving {
        wisp.shown = ease_toward(wisp.shown, wisp.target, dt, EASE_MS);
        wisp.privacy = ease_toward(wisp.privacy, as_f64(wisp.private), dt, PRIVACY_MS / 3.0);
        wisp.happy = ease_toward(wisp.happy, as_f64(nourishing), dt, 400.0);
        wisp.sleep = ease_toward(wisp.sleep, as_f64(asleep), dt, 900.0);
        wisp.wary = ease_toward(wisp.wary, targets.0, dt, 450.0);
        wisp.calm = ease_toward(wisp.calm, targets.1, dt, 450.0);
        wisp.curious = ease_toward(wisp.curious, targets.2, dt, 450.0);
        wisp.recovering = ease_toward(
            wisp.recovering,
            as_f64(wisp.trend == Trend::Falling),
            dt,
            SKY_MS,
        );
        wisp.wearing = ease_toward(
            wisp.wearing,
            as_f64(wisp.trend == Trend::Rising),
            dt,
            SKY_MS,
        );
        wisp.night_mix = ease_toward(wisp.night_mix, as_f64(wisp.night), dt, 4000.0);
        wisp.welcome_mix = ease_toward(wisp.welcome_mix, as_f64(wisp.welcome), dt, 1500.0);
    } else {
        wisp.shown = wisp.target;
        wisp.privacy = as_f64(wisp.private);
        wisp.happy = as_f64(nourishing);
        wisp.sleep = as_f64(asleep);
        (wisp.wary, wisp.calm, wisp.curious) = targets;
        wisp.recovering = as_f64(wisp.trend == Trend::Falling);
        wisp.wearing = as_f64(wisp.trend == Trend::Rising);
        wisp.night_mix = as_f64(wisp.night);
        wisp.welcome_mix = as_f64(wisp.welcome);
    }

    let dose = wisp.shown;
    let mut look: Look = wisp.stops.at(dose);
    look.glow = look.glow.mix(NIGHT_GLOW, NIGHT_WARMTH * wisp.night_mix);
    look.core = look
        .core
        .mix(NIGHT_GLOW, NIGHT_WARMTH * 0.3 * wisp.night_mix);
    let height = wisp.height;
    let rested = (1.0 - dose / 0.5).max(0.0);
    let engaged = if (0.15..0.6).contains(&dose) {
        1.0 - ((dose - 0.35) / 0.25).abs().min(1.0)
    } else {
        0.0
    };
    let clouded = ((dose - 0.45) / 0.3).clamp(0.0, 1.0);
    let drained = ((dose - 0.72) / 0.28).clamp(0.0, 1.0);
    let awake = 1.0 - wisp.sleep;

    // The sky is behind everything, then the moss the wisp rests on.
    let width = wisp.width;
    let home_x = width / 2.0;
    draw_sky(
        cr,
        width,
        height,
        wisp.recovering * (1.0 - wisp.privacy),
        wisp.wearing * (1.0 - wisp.privacy),
        dark,
    );
    draw_moss(cr, width, height, dose, wisp.happy, dark, now, moving);

    // --- Where and how big ---------------------------------------------------
    let mut radius =
        BODY_RADIUS * along(&[(0.0, 1.0), (0.5, 0.88), (0.75, 0.78), (1.0, 0.7)], dose);
    let mut x = home_x;
    let mut y = height - MOSS_FLOOR - MOSS_HEIGHT - radius * 1.15;
    let mut brightness = (look.brightness * (1.0 + WELCOME_BRIGHTER * wisp.welcome_mix)).min(1.0);
    let mut sway = 0.0;
    let mut blink = 0.0;

    let since_notice = now - wisp.notice_start;
    let noticing = moving && (0.0..NOTICE_MS).contains(&since_notice);
    if noticing {
        y -= (since_notice / NOTICE_MS * PI).sin() * NOTICE_HOP;
    }
    brightness *= 1.0 - WARY_DIM * wisp.wary;

    if moving {
        let slow = (1.0 + wisp.sleep * 0.4) * (1.0 + (NIGHT_BREATH - 1.0) * wisp.night_mix);
        let breath = (now / (BREATH_MS * slow) * TAU).sin();
        radius *= (1.0 + breath * BREATH_DEPTH) * (1.0 + WELCOME_BIGGER * wisp.welcome_mix);
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
        if now >= wisp.next_blink {
            wisp.next_blink = now + 2500.0 + random(wisp) * 4500.0;
        }
        let until_blink = wisp.next_blink - now;
        if until_blink < BLINK_MS {
            blink = 1.0 - ((until_blink / BLINK_MS) * 2.0 - 1.0).abs();
        }
        if now >= wisp.next_glance {
            wisp.next_glance = now + 2000.0 + random(wisp) * 3000.0;
            wisp.glance_target = (random(wisp) * 2.0 - 1.0) * engaged.max(wisp.curious * 0.8);
        }
        if just_noticed && nourishing {
            for _ in 0..3 {
                spawn(wisp, now, x, y - radius, ParticleKind::Mote);
            }
            wisp.last_mote = now;
        }
        wisp.glance = ease_toward(wisp.glance, wisp.glance_target, dt, 250.0);

        // Particles.
        if wisp.happy > 0.5 && now - wisp.last_mote > MOTE_EVERY_MS * (0.6 + random(wisp) * 0.8) {
            wisp.last_mote = now;
            spawn(wisp, now, x, y - radius, ParticleKind::Mote);
        }
        if clouded > 0.3
            && awake > 0.5
            && now - wisp.last_smoke > SMOKE_EVERY_MS * (0.7 + random(wisp) * 0.6)
        {
            wisp.last_smoke = now;
            spawn(wisp, now, x, y - radius * 1.4, ParticleKind::Smoke);
        }
        if wisp.sleep > 0.8 && wisp.privacy < 0.1 && now - wisp.last_z > Z_EVERY_MS {
            wisp.last_z = now;
            spawn(
                wisp,
                now,
                x + radius * 0.9,
                y - radius * 0.9,
                ParticleKind::Z,
            );
        }
    }

    // Giving privacy: turn, slip toward the edge, fade.
    let privacy = wisp.privacy;
    x += privacy * privacy * home_x;
    let presence = (1.0 - privacy * 1.4).clamp(0.0, 1.0);
    if nourishing {
        brightness = (brightness * 1.15).min(1.0);
    }

    if presence > 0.0 {
        draw_particles(wisp, cr, now, look, dark, presence);
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
                (0.35, 0.88),
                (0.6, 0.42),
                (0.85, 0.18),
                (1.0, 0.1),
            ],
            dose,
        ) * (1.0 - blink)
            * (1.0 - WARY_LIDS * wisp.wary)
            * (1.0 - wisp.sleep)
            * (1.0 - (privacy * 3.0).min(1.0));
        draw_face(
            cr,
            Face {
                x,
                y,
                radius,
                openness,
                happy: wisp.happy * awake,
                glance: wisp.glance,
                smile: (along(&[(0.0, 1.0), (0.3, 0.65), (0.55, 0.05)], dose)
                    * (1.0 - 0.7 * wisp.wary))
                    .max(0.9 * wisp.calm * awake)
                    * awake.max(0.4),
                brow: (0.7 * wisp.happy * awake + 0.6 * wisp.curious * awake + 0.4 * rested
                    - 1.0 * wisp.wary
                    - 0.8 * clouded
                    - 0.5 * drained)
                    .clamp(-1.0, 1.0)
                    * (1.0 - wisp.sleep * 0.6),
                sleepy: (drained * awake).max(wisp.sleep),
                blush: along(&[(0.0, 1.0), (0.5, 0.3), (1.0, 0.12)], dose),
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
        || settling(wisp.wary)
        || settling(wisp.calm)
        || settling(wisp.curious)
        || settling(wisp.welcome_mix)
        || (wisp.target - wisp.shown).abs() > 0.002
        || settling(privacy)
        || settling(wisp.sleep)
        || settling(wisp.happy);
    let lively = wisp.particles.iter().any(|p| p.kind != ParticleKind::Z)
        || nourishing
        || dose >= 0.45
        || engaged > 0.2;
    let fps = if bursting {
        BURST_FPS
    } else if wisp.sleep > 0.9 || privacy > 0.99 {
        DOZING_FPS
    } else if lively {
        LIVELY_FPS
    } else {
        RESTING_FPS
    };
    let mut delay = 1000.0 / fps;
    // Wake in time for the next blink rather than stepping over it.
    if awake > 0.9 && privacy < 0.01 {
        let to_blink = wisp.next_blink - BLINK_MS - now;
        if to_blink > 0.0 {
            delay = delay.min(to_blink);
        }
    }
    Some(delay)
}

/// The nook's weather: a soft wash of colour behind the wisp, fading out
/// downward so the moss keeps its own green.
fn draw_sky(
    cr: &mut dyn Canvas,
    width: f64,
    height: f64,
    recovering: f64,
    wearing: f64,
    dark: bool,
) {
    let strength = recovering.max(wearing);
    if strength < 0.01 {
        return;
    }
    let colour = SKY_RECOVERING.mix(SKY_WEARING, wearing / (recovering + wearing).max(1e-6));
    // A dark chrome takes less of it: the same wash reads twice as strong.
    let alpha = SKY_ALPHA * strength * if dark { 0.6 } else { 1.0 };
    cr.set_paint(Paint::Linear {
        from: (0.0, 0.0),
        to: (0.0, height),
        stops: &[
            stop(0.0, colour, alpha),
            stop(0.65, colour, alpha * 0.45),
            stop(1.0, colour, 0.0),
        ],
    });
    // Rounded like the nook it sits in.
    let r = height / 3.2;
    cr.new_path();
    cr.arc(r, r, r, PI, 1.5 * PI);
    cr.arc(width - r, r, r, 1.5 * PI, TAU);
    cr.line_to(width, height);
    cr.line_to(0.0, height);
    cr.close_path();
    cr.fill();
}

/// A soft mound of moss along the nook's floor, with a few tufts.
#[allow(clippy::too_many_arguments)]
fn draw_moss(
    cr: &mut dyn Canvas,
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

    cr.set_paint(Paint::solid(colour, 0.8));
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
    cr.fill();

    // Cushions along the mound's top: plump and lighter when fresh, flatter
    // as they dry.
    let plump = 1.0 - dry * 0.4;
    let cushion = colour.mix(Rgb(0.85, 0.92, 0.6), 0.18 * (1.0 - dry));
    cr.set_paint(Paint::solid(cushion, 0.85));
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
        cr.save();
        cr.translate(cx, ground);
        cr.scale(size * 1.15, size * plump);
        cr.arc(0.0, 0.0, 1.0, PI, TAU);
        cr.restore();
        cr.fill();
    }

    // A few short sprigs stand up when fresh and bow over as they dry.
    let lift = 1.0 - dry * 0.5;
    let stir = if moving {
        (now / 3100.0).sin() * 0.5
    } else {
        0.0
    };
    cr.set_paint(Paint::solid(
        cushion.mix(Rgb(0.9, 0.95, 0.7), 0.15 * (1.0 - dry)),
        0.9,
    ));
    cr.set_line_width(1.2);
    cr.set_round_ends(true);
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
        cr.stroke();
        cr.disc(bx + lean, by - rise, 0.9 * lift);
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
fn draw_sprite(cr: &mut dyn Canvas, s: Sprite) {
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
        cr.set_paint(Paint::glow(
            x,
            y,
            well,
            &[
                stop(0.0, ink, WELL_DEPTH * presence),
                stop(0.6, ink, WELL_DEPTH * 0.4 * presence),
                stop(1.0, ink, 0.0),
            ],
        ));
        cr.disc(x, y, well);
    }
    cr.set_adding(dark);
    let halo = r * 2.4;
    cr.set_paint(Paint::glow(
        x,
        y,
        halo,
        &[
            stop(0.0, look.glow, 0.55 * brightness * presence),
            stop(0.45, look.glow, 0.24 * brightness * presence),
            stop(1.0, look.glow, 0.0),
        ],
    ));
    cr.disc(x, y, halo);
    cr.set_adding(false);

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
    cr.set_paint(Paint::Radial {
        inner: (x - r * 0.25, y - r * 0.3, r * 0.1),
        outer: (x, y, r * 1.35),
        stops: &[
            stop(0.0, look.core, lit),
            stop(0.7, warm, lit),
            stop(1.0, warm, lit * 0.9),
        ],
    });
    cr.fill();
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
    /// 1 = brows lifted and open, -1 = lowered and watchful.
    brow: f64,
    /// 1 = heavy-lidded, mouth fallen open a little: sleepy, never sad.
    sleepy: f64,
    blush: f64,
    ink: f64,
    presence: f64,
}

fn draw_face(cr: &mut dyn Canvas, f: Face) {
    let r = f.radius;
    let alpha = f.presence * f.ink;
    let eye_y = f.y - r * 0.08;
    let (eye_w, eye_h) = (r * 0.22, r * 0.38);
    cr.set_round_ends(true);

    for side in [-1.0, 1.0] {
        // Cheeks first, under the eyes.
        let cheek_x = f.x + side * r * 0.62;
        let cheek_y = f.y + r * 0.2;
        cr.set_paint(Paint::glow(
            cheek_x,
            cheek_y,
            r * 0.22,
            &[
                stop(0.0, BLUSH, 0.45 * f.blush * f.presence),
                stop(1.0, BLUSH, 0.0),
            ],
        ));
        cr.disc(cheek_x, cheek_y, r * 0.22);

        let eye_x = f.x + side * r * 0.4 + f.glance * r * 0.09;

        // The brow, a short stroke that lifts with a bright mood and lowers
        // with a heavy one, its outer end dipping as it goes.
        if f.presence > 0.0 && f.openness > 0.05 || f.happy > 0.5 {
            let brow_y = eye_y - r * BROW_ABOVE - r * BROW_TRAVEL * f.brow;
            // Arched while the mood is bright, flat while it is heavy.
            let arch = r * BROW_ARCH * f.brow.max(0.0);
            cr.set_paint(Paint::solid(FACE_INK, alpha * 0.8));
            cr.set_line_width(r * 0.062);
            cr.move_to(eye_x - eye_w * 0.85, brow_y);
            cr.curve_to(
                eye_x - eye_w * 0.3,
                brow_y - arch,
                eye_x + eye_w * 0.3,
                brow_y - arch,
                eye_x + eye_w * 0.85,
                brow_y,
            );
            cr.stroke();
        }

        if f.happy > 0.5 {
            // ^ ^
            cr.set_paint(Paint::solid(FACE_INK, alpha));
            cr.set_line_width(r * 0.09);
            cr.set_line_width(r * 0.11);
            cr.arc(
                eye_x,
                eye_y + eye_h * 0.24,
                eye_w * 0.78,
                PI * 1.1,
                PI * 1.9,
            );
            cr.stroke();
        } else if f.openness < 0.12 {
            // Closed and gently curved: asleep, or mid-blink.
            cr.set_paint(Paint::solid(FACE_INK, alpha));
            cr.set_line_width(r * 0.08);
            cr.arc(
                eye_x,
                eye_y - eye_h * 0.05,
                eye_w * 0.55,
                PI * 0.15,
                PI * 0.85,
            );
            cr.stroke();
        } else {
            // An oval with the lid lowered from the top.
            cr.save();
            let lid = eye_y - eye_h / 2.0 + eye_h * (1.0 - f.openness);
            cr.rectangle(eye_x - eye_w, lid, eye_w * 2.0, eye_h * 1.2);
            cr.clip();
            cr.save();
            cr.translate(eye_x, eye_y);
            cr.scale(eye_w / 2.0, eye_h / 2.0);
            cr.arc(0.0, 0.0, 1.0, 0.0, TAU);
            cr.restore();
            cr.set_paint(Paint::solid(FACE_INK, alpha));
            cr.fill();
            if f.openness > 0.5 {
                cr.set_paint(Paint::solid(Rgb(1.0, 1.0, 1.0), 0.9 * f.presence));
                cr.disc(eye_x - eye_w * 0.18, eye_y - eye_h * 0.2, eye_w * 0.2);
            }
            cr.restore();
        }
    }

    // The mouth: a smile that flattens as the dose rises, and falls a little
    // open when the wisp is sleepy.
    cr.set_paint(Paint::solid(FACE_INK, alpha * 0.9));
    let mouth_y = f.y + r * 0.32;
    if f.happy > 0.5 {
        cr.arc(f.x, mouth_y - r * 0.14, r * 0.26, PI * 0.1, PI * 0.9);
        cr.close_path();
        cr.fill();
    } else if f.sleepy > 0.6 {
        // A small oval, as if halfway through a yawn.
        cr.save();
        cr.translate(f.x, mouth_y);
        cr.scale(r * 0.1, r * 0.13 * f.sleepy);
        cr.arc(0.0, 0.0, 1.0, 0.0, TAU);
        cr.restore();
        cr.fill();
    } else {
        cr.set_line_width(r * 0.085);
        let width = r * 0.15;
        let curve = r * 0.16 * f.smile;
        cr.move_to(f.x - width, mouth_y);
        cr.curve_to(
            f.x - width * 0.4,
            mouth_y + curve,
            f.x + width * 0.4,
            mouth_y + curve,
            f.x + width,
            mouth_y,
        );
        cr.stroke();
    }
}

fn spawn(wisp: &mut Wisp, now: f64, x: f64, y: f64, kind: ParticleKind) {
    let jitter = random(wisp) - 0.5;
    let spread = random(wisp) - 0.5;
    let (life, dx, dy, size) = match kind {
        ParticleKind::Mote => (900.0, spread * 8.0, -16.0, 1.4),
        ParticleKind::Smoke => (2600.0, spread * 4.0, -12.0, 5.0),
        ParticleKind::Z => (2600.0, 6.0, -14.0, 3.2),
    };
    wisp.particles.push(Particle {
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
    wisp: &mut Wisp,
    cr: &mut dyn Canvas,
    now: f64,
    look: Look,
    dark: bool,
    presence: f64,
) {
    wisp.particles.retain(|p| now - p.born < p.life);
    for p in &wisp.particles {
        let age = (now - p.born) / p.life;
        let px = p.x + p.dx * age;
        let py = p.y + p.dy * age;
        let fade = (1.0 - age) * presence;
        match p.kind {
            ParticleKind::Mote => {
                cr.set_paint(Paint::solid(look.core, 0.85 * fade));
                cr.disc(px, py, p.size * (1.0 - age * 0.5));
            }
            ParticleKind::Smoke => {
                let size = p.size * (1.0 + age * 1.6);
                let grey = look.glow.mix(Rgb(0.6, 0.62, 0.66), 0.6);
                cr.set_paint(Paint::glow(
                    px,
                    py,
                    size,
                    &[
                        stop(0.0, grey, 0.1 * fade),
                        stop(0.6, grey, 0.04 * fade),
                        stop(1.0, grey, 0.0),
                    ],
                ));
                cr.disc(px, py, size);
            }
            ParticleKind::Z => {
                let ink = if dark {
                    Rgb(0.85, 0.86, 0.83)
                } else {
                    Rgb(0.17, 0.18, 0.16)
                };
                cr.set_paint(Paint::solid(ink, 0.45 * fade));
                cr.set_line_width(1.1);
                cr.set_round_ends(true);
                let s = p.size * (1.0 + age * 0.5);
                cr.move_to(px, py);
                cr.line_to(px + s, py);
                cr.line_to(px, py + s);
                cr.line_to(px + s, py + s);
                cr.stroke();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::svg::SvgCanvas;

    const NOOK: (f64, f64) = (152.0, 56.0);
    /// Long enough for everything that eases to have arrived.
    const FRAMES: usize = 400;

    /// The wisp in one of its moods, drawn as SVG. Every moment it depends on
    /// is handed in, so the same call always draws the same thing.
    fn render(dose: f64, mode: Mode, trend: Trend, night: bool, dark: bool) -> String {
        let mut wisp = Wisp::new(NOOK.0, NOOK.1);
        wisp.update(dose, mode, trend, false, night, false);
        let mut drawn = String::new();
        for frame in 0..FRAMES {
            let mut canvas = SvgCanvas::new(NOOK.0, NOOK.1);
            wisp.draw(frame as f64 * 16.0, true, dark, &mut canvas);
            drawn = canvas.finish();
        }
        drawn
    }

    /// The wisp has no tests of its own beyond this: it is a drawing, and what
    /// matters is that it keeps looking like itself. Each of these is a mood
    /// rendered on a canvas that shares nothing with cairo, kept beside the
    /// code. Run with `GLIMMERWOOD_BLESS=1` to write them down again after a
    /// change, and look at what moved before committing it.
    #[test]
    fn the_wisp_still_looks_like_itself() {
        let moods: [(&str, f64, Mode, Trend, bool, bool); 6] = [
            (
                "rested",
                0.05,
                Mode::Nourishing,
                Trend::Falling,
                false,
                false,
            ),
            ("everyday", 0.35, Mode::Holding, Trend::Steady, false, false),
            ("clouded", 0.62, Mode::Draining, Trend::Rising, false, false),
            ("drained", 0.92, Mode::Draining, Trend::Rising, false, false),
            ("night", 0.4, Mode::Resting, Trend::Falling, true, false),
            ("away-dark", 0.3, Mode::Away, Trend::Steady, false, true),
        ];

        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/wisp/");
        let bless = std::env::var_os("GLIMMERWOOD_BLESS").is_some();
        for (name, dose, mode, trend, night, dark) in moods {
            let drawn = render(dose, mode, trend, night, dark);
            let path = format!("{dir}{name}.svg");
            if bless {
                std::fs::write(&path, &drawn).expect("write the wisp's picture");
                continue;
            }
            let kept = std::fs::read_to_string(&path)
                .unwrap_or_else(|_| panic!("{name}.svg is missing; run with GLIMMERWOOD_BLESS=1"));
            assert_eq!(
                kept, drawn,
                "the wisp draws {name} differently now; \
                 look at it, and if it is right run with GLIMMERWOOD_BLESS=1"
            );
        }
    }

    #[test]
    fn a_wisp_that_is_told_nothing_new_asks_for_no_redraw() {
        let mut wisp = Wisp::new(NOOK.0, NOOK.1);
        assert!(wisp.update(0.5, Mode::Draining, Trend::Rising, false, false, false));
        assert!(!wisp.update(0.5, Mode::Draining, Trend::Rising, false, false, false));
    }
}

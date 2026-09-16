//! The wisp, drawn into a buffer of pixels.
//!
//! GTK lends the wisp cairo, and the welcome page gets it as SVG. This is the
//! third surface behind the same `Canvas`: plain Rust, no toolkit, drawing
//! into a buffer of pixels that can be compared byte for byte. That is what
//! the mood tests hold the drawing to, so a change to the wisp's face has to
//! look the same through every surface it goes out on.
//!
//! The wisp is 152x56 and asks for two to six frames a second. A CPU does not
//! notice that.

use glimmerwood_core::canvas::{Canvas, Paint, Stop};
use glimmerwood_core::oklab::Rgb;
use tiny_skia::{
    BlendMode, FillRule, GradientStop, LineCap, LineJoin, LinearGradient, Mask, Paint as SkPaint,
    PathBuilder, Pixmap, Point, RadialGradient, Shader, SpreadMode, Stroke, Transform,
};

/// `[a, b, c, d, e, f]`, as every 2D library writes an affine transform.
type Matrix = [f64; 6];

const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

#[derive(Clone)]
enum Ink {
    Solid(Rgb, f64),
    Radial {
        inner: (f64, f64, f64),
        outer: (f64, f64, f64),
        stops: Vec<Stop>,
    },
    Linear {
        from: (f64, f64),
        to: (f64, f64),
        stops: Vec<Stop>,
    },
}

#[derive(Clone)]
struct State {
    matrix: Matrix,
    ink: Ink,
    width: f64,
    round: bool,
    adding: bool,
    clip: Option<Mask>,
}

/// A canvas that paints into a pixmap.
pub struct Raster {
    pixmap: Pixmap,
    state: State,
    stack: Vec<State>,
    path: PathBuilder,
    /// Where the path is, so a `close` and an `arc` know whether one is open.
    started: bool,
}

impl Raster {
    pub fn new(width: u32, height: u32) -> Option<Raster> {
        Some(Raster {
            pixmap: Pixmap::new(width, height)?,
            state: State {
                matrix: IDENTITY,
                ink: Ink::Solid(Rgb(0.0, 0.0, 0.0), 1.0),
                width: 1.0,
                round: false,
                adding: false,
                clip: None,
            },
            stack: Vec::new(),
            path: PathBuilder::new(),
            started: false,
        })
    }

    /// The pixels, premultiplied RGBA, eight bits each, row by row.
    pub fn data(&self) -> &[u8] {
        self.pixmap.data()
    }

    pub fn width(&self) -> u32 {
        self.pixmap.width()
    }

    pub fn height(&self) -> u32 {
        self.pixmap.height()
    }

    /// The pixels as a PNG, for looking at.
    pub fn png(&self) -> Option<Vec<u8>> {
        self.pixmap.encode_png().ok()
    }

    fn at(&self, x: f64, y: f64) -> (f32, f32) {
        let [a, b, c, d, e, f] = self.state.matrix;
        ((a * x + c * y + e) as f32, (b * x + d * y + f) as f32)
    }

    fn scale_of(&self) -> f64 {
        let [a, b, c, d, ..] = self.state.matrix;
        (((a * a + b * b).sqrt() + (c * c + d * d).sqrt()) / 2.0).abs()
    }

    fn shader(&self) -> Shader<'static> {
        match &self.state.ink {
            Ink::Solid(colour, alpha) => Shader::SolidColor(colour_of(*colour, *alpha)),
            Ink::Radial {
                inner,
                outer,
                stops,
            } => {
                let (fx, fy) = self.at(inner.0, inner.1);
                let (cx, cy) = self.at(outer.0, outer.1);
                let scale = self.scale_of();
                RadialGradient::new(
                    Point::from_xy(fx, fy),
                    (inner.2 * scale) as f32,
                    Point::from_xy(cx, cy),
                    (outer.2 * scale).max(0.01) as f32,
                    stops_of(stops),
                    SpreadMode::Pad,
                    Transform::identity(),
                )
                .unwrap_or_else(|| Shader::SolidColor(last_colour(stops)))
            }
            Ink::Linear { from, to, stops } => {
                let (x1, y1) = self.at(from.0, from.1);
                let (x2, y2) = self.at(to.0, to.1);
                LinearGradient::new(
                    Point::from_xy(x1, y1),
                    Point::from_xy(x2, y2),
                    stops_of(stops),
                    SpreadMode::Pad,
                    Transform::identity(),
                )
                .unwrap_or_else(|| Shader::SolidColor(last_colour(stops)))
            }
        }
    }

    fn paint(&self) -> SkPaint<'static> {
        SkPaint {
            shader: self.shader(),
            anti_alias: true,
            blend_mode: if self.state.adding {
                BlendMode::Plus
            } else {
                BlendMode::SourceOver
            },
            ..SkPaint::default()
        }
    }
}

fn colour_of(Rgb(r, g, b): Rgb, alpha: f64) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba(
        r.clamp(0.0, 1.0) as f32,
        g.clamp(0.0, 1.0) as f32,
        b.clamp(0.0, 1.0) as f32,
        alpha.clamp(0.0, 1.0) as f32,
    )
    .unwrap_or(tiny_skia::Color::TRANSPARENT)
}

fn stops_of(stops: &[Stop]) -> Vec<GradientStop> {
    stops
        .iter()
        .map(|s| GradientStop::new(s.at as f32, colour_of(s.colour, s.alpha)))
        .collect()
}

/// What to paint with if a gradient turns out to be degenerate.
fn last_colour(stops: &[Stop]) -> tiny_skia::Color {
    stops.last().map_or(tiny_skia::Color::TRANSPARENT, |s| {
        colour_of(s.colour, s.alpha)
    })
}

impl Canvas for Raster {
    fn save(&mut self) {
        self.stack.push(self.state.clone());
    }

    fn restore(&mut self) {
        if let Some(state) = self.stack.pop() {
            self.state = state;
        }
    }

    fn translate(&mut self, dx: f64, dy: f64) {
        let [a, b, c, d, e, f] = self.state.matrix;
        self.state.matrix = [a, b, c, d, e + a * dx + c * dy, f + b * dx + d * dy];
    }

    fn scale(&mut self, sx: f64, sy: f64) {
        let [a, b, c, d, e, f] = self.state.matrix;
        self.state.matrix = [a * sx, b * sx, c * sy, d * sy, e, f];
    }

    fn move_to(&mut self, x: f64, y: f64) {
        let (x, y) = self.at(x, y);
        self.path.move_to(x, y);
        self.started = true;
    }

    fn line_to(&mut self, x: f64, y: f64) {
        let (x, y) = self.at(x, y);
        if self.started {
            self.path.line_to(x, y);
        } else {
            self.path.move_to(x, y);
            self.started = true;
        }
    }

    fn curve_to(&mut self, c1x: f64, c1y: f64, c2x: f64, c2y: f64, x: f64, y: f64) {
        let (c1x, c1y) = self.at(c1x, c1y);
        let (c2x, c2y) = self.at(c2x, c2y);
        let (x, y) = self.at(x, y);
        if !self.started {
            self.path.move_to(c1x, c1y);
            self.started = true;
        }
        self.path.cubic_to(c1x, c1y, c2x, c2y, x, y);
    }

    fn arc(&mut self, x: f64, y: f64, radius: f64, from: f64, to: f64) {
        // Walked as segments, fine enough that nothing shows at this size.
        let steps = (((to - from).abs() * radius.max(1.0)) as usize).clamp(8, 180);
        for step in 0..=steps {
            let angle = from + (to - from) * (step as f64 / steps as f64);
            let (px, py) = self.at(x + radius * angle.cos(), y + radius * angle.sin());
            if step == 0 && !self.started {
                self.path.move_to(px, py);
                self.started = true;
            } else {
                self.path.line_to(px, py);
            }
        }
    }

    fn rectangle(&mut self, x: f64, y: f64, width: f64, height: f64) {
        self.move_to(x, y);
        self.line_to(x + width, y);
        self.line_to(x + width, y + height);
        self.line_to(x, y + height);
        self.close_path();
    }

    fn close_path(&mut self) {
        if self.started {
            self.path.close();
        }
    }

    fn new_path(&mut self) {
        self.path = PathBuilder::new();
        self.started = false;
    }

    fn set_paint(&mut self, paint: Paint<'_>) {
        self.state.ink = match paint {
            Paint::Solid { colour, alpha } => Ink::Solid(colour, alpha),
            Paint::Radial {
                inner,
                outer,
                stops,
            } => Ink::Radial {
                inner,
                outer,
                stops: stops.to_vec(),
            },
            Paint::Linear { from, to, stops } => Ink::Linear {
                from,
                to,
                stops: stops.to_vec(),
            },
        };
    }

    fn set_line_width(&mut self, width: f64) {
        self.state.width = width;
    }

    fn set_round_ends(&mut self, round: bool) {
        self.state.round = round;
    }

    fn set_adding(&mut self, adding: bool) {
        self.state.adding = adding;
    }

    fn fill(&mut self) {
        let Some(path) = std::mem::take(&mut self.path).finish() else {
            self.started = false;
            return;
        };
        self.started = false;
        let paint = self.paint();
        self.pixmap.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            self.state.clip.as_ref(),
        );
    }

    fn stroke(&mut self) {
        let Some(path) = std::mem::take(&mut self.path).finish() else {
            self.started = false;
            return;
        };
        self.started = false;
        let paint = self.paint();
        let stroke = Stroke {
            width: (self.state.width * self.scale_of()) as f32,
            line_cap: if self.state.round {
                LineCap::Round
            } else {
                LineCap::Butt
            },
            line_join: if self.state.round {
                LineJoin::Round
            } else {
                LineJoin::Miter
            },
            ..Stroke::default()
        };
        self.pixmap.stroke_path(
            &path,
            &paint,
            &stroke,
            Transform::identity(),
            self.state.clip.as_ref(),
        );
    }

    fn clip(&mut self) {
        let Some(path) = std::mem::take(&mut self.path).finish() else {
            self.started = false;
            return;
        };
        self.started = false;
        let mut mask = Mask::new(self.pixmap.width(), self.pixmap.height());
        let Some(mask) = mask.as_mut() else { return };
        mask.fill_path(&path, FillRule::Winding, true, Transform::identity());
        // Narrowing, never widening: a clip inside a clip keeps both.
        let mask = match self.state.clip.take() {
            Some(outer) => {
                let mut both = mask.clone();
                for (pixel, was) in both.data_mut().iter_mut().zip(outer.data()) {
                    *pixel = ((u16::from(*pixel) * u16::from(*was)) / 255) as u8;
                }
                both
            }
            None => mask.clone(),
        };
        self.state.clip = Some(mask);
    }
}

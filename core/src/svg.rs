//! A canvas that writes SVG.
//!
//! cairo is the only surface the wisp has had, and a surface that agrees with
//! cairo about everything proves nothing about the trait between them. This
//! one shares no code with it at all, so what it draws is a second opinion —
//! and being plain text, it can be checked into the tests and compared.

use std::fmt::Write as _;

use crate::canvas::{Canvas, Paint, Stop};
use crate::oklab::Rgb;

/// Coordinates are rounded to this many places. The wisp's shape comes out of
/// sines and exponentials, and different platforms' libm answer those a
/// fraction differently; two places is far below anything visible and far
/// above that noise.
const PLACES: usize = 2;

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
    /// Clip groups opened since this state was saved, to close on restore.
    groups: usize,
}

pub struct SvgCanvas {
    body: String,
    defs: String,
    state: State,
    stack: Vec<State>,
    path: String,
    ids: u32,
    width: f64,
    height: f64,
}

impl SvgCanvas {
    pub fn new(width: f64, height: f64) -> SvgCanvas {
        SvgCanvas {
            body: String::new(),
            defs: String::new(),
            state: State {
                matrix: IDENTITY,
                ink: Ink::Solid(Rgb(0.0, 0.0, 0.0), 1.0),
                width: 1.0,
                round: false,
                adding: false,
                groups: 0,
            },
            stack: Vec::new(),
            path: String::new(),
            ids: 0,
            width,
            height,
        }
    }

    /// The finished drawing.
    pub fn finish(mut self) -> String {
        while self.state.groups > 0 {
            self.body.push_str("</g>");
            self.state.groups -= 1;
        }
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" \
             viewBox=\"0 0 {} {}\">\n<defs>\n{}</defs>\n{}</svg>\n",
            round(self.width),
            round(self.height),
            round(self.width),
            round(self.height),
            self.defs,
            self.body,
        )
    }

    fn id(&mut self) -> String {
        self.ids += 1;
        format!("g{}", self.ids)
    }

    /// A point, in the transform in force — the same moment cairo takes it.
    fn at(&self, x: f64, y: f64) -> (f64, f64) {
        let [a, b, c, d, e, f] = self.state.matrix;
        (a * x + c * y + e, b * x + d * y + f)
    }

    fn scale_of(&self) -> f64 {
        let [a, b, c, d, ..] = self.state.matrix;
        (((a * a + b * b).sqrt() + (c * c + d * d).sqrt()) / 2.0).abs()
    }

    /// The paint, as a `fill`/`stroke` value, defining a gradient if it needs one.
    fn ink(&mut self) -> String {
        match self.state.ink.clone() {
            Ink::Solid(colour, alpha) => colour_of(colour, alpha),
            Ink::Radial {
                inner,
                outer,
                stops,
            } => {
                let id = self.id();
                let (fx, fy) = self.at(inner.0, inner.1);
                let (cx, cy) = self.at(outer.0, outer.1);
                let r = outer.2 * self.scale_of();
                let _ = write!(
                    self.defs,
                    "<radialGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" \
                     cx=\"{}\" cy=\"{}\" r=\"{}\" fx=\"{}\" fy=\"{}\">{}</radialGradient>\n",
                    round(cx),
                    round(cy),
                    round(r),
                    round(fx),
                    round(fy),
                    stops_of(&stops),
                );
                format!("url(#{id})")
            }
            Ink::Linear { from, to, stops } => {
                let id = self.id();
                let (x1, y1) = self.at(from.0, from.1);
                let (x2, y2) = self.at(to.0, to.1);
                let _ = write!(
                    self.defs,
                    "<linearGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" \
                     x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\">{}</linearGradient>\n",
                    round(x1),
                    round(y1),
                    round(x2),
                    round(y2),
                    stops_of(&stops),
                );
                format!("url(#{id})")
            }
        }
    }

    fn blend(&self) -> &'static str {
        if self.state.adding {
            " style=\"mix-blend-mode:plus-lighter\""
        } else {
            ""
        }
    }
}

fn round(value: f64) -> String {
    let text = format!("{value:.PLACES$}");
    // So that -0.00 and 0.00 are the same thing to a comparison.
    if text
        .trim_start_matches('-')
        .chars()
        .all(|c| c == '0' || c == '.')
    {
        return "0".into();
    }
    let text = text.trim_end_matches('0').trim_end_matches('.').to_string();
    if text.is_empty() { "0".into() } else { text }
}

fn colour_of(Rgb(r, g, b): Rgb, alpha: f64) -> String {
    let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let alpha = alpha.clamp(0.0, 1.0);
    if alpha >= 1.0 {
        format!("#{:02x}{:02x}{:02x}", byte(r), byte(g), byte(b))
    } else {
        format!("rgba({},{},{},{})", byte(r), byte(g), byte(b), round(alpha))
    }
}

fn stops_of(stops: &[Stop]) -> String {
    stops
        .iter()
        .map(|s| {
            format!(
                "<stop offset=\"{}\" stop-color=\"{}\" stop-opacity=\"{}\"/>",
                round(s.at),
                colour_of(s.colour, 1.0),
                round(s.alpha.clamp(0.0, 1.0)),
            )
        })
        .collect()
}

impl Canvas for SvgCanvas {
    fn save(&mut self) {
        self.stack.push(self.state.clone());
        self.state.groups = 0;
    }

    fn restore(&mut self) {
        while self.state.groups > 0 {
            self.body.push_str("</g>");
            self.state.groups -= 1;
        }
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
        let _ = write!(self.path, "M{} {}", round(x), round(y));
    }

    fn line_to(&mut self, x: f64, y: f64) {
        let (x, y) = self.at(x, y);
        let _ = write!(self.path, "L{} {}", round(x), round(y));
    }

    fn curve_to(&mut self, c1x: f64, c1y: f64, c2x: f64, c2y: f64, x: f64, y: f64) {
        let (c1x, c1y) = self.at(c1x, c1y);
        let (c2x, c2y) = self.at(c2x, c2y);
        let (x, y) = self.at(x, y);
        let _ = write!(
            self.path,
            "C{} {} {} {} {} {}",
            round(c1x),
            round(c1y),
            round(c2x),
            round(c2y),
            round(x),
            round(y),
        );
    }

    fn arc(&mut self, x: f64, y: f64, radius: f64, from: f64, to: f64) {
        // SVG has no centre-and-angles arc, so this walks it as line segments
        // fine enough that nothing shows: the wisp's arcs are a few pixels
        // across, and its full circles are its eyes and its glow.
        let steps = (((to - from).abs() * radius.max(1.0)) as usize).clamp(8, 180);
        for step in 0..=steps {
            let angle = from + (to - from) * (step as f64 / steps as f64);
            let (px, py) = self.at(x + radius * angle.cos(), y + radius * angle.sin());
            let cmd = if step == 0 && self.path.is_empty() {
                "M"
            } else {
                "L"
            };
            let _ = write!(self.path, "{cmd}{} {}", round(px), round(py));
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
        self.path.push('Z');
    }

    fn new_path(&mut self) {
        self.path.clear();
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
        if self.path.is_empty() {
            return;
        }
        let path = std::mem::take(&mut self.path);
        let ink = self.ink();
        let blend = self.blend();
        let _ = write!(self.body, "<path d=\"{path}\" fill=\"{ink}\"{blend}/>\n");
    }

    fn stroke(&mut self) {
        if self.path.is_empty() {
            return;
        }
        let path = std::mem::take(&mut self.path);
        let ink = self.ink();
        let blend = self.blend();
        let width = round(self.state.width * self.scale_of());
        let ends = if self.state.round {
            " stroke-linecap=\"round\" stroke-linejoin=\"round\""
        } else {
            ""
        };
        let _ = write!(
            self.body,
            "<path d=\"{path}\" fill=\"none\" stroke=\"{ink}\" stroke-width=\"{width}\"{ends}{blend}/>\n",
        );
    }

    fn clip(&mut self) {
        if self.path.is_empty() {
            return;
        }
        let path = std::mem::take(&mut self.path);
        let id = self.id();
        let _ = write!(
            self.defs,
            "<clipPath id=\"{id}\"><path d=\"{path}\"/></clipPath>\n"
        );
        let _ = write!(self.body, "<g clip-path=\"url(#{id})\">");
        self.state.groups += 1;
    }
}

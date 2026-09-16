//! The small 2D vocabulary the wisp is drawn in.
//!
//! The wisp is the same drawing wherever Glimmerwood runs, so it is written
//! once against this trait and each platform lends it a surface: cairo under
//! GTK, and whatever the others bring. Nothing here is more than every 2D
//! library has had for thirty years — paths, fills, strokes, a clip, a
//! transform stack and two kinds of gradient.
//!
//! It is stateful, like the libraries behind it: set a paint, build a path,
//! then fill or stroke it. A path's points are taken in the transform in
//! force when they are added, so the usual trick of scaling, drawing a
//! circle and restoring gives an ellipse.

use crate::oklab::Rgb;

/// One colour along a gradient, at `at` between 0 and 1.
#[derive(Clone, Copy, Debug)]
pub struct Stop {
    pub at: f64,
    pub colour: Rgb,
    pub alpha: f64,
}

pub const fn stop(at: f64, colour: Rgb, alpha: f64) -> Stop {
    Stop { at, colour, alpha }
}

/// What the next fill or stroke paints with.
#[derive(Clone, Copy, Debug)]
pub enum Paint<'a> {
    Solid {
        colour: Rgb,
        alpha: f64,
    },
    /// Between two circles, each `(x, y, radius)`, as every library draws them.
    Radial {
        inner: (f64, f64, f64),
        outer: (f64, f64, f64),
        stops: &'a [Stop],
    },
    Linear {
        from: (f64, f64),
        to: (f64, f64),
        stops: &'a [Stop],
    },
}

impl<'a> Paint<'a> {
    pub const fn solid(colour: Rgb, alpha: f64) -> Paint<'a> {
        Paint::Solid { colour, alpha }
    }

    /// A glow: circles sharing a centre, from nothing out to `radius`.
    pub const fn glow(x: f64, y: f64, radius: f64, stops: &'a [Stop]) -> Paint<'a> {
        Paint::Radial {
            inner: (x, y, 0.0),
            outer: (x, y, radius),
            stops,
        }
    }
}

/// A surface the wisp can be drawn on.
pub trait Canvas {
    fn save(&mut self);
    fn restore(&mut self);
    fn translate(&mut self, dx: f64, dy: f64);
    fn scale(&mut self, sx: f64, sy: f64);

    fn move_to(&mut self, x: f64, y: f64);
    fn line_to(&mut self, x: f64, y: f64);
    fn curve_to(&mut self, c1x: f64, c1y: f64, c2x: f64, c2y: f64, x: f64, y: f64);
    /// An arc clockwise from `from` to `to`, in radians.
    fn arc(&mut self, x: f64, y: f64, radius: f64, from: f64, to: f64);
    fn rectangle(&mut self, x: f64, y: f64, width: f64, height: f64);
    fn close_path(&mut self);
    /// Throw away any path in progress.
    fn new_path(&mut self);

    fn set_paint(&mut self, paint: Paint<'_>);
    fn set_line_width(&mut self, width: f64);
    /// Round caps and joins, rather than square ends and mitred corners.
    fn set_round_ends(&mut self, round: bool);
    /// Add light rather than paint over it, so glows build up in the dark.
    fn set_adding(&mut self, adding: bool);

    /// Fill the path, and clear it.
    fn fill(&mut self);
    /// Stroke the path, and clear it.
    fn stroke(&mut self);
    /// Narrow everything after this to the path, until the next `restore`.
    fn clip(&mut self);

    /// A filled circle: the one shape drawn often enough to be worth naming.
    fn disc(&mut self, x: f64, y: f64, radius: f64) {
        self.arc(x, y, radius, 0.0, std::f64::consts::TAU);
        self.fill();
    }
}

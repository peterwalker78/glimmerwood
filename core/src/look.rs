//! How the wisp looks along the dose, from `core/data/wisp.toml`.

use serde::Deserialize;

use crate::oklab::Rgb;

/// The chrome's paper, as `ui/chrome/chrome.css` sets it. The shells need it
/// where they paint something themselves rather than letting a chrome page
/// do it: the strip between the column and the page, a window's own
/// background behind a view that hasn't loaded yet.
pub const PAPER_LIGHT: (u8, u8, u8) = (0xec, 0xec, 0xe8);
pub const PAPER_DARK: (u8, u8, u8) = (0x1d, 0x1e, 0x1c);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    stop: Vec<RawStop>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStop {
    at: f64,
    core: String,
    glow: String,
    brightness: f64,
    radius: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub core: Rgb,
    pub glow: Rgb,
    pub brightness: f64,
    pub radius: f64,
}

#[derive(Clone, Debug)]
pub struct Stops(Vec<(f64, Look)>);

impl Stops {
    pub fn bundled() -> Stops {
        parse(include_str!("../data/wisp.toml")).expect("core/data/wisp.toml is valid")
    }

    /// The look at `dose`, blended between the stops either side.
    pub fn at(&self, dose: f64) -> Look {
        let dose = dose.clamp(0.0, 1.0);
        let upper = self.0.iter().position(|(at, _)| *at > dose);
        let (lower, upper) = match upper {
            None => {
                let last = self.0[self.0.len() - 1];
                (last, last)
            }
            Some(0) => (self.0[0], self.0[0]),
            Some(i) => (self.0[i - 1], self.0[i]),
        };
        if lower.0 == upper.0 {
            return lower.1;
        }
        let t = (dose - lower.0) / (upper.0 - lower.0);
        let (a, b) = (lower.1, upper.1);
        Look {
            core: a.core.mix(b.core, t),
            glow: a.glow.mix(b.glow, t),
            brightness: a.brightness + (b.brightness - a.brightness) * t,
            radius: a.radius + (b.radius - a.radius) * t,
        }
    }
}

fn parse(text: &str) -> Result<Stops, String> {
    let file: File = toml::from_str(text).map_err(|e| e.to_string())?;
    if file.stop.len() < 2 {
        return Err("the wisp needs at least two looks to blend".into());
    }
    if file.stop.first().map(|s| s.at) != Some(0.0) || file.stop.last().map(|s| s.at) != Some(1.0) {
        return Err("looks must start at dose 0 and end at dose 1".into());
    }
    if !file.stop.windows(2).all(|w| w[0].at < w[1].at) {
        return Err("looks must be in rising order of dose".into());
    }
    file.stop
        .into_iter()
        .map(|s| {
            let colour = |hex: &str| {
                Rgb::from_hex(hex).ok_or_else(|| format!("colours at {} must be #rrggbb", s.at))
            };
            if !(0.0..=1.0).contains(&s.brightness) || s.radius <= 0.0 {
                return Err(format!("brightness or radius out of range at {}", s.at));
            }
            Ok((
                s.at,
                Look {
                    core: colour(&s.core)?,
                    glow: colour(&s.glow)?,
                    brightness: s.brightness,
                    radius: s.radius,
                },
            ))
        })
        .collect::<Result<_, _>>()
        .map(Stops)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_are_exact_at_stops_and_blend_between() {
        let stops = Stops::bundled();
        assert_eq!(stops.at(0.0).radius, 7.0);
        assert_eq!(stops.at(1.0).radius, 3.8);
        let mid = stops.at(0.175).radius;
        assert!(mid < 7.0 && mid > 6.5, "{mid}");
        assert_eq!(stops.at(-1.0), stops.at(0.0));
    }

    #[test]
    fn broken_looks_are_rejected() {
        let good = include_str!("../data/wisp.toml");
        assert!(parse(&good.replace("#f2c063", "gold")).is_err());
        assert!(parse(&good.replace("at = 0.35", "at = 0.9")).is_err());
    }
}

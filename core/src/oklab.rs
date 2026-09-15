//! Colour blending in OKLab, so a mix of gold and blue-grey passes through a
//! clean slate instead of a muddy grey. Pure.

/// Non-linear sRGB, each channel 0-1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb(pub f64, pub f64, pub f64);

impl Rgb {
    /// `#rrggbb`.
    pub fn from_hex(hex: &str) -> Option<Rgb> {
        let digits = hex.strip_prefix('#').filter(|d| d.len() == 6)?;
        let n = u32::from_str_radix(digits, 16).ok()?;
        let channel = |shift: u32| f64::from((n >> shift) & 0xff) / 255.0;
        Some(Rgb(channel(16), channel(8), channel(0)))
    }

    pub fn mix(self, other: Rgb, t: f64) -> Rgb {
        let (a, b) = (to_lab(self), to_lab(other));
        from_lab([
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ])
    }
}

fn to_linear(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn to_gamma(c: f64) -> f64 {
    if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

fn to_lab(Rgb(r, g, b): Rgb) -> [f64; 3] {
    let (r, g, b) = (to_linear(r), to_linear(g), to_linear(b));
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    [
        0.210_454_255_3 * l + 0.793_617_785 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s,
    ]
}

fn from_lab([l, a, b]: [f64; 3]) -> Rgb {
    let l_ = (l + 0.396_337_777_4 * a + 0.215_803_757_3 * b).powi(3);
    let m_ = (l - 0.105_561_345_8 * a - 0.063_854_172_8 * b).powi(3);
    let s_ = (l - 0.089_484_177_5 * a - 1.291_485_548 * b).powi(3);
    let clamp = |v: f64| to_gamma(v).clamp(0.0, 1.0);
    Rgb(
        clamp(4.076_741_662_1 * l_ - 3.307_711_591_3 * m_ + 0.230_969_929_2 * s_),
        clamp(-1.268_438_004_6 * l_ + 2.609_757_401_1 * m_ - 0.341_319_396_5 * s_),
        clamp(-0.004_196_086_3 * l_ - 0.703_418_614_7 * m_ + 1.707_614_701 * s_),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Rgb, b: Rgb) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3 && (a.2 - b.2).abs() < 1e-3
    }

    #[test]
    fn hex_parses_and_rejects_nonsense() {
        assert_eq!(Rgb::from_hex("#ff8000"), Some(Rgb(1.0, 128.0 / 255.0, 0.0)));
        assert_eq!(Rgb::from_hex("ff8000"), None);
        assert_eq!(Rgb::from_hex("#ff80"), None);
    }

    #[test]
    fn mixing_round_trips_at_the_ends() {
        let gold = Rgb::from_hex("#f2c063").unwrap();
        let slate = Rgb::from_hex("#8e9fb2").unwrap();
        assert!(close(gold.mix(slate, 0.0), gold));
        assert!(close(gold.mix(slate, 1.0), slate));
    }

    #[test]
    fn the_midpoint_keeps_its_lightness() {
        // A straight sRGB average of gold and slate sags darker; OKLab holds up.
        let gold = Rgb::from_hex("#f2c063").unwrap();
        let slate = Rgb::from_hex("#8e9fb2").unwrap();
        let lightness = |c: Rgb| to_lab(c)[0];
        let mid = gold.mix(slate, 0.5);
        let expected = (lightness(gold) + lightness(slate)) / 2.0;
        assert!((lightness(mid) - expected).abs() < 1e-3);
    }
}

//! Where the sun is, so Home's garden is lit like the world outside the
//! window rather than by the clock.
//!
//! Pure arithmetic, apart from one look-up at start-up: the local time zone's
//! coordinates, which the tz database already keeps. Nothing is asked of the
//! network, and no location service is involved.

use std::f64::consts::PI;
use std::fs;

const RAD: f64 = PI / 180.0;

/// Somewhere on the Earth, in degrees: north and east positive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coords {
    pub lat: f64,
    pub lon: f64,
}

/// How the sun stands right now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Position {
    /// Degrees above the horizon; negative once it has set.
    pub altitude: f64,
    /// Before the sun's highest point, so the light is a morning light.
    pub climbing: bool,
}

/// The coordinates of the local time zone's principal place, or `None` if the
/// zone can't be read. A zone is a region, so this is a town-sized guess: for
/// sunset it is worth a minute or two, which is far closer than a fixed hour.
pub fn here() -> Option<Coords> {
    coords_of(&zone_name()?, &read_zone_table()?)
}

fn zone_name() -> Option<String> {
    if let Ok(tz) = std::env::var("TZ")
        && !tz.is_empty()
    {
        return Some(tz.trim_start_matches(':').to_owned());
    }
    let link = fs::read_link("/etc/localtime").ok()?;
    let path = link.to_str()?;
    let (_, zone) = path.split_once("zoneinfo/")?;
    Some(zone.to_owned())
}

fn read_zone_table() -> Option<String> {
    [
        "/usr/share/zoneinfo/zone1970.tab",
        "/usr/share/zoneinfo/zone.tab",
    ]
    .into_iter()
    .find_map(|path| fs::read_to_string(path).ok())
}

/// The coordinates listed for `zone` in a tz database zone table.
fn coords_of(zone: &str, table: &str) -> Option<Coords> {
    table
        .lines()
        .filter(|line| !line.starts_with('#'))
        .find_map(|line| {
            let mut fields = line.split('\t');
            let position = fields.nth(1)?;
            (fields.next()? == zone).then_some(position)
        })
        .and_then(parse_iso6709)
}

/// `+513030-0000731` (degrees, minutes and optional seconds) as degrees.
fn parse_iso6709(text: &str) -> Option<Coords> {
    let split = text[1..].find(['+', '-'])? + 1;
    let (lat, lon) = text.split_at(split);
    Some(Coords {
        lat: degrees(lat, 2)?,
        lon: degrees(lon, 3)?,
    })
}

/// A signed `DDMM[SS]` field, `width` digits of degrees.
fn degrees(text: &str, width: usize) -> Option<f64> {
    let sign = match text.as_bytes().first()? {
        b'+' => 1.0,
        b'-' => -1.0,
        _ => return None,
    };
    let digits = &text[1..];
    if !digits.bytes().all(|b| b.is_ascii_digit()) || digits.len() < width + 2 {
        return None;
    }
    let part = |from: usize, len: usize| digits.get(from..from + len)?.parse::<f64>().ok();
    let seconds = if digits.len() >= width + 4 {
        part(width + 2, 2)?
    } else {
        0.0
    };
    Some(sign * (part(0, width)? + part(width, 2)? / 60.0 + seconds / 3600.0))
}

/// Where the sun stands at `unix_ms`, by the usual astronomical almanac
/// formulae (good to well under a degree, which is a minute or so of time).
pub fn position(at: Coords, unix_ms: i64) -> Position {
    let days = unix_ms as f64 / 86_400_000.0 - 10_957.5; // Since J2000.0.
    let centuries = days / 36_525.0;

    let mean_longitude = (280.46646 + centuries * (36000.76983 + centuries * 0.0003032)) % 360.0;
    let mean_anomaly = 357.52911 + centuries * (35999.05029 - 0.0001537 * centuries);
    let centre = (mean_anomaly * RAD).sin()
        * (1.914602 - centuries * (0.004817 + 0.000014 * centuries))
        + (2.0 * mean_anomaly * RAD).sin() * (0.019993 - 0.000101 * centuries)
        + (3.0 * mean_anomaly * RAD).sin() * 0.000289;
    // The apparent longitude, nodding with the moon's pull on the Earth.
    let nod = (125.04 - 1934.136 * centuries) * RAD;
    let longitude = (mean_longitude + centre - 0.00569 - 0.00478 * nod.sin()) * RAD;
    let mean_obliquity = 23.0
        + (26.0
            + (21.448 - centuries * (46.815 + centuries * (0.00059 - centuries * 0.001813)))
                / 60.0)
            / 60.0;
    let obliquity = (mean_obliquity + 0.00256 * nod.cos()) * RAD;
    let declination = (obliquity.sin() * longitude.sin()).asin();

    // How far the sun runs ahead of or behind the clock.
    let y = (obliquity / 2.0).tan().powi(2);
    let eccentricity = 0.016708634 - centuries * (0.000042037 + 0.0000001267 * centuries);
    let l = mean_longitude * RAD;
    let m = mean_anomaly * RAD;
    let equation_of_time = 4.0
        * (y * (2.0 * l).sin() - 2.0 * eccentricity * m.sin()
            + 4.0 * eccentricity * y * m.sin() * (2.0 * l).cos()
            - 0.5 * y * y * (4.0 * l).sin()
            - 1.25 * eccentricity * eccentricity * (2.0 * m).sin())
        / RAD;

    let minutes_utc = (unix_ms as f64 / 60_000.0).rem_euclid(1440.0);
    let solar_minutes = minutes_utc + equation_of_time + 4.0 * at.lon;
    let hour_angle = (solar_minutes / 4.0 - 180.0) * RAD;
    let lat = at.lat * RAD;
    let altitude = (lat.sin() * declination.sin()
        + lat.cos() * declination.cos() * hour_angle.cos())
    .clamp(-1.0, 1.0)
    .asin()
        / RAD;
    Position {
        altitude,
        climbing: hour_angle.sin() < 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// London, from the tz database's own entry for Europe/London.
    const LONDON: Coords = Coords {
        lat: 51.508_33,
        lon: -0.125_28,
    };

    /// Midday UTC on a date, as Unix milliseconds.
    fn noon_utc(year: i32, month: u32, day: u32) -> i64 {
        // Days from 1970 to the start of `year`, then to the month.
        let leap = |y: i32| (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        let mut days: i64 = (1970..year).map(|y| if leap(y) { 366 } else { 365 }).sum();
        let lengths = [
            31,
            if leap(year) { 29 } else { 28 },
            31,
            30,
            31,
            30,
            31,
            31,
            30,
            31,
            30,
            31,
        ];
        days += lengths[..(month - 1) as usize].iter().sum::<i64>() + i64::from(day) - 1;
        (days * 86_400 + 12 * 3600) * 1000
    }

    /// The sun's highest point of the day and when it comes, by the minute.
    fn highest(at: Coords, day_ms: i64) -> (f64, i64) {
        (0..1440)
            .map(|m| (position(at, day_ms + m * 60_000).altitude, m))
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .expect("a day has minutes")
    }

    #[test]
    fn midday_height_follows_the_seasons() {
        // 90° - latitude, give or take the Earth's 23.44° tilt.
        let midnight = |y, m, d| noon_utc(y, m, d) - 12 * 3600 * 1000;
        let june = highest(LONDON, midnight(2026, 6, 21)).0;
        let december = highest(LONDON, midnight(2026, 12, 21)).0;
        let equinox = highest(LONDON, midnight(2026, 9, 23)).0;
        assert!((june - 61.9).abs() < 0.5, "June {june}");
        assert!((december - 15.0).abs() < 0.5, "December {december}");
        assert!((equinox - 38.5).abs() < 0.7, "equinox {equinox}");
    }

    #[test]
    fn noon_is_near_the_middle_of_the_day_and_the_sun_climbs_to_it() {
        let midnight = noon_utc(2026, 9, 15) - 12 * 3600 * 1000;
        let (_, minute) = highest(LONDON, midnight);
        // Solar noon at Greenwich in mid-September: just before 12:00 UTC.
        assert!((11 * 60..12 * 60).contains(&minute), "solar noon {minute}");
        assert!(position(LONDON, midnight + (minute - 60) * 60_000).climbing);
        assert!(!position(LONDON, midnight + (minute + 60) * 60_000).climbing);
    }

    #[test]
    fn day_and_night_are_even_at_the_equinox() {
        // Sunrise and sunset are reckoned from the sun's upper limb through
        // a refracting atmosphere, which is why an equinox day runs a few
        // minutes past twelve hours.
        let midnight = noon_utc(2026, 9, 23) - 12 * 3600 * 1000;
        let up: Vec<i64> = (0..1440)
            .filter(|m| position(LONDON, midnight + m * 60_000).altitude > -0.833)
            .collect();
        let (sunrise, sunset) = (up[0], up[up.len() - 1]);
        let length = sunset - sunrise;
        assert!(
            (720..=735).contains(&length),
            "{length} minutes of daylight"
        );
        // Around 6 in the morning and 6 in the evening, UTC, as an equinox
        // near the Greenwich meridian should be.
        assert!(
            (5 * 60..6 * 60 + 30).contains(&sunrise),
            "sunrise {sunrise}"
        );
        assert!(
            (17 * 60 + 30..18 * 60 + 30).contains(&sunset),
            "sunset {sunset}"
        );
    }

    #[test]
    fn the_light_goes_soon_after_the_sun_does() {
        // Mid-September in London: sunset is around 19:15 local (18:15 UTC),
        // and civil twilight is over half an hour later.
        let midnight = noon_utc(2026, 9, 15) - 12 * 3600 * 1000;
        let sunset = (0..1440)
            .find(|m| position(LONDON, midnight + m * 60_000).altitude < -0.833 && *m > 12 * 60)
            .expect("the sun sets");
        assert!((18 * 60..18 * 60 + 40).contains(&sunset), "sunset {sunset}");
        let dusk = (sunset..1440)
            .find(|m| position(LONDON, midnight + m * 60_000).altitude < -6.0)
            .expect("the light goes");
        assert!(
            (25..45).contains(&(dusk - sunset)),
            "dusk {}",
            dusk - sunset
        );
    }

    #[test]
    fn the_southern_hemisphere_has_its_own_seasons() {
        let sydney = Coords {
            lat: -33.86667,
            lon: 151.2,
        };
        let midnight = |m, d| noon_utc(2026, m, d) - 12 * 3600 * 1000;
        let june = highest(sydney, midnight(6, 21)).0;
        let december = highest(sydney, midnight(12, 21)).0;
        assert!(december > june + 40.0, "{june} then {december}");
    }

    #[test]
    fn zone_coordinates_are_read_from_the_table() {
        let table = "\
#code\tcoordinates\tTZ\tcomments
GB,GG,IM,JE\t+513030-0000731\tEurope/London
AU\t-3352+15113\tAustralia/Sydney\tNew South Wales
";
        let london = coords_of("Europe/London", table).expect("London");
        assert!((london.lat - 51.5083).abs() < 0.001, "{london:?}");
        assert!((london.lon + 0.1253).abs() < 0.001, "{london:?}");
        // Minutes only, and south of the equator.
        let sydney = coords_of("Australia/Sydney", table).expect("Sydney");
        assert!((sydney.lat + 33.8667).abs() < 0.001, "{sydney:?}");
        assert!((sydney.lon - 151.2167).abs() < 0.001, "{sydney:?}");
        assert_eq!(coords_of("Mars/Olympus", table), None);
    }
}

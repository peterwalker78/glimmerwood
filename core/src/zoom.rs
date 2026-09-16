//! How large a site's pages are drawn, and remembering it per site.
//!
//! A reader who needs a site bigger needs it bigger every time, so the level
//! is kept against the host rather than the tab. There is no widget for it:
//! the keyboard sets it, and the only sign it has changed is the page.

use std::collections::BTreeMap;
use std::path::Path;

use crate::nav;

/// The levels the keyboard steps through. Anything already in the file is kept
/// as it is, so a level typed into the file by hand survives a step.
pub const STEPS: &[f64] = &[
    0.5, 0.67, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0,
];

/// The level everything starts at.
pub const PLAIN: f64 = 1.0;

#[derive(Debug, Default)]
pub struct Zooms {
    /// Only sites that differ from plain are kept, so the file stays a list of
    /// deliberate choices rather than every site ever opened.
    per_host: BTreeMap<String, f64>,
}

impl Zooms {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(_) => Self::new(),
        }
    }

    pub fn parse(text: &str) -> Self {
        let mut per_host = BTreeMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((host, level)) = line.split_once('=') else {
                continue;
            };
            let host = host.trim().trim_matches('"');
            let Ok(level) = level.trim().parse::<f64>() else {
                continue;
            };
            if !host.is_empty() && sane(level) {
                per_host.insert(host.to_owned(), level);
            }
        }
        Self { per_host }
    }

    pub fn to_text(&self) -> String {
        let mut out = String::from("# How large each site is drawn. Delete a line to forget it.\n");
        for (host, level) in &self.per_host {
            out.push_str(&format!("\"{host}\" = {level}\n"));
        }
        out
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        std::fs::write(path, self.to_text()).map_err(|err| err.to_string())
    }

    /// How large `uri` should be drawn.
    pub fn of(&self, uri: &str) -> f64 {
        let host = nav::host_of(uri);
        if host.is_empty() {
            return PLAIN;
        }
        self.per_host.get(&host).copied().unwrap_or(PLAIN)
    }

    /// One step larger or smaller, and the level to draw at. A page with no
    /// host of its own — a local page, a blank tab — is left alone.
    pub fn step(&mut self, uri: &str, by: isize) -> f64 {
        let host = nav::host_of(uri);
        if host.is_empty() {
            return PLAIN;
        }
        let level = self.per_host.get(&host).copied().unwrap_or(PLAIN);
        let next = stepped(level, by);
        self.set(&host, next);
        next
    }

    /// Back to plain, and forgotten.
    pub fn reset(&mut self, uri: &str) -> f64 {
        let host = nav::host_of(uri);
        self.per_host.remove(&host);
        PLAIN
    }

    fn set(&mut self, host: &str, level: f64) {
        if (level - PLAIN).abs() < f64::EPSILON {
            self.per_host.remove(host);
        } else {
            self.per_host.insert(host.to_owned(), level);
        }
    }
}

/// The next step along from `level`, stopping at both ends.
fn stepped(level: f64, by: isize) -> f64 {
    if by == 0 {
        return level;
    }
    let mut steps: Vec<f64> = STEPS.to_vec();
    // A level that came from the file may sit between two steps; treat it as
    // one of them for the purpose of moving along.
    if !steps.iter().any(|step| (step - level).abs() < 0.001) {
        steps.push(level);
        steps.sort_by(|a, b| a.partial_cmp(b).expect("no NaN among the steps"));
    }
    let at = steps
        .iter()
        .position(|step| (step - level).abs() < 0.001)
        .unwrap_or(0);
    let next = (at as isize + by).clamp(0, steps.len() as isize - 1) as usize;
    steps[next]
}

fn sane(level: f64) -> bool {
    level.is_finite() && (0.25..=5.0).contains(&level)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_site_starts_plain() {
        let zooms = Zooms::new();
        assert_eq!(zooms.of("https://example.org/"), PLAIN);
    }

    #[test]
    fn stepping_up_and_back_down_returns_to_plain() {
        let mut zooms = Zooms::new();
        let up = zooms.step("https://example.org/a", 1);
        assert!(up > PLAIN);
        assert_eq!(zooms.step("https://example.org/b", -1), PLAIN);
        assert_eq!(zooms.to_text().lines().count(), 1, "nothing left to keep");
    }

    #[test]
    fn a_level_is_kept_against_the_site_not_the_page() {
        let mut zooms = Zooms::new();
        zooms.step("https://www.example.org/one", 1);
        assert_eq!(zooms.of("https://example.org/two"), 1.1);
    }

    #[test]
    fn stepping_stops_at_both_ends() {
        let mut zooms = Zooms::new();
        for _ in 0..40 {
            zooms.step("https://example.org/", 1);
        }
        assert_eq!(zooms.of("https://example.org/"), 3.0);
        for _ in 0..40 {
            zooms.step("https://example.org/", -1);
        }
        assert_eq!(zooms.of("https://example.org/"), 0.5);
    }

    #[test]
    fn local_pages_are_left_alone() {
        let mut zooms = Zooms::new();
        assert_eq!(zooms.step("glimmerwood://home/", 1), PLAIN);
        assert_eq!(zooms.of("glimmerwood://home/"), PLAIN);
    }

    #[test]
    fn resetting_forgets_the_site() {
        let mut zooms = Zooms::new();
        zooms.step("https://example.org/", 2);
        assert_eq!(zooms.reset("https://example.org/"), PLAIN);
        assert!(!zooms.to_text().contains("example.org"));
    }

    #[test]
    fn a_level_typed_in_by_hand_survives_a_step() {
        let mut zooms = Zooms::parse("\"example.org\" = 1.05\n");
        assert_eq!(zooms.of("https://example.org/"), 1.05);
        assert_eq!(zooms.step("https://example.org/", 1), 1.1);
    }

    #[test]
    fn nonsense_in_the_file_is_ignored() {
        let zooms = Zooms::parse("nonsense\n\"x.org\" = huge\n\"y.org\" = 99\n\"z.org\" = 1.5\n");
        assert_eq!(zooms.of("https://x.org/"), PLAIN);
        assert_eq!(zooms.of("https://y.org/"), PLAIN);
        assert_eq!(zooms.of("https://z.org/"), 1.5);
    }

    #[test]
    fn what_is_written_reads_back_the_same() {
        let mut zooms = Zooms::new();
        zooms.step("https://a.example/", 1);
        zooms.step("https://b.example/", -2);
        let again = Zooms::parse(&zooms.to_text());
        assert_eq!(
            again.of("https://a.example/"),
            zooms.of("https://a.example/")
        );
        assert_eq!(
            again.of("https://b.example/"),
            zooms.of("https://b.example/")
        );
    }
}

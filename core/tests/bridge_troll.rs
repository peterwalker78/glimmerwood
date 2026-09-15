//! The bridge troll detector.
//!
//! Wisp's one rule is to be a spirit guide, not a bridge troll: nothing may
//! block navigation, raise modal dialogs, or use red/alarm styling. This test
//! catches the mechanical versions of those mistakes in the chrome's source.
//! It is a floor, not a review: tone and feel still need a person.

use std::fs;
use std::path::{Path, PathBuf};

/// Browser APIs that stop the user until they answer.
const MODAL: &[&str] = &["alert(", "confirm(", "prompt(", "<dialog", "showModal"];

const RED_NAMES: &[&str] = &[
    "red",
    "darkred",
    "crimson",
    "firebrick",
    "indianred",
    "orangered",
    "tomato",
];

#[test]
fn chrome_has_no_modal_dialogs() {
    let mut found = Vec::new();
    for file in ui_sources(&["ts", "html"]) {
        for (n, line) in read(&file).lines().enumerate() {
            for token in MODAL {
                if line.contains(token) {
                    found.push(format!("{}:{}: {token}", file.display(), n + 1));
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "modal UI is not allowed; show it ambiently instead:\n{}",
        found.join("\n")
    );
}

#[test]
fn chrome_uses_no_red() {
    let mut found = Vec::new();
    let wisp = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/wisp.toml");
    for file in ui_sources(&["css", "html"]).into_iter().chain([wisp]) {
        let toml = file.extension().is_some_and(|e| e == "toml");
        for (n, line) in read(&file).lines().enumerate() {
            if toml && line.trim_start().starts_with('#') {
                continue;
            }
            for colour in colours(line) {
                if is_red(colour) {
                    found.push(format!("{}:{}: {line}", file.display(), n + 1));
                }
            }
            if RED_NAMES.iter().any(|name| has_word(line, name)) {
                found.push(format!("{}:{}: {line}", file.display(), n + 1));
            }
        }
    }
    assert!(
        found.is_empty(),
        "red and alarm colours are not allowed in the chrome:\n{}",
        found.join("\n")
    );
}

fn ui_sources(extensions: &[&str]) -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ui");
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("ui/ is readable") {
            let path = entry.expect("ui/ entry").path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if path.is_dir() {
                if name != "dist" && name != "vendor" {
                    stack.push(path);
                }
            } else if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| extensions.contains(&e))
            {
                out.push(path);
            }
        }
    }
    assert!(!out.is_empty(), "found no UI sources to check");
    out
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn has_word(line: &str, word: &str) -> bool {
    line.match_indices(word).any(|(i, _)| {
        let before = line[..i].chars().next_back();
        let after = line[i + word.len()..].chars().next();
        let boundary = |c: Option<char>| !c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '-');
        boundary(before) && boundary(after)
    })
}

/// Colours written as `#rgb`, `#rrggbb` (with or without alpha) or `rgb(…)`.
fn colours(line: &str) -> Vec<(f64, f64, f64)> {
    let mut out = Vec::new();
    for (i, _) in line.match_indices('#') {
        let hex: String = line[i + 1..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        let channel = |s: &str| u8::from_str_radix(s, 16).ok().map(|v| f64::from(v) / 255.0);
        let rgb = match hex.len() {
            3 | 4 => {
                let d: Vec<String> = hex.chars().map(|c| format!("{c}{c}")).collect();
                (channel(&d[0]), channel(&d[1]), channel(&d[2]))
            }
            6 | 8 => (
                channel(&hex[0..2]),
                channel(&hex[2..4]),
                channel(&hex[4..6]),
            ),
            _ => continue,
        };
        if let (Some(r), Some(g), Some(b)) = rgb {
            out.push((r, g, b));
        }
    }
    for (i, _) in line.match_indices("rgb") {
        let Some(open) = line[i..].find('(') else {
            continue;
        };
        let Some(close) = line[i + open..].find(')') else {
            continue;
        };
        let inner = &line[i + open + 1..i + open + close];
        let nums: Vec<f64> = inner
            .split(|c: char| c == ',' || c == '/' || c.is_whitespace())
            .filter_map(|s| s.trim().parse::<f64>().ok())
            .collect();
        if let [r, g, b, ..] = nums[..] {
            out.push((r / 255.0, g / 255.0, b / 255.0));
        }
    }
    out
}

/// Saturated enough to read as a colour, in the red hue band, and neither
/// nearly black nor nearly white.
fn is_red((r, g, b): (f64, f64, f64)) -> bool {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let lightness = (max + min) / 2.0;
    let delta = max - min;
    if delta < 1e-6 || !(0.12..=0.92).contains(&lightness) {
        return false;
    }
    let saturation = delta / (1.0 - (2.0 * lightness - 1.0).abs());
    let hue = if max == r {
        60.0 * ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    saturation >= 0.3 && !(20.0..335.0).contains(&hue)
}

#[test]
fn detector_knows_red_from_amber_and_grey() {
    let first = |s: &str| colours(s)[0];
    assert!(is_red(first("color: #c0392b;")));
    assert!(is_red(first("background: rgb(220 40 40 / 0.5)")));
    assert!(is_red(first("fill: #e0457b")));
    assert!(
        !is_red(first("color: #d99a2b;")),
        "amber is the ember, not an alarm"
    );
    assert!(!is_red(first("color: #2a2b28;")));
    assert!(!is_red(first("color: #ecece8;")));
    assert!(has_word("color: red;", "red"));
    assert!(!has_word("--hairline: rgb(0 0 0)", "red"));
    assert!(!has_word("reduced-motion", "red"));
}

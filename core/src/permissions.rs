//! Which sites may use the camera, the microphone, the screen or the
//! person's location, remembered per site.
//!
//! Only a yes is kept. "Not now" is forgotten as soon as it is said, so the
//! site can ask again another time. Notifications have no place here at all:
//! they are refused without asking, and no line in the file can allow them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::nav;
use crate::protocol::Want;

/// How each thing is spelled in the file.
const NAMES: &[(Want, &str)] = &[
    (Want::Camera, "camera"),
    (Want::Microphone, "microphone"),
    (Want::Screen, "screen"),
    (Want::Location, "location"),
];

#[derive(Debug, Default, PartialEq)]
pub struct Allowed {
    per_host: BTreeMap<String, BTreeSet<Want>>,
}

impl Allowed {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(_) => Self::new(),
        }
    }

    /// Anything the file says that isn't a site and a list of known things
    /// is passed over, so a hand edit gone wrong costs only that line.
    pub fn parse(text: &str) -> Self {
        let mut per_host = BTreeMap::new();
        let Ok(table) = text.parse::<toml::Table>() else {
            return Self::new();
        };
        for (host, value) in table {
            let wants: BTreeSet<Want> = value
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|name| name.as_str().and_then(want_named))
                .collect();
            if !host.is_empty() && !wants.is_empty() {
                per_host.insert(host, wants);
            }
        }
        Self { per_host }
    }

    pub fn to_text(&self) -> String {
        let mut out = String::from(
            "# What each site may use without asking. Delete a line to be asked again.\n",
        );
        for (host, wants) in &self.per_host {
            let names: Vec<String> = wants
                .iter()
                .map(|w| format!("\"{}\"", name_of(*w)))
                .collect();
            out.push_str(&format!("\"{host}\" = [{}]\n", names.join(", ")));
        }
        out
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|err| err.to_string())?;
        }
        std::fs::write(path, self.to_text()).map_err(|err| err.to_string())
    }

    /// Whether the page at `uri` may have everything in `wants` without
    /// asking. A page with no site of its own is never let through.
    pub fn allows(&self, uri: &str, wants: &[Want]) -> bool {
        let host = nav::host_of(uri);
        !wants.is_empty()
            && self
                .per_host
                .get(&host)
                .is_some_and(|kept| wants.iter().all(|want| kept.contains(want)))
    }

    /// Whether the page's site may use any one of `wants`.
    pub fn allows_any(&self, uri: &str, wants: &[Want]) -> bool {
        wants.iter().any(|want| self.allows(uri, &[*want]))
    }

    /// Remember a yes. Nothing is kept for a page with no site.
    pub fn allow(&mut self, uri: &str, wants: &[Want]) {
        let host = nav::host_of(uri);
        if host.is_empty() || wants.is_empty() {
            return;
        }
        self.per_host
            .entry(host)
            .or_default()
            .extend(wants.iter().copied());
    }
}

fn want_named(name: &str) -> Option<Want> {
    NAMES
        .iter()
        .find(|(_, n)| *n == name)
        .map(|(want, _)| *want)
}

fn name_of(want: Want) -> &'static str {
    NAMES
        .iter()
        .find(|(w, _)| *w == want)
        .map(|(_, name)| *name)
        .expect("every want has a name")
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEET: &str = "https://meet.example.org/abc-defg";

    #[test]
    fn a_site_starts_out_asking() {
        assert!(!Allowed::new().allows(MEET, &[Want::Camera]));
    }

    #[test]
    fn a_yes_is_remembered_for_that_site_only() {
        let mut allowed = Allowed::new();
        allowed.allow(MEET, &[Want::Camera, Want::Microphone]);
        assert!(allowed.allows("https://meet.example.org/other-room", &[Want::Camera]));
        assert!(allowed.allows(MEET, &[Want::Camera, Want::Microphone]));
        assert!(!allowed.allows("https://example.com/", &[Want::Camera]));
    }

    #[test]
    fn a_yes_to_one_thing_is_not_a_yes_to_another() {
        let mut allowed = Allowed::new();
        allowed.allow(MEET, &[Want::Microphone]);
        assert!(!allowed.allows(MEET, &[Want::Camera, Want::Microphone]));
        assert!(!allowed.allows(MEET, &[Want::Location]));
        assert!(allowed.allows_any(MEET, &[Want::Camera, Want::Microphone]));
    }

    #[test]
    fn pages_without_a_site_are_never_let_through() {
        let mut allowed = Allowed::new();
        allowed.allow("glimmerwood://home/", &[Want::Camera]);
        allowed.allow("file:///home/someone/page.html", &[Want::Location]);
        assert_eq!(allowed, Allowed::new());
        assert!(!allowed.allows("glimmerwood://home/", &[Want::Camera]));
    }

    #[test]
    fn asking_for_nothing_is_not_allowed() {
        let mut allowed = Allowed::new();
        allowed.allow(MEET, &[Want::Camera]);
        assert!(!allowed.allows(MEET, &[]));
    }

    #[test]
    fn remembered_choices_survive_a_restart() {
        let mut allowed = Allowed::new();
        allowed.allow(MEET, &[Want::Screen, Want::Camera]);
        allowed.allow("https://www.maps.example/", &[Want::Location]);
        let back = Allowed::parse(&allowed.to_text());
        assert_eq!(back, allowed);
        assert!(back.allows("https://maps.example/", &[Want::Location]));
    }

    #[test]
    fn notifications_can_never_be_allowed_from_the_file() {
        let allowed = Allowed::parse(
            "\"news.example\" = [\"notifications\"]\n\"meet.example.org\" = [\"camera\", \"push\"]\n",
        );
        assert!(!allowed.to_text().contains("news.example"));
        assert!(!allowed.to_text().contains("push"));
        assert!(allowed.allows(MEET, &[Want::Camera]));
    }

    #[test]
    fn a_broken_file_starts_over_rather_than_failing() {
        assert_eq!(Allowed::parse("this is = = not toml"), Allowed::new());
        assert_eq!(Allowed::parse("\"a.example\" = 3"), Allowed::new());
    }
}

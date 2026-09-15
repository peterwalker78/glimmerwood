//! What the user chose on the Settings page, kept in `settings.toml` in the
//! config directory beside their reputation file.
//!
//! Parsing and writing are pure; reading and saving the file are the two
//! functions at the bottom.

use std::path::Path;

use serde::Deserialize;

use crate::dose::Rates;

/// The half hours the night may start in, and end in.
pub const NIGHT_STARTS: (&str, &str) = ("20:00", "01:00");
pub const NIGHT_ENDS: (&str, &str) = ("04:00", "08:00");

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// The wisp asks how sites it hasn't met leave the user.
    pub ask_about_new_places: bool,
    pub night_starts: String,
    pub night_ends: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct File {
    ask_about_new_places: Option<bool>,
    night: Option<NightFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NightFile {
    starts: Option<String>,
    ends: Option<String>,
}

impl Settings {
    /// Nothing chosen yet: the bundled rates' night.
    pub fn defaults(rates: &Rates) -> Settings {
        Settings {
            ask_about_new_places: true,
            night_starts: rates.night.starts_at.clone(),
            night_ends: rates.night.ends_at.clone(),
        }
    }

    /// Anything the file leaves out keeps its default. A mistake is
    /// reported, not guessed at.
    pub fn parse(text: &str, rates: &Rates) -> Result<Settings, String> {
        let file: File = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut settings = Settings::defaults(rates);
        if let Some(ask) = file.ask_about_new_places {
            settings.ask_about_new_places = ask;
        }
        if let Some(night) = file.night {
            settings.night_starts = night.starts.unwrap_or(settings.night_starts);
            settings.night_ends = night.ends.unwrap_or(settings.night_ends);
        }
        settings.set_night(&settings.night_starts.clone(), &settings.night_ends.clone())?;
        Ok(settings)
    }

    pub fn to_toml(&self) -> String {
        format!(
            "# Glimmerwood's settings, changed from its Settings page (Ctrl+comma).\n\
             # Editing by hand works too; Glimmerwood reads them when it starts.\n\
             \n\
             ask-about-new-places = {}\n\
             \n\
             # When the wisp winds down, in half hours: starting from {} to {}\n\
             # and ending from {} to {}.\n\
             [night]\n\
             starts = \"{}\"\n\
             ends = \"{}\"\n",
            self.ask_about_new_places,
            NIGHT_STARTS.0,
            NIGHT_STARTS.1,
            NIGHT_ENDS.0,
            NIGHT_ENDS.1,
            self.night_starts,
            self.night_ends,
        )
    }

    /// Move the night, if both times are among the choices.
    pub fn set_night(&mut self, starts: &str, ends: &str) -> Result<(), String> {
        if !night_choices(NIGHT_STARTS).iter().any(|t| t == starts) {
            return Err(format!(
                "the night starts between {} and {}, on the hour or half past",
                NIGHT_STARTS.0, NIGHT_STARTS.1
            ));
        }
        if !night_choices(NIGHT_ENDS).iter().any(|t| t == ends) {
            return Err(format!(
                "the night ends between {} and {}, on the hour or half past",
                NIGHT_ENDS.0, NIGHT_ENDS.1
            ));
        }
        self.night_starts = starts.to_owned();
        self.night_ends = ends.to_owned();
        Ok(())
    }
}

/// Every half hour from `from` to `to`, going past midnight if need be.
pub fn night_choices((from, to): (&str, &str)) -> Vec<String> {
    let minutes = |t: &str| -> u32 {
        let (h, m) = t.split_once(':').expect("HH:MM");
        h.parse::<u32>().expect("hour") * 60 + m.parse::<u32>().expect("minute")
    };
    let (start, end) = (minutes(from), minutes(to));
    let span = (end + 24 * 60 - start) % (24 * 60);
    (0..=span / 30)
        .map(|i| {
            let t = (start + i * 30) % (24 * 60);
            format!("{:02}:{:02}", t / 60, t % 60)
        })
        .collect()
}

/// The user's settings, or the defaults if there's no file or it has a
/// mistake (which is reported).
pub fn load(rates: &Rates, path: &Path) -> Settings {
    match std::fs::read_to_string(path) {
        Ok(text) => Settings::parse(&text, rates).unwrap_or_else(|err| {
            eprintln!(
                "glimmerwood: ignoring {} until it's fixed: {err}",
                path.display()
            );
            Settings::defaults(rates)
        }),
        Err(_) => Settings::defaults(rates),
    }
}

pub fn save(settings: &Settings, path: &Path) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|err| err.to_string())?;
    }
    std::fs::write(path, settings.to_toml()).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_settings_keep_their_defaults() {
        let rates = Rates::bundled();
        let settings = Settings::parse("", &rates).expect("empty is fine");
        assert_eq!(settings, Settings::defaults(&rates));
        assert!(settings.ask_about_new_places);
        assert_eq!(
            (settings.night_starts.as_str(), settings.night_ends.as_str()),
            ("23:00", "05:00")
        );
        let settings =
            Settings::parse("[night]\nstarts = \"22:30\"", &rates).expect("partial is fine");
        assert_eq!(settings.night_starts, "22:30");
        assert_eq!(settings.night_ends, "05:00");
    }

    #[test]
    fn settings_survive_being_written_and_read_back() {
        let rates = Rates::bundled();
        let mut settings = Settings::defaults(&rates);
        settings.ask_about_new_places = false;
        settings.set_night("00:30", "07:30").expect("allowed");
        assert_eq!(Settings::parse(&settings.to_toml(), &rates), Ok(settings));
    }

    #[test]
    fn the_night_moves_only_within_its_choices() {
        let mut settings = Settings::defaults(&Rates::bundled());
        assert!(settings.set_night("19:30", "05:00").is_err());
        assert!(settings.set_night("23:15", "05:00").is_err());
        assert!(settings.set_night("23:00", "09:00").is_err());
        assert!(Settings::parse("[night]\nstarts = \"12:00\"", &Rates::bundled()).is_err());
        assert!(Settings::parse("shiny = true", &Rates::bundled()).is_err());
        assert_eq!(
            night_choices(NIGHT_STARTS),
            [
                "20:00", "20:30", "21:00", "21:30", "22:00", "22:30", "23:00", "23:30", "00:00",
                "00:30", "01:00"
            ]
        );
        assert_eq!(night_choices(NIGHT_ENDS).len(), 9);
    }
}

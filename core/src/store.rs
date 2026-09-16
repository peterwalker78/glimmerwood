//! Dose history on disk, and what Home remembers.
//!
//! One row per minute at most, holding what the engine knew: the dose, the
//! day's draining load, and the reputation entry that matched (never the
//! address visited). Beside it, what Home has explained and the days the
//! garden grew. The type signatures are the privacy boundary: nothing here
//! accepts a URL. Bookmarks, which are addresses, live in their own file.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use crate::dose::{Mode, Moment, Place};
use crate::home::{Minute, Plant};

/// History kept, in days.
pub const RETENTION_DAYS: i64 = 90;

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS sessions (
        minute       INTEGER PRIMARY KEY,
        utc_offset_s INTEGER NOT NULL,
        dose         REAL    NOT NULL,
        day_load     REAL    NOT NULL,
        mode         TEXT    NOT NULL,
        entry        TEXT,
        weight       REAL
    );
    CREATE TABLE IF NOT EXISTS home (
        key   TEXT    PRIMARY KEY,
        value INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS garden (
        day       INTEGER PRIMARY KEY,
        plant     TEXT,
        fireflies INTEGER NOT NULL
    );
";

/// A finished day in the garden: what it grew, if anything.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GardenDay {
    pub day: i64,
    pub plant: Option<Plant>,
    pub fireflies: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub at: Moment,
    pub dose: f64,
    pub day_load: f64,
    pub mode: Mode,
    /// Where attention was, as the reputation lists see it.
    pub place: Option<Place>,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Store> {
        Store::init(Connection::open(path)?)
    }

    #[cfg(test)]
    pub fn in_memory() -> Store {
        Store::init(Connection::open_in_memory().expect("in-memory sqlite")).expect("schema")
    }

    fn init(conn: Connection) -> rusqlite::Result<Store> {
        conn.execute_batch(SCHEMA)?;
        Ok(Store { conn })
    }

    /// Record the minute `sample.at` falls in, replacing an earlier sample
    /// for the same minute.
    pub fn record(&self, sample: &Sample) -> rusqlite::Result<()> {
        let (entry, weight) = match &sample.place {
            Some(Place::Listed { entry, weight, .. }) => (Some(entry.as_str()), Some(*weight)),
            _ => (None, None),
        };
        self.conn.execute(
            "INSERT OR REPLACE INTO sessions
                 (minute, utc_offset_s, dose, day_load, mode, entry, weight)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                sample.at.ms.div_euclid(60_000),
                sample.at.utc_offset_s,
                sample.dose,
                sample.day_load,
                mode_key(sample.mode),
                entry,
                weight,
            ],
        )?;
        Ok(())
    }

    pub fn latest(&self) -> rusqlite::Result<Option<Sample>> {
        self.conn
            .query_row(
                "SELECT minute, utc_offset_s, dose, day_load, mode, entry, weight
                 FROM sessions ORDER BY minute DESC LIMIT 1",
                [],
                sample_from_row,
            )
            .optional()
    }

    /// Every recorded minute from `since` on, oldest first.
    pub fn samples_since(&self, since: Moment) -> rusqlite::Result<Vec<Sample>> {
        self.conn
            .prepare(
                "SELECT minute, utc_offset_s, dose, day_load, mode, entry, weight
                 FROM sessions WHERE minute >= ?1 ORDER BY minute",
            )?
            .query_map([since.ms.div_euclid(60_000)], sample_from_row)?
            .collect()
    }

    /// Every recorded minute from `since` on, as kinds of time.
    pub fn minutes_since(&self, since: Moment) -> rusqlite::Result<Vec<Minute>> {
        self.conn
            .prepare(
                "SELECT minute, utc_offset_s, mode, weight FROM sessions
                 WHERE minute >= ?1 ORDER BY minute",
            )?
            .query_map([since.ms.div_euclid(60_000)], |row| {
                let mode: String = row.get(2)?;
                Ok(Minute {
                    at: Moment {
                        ms: row.get::<_, i64>(0)? * 60_000,
                        utc_offset_s: row.get(1)?,
                    },
                    mode: mode_from_key(&mode),
                    weight: row.get(3)?,
                })
            })?
            .collect()
    }

    /// Minutes on each nourishing list entry from `since` on.
    pub fn nourishing_entries_since(&self, since: Moment) -> rusqlite::Result<Vec<(String, f64)>> {
        self.conn
            .prepare(
                "SELECT entry, COUNT(*) FROM sessions
                 WHERE minute >= ?1 AND weight > 0 AND entry IS NOT NULL
                 GROUP BY entry",
            )?
            .query_map([since.ms.div_euclid(60_000)], |row| {
                Ok((row.get(0)?, row.get::<_, i64>(1)? as f64))
            })?
            .collect()
    }

    /// The first minute ever recorded.
    pub fn first(&self) -> rusqlite::Result<Option<Moment>> {
        self.conn
            .query_row(
                "SELECT minute, utc_offset_s FROM sessions ORDER BY minute LIMIT 1",
                [],
                |row| {
                    Ok(Moment {
                        ms: row.get::<_, i64>(0)? * 60_000,
                        utc_offset_s: row.get(1)?,
                    })
                },
            )
            .optional()
    }

    pub fn home_value(&self, key: &str) -> rusqlite::Result<Option<i64>> {
        self.conn
            .query_row("SELECT value FROM home WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()
    }

    pub fn set_home_value(&self, key: &str, value: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO home (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    /// Everything Home remembers under keys starting with `prefix`, with the
    /// prefix taken off.
    pub fn home_values_with_prefix(&self, prefix: &str) -> rusqlite::Result<Vec<(String, i64)>> {
        self.conn
            .prepare("SELECT key, value FROM home WHERE substr(key, 1, length(?1)) = ?1")?
            .query_map([prefix], |row| {
                let key: String = row.get(0)?;
                Ok((key[prefix.len()..].to_owned(), row.get(1)?))
            })?
            .collect()
    }

    /// Every garden day recorded, oldest first.
    pub fn garden(&self) -> rusqlite::Result<Vec<GardenDay>> {
        self.conn
            .prepare("SELECT day, plant, fireflies FROM garden ORDER BY day")?
            .query_map([], |row| {
                let plant: Option<String> = row.get(1)?;
                Ok(GardenDay {
                    day: row.get(0)?,
                    plant: plant.as_deref().and_then(Plant::from_key),
                    fireflies: row.get::<_, i64>(2)? != 0,
                })
            })?
            .collect()
    }

    pub fn record_garden_day(&self, day: &GardenDay) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO garden (day, plant, fireflies) VALUES (?1, ?2, ?3)",
            params![day.day, day.plant.map(Plant::key), i64::from(day.fireflies)],
        )?;
        Ok(())
    }

    /// Drop history older than the retention period.
    pub fn prune(&self, now: Moment) -> rusqlite::Result<usize> {
        let cutoff = now.ms.div_euclid(60_000) - RETENTION_DAYS * 24 * 60;
        self.conn
            .execute("DELETE FROM sessions WHERE minute < ?1", [cutoff])
    }

    /// Everything the dose engine and garden know, gone in one action. Every
    /// table is emptied, including any added after this was written.
    #[allow(dead_code, reason = "not reachable from the interface yet")]
    /// Take the sites out of the last `ms`, leaving how the time felt. The
    /// dose, the day's load and the weight stay exactly as they were: this
    /// forgets where you were, not that the hour happened.
    pub fn forget_entries_since(&self, now: Moment, ms: i64) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE sessions SET entry = NULL WHERE minute >= ?1",
            params![(now.ms - ms).div_euclid(60_000)],
        )
    }

    pub fn forget_everything(&self) -> rusqlite::Result<()> {
        let tables: Vec<String> = self
            .conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            )?
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        let tx = self.conn.unchecked_transaction()?;
        for table in tables {
            tx.execute(
                &format!("DELETE FROM \"{}\"", table.replace('"', "\"\"")),
                [],
            )?;
        }
        tx.commit()?;
        // Reclaim the pages so the deleted rows don't linger in the file.
        self.conn.execute_batch("VACUUM")?;
        Ok(())
    }
}

/// A `sessions` row selected as minute, utc_offset_s, dose, day_load, mode,
/// entry, weight.
fn sample_from_row(row: &rusqlite::Row) -> rusqlite::Result<Sample> {
    let entry: Option<String> = row.get(5)?;
    let weight: Option<f64> = row.get(6)?;
    let mode: String = row.get(4)?;
    Ok(Sample {
        at: Moment {
            ms: row.get::<_, i64>(0)? * 60_000,
            utc_offset_s: row.get(1)?,
        },
        dose: row.get(2)?,
        day_load: row.get(3)?,
        mode: mode_from_key(&mode),
        place: match (entry, weight) {
            (Some(entry), Some(weight)) => Some(Place::Listed {
                entry,
                weight,
                news: false,
            }),
            _ => None,
        },
    })
}

fn mode_key(mode: Mode) -> &'static str {
    match mode {
        Mode::Away => "away",
        Mode::Draining => "draining",
        Mode::Resting => "resting",
        Mode::Nourishing => "nourishing",
        Mode::Holding => "holding",
    }
}

fn mode_from_key(key: &str) -> Mode {
    match key {
        "draining" => Mode::Draining,
        "resting" => Mode::Resting,
        "nourishing" => Mode::Nourishing,
        "holding" => Mode::Holding,
        _ => Mode::Away,
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn forgetting_an_hour_takes_the_sites_and_leaves_the_feeling() {
        let store = Store::in_memory();
        let now = Moment {
            ms: 10_000 * 60_000,
            utc_offset_s: 0,
        };
        let sample = |ms: i64, entry: &str| Sample {
            at: Moment { ms, ..now },
            dose: 0.4,
            day_load: 0.2,
            mode: Mode::Draining,
            place: Some(Place::Listed {
                entry: entry.to_owned(),
                weight: -1.0,
                news: false,
            }),
        };
        store
            .record(&sample(now.ms - 3 * 3_600_000, "kept.example"))
            .unwrap();
        store
            .record(&sample(now.ms - 10 * 60_000, "gone.example"))
            .unwrap();

        assert_eq!(store.forget_entries_since(now, 3_600_000).unwrap(), 1);

        let samples = store.samples_since(Moment { ms: 0, ..now }).unwrap();
        let named: Vec<&str> = samples
            .iter()
            .filter_map(|s| match &s.place {
                Some(Place::Listed { entry, .. }) => Some(entry.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            named,
            vec!["kept.example"],
            "only the older site is still named"
        );
        // How the time felt is untouched: both minutes are still there, both
        // still wearing.
        let minutes = store.minutes_since(Moment { ms: 0, ..now }).unwrap();
        assert_eq!(minutes.len(), 2);
        assert!(minutes.iter().all(|m| m.weight == Some(-1.0)));
    }
    use super::*;
    use crate::reputation::Lists;

    fn at(minute: i64) -> Moment {
        Moment {
            ms: 1_789_426_800_000 + minute * 60_000 + 12_345,
            utc_offset_s: 3600,
        }
    }

    fn sample(minute: i64, dose: f64) -> Sample {
        Sample {
            at: at(minute),
            dose,
            day_load: 3.0,
            mode: Mode::Draining,
            place: Some(Place::Listed {
                entry: "tiktok.com".into(),
                weight: -1.0,
                news: false,
            }),
        }
    }

    #[test]
    fn keeps_one_row_per_minute_and_resumes_from_the_latest() {
        let store = Store::in_memory();
        store.record(&sample(0, 0.1)).unwrap();
        store.record(&sample(0, 0.2)).unwrap();
        store.record(&sample(1, 0.3)).unwrap();
        let count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2);
        let latest = store.latest().unwrap().expect("a sample");
        assert_eq!(latest.dose, 0.3);
        let all = store.samples_since(at(0)).unwrap();
        assert_eq!(all.iter().map(|s| s.dose).collect::<Vec<_>>(), [0.2, 0.3]);
        assert_eq!(store.samples_since(at(1)).unwrap().len(), 1);
        assert_eq!(latest.at.ms, at(1).ms - 12_345);
        assert_eq!(latest.place, sample(1, 0.3).place);
    }

    #[test]
    fn home_reads_kinds_of_time_and_remembers_what_it_said() {
        let store = Store::in_memory();
        store.record(&sample(0, 0.1)).unwrap();
        let mut good = sample(1, 0.1);
        good.mode = Mode::Nourishing;
        good.place = Some(Place::Listed {
            entry: "gutenberg.org".into(),
            weight: 1.0,
            news: false,
        });
        store.record(&good).unwrap();
        let minutes = store.minutes_since(at(0)).unwrap();
        assert_eq!(minutes.len(), 2);
        assert_eq!(minutes[1].weight, Some(1.0));
        assert_eq!(
            store.nourishing_entries_since(at(0)).unwrap(),
            vec![("gutenberg.org".to_string(), 1.0)]
        );
        assert_eq!(
            store.first().unwrap().map(|m| m.ms),
            Some(at(0).ms - 12_345)
        );

        assert_eq!(store.home_value("visits").unwrap(), None);
        store.set_home_value("visits", 2).unwrap();
        assert_eq!(store.home_value("visits").unwrap(), Some(2));
        store.set_home_value("offered:rhs.org.uk", 40).unwrap();
        store.set_home_value("offered_not:x", 1).unwrap();
        assert_eq!(
            store.home_values_with_prefix("offered:").unwrap(),
            vec![("rhs.org.uk".to_string(), 40)]
        );

        let day = GardenDay {
            day: 7,
            plant: Some(Plant::Fern),
            fireflies: true,
        };
        store.record_garden_day(&day).unwrap();
        // A day, once grown, never changes.
        store
            .record_garden_day(&GardenDay { plant: None, ..day })
            .unwrap();
        assert_eq!(store.garden().unwrap(), vec![day]);
    }

    #[test]
    fn history_older_than_ninety_days_is_pruned() {
        let store = Store::in_memory();
        store.record(&sample(0, 0.1)).unwrap();
        store.record(&sample(RETENTION_DAYS * 1440, 0.2)).unwrap();
        let removed = store.prune(at(RETENTION_DAYS * 1440 + 1)).unwrap();
        assert_eq!(removed, 1);
        assert_eq!(store.latest().unwrap().unwrap().dose, 0.2);
    }

    #[test]
    fn forgetting_empties_every_table_even_new_ones() {
        let store = Store::in_memory();
        store.record(&sample(0, 0.1)).unwrap();
        store.set_home_value("visits", 3).unwrap();
        store
            .conn
            .execute_batch(
                "CREATE TABLE later_feature (seed INTEGER); INSERT INTO later_feature VALUES (7);",
            )
            .unwrap();
        store.forget_everything().unwrap();
        assert_eq!(store.latest().unwrap(), None);
        assert_eq!(store.home_value("visits").unwrap(), None);
        let garden: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM later_feature", [], |r| r.get(0))
            .unwrap();
        assert_eq!(garden, 0);
    }

    #[test]
    fn addresses_never_reach_the_database_file() {
        // Browse to a page whose address carries a canary, record what the
        // engine would, then search every byte on disk for the canary.
        let canary = "wisp-canary-7f3a9c";
        let address = format!("https://www.tiktok.com/@{canary}/video/1?q={canary}");
        let place = Lists::bundled().place(&address);
        assert!(matches!(place, Place::Listed { .. }));

        let dir = std::env::temp_dir().join(format!("wisp-store-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("glimmerwood.sqlite");
        {
            let store = Store::open(&path).unwrap();
            let mut s = sample(0, 0.4);
            s.place = Some(place);
            store.record(&s).unwrap();
        }
        for entry in std::fs::read_dir(&dir).unwrap() {
            let bytes = std::fs::read(entry.unwrap().path()).unwrap();
            let found = bytes.windows(canary.len()).any(|w| w == canary.as_bytes());
            assert!(!found, "the address leaked into the database");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

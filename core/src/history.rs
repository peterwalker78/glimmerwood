//! Where you have been this week, and no longer than that.
//!
//! Kept apart from the dose store on purpose: that one never sees an address,
//! and this one holds nothing but addresses. Seven days, then gone, so this
//! is somewhere to find Tuesday's page again rather than an archive of a
//! life. Sites on the private list never arrive here at all — the companion
//! decides that before it calls `record`.

use std::path::Path;

use rusqlite::{Connection, params};

use crate::dose::Moment;
use crate::nav;
use crate::pages;

/// How long a page is remembered. Not configurable: a week is what makes it
/// useful for finding something again, and anything longer is an archive.
pub const KEEP_DAYS: i64 = 7;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS visits (
        at    INTEGER NOT NULL,
        url   TEXT    NOT NULL,
        title TEXT    NOT NULL,
        host  TEXT    NOT NULL
    );
    CREATE INDEX IF NOT EXISTS visits_at ON visits (at);
";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Visit {
    pub at: i64,
    pub url: String,
    pub title: String,
    pub host: String,
}

pub struct History {
    conn: Connection,
}

impl History {
    pub fn open(path: &Path) -> rusqlite::Result<History> {
        History::init(Connection::open(path)?)
    }

    #[cfg(test)]
    pub fn in_memory() -> History {
        History::init(Connection::open_in_memory().expect("in-memory sqlite")).expect("schema")
    }

    fn init(conn: Connection) -> rusqlite::Result<History> {
        conn.execute_batch(SCHEMA)?;
        Ok(History { conn })
    }

    /// Remember a page. A page reloaded or returned to within the minute
    /// replaces its earlier row rather than adding another, so a site that
    /// redirects through itself doesn't fill the week.
    pub fn record(&self, at: Moment, url: &str, title: &str) -> rusqlite::Result<()> {
        if !worth_keeping(url) {
            return Ok(());
        }
        let host = nav::host_of(url);
        let recent = at.ms - 60_000;
        self.conn.execute(
            "DELETE FROM visits WHERE url = ?1 AND at >= ?2",
            params![url, recent],
        )?;
        self.conn.execute(
            "INSERT INTO visits (at, url, title, host) VALUES (?1, ?2, ?3, ?4)",
            params![at.ms, url, title, host],
        )?;
        Ok(())
    }

    /// Everything since `from`, newest first.
    pub fn since(&self, from: i64) -> rusqlite::Result<Vec<Visit>> {
        let mut statement = self
            .conn
            .prepare("SELECT at, url, title, host FROM visits WHERE at >= ?1 ORDER BY at DESC")?;
        let rows = statement.query_map(params![from], |row| {
            Ok(Visit {
                at: row.get(0)?,
                url: row.get(1)?,
                title: row.get(2)?,
                host: row.get(3)?,
            })
        })?;
        rows.collect()
    }

    /// The week, which is all there ever is.
    pub fn week(&self, now: Moment) -> rusqlite::Result<Vec<Visit>> {
        self.since(now.ms - KEEP_DAYS * DAY_MS)
    }

    /// Drop anything past the week. Called whenever the companion prunes.
    pub fn prune(&self, now: Moment) -> rusqlite::Result<usize> {
        self.conn.execute(
            "DELETE FROM visits WHERE at < ?1",
            params![now.ms - KEEP_DAYS * DAY_MS],
        )
    }

    /// Forget the last `ms` milliseconds: a way out of somewhere you didn't
    /// mean to go, without wiping the week.
    pub fn forget_since(&self, now: Moment, ms: i64) -> rusqlite::Result<usize> {
        self.conn
            .execute("DELETE FROM visits WHERE at >= ?1", params![now.ms - ms])
    }

    pub fn forget_everything(&self) -> rusqlite::Result<()> {
        self.conn.execute_batch("DELETE FROM visits;")
    }

    pub fn is_empty(&self) -> bool {
        self.conn
            .query_row("SELECT COUNT(*) FROM visits", [], |row| {
                row.get::<_, i64>(0)
            })
            .map(|count| count == 0)
            .unwrap_or(true)
    }
}

/// Glimmerwood's own pages, blank tabs and anything that isn't the web are
/// not places you went.
fn worth_keeping(url: &str) -> bool {
    !url.is_empty() && !pages::is_local_page(url) && !nav::host_of(url).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: i64) -> Moment {
        Moment {
            ms,
            utc_offset_s: 0,
        }
    }

    #[test]
    fn a_page_is_remembered_with_its_site() {
        let history = History::in_memory();
        history
            .record(at(1_000), "https://www.example.org/a", "An example")
            .unwrap();
        let visits = history.since(0).unwrap();
        assert_eq!(visits.len(), 1);
        assert_eq!(visits[0].host, "example.org");
        assert_eq!(visits[0].title, "An example");
    }

    #[test]
    fn glimmerwoods_own_pages_are_not_places_you_went() {
        let history = History::in_memory();
        history
            .record(at(1), "glimmerwood://home/", "Home")
            .unwrap();
        history.record(at(2), "", "").unwrap();
        history.record(at(3), "about:blank", "").unwrap();
        assert!(history.is_empty());
    }

    #[test]
    fn returning_within_the_minute_replaces_rather_than_repeats() {
        let history = History::in_memory();
        history
            .record(at(0), "https://example.org/", "One")
            .unwrap();
        history
            .record(at(30_000), "https://example.org/", "One again")
            .unwrap();
        let visits = history.since(0).unwrap();
        assert_eq!(visits.len(), 1);
        assert_eq!(visits[0].title, "One again");
    }

    #[test]
    fn coming_back_later_is_its_own_visit() {
        let history = History::in_memory();
        history
            .record(at(0), "https://example.org/", "One")
            .unwrap();
        history
            .record(at(120_000), "https://example.org/", "One")
            .unwrap();
        assert_eq!(history.since(0).unwrap().len(), 2);
    }

    #[test]
    fn nothing_older_than_the_week_survives_a_prune() {
        let history = History::in_memory();
        let now = at(30 * DAY_MS);
        history
            .record(at(now.ms - 8 * DAY_MS), "https://old.example/", "Old")
            .unwrap();
        history
            .record(at(now.ms - 2 * DAY_MS), "https://recent.example/", "Recent")
            .unwrap();
        assert_eq!(history.prune(now).unwrap(), 1);
        let left = history.week(now).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].host, "recent.example");
    }

    #[test]
    fn the_week_never_reaches_past_seven_days_even_unpruned() {
        let history = History::in_memory();
        let now = at(30 * DAY_MS);
        history
            .record(at(now.ms - 8 * DAY_MS), "https://old.example/", "Old")
            .unwrap();
        assert!(history.week(now).unwrap().is_empty());
    }

    #[test]
    fn the_last_hour_can_go_without_the_week_going() {
        let history = History::in_memory();
        let now = at(10 * DAY_MS);
        history
            .record(
                at(now.ms - 3 * 60 * 60 * 1000),
                "https://kept.example/",
                "Kept",
            )
            .unwrap();
        history
            .record(at(now.ms - 10 * 60 * 1000), "https://gone.example/", "Gone")
            .unwrap();
        assert_eq!(history.forget_since(now, 60 * 60 * 1000).unwrap(), 1);
        let left = history.week(now).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].host, "kept.example");
    }

    #[test]
    fn newest_comes_first() {
        let history = History::in_memory();
        history
            .record(at(1_000), "https://one.example/", "One")
            .unwrap();
        history
            .record(at(2_000), "https://two.example/", "Two")
            .unwrap();
        let visits = history.since(0).unwrap();
        assert_eq!(visits[0].host, "two.example");
    }
}

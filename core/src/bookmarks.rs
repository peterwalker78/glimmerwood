//! The user's bookmarks: their own data, in their own file, apart
//! from the dose history so forgetting one never touches the other.

use std::path::Path;

use rusqlite::{Connection, params};

use crate::dose::Moment;

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS bookmarks (
        id    INTEGER PRIMARY KEY,
        url   TEXT    NOT NULL UNIQUE,
        title TEXT    NOT NULL,
        added INTEGER NOT NULL
    );
";

#[derive(Clone, Debug, PartialEq)]
pub struct Bookmark {
    pub id: i64,
    pub url: String,
    pub title: String,
}

pub struct Bookmarks {
    conn: Connection,
}

impl Bookmarks {
    pub fn open(path: &Path) -> rusqlite::Result<Bookmarks> {
        Bookmarks::init(Connection::open(path)?)
    }

    /// Bookmarks that last as long as the process: for the feel lab.
    pub fn in_memory() -> Bookmarks {
        Bookmarks::init(Connection::open_in_memory().expect("in-memory sqlite")).expect("schema")
    }

    fn init(conn: Connection) -> rusqlite::Result<Bookmarks> {
        conn.execute_batch(SCHEMA)?;
        Ok(Bookmarks { conn })
    }

    /// Newest first.
    pub fn list(&self) -> rusqlite::Result<Vec<Bookmark>> {
        self.conn
            .prepare("SELECT id, url, title FROM bookmarks ORDER BY added DESC, id DESC")?
            .query_map([], |row| {
                Ok(Bookmark {
                    id: row.get(0)?,
                    url: row.get(1)?,
                    title: row.get(2)?,
                })
            })?
            .collect()
    }

    pub fn contains(&self, url: &str) -> rusqlite::Result<bool> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM bookmarks WHERE url = ?1",
                [url],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n > 0)
    }

    /// Bookmark `url`, or forget it if it's already bookmarked. Returns
    /// whether it is bookmarked now.
    pub fn toggle(&self, url: &str, title: &str, now: Moment) -> rusqlite::Result<bool> {
        if self.contains(url)? {
            self.conn
                .execute("DELETE FROM bookmarks WHERE url = ?1", [url])?;
            return Ok(false);
        }
        self.conn.execute(
            "INSERT INTO bookmarks (url, title, added) VALUES (?1, ?2, ?3)",
            params![url, title, now.ms],
        )?;
        Ok(true)
    }

    pub fn remove(&self, id: i64) -> rusqlite::Result<()> {
        self.conn
            .execute("DELETE FROM bookmarks WHERE id = ?1", [id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(n: i64) -> Moment {
        Moment {
            ms: 1_789_426_800_000 + n,
            utc_offset_s: 0,
        }
    }

    #[test]
    fn bookmarks_toggle_list_newest_first_and_remove() {
        let b = Bookmarks::in_memory();
        assert!(b.toggle("https://a.example/", "A", at(1)).unwrap());
        assert!(b.toggle("https://b.example/", "B", at(2)).unwrap());
        let list = b.list().unwrap();
        assert_eq!(
            list.iter().map(|x| x.title.as_str()).collect::<Vec<_>>(),
            ["B", "A"]
        );
        assert!(!b.toggle("https://a.example/", "A", at(3)).unwrap());
        assert!(!b.contains("https://a.example/").unwrap());
        b.remove(list[0].id).unwrap();
        assert!(b.list().unwrap().is_empty());
    }
}

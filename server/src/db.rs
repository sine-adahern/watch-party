//! Phase 4: the video library persisted in SQLite (via `rusqlite`, bundled so
//! there's no system dependency). Replaces the Phase 1 `videos.json` file.
//!
//! The connection lives behind a `std::sync::Mutex` in `AppState`. Queries are
//! tiny, so we run them directly under a short lock (never across `.await`). For
//! heavier load you'd move to `sqlx` or wrap calls in `spawn_blocking`.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection};

use shared::Video;

pub fn open(path: &Path) -> Mutex<Connection> {
    let conn = Connection::open(path).expect("open sqlite database");
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS videos (
            id          TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            uploaded_at INTEGER NOT NULL
        );",
    )
    .expect("initialise schema");
    Mutex::new(conn)
}

pub fn list(conn: &Connection) -> Vec<Video> {
    let Ok(mut stmt) =
        conn.prepare("SELECT id, name, uploaded_at FROM videos ORDER BY uploaded_at DESC")
    else {
        return Vec::new();
    };
    let rows = stmt.query_map([], |r| {
        Ok(Video {
            id: r.get(0)?,
            name: r.get(1)?,
            uploaded_at: r.get(2)?,
        })
    });
    match rows {
        Ok(iter) => iter.filter_map(Result::ok).collect(),
        Err(_) => Vec::new(),
    }
}

pub fn insert(conn: &Connection, v: &Video) {
    let _ = conn.execute(
        "INSERT OR REPLACE INTO videos (id, name, uploaded_at) VALUES (?1, ?2, ?3)",
        params![v.id, v.name, v.uploaded_at],
    );
}

/// Returns true if a row was actually removed.
pub fn delete(conn: &Connection, id: &str) -> bool {
    conn.execute("DELETE FROM videos WHERE id = ?1", params![id])
        .map(|n| n > 0)
        .unwrap_or(false)
}

use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};

const KEEP_NEWEST: u64 = 10_000;

#[derive(Debug, Clone)]
pub struct Entry {
    pub key: [u8; 32],
    pub session_id: String,
    pub turn: u64,
    pub digest: String,
    pub raw: Vec<u8>,
    pub exit_code: i32,
    pub created_at: u64,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Stats {
    pub hits: u64,
    pub misses: u64,
    pub bypass: u64,
    pub uncached_failures: u64,
    pub no_exit_code: u64,
    pub tokens_saved: u64,
}

pub struct Store {
    conn: Connection,
    cache_dir: PathBuf,
}

impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Store> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let cache_dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_millis(5000))?;
        // WAL is persistent: only the first connection can flip the mode, and that
        // switch does NOT honor the busy handler — later openers may hit immediate
        // SQLITE_BUSY here. The mode already being WAL makes failure harmless.
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        let _ = conn.pragma_update(None, "synchronous", "NORMAL");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS entries (
                key        BLOB PRIMARY KEY,
                session_id TEXT NOT NULL,
                turn       INTEGER NOT NULL,
                digest     TEXT NOT NULL,
                raw        BLOB NOT NULL,
                exit_code  INTEGER NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS seen (
                session_id TEXT NOT NULL,
                key        BLOB NOT NULL,
                lines      INTEGER NOT NULL,
                PRIMARY KEY (session_id, key)
            );
            CREATE TABLE IF NOT EXISTS events (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                kind         TEXT NOT NULL,
                tokens_saved INTEGER NOT NULL DEFAULT 0,
                ts           INTEGER NOT NULL,
                label        TEXT NOT NULL DEFAULT ''
            );",
        )?;
        // migration for dbs created before the label column (M1 pre-release only)
        let _ = conn.execute("ALTER TABLE events ADD COLUMN label TEXT NOT NULL DEFAULT ''", []);
        Ok(Store { conn, cache_dir })
    }

    /// Marker/marker-lock root — next to the DB (tests point RERAN-less temp DBs here).
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    pub fn get(&self, key: &[u8; 32]) -> rusqlite::Result<Option<Entry>> {
        self.conn
            .query_row(
                "SELECT key, session_id, turn, digest, raw, exit_code, created_at
                 FROM entries WHERE key = ?1",
                params![key.as_slice()],
                |row| {
                    Ok(Entry {
                        key: row.get::<_, Vec<u8>>(0)?.try_into().unwrap_or([0; 32]),
                        session_id: row.get(1)?,
                        turn: row.get(2)?,
                        digest: row.get(3)?,
                        raw: row.get(4)?,
                        exit_code: row.get(5)?,
                        created_at: row.get(6)?,
                    })
                },
            )
            .optional()
    }

    pub fn put(&self, e: &Entry) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO entries
             (key, session_id, turn, digest, raw, exit_code, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![e.key.as_slice(), e.session_id, e.turn, e.digest, e.raw, e.exit_code, e.created_at],
        )?;
        // idempotent eviction: keep the newest KEEP_NEWEST entries (bkt#38: tolerant of racing cleaners)
        self.conn.execute(
            "DELETE FROM entries WHERE key IN (
                SELECT key FROM entries ORDER BY created_at DESC LIMIT -1 OFFSET ?1
            )",
            params![KEEP_NEWEST],
        )?;
        Ok(())
    }

    pub fn record_seen(&self, session: &str, key: &[u8; 32], lines: u64) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO seen (session_id, key, lines) VALUES (?1, ?2, ?3)
             ON CONFLICT(session_id, key) DO UPDATE SET lines = MAX(lines, excluded.lines)",
            params![session, key.as_slice(), lines],
        )?;
        Ok(())
    }

    pub fn saw_whole(&self, session: &str, key: &[u8; 32], lines: u64) -> rusqlite::Result<bool> {
        let seen: Option<u64> = self
            .conn
            .query_row(
                "SELECT lines FROM seen WHERE session_id = ?1 AND key = ?2",
                params![session, key.as_slice()],
                |row| row.get(0),
            )
            .optional()?;
        Ok(seen.is_some_and(|s| s >= lines))
    }

    /// Monotonic per-session counter: distinct cached commands so far.
    pub fn session_turn(&self, session: &str) -> rusqlite::Result<u64> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM seen WHERE session_id = ?1",
            params![session],
            |row| row.get(0),
        )
    }

    pub fn record_event(&self, kind: &str, tokens_saved: u64, label: &str) -> rusqlite::Result<()> {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let label: String = label.chars().take(60).collect();
        self.conn.execute(
            "INSERT INTO events (kind, tokens_saved, ts, label) VALUES (?1, ?2, ?3, ?4)",
            params![kind, tokens_saved, ts, label],
        )?;
        Ok(())
    }

    /// Top commands by tokens saved (hits only) — `reran gain` "By command".
    pub fn top_labels(&self, limit: usize) -> rusqlite::Result<Vec<(String, u64, u64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT label, COUNT(*), SUM(tokens_saved) FROM events
             WHERE kind = 'hit' AND label != ''
             GROUP BY label ORDER BY SUM(tokens_saved) DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, Option<u64>>(2)?.unwrap_or(0),
            ))
        })?;
        rows.collect()
    }

    /// Most recent events, newest first — `reran gain --history`.
    /// Returns (kind, tokens_saved, label) triples.
    pub fn recent_events(&self, limit: usize) -> rusqlite::Result<Vec<(String, u64, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT kind, tokens_saved, label FROM events ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        rows.collect()
    }

    pub fn stats(&self) -> rusqlite::Result<Stats> {
        let mut stmt = self.conn.prepare(
            "SELECT kind, COUNT(*), SUM(tokens_saved) FROM events GROUP BY kind",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, Option<u64>>(2)?.unwrap_or(0),
            ))
        })?;
        let mut s = Stats::default();
        for row in rows {
            let (kind, count, saved) = row?;
            match kind.as_str() {
                "hit" => {
                    s.hits = count;
                    s.tokens_saved = saved;
                }
                "miss" => s.misses = count,
                "bypass" => s.bypass = count,
                "uncached_failure" => s.uncached_failures = count,
                "no_exit_code" => s.no_exit_code = count,
                _ => {}
            }
        }
        Ok(s)
    }

    pub fn default_db_path() -> PathBuf {
        if let Ok(p) = std::env::var("RERAN_DB") {
            return PathBuf::from(p);
        }
        let cache = std::env::var("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".cache")
            });
        cache.join("reran").join("reran.db")
    }
}

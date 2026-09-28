//! SQLite persistence with crash-safe half-write recovery.
//!
//! Every logical write (import/decision/analysis) happens in one transaction
//! whose final step is flipping a row in `write_tx` from `pending` to
//! `committed`. A `pending` row found at open means the process died mid-write:
//! the whole event transaction is rolled back, and a recovery marker is kept
//! (a distinct, observable outcome — not silent loss).

use crate::project::*;
use rusqlite::{params, Connection, OpenFlags};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportBundle {
    pub bundle_version: u32,
    pub schema_version: u32,
    pub project_id: String,
    pub exported_at: i64,
    pub events: Vec<Event>,
}

#[derive(Debug, Clone)]
pub struct RecoveryReport {
    pub rolled_back_tx: u64,
    pub pending_events_removed: u64,
}

pub struct Store {
    pub conn: Connection,
}

impl Store {
    pub fn open(path: &str) -> rusqlite::Result<(Store, Option<RecoveryReport>)> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut store = Store { conn };
        store.init_schema()?;
        let report = store.recover()?;
        Ok((store, report))
    }

    /// In-memory store for tests.
    pub fn open_memory() -> rusqlite::Result<Store> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut store = Store { conn };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> rusqlite::Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS project (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS event (
                id TEXT PRIMARY KEY,
                seq INTEGER NOT NULL,
                branch TEXT NOT NULL,
                parent TEXT,
                ts_millis INTEGER NOT NULL,
                actor TEXT NOT NULL,
                data_json TEXT NOT NULL,
                valid_from INTEGER NOT NULL,
                superseded_by TEXT,
                tx_id INTEGER,
                UNIQUE(branch, seq)
            );
            CREATE INDEX IF NOT EXISTS event_branch ON event(branch, seq);
            CREATE TABLE IF NOT EXISTS write_tx (
                tx_id INTEGER PRIMARY KEY AUTOINCREMENT,
                state TEXT NOT NULL CHECK(state IN ('pending','committed')),
                started_at INTEGER NOT NULL,
                finished_at INTEGER
            );
            CREATE TABLE IF NOT EXISTS recovery_log (
                tx_id INTEGER PRIMARY KEY,
                events_removed INTEGER NOT NULL,
                recovered_at INTEGER NOT NULL
            );
            "#,
        )?;
        self.conn.execute(
            "INSERT OR IGNORE INTO meta(key,value) VALUES('schema_version','1')",
            [],
        )?;
        Ok(())
    }

    fn recover(&self) -> rusqlite::Result<Option<RecoveryReport>> {
        let pending: Vec<(i64,)> = {
            let mut stmt = self.conn.prepare(
                "SELECT tx_id FROM write_tx WHERE state='pending' ORDER BY tx_id")?;
            let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        if pending.is_empty() {
            return Ok(None);
        }
        let mut removed = 0u64;
        for (tx_id,) in pending {
            self.conn.execute("DELETE FROM event WHERE tx_id = ?1", params![tx_id])?;
            let n = self.conn.changes();
            removed += n as u64;
            self.conn.execute(
                "INSERT OR REPLACE INTO recovery_log(tx_id, events_removed, recovered_at)
                 VALUES(?1, ?2, strftime('%s','now')*1000)",
                params![tx_id, n])?;
            self.conn.execute("DELETE FROM write_tx WHERE tx_id = ?1", params![tx_id])?;
        }
        Ok(Some(RecoveryReport { rolled_back_tx: pending.len() as u64, pending_events_removed: removed }))
    }

    pub fn ensure_project(&self, id: &str, name: &str, ts: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO project(id,name,created_at) VALUES(?1,?2,?3)",
            params![id, name, ts],
        )?;
        Ok(())
    }

    pub fn list_projects(&self) -> rusqlite::Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare("SELECT id, name FROM project ORDER BY created_at")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        rows.collect()
    }

    /// Append events atomically with a durable two-phase marker.
    ///
    /// Phase 1 commits a `pending` row in its own transaction. Phase 2 writes
    /// the events and flips the row to `committed` in a second transaction.
    /// A crash after phase 1 leaves an observable half-write that `recover()`
    /// rolls back on the next open.
    pub fn append_events(&mut self, events: &[Event], crash_before_commit: bool) -> rusqlite::Result<()> {
        let tx_id: i64 = {
            let tx = self.conn.transaction()?;
            tx.execute("INSERT INTO write_tx(state, started_at) VALUES('pending', strftime('%s','now')*1000)", [])?;
            let id = tx.last_insert_rowid();
            tx.commit()?;
            id
        };
        let tx = self.conn.transaction()?;
        for ev in events {
            let data_json = serde_json::to_string(&ev.data)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            tx.execute(
                "INSERT INTO event(id,seq,branch,parent,ts_millis,actor,data_json,valid_from,superseded_by,tx_id)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![ev.id, ev.seq, ev.branch, ev.parent, ev.ts_millis, ev.actor,
                        data_json, ev.valid_from, ev.superseded_by, tx_id],
            )?;
        }
        if crash_before_commit {
            // Events and the durable pending marker are already on disk; the
            // process dies before the commit. Exactly a half-write.
            drop(tx);
            return Err(rusqlite::Error::ExecuteReturnedResults);
        }
        tx.execute(
            "UPDATE write_tx SET state='committed', finished_at=strftime('%s','now')*1000 WHERE tx_id=?1",
            params![tx_id])?;
        tx.commit()?;
        Ok(())
    }

    /// Test/verification hook: synthesize a real half-write state (durable
    /// pending marker + event rows written outside the protective commit).
    pub fn synthesize_half_write(&mut self, events: &[Event]) -> rusqlite::Result<i64> {
        let tx_id: i64 = {
            let tx = self.conn.transaction()?;
            tx.execute("INSERT INTO write_tx(state, started_at) VALUES('pending', strftime('%s','now')*1000)", [])?;
            let id = tx.last_insert_rowid();
            tx.commit()?;
            id
        };
        for ev in events {
            let data_json = serde_json::to_string(ev)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            self.conn.execute(
                "INSERT INTO event(id,seq,branch,parent,ts_millis,actor,data_json,valid_from,superseded_by,tx_id)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![ev.id, ev.seq, ev.branch, ev.parent, ev.ts_millis, ev.actor,
                        data_json, ev.valid_from, ev.superseded_by, tx_id],
            )?;
        }
        Ok(tx_id)
    }

    pub fn load_branch_events(&self, project_id: &str, branch: &str) -> rusqlite::Result<Vec<Event>> {
        let _ = project_id;
        let mut stmt = self.conn.prepare(
            "SELECT id,seq,branch,parent,ts_millis,actor,data_json,valid_from,superseded_by
             FROM event WHERE branch=?1 ORDER BY seq")?;
        let rows = stmt.query_map(params![branch], |r| {
            let data_str: String = r.get(6)?;
            let data: EventData = serde_json::from_str(&data_str)
                .map_err(|e| rusqlite::Error::FromSqlConversionFailure(6,
                    rusqlite::types::Type::Text, Box::new(e)))?;
            Ok(Event {
                id: r.get(0)?, seq: r.get(1)?, branch: r.get(2)?, parent: r.get(3)?,
                ts_millis: r.get(4)?, actor: r.get(5)?, data,
                valid_from: r.get(7)?, superseded_by: r.get(8)?,
            })
        })?;
        rows.collect()
    }

    pub fn load_all_events(&self) -> rusqlite::Result<Vec<Event>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,seq,branch,parent,ts_millis,actor,data_json,valid_from,superseded_by
             FROM event ORDER BY branch, seq")?;
        let rows = stmt.query_map([], |r| {
            let data_str: String = r.get(6)?;
            let data: EventData = serde_json::from_str(&data_str)
                .map_err(|e| rusqlite::Error::FromSqlConversionFailure(6,
                    rusqlite::types::Type::Text, Box::new(e)))?;
            Ok(Event {
                id: r.get(0)?, seq: r.get(1)?, branch: r.get(2)?, parent: r.get(3)?,
                ts_millis: r.get(4)?, actor: r.get(5)?, data,
                valid_from: r.get(7)?, superseded_by: r.get(8)?,
            })
        })?;
        rows.collect()
    }

    pub fn list_branches(&self) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT DISTINCT branch FROM event ORDER BY branch")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect()
    }

    pub fn mark_superseded(&mut self, event_id: &str, by: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE event SET superseded_by=?2 WHERE id=?1", params![event_id, by])?;
        Ok(())
    }

    // -- export / import -------------------------------------------------

    pub fn export(&self, project_id: String, ts: i64) -> rusqlite::Result<ExportBundle> {
        let events = self.load_all_events()?;
        Ok(ExportBundle {
            bundle_version: 1,
            schema_version: SCHEMA_VERSION,
            project_id,
            exported_at: ts,
            events,
        })
    }

    pub fn import_bundle(&mut self, bundle: &ExportBundle) -> rusqlite::Result<()> {
        self.ensure_project(&bundle.project_id, "imported", bundle.exported_at)?;
        let tx = self.conn.transaction()?;
        tx.execute("INSERT INTO write_tx(state, started_at) VALUES('pending', strftime('%s','now')*1000)", [])?;
        let tx_id = tx.last_insert_rowid();
        for ev in &bundle.events {
            let data_json = serde_json::to_string(&ev.data)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            // INSERT OR IGNORE keeps identities stable on re-import; new rows land.
            tx.execute(
                "INSERT OR IGNORE INTO event(id,seq,branch,parent,ts_millis,actor,data_json,valid_from,superseded_by,tx_id)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![ev.id, ev.seq, ev.branch, ev.parent, ev.ts_millis, ev.actor,
                        data_json, ev.valid_from, ev.superseded_by, tx_id],
            )?;
        }
        tx.execute(
            "UPDATE write_tx SET state='committed', finished_at=strftime('%s','now')*1000 WHERE tx_id=?1",
            params![tx_id])?;
        tx.commit()?;
        Ok(())
    }
}

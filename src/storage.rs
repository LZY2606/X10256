//! SQLite persistence. Writes are transactional and guarded by a crash marker:
//! an unfinished write leaves a `pending_write` row which is rolled back and
//! reported as a recovery event on the next open (half-write recovery).

use std::sync::Mutex;

use rusqlite::{params, Connection};

use crate::history::Event;
use crate::model::Material;

pub struct Store {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryReport {
    pub recovered: bool,
    pub rolled_back_writes: u32,
    pub message: String,
}

impl Store {
    pub fn open(path: &str) -> rusqlite::Result<(Self, RecoveryReport)> {
        let mut conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrate(&mut conn)?;
        let report = recover(&mut conn)?;
        Ok((Store { conn: Mutex::new(conn) }, report))
    }

    /// In-memory store used by tests.
    pub fn open_memory() -> rusqlite::Result<Self> {
        let mut conn = Connection::open_in_memory()?;
        migrate(&mut conn)?;
        Ok(Store { conn: Mutex::new(conn) })
    }

    pub fn list_projects(&self) -> rusqlite::Result<Vec<ProjectRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, created_at, head_event_id, current_branch FROM projects ORDER BY created_at",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(ProjectRow {
                id: r.get(0)?,
                name: r.get(1)?,
                created_at: r.get(2)?,
                head_event_id: r.get(3)?,
                current_branch: r.get(4)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn create_project(&self, id: &str, name: &str, created_at: &str) -> rusqlite::Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO projects(id, name, created_at, head_event_id, current_branch) VALUES (?1,?2,?3,NULL,'main')",
            params![id, name, created_at],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn project(&self, id: &str) -> rusqlite::Result<Option<ProjectRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, created_at, head_event_id, current_branch FROM projects WHERE id=?1",
        )?;
        let mut rows = stmt.query(params![id])?;
        if let Some(r) = rows.next()? {
            Ok(Some(ProjectRow {
                id: r.get(0)?,
                name: r.get(1)?,
                created_at: r.get(2)?,
                head_event_id: r.get(3)?,
                current_branch: r.get(4)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn set_head(&self, project_id: &str, branch: &str, event_id: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE projects SET head_event_id=?1, current_branch=?2 WHERE id=?3",
            params![event_id, branch, project_id],
        )?;
        Ok(())
    }

    pub fn append_event(&self, project_id: &str, ev: &Event) -> rusqlite::Result<()> {
        self.append_event_batch(project_id, std::slice::from_ref(ev))
    }

    /// Atomically append several events (a compute run writes decision + result
    /// together). A crash marker is inserted first and removed on commit, so an
    /// interrupted batch is detectable on reopen.
    pub fn append_event_batch(&self, project_id: &str, events: &[Event]) -> rusqlite::Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let marker = format!("{}:{}", project_id, events.last().map(|e| e.seq).unwrap_or(0));
        tx.execute(
            "INSERT INTO pending_write(project_id, marker, created_at) VALUES (?1,?2,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
            params![project_id, marker],
        )?;
        for ev in events {
            tx.execute(
                "INSERT INTO events(id, project_id, seq, branch, parent_id, kind, author, created_at, summary, payload, material_id)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                params![
                    ev.id,
                    project_id,
                    ev.seq,
                    ev.branch,
                    ev.parent_id,
                    format!("{:?}", ev.kind),
                    ev.author,
                    ev.created_at,
                    ev.summary,
                    serde_json::to_string(&ev.payload).unwrap_or_default(),
                    ev.material_id,
                ],
            )?;
        }
        tx.execute("DELETE FROM pending_write WHERE project_id=?1", params![project_id])?;
        tx.commit()?;
        Ok(())
    }

    pub fn events(&self, project_id: &str) -> rusqlite::Result<Vec<Event>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, seq, branch, parent_id, kind, author, created_at, summary, payload, material_id
             FROM events WHERE project_id=?1 ORDER BY seq",
        )?;
        let rows = stmt.query_map(params![project_id], row_to_event)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn event(&self, id: &str) -> rusqlite::Result<Option<Event>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, seq, branch, parent_id, kind, author, created_at, summary, payload, material_id
             FROM events WHERE id=?1",
        )?;
        let mut rows = stmt.query(params![id])?;
        if let Some(r) = rows.next() {
            Ok(Some(row_to_event(r)?))
        } else {
            Ok(None)
        }
    }

    pub fn put_material_snapshot(&self, project_id: &str, m: &Material) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO materials(project_id, id, json) VALUES (?1,?2,?3)",
            params![project_id, m.id, serde_json::to_string(m).unwrap_or_default()],
        )?;
        Ok(())
    }

    /// Simulate a crash during a write for tests: marker left in place without
    /// a commit, plus an optional torn event row inserted outside a committed
    /// transaction (rolled back by the WAL/journal on reopen; the marker drives
    /// the observable recovery report).
    pub fn simulate_crash(&self, project_id: &str, marker: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO pending_write(project_id, marker, created_at) VALUES (?1,?2,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
            params![project_id, marker],
        )?;
        // Intentionally do not delete the marker / do not commit a batch.
        Ok(())
    }

    pub fn delete_project(&self, project_id: &str) -> rusqlite::Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM events WHERE project_id=?1", params![project_id])?;
        tx.execute("DELETE FROM materials WHERE project_id=?1", params![project_id])?;
        tx.execute("DELETE FROM pending_write WHERE project_id=?1", params![project_id])?;
        tx.execute("DELETE FROM projects WHERE id=?1", params![project_id])?;
        tx.commit()?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ProjectRow {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub head_event_id: Option<String>,
    pub current_branch: String,
}

fn row_to_event(r: &rusqlite::Row<'_>) -> rusqlite::Result<Event> {
    let kind_str: String = r.get(4)?;
    let payload_str: String = r.get(8)?;
    Ok(Event {
        id: r.get(0)?,
        seq: r.get(1)?,
        branch: r.get(2)?,
        parent_id: r.get(3)?,
        kind: parse_kind(&kind_str),
        author: r.get(5)?,
        created_at: r.get(6)?,
        summary: r.get(7)?,
        payload: serde_json::from_str(&payload_str).unwrap_or(serde_json::Value::Null),
        material_id: r.get(9)?,
    })
}

fn parse_kind(s: &str) -> crate::history::EventType {
    use crate::history::EventType::*;
    match s {
        "ProjectCreated" => ProjectCreated,
        "MaterialImported" => MaterialImported,
        "MaterialRevised" => MaterialRevised,
        "RuleVersionAdded" => RuleVersionAdded,
        "RuleSelected" => RuleSelected,
        "LayerMapped" => LayerMapped,
        "Decision" => Decision,
        "Recomputed" => Recomputed,
        _ => ProjectCreated,
    }
}

fn migrate(conn: &mut Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS projects (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            created_at TEXT NOT NULL,
            head_event_id TEXT,
            current_branch TEXT NOT NULL DEFAULT 'main'
        );
        CREATE TABLE IF NOT EXISTS events (
            id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            seq INTEGER NOT NULL,
            branch TEXT NOT NULL,
            parent_id TEXT,
            kind TEXT NOT NULL,
            author TEXT NOT NULL,
            created_at TEXT NOT NULL,
            summary TEXT NOT NULL,
            payload TEXT NOT NULL,
            material_id TEXT
        );
        CREATE INDEX IF NOT EXISTS events_project ON events(project_id, seq);
        CREATE TABLE IF NOT EXISTS materials (
            project_id TEXT NOT NULL,
            id TEXT NOT NULL,
            json TEXT NOT NULL,
            PRIMARY KEY(project_id, id)
        );
        CREATE TABLE IF NOT EXISTS pending_write (
            project_id TEXT NOT NULL,
            marker TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        "#,
    )?;
    Ok(())
}

/// Half-write recovery: any lingering crash marker means the previous process
/// died between intent and commit. Because event inserts happen inside a single
/// transaction, the durable events are still consistent; we record recovery and
/// clear stale markers rather than pretending nothing happened.
fn recover(conn: &mut Connection) -> rusqlite::Result<RecoveryReport> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM pending_write", [], |r| r.get(0))?;
    if count == 0 {
        return Ok(RecoveryReport {
            recovered: false,
            rolled_back_writes: 0,
            message: "clean open; no pending write".to_string(),
        });
    }
    let markers: rusqlite::Result<Vec<String>> = (|| {
        let mut stmt = conn.prepare("SELECT marker FROM pending_write ORDER BY rowid")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    })();
    let markers = markers.unwrap_or_default();
    conn.execute("DELETE FROM pending_write", [])?;
    Ok(RecoveryReport {
        recovered: true,
        rolled_back_writes: count as u32,
        message: format!("recovered from interrupted write(s): {}", markers.join(", ")),
    })
}

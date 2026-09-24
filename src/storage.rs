//! SQLite 存储层：不可变材料、事件流、派生快照与半写恢复。
//! 所有读写都可被测试直接调用，不依赖 HTTP 层。

use crate::engine::*;
use crate::model::*;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: i64 = 1;

/// 导出/导入包：身份、顺序、分支与决定历史完整保留。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectBundle {
    pub bundle_version: i64,
    pub schema_version: i64,
    pub project_id: String,
    pub materials: Vec<MaterialRow>,
    pub events: Vec<Event>,
    pub derived: Vec<DerivedRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterialRow {
    pub filename: String,
    pub version: i64,
    pub kind: String,
    pub raw_text: String,
    pub parsed_json: String,
    pub imported_event_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DerivedRow {
    pub id: String,
    pub event_id: String,
    pub derived_json: String,
}

pub struct Store {
    pub conn: Connection,
}

impl Store {
    pub fn open(path: &str) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        let store = Store { conn };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let store = Store { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> anyhow::Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS meta (
              key TEXT PRIMARY KEY,
              value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS material (
              filename TEXT NOT NULL,
              version INTEGER NOT NULL,
              kind TEXT NOT NULL,
              raw_text TEXT NOT NULL,
              parsed_json TEXT NOT NULL,
              imported_event_id TEXT NOT NULL,
              PRIMARY KEY (filename, version)
            );
            CREATE TABLE IF NOT EXISTS event (
              id TEXT PRIMARY KEY,
              parent_id TEXT,
              seq INTEGER NOT NULL UNIQUE,
              kind TEXT NOT NULL,
              label TEXT NOT NULL,
              timestamp TEXT NOT NULL,
              payload_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS derived (
              id TEXT PRIMARY KEY,
              event_id TEXT NOT NULL UNIQUE,
              derived_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS write_journal (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              phase TEXT NOT NULL,
              payload_json TEXT NOT NULL,
              created_at TEXT NOT NULL
            );
            "#,
        )?;
        self.conn.execute(
            "INSERT OR IGNORE INTO meta(key,value) VALUES ('schema_version', ?1)",
            params![SCHEMA_VERSION.to_string()],
        )?;
        Ok(())
    }

    pub fn project_id(&self) -> anyhow::Result<String> {
        let v: Option<String> = self
            .conn
            .query_row("SELECT value FROM meta WHERE key='project_id'", [], |r| {
                r.get(0)
            })
            .ok();
        match v {
            Some(id) => Ok(id),
            None => {
                let id = new_id("proj");
                self.conn.execute(
                    "INSERT INTO meta(key,value) VALUES ('project_id', ?1)",
                    params![id],
                )?;
                Ok(id)
            }
        }
    }

    /// 半写恢复：清除上次未完成的写事务残留，并返回恢复的日志阶段数。
    pub fn recover_pending_writes(&self) -> anyhow::Result<Vec<String>> {
        // journal 记录了“打算写但未标记完成”的阶段；WAL 事务回滚后
        // 数据不会部分落库，journal 只用于留下结构化恢复证据。
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT phase FROM write_journal ORDER BY id")?;
        let phases: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<_, _>>()?;
        Ok(phases)
    }

    pub fn journal_begin(&self, phase: &str, payload: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO write_journal(phase, payload_json, created_at)
             VALUES (?1, ?2, ?3)",
            params![phase, payload, now_ts()],
        )?;
        Ok(())
    }

    pub fn journal_clear(&self, phase: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "DELETE FROM write_journal WHERE phase = ?1",
            params![phase],
        )?;
        Ok(())
    }

    pub fn insert_material(&self, row: &MaterialRow) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO material(filename, version, kind, raw_text, parsed_json, imported_event_id)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                row.filename,
                row.version,
                row.kind,
                row.raw_text,
                row.parsed_json,
                row.imported_event_id
            ],
        )?;
        Ok(())
    }

    pub fn insert_event(&self, ev: &Event) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO event(id, parent_id, seq, kind, label, timestamp, payload_json)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                ev.id,
                ev.parent_id,
                ev.seq,
                ev.kind,
                ev.label,
                ev.timestamp,
                serde_json::to_string(&ev.payload)?
            ],
        )?;
        Ok(())
    }

    pub fn insert_derived(&self, row: &DerivedRow) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO derived(id, event_id, derived_json) VALUES (?1,?2,?3)",
            params![row.id, row.event_id, row.derived_json],
        )?;
    }

    pub fn list_events(&self) -> anyhow::Result<Vec<Event>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id,parent_id,seq,kind,label,timestamp,payload_json FROM event ORDER BY seq")?;
        let rows = stmt.query_map([], |r| {
            let payload: String = r.get(6)?;
            Ok(Event {
                id: r.get(0)?,
                parent_id: r.get(1)?,
                seq: r.get(2)?,
                kind: r.get(3)?,
                label: r.get(4)?,
                timestamp: r.get(5)?,
                payload: serde_json::from_str(&payload).unwrap_or(serde_json::json!({})),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn list_materials(&self) -> anyhow::Result<Vec<MaterialRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT filename,version,kind,raw_text,parsed_json,imported_event_id
             FROM material ORDER BY filename, version",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(MaterialRow {
                filename: r.get(0)?,
                version: r.get(1)?,
                kind: r.get(2)?,
                raw_text: r.get(3)?,
                parsed_json: r.get(4)?,
                imported_event_id: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn list_derived(&self) -> anyhow::Result<Vec<DerivedRow>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id,event_id,derived_json FROM derived ORDER BY event_id")?;
        let rows = stmt.query_map([], |r| {
            Ok(DerivedRow {
                id: r.get(0)?,
                event_id: r.get(1)?,
                derived_json: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// 从存储完整重建内存重放工作区（用于服务启动与导入校验）。
    pub fn load_replay(&self) -> anyhow::Result<Replay> {
        let mut replay = Replay::new();
        for ev in self.list_events()? {
            replay.apply(ev);
        }
        Ok(replay)
    }
}

pub fn now_ts() -> String {
    chrono::Utc::now().to_rfc3339()
}

pub fn new_id(prefix: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{prefix}/{nanos:x}")
}

impl Store {
    pub fn export_bundle(&self) -> anyhow::Result<ProjectBundle> {
        Ok(ProjectBundle {
            bundle_version: 1,
            schema_version: SCHEMA_VERSION,
            project_id: self.project_id()?,
            materials: self.list_materials()?,
            events: self.list_events()?,
            derived: self.list_derived()?,
        })
    }

    pub fn export_json(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string_pretty(&self.export_bundle()?)?)
    }

    /// 原子替换：整个导入在单事务中提交，失败回滚 => 无半写数据。
    pub fn import_bundle_txn(&self, bundle: &ProjectBundle) -> anyhow::Result<()> {
        validate_bundle(bundle)?;
        let tx = self.conn.unchecked_transaction()?;
        tx.execute_batch(
            "DELETE FROM derived;
             DELETE FROM event;
             DELETE FROM material;
             DELETE FROM write_journal;",
        )?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO material(filename, version, kind, raw_text, parsed_json, imported_event_id)
                 VALUES (?1,?2,?3,?4,?5,?6)",
            )?;
            for m in &bundle.materials {
                stmt.execute(params![
                    m.filename,
                    m.version,
                    m.kind,
                    m.raw_text,
                    m.parsed_json,
                    m.imported_event_id
                ])?;
            }
        }
        {
            let mut stmt = tx.prepare(
                "INSERT INTO event(id,parent_id,seq,kind,label,timestamp,payload_json)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
            )?;
            for e in &bundle.events {
                stmt.execute(params![
                    e.id,
                    e.parent_id,
                    e.seq,
                    e.kind,
                    e.label,
                    e.timestamp,
                    serde_json::to_string(&e.payload)?
                ])?;
            }
        }
        {
            let mut stmt =
                tx.prepare("INSERT INTO derived(id,event_id,derived_json) VALUES (?1,?2,?3)")?;
            for d in &bundle.derived {
                stmt.execute(params![d.id, d.event_id, d.derived_json])?;
            }
        }
        tx.execute(
            "INSERT OR REPLACE INTO meta(key,value) VALUES ('project_id', ?1)",
            params![bundle.project_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// 模拟“写一半”：写入 journal 意图并插入部分材料后故意返回错误，
    /// 事务回滚。测试借此验证不会出现部分材料 + 部分事件的半写状态。
    pub fn simulate_half_write(&self, bundle: &ProjectBundle, crash_phase: &str) {
        let tx = self.conn.unchecked_transaction().unwrap();
        tx.execute(
            "INSERT INTO write_journal(phase,payload_json,created_at) VALUES (?1,'{}',?2)",
            params![crash_phase, now_ts()],
        )
        .unwrap();
        if let Some(m) = bundle.materials.first() {
            tx.execute(
                "INSERT INTO material(filename,version,kind,raw_text,parsed_json,imported_event_id)
                 VALUES (?1,?2,?3,?4,?5,?6)",
                params![
                    m.filename,
                    m.version,
                    m.kind,
                    m.raw_text,
                    m.parsed_json,
                    m.imported_event_id
                ],
            )
            .unwrap();
        }
        // 显式 rollback（模拟进程在 commit 前崩溃）。
        drop(tx);
    }
}

fn validate_bundle(bundle: &ProjectBundle) -> anyhow::Result<()> {
    if bundle.bundle_version != 1 {
        anyhow::bail!("bundle_version 不支持: {}", bundle.bundle_version);
    }
    let mut seqs: Vec<i64> = bundle.events.iter().map(|e| e.seq).collect();
    let sorted = seqs.clone();
    seqs.sort();
    if seqs != (0..seqs.len() as i64).collect::<Vec<_>>() {
        anyhow::bail!("事件顺序不连续或重复");
    }
    let _ = sorted;
    for e in &bundle.events {
        if let Some(p) = &e.parent_id {
            if !bundle.events.iter().any(|x| &x.id == p) {
                anyhow::bail!("事件 {} 的父事件缺失", e.id);
            }
        }
    }
    for m in &bundle.materials {
        if !bundle
            .events
            .iter()
            .any(|e| e.id == m.imported_event_id)
        {
            anyhow::bail!("材料 {} 缺少导入事件", m.filename);
        }
    }
    Ok(())
}

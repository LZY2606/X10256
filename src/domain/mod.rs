//! Domain core: identity, units, diagnostics, budgets and stable hashing.

pub mod archive;
pub mod connectivity;
pub mod excellon;
pub mod geometry;
pub mod gerber;
pub mod model;
pub mod pipeline;
pub mod stackup;

use serde::{Deserialize, Serialize};

/// A position inside an imported source file. Always 1-based, always tied to
/// the raw material so diagnostics can be traced back after normalization.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SrcPos {
    pub line: usize,
    pub col: usize,
    pub fragment: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Note,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
        }
    }
}

/// Diagnostic attached to the raw material. Unconvertible values stay in place
/// (raw text retained) instead of being replaced by defaults.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Diagnostic {
    pub code: String,
    pub severity: String,
    pub message: String,
    pub material_id: Option<String>,
    pub pos: Option<SrcPos>,
}

impl Diagnostic {
    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Diagnostic {
            code: code.into(),
            severity: Severity::Error.as_str().to_string(),
            message: message.into(),
            material_id: None,
            pos: None,
        }
    }
    pub fn warning(code: impl Into<String>, message: impl Into<String>) -> Self {
        Diagnostic {
            code: code.into(),
            severity: Severity::Warning.as_str().to_string(),
            message: message.into(),
            material_id: None,
            pos: None,
        }
    }
    pub fn note(code: impl Into<String>, message: impl Into<String>) -> Self {
        Diagnostic {
            code: code.into(),
            severity: Severity::Note.as_str().to_string(),
            message: message.into(),
            material_id: None,
            pos: None,
        }
    }
    pub fn at(mut self, pos: SrcPos) -> Self {
        self.pos = Some(pos);
        self
    }
    pub fn for_material(mut self, id: &str) -> Self {
        self.material_id = Some(id.to_string());
        self
    }
}

/// Resource budget report. Budget exhaustion is a first-class outcome, never
/// silently truncated output.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BudgetReport {
    pub budget: String,
    pub used: usize,
    pub limit: usize,
    pub at_object: Option<String>,
}

/// The three required distinct terminal outcomes plus normal completion.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Termination {
    Completed,
    FormatCorrupted { diagnostics: Vec<Diagnostic> },
    SemanticUnresolved { diagnostics: Vec<Diagnostic> },
    BudgetExhausted { report: BudgetReport },
}

impl Termination {
    pub fn label(&self) -> &'static str {
        match self {
            Termination::Completed => "completed",
            Termination::FormatCorrupted { .. } => "format_corrupted",
            Termination::SemanticUnresolved { .. } => "semantic_unresolved",
            Termination::BudgetExhausted { .. } => "budget_exhausted",
        }
    }
    pub fn is_completed(&self) -> bool {
        matches!(self, Termination::Completed)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Unit {
    Inch,
    Mm,
}

impl Unit {
    pub fn as_str(self) -> &'static str {
        match self {
            Unit::Inch => "inch",
            Unit::Mm => "mm",
        }
    }
    pub fn parse(s: &str) -> Option<Unit> {
        match s.trim().to_ascii_lowercase().as_str() {
            "inch" | "in" => Some(Unit::Inch),
            "mm" | "metric" => Some(Unit::Mm),
            _ => None,
        }
    }
    /// Multiplier converting a value in this unit into millimetres.
    pub fn to_mm(self) -> f64 {
        match self {
            Unit::Inch => 25.4,
            Unit::Mm => 1.0,
        }
    }
}

/// FNV-1a 64 bit stable identity hash (no random salt, equivalent input ->
/// equivalent identity across processes and export/import round trips).
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub fn id_hash(prefix: &str, parts: &[&str]) -> String {
    let mut h = 0xcbf29ce484222325u64;
    for p in parts {
        h ^= fnv1a64(p.as_bytes());
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{prefix}_{:012x}", h)
}

pub fn short_id_hash(prefix: &str, text: &str) -> String {
    format!("{prefix}_{:012x}", fnv1a64(text.as_bytes()))
}

/// Injectable clock so replay and tests are deterministic.
pub trait Clock: Send + Sync {
    fn now(&self) -> String;
}

pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        format_unix(secs)
    }
}

#[derive(Default)]
pub struct FixedClock {
    pub stamp: String,
}
impl FixedClock {
    pub fn new(stamp: &str) -> Self {
        FixedClock {
            stamp: stamp.to_string(),
        }
    }
}
impl Clock for FixedClock {
    fn now(&self) -> String {
        self.stamp.clone()
    }
}

/// Minimal UTC formatter (seconds precision) without extra dependencies.
pub fn format_unix(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    let (hour, min, sec) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (y + if m <= 2 { 1 } else { 0 }, m, d)
}

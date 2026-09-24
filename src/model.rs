//! Core domain model: immutable materials, modifiable decisions, events,
//! versioned parameters, and machine-readable diagnostics.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub type Nm = i64;

pub fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex(&h.finalize())
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

pub fn make_id(prefix: &str, parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update([0u8]);
    }
    format!("{}_{}", prefix, &hex(&h.finalize())[..24])
}

/// Three fundamentally different failure outcomes plus generic conflicts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CaseError {
    /// Resource budget (raster cells / ops) exhausted before completion.
    ResourceBudgetExhausted { detail: String, used: u64, budget: u64 },
    /// Input bytes do not conform to the expected format; position attached.
    FormatCorrupt { file: String, line: u32, message: String },
    /// Input parses but cannot be resolved semantically (missing refs,
    /// unconvertible units, contradictory requirements).
    SemanticUnsolvable { diagnostics: Vec<Diagnostic> },
    /// A requested operation conflicts with existing state.
    Conflict { message: String },
    NotFound { what: String },
}

impl std::fmt::Display for CaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CaseError::ResourceBudgetExhausted { detail, used, budget } =>
                write!(f, "resource budget exhausted: {detail} (used {used} of {budget})"),
            CaseError::FormatCorrupt { file, line, message } =>
                write!(f, "format corrupt at {file}:{line}: {message}"),
            CaseError::SemanticUnsolvable { diagnostics } =>
                write!(f, "semantic unsolvable: {} diagnostic(s)", diagnostics.len()),
            CaseError::Conflict { message } => write!(f, "conflict: {message}"),
            CaseError::NotFound { what } => write!(f, "not found: {what}"),
        }
    }
}

impl std::error::Error for CaseError {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Diagnostic {
    pub severity: String, // "error" | "warning"
    pub code: String,     // machine readable, e.g. "unit-unknown"
    pub file: String,
    pub line: u32,
    pub message: String,
}

impl Diagnostic {
    pub fn error(code: &str, file: &str, line: u32, message: &str) -> Self {
        Diagnostic { severity: "error".into(), code: code.into(), file: file.into(), line, message: message.into() }
    }
    pub fn warning(code: &str, file: &str, line: u32, message: &str) -> Self {
        Diagnostic { severity: "warning".into(), code: code.into(), file: file.into(), line, message: message.into() }
    }
}

/// Original coordinate format/unit of a source file, always preserved.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileBasis {
    pub unit: String,          // "mm" | "inch" | "unknown"
    pub coord_format: String,  // e.g. "2.4" (integer.decimal places)
    pub raw_mo_line: String,   // original MO line as found
    pub raw_fs_line: String,   // original FS line as found
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MaterialKind {
    Gerber,
    Excellon,
    Stackup,
    LayerMap,
}

/// Immutable source material. Never mutated after import; revisions create
/// new materials and re-point dependencies via events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Material {
    pub id: String,
    pub kind: MaterialKind,
    pub name: String,
    pub content: String,      // raw bytes preserved verbatim (utf8)
    pub sha256: String,
    pub basis: Option<FileBasis>,
    pub created_seq: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Assignment {
    pub material_id: String,
    pub layer: String, // "L1".."Ln" | "drills" | "stackup"
    pub role: String,  // "copper" | "mask" | "drill" | "stackup"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Params {
    pub tolerance_nm: Nm,   // geometric contact tolerance (versioned)
    pub param_version: i64, // bumped on every parameter revision
    pub max_cells: u64,     // resource budget
}

impl Default for Params {
    fn default() -> Self {
        Params { tolerance_nm: 4_000, param_version: 0, max_cells: 20_000_000 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventPayload {
    ImportMaterial { material_id: String, name: String, kind: MaterialKind },
    AssignLayerRoles { assignments: Vec<Assignment> },
    SetParams { tolerance_nm: Nm, max_cells: u64, param_version: i64 },
    ReviseSource { old_material_id: String, new_material_id: String },
    ReviseLayerRole { material_id: String, layer: String, role: String },
    Decision { action: String, target: String, author: String, note: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub seq: i64,
    pub parents: Vec<String>,
    pub payload: EventPayload,
    pub input_digest: String,
    pub param_version: i64,
    pub created_at: i64,
}

impl Event {
    pub fn kind_str(&self) -> &'static str {
        match &self.payload {
            EventPayload::ImportMaterial { .. } => "import_material",
            EventPayload::AssignLayerRoles { .. } => "assign_layer_roles",
            EventPayload::SetParams { .. } => "set_params",
            EventPayload::ReviseSource { .. } => "revise_source",
            EventPayload::ReviseLayerRole { .. } => "revise_layer_role",
            EventPayload::Decision { .. } => "decision",
        }
    }
}

/// A recorded decision (accepted AND rejected outcomes are both kept).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub event_id: String,
    pub seq: i64,
    pub action: String, // "accept_world" | "reject_world" | "adjust_params"
    pub target: String,
    pub author: String,
    pub note: String,
}

/// State folded from the event log up to a chosen event (checkout).
#[derive(Debug, Clone)]
pub struct CaseState {
    pub materials: Vec<Material>,
    pub assignments: Vec<Assignment>,
    pub params: Params,
    pub decisions: Vec<Decision>,
    pub head_event: String,
    pub head_seq: i64,
    pub event_ids: Vec<String>,
}

impl Default for CaseState {
    fn default() -> Self {
        CaseState {
            materials: vec![],
            assignments: vec![],
            params: Params::default(),
            decisions: vec![],
            head_event: String::new(),
            head_seq: 0,
            event_ids: vec![],
        }
    }
}

impl CaseState {
    pub fn material(&self, id: &str) -> Option<&Material> {
        self.materials.iter().find(|m| m.id == id)
    }
    pub fn assignment_for_layer(&self, layer: &str, role: &str) -> Option<&Assignment> {
        self.assignments.iter().find(|a| a.layer == layer && a.role == role)
    }
    pub fn copper_layers(&self) -> Vec<(&Assignment, &Material)> {
        let mut v: Vec<_> = self.assignments.iter()
            .filter(|a| a.role == "copper")
            .filter_map(|a| self.material(&a.material_id).map(|m| (a, m)))
            .collect();
        v.sort_by(|x, y| x.0.layer.cmp(&y.0.layer));
        v
    }
}

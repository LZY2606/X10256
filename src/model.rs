use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Mm,
    In,
}

impl Unit {
    pub fn as_str(&self) -> &'static str {
        match self {
            Unit::Mm => "mm",
            Unit::In => "in",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZeroMode {
    Leading,
    Trailing,
    ExplicitDecimal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Notation {
    Absolute,
    Incremental,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordinateFormat {
    pub integer_digits: u8,
    pub decimal_digits: u8,
    pub zero_mode: ZeroMode,
    pub notation: Notation,
    pub raw: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Polarity {
    Positive,
    Negative,
}

impl Polarity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Polarity::Positive => "positive",
            Polarity::Negative => "negative",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Gerber,
    Excellon,
    Stackup,
    ManualLayerMapping,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerRole {
    Copper,
    SolderMask,
    Legend,
    Outline,
    Drill,
    Mechanical,
    Document,
}

impl LayerRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            LayerRole::Copper => "copper",
            LayerRole::SolderMask => "solder_mask",
            LayerRole::Legend => "legend",
            LayerRole::Outline => "outline",
            LayerRole::Drill => "drill",
            LayerRole::Mechanical => "mechanical",
            LayerRole::Document => "document",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourcePosition {
    pub file_id: String,
    pub filename: String,
    pub line: usize,
    pub column: usize,
    pub offset: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "severity", rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub id: String,
    pub code: String,
    pub severity: Severity,
    pub message: String,
    pub position: Option<SourcePosition>,
    pub raw_value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawCoordinate {
    pub text: String,
    pub unit: Unit,
    pub basis: String,
    pub format: CoordinateFormat,
    pub normalized_mm: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawPoint {
    pub x: RawCoordinate,
    pub y: RawCoordinate,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PointMm {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApertureShape {
    Circle,
    Rectangle,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Aperture {
    pub id: String,
    pub file_id: String,
    pub code: String,
    pub shape: ApertureShape,
    pub width_mm: f64,
    pub height_mm: f64,
    pub unit: Unit,
    pub raw: String,
    pub position: SourcePosition,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepRepeat {
    pub id: String,
    pub file_id: String,
    pub nx: u32,
    pub ny: u32,
    pub dx_mm: f64,
    pub dy_mm: f64,
    pub unit: Unit,
    pub raw: String,
    pub position: SourcePosition,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrawOperation {
    pub id: String,
    pub file_id: String,
    pub sequence: usize,
    pub command_index: usize,
    pub repeat_index: Option<u32>,
    pub aperture_code: String,
    pub polarity: Polarity,
    pub start: RawPoint,
    pub end: RawPoint,
    pub width_mm: f64,
    pub height_mm: f64,
    pub step_repeat_id: Option<String>,
    pub position: SourcePosition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Plating {
    Plated,
    Unplated,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Drill {
    pub id: String,
    pub file_id: String,
    pub sequence: usize,
    pub tool: String,
    pub diameter_mm: f64,
    pub diameter_unit: Unit,
    pub plating: Plating,
    pub start_layer: String,
    pub end_layer: String,
    pub layer_range_raw: String,
    pub blind_buried: bool,
    pub position: RawPoint,
    pub source_position: SourcePosition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerDeclaration {
    pub name: String,
    pub role: LayerRole,
    pub order: i32,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManualLayerMapping {
    pub filename: String,
    pub layer_name: String,
    pub role: LayerRole,
    pub confidence: String,
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceFile {
    pub id: String,
    pub filename: String,
    pub kind: FileKind,
    pub content: String,
    pub sha256: String,
    pub unit: Option<Unit>,
    pub coordinate_format: Option<CoordinateFormat>,
    pub declared_role: Option<LayerRole>,
    pub version: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialBundle {
    pub files: Vec<SourceFile>,
    pub apertures: Vec<Aperture>,
    pub step_repeats: Vec<StepRepeat>,
    pub operations: Vec<DrawOperation>,
    pub drills: Vec<Drill>,
    pub stackup: Vec<LayerDeclaration>,
    pub manual_mappings: Vec<ManualLayerMapping>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeStatus {
    Complete,
    Ambiguous,
    Conflict,
    ResourceBudgetExhausted,
    FormatCorrupt,
    SemanticallyUnsolvable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleVersion {
    pub id: String,
    pub sequence: u32,
    pub contact_tolerance_mm: f64,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionKind {
    AcceptCandidate,
    RejectCandidate,
    ReviseLayerRole,
    AdoptCounterexample,
    RejectCounterexample,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub id: String,
    pub kind: DecisionKind,
    pub target_id: String,
    pub rationale: String,
    pub conflicts_with: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    ProjectCreated,
    MaterialsImported,
    MaterialsReplaced,
    RuleRevised,
    ComputationRecorded,
    DecisionRecorded,
    CounterexampleEvaluated,
    BranchOpened,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub seq: u32,
    pub parent_id: Option<String>,
    pub kind: EventKind,
    pub logical_clock: String,
    pub rule_version_id: Option<String>,
    pub input_summary: String,
    pub parent_dependencies: Vec<String>,
    pub status: ComputeStatus,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub description: String,
    pub coordinate_basis: String,
}

pub fn stable_id(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    format!("id_{}", hex::encode(hasher.finalize()))
}

pub fn sha256_text(input: &str) -> String {
    hex::encode(Sha256::digest(input.as_bytes()))
}

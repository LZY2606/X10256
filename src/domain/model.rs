use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Mm,
    In,
}

impl Unit {
    pub fn as_code(self) -> &'static str {
        match self {
            Unit::Mm => "MM",
            Unit::In => "IN",
        }
    }

    pub fn to_mm(self, value: f64) -> f64 {
        match self {
            Unit::Mm => value,
            Unit::In => value * 25.4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoordinateBasis {
    Absolute,
    Incremental,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerRole {
    Copper,
    PlatedDrill,
    UnplatedDrill,
    SolderMask,
    Silkscreen,
    Mechanical,
    Unknown,
}

impl LayerRole {
    pub fn as_label(self) -> &'static str {
        match self {
            LayerRole::Copper => "copper",
            LayerRole::PlatedDrill => "plated_drill",
            LayerRole::UnplatedDrill => "unplated_drill",
            LayerRole::Silkscreen => "silkscreen",
            LayerRole::Mechanical => "mechanical",
            LayerRole::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Polarity {
    Dark,
    Clear,
}

impl Polarity {
    pub fn as_code(self) -> &'static str {
        match self {
            Polarity::Dark => "D",
            Polarity::Clear => "C",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Gerber,
    Excellon,
    Stackup,
    LayerMap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisStatus {
    Ready,
    Incomplete,
    Corrupt,
    BudgetExhausted,
    SemanticallyUnsolvable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawCoordinate {
    pub raw: String,
    pub unit: Unit,
    pub basis: CoordinateBasis,
    pub numeric: f64,
    pub mm: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FormatDescriptor {
    pub format: String,
    pub unit: Unit,
    pub coordinate_basis: CoordinateBasis,
    pub coordinate_precision: String,
    pub zero_omission: String,
    pub coordinate_notation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApertureDefinition {
    pub id: String,
    pub shape: String,
    pub parameters_mm: Vec<f64>,
    pub raw_parameter_text: String,
    pub unit: Unit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StepRepeatDefinition {
    pub id: String,
    pub aperture_id: Option<String>,
    pub x_count: i32,
    pub y_count: i32,
    pub dx_mm: f64,
    pub dy_mm: f64,
    pub raw: String,
    pub unit: Unit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawPoint {
    pub x: RawCoordinate,
    pub y: RawCoordinate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Shape {
    pub id: String,
    pub kind: String,
    pub x_mm: f64,
    pub y_mm: f64,
    pub width_mm: f64,
    pub height_mm: f64,
    pub polarity: Polarity,
    pub operation_index: usize,
    pub step_instance: String,
    pub aperture_id: Option<String>,
    pub source_line: usize,
    pub raw: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Drill {
    pub id: String,
    pub x_mm: f64,
    pub y_mm: f64,
    pub diameter_mm: f64,
    pub plated: bool,
    pub from_layer: i32,
    pub to_layer: i32,
    pub source_line: usize,
    pub raw: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Diagnostic {
    pub id: String,
    pub code: String,
    pub severity: Severity,
    pub source_id: String,
    pub line: usize,
    pub message: String,
    pub raw_value: Option<String>,
    pub recoverable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LayerSpec {
    pub order: i32,
    pub name: String,
    pub role: LayerRole,
    pub declared_source: Option<String>,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ManualLayerMapping {
    pub source_id: String,
    pub target_layer: String,
    pub declared_role: LayerRole,
    pub active: bool,
    pub raw: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParsedGerber {
    pub format: FormatDescriptor,
    pub apertures: BTreeMap<String, ApertureDefinition>,
    pub step_repeats: Vec<StepRepeatDefinition>,
    pub shapes: Vec<Shape>,
    pub layer_role_declaration: Option<(LayerRole, String)>,
    pub raw_polarities: Vec<(usize, Polarity, String)>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParsedExcellon {
    pub format: FormatDescriptor,
    pub drills: Vec<Drill>,
    pub layer_role_declaration: Option<(LayerRole, String)>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParsedStackup {
    pub unit: Unit,
    pub layers: Vec<LayerSpec>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParsedLayerMap {
    pub mappings: Vec<ManualLayerMapping>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ParsedSource {
    Gerber(ParsedGerber),
    Excellon(ParsedExcellon),
    Stackup(ParsedStackup),
    LayerMap(ParsedLayerMap),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceMaterial {
    pub id: String,
    pub filename: String,
    pub kind: SourceKind,
    pub content: String,
    pub revision: i32,
    pub supersedes: Option<String>,
    pub immutable: bool,
    pub sha256: String,
    pub imported_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParseEnvelope {
    pub material_id: String,
    pub parsed: ParsedSource,
    pub status: AnalysisStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct RuleParameters {
    pub contact_tolerance_mm: f64,
    pub max_step_instances: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuleVersion {
    pub id: String,
    pub parent_id: Option<String>,
    pub version: i32,
    pub parameters: RuleParameters,
    pub rationale: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InputDigest {
    pub material_ids: Vec<String>,
    pub material_checksums: BTreeMap<String, String>,
    pub decision_event_ids: Vec<String>,
    pub rule_version_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GeometryView {
    pub copper_shapes: Vec<Shape>,
    pub mask_openings: Vec<Shape>,
    pub drills: Vec<Drill>,
    pub layers: Vec<LayerSpec>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContactEvidence {
    pub a: String,
    pub b: String,
    pub distance_mm: f64,
    pub tolerance_mm: f64,
    pub rule_version_id: String,
    pub touching: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConnectivityComponent {
    pub id: String,
    pub member_object_ids: Vec<String>,
    pub layer_path: Vec<i32>,
    pub evidence: Vec<ContactEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConnectivityCandidateSet {
    pub id: String,
    pub selected: bool,
    pub basis: String,
    pub components: Vec<ConnectivityComponent>,
    pub excluded_proximity: Vec<ContactEvidence>,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Contradiction {
    pub id: String,
    pub code: String,
    pub location_object_id: String,
    pub decision_ids: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnalysisResult {
    pub id: String,
    pub event_id: String,
    pub status: AnalysisStatus,
    pub input_digest: InputDigest,
    pub rule_version_id: String,
    pub parent_result_id: Option<String>,
    pub geometry: GeometryView,
    pub candidates: Vec<ConnectivityCandidateSet>,
    pub contradictions: Vec<Contradiction>,
    pub diagnostics: Vec<Diagnostic>,
    pub effective_range: EffectiveRange,
    pub invalid_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EffectiveRange {
    pub x_min_mm: f64,
    pub y_min_mm: f64,
    pub x_max_mm: f64,
    pub y_max_mm: f64,
    pub layer_min: i32,
    pub layer_max: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "decision_type", rename_all = "snake_case")]
pub enum DecisionPayload {
    RoleRevision { source_id: String, layer: String, role: LayerRole },
    ConnectivityChoice { candidate_set_id: String, candidate_id: String, accepted: bool },
    ToleranceRevision { contact_tolerance_mm: f64 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum EventPayload {
    ProjectCreated { name: String },
    MaterialImported { material_id: String },
    MaterialRevised { material_id: String, supersedes: String },
    RuleVersionCreated { rule_version_id: String },
    DecisionMade(DecisionPayload),
    AnalysisRequested,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Event {
    pub id: String,
    pub project_id: String,
    pub seq: i64,
    pub created_at: String,
    pub payload: EventPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoredResult {
    pub event_id: String,
    pub result: AnalysisResult,
}

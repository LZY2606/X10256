use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitKind {
    Mm,
    Inch,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialKind {
    Gerber,
    Excellon,
    Stackup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerRole {
    Copper,
    SolderMask,
    DrillGuide,
    Mechanical,
    Plane,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Polarity {
    Positive,
    Negative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineOutcome {
    Ok,
    ResourceBudgetExhausted,
    FormatCorrupt,
    SemanticallyUnsolvable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionKind {
    ParameterRevision,
    MaterialRevision,
    UnitSwitch,
    AcceptCandidate,
    RejectCandidate,
    MappingDecision,
    RoleRevision,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub line: usize,
    pub column: usize,
    pub source_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub position: Option<Position>,
    pub material_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawPoint {
    pub x: String,
    pub y: String,
    pub format: String,
    pub unit: UnitKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Measure {
    pub raw: String,
    pub unit: UnitKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PointMm {
    pub x: f64,
    pub y: f64,
    pub coordinate_basis: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApertureRecord {
    pub id: String,
    pub shape: String,
    pub width: Measure,
    pub height: Measure,
    pub raw: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepRepeat {
    pub x_count: i64,
    pub y_count: i64,
    pub x_distance: Measure,
    pub y_distance: Measure,
    pub raw: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GerberOperation {
    pub ordinal: usize,
    pub op: String,
    pub polarity: Polarity,
    pub aperture: Option<String>,
    pub from: Option<RawPoint>,
    pub to: Option<RawPoint>,
    pub step_repeat: Option<StepRepeat>,
    pub raw: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GerberDocument {
    pub coordinate_format: String,
    pub unit: UnitKind,
    pub polarity: Polarity,
    pub apertures: BTreeMap<String, ApertureRecord>,
    pub operations: Vec<GerberOperation>,
    pub layer_role_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrillHit {
    pub ordinal: usize,
    pub point: RawPoint,
    pub tool_id: String,
    pub plated: bool,
    pub raw: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrillTool {
    pub id: String,
    pub diameter: Measure,
    pub plated: bool,
    pub first_layer: String,
    pub last_layer: String,
    pub raw: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExcellonDocument {
    pub coordinate_format: String,
    pub unit: UnitKind,
    pub tools: BTreeMap<String, DrillTool>,
    pub hits: Vec<DrillHit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackLayer {
    pub id: String,
    pub role: LayerRole,
    pub role_label: String,
    pub order: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackupDocument {
    pub time_basis: String,
    pub coordinate_basis: String,
    pub layers: Vec<StackLayer>,
    pub manual_mapping: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ParsedDocument {
    Gerber(GerberDocument),
    Excellon(ExcellonDocument),
    Stackup(StackupDocument),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterialRevision {
    pub revision_id: String,
    pub parent_revision_id: Option<String>,
    pub material_id: String,
    pub filename: String,
    pub kind: MaterialKind,
    pub role: String,
    pub raw_content: String,
    pub document: Option<ParsedDocument>,
    pub diagnostics: Vec<Diagnostic>,
    pub created_event_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputRef {
    pub material_id: String,
    pub revision_id: String,
    pub filename: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputSummary {
    pub inputs: Vec<InputRef>,
    pub parameter_version: String,
    pub parameter_values: ParameterValues,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParameterValues {
    pub contact_tolerance_mm: f64,
    pub contact_tolerance: DisplayNumber,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DisplayNumber {
    pub value: f64,
    pub unit: String,
    pub basis: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum EventPayload {
    Bootstrap {},
    ParameterRevision {
        from: ParameterValues,
        to: ParameterValues,
        reason: String,
    },
    MaterialRevision {
        material_id: String,
        filename: String,
        parent_revision_id: Option<String>,
        sha256: String,
    },
    UnitSwitch {
        material_id: String,
        old_revision_id: String,
        new_revision_id: String,
        old_unit: UnitKind,
        new_unit: UnitKind,
        raw_coordinates_retained: bool,
    },
    StepRepeatObserved {
        material_id: String,
        revision_id: String,
        step_id: String,
        expanded_features: Vec<String>,
    },
    NegativePolarityObserved {
        material_id: String,
        revision_id: String,
        operation_ordinal: usize,
        removed_area: String,
    },
    BlindRangeObserved {
        tool_id: String,
        first_layer: String,
        last_layer: String,
        candidate_ids: Vec<String>,
    },
    TangencyObserved {
        candidate_id: String,
        distance: DisplayNumber,
        within_version: String,
    },
    NoNetlistMultiSolution {
        ambiguity_id: String,
        legal_candidate_ids: Vec<String>,
        note: String,
    },
    AcceptCandidate {
        candidate_id: String,
        rationale: String,
    },
    RejectCandidate {
        candidate_id: String,
        rationale: String,
    },
    MappingDecision {
        ambiguity_id: String,
        material_id: String,
        chosen_layer: Option<String>,
        conflicts_with: Vec<String>,
        adopted: bool,
        rationale: String,
    },
    RoleRevision {
        layer_id: String,
        old_role: LayerRole,
        new_role: LayerRole,
        rationale: String,
    },
    SemanticConflict {
        code: String,
        message: String,
        affected_object_ids: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub event_id: String,
    pub seq: i64,
    pub event_type: DecisionKind,
    pub created_unix_ms: u128,
    pub time_basis: String,
    pub payload: EventPayload,
    pub input_summary: InputSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RectMm {
    pub id: String,
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub unit: String,
    pub coordinate_basis: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopperFeature {
    pub id: String,
    pub layer_id: String,
    pub material_id: String,
    pub revision_id: String,
    pub source_operation: String,
    pub polarity: Polarity,
    pub rect: RectMm,
    pub valid_from_event_id: String,
    pub invalid_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HoleFeature {
    pub id: String,
    pub tool_id: String,
    pub material_id: String,
    pub revision_id: String,
    pub source_hit: String,
    pub plated: bool,
    pub first_layer: String,
    pub last_layer: String,
    pub center: PointMm,
    pub diameter: DisplayNumber,
    pub rect: RectMm,
    pub valid_from_event_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaskOpening {
    pub id: String,
    pub layer_id: String,
    pub material_id: String,
    pub revision_id: String,
    pub rect: RectMm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateStatus {
    Pending,
    Accepted,
    Rejected,
    Conflicted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectivityCandidate {
    pub id: String,
    pub status: CandidateStatus,
    pub kind: String,
    pub layer_id: Option<String>,
    pub from_feature_id: String,
    pub to_feature_id: String,
    pub distance: DisplayNumber,
    pub reason: String,
    pub source_event_id: String,
    pub decision_event_id: Option<String>,
    pub valid: bool,
    pub invalid_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ambiguity {
    pub id: String,
    pub kind: String,
    pub object_id: String,
    pub legal_options: Vec<String>,
    pub status: CandidateStatus,
    pub decision_event_id: Option<String>,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentNode {
    pub feature_id: String,
    pub layer_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectedComponent {
    pub id: String,
    pub net_label: Option<String>,
    pub nodes: Vec<ComponentNode>,
    pub accepted_edge_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contradiction {
    pub id: String,
    pub code: String,
    pub message: String,
    pub object_ids: Vec<String>,
    pub event_id: String,
    pub valid: bool,
    pub resolution_event_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DerivedSnapshot {
    pub snapshot_id: String,
    pub event_id: String,
    pub seq: i64,
    pub outcome: EngineOutcome,
    pub diagnostics: Vec<Diagnostic>,
    pub input_summary: InputSummary,
    pub parent_snapshot_id: Option<String>,
    pub copper: Vec<CopperFeature>,
    pub holes: Vec<HoleFeature>,
    pub mask_openings: Vec<MaskOpening>,
    pub candidates: Vec<ConnectivityCandidate>,
    pub ambiguities: Vec<Ambiguity>,
    pub components: Vec<ConnectedComponent>,
    pub contradictions: Vec<Contradiction>,
    pub layers: Vec<StackLayer>,
    pub effective_from_event_id: String,
    pub effective_until_event_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectBundle {
    pub schema_version: String,
    pub project_id: String,
    pub exported_unix_ms: u128,
    pub materials: Vec<MaterialRevision>,
    pub events: Vec<Event>,
    pub snapshots: Vec<DerivedSnapshot>,
}

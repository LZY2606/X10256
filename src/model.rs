use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// 内部统一坐标单位:纳米。坐标基准(原点)在各图层的 datum 字段中声明。
pub type Nm = i64;

pub const NM_PER_MM: f64 = 1_000_000.0;
pub const NM_PER_INCH: f64 = 25_400_000.0;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Polarity {
    Pos,
    Neg,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub enum Shape {
    Rect { x0: Nm, y0: Nm, x1: Nm, y1: Nm },
    Circle { cx: Nm, cy: Nm, r: Nm },
}

impl Shape {
    pub fn bbox(&self) -> (Nm, Nm, Nm, Nm) {
        match *self {
            Shape::Rect { x0, y0, x1, y1 } => (x0, y0, x1, y1),
            Shape::Circle { cx, cy, r } => (cx - r, cy - r, cx + r, cy + r),
        }
    }
    pub fn grow(&self, d: Nm) -> Shape {
        match *self {
            Shape::Rect { x0, y0, x1, y1 } => Shape::Rect {
                x0: x0 - d,
                y0: y0 - d,
                x1: x1 + d,
                y1: y1 + d,
            },
            Shape::Circle { cx, cy, r } => Shape::Circle { cx, cy, r: r + d },
        }
    }
    pub fn translate(&self, dx: Nm, dy: Nm) -> Shape {
        match *self {
            Shape::Rect { x0, y0, x1, y1 } => Shape::Rect {
                x0: x0 + dx,
                y0: y0 + dy,
                x1: x1 + dx,
                y1: y1 + dy,
            },
            Shape::Circle { cx, cy, r } => Shape::Circle {
                cx: cx + dx,
                cy: cy + dy,
                r,
            },
        }
    }
}

/// 解析后的几何基元。原始表示(行号、原始坐标文本、单位)全部保留。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Prim {
    pub id: String,
    pub material_id: String,
    pub shape: Shape,
    pub polarity: Polarity,
    /// 文件内操作顺序,正负极性按此顺序叠加。
    pub order: u32,
    pub line: u32,
    pub raw: String,
    pub unit: String,
    pub datum: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hole {
    pub id: String,
    pub material_id: String,
    pub x: Nm,
    pub y: Nm,
    pub dia: Nm,
    pub tool: String,
    pub line: u32,
    pub raw: String,
    pub unit: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum LayerRole {
    Copper,
    Dielectric,
    SolderMask,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StackLayer {
    pub id: String,
    pub role: LayerRole,
    pub index: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ViaDef {
    pub id: String,
    pub from_layer: String,
    pub to_layer: String,
    pub plated: bool,
}

/// 三种必须区分的失败结果。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum FailureKind {
    /// 资源预算耗尽
    BudgetExhausted,
    /// 格式损坏
    CorruptFormat,
    /// 语义无解
    SemanticUnsolvable,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    pub id: String,
    pub severity: String, // "error" | "warning"
    pub kind: String,
    pub material_id: Option<String>,
    pub line: Option<u32>,
    pub message: String,
}

/// 版本化的计算参数。几何接触容差是版本参数。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Params {
    pub version: i64,
    pub contact_tol_nm: Nm,
    pub ambiguity_band_nm: Nm,
    pub max_prims: usize,
    pub max_solutions: usize,
    pub engine_version: u32,
    pub note: String,
}

pub fn sha_hex(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update(b"\x1f");
    }
    hex(&h.finalize())
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

pub fn short_id(prefix: &str, parts: &[&str]) -> String {
    let h = sha_hex(parts);
    format!("{}-{}", prefix, &h[..12])
}

//! Core domain types: stable identities, units/coordinate bases, raw source
//! preservation and position-bearing diagnostics.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Canonical board geometry is integer nanometres on the board coordinate
/// basis.  Every derived object also remembers the file id / raw text it came
/// from, so the original representation is never lost.
pub type Nm = i64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LengthUnit {
    Mm,
    Inch,
}

impl LengthUnit {
    pub fn as_str(self) -> &'static str {
        match self {
            LengthUnit::Mm => "mm",
            LengthUnit::Inch => "inch",
        }
    }
    /// Convert a value expressed in this unit into integer nanometres.
    pub fn to_nm(self, value: f64) -> Nm {
        let scale = match self {
            LengthUnit::Mm => 1_000_000.0,
            LengthUnit::Inch => 25_400_000.0,
        };
        (value * scale).round() as Nm
    }
}

impl fmt::Display for LengthUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Gerber FS specification / Excellon coordinate format, e.g. 2:3 inches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordFmt {
    pub integer: u8,
    pub decimal: u8,
}

/// A coordinate exactly as written in the source, plus its parsed value in
/// file units.  The text is immutable evidence; the float is derived.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawCoord {
    pub text: String,
    pub file_value: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub struct SrcPos {
    pub line: u32,
 pub col: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diag {
    pub code: String,
    pub severity: DiagSeverity,
    pub file_id: String,
    pub message: String,
    pub position: Option<SrcPos>,
    /// Original source fragment the diagnostic is attached to. Never dropped.
    pub raw: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Gerber,
    Excellon,
    Stackup,
    ManualMap,
}

impl FileKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FileKind::Gerber => "gerber",
            FileKind::Excellon => "excellon",
            FileKind::Stackup => "stackup",
            FileKind::ManualMap => "manual_map",
        }
    }
}

/// One delimited source record with its exact byte span.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawRecord {
    pub index: u32,
    pub start: SrcPos,
    pub end: SrcPos,
    pub text: String,
    pub kind: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Polarity {
    Dark,
    Clear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlotCmd {
    /// D01 - draw/plot segment
    Draw,
    /// D02 - move, no exposure
    Move,
    /// D03 - flash
    Flash,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pt {
    pub x: RawCoord,
    pub y: RawCoord,
}

/// A point in canonical nanometre board coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub struct PtNm {
    pub x: Nm,
    pub y: Nm,
}

impl PtNm {
    pub fn new(x: Nm, y: Nm) -> Self {
        PtNm { x, y }
    }
}

macro_rules! string_id {
    ($name:ident, $prefix:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name(pub String);
        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
            pub fn from_owned(s: String) -> Self {
                $name(s)
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                $name(s.to_string())
            }
        }
        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

string_id!(FileId, "F");
string_id!(EventId, "E");
string_id!(ObjectId, "O");

/// Deterministic hashed id helper (sha256 truncated).
pub fn stable_id(namespace: &str, parts: &[&str]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    Sha256::update(&mut h, namespace.as_bytes());
    for p in parts {
        Sha256::update(&mut h, [0u8]);
        Sha256::update(&mut h, p.as_bytes());
    }
    let digest = h.finalize();
    format!("{}:{}", namespace, &hex::encode(digest)[..16])
}

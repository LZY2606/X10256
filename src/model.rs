use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::geom::Nm;

pub fn sha256hex(data: &str) -> String {
    let mut h = Sha256::new();
    h.update(data.as_bytes());
    h.finalize().iter().map(|b| format!("{:02x}", b)).collect()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diag {
    pub file: String,
    pub line: u32,
    pub col: u32,
    pub code: String,
    pub message: String,
    pub raw: String,
}

impl Diag {
    pub fn new(file: &str, line: u32, col: u32, code: &str, message: &str, raw: &str) -> Diag {
        Diag {
            file: file.to_string(),
            line,
            col,
            code: code.to_string(),
            message: message.to_string(),
            raw: raw.to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Units {
    Mm,
    In,
}

impl Units {
    pub fn to_nm(&self, v: f64) -> Nm {
        match self {
            Units::Mm => (v * 1_000_000.0).round() as Nm,
            Units::In => (v * 25_400_000.0).round() as Nm,
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Units::Mm => "MM",
            Units::In => "IN",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Polarity {
    Dark,
    Clear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Params {
    pub version: u32,
    pub tolerance_nm: Nm,
    pub budget_ops: u64,
    pub adjacency_window_nm: Nm,
}

impl Default for Params {
    fn default() -> Self {
        Params { version: 0, tolerance_nm: 0, budget_ops: 10_000, adjacency_window_nm: 500_000 }
    }
}

//! Versioned geometry/connectivity parameters.
//!
//! The geometric contact tolerance is deliberately a *versioned parameter*,
//! never an invisible constant: different parameter revisions produce a new
//! version, and every computation records which version it used.

use serde::{Deserialize, Serialize};

/// Versioned rule parameters for reconstruction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamsV1 {
    /// Version identifier, e.g. `params/v1#h3k...`.
    pub id: String,
    /// Geometric contact tolerance in millimetres. Two copper boundaries with
    /// a positive gap `<= tolerance` are *tangent-candidate* contacts, not
    /// guaranteed conduction.
    pub contact_tolerance_mm: f64,
    /// If true, boundary-tangent (gap == 0) pairs are accepted as hard
    /// contacts; otherwise they remain candidates.
    pub tangent_connects: bool,
    /// Enumeration cap for solutions made of candidate (soft) contacts.
    pub max_candidate_solutions: usize,
    /// Enumeration cap for candidate contacts before enumeration begins.
    pub max_soft_edges: usize,
    /// Command budget while reconstructing one material (resource exhaustion).
    pub command_budget: u64,
}

impl ParamsV1 {
    pub fn v1_default() -> Self {
        ParamsV1 {
            id: String::new(),
            contact_tolerance_mm: 0.05,
            tangent_connects: false,
            max_candidate_solutions: 64,
            max_soft_edges: 24,
            command_budget: 200_000,
        }
    }
}

/// Deterministic identity for a parameter set, independent of stored row id.
pub fn params_id(p: &ParamsV1) -> String {
    let body = format!(
        "v1:tol={}:tan={}:sol={}:edge={}:budget={}",
        norm(p.contact_tolerance_mm),
        p.tangent_connects,
        p.max_candidate_solutions,
        p.max_soft_edges,
        p.command_budget
    );
    format!("params/{}#{}", 1, short_hash(body.as_bytes()))
}

pub fn with_id(mut p: ParamsV1) -> ParamsV1 {
    p.id = params_id(&p);
    p
}

pub(crate) fn norm(v: f64) -> String {
    if v == v.trunc() {
        format!("{:.1}", v)
    } else {
        let s = format!("{:.6}", v);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Short, deterministic, content-derived identifier (FNV-1a 64 bit, hex).
pub fn short_hash(data: &[u8]) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_b325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = OFFSET;
    for b in data {
        h ^= *b as u64;
        h = h.wrapping_mul(PRIME);
    }
    format!("{:016x}", h)
}

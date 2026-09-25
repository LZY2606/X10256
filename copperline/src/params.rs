//! Versioned rule parameters. The contact tolerance is the headline versioned
//! parameter: changing it creates a new parameter version and a new derived
//! result; old results remain addressable forever.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Params {
    /// Version number, starting at 1.
    pub version: u32,
    /// Geometric contact tolerance in millimetres. `0.0` = exact/tangent only.
    pub touch_tolerance_mm: f64,
    /// Near-miss reporting band (mm); proximity inside it is shown but never
    /// treated as conduction.
    pub proximity_band_mm: f64,
    /// Resource budget.
    pub max_primitives: usize,
    pub max_segments: usize,
    pub max_candidates: usize,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            version: 1,
            touch_tolerance_mm: 0.0,
            proximity_band_mm: 0.15,
            max_primitives: 100_000,
            max_segments: 100_000,
            max_candidates: 64,
        }
    }
}

impl Params {
    pub fn with_version(mut self, version: u32) -> Self {
        self.version = version;
        self
    }
    /// Content hash input so identical parameter sets share an identity.
    pub fn digest_input(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}|{}",
            self.version,
            self.touch_tolerance_mm,
            self.proximity_band_mm,
            self.max_primitives,
            self.max_segments,
            self.max_candidates
        )
    }
}

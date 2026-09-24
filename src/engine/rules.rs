//! Versioned rule parameters.
//!
//! The geometric contact tolerance is a *version parameter*: changing it
//! creates a new parameter version and invalidates derived results, but never
//! overwrites history.

use serde::{Deserialize, Serialize};

pub const TANGENT_CONNECTED: &str = "tangent_connected";
pub const TANGENT_CANDIDATE: &str = "tangent_candidate";
pub const TANGENT_GAP: &str = "tangent_gap";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleVersion {
    pub id: String,
    pub label: String,
    pub note: String,
    pub params: RuleParams,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleParams {
    /// Geometric contact tolerance in nm. Distance <= tolerance is contact.
    pub contact_tolerance_nm: i64,
    /// How exact tangency (d == 0) is treated.
    pub tangent_policy: String,
    /// Raster cell size in nm (<= contact tolerance, positive).
    pub cell_nm: i64,
    /// Maximum raster cells per layer; beyond this computation returns
    /// BudgetExhausted instead of silently truncating.
    pub max_cells: u64,
    /// Maximum number of ambiguity solutions enumerated.
    pub max_candidates: usize,
}

impl Default for RuleParams {
    fn default() -> Self {
        let tol = 2_000;
        Self {
            contact_tolerance_nm: tol,
            tangent_policy: TANGENT_CANDIDATE.to_string(),
            cell_nm: tol / 2,
            max_cells: 16_000_000,
            max_candidates: 8,
        }
    }
}

impl RuleVersion {
    pub fn v1() -> Self {
        Self {
            id: "rules-v1".to_string(),
            label: "v1 · 2 um contact tolerance, tangency = candidate".to_string(),
            note: "Initial process rule set.".to_string(),
            params: RuleParams::default(),
        }
    }

    pub fn v2() -> Self {
        // Parameter revision: 1 um tolerance, exact tangency counts as contact.
        let tol = 1_000;
        Self {
            id: "rules-v2".to_string(),
            label: "v2 · 1 um contact tolerance, tangency = connected".to_string(),
            note: "Parameter revision: tightened tolerance; boundary tangency now connects."
                .to_string(),
            params: RuleParams {
                contact_tolerance_nm: tol,
                tangent_policy: TANGENT_CONNECTED.to_string(),
                cell_nm: tol / 2,
                max_cells: 64_000_000,
                max_candidates: 8,
            },
        }
    }
}

impl RuleParams {
    pub fn validate(&self) -> Option<String> {
        if self.contact_tolerance_nm <= 0 {
            return Some("contact_tolerance_nm must be positive".to_string());
        }
        if self.cell_nm <= 0 || self.cell_nm > self.contact_tolerance_nm {
            return Some("cell_nm must be in (0, contact_tolerance_nm]".to_string());
        }
        if self.max_cells == 0 {
            return Some("max_cells must be positive".to_string());
        }
        if self.max_candidates == 0 {
            return Some("max_candidates must be positive".to_string());
        }
        if !matches!(self.tangent_policy.as_str(), TANGENT_CONNECTED | TANGENT_CANDIDATE | TANGENT_GAP) {
            return Some(format!("unknown tangent_policy '{}'", self.tangent_policy));
        }
        None
    }
}

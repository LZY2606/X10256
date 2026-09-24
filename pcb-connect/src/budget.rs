//! Explicit, testable resource budgets so runaway expansion is a distinct
//! `budget_exceeded` outcome rather than a panic or an empty-but-"ok" result.

use crate::core::{Diag, DiagKind, Loc, Outcome};

#[derive(Clone, Debug)]
pub struct Budget {
    pub name: String,
    pub limit: usize,
    pub used: usize,
}

impl Budget {
    pub fn new(name: &str, limit: usize) -> Self {
        Budget { name: name.to_string(), limit, used: 0 }
    }
    pub fn tick(&mut self, loc: &Loc) -> Outcome<()> {
        self.used += 1;
        if self.used > self.limit {
            let diag = Diag {
                code: "BUDGET_EXCEEDED".to_string(),
                kind: DiagKind::Budget,
                message: format!("resource budget '{}' exhausted: {} > {}", self.name, self.used, self.limit),
                loc: Some(loc.clone()),
                raw_value: None,
            };
            Outcome::BudgetExceeded { budget: self.name.clone(), used: self.used, limit: self.limit, diag }
        } else {
            Outcome::Ok(())
        }
    }
}

/// Default budgets; small on purpose so they are testable.
pub fn step_repeat_budget() -> Budget {
    Budget::new("step_repeat_instances", 5_000)
}
pub fn operation_budget() -> Budget {
    Budget::new("source_operations", 200_000)
}
pub fn line_budget() -> Budget {
    Budget::new("source_lines", 200_000)
}

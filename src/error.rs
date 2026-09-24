//! Three explicitly distinct failure outcomes plus ordinary validation errors.
//!
//! - [`Failure::CorruptFormat`]: bytes/structure cannot be decoded.
//! - [`Failure::SemanticUnresolved`]: input decodes but the domain rules admit no answer
//!   (e.g. value that cannot be converted, conflicting manual decisions).
//! - [`Failure::BudgetExhausted`]: geometry/resource limits configured by the rule
//!   parameter version are exceeded; the input is valid, computation is bounded.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    CorruptFormat,
    SemanticUnresolved,
    BudgetExhausted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    pub kind: FailureKind,
    /// Stable position reference: material id + 1-based line, or field path.
    pub position: String,
    pub code: String,
    pub message: String,
    /// Raw value kept in place when conversion is impossible.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

impl Diagnostic {
    pub fn new(kind: FailureKind, code: &str, position: impl Into<String>, message: impl Into<String>) -> Self {
        Self { kind, position: position.into(), code: code.to_string(), message: message.into(), raw: None }
    }

    pub fn with_raw(mut self, raw: impl Into<String>) -> Self {
        self.raw = Some(raw.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Failure {
    pub kind: FailureKind,
    pub diagnostics: Vec<Diagnostic>,
}

impl Failure {
    pub fn single(kind: FailureKind, code: &str, position: impl Into<String>, message: impl Into<String>) -> Self {
        Self { kind, diagnostics: vec![Diagnostic::new(kind, code, position, message)] }
    }

    pub fn code(&self) -> String {
        self.diagnostics.first().map(|d| d.code.clone()).unwrap_or_else(|| format!("{:?}", self.kind))
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {:?}", self.kind, self.diagnostics)
    }
}

impl std::error::Error for Failure {}

pub type DomainResult<T> = Result<T, Failure>;

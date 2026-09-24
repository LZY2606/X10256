//! Three mutually exclusive failure outcomes required by the domain:
//! resource budget exhausted, malformed format, and semantically undecidable.

use serde::Serialize;

/// Byte/line anchor inside an imported source file.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Position {
    pub line: usize,
    pub column: usize,
    pub byte_offset: usize,
}

impl Position {
    pub fn new(line: usize, column: usize, byte_offset: usize) -> Self {
        Self { line, column, byte_offset }
    }
    pub fn unknown() -> Self {
        Self { line: 0, column: 0, byte_offset: 0 }
    }
}

/// A non-fatal or fatal diagnostic that always carries a source position.
#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub position: Position,
    pub source_id: Option<String>,
    /// Raw source token that could not be converted; never dropped.
    pub raw_value: Option<String>,
}

impl Diagnostic {
    pub fn new(code: impl Into<String>, message: impl Into<String>, position: Position) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            position,
            source_id: None,
            raw_value: None,
        }
    }

    pub fn with_source(mut self, id: impl Into<String>) -> Self {
        self.source_id = Some(id.into());
        self
    }

    pub fn with_raw(mut self, raw: impl Into<String>) -> Self {
        self.raw_value = Some(raw.into());
        self
    }
}

/// Result category for a single import/derivation attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FailureClass {
    /// The input is structurally unreadable (lexer/parser level).
    MalformedFormat,
    /// The structure parses but the declared meaning is contradictory/ambiguous.
    SemanticUndecidable,
    /// A declared resource budget (grid cells, time, candidates) was exceeded.
    BudgetExhausted,
}

#[derive(Debug, Clone, Serialize)]
pub struct EngineError {
    pub class: FailureClass,
    pub message: String,
    pub diagnostics: Vec<Diagnostic>,
}

impl EngineError {
    pub fn malformed(message: impl Into<String>) -> Self {
        Self {
            class: FailureClass::MalformedFormat,
            message: message.into(),
            diagnostics: Vec::new(),
        }
    }

    pub fn semantic(message: impl Into<String>) -> Self {
        Self {
            class: FailureClass::SemanticUndecidable,
            message: message.into(),
            diagnostics: Vec::new(),
        }
    }

    pub fn budget(message: impl Into<String>) -> Self {
        Self {
            class: FailureClass::BudgetExhausted,
            message: message.into(),
            diagnostics: Vec::new(),
        }
    }

    pub fn with_diagnostic(mut self, d: Diagnostic) -> Self {
        self.diagnostics.push(d);
        self
    }

    pub fn with_diagnostics(mut self, ds: Vec<Diagnostic>) -> Self {
        self.diagnostics.extend(ds);
        self
    }
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.class, self.message)
    }
}

impl std::error::Error for EngineError {}

pub type EngineResult<T> = Result<T, EngineError>;

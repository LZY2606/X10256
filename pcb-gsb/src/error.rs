//! Three distinct failure classes required by the domain.
//!
//! * [`GsbError::Corrupt`]   - the source bytes cannot be parsed as the declared format.
//! * [`GsbError::Semantic`]  - parseable input but no valid domain meaning exists.
//! * [`GsbError::Budget`]    - computation would exceed the configured resource budget.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pos {
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub source_material_id: Option<String>,
    pub snippet: Option<String>,
}

impl Pos {
    pub fn line(line: u32) -> Self {
        Pos { line: Some(line), column: None, source_material_id: None, snippet: None }
    }
    pub fn with_material(mut self, id: &str) -> Self {
        self.source_material_id = Some(id.to_string());
        self
    }
    pub fn snippet(mut self, text: &str) -> Self {
        let trimmed = text.trim_end_matches(['\r', '\n']);
        self.snippet = Some(trimmed.chars().take(120).collect());
        self
    }
}

impl fmt::Display for Pos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(m) = &self.source_material_id {
            write!(f, "{m}")?;
        }
        if let Some(l) = self.line {
            write!(f, ":{l}")?;
            if let Some(c) = self.column {
                write!(f, ":{c}")?;
            }
        }
        if let Some(s) = &self.snippet {
            write!(f, " near `{s}`")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GsbError {
    /// Format-level corruption: bytes/commands do not conform to the format grammar.
    Corrupt { message: String, pos: Option<Pos> },
    /// Well-formed input with no semantically valid interpretation.
    Semantic { message: String, pos: Option<Pos> },
    /// Configured resource budget exhausted; result is deliberately not produced.
    Budget { message: String, budget: String, used: u64, limit: u64 },
}

impl fmt::Display for GsbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GsbError::Corrupt { message, pos } => {
                write!(f, "corrupt format: {message}")?;
                if let Some(p) = pos { write!(f, " ({p})")?; }
                Ok(())
            }
            GsbError::Semantic { message, pos } => {
                write!(f, "semantically unsolvable: {message}")?;
                if let Some(p) = pos { write!(f, " ({p})")?; }
                Ok(())
            }
            GsbError::Budget { message, budget, used, limit } => write!(
                f,
                "resource budget exhausted ({budget}): {message} (used {used}/{limit})"
            ),
        }
    }
}

impl std::error::Error for GsbError {}

pub type GsbResult<T> = Result<T, GsbError>;

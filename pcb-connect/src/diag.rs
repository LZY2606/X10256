//! 带位置的诊断。输入不完整 / 不可转换 / 语义无解时，绝不用空值或默认值伪装成功。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeKind {
    /// 结构解析失败：文件损坏、无法识别的命令、括号不闭合等。
    Corrupt,
    /// 结构可解析，但语义无解：引用未定义 aperture、盲孔层范围倒置等。
    Semantic,
    /// 可解析但触发资源预算（网格单元数、候选解数等）。
    Budget,
    /// 完全成功（可能仍有 warning 级别诊断）。
    Ok,
}

impl OutcomeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            OutcomeKind::Corrupt => "corrupt",
            OutcomeKind::Semantic => "semantic",
            OutcomeKind::Budget => "budget",
            OutcomeKind::Ok => "ok",
        }
    }
}

/// 1 起始的位置定位；`line=0` 表示无法归到具体行。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub line: usize,
    pub column: usize,
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub id: String,
    pub severity: Severity,
    pub outcome: OutcomeKind,
    pub code: String,
    pub message: String,
    pub position: Position,
}

impl Diagnostic {
    pub fn new(
        severity: Severity,
        outcome: OutcomeKind,
        code: &str,
        message: impl Into<String>,
        line: usize,
        column: usize,
        snippet: impl Into<String>,
    ) -> Self {
        let message = message.into();
        let id = crate::ids::short_id(
            "diag",
            &[code.into(), message.clone(), line.to_string(), column.to_string(), snippet.into()],
        );
        Diagnostic {
            id,
            severity,
            outcome,
            code: code.to_string(),
            message,
            position: Position { line, column, snippet: snippet.into() },
        }
    }
}

/// 统一的“三种失败 + 成功”容器，业务层与存储层直接复用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome<T> {
    pub kind: OutcomeKind,
    pub value: Option<T>,
    pub diagnostics: Vec<Diagnostic>,
}

impl<T> Outcome<T> {
    pub fn ok(value: T, diagnostics: Vec<Diagnostic>) -> Self {
        Outcome { kind: OutcomeKind::Ok, value: Some(value), diagnostics }
    }
    pub fn fail(kind: OutcomeKind, diagnostics: Vec<Diagnostic>) -> Self {
 Outcome { kind, value: None, diagnostics }
    }
    pub fn is_ok(&self) -> bool {
        self.kind == OutcomeKind::Ok
    }
}

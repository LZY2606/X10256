//! Manual layer map parser.
//!
//! The map is immutable source material: every filename, target layer and
//! role hint is retained exactly. A malformed entry yields a positional
//! diagnostic and is *not* silently dropped or defaulted.

use crate::domain::*;

#[derive(serde::Deserialize)]
struct RawHandMap {
    #[serde(default)]
    mapping: Vec<RawEntry>,
}

#[derive(serde::Deserialize)]
struct RawEntry {
    filename: String,
    layer: String,
    role_hint: Option<String>,
}

pub fn parse(source_id: &str, content: &str) -> HandMapDoc {
    let mut diagnostics = Vec::new();
    let mut entries = Vec::new();

    let raw: RawHandMap = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(e) => {
            // Compute an approximate 1-based line for the error.
            let line = e.line().max(1);
            diagnostics.push(Diagnostic {
                code: "HANDMAP_JSON_CORRUPT".into(),
                severity: Severity::Error,
                line,
                col: e.column(),
                message: format!("manual map JSON parse failed: {}", e),
                raw: content.chars().take(80).collect(),
            });
            return HandMapDoc {
                source_id: source_id.to_string(),
                entries,
                diagnostics,
            };
        }
    };

    for (i, e) in raw.mapping.into_iter().enumerate() {
        if e.filename.is_empty() || e.layer.is_empty() {
            diagnostics.push(Diagnostic {
                code: "HANDMAP_ENTRY_INCOMPLETE".into(),
                severity: Severity::Error,
                line: i + 2,
                col: 1,
                message: "mapping entry requires both filename and layer".into(),
                raw: format!("{:?}", e.filename),
            });
        }
        if let Some(hint) = &e.role_hint {
            if LayerRole::parse(hint).is_none() {
                diagnostics.push(Diagnostic {
                    code: "HANDMAP_ROLE_UNKNOWN".into(),
                    severity: Severity::Warning,
                    line: i + 2,
                    col: 1,
                    message: "role hint not a known layer role; retained verbatim".into(),
                    raw: hint.clone(),
                });
            }
        }
        entries.push(HandMappingEntry {
            filename: e.filename,
            layer: e.layer,
            role_hint: e.role_hint,
        });
    }

    HandMapDoc {
        source_id: source_id.to_string(),
        entries,
        diagnostics,
    }
}

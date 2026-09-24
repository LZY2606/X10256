//! 层叠说明解析：文件 -> 图层角色的手工映射原样保留（含行号）。

use crate::model::*;

pub fn parse_stackup(text: &str, filename: &str) -> ParsedFile {
    let mut entries = Vec::new();
    let mut diags = Vec::new();
    for (idx, raw_line) in text.lines().enumerate() {
        let line_no = (idx + 1) as i64;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(2, ',');
        let file = parts.next().unwrap_or("").trim().to_string();
        let role = parts.next().unwrap_or("").trim().to_string();
        if file.is_empty() {
            diags.push(Diagnostic {
                kind: DiagnosticKind::CorruptFormat,
                file: filename.to_string(),
                line: line_no,
                code: "STACKUP-ROW".to_string(),
                message: "层叠行缺少文件名".to_string(),
                raw_snippet: line.to_string(),
            });
            continue;
        }
        if LayerRole::parse(&role).is_none() {
            diags.push(Diagnostic {
                kind: DiagnosticKind::SemanticUnresolved,
                file: filename.to_string(),
                line: line_no,
                code: "ROLE-UNKNOWN".to_string(),
                message: format!("未知图层角色: {role}"),
                raw_snippet: line.to_string(),
            });
            continue;
        }
        entries.push(StackupEntry {
            file,
            role,
            source_line: line_no,
        });
    }
    ParsedFile {
        kind: FileKind::Stackup.as_str().to_string(),
        gerber: None,
        excellon: None,
        stackup: Some(StackupDoc { entries }),
        diagnostics: diags,
    }
}

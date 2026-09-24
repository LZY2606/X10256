//! Immutable imported material.
//!
//! A [`Material`] never changes: editing a source file creates a new revision
//! with its own id, hash and parsed document. Raw bytes, original coordinate
//! format, unit, polarity, apertures, step/repeat, layer roles and manual
//! mappings are all retained here; derived normalisation lives in the engine.

use crate::excellon::ExcellonDoc;
use crate::gerber::GerberDoc;
use crate::model::Diagnostic;
use crate::stackup::{MappingDoc, StackupDoc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MaterialKind {
    Gerber,
    Excellon,
    Stackup,
    LayerMap,
    Unknown,
}

impl MaterialKind {
    pub fn from_name(name: &str) -> Self {
        let n = name.to_ascii_lowercase();
        if n.ends_with(".map") {
            MaterialKind::LayerMap
        } else if n.ends_with(".stack") || n.ends_with(".stackup") {
            MaterialKind::Stackup
        } else if n.contains("drill") || n.ends_with(".drl") || n.ends_with(".xln") {
            MaterialKind::Excellon
        } else if n.ends_with(".gbr") || n.ends_with(".ger") || n.ends_with(".gtl") || n.ends_with(".gbl")
            || n.ends_with(".gts") || n.ends_with(".gbs") || n.ends_with(".gko") || n.ends_with(".g1")
            || n.ends_with(".g2") || n.ends_with(".g3") || n.ends_with(".g4")
        {
            MaterialKind::Gerber
        } else {
            MaterialKind::Unknown
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ParsedDoc {
    Gerber(GerberDoc),
    Excellon(ExcellonDoc),
    Stackup(StackupDoc),
    LayerMap(MappingDoc),
    /// Bytes were structurally damaged and could not be parsed at all.
    Unparseable { reason: String },
}

impl ParsedDoc {
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        match self {
            ParsedDoc::Gerber(d) => d.diagnostics.clone(),
            ParsedDoc::Excellon(d) => d.diagnostics.clone(),
            ParsedDoc::Stackup(d) => d.diagnostics.clone(),
            ParsedDoc::LayerMap(d) => d.diagnostics.clone(),
            ParsedDoc::Unparseable { .. } => Vec::new(),
        }
    }
}

pub fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex::encode(h.finalize())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Material {
    pub id: String,
    pub file_name: String,
    pub kind: MaterialKind,
    pub revision: String,
    pub supersedes: Option<String>,
    /// Original bytes, always retained (import keeps the original representation).
    pub raw_text: String,
    pub sha256: String,
    pub parsed: ParsedDoc,
    /// Diagnostics produced while parsing (positions anchored to raw_text).
    pub diagnostics: Vec<Diagnostic>,
}

impl Material {
    pub fn reparse(id: String, file_name: &str, raw_text: String, revision: String, supersedes: Option<String>) -> Self {
        let kind = MaterialKind::from_name(file_name);
        let sha = sha256_hex(raw_text.as_bytes());
        let (parsed, diagnostics): (ParsedDoc, Vec<Diagnostic>) = match kind {
            MaterialKind::Gerber => match crate::gerber::parse(&raw_text, &id) {
                Ok(doc) => (ParsedDoc::Gerber(doc.clone()), doc.diagnostics),
                Err((doc, diags)) => (ParsedDoc::Gerber(doc), diags),
            },
            MaterialKind::Excellon => match crate::excellon::parse(&raw_text, &id) {
                Ok(doc) => (ParsedDoc::Excellon(doc.clone()), doc.diagnostics),
                Err((doc, diags)) => (ParsedDoc::Excellon(doc), diags),
            },
            MaterialKind::Stackup => match crate::stackup::parse_stackup(&raw_text, &id) {
                Ok(doc) => (ParsedDoc::Stackup(doc.clone()), doc.diagnostics),
                Err((doc, diags)) => (ParsedDoc::Stackup(doc), diags),
            },
            MaterialKind::LayerMap => match crate::stackup::parse_mapping(&raw_text, &id) {
                Ok(doc) => (ParsedDoc::LayerMap(doc.clone()), doc.diagnostics),
                Err((doc, diags)) => (ParsedDoc::LayerMap(doc), diags),
            },
            MaterialKind::Unknown => (
                ParsedDoc::Unparseable { reason: "unknown file extension".to_string() },
                vec![Diagnostic::err(
                    "IMPORT_UNKNOWN_KIND",
                    "file name does not identify a Gerber/Excellon/stackup/mapping file",
                )],
            ),
        };
        Material {
            id,
            file_name: file_name.to_string(),
            kind,
            revision,
            supersedes,
            raw_text,
            sha256: sha,
            parsed,
            diagnostics,
        }
    }

    pub fn has_parse_errors(&self) -> bool {
        self.diagnostics.iter().any(|d| d.is_error())
    }
}

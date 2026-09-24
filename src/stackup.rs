use crate::gerber::Budget;
use crate::model::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum LayerRole {
    Copper,
    SolderMask,
    Dielectric,
    Paste,
}

impl LayerRole {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "copper" => Some(LayerRole::Copper),
            "solder_mask" => Some(LayerRole::SolderMask),
            "dielectric" => Some(LayerRole::Dielectric),
            "paste" => Some(LayerRole::Paste),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            LayerRole::Copper => "copper",
            LayerRole::SolderMask => "solder_mask",
            LayerRole::Dielectric => "dielectric",
            LayerRole::Paste => "paste",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StackLayer {
    pub id: String,
    pub name: String,
    pub role: LayerRole,
    /// 手工层映射：层 -> 源文件名。映射关系随层叠材料保留。
    pub source: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DrillSpan {
    pub id: String,
    pub source: String,
    /// 盲埋孔层范围：起止层 id（按层叠顺序）。
    pub span: (String, String),
    pub plated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StackupDoc {
    pub board: String,
    pub layers: Vec<StackLayer>,
    pub drills: Vec<DrillSpan>,
    pub manual_mapping_note: Option<String>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StackupFile {
    pub board: String,
    pub layers: Vec<StackLayerFile>,
    #[serde(default)]
    pub drills: Vec<DrillSpanFile>,
    pub manual_mapping_note: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StackLayerFile {
    pub id: String,
    pub name: String,
    pub role: String,
    pub source: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DrillSpanFile {
    pub id: String,
    pub source: String,
    pub span: (String, String),
    #[serde(default = "default_true")]
    pub plated: bool,
}

fn default_true() -> bool {
    true
}

pub enum StackupParse {
    Ok(StackupDoc),
    Corrupt(String),
}

pub fn parse_stackup(content: &str, material_id: &str, budget: &mut Budget) -> Result<StackupParse, FailureKind> {
    budget.spend(1)?;
    let parsed: Result<StackupFile, _> = serde_json::from_str(content);
    let file = match parsed {
        Ok(f) => f,
        Err(e) => {
            // 定位到行列，附诊断而不是吞掉
            return Ok(StackupParse::Corrupt(format!("层叠说明不是合法 JSON：{}（行 {} 列 {}）", e, e.line(), e.column())));
        }
    };
    let mid = Some(material_id);
    let mut diags = Vec::new();
    let mut layers = Vec::new();
    for (i, l) in file.layers.iter().enumerate() {
        budget.spend(1)?;
        match LayerRole::from_str(&l.role) {
            Some(role) => layers.push(StackLayer {
                id: l.id.clone(),
                name: l.name.clone(),
                role,
                source: l.source.clone(),
            }),
            None => diags.push(Diagnostic::error(
                "unknown-role",
                &format!("未知图层角色 '{}'，该层保留但不参与连通计算", l.role),
                mid,
                Some(i + 1),
                None,
            )),
        }
    }
    let drills: Vec<DrillSpan> = file
        .drills
        .iter()
        .map(|d| DrillSpan {
            id: d.id.clone(),
            source: d.source.clone(),
            span: d.span.clone(),
            plated: d.plated,
        })
        .collect();
    Ok(StackupParse::Ok(StackupDoc {
        board: file.board,
        layers,
        drills,
        manual_mapping_note: file.manual_mapping_note,
        diagnostics: diags,
    }))
}

//! Electrical connectivity candidates.
//!
//! Hard rule enforced here: *proximity is never conduction*. Two pieces of
//! copper are only linked when their geometries touch within the versioned
//! contact tolerance (`touch_gap == 0` still allows boundary tangency).
//! Without a netlist the solver emits **candidates** only — every legal
//! alternative for ambiguous links is retained.

use super::build::{CopperRegion, PlatedBarrel, ReconstructedLayer};
use super::polygon::{polygon_distance, Polygon};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    /// Same-layer copper touching (overlap or boundary tangency).
    CopperContact,
    /// Copper meeting within the positive tolerance band (still a real touch
    /// at that tolerance, but it would NOT connect under a smaller version).
    ToleranceBand,
    /// Plated barrel that reaches copper on the layer.
    PlatedBarrel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Link {
    pub id: String,
    pub a: String,
    pub b: String,
    pub layer_a: usize,
    pub layer_b: usize,
    pub kind: LinkKind,
    pub gap_mm: f64,
    /// True when connectivity depends on the current tolerance version.
    pub tolerance_sensitive: bool,
    /// Originating barrel id for barrel links.
    pub barrel: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proximity {
    pub a: String,
    pub b: String,
    pub gap_mm: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Component {
    pub id: String,
    /// Ranked stable index used for display/color.
    pub ordinal: usize,
    pub copper: Vec<String>,
    pub layers: Vec<usize>,
}

/// One legal alternative assignment for ambiguous links, expressed as sets of
/// merged component ordinals. All legal candidates are kept; nothing is picked
/// silently.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateNet {
    pub id: String,
    pub groups: Vec<Vec<usize>>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectivityResult {
    pub components: Vec<Component>,
    pub links: Vec<Link>,
    pub proximity: Vec<Proximity>,
    pub candidates: Vec<CandidateNet>,
    pub touch_tolerance_mm: f64,
    /// Proximity reporting band: gaps larger than the contact tolerance but
    /// within this band are surfaced as near-misses, never as links.
    pub proximity_band_mm: f64,
    pub netlist_present: bool,
}

struct DSU {
    parent: Vec<usize>,
}
impl DSU {
    fn new(n: usize) -> Self {
        DSU { parent: (0..n).collect() }
    }
    fn find(&mut self, x: usize) -> usize {
        if self.parent[x] != x {
            self.parent[x] = self.find(self.parent[x]);
        }
        self.parent[x]
    }
    fn join(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            self.parent[rb] = ra;
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn connectivity(
    recon: &ReconstructedLayer,
    touch_tolerance_mm: f64,
    proximity_band_mm: f64,
    netlist_present: bool,
    max_candidates: usize,
) -> ConnectivityResult {
    let copper = &recon.copper;
    let n = copper.len();
    let mut links: Vec<Link> = Vec::new();
    let mut proximity: Vec<Proximity> = Vec::new();
    let mut dsu = DSU::new(n);
    // Ambiguous links connect only under tolerance (not at 0) and therefore
    // drive the legal-candidate enumeration.
    let mut ambiguous_pairs: Vec<(usize, usize)> = Vec::new();

    // Same layer copper contacts.
    for i in 0..n {
        for j in (i + 1)..n {
            if copper[i].layer_index != copper[j].layer_index {
                continue;
            }
            let gap = poly_gap(&copper[i].polygon, &copper[j].polygon);
            if gap <= touch_tolerance_mm + 1e-9 {
                let sensitive = gap > 1e-9;
                links.push(Link {
                    id: format!("lk_{}_{}", copper[i].id, copper[j].id),
                    a: copper[i].id.clone(),
                    b: copper[j].id.clone(),
                    layer_a: copper[i].layer_index,
                    layer_b: copper[j].layer_index,
                    kind: if sensitive {
                        LinkKind::ToleranceBand
                    } else {
                        LinkKind::CopperContact
                    },
                    gap_mm: round(gap),
                    tolerance_sensitive: sensitive,
                    barrel: None,
                });
                dsu.join(i, j);
                if sensitive {
                    ambiguous_pairs.push((i, j));
                }
            } else if gap <= proximity_band_mm + 1e-9 {
                proximity.push(Proximity {
                    a: copper[i].id.clone(),
                    b: copper[j].id.clone(),
                    gap_mm: round(gap),
                });
            }
        }
    }

    // Plated barrels join copper they reach on each layer within their span.
    for barrel in &recon.barrels {
        if !matches!(barrel.plating, crate::model::Plating::Plated) {
            continue;
        }
        let mut hit_nodes: Vec<(usize, usize)> = Vec::new(); // (copper index, layer)
        for (ci, cop) in copper.iter().enumerate() {
            if cop.layer_index < barrel.span_start || cop.layer_index > barrel.span_end {
                continue;
            }
            let d = point_polygon_distance((barrel.x, barrel.y), &cop.polygon);
            if d <= barrel.radius + touch_tolerance_mm + 1e-9 {
                hit_nodes.push((ci, cop.layer_index));
            } else if d - barrel.radius <= proximity_band_mm + 1e-9 {
                proximity.push(Proximity {
                    a: cop.id.clone(),
                    b: barrel.id.clone(),
                    gap_mm: round((d - barrel.radius).max(0.0)),
                });
            }
        }
        for w in hit_nodes.windows(2) {
            let (a, la) = w[0];
            let (b, lb) = w[1];
            links.push(Link {
                id: format!("lk_{}_{}_{}", barrel.id, copper[a].id, copper[b].id),
                a: copper[a].id.clone(),
                b: copper[b].id.clone(),
                layer_a: la,
                layer_b: lb,
                kind: LinkKind::PlatedBarrel,
                gap_mm: 0.0,
                tolerance_sensitive: false,
                barrel: Some(barrel.id.clone()),
            });
            dsu.join(a, b);
        }
    }

    // Build stable components.
    let mut groups: std::collections::BTreeMap<usize, Vec<usize>> = std::collections::BTreeMap::new();
    for i in 0..n {
        groups.entry(dsu.find(i)).or_default().push(i);
    }
    let mut ordinal = std::collections::HashMap::new();
    let mut components = Vec::new();
    for (k, members) in groups.into_iter().enumerate() {
        for &m in &members.1 {
            ordinal.insert(m, k);
        }
        let mut layers: Vec<usize> = members.1.iter().map(|&i| copper[i].layer_index).collect();
        layers.sort_unstable();
        layers.dedup();
        components.push(Component {
            id: format!("cmp_{:03}", k),
            ordinal: k,
            copper: members.1.iter().map(|&i| copper[i].id.clone()).collect(),
            layers,
        });
    }

    // Legal candidates: each ambiguity independently may or may not connect.
    // Enumerate partitions over the distinct ambiguous component pairs.
    let candidates = if !netlist_present && !ambiguous_pairs.is_empty() {
        enumerate_candidates(&ambiguous_pairs, &ordinal, &components, max_candidates)
    } else {
        Vec::new()
    };

    ConnectivityResult {
        components,
        links,
        proximity,
        candidates,
        touch_tolerance_mm,
        proximity_band_mm,
        netlist_present,
    }
}

fn poly_gap(a: &Polygon, b: &Polygon) -> f64 {
    polygon_distance(a, b)
}

fn enumerate_candidates(
    pairs: &[(usize, usize)],
    ordinal: &std::collections::HashMap<usize, usize>,
    base: &[Component],
    max: usize,
) -> Vec<CandidateNet> {
    // Distinct component-level ambiguous edges.
    let mut edges: Vec<(usize, usize)> = pairs
        .iter()
        .map(|&(a, b)| (ordinal[&a], ordinal[&b]))
        .collect();
    edges.sort_unstable();
    edges.dedup();
    let k = edges.len();
    let mut out = Vec::new();
    let mut mask: usize = 0;
    let limit = 1usize.checked_shl(k as u32).unwrap_or(usize::MAX);
    let limit = limit.min(max as usize + 1);
    while mask < limit && out.len() < max {
        let mut parent: Vec<usize> = (0..base.len()).collect();
        let find = |p: &mut Vec<usize>, mut x: usize| -> usize {
            while p[x] != x {
                p[x] = p[p[x]];
                x = p[x];
            }
            x
        };
        for (i, &(a, b)) in edges.iter().enumerate() {
            if mask & (1 << i) != 0 {
                let ra = find(&mut parent, a);
                let rb = find(&mut parent, b);
                if ra != rb {
                    parent[rb] = ra;
                }
            }
        }
        let mut grp: std::collections::BTreeMap<usize, Vec<usize>> = std::collections::BTreeMap::new();
        for c in 0..base.len() {
            let r = find(&mut parent, c);
            grp.entry(r).or_default().push(c);
        }
        let groups: Vec<Vec<usize>> = grp.into_values().collect();
        out.push(CandidateNet {
            id: format!("cand_{:03}", out.len()),
            groups,
            note: if mask == 0 {
                "all tolerance-band links rejected".into()
            } else if mask == (1usize.checked_shl(k as u32).unwrap_or(0) - 1) {
                "all tolerance-band links accepted".into()
            } else {
                "mixed acceptance of tolerance-band links".into()
            },
        });
        mask += 1;
    }
    out
}

fn point_polygon_distance(p: (f64, f64), poly: &Polygon) -> f64 {
    use super::polygon::polygons_distance;
    let dot = vec![p];
    polygons_distance(&dot, poly)
}

fn round(v: f64) -> f64 {
    (v * 1e6).round() / 1e6
}

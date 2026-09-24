//! Foundational domain model: units, exact integer coordinates, shapes, diagnostics.
//!
//! Internal coordinates are integer decananometres (1 H = 1e-10 m = 1e-7 mm).
//! 1 inch = 254_000_000 H exactly, so both metric and inch inputs convert
//! exactly within the supported digit ranges. No floating point is used in
//! geometry, which keeps boundary-tangency cases deterministic.

use serde::{Deserialize, Serialize};

/// 1 H = 1e-10 metre. 1 mm = 10_000_000 H; 1 inch = 254_000_000 H.
pub const H_PER_MM: i64 = 10_000_000;
pub const H_PER_IN: i64 = 254_000_000;

/// Declared coordinate unit of a source document. Preserved verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Mm,
    Inch,
}

impl Unit {
    pub fn code(self) -> &'static str {
        match self {
            Unit::Mm => "mm",
            Unit::Inch => "inch",
        }
    }
}

/// `%FSLAX...Y...%` coordinate format. Kept alongside the raw token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PosSpec {
    pub digits_int: u8,
    pub digits_frac: u8,
}

/// A coordinate exactly as it appeared in the source plus its normalized form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawCoord {
    /// Raw token without sign, e.g. "01000" or "00010".
    pub token: String,
    pub negative: bool,
}

/// Either an exactly normalized value or an unconvertible value that must stay
/// in place with a diagnostic (never silently dropped to draw a picture).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Coord {
    /// Normalized value in internal decananometres.
    H(i64),
    /// Value that cannot be normalized; `raw` is preserved.
    Unconvertible { raw: String, why: String },
}

impl Coord {
    pub fn h(&self) -> Option<i64> {
        match self {
            Coord::H(v) => Some(*v),
            Coord::Unconvertible { .. } => None,
        }
    }
    pub fn is_bad(&self) -> bool {
        matches!(self, Coord::Unconvertible { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Point {
    pub x: i64,
    pub y: i64,
}

impl Point {
    pub fn new(x: i64, y: i64) -> Self {
        Point { x, y }
    }
}

/// Axis-aligned rectangle and circles/capsules are the supported copper
/// primitives. Rectangles cover flashes and line strokes; capsules cover
/// drawn segments with circular and obround apertures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum Shape {
    Rect {
        x0: i64,
        y0: i64,
        x1: i64,
        y1: i64,
    },
    Circle {
        cx: i64,
        cy: i64,
        r: i64,
    },
    /// Stadium around segment p1-p2 with radius r (p1 != p2).
    Capsule {
        p1: Point,
        p2: Point,
        r: i64,
    },
}

impl Shape {
    pub fn bbox(&self) -> (i64, i64, i64, i64) {
        match self {
            Shape::Rect { x0, y0, x1, y1 } => (*x0, *y0, *x1, *y1),
            Shape::Circle { cx, cy, r } => (cx - r, cy - r, cx + r, cy + r),
            Shape::Capsule { p1, p2, r } => {
                (p1.x.min(p2.x) - r, p1.y.min(p2.y) - r, p1.x.max(p2.x) + r, p1.y.max(p2.y) + r)
            }
        }
    }
}

/// Positional diagnostic carrying the 1-based source line and column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diag {
    pub code: String,
    pub line: usize,
    pub col: usize,
    pub message: String,
    pub raw: String,
}

impl Diag {
    pub fn new(code: impl Into<String>, line: usize, col: usize, message: impl Into<String>, raw: impl Into<String>) -> Self {
        Diag { code: code.into(), line, col, message: message.into(), raw: raw.into() }
    }
}

/// Three strictly different failure outcomes required by the specification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum LoadError {
    /// Structural/format damage: file is not parseable.
    Corrupt {
        file_id: String,
        diags: Vec<Diag>,
    },
    /// Syntax is fine but the semantics cannot be resolved.
    Unresolvable {
        file_id: String,
        diags: Vec<Diag>,
    },
    /// A bounded resource budget (shape count / candidate partitions) was hit.
    BudgetExhausted {
        budget: String,
        limit: usize,
        used: usize,
    },
}

/// Convert a decimal fixed-point value (`int.frac`) to internal H units.
/// Returns None (unconvertible) on out-of-range values rather than wrapping.
pub fn fixed_to_h(int_part: &str, frac_part: Option<&str>, frac_digits: u8, unit: Unit) -> Result<i64, String> {
    let int: i64 = int_part.parse().map_err(|_| format!("integer part '{int_part}' out of range"))?;
    let scale = unit_factor(unit);
    let mut value = int.checked_mul(scale).ok_or_else(|| "coordinate overflow".to_string())?;
    if let Some(frac) = frac_part {
        let digits = frac.len() as u8;
        if digits > frac_digits {
            return Err(format!("{digits} fractional digits exceed format {frac_digits}"));
        }
        let frac_val: i64 = if frac.is_empty() { 0 } else { frac.parse().map_err(|_| "bad fraction")? };
        let place = 10i64.pow(u32::from(frac_digits - digits));
        let per_unit = scale.checked_div(10i64.pow(u32::from(frac_digits))).ok_or("bad format scale")?;
        let add = frac_val
            .checked_mul(place)
            .and_then(|v| v.checked_mul(per_unit))
            .ok_or_else(|| "fraction overflow".to_string())?;
        value = value.checked_add(add).ok_or_else(|| "coordinate overflow".to_string())?;
    }
    Ok(value)
}

fn unit_factor(unit: Unit) -> i64 {
    match unit {
        Unit::Mm => H_PER_MM,
        Unit::Inch => H_PER_IN,
    }
}

/// Parse a raw Gerber/Excellon coordinate token using a declared format.
pub fn coord_from_token(token: &str, negative: bool, spec: PosSpec, unit: Unit) -> Coord {
    let digits: String = token.chars().filter(|c| c.is_ascii_digit()).collect();
    let total = usize::from(spec.digits_int) + usize::from(spec.digits_frac);
    if digits.len() > total {
        return Coord::Unconvertible {
            raw: token.to_string(),
            why: format!("{} digits exceed format X{}Y{}", digits.len(), spec.digits_int, spec.digits_frac),
        };
    }
    let padded = format!("{:0>width$}", digits, width = total);
    let (int_part, frac_part) = padded.split_at(usize::from(spec.digits_int));
    match fixed_to_h(int_part, Some(frac_part), spec.digits_frac, unit) {
        Ok(v) => Coord::H(if negative { -v } else { v }),
        Err(why) => Coord::Unconvertible { raw: token.to_string(), why },
    }
}

/// Metric aperture size string such as "0.3" to H; inch values supported too.
pub fn decimal_size_to_h(text: &str, unit: Unit) -> Result<i64, String> {
    let (neg, body) = match text.strip_prefix('-') {
        Some(b) => (true, b),
        None => (false, text),
    };
    let (int_part, frac_part) = match body.split_once('.') {
        Some((a, b)) => (a, Some(b)),
        None => (body, None),
    };
    let frac_digits = frac_part.map(|f| f.len() as u8).unwrap_or(0);
    let mut v = fixed_to_h(int_part, frac_part, frac_digits, unit).map_err(|e| e)?;
    if neg {
        v = -v;
    }
    Ok(v)
}

// ---------------------------------------------------------------------------
// Exact geometry
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gap {
    /// Strict positive gap; distance carried as squared H to stay integer.
    Disjoint { gap2: i128 },
    /// Boundary touches at >=1 point, interiors disjoint.
    Touch,
    /// Interiors overlap by positive area.
    Overlap,
}

fn dot_i128(ax: i128, ay: i128, bx: i128, by: i128) -> i128 {
    ax * bx + ay * by
}

fn point_seg_dist2(px: i128, py: i128, ax: i128, ay: i128, bx: i128, by: i128) -> i128 {
    let abx = bx - ax;
    let aby = by - ay;
    let apx = px - ax;
    let apy = py - ay;
    let len2 = abx * abx + aby * aby;
    let t = if len2 == 0 { 0 } else { (dot_i128(apx, apy, abx, aby) * 2) / (2 * len2) };
    let t = t.clamp(0, 1);
    let cx = ax + abx * t;
    let cy = ay + aby * t;
    // cx/cy may be fractional (denominator len2). Compute squared distance as
    // a rational and return (numerator, denominator) via integer math:
    let projx_num = ax * len2 + abx * dot_i128(apx, apy, abx, aby).clamp(0, len2);
    let projy_num = ay * len2 + aby * dot_i128(apx, apy, abx, aby).clamp(0, len2);
    let _ = (cx, cy);
    let dx = px * len2 - projx_num;
    let dy = py * len2 - projy_num;
    dx * dx + dy * dy
}

/// Squared distance denominator: the returned value is `distance^2 * len2^2`
/// style rational numerator. We instead provide direct integer comparison via
/// [`seg_dist2_exact`] returning numerator and denominator.
pub fn point_seg_dist2_rat(px: i128, py: i128, ax: i128, ay: i128, bx: i128, by: i128) -> (i128, i128) {
    let abx = bx - ax;
    let aby = by - ay;
    let apx = px - ax;
    let apy = py - ay;
    let len2 = abx * abx + aby * aby;
    if len2 == 0 {
        let dx = px - ax;
        let dy = py - ay;
        return (dx * dx + dy * dy, 1);
    }
    let t_num = dot_i128(apx, apy, abx, aby).clamp(0, len2);
    let projx = ax * len2 + abx * t_num;
    let projy = ay * len2 + aby * t_num;
    let dx = px * len2 - projx;
    let dy = py * len2 - projy;
    (dx * dx + dy * dy, len2 * len2)
}

fn point_rect_gap(px: i128, py: i128, r: &Shape) -> Gap {
    let Shape::Rect { x0, y0, x1, y1 } = r else { unreachable!() };
    let inside = px >= *x0 as i128 && px <= *x1 as i128 && py >= *y0 as i128 && py <= *y1 as i128;
    if inside {
        return Gap::Overlap;
    }
    let cx = px.clamp(*x0 as i128, *x1 as i128);
    let cy = py.clamp(*y0 as i128, *y1 as i128);
    let d2 = (px - cx).pow(2) + (py - cy).pow(2);
    if d2 == 0 {
        Gap::Touch
    } else {
        Gap::Disjoint { gap2: d2 }
    }
}

fn rects_gap(a: &Shape, b: &Shape) -> Gap {
    let (Shape::Rect { x0: ax0, y0: ay0, x1: ax1, y1: ay1 }) = a else { unreachable!() };
    let (Shape::Rect { x0: bx0, y0: by0, x1: bx1, y1: by1 }) = b else { unreachable!() };
    let dx = (ax0 - bx1).max(bx0 - ax1).max(0);
    let dy = (ay0 - by1).max(by0 - ay1).max(0);
    match (dx, dy) {
        (0, 0) => Gap::Overlap,
        _ => Gap::Disjoint { gap2: (dx as i128).pow(2) + (dy as i128).pow(2) },
    }
}

fn circle_circle_gap(a: &Shape, b: &Shape) -> Gap {
    let (Shape::Circle { cx: acx, cy: acy, r: ar }) = a else { unreachable!() };
    let (Shape::Circle { cx: bcx, cy: bcy, r: br }) = b else { unreachable!() };
    let dx = (acx - bcx) as i128;
    let dy = (acy - bcy) as i128;
    let d2 = dx * dx + dy * dy;
    let rr = (*ar + *br) as i128;
    let rr2 = rr * rr;
    if d2 > rr2 {
        Gap::Disjoint { gap2: d2 }
    } else if d2 < rr2 {
        // Centre inside => positive overlap unless fully contained (still overlap).
        Gap::Overlap
    } else {
        Gap::Touch
    }
}

fn point_in_circle(px: i128, py: i128, c: &Shape) -> i128 {
    let Shape::Circle { cx, cy, r } = c else { unreachable!() };
    let dx = px - *cx as i128;
    let dy = py - *cy as i128;
    dx * dx + dy * dy - (*r as i128).pow(2)
}

fn circle_rect_gap(c: &Shape, r: &Shape) -> Gap {
    let Shape::Circle { cx, cy, rad } = c else { unreachable!() };
    let Shape::Rect { x0, y0, x1, y1 } = r else { unreachable!() };
    let inside = *cx >= *x0 && *cx <= *x1 && *cy >= *y0 && *cy <= *y1;
    let near_x = (*cx as i128).clamp(*x0 as i128, *x1 as i128);
    let near_y = (*cy as i128).clamp(*y0 as i128, *y1 as i128);
    let d2 = ((*cx as i128) - near_x).pow(2) + ((*cy as i128) - near_y).pow(2);
    let rr = (*rad as i128).pow(2);
    if inside || d2 < rr {
        Gap::Overlap
    } else if d2 == rr {
        Gap::Touch
    } else {
        Gap::Disjoint { gap2: d2 }
    }
}

fn seg_rect_gap(p1: Point, p2: Point, r: i64, rect: &Shape) -> Gap {
    // Capsule vs rect: shrink rect by r (halfplanes), then test segment vs
    // grown rect. We sample endpoints and check exact clearance rationally.
    let Shape::Rect { x0, y0, x1, y1 } = *rect;
    let mut best = Gap::Disjoint { gap2: i128::MAX };
    // distance from each of 4 rect corners to segment (capsule boundary),
    // plus distance from segment endpoints to rect.
    let seg_pts = [(p1.x, p1.y), (p2.x, p2.y)];
    for (x, y) in seg_pts {
        match point_rect_gap(x as i128, y as i128, rect) {
            Gap::Overlap => return Gap::Overlap,
            Gap::Touch => best = Gap::Touch,
            Gap::Disjoint { gap2 } => best = min_gap(best, Gap::Disjoint { gap2 }),
        }
    }
    for (x, y) in [(x0, y0), (x0, y1), (x1, y0), (x1, y1)] {
        let (num, den) = point_seg_dist2_rat(x as i128, y as i128, p1.x as i128, p1.y as i128, p2.x as i128, p2.y as i128);
        // segment distance vs corner, compare with radius r.
        let rr = (r as i128) * (r as i128) * den;
        if num < rr {
            return Gap::Overlap;
        }
        if num == rr {
            best = Gap::Touch;
        } else {
            // rational gap^2 = num/den - r^2 ; only used for tolerance compare
            best = min_gap(best, Gap::Disjoint { gap2: (num - rr) / den.max(1) });
        }
    }
    // segment may cross rect even if endpoints outside: check axis-aligned
    // segment crossing (all our segments are horizontal/vertical in practice).
    let crosses = {
        let horiz = p1.y == p2.y && p1.y >= y0 && p1.y <= y1 && ((p1.x - x1) * (p2.x - x1) <= 0 || (p1.x - x0) * (p2.x - x0) <= 0);
        let vert = p1.x == p2.x && p1.x >= x0 && p1.x <= x1 && ((p1.y - y1) * (p2.y - y1) <= 0 || (p1.y - y0) * (p2.y - y0) <= 0);
        horiz || vert
    };
    if crosses {
        Gap::Overlap
    } else {
        best
    }
}

fn min_gap(a: Gap, b: Gap) -> Gap {
    match (a, b) {
        (Gap::Overlap, _) | (_, Gap::Overlap) => Gap::Overlap,
        (Gap::Touch, _) | (_, Gap::Touch) => Gap::Touch,
        (Gap::Disjoint { gap2: ga }, Gap::Disjoint { gap2: gb }) => Gap::Disjoint { gap2: ga.min(gb) },
    }
}

fn seg_seg_gap(p1: Point, p2: Point, r1: i64, p3: Point, p4: Point, r2: i64) -> Gap {
    // Two capsules. General segment distance via 4 endpoint-to-segment tests;
    // works exactly for our axis-aligned segments (parallel/perpendicular).
    let r = r1 + r2;
    let mut best = Gap::Disjoint { gap2: i128::MAX };
    let endpoints = [(p3, p4), (p4, p3), (p1, p2), (p2, p1)];
    // distances of endpoints of one segment to the other, with combined radius
    let tests = [
        (p3, p1, p2, r),
        (p4, p1, p2, r),
        (p1, p3, p4, r),
        (p2, p3, p4, r),
    ];
    for (pt, a, b, rad) in tests {
        let (num, den) = point_seg_dist2_rat(pt.x as i128, pt.y as i128, a.x as i128, a.y as i128, b.x as i128, b.y as i128);
        let rr = (rad as i128) * (rad as i128) * den;
        if num < rr {
            return Gap::Overlap;
        }
        if num == rr {
            best = Gap::Touch;
        } else {
            best = min_gap(best, Gap::Disjoint { gap2: (num - rr) / den.max(1) });
        }
    }
    let _ = endpoints;
    // crossing of axis-aligned perpendicular segments
    let cross = {
        let a_horiz = p1.y == p2.y;
        let b_horiz = p3.y == p4.y;
        if a_horiz && !b_horiz {
            let y = p1.y;
            let x = p3.x;
            y >= p3.y.min(p4.y) && y <= p3.y.max(p4.y) && x >= p1.x.min(p2.x) && x <= p1.x.max(p2.x)
        } else if !a_horiz && b_horiz {
            let y = p3.y;
            let x = p1.x;
            y >= p1.y.min(p2.y) && y <= p1.y.max(p2.y) && x >= p3.x.min(p4.x) && x <= p3.x.max(p4.x)
        } else if a_horiz && b_horiz && p1.y == p3.y {
            p1.x.min(p2.x) <= p3.x.max(p4.x) && p3.x.min(p4.x) <= p1.x.max(p2.x)
        } else if !a_horiz && !b_horiz && p1.x == p3.x {
            p1.y.min(p2.y) <= p3.y.max(p4.y) && p3.y.min(p4.y) <= p1.y.max(p2.y)
        } else {
            false
        }
    };
    if cross {
        Gap::Overlap
    } else {
        best
    }
}

/// Exact topological relationship between two copper shapes.
pub fn shape_gap(a: &Shape, b: &Shape) -> Gap {
    match (a, b) {
        (Shape::Rect { .. }, Shape::Rect { .. }) => rects_gap(a, b),
        (Shape::Circle { .. }, Shape::Circle { .. }) => circle_circle_gap(a, b),
        (Shape::Circle { .. }, Shape::Rect { .. }) => circle_rect_gap(a, b),
        (Shape::Rect { .. }, Shape::Circle { .. }) => circle_rect_gap(b, a),
        (Shape::Capsule { p1, p2, r }, Shape::Rect { .. }) => seg_rect_gap(*p1, *p2, *r, b),
        (Shape::Rect { .. }, Shape::Capsule { p1, p2, r }) => seg_rect_gap(*p1, *p2, *r, a),
        (Shape::Capsule { p1, p2, r: ra }, Shape::Circle { cx, cy, r: rb }) => {
            seg_seg_gap(*p1, *p2, *ra, Point::new(*cx, *cy), Point::new(*cx, *cy), *rb)
        }
        (Shape::Circle { cx, cy, r: ra }, Shape::Capsule { p1, p2, r: rb }) => {
            seg_seg_gap(*p1, *p2, *rb, Point::new(*cx, *cy), Point::new(*cx, *cy), *ra)
        }
        (Shape::Capsule { p1: a1, p2: a2, r: ra }, Shape::Capsule { p1: b1, p2: b2, r: rb }) => {
            seg_seg_gap(*a1, *a2, *ra, *b1, *b2, *rb)
        }
    }
}

/// True when shape `inner` is completely contained in `outer` (used by the
/// polarity subtraction engine; only exact full containment removes copper).
pub fn contains(outer: &Shape, inner: &Shape) -> bool {
    fn rect_contains(outer: &Shape, p: Point) -> bool {
        let Shape::Rect { x0, y0, x1, y1 } = outer else { return false };
        p.x >= *x0 && p.x <= *x1 && p.y >= *y0 && p.y <= *y1
    }
    match (outer, inner) {
        (Shape::Rect { .. }, Shape::Rect { .. }) => {
            let (ax0, ay0, ax1, ay1) = outer.bbox();
            let (bx0, by0, bx1, by1) = inner.bbox();
            bx0 >= ax0 && by0 >= ay0 && bx1 <= ax1 && by1 <= ay1
        }
        (Shape::Circle { .. }, Shape::Circle { .. }) => {
            let Shape::Circle { cx: ocx, cy: ocy, r: orad } = outer else { return false };
            let Shape::Circle { cx: icx, cy: icy, r: irad } = inner else { return false };
            let d2 = ((ocx - icx) as i128).pow(2) + ((ocy - icy) as i128).pow(2);
            let diff = (*orad - *irad) as i128;
            d2 <= diff * diff
        }
        (Shape::Rect { .. }, Shape::Circle { cx, cy, r }) => {
            for (dx, dy) in [(-1, -1), (-1, 1), (1, -1), (1, 1)] {
                if !rect_contains(outer, Point::new(cx + dx * r, cy + dy * r)) {
                    return false;
                }
            }
            true
        }
        (Shape::Rect { .. }, Shape::Capsule { p1, p2, r }) => {
            rect_contains(outer, Point::new(p1.x - r, p1.y - r))
                && rect_contains(outer, Point::new(p1.x - r, p1.y + r))
                && rect_contains(outer, Point::new(p1.x + r, p1.y - r))
                && rect_contains(outer, Point::new(p1.x + r, p1.y + r))
                && rect_contains(outer, Point::new(p2.x - r, p2.y - r))
                && rect_contains(outer, Point::new(p2.x - r, p2.y + r))
                && rect_contains(outer, Point::new(p2.x + r, p2.y - r))
                && rect_contains(outer, Point::new(p2.x + r, p2.y + r))
        }
        _ => false,
    }
}

/// Human-readable normalized length in mm, always carrying its unit.
pub fn h_to_mm_string(h: i64) -> String {
    let sign = if h < 0 { "-" } else { "" };
    let v = h.unsigned_abs();
    let whole = v / H_PER_MM as u64;
    let frac = v % H_PER_MM as u64;
    format!("{sign}{whole}.{frac:07} mm")
}

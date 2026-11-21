//! ASCII DXF reader for the declared subset.
//!
//! The reader accepts group-code pairs from an ASCII DXF file and keeps only
//! what the drawing model can represent. It reads the HEADER `$INSUNITS` value,
//! the LAYER table, BLOCKS definitions, and LINE, CIRCLE, ARC, ELLIPSE, POINT,
//! TEXT, LWPOLYLINE and the older POLYLINE/VERTEX/SEQEND sequence, including
//! INSERT references that are expanded into the current space and the ATTRIB
//! values that follow them.
//!
//! ARC and ELLIPSE are read into the analytic entity kinds; rotated or aligned
//! linear, radius, diameter and ordinate DIMENSION references become dimension
//! entities. SPLINE and
//! LEADER become polylines (from fit or control points for a spline), and a 3D
//! POLYLINE is projected onto the XY plane. Bulge arcs inside polylines are
//! flattened to line segments and reported as a loss; entities the reader does
//! not understand are skipped once per type. Nothing is dropped silently.

use std::{collections::BTreeMap, f64::consts::TAU};

use serde::Serialize;

use crate::{
    ServiceError,
    schema::{
        CONTINUOUS_LINETYPE, DimensionStyle, Drawing, Entity, EntityKind, Layer, Linetype, MAX_LINEWEIGHT, OrdinateAxis, TextStyle, Units,
    },
};

/// Maximum INSERT nesting the reader expands before it stops.
const MAX_BLOCK_DEPTH: usize = 8;

/// Angular step used when a curved entity is flattened to a polyline.
const FLATTEN_STEP_DEGREES: f64 = 5.0;

/// Smallest number of segments a flattened curve keeps, so a short arc is still
/// recognisably curved.
const MIN_SEGMENTS: usize = 8;

/// One item the reader approximated, skipped or assumed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Loss {
    /// Stable loss code.
    pub(crate) code: String,
    /// `notice` for an approximation that keeps the geometry, `warning` for
    /// content the drawing does not carry.
    pub(crate) severity: &'static str,
    /// Human-readable detail, including the entity name and a handle or line.
    pub(crate) detail: String,
}

impl Loss {
    fn notice(code: &str, detail: String) -> Self {
        Self { code: code.to_string(), severity: "notice", detail }
    }

    fn warning(code: &str, detail: String) -> Self {
        Self { code: code.to_string(), severity: "warning", detail }
    }
}

#[derive(Debug, Clone)]
struct Pair {
    code: i32,
    value: String,
    /// 1-based line of the group code in the source file, for loss details.
    line: usize,
}

fn invalid(message: impl Into<String>) -> ServiceError {
    ServiceError::new("format/invalid", message)
}

/// Splits ASCII DXF into group-code pairs, keeping the line of each group code.
fn pairs(text: &str) -> Result<Vec<Pair>, ServiceError> {
    let mut lines = text.lines().enumerate();
    let mut out = Vec::new();
    while let Some((index, code_line)) = lines.next() {
        let code = code_line.trim();
        if code.is_empty() {
            continue;
        }
        let code: i32 = code.parse().map_err(|_| invalid(format!("`{code}` is not a DXF group code")))?;
        let Some((_, value)) = lines.next()
        else {
            return Err(invalid("a DXF group code has no value"));
        };
        out.push(Pair { code, value: value.trim_end_matches('\r').to_string(), line: index + 1 });
    }
    Ok(out)
}

fn field(block: &[Pair], code: i32) -> Option<&str> {
    block.iter().find(|pair| pair.code == code).map(|pair| pair.value.as_str())
}

fn number(block: &[Pair], code: i32) -> Option<f64> {
    let value = field(block, code)?.trim().parse::<f64>().ok()?;
    value.is_finite().then_some(value)
}

fn integer(block: &[Pair], code: i32) -> Option<i64> {
    field(block, code)?.trim().parse::<i64>().ok()
}

fn point_at(block: &[Pair], x_code: i32, y_code: i32) -> Option<[f64; 2]> {
    number(block, x_code).zip(number(block, y_code)).map(|(x, y)| [x, y])
}

/// Reads every `x_code`/`y_code` pair in order, keeping finite coordinates.
///
/// Repeated group codes carry one point per run, which is how SPLINE fit and
/// control points, LEADER vertices and other point lists are written.
fn point_run(block: &[Pair], x_code: i32, y_code: i32) -> Vec<[f64; 2]> {
    let mut points = Vec::new();
    let mut pending_x: Option<f64> = None;
    for pair in block {
        if pair.code == x_code {
            pending_x = pair.value.trim().parse::<f64>().ok().filter(|value| value.is_finite());
        }
        else if pair.code == y_code
            && let (Some(x), Ok(y)) = (pending_x.take(), pair.value.trim().parse::<f64>())
            && y.is_finite()
        {
            points.push([x, y]);
        }
    }
    points
}

fn layer_name(block: &[Pair]) -> String {
    field(block, 8).map(str::trim).filter(|name| !name.is_empty()).unwrap_or("0").to_string()
}

/// Resolves a block entity layer: layer `0` inside a block inherits the layer of
/// the INSERT that expanded it, which is how CAD systems compose blocks.
fn resolve_layer(raw: String, inherit: Option<&str>) -> String {
    if raw == "0" { inherit.map(str::to_string).unwrap_or(raw) } else { raw }
}

/// Maps an AutoCAD colour index (ACI) to an approximate `#rrggbb` value.
///
/// Indices 1-9 are exact; 10-249 use the 24-hue wheel at ten brightness steps,
/// and 250-255 are the standard greys. `0` (ByBlock) and `256` (ByLayer) have no
/// colour of their own.
fn aci_color(index: i64) -> Option<String> {
    match index.abs() {
        1 => Some("#ff0000".to_string()),
        2 => Some("#ffff00".to_string()),
        3 => Some("#00ff00".to_string()),
        4 => Some("#00ffff".to_string()),
        5 => Some("#0000ff".to_string()),
        6 => Some("#ff00ff".to_string()),
        7 => Some("#d0d7de".to_string()),
        8 => Some("#808080".to_string()),
        9 => Some("#c0c0c0".to_string()),
        10..=249 => {
            let offset = index.abs() - 10;
            let hue = (offset / 10) as f64 * 15.0;
            let brightness = match offset % 10 {
                0 => 0.5,
                1 => 1.0,
                2 => 0.8,
                3 => 0.6,
                4 => 0.9,
                5 => 0.7,
                6 => 0.95,
                7 => 0.75,
                8 => 0.85,
                _ => 0.65,
            };
            Some(hsv_hex(hue, 1.0, brightness))
        }
        250..=255 => {
            let gray = [0x33, 0x50, 0x69, 0x82, 0x93, 0xaa][(index.abs() - 250) as usize];
            Some(format!("#{gray:02x}{gray:02x}{gray:02x}"))
        }
        _ => None,
    }
}

fn hsv_hex(hue: f64, saturation: f64, value: f64) -> String {
    let chroma = value * saturation;
    let sector = hue.rem_euclid(360.0) / 60.0;
    let x = chroma * (1.0 - (sector % 2.0 - 1.0).abs());
    let (red, green, blue) = match sector as u32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let offset = value - chroma;
    let channel = |channel: f64| ((channel + offset) * 255.0).round().clamp(0.0, 255.0) as u8;
    format!("#{:02x}{:02x}{:02x}", channel(red), channel(green), channel(blue))
}

/// One LAYER table entry with the variables the document models.
struct LayerEntry {
    name: String,
    color: Option<i64>,
    linetype: Option<String>,
    lineweight: Option<i64>,
}

fn parse_layer(block: &[Pair]) -> Option<LayerEntry> {
    let name = field(block, 2)?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    Some(LayerEntry {
        name,
        color: integer(block, 62),
        linetype: field(block, 6).map(str::trim).filter(|name| !name.is_empty()).map(str::to_string),
        lineweight: integer(block, 370),
    })
}

/// Reads one `LTYPE` table entry, keeping its description and dash pattern.
fn parse_linetype(block: &[Pair]) -> Option<Linetype> {
    let name = field(block, 2)?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    let description = field(block, 3).map(str::trim).unwrap_or("").to_string();
    let pattern = block
        .iter()
        .filter(|pair| pair.code == 49)
        .filter_map(|pair| pair.value.trim().parse::<f64>().ok())
        .filter(|dash| dash.is_finite() && *dash >= 0.0)
        .collect::<Vec<_>>();
    Some(Linetype { id: name.clone(), name, description, pattern })
}

/// Reads one `DIMSTYLE` table entry, keeping only the modelled variables.
///
/// Missing or invalid variables fall back to the `standard` defaults, so a
/// malformed entry cannot make the imported drawing invalid.
fn parse_dim_style(block: &[Pair]) -> Option<DimensionStyle> {
    let name = field(block, 2)?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    let positive = |value: Option<f64>, fallback: f64| value.filter(|value| value.is_finite() && *value > 0.0).unwrap_or(fallback);
    let non_negative = |value: Option<f64>, fallback: f64| value.filter(|value| value.is_finite() && *value >= 0.0).unwrap_or(fallback);
    Some(DimensionStyle {
        id: name.clone(),
        name,
        scale: positive(number(block, 40), 1.0),
        arrow_size: positive(number(block, 41), 2.5),
        origin_offset: non_negative(number(block, 42), 0.625),
        extension: non_negative(number(block, 44), 1.25),
        text_height: positive(number(block, 140), 2.5),
        precision: integer(block, 271).map_or(2, |value| value.clamp(0, 8) as u32),
    })
}

/// Reads one `STYLE` table entry, keeping only the modelled variables.
fn parse_text_style(block: &[Pair]) -> Option<TextStyle> {
    let name = field(block, 2)?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    let font = field(block, 3).map(str::trim).filter(|font| !font.is_empty()).unwrap_or("txt").to_string();
    let non_negative = |value: Option<f64>, fallback: f64| value.filter(|value| value.is_finite() && *value >= 0.0).unwrap_or(fallback);
    let oblique = number(block, 50).filter(|value| value.is_finite() && (-85.0..=85.0).contains(value)).unwrap_or(0.0);
    Some(TextStyle {
        id: name.clone(),
        name,
        font,
        height: non_negative(number(block, 40), 0.0),
        width_factor: number(block, 41).filter(|value| value.is_finite() && *value > 0.0).unwrap_or(1.0),
        oblique,
    })
}

fn closed_flag(block: &[Pair]) -> bool {
    integer(block, 70).is_some_and(|flags| flags & 1 == 1)
}

/// Signed distance of `point` from the `from -> to` line, measured along the
/// left-hand normal. A zero-length segment has no direction, so the vertical
/// normal is used; this matches [`crate::schema::offset_points`].
fn signed_offset(from: [f64; 2], to: [f64; 2], point: [f64; 2]) -> f64 {
    let dx = to[0] - from[0];
    let dy = to[1] - from[1];
    let length = (dx * dx + dy * dy).sqrt();
    let normal = if length > f64::EPSILON { [-dy / length, dx / length] } else { [0.0, 1.0] };
    (point[0] - from[0]) * normal[0] + (point[1] - from[1]) * normal[1]
}

#[derive(Debug, Clone, Copy)]
struct Vertex {
    x: f64,
    y: f64,
    bulge: f64,
}

/// Reads ordered LWPOLYLINE vertices with their optional bulge (code 42).
fn polyline_vertices(block: &[Pair]) -> Vec<Vertex> {
    let mut vertices: Vec<Vertex> = Vec::new();
    let mut pending_x: Option<f64> = None;
    for pair in block {
        match pair.code {
            10 => {
                pending_x = pair.value.trim().parse::<f64>().ok().filter(|value| value.is_finite());
            }
            20 => {
                if let (Some(x), Ok(y)) = (pending_x.take(), pair.value.trim().parse::<f64>())
                    && y.is_finite()
                {
                    vertices.push(Vertex { x, y, bulge: 0.0 });
                }
            }
            42 => {
                if let (Some(vertex), Ok(bulge)) = (vertices.last_mut(), pair.value.trim().parse::<f64>())
                    && bulge.is_finite()
                {
                    vertex.bulge = bulge;
                }
            }
            _ => {}
        }
    }
    vertices
}

fn segments_for(degrees: f64) -> usize {
    let wanted = (degrees / FLATTEN_STEP_DEGREES).ceil();
    let wanted = if wanted.is_finite() { wanted.max(0.0) as usize } else { MIN_SEGMENTS };
    wanted.max(MIN_SEGMENTS)
}

/// Appends the interior points of one bulge arc, excluding both endpoints.
fn append_bulge_arc(out: &mut Vec<[f64; 2]>, from: [f64; 2], to: [f64; 2], bulge: f64) {
    let dx = to[0] - from[0];
    let dy = to[1] - from[1];
    let chord = (dx * dx + dy * dy).sqrt();
    if chord < 1e-12 {
        return;
    }
    let sweep = 4.0 * bulge.atan();
    let half = sweep / 2.0;
    if sweep.abs() < 1e-12 || half.sin().abs() < 1e-12 {
        return;
    }
    let height = (chord / 2.0) / half.tan();
    let mid = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
    let center = [mid[0] - dy / chord * height, mid[1] + dx / chord * height];
    let radius = chord / (2.0 * half.sin().abs());
    let start = (from[1] - center[1]).atan2(from[0] - center[0]);
    let segments = segments_for(sweep.abs().to_degrees());
    for step in 1..segments {
        let angle = start + sweep * (step as f64 / segments as f64);
        out.push([center[0] + radius * angle.cos(), center[1] + radius * angle.sin()]);
    }
}

/// Flattens vertices and their bulges into a plain point chain.
fn flatten_vertices(vertices: &[Vertex], closed: bool) -> Vec<[f64; 2]> {
    let count = vertices.len();
    if count == 0 {
        return Vec::new();
    }
    let mut points = Vec::new();
    let segments = if closed { count } else { count - 1 };
    for index in 0..segments {
        let start = vertices[index];
        let end = vertices[(index + 1) % count];
        points.push([start.x, start.y]);
        if start.bulge.abs() > 1e-12 {
            append_bulge_arc(&mut points, [start.x, start.y], [end.x, end.y], start.bulge);
        }
    }
    if !closed {
        let last = vertices[count - 1];
        points.push([last.x, last.y]);
    }
    points
}

/// One HATCH boundary read from the entity body.
struct HatchData {
    points: Vec<[f64; 2]>,
    pattern: String,
    /// Boundary paths that used a bulge arc or were flattened.
    bulges: usize,
    /// Edge types the reader cannot represent; the whole hatch is skipped.
    curved_edges: usize,
}

/// Index of the next group code that starts a new boundary path or ends the
/// path list; used to bound one edge record.
fn hatch_run_end(block: &[Pair], start: usize) -> usize {
    let mut end = start;
    while end < block.len() && !matches!(block[end].code, 72 | 91 | 92 | 97 | 98) {
        end += 1;
    }
    end
}

/// Reads the vertices of a polyline boundary path, including their bulges.
fn hatch_polyline_vertices(block: &[Pair], start: usize) -> Vec<Vertex> {
    let mut vertices: Vec<Vertex> = Vec::new();
    let mut pending_x: Option<f64> = None;
    let mut cursor = start;
    while cursor < block.len() {
        let pair = &block[cursor];
        match pair.code {
            10 => {
                pending_x = pair.value.trim().parse::<f64>().ok().filter(|value| value.is_finite());
            }
            20 => {
                if let (Some(x), Ok(y)) = (pending_x.take(), pair.value.trim().parse::<f64>())
                    && y.is_finite()
                {
                    vertices.push(Vertex { x, y, bulge: 0.0 });
                }
            }
            42 => {
                if let (Some(vertex), Ok(bulge)) = (vertices.last_mut(), pair.value.trim().parse::<f64>())
                    && bulge.is_finite()
                {
                    vertex.bulge = bulge;
                }
            }
            92 | 97 | 98 => break,
            _ => {}
        }
        cursor += 1;
    }
    vertices
}

/// Samples one DXF arc edge into points between its two angles.
fn hatch_arc_edge(edge: &[Pair]) -> Option<Vec<[f64; 2]>> {
    let center = point_at(edge, 10, 20)?;
    let radius = number(edge, 40).filter(|radius| *radius > 0.0)?;
    let start = number(edge, 50).unwrap_or(0.0);
    let end = number(edge, 51).unwrap_or(start + 360.0);
    let counter_clockwise = integer(edge, 73).is_none_or(|flag| flag != 0);
    let segments = segments_for((end - start).abs().rem_euclid(360.0).max(0.0));
    let mut points = Vec::with_capacity(segments + 1);
    for step in 0..=segments {
        let fraction = step as f64 / segments as f64;
        let angle = if counter_clockwise { start + (end - start) * fraction } else { start - (start - end) * fraction };
        points.push([center[0] + radius * angle.to_radians().cos(), center[1] + radius * angle.to_radians().sin()]);
    }
    Some(points)
}

/// Reads the boundary points of the first HATCH path in an entity body.
fn parse_hatch(block: &[Pair]) -> Option<HatchData> {
    let solid = integer(block, 70).is_some_and(|flags| flags & 1 == 1);
    let pattern = field(block, 2).map(str::trim).filter(|name| !name.is_empty()).unwrap_or("SOLID").to_string();
    let start = block.iter().position(|pair| pair.code == 92)?;
    let flags = integer(&block[start..], 92).unwrap_or(1);
    let mut curved_edges = 0usize;
    let mut bulges = 0usize;
    let mut points = if flags & 2 == 0 {
        let vertices = hatch_polyline_vertices(block, start + 1);
        if vertices.iter().any(|vertex| vertex.bulge.abs() > 1e-12) {
            bulges += 1;
        }
        flatten_vertices(&vertices, true)
    }
    else {
        let mut points: Vec<[f64; 2]> = Vec::new();
        let mut cursor = start + 1;
        while cursor < block.len() {
            if matches!(block[cursor].code, 92 | 97 | 98) {
                break;
            }
            if block[cursor].code != 72 {
                cursor += 1;
                continue;
            }
            let edge_type = integer(&block[cursor..], 72).unwrap_or(0);
            let end = hatch_run_end(block, cursor + 1);
            let edge = &block[cursor + 1..end];
            match edge_type {
                1 => {
                    if let (Some(from), Some(to)) = (point_at(edge, 10, 20), point_at(edge, 11, 21)) {
                        points.push(from);
                        points.push(to);
                    }
                }
                2 => {
                    if let Some(sampled) = hatch_arc_edge(edge) {
                        points.extend(sampled);
                    }
                    else {
                        curved_edges += 1;
                    }
                }
                _ => curved_edges += 1,
            }
            cursor = end;
        }
        points
    };

    let mut unique: Vec<[f64; 2]> = Vec::with_capacity(points.len());
    for point in points.drain(..) {
        if unique.last() != Some(&point) {
            unique.push(point);
        }
    }
    while unique.len() > 1 && unique.first() == unique.last() {
        unique.pop();
    }
    Some(HatchData { points: unique, pattern: if solid { "SOLID".to_string() } else { pattern }, bulges, curved_edges })
}

/// Strips the inline MTEXT formatting from one MLEADER context string.
fn mleader_plain_text(raw: &str) -> String {
    let mut cleaned = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\\' {
            match chars.peek().copied() {
                Some('P') => {
                    chars.next();
                    cleaned.push('\n');
                }
                Some('~') => {
                    chars.next();
                    cleaned.push(' ');
                }
                Some('\\') => {
                    chars.next();
                    cleaned.push('\\');
                }
                Some(next) if next.is_ascii_alphabetic() => {
                    for skipped in chars.by_ref() {
                        if skipped == ';' {
                            break;
                        }
                    }
                }
                _ => cleaned.push(character),
            }
        }
        else if character != '{' && character != '}' {
            cleaned.push(character);
        }
    }
    cleaned.trim().to_string()
}

/// One MLEADER read from the entity body.
struct MLeaderData {
    points: Vec<[f64; 2]>,
    lines: Vec<String>,
    /// The entity carried context data but no inline text block.
    text_missing: bool,
}

/// Reads a MLEADER leader path and its inline text lines.
///
/// Context data is a flat list of group pairs split by `304` marker strings.
/// Only the `LEADER{...}` section contributes vertices and only the
/// `TEXT{...}` section contributes text, so the text insertion point is not
/// mistaken for a leader vertex.
fn parse_mleader(block: &[Pair]) -> MLeaderData {
    let mut points: Vec<[f64; 2]> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    let mut pending_x: Option<f64> = None;
    let mut in_leader = false;
    let mut in_text = false;
    let mut saw_context = false;
    let mut saw_text = false;
    for pair in block {
        if pair.code == 304 {
            saw_context = true;
            let raw = pair.value.trim();
            let upper = raw.to_ascii_uppercase();
            if upper.starts_with("LEADER{") {
                in_leader = true;
                in_text = false;
            }
            else if upper.starts_with("TEXT{") {
                in_text = true;
                in_leader = false;
                saw_text = true;
            }
            else if raw == "}" {
                in_leader = false;
                in_text = false;
            }
            else if in_text {
                saw_text = true;
                lines.extend(mleader_plain_text(raw).lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_string));
            }
            continue;
        }
        if !in_leader {
            continue;
        }
        match pair.code {
            10 => {
                pending_x = pair.value.trim().parse::<f64>().ok().filter(|value| value.is_finite());
            }
            20 => {
                if let (Some(x), Ok(y)) = (pending_x.take(), pair.value.trim().parse::<f64>())
                    && y.is_finite()
                {
                    points.push([x, y]);
                }
            }
            _ => {}
        }
    }
    MLeaderData { points, lines, text_missing: saw_context && !saw_text }
}

/// Accumulated placement of a block reference.
#[derive(Debug, Clone, Copy)]
struct Xform {
    tx: f64,
    ty: f64,
    scale_x: f64,
    scale_y: f64,
    rotation: f64,
}

impl Xform {
    fn scale(factor: f64) -> Self {
        Self { tx: 0.0, ty: 0.0, scale_x: factor, scale_y: factor, rotation: 0.0 }
    }

    fn apply(self, point: [f64; 2]) -> [f64; 2] {
        let [x, y] = self.apply_vector(point);
        [self.tx + x, self.ty + y]
    }

    /// Applies scale and rotation without the translation, for direction
    /// vectors such as an ellipse's major axis.
    fn apply_vector(self, vector: [f64; 2]) -> [f64; 2] {
        let x = vector[0] * self.scale_x;
        let y = vector[1] * self.scale_y;
        let (sin, cos) = self.rotation.sin_cos();
        [x * cos - y * sin, x * sin + y * cos]
    }

    /// Average scale, used for scalars such as a circle radius or text height.
    fn factor(self) -> f64 {
        (self.scale_x.abs() + self.scale_y.abs()) / 2.0
    }

    fn compose(self, child: Self) -> Self {
        let origin = self.apply([child.tx, child.ty]);
        Self {
            tx: origin[0],
            ty: origin[1],
            scale_x: self.scale_x * child.scale_x,
            scale_y: self.scale_y * child.scale_y,
            rotation: self.rotation + child.rotation,
        }
    }

    /// Builds the transform of one INSERT: `insertion + R * S * (point - base)`.
    fn insert(insertion: [f64; 2], scale_x: f64, scale_y: f64, rotation: f64, base: [f64; 2]) -> Self {
        let (sin, cos) = rotation.sin_cos();
        let base_x = base[0] * scale_x;
        let base_y = base[1] * scale_y;
        let offset = [base_x * cos - base_y * sin, base_x * sin + base_y * cos];
        Self { tx: insertion[0] - offset[0], ty: insertion[1] - offset[1], scale_x, scale_y, rotation }
    }
}

#[derive(Debug, Clone)]
struct BlockDef {
    base: [f64; 2],
    pairs: Vec<Pair>,
}

struct Skipped {
    count: usize,
    line: usize,
    handle: Option<String>,
}

struct Reader {
    drawing: Drawing,
    losses: Vec<Loss>,
    implicit_layers: BTreeMap<String, usize>,
    skipped: BTreeMap<String, Skipped>,
    missing_blocks: BTreeMap<String, usize>,
    unsupported_colors: usize,
    expanded_blocks: usize,
    block_recursions: usize,
    dimensions_without_type: usize,
    dimensions_without_definition: usize,
    attribute_values: usize,
    empty_attributes: usize,
    spline_fit_points: usize,
    spline_control_polygons: usize,
    leaders: usize,
    projected_polylines: usize,
    hatches: usize,
    hatch_bulges: usize,
    mleaders: usize,
    mleader_text_lines: usize,
    mleader_missing_text: usize,
}

impl Reader {
    fn new(units: Units, title: Option<String>) -> Self {
        Self {
            drawing: Drawing::empty(title, units),
            losses: Vec::new(),
            implicit_layers: BTreeMap::new(),
            skipped: BTreeMap::new(),
            missing_blocks: BTreeMap::new(),
            unsupported_colors: 0,
            expanded_blocks: 0,
            block_recursions: 0,
            dimensions_without_type: 0,
            dimensions_without_definition: 0,
            attribute_values: 0,
            empty_attributes: 0,
            spline_fit_points: 0,
            spline_control_polygons: 0,
            leaders: 0,
            projected_polylines: 0,
            hatches: 0,
            hatch_bulges: 0,
            mleaders: 0,
            mleader_text_lines: 0,
            mleader_missing_text: 0,
        }
    }

    fn ensure_layer(&mut self, name: &str) {
        if self.drawing.layer(name).is_none() {
            self.drawing.layers.push(Layer {
                id: name.to_string(),
                name: name.to_string(),
                color: "#d0d7de".to_string(),
                visible: true,
                locked: false,
                linetype: CONTINUOUS_LINETYPE.to_string(),
                lineweight: 0.0,
            });
            *self.implicit_layers.entry(name.to_string()).or_insert(0) += 1;
        }
    }

    fn push(&mut self, layer: String, kind: EntityKind) {
        self.ensure_layer(&layer);
        let id = self.drawing.take_entity_id();
        self.drawing.entities.push(Entity { id, layer, kind });
    }

    /// Records one skipped entity type; only the first occurrence keeps a
    /// handle and line so the report does not grow per entity.
    fn skip(&mut self, kind: &str, block: &[Pair]) {
        if !self.skipped.contains_key(kind) {
            let handle = field(block, 5).map(str::trim).filter(|handle| !handle.is_empty()).map(str::to_string);
            let line = block.first().map(|pair| pair.line).unwrap_or(0);
            self.skipped.insert(kind.to_string(), Skipped { count: 0, line, handle });
        }
        if let Some(entry) = self.skipped.get_mut(kind) {
            entry.count += 1;
        }
    }

    fn apply_layer_table_entry(&mut self, entry: LayerEntry) {
        let color = match entry.color {
            Some(index) => match aci_color(index) {
                Some(value) => value,
                None => {
                    self.unsupported_colors += 1;
                    "#d0d7de".to_string()
                }
            },
            None => "#d0d7de".to_string(),
        };
        let linetype = entry.linetype.unwrap_or_else(|| CONTINUOUS_LINETYPE.to_string());
        if self.drawing.linetypes.iter().all(|existing| existing.id != linetype) {
            self.drawing.linetypes.push(Linetype {
                id: linetype.clone(),
                name: linetype.clone(),
                description: String::new(),
                pattern: Vec::new(),
            });
        }
        let lineweight = entry.lineweight.map_or(0.0, |value| (value as f64 / 100.0).clamp(0.0, MAX_LINEWEIGHT));
        if let Some(layer) = self.drawing.layer_mut(&entry.name) {
            layer.color = color;
            layer.linetype = linetype;
            layer.lineweight = lineweight;
        }
        else {
            self.drawing.layers.push(Layer {
                id: entry.name.clone(),
                name: entry.name,
                color,
                visible: true,
                locked: false,
                linetype,
                lineweight,
            });
        }
    }

    fn parse_entities(&mut self, blocks: &BTreeMap<String, BlockDef>, pairs: &[Pair], xform: Xform, inherit: Option<&str>, depth: usize) {
        let mut index = 0;
        // Layer inherited by the ATTRIB values that follow an INSERT.
        let mut attribute_layer: Option<String> = None;
        while index < pairs.len() {
            if pairs[index].code != 0 {
                index += 1;
                continue;
            }
            let kind = pairs[index].value.trim().to_ascii_uppercase();
            if kind != "ATTRIB" && kind != "SEQEND" {
                attribute_layer = None;
            }
            let end = block_end(pairs, index + 1);
            let block = &pairs[index + 1..end];
            let layer = resolve_layer(layer_name(block), inherit);
            match kind.as_str() {
                "LINE" => {
                    let (Some(from), Some(to)) = (point_at(block, 10, 20), point_at(block, 11, 21))
                    else {
                        self.skip("LINE (missing coordinates)", block);
                        index = end;
                        continue;
                    };
                    self.push(layer, EntityKind::Line { from: xform.apply(from), to: xform.apply(to) });
                }
                "CIRCLE" => {
                    let (Some(center), Some(radius)) = (point_at(block, 10, 20), number(block, 40).filter(|radius| *radius > 0.0))
                    else {
                        self.skip("CIRCLE (missing centre or radius)", block);
                        index = end;
                        continue;
                    };
                    self.push(layer, EntityKind::Circle { center: xform.apply(center), radius: radius * xform.factor() });
                }
                "ARC" => {
                    let (Some(center), Some(radius)) = (point_at(block, 10, 20), number(block, 40).filter(|radius| *radius > 0.0))
                    else {
                        self.skip("ARC (missing centre or radius)", block);
                        index = end;
                        continue;
                    };
                    let start_angle = number(block, 50).unwrap_or(0.0) + xform.rotation.to_degrees();
                    let end_angle = number(block, 51).unwrap_or(360.0) + xform.rotation.to_degrees();
                    self.push(layer, EntityKind::Arc { center: xform.apply(center), radius: radius * xform.factor(), start_angle, end_angle });
                }
                "ELLIPSE" => {
                    let Some(center) = point_at(block, 10, 20)
                    else {
                        self.skip("ELLIPSE (missing centre)", block);
                        index = end;
                        continue;
                    };
                    let major = [number(block, 11).unwrap_or(1.0), number(block, 21).unwrap_or(0.0)];
                    let major = xform.apply_vector(major);
                    let rx = (major[0] * major[0] + major[1] * major[1]).sqrt();
                    if rx < 1e-12 {
                        self.skip("ELLIPSE (missing major axis)", block);
                        index = end;
                        continue;
                    }
                    let ratio = number(block, 40).unwrap_or(1.0).abs().clamp(1e-6, 1.0);
                    let start_angle = number(block, 41).unwrap_or(0.0).to_degrees();
                    let end_angle = number(block, 42).unwrap_or(TAU).to_degrees();
                    self.push(
                        layer,
                        EntityKind::Ellipse {
                            center: xform.apply(center),
                            rx,
                            ry: rx * ratio,
                            rotation: major[1].atan2(major[0]).to_degrees(),
                            start_angle,
                            end_angle,
                        },
                    );
                }
                "POINT" => {
                    let Some(at) = point_at(block, 10, 20)
                    else {
                        self.skip("POINT (missing position)", block);
                        index = end;
                        continue;
                    };
                    self.push(layer, EntityKind::Point { at: xform.apply(at) });
                }
                "SPLINE" => {
                    let mut points = point_run(block, 11, 21);
                    let from_fit_points = points.len() >= 2;
                    if !from_fit_points {
                        points = point_run(block, 10, 20);
                    }
                    if points.len() < 2 {
                        self.skip("SPLINE (fewer than two points)", block);
                        index = end;
                        continue;
                    }
                    if from_fit_points {
                        self.spline_fit_points += 1;
                    }
                    else {
                        self.spline_control_polygons += 1;
                    }
                    let closed = integer(block, 70).is_some_and(|flags| flags & 1 == 1);
                    let points = points.into_iter().map(|point| xform.apply(point)).collect();
                    self.push(layer, EntityKind::Polyline { points, closed });
                }
                "LEADER" => {
                    let points = point_run(block, 10, 20);
                    if points.len() < 2 {
                        self.skip("LEADER (fewer than two vertices)", block);
                        index = end;
                        continue;
                    }
                    self.leaders += 1;
                    let points = points.into_iter().map(|point| xform.apply(point)).collect();
                    self.push(layer, EntityKind::Polyline { points, closed: false });
                }
                "MLEADER" => {
                    let MLeaderData { points, lines, text_missing } = parse_mleader(block);
                    if points.len() < 2 {
                        self.skip("MLEADER (fewer than two leader vertices)", block);
                        index = end;
                        continue;
                    }
                    self.mleaders += 1;
                    let height = number(block, 40).filter(|height| *height > 0.0).unwrap_or(2.5) * xform.factor();
                    let anchor = xform.apply(points[0]);
                    let points = points.into_iter().map(|point| xform.apply(point)).collect();
                    self.push(layer.clone(), EntityKind::Polyline { points, closed: false });
                    if lines.is_empty() && text_missing {
                        self.mleader_missing_text += 1;
                    }
                    let mut emitted = 0usize;
                    for line in lines.iter().map(|line| line.trim()).filter(|line| !line.is_empty()) {
                        let at = [anchor[0], anchor[1] - height * 1.4 * emitted as f64];
                        self.push(layer.clone(), EntityKind::Text { at, height, content: line.to_string() });
                        emitted += 1;
                    }
                    self.mleader_text_lines += emitted;
                }
                "TEXT" => {
                    let Some(at) = point_at(block, 10, 20)
                    else {
                        self.skip("TEXT (missing anchor)", block);
                        index = end;
                        continue;
                    };
                    let height = number(block, 40).unwrap_or(2.5) * xform.factor();
                    let content = field(block, 1).unwrap_or("").trim_end_matches('\r').to_string();
                    self.push(layer, EntityKind::Text { at: xform.apply(at), height, content });
                }
                "LWPOLYLINE" => {
                    let vertices = polyline_vertices(block);
                    let closed = closed_flag(block);
                    let points = flatten_vertices(&vertices, closed).into_iter().map(|point| xform.apply(point)).collect();
                    self.push(layer, EntityKind::Polyline { points, closed });
                }
                "POLYLINE" => {
                    let closed = closed_flag(block);
                    let is_3d = integer(block, 70).is_some_and(|flags| flags & 8 != 0);
                    let mut projected = is_3d;
                    let mut vertices = Vec::new();
                    let mut cursor = end;
                    while cursor < pairs.len() {
                        if pairs[cursor].code != 0 {
                            cursor += 1;
                            continue;
                        }
                        let entry = pairs[cursor].value.trim().to_ascii_uppercase();
                        if entry == "VERTEX" {
                            let vertex_end = block_end(pairs, cursor + 1);
                            let vertex = &pairs[cursor + 1..vertex_end];
                            if let Some(point) = point_at(vertex, 10, 20) {
                                projected |= number(vertex, 30).is_some_and(|z| z.abs() > 1e-9);
                                vertices.push(Vertex { x: point[0], y: point[1], bulge: number(vertex, 42).unwrap_or(0.0) });
                            }
                            cursor = vertex_end;
                            continue;
                        }
                        if entry == "SEQEND" {
                            cursor = block_end(pairs, cursor + 1);
                        }
                        break;
                    }
                    if projected {
                        self.projected_polylines += 1;
                    }
                    let points = flatten_vertices(&vertices, closed).into_iter().map(|point| xform.apply(point)).collect();
                    self.push(layer, EntityKind::Polyline { points, closed });
                    index = cursor.max(end);
                    continue;
                }
                "HATCH" => match parse_hatch(block) {
                    Some(data) if data.curved_edges > 0 => self.skip("HATCH (curved boundary edge)", block),
                    Some(data) if data.points.len() >= 3 => {
                        self.hatches += 1;
                        self.hatch_bulges += data.bulges;
                        let points = data.points.into_iter().map(|point| xform.apply(point)).collect();
                        self.push(layer, EntityKind::Hatch { points, pattern: data.pattern });
                    }
                    Some(_) => self.skip("HATCH (fewer than three boundary points)", block),
                    None => self.skip("HATCH (no boundary path)", block),
                },
                "DIMENSION" => {
                    let dimension_type = match integer(block, 70) {
                        Some(flags) => flags & 0x07,
                        None => {
                            self.dimensions_without_type += 1;
                            1
                        }
                    };
                    let text = field(block, 1).map(str::trim).filter(|text| !text.is_empty() && *text != "<>").map(str::to_string);
                    match dimension_type {
                        // Rotated (0) and aligned (1) dimensions measure the same
                        // segment. A rotated dimension is only representable when
                        // its rotation follows the measured segment, because the
                        // model offsets along that segment's normal.
                        0 | 1 => {
                            let (Some(from), Some(to)) = (point_at(block, 13, 23), point_at(block, 14, 24))
                            else {
                                self.skip("DIMENSION (missing extension line origins)", block);
                                index = end;
                                continue;
                            };
                            let (from, to) = (xform.apply(from), xform.apply(to));
                            if dimension_type == 0 {
                                let rotation = number(block, 50).unwrap_or(0.0) + xform.rotation.to_degrees();
                                let direction = (to[1] - from[1]).atan2(to[0] - from[0]).to_degrees();
                                let delta = (rotation - direction).rem_euclid(180.0);
                                if delta > 1e-6 && (180.0 - delta) > 1e-6 {
                                    self.skip("DIMENSION (rotated away from its measurement)", block);
                                    index = end;
                                    continue;
                                }
                            }
                            let offset = match point_at(block, 10, 20) {
                                Some(definition) => signed_offset(from, to, xform.apply(definition)),
                                None => {
                                    self.dimensions_without_definition += 1;
                                    0.0
                                }
                            };
                            self.push(layer, EntityKind::Dimension { from, to, offset, text });
                        }
                        // Diameter (3) and radius (4) references carry the centre
                        // in 15/25 and a point on the circle in 10/20.
                        3 | 4 => {
                            let (Some(center), Some(target)) = (point_at(block, 15, 25), point_at(block, 10, 20))
                            else {
                                self.skip("DIMENSION (missing centre or circle point)", block);
                                index = end;
                                continue;
                            };
                            let center = xform.apply(center);
                            let target = xform.apply(target);
                            let radius = ((target[0] - center[0]).powi(2) + (target[1] - center[1]).powi(2)).sqrt();
                            if radius < 1e-12 {
                                self.skip("DIMENSION (zero radius)", block);
                                index = end;
                                continue;
                            }
                            let angle = (target[1] - center[1]).atan2(target[0] - center[0]).to_degrees();
                            let offset = number(block, 40).unwrap_or(0.0) * xform.factor();
                            self.push(
                                layer,
                                EntityKind::RadialDimension { center, radius, angle, offset, diameter: dimension_type == 3, text },
                            );
                        }
                        // Ordinate (6) references carry the datum in 13/23, the
                        // measured feature in 14/24 and the dimension-line point in
                        // 10/20. Bit 64 of the type flags selects the X axis.
                        6 => {
                            let (Some(origin), Some(feature)) = (point_at(block, 13, 23), point_at(block, 14, 24))
                            else {
                                self.skip("DIMENSION (missing ordinate datum or feature)", block);
                                index = end;
                                continue;
                            };
                            let axis = if integer(block, 70).is_some_and(|flags| flags & 64 != 0) { OrdinateAxis::X } else { OrdinateAxis::Y };
                            let origin = xform.apply(origin);
                            let feature = xform.apply(feature);
                            let offset = match point_at(block, 10, 20) {
                                Some(definition) => {
                                    let definition = xform.apply(definition);
                                    match axis {
                                        OrdinateAxis::X => definition[1] - origin[1],
                                        OrdinateAxis::Y => definition[0] - origin[0],
                                    }
                                }
                                None => 0.0,
                            };
                            self.push(layer, EntityKind::OrdinateDimension { origin, feature, axis, offset, text });
                        }
                        other => {
                            let label = match other {
                                2 => "DIMENSION (angular)",
                                5 => "DIMENSION (3-point angular)",
                                _ => "DIMENSION (unknown type)",
                            };
                            self.skip(label, block);
                        }
                    }
                }
                "ATTRIB" => {
                    let content = field(block, 1).unwrap_or("").trim_end_matches('\r').to_string();
                    if content.trim().is_empty() {
                        self.empty_attributes += 1;
                    }
                    else {
                        let Some(at) = point_at(block, 10, 20)
                        else {
                            self.skip("ATTRIB (missing position)", block);
                            index = end;
                            continue;
                        };
                        let height = number(block, 40).unwrap_or(2.5) * xform.factor();
                        let raw = layer_name(block);
                        let layer = if raw == "0" { attribute_layer.clone().unwrap_or(raw) } else { raw };
                        self.attribute_values += 1;
                        self.push(layer, EntityKind::Text { at: xform.apply(at), height, content });
                    }
                }
                "INSERT" => {
                    let Some(name) = field(block, 2).map(str::trim).filter(|name| !name.is_empty())
                    else {
                        self.skip("INSERT (missing block name)", block);
                        index = end;
                        continue;
                    };
                    attribute_layer = Some(layer.clone());
                    let insertion = point_at(block, 10, 20).unwrap_or([0.0, 0.0]);
                    let scale_x = number(block, 41).unwrap_or(1.0);
                    let scale_y = number(block, 42).unwrap_or(scale_x);
                    let rotation = number(block, 50).unwrap_or(0.0).to_radians();
                    let Some(definition) = blocks.get(name)
                    else {
                        *self.missing_blocks.entry(name.to_string()).or_insert(0) += 1;
                        index = end;
                        continue;
                    };
                    if depth >= MAX_BLOCK_DEPTH {
                        self.block_recursions += 1;
                    }
                    else {
                        self.expanded_blocks += 1;
                        let local = Xform::insert(insertion, scale_x, scale_y, rotation, definition.base);
                        let combined = xform.compose(local);
                        self.parse_entities(blocks, &definition.pairs, combined, Some(&layer), depth + 1);
                    }
                }
                // Structural markers are consumed by their owning entity.
                "SEQEND" | "VERTEX" | "ENDBLK" => {}
                _ => self.skip(&kind, block),
            }
            index = end;
        }
    }

    /// Reads the LTYPE table entries of the TABLES section.
    ///
    /// An entry whose name matches a line type already in the document replaces
    /// it, so a drawing never carries duplicate line type names.
    fn parse_linetype_table(&mut self, pairs: &[Pair]) {
        let mut index = 0;
        while index < pairs.len() {
            if pairs[index].code == 0 && pairs[index].value.trim().eq_ignore_ascii_case("LTYPE") {
                let end = block_end(pairs, index + 1);
                if let Some(linetype) = parse_linetype(&pairs[index + 1..end]) {
                    if let Some(existing) = self.drawing.linetypes.iter_mut().find(|existing| existing.id == linetype.id) {
                        *existing = linetype;
                    }
                    else {
                        self.drawing.linetypes.push(linetype);
                    }
                }
                index = end;
                continue;
            }
            index += 1;
        }
    }

    /// Reads the LAYER table entries of the TABLES section.
    fn parse_layer_table(&mut self, pairs: &[Pair]) {
        let mut index = 0;
        while index < pairs.len() {
            if pairs[index].code == 0 && pairs[index].value.trim().eq_ignore_ascii_case("LAYER") {
                let end = block_end(pairs, index + 1);
                if let Some(entry) = parse_layer(&pairs[index + 1..end]) {
                    self.apply_layer_table_entry(entry);
                }
                index = end;
                continue;
            }
            index += 1;
        }
    }

    /// Reads the DIMSTYLE table entries of the TABLES section.
    ///
    /// Only the modelled variables are read; an entry whose name matches a
    /// style already in the document replaces it, so a drawing never carries
    /// duplicate style names.
    fn parse_dim_style_table(&mut self, pairs: &[Pair]) {
        let mut index = 0;
        while index < pairs.len() {
            if pairs[index].code == 0 && pairs[index].value.trim().eq_ignore_ascii_case("DIMSTYLE") {
                let end = block_end(pairs, index + 1);
                if let Some(style) = parse_dim_style(&pairs[index + 1..end]) {
                    if let Some(existing) = self.drawing.dim_styles.iter_mut().find(|existing| existing.id == style.id) {
                        *existing = style;
                    }
                    else {
                        self.drawing.dim_styles.push(style);
                    }
                }
                index = end;
                continue;
            }
            index += 1;
        }
    }

    /// Reads the STYLE table entries of the TABLES section.
    ///
    /// Only the primary font, fixed height, width factor and oblique angle are
    /// kept; other text-style variables (generation flags, big font) are not
    /// modelled.
    fn parse_text_style_table(&mut self, pairs: &[Pair]) {
        let mut index = 0;
        while index < pairs.len() {
            if pairs[index].code == 0 && pairs[index].value.trim().eq_ignore_ascii_case("STYLE") {
                let end = block_end(pairs, index + 1);
                if let Some(style) = parse_text_style(&pairs[index + 1..end]) {
                    if let Some(existing) = self.drawing.text_styles.iter_mut().find(|existing| existing.id == style.id) {
                        *existing = style;
                    }
                    else {
                        self.drawing.text_styles.push(style);
                    }
                }
                index = end;
                continue;
            }
            index += 1;
        }
    }

    fn finish(mut self) -> (Drawing, Vec<Loss>) {
        let mut losses = std::mem::take(&mut self.losses);

        for (name, count) in &self.implicit_layers {
            losses.push(Loss::warning(
                "dxf/implicit-layer",
                format!("layer `{name}` was not in the LAYER table; it was created with the default colour ({count} entity reference(s))"),
            ));
        }
        for (kind, skipped) in &self.skipped {
            let mut detail = format!("`{kind}` is not supported; {} entity/entities skipped (first at line {}", skipped.count, skipped.line);
            if let Some(handle) = &skipped.handle {
                detail.push_str(&format!(", handle {handle}"));
            }
            detail.push(')');
            losses.push(Loss::warning("dxf/unsupported-entity", detail));
        }
        for (name, count) in &self.missing_blocks {
            losses.push(Loss::warning(
                "dxf/missing-block",
                format!("INSERT references block `{name}`, which is not defined ({count} reference(s))"),
            ));
        }
        if self.unsupported_colors > 0 {
            losses.push(Loss::warning(
                "dxf/layer-color",
                format!("{} layer colour(s) outside the ACI palette were replaced with the default colour", self.unsupported_colors),
            ));
        }
        if self.expanded_blocks > 0 {
            losses.push(Loss::notice(
                "dxf/expanded-block",
                format!("{} block reference(s) were expanded into the drawing", self.expanded_blocks),
            ));
        }
        if self.block_recursions > 0 {
            losses.push(Loss::warning(
                "dxf/block-recursion",
                format!("{} block reference(s) nested deeper than {MAX_BLOCK_DEPTH} levels were not expanded", self.block_recursions),
            ));
        }
        if self.dimensions_without_type > 0 {
            losses.push(Loss::notice(
                "dxf/dimension-type",
                format!("{} dimension(s) had no type code and were read as aligned", self.dimensions_without_type),
            ));
        }
        if self.dimensions_without_definition > 0 {
            losses.push(Loss::notice(
                "dxf/dimension-offset",
                format!(
                    "{} dimension(s) had no definition point; their dimension line sits on the measured segment",
                    self.dimensions_without_definition
                ),
            ));
        }
        if self.attribute_values > 0 {
            losses
                .push(Loss::notice("dxf/attribute-text", format!("{} block attribute value(s) were imported as text", self.attribute_values)));
        }
        if self.empty_attributes > 0 {
            losses.push(Loss::warning(
                "dxf/attribute-empty",
                format!("{} block attribute(s) had no value and were skipped", self.empty_attributes),
            ));
        }
        if self.spline_fit_points > 0 {
            losses.push(Loss::notice(
                "dxf/spline-fit-points",
                format!(
                    "{} SPLINE entity/entities were imported as a polyline through their fit points; the fitted curve was not evaluated",
                    self.spline_fit_points
                ),
            ));
        }
        if self.spline_control_polygons > 0 {
            losses.push(Loss::notice(
                "dxf/spline-control-polygon",
                format!(
                    "{} SPLINE entity/entities had no fit points and were imported as their control polygon; the fitted curve was not evaluated",
                    self.spline_control_polygons
                ),
            ));
        }
        if self.leaders > 0 {
            losses.push(Loss::notice(
                "dxf/leader-polyline",
                format!(
                    "{} LEADER entity/entities were imported as open polylines; arrowheads and annotation text are not modelled",
                    self.leaders
                ),
            ));
        }
        if self.projected_polylines > 0 {
            losses.push(Loss::notice(
                "dxf/projected-3d",
                format!(
                    "{} 3D POLYLINE entity/entities were projected onto the XY plane; Z coordinates were dropped",
                    self.projected_polylines
                ),
            ));
        }
        if self.hatches > 0 {
            losses.push(Loss::notice(
                "dxf/hatch-boundary",
                format!("{} HATCH entity/entities were imported as closed filled polygons", self.hatches),
            ));
        }
        if self.hatch_bulges > 0 {
            losses.push(Loss::notice(
                "dxf/hatch-bulge",
                format!("{} HATCH boundary path(s) contained bulge arcs that were flattened to line segments", self.hatch_bulges),
            ));
        }
        if self.mleaders > 0 {
            losses.push(Loss::notice(
                "dxf/mleader-leader",
                format!(
                    "{} MLEADER entity/entities were read as an open polyline plus TEXT; leader style, landing, arrow and background are not modelled",
                    self.mleaders
                ),
            ));
        }
        if self.mleader_text_lines > 0 {
            losses.push(Loss::notice(
                "dxf/mleader-text",
                format!("{} MLEADER text line(s) became single-line TEXT entities with inline formatting removed", self.mleader_text_lines),
            ));
        }
        if self.mleader_missing_text > 0 {
            losses.push(Loss::warning(
                "dxf/mleader-text-missing",
                format!(
                    "{} MLEADER entity/entities carried no inline text block; only the leader polyline was imported",
                    self.mleader_missing_text
                ),
            ));
        }

        (self.drawing, losses)
    }
}

/// Reads `$INSUNITS` and returns the document unit, the coordinate scale that
/// converts the file into that unit, and a notice when the header is missing or
/// names a unit the document model cannot store.
fn parse_units(pairs: &[Pair]) -> (Units, f64, Option<String>) {
    let mut index = 0;
    while index < pairs.len() {
        if pairs[index].code == 9 && pairs[index].value.trim().eq_ignore_ascii_case("$INSUNITS") {
            if let Some(pair) = pairs.get(index + 1)
                && pair.code == 70
            {
                return match pair.value.trim().parse::<i64>() {
                    Ok(1) => (Units::Inch, 1.0, None),
                    Ok(4) => (Units::Mm, 1.0, None),
                    Ok(5) => (Units::Mm, 10.0, Some("$INSUNITS=5 (centimetres) was converted to millimetres".to_string())),
                    Ok(6) => (Units::Mm, 1000.0, Some("$INSUNITS=6 (metres) was converted to millimetres".to_string())),
                    Ok(value) => {
                        (Units::Mm, 1.0, Some(format!("$INSUNITS={value} has no drawing unit in this model; the drawing uses millimetres")))
                    }
                    Err(_) => (Units::Mm, 1.0, Some("$INSUNITS is malformed; the drawing uses millimetres".to_string())),
                };
            }
            return (Units::Mm, 1.0, Some("$INSUNITS is missing a value; the drawing uses millimetres".to_string()));
        }
        index += 1;
    }
    (Units::Mm, 1.0, Some("$INSUNITS is missing; the drawing uses millimetres".to_string()))
}

fn block_end(pairs: &[Pair], start: usize) -> usize {
    let mut end = start;
    while end < pairs.len() && pairs[end].code != 0 {
        end += 1;
    }
    end
}

/// Returns the pairs of one named section, without its header and ENDSEC.
fn section_pairs<'a>(pairs: &'a [Pair], name: &str) -> Option<&'a [Pair]> {
    let mut index = 0;
    while index + 1 < pairs.len() {
        if pairs[index].code == 0
            && pairs[index].value.trim().eq_ignore_ascii_case("SECTION")
            && pairs[index + 1].code == 2
            && pairs[index + 1].value.trim().eq_ignore_ascii_case(name)
        {
            let start = index + 2;
            let mut end = start;
            while end < pairs.len() {
                if pairs[end].code == 0 && pairs[end].value.trim().eq_ignore_ascii_case("ENDSEC") {
                    return Some(&pairs[start..end]);
                }
                end += 1;
            }
            return Some(&pairs[start..]);
        }
        index += 1;
    }
    None
}

/// Collects BLOCK definitions; each entry keeps its raw body for later parsing.
fn collect_blocks(pairs: &[Pair]) -> BTreeMap<String, BlockDef> {
    let mut blocks = BTreeMap::new();
    let mut index = 0;
    while index < pairs.len() {
        if pairs[index].code == 0 && pairs[index].value.trim().eq_ignore_ascii_case("BLOCK") {
            let header_end = block_end(pairs, index + 1);
            let header = &pairs[index + 1..header_end];
            let name = field(header, 2).or_else(|| field(header, 3)).map(str::trim).unwrap_or("").to_string();
            let base = [number(header, 10).unwrap_or(0.0), number(header, 20).unwrap_or(0.0)];
            let mut cursor = header_end;
            let mut body = Vec::new();
            while cursor < pairs.len() {
                if pairs[cursor].code == 0 && pairs[cursor].value.trim().eq_ignore_ascii_case("ENDBLK") {
                    cursor = block_end(pairs, cursor + 1);
                    break;
                }
                body.push(pairs[cursor].clone());
                cursor += 1;
            }
            if !name.is_empty() {
                blocks.insert(name, BlockDef { base, pairs: body });
            }
            index = cursor;
            continue;
        }
        index += 1;
    }
    blocks
}

/// Parses the declared ASCII DXF subset into a new drawing.
pub(crate) fn dxf(text: &str, title: Option<String>) -> Result<(Drawing, Vec<Loss>), ServiceError> {
    if !text.contains("SECTION") {
        return Err(invalid("the file has no DXF SECTION; it is not an ASCII DXF drawing"));
    }
    let pairs = pairs(text)?;
    let (units, scale, units_notice) = parse_units(&pairs);
    let blocks = collect_blocks(&pairs);
    let entities = section_pairs(&pairs, "ENTITIES").ok_or_else(|| invalid("the file has no ENTITIES section"))?;

    let mut reader = Reader::new(units, title);
    if let Some(detail) = units_notice {
        reader.losses.push(Loss::notice("dxf/assumed-units", detail));
    }
    if let Some(tables) = section_pairs(&pairs, "TABLES") {
        reader.parse_linetype_table(tables);
        reader.parse_layer_table(tables);
        reader.parse_text_style_table(tables);
        reader.parse_dim_style_table(tables);
    }
    reader.parse_entities(&blocks, entities, Xform::scale(scale), None, 0);
    Ok(reader.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a minimal ASCII DXF with optional BLOCKS content and the given
    /// ENTITIES body, both written as raw group-code pairs.
    fn document(blocks: &str, entities: &str) -> String {
        format!(
            "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nBLOCKS\n{blocks}\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n{entities}\n0\nENDSEC\n0\nEOF\n"
        )
    }

    fn codes(losses: &[Loss]) -> Vec<&str> {
        losses.iter().map(|loss| loss.code.as_str()).collect()
    }

    #[test]
    fn hatch_import_reads_a_polyline_boundary_and_keeps_its_pattern() {
        let entities = [
            "0", "HATCH", "8", "FILL", "2", "ANSI31", "70", "0", "71", "0", "91", "1", "92", "1", "72", "0", "73", "1", "93", "3", "10", "0",
            "20", "0", "10", "4", "20", "0", "10", "4", "20", "4", "97", "0", "75", "0", "76", "1", "98", "0",
        ]
        .join("\n");
        let (drawing, losses) = dxf(&document("", &entities), None).expect("read hatch");

        assert_eq!(drawing.entities.len(), 1);
        let EntityKind::Hatch { points, pattern } = &drawing.entities[0].kind
        else {
            panic!("expected a hatch, got {:?}", drawing.entities[0].kind);
        };
        assert_eq!(points.as_slice(), &[[0.0, 0.0], [4.0, 0.0], [4.0, 4.0]]);
        assert_eq!(pattern, "ANSI31");
        assert_eq!(drawing.entities[0].layer, "FILL");
        assert!(codes(&losses).contains(&"dxf/hatch-boundary"));
        assert!(!codes(&losses).contains(&"dxf/unsupported-entity"), "{:?}", codes(&losses));
    }

    #[test]
    fn hatch_import_samples_line_and_arc_edges_and_skips_unknown_edges() {
        let entities = [
            "0", "HATCH", "8", "FILL", "2", "SOLID", "70", "1", "91", "1", "92", "3", "93", "2", "72", "1", "10", "0", "20", "0", "11", "4",
            "21", "0", "72", "2", "10", "4", "20", "0", "40", "2", "50", "0", "51", "180", "73", "1", "97", "0", "75", "0", "76", "1", "98",
            "0",
        ]
        .join("\n");
        let (drawing, losses) = dxf(&document("", &entities), None).expect("read edge hatch");
        assert_eq!(drawing.entities.len(), 1, "{:?}", codes(&losses));
        let EntityKind::Hatch { points, .. } = &drawing.entities[0].kind
        else {
            panic!("expected a hatch, got {:?}", drawing.entities[0].kind);
        };
        assert!(points.len() > 4, "the arc edge is sampled: {points:?}");
        assert_eq!(points[0], [0.0, 0.0]);
        assert!((points.last().expect("points")[1]).abs() < 1e-9);

        let spline_edge =
            ["0", "HATCH", "8", "FILL", "2", "SOLID", "91", "1", "92", "3", "93", "1", "72", "4", "94", "3", "97", "0"].join("\n");
        let (drawing, losses) = dxf(&document("", &spline_edge), None).expect("read unsupported hatch");
        assert!(drawing.entities.is_empty());
        assert!(losses.iter().any(|loss| loss.code == "dxf/unsupported-entity" && loss.detail.contains("curved")));
    }

    #[test]
    fn mleader_import_reads_leader_vertices_and_flattens_multiline_text() {
        let entities = [
            "0",
            "MLEADER",
            "8",
            "NOTES",
            "40",
            "2.5",
            "304",
            "CONTEXT_DATA{",
            "304",
            "LEADER{",
            "290",
            "1",
            "10",
            "0",
            "20",
            "0",
            "10",
            "10",
            "20",
            "5",
            "304",
            "}",
            "304",
            "TEXT{",
            "10",
            "99",
            "20",
            "99",
            "304",
            "{\\fArial|b0;\\Pfirst line\\Psecond line}",
            "304",
            "}",
            "304",
            "}",
        ]
        .join("\n");
        let (drawing, losses) = dxf(&document("", &entities), None).expect("read mleader");

        assert_eq!(drawing.entities.len(), 3, "{:?}", codes(&losses));
        let EntityKind::Polyline { points, closed } = &drawing.entities[0].kind
        else {
            panic!("expected a leader polyline, got {:?}", drawing.entities[0].kind);
        };
        assert_eq!(points.as_slice(), &[[0.0, 0.0], [10.0, 5.0]]);
        assert!(!closed);
        let EntityKind::Text { at, height, content } = &drawing.entities[1].kind
        else {
            panic!("expected a text line, got {:?}", drawing.entities[1].kind);
        };
        assert_eq!((*at, *height, content.as_str()), ([0.0, 0.0], 2.5, "first line"));
        let EntityKind::Text { at, content, .. } = &drawing.entities[2].kind
        else {
            panic!("expected a text line, got {:?}", drawing.entities[2].kind);
        };
        assert!((at[0] - 0.0).abs() < 1e-9 && (at[1] + 3.5).abs() < 1e-9, "stacked line at {at:?}");
        assert_eq!(content, "second line");
        assert!(codes(&losses).contains(&"dxf/mleader-leader"));
        assert!(codes(&losses).contains(&"dxf/mleader-text"));

        let textless =
            ["0", "MLEADER", "8", "NOTES", "304", "CONTEXT_DATA{", "304", "LEADER{", "10", "0", "20", "0", "10", "5", "20", "5", "304", "}"]
                .join("\n");
        let (drawing, losses) = dxf(&document("", &textless), None).expect("read textless mleader");
        assert_eq!(drawing.entities.len(), 1);
        assert!(losses.iter().any(|loss| loss.code == "dxf/mleader-text-missing" && loss.severity == "warning"));
    }

    #[test]
    fn dimension_import_reads_aligned_and_rotated_references() {
        let entities = [
            "0",
            "DIMENSION",
            "8",
            "DIMENSIONS",
            "70",
            "1",
            "13",
            "0",
            "23",
            "0",
            "14",
            "10",
            "24",
            "0",
            "10",
            "5",
            "20",
            "3",
            "1",
            "W 10",
            "0",
            "DIMENSION",
            "8",
            "DIMENSIONS",
            "70",
            "0",
            "50",
            "90",
            "13",
            "0",
            "23",
            "0",
            "14",
            "0",
            "24",
            "10",
            "10",
            "2",
            "20",
            "5",
            "1",
            "<>",
        ]
        .join("\n");
        let (drawing, losses) = dxf(&document("", &entities), Some("dimensions".to_string())).expect("read dimensions");

        assert_eq!(drawing.entities.len(), 2);
        assert_eq!(drawing.entities[0].layer, "DIMENSIONS");
        let EntityKind::Dimension { from, to, offset, text } = &drawing.entities[0].kind
        else {
            panic!("expected a dimension, got {:?}", drawing.entities[0].kind);
        };
        assert_eq!((*from, *to), ([0.0, 0.0], [10.0, 0.0]));
        assert!((offset - 3.0).abs() < 1e-9);
        assert_eq!(text.as_deref(), Some("W 10"));

        // A rotated dimension whose rotation follows its segment keeps the same
        // measured points, and `<>` means "use the measured value".
        let EntityKind::Dimension { offset, text, .. } = &drawing.entities[1].kind
        else {
            panic!("expected a rotated dimension, got {:?}", drawing.entities[1].kind);
        };
        assert!((offset + 2.0).abs() < 1e-9);
        assert_eq!(text, &None);
        assert!(!codes(&losses).contains(&"dxf/unsupported-entity"), "{:?}", codes(&losses));
    }

    #[test]
    fn dimension_import_skips_types_the_model_cannot_carry() {
        let entities = [
            "0",
            "DIMENSION",
            "70",
            "2",
            "13",
            "0",
            "23",
            "0",
            "14",
            "10",
            "24",
            "0",
            "0",
            "DIMENSION",
            "70",
            "0",
            "50",
            "90",
            "13",
            "0",
            "23",
            "0",
            "14",
            "10",
            "24",
            "0",
            "10",
            "5",
            "20",
            "3",
        ]
        .join("\n");
        let (drawing, losses) = dxf(&document("", &entities), None).expect("read dimensions");

        assert!(drawing.entities.is_empty());
        let skipped = losses.iter().filter(|loss| loss.code == "dxf/unsupported-entity").collect::<Vec<_>>();
        assert_eq!(skipped.len(), 2, "{:?}", codes(&losses));
        assert!(skipped.iter().any(|loss| loss.detail.contains("angular")));
        assert!(skipped.iter().any(|loss| loss.detail.contains("rotated")));
        assert!(skipped.iter().all(|loss| loss.severity == "warning"));
    }

    #[test]
    fn dimension_import_reports_a_missing_type_and_definition_point() {
        let entities = ["0", "DIMENSION", "13", "0", "23", "0", "14", "4", "24", "0"].join("\n");
        let (drawing, losses) = dxf(&document("", &entities), None).expect("read dimensions");

        let EntityKind::Dimension { offset, .. } = &drawing.entities[0].kind
        else {
            panic!("expected a dimension");
        };
        assert_eq!(*offset, 0.0, "a missing definition point keeps the measured segment");
        assert!(codes(&losses).contains(&"dxf/dimension-type"));
        assert!(codes(&losses).contains(&"dxf/dimension-offset"));
    }

    #[test]
    fn attribute_values_follow_the_insert_layer() {
        let blocks = ["0", "BLOCK", "8", "0", "2", "TITLE", "70", "0", "10", "0", "20", "0", "0", "ENDBLK"].join("\n");
        let entities = [
            "0", "INSERT", "8", "TITLE", "2", "TITLE", "10", "0", "20", "0", "0", "ATTRIB", "8", "0", "10", "2", "20", "3", "40", "2.5", "1",
            "A-101", "0", "ATTRIB", "8", "NOTES", "10", "4", "20", "3", "40", "2.5", "1", "NOTE", "0", "ATTRIB", "8", "0", "10", "6", "20",
            "3", "40", "2.5", "1", "", "0", "SEQEND", "8", "0",
        ]
        .join("\n");
        let (drawing, losses) = dxf(&document(&blocks, &entities), None).expect("read attributes");

        assert_eq!(drawing.entities.len(), 2);
        assert_eq!(drawing.entities[0].layer, "TITLE");
        let EntityKind::Text { at, height, content } = &drawing.entities[0].kind
        else {
            panic!("expected text, got {:?}", drawing.entities[0].kind);
        };
        assert_eq!((*at, *height, content.as_str()), ([2.0, 3.0], 2.5, "A-101"));
        assert_eq!(drawing.entities[1].layer, "NOTES");
        assert!(codes(&losses).contains(&"dxf/attribute-text"));
        assert!(codes(&losses).contains(&"dxf/attribute-empty"));
        assert_eq!(
            losses.iter().find(|loss| loss.code == "dxf/attribute-text").map(|loss| loss.detail.as_str()),
            Some("2 block attribute value(s) were imported as text")
        );
        assert_eq!(
            losses.iter().find(|loss| loss.code == "dxf/attribute-empty").map(|loss| loss.detail.as_str()),
            Some("1 block attribute(s) had no value and were skipped")
        );
    }

    #[test]
    fn spline_import_prefers_fit_points_and_falls_back_to_control_points() {
        let entities = [
            "0", "SPLINE", "8", "WALLS", "70", "0", "71", "3", "10", "0", "20", "0", "10", "5", "20", "10", "10", "10", "20", "0", "11", "0",
            "21", "0", "11", "5", "21", "5", "11", "10", "21", "0", "0", "SPLINE", "8", "WALLS", "70", "1", "10", "2", "20", "2", "10", "8",
            "20", "2", "10", "8", "20", "8", "0", "SPLINE", "8", "WALLS",
        ]
        .join("\n");
        let (drawing, losses) = dxf(&document("", &entities), None).expect("read splines");

        assert_eq!(drawing.entities.len(), 2);
        let EntityKind::Polyline { points, closed } = &drawing.entities[0].kind
        else {
            panic!("expected a spline polyline, got {:?}", drawing.entities[0].kind);
        };
        assert_eq!(points.as_slice(), &[[0.0, 0.0], [5.0, 5.0], [10.0, 0.0]]);
        assert!(!closed, "fit points of an open spline keep the chain open");
        let EntityKind::Polyline { points, closed } = &drawing.entities[1].kind
        else {
            panic!("expected a spline polyline, got {:?}", drawing.entities[1].kind);
        };
        assert_eq!(points.as_slice(), &[[2.0, 2.0], [8.0, 2.0], [8.0, 8.0]]);
        assert!(closed, "a closed SPLINE keeps the closed flag");
        assert!(codes(&losses).contains(&"dxf/spline-fit-points"));
        assert!(codes(&losses).contains(&"dxf/spline-control-polygon"));
        assert!(codes(&losses).contains(&"dxf/unsupported-entity"), "a SPLINE without points is still skipped");
    }

    #[test]
    fn leader_import_becomes_an_open_polyline() {
        let entities =
            ["0", "LEADER", "8", "NOTES", "3", "STANDARD", "76", "3", "10", "0", "20", "0", "10", "4", "20", "2", "10", "8", "20", "2"]
                .join("\n");
        let (drawing, losses) = dxf(&document("", &entities), None).expect("read leader");

        assert_eq!(drawing.entities.len(), 1);
        let EntityKind::Polyline { points, closed } = &drawing.entities[0].kind
        else {
            panic!("expected a leader polyline, got {:?}", drawing.entities[0].kind);
        };
        assert_eq!(points.as_slice(), &[[0.0, 0.0], [4.0, 2.0], [8.0, 2.0]]);
        assert!(!closed);
        assert!(codes(&losses).contains(&"dxf/leader-polyline"));
        assert!(!codes(&losses).contains(&"dxf/unsupported-entity"), "{:?}", codes(&losses));
    }

    #[test]
    fn three_dimensional_polyline_is_projected_onto_the_plane() {
        let entities = [
            "0", "POLYLINE", "8", "WALLS", "70", "8", "0", "VERTEX", "8", "WALLS", "10", "0", "20", "0", "30", "5", "0", "VERTEX", "8",
            "WALLS", "10", "10", "20", "0", "30", "6", "0", "SEQEND", "8", "WALLS",
        ]
        .join("\n");
        let (drawing, losses) = dxf(&document("", &entities), None).expect("read 3D polyline");

        assert_eq!(drawing.entities.len(), 1);
        let EntityKind::Polyline { points, .. } = &drawing.entities[0].kind
        else {
            panic!("expected a projected polyline, got {:?}", drawing.entities[0].kind);
        };
        assert_eq!(points.as_slice(), &[[0.0, 0.0], [10.0, 0.0]]);
        assert!(codes(&losses).contains(&"dxf/projected-3d"));
    }
}

//! ASCII DXF reader for the declared subset.
//!
//! The reader accepts group-code pairs from an ASCII DXF file and keeps only
//! what the drawing model can represent. It reads the HEADER `$INSUNITS` value,
//! the LAYER table, BLOCKS definitions, and LINE, CIRCLE, ARC, ELLIPSE, POINT,
//! TEXT, LWPOLYLINE and the older POLYLINE/VERTEX/SEQEND sequence, including
//! INSERT references that are expanded into the current space.
//!
//! ARC and ELLIPSE are read into the analytic entity kinds. Bulge arcs inside
//! polylines are flattened to line segments and reported as a loss; entities the
//! reader does not understand are skipped once per type. Nothing is dropped
//! silently.

use std::{collections::BTreeMap, f64::consts::TAU};

use serde::Serialize;

use crate::{
    ServiceError,
    schema::{Drawing, Entity, EntityKind, Layer, Units},
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

fn parse_layer(block: &[Pair]) -> Option<(String, Option<i64>)> {
    let name = field(block, 2)?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    Some((name, integer(block, 62)))
}

fn closed_flag(block: &[Pair]) -> bool {
    integer(block, 70).is_some_and(|flags| flags & 1 == 1)
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

    fn apply_layer_table_entry(&mut self, name: &str, color: Option<i64>) {
        let color = match color {
            Some(index) => match aci_color(index) {
                Some(value) => value,
                None => {
                    self.unsupported_colors += 1;
                    "#d0d7de".to_string()
                }
            },
            None => "#d0d7de".to_string(),
        };
        if let Some(layer) = self.drawing.layer_mut(name) {
            layer.color = color;
        }
        else {
            self.drawing.layers.push(Layer { id: name.to_string(), name: name.to_string(), color, visible: true, locked: false });
        }
    }

    fn parse_entities(&mut self, blocks: &BTreeMap<String, BlockDef>, pairs: &[Pair], xform: Xform, inherit: Option<&str>, depth: usize) {
        let mut index = 0;
        while index < pairs.len() {
            if pairs[index].code != 0 {
                index += 1;
                continue;
            }
            let kind = pairs[index].value.trim().to_ascii_uppercase();
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
                    let points = flatten_vertices(&vertices, closed).into_iter().map(|point| xform.apply(point)).collect();
                    self.push(layer, EntityKind::Polyline { points, closed });
                    index = cursor.max(end);
                    continue;
                }
                "INSERT" => {
                    let Some(name) = field(block, 2).map(str::trim).filter(|name| !name.is_empty())
                    else {
                        self.skip("INSERT (missing block name)", block);
                        index = end;
                        continue;
                    };
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
                "ATTRIB" | "SEQEND" | "VERTEX" | "ENDBLK" => {}
                _ => self.skip(&kind, block),
            }
            index = end;
        }
    }

    /// Reads the LAYER table entries of the TABLES section.
    fn parse_layer_table(&mut self, pairs: &[Pair]) {
        let mut index = 0;
        while index < pairs.len() {
            if pairs[index].code == 0 && pairs[index].value.trim().eq_ignore_ascii_case("LAYER") {
                let end = block_end(pairs, index + 1);
                if let Some((name, color)) = parse_layer(&pairs[index + 1..end]) {
                    self.apply_layer_table_entry(&name, color);
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
        reader.parse_layer_table(tables);
    }
    reader.parse_entities(&blocks, entities, Xform::scale(scale), None, 0);
    Ok(reader.finish())
}

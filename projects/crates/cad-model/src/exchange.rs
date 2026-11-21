//! Exchange writers.
//!
//! SVG keeps every supported entity and each layer's colour. DXF is ASCII R12
//! with a HEADER, a LAYER table and a LINE/CIRCLE/ARC/ELLIPSE/TEXT/POLYLINE
//! entity section; polylines keep their vertices as POLYLINE/VERTEX/SEQEND and
//! dimensions are written as primitives whose approximation is reported as a
//! loss. PDF writes a minimal vector page with a real xref table and reports
//! characters the standard Helvetica encoding cannot carry. SVG and PDF are
//! lowered onto the shared `document-drawing` scene; DXF keeps analytic arcs and
//! ellipses, and STL extrudes five-degree polyline samples.

use document_drawing::{Color, Drawing as FlatDrawing, Element, Point as FlatPoint, Shape, Size as FlatSize, Style};
use geometry_planar::{Point as PlanarPoint, Rect, signed_polygon_area};
use serde::Serialize;

use crate::schema::{Drawing, Entity, EntityKind, Units, arc_sweep, offset_points, sample_arc, sample_ellipse};

/// One item an exchange target could not keep.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Loss {
    /// Stable loss code.
    pub(crate) code: String,
    /// Always `warning` for the current writers.
    pub(crate) severity: &'static str,
    /// Layer locator.
    pub(crate) layer_id: Option<String>,
    /// Entity locator.
    pub(crate) entity_id: Option<String>,
    /// Human-readable message.
    pub(crate) message: String,
}

impl Loss {
    fn warning(code: &str, layer_id: Option<&str>, entity_id: Option<&str>, message: String) -> Self {
        Self {
            code: code.to_string(),
            severity: "warning",
            layer_id: layer_id.map(str::to_string),
            entity_id: entity_id.map(str::to_string),
            message,
        }
    }
}

/// Formats a number without exponent notation and without trailing zeros.
fn num(value: f64) -> String {
    if !value.is_finite() {
        return "0".to_string();
    }
    let mut text = format!("{value:.4}");
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    if text == "-0" { "0".to_string() } else { text }
}

/// Formats a number with round-trip precision, used by analytic DXF entities
/// whose angles and radii must survive a read-back unchanged.
fn num_exact(value: f64) -> String {
    if !value.is_finite() {
        return "0".to_string();
    }
    let text = format!("{value}");
    if text == "-0" { "0".to_string() } else { text }
}

fn escape_xml(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

fn stroke_width(units: Units) -> f64 {
    match units {
        Units::Mm => 0.5,
        Units::Inch => 0.02,
    }
}

fn dimension_label(from: [f64; 2], to: [f64; 2], text: &Option<String>) -> String {
    if let Some(text) = text {
        return text.clone();
    }
    let distance = ((to[0] - from[0]).powi(2) + (to[1] - from[1]).powi(2)).sqrt();
    num(distance)
}

#[derive(Debug, Clone, Copy)]
struct Frame {
    min_x: f64,
    min_y: f64,
    width: f64,
    height: f64,
}

impl Frame {
    fn of(drawing: &Drawing) -> Self {
        match visible_extents(drawing) {
            Some(extents) => {
                let width = extents.width().max(1.0);
                let height = extents.height().max(1.0);
                let pad = (width.max(height) * 0.05).max(1.0);
                Self { min_x: extents.min.x - pad, min_y: extents.min.y - pad, width: width + pad * 2.0, height: height + pad * 2.0 }
            }
            None => Self { min_x: 0.0, min_y: 0.0, width: 297.0, height: 210.0 },
        }
    }

    /// Maps model coordinates (y up) onto the SVG canvas (y down).
    fn map(self, point: [f64; 2]) -> FlatPoint {
        FlatPoint::new(point[0] - self.min_x, self.min_y + self.height - point[1])
    }
}

/// Bounds of the entities on visible layers, ignoring non-finite coordinates.
fn visible_extents(drawing: &Drawing) -> Option<Rect> {
    let mut extents: Option<Rect> = None;
    for entity in &drawing.entities {
        if !drawing.layer(&entity.layer).is_some_and(|layer| layer.visible) {
            continue;
        }
        for point in entity.bounds_points() {
            if !point[0].is_finite() || !point[1].is_finite() {
                continue;
            }
            let point = PlanarPoint::new(point[0], point[1]);
            let point_rect = Rect::from_points(point, point);
            extents = Some(extents.map_or(point_rect, |extents| extents.union(point_rect)));
        }
    }
    extents
}

/// Parses a CSS colour used by a layer; malformed values keep the previous
/// default ink.
fn color_of(value: &str) -> Color {
    let digits = value.trim().trim_start_matches('#').to_ascii_lowercase();
    let expanded = match digits.len() {
        3 => digits.chars().flat_map(|character| [character, character]).collect::<String>(),
        6 => digits,
        _ => return Color::rgb(31, 41, 51),
    };
    let channel = |range: std::ops::Range<usize>| u8::from_str_radix(&expanded[range], 16).unwrap_or(0);
    Color::rgb(channel(0..2), channel(2..4), channel(4..6))
}

/// How model coordinates become flat-drawing coordinates.
struct Projection<'a> {
    map: &'a dyn Fn([f64; 2]) -> FlatPoint,
    scale: f64,
    stroke: f64,
    label_offset: f64,
    label_size: f64,
}

fn build_drawing(drawing: &Drawing, size: FlatSize, projection: &Projection<'_>) -> FlatDrawing {
    let mut flat = FlatDrawing::new(size);
    for layer in &drawing.layers {
        if !layer.visible {
            continue;
        }
        let color = color_of(&layer.color);
        let stroke_style = Style::stroke(color, projection.stroke);
        let text_style = Style::fill(color);
        for entity in drawing.entities.iter().filter(|entity| entity.layer == layer.id) {
            if entity.is_finite() {
                push_entity(&mut flat, entity, color, &stroke_style, &text_style, projection);
            }
        }
    }
    flat
}

fn push_entity(flat: &mut FlatDrawing, entity: &Entity, color: Color, stroke_style: &Style, text_style: &Style, projection: &Projection<'_>) {
    let map = projection.map;
    let scale = projection.scale;
    match &entity.kind {
        EntityKind::Line { from, to } => {
            flat.push(Element::new(Shape::Line { from: map(*from), to: map(*to) }, stroke_style.clone()));
        }
        EntityKind::Circle { center, radius } => {
            flat.push(Element::new(Shape::Circle { center: map(*center), radius: radius.abs() * scale }, stroke_style.clone()));
        }
        EntityKind::Arc { center, radius, start_angle, end_angle } => {
            let sweep = arc_sweep(*start_angle, *end_angle);
            flat.push(Element::new(
                Shape::Arc {
                    center: map(*center),
                    radius: radius.abs() * scale,
                    start_degrees: -*start_angle,
                    end_degrees: -(*start_angle + sweep),
                },
                stroke_style.clone(),
            ));
        }
        EntityKind::Ellipse { center, rx, ry, rotation, start_angle, end_angle } => {
            let rx = rx.abs();
            let ry = ry.abs();
            let sweep = arc_sweep(*start_angle, *end_angle);
            let axis = rotation.rem_euclid(180.0);
            if sweep >= 360.0 - 1e-9 && (axis < 1e-6 || (axis - 90.0).abs() < 1e-6) {
                let radii =
                    if (axis - 90.0).abs() < 1e-6 { FlatSize::new(ry * scale, rx * scale) } else { FlatSize::new(rx * scale, ry * scale) };
                flat.push(Element::new(Shape::Ellipse { center: map(*center), radii }, stroke_style.clone()));
            }
            else {
                let points =
                    sample_ellipse(*center, rx, ry, *rotation, *start_angle, *end_angle).iter().map(|point| map(*point)).collect::<Vec<_>>();
                let shape = if sweep >= 360.0 - 1e-9 { Shape::Polygon { points } } else { Shape::Polyline { points } };
                flat.push(Element::new(shape, stroke_style.clone()));
            }
        }
        EntityKind::Point { at } => {
            flat.push(Element::new(Shape::Circle { center: map(*at), radius: projection.stroke * 2.0 * scale }, Style::fill(color)));
        }
        EntityKind::Polyline { points, closed } => {
            let points = points.iter().map(|point| map(*point)).collect::<Vec<_>>();
            let shape = if *closed { Shape::Polygon { points } } else { Shape::Polyline { points } };
            flat.push(Element::new(shape, stroke_style.clone()));
        }
        EntityKind::Text { at, height, content } => {
            flat.push(Element::new(
                Shape::Text { at: map(*at), text: content.clone(), size: (height.abs() * scale).max(0.01) },
                text_style.clone(),
            ));
        }
        EntityKind::Dimension { from, to, offset, text } => {
            let (p1, p2) = offset_points(*from, *to, *offset);
            for (start, end) in [(*from, p1), (*to, p2), (p1, p2)] {
                flat.push(Element::new(Shape::Line { from: map(start), to: map(end) }, stroke_style.clone()));
            }
            let midpoint = [(p1[0] + p2[0]) / 2.0, (p1[1] + p2[1]) / 2.0 + projection.label_offset];
            flat.push(Element::new(
                Shape::Text { at: map(midpoint), text: dimension_label(*from, *to, text), size: projection.label_size.max(0.01) },
                text_style.clone(),
            ));
        }
    }
}

fn svg_document(drawing: &Drawing, frame: Frame) -> String {
    let stroke = stroke_width(drawing.units);
    let map = |point: [f64; 2]| frame.map(point);
    let projection = Projection { map: &map, scale: 1.0, stroke, label_offset: 1.0, label_size: stroke * 3.5 };
    let flat = build_drawing(drawing, FlatSize::new(frame.width, frame.height), &projection);
    let svg = flat.to_svg().unwrap_or_else(|_| {
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">\n</svg>\n",
            num(frame.width),
            num(frame.height),
            num(frame.width),
            num(frame.height)
        )
    });
    decorate_svg(svg, drawing)
}

/// Re-attaches the OpenCAD metadata the shared writer does not know about.
fn decorate_svg(mut svg: String, drawing: &Drawing) -> String {
    let Some(index) = svg.find('>')
    else {
        return svg;
    };
    let attributes = format!(" data-open-cad=\"drawing\" data-revision=\"{}\"", drawing.revision);
    let title = format!("\n<title>{}</title>", escape_xml(drawing.title.as_deref().unwrap_or("Untitled drawing")));
    svg.insert_str(index, &attributes);
    svg.insert_str(index + attributes.len() + 1, &title);
    svg
}

/// Renders one drawing to an SVG document.
pub(crate) fn svg(drawing: &Drawing) -> String {
    svg_document(drawing, Frame::of(drawing))
}

/// Renders one drawing to an SVG preview and reports the canvas size.
///
/// The document is produced in memory; nothing is written to disk for a
/// read-only preview.
pub(crate) fn svg_preview(drawing: &Drawing) -> (String, f64, f64) {
    let frame = Frame::of(drawing);
    (svg_document(drawing, frame), frame.width, frame.height)
}

fn dxf_pair(out: &mut String, code: i64, value: impl std::fmt::Display) {
    out.push_str(&format!("{code}\n{value}\n"));
}

fn dxf_layer_name(layer: &str) -> String {
    layer.replace(['\n', '\r'], " ")
}

fn dxf_text(value: &str) -> String {
    value.replace(['\n', '\r'], " ")
}

fn dxf_line(out: &mut String, layer: &str, from: [f64; 2], to: [f64; 2]) {
    dxf_pair(out, 0, "LINE");
    dxf_pair(out, 8, dxf_layer_name(layer));
    dxf_pair(out, 10, num(from[0]));
    dxf_pair(out, 20, num(from[1]));
    dxf_pair(out, 30, "0");
    dxf_pair(out, 11, num(to[0]));
    dxf_pair(out, 21, num(to[1]));
    dxf_pair(out, 31, "0");
}

fn dxf_text_entity(out: &mut String, layer: &str, at: [f64; 2], height: f64, content: &str) {
    dxf_pair(out, 0, "TEXT");
    dxf_pair(out, 8, dxf_layer_name(layer));
    dxf_pair(out, 10, num(at[0]));
    dxf_pair(out, 20, num(at[1]));
    dxf_pair(out, 30, "0");
    dxf_pair(out, 40, num(height.abs().max(0.01)));
    dxf_pair(out, 1, dxf_text(content));
}

/// Renders one drawing to ASCII DXF R12 and reports what the target lost.
pub(crate) fn dxf(drawing: &Drawing) -> (String, Vec<Loss>) {
    let mut out = String::new();
    let mut losses = Vec::new();

    dxf_pair(&mut out, 0, "SECTION");
    dxf_pair(&mut out, 2, "HEADER");
    dxf_pair(&mut out, 9, "$ACADVER");
    dxf_pair(&mut out, 1, "AC1009");
    dxf_pair(&mut out, 9, "$INSUNITS");
    dxf_pair(&mut out, 70, drawing.units.dxf_insunits());
    dxf_pair(&mut out, 0, "ENDSEC");

    dxf_pair(&mut out, 0, "SECTION");
    dxf_pair(&mut out, 2, "TABLES");
    dxf_pair(&mut out, 0, "TABLE");
    dxf_pair(&mut out, 2, "LAYER");
    dxf_pair(&mut out, 70, drawing.layers.len());
    for layer in &drawing.layers {
        dxf_pair(&mut out, 0, "LAYER");
        dxf_pair(&mut out, 2, dxf_layer_name(&layer.id));
        dxf_pair(&mut out, 70, 0);
        dxf_pair(&mut out, 62, 7);
        dxf_pair(&mut out, 6, "CONTINUOUS");
        if layer.color != "#ffffff" {
            losses.push(Loss::warning(
                "dxf/layer-color",
                Some(&layer.id),
                None,
                format!("layer colour `{}` is written as the default R12 colour 7", layer.color),
            ));
        }
    }
    dxf_pair(&mut out, 0, "ENDTAB");
    dxf_pair(&mut out, 0, "ENDSEC");

    dxf_pair(&mut out, 0, "SECTION");
    dxf_pair(&mut out, 2, "ENTITIES");
    for entity in &drawing.entities {
        match &entity.kind {
            EntityKind::Line { from, to } => dxf_line(&mut out, &entity.layer, *from, *to),
            EntityKind::Circle { center, radius } => {
                dxf_pair(&mut out, 0, "CIRCLE");
                dxf_pair(&mut out, 8, dxf_layer_name(&entity.layer));
                dxf_pair(&mut out, 10, num(center[0]));
                dxf_pair(&mut out, 20, num(center[1]));
                dxf_pair(&mut out, 30, "0");
                dxf_pair(&mut out, 40, num(radius.abs()));
            }
            EntityKind::Arc { center, radius, start_angle, end_angle } => {
                dxf_pair(&mut out, 0, "ARC");
                dxf_pair(&mut out, 8, dxf_layer_name(&entity.layer));
                dxf_pair(&mut out, 10, num_exact(center[0]));
                dxf_pair(&mut out, 20, num_exact(center[1]));
                dxf_pair(&mut out, 30, "0");
                dxf_pair(&mut out, 40, num_exact(radius.abs()));
                dxf_pair(&mut out, 50, num_exact(*start_angle));
                dxf_pair(&mut out, 51, num_exact(*end_angle));
            }
            EntityKind::Ellipse { center, rx, ry, rotation, start_angle, end_angle } => {
                let (major, minor, rotation, start_angle, end_angle) = if ry.abs() > rx.abs() {
                    // DXF requires the major-axis vector; rotating by 90 degrees
                    // and shifting the parameter by -90 keeps the same curve.
                    (ry.abs(), rx.abs(), rotation + 90.0, start_angle - 90.0, end_angle - 90.0)
                }
                else {
                    (rx.abs(), ry.abs(), *rotation, *start_angle, *end_angle)
                };
                let (sin, cos) = rotation.to_radians().sin_cos();
                dxf_pair(&mut out, 0, "ELLIPSE");
                dxf_pair(&mut out, 8, dxf_layer_name(&entity.layer));
                dxf_pair(&mut out, 10, num_exact(center[0]));
                dxf_pair(&mut out, 20, num_exact(center[1]));
                dxf_pair(&mut out, 30, "0");
                dxf_pair(&mut out, 11, num_exact(major * cos));
                dxf_pair(&mut out, 21, num_exact(major * sin));
                dxf_pair(&mut out, 31, "0");
                dxf_pair(&mut out, 40, num_exact((minor / major).clamp(1e-9, 1.0)));
                dxf_pair(&mut out, 41, num_exact(start_angle.to_radians()));
                dxf_pair(&mut out, 42, num_exact(end_angle.to_radians()));
            }
            EntityKind::Point { at } => {
                dxf_pair(&mut out, 0, "POINT");
                dxf_pair(&mut out, 8, dxf_layer_name(&entity.layer));
                dxf_pair(&mut out, 10, num(at[0]));
                dxf_pair(&mut out, 20, num(at[1]));
                dxf_pair(&mut out, 30, "0");
            }
            EntityKind::Text { at, height, content } => {
                dxf_text_entity(&mut out, &entity.layer, *at, *height, content);
            }
            EntityKind::Polyline { points, closed } => {
                let layer = dxf_layer_name(&entity.layer);
                dxf_pair(&mut out, 0, "POLYLINE");
                dxf_pair(&mut out, 8, &layer);
                dxf_pair(&mut out, 66, 1);
                dxf_pair(&mut out, 10, "0");
                dxf_pair(&mut out, 20, "0");
                dxf_pair(&mut out, 30, "0");
                dxf_pair(&mut out, 70, if *closed { 1 } else { 0 });
                for point in points {
                    dxf_pair(&mut out, 0, "VERTEX");
                    dxf_pair(&mut out, 8, &layer);
                    dxf_pair(&mut out, 10, num(point[0]));
                    dxf_pair(&mut out, 20, num(point[1]));
                    dxf_pair(&mut out, 30, "0");
                }
                dxf_pair(&mut out, 0, "SEQEND");
                dxf_pair(&mut out, 8, &layer);
            }
            EntityKind::Dimension { from, to, offset, text } => {
                let (p1, p2) = offset_points(*from, *to, *offset);
                dxf_line(&mut out, &entity.layer, *from, p1);
                dxf_line(&mut out, &entity.layer, *to, p2);
                dxf_line(&mut out, &entity.layer, p1, p2);
                let label = dimension_label(*from, *to, text);
                dxf_text_entity(&mut out, &entity.layer, [(p1[0] + p2[0]) / 2.0, (p1[1] + p2[1]) / 2.0], 2.5, &label);
                losses.push(Loss::warning(
                    "dxf/dimension-lines",
                    Some(&entity.layer),
                    Some(&entity.id),
                    "dimension is written as extension lines, a dimension line and TEXT".to_string(),
                ));
            }
        }
    }
    dxf_pair(&mut out, 0, "ENDSEC");
    dxf_pair(&mut out, 0, "EOF");

    (out, losses)
}

/// A4 landscape in PostScript points.
const PDF_PAGE_WIDTH: f64 = 841.89;
const PDF_PAGE_HEIGHT: f64 = 595.28;
const PDF_MARGIN: f64 = 28.0;

/// Escapes one text string for a PDF literal and reports replacements.
///
/// Only printable ASCII is kept; the standard Type1 fonts need an encoding
/// vector for anything else, so those characters become `?` and the export
/// reports the loss.
fn pdf_text(value: &str) -> (String, usize) {
    let mut out = String::with_capacity(value.len());
    let mut replaced = 0;
    for character in value.chars() {
        match character {
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            '\\' => out.push_str("\\\\"),
            '\n' | '\r' => out.push(' '),
            ' '..='~' => out.push(character),
            _ => {
                out.push('?');
                replaced += 1;
            }
        }
    }
    (out, replaced)
}

/// Renders one drawing to a vector PDF and reports text losses.
///
/// The page is A4 landscape. Content is fitted into the printable area with a
/// uniform scale, so the output is a drawing sheet rather than a 1:1 plot.
pub(crate) fn pdf(drawing: &Drawing) -> (String, Vec<Loss>) {
    let extents = visible_extents(drawing);
    let (min_x, min_y, width, height) = match extents {
        Some(extents) => (extents.min.x, extents.min.y, extents.width().max(1.0), extents.height().max(1.0)),
        None => (0.0, 0.0, 297.0, 210.0),
    };
    let usable_width = PDF_PAGE_WIDTH - PDF_MARGIN * 2.0;
    let usable_height = PDF_PAGE_HEIGHT - PDF_MARGIN * 2.0;
    let scale = (usable_width / width).min(usable_height / height);
    let offset_x = (PDF_PAGE_WIDTH - width * scale) / 2.0 - min_x * scale;
    let offset_y = (PDF_PAGE_HEIGHT - height * scale) / 2.0 - min_y * scale;
    let map = |point: [f64; 2]| FlatPoint::new(point[0] * scale + offset_x, PDF_PAGE_HEIGHT - (point[1] * scale + offset_y));
    let projection = Projection { map: &map, scale, stroke: 0.7, label_offset: 1.75, label_size: 1.75 * scale };
    let flat = build_drawing(drawing, FlatSize::new(PDF_PAGE_WIDTH, PDF_PAGE_HEIGHT), &projection);
    let out = flat.to_pdf().map_or_else(|_| String::from("%PDF-1.7\n%%EOF\n"), |bytes| String::from_utf8_lossy(&bytes).into_owned());

    let mut replaced = 0;
    for layer in drawing.layers.iter().filter(|layer| layer.visible) {
        for entity in drawing.entities.iter().filter(|entity| entity.layer == layer.id) {
            match &entity.kind {
                EntityKind::Text { content, .. } => replaced += pdf_text(content).1,
                EntityKind::Dimension { from, to, text, .. } => replaced += pdf_text(&dimension_label(*from, *to, text)).1,
                _ => {}
            }
        }
    }

    let mut losses = Vec::new();
    if replaced > 0 {
        losses.push(Loss::warning(
            "pdf/text-encoding",
            None,
            None,
            format!("{replaced} character(s) outside printable ASCII were written as `?`"),
        ));
    }
    (out, losses)
}

/// Default extrusion thickness of an STL projection, in drawing units.
pub(crate) const STL_DEFAULT_THICKNESS: f64 = 1.0;

/// Result of one ASCII STL projection.
pub(crate) struct StlReport {
    /// ASCII STL document.
    pub(crate) content: String,
    /// Number of triangles written.
    pub(crate) triangles: usize,
    /// Profiles or triangles the projection could not keep.
    pub(crate) losses: Vec<Loss>,
}

/// Removes repeated vertices and the closing duplicate of a profile.
fn closed_profile(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut profile: Vec<[f64; 2]> = Vec::with_capacity(points.len());
    for point in points {
        if profile.last() != Some(point) {
            profile.push(*point);
        }
    }
    while profile.len() > 1 && profile.first() == profile.last() {
        profile.pop();
    }
    profile
}

fn signed_area(points: &[[f64; 2]]) -> f64 {
    let polygon = points.iter().map(|point| PlanarPoint::new(point[0], point[1])).collect::<Vec<_>>();
    signed_polygon_area(&polygon)
}

fn cross(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Whether `point` lies strictly inside the triangle, ignoring boundary points.
fn inside_triangle(point: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> bool {
    let first = cross(a, b, point);
    let second = cross(b, c, point);
    let third = cross(c, a, point);
    (first > 1e-12 && second > 1e-12 && third > 1e-12) || (first < -1e-12 && second < -1e-12 && third < -1e-12)
}

/// Ear-clipping triangulation of a simple polygon, returned counter-clockwise.
///
/// A convex polygon falls out of the same loop; a self-intersecting or
/// degenerate profile returns `None` so the caller can report the loss.
fn triangulate(points: &[[f64; 2]]) -> Option<Vec<[usize; 3]>> {
    let count = points.len();
    if count < 3 {
        return None;
    }
    let area = signed_area(points);
    if area.abs() < 1e-12 {
        return None;
    }
    let mut remaining: Vec<usize> = (0..count).collect();
    if area < 0.0 {
        remaining.reverse();
    }
    let mut triangles = Vec::with_capacity(count - 2);
    let mut guard = 0usize;
    while remaining.len() > 3 {
        guard += 1;
        if guard > count * count {
            return None;
        }
        let length = remaining.len();
        let mut clipped = false;
        for index in 0..length {
            let previous = remaining[(index + length - 1) % length];
            let current = remaining[index];
            let next = remaining[(index + 1) % length];
            let (a, b, c) = (points[previous], points[current], points[next]);
            if cross(a, b, c) <= 1e-12 {
                continue;
            }
            if remaining.iter().copied().any(|candidate| {
                candidate != previous && candidate != current && candidate != next && inside_triangle(points[candidate], a, b, c)
            }) {
                continue;
            }
            triangles.push([previous, current, next]);
            remaining.remove(index);
            clipped = true;
            break;
        }
        if !clipped {
            return None;
        }
    }
    triangles.push([remaining[0], remaining[1], remaining[2]]);
    Some(triangles)
}

fn stl_triangle(out: &mut String, a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> bool {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let normal = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
    let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
    if length < 1e-12 {
        return false;
    }
    let normal = [normal[0] / length, normal[1] / length, normal[2] / length];
    out.push_str(&format!("  facet normal {} {} {}\n    outer loop\n", num(normal[0]), num(normal[1]), num(normal[2])));
    for vertex in [a, b, c] {
        out.push_str(&format!("      vertex {} {} {}\n", num(vertex[0]), num(vertex[1]), num(vertex[2])));
    }
    out.push_str("    endloop\n  endfacet\n");
    true
}

/// Triangles of one closed profile extruded from `z = 0` to `z = height`.
///
/// The caps reuse the ear-clipping triangulation and each profile edge becomes
/// two side-wall triangles. The order is stable so the STL and STEP writers
/// project the same solid.
fn extrusion_triangles(profile: &[[f64; 2]], cap: &[[usize; 3]], height: f64) -> Vec<[[f64; 3]; 3]> {
    let mut triangles = Vec::with_capacity(cap.len() * 2 + profile.len() * 2);
    let point = |index: usize, z: f64| [profile[index][0], profile[index][1], z];
    for [a, b, c] in cap {
        triangles.push([point(*a, height), point(*b, height), point(*c, height)]);
        triangles.push([point(*a, 0.0), point(*c, 0.0), point(*b, 0.0)]);
    }
    for index in 0..profile.len() {
        let next = (index + 1) % profile.len();
        let a0 = [profile[index][0], profile[index][1], 0.0];
        let b0 = [profile[next][0], profile[next][1], 0.0];
        let b1 = [profile[next][0], profile[next][1], height];
        let a1 = [profile[index][0], profile[index][1], height];
        triangles.push([a0, b0, b1]);
        triangles.push([a0, b1, a1]);
    }
    triangles
}

/// Extrudes every closed polyline along +Z and writes an ASCII STL solid.
///
/// The projection is a solid, not a surface model: each closed profile becomes
/// two triangulated caps plus one side wall per edge. Circles, text and
/// dimensions are not extruded, and an open polyline is reported as a loss
/// instead of being closed silently.
pub(crate) fn stl(drawing: &Drawing, thickness: f64) -> StlReport {
    let height = if thickness.is_finite() && thickness > 0.0 { thickness } else { STL_DEFAULT_THICKNESS };
    let mut content = String::from("solid open-cad\n");
    let mut triangles = 0usize;
    let mut degenerate = 0usize;
    let mut losses = Vec::new();

    for entity in &drawing.entities {
        let (profile, closed) = match &entity.kind {
            EntityKind::Polyline { points, closed } => (closed_profile(points), *closed),
            EntityKind::Arc { center, radius, start_angle, end_angle } => {
                (closed_profile(&sample_arc(*center, radius.abs(), *start_angle, *end_angle)), false)
            }
            EntityKind::Ellipse { center, rx, ry, rotation, start_angle, end_angle } => {
                let sampled = sample_ellipse(*center, rx.abs(), ry.abs(), *rotation, *start_angle, *end_angle);
                let closed = arc_sweep(*start_angle, *end_angle) >= 360.0 - 1e-9;
                (closed_profile(&sampled), closed)
            }
            _ => continue,
        };
        if !closed {
            losses.push(Loss::warning(
                "stl/open-profile",
                Some(&entity.layer),
                Some(&entity.id),
                format!("profile `{}` is not closed and was not extruded", entity.id),
            ));
            continue;
        }
        if profile.len() < 3 {
            losses.push(Loss::warning(
                "stl/degenerate-triangle",
                Some(&entity.layer),
                Some(&entity.id),
                format!("polyline `{}` has fewer than three distinct vertices", entity.id),
            ));
            continue;
        }
        let Some(cap) = triangulate(&profile)
        else {
            losses.push(Loss::warning(
                "stl/invalid-profile",
                Some(&entity.layer),
                Some(&entity.id),
                format!("polyline `{}` could not be triangulated", entity.id),
            ));
            continue;
        };

        for triangle in extrusion_triangles(&profile, &cap, height) {
            if stl_triangle(&mut content, triangle[0], triangle[1], triangle[2]) {
                triangles += 1;
            }
            else {
                degenerate += 1;
            }
        }
    }

    if degenerate > 0 {
        losses.push(Loss::warning("stl/degenerate-triangle", None, None, format!("{degenerate} degenerate triangle(s) were skipped")));
    }
    content.push_str("endsolid\n");
    StlReport { content, triangles, losses }
}

/// Result of one OBJ projection.
pub(crate) struct ObjReport {
    /// Wavefront OBJ text.
    pub(crate) content: String,
    /// Number of triangles written.
    pub(crate) triangles: usize,
    /// Profiles or triangles the projection could not keep.
    pub(crate) losses: Vec<Loss>,
}

/// Extrudes every closed profile along +Z and writes a Wavefront OBJ mesh.
///
/// The projection uses the same closed-profile selection and extrusion as the
/// STL writer, so both formats describe the same solid. Circles, text and
/// dimensions are not extruded, and an open profile is reported with
/// `obj/open-profile` instead of being closed silently.
pub(crate) fn obj(drawing: &Drawing, thickness: f64) -> ObjReport {
    let height = if thickness.is_finite() && thickness > 0.0 { thickness } else { STL_DEFAULT_THICKNESS };
    let mut content = String::from("# OpenCAD OBJ extrusion\n");
    let mut vertices = 0usize;
    let mut triangles = 0usize;
    let mut degenerate = 0usize;
    let mut losses = Vec::new();

    for entity in &drawing.entities {
        let (profile, closed) = match &entity.kind {
            EntityKind::Polyline { points, closed } => (closed_profile(points), *closed),
            EntityKind::Arc { center, radius, start_angle, end_angle } => {
                (closed_profile(&sample_arc(*center, radius.abs(), *start_angle, *end_angle)), false)
            }
            EntityKind::Ellipse { center, rx, ry, rotation, start_angle, end_angle } => {
                let sampled = sample_ellipse(*center, rx.abs(), ry.abs(), *rotation, *start_angle, *end_angle);
                let closed = arc_sweep(*start_angle, *end_angle) >= 360.0 - 1e-9;
                (closed_profile(&sampled), closed)
            }
            _ => continue,
        };
        if !closed {
            losses.push(Loss::warning(
                "obj/open-profile",
                Some(&entity.layer),
                Some(&entity.id),
                format!("profile `{}` is not closed and was not extruded", entity.id),
            ));
            continue;
        }
        if profile.len() < 3 {
            losses.push(Loss::warning(
                "obj/degenerate-triangle",
                Some(&entity.layer),
                Some(&entity.id),
                format!("polyline `{}` has fewer than three distinct vertices", entity.id),
            ));
            continue;
        }
        let Some(cap) = triangulate(&profile)
        else {
            losses.push(Loss::warning(
                "obj/invalid-profile",
                Some(&entity.layer),
                Some(&entity.id),
                format!("polyline `{}` could not be triangulated", entity.id),
            ));
            continue;
        };

        for triangle in extrusion_triangles(&profile, &cap, height) {
            let u = [triangle[1][0] - triangle[0][0], triangle[1][1] - triangle[0][1], triangle[1][2] - triangle[0][2]];
            let v = [triangle[2][0] - triangle[0][0], triangle[2][1] - triangle[0][1], triangle[2][2] - triangle[0][2]];
            let normal = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
            if (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt() < 1e-12 {
                degenerate += 1;
                continue;
            }
            for vertex in triangle {
                content.push_str(&format!("v {} {} {}\n", num(vertex[0]), num(vertex[1]), num(vertex[2])));
            }
            vertices += 3;
            content.push_str(&format!("f {} {} {}\n", vertices - 2, vertices - 1, vertices));
            triangles += 1;
        }
    }

    if degenerate > 0 {
        losses.push(Loss::warning("obj/degenerate-triangle", None, None, format!("{degenerate} degenerate triangle(s) were skipped")));
    }
    content.push_str("# end of OpenCAD OBJ\n");
    ObjReport { content, triangles, losses }
}

/// Result of one STEP projection.
pub(crate) struct StepReport {
    /// ISO-10303-21 text.
    pub(crate) content: String,
    /// Number of planar faces written.
    pub(crate) faces: usize,
    /// Profiles or triangles the projection could not keep.
    pub(crate) losses: Vec<Loss>,
}

/// Formats a STEP real literal: always a decimal point, no exponent notation.
fn step_num(value: f64) -> String {
    if !value.is_finite() {
        return "0.".to_string();
    }
    let mut text = format!("{value:.6}");
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.push('0');
        }
    }
    if text == "-0." || text == "-0.0" { "0.".to_string() } else { text }
}

fn step_cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn step_normalize(vector: [f64; 3]) -> Option<[f64; 3]> {
    let length = (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt();
    if !length.is_finite() || length < 1e-12 { None } else { Some([vector[0] / length, vector[1] / length, vector[2] / length]) }
}

/// Sequential STEP entity allocator; ids follow insertion order, so the same
/// drawing always produces the same instance names.
struct StepBuilder {
    lines: Vec<String>,
    next_id: usize,
}

impl StepBuilder {
    fn new() -> Self {
        Self { lines: Vec::new(), next_id: 1 }
    }

    fn add(&mut self, body: impl Into<String>) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        let body: String = body.into();
        self.lines.push(format!("#{id} = {body};"));
        id
    }
}

fn step_length_unit(builder: &mut StepBuilder, units: Units) -> usize {
    let millimetre = builder.add("(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.))");
    match units {
        Units::Mm => millimetre,
        Units::Inch => {
            let dimensions = builder.add("DIMENSIONAL_EXPONENTS(1.,0.,0.,0.,0.,0.,0.)");
            let measure = builder.add(format!("LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(25.4),#{millimetre})"));
            builder.add(format!("(CONVERSION_BASED_UNIT('INCH',#{measure}) LENGTH_UNIT() NAMED_UNIT(#{dimensions}))"))
        }
    }
}

/// Writes one triangle as an `ADVANCED_FACE` over a planar `POLY_LOOP`.
///
/// Returns `None` for a degenerate (zero-area) triangle so the caller can
/// report it instead of emitting an unusable face.
fn step_face(builder: &mut StepBuilder, triangle: [[f64; 3]; 3]) -> Option<usize> {
    let [a, b, c] = triangle;
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let normal = step_normalize(step_cross(u, v))?;
    let reference = step_normalize(u)?;

    let point = |builder: &mut StepBuilder, vertex: [f64; 3]| {
        builder.add(format!("CARTESIAN_POINT('',({},{},{}))", step_num(vertex[0]), step_num(vertex[1]), step_num(vertex[2])))
    };
    let p0 = point(builder, a);
    let p1 = point(builder, b);
    let p2 = point(builder, c);
    let axis = builder.add(format!("DIRECTION('',({},{},{}))", step_num(normal[0]), step_num(normal[1]), step_num(normal[2])));
    let ref_direction =
        builder.add(format!("DIRECTION('',({},{},{}))", step_num(reference[0]), step_num(reference[1]), step_num(reference[2])));
    let placement = builder.add(format!("AXIS2_PLACEMENT_3D('',#{p0},#{axis},#{ref_direction})"));
    let plane = builder.add(format!("PLANE('',#{placement})"));
    let poly_loop = builder.add(format!("POLY_LOOP('',(#{p0},#{p1},#{p2}))"));
    let bound = builder.add(format!("FACE_OUTER_BOUND('',#{poly_loop},.T.)"));
    Some(builder.add(format!("ADVANCED_FACE('',(#{bound}),#{plane},.T.)")))
}

/// Extrudes every closed polyline along +Z and writes AP203-style STEP text.
///
/// The projection is a faceted BREP, not a modelling kernel: every triangle of
/// one extrusion becomes one planar `ADVANCED_FACE`, and each closed profile
/// becomes its own `CLOSED_SHELL` inside a `MANIFOLD_SOLID_BREP`, so disjoint
/// profiles stay separate solids. There are no booleans and no analytic
/// surfaces. Circles, text and dimensions are not extruded, and an open polyline
/// is reported with `step/open-profile` instead of being closed silently.
pub(crate) fn step(drawing: &Drawing, thickness: f64) -> StepReport {
    let height = if thickness.is_finite() && thickness > 0.0 { thickness } else { STL_DEFAULT_THICKNESS };
    let mut builder = StepBuilder::new();
    let mut losses = Vec::new();
    let mut solids: Vec<usize> = Vec::new();
    let mut faces = 0usize;
    let mut degenerate = 0usize;

    let length_unit = step_length_unit(&mut builder, drawing.units);
    let uncertainty = builder.add(format!("UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-6),#{length_unit},'distance_accuracy_value','')"));
    let angle_unit = builder.add("(NAMED_UNIT(*) PLANE_ANGLE_UNIT() SI_UNIT($,.RADIAN.))");
    let solid_angle_unit = builder.add("(NAMED_UNIT(*) SI_UNIT($,.STERADIAN.) SOLID_ANGLE_UNIT())");
    let context = builder.add(format!(
        "(GEOMETRIC_REPRESENTATION_CONTEXT(3) GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#{uncertainty})) GLOBAL_UNIT_ASSIGNED_CONTEXT((#{length_unit},#{angle_unit},#{solid_angle_unit})) REPRESENTATION_CONTEXT('OpenCAD',''))"
    ));

    let application_context = builder.add("APPLICATION_CONTEXT('configuration controlled 3d designs of mechanical parts and assemblies')");
    builder.add(format!("APPLICATION_PROTOCOL_DEFINITION('international standard','config_control_design',1994,#{application_context})"));
    let product_context = builder.add(format!("PRODUCT_CONTEXT('',#{application_context},'mechanical')"));
    let product = builder.add(format!("PRODUCT('open-cad','OpenCAD part','',(#{product_context}))"));
    let formation = builder.add(format!("PRODUCT_DEFINITION_FORMATION_WITH_SPECIFIED_SOURCE('','',#{product},.NOT_KNOWN.)"));
    let definition_context = builder.add(format!("PRODUCT_DEFINITION_CONTEXT('part definition',#{application_context},'design')"));
    let definition = builder.add(format!("PRODUCT_DEFINITION('design','',#{formation},#{definition_context})"));
    let definition_shape = builder.add(format!("PRODUCT_DEFINITION_SHAPE('','',#{definition})"));

    let origin = builder.add("CARTESIAN_POINT('',(0.,0.,0.))");
    let z_axis = builder.add("DIRECTION('',(0.,0.,1.))");
    let x_axis = builder.add("DIRECTION('',(1.,0.,0.))");
    let shape_axis = builder.add(format!("AXIS2_PLACEMENT_3D('',#{origin},#{z_axis},#{x_axis})"));

    for entity in &drawing.entities {
        let EntityKind::Polyline { points, closed } = &entity.kind
        else {
            continue;
        };
        if !*closed {
            losses.push(Loss::warning(
                "step/open-profile",
                Some(&entity.layer),
                Some(&entity.id),
                format!("polyline `{}` is not closed and was not extruded", entity.id),
            ));
            continue;
        }
        let profile = closed_profile(points);
        if profile.len() < 3 {
            losses.push(Loss::warning(
                "step/degenerate-triangle",
                Some(&entity.layer),
                Some(&entity.id),
                format!("polyline `{}` has fewer than three distinct vertices", entity.id),
            ));
            continue;
        }
        let Some(cap) = triangulate(&profile)
        else {
            losses.push(Loss::warning(
                "step/invalid-profile",
                Some(&entity.layer),
                Some(&entity.id),
                format!("polyline `{}` could not be triangulated", entity.id),
            ));
            continue;
        };

        let mut profile_faces = Vec::new();
        for triangle in extrusion_triangles(&profile, &cap, height) {
            match step_face(&mut builder, triangle) {
                Some(face) => profile_faces.push(face),
                None => degenerate += 1,
            }
        }
        if profile_faces.is_empty() {
            continue;
        }
        faces += profile_faces.len();
        let shell =
            builder.add(format!("CLOSED_SHELL('',({}))", profile_faces.iter().map(|id| format!("#{id}")).collect::<Vec<_>>().join(",")));
        solids.push(builder.add(format!("MANIFOLD_SOLID_BREP('OpenCAD extrusion',#{shell})")));
    }

    let shape_items = solids.iter().map(|id| format!("#{id}")).chain(std::iter::once(format!("#{shape_axis}"))).collect::<Vec<_>>().join(",");
    let shape_representation = builder.add(format!("SHAPE_REPRESENTATION('',({shape_items}),#{context})"));
    builder.add(format!("SHAPE_DEFINITION_REPRESENTATION(#{definition_shape},#{shape_representation})"));

    if degenerate > 0 {
        losses.push(Loss::warning("step/degenerate-triangle", None, None, format!("{degenerate} degenerate triangle(s) were skipped")));
    }

    let mut content = String::from("ISO-10303-21;\n");
    content.push_str("HEADER;\n");
    content.push_str("FILE_DESCRIPTION(('OpenCAD faceted BREP projection'),'2;1');\n");
    content.push_str("FILE_NAME('open-cad.step','1970-01-01T00:00:00',('OpenCAD'),('OpenCAD'),'OpenCAD exchange writer','OpenCAD','');\n");
    content.push_str("FILE_SCHEMA(('CONFIG_CONTROL_DESIGN'));\n");
    content.push_str("ENDSEC;\n");
    content.push_str("DATA;\n");
    for line in &builder.lines {
        content.push_str(line);
        content.push('\n');
    }
    content.push_str("ENDSEC;\n");
    content.push_str("END-ISO-10303-21;\n");

    StepReport { content, faces, losses }
}

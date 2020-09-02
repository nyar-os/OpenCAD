//! Exchange writers.
//!
//! SVG keeps every supported entity and each layer's colour. DXF is ASCII R12
//! with a HEADER, a LAYER table and a LINE/CIRCLE/ARC/ELLIPSE/TEXT/POLYLINE
//! entity section; polylines keep their vertices as POLYLINE/VERTEX/SEQEND and
//! dimensions are written as primitives whose approximation is reported as a
//! loss. PDF writes a minimal vector page with a real xref table and reports
//! characters the standard Helvetica encoding cannot carry. Arcs and ellipses
//! are written as SVG path arcs, as DXF analytic entities, as PDF cubic Bezier
//! segments and as five-degree polyline samples when extruded to STL.

use serde::Serialize;

use crate::schema::{Drawing, Entity, EntityKind, Units, arc_point, arc_sweep, ellipse_point, offset_points, sample_arc, sample_ellipse};

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

#[derive(Debug, Clone, Copy)]
struct Bounds {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
    seen: bool,
}

impl Bounds {
    fn empty() -> Self {
        Self { min_x: f64::INFINITY, min_y: f64::INFINITY, max_x: f64::NEG_INFINITY, max_y: f64::NEG_INFINITY, seen: false }
    }

    fn add(&mut self, point: [f64; 2]) {
        if !point[0].is_finite() || !point[1].is_finite() {
            return;
        }
        self.min_x = self.min_x.min(point[0]);
        self.min_y = self.min_y.min(point[1]);
        self.max_x = self.max_x.max(point[0]);
        self.max_y = self.max_y.max(point[1]);
        self.seen = true;
    }

    fn add_entity(&mut self, entity: &Entity) {
        for point in entity.bounds_points() {
            self.add(point);
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
        let mut bounds = Bounds::empty();
        for entity in &drawing.entities {
            if drawing.layer(&entity.layer).is_some_and(|layer| layer.visible) {
                bounds.add_entity(entity);
            }
        }
        if bounds.seen {
            let width = (bounds.max_x - bounds.min_x).max(1.0);
            let height = (bounds.max_y - bounds.min_y).max(1.0);
            let pad = (width.max(height) * 0.05).max(1.0);
            Self { min_x: bounds.min_x - pad, min_y: bounds.min_y - pad, width: width + pad * 2.0, height: height + pad * 2.0 }
        }
        else {
            Self { min_x: 0.0, min_y: 0.0, width: 297.0, height: 210.0 }
        }
    }
}

fn svg_document(drawing: &Drawing, frame: Frame) -> String {
    let Frame { min_x, min_y, width, height } = frame;
    let flip_axis = 2.0 * min_y + height;

    let unit = match drawing.units {
        Units::Mm => "mm",
        Units::Inch => "in",
    };
    let stroke = stroke_width(drawing.units);

    let mut body = String::new();
    body.push_str(&format!("<g fill=\"none\" stroke-width=\"{}\" stroke-linecap=\"round\" stroke-linejoin=\"round\">\n", num(stroke)));
    for layer in &drawing.layers {
        if !layer.visible {
            continue;
        }
        let color = escape_xml(&layer.color);
        body.push_str(&format!("  <g id=\"layer-{}\" stroke=\"{}\">\n", escape_xml(&layer.id), color));
        for entity in drawing.entities.iter().filter(|entity| entity.layer == layer.id) {
            render_svg_entity(&mut body, entity, &color, flip_axis, stroke);
        }
        body.push_str("  </g>\n");
    }
    body.push_str("</g>\n");

    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" version=\"1.1\" viewBox=\"{} {} {} {}\" width=\"{}{}\" height=\"{}{}\" data-open-cad=\"drawing\" data-revision=\"{}\">\n<title>{}</title>\n{}</svg>\n",
        num(min_x),
        num(min_y),
        num(width),
        num(height),
        num(width),
        unit,
        num(height),
        unit,
        drawing.revision,
        escape_xml(drawing.title.as_deref().unwrap_or("Untitled drawing")),
        body
    )
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

fn svg_y(point: [f64; 2], flip_axis: f64) -> f64 {
    flip_axis - point[1]
}

/// Parameters of one SVG path made of elliptical-arc commands.
struct SvgArc {
    start_point: [f64; 2],
    sweep: f64,
    start_angle: f64,
    rx: f64,
    ry: f64,
    axis_rotation: f64,
}

/// Appends one SVG elliptical-arc command and returns the full path data.
///
/// The drawing is counter-clockwise in mathematical coordinates; the SVG
/// writer flips Y, so the screen-space sweep flag is always `0`.
fn svg_arc_path(arc: SvgArc, flip_axis: f64, point: impl Fn(f64) -> [f64; 2]) -> String {
    let mut path = format!("M {} {}", num(arc.start_point[0]), num(flip_axis - arc.start_point[1]));
    let mut remaining = arc.sweep;
    let mut angle = arc.start_angle;
    while remaining > 1e-9 {
        let chunk = remaining.min(180.0);
        angle += chunk;
        let end = point(angle);
        path.push_str(&format!(
            " A {} {} {} {} 0 {} {}",
            num(arc.rx),
            num(arc.ry),
            num(arc.axis_rotation),
            if chunk > 180.0 { 1 } else { 0 },
            num(end[0]),
            num(flip_axis - end[1])
        ));
        remaining -= chunk;
    }
    path
}

fn render_svg_entity(body: &mut String, entity: &Entity, color: &str, flip_axis: f64, stroke: f64) {
    match &entity.kind {
        EntityKind::Line { from, to } => {
            body.push_str(&format!(
                "    <line data-id=\"{}\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" />\n",
                escape_xml(&entity.id),
                num(from[0]),
                num(svg_y(*from, flip_axis)),
                num(to[0]),
                num(svg_y(*to, flip_axis))
            ));
        }
        EntityKind::Circle { center, radius } => {
            body.push_str(&format!(
                "    <circle data-id=\"{}\" cx=\"{}\" cy=\"{}\" r=\"{}\" />\n",
                escape_xml(&entity.id),
                num(center[0]),
                num(svg_y(*center, flip_axis)),
                num(radius.abs())
            ));
        }
        EntityKind::Arc { center, radius, start_angle, end_angle } => {
            let radius = radius.abs();
            let sweep = arc_sweep(*start_angle, *end_angle);
            let start = arc_point(*center, radius, *start_angle);
            let path = svg_arc_path(
                SvgArc { start_point: start, sweep, start_angle: *start_angle, rx: radius, ry: radius, axis_rotation: 0.0 },
                flip_axis,
                |angle| arc_point(*center, radius, angle),
            );
            body.push_str(&format!("    <path data-id=\"{}\" d=\"{path}\" />\n", escape_xml(&entity.id)));
        }
        EntityKind::Ellipse { center, rx, ry, rotation, start_angle, end_angle } => {
            let rx = rx.abs();
            let ry = ry.abs();
            let sweep = arc_sweep(*start_angle, *end_angle);
            let start = ellipse_point(*center, rx, ry, *rotation, *start_angle);
            let path = svg_arc_path(
                SvgArc { start_point: start, sweep, start_angle: *start_angle, rx, ry, axis_rotation: -*rotation },
                flip_axis,
                |parameter| ellipse_point(*center, rx, ry, *rotation, parameter),
            );
            body.push_str(&format!("    <path data-id=\"{}\" d=\"{path}\" />\n", escape_xml(&entity.id)));
        }
        EntityKind::Point { at } => {
            body.push_str(&format!(
                "    <circle data-id=\"{}\" cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"{color}\" stroke=\"none\" />\n",
                escape_xml(&entity.id),
                num(at[0]),
                num(svg_y(*at, flip_axis)),
                num(stroke * 2.0)
            ));
        }
        EntityKind::Polyline { points, closed } => {
            let points =
                points.iter().map(|point| format!("{},{}", num(point[0]), num(svg_y(*point, flip_axis)))).collect::<Vec<_>>().join(" ");
            let tag = if *closed { "polygon" } else { "polyline" };
            body.push_str(&format!("    <{tag} data-id=\"{}\" points=\"{points}\" />\n", escape_xml(&entity.id)));
        }
        EntityKind::Text { at, height, content } => {
            body.push_str(&format!(
                "    <text data-id=\"{}\" x=\"{}\" y=\"{}\" font-size=\"{}\" fill=\"{color}\" stroke=\"none\" font-family=\"sans-serif\">{}</text>\n",
                escape_xml(&entity.id),
                num(at[0]),
                num(svg_y(*at, flip_axis)),
                num(height.abs()),
                escape_xml(content)
            ));
        }
        EntityKind::Dimension { from, to, offset, text } => {
            let (p1, p2) = offset_points(*from, *to, *offset);
            let label = dimension_label(*from, *to, text);
            body.push_str(&format!(
                "    <g data-id=\"{}\" class=\"dimension\">\n      <line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" />\n      <line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" />\n      <line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" />\n      <text x=\"{}\" y=\"{}\" font-size=\"{}\" fill=\"{color}\" stroke=\"none\" font-family=\"sans-serif\">{}</text>\n    </g>\n",
                escape_xml(&entity.id),
                num(from[0]),
                num(svg_y(*from, flip_axis)),
                num(p1[0]),
                num(svg_y(p1, flip_axis)),
                num(to[0]),
                num(svg_y(*to, flip_axis)),
                num(p2[0]),
                num(svg_y(p2, flip_axis)),
                num(p1[0]),
                num(svg_y(p1, flip_axis)),
                num(p2[0]),
                num(svg_y(p2, flip_axis)),
                num((p1[0] + p2[0]) / 2.0),
                num(svg_y([(p1[0] + p2[0]) / 2.0, (p1[1] + p2[1]) / 2.0], flip_axis) - 1.0),
                num(stroke * 3.5),
                escape_xml(&label)
            ));
        }
    }
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

fn pdf_color(value: &str) -> (f64, f64, f64) {
    let digits = value.trim().trim_start_matches('#').to_ascii_lowercase();
    let expanded = match digits.len() {
        3 => digits.chars().flat_map(|character| [character, character]).collect::<String>(),
        6 => digits,
        _ => return (0.12, 0.16, 0.2),
    };
    let channel = |range: std::ops::Range<usize>| u8::from_str_radix(&expanded[range], 16).map(|value| f64::from(value) / 255.0).unwrap_or(0.0);
    (channel(0..2), channel(2..4), channel(4..6))
}

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

fn pdf_line(content: &mut String, from: [f64; 2], to: [f64; 2], map: impl Fn([f64; 2]) -> [f64; 2]) {
    let a = map(from);
    let b = map(to);
    content.push_str(&format!("{} {} m {} {} l S\n", num(a[0]), num(a[1]), num(b[0]), num(b[1])));
}

fn pdf_circle(content: &mut String, center: [f64; 2], radius: f64, map: impl Fn([f64; 2]) -> [f64; 2], scale: f64) {
    let c = map(center);
    let r = radius.abs() * scale;
    if r <= 0.0 {
        return;
    }
    let k = 0.552_284_749_830_793_6 * r;
    let (cx, cy) = (c[0], c[1]);
    content.push_str(&format!("{} {} m\n", num(cx + r), num(cy)));
    content.push_str(&format!("{} {} {} {} {} {} c\n", num(cx + r), num(cy + k), num(cx + k), num(cy + r), num(cx), num(cy + r)));
    content.push_str(&format!("{} {} {} {} {} {} c\n", num(cx - k), num(cy + r), num(cx - r), num(cy + k), num(cx - r), num(cy)));
    content.push_str(&format!("{} {} {} {} {} {} c\n", num(cx - r), num(cy - k), num(cx - k), num(cy - r), num(cx), num(cy - r)));
    content.push_str(&format!("{} {} {} {} {} {} c\nS\n", num(cx + k), num(cy - r), num(cx + r), num(cy - k), num(cx + r), num(cy)));
}

/// Approximates a parametric curve with cubic Bezier segments.
///
/// The sweep is split into pieces of at most 90 degrees; each cubic uses the
/// parametric derivative at both ends, so circles and ellipses keep their
/// tangent directions. `sample` and `derivative` take radians.
fn pdf_curve(
    content: &mut String,
    start_degrees: f64,
    sweep_degrees: f64,
    sample: impl Fn(f64) -> [f64; 2],
    derivative: impl Fn(f64) -> [f64; 2],
    map: impl Fn([f64; 2]) -> [f64; 2],
) {
    let start = start_degrees.to_radians();
    let sweep = sweep_degrees.to_radians();
    if sweep.abs() < 1e-9 {
        return;
    }
    let steps = (sweep.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0);
    let first = map(sample(start));
    content.push_str(&format!("{} {} m\n", num(first[0]), num(first[1])));
    for index in 0..steps as usize {
        let t0 = start + sweep * index as f64 / steps;
        let t1 = start + sweep * (index + 1) as f64 / steps;
        let step = t1 - t0;
        let p0 = sample(t0);
        let p3 = sample(t1);
        let d0 = derivative(t0);
        let d1 = derivative(t1);
        let c1 = map([p0[0] + d0[0] * step / 3.0, p0[1] + d0[1] * step / 3.0]);
        let c2 = map([p3[0] - d1[0] * step / 3.0, p3[1] - d1[1] * step / 3.0]);
        let end = map(p3);
        content.push_str(&format!("{} {} {} {} {} {} c\n", num(c1[0]), num(c1[1]), num(c2[0]), num(c2[1]), num(end[0]), num(end[1])));
    }
    content.push_str("S\n");
}

fn pdf_text_run(content: &mut String, text: &str, at: [f64; 2], size: f64, map: impl Fn([f64; 2]) -> [f64; 2]) -> usize {
    let (escaped, replaced) = pdf_text(text);
    let point = map(at);
    content.push_str(&format!("BT /F1 {} Tf {} {} Td ({}) Tj ET\n", num(size.max(0.5)), num(point[0]), num(point[1]), escaped));
    replaced
}

/// Renders one drawing to a minimal vector PDF and reports text losses.
///
/// The page is A4 landscape. Content is fitted into the printable area with a
/// uniform scale, so the output is a drawing sheet rather than a 1:1 plot.
pub(crate) fn pdf(drawing: &Drawing) -> (String, Vec<Loss>) {
    let mut bounds = Bounds::empty();
    for entity in &drawing.entities {
        if drawing.layer(&entity.layer).is_some_and(|layer| layer.visible) {
            bounds.add_entity(entity);
        }
    }
    let (min_x, min_y, width, height) = if bounds.seen {
        ((bounds.min_x), (bounds.min_y), (bounds.max_x - bounds.min_x).max(1.0), (bounds.max_y - bounds.min_y).max(1.0))
    }
    else {
        (0.0, 0.0, 297.0, 210.0)
    };
    let usable_width = PDF_PAGE_WIDTH - PDF_MARGIN * 2.0;
    let usable_height = PDF_PAGE_HEIGHT - PDF_MARGIN * 2.0;
    let scale = (usable_width / width).min(usable_height / height);
    let offset_x = (PDF_PAGE_WIDTH - width * scale) / 2.0 - min_x * scale;
    let offset_y = (PDF_PAGE_HEIGHT - height * scale) / 2.0 - min_y * scale;
    let map = |point: [f64; 2]| [point[0] * scale + offset_x, point[1] * scale + offset_y];

    let mut content = String::new();
    content.push_str("q\n");
    content.push_str(&format!("{} w\n1 J\n1 j\n", num(0.7)));
    let mut replaced = 0;
    for layer in &drawing.layers {
        if !layer.visible {
            continue;
        }
        let (red, green, blue) = pdf_color(&layer.color);
        content.push_str(&format!("{} {} {} RG\n", num(red), num(green), num(blue)));
        for entity in drawing.entities.iter().filter(|entity| entity.layer == layer.id) {
            match &entity.kind {
                EntityKind::Line { from, to } => pdf_line(&mut content, *from, *to, map),
                EntityKind::Circle { center, radius } => pdf_circle(&mut content, *center, *radius, map, scale),
                EntityKind::Arc { center, radius, start_angle, end_angle } => {
                    let radius = radius.abs();
                    let sweep = arc_sweep(*start_angle, *end_angle);
                    pdf_curve(
                        &mut content,
                        *start_angle,
                        sweep,
                        |angle| [center[0] + radius * angle.cos(), center[1] + radius * angle.sin()],
                        |angle| [-radius * angle.sin(), radius * angle.cos()],
                        map,
                    );
                }
                EntityKind::Ellipse { center, rx, ry, rotation, start_angle, end_angle } => {
                    let rx = rx.abs();
                    let ry = ry.abs();
                    let sweep = arc_sweep(*start_angle, *end_angle);
                    let (sin, cos) = rotation.to_radians().sin_cos();
                    let sample = |parameter: f64| {
                        let x = rx * parameter.cos();
                        let y = ry * parameter.sin();
                        [center[0] + x * cos - y * sin, center[1] + x * sin + y * cos]
                    };
                    let derivative = |parameter: f64| {
                        let x = -rx * parameter.sin();
                        let y = ry * parameter.cos();
                        [x * cos - y * sin, x * sin + y * cos]
                    };
                    pdf_curve(&mut content, *start_angle, sweep, sample, derivative, map);
                }
                EntityKind::Point { at } => pdf_circle(&mut content, *at, 0.5, map, scale),
                EntityKind::Polyline { points, closed } => {
                    if let Some(first) = points.first() {
                        let start = map(*first);
                        content.push_str(&format!("{} {} m\n", num(start[0]), num(start[1])));
                        for point in points.iter().skip(1) {
                            let next = map(*point);
                            content.push_str(&format!("{} {} l\n", num(next[0]), num(next[1])));
                        }
                        if *closed {
                            content.push_str("h\n");
                        }
                        content.push_str("S\n");
                    }
                }
                EntityKind::Text { at, height, content: text } => {
                    content.push_str(&format!("{} {} {} rg\n", num(red), num(green), num(blue)));
                    let size = height.abs() * scale;
                    replaced += pdf_text_run(&mut content, text, *at, size, map);
                    content.push_str(&format!("{} {} {} RG\n", num(red), num(green), num(blue)));
                }
                EntityKind::Dimension { from, to, offset, text } => {
                    let (p1, p2) = offset_points(*from, *to, *offset);
                    pdf_line(&mut content, *from, p1, map);
                    pdf_line(&mut content, *to, p2, map);
                    pdf_line(&mut content, p1, p2, map);
                    let label = dimension_label(*from, *to, text);
                    content.push_str(&format!("{} {} {} rg\n", num(red), num(green), num(blue)));
                    let midpoint = [(p1[0] + p2[0]) / 2.0, (p1[1] + p2[1]) / 2.0 + 1.75];
                    replaced += pdf_text_run(&mut content, &label, midpoint, 1.75 * scale, map);
                    content.push_str(&format!("{} {} {} RG\n", num(red), num(green), num(blue)));
                }
            }
        }
    }
    content.push_str("Q\n");

    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>",
            num(PDF_PAGE_WIDTH),
            num(PDF_PAGE_HEIGHT)
        ),
        format!("<< /Length {} >>\nstream\n{}endstream", content.len(), content),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string(),
    ];

    let mut out = String::from("%PDF-1.4\n");
    let mut offsets = Vec::with_capacity(objects.len());
    for (index, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.push_str(&format!("{} 0 obj\n{}\nendobj\n", index + 1, body));
    }
    let xref = out.len();
    out.push_str(&format!("xref\n0 {}\n", objects.len() + 1));
    out.push_str("0000000000 65535 f \n");
    for offset in offsets {
        out.push_str(&format!("{offset:010} 00000 n \n"));
    }
    out.push_str(&format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1));

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
    let double = points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let next = points[(index + 1) % points.len()];
            point[0] * next[1] - next[0] * point[1]
        })
        .sum::<f64>();
    double / 2.0
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

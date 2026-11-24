//! Exchange writers.
//!
//! SVG keeps every supported entity and each layer's colour. DXF is ASCII R12
//! with a HEADER, a LAYER table and a LINE/CIRCLE/ARC/ELLIPSE/TEXT/POLYLINE
//! entity section; polylines keep their vertices as POLYLINE/VERTEX/SEQEND and
//! linear dimensions are written as primitives whose approximation is reported
//! as a loss, while radial, diameter and ordinate dimensions keep their DXF
//! DIMENSION record. PDF writes a minimal vector page with a real xref table and reports
//! characters the standard Helvetica encoding cannot carry. SVG and PDF are
//! lowered onto the shared `document-drawing` scene; DXF keeps analytic arcs and
//! ellipses, and STL extrudes five-degree polyline samples.

use document_drawing::{Color, Drawing as FlatDrawing, Element, Point as FlatPoint, Shape, Size as FlatSize, Style};
use geometry_planar::{Point as PlanarPoint, Rect, signed_polygon_area};
use serde::Serialize;

use crate::schema::{
    CONTINUOUS_LINETYPE, DimensionStyle, Drawing, Entity, EntityKind, OrdinateAxis, Units, angular_points, angular_sweep, angular_value,
    arc_sweep, offset_points, ordinate_points, ordinate_value, radial_label, radial_target, sample_arc, sample_ellipse, sample_spline,
};

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

/// Margin between the sheet edge and the inner drawing frame border, in the
/// declared drawing units (10 mm / 0.5 in).
fn frame_margin(units: Units) -> f64 {
    match units {
        Units::Mm => 10.0,
        Units::Inch => 0.5,
    }
}

fn dimension_label(from: [f64; 2], to: [f64; 2], text: &Option<String>) -> String {
    if let Some(text) = text {
        return text.clone();
    }
    let distance = ((to[0] - from[0]).powi(2) + (to[1] - from[1]).powi(2)).sqrt();
    num(distance)
}

fn radial_dimension_label(radius: f64, diameter: bool, text: &Option<String>) -> String {
    if let Some(text) = text {
        return text.clone();
    }
    if diameter { format!("DIA{}", num(radius.abs() * 2.0)) } else { format!("R{}", num(radius.abs())) }
}

fn ordinate_dimension_label(origin: [f64; 2], feature: [f64; 2], axis: OrdinateAxis, text: &Option<String>) -> String {
    if let Some(text) = text {
        return text.clone();
    }
    num(ordinate_value(origin, feature, axis))
}

fn angular_dimension_label(center: [f64; 2], first: [f64; 2], second: [f64; 2], text: &Option<String>) -> String {
    if let Some(text) = text {
        return text.clone();
    }
    format!("{}°", num(angular_value(center, first, second)))
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
                let pad = (width.max(height) * 0.08).max(frame_margin(drawing.units) * 1.6);
                // The title block hangs from the inner border's bottom-right corner, so the
                // bottom padding must clear the block's full height plus a gap; otherwise a
                // tall part runs straight through the title rows.
                let sheet = match drawing.units {
                    Units::Mm => 1.0,
                    Units::Inch => 1.0 / 25.4,
                };
                let bottom = (frame_margin(drawing.units) + 28.0 * sheet).max(pad);
                Self {
                    min_x: extents.min.x - pad,
                    min_y: extents.min.y - bottom,
                    width: width + pad * 2.0,
                    height: height + pad + bottom,
                }
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
        if !drawing.layer(&entity.layer).is_some_and(|layer| layer.visible && !layer.frozen && layer.plottable) {
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

/// Compact palette used when a layer colour is written as an ACI index.
///
/// Each entry pairs the ACI number with the `#rrggbb` value the reader maps it
/// back to, so a layer colour that lands on one of these values round-trips.
const ACI_PALETTE: [(i64, [u8; 3]); 9] = [
    (1, [255, 0, 0]),
    (2, [255, 255, 0]),
    (3, [0, 255, 0]),
    (4, [0, 255, 255]),
    (5, [0, 0, 255]),
    (6, [255, 0, 255]),
    (7, [208, 215, 222]),
    (8, [128, 128, 128]),
    (9, [192, 192, 192]),
];

/// Parses a `#rrggbb` colour into its RGB channels.
fn rgb_of(value: &str) -> Option<[u8; 3]> {
    let digits = value.trim().trim_start_matches('#');
    if digits.len() != 6 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |range: std::ops::Range<usize>| u8::from_str_radix(&digits[range], 16).ok();
    Some([channel(0..2)?, channel(2..4)?, channel(4..6)?])
}

/// Maps a `#rrggbb` colour to the nearest ACI index in the compact palette.
///
/// The nearest index uses the squared Euclidean distance in RGB space. `None`
/// means the value is not a valid `#rrggbb` colour, which is the only case the
/// writer cannot represent.
fn nearest_aci(value: &str) -> Option<i64> {
    let rgb = rgb_of(value)?;
    ACI_PALETTE
        .iter()
        .map(|(index, color)| {
            let distance = color
                .iter()
                .zip(rgb)
                .map(|(left, right)| {
                    let delta = i32::from(*left) - i32::from(right);
                    delta * delta
                })
                .sum::<i32>();
            (*index, distance)
        })
        .min_by_key(|(_, distance)| *distance)
        .map(|(index, _)| index)
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

/// Rec. 709 luma of a colour over its gamma-encoded channels on a `0..=1`
/// scale, a cheap stand-in for perceived lightness on a white sheet.
fn luma(color: Color) -> f64 {
    (0.2126 * f64::from(color.red()) + 0.7152 * f64::from(color.green()) + 0.0722 * f64::from(color.blue())) / 255.0
}

/// Dark ink that light layer colours are blended toward on export.
const EXPORT_INK: Color = Color::rgb(31, 41, 51);
/// Layer colours brighter than this are too faint to read on a white sheet.
const LIGHT_INK_LIMIT: f64 = 0.7;
/// Luma that normalised light colours are blended down to.
const LEGIBLE_INK_LUMA: f64 = 0.35;

/// Normalises one export colour for legibility on the white SVG and PDF sheet.
///
/// Layer colours are authored for the dark viewport, so a light default such
/// as `#d0d7de` is nearly invisible once exported onto white. Colours already
/// dark enough are kept untouched; lighter ones are blended channel-wise
/// toward [`EXPORT_INK`] until they reach [`LEGIBLE_INK_LUMA`], which keeps
/// the original hue direction.
fn export_color(color: Color) -> Color {
    let lightness = luma(color);
    if lightness <= LIGHT_INK_LIMIT {
        return color;
    }
    let ink = luma(EXPORT_INK);
    let mix = (lightness - LEGIBLE_INK_LUMA) / (lightness - ink);
    let blend = |value: u8, target: u8| (f64::from(value) + (f64::from(target) - f64::from(value)) * mix).round().clamp(0.0, 255.0) as u8;
    Color::rgb(blend(color.red(), EXPORT_INK.red()), blend(color.green(), EXPORT_INK.green()), blend(color.blue(), EXPORT_INK.blue()))
}

/// How model coordinates become flat-drawing coordinates.
struct Projection<'a> {
    map: &'a dyn Fn([f64; 2]) -> FlatPoint,
    scale: f64,
    /// Default outline width in flat-drawing units.
    stroke: f64,
    /// Factor that converts a model length into flat-drawing units.
    lineweight_scale: f64,
    label_offset: f64,
    label_size: f64,
    /// Dimension styles consulted for arrowhead sizes; the first entry is the
    /// fallback for entities without a style reference.
    dim_styles: &'a [DimensionStyle],
}

/// Converts a layer lineweight in millimetres to drawing units.
fn lineweight_units(units: Units, lineweight: f64) -> f64 {
    match units {
        Units::Mm => lineweight,
        Units::Inch => lineweight / 25.4,
    }
}

/// Reduces a line type dash pattern to the single dash/gap pair the shared
/// vector scene carries.
///
/// A pattern with one entry repeats that entry as the gap, a longer pattern
/// averages its dash entries and its gap entries. A zero-length dash or gap
/// has no representation and the stroke stays solid.
fn dash_pair(pattern: &[f64], scale: f64) -> Option<[f64; 2]> {
    let dashes = pattern.iter().copied().filter(|dash| dash.is_finite() && *dash >= 0.0).collect::<Vec<_>>();
    if dashes.is_empty() {
        return None;
    }
    let even = dashes.iter().step_by(2).copied().collect::<Vec<_>>();
    let odd = dashes.iter().skip(1).step_by(2).copied().collect::<Vec<_>>();
    let on = even.iter().sum::<f64>() / even.len() as f64;
    let off = if odd.is_empty() { on } else { odd.iter().sum::<f64>() / odd.len() as f64 };
    if on <= 0.0 || off <= 0.0 {
        return None;
    }
    Some([on * scale, off * scale])
}

/// Outline width and dash pair of one layer, in flat-drawing units.
fn layer_stroke(drawing: &Drawing, layer: &crate::schema::Layer, projection: &Projection<'_>) -> (f64, Option<[f64; 2]>) {
    let width = if layer.lineweight > 0.0 {
        lineweight_units(drawing.units, layer.lineweight) * projection.lineweight_scale
    }
    else {
        projection.stroke
    };
    let dash =
        drawing.linetypes.iter().find(|line| line.id == layer.linetype).and_then(|line| dash_pair(&line.pattern, projection.lineweight_scale));
    (width, dash)
}

fn build_drawing(drawing: &Drawing, size: FlatSize, projection: &Projection<'_>) -> FlatDrawing {
    let mut flat = FlatDrawing::new(size);
    for entity in drawing.ordered_entities() {
        let Some(layer) = drawing.layer(&entity.layer)
        else {
            continue;
        };
        if !layer.visible || layer.frozen || !layer.plottable || !entity.is_finite() {
            continue;
        }
        let color = export_color(color_of(&layer.color));
        let (width, dash) = layer_stroke(drawing, layer, projection);
        let mut stroke_style = Style::stroke(color, width);
        stroke_style.dash = dash;
        let text_style = Style::fill(color);
        push_entity(&mut flat, entity, color, &stroke_style, &text_style, projection);
    }
    flat
}

/// Solid arrowhead length of a dimension, in flat-drawing units.
///
/// The style reference selects the dimension style; a missing or unknown id
/// falls back to the drawing's first style, and an empty style table uses the
/// `standard` arrow size of 2.5. The model length is scaled into flat-drawing
/// units so the arrow keeps its proportion on a fitted PDF page.
fn arrow_length(style: Option<&str>, projection: &Projection<'_>) -> f64 {
    let arrow_size = projection
        .dim_styles
        .iter()
        .find(|candidate| Some(candidate.id.as_str()) == style)
        .or_else(|| projection.dim_styles.first())
        .map_or(2.5, |style| style.arrow_size);
    arrow_size.max(0.0) * projection.scale
}

/// A solid paint without an outline, for arrowheads and centre marks.
///
/// `Style::fill` keeps the default black stroke, which would wrap every
/// filled triangle in a visible border.
fn solid(color: Color) -> Style {
    Style { stroke: None, ..Style::fill(color) }
}

/// Pushes one solid triangular arrowhead with its tip at `tip`.
///
/// `direction` is the way the arrow points; the triangle base sits `length`
/// behind the tip against the arrow axis. A degenerate direction or a
/// non-positive length is skipped so a collapsed dimension draws no spike.
fn push_arrowhead(
    flat: &mut FlatDrawing,
    map: &dyn Fn([f64; 2]) -> FlatPoint,
    color: Color,
    tip: [f64; 2],
    direction: [f64; 2],
    length: f64,
) {
    let norm = (direction[0] * direction[0] + direction[1] * direction[1]).sqrt();
    if norm <= f64::EPSILON || !length.is_finite() || length <= 0.0 {
        return;
    }
    let unit = [direction[0] / norm, direction[1] / norm];
    let normal = [-unit[1], unit[0]];
    let half = length * 0.35;
    let back = [tip[0] - unit[0] * length, tip[1] - unit[1] * length];
    let points =
        vec![map(tip), map([back[0] + normal[0] * half, back[1] + normal[1] * half]), map([back[0] - normal[0] * half, back[1] - normal[1] * half])];
    flat.push(Element::new(Shape::Polygon { points }, solid(color)));
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
        EntityKind::Spline { points, closed } => {
            let points = sample_spline(points, *closed).iter().map(|point| map(*point)).collect::<Vec<_>>();
            let shape = if *closed { Shape::Polygon { points } } else { Shape::Polyline { points } };
            flat.push(Element::new(shape, stroke_style.clone()));
        }
        EntityKind::Text { at, height, content } => {
            flat.push(Element::new(
                Shape::Text { at: map(*at), text: content.clone(), size: (height.abs() * scale).max(0.01) },
                text_style.clone(),
            ));
        }
        EntityKind::Dimension { from, to, offset, text, style } => {
            let (p1, p2) = offset_points(*from, *to, *offset);
            for (start, end) in [(*from, p1), (*to, p2), (p1, p2)] {
                flat.push(Element::new(Shape::Line { from: map(start), to: map(end) }, stroke_style.clone()));
            }
            let arrow = arrow_length(style.as_deref(), projection);
            push_arrowhead(flat, map, color, p1, [p1[0] - p2[0], p1[1] - p2[1]], arrow);
            push_arrowhead(flat, map, color, p2, [p2[0] - p1[0], p2[1] - p1[1]], arrow);
            let midpoint = [(p1[0] + p2[0]) / 2.0, (p1[1] + p2[1]) / 2.0 + projection.label_offset];
            flat.push(Element::new(
                Shape::Text { at: map(midpoint), text: dimension_label(*from, *to, text), size: projection.label_size.max(0.01) },
                text_style.clone(),
            ));
        }
        EntityKind::RadialDimension { center, radius, angle, offset, diameter, text } => {
            let target = radial_target(*center, *radius, *angle);
            let start = if *diameter { radial_target(*center, *radius, *angle + 180.0) } else { *center };
            flat.push(Element::new(Shape::Line { from: map(start), to: map(target) }, stroke_style.clone()));
            let arrow = arrow_length(None, projection);
            push_arrowhead(flat, map, color, target, [target[0] - center[0], target[1] - center[1]], arrow);
            if !*diameter {
                flat.push(Element::new(Shape::Circle { center: map(*center), radius: arrow * 0.18 }, solid(color)));
            }
            let label = radial_label(*center, *radius, *angle, *offset);
            flat.push(Element::new(
                Shape::Text { at: map(label), text: radial_dimension_label(*radius, *diameter, text), size: projection.label_size.max(0.01) },
                text_style.clone(),
            ));
        }
        EntityKind::OrdinateDimension { origin, feature, axis, offset, text } => {
            let (base, guide) = ordinate_points(*origin, *feature, *axis, *offset);
            flat.push(Element::new(Shape::Line { from: map(*feature), to: map(guide) }, stroke_style.clone()));
            flat.push(Element::new(Shape::Line { from: map(base), to: map(guide) }, stroke_style.clone()));
            let arrow = arrow_length(None, projection);
            push_arrowhead(flat, map, color, guide, [guide[0] - feature[0], guide[1] - feature[1]], arrow);
            let midpoint = [(base[0] + guide[0]) / 2.0, (base[1] + guide[1]) / 2.0 + projection.label_offset];
            flat.push(Element::new(
                Shape::Text {
                    at: map(midpoint),
                    text: ordinate_dimension_label(*origin, *feature, *axis, text),
                    size: projection.label_size.max(0.01),
                },
                text_style.clone(),
            ));
        }
        EntityKind::AngularDimension { center, first, second, offset, text } => {
            let radius = offset.abs();
            let (start, sweep) = angular_sweep(*center, *first, *second);
            let start_point = radial_target(*center, radius, start);
            let end_point = radial_target(*center, radius, start + sweep);
            let label_point = radial_target(*center, radius, start + sweep / 2.0);
            for (leg_from, leg_to) in [(*first, start_point), (*second, end_point)] {
                flat.push(Element::new(Shape::Line { from: map(leg_from), to: map(leg_to) }, stroke_style.clone()));
            }
            flat.push(Element::new(
                Shape::Arc { center: map(*center), radius: radius * scale, start_degrees: -start, end_degrees: -(start + sweep) },
                stroke_style.clone(),
            ));
            let arrow = arrow_length(None, projection);
            let start_radians = start.to_radians();
            let end_radians = (start + sweep).to_radians();
            push_arrowhead(flat, map, color, start_point, [start_radians.sin(), -start_radians.cos()], arrow);
            push_arrowhead(flat, map, color, end_point, [-end_radians.sin(), end_radians.cos()], arrow);
            flat.push(Element::new(
                Shape::Text {
                    at: map(label_point),
                    text: angular_dimension_label(*center, *first, *second, text),
                    size: projection.label_size.max(0.01),
                },
                text_style.clone(),
            ));
        }
        EntityKind::Hatch { points, .. } => {
            if points.len() < 3 {
                return;
            }
            let points = points.iter().map(|point| map(*point)).collect::<Vec<_>>();
            let mut style = stroke_style.clone();
            style.fill = Some(color);
            style.opacity = 0.35;
            flat.push(Element::new(Shape::Polygon { points }, style));
        }
    }
}

/// Pushes the engineering sheet frame and title block onto an SVG scene.
///
/// Two borders inset from the sheet edge follow drafting practice; the title
/// block sits in the lower-right corner of the inner border and reports the
/// drawing title, its author, company and project, the declared units and the
/// current revision. The shapes are placed in canvas coordinates directly
/// because the sheet, unlike the entities, is not projected from the model.
fn push_sheet_frame(flat: &mut FlatDrawing, drawing: &Drawing, frame: Frame, stroke: f64) {
    let margin = frame_margin(drawing.units);
    let border = Style::stroke(EXPORT_INK, stroke.max(0.35));
    let label = Style::fill(EXPORT_INK);
    let outer = margin * 0.4;
    let inner = margin.max(outer + stroke * 3.0);
    let rectangle = |x: f64, y: f64, width: f64, height: f64| Shape::Polygon {
        points: vec![
            FlatPoint::new(x, y),
            FlatPoint::new(x + width, y),
            FlatPoint::new(x + width, y + height),
            FlatPoint::new(x, y + height),
        ],
    };
    flat.push(Element::new(rectangle(outer, outer, frame.width - outer * 2.0, frame.height - outer * 2.0), border.clone()));
    flat.push(Element::new(rectangle(inner, inner, frame.width - inner * 2.0, frame.height - inner * 2.0), border.clone()));

    let sheet: f64 = match drawing.units {
        Units::Mm => 1.0,
        Units::Inch => 1.0 / 25.4,
    };
    let inner_width = (frame.width - inner * 2.0).max(0.0);
    let inner_height = (frame.height - inner * 2.0).max(0.0);
    let width = (180.0 * sheet).min(inner_width * 0.8);
    let height = (24.0 * sheet).min(inner_height * 0.6);
    if width < 1.0 || height < 1.0 {
        return;
    }
    let row = height / 3.0;
    let left = frame.width - inner - width;
    let top = frame.height - inner - height;
    let size = (row * 0.5).max(0.5);
    flat.push(Element::new(rectangle(left, top, width, height), border.clone()));
    for index in 1..3 {
        let y = top + row * index as f64;
        flat.push(Element::new(Shape::Line { from: FlatPoint::new(left, y), to: FlatPoint::new(left + width, y) }, border.clone()));
    }
    let meta_x = left + width * 0.55;
    flat.push(Element::new(
        Shape::Line { from: FlatPoint::new(meta_x, top + row * 2.0), to: FlatPoint::new(meta_x, top + height) },
        border.clone(),
    ));

    let byline = [drawing.properties.author.as_str(), drawing.properties.company.as_str(), drawing.properties.project.as_str()]
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" / ");
    let byline = if byline.is_empty() { String::from("Unattributed") } else { byline };
    let units_label = format!("Units: {}", drawing.units.as_str());
    let revision_label = format!("Rev {}", drawing.revision);
    let baseline = |index: usize| top + row * (index as f64 + 1.0) - row * 0.28;
    let cells = [
        (drawing.title.as_deref().unwrap_or("Untitled drawing"), left, 0usize),
        (byline.as_str(), left, 1),
        (units_label.as_str(), left, 2),
        (revision_label.as_str(), meta_x, 2),
    ];
    for (value, x, index) in cells {
        flat.push(Element::new(Shape::Text { at: FlatPoint::new(x + row * 0.3, baseline(index)), text: value.to_string(), size }, label.clone()));
    }
}

fn svg_document(drawing: &Drawing, frame: Frame) -> String {
    let stroke = stroke_width(drawing.units);
    let map = |point: [f64; 2]| frame.map(point);
    let projection = Projection {
        map: &map,
        scale: 1.0,
        stroke,
        lineweight_scale: 1.0,
        label_offset: 1.0,
        label_size: stroke * 3.5,
        dim_styles: &drawing.dim_styles,
    };
    let mut flat = build_drawing(drawing, FlatSize::new(frame.width, frame.height), &projection);
    push_sheet_frame(&mut flat, drawing, frame, stroke);
    let svg = flat.to_svg().unwrap_or_else(|_| {
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">\n</svg>\n",
            num(frame.width),
            num(frame.height),
            num(frame.width),
            num(frame.height)
        )
    });
    scale_svg_size(decorate_svg(svg, drawing), drawing.units)
}

/// Pixels per millimetre used for the rendered size of an exported SVG.
const SVG_PX_PER_MILLIMETRE: f64 = 8.0;

/// Scales the physical `width`/`height` of the SVG to readable pixels.
///
/// The shared writer emits `width`/`height` equal to the frame size in
/// drawing units, so a 133 mm sheet would open as a 133 px stamp. The
/// `viewBox` keeps the unit coordinates; only the rendered size changes,
/// matching the pixel canvas the other Open* exporters produce.
fn scale_svg_size(svg: String, units: Units) -> String {
    let factor = match units {
        Units::Mm => SVG_PX_PER_MILLIMETRE,
        Units::Inch => SVG_PX_PER_MILLIMETRE * 25.4,
    };
    let Some(tag_start) = svg.find("<svg")
    else {
        return svg;
    };
    let Some(tag_offset) = svg[tag_start..].find('>')
    else {
        return svg;
    };
    let tag_end = tag_start + tag_offset;
    let mut tag = svg[..tag_end].to_string();
    for attribute in ["width", "height"] {
        let needle = format!("{attribute}=\"");
        let Some(start) = tag.find(&needle)
        else {
            continue;
        };
        let value_start = start + needle.len();
        let Some(quote_offset) = tag[value_start..].find('"')
        else {
            continue;
        };
        if let Ok(number) = tag[value_start..value_start + quote_offset].parse::<f64>()
            && number.is_finite()
            && number > 0.0
        {
            let scaled = format!("{:.1}", number * factor);
            tag.replace_range(value_start..value_start + quote_offset, &scaled);
        }
    }
    tag.push_str(&svg[tag_end..]);
    tag
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

/// Writes one HATCH with a single external polygon boundary made of line edges.
///
/// The boundary is stored once and reused by every edge, which is how DXF
/// describes a closed polygonal path. The pattern name is written on code 2;
/// no pattern definition is emitted, so a non-solid pattern is reported as a
/// loss by the caller.
fn dxf_hatch(out: &mut String, layer: &str, points: &[[f64; 2]], pattern: &str) {
    let solid = pattern.eq_ignore_ascii_case("SOLID") || pattern.trim().is_empty();
    dxf_pair(out, 0, "HATCH");
    dxf_pair(out, 8, dxf_layer_name(layer));
    dxf_pair(out, 2, dxf_text(pattern));
    dxf_pair(out, 70, if solid { 1 } else { 0 });
    dxf_pair(out, 71, 0);
    dxf_pair(out, 91, 1);
    dxf_pair(out, 92, 1);
    dxf_pair(out, 72, 0);
    dxf_pair(out, 73, 1);
    dxf_pair(out, 93, points.len());
    for point in points {
        dxf_pair(out, 10, num_exact(point[0]));
        dxf_pair(out, 20, num_exact(point[1]));
    }
    dxf_pair(out, 97, 0);
    dxf_pair(out, 75, 0);
    dxf_pair(out, 76, 1);
    dxf_pair(out, 98, 0);
}

/// Writes one POLYLINE with VERTEX records and a closing SEQEND.
fn dxf_polyline(out: &mut String, layer: &str, points: &[[f64; 2]], closed: bool) {
    let layer = dxf_layer_name(layer);
    dxf_pair(out, 0, "POLYLINE");
    dxf_pair(out, 8, &layer);
    dxf_pair(out, 66, 1);
    dxf_pair(out, 10, "0");
    dxf_pair(out, 20, "0");
    dxf_pair(out, 30, "0");
    dxf_pair(out, 70, if closed { 1 } else { 0 });
    for point in points {
        dxf_pair(out, 0, "VERTEX");
        dxf_pair(out, 8, &layer);
        dxf_pair(out, 10, num(point[0]));
        dxf_pair(out, 20, num(point[1]));
        dxf_pair(out, 30, "0");
    }
    dxf_pair(out, 0, "SEQEND");
    dxf_pair(out, 8, &layer);
}

/// Writes one planar cubic SPLINE with a clamped uniform knot vector.
///
/// The fit points double as control points so readers that ignore fit points
/// still have a control polygon, and the fit points make our own reader and
/// other CAD tools pass the curve through the authored points.
fn dxf_spline(out: &mut String, layer: &str, points: &[[f64; 2]], closed: bool) {
    let layer = dxf_layer_name(layer);
    let (degree, knots) = clamped_spline_knots(points.len());
    dxf_pair(out, 0, "SPLINE");
    dxf_pair(out, 8, &layer);
    dxf_pair(out, 70, 8 | u32::from(closed));
    dxf_pair(out, 71, degree);
    dxf_pair(out, 72, knots.len());
    dxf_pair(out, 73, points.len());
    dxf_pair(out, 74, points.len());
    for knot in knots {
        dxf_pair(out, 40, num_exact(knot));
    }
    for point in points {
        dxf_pair(out, 10, num_exact(point[0]));
        dxf_pair(out, 20, num_exact(point[1]));
        dxf_pair(out, 30, "0");
    }
    for point in points {
        dxf_pair(out, 11, num_exact(point[0]));
        dxf_pair(out, 21, num_exact(point[1]));
        dxf_pair(out, 31, "0");
    }
}

/// Clamped uniform knot vector and degree for `count` fit points.
fn clamped_spline_knots(count: usize) -> (usize, Vec<f64>) {
    let degree = count.saturating_sub(1).min(3);
    let internal = count.saturating_sub(degree + 1);
    let mut knots = vec![0.0; degree + 1];
    knots.extend((1..=internal).map(|step| step as f64));
    let last = internal as f64 + 1.0;
    knots.extend(std::iter::repeat_n(last, degree + 1));
    (degree, knots)
}

/// Renders one drawing to the automatically chosen ASCII DXF version.
pub(crate) fn dxf(drawing: &Drawing) -> (String, Vec<Loss>) {
    dxf_with_flavor(drawing, false)
}

/// Renders one drawing to ASCII DXF R12 (`AC1009`).
///
/// Entities R12 cannot carry are simplified: an ellipse becomes a sampled
/// `POLYLINE` and a hatch becomes its closed boundary `POLYLINE`. Header and
/// layer values R12 does not know (`$INSUNITS`, lineweight and the plot flag)
/// are omitted and reported, so the file stays openable by AutoCAD R12.
pub(crate) fn dxf_r12(drawing: &Drawing) -> (String, Vec<Loss>) {
    dxf_with_flavor(drawing, true)
}

fn dxf_with_flavor(drawing: &Drawing, r12: bool) -> (String, Vec<Loss>) {
    let mut out = String::new();
    let mut losses = Vec::new();

    dxf_pair(&mut out, 0, "SECTION");
    dxf_pair(&mut out, 2, "HEADER");
    dxf_pair(&mut out, 9, "$ACADVER");
    let custom_lines = drawing.layers.iter().any(|layer| layer.linetype != CONTINUOUS_LINETYPE || layer.lineweight > 0.0);
    let layer_flags = drawing.layers.iter().any(|layer| !layer.plottable);
    let version = if r12 {
        "AC1009"
    }
    else if custom_lines
        || layer_flags
        || drawing.entities.iter().any(|entity| matches!(entity.kind, EntityKind::Hatch { .. } | EntityKind::Spline { .. }))
    {
        "AC1015"
    }
    else {
        "AC1009"
    };
    dxf_pair(&mut out, 1, version);
    if r12 {
        losses.push(Loss::warning(
            "dxf/r12-units",
            None,
            None,
            "the R12 header omits `$INSUNITS`, so the drawing unit is not written".to_string(),
        ));
    }
    else {
        dxf_pair(&mut out, 9, "$INSUNITS");
        dxf_pair(&mut out, 70, drawing.units.dxf_insunits());
    }
    dxf_pair(&mut out, 0, "ENDSEC");

    dxf_pair(&mut out, 0, "SECTION");
    dxf_pair(&mut out, 2, "TABLES");
    dxf_pair(&mut out, 0, "TABLE");
    dxf_pair(&mut out, 2, "LTYPE");
    dxf_pair(&mut out, 70, drawing.linetypes.len());
    for linetype in &drawing.linetypes {
        let total = linetype.pattern.iter().sum::<f64>();
        dxf_pair(&mut out, 0, "LTYPE");
        dxf_pair(&mut out, 2, dxf_text(&linetype.name));
        dxf_pair(&mut out, 70, 0);
        dxf_pair(&mut out, 3, dxf_text(&linetype.description));
        dxf_pair(&mut out, 72, 65);
        dxf_pair(&mut out, 73, linetype.pattern.len());
        dxf_pair(&mut out, 40, num_exact(total));
        for dash in &linetype.pattern {
            dxf_pair(&mut out, 49, num_exact(*dash));
        }
    }
    dxf_pair(&mut out, 0, "ENDTAB");
    dxf_pair(&mut out, 0, "TABLE");
    dxf_pair(&mut out, 2, "LAYER");
    dxf_pair(&mut out, 70, drawing.layers.len());
    for layer in &drawing.layers {
        dxf_pair(&mut out, 0, "LAYER");
        dxf_pair(&mut out, 2, dxf_layer_name(&layer.id));
        let mut flags = 0;
        if layer.frozen {
            flags |= 1;
        }
        if layer.locked {
            flags |= 4;
        }
        dxf_pair(&mut out, 70, flags);
        let aci = nearest_aci(&layer.color);
        dxf_pair(&mut out, 62, aci.unwrap_or(7));
        dxf_pair(&mut out, 6, dxf_text(&layer.linetype));
        if r12 {
            if layer.lineweight > 0.0 {
                losses.push(Loss::warning(
                    "dxf/r12-lineweight",
                    Some(&layer.id),
                    None,
                    format!("layer lineweight `{}` is not written in R12", num(layer.lineweight)),
                ));
            }
            if !layer.plottable {
                losses.push(Loss::warning(
                    "dxf/r12-plottable",
                    Some(&layer.id),
                    None,
                    "the R12 LAYER record has no plot flag, so the layer is treated as plottable".to_string(),
                ));
            }
        }
        else {
            dxf_pair(&mut out, 370, (layer.lineweight * 100.0).round() as i64);
            dxf_pair(&mut out, 290, i64::from(layer.plottable));
        }
        if aci.is_none() {
            losses.push(Loss::warning(
                "dxf/layer-color",
                Some(&layer.id),
                None,
                format!("layer colour `{}` is not a valid `#rrggbb` value and is written as ACI 7", layer.color),
            ));
        }
    }
    dxf_pair(&mut out, 0, "ENDTAB");
    dxf_pair(&mut out, 0, "TABLE");
    dxf_pair(&mut out, 2, "STYLE");
    dxf_pair(&mut out, 70, drawing.text_styles.len());
    for style in &drawing.text_styles {
        dxf_pair(&mut out, 0, "STYLE");
        dxf_pair(&mut out, 2, dxf_text(&style.name));
        dxf_pair(&mut out, 70, 0);
        dxf_pair(&mut out, 40, num_exact(style.height));
        dxf_pair(&mut out, 41, num_exact(style.width_factor));
        dxf_pair(&mut out, 50, num_exact(style.oblique));
        dxf_pair(&mut out, 3, dxf_text(&style.font));
    }
    dxf_pair(&mut out, 0, "ENDTAB");
    dxf_pair(&mut out, 0, "TABLE");
    dxf_pair(&mut out, 2, "DIMSTYLE");
    dxf_pair(&mut out, 70, drawing.dim_styles.len());
    for (index, style) in drawing.dim_styles.iter().enumerate() {
        dxf_pair(&mut out, 0, "DIMSTYLE");
        dxf_pair(&mut out, 105, format!("{:X}", index + 1));
        dxf_pair(&mut out, 2, dxf_text(&style.name));
        dxf_pair(&mut out, 70, 0);
        dxf_pair(&mut out, 40, num_exact(style.scale));
        dxf_pair(&mut out, 41, num_exact(style.arrow_size));
        dxf_pair(&mut out, 42, num_exact(style.origin_offset));
        dxf_pair(&mut out, 44, num_exact(style.extension));
        dxf_pair(&mut out, 140, num_exact(style.text_height));
        dxf_pair(&mut out, 271, style.precision);
    }
    dxf_pair(&mut out, 0, "ENDTAB");
    dxf_pair(&mut out, 0, "ENDSEC");

    dxf_pair(&mut out, 0, "SECTION");
    dxf_pair(&mut out, 2, "ENTITIES");
    for entity in drawing.ordered_entities() {
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
                if r12 {
                    let sweep = arc_sweep(*start_angle, *end_angle);
                    let sampled = sample_ellipse(*center, rx.abs(), ry.abs(), *rotation, *start_angle, *end_angle);
                    let closed = sweep >= 360.0 - 1e-9;
                    let points = if closed { closed_profile(&sampled) } else { sampled };
                    dxf_polyline(&mut out, &entity.layer, &points, closed);
                    losses.push(Loss::warning(
                        "dxf/r12-ellipse",
                        Some(&entity.layer),
                        Some(&entity.id),
                        "ellipse is written as a sampled POLYLINE because R12 has no ELLIPSE entity".to_string(),
                    ));
                    continue;
                }
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
                dxf_polyline(&mut out, &entity.layer, points, *closed);
            }
            EntityKind::Spline { points, closed } => {
                if r12 {
                    let sampled = sample_spline(points, *closed);
                    let outline = if *closed { closed_profile(&sampled) } else { sampled };
                    dxf_polyline(&mut out, &entity.layer, &outline, *closed);
                    losses.push(Loss::warning(
                        "dxf/r12-spline",
                        Some(&entity.layer),
                        Some(&entity.id),
                        "spline is written as a sampled POLYLINE because R12 has no SPLINE entity".to_string(),
                    ));
                    continue;
                }
                dxf_spline(&mut out, &entity.layer, points, *closed);
            }
            EntityKind::Dimension { from, to, offset, text, .. } => {
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
            EntityKind::RadialDimension { center, radius, angle, offset, diameter, text } => {
                let target = radial_target(*center, *radius, *angle);
                dxf_pair(&mut out, 0, "DIMENSION");
                dxf_pair(&mut out, 8, dxf_layer_name(&entity.layer));
                dxf_pair(&mut out, 70, if *diameter { 3 } else { 4 });
                dxf_pair(&mut out, 10, num_exact(target[0]));
                dxf_pair(&mut out, 20, num_exact(target[1]));
                dxf_pair(&mut out, 30, "0");
                dxf_pair(&mut out, 15, num_exact(center[0]));
                dxf_pair(&mut out, 25, num_exact(center[1]));
                dxf_pair(&mut out, 35, "0");
                dxf_pair(&mut out, 40, num_exact(*offset));
                dxf_pair(&mut out, 50, num_exact(*angle));
                match text {
                    Some(label) => dxf_pair(&mut out, 1, dxf_text(label)),
                    None => dxf_pair(&mut out, 1, "<>"),
                }
            }
            EntityKind::OrdinateDimension { origin, feature, axis, offset, text } => {
                let (base, _guide) = ordinate_points(*origin, *feature, *axis, *offset);
                let flags = if *axis == OrdinateAxis::X { 6 | 64 } else { 6 };
                dxf_pair(&mut out, 0, "DIMENSION");
                dxf_pair(&mut out, 8, dxf_layer_name(&entity.layer));
                dxf_pair(&mut out, 70, flags);
                dxf_pair(&mut out, 13, num_exact(origin[0]));
                dxf_pair(&mut out, 23, num_exact(origin[1]));
                dxf_pair(&mut out, 33, "0");
                dxf_pair(&mut out, 14, num_exact(feature[0]));
                dxf_pair(&mut out, 24, num_exact(feature[1]));
                dxf_pair(&mut out, 34, "0");
                dxf_pair(&mut out, 10, num_exact(base[0]));
                dxf_pair(&mut out, 20, num_exact(base[1]));
                dxf_pair(&mut out, 30, "0");
                match text {
                    Some(label) => dxf_pair(&mut out, 1, dxf_text(label)),
                    None => dxf_pair(&mut out, 1, "<>"),
                }
            }
            EntityKind::AngularDimension { center, first, second, offset, text } => {
                let (start_point, _end_point, middle) = angular_points(*center, *first, *second, *offset);
                dxf_pair(&mut out, 0, "DIMENSION");
                dxf_pair(&mut out, 8, dxf_layer_name(&entity.layer));
                dxf_pair(&mut out, 70, 5);
                dxf_pair(&mut out, 13, num_exact(first[0]));
                dxf_pair(&mut out, 23, num_exact(first[1]));
                dxf_pair(&mut out, 33, "0");
                dxf_pair(&mut out, 14, num_exact(second[0]));
                dxf_pair(&mut out, 24, num_exact(second[1]));
                dxf_pair(&mut out, 34, "0");
                dxf_pair(&mut out, 15, num_exact(center[0]));
                dxf_pair(&mut out, 25, num_exact(center[1]));
                dxf_pair(&mut out, 35, "0");
                dxf_pair(&mut out, 16, num_exact(start_point[0]));
                dxf_pair(&mut out, 26, num_exact(start_point[1]));
                dxf_pair(&mut out, 36, "0");
                dxf_pair(&mut out, 10, num_exact(middle[0]));
                dxf_pair(&mut out, 20, num_exact(middle[1]));
                dxf_pair(&mut out, 30, "0");
                match text {
                    Some(label) => dxf_pair(&mut out, 1, dxf_text(label)),
                    None => dxf_pair(&mut out, 1, "<>"),
                }
            }
            EntityKind::Hatch { points, pattern } => {
                if points.len() < 3 {
                    losses.push(Loss::warning(
                        "dxf/hatch-boundary",
                        Some(&entity.layer),
                        Some(&entity.id),
                        format!("hatch `{}` has fewer than 3 boundary points and was skipped", entity.id),
                    ));
                    continue;
                }
                if r12 {
                    let boundary = closed_profile(points);
                    dxf_polyline(&mut out, &entity.layer, &boundary, true);
                    losses.push(Loss::warning(
                        "dxf/r12-hatch",
                        Some(&entity.layer),
                        Some(&entity.id),
                        format!("hatch pattern `{pattern}` becomes its closed boundary POLYLINE because R12 has no HATCH entity"),
                    ));
                    continue;
                }
                dxf_hatch(&mut out, &entity.layer, points, pattern);
                if !pattern.eq_ignore_ascii_case("SOLID") && !pattern.trim().is_empty() {
                    losses.push(Loss::warning(
                        "dxf/hatch-pattern",
                        Some(&entity.layer),
                        Some(&entity.id),
                        format!("hatch pattern `{pattern}` is written by name; no pattern definition is emitted"),
                    ));
                }
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
    let projection =
        Projection { map: &map, scale, stroke: 0.7, lineweight_scale: scale, label_offset: 1.75, label_size: 1.75 * scale, dim_styles: &drawing.dim_styles };
    let flat = build_drawing(drawing, FlatSize::new(PDF_PAGE_WIDTH, PDF_PAGE_HEIGHT), &projection);
    let out = flat.to_pdf().map_or_else(|_| String::from("%PDF-1.7\n%%EOF\n"), |bytes| String::from_utf8_lossy(&bytes).into_owned());

    let mut replaced = 0;
    for layer in drawing.layers.iter().filter(|layer| layer.visible) {
        for entity in drawing.entities.iter().filter(|entity| entity.layer == layer.id) {
            match &entity.kind {
                EntityKind::Text { content, .. } => replaced += pdf_text(content).1,
                EntityKind::Dimension { from, to, text, .. } => replaced += pdf_text(&dimension_label(*from, *to, text)).1,
                EntityKind::RadialDimension { radius, diameter, text, .. } => {
                    replaced += pdf_text(&radial_dimension_label(*radius, *diameter, text)).1;
                }
                EntityKind::OrdinateDimension { origin, feature, axis, text, .. } => {
                    replaced += pdf_text(&ordinate_dimension_label(*origin, *feature, *axis, text)).1;
                }
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

/// Selects the extrudable profile of one entity for the 3D projections.
///
/// Curve entities (polyline, spline, arc, ellipse, circle) return their sampled
/// profile and whether it is closed; a full circle and a full-sweep ellipse are
/// closed, an arc and an open polyline are not. Lines, text and dimensions
/// return `None` and are never extruded. Entities on hidden or frozen layers
/// return `None` too, matching the painting exporters, so construction
/// geometry never leaks into the solid. The STL, OBJ and STEP writers share
/// this selection so all three describe the same solid.
fn extrude_profile(drawing: &Drawing, entity: &Entity) -> Option<(Vec<[f64; 2]>, bool)> {
    if !drawing.layer(&entity.layer).is_some_and(|layer| layer.visible && !layer.frozen) {
        return None;
    }
    match &entity.kind {
        EntityKind::Polyline { points, closed } => Some((closed_profile(points), *closed)),
        EntityKind::Spline { points, closed } => Some((closed_profile(&sample_spline(points, *closed)), *closed)),
        EntityKind::Arc { center, radius, start_angle, end_angle } => {
            Some((closed_profile(&sample_arc(*center, radius.abs(), *start_angle, *end_angle)), false))
        }
        EntityKind::Ellipse { center, rx, ry, rotation, start_angle, end_angle } => {
            let sampled = sample_ellipse(*center, rx.abs(), ry.abs(), *rotation, *start_angle, *end_angle);
            let closed = arc_sweep(*start_angle, *end_angle) >= 360.0 - 1e-9;
            Some((if closed { seam_profile(&sampled) } else { closed_profile(&sampled) }, closed))
        }
        EntityKind::Circle { center, radius } => Some((seam_profile(&sample_arc(*center, radius.abs(), 0.0, 0.0)), true)),
        _ => None,
    }
}

/// Deduplicates a closed profile sampled over a full turn.
///
/// Sampling from `0` through `360` degrees repeats the start point with a
/// floating-point offset of about `1e-15` of the radius, which exact
/// [`closed_profile`] deduplication keeps and ear clipping then turns into
/// degenerate seam triangles.
fn seam_profile(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut profile = closed_profile(points);
    if profile.len() > 2 {
        let first = profile[0];
        let last = profile[profile.len() - 1];
        let (dx, dy) = (last[0] - first[0], last[1] - first[1]);
        if dx * dx + dy * dy < 1e-12 {
            profile.pop();
        }
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

/// Extrudes every closed profile along +Z and writes an ASCII STL solid.
///
/// The projection is a solid, not a surface model: each closed profile becomes
/// two triangulated caps plus one side wall per edge. Polylines, splines, full
/// circles and full ellipses are extruded (curves are sampled); lines, text and
/// dimensions are not, and an open profile is reported as a loss instead of
/// being closed silently.
pub(crate) fn stl(drawing: &Drawing, thickness: f64) -> StlReport {
    let height = if thickness.is_finite() && thickness > 0.0 { thickness } else { STL_DEFAULT_THICKNESS };
    let mut content = String::from("solid open-cad\n");
    let mut triangles = 0usize;
    let mut degenerate = 0usize;
    let mut losses = Vec::new();

    for entity in &drawing.entities {
        let Some((profile, closed)) = extrude_profile(drawing, entity)
        else {
            continue;
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
                format!("profile `{}` has fewer than three distinct vertices", entity.id),
            ));
            continue;
        }
        let Some(cap) = triangulate(&profile)
        else {
            losses.push(Loss::warning(
                "stl/invalid-profile",
                Some(&entity.layer),
                Some(&entity.id),
                format!("profile `{}` could not be triangulated", entity.id),
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
/// STL writer, so both formats describe the same solid. Lines, text and
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
        let Some((profile, closed)) = extrude_profile(drawing, entity)
        else {
            continue;
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
                format!("profile `{}` has fewer than three distinct vertices", entity.id),
            ));
            continue;
        }
        let Some(cap) = triangulate(&profile)
        else {
            losses.push(Loss::warning(
                "obj/invalid-profile",
                Some(&entity.layer),
                Some(&entity.id),
                format!("profile `{}` could not be triangulated", entity.id),
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

/// Extrudes every closed profile along +Z and writes AP203-style STEP text.
///
/// The projection is a faceted BREP, not a modelling kernel: every triangle of
/// one extrusion becomes one planar `ADVANCED_FACE`, and each closed profile
/// becomes its own `CLOSED_SHELL` inside a `MANIFOLD_SOLID_BREP`, so disjoint
/// profiles stay separate solids. There are no booleans and no analytic
/// surfaces. The profile selection matches the STL and OBJ writers, so lines,
/// text and dimensions are not extruded and an open profile is reported with
/// `step/open-profile` instead of being closed silently.
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
        let Some((profile, closed)) = extrude_profile(drawing, entity)
        else {
            continue;
        };
        if !closed {
            losses.push(Loss::warning(
                "step/open-profile",
                Some(&entity.layer),
                Some(&entity.id),
                format!("profile `{}` is not closed and was not extruded", entity.id),
            ));
            continue;
        }
        if profile.len() < 3 {
            losses.push(Loss::warning(
                "step/degenerate-triangle",
                Some(&entity.layer),
                Some(&entity.id),
                format!("profile `{}` has fewer than three distinct vertices", entity.id),
            ));
            continue;
        }
        let Some(cap) = triangulate(&profile)
        else {
            losses.push(Loss::warning(
                "step/invalid-profile",
                Some(&entity.layer),
                Some(&entity.id),
                format!("profile `{}` could not be triangulated", entity.id),
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

#[cfg(test)]
#[path = "../tests/unit/exchange.rs"]
mod tests;

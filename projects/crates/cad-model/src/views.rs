//! Multiview projection sheets, section views and leader notes.
//!
//! The module projects a small declarative three-dimensional part model —
//! boxes, cylinders, cones, spheres and wedges, with optional void features
//! that carve hidden outlines — onto the drawing plane. `add_multiview_sheet`
//! lays out first- or third-angle front, top and left views plus an isometric
//! view with labels and the ISO projection symbol; `add_section_view` cuts the
//! part by a plane and draws the cut outline with 45-degree hatch lines;
//! `add_leader_note` draws a polyline leader with an arrowhead and a note.
//!
//! All projection, layout and hatching mathematics lives here; the binding and
//! the SDK only forward the request. The exporters frame the sheet from the
//! visible extents, so these operations never draw an outer sheet frame.

use std::f64::consts::{FRAC_PI_6, TAU};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    ServiceError, argument, locked_layer,
    schema::{CONTINUOUS_LINETYPE, Drawing, Entity, EntityKind, Layer, Linetype, PROPERTY_VALUE_LIMIT},
    store,
};

/// Gap between the projected views on the sheet, in layout units.
const VIEW_GAP: f64 = 18.0;

/// Layer the generated views, sections and their labels are drawn on.
const VIEWS_LAYER: &str = "views";

/// Layer and line type carrying the hidden outlines projected from voids.
const HIDDEN_LAYER: &str = "hidden";

/// Dash pattern of the generated hidden line type.
const HIDDEN_PATTERN: [f64; 2] = [3.0, 2.0];

/// Layer the generated leader notes are drawn on.
const NOTES_LAYER: &str = "notes";

/// Number of rim samples used when a circular feature is projected into the
/// isometric view or cut by a parallel section plane.
const RIM_SAMPLES: usize = 72;

/// Number of axial stations sampled for an offset cone section.
const CONE_STATIONS: u32 = 64;

/// Cap on the hatch segments one section view may generate.
const HATCH_LINE_LIMIT: u64 = 2000;

/// Approximate glyph width as a fraction of the text height.
const TEXT_WIDTH_FACTOR: f64 = 0.6;

/// One of the three principal axes of the part model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Axis3 {
    /// The X axis.
    X,
    /// The Y axis.
    Y,
    /// The Z axis.
    Z,
}

impl Axis3 {
    /// Index of this axis inside a `[x, y, z]` triple.
    fn index(self) -> usize {
        match self {
            Self::X => 0,
            Self::Y => 1,
            Self::Z => 2,
        }
    }

    /// The two axes perpendicular to this one, in ascending order.
    fn transverse(self) -> [Self; 2] {
        match self {
            Self::X => [Self::Y, Self::Z],
            Self::Y => [Self::X, Self::Z],
            Self::Z => [Self::X, Self::Y],
        }
    }
}

/// One convex solid of the declarative part model.
///
/// Cylinders and cones store their transverse centre as a two-coordinate
/// point: the coordinates are the values along the two axes perpendicular to
/// `axis`, in ascending axis order (`[y, z]` for `x`, `[x, z]` for `y`,
/// `[x, y]` for `z`), while `from` and `to` are coordinates along `axis`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Feature3d {
    /// Axis-aligned box spanning `min` to `max`.
    Box {
        /// Lower corner.
        min: [f64; 3],
        /// Upper corner.
        max: [f64; 3],
    },
    /// Right circular cylinder along one principal axis.
    Cylinder {
        /// Axis the cylinder runs along.
        axis: Axis3,
        /// Centre of the end faces, transverse to `axis`.
        center: [f64; 2],
        /// Coordinate of the base face along `axis`.
        from: f64,
        /// Coordinate of the top face along `axis`.
        to: f64,
        /// Radius, positive.
        radius: f64,
    },
    /// Circular cone or frustum along one principal axis.
    Cone {
        /// Axis the cone runs along.
        axis: Axis3,
        /// Centre of the end faces, transverse to `axis`.
        center: [f64; 2],
        /// Coordinate of the base face along `axis`.
        from: f64,
        /// Coordinate of the top face along `axis`.
        to: f64,
        /// Radius at `from`, positive.
        base_radius: f64,
        /// Radius at `to`, zero for a full cone.
        top_radius: f64,
    },
    /// Sphere around `center`.
    Sphere {
        /// Centre point.
        center: [f64; 3],
        /// Radius, positive.
        radius: f64,
    },
    /// Box whose top face slopes along X between two heights.
    Wedge {
        /// Lower corner of the footprint.
        min: [f64; 3],
        /// Upper corner of the bounding box.
        max: [f64; 3],
        /// Height of the top face at `min`'s x coordinate.
        top_from: f64,
        /// Height of the top face at `max`'s x coordinate.
        top_to: f64,
    },
}

/// A part: solid features plus void features that carve hidden outlines.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PartSpec {
    /// Material features; the part needs at least one.
    pub solids: Vec<Feature3d>,
    /// Removed material, drawn as hidden outlines.
    pub voids: Vec<Feature3d>,
}

/// A rectangle, a disc or a polygon in view coordinates.
#[derive(Debug, Clone)]
enum Shape2 {
    /// Axis-aligned rectangle from the lower to the upper corner.
    Rect([f64; 2], [f64; 2]),
    /// Disc.
    Disc([f64; 2], f64),
    /// Closed polygon.
    Poly(Vec<[f64; 2]>),
}

impl Shape2 {
    /// Bounds of the shape.
    fn bounds(&self) -> ([f64; 2], [f64; 2]) {
        match self {
            Self::Rect(a, b) => (*a, *b),
            Self::Disc(c, r) => ([c[0] - r, c[1] - r], [c[0] + r, c[1] + r]),
            Self::Poly(points) => {
                let min =
                    [points.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min), points.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min)];
                let max = [
                    points.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max),
                    points.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max),
                ];
                (min, max)
            }
        }
    }

    /// The shape scaled about the view origin.
    fn scaled(&self, scale: f64) -> Shape2 {
        match self {
            Self::Rect(a, b) => Self::Rect([a[0] * scale, a[1] * scale], [b[0] * scale, b[1] * scale]),
            Self::Disc(c, r) => Self::Disc([c[0] * scale, c[1] * scale], r * scale),
            Self::Poly(points) => Self::Poly(points.iter().map(|p| [p[0] * scale, p[1] * scale]).collect()),
        }
    }

    /// The shape moved by an offset.
    fn translated(&self, offset: [f64; 2]) -> Shape2 {
        match self {
            Self::Rect(a, b) => Self::Rect([a[0] + offset[0], a[1] + offset[1]], [b[0] + offset[0], b[1] + offset[1]]),
            Self::Disc(c, r) => Self::Disc([c[0] + offset[0], c[1] + offset[1]], *r),
            Self::Poly(points) => Self::Poly(points.iter().map(|p| [p[0] + offset[0], p[1] + offset[1]]).collect()),
        }
    }

    /// The shape rotated by `-angle` (given as its sine and cosine) about the
    /// origin; a rectangle becomes its corner polygon.
    fn rotated(&self, sin: f64, cos: f64) -> Shape2 {
        let turn = |p: &[f64; 2]| [p[0] * cos + p[1] * sin, -p[0] * sin + p[1] * cos];
        match self {
            Self::Rect(a, b) => Self::Poly([*a, [b[0], a[1]], *b, [a[0], b[1]]].iter().map(turn).collect()),
            Self::Disc(c, r) => Self::Disc(turn(c), *r),
            Self::Poly(points) => Self::Poly(points.iter().map(turn).collect()),
        }
    }

    /// The entity kind drawing the shape outline.
    fn kind(&self) -> EntityKind {
        match self {
            Self::Rect([u0, v0], [u1, v1]) => {
                EntityKind::Polyline { points: vec![[*u0, *v0], [*u1, *v0], [*u1, *v1], [*u0, *v1]], closed: true }
            }
            Self::Disc(center, radius) => EntityKind::Circle { center: *center, radius: *radius },
            Self::Poly(points) => EntityKind::Polyline { points: points.clone(), closed: true },
        }
    }
}

/// Bounds of a shape list, `None` when the list is empty.
fn shapes_bounds(shapes: &[Shape2]) -> Option<([f64; 2], [f64; 2])> {
    shapes.iter().map(Shape2::bounds).reduce(|(a, b), (c, _)| ([a[0].min(c[0]), a[1].min(c[1])], [b[0].max(c[0]), b[1].max(c[1])]))
}

/// Bounds of a shape list, an argument error when the list is empty.
fn require_bounds(shapes: &[Shape2], what: &str) -> Result<([f64; 2], [f64; 2]), ServiceError> {
    shapes_bounds(shapes).ok_or_else(|| argument(format!("`{what}` produced no geometry")))
}

fn finite3(point: &[f64; 3]) -> bool {
    point.iter().all(|value| value.is_finite())
}

fn finite2(point: &[f64; 2]) -> bool {
    point.iter().all(|value| value.is_finite())
}

/// Validates one feature: finite coordinates and positive extents.
fn validate_feature(feature: &Feature3d) -> Result<(), ServiceError> {
    match feature {
        Feature3d::Box { min, max } => {
            if !finite3(min) || !finite3(max) {
                return Err(argument("a box needs finite `min` and `max` corners"));
            }
            if !(min[0] < max[0] && min[1] < max[1] && min[2] < max[2]) {
                return Err(argument("a box needs `min` strictly below `max` on every axis"));
            }
        }
        Feature3d::Cylinder { axis: _, center, from, to, radius } => {
            if !finite2(center) || !from.is_finite() || !to.is_finite() || from == to {
                return Err(argument("a cylinder needs a finite `center` and distinct finite `from` and `to`"));
            }
            if !radius.is_finite() || *radius <= 0.0 {
                return Err(argument("a cylinder needs a positive finite `radius`"));
            }
        }
        Feature3d::Cone { axis: _, center, from, to, base_radius, top_radius } => {
            if !finite2(center) || !from.is_finite() || !to.is_finite() || from == to {
                return Err(argument("a cone needs a finite `center` and distinct finite `from` and `to`"));
            }
            if !base_radius.is_finite() || *base_radius <= 0.0 || !top_radius.is_finite() || *top_radius < 0.0 {
                return Err(argument("a cone needs a positive `base_radius` and a non-negative `top_radius`"));
            }
        }
        Feature3d::Sphere { center, radius } => {
            if !finite3(center) {
                return Err(argument("a sphere needs a finite `center`"));
            }
            if !radius.is_finite() || *radius <= 0.0 {
                return Err(argument("a sphere needs a positive finite `radius`"));
            }
        }
        Feature3d::Wedge { min, max, top_from, top_to } => {
            if !finite3(min) || !finite3(max) || !top_from.is_finite() || !top_to.is_finite() {
                return Err(argument("a wedge needs finite corners and top heights"));
            }
            if !(min[0] < max[0] && min[1] < max[1] && min[2] < max[2]) {
                return Err(argument("a wedge needs `min` strictly below `max` on every axis"));
            }
            if *top_from < min[2] || *top_to < min[2] || *top_from > max[2] || *top_to > max[2] || (*top_from == min[2] && *top_to == min[2]) {
                return Err(argument("a wedge needs both top heights above its base and inside its bounds"));
            }
        }
    }
    Ok(())
}

/// Validates a part: at least one solid, every feature sound.
fn validate_part(part: &PartSpec) -> Result<(), ServiceError> {
    if part.solids.is_empty() {
        return Err(argument("the part needs at least one solid feature"));
    }
    for feature in part.solids.iter().chain(part.voids.iter()) {
        validate_feature(feature)?;
    }
    Ok(())
}

/// One of the three orthographic views of a sheet.
#[derive(Debug, Clone, Copy)]
enum View {
    /// Looking along -Y: horizontal X, vertical Z.
    Front,
    /// Looking along -Z: horizontal X, vertical Y.
    Top,
    /// Looking along +X: horizontal Y, vertical Z.
    Left,
}

impl View {
    /// The part axes mapped to the horizontal and vertical view axes.
    fn axes(self) -> (Axis3, Axis3) {
        match self {
            Self::Front => (Axis3::X, Axis3::Z),
            Self::Top => (Axis3::X, Axis3::Y),
            Self::Left => (Axis3::Y, Axis3::Z),
        }
    }

    /// Label drawn under the placed view.
    fn label(self) -> &'static str {
        match self {
            Self::Front => "FRONT",
            Self::Top => "TOP",
            Self::Left => "LEFT",
        }
    }
}

/// Coordinate of a point along one axis.
fn coord(point: &[f64; 3], axis: Axis3) -> f64 {
    point[axis.index()]
}

/// Transverse centre coordinate of a cylinder or cone along one of its two
/// perpendicular axes.
fn transverse_coord(axis: Axis3, center: &[f64; 2], query: Axis3) -> f64 {
    let [t0, _] = axis.transverse();
    if query == t0 { center[0] } else { center[1] }
}

/// Outlines one feature projects to in one orthographic view.
fn project_feature(feature: &Feature3d, view: View) -> Vec<Shape2> {
    let (u, v) = view.axes();
    match feature {
        Feature3d::Box { min, max } => vec![Shape2::Rect([coord(min, u), coord(min, v)], [coord(max, u), coord(max, v)])],
        Feature3d::Sphere { center, radius } => vec![Shape2::Disc([coord(center, u), coord(center, v)], *radius)],
        Feature3d::Cylinder { axis, center, from, to, radius } => {
            if u == *axis || v == *axis {
                let along = if u == *axis { u } else { v };
                let across = if u == *axis { v } else { u };
                let lo = from.min(*to);
                let hi = from.max(*to);
                let c = transverse_coord(*axis, center, across);
                let (u_range, v_range) = if along == u { ([lo, hi], [c - radius, c + radius]) } else { ([c - radius, c + radius], [lo, hi]) };
                vec![Shape2::Rect([u_range[0], v_range[0]], [u_range[1], v_range[1]])]
            }
            else {
                vec![Shape2::Disc([transverse_coord(*axis, center, u), transverse_coord(*axis, center, v)], *radius)]
            }
        }
        Feature3d::Cone { axis, center, from, to, base_radius, top_radius } => {
            if u == *axis || v == *axis {
                let along = if u == *axis { u } else { v };
                let across = if u == *axis { v } else { u };
                let c = transverse_coord(*axis, center, across);
                let points = if along == u {
                    [[*from, c - *base_radius], [*from, c + *base_radius], [*to, c + *top_radius], [*to, c - *top_radius]]
                }
                else {
                    [[c - *base_radius, *from], [c + *base_radius, *from], [c + *top_radius, *to], [c - *top_radius, *to]]
                };
                vec![Shape2::Poly(points.to_vec())]
            }
            else {
                let center_uv = [transverse_coord(*axis, center, u), transverse_coord(*axis, center, v)];
                let mut shapes = vec![Shape2::Disc(center_uv, *base_radius)];
                if *top_radius > 0.0 {
                    shapes.push(Shape2::Disc(center_uv, *top_radius));
                }
                shapes
            }
        }
        Feature3d::Wedge { min, max, top_from, top_to } => match view {
            View::Front => vec![Shape2::Poly(vec![[min[0], min[2]], [max[0], min[2]], [max[0], *top_to], [min[0], *top_from]])],
            View::Top => vec![Shape2::Rect([min[0], min[1]], [max[0], max[1]])],
            View::Left => vec![Shape2::Rect([min[1], min[2]], [max[1], top_from.max(*top_to)])],
        },
    }
}

/// Isometric image of a point: u along the 30-degree plan diagonal, v the
/// raised height.
fn iso_point(point: &[f64; 3]) -> [f64; 2] {
    [(point[0] - point[1]) * FRAC_PI_6.cos(), (point[0] + point[1]) * FRAC_PI_6.sin() + point[2]]
}

/// Convex hull of projected samples (monotone chain).
fn convex_hull(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut pts = points.to_vec();
    pts.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    pts.dedup_by(|a, b| a[0].to_bits() == b[0].to_bits() && a[1].to_bits() == b[1].to_bits());
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: &[f64; 2], a: &[f64; 2], b: &[f64; 2]| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut lower: Vec<[f64; 2]> = Vec::with_capacity(pts.len());
    for p in &pts {
        while lower.len() >= 2 && cross(&lower[lower.len() - 2], &lower[lower.len() - 1], p) <= 0.0 {
            lower.pop();
        }
        lower.push(*p);
    }
    let mut upper: Vec<[f64; 2]> = Vec::with_capacity(pts.len());
    for p in pts.iter().rev() {
        while upper.len() >= 2 && cross(&upper[upper.len() - 2], &upper[upper.len() - 1], p) <= 0.0 {
            upper.pop();
        }
        upper.push(*p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// Rim samples of a cylinder or cone: a circle of the given radius at one
/// axial station, projected into the isometric view.
fn rim_samples(axis: Axis3, center: &[f64; 2], station: f64, radius: f64) -> Vec<[f64; 2]> {
    let [t0, t1] = axis.transverse();
    (0..RIM_SAMPLES)
        .map(|step| {
            let angle = TAU * step as f64 / RIM_SAMPLES as f64;
            let (sin, cos) = angle.sin_cos();
            let mut point = [0.0; 3];
            point[axis.index()] = station;
            point[t0.index()] = center[0] + radius * cos;
            point[t1.index()] = center[1] + radius * sin;
            iso_point(&point)
        })
        .collect()
}

/// Isometric outline of one feature: the convex hull of its silhouette
/// samples.
fn project_iso(feature: &Feature3d) -> Vec<Shape2> {
    match feature {
        Feature3d::Box { min, max } => {
            let points = [
                [min[0], min[1], min[2]],
                [max[0], min[1], min[2]],
                [max[0], max[1], min[2]],
                [min[0], max[1], min[2]],
                [min[0], min[1], max[2]],
                [max[0], min[1], max[2]],
                [max[0], max[1], max[2]],
                [min[0], max[1], max[2]],
            ];
            vec![Shape2::Poly(convex_hull(&points.iter().map(iso_point).collect::<Vec<_>>()))]
        }
        Feature3d::Sphere { center, radius } => vec![Shape2::Disc(iso_point(center), *radius)],
        Feature3d::Cylinder { axis, center, from, to, radius } => {
            let mut samples = rim_samples(*axis, center, *from, *radius);
            samples.extend(rim_samples(*axis, center, *to, *radius));
            vec![Shape2::Poly(convex_hull(&samples))]
        }
        Feature3d::Cone { axis, center, from, to, base_radius, top_radius } => {
            let mut samples = rim_samples(*axis, center, *from, *base_radius);
            if *top_radius > 0.0 {
                samples.extend(rim_samples(*axis, center, *to, *top_radius));
            }
            else {
                let [t0, t1] = axis.transverse();
                let mut apex = [0.0; 3];
                apex[axis.index()] = *to;
                apex[t0.index()] = center[0];
                apex[t1.index()] = center[1];
                samples.push(iso_point(&apex));
            }
            vec![Shape2::Poly(convex_hull(&samples))]
        }
        Feature3d::Wedge { min, max, top_from, top_to } => {
            let points = [
                [min[0], min[1], min[2]],
                [max[0], min[1], min[2]],
                [max[0], max[1], min[2]],
                [min[0], max[1], min[2]],
                [min[0], min[1], *top_from],
                [max[0], min[1], *top_to],
                [max[0], max[1], *top_to],
                [min[0], max[1], *top_from],
            ];
            vec![Shape2::Poly(convex_hull(&points.iter().map(iso_point).collect::<Vec<_>>()))]
        }
    }
}

/// Projection convention of a multiview sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Projection {
    /// ISO first angle: the top view sits below the front view.
    First,
    /// ISO third angle: the top view sits above the front view.
    Third,
}

/// Arguments of [`add_multiview_sheet`].
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MultiViewSheetRequest {
    /// The projected part; needs at least one solid feature.
    pub part: PartSpec,
    /// `first` (default) or `third` angle projection.
    pub projection: Option<String>,
    /// Linear scale factor applied to every view; defaults to `1`.
    pub scale: Option<f64>,
    /// Height of the view labels; defaults to `3.5`.
    pub text_height: Option<f64>,
    /// Lower-left corner of the placed view group; defaults to the origin.
    pub origin: Option<[f64; 2]>,
    /// Layer carrying the visible outlines and labels; defaults to `views`.
    pub layer: Option<String>,
}

/// Creates a layer with a neutral colour when the drawing lacks it.
fn ensure_layer(drawing: &mut Drawing, name: &str) {
    if drawing.layer(name).is_none() {
        drawing.layers.push(Layer {
            id: name.to_string(),
            name: name.to_string(),
            color: "#d0d7de".to_string(),
            visible: true,
            locked: false,
            frozen: false,
            plottable: true,
            linetype: CONTINUOUS_LINETYPE.to_string(),
            lineweight: 0.0,
        });
    }
}

/// Creates the hidden layer and its dashed line type when missing.
fn ensure_hidden_layer(drawing: &mut Drawing) {
    if !drawing.linetypes.iter().any(|linetype| linetype.id == HIDDEN_LAYER) {
        drawing.linetypes.push(Linetype {
            id: HIDDEN_LAYER.to_string(),
            name: HIDDEN_LAYER.to_string(),
            description: "Dashed line for hidden outlines".to_string(),
            pattern: HIDDEN_PATTERN.to_vec(),
        });
    }
    if drawing.layer(HIDDEN_LAYER).is_none() {
        drawing.layers.push(Layer {
            id: HIDDEN_LAYER.to_string(),
            name: HIDDEN_LAYER.to_string(),
            color: "#8c8c8c".to_string(),
            visible: true,
            locked: false,
            frozen: false,
            plottable: true,
            linetype: HIDDEN_LAYER.to_string(),
            lineweight: 0.0,
        });
    }
}

/// Estimated rendered width of one text line.
fn text_width(content: &str, text_height: f64) -> f64 {
    content.chars().count() as f64 * text_height * TEXT_WIDTH_FACTOR
}

/// One placed view: its centre, size and outlines in view-local coordinates.
struct PlacedView {
    label: &'static str,
    center: [f64; 2],
    size: [f64; 2],
    shapes: Vec<Shape2>,
    hidden: Vec<Shape2>,
}

/// Sizes one projected view from its raw outlines and centres them on the
/// combined bounds of the visible and hidden geometry.
fn place_view(label: &'static str, shapes: impl Iterator<Item = Shape2>, hidden: impl Iterator<Item = Shape2>, scale: f64) -> PlacedView {
    let shapes: Vec<Shape2> = shapes.map(|shape| shape.scaled(scale)).collect();
    let hidden: Vec<Shape2> = hidden.map(|shape| shape.scaled(scale)).collect();
    let bounds = shapes_bounds(&shapes)
        .into_iter()
        .chain(shapes_bounds(&hidden))
        .reduce(|(a, b), (c, _)| ([a[0].min(c[0]), a[1].min(c[1])], [b[0].max(c[0]), b[1].max(c[1])]))
        .unwrap_or(([0.0, 0.0], [0.0, 0.0]));
    let size = [bounds.1[0] - bounds.0[0], bounds.1[1] - bounds.0[1]];
    let center = [(bounds.0[0] + bounds.1[0]) / 2.0, (bounds.0[1] + bounds.1[1]) / 2.0];
    let shift = [-center[0], -center[1]];
    let shapes = shapes.into_iter().map(|shape| shape.translated(shift)).collect();
    let hidden = hidden.into_iter().map(|shape| shape.translated(shift)).collect();
    PlacedView { label, center: [0.0, 0.0], size, shapes, hidden }
}

/// JSON summary of one placed view.
fn view_json(view: &PlacedView) -> Value {
    json!({ "center": view.center, "width": view.size[0], "height": view.size[1] })
}

/// The truncated-cone and circle pair of the ISO projection symbol with its
/// angle label, anchored at the given lower-left corner.
fn projection_symbol(first_angle: bool, at: [f64; 2], text_height: f64, layer: &str) -> Vec<(String, EntityKind)> {
    let [sx, sy] = at;
    let trapezoid = if first_angle {
        vec![[sx, sy - 2.0], [sx + 14.0, sy - 0.75], [sx + 14.0, sy + 0.75], [sx, sy + 2.0]]
    }
    else {
        vec![[sx + 7.0, sy - 0.75], [sx + 21.0, sy - 2.0], [sx + 21.0, sy + 2.0], [sx + 7.0, sy + 0.75]]
    };
    let circle_x = if first_angle { sx + 21.0 } else { sx };
    vec![
        (layer.to_string(), EntityKind::Polyline { points: trapezoid, closed: true }),
        (layer.to_string(), EntityKind::Circle { center: [circle_x, sy], radius: 1.5 }),
        (layer.to_string(), EntityKind::Circle { center: [circle_x, sy], radius: 3.0 }),
        (
            layer.to_string(),
            EntityKind::Text {
                at: [sx + 28.0, sy - text_height / 2.0],
                height: text_height,
                content: if first_angle { "FIRST ANGLE".to_string() } else { "THIRD ANGLE".to_string() },
            },
        ),
    ]
}

/// Projects a part into a first- or third-angle multiview sheet with an
/// isometric view.
///
/// The request carries a [`PartSpec`]; `projection` selects the ISO first (the
/// default) or third angle convention. The front, top and left views share
/// their alignment axes, the isometric view is placed in the upper-right
/// corner of the group, every view gets a centred label below it, and the
/// group carries the ISO projection symbol with its angle label. Void features
/// project to hidden outlines on a generated `hidden` layer with a dashed line
/// type. The whole group is scaled about the view origin and its lower-left
/// corner is placed at `origin`. The edit is one history step and one revision,
/// and the response carries `{ revision, drawing, created, views, projection }`
/// where `views` reports the centre and size of each placed view.
///
/// A part without solids, a non-finite or non-positive scale, text height or
/// origin, and an unknown projection name report `session/argument-invalid`; a
/// locked or frozen target layer reports `drawing/layer-locked` or
/// `drawing/layer-frozen`.
pub fn add_multiview_sheet(handle: u64, request: MultiViewSheetRequest) -> Result<Value, ServiceError> {
    validate_part(&request.part)?;
    let projection = match request.projection.as_deref().map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        None | Some("first") => Projection::First,
        Some("third") => Projection::Third,
        Some(other) => return Err(argument(format!("`{other}` is not a projection; use `first` or `third`"))),
    };
    let scale = request.scale.unwrap_or(1.0);
    if !scale.is_finite() || scale <= 0.0 {
        return Err(argument("`scale` must be a positive finite number"));
    }
    let text_height = request.text_height.unwrap_or(3.5);
    if !text_height.is_finite() || text_height <= 0.0 {
        return Err(argument("`textHeight` must be a positive finite number"));
    }
    let origin = request.origin.unwrap_or([0.0, 0.0]);
    if !finite2(&origin) {
        return Err(argument("`origin` must contain finite coordinates"));
    }
    let layer = request.layer.map(|name| name.trim().to_string()).filter(|name| !name.is_empty()).unwrap_or_else(|| VIEWS_LAYER.to_string());
    let has_voids = !request.part.voids.is_empty();
    let (created, views) = store::edit(handle, |drawing| {
        ensure_layer(drawing, &layer);
        if let Some(error) = locked_layer(drawing, &layer) {
            return Err(error);
        }
        if has_voids {
            ensure_hidden_layer(drawing);
            if let Some(error) = locked_layer(drawing, HIDDEN_LAYER) {
                return Err(error);
            }
        }
        let mut placed: Vec<PlacedView> = Vec::new();
        for view in [View::Front, View::Top, View::Left] {
            placed.push(place_view(
                view.label(),
                request.part.solids.iter().flat_map(|feature| project_feature(feature, view)),
                request.part.voids.iter().flat_map(|feature| project_feature(feature, view)),
                scale,
            ));
        }
        placed.push(place_view(
            "ISO",
            request.part.solids.iter().flat_map(project_iso),
            request.part.voids.iter().flat_map(project_iso),
            scale,
        ));
        let [fw, fh] = placed[0].size;
        let [_tw, th] = placed[1].size;
        let [lw, _lh] = placed[2].size;
        let [iw, ih] = placed[3].size;
        let first_angle = projection == Projection::First;
        placed[1].center = [0.0, if first_angle { -(fh / 2.0 + VIEW_GAP + th / 2.0) } else { fh / 2.0 + VIEW_GAP + th / 2.0 }];
        placed[2].center = [if first_angle { fw / 2.0 + VIEW_GAP + lw / 2.0 } else { -(fw / 2.0 + VIEW_GAP + lw / 2.0) }, 0.0];
        let right_edge = if first_angle { fw / 2.0 + VIEW_GAP + lw } else { fw / 2.0 };
        let top_edge = if first_angle { fh / 2.0 } else { fh / 2.0 + VIEW_GAP + th };
        placed[3].center = [right_edge + VIEW_GAP + iw / 2.0, top_edge - ih / 2.0];
        let group_min = placed.iter().fold([f64::INFINITY; 2], |acc, view| {
            [acc[0].min(view.center[0] - view.size[0] / 2.0), acc[1].min(view.center[1] - view.size[1] / 2.0)]
        });
        let shift = [origin[0] - group_min[0], origin[1] - group_min[1]];
        for view in &mut placed {
            view.center = [view.center[0] + shift[0], view.center[1] + shift[1]];
        }
        let symbol = [origin[0], origin[1] - VIEW_GAP * 0.75];
        let mut kinds: Vec<(String, EntityKind)> = Vec::new();
        for view in &placed {
            for shape in &view.shapes {
                kinds.push((layer.clone(), shape.translated(view.center).kind()));
            }
            for shape in &view.hidden {
                kinds.push((HIDDEN_LAYER.to_string(), shape.translated(view.center).kind()));
            }
            kinds.push((
                layer.clone(),
                EntityKind::Text {
                    at: [view.center[0] - text_width(view.label, text_height) / 2.0, view.center[1] - view.size[1] / 2.0 - text_height * 1.5],
                    height: text_height,
                    content: view.label.to_string(),
                },
            ));
        }
        kinds.extend(projection_symbol(first_angle, symbol, text_height, &layer));
        let mut created = Vec::with_capacity(kinds.len());
        for (view_layer, kind) in kinds {
            let entity = Entity { id: drawing.take_entity_id(), layer: view_layer, kind };
            drawing.entities.push(entity.clone());
            created.push(entity);
        }
        drawing.revision += 1;
        let views = json!({
            "front": view_json(&placed[0]),
            "top": view_json(&placed[1]),
            "left": view_json(&placed[2]),
            "iso": view_json(&placed[3]),
        });
        Ok((created, views))
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "created": created,
            "views": views,
            "projection": if projection == Projection::First { "first" } else { "third" },
        })
    })
}

/// Arguments of [`add_section_view`].
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SectionViewRequest {
    /// The cut part; needs at least one solid feature.
    pub part: PartSpec,
    /// Axis the cutting plane is perpendicular to; required.
    pub axis: Option<Axis3>,
    /// Coordinate of the cutting plane along `axis`; required.
    pub at: Option<f64>,
    /// Hatch angle in degrees; defaults to `45`.
    pub hatch_angle: Option<f64>,
    /// Distance between hatch lines; defaults to `4`.
    pub hatch_spacing: Option<f64>,
    /// Height of the section label; defaults to `3.5`.
    pub text_height: Option<f64>,
    /// Lower-left corner of the placed section; defaults to the origin.
    pub origin: Option<[f64; 2]>,
    /// Section caption; defaults to `SECTION A-A`.
    pub label: Option<String>,
    /// Layer carrying the section; defaults to `views`.
    pub layer: Option<String>,
}

/// Section regions one feature contributes to a plane perpendicular to `cut`,
/// expressed in the two remaining axes in ascending order.
fn section_feature(feature: &Feature3d, cut: Axis3, at: f64) -> Vec<Shape2> {
    let transverse: Vec<Axis3> = [Axis3::X, Axis3::Y, Axis3::Z].into_iter().filter(|a| *a != cut).collect();
    let (u, v) = (transverse[0], transverse[1]);
    match feature {
        Feature3d::Box { min, max } => {
            if at < coord(min, cut) || at > coord(max, cut) {
                return Vec::new();
            }
            vec![Shape2::Rect([coord(min, u), coord(min, v)], [coord(max, u), coord(max, v)])]
        }
        Feature3d::Sphere { center, radius } => {
            let d = at - coord(center, cut);
            if d.abs() >= *radius {
                return Vec::new();
            }
            vec![Shape2::Disc([coord(center, u), coord(center, v)], (radius * radius - d * d).sqrt())]
        }
        Feature3d::Cylinder { axis, center, from, to, radius } => {
            let lo = from.min(*to);
            let hi = from.max(*to);
            if cut == *axis {
                if at < lo || at > hi {
                    return Vec::new();
                }
                vec![Shape2::Disc([transverse_coord(*axis, center, u), transverse_coord(*axis, center, v)], *radius)]
            }
            else {
                let d = at - transverse_coord(*axis, center, cut);
                if d.abs() >= *radius {
                    return Vec::new();
                }
                let h = (radius * radius - d * d).sqrt();
                let [t0, t1] = axis.transverse();
                let other = if cut == t0 { t1 } else { t0 };
                let c = transverse_coord(*axis, center, other);
                let (u_range, v_range) = if *axis == u { ([lo, hi], [c - h, c + h]) } else { ([c - h, c + h], [lo, hi]) };
                vec![Shape2::Rect([u_range[0], v_range[0]], [u_range[1], v_range[1]])]
            }
        }
        Feature3d::Cone { axis, center, from, to, base_radius, top_radius } => {
            let lo = from.min(*to);
            let hi = from.max(*to);
            let radius_at = |station: f64| {
                let t = (station - *from) / (*to - *from);
                *base_radius + t * (*top_radius - *base_radius)
            };
            if cut == *axis {
                if at < lo || at > hi {
                    return Vec::new();
                }
                let r = radius_at(at);
                if r <= 0.0 {
                    return Vec::new();
                }
                vec![Shape2::Disc([transverse_coord(*axis, center, u), transverse_coord(*axis, center, v)], r)]
            }
            else {
                let d = at - transverse_coord(*axis, center, cut);
                let [t0, t1] = axis.transverse();
                let other = if cut == t0 { t1 } else { t0 };
                let c = transverse_coord(*axis, center, other);
                let mk = |along: f64, across: f64| -> [f64; 2] { if *axis == u { [along, across] } else { [across, along] } };
                if d.abs() < 1e-9 {
                    vec![Shape2::Poly(vec![
                        mk(*from, c - *base_radius),
                        mk(*from, c + *base_radius),
                        mk(*to, c + *top_radius),
                        mk(*to, c - *top_radius),
                    ])]
                }
                else {
                    let mut upper: Vec<[f64; 2]> = Vec::new();
                    let mut lower: Vec<[f64; 2]> = Vec::new();
                    for step in 0..=CONE_STATIONS {
                        let station = lo + (hi - lo) * step as f64 / CONE_STATIONS as f64;
                        let r = radius_at(station);
                        if r > d.abs() {
                            let w = (r * r - d * d).sqrt();
                            upper.push(mk(station, c + w));
                            lower.push(mk(station, c - w));
                        }
                    }
                    if upper.len() < 2 {
                        return Vec::new();
                    }
                    lower.reverse();
                    upper.extend(lower);
                    vec![Shape2::Poly(upper)]
                }
            }
        }
        Feature3d::Wedge { min, max, top_from, top_to } => {
            let z0 = min[2];
            if cut == Axis3::X {
                if at < min[0] || at > max[0] {
                    return Vec::new();
                }
                let t = (at - min[0]) / (max[0] - min[0]);
                let top = *top_from + t * (*top_to - *top_from);
                vec![Shape2::Rect([min[1], z0], [max[1], top])]
            }
            else if cut == Axis3::Y {
                if at < min[1] || at > max[1] {
                    return Vec::new();
                }
                vec![Shape2::Poly(vec![[min[0], z0], [max[0], z0], [max[0], *top_to], [min[0], *top_from]])]
            }
            else {
                if at < z0 || at > top_from.max(*top_to) {
                    return Vec::new();
                }
                let (x_span, y_span) = if top_from == top_to {
                    if at > *top_from {
                        return Vec::new();
                    }
                    ([min[0], max[0]], [min[1], max[1]])
                }
                else {
                    let x_at = (min[0] + (at - *top_from) / (*top_to - *top_from) * (max[0] - min[0])).clamp(min[0], max[0]);
                    let span = if *top_to > *top_from { [min[0], x_at] } else { [x_at, max[0]] };
                    (span, [min[1], max[1]])
                };
                vec![Shape2::Rect([x_span[0], y_span[0]], [x_span[1], y_span[1]])]
            }
        }
    }
}

/// Horizontal intervals covering a rotated shape at scanline `level`.
fn shape_intervals(shape: &Shape2, level: f64) -> Vec<[f64; 2]> {
    match shape {
        Shape2::Disc([cu, cv], r) => {
            let d = level - cv;
            if d.abs() >= *r {
                return Vec::new();
            }
            let h = (r * r - d * d).sqrt();
            vec![[cu - h, cu + h]]
        }
        Shape2::Poly(points) => {
            let mut crossings: Vec<f64> = Vec::new();
            for index in 0..points.len() {
                let [u0, v0] = points[index];
                let [u1, v1] = points[(index + 1) % points.len()];
                if (v0 <= level && v1 > level) || (v1 <= level && v0 > level) {
                    crossings.push(u0 + (level - v0) * (u1 - u0) / (v1 - v0));
                }
            }
            crossings.sort_by(f64::total_cmp);
            crossings.chunks(2).filter(|pair| pair.len() == 2 && pair[1] - pair[0] > 1e-12).map(|pair| [pair[0], pair[1]]).collect()
        }
        Shape2::Rect(_, _) => Vec::new(),
    }
}

/// Sorts and merges an interval list into a disjoint cover.
fn merge_intervals(intervals: &mut Vec<[f64; 2]>) {
    intervals.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let mut merged: Vec<[f64; 2]> = Vec::with_capacity(intervals.len());
    for interval in intervals.iter() {
        match merged.last_mut() {
            Some(last) if interval[0] <= last[1] => last[1] = last[1].max(interval[1]),
            _ => merged.push(*interval),
        }
    }
    *intervals = merged;
}

/// Subtracts disjoint void intervals from disjoint solid intervals.
fn subtract_intervals(solid: &[[f64; 2]], void: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut spans: Vec<[f64; 2]> = Vec::new();
    for &[start, end] in solid {
        let mut cursor = start;
        for &[vs, ve] in void {
            if ve <= cursor || vs >= end {
                continue;
            }
            if vs > cursor {
                spans.push([cursor, vs]);
            }
            cursor = cursor.max(ve);
            if cursor >= end {
                break;
            }
        }
        if cursor < end {
            spans.push([cursor, end]);
        }
    }
    spans
}

/// One hatch segment: its two endpoints.
type HatchSpan = ([f64; 2], [f64; 2]);

/// Hatch segments filling the solid shapes minus the void shapes.
///
/// The shapes are rotated by `-angle`, scanlines `v = k * spacing` anchored to
/// the world origin cross them under the same half-open even-odd rule the hatch
/// pattern uses, and the surviving intervals are rotated back.
fn hatch_lines(solids: &[Shape2], voids: &[Shape2], angle: f64, spacing: f64) -> Result<Vec<HatchSpan>, ServiceError> {
    let (sin, cos) = angle.to_radians().sin_cos();
    let rotated: Vec<Shape2> = solids.iter().map(|shape| shape.rotated(sin, cos)).collect();
    let rotated_voids: Vec<Shape2> = voids.iter().map(|shape| shape.rotated(sin, cos)).collect();
    let (min, max) = require_bounds(&rotated, "section")?;
    let (min_v, max_v) = (min[1], max[1]);
    let epsilon = 1e-9 * (max_v - min_v).abs().max(spacing);
    let first = (min_v / spacing).floor();
    let last = (max_v / spacing).ceil();
    let count = (last - first) as u64;
    if count > HATCH_LINE_LIMIT {
        return Err(ServiceError::new(
            "hatch/spacing-too-fine",
            format!("the hatch needs more than {HATCH_LINE_LIMIT} lines; increase `spacing`"),
        ));
    }
    let mut spans = Vec::new();
    for step in 0..=count {
        let level = (first + step as f64) * spacing;
        if level <= min_v + epsilon || level >= max_v - epsilon {
            continue;
        }
        let mut solid: Vec<[f64; 2]> = rotated.iter().flat_map(|shape| shape_intervals(shape, level)).collect();
        merge_intervals(&mut solid);
        let mut void: Vec<[f64; 2]> = rotated_voids.iter().flat_map(|shape| shape_intervals(shape, level)).collect();
        merge_intervals(&mut void);
        for interval in subtract_intervals(&solid, &void) {
            let back = |u: f64| [u * cos - level * sin, u * sin + level * cos];
            spans.push((back(interval[0]), back(interval[1])));
        }
    }
    Ok(spans)
}

/// Cuts a part by a plane and draws the section with hatch lines.
///
/// The plane is perpendicular to `axis` at coordinate `at`. The section view
/// shows the cut outline of every solid, the cut outline of every void as a
/// visible edge, and hatch lines at `hatchAngle` degrees (default `45`) with
/// `hatchSpacing` (default `4`) between them; void regions stay unhatched. A
/// centred caption sits above the section and a cutting-plane indicator below.
/// The section is placed with its lower-left corner at `origin`. The edit is
/// one history step and one revision, and the response carries
/// `{ revision, drawing, created }`.
///
/// A missing `axis` or `at`, a non-finite cut coordinate, a non-positive or
/// non-finite spacing, a non-finite angle or text height, a non-finite origin
/// and an over-long label report `session/argument-invalid`; a plane that
/// misses the part reports `section/empty`; a spacing finer than the line
/// budget reports `hatch/spacing-too-fine`; a locked or frozen target layer
/// reports `drawing/layer-locked` or `drawing/layer-frozen`.
pub fn add_section_view(handle: u64, request: SectionViewRequest) -> Result<Value, ServiceError> {
    validate_part(&request.part)?;
    let cut = request.axis.ok_or_else(|| argument("`axis` is required; use `x`, `y` or `z`"))?;
    let at = request.at.ok_or_else(|| argument("`at` is required"))?;
    if !at.is_finite() {
        return Err(argument("`at` must be finite"));
    }
    let hatch_angle = request.hatch_angle.unwrap_or(45.0);
    if !hatch_angle.is_finite() {
        return Err(argument("`hatchAngle` must be finite"));
    }
    let hatch_spacing = request.hatch_spacing.unwrap_or(4.0);
    if !hatch_spacing.is_finite() || hatch_spacing <= 0.0 {
        return Err(argument("`hatchSpacing` must be a positive finite number"));
    }
    let text_height = request.text_height.unwrap_or(3.5);
    if !text_height.is_finite() || text_height <= 0.0 {
        return Err(argument("`textHeight` must be a positive finite number"));
    }
    let origin = request.origin.unwrap_or([0.0, 0.0]);
    if !finite2(&origin) {
        return Err(argument("`origin` must contain finite coordinates"));
    }
    let label =
        request.label.map(|value| value.trim().to_string()).filter(|value| !value.is_empty()).unwrap_or_else(|| "SECTION A-A".to_string());
    if label.chars().count() > PROPERTY_VALUE_LIMIT {
        return Err(argument("the section label is too long"));
    }
    let layer = request.layer.map(|name| name.trim().to_string()).filter(|name| !name.is_empty()).unwrap_or_else(|| VIEWS_LAYER.to_string());
    let created = store::edit(handle, |drawing| {
        ensure_layer(drawing, &layer);
        if let Some(error) = locked_layer(drawing, &layer) {
            return Err(error);
        }
        let mut solids: Vec<Shape2> = request.part.solids.iter().flat_map(|feature| section_feature(feature, cut, at)).collect();
        if solids.is_empty() {
            return Err(ServiceError::new("section/empty", "the cutting plane misses the part"));
        }
        let mut voids: Vec<Shape2> = request.part.voids.iter().flat_map(|feature| section_feature(feature, cut, at)).collect();
        let (min, _) = require_bounds(&solids, "section")?;
        let shift = [origin[0] - min[0], origin[1] - min[1]];
        for shape in &mut solids {
            *shape = shape.translated(shift);
        }
        for shape in &mut voids {
            *shape = shape.translated(shift);
        }
        let (min, max) = require_bounds(&solids, "section")?;
        let spans = hatch_lines(&solids, &voids, hatch_angle, hatch_spacing)?;
        let mut kinds: Vec<EntityKind> = Vec::new();
        for shape in solids.iter().chain(voids.iter()) {
            kinds.push(shape.kind());
        }
        for (from, to) in spans {
            kinds.push(EntityKind::Line { from, to });
        }
        kinds.push(EntityKind::Text {
            at: [(min[0] + max[0]) / 2.0 - text_width(&label, text_height) / 2.0, max[1] + text_height * 1.2],
            height: text_height,
            content: label.clone(),
        });
        let indicator_y = min[1] - text_height * 2.0;
        kinds.push(EntityKind::Line { from: [min[0], indicator_y], to: [max[0], indicator_y] });
        kinds.push(EntityKind::Line { from: [min[0], indicator_y], to: [min[0], indicator_y + text_height * 1.5] });
        kinds.push(EntityKind::Line { from: [max[0], indicator_y], to: [max[0], indicator_y + text_height * 1.5] });
        let mut created = Vec::with_capacity(kinds.len());
        for kind in kinds {
            let entity = Entity { id: drawing.take_entity_id(), layer: layer.clone(), kind };
            drawing.entities.push(entity.clone());
            created.push(entity);
        }
        drawing.revision += 1;
        Ok(created)
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "created": created,
        })
    })
}

/// Arguments of [`add_leader_note`].
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LeaderNoteRequest {
    /// Layer carrying the note; defaults to `notes`.
    pub layer: Option<String>,
    /// Leader points from the referenced object to the note; at least two.
    pub points: Vec<[f64; 2]>,
    /// Note text; non-empty.
    pub text: String,
    /// Height of the note text; defaults to `3.5`.
    pub text_height: Option<f64>,
    /// Whether an arrowhead marks the first point; defaults to `true`.
    pub arrow: Option<bool>,
}

/// Draws a leader note: a polyline, an optional arrowhead and the text.
///
/// `points` chain from the referenced object to the note. The first point
/// carries an arrowhead (two wings at 15 degrees, `textHeight * 1.5` long)
/// unless `arrow` is `false`, and the text is anchored beside the last point
/// on the side the final segment points to. The layer is created when missing.
/// The edit is one history step and one revision, and the response carries
/// `{ revision, drawing, created }`.
///
/// Fewer than two points, repeated consecutive points, non-finite coordinates,
/// a blank text and a non-positive text height report `session/argument-invalid`;
/// a locked or frozen target layer reports `drawing/layer-locked` or
/// `drawing/layer-frozen`.
pub fn add_leader_note(handle: u64, request: LeaderNoteRequest) -> Result<Value, ServiceError> {
    if request.points.len() < 2 {
        return Err(argument("a leader note needs at least two points"));
    }
    if !request.points.iter().all(finite2) {
        return Err(argument("`points` must contain finite coordinates"));
    }
    for pair in request.points.windows(2) {
        if (pair[1][0] - pair[0][0]).abs() + (pair[1][1] - pair[0][1]).abs() <= 1e-12 {
            return Err(argument("consecutive leader points must differ"));
        }
    }
    let text = request.text.trim().to_string();
    if text.is_empty() {
        return Err(argument("a leader note needs a non-empty `text`"));
    }
    let text_height = request.text_height.unwrap_or(3.5);
    if !text_height.is_finite() || text_height <= 0.0 {
        return Err(argument("`textHeight` must be a positive finite number"));
    }
    let arrow = request.arrow.unwrap_or(true);
    let layer = request.layer.map(|name| name.trim().to_string()).filter(|name| !name.is_empty()).unwrap_or_else(|| NOTES_LAYER.to_string());
    let created = store::edit(handle, |drawing| {
        ensure_layer(drawing, &layer);
        if let Some(error) = locked_layer(drawing, &layer) {
            return Err(error);
        }
        let mut kinds: Vec<EntityKind> = vec![EntityKind::Polyline { points: request.points.clone(), closed: false }];
        if arrow {
            let tip = request.points[0];
            let next = request.points[1];
            let dx = (next[0] - tip[0]) / ((next[0] - tip[0]).powi(2) + (next[1] - tip[1]).powi(2)).sqrt();
            let dy = (next[1] - tip[1]) / ((next[0] - tip[0]).powi(2) + (next[1] - tip[1]).powi(2)).sqrt();
            let wing = text_height * 1.5;
            for sign in [-1.0, 1.0] {
                let (sin, cos) = (15f64.to_radians() * sign).sin_cos();
                let bx = -dx;
                let by = -dy;
                kinds.push(EntityKind::Line { from: tip, to: [tip[0] + (bx * cos - by * sin) * wing, tip[1] + (bx * sin + by * cos) * wing] });
            }
        }
        let last = request.points[request.points.len() - 1];
        let previous = request.points[request.points.len() - 2];
        let at = if last[0] >= previous[0] {
            [last[0] + text_height * 0.5, last[1] + text_height * 0.6]
        }
        else {
            [last[0] - text_height * 0.5 - text_width(&text, text_height), last[1] + text_height * 0.6]
        };
        kinds.push(EntityKind::Text { at, height: text_height, content: text });
        let mut created = Vec::with_capacity(kinds.len());
        for kind in kinds {
            let entity = Entity { id: drawing.take_entity_id(), layer: layer.clone(), kind };
            drawing.entities.push(entity.clone());
            created.push(entity);
        }
        drawing.revision += 1;
        Ok(created)
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "created": created,
        })
    })
}

#[cfg(test)]
#[path = "../tests/unit/views.rs"]
mod tests;

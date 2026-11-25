//! Multiview projection sheets, section views and leader notes.
//!
//! The module projects a small declarative three-dimensional part model —
//! boxes, cylinders, cones, spheres and wedges, with optional void features
//! that carve hidden outlines — onto the drawing plane. `add_multiview_sheet`
//! lays out first- or third-angle front, top and left views plus an isometric
//! view with labels and the ISO projection symbol; `add_section_view` cuts the
//! part by a plane and draws the cut outline with 45-degree hatch lines;
//! `add_leader_note` draws a polyline leader with an arrowhead and a note;
//! `add_isometric_view` draws a standalone isometric view with an axis
//! indicator; `add_exploded_assembly` spaces components apart and numbers
//! them with balloons; `add_hole_table` tabulates the cylindrical voids of a
//! part as a framed hole table.
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

/// Nearest point to `point` on a segment.
fn nearest_on_segment(point: [f64; 2], from: [f64; 2], to: [f64; 2]) -> [f64; 2] {
    let span = [to[0] - from[0], to[1] - from[1]];
    let length = span[0] * span[0] + span[1] * span[1];
    if length <= f64::EPSILON {
        return from;
    }
    let t = (((point[0] - from[0]) * span[0] + (point[1] - from[1]) * span[1]) / length).clamp(0.0, 1.0);
    [from[0] + span[0] * t, from[1] + span[1] * t]
}

/// Nearest point to `point` on the outline of a projected shape.
fn nearest_on_shape(shape: &Shape2, point: [f64; 2]) -> [f64; 2] {
    match shape {
        Shape2::Rect(a, b) => {
            let corners = [[a[0], a[1]], [b[0], a[1]], [b[0], b[1]], [a[0], b[1]]];
            (0..4)
                .map(|index| nearest_on_segment(point, corners[index], corners[(index + 1) % 4]))
                .min_by(|left, right| {
                    let left = (left[0] - point[0]).powi(2) + (left[1] - point[1]).powi(2);
                    let right = (right[0] - point[0]).powi(2) + (right[1] - point[1]).powi(2);
                    left.total_cmp(&right)
                })
                .unwrap_or(*a)
        }
        Shape2::Disc(center, radius) => {
            let span = [point[0] - center[0], point[1] - center[1]];
            let length = (span[0] * span[0] + span[1] * span[1]).sqrt();
            if length <= f64::EPSILON {
                [center[0] + *radius, center[1]]
            }
            else {
                [center[0] + span[0] / length * radius, center[1] + span[1] / length * radius]
            }
        }
        Shape2::Poly(points) => {
            if points.len() < 2 {
                return points.first().copied().unwrap_or(point);
            }
            (0..points.len())
                .map(|index| nearest_on_segment(point, points[index], points[(index + 1) % points.len()]))
                .min_by(|left, right| {
                    let left = (left[0] - point[0]).powi(2) + (left[1] - point[1]).powi(2);
                    let right = (right[0] - point[0]).powi(2) + (right[1] - point[1]).powi(2);
                    left.total_cmp(&right)
                })
                .unwrap_or(points[0])
        }
    }
}
/// Bounds of a shape list, `None` when the list is empty.
fn shapes_bounds(shapes: &[Shape2]) -> Option<([f64; 2], [f64; 2])> {
    shapes.iter().map(Shape2::bounds).reduce(|(a, b), (c, d)| ([a[0].min(c[0]), a[1].min(c[1])], [b[0].max(d[0]), b[1].max(d[1])]))
}

/// Union of two optional bounds, `None` only when both are empty.
fn union_bounds(left: Option<([f64; 2], [f64; 2])>, right: Option<([f64; 2], [f64; 2])>) -> Option<([f64; 2], [f64; 2])> {
    match (left, right) {
        (Some((a, b)), Some((c, d))) => Some(([a[0].min(c[0]), a[1].min(c[1])], [b[0].max(d[0]), b[1].max(d[1])])),
        (Some(bounds), None) | (None, Some(bounds)) => Some(bounds),
        (None, None) => None,
    }
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

/// Scale annotation such as `SCALE 1:2` for a model-to-paper factor.
fn scale_annotation(scale: f64) -> String {
    if scale >= 1.0 { format!("SCALE {}:1", format_number(scale)) } else { format!("SCALE 1:{}", format_number(1.0 / scale)) }
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
        .reduce(|(a, b), (c, d)| ([a[0].min(c[0]), a[1].min(c[1])], [b[0].max(d[0]), b[1].max(d[1])]))
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
            let scale_text = scale_annotation(scale);
            kinds.push((
                layer.clone(),
                EntityKind::Text {
                    at: [view.center[0] - text_width(&scale_text, text_height) / 2.0, view.center[1] - view.size[1] / 2.0 - text_height * 2.9],
                    height: text_height,
                    content: scale_text,
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
            at: [(min[0] + max[0]) / 2.0 - text_width(&label, text_height) / 2.0, max[1] + text_height * 3.4],
            height: text_height,
            content: label.clone(),
        });
        let scale_text = "SCALE 1:1";
        kinds.push(EntityKind::Text {
            at: [(min[0] + max[0]) / 2.0 - text_width(scale_text, text_height) / 2.0, max[1] + text_height * 1.5],
            height: text_height,
            content: scale_text.to_string(),
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
                let bx = dx;
                let by = dy;
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

/// Arguments of [`add_isometric_view`].
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct IsometricViewRequest {
    /// The projected part; needs at least one solid feature.
    pub part: PartSpec,
    /// Linear scale factor applied to the view; defaults to `1`.
    pub scale: Option<f64>,
    /// Height of the view label and axis letters; defaults to `3.5`.
    pub text_height: Option<f64>,
    /// Lower-left corner of the placed view; defaults to the origin.
    pub origin: Option<[f64; 2]>,
    /// Caption under the view; defaults to `ISOMETRIC`.
    pub label: Option<String>,
    /// Layer carrying the outlines and labels; defaults to `views`.
    pub layer: Option<String>,
}

/// Draws a standalone isometric view of a part with an axis indicator.
///
/// The solids project to visible outlines on `layer` (default `views`), voids
/// to dashed hidden outlines on a generated `hidden` layer, and the view
/// carries a caption centred below it plus a three-line axis indicator with
/// `X`, `Y` and `Z` letters at its upper right. The whole group is scaled
/// about the view origin and its lower-left corner is placed at `origin`.
/// The edit is one history step and one revision, and the response carries
/// `{ revision, drawing, created, view }` where `view` reports the placed
/// centre and size.
///
/// A part without solids, a non-finite or non-positive scale or text height,
/// a non-finite origin, an over-long label and a locked or frozen target
/// layer report `session/argument-invalid`, `drawing/layer-locked` or
/// `drawing/layer-frozen`.
pub fn add_isometric_view(handle: u64, request: IsometricViewRequest) -> Result<Value, ServiceError> {
    validate_part(&request.part)?;
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
    let label =
        request.label.map(|value| value.trim().to_string()).filter(|value| !value.is_empty()).unwrap_or_else(|| "ISOMETRIC".to_string());
    if label.chars().count() > PROPERTY_VALUE_LIMIT {
        return Err(argument("the view label is too long"));
    }
    let layer = request.layer.map(|name| name.trim().to_string()).filter(|name| !name.is_empty()).unwrap_or_else(|| VIEWS_LAYER.to_string());
    let has_voids = !request.part.voids.is_empty();
    let (created, size) = store::edit(handle, |drawing| {
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
        let view = place_view("ISO", request.part.solids.iter().flat_map(project_iso), request.part.voids.iter().flat_map(project_iso), scale);
        let size = view.size;
        let shift = [origin[0] + view.size[0] / 2.0, origin[1] + view.size[1] / 2.0];
        let min = [origin[0], origin[1]];
        let max = [origin[0] + view.size[0], origin[1] + view.size[1]];
        let mut kinds: Vec<(String, EntityKind)> = Vec::new();
        for shape in &view.shapes {
            kinds.push((layer.clone(), shape.translated(shift).kind()));
        }
        for shape in &view.hidden {
            kinds.push((HIDDEN_LAYER.to_string(), shape.translated(shift).kind()));
        }
        kinds.push((
            layer.clone(),
            EntityKind::Text {
                at: [shift[0] - text_width(&label, text_height) / 2.0, min[1] - text_height * 1.5],
                height: text_height,
                content: label.clone(),
            },
        ));
        let scale_text = scale_annotation(scale);
        kinds.push((
            layer.clone(),
            EntityKind::Text {
                at: [shift[0] - text_width(&scale_text, text_height) / 2.0, min[1] - text_height * 2.9],
                height: text_height,
                content: scale_text,
            },
        ));
        let (sin, cos) = FRAC_PI_6.sin_cos();
        let anchor = [max[0] + text_height * 1.5, max[1] + text_height * 1.5];
        let length = text_height * 4.0;
        let arms = [("X", [cos * length, sin * length]), ("Y", [-cos * length, sin * length]), ("Z", [0.0, length])];
        for (letter, direction) in arms {
            let end = [anchor[0] + direction[0], anchor[1] + direction[1]];
            kinds.push((layer.clone(), EntityKind::Line { from: anchor, to: end }));
            let unit = [direction[0] / length, direction[1] / length];
            kinds.push((
                layer.clone(),
                EntityKind::Text {
                    at: [end[0] + unit[0] * text_height * 0.4 - text_width(letter, text_height) / 2.0, end[1] + unit[1] * text_height * 0.4],
                    height: text_height,
                    content: letter.to_string(),
                },
            ));
        }
        let mut created = Vec::with_capacity(kinds.len());
        for (view_layer, kind) in kinds {
            let entity = Entity { id: drawing.take_entity_id(), layer: view_layer, kind };
            drawing.entities.push(entity.clone());
            created.push(entity);
        }
        drawing.revision += 1;
        Ok((created, size))
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "created": created,
            "view": {
                "center": [origin[0] + size[0] / 2.0, origin[1] + size[1] / 2.0],
                "width": size[0],
                "height": size[1],
            },
        })
    })
}

/// One component of an exploded assembly.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AssemblyComponent {
    /// Optional component name written beside the balloon and in the BOM.
    pub name: Option<String>,
    /// The component part; needs at least one solid feature.
    pub part: PartSpec,
    /// Assembled position offset along the part axes; defaults to the origin.
    pub position: Option<[f64; 3]>,
    /// Explode translation applied on top of `position`; defaults to zero.
    pub explode: Option<[f64; 3]>,
    /// Quantity reported in the BOM; defaults to `1`.
    pub quantity: Option<u32>,
    /// Optional material reported in the BOM; defaults to an empty cell.
    pub material: Option<String>,
}

/// Arguments of [`add_exploded_assembly`].
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ExplodedAssemblyRequest {
    /// The exploded components; at least one, at most 64.
    pub components: Vec<AssemblyComponent>,
    /// Linear scale factor applied to every component; defaults to `1`.
    pub scale: Option<f64>,
    /// Height of the balloon numbers and names; defaults to `3.5`.
    pub text_height: Option<f64>,
    /// Lower-left corner of the placed group; defaults to the origin.
    pub origin: Option<[f64; 2]>,
    /// Layer carrying the outlines and balloons; defaults to `views`.
    pub layer: Option<String>,
}

/// Maximum number of balloon items one exploded assembly may carry.
const BALLOON_LIMIT: usize = 64;

/// Projected shapes, hidden shapes and bounds of one exploded component.
type PlacedComponent = (Vec<Shape2>, Vec<Shape2>, ([f64; 2], [f64; 2]));

/// Draws an exploded assembly view with numbered balloons and a BOM.
///
/// Every component projects to an isometric image at `position + explode`;
/// solids land on `layer` (default `views`), voids as dashed outlines on a
/// generated `hidden` layer. Balloons carrying the 1-based component numbers
/// are distributed evenly on a ring around the group, each leader ending on
/// the nearest point of its component outline, and an optional `name` sits
/// outside its balloon. A framed `BILL OF MATERIALS` table below the group
/// lists `ITEM`, `PART`, `QTY` and `MATERIAL` from the component order,
/// `quantity` (default `1`) and material. The group is scaled and its
/// lower-left corner is placed at `origin`. The edit is one history step and
/// one revision, and the response carries
/// `{ revision, drawing, created, items }` where `items` reports the number,
/// name, quantity, material, placed centre and balloon point of each
/// component.
///
/// An empty or over-long component list, a component without solids, a
/// non-finite position or explode offset, a non-finite or non-positive scale
/// or text height, a non-finite origin, an over-long name or material and a
/// locked or frozen target layer report `session/argument-invalid`,
/// `drawing/layer-locked` or `drawing/layer-frozen`.
pub fn add_exploded_assembly(handle: u64, request: ExplodedAssemblyRequest) -> Result<Value, ServiceError> {
    if request.components.is_empty() {
        return Err(argument("the assembly needs at least one component"));
    }
    if request.components.len() > BALLOON_LIMIT {
        return Err(argument(format!("the assembly carries more than {BALLOON_LIMIT} components")));
    }
    for component in &request.components {
        validate_part(&component.part)?;
        if component.position.map(|value| !finite3(&value)).unwrap_or(false) {
            return Err(argument("component `position` must contain finite coordinates"));
        }
        if component.explode.map(|value| !finite3(&value)).unwrap_or(false) {
            return Err(argument("component `explode` must contain finite coordinates"));
        }
        if component.name.as_deref().map(|value| value.trim().chars().count() > PROPERTY_VALUE_LIMIT).unwrap_or(false) {
            return Err(argument("a component name is too long"));
        }
        if component.material.as_deref().map(|value| value.trim().chars().count() > PROPERTY_VALUE_LIMIT).unwrap_or(false) {
            return Err(argument("a component material is too long"));
        }
        if component.quantity == Some(0) {
            return Err(argument("a component `quantity` must be at least 1"));
        }
    }
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
    let has_voids = request.components.iter().any(|component| !component.part.voids.is_empty());
    let created_items = store::edit(handle, |drawing| {
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
        let mut placed: Vec<PlacedComponent> = Vec::with_capacity(request.components.len());
        for component in &request.components {
            let delta = [
                component.position.map(|p| p[0]).unwrap_or(0.0) + component.explode.map(|e| e[0]).unwrap_or(0.0),
                component.position.map(|p| p[1]).unwrap_or(0.0) + component.explode.map(|e| e[1]).unwrap_or(0.0),
                component.position.map(|p| p[2]).unwrap_or(0.0) + component.explode.map(|e| e[2]).unwrap_or(0.0),
            ];
            let shift2 = iso_point(&delta);
            let shapes: Vec<Shape2> =
                component.part.solids.iter().flat_map(project_iso).map(|shape| shape.scaled(scale).translated(shift2)).collect();
            let hidden: Vec<Shape2> =
                component.part.voids.iter().flat_map(project_iso).map(|shape| shape.scaled(scale).translated(shift2)).collect();
            let bounds = union_bounds(shapes_bounds(&shapes), shapes_bounds(&hidden)).unwrap_or(([0.0, 0.0], [0.0, 0.0]));
            placed.push((shapes, hidden, bounds));
        }
        let group = placed
            .iter()
            .filter_map(|(shapes, hidden, _)| union_bounds(shapes_bounds(shapes), shapes_bounds(hidden)))
            .reduce(|acc, item| union_bounds(Some(acc), Some(item)).unwrap_or(item))
            .unwrap_or(([0.0, 0.0], [0.0, 0.0]));
        let group_min = group.0;
        let group_max = group.1;
        let shift = [origin[0] - group_min[0], origin[1] - group_min[1]];
        let center = [(group_min[0] + group_max[0]) / 2.0, (group_min[1] + group_max[1]) / 2.0];
        let reach = placed
            .iter()
            .map(|(_, _, (min, max))| {
                [[min[0], min[1]], [max[0], min[1]], [min[0], max[1]], [max[0], max[1]]]
                    .iter()
                    .map(|corner| ((corner[0] - center[0]).powi(2) + (corner[1] - center[1]).powi(2)).sqrt())
                    .fold(0.0_f64, f64::max)
            })
            .fold(0.0_f64, f64::max);
        let radius = text_height * 1.5;
        let ring = reach + VIEW_GAP + radius * 2.0;
        let mut order: Vec<usize> = (0..placed.len()).collect();
        order.sort_by(|left, right| {
            let left_angle = (placed[*left].2.0[1] - center[1]).atan2(placed[*left].2.0[0] - center[0]);
            let right_angle = (placed[*right].2.0[1] - center[1]).atan2(placed[*right].2.0[0] - center[0]);
            left_angle.total_cmp(&right_angle).then(left.cmp(right))
        });
        let start_angle = order
            .first()
            .map(|index| (placed[*index].2.0[1] - center[1]).atan2(placed[*index].2.0[0] - center[0]))
            .unwrap_or(std::f64::consts::FRAC_PI_2);
        let step = std::f64::consts::TAU / placed.len() as f64;
        let mut kinds: Vec<(String, EntityKind)> = Vec::new();
        let mut items: Vec<Value> = Vec::with_capacity(placed.len());
        let mut balloon_floor = f64::INFINITY;
        for (rank, index) in order.iter().copied().enumerate() {
            let (shapes, hidden, _) = &placed[index];
            for shape in shapes {
                kinds.push((layer.clone(), shape.translated(shift).kind()));
            }
            for shape in hidden {
                kinds.push((HIDDEN_LAYER.to_string(), shape.translated(shift).kind()));
            }
            let angle = start_angle + step * rank as f64;
            let balloon = [center[0] + ring * angle.cos() + shift[0], center[1] + ring * angle.sin() + shift[1]];
            let local = [balloon[0] - shift[0], balloon[1] - shift[1]];
            let target_local = shapes
                .iter()
                .map(|shape| nearest_on_shape(shape, local))
                .min_by(|left, right| {
                    let left = (left[0] - local[0]).powi(2) + (left[1] - local[1]).powi(2);
                    let right = (right[0] - local[0]).powi(2) + (right[1] - local[1]).powi(2);
                    left.total_cmp(&right)
                })
                .unwrap_or(local);
            let target = [target_local[0] + shift[0], target_local[1] + shift[1]];
            let span = ((target[0] - balloon[0]).powi(2) + (target[1] - balloon[1]).powi(2)).sqrt();
            let direction = if span > 0.0 { [(target[0] - balloon[0]) / span, (target[1] - balloon[1]) / span] } else { [1.0, 0.0] };
            let start = [balloon[0] + direction[0] * radius, balloon[1] + direction[1] * radius];
            kinds.push((layer.clone(), EntityKind::Line { from: start, to: target }));
            kinds.push((layer.clone(), EntityKind::Circle { center: balloon, radius }));
            let number = (index + 1).to_string();
            kinds.push((
                layer.clone(),
                EntityKind::Text {
                    at: [balloon[0] - text_width(&number, text_height) / 2.0, balloon[1] - text_height / 2.0],
                    height: text_height,
                    content: number.clone(),
                },
            ));
            let name = request.components[index].name.as_deref().map(str::trim).filter(|value| !value.is_empty());
            if let Some(name) = name {
                let outward = radius + text_height * 0.5;
                let text_x = if angle.cos() >= 0.0 { balloon[0] + outward } else { balloon[0] - outward - text_width(name, text_height) };
                kinds.push((
                    layer.clone(),
                    EntityKind::Text { at: [text_x, balloon[1] - text_height / 2.0], height: text_height, content: name.to_string() },
                ));
            }
            balloon_floor = balloon_floor.min(balloon[1] - radius);
            let quantity = request.components[index].quantity.unwrap_or(1).max(1);
            let material = request.components[index].material.as_deref().map(str::trim).filter(|value| !value.is_empty());
            items.push(json!({
                "number": index + 1,
                "name": name,
                "quantity": quantity,
                "material": material,
                "center": target,
                "balloon": balloon,
            }));
        }
        push_bom(&mut kinds, &layer, &request.components, [origin[0], origin[1].min(balloon_floor) - VIEW_GAP], text_height);
        let mut created = Vec::with_capacity(kinds.len());
        for (view_layer, kind) in kinds {
            let entity = Entity { id: drawing.take_entity_id(), layer: view_layer, kind };
            drawing.entities.push(entity.clone());
            created.push(entity);
        }
        drawing.revision += 1;
        Ok((created, items))
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "created": created_items.0,
            "items": created_items.1,
        })
    })
}

/// Column headers of the exploded-assembly bill of materials.
const BOM_COLUMNS: [&str; 4] = ["ITEM", "PART", "QTY", "MATERIAL"];

/// Fraction of the BOM width each column occupies, left to right.
const BOM_COLUMN_SHARE: [f64; 4] = [0.1, 0.5, 0.12, 0.28];

/// Caption of the exploded-assembly bill of materials.
const BOM_TITLE: &str = "BILL OF MATERIALS";

/// Appends a framed, column-aligned bill of materials below the balloons.
fn push_bom(kinds: &mut Vec<(String, EntityKind)>, layer: &str, components: &[AssemblyComponent], top_left: [f64; 2], text_height: f64) {
    let width = 70.0 * text_height;
    let row_height = 2.5 * text_height;
    let row_count = components.len() + 2;
    let left = top_left[0];
    let top = top_left[1];
    let bottom = top - row_count as f64 * row_height;
    let right = left + width;
    let line = |from: [f64; 2], to: [f64; 2]| EntityKind::Line { from, to };
    kinds.push((layer.to_string(), line([left, top], [right, top])));
    kinds.push((layer.to_string(), line([right, top], [right, bottom])));
    kinds.push((layer.to_string(), line([right, bottom], [left, bottom])));
    kinds.push((layer.to_string(), line([left, bottom], [left, top])));
    for row in 1..row_count {
        let y = top - row as f64 * row_height;
        kinds.push((layer.to_string(), line([left, y], [right, y])));
    }
    let mut column = left;
    let mut separators = Vec::with_capacity(BOM_COLUMNS.len());
    for share in BOM_COLUMN_SHARE.iter().take(BOM_COLUMNS.len() - 1) {
        column += width * share;
        separators.push(column);
        kinds.push((layer.to_string(), line([column, top - row_height], [column, bottom])));
    }
    let inset = text_height * 0.5;
    let baseline = |row: usize| top - (row + 1) as f64 * row_height + (row_height - text_height) * 0.5;
    kinds.push((layer.to_string(), EntityKind::Text { at: [left + inset, baseline(0)], height: text_height, content: BOM_TITLE.to_string() }));
    for (column_index, title) in BOM_COLUMNS.iter().enumerate() {
        let x = if column_index == 0 { left } else { separators[column_index - 1] };
        kinds.push((layer.to_string(), EntityKind::Text { at: [x + inset, baseline(1)], height: text_height, content: (*title).to_string() }));
    }
    for (index, component) in components.iter().enumerate() {
        let row = index + 2;
        let name = component
            .name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("PART {}", index + 1));
        let values = [
            (index + 1).to_string(),
            name,
            component.quantity.unwrap_or(1).max(1).to_string(),
            component.material.as_deref().map(str::trim).unwrap_or("").to_string(),
        ];
        for (column_index, value) in values.iter().enumerate() {
            if value.is_empty() {
                continue;
            }
            let x = if column_index == 0 { left } else { separators[column_index - 1] };
            kinds.push((layer.to_string(), EntityKind::Text { at: [x + inset, baseline(row)], height: text_height, content: value.clone() }));
        }
    }
}

/// Arguments of [`add_hole_table`].
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HoleTableRequest {
    /// The part whose cylindrical voids are tabulated; needs at least one.
    pub part: PartSpec,
    /// Lower-left corner of the table; defaults to the origin.
    pub origin: Option<[f64; 2]>,
    /// Height of the table text; defaults to `3.5`.
    pub text_height: Option<f64>,
    /// Table caption; defaults to `HOLE TABLE`.
    pub title: Option<String>,
    /// Layer carrying the table; defaults to `holes`.
    pub layer: Option<String>,
}

/// Layer and default caption of the hole table.
const HOLE_TABLE_LAYER: &str = "holes";
const HOLE_TABLE_TITLE: &str = "HOLE TABLE";

/// Column headers of the hole table, left to right.
const HOLE_COLUMNS: [&str; 6] = ["No.", "Axis", "Pos A", "Pos B", "Dia", "Depth"];

/// Fraction of the table width each hole column occupies, left to right.
const HOLE_COLUMN_SHARE: [f64; 6] = [0.1, 0.12, 0.2, 0.2, 0.19, 0.19];

/// Formats one table number with at most three decimals and no trailing zeros.
fn format_number(value: f64) -> String {
    if !value.is_finite() || value.abs() < 0.0005 {
        return "0".to_string();
    }
    let mut text = format!("{value:.3}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    text
}

/// Tabulates the cylindrical voids of a part as a framed hole table.
///
/// Every `cylinder` void becomes one numbered row in void order: the two
/// coordinates transverse to its axis (ascending axis order) are `Pos A` and
/// `Pos B`, the diameter is twice the radius and the depth is the axial
/// length. A caption row sits above the
/// header unless `title` is empty; the bottom-left corner sits at `origin`,
/// the width is `70 * textHeight` and every row is `2.5 * textHeight` tall.
/// The edit is one history step and one revision, and the response carries
/// `{ revision, drawing, columns, rows, created }`.
///
/// A part without cylindrical voids reports `hole-table/empty`; a non-finite
/// or non-positive text height, a non-finite origin and an over-long caption
/// report `session/argument-invalid`; a locked or frozen layer reports
/// `drawing/layer-locked` or `drawing/layer-frozen`.
pub fn add_hole_table(handle: u64, request: HoleTableRequest) -> Result<Value, ServiceError> {
    validate_part(&request.part)?;
    let text_height = request.text_height.unwrap_or(3.5);
    if !text_height.is_finite() || text_height <= 0.0 {
        return Err(argument("`textHeight` must be a positive finite number"));
    }
    let origin = request.origin.unwrap_or([0.0, 0.0]);
    if !finite2(&origin) {
        return Err(argument("`origin` must contain finite coordinates"));
    }
    let caption = match request.title.as_deref() {
        None => Some(HOLE_TABLE_TITLE.to_string()),
        Some(raw) => {
            let value = raw.trim().to_string();
            if value.chars().count() > PROPERTY_VALUE_LIMIT {
                return Err(argument("the table title is too long"));
            }
            (!value.is_empty()).then_some(value)
        }
    };
    let layer =
        request.layer.map(|name| name.trim().to_string()).filter(|name| !name.is_empty()).unwrap_or_else(|| HOLE_TABLE_LAYER.to_string());
    let width = text_height * 70.0;
    if !width.is_finite() {
        return Err(argument("`textHeight` is too large"));
    }
    let row_height = text_height * 2.5;
    let (created, rows) = store::edit(handle, |drawing| {
        let holes: Vec<(Axis3, [f64; 2], f64, f64, f64)> = request
            .part
            .voids
            .iter()
            .filter_map(|feature| {
                if let Feature3d::Cylinder { axis, center, from, to, radius } = feature {
                    Some((*axis, *center, from.min(*to), from.max(*to), *radius))
                }
                else {
                    None
                }
            })
            .collect();
        if holes.is_empty() {
            return Err(ServiceError::new("hole-table/empty", "the part has no cylindrical void features to tabulate"));
        }
        ensure_layer(drawing, &layer);
        if let Some(error) = locked_layer(drawing, &layer) {
            return Err(error);
        }
        let cells: Vec<[String; 6]> = holes
            .iter()
            .enumerate()
            .map(|(index, (axis, center, from, to, radius))| {
                [
                    (index + 1).to_string(),
                    match axis {
                        Axis3::X => "X".to_string(),
                        Axis3::Y => "Y".to_string(),
                        Axis3::Z => "Z".to_string(),
                    },
                    format_number(center[0]),
                    format_number(center[1]),
                    format_number(2.0 * radius),
                    format_number(to - from),
                ]
            })
            .collect();
        let rows: Vec<Value> = holes
            .iter()
            .enumerate()
            .map(|(index, (axis, center, from, to, radius))| {
                json!({
                    "no": index + 1,
                    "axis": match axis { Axis3::X => "x", Axis3::Y => "y", Axis3::Z => "z" },
                    "posA": center[0],
                    "posB": center[1],
                    "diameter": 2.0 * radius,
                    "depth": to - from,
                })
            })
            .collect();
        let [x0, y0] = origin;
        let x1 = x0 + width;
        let y1 = y0 + row_height * (cells.len() + 1 + usize::from(caption.is_some())) as f64;
        let mut kinds = Vec::new();
        kinds.extend_from_slice(&[
            EntityKind::Line { from: [x0, y0], to: [x1, y0] },
            EntityKind::Line { from: [x1, y0], to: [x1, y1] },
            EntityKind::Line { from: [x1, y1], to: [x0, y1] },
            EntityKind::Line { from: [x0, y1], to: [x0, y0] },
        ]);
        let mut separator = y1;
        if caption.is_some() {
            separator -= row_height;
            kinds.push(EntityKind::Line { from: [x0, separator], to: [x1, separator] });
        }
        separator -= row_height;
        kinds.push(EntityKind::Line { from: [x0, separator], to: [x1, separator] });
        for _ in 1..cells.len() {
            separator -= row_height;
            kinds.push(EntityKind::Line { from: [x0, separator], to: [x1, separator] });
        }
        let mut column_x = [0.0f64; HOLE_COLUMN_SHARE.len()];
        let mut cursor = x0;
        for (index, share) in HOLE_COLUMN_SHARE.iter().enumerate() {
            column_x[index] = cursor;
            cursor += width * share;
        }
        let inset = text_height * 0.5;
        let header_bottom = y1 - row_height * (1.0 + f64::from(u8::from(caption.is_some())));
        for x in &column_x[1..] {
            kinds.push(EntityKind::Line { from: [*x, y0], to: [*x, header_bottom] });
        }
        let mut baseline = y1 - row_height + (row_height - text_height) / 2.0;
        if let Some(caption) = &caption {
            kinds.push(EntityKind::Text { at: [x0 + inset, baseline], height: text_height, content: caption.clone() });
            baseline -= row_height;
        }
        for (index, header) in HOLE_COLUMNS.iter().enumerate() {
            kinds.push(EntityKind::Text { at: [column_x[index] + inset, baseline], height: text_height, content: (*header).to_string() });
        }
        baseline -= row_height;
        for row in &cells {
            for (index, value) in row.iter().enumerate() {
                kinds.push(EntityKind::Text { at: [column_x[index] + inset, baseline], height: text_height, content: value.clone() });
            }
            baseline -= row_height;
        }
        let mut created = Vec::with_capacity(kinds.len());
        for kind in kinds {
            let entity = Entity { id: drawing.take_entity_id(), layer: layer.clone(), kind };
            drawing.entities.push(entity.clone());
            created.push(entity);
        }
        drawing.revision += 1;
        Ok((created, rows))
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "columns": HOLE_COLUMNS,
            "rows": rows,
            "created": created,
        })
    })
}

#[cfg(test)]
#[path = "../tests/unit/views.rs"]
mod tests;

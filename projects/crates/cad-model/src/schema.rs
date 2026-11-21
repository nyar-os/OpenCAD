//! Drawing document schema shared by the native store, the CLI and the SDK.
//!
//! The schema is deliberately small: one drawing, ordered layers and a flat
//! entity list whose geometry is limited to lines, circles, arcs, ellipses,
//! polylines, text, linear dimensions, radial/diameter dimensions and ordinate
//! dimensions. Every value is serialisable, so the
//! same document is written to `.ocad` files and returned to callers as JSON.

use geometry_planar::{Point as PlanarPoint, arc_points};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Layer identifier reserved for the default geometry layer.
pub const DEFAULT_LAYER: &str = "0";

/// Layer identifier reserved for dimensions.
pub const DIMENSION_LAYER: &str = "dimensions";

/// Angular step, in degrees, used when a curve is sampled into points.
pub const CURVE_STEP_DEGREES: f64 = 5.0;

/// Unit a drawing declares. The unit is an enum, so an opened file can never
/// carry an unrecognised one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Units {
    /// Millimetres.
    #[default]
    Mm,
    /// Inches.
    Inch,
}

impl Units {
    /// Short lowercase name used in JSON and file headers.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mm => "mm",
            Self::Inch => "inch",
        }
    }

    /// Parses a declared unit name. Unknown names are rejected instead of
    /// defaulting, so a typo cannot silently change a drawing's scale.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "mm" | "millimeter" | "millimetre" | "millimeters" | "millimetres" => Some(Self::Mm),
            "inch" | "in" | "inches" => Some(Self::Inch),
            _ => None,
        }
    }

    /// `INSUNITS` code used in a DXF header (1 = inches, 4 = millimetres).
    pub fn dxf_insunits(self) -> i64 {
        match self {
            Self::Mm => 4,
            Self::Inch => 1,
        }
    }
}

/// Name of the solid line type every drawing declares.
pub const CONTINUOUS_LINETYPE: &str = "continuous";

/// Most dash lengths one line type pattern may carry.
pub const LINETYPE_PATTERN_LIMIT: usize = 32;

/// Heaviest lineweight a layer may declare, in millimetres.
pub const MAX_LINEWEIGHT: f64 = 10.0;

/// One named line type, mirroring the DXF `LTYPE` table entry.
///
/// `pattern` holds dash lengths in drawing units, alternating dash and gap. An
/// empty pattern is a solid line; `0.0` is a dot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Linetype {
    /// Stable line type identifier inside the document; equals the name.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Dash lengths in drawing units; empty means a solid line.
    pub pattern: Vec<f64>,
}

impl Linetype {
    /// Builds the `continuous` line type every drawing declares by default.
    pub fn continuous() -> Self {
        Self {
            id: CONTINUOUS_LINETYPE.to_string(),
            name: CONTINUOUS_LINETYPE.to_string(),
            description: "Solid line".to_string(),
            pattern: Vec::new(),
        }
    }

    /// Validates the invariants an opened document must keep.
    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() || self.name.trim().is_empty() {
            return Err("a line type needs a non-empty name".to_string());
        }
        if self.description.chars().count() > 256 {
            return Err(format!("line type `{}` has a description longer than 256 characters", self.name));
        }
        if self.pattern.len() > LINETYPE_PATTERN_LIMIT {
            return Err(format!("line type `{}` has more than {LINETYPE_PATTERN_LIMIT} dashes", self.name));
        }
        if self.pattern.iter().any(|dash| !dash.is_finite() || *dash < 0.0) {
            return Err(format!("line type `{}` has a negative or non-finite dash", self.name));
        }
        if !self.pattern.is_empty() && self.pattern.iter().all(|dash| *dash == 0.0) {
            return Err(format!("line type `{}` has a pattern of only zero dashes", self.name));
        }
        Ok(())
    }
}

/// Line types a document starts with.
pub fn default_linetypes() -> Vec<Linetype> {
    vec![Linetype::continuous()]
}

/// Name assigned to a layer that does not declare a line type.
fn default_linetype_name() -> String {
    CONTINUOUS_LINETYPE.to_string()
}

/// Whether a layer that does not declare a plot flag is plotted.
fn default_plottable() -> bool {
    true
}

/// One layer of a drawing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    /// Stable layer identifier inside the document.
    pub id: String,
    /// Display name.
    pub name: String,
    /// CSS colour, e.g. `#d0d7de`.
    pub color: String,
    /// Whether the layer is painted by the exporters.
    pub visible: bool,
    /// Whether new and existing geometry on the layer is writable.
    pub locked: bool,
    /// Whether the layer is frozen. A frozen layer behaves like a hidden one:
    /// exporters skip it and writing to it is refused.
    #[serde(default)]
    pub frozen: bool,
    /// Whether plots and print exports include the layer.
    #[serde(default = "default_plottable")]
    pub plottable: bool,
    /// Line type id assigned to the layer's geometry.
    #[serde(default = "default_linetype_name")]
    pub linetype: String,
    /// Lineweight in millimetres; `0` uses the exporter default.
    #[serde(default)]
    pub lineweight: f64,
}

impl Layer {
    /// Builds the default geometry layer.
    pub fn default_geometry() -> Self {
        Self {
            id: DEFAULT_LAYER.to_string(),
            name: DEFAULT_LAYER.to_string(),
            color: "#d0d7de".to_string(),
            visible: true,
            locked: false,
            frozen: false,
            plottable: true,
            linetype: CONTINUOUS_LINETYPE.to_string(),
            lineweight: 0.0,
        }
    }

    /// Builds the default dimension layer.
    pub fn default_dimension() -> Self {
        Self {
            id: DIMENSION_LAYER.to_string(),
            name: DIMENSION_LAYER.to_string(),
            color: "#0b7285".to_string(),
            visible: true,
            locked: false,
            frozen: false,
            plottable: true,
            linetype: CONTINUOUS_LINETYPE.to_string(),
            lineweight: 0.0,
        }
    }
}

/// One named dimension style, mirroring the DXF `DIMSTYLE` table entry.
///
/// Only the variables the document model honours are kept: the overall scale,
/// arrowhead size, text height, extension line length and origin offset, and
/// the number of decimal places in a measured label.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DimensionStyle {
    /// Stable style identifier inside the document; equals the name.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Overall scale applied to the style's lengths (DXF `DIMSCALE`).
    pub scale: f64,
    /// Arrowhead size in drawing units (DXF `DIMASZ`).
    pub arrow_size: f64,
    /// Dimension text height in drawing units (DXF `DIMTXT`).
    pub text_height: f64,
    /// Extension line length beyond the dimension line (DXF `DIMEXE`).
    pub extension: f64,
    /// Gap between a measured point and its extension line (DXF `DIMEXO`).
    pub origin_offset: f64,
    /// Decimal places in a measured label, `0..=8` (DXF `DIMDEC`).
    pub precision: u32,
}

impl DimensionStyle {
    /// Builds the `standard` style every drawing declares by default.
    pub fn standard() -> Self {
        Self {
            id: "standard".to_string(),
            name: "standard".to_string(),
            scale: 1.0,
            arrow_size: 2.5,
            text_height: 2.5,
            extension: 1.25,
            origin_offset: 0.625,
            precision: 2,
        }
    }

    /// Validates the numeric invariants an opened document must keep.
    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() || self.name.trim().is_empty() {
            return Err("a dimension style needs a non-empty name".to_string());
        }
        let positive = |value: f64| value.is_finite() && value > 0.0;
        let non_negative = |value: f64| value.is_finite() && value >= 0.0;
        if !positive(self.scale) {
            return Err(format!("dimension style `{}` has a non-positive scale", self.name));
        }
        if !positive(self.arrow_size) || !positive(self.text_height) {
            return Err(format!("dimension style `{}` has a non-positive size", self.name));
        }
        if !non_negative(self.extension) || !non_negative(self.origin_offset) {
            return Err(format!("dimension style `{}` has a negative offset", self.name));
        }
        if self.precision > 8 {
            return Err(format!("dimension style `{}` has a precision above 8", self.name));
        }
        Ok(())
    }
}

/// Dimension styles a document starts with.
pub fn default_dimension_styles() -> Vec<DimensionStyle> {
    vec![DimensionStyle::standard()]
}

/// One named text style, mirroring the DXF `STYLE` table entry.
///
/// The style keeps the primary font, a fixed cap height (`0` means each text
/// entity supplies its own), the width factor and the oblique angle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextStyle {
    /// Stable style identifier inside the document; equals the name.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Primary font file or family name (DXF code 3).
    pub font: String,
    /// Fixed cap height in drawing units; `0` leaves the height to the entity.
    pub height: f64,
    /// Horizontal width factor, `1.0` is normal (DXF code 41).
    pub width_factor: f64,
    /// Oblique angle in degrees (DXF code 50).
    pub oblique: f64,
}

impl TextStyle {
    /// Builds the `standard` style every drawing declares by default.
    pub fn standard() -> Self {
        Self { id: "standard".to_string(), name: "standard".to_string(), font: "txt".to_string(), height: 0.0, width_factor: 1.0, oblique: 0.0 }
    }

    /// Validates the invariants an opened document must keep.
    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() || self.name.trim().is_empty() {
            return Err("a text style needs a non-empty name".to_string());
        }
        if self.font.trim().is_empty() {
            return Err(format!("text style `{}` has an empty font", self.name));
        }
        if !self.height.is_finite() || self.height < 0.0 {
            return Err(format!("text style `{}` has a negative height", self.name));
        }
        if !self.width_factor.is_finite() || self.width_factor <= 0.0 {
            return Err(format!("text style `{}` has a non-positive width factor", self.name));
        }
        if !self.oblique.is_finite() || !(-85.0..=85.0).contains(&self.oblique) {
            return Err(format!("text style `{}` has an oblique angle outside -85..85", self.name));
        }
        Ok(())
    }
}

/// Text styles a document starts with.
pub fn default_text_styles() -> Vec<TextStyle> {
    vec![TextStyle::standard()]
}

/// Descriptive properties of a drawing, kept beside the geometry.
///
/// These fields carry the values a title block reports (author, description,
/// company, project and keywords) without changing any coordinate. An empty
/// string means the field is not set.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DrawingProperties {
    /// Person or team that produced the drawing.
    pub author: String,
    /// Free-form description of the drawing.
    pub description: String,
    /// Company or organisation the drawing belongs to.
    pub company: String,
    /// Project name.
    pub project: String,
    /// Keyword list, kept verbatim.
    pub keywords: String,
}

/// Longest value a [`DrawingProperties`] field accepts.
pub const PROPERTY_VALUE_LIMIT: usize = 512;

impl DrawingProperties {
    /// Validates the length bounds an opened document must keep.
    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("author", &self.author),
            ("description", &self.description),
            ("company", &self.company),
            ("project", &self.project),
            ("keywords", &self.keywords),
        ] {
            if value.chars().count() > PROPERTY_VALUE_LIMIT {
                return Err(format!("drawing {name} is longer than {PROPERTY_VALUE_LIMIT} characters"));
            }
        }
        Ok(())
    }
}

/// One named group of entities.
///
/// Membership is metadata: grouping never changes geometry, layers or the
/// revision of an entity, and member ids point at entities of the same drawing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityGroup {
    /// Stable group identifier inside the document; equals the name.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Member entity ids in insertion order.
    pub entities: Vec<String>,
}

/// Geometry carried by one entity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EntityKind {
    /// A straight segment between two points.
    Line {
        /// Start point `[x, y]`.
        from: [f64; 2],
        /// End point `[x, y]`.
        to: [f64; 2],
    },
    /// A full circle.
    Circle {
        /// Centre `[x, y]`.
        center: [f64; 2],
        /// Radius in drawing units.
        radius: f64,
    },
    /// A circular arc swept counter-clockwise from the start to the end angle.
    Arc {
        /// Centre `[x, y]`.
        center: [f64; 2],
        /// Radius in drawing units.
        radius: f64,
        /// Start angle in degrees, measured from the positive X axis.
        start_angle: f64,
        /// End angle in degrees, swept counter-clockwise from the start angle.
        end_angle: f64,
    },
    /// A full or partial ellipse.
    Ellipse {
        /// Centre `[x, y]`.
        center: [f64; 2],
        /// Radius along the major (local X) axis.
        rx: f64,
        /// Radius along the minor (local Y) axis.
        ry: f64,
        /// Rotation of the major axis in degrees, counter-clockwise.
        rotation: f64,
        /// Start parameter in degrees, measured from the major axis.
        start_angle: f64,
        /// End parameter in degrees, swept counter-clockwise from the start.
        end_angle: f64,
    },
    /// A single point marker.
    Point {
        /// Position `[x, y]`.
        at: [f64; 2],
    },
    /// An ordered point chain, optionally closed.
    Polyline {
        /// Vertices in order.
        points: Vec<[f64; 2]>,
        /// Whether the last vertex connects back to the first.
        #[serde(default)]
        closed: bool,
    },
    /// Single-line text.
    Text {
        /// Baseline anchor `[x, y]`.
        at: [f64; 2],
        /// Cap height in drawing units.
        height: f64,
        /// Text content.
        content: String,
    },
    /// A linear dimension between two points, offset along the normal.
    Dimension {
        /// First measured point `[x, y]`.
        from: [f64; 2],
        /// Second measured point `[x, y]`.
        to: [f64; 2],
        /// Signed distance of the dimension line from the measured segment.
        offset: f64,
        /// Optional label; the measured distance is used when absent.
        #[serde(default)]
        text: Option<String>,
    },
    /// A radial or diameter dimension of a circle or arc.
    RadialDimension {
        /// Centre of the measured circle or arc `[x, y]`.
        center: [f64; 2],
        /// Radius of the measured circle or arc.
        radius: f64,
        /// Angle of the measured point, in degrees from the positive X axis.
        angle: f64,
        /// Signed distance from the measured point to the label along the leader.
        #[serde(default)]
        offset: f64,
        /// Whether the dimension measures the diameter instead of the radius.
        #[serde(default)]
        diameter: bool,
        /// Optional label; `R` or `DIA` followed by the measured value is used
        /// when absent.
        #[serde(default)]
        text: Option<String>,
    },
    /// An ordinate dimension measuring one axis-aligned distance from a datum.
    OrdinateDimension {
        /// Datum origin `[x, y]`.
        origin: [f64; 2],
        /// Measured feature point `[x, y]`.
        feature: [f64; 2],
        /// Axis the dimension measures.
        axis: OrdinateAxis,
        /// Signed perpendicular distance of the dimension line from the origin.
        #[serde(default)]
        offset: f64,
        /// Optional label; the measured value is used when absent.
        #[serde(default)]
        text: Option<String>,
    },
    /// A filled region bounded by one closed polygon.
    ///
    /// The pattern name is kept as a description; the exporters paint a solid
    /// tint because no hatch pattern definition is modelled.
    Hatch {
        /// Boundary vertices in order; the closing edge is implicit.
        points: Vec<[f64; 2]>,
        /// Pattern name, `SOLID` for a solid fill.
        #[serde(default = "default_hatch_pattern")]
        pattern: String,
    },
}

/// Default pattern of a [`EntityKind::Hatch`] that does not name one.
pub fn default_hatch_pattern() -> String {
    "SOLID".to_string()
}

/// Axis an ordinate dimension measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrdinateAxis {
    /// Measure the X coordinate difference.
    X,
    /// Measure the Y coordinate difference.
    Y,
}

impl OrdinateAxis {
    /// Short lowercase name used in JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::X => "x",
            Self::Y => "y",
        }
    }
}

/// One drawable object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    /// Stable entity identifier inside the document, e.g. `entity-1`.
    pub id: String,
    /// Identifier of the owning layer.
    pub layer: String,
    /// Geometry of the entity.
    pub kind: EntityKind,
}

impl Entity {
    /// Returns the coordinates this entity contributes to a bounding box.
    pub fn bounds_points(&self) -> Vec<[f64; 2]> {
        match &self.kind {
            EntityKind::Line { from, to } => vec![*from, *to],
            EntityKind::Circle { center, radius } => {
                let r = radius.abs();
                vec![[center[0] - r, center[1] - r], [center[0] + r, center[1] + r]]
            }
            EntityKind::Arc { center, radius, start_angle, end_angle } => sample_arc(*center, radius.abs(), *start_angle, *end_angle),
            EntityKind::Ellipse { center, rx, ry, rotation, start_angle, end_angle } => {
                sample_ellipse(*center, rx.abs(), ry.abs(), *rotation, *start_angle, *end_angle)
            }
            EntityKind::Point { at } => vec![*at],
            EntityKind::Polyline { points, .. } => points.clone(),
            EntityKind::Text { at, height, .. } => {
                let width = at[0] + height * 0.6 * self.label_len() as f64;
                vec![*at, [width, at[1] + height]]
            }
            EntityKind::Dimension { from, to, offset, .. } => {
                let (p1, p2) = offset_points(*from, *to, *offset);
                vec![*from, *to, p1, p2]
            }
            EntityKind::RadialDimension { center, radius, angle, offset, .. } => {
                let target = radial_target(*center, *radius, *angle);
                vec![*center, target, radial_label(*center, *radius, *angle, *offset)]
            }
            EntityKind::OrdinateDimension { origin, feature, axis, offset, .. } => {
                let (base, guide) = ordinate_points(*origin, *feature, *axis, *offset);
                vec![*origin, *feature, base, guide]
            }
            EntityKind::Hatch { points, .. } => points.clone(),
        }
    }

    fn label_len(&self) -> usize {
        match &self.kind {
            EntityKind::Text { content, .. } => content.chars().count(),
            _ => 0,
        }
    }

    /// Point an absolute move is measured from: the first coordinate the
    /// entity owns.
    pub fn anchor(&self) -> [f64; 2] {
        match &self.kind {
            EntityKind::Line { from, .. } => *from,
            EntityKind::Circle { center, .. } => *center,
            EntityKind::Arc { center, .. } => *center,
            EntityKind::Ellipse { center, .. } => *center,
            EntityKind::Point { at } => *at,
            EntityKind::Polyline { points, .. } => points.first().copied().unwrap_or([0.0, 0.0]),
            EntityKind::Text { at, .. } => *at,
            EntityKind::Dimension { from, .. } => *from,
            EntityKind::RadialDimension { center, .. } => *center,
            EntityKind::OrdinateDimension { origin, .. } => *origin,
            EntityKind::Hatch { points, .. } => points.first().copied().unwrap_or([0.0, 0.0]),
        }
    }

    /// Moves every coordinate of the entity by `delta`.
    pub fn translate(&mut self, delta: [f64; 2]) {
        let shift = |point: &mut [f64; 2]| {
            point[0] += delta[0];
            point[1] += delta[1];
        };
        match &mut self.kind {
            EntityKind::Line { from, to } => {
                shift(from);
                shift(to);
            }
            EntityKind::Circle { center, .. } => shift(center),
            EntityKind::Arc { center, .. } => shift(center),
            EntityKind::Ellipse { center, .. } => shift(center),
            EntityKind::Point { at } => shift(at),
            EntityKind::Polyline { points, .. } => {
                for point in points {
                    shift(point);
                }
            }
            EntityKind::Text { at, .. } => shift(at),
            EntityKind::Dimension { from, to, .. } => {
                shift(from);
                shift(to);
            }
            EntityKind::RadialDimension { center, .. } => shift(center),
            EntityKind::OrdinateDimension { origin, feature, .. } => {
                shift(origin);
                shift(feature);
            }
            EntityKind::Hatch { points, .. } => {
                for point in points {
                    shift(point);
                }
            }
        }
    }

    /// Whether every coordinate and scalar of the entity is finite.
    pub fn is_finite(&self) -> bool {
        match &self.kind {
            EntityKind::Line { from, to } => finite_point(*from) && finite_point(*to),
            EntityKind::Circle { center, radius } => finite_point(*center) && radius.is_finite(),
            EntityKind::Arc { center, radius, start_angle, end_angle } => {
                finite_point(*center) && radius.is_finite() && start_angle.is_finite() && end_angle.is_finite()
            }
            EntityKind::Ellipse { center, rx, ry, rotation, start_angle, end_angle } => {
                finite_point(*center)
                    && rx.is_finite()
                    && ry.is_finite()
                    && rotation.is_finite()
                    && start_angle.is_finite()
                    && end_angle.is_finite()
            }
            EntityKind::Point { at } => finite_point(*at),
            EntityKind::Polyline { points, .. } => points.iter().all(|point| finite_point(*point)),
            EntityKind::Text { at, height, .. } => finite_point(*at) && height.is_finite(),
            EntityKind::Dimension { from, to, offset, .. } => finite_point(*from) && finite_point(*to) && offset.is_finite(),
            EntityKind::RadialDimension { center, radius, angle, offset, .. } => {
                finite_point(*center) && radius.is_finite() && angle.is_finite() && offset.is_finite()
            }
            EntityKind::OrdinateDimension { origin, feature, offset, .. } => {
                finite_point(*origin) && finite_point(*feature) && offset.is_finite()
            }
            EntityKind::Hatch { points, .. } => points.iter().all(|point| finite_point(*point)),
        }
    }

    /// Validates the numeric invariants an opened document must keep.
    ///
    /// Coordinates and angles must be finite, and a circle, arc or ellipse must
    /// carry positive radii. The returned message is caller-facing.
    pub fn validate(&self) -> Result<(), String> {
        if !self.is_finite() {
            return Err(format!("entity `{}` carries a non-finite coordinate", self.id));
        }
        let non_positive = |value: f64| !value.is_finite() || value <= 0.0;
        match &self.kind {
            EntityKind::Arc { radius, .. } if non_positive(*radius) => {
                return Err(format!("entity `{}` has a non-positive radius", self.id));
            }
            EntityKind::Ellipse { rx, ry, .. } if non_positive(*rx) || non_positive(*ry) => {
                return Err(format!("entity `{}` has a non-positive ellipse radius", self.id));
            }
            EntityKind::RadialDimension { radius, .. } if non_positive(*radius) => {
                return Err(format!("entity `{}` has a non-positive radius", self.id));
            }
            EntityKind::Hatch { points, .. } if points.len() < 3 => {
                return Err(format!("entity `{}` hatch needs at least 3 boundary points", self.id));
            }
            _ => {}
        }
        Ok(())
    }
}

/// Computes the two dimension-line endpoints for a measured segment.
///
/// `offset` is applied along the unit normal of the `from -> to` direction. A
/// zero-length segment has no direction, so the vertical normal is used.
pub fn offset_points(from: [f64; 2], to: [f64; 2], offset: f64) -> ([f64; 2], [f64; 2]) {
    let dx = to[0] - from[0];
    let dy = to[1] - from[1];
    let length = (dx * dx + dy * dy).sqrt();
    let normal = if length > f64::EPSILON { [-dy / length, dx / length] } else { [0.0, 1.0] };
    ([from[0] + normal[0] * offset, from[1] + normal[1] * offset], [to[0] + normal[0] * offset, to[1] + normal[1] * offset])
}

/// Point a radial dimension measures on its circle.
pub fn radial_target(center: [f64; 2], radius: f64, angle: f64) -> [f64; 2] {
    arc_point(center, radius.abs(), angle)
}

/// Label anchor of a radial dimension, `offset` beyond the measured point.
pub fn radial_label(center: [f64; 2], radius: f64, angle: f64, offset: f64) -> [f64; 2] {
    let target = radial_target(center, radius, angle);
    let dx = target[0] - center[0];
    let dy = target[1] - center[1];
    let length = (dx * dx + dy * dy).sqrt();
    if length > f64::EPSILON { [target[0] + dx / length * offset, target[1] + dy / length * offset] } else { target }
}

/// Dimension-line base and guide points of an ordinate dimension.
///
/// The base sits on the datum and the guide below the measured feature, both
/// offset perpendicular to the measured axis. `X` measures a horizontal
/// distance, `Y` a vertical one.
pub fn ordinate_points(origin: [f64; 2], feature: [f64; 2], axis: OrdinateAxis, offset: f64) -> ([f64; 2], [f64; 2]) {
    match axis {
        OrdinateAxis::X => ([origin[0], origin[1] + offset], [feature[0], origin[1] + offset]),
        OrdinateAxis::Y => ([origin[0] + offset, origin[1]], [origin[0] + offset, feature[1]]),
    }
}

/// Signed distance an ordinate dimension measures along its axis.
pub fn ordinate_value(origin: [f64; 2], feature: [f64; 2], axis: OrdinateAxis) -> f64 {
    match axis {
        OrdinateAxis::X => feature[0] - origin[0],
        OrdinateAxis::Y => feature[1] - origin[1],
    }
}

/// A drawing document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Drawing {
    /// Stable document identifier.
    pub id: String,
    /// Optional human title.
    pub title: Option<String>,
    /// Monotonic revision; every mutation increments it.
    pub revision: u64,
    /// Declared drawing unit.
    pub units: Units,
    /// Layers in paint order.
    pub layers: Vec<Layer>,
    /// Named dimension styles; a document always keeps at least one.
    #[serde(default = "default_dimension_styles")]
    pub dim_styles: Vec<DimensionStyle>,
    /// Named text styles; a document always keeps at least one.
    #[serde(default = "default_text_styles")]
    pub text_styles: Vec<TextStyle>,
    /// Named entity groups.
    #[serde(default)]
    pub groups: Vec<EntityGroup>,
    /// Named line types; a document always keeps at least one.
    #[serde(default = "default_linetypes")]
    pub linetypes: Vec<Linetype>,
    /// Descriptive title-block properties.
    #[serde(default)]
    pub properties: DrawingProperties,
    /// Entity ids in explicit paint order, back to front. When empty, the layer
    /// order followed by the entity list order is used.
    #[serde(default)]
    pub draw_order: Vec<String>,
    /// Flat entity list.
    pub entities: Vec<Entity>,
    /// Next numeric suffix used for a new entity identifier.
    pub next_entity: u64,
}

impl Drawing {
    /// Creates an empty drawing with the two default layers.
    pub fn empty(title: Option<String>, units: Units) -> Self {
        Self {
            id: "drawing-1".to_string(),
            title,
            revision: 1,
            units,
            layers: vec![Layer::default_geometry(), Layer::default_dimension()],
            dim_styles: default_dimension_styles(),
            text_styles: default_text_styles(),
            groups: Vec::new(),
            linetypes: default_linetypes(),
            properties: DrawingProperties::default(),
            draw_order: Vec::new(),
            entities: Vec::new(),
            next_entity: 1,
        }
    }

    /// Looks up a layer by id.
    pub fn layer(&self, id: &str) -> Option<&Layer> {
        self.layers.iter().find(|layer| layer.id == id)
    }

    /// Looks up a layer by id for editing.
    pub fn layer_mut(&mut self, id: &str) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|layer| layer.id == id)
    }

    /// Entities in paint order, back to front.
    ///
    /// An explicit [`Drawing::draw_order`] paints the listed entities first, in
    /// the order given, and then any remaining entities in layer order followed
    /// by insertion order. An empty draw order keeps the historical layer-first
    /// order.
    pub fn ordered_entities(&self) -> Vec<&Entity> {
        let mut ordered = Vec::with_capacity(self.entities.len());
        let mut seen = std::collections::HashSet::new();
        if self.draw_order.is_empty() {
            for layer in &self.layers {
                for entity in self.entities.iter().filter(|entity| entity.layer == layer.id) {
                    if seen.insert(entity.id.as_str()) {
                        ordered.push(entity);
                    }
                }
            }
        }
        else {
            for id in &self.draw_order {
                if let Some(entity) = self.entities.iter().find(|entity| &entity.id == id)
                    && seen.insert(entity.id.as_str())
                {
                    ordered.push(entity);
                }
            }
        }
        for entity in &self.entities {
            if seen.insert(entity.id.as_str()) {
                ordered.push(entity);
            }
        }
        ordered
    }

    /// Entity ids in paint order, back to front.
    pub fn ordered_entity_ids(&self) -> Vec<String> {
        self.ordered_entities().into_iter().map(|entity| entity.id.clone()).collect()
    }

    /// Drops one id from the explicit paint order.
    pub fn forget_draw_order(&mut self, id: &str) {
        self.draw_order.retain(|existing| existing != id);
    }

    /// JSON summary used by `create`, `open` and the editing calls.
    pub fn summary(&self) -> Value {
        let layers = self
            .layers
            .iter()
            .map(|layer| {
                let entity_count = self.entities.iter().filter(|entity| entity.layer == layer.id).count();
                json!({
                    "id": layer.id,
                    "name": layer.name,
                    "color": layer.color,
                    "visible": layer.visible,
                    "locked": layer.locked,
                    "frozen": layer.frozen,
                    "plottable": layer.plottable,
                    "linetype": layer.linetype,
                    "lineweight": layer.lineweight,
                    "entityCount": entity_count,
                })
            })
            .collect::<Vec<_>>();
        json!({
            "id": self.id,
            "title": self.title,
            "revision": self.revision,
            "units": self.units.as_str(),
            "layers": layers,
            "dimStyles": self.dim_styles,
            "textStyles": self.text_styles,
            "linetypes": self.linetypes,
            "groups": self.groups.len(),
            "properties": self.properties,
            "entities": self.entities.len(),
        })
    }

    /// Assigns the next deterministic entity identifier.
    pub fn take_entity_id(&mut self) -> String {
        let id = format!("entity-{}", self.next_entity);
        self.next_entity += 1;
        id
    }

    /// Validates the invariants an opened document must keep.
    ///
    /// The returned message is caller-facing; the caller wraps it in the
    /// `format/invalid` code so every validation failure is distinguishable
    /// from an unsupported version.
    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() {
            return Err("the drawing id is empty".to_string());
        }
        let mut layer_ids = std::collections::HashSet::new();
        for layer in &self.layers {
            if layer.id.trim().is_empty() {
                return Err("a layer id is empty".to_string());
            }
            if !layer_ids.insert(layer.id.as_str()) {
                return Err(format!("duplicate layer id `{}`", layer.id));
            }
        }
        if self.dim_styles.is_empty() {
            return Err("the drawing has no dimension style".to_string());
        }
        let mut style_ids = std::collections::HashSet::new();
        for style in &self.dim_styles {
            style.validate()?;
            if !style_ids.insert(style.id.as_str()) {
                return Err(format!("duplicate dimension style id `{}`", style.id));
            }
        }
        if self.text_styles.is_empty() {
            return Err("the drawing has no text style".to_string());
        }
        if self.linetypes.is_empty() {
            return Err("the drawing has no line type".to_string());
        }
        let mut linetype_ids = std::collections::HashSet::new();
        for linetype in &self.linetypes {
            linetype.validate()?;
            if !linetype_ids.insert(linetype.id.as_str()) {
                return Err(format!("duplicate line type id `{}`", linetype.id));
            }
        }
        for layer in &self.layers {
            if !linetype_ids.contains(layer.linetype.as_str()) {
                return Err(format!("layer `{}` references missing line type `{}`", layer.id, layer.linetype));
            }
            if !layer.lineweight.is_finite() || !(0.0..=MAX_LINEWEIGHT).contains(&layer.lineweight) {
                return Err(format!("layer `{}` has a lineweight outside 0..={MAX_LINEWEIGHT} mm", layer.id));
            }
        }
        let mut text_style_ids = std::collections::HashSet::new();
        for style in &self.text_styles {
            style.validate()?;
            if !text_style_ids.insert(style.id.as_str()) {
                return Err(format!("duplicate text style id `{}`", style.id));
            }
        }
        self.properties.validate()?;
        let mut entity_ids = std::collections::HashSet::new();
        for entity in &self.entities {
            if entity.id.trim().is_empty() {
                return Err("an entity id is empty".to_string());
            }
            if !entity_ids.insert(entity.id.as_str()) {
                return Err(format!("duplicate entity id `{}`", entity.id));
            }
            if !layer_ids.contains(entity.layer.as_str()) {
                return Err(format!("entity `{}` references missing layer `{}`", entity.id, entity.layer));
            }
            entity.validate()?;
        }
        let mut order_ids = std::collections::HashSet::new();
        for id in &self.draw_order {
            if !entity_ids.contains(id.as_str()) {
                return Err(format!("draw order references missing entity `{id}`"));
            }
            if !order_ids.insert(id.as_str()) {
                return Err(format!("draw order lists entity `{id}` twice"));
            }
        }
        let mut group_ids = std::collections::HashSet::new();
        for group in &self.groups {
            if group.id.trim().is_empty() || group.name.trim().is_empty() {
                return Err("an entity group needs a non-empty name".to_string());
            }
            if !group_ids.insert(group.id.as_str()) {
                return Err(format!("duplicate entity group id `{}`", group.id));
            }
            let mut members = std::collections::HashSet::new();
            for member in &group.entities {
                if !entity_ids.contains(member.as_str()) {
                    return Err(format!("entity group `{}` references missing entity `{member}`", group.id));
                }
                if !members.insert(member.as_str()) {
                    return Err(format!("entity group `{}` lists entity `{member}` twice", group.id));
                }
            }
        }
        Ok(())
    }
}

/// Whether a point has two finite coordinates.
pub fn finite_point(point: [f64; 2]) -> bool {
    point[0].is_finite() && point[1].is_finite()
}

/// Angular sweep of an arc, normalised into `(0, 360]` degrees.
///
/// A start angle equal to the end angle denotes a full turn, matching the
/// counter-clockwise convention of [`EntityKind::Arc`].
pub fn arc_sweep(start_angle: f64, end_angle: f64) -> f64 {
    let sweep = (end_angle - start_angle).rem_euclid(360.0);
    if sweep <= 1e-12 { 360.0 } else { sweep }
}

/// Normalises an angle in degrees into `[0, 360)`.
pub fn normalize_angle(angle: f64) -> f64 {
    let normalized = angle.rem_euclid(360.0);
    if normalized == 360.0 { 0.0 } else { normalized }
}

/// Point on a circle at `angle_degrees`, measured counter-clockwise from the
/// positive X axis.
pub fn arc_point(center: [f64; 2], radius: f64, angle_degrees: f64) -> [f64; 2] {
    let angle = angle_degrees.to_radians();
    [center[0] + radius * angle.cos(), center[1] + radius * angle.sin()]
}

/// Point on an ellipse at parameter `parameter_degrees`.
///
/// The parameter is measured counter-clockwise from the major axis, which is
/// itself rotated by `rotation_degrees`.
pub fn ellipse_point(center: [f64; 2], rx: f64, ry: f64, rotation_degrees: f64, parameter_degrees: f64) -> [f64; 2] {
    let parameter = parameter_degrees.to_radians();
    let rotation = rotation_degrees.to_radians();
    let x = rx * parameter.cos();
    let y = ry * parameter.sin();
    [center[0] + x * rotation.cos() - y * rotation.sin(), center[1] + x * rotation.sin() + y * rotation.cos()]
}

/// Samples an arc into points, at most [`CURVE_STEP_DEGREES`] apart.
pub fn sample_arc(center: [f64; 2], radius: f64, start_angle: f64, end_angle: f64) -> Vec<[f64; 2]> {
    let sweep = arc_sweep(start_angle, end_angle);
    let segments = (sweep / CURVE_STEP_DEGREES).ceil().max(1.0) as usize;
    arc_points(PlanarPoint::new(center[0], center[1]), radius, start_angle.to_radians(), (start_angle + sweep).to_radians(), segments)
        .into_iter()
        .map(|point| [point.x, point.y])
        .collect()
}

/// Samples a full or partial ellipse into points, at most
/// [`CURVE_STEP_DEGREES`] apart.
pub fn sample_ellipse(center: [f64; 2], rx: f64, ry: f64, rotation_degrees: f64, start_angle: f64, end_angle: f64) -> Vec<[f64; 2]> {
    let sweep = arc_sweep(start_angle, end_angle);
    sample_curve(sweep, |step| ellipse_point(center, rx, ry, rotation_degrees, start_angle + step))
}

/// Evaluates `point(step)` from `0` through `sweep`, including both ends.
fn sample_curve(sweep: f64, point: impl Fn(f64) -> [f64; 2]) -> Vec<[f64; 2]> {
    let steps = (sweep / CURVE_STEP_DEGREES).ceil().max(1.0);
    let count = steps as usize;
    (0..=count).map(|index| point(sweep * index as f64 / count as f64)).collect()
}

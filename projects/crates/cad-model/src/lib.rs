//! Drawing model, store and exchange core of OpenCAD.
//!
//! The crate owns a real drawing document: two default layers, a flat entity
//! list (line, circle, arc, ellipse, polyline, text, linear, radial/diameter
//! and ordinate dimension), a
//! process-wide handle store, JSON persistence (`.ocad`), inspection,
//! transform/copy/offset/measure operations, a read-only SVG preview,
//! SVG/DXF/PDF/STEP/STL/OBJ export, a bounded undo/redo history, layer and
//! entity management (including duplication and layer merge), an ASCII DXF
//! reader for a declared subset and faceted BREP and mesh writers that extrude
//! closed profiles.
//!
//! It deliberately does not ship a constraint solver, an interactive viewport,
//! a STEP reader or a modelling kernel. Unimplemented routes report the stable
//! `capability/not-implemented` code instead of a fabricated result; the preview
//! renderer paints an SVG but does not select or manipulate geometry.

#![deny(missing_docs)]

use std::{fs, io, path::Path};

use geometry_planar::{Point as PlanarPoint, Rect, polygon_area};
use serde_json::{Map, Value, json};

mod exchange;
mod import;
mod inspection;
mod schema;
mod store;

pub use schema::{
    CONTINUOUS_LINETYPE, CURVE_STEP_DEGREES, DEFAULT_LAYER, DEFAULT_LAYOUT_HEIGHT, DEFAULT_LAYOUT_WIDTH, DIMENSION_LAYER, DimensionStyle,
    Drawing, DrawingProperties, Entity, EntityGroup, EntityKind, LINETYPE_PATTERN_LIMIT, Layer, Layout, Linetype, MAX_LINEWEIGHT, OrdinateAxis,
    PROPERTY_VALUE_LIMIT, TextStyle, Units, angular_points, angular_sweep, angular_value, arc_point, arc_sweep, default_dimension_styles,
    default_hatch_pattern, default_linetypes, default_text_styles, ellipse_point, finite_point, normalize_angle, offset_points,
    ordinate_points, ordinate_value, radial_label, radial_target, sample_arc, sample_ellipse,
};

/// Stable code reported by every operation that is not wired yet.
pub const NOT_IMPLEMENTED: &str = "capability/not-implemented";

/// Formats the product must classify.
pub const DRAWING_FORMATS: &[&str] = &["dwg", "dxf", "step", "stp", "iges", "igs", "stl", "obj", "svg", "pdf", "png", "jpg"];

/// Inspection checks exposed by [`capabilities`].
pub const INSPECT_CHECKS: &[&str] = &[
    "units",
    "layers",
    "empty-layer",
    "geometry",
    "polylines",
    "constraints",
    "dimensions",
    "references",
    "duplicate-entity",
    "zero-length",
    "open-contour",
];

/// Static render targets.
pub const RENDER_FORMATS: &[&str] = &["svg", "png", "pdf"];

/// Array placements [`array`] accepts.
pub const ARRAY_MODES: &[&str] = &["rectangular", "polar"];

/// Exchange formats [`export`] knows about.
pub const EXPORT_TARGETS: &[&str] = &["dxf", "dxf-r12", "obj", "step", "stl", "svg", "pdf"];

const FILE_FORMAT: &str = "open-cad-drawing";
const FILE_VERSION: u64 = 1;

/// Stable failure of one drawing operation.
#[derive(Debug)]
pub struct ServiceError {
    /// Stable, comparable code.
    pub code: String,
    /// Human-readable message; never carries binary payloads.
    pub message: String,
}

impl ServiceError {
    /// Builds a failure from a stable code and a message.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into() }
    }
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ServiceError {}

fn not_implemented(feature: &str) -> ServiceError {
    ServiceError::new(NOT_IMPLEMENTED, format!("`{feature}` is not implemented in this build"))
}

fn invalid(message: impl Into<String>) -> ServiceError {
    ServiceError::new("format/invalid", message)
}

fn argument(message: impl Into<String>) -> ServiceError {
    ServiceError::new("session/argument-invalid", message)
}

/// What this build can actually do.
///
/// The matrix only claims what the test suite exercises: the native `.ocad`
/// project is partial, SVG, PDF, STEP, STL and OBJ export are supported, DXF
/// export plus the declared ASCII import subset are partial, and the supported
/// document checks run. SVG is the only preview renderer; DWG and the STL
/// reader stay unsupported. STEP and OBJ are partial overall because only their
/// export direction is implemented: `exportTargets.step` and
/// `exportTargets.obj` are supported and there is no STEP or OBJ reader.
/// `entities` and `operations` name the modelled geometry and the wired edit
/// operations (transform, copy, rectangular/polar array, polyline explode and
/// measure); constraint solving and a 3D kernel stay unsupported.
pub fn capabilities() -> Value {
    let map = |keys: &[&str], level: fn(&str) -> &'static str| {
        let mut entries = Map::new();
        for key in keys {
            entries.insert((*key).to_string(), json!(level(key)));
        }
        Value::Object(entries)
    };

    let format_level = |name: &str| match name {
        "svg" | "pdf" => "supported",
        "dxf" | "step" | "obj" => "partial",
        _ => "unsupported",
    };
    let check_level = |name: &str| match name {
        "units" | "layers" | "empty-layer" | "geometry" | "polylines" | "dimensions" | "duplicate-entity" | "zero-length" | "open-contour" => {
            "supported"
        }
        _ => "unsupported",
    };
    let renderer_level = |name: &str| match name {
        "svg" => "supported",
        _ => "unsupported",
    };
    let export_level = |name: &str| match name {
        "svg" | "dxf" | "dxf-r12" | "pdf" | "stl" | "step" | "obj" => "supported",
        _ => "unsupported",
    };

    json!({
        "engine": { "package": env!("CARGO_PKG_NAME"), "version": env!("CARGO_PKG_VERSION") },
        "project": "partial",
        "formats": map(DRAWING_FORMATS, format_level),
        "checks": map(INSPECT_CHECKS, check_level),
        "renderers": map(RENDER_FORMATS, renderer_level),
        "exportTargets": map(EXPORT_TARGETS, export_level),
        "entities": map(
            &[
                "line",
                "circle",
                "arc",
                "ellipse",
                "point",
                "polyline",
                "text",
                "dimension",
                "radial-dimension",
                "ordinate-dimension",
                "angular-dimension",
                "hatch",
            ],
            |name| match name {
                "line" | "circle" | "arc" | "ellipse" | "point" | "polyline" | "text" | "dimension" | "radial-dimension"
                | "ordinate-dimension" | "angular-dimension" | "hatch" => "supported",
                _ => "unsupported",
            },
        ),
        "operations": map(
            &[
                "transform",
                "copy",
                "array",
                "explode",
                "offset",
                "fillet",
                "chamfer",
                "measure",
                "measure-entity",
                "measure-points",
                "stats",
                "dimension-style",
                "text-style",
                "text-search",
                "group",
                "select",
                "properties",
                "snap",
                "linetype",
                "ucs",
                "constraints",
                "kernel3d",
            ],
            |name| match name {
                "transform" | "copy" | "array" | "explode" | "offset" | "fillet" | "chamfer" | "measure" | "measure-entity"
                | "measure-points" | "stats" | "dimension-style" | "text-style" | "text-search" | "group" | "select" | "properties" | "snap"
                | "linetype" | "ucs" => "supported",
                _ => "unsupported",
            },
        ),
    })
}

/// Arguments of [`create`].
#[derive(Debug, Clone, Default)]
pub struct CreateRequest {
    /// Drawing title.
    pub title: Option<String>,
    /// Declared unit, `mm` or `inch`; defaults to `mm`.
    pub units: Option<String>,
    /// Template name. The MVP accepts the field for API compatibility and does
    /// not load a template yet.
    pub template: Option<String>,
}

/// Creates an empty drawing and returns `{ handle, drawing }`.
pub fn create(request: CreateRequest) -> Result<Value, ServiceError> {
    let units = match request.units.as_deref() {
        Some(raw) => Units::parse(raw).ok_or_else(|| argument(format!("`{raw}` is not a drawing unit; use `mm` or `inch`")))?,
        None => Units::Mm,
    };
    let drawing = Drawing::empty(request.title, units);
    let handle = store::insert(drawing);
    let summary = store::read(handle, |session| session.summary())?;
    Ok(json!({ "handle": handle, "drawing": summary }))
}

/// Opens a `.ocad` project and returns `{ handle, drawing }`.
///
/// The file must be UTF-8 JSON with the `open-cad-drawing` tag and version 1.
/// Malformed content reports `format/invalid`; a future version reports
/// `format/unsupported-version`; neither path panics.
pub fn open(path: &Path) -> Result<Value, ServiceError> {
    let bytes = fs::read(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            ServiceError::new("drawing/not-found", format!("`{}` does not exist", path.display()))
        }
        else {
            ServiceError::new("drawing/io-error", format!("could not read `{}`: {error}", path.display()))
        }
    })?;
    let text = String::from_utf8(bytes).map_err(|_| invalid(format!("`{}` is not UTF-8", path.display())))?;
    let value: Value = serde_json::from_str(&text).map_err(|error| invalid(format!("`{}` is not valid JSON: {error}", path.display())))?;

    let format = value.get("format").and_then(Value::as_str).unwrap_or("");
    if format != FILE_FORMAT {
        return Err(invalid(format!("`{}` is not an OpenCAD drawing file", path.display())));
    }
    let version =
        value.get("version").and_then(Value::as_u64).ok_or_else(|| invalid(format!("`{}` has no integer version", path.display())))?;
    if version != FILE_VERSION {
        return Err(ServiceError::new(
            "format/unsupported-version",
            format!("`{}` uses version {version}; this build reads version {FILE_VERSION}", path.display()),
        ));
    }
    let document = value.get("document").ok_or_else(|| invalid(format!("`{}` has no document", path.display())))?;
    let drawing: Drawing =
        serde_json::from_value(document.clone()).map_err(|error| invalid(format!("`{}` has an invalid document: {error}", path.display())))?;
    drawing.validate().map_err(|message| invalid(format!("`{}` is invalid: {message}", path.display())))?;

    let handle = store::insert(drawing);
    let summary = store::read(handle, |session| session.summary())?;
    Ok(json!({ "handle": handle, "drawing": summary }))
}

/// Releases one drawing handle.
pub fn close(handle: u64) -> Result<(), ServiceError> {
    store::remove(handle)
}

/// Writes one open drawing to its project path and returns the byte count.
///
/// The write goes through a temporary file in the target directory and is
/// renamed into place; an existing target is only replaced when `overwrite` is
/// true.
pub fn save(handle: u64, path: &Path, overwrite: bool) -> Result<usize, ServiceError> {
    store::persist(handle, |drawing| {
        if !overwrite && path.exists() {
            return Err(ServiceError::new(
                "drawing/overwrite-refused",
                format!("`{}` already exists; pass overwrite to replace it", path.display()),
            ));
        }
        let envelope = json!({
            "format": FILE_FORMAT,
            "version": FILE_VERSION,
            "document": &drawing,
        });
        let mut bytes = serde_json::to_vec_pretty(&envelope)
            .map_err(|error| ServiceError::new("drawing/io-error", format!("could not serialise the drawing: {error}")))?;
        bytes.push(b'\n');
        write_atomic(path, &bytes)
            .map_err(|error| ServiceError::new("drawing/io-error", format!("could not write `{}`: {error}", path.display())))?;
        Ok(bytes.len())
    })
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let target =
        if path.parent().is_some_and(|parent| !parent.as_os_str().is_empty()) { path.to_path_buf() } else { Path::new(".").join(path) };
    boost_session::save_atomic(&target, bytes)
}

/// Runs the requested checks and reports every finding.
///
/// `units`, `layers`, `geometry`, `polylines`, `dimensions`, `duplicate-entity`,
/// `zero-length` and `open-contour` run by default; `empty-layer` is opt-in and
/// warns about layers without entities. `constraints` and `references` are not
/// implemented and requesting them fails with `capability/not-implemented`
/// rather than reporting a false pass.
pub fn inspect(handle: u64, checks: Option<Vec<String>>) -> Result<Value, ServiceError> {
    let requested = match checks {
        Some(names) if !names.is_empty() => names,
        _ => inspection::DEFAULT_CHECKS.iter().map(|name| (*name).to_string()).collect(),
    };
    let mut selected = Vec::with_capacity(requested.len());
    for raw in requested {
        let name = raw.trim().to_ascii_lowercase();
        if inspection::SUPPORTED_CHECKS.contains(&name.as_str()) {
            selected.push(name);
        }
        else {
            return Err(not_implemented(&format!("inspection check `{name}`")));
        }
    }
    let refs = selected.iter().map(String::as_str).collect::<Vec<_>>();
    store::read(handle, |session| inspection::run(&session.drawing, &refs).to_json())
}

/// Summarises one drawing: units, layers and entity count.
pub fn summary(handle: u64) -> Result<Value, ServiceError> {
    store::read(handle, |session| session.summary())
}

/// Restores the previous snapshot of one drawing and returns its summary.
pub fn undo(handle: u64) -> Result<Value, ServiceError> {
    store::undo(handle)?;
    store::read(handle, history_report)
}

/// Reapplies the snapshot {@link undo} replaced and returns its summary.
pub fn redo(handle: u64) -> Result<Value, ServiceError> {
    store::redo(handle)?;
    store::read(handle, history_report)
}

fn history_report(session: &store::Session) -> Value {
    let summary = session.summary();
    let can_undo = summary["canUndo"].clone();
    let can_redo = summary["canRedo"].clone();
    json!({
        "revision": session.drawing.revision,
        "drawing": summary,
        "canUndo": can_undo,
        "canRedo": can_redo,
    })
}

/// Fields one drawing-properties update may change.
///
/// Every field is optional; `None` keeps the current value and an empty string
/// clears it. `title` is stored apart from the other properties, so an empty
/// title removes it.
#[derive(Debug, Clone, Default)]
pub struct PropertiesUpdate {
    /// Drawing title; an empty value removes it.
    pub title: Option<String>,
    /// Person or team that produced the drawing.
    pub author: Option<String>,
    /// Free-form description of the drawing.
    pub description: Option<String>,
    /// Company or organisation the drawing belongs to.
    pub company: Option<String>,
    /// Project name.
    pub project: Option<String>,
    /// Keyword list, kept verbatim.
    pub keywords: Option<String>,
}

/// Reads the title and descriptive properties of one drawing.
///
/// The response carries `{ revision, title, properties }`; the query does not
/// change the drawing or add a history step.
pub fn properties(handle: u64) -> Result<Value, ServiceError> {
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "title": session.drawing.title,
            "properties": session.drawing.properties,
        })
    })
}

/// Validates and trims one property value.
fn property_value(raw: &str, field: &str) -> Result<String, ServiceError> {
    let value = raw.trim();
    if value.chars().count() > PROPERTY_VALUE_LIMIT {
        return Err(argument(format!("`{field}` is longer than {PROPERTY_VALUE_LIMIT} characters")));
    }
    Ok(value.to_string())
}

/// Updates the title and descriptive properties of one drawing.
///
/// Values are trimmed and an empty value clears the field. Nothing is written
/// and no history step is added when every field is omitted. The response
/// carries `{ revision, drawing, title, properties }`.
pub fn update_properties(handle: u64, update: PropertiesUpdate) -> Result<Value, ServiceError> {
    if update.title.is_none()
        && update.author.is_none()
        && update.description.is_none()
        && update.company.is_none()
        && update.project.is_none()
        && update.keywords.is_none()
    {
        return Err(argument("pass at least one of `title`, `author`, `description`, `company`, `project` or `keywords`"));
    }
    let title = match &update.title {
        Some(raw) => property_value(raw, "title")?,
        None => String::new(),
    };
    let author = match &update.author {
        Some(raw) => Some(property_value(raw, "author")?),
        None => None,
    };
    let description = match &update.description {
        Some(raw) => Some(property_value(raw, "description")?),
        None => None,
    };
    let company = match &update.company {
        Some(raw) => Some(property_value(raw, "company")?),
        None => None,
    };
    let project = match &update.project {
        Some(raw) => Some(property_value(raw, "project")?),
        None => None,
    };
    let keywords = match &update.keywords {
        Some(raw) => Some(property_value(raw, "keywords")?),
        None => None,
    };
    store::edit(handle, |drawing| {
        if update.title.is_some() {
            drawing.title = if title.is_empty() { None } else { Some(title.clone()) };
        }
        if let Some(value) = author {
            drawing.properties.author = value;
        }
        if let Some(value) = description {
            drawing.properties.description = value;
        }
        if let Some(value) = company {
            drawing.properties.company = value;
        }
        if let Some(value) = project {
            drawing.properties.project = value;
        }
        if let Some(value) = keywords {
            drawing.properties.keywords = value;
        }
        drawing.revision += 1;
        Ok(())
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "title": session.drawing.title,
            "properties": session.drawing.properties,
        })
    })
}

/// Options of [`export_with_options`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ExportOptions {
    /// Extrusion thickness for `stl` and `step`, in drawing units. `None` uses
    /// 1.0.
    pub thickness: Option<f64>,
}

/// Projects one drawing into an exchange format.
///
/// `svg`, `dxf`, `dxf-r12`, `pdf`, `stl`, `obj` and `step` are written
/// atomically and report any loss. The STL, OBJ and STEP writers extrude closed
/// profiles with the default thickness; use [`export_with_options`] to choose
/// another. An unknown format reports `format/unsupported`.
pub fn export(handle: u64, format: &str, path: &Path) -> Result<Value, ServiceError> {
    export_with_options(handle, format, path, ExportOptions::default())
}

/// Projects one drawing into an exchange format with explicit options.
///
/// The report carries `{ revision, format, target, path, bytes, losses,
/// editable }` plus `triangles` for STL and OBJ and `faces` for STEP.
pub fn export_with_options(handle: u64, format: &str, path: &Path, options: ExportOptions) -> Result<Value, ServiceError> {
    let format = format.trim().to_ascii_lowercase();
    store::persist(handle, |drawing| {
        let mut triangles: Option<usize> = None;
        let mut faces: Option<usize> = None;
        let (contents, losses) = match format.as_str() {
            "svg" => (exchange::svg(drawing), Vec::new()),
            "dxf" => exchange::dxf(drawing),
            "dxf-r12" => exchange::dxf_r12(drawing),
            "pdf" => exchange::pdf(drawing),
            "stl" => {
                let report = exchange::stl(drawing, options.thickness.unwrap_or(exchange::STL_DEFAULT_THICKNESS));
                triangles = Some(report.triangles);
                (report.content, report.losses)
            }
            "obj" => {
                let report = exchange::obj(drawing, options.thickness.unwrap_or(exchange::STL_DEFAULT_THICKNESS));
                triangles = Some(report.triangles);
                (report.content, report.losses)
            }
            "step" => {
                let report = exchange::step(drawing, options.thickness.unwrap_or(exchange::STL_DEFAULT_THICKNESS));
                faces = Some(report.faces);
                (report.content, report.losses)
            }
            other => {
                return Err(ServiceError::new(
                    "format/unsupported",
                    format!("`{other}` is not an export target of this build; use `svg`, `dxf`, `dxf-r12`, `pdf`, `stl`, `obj` or `step`"),
                ));
            }
        };
        let bytes = contents.len();
        write_atomic(path, contents.as_bytes())
            .map_err(|error| ServiceError::new("drawing/io-error", format!("could not write `{}`: {error}", path.display())))?;
        let mut report = json!({
            "revision": drawing.revision,
            "format": format,
            "target": path.display().to_string(),
            "path": path.display().to_string(),
            "bytes": bytes,
            "losses": losses,
            "editable": false,
        });
        if let Some(triangles) = triangles {
            report["triangles"] = json!(triangles);
        }
        if let Some(faces) = faces {
            report["faces"] = json!(faces);
        }
        Ok(report)
    })
}

/// Renders one open drawing for a read-only preview.
///
/// `svg` is produced in memory and returns `{ revision, format, svg, width,
/// height }`; the canvas size comes from the same bounds computation the SVG
/// writer uses. `png` and `pdf` previews are not implemented: raster and print
/// output belong to [`export`], which writes a file and reports its losses.
pub fn render(handle: u64, format: &str) -> Result<Value, ServiceError> {
    let format = format.trim().to_ascii_lowercase();
    if format != "svg" {
        return Err(not_implemented(&format!("preview render to {format}")));
    }
    store::read(handle, |session| {
        let (svg, width, height) = exchange::svg_preview(&session.drawing);
        json!({
            "revision": session.drawing.revision,
            "format": "svg",
            "svg": svg,
            "width": width,
            "height": height,
        })
    })
}

fn require_finite(points: &[[f64; 2]], field: &str) -> Result<(), ServiceError> {
    if points.iter().all(|point| finite_point(*point)) { Ok(()) } else { Err(argument(format!("`{field}` must contain finite coordinates"))) }
}

fn push_entity(handle: u64, layer: &str, kind: EntityKind) -> Result<Value, ServiceError> {
    let entity = store::edit(handle, |drawing| {
        if drawing.layer(layer).is_none() {
            return Err(ServiceError::new("object/not-found", format!("layer `{layer}` does not exist")));
        }
        if let Some(error) = locked_layer(drawing, layer) {
            return Err(error);
        }
        let id = drawing.take_entity_id();
        let entity = Entity { id, layer: layer.to_string(), kind };
        drawing.entities.push(entity.clone());
        drawing.revision += 1;
        Ok(entity)
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "entity": entity,
        })
    })
}

/// Adds a line to one layer.
pub fn add_line(handle: u64, layer: &str, from: [f64; 2], to: [f64; 2]) -> Result<Value, ServiceError> {
    require_finite(&[from, to], "from/to")?;
    push_entity(handle, layer, EntityKind::Line { from, to })
}

/// Adds a point entity to one layer.
pub fn add_point(handle: u64, layer: &str, at: [f64; 2]) -> Result<Value, ServiceError> {
    require_finite(&[at], "at")?;
    push_entity(handle, layer, EntityKind::Point { at })
}

/// Adds a circle to one layer.
pub fn add_circle(handle: u64, layer: &str, center: [f64; 2], radius: f64) -> Result<Value, ServiceError> {
    require_finite(&[center], "center")?;
    if !radius.is_finite() {
        return Err(argument("`radius` must be finite"));
    }
    push_entity(handle, layer, EntityKind::Circle { center, radius })
}

/// Adds a circular arc to one layer.
///
/// Angles are degrees and the arc always sweeps counter-clockwise from
/// `start_angle` to `end_angle`. A non-positive radius or a non-finite value
/// reports `session/argument-invalid`.
pub fn add_arc(handle: u64, layer: &str, center: [f64; 2], radius: f64, start_angle: f64, end_angle: f64) -> Result<Value, ServiceError> {
    require_finite(&[center], "center")?;
    if !radius.is_finite() || radius <= 0.0 {
        return Err(argument("`radius` must be a positive finite number"));
    }
    if !start_angle.is_finite() || !end_angle.is_finite() {
        return Err(argument("`startAngle`/`endAngle` must be finite"));
    }
    push_entity(handle, layer, EntityKind::Arc { center, radius, start_angle, end_angle })
}

/// Adds a full or partial ellipse to one layer.
///
/// `rx`/`ry` are the radii along the rotated local axes, `rotation` is the
/// major-axis angle in degrees, and `start_angle`/`end_angle` are parameters in
/// degrees measured from that axis. A non-positive radius reports
/// `session/argument-invalid`.
#[allow(clippy::too_many_arguments)]
pub fn add_ellipse(
    handle: u64,
    layer: &str,
    center: [f64; 2],
    rx: f64,
    ry: f64,
    rotation: f64,
    start_angle: f64,
    end_angle: f64,
) -> Result<Value, ServiceError> {
    require_finite(&[center], "center")?;
    if !rx.is_finite() || rx <= 0.0 || !ry.is_finite() || ry <= 0.0 {
        return Err(argument("`rx`/`ry` must be positive finite numbers"));
    }
    if !rotation.is_finite() || !start_angle.is_finite() || !end_angle.is_finite() {
        return Err(argument("`rotation`, `startAngle` and `endAngle` must be finite"));
    }
    push_entity(handle, layer, EntityKind::Ellipse { center, rx, ry, rotation, start_angle, end_angle })
}

/// Adds a polyline to one layer.
pub fn add_polyline(handle: u64, layer: &str, points: Vec<[f64; 2]>, closed: bool) -> Result<Value, ServiceError> {
    require_finite(&points, "points")?;
    push_entity(handle, layer, EntityKind::Polyline { points, closed })
}

/// Largest polygon side count [`add_polygon`] accepts.
const MAX_POLYGON_SIDES: u32 = 1024;

/// Adds an axis-aligned rectangle as one closed polyline.
///
/// `from` and `to` are opposite corners; both coordinates must be finite and
/// `to` must lie right of and above `from`, so the width and height are
/// positive. The four vertices run counter-clockwise and the shape is stored as
/// a closed polyline, so it shares polyline history, measurement and export
/// behaviour in a single revision and undo step. A non-positive width or height
/// reports `session/argument-invalid`.
pub fn add_rectangle(handle: u64, layer: &str, from: [f64; 2], to: [f64; 2]) -> Result<Value, ServiceError> {
    require_finite(&[from, to], "from/to")?;
    if to[0] <= from[0] || to[1] <= from[1] {
        return Err(argument("`to` must be to the right of and above `from` so the width and height are positive"));
    }
    let points = vec![[from[0], from[1]], [to[0], from[1]], [to[0], to[1]], [from[0], to[1]]];
    add_polyline(handle, layer, points, true)
}

/// Adds a regular polygon as one closed polyline.
///
/// `sides` counts the vertices and must be an integer from 3 to 1024.
/// `rotation` is the angle of the first vertex in degrees; the remaining
/// vertices follow counter-clockwise at equal angles and the shape closes back
/// to the first one. The result is one closed polyline, so it shares polyline
/// history, measurement and export behaviour in a single revision and undo
/// step. A side count outside 3-1024, a non-positive radius or a non-finite
/// value reports `session/argument-invalid`.
pub fn add_polygon(handle: u64, layer: &str, center: [f64; 2], radius: f64, sides: u32, rotation: f64) -> Result<Value, ServiceError> {
    require_finite(&[center], "center")?;
    if !radius.is_finite() || radius <= 0.0 {
        return Err(argument("`radius` must be a positive finite number"));
    }
    if !(3..=MAX_POLYGON_SIDES).contains(&sides) {
        return Err(argument(format!("`sides` must be an integer from 3 to {MAX_POLYGON_SIDES}")));
    }
    if !rotation.is_finite() {
        return Err(argument("`rotation` must be finite"));
    }
    let step = std::f64::consts::TAU / f64::from(sides);
    let start = rotation.to_radians();
    let points = (0..sides)
        .map(|index| {
            let (sin, cos) = (start + step * f64::from(index)).sin_cos();
            [center[0] + radius * cos, center[1] + radius * sin]
        })
        .collect();
    add_polyline(handle, layer, points, true)
}

/// Adds single-line text to one layer.
pub fn add_text(handle: u64, layer: &str, at: [f64; 2], height: f64, content: &str) -> Result<Value, ServiceError> {
    require_finite(&[at], "at")?;
    if !height.is_finite() {
        return Err(argument("`height` must be finite"));
    }
    push_entity(handle, layer, EntityKind::Text { at, height, content: content.to_string() })
}

/// Adds a linear dimension to one layer.
pub fn add_dimension(handle: u64, layer: &str, from: [f64; 2], to: [f64; 2], offset: f64, text: Option<String>) -> Result<Value, ServiceError> {
    require_finite(&[from, to], "from/to")?;
    if !offset.is_finite() {
        return Err(argument("`offset` must be finite"));
    }
    push_entity(handle, layer, EntityKind::Dimension { from, to, offset, text })
}

/// Adds a radial or diameter dimension to one layer.
///
/// `angle` locates the measured point on the circle in degrees from the
/// positive X axis, and `offset` is the signed distance from that point to the
/// label along the leader. `diameter` switches the measured value from the
/// radius to the diameter. A non-positive radius reports
/// `session/argument-invalid`.
#[allow(clippy::too_many_arguments)]
pub fn add_radial_dimension(
    handle: u64,
    layer: &str,
    center: [f64; 2],
    radius: f64,
    angle: f64,
    offset: f64,
    diameter: bool,
    text: Option<String>,
) -> Result<Value, ServiceError> {
    require_finite(&[center], "center")?;
    if !radius.is_finite() || radius <= 0.0 {
        return Err(argument("`radius` must be a positive finite number"));
    }
    if !angle.is_finite() || !offset.is_finite() {
        return Err(argument("`angle`/`offset` must be finite"));
    }
    push_entity(handle, layer, EntityKind::RadialDimension { center, radius, angle, offset, diameter, text })
}

/// Adds an ordinate dimension to one layer.
///
/// The dimension measures the `axis`-aligned distance from `origin` to
/// `feature`; `offset` is the signed perpendicular distance of the dimension
/// line from the origin.
pub fn add_ordinate_dimension(
    handle: u64,
    layer: &str,
    origin: [f64; 2],
    feature: [f64; 2],
    axis: OrdinateAxis,
    offset: f64,
    text: Option<String>,
) -> Result<Value, ServiceError> {
    require_finite(&[origin, feature], "origin/feature")?;
    if !offset.is_finite() {
        return Err(argument("`offset` must be finite"));
    }
    push_entity(handle, layer, EntityKind::OrdinateDimension { origin, feature, axis, offset, text })
}

/// Adds an angular dimension between two rays that share `center`.
///
/// The angle is swept counter-clockwise from the `first` ray to the `second` ray
/// and is reported in degrees. `offset` is the positive radius of the dimension
/// arc; the rays must be distinct and non-collinear.
pub fn add_angular_dimension(
    handle: u64,
    layer: &str,
    center: [f64; 2],
    first: [f64; 2],
    second: [f64; 2],
    offset: f64,
    text: Option<String>,
) -> Result<Value, ServiceError> {
    require_finite(&[center, first, second], "center/first/second")?;
    if !offset.is_finite() || offset <= 0.0 {
        return Err(argument("`offset` must be a positive finite number"));
    }
    let (_, sweep) = angular_sweep(center, first, second);
    if sweep <= 1e-9 {
        return Err(argument("the two rays of an angular dimension must not be collinear"));
    }
    push_entity(handle, layer, EntityKind::AngularDimension { center, first, second, offset, text })
}

/// Adds a filled hatch bounded by one closed polygon.
///
/// The boundary needs at least three distinct vertices; the closing edge is
/// implicit. `pattern` is the pattern name (`SOLID` for a solid fill) and is
/// kept as a description, because no pattern definition is modelled.
pub fn add_hatch(handle: u64, layer: &str, points: Vec<[f64; 2]>, pattern: Option<String>) -> Result<Value, ServiceError> {
    require_finite(&points, "points")?;
    if points.len() < 3 {
        return Err(argument("a hatch needs at least 3 boundary points"));
    }
    let pattern = pattern.map(|value| value.trim().to_string()).filter(|value| !value.is_empty()).unwrap_or_else(default_hatch_pattern);
    push_entity(handle, layer, EntityKind::Hatch { points, pattern })
}

/// Deletes one entity by id.
pub fn delete_entity(handle: u64, entity: &str) -> Result<Value, ServiceError> {
    store::edit(handle, |drawing| {
        let index = drawing
            .entities
            .iter()
            .position(|candidate| candidate.id == entity)
            .ok_or_else(|| ServiceError::new("object/not-found", format!("entity `{entity}` does not exist")))?;
        let layer = drawing.entities[index].layer.clone();
        if let Some(error) = locked_layer(drawing, &layer) {
            return Err(error);
        }
        drawing.entities.remove(index);
        drawing.forget_draw_order(entity);
        detach_entity_from_groups(drawing, entity);
        drawing.revision += 1;
        Ok(())
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "deleted": entity,
        })
    })
}

/// Copies one entity under the next deterministic id and returns it.
///
/// The copy keeps the source layer and geometry. An unknown id reports
/// `object/not-found`; a locked source layer refuses with
/// `drawing/layer-locked`, matching single-entity creation.
pub fn duplicate_entity(handle: u64, entity: &str) -> Result<Value, ServiceError> {
    let copy = store::edit(handle, |drawing| {
        let index = entity_index(drawing, entity)?;
        let layer = drawing.entities[index].layer.clone();
        if let Some(error) = locked_layer(drawing, &layer) {
            return Err(error);
        }
        let kind = drawing.entities[index].kind.clone();
        let id = drawing.take_entity_id();
        let copy = Entity { id, layer, kind };
        drawing.entities.push(copy.clone());
        drawing.revision += 1;
        Ok(copy)
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "entity": copy,
        })
    })
}

fn locked_layer(drawing: &Drawing, layer: &str) -> Option<ServiceError> {
    match drawing.layer(layer) {
        Some(record) if record.locked => Some(ServiceError::new("drawing/layer-locked", format!("layer `{layer}` is locked"))),
        Some(record) if record.frozen => Some(ServiceError::new("drawing/layer-frozen", format!("layer `{layer}` is frozen"))),
        _ => None,
    }
}

fn layer_json(drawing: &Drawing, id: &str) -> Option<Value> {
    drawing.layer(id).map(|layer| {
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
            "entityCount": drawing.entities.iter().filter(|entity| entity.layer == layer.id).count(),
        })
    })
}

fn entity_index(drawing: &Drawing, entity: &str) -> Result<usize, ServiceError> {
    drawing
        .entities
        .iter()
        .position(|candidate| candidate.id == entity)
        .ok_or_else(|| ServiceError::new("object/not-found", format!("entity `{entity}` does not exist")))
}

/// Moves one entity by an offset or to an absolute anchor.
///
/// `offset` and `to` are mutually exclusive; `to` is applied to the anchor the
/// entity reports (a line's start, a circle's centre, a polyline's first
/// vertex, text's baseline or a dimension's first measured point).
pub fn move_entity(handle: u64, entity: &str, offset: Option<[f64; 2]>, to: Option<[f64; 2]>) -> Result<Value, ServiceError> {
    if offset.is_some() == to.is_some() {
        return Err(argument("pass either an offset or an absolute target, not both"));
    }
    if let Some(offset) = offset {
        require_finite(&[offset], "offset")?;
    }
    if let Some(to) = to {
        require_finite(&[to], "to")?;
    }
    store::edit(handle, |drawing| {
        let index = entity_index(drawing, entity)?;
        let layer = drawing.entities[index].layer.clone();
        if let Some(error) = locked_layer(drawing, &layer) {
            return Err(error);
        }
        let delta = match to {
            Some(target) => {
                let anchor = drawing.entities[index].anchor();
                [target[0] - anchor[0], target[1] - anchor[1]]
            }
            None => offset.unwrap_or([0.0, 0.0]),
        };
        drawing.entities[index].translate(delta);
        drawing.revision += 1;
        Ok(())
    })?;
    store::read(handle, |session| {
        let entity = session.drawing.entities.iter().find(|candidate| candidate.id == entity).cloned();
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "entity": entity,
        })
    })
}

/// Updates the editable payload of one entity.
///
/// `content` and `height` only apply to text; `radius` only applies to a circle
/// or an arc. A field that does not fit the entity reports
/// `object/unsupported`.
pub fn update_entity(
    handle: u64,
    entity: &str,
    content: Option<String>,
    height: Option<f64>,
    radius: Option<f64>,
) -> Result<Value, ServiceError> {
    if content.is_none() && height.is_none() && radius.is_none() {
        return Err(argument("pass at least one of `content`, `height` or `radius`"));
    }
    if let Some(height) = height
        && !height.is_finite()
    {
        return Err(argument("`height` must be finite"));
    }
    if let Some(radius) = radius
        && !radius.is_finite()
    {
        return Err(argument("`radius` must be finite"));
    }
    store::edit(handle, |drawing| {
        let index = entity_index(drawing, entity)?;
        let layer = drawing.entities[index].layer.clone();
        if let Some(error) = locked_layer(drawing, &layer) {
            return Err(error);
        }
        let unsupported = || ServiceError::new("object/unsupported", format!("entity `{entity}` does not carry that field"));
        match &mut drawing.entities[index].kind {
            EntityKind::Text { content: current, height: current_height, .. } => {
                if radius.is_some() {
                    return Err(unsupported());
                }
                if let Some(value) = content {
                    *current = value;
                }
                if let Some(value) = height {
                    *current_height = value;
                }
            }
            EntityKind::Circle { radius: current, .. } => {
                if content.is_some() || height.is_some() {
                    return Err(unsupported());
                }
                if let Some(value) = radius {
                    *current = value.abs();
                }
            }
            EntityKind::Arc { radius: current, .. } => {
                if content.is_some() || height.is_some() {
                    return Err(unsupported());
                }
                if let Some(value) = radius
                    && value > 0.0
                {
                    *current = value;
                }
            }
            _ => return Err(unsupported()),
        }
        drawing.revision += 1;
        Ok(())
    })?;
    store::read(handle, |session| {
        let entity = session.drawing.entities.iter().find(|candidate| candidate.id == entity).cloned();
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "entity": entity,
        })
    })
}

/// One edit applied by [`update_polyline`].
///
/// A caller either replaces the vertex list or edits one vertex; the closed flag
/// can accompany any edit. `insert_at` accepts the current vertex count to
/// append.
#[derive(Debug, Clone, Default)]
pub struct PolylineUpdate {
    /// Replacement vertices; omit to keep the current list.
    pub points: Option<Vec<[f64; 2]>>,
    /// New closed flag; omit to keep the current flag.
    pub closed: Option<bool>,
    /// Zero-based index the inserted vertex takes.
    pub insert_at: Option<usize>,
    /// New vertex, required together with `insert_at`.
    pub insert_point: Option<[f64; 2]>,
    /// Zero-based index of the vertex to remove.
    pub remove_at: Option<usize>,
}

/// Edits the vertices or the closed flag of one polyline.
///
/// The caller passes a replacement `points` list, an `insert_at`/`insert_point`
/// pair, a `remove_at` index or a new `closed` flag. Replacing the list cannot
/// be combined with a single-vertex edit. A polyline always keeps at least two
/// vertices, and an index outside the current list reports
/// `session/argument-invalid`. A non-polyline reports `object/unsupported` and
/// a locked or frozen layer reports `drawing/layer-locked` or
/// `drawing/layer-frozen`. The response carries `{ revision, drawing, entity }`.
pub fn update_polyline(handle: u64, entity: &str, update: PolylineUpdate) -> Result<Value, ServiceError> {
    let replacing = update.points.is_some();
    let inserting = update.insert_at.is_some();
    let removing = update.remove_at.is_some();
    if !replacing && update.closed.is_none() && !inserting && !removing {
        return Err(argument("pass at least one of `points`, `closed`, `insertAt` or `removeAt`"));
    }
    if replacing && (inserting || removing) {
        return Err(argument("replace the vertex list or edit one vertex, not both"));
    }
    if inserting != update.insert_point.is_some() {
        return Err(argument("`insertAt` and `insertPoint` must be passed together"));
    }
    if let Some(points) = &update.points {
        require_finite(points, "points")?;
        if points.len() < 2 {
            return Err(argument("a polyline needs at least 2 vertices"));
        }
    }
    if let Some(point) = update.insert_point {
        require_finite(&[point], "insertPoint")?;
    }
    store::edit(handle, |drawing| {
        let index = entity_index(drawing, entity)?;
        let layer = drawing.entities[index].layer.clone();
        if let Some(error) = locked_layer(drawing, &layer) {
            return Err(error);
        }
        if !matches!(&drawing.entities[index].kind, EntityKind::Polyline { .. }) {
            return Err(ServiceError::new(
                "object/unsupported",
                format!("entity `{entity}` is a {}, not a polyline", kind_name(&drawing.entities[index].kind)),
            ));
        }
        let EntityKind::Polyline { points, closed } = &mut drawing.entities[index].kind
        else {
            unreachable!("the entity kind was checked above");
        };
        if let Some(replacement) = update.points.clone() {
            *points = replacement;
        }
        if let Some(value) = update.closed {
            *closed = value;
        }
        if let Some(at) = update.insert_at {
            if at > points.len() {
                return Err(argument(format!("`insertAt` {at} is beyond the polyline's {} vertices", points.len())));
            }
            points.insert(at, update.insert_point.expect("the insert point is validated above"));
        }
        if let Some(at) = update.remove_at {
            if at >= points.len() {
                return Err(argument(format!("`removeAt` {at} is outside the polyline's {} vertices", points.len())));
            }
            if points.len() <= 2 {
                return Err(argument("a polyline needs at least 2 vertices"));
            }
            points.remove(at);
        }
        drawing.revision += 1;
        Ok(())
    })?;
    store::read(handle, |session| {
        let entity = session.drawing.entities.iter().find(|candidate| candidate.id == entity).cloned();
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "entity": entity,
        })
    })
}

/// Updates the radius or sweep angles of one arc.
///
/// At least one of `radius`, `start_angle` or `end_angle` is required. Angles
/// are degrees and the arc keeps sweeping counter-clockwise from the start to
/// the end angle; a start equal to the end still denotes a full turn. A
/// non-positive radius or a non-finite value reports
/// `session/argument-invalid`, a non-arc reports `object/unsupported` and a
/// locked or frozen layer reports `drawing/layer-locked` or
/// `drawing/layer-frozen`. The response carries `{ revision, drawing, entity }`.
pub fn update_arc(
    handle: u64,
    entity: &str,
    radius: Option<f64>,
    start_angle: Option<f64>,
    end_angle: Option<f64>,
) -> Result<Value, ServiceError> {
    if radius.is_none() && start_angle.is_none() && end_angle.is_none() {
        return Err(argument("pass at least one of `radius`, `startAngle` or `endAngle`"));
    }
    if let Some(radius) = radius
        && (!radius.is_finite() || radius <= 0.0)
    {
        return Err(argument("`radius` must be a positive finite number"));
    }
    for (name, value) in [("startAngle", start_angle), ("endAngle", end_angle)] {
        if let Some(angle) = value
            && !angle.is_finite()
        {
            return Err(argument(format!("`{name}` must be finite")));
        }
    }
    store::edit(handle, |drawing| {
        let index = entity_index(drawing, entity)?;
        let layer = drawing.entities[index].layer.clone();
        if let Some(error) = locked_layer(drawing, &layer) {
            return Err(error);
        }
        if !matches!(&drawing.entities[index].kind, EntityKind::Arc { .. }) {
            return Err(ServiceError::new(
                "object/unsupported",
                format!("entity `{entity}` is a {}, not an arc", kind_name(&drawing.entities[index].kind)),
            ));
        }
        let EntityKind::Arc { radius: current_radius, start_angle: current_start, end_angle: current_end, .. } =
            &mut drawing.entities[index].kind
        else {
            unreachable!("the entity kind was checked above");
        };
        if let Some(value) = radius {
            *current_radius = value;
        }
        if let Some(value) = start_angle {
            *current_start = value;
        }
        if let Some(value) = end_angle {
            *current_end = value;
        }
        drawing.revision += 1;
        Ok(())
    })?;
    store::read(handle, |session| {
        let entity = session.drawing.entities.iter().find(|candidate| candidate.id == entity).cloned();
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "entity": entity,
        })
    })
}

/// Moves one entity onto another layer without changing its geometry.
///
/// The target layer must exist and neither the current nor the target layer may
/// be locked or frozen. A move onto the current layer is a no-op that keeps the
/// revision. The response carries `{ revision, drawing, entity, from, to,
/// moved }`.
pub fn assign_entity_layer(handle: u64, entity: &str, layer: &str) -> Result<Value, ServiceError> {
    if layer.trim().is_empty() {
        return Err(argument("a target layer id is required"));
    }
    let (from, moved) = store::edit(handle, |drawing| {
        let index = entity_index(drawing, entity)?;
        if drawing.layer(layer).is_none() {
            return Err(ServiceError::new("object/not-found", format!("layer `{layer}` does not exist")));
        }
        let from = drawing.entities[index].layer.clone();
        if let Some(error) = locked_layer(drawing, &from) {
            return Err(error);
        }
        if let Some(error) = locked_layer(drawing, layer) {
            return Err(error);
        }
        let moved = from != layer;
        if moved {
            drawing.entities[index].layer = layer.to_string();
            drawing.revision += 1;
        }
        Ok((from, moved))
    })?;
    store::read(handle, |session| {
        let entity = session.drawing.entities.iter().find(|candidate| candidate.id == entity).cloned();
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "entity": entity,
            "from": from,
            "to": layer,
            "moved": moved,
        })
    })
}

/// Options of [`find_text`] and [`replace_text`].
#[derive(Debug, Clone, Default)]
pub struct TextSearchOptions {
    /// Match case exactly; `false` matches ASCII letters case-insensitively.
    pub match_case: bool,
    /// Restrict the search to one layer id.
    pub layer: Option<String>,
}

fn text_selected(layer: &str, options: &TextSearchOptions) -> bool {
    options.layer.as_deref().is_none_or(|wanted| wanted == layer)
}

fn text_matches(content: &str, pattern: &str, match_case: bool) -> bool {
    if match_case { content.contains(pattern) } else { content.to_ascii_lowercase().contains(&pattern.to_ascii_lowercase()) }
}

fn text_occurrences(content: &str, pattern: &str, match_case: bool) -> usize {
    if match_case { content.matches(pattern).count() } else { content.to_ascii_lowercase().matches(&pattern.to_ascii_lowercase()).count() }
}

/// Replaces every occurrence of `pattern` in `content`.
///
/// The case-insensitive path lowercases ASCII letters only, so byte offsets stay
/// aligned with the original string and the replacement keeps the original case
/// of every character it does not replace.
fn text_replace(content: &str, pattern: &str, replacement: &str, match_case: bool) -> String {
    if match_case {
        return content.replace(pattern, replacement);
    }
    let haystack = content.to_ascii_lowercase();
    let needle = pattern.to_ascii_lowercase();
    let mut result = String::with_capacity(content.len());
    let mut cursor = 0;
    while let Some(offset) = haystack[cursor..].find(&needle) {
        let start = cursor + offset;
        result.push_str(&content[cursor..start]);
        result.push_str(replacement);
        cursor = start + needle.len();
    }
    result.push_str(&content[cursor..]);
    result
}

/// Finds text entities whose content contains `pattern`.
///
/// Matching is ASCII case-insensitive unless `options.match_case` is set, and
/// `options.layer` narrows the search to one layer. The response carries
/// `{ revision, pattern, matchCase, layer, count, matches }`, where every match
/// is `{ id, layer, content, at }`. The query never mutates the drawing.
pub fn find_text(handle: u64, pattern: &str, options: TextSearchOptions) -> Result<Value, ServiceError> {
    if pattern.is_empty() {
        return Err(argument("a text search needs a non-empty pattern"));
    }
    store::read(handle, |session| {
        let matches = session
            .drawing
            .entities
            .iter()
            .filter(|entity| text_selected(&entity.layer, &options))
            .filter_map(|entity| match &entity.kind {
                EntityKind::Text { at, content, .. } if text_matches(content, pattern, options.match_case) => {
                    Some(json!({ "id": entity.id, "layer": entity.layer, "content": content, "at": at }))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        json!({
            "revision": session.drawing.revision,
            "pattern": pattern,
            "matchCase": options.match_case,
            "layer": options.layer,
            "count": matches.len(),
            "matches": matches,
        })
    })
}

/// Replaces `pattern` in every matching text entity and returns one history step.
///
/// Matching uses the same rule as [`find_text`]. The response carries
/// `{ revision, drawing, pattern, replacement, matchCase, count, occurrences }`;
/// `count` is the number of changed entities and `occurrences` the number of
/// replaced substrings. A match on a locked layer refuses the whole edit with
/// `drawing/layer-locked`; when nothing matches the revision does not change.
pub fn replace_text(handle: u64, pattern: &str, replacement: &str, options: TextSearchOptions) -> Result<Value, ServiceError> {
    if pattern.is_empty() {
        return Err(argument("a text search needs a non-empty pattern"));
    }
    let pending = store::read(handle, |session| {
        session.drawing.entities.iter().any(|entity| {
            text_selected(&entity.layer, &options)
                && matches!(&entity.kind, EntityKind::Text { content, .. } if text_matches(content, pattern, options.match_case))
        })
    })?;
    let (count, occurrences) = if pending {
        store::edit(handle, |drawing| {
            for entity in &drawing.entities {
                if !text_selected(&entity.layer, &options) {
                    continue;
                }
                if let EntityKind::Text { content, .. } = &entity.kind
                    && text_matches(content, pattern, options.match_case)
                    && let Some(error) = locked_layer(drawing, &entity.layer)
                {
                    return Err(error);
                }
            }
            let mut count = 0usize;
            let mut occurrences = 0usize;
            for entity in &mut drawing.entities {
                if !text_selected(&entity.layer, &options) {
                    continue;
                }
                if let EntityKind::Text { content, .. } = &mut entity.kind
                    && text_matches(content, pattern, options.match_case)
                {
                    occurrences += text_occurrences(content, pattern, options.match_case);
                    *content = text_replace(content, pattern, replacement, options.match_case);
                    count += 1;
                }
            }
            if count > 0 {
                drawing.revision += 1;
            }
            Ok((count, occurrences))
        })?
    }
    else {
        (0, 0)
    };
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "pattern": pattern,
            "replacement": replacement,
            "matchCase": options.match_case,
            "count": count,
            "occurrences": occurrences,
        })
    })
}

/// Axis of a mirror transform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirrorAxis {
    /// Reflect across the horizontal line `y = at` (flips Y).
    X,
    /// Reflect across the vertical line `x = at` (flips X).
    Y,
}

/// Selection a transform applies to.
#[derive(Debug, Clone)]
pub enum TransformTarget {
    /// Explicit entity ids; repeated ids are applied once.
    Ids(Vec<String>),
    /// Every entity on one layer, in document order.
    Layer(String),
    /// Every entity in the drawing.
    All,
}

/// One affine operation applied by [`transform`].
#[derive(Debug, Clone, Copy)]
pub enum TransformOp {
    /// Offset every selected entity.
    Translate {
        /// X offset.
        dx: f64,
        /// Y offset.
        dy: f64,
    },
    /// Rotate around a pivot, counter-clockwise.
    Rotate {
        /// Rotation in degrees.
        angle: f64,
        /// Pivot X.
        cx: f64,
        /// Pivot Y.
        cy: f64,
    },
    /// Uniformly scale around a pivot.
    Scale {
        /// Positive scale factor.
        factor: f64,
        /// Pivot X.
        cx: f64,
        /// Pivot Y.
        cy: f64,
    },
    /// Reflect across a horizontal or vertical line.
    Mirror {
        /// Axis to reflect across.
        axis: MirrorAxis,
        /// Coordinate of the reflection line on the mirrored axis.
        at: f64,
    },
}

/// Arguments of [`transform`].
#[derive(Debug, Clone)]
pub struct TransformRequest {
    /// Sheets are not modelled; a non-empty value is rejected.
    pub sheet: Option<String>,
    /// Entities the operation applies to.
    pub target: TransformTarget,
    /// Affine operation to apply.
    pub operation: TransformOp,
}

/// Arguments of [`copy`].
#[derive(Debug, Clone)]
pub struct CopyRequest {
    /// Source entity ids; repeated ids are copied once.
    pub ids: Vec<String>,
    /// X offset of each successive copy.
    pub dx: f64,
    /// Y offset of each successive copy.
    pub dy: f64,
    /// Number of copies to create, between 1 and 10 000.
    pub count: u32,
}

fn validate_transform(operation: &TransformOp) -> Result<(), ServiceError> {
    match *operation {
        TransformOp::Translate { dx, dy } => {
            if !dx.is_finite() || !dy.is_finite() {
                return Err(argument("`dx`/`dy` must be finite"));
            }
        }
        TransformOp::Rotate { angle, cx, cy } => {
            if !angle.is_finite() || !cx.is_finite() || !cy.is_finite() {
                return Err(argument("`angle`, `cx` and `cy` must be finite"));
            }
        }
        TransformOp::Scale { factor, cx, cy } => {
            if !factor.is_finite() || factor <= 0.0 {
                return Err(argument("`factor` must be a positive finite number"));
            }
            if !cx.is_finite() || !cy.is_finite() {
                return Err(argument("`cx` and `cy` must be finite"));
            }
        }
        TransformOp::Mirror { at, .. } => {
            if !at.is_finite() {
                return Err(argument("`at` must be finite"));
            }
        }
    }
    Ok(())
}

fn rotate_about(point: [f64; 2], angle: f64, pivot: [f64; 2]) -> [f64; 2] {
    let (sin, cos) = angle.to_radians().sin_cos();
    let x = point[0] - pivot[0];
    let y = point[1] - pivot[1];
    [pivot[0] + x * cos - y * sin, pivot[1] + x * sin + y * cos]
}

fn scale_about(point: [f64; 2], factor: f64, pivot: [f64; 2]) -> [f64; 2] {
    [pivot[0] + (point[0] - pivot[0]) * factor, pivot[1] + (point[1] - pivot[1]) * factor]
}

fn mirror_point(point: [f64; 2], axis: MirrorAxis, at: f64) -> [f64; 2] {
    match axis {
        MirrorAxis::X => [point[0], 2.0 * at - point[1]],
        MirrorAxis::Y => [2.0 * at - point[0], point[1]],
    }
}

/// Applies one affine operation to a single entity in place.
fn apply_transform(entity: &mut Entity, operation: TransformOp) {
    match operation {
        TransformOp::Translate { dx, dy } => entity.translate([dx, dy]),
        TransformOp::Rotate { angle, cx, cy } => {
            let pivot = [cx, cy];
            let rotate = |point: &mut [f64; 2]| *point = rotate_about(*point, angle, pivot);
            match &mut entity.kind {
                EntityKind::Line { from, to } => {
                    rotate(from);
                    rotate(to);
                }
                EntityKind::Circle { center, .. } => rotate(center),
                EntityKind::Arc { center, start_angle, end_angle, .. } => {
                    rotate(center);
                    *start_angle = normalize_angle(*start_angle + angle);
                    *end_angle = normalize_angle(*end_angle + angle);
                }
                EntityKind::Ellipse { center, rotation, start_angle, end_angle, .. } => {
                    rotate(center);
                    *rotation = normalize_angle(*rotation + angle);
                    *start_angle = normalize_angle(*start_angle + angle);
                    *end_angle = normalize_angle(*end_angle + angle);
                }
                EntityKind::Point { at } => rotate(at),
                EntityKind::Polyline { points, .. } => points.iter_mut().for_each(rotate),
                EntityKind::Text { at, .. } => rotate(at),
                EntityKind::Dimension { from, to, .. } => {
                    rotate(from);
                    rotate(to);
                }
                EntityKind::RadialDimension { center, angle: current, .. } => {
                    rotate(center);
                    *current = normalize_angle(*current + angle);
                }
                EntityKind::OrdinateDimension { origin, feature, .. } => {
                    rotate(origin);
                    rotate(feature);
                }
                EntityKind::AngularDimension { center, first, second, .. } => {
                    rotate(center);
                    rotate(first);
                    rotate(second);
                }
                EntityKind::Hatch { points, .. } => points.iter_mut().for_each(rotate),
            }
        }
        TransformOp::Scale { factor, cx, cy } => {
            let pivot = [cx, cy];
            let scale = |point: &mut [f64; 2]| *point = scale_about(*point, factor, pivot);
            match &mut entity.kind {
                EntityKind::Line { from, to } => {
                    scale(from);
                    scale(to);
                }
                EntityKind::Circle { center, radius } => {
                    scale(center);
                    *radius *= factor;
                }
                EntityKind::Arc { center, radius, .. } => {
                    scale(center);
                    *radius *= factor;
                }
                EntityKind::Ellipse { center, rx, ry, .. } => {
                    scale(center);
                    *rx *= factor;
                    *ry *= factor;
                }
                EntityKind::Point { at } => scale(at),
                EntityKind::Polyline { points, .. } => points.iter_mut().for_each(scale),
                EntityKind::Text { at, height, .. } => {
                    scale(at);
                    *height *= factor;
                }
                EntityKind::Dimension { from, to, offset, .. } => {
                    scale(from);
                    scale(to);
                    *offset *= factor;
                }
                EntityKind::RadialDimension { center, radius, offset, .. } => {
                    scale(center);
                    *radius *= factor;
                    *offset *= factor;
                }
                EntityKind::OrdinateDimension { origin, feature, offset, .. } => {
                    scale(origin);
                    scale(feature);
                    *offset *= factor;
                }
                EntityKind::AngularDimension { center, first, second, offset, .. } => {
                    scale(center);
                    scale(first);
                    scale(second);
                    *offset *= factor;
                }
                EntityKind::Hatch { points, .. } => points.iter_mut().for_each(scale),
            }
        }
        TransformOp::Mirror { axis, at } => {
            let mirror = |point: &mut [f64; 2]| *point = mirror_point(*point, axis, at);
            match &mut entity.kind {
                EntityKind::Line { from, to } => {
                    mirror(from);
                    mirror(to);
                }
                EntityKind::Circle { center, .. } => mirror(center),
                EntityKind::Arc { center, start_angle, end_angle, .. } => {
                    mirror(center);
                    let (old_start, old_end) = (*start_angle, *end_angle);
                    let (start, end) = match axis {
                        MirrorAxis::X => (-old_end, -old_start),
                        MirrorAxis::Y => (180.0 - old_end, 180.0 - old_start),
                    };
                    *start_angle = normalize_angle(start);
                    *end_angle = normalize_angle(end);
                }
                EntityKind::Ellipse { center, rotation, start_angle, end_angle, .. } => {
                    mirror(center);
                    let (old_start, old_end) = (*start_angle, *end_angle);
                    let (start, end, rotated) = match axis {
                        MirrorAxis::X => (-old_end, -old_start, -*rotation),
                        MirrorAxis::Y => (180.0 - old_end, 180.0 - old_start, 180.0 - *rotation),
                    };
                    *rotation = normalize_angle(rotated);
                    *start_angle = normalize_angle(start);
                    *end_angle = normalize_angle(end);
                }
                EntityKind::Point { at } => mirror(at),
                EntityKind::Polyline { points, .. } => points.iter_mut().for_each(mirror),
                EntityKind::Text { at, .. } => mirror(at),
                EntityKind::Dimension { from, to, offset, .. } => {
                    mirror(from);
                    mirror(to);
                    *offset = -*offset;
                }
                EntityKind::RadialDimension { center, angle: current, .. } => {
                    mirror(center);
                    let reflected = match axis {
                        MirrorAxis::X => -*current,
                        MirrorAxis::Y => 180.0 - *current,
                    };
                    *current = normalize_angle(reflected);
                }
                EntityKind::OrdinateDimension { origin, feature, axis: measured, offset, .. } => {
                    mirror(origin);
                    mirror(feature);
                    let flips = matches!((*measured, axis), (OrdinateAxis::X, MirrorAxis::X) | (OrdinateAxis::Y, MirrorAxis::Y));
                    if flips {
                        *offset = -*offset;
                    }
                }
                EntityKind::AngularDimension { center, first, second, .. } => {
                    mirror(center);
                    mirror(first);
                    mirror(second);
                }
                EntityKind::Hatch { points, .. } => points.iter_mut().for_each(mirror),
            }
        }
    }
}

fn require_sheet_absent(sheet: Option<&str>) -> Result<(), ServiceError> {
    match sheet.map(str::trim) {
        Some(name) if !name.is_empty() => Err(argument("sheets are not modelled; omit `sheet`")),
        _ => Ok(()),
    }
}

/// Applies one affine transform to a selection and returns the new bounds.
///
/// The selection is `{ ids: [...] }`, `{ layer: "..." }` or `{ all: true }`;
/// the operation is `translate`, `rotate`, `scale` or `mirror`. Rotating an arc
/// adds the angle to both stored angles, scaling multiplies radii and pivot
/// offsets, and mirroring an arc swaps and reflects its angles so the result
/// still sweeps counter-clockwise. Locked layers refuse with
/// `drawing/layer-locked`. The response carries `{ revision, drawing,
/// affected, bounds }`.
pub fn transform(handle: u64, request: TransformRequest) -> Result<Value, ServiceError> {
    require_sheet_absent(request.sheet.as_deref())?;
    validate_transform(&request.operation)?;
    let affected = store::edit(handle, |drawing| {
        let mut indices = match &request.target {
            TransformTarget::Ids(ids) => {
                if ids.is_empty() {
                    return Err(argument("`target.ids` is empty"));
                }
                let mut indices = Vec::with_capacity(ids.len());
                for id in ids {
                    indices.push(entity_index(drawing, id)?);
                }
                indices
            }
            TransformTarget::Layer(layer) => {
                if drawing.layer(layer).is_none() {
                    return Err(ServiceError::new("object/not-found", format!("layer `{layer}` does not exist")));
                }
                drawing.entities.iter().enumerate().filter(|(_, entity)| entity.layer == *layer).map(|(index, _)| index).collect()
            }
            TransformTarget::All => (0..drawing.entities.len()).collect(),
        };
        indices.sort_unstable();
        indices.dedup();
        for &index in &indices {
            if let Some(error) = locked_layer(drawing, &drawing.entities[index].layer) {
                return Err(error);
            }
        }
        for &index in &indices {
            apply_transform(&mut drawing.entities[index], request.operation);
        }
        if !indices.is_empty() {
            drawing.revision += 1;
        }
        Ok(indices.len())
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "affected": affected,
            "bounds": bounds_json(&session.drawing),
        })
    })
}

/// Duplicates entities `count` times with a cumulative offset.
///
/// Each copy keeps the source layer and geometry, receives a fresh id and is
/// offset by `(dx * n, dy * n)` for copy number `n`. `count` must be between 1
/// and 10 000. Locked source layers refuse with `drawing/layer-locked`. The
/// response carries `{ revision, drawing, created, count }`.
pub fn copy(handle: u64, request: CopyRequest) -> Result<Value, ServiceError> {
    if request.ids.is_empty() {
        return Err(argument("`ids` is empty"));
    }
    if request.count == 0 || request.count > 10_000 {
        return Err(argument("`count` must be between 1 and 10000"));
    }
    if !request.dx.is_finite() || !request.dy.is_finite() {
        return Err(argument("`dx`/`dy` must be finite"));
    }
    let created = store::edit(handle, |drawing| {
        let mut indices = Vec::with_capacity(request.ids.len());
        for id in &request.ids {
            indices.push(entity_index(drawing, id)?);
        }
        indices.sort_unstable();
        indices.dedup();
        for &index in &indices {
            if let Some(error) = locked_layer(drawing, &drawing.entities[index].layer) {
                return Err(error);
            }
        }
        let mut created = Vec::with_capacity(indices.len() * request.count as usize);
        for &index in &indices {
            let source = drawing.entities[index].clone();
            for step in 1..=request.count {
                let id = drawing.take_entity_id();
                let mut entity = Entity { id: id.clone(), layer: source.layer.clone(), kind: source.kind.clone() };
                entity.translate([request.dx * f64::from(step), request.dy * f64::from(step)]);
                drawing.entities.push(entity);
                created.push(id);
            }
        }
        drawing.revision += 1;
        Ok(created)
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "created": created,
            "count": created.len(),
        })
    })
}

/// Placement of an [`array`] operation.
#[derive(Debug, Clone, Copy)]
pub enum ArrayMode {
    /// Copies on a rectangular grid. The source stays at row 0, column 0.
    Rectangular {
        /// Number of rows, including the source row.
        rows: u32,
        /// Number of columns, including the source column.
        columns: u32,
        /// Vertical distance between adjacent rows.
        row_spacing: f64,
        /// Horizontal distance between adjacent columns.
        column_spacing: f64,
    },
    /// Copies rotated around one centre. The source stays at angle 0.
    Polar {
        /// Rotation centre `[x, y]`.
        center: [f64; 2],
        /// Number of items, including the source.
        count: u32,
        /// Total angle in degrees; copies are placed counter-clockwise at
        /// `total_angle * step / count`, so a full turn closes evenly.
        total_angle: f64,
    },
}

/// Arguments of [`array`].
#[derive(Debug, Clone)]
pub struct ArrayRequest {
    /// Source entity ids; repeated ids are arrayed once.
    pub ids: Vec<String>,
    /// Grid or polar placement.
    pub mode: ArrayMode,
}

/// Name of an entity kind used in user-facing messages.
fn kind_name(kind: &EntityKind) -> &'static str {
    match kind {
        EntityKind::Line { .. } => "line",
        EntityKind::Circle { .. } => "circle",
        EntityKind::Arc { .. } => "arc",
        EntityKind::Ellipse { .. } => "ellipse",
        EntityKind::Point { .. } => "point",
        EntityKind::Polyline { .. } => "polyline",
        EntityKind::Text { .. } => "text",
        EntityKind::Dimension { .. } => "dimension",
        EntityKind::RadialDimension { .. } => "radial dimension",
        EntityKind::OrdinateDimension { .. } => "ordinate dimension",
        EntityKind::AngularDimension { .. } => "angular dimension",
        EntityKind::Hatch { .. } => "hatch",
    }
}

fn validate_array(mode: &ArrayMode) -> Result<u64, ServiceError> {
    match *mode {
        ArrayMode::Rectangular { rows, columns, row_spacing, column_spacing } => {
            if rows == 0 || columns == 0 {
                return Err(argument("`rows` and `columns` must be at least 1"));
            }
            let positions = u64::from(rows) * u64::from(columns);
            if !(2..=10_000).contains(&positions) {
                return Err(argument("`rows * columns` must be between 2 and 10000"));
            }
            if !row_spacing.is_finite() || !column_spacing.is_finite() {
                return Err(argument("`row_spacing`/`column_spacing` must be finite"));
            }
            Ok(positions - 1)
        }
        ArrayMode::Polar { center, count, total_angle } => {
            if !(2..=10_000).contains(&count) {
                return Err(argument("`count` must be between 2 and 10000"));
            }
            if !finite_point(center) {
                return Err(argument("`center` must contain finite coordinates"));
            }
            if !total_angle.is_finite() || total_angle.abs() <= f64::EPSILON {
                return Err(argument("`total_angle` must be a non-zero finite number"));
            }
            Ok(u64::from(count) - 1)
        }
    }
}

/// Duplicates entities into a rectangular or polar array.
///
/// The source entity is not copied onto its own position: a rectangular array
/// with `rows * columns` cells creates `rows * columns - 1` copies, and a polar
/// array with `count` items creates `count - 1` copies. Ids are assigned in a
/// deterministic order (source id order, then row-major or increasing angle).
/// Locked source layers refuse with `drawing/layer-locked`. The response
/// carries `{ revision, drawing, created, count }`.
pub fn array(handle: u64, request: ArrayRequest) -> Result<Value, ServiceError> {
    if request.ids.is_empty() {
        return Err(argument("`ids` is empty"));
    }
    validate_array(&request.mode)?;
    let created = store::edit(handle, |drawing| {
        let mut indices = Vec::with_capacity(request.ids.len());
        for id in &request.ids {
            indices.push(entity_index(drawing, id)?);
        }
        indices.sort_unstable();
        indices.dedup();
        for &index in &indices {
            if let Some(error) = locked_layer(drawing, &drawing.entities[index].layer) {
                return Err(error);
            }
        }
        let mut created = Vec::new();
        for &index in &indices {
            let source = drawing.entities[index].clone();
            match request.mode {
                ArrayMode::Rectangular { rows, columns, row_spacing, column_spacing } => {
                    for row in 0..rows {
                        for column in 0..columns {
                            if row == 0 && column == 0 {
                                continue;
                            }
                            let offset = [column_spacing * f64::from(column), row_spacing * f64::from(row)];
                            let id = drawing.take_entity_id();
                            let mut entity = Entity { id: id.clone(), layer: source.layer.clone(), kind: source.kind.clone() };
                            entity.translate(offset);
                            drawing.entities.push(entity);
                            created.push(id);
                        }
                    }
                }
                ArrayMode::Polar { center, count, total_angle } => {
                    for step in 1..count {
                        let angle = total_angle * f64::from(step) / f64::from(count);
                        let id = drawing.take_entity_id();
                        let mut entity = Entity { id: id.clone(), layer: source.layer.clone(), kind: source.kind.clone() };
                        apply_transform(&mut entity, TransformOp::Rotate { angle, cx: center[0], cy: center[1] });
                        drawing.entities.push(entity);
                        created.push(id);
                    }
                }
            }
        }
        drawing.revision += 1;
        Ok(created)
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "created": created,
            "count": created.len(),
        })
    })
}

/// Splits one polyline into independent line entities.
///
/// The source polyline is removed and replaced, at the same list position, by
/// one line per edge in vertex order; a closed polyline also gets its closing
/// edge when the last vertex differs from the first. Each line receives a fresh
/// deterministic id. A non-polyline reports `object/unsupported`; a locked
/// layer reports `drawing/layer-locked`; fewer than two distinct vertices
/// reports `session/argument-invalid`. The response carries `{ revision,
/// drawing, entity, created, count }`.
pub fn explode(handle: u64, entity: &str) -> Result<Value, ServiceError> {
    let created = store::edit(handle, |drawing| {
        let index = entity_index(drawing, entity)?;
        let source = drawing.entities[index].clone();
        if let Some(error) = locked_layer(drawing, &source.layer) {
            return Err(error);
        }
        let EntityKind::Polyline { points, closed } = &source.kind
        else {
            return Err(ServiceError::new("object/unsupported", format!("entity `{entity}` is a {}, not a polyline", kind_name(&source.kind))));
        };
        let mut segments = points.windows(2).map(|pair| (pair[0], pair[1])).collect::<Vec<_>>();
        if *closed
            && let (Some(first), Some(last)) = (points.first(), points.last())
            && first != last
        {
            segments.push((*last, *first));
        }
        if segments.is_empty() {
            return Err(argument(format!("polyline `{entity}` needs at least 2 vertices to explode")));
        }
        let mut lines = Vec::with_capacity(segments.len());
        let mut created = Vec::with_capacity(segments.len());
        for (from, to) in segments {
            let id = drawing.take_entity_id();
            created.push(id.clone());
            lines.push(Entity { id, layer: source.layer.clone(), kind: EntityKind::Line { from, to } });
        }
        drawing.entities.splice(index..=index, lines);
        if let Some(position) = drawing.draw_order.iter().position(|id| id == entity) {
            drawing.draw_order.splice(position..=position, created.clone());
        }
        detach_entity_from_groups(drawing, entity);
        drawing.revision += 1;
        Ok(created)
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "entity": entity,
            "created": created,
            "count": created.len(),
        })
    })
}

/// Offsets a circle, arc, polyline or straight line into a new entity on the
/// same layer.
///
/// A positive `distance` grows circles and arcs and offsets an open polyline or
/// line to the left of its travel direction; a negative distance shrinks or
/// offsets to the right. Closed polylines offset around every vertex with
/// mitered joins. Text, point, ellipse and dimension entities report
/// `object/unsupported`; a resulting non-positive radius, a collapsed polyline
/// or a zero-length line reports `session/argument-invalid`; a locked layer
/// reports `drawing/layer-locked`. The response carries
/// `{ revision, drawing, entity, source }`.
pub fn offset(handle: u64, entity_id: &str, distance: f64) -> Result<Value, ServiceError> {
    if !distance.is_finite() || distance == 0.0 {
        return Err(argument("`distance` must be a non-zero finite number"));
    }
    let created = store::edit(handle, |drawing| {
        let index = entity_index(drawing, entity_id)?;
        let source = drawing.entities[index].clone();
        if let Some(error) = locked_layer(drawing, &source.layer) {
            return Err(error);
        }
        let kind = match &source.kind {
            EntityKind::Circle { center, radius } => {
                let offset_radius = radius + distance;
                if offset_radius <= 0.0 {
                    return Err(argument(format!("offsetting circle `{entity_id}` by {distance} leaves a non-positive radius")));
                }
                EntityKind::Circle { center: *center, radius: offset_radius }
            }
            EntityKind::Arc { center, radius, start_angle, end_angle } => {
                let offset_radius = radius + distance;
                if offset_radius <= 0.0 {
                    return Err(argument(format!("offsetting arc `{entity_id}` by {distance} leaves a non-positive radius")));
                }
                EntityKind::Arc { center: *center, radius: offset_radius, start_angle: *start_angle, end_angle: *end_angle }
            }
            EntityKind::Polyline { points, closed } => {
                EntityKind::Polyline { points: offset_polyline(points, *closed, distance)?, closed: *closed }
            }
            EntityKind::Line { from, to } => {
                let normal =
                    segment_normal(*from, *to).ok_or_else(|| argument(format!("line `{entity_id}` has zero length and cannot be offset")))?;
                let shift = [normal[0] * distance, normal[1] * distance];
                EntityKind::Line { from: [from[0] + shift[0], from[1] + shift[1]], to: [to[0] + shift[0], to[1] + shift[1]] }
            }
            other => {
                return Err(ServiceError::new(
                    "object/unsupported",
                    format!("entity `{entity_id}` is a {} and cannot be offset", kind_name(other)),
                ));
            }
        };
        let id = drawing.take_entity_id();
        let entity = Entity { id, layer: source.layer.clone(), kind };
        drawing.entities.push(entity.clone());
        drawing.revision += 1;
        Ok(entity)
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "entity": created,
            "source": entity_id,
        })
    })
}

/// One line corner shared by `fillet` and `chamfer`.
struct LineCorner {
    /// Intersection of the two infinite lines.
    point: [f64; 2],
    /// Unit direction from `point` towards each kept endpoint.
    direction: [[f64; 2]; 2],
    /// Distance from `point` to the discarded endpoint, measured along the kept direction.
    near: [f64; 2],
    /// Distance from `point` to the kept endpoint.
    far: [f64; 2],
    /// Whether the discarded endpoint is the `to` endpoint of each line.
    near_is_to: [bool; 2],
    /// Angle between the two kept directions, in radians.
    angle: f64,
}

fn line_endpoints(kind: &EntityKind) -> Option<([f64; 2], [f64; 2])> {
    match kind {
        EntityKind::Line { from, to } => Some((*from, *to)),
        _ => None,
    }
}

fn line_intersection(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> Option<[f64; 2]> {
    let first = [b[0] - a[0], b[1] - a[1]];
    let second = [d[0] - c[0], d[1] - c[1]];
    let denominator = first[0] * second[1] - first[1] * second[0];
    if denominator.abs() < 1e-12 {
        return None;
    }
    let offset = [c[0] - a[0], c[1] - a[1]];
    let parameter = (offset[0] * second[1] - offset[1] * second[0]) / denominator;
    Some([a[0] + first[0] * parameter, a[1] + first[1] * parameter])
}

fn distance(from: [f64; 2], to: [f64; 2]) -> f64 {
    (to[0] - from[0]).hypot(to[1] - from[1])
}

fn unit_direction(from: [f64; 2], to: [f64; 2]) -> Option<[f64; 2]> {
    let length = distance(from, to);
    if length < 1e-12 { None } else { Some([(to[0] - from[0]) / length, (to[1] - from[1]) / length]) }
}

/// Picks the corner where the two closest endpoints meet and measures both lines
/// from that intersection towards the endpoints they keep.
fn line_corner(first: &EntityKind, second: &EntityKind) -> Result<LineCorner, ServiceError> {
    let (first_from, first_to) = line_endpoints(first).ok_or_else(|| argument("both fillet/chamfer references must be lines"))?;
    let (second_from, second_to) = line_endpoints(second).ok_or_else(|| argument("both fillet/chamfer references must be lines"))?;
    let first_points = [first_from, first_to];
    let second_points = [second_from, second_to];
    let mut chosen = (0usize, 0usize, f64::INFINITY);
    for (first_index, first_point) in first_points.iter().enumerate() {
        for (second_index, second_point) in second_points.iter().enumerate() {
            let gap = distance(*first_point, *second_point);
            if gap < chosen.2 {
                chosen = (first_index, second_index, gap);
            }
        }
    }
    let point = line_intersection(first_from, first_to, second_from, second_to)
        .ok_or_else(|| argument("the two lines are parallel and have no fillet corner"))?;
    let keep = [first_points[1 - chosen.0], second_points[1 - chosen.1]];
    let near = [first_points[chosen.0], second_points[chosen.1]];
    let direction = [
        unit_direction(point, keep[0]).ok_or_else(|| argument("a zero-length line has no fillet corner"))?,
        unit_direction(point, keep[1]).ok_or_else(|| argument("a zero-length line has no fillet corner"))?,
    ];
    let dot = (direction[0][0] * direction[1][0] + direction[0][1] * direction[1][1]).clamp(-1.0, 1.0);
    let angle = dot.acos();
    if !(1e-9..std::f64::consts::PI - 1e-9).contains(&angle) {
        return Err(argument("the two lines are collinear and have no fillet corner"));
    }
    let measure = |index: usize| {
        let toward = direction[index];
        let near_distance = (near[index][0] - point[0]) * toward[0] + (near[index][1] - point[1]) * toward[1];
        (near_distance, distance(point, keep[index]))
    };
    let (near_first, far_first) = measure(0);
    let (near_second, far_second) = measure(1);
    Ok(LineCorner {
        point,
        direction,
        near: [near_first, near_second],
        far: [far_first, far_second],
        near_is_to: [chosen.0 == 1, chosen.1 == 1],
        angle,
    })
}

fn trim_line(kind: &mut EntityKind, near_is_to: bool, point: [f64; 2]) -> Result<(), ServiceError> {
    match kind {
        EntityKind::Line { from, to } => {
            if near_is_to {
                *to = point;
            }
            else {
                *from = point;
            }
            Ok(())
        }
        _ => Err(argument("both fillet/chamfer references must be lines")),
    }
}

/// Adds a fillet arc between two lines and trims both lines to the tangent points.
///
/// The corner is the pair of closest endpoints. Both lines must reach the
/// tangent points, so a radius that would land outside either segment reports
/// `session/argument-invalid`; parallel, collinear and zero-length lines also
/// report it. The arc is added on the first line's layer and the response
/// carries `{ revision, drawing, entity, sources }`.
pub fn fillet(handle: u64, first_id: &str, second_id: &str, radius: f64) -> Result<Value, ServiceError> {
    if !radius.is_finite() || radius <= 0.0 {
        return Err(argument("`radius` must be a positive finite number"));
    }
    if first_id == second_id {
        return Err(argument("a fillet needs two different lines"));
    }
    let created = store::edit(handle, |drawing| {
        let first_index = entity_index(drawing, first_id)?;
        let second_index = entity_index(drawing, second_id)?;
        for (id, index) in [(first_id, first_index), (second_id, second_index)] {
            let kind = &drawing.entities[index].kind;
            if line_endpoints(kind).is_none() {
                return Err(ServiceError::new("object/unsupported", format!("entity `{id}` is a {} and cannot be filleted", kind_name(kind))));
            }
        }
        let corner = line_corner(&drawing.entities[first_index].kind, &drawing.entities[second_index].kind)?;
        for index in [first_index, second_index] {
            if let Some(error) = locked_layer(drawing, &drawing.entities[index].layer) {
                return Err(error);
            }
        }
        let half = corner.angle / 2.0;
        let tangent = radius / half.tan();
        for index in 0..2 {
            let low = corner.near[index].max(0.0);
            if tangent < low - 1e-9 || tangent > corner.far[index] + 1e-9 {
                return Err(argument(format!(
                    "a radius of {radius} does not fit between the corner and the kept endpoint of `{}`",
                    if index == 0 { first_id } else { second_id }
                )));
            }
        }
        let bisector = [corner.direction[0][0] + corner.direction[1][0], corner.direction[0][1] + corner.direction[1][1]];
        let bisector = unit_direction([0.0, 0.0], bisector).ok_or_else(|| argument("the two lines point in opposite directions"))?;
        let center_distance = radius / half.sin();
        let center = [corner.point[0] + bisector[0] * center_distance, corner.point[1] + bisector[1] * center_distance];
        let tangent_points = [
            [corner.point[0] + corner.direction[0][0] * tangent, corner.point[1] + corner.direction[0][1] * tangent],
            [corner.point[0] + corner.direction[1][0] * tangent, corner.point[1] + corner.direction[1][1] * tangent],
        ];
        trim_line(&mut drawing.entities[first_index].kind, corner.near_is_to[0], tangent_points[0])?;
        trim_line(&mut drawing.entities[second_index].kind, corner.near_is_to[1], tangent_points[1])?;

        let start = (tangent_points[0][1] - center[1]).atan2(tangent_points[0][0] - center[0]).to_degrees();
        let end = (tangent_points[1][1] - center[1]).atan2(tangent_points[1][0] - center[0]).to_degrees();
        let (start_angle, end_angle) = if (end - start).rem_euclid(360.0) <= 180.0 { (start, end) } else { (end, start) };
        let id = drawing.take_entity_id();
        let entity =
            Entity { id, layer: drawing.entities[first_index].layer.clone(), kind: EntityKind::Arc { center, radius, start_angle, end_angle } };
        drawing.entities.push(entity.clone());
        drawing.revision += 1;
        Ok(entity)
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "entity": created,
            "sources": [first_id, second_id],
        })
    })
}

/// Adds a chamfer line between two lines and trims both lines to the chamfer points.
///
/// `distance_a` is measured from the corner along the first line and
/// `distance_b` along the second. The same corner, fit and parallel rules as
/// `fillet` apply. The chamfer line is added on the first line's layer and the
/// response carries `{ revision, drawing, entity, sources }`.
pub fn chamfer(handle: u64, first_id: &str, second_id: &str, distance_a: f64, distance_b: f64) -> Result<Value, ServiceError> {
    for value in [distance_a, distance_b] {
        if !value.is_finite() || value <= 0.0 {
            return Err(argument("chamfer distances must be positive finite numbers"));
        }
    }
    if first_id == second_id {
        return Err(argument("a chamfer needs two different lines"));
    }
    let created = store::edit(handle, |drawing| {
        let first_index = entity_index(drawing, first_id)?;
        let second_index = entity_index(drawing, second_id)?;
        for (id, index) in [(first_id, first_index), (second_id, second_index)] {
            let kind = &drawing.entities[index].kind;
            if line_endpoints(kind).is_none() {
                return Err(ServiceError::new("object/unsupported", format!("entity `{id}` is a {} and cannot be chamfered", kind_name(kind))));
            }
        }
        let corner = line_corner(&drawing.entities[first_index].kind, &drawing.entities[second_index].kind)?;
        for index in [first_index, second_index] {
            if let Some(error) = locked_layer(drawing, &drawing.entities[index].layer) {
                return Err(error);
            }
        }
        let distances = [distance_a, distance_b];
        let mut trimmed = [[0.0, 0.0]; 2];
        for index in 0..2 {
            let low = corner.near[index].max(0.0);
            if distances[index] < low - 1e-9 || distances[index] > corner.far[index] + 1e-9 {
                return Err(argument(format!(
                    "a chamfer distance of {} does not fit between the corner and the kept endpoint of `{}`",
                    distances[index],
                    if index == 0 { first_id } else { second_id }
                )));
            }
            trimmed[index] = [
                corner.point[0] + corner.direction[index][0] * distances[index],
                corner.point[1] + corner.direction[index][1] * distances[index],
            ];
        }
        trim_line(&mut drawing.entities[first_index].kind, corner.near_is_to[0], trimmed[0])?;
        trim_line(&mut drawing.entities[second_index].kind, corner.near_is_to[1], trimmed[1])?;

        let id = drawing.take_entity_id();
        let entity =
            Entity { id, layer: drawing.entities[first_index].layer.clone(), kind: EntityKind::Line { from: trimmed[0], to: trimmed[1] } };
        drawing.entities.push(entity.clone());
        drawing.revision += 1;
        Ok(entity)
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "entity": created,
            "sources": [first_id, second_id],
        })
    })
}

fn segment_normal(from: [f64; 2], to: [f64; 2]) -> Option<[f64; 2]> {
    let dx = to[0] - from[0];
    let dy = to[1] - from[1];
    let length = (dx * dx + dy * dy).sqrt();
    if length < 1e-12 { None } else { Some([-dy / length, dx / length]) }
}

fn offset_polyline(points: &[[f64; 2]], closed: bool, distance: f64) -> Result<Vec<[f64; 2]>, ServiceError> {
    if points.len() < 2 {
        return Err(argument("a polyline needs at least 2 vertices to offset"));
    }
    let repeated = closed && points.first() == points.last();
    let unique = if repeated { &points[..points.len() - 1] } else { points };
    let mut normals = unique
        .windows(2)
        .map(|pair| segment_normal(pair[0], pair[1]))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| argument("a polyline with a zero-length segment cannot be offset"))?;
    if closed {
        let closing = segment_normal(*unique.last().expect("non-empty"), unique[0])
            .ok_or_else(|| argument("a polyline with a zero-length closing segment cannot be offset"))?;
        normals.push(closing);
    }
    let mut moved = offset_vertices(unique, &normals, closed, distance)?;
    if repeated {
        moved.push(moved[0]);
    }
    Ok(moved)
}

fn offset_vertices(points: &[[f64; 2]], normals: &[[f64; 2]], closed: bool, distance: f64) -> Result<Vec<[f64; 2]>, ServiceError> {
    let mut moved = Vec::with_capacity(points.len());
    for (index, point) in points.iter().enumerate() {
        let previous = if index > 0 {
            Some(normals[index - 1])
        }
        else if closed && normals.len() == points.len() {
            Some(normals[normals.len() - 1])
        }
        else {
            None
        };
        let next = if index < normals.len() { Some(normals[index]) } else { None };
        let displacement = match (previous, next) {
            (Some(a), Some(b)) => {
                let sum = [a[0] + b[0], a[1] + b[1]];
                let denominator = 1.0 + a[0] * b[0] + a[1] * b[1];
                if denominator.abs() < 1e-9 || (sum[0] * sum[0] + sum[1] * sum[1]).sqrt() < 1e-9 {
                    [a[0] * distance, a[1] * distance]
                }
                else {
                    let factor = distance / denominator;
                    [sum[0] * factor, sum[1] * factor]
                }
            }
            (Some(normal), None) | (None, Some(normal)) => [normal[0] * distance, normal[1] * distance],
            (None, None) => return Err(argument("a polyline needs at least 2 distinct vertices to offset")),
        };
        let moved_point = [point[0] + displacement[0], point[1] + displacement[1]];
        if !finite_point(moved_point) {
            return Err(argument("the offset produces a non-finite vertex"));
        }
        moved.push(moved_point);
    }
    Ok(moved)
}

fn drawing_extents(drawing: &Drawing) -> ([f64; 2], [f64; 2], bool) {
    let mut extents: Option<Rect> = None;
    for entity in &drawing.entities {
        for point in entity.bounds_points() {
            if !finite_point(point) {
                continue;
            }
            let point = PlanarPoint::new(point[0], point[1]);
            let point_rect = Rect::from_points(point, point);
            extents = Some(extents.map_or(point_rect, |extents| extents.union(point_rect)));
        }
    }
    match extents {
        Some(extents) => ([extents.min.x, extents.min.y], [extents.max.x, extents.max.y], true),
        None => ([0.0, 0.0], [0.0, 0.0], false),
    }
}

fn bounds_json(drawing: &Drawing) -> Value {
    let (min, max, seen) = drawing_extents(drawing);
    if seen {
        json!({ "minX": min[0], "minY": min[1], "maxX": max[0], "maxY": max[1] })
    }
    else {
        json!({ "minX": 0.0, "minY": 0.0, "maxX": 0.0, "maxY": 0.0 })
    }
}

fn polyline_area(points: &[[f64; 2]]) -> f64 {
    if points.len() < 3 {
        return 0.0;
    }
    let polygon = points.iter().map(|point| PlanarPoint::new(point[0], point[1])).collect::<Vec<_>>();
    polygon_area(&polygon)
}

fn point_distance(from: [f64; 2], to: [f64; 2]) -> f64 {
    PlanarPoint::new(from[0], from[1]).distance(PlanarPoint::new(to[0], to[1]))
}

fn curve_length(points: &[[f64; 2]]) -> f64 {
    points.windows(2).map(|pair| point_distance(pair[0], pair[1])).sum()
}

/// Whether an entity encloses an area, so its `length` is a perimeter.
fn entity_closed(kind: &EntityKind) -> bool {
    match kind {
        EntityKind::Polyline { closed, .. } => *closed,
        EntityKind::Hatch { .. } | EntityKind::Circle { .. } => true,
        EntityKind::Ellipse { start_angle, end_angle, .. } => arc_sweep(*start_angle, *end_angle) >= 360.0 - 1e-9,
        _ => false,
    }
}

/// Path length and enclosed area one entity contributes to [`measure`] and
/// [`stats`].
fn entity_length_area(kind: &EntityKind) -> (f64, f64) {
    match kind {
        EntityKind::Line { from, to } => (point_distance(*from, *to), 0.0),
        EntityKind::Polyline { points, closed } => {
            let mut length = curve_length(points);
            let mut area = 0.0;
            if *closed && points.len() > 1 {
                length += point_distance(points[0], points[points.len() - 1]);
                area += polyline_area(points);
            }
            (length, area)
        }
        EntityKind::Arc { radius, start_angle, end_angle, .. } => (radius.abs() * arc_sweep(*start_angle, *end_angle).to_radians(), 0.0),
        EntityKind::Ellipse { center, rx, ry, rotation, start_angle, end_angle } => {
            let sweep = arc_sweep(*start_angle, *end_angle);
            let length = curve_length(&sample_ellipse(*center, rx.abs(), ry.abs(), *rotation, *start_angle, *end_angle));
            let area = if sweep >= 360.0 - 1e-9 { std::f64::consts::PI * rx.abs() * ry.abs() } else { 0.0 };
            (length, area)
        }
        EntityKind::Circle { radius, .. } => (0.0, std::f64::consts::PI * radius * radius),
        EntityKind::Hatch { points, .. } => {
            let mut length = curve_length(points);
            if points.len() > 1 {
                length += point_distance(points[0], points[points.len() - 1]);
            }
            (length, polyline_area(points))
        }
        EntityKind::Point { .. }
        | EntityKind::Text { .. }
        | EntityKind::Dimension { .. }
        | EntityKind::RadialDimension { .. }
        | EntityKind::OrdinateDimension { .. }
        | EntityKind::AngularDimension { .. } => (0.0, 0.0),
    }
}

/// Direction of a [`ucs`] conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UcsDirection {
    /// Convert user-coordinate values into world coordinates.
    ToWorld,
    /// Convert world coordinates into user-coordinate values.
    ToUcs,
}

/// Arguments of [`ucs`].
#[derive(Debug, Clone)]
pub struct UcsRequest {
    /// UCS origin in world coordinates.
    pub origin: [f64; 2],
    /// Angle of the UCS X axis from the world X axis, in degrees.
    pub rotation: f64,
    /// Conversion direction.
    pub direction: UcsDirection,
    /// Points to convert without touching the drawing.
    pub points: Vec<[f64; 2]>,
    /// Entities whose geometry is converted in place.
    pub ids: Vec<String>,
}

fn ucs_point(point: [f64; 2], origin: [f64; 2], rotation: f64, direction: UcsDirection) -> [f64; 2] {
    let (sin, cos) = rotation.to_radians().sin_cos();
    match direction {
        UcsDirection::ToWorld => [origin[0] + point[0] * cos - point[1] * sin, origin[1] + point[0] * sin + point[1] * cos],
        UcsDirection::ToUcs => {
            let dx = point[0] - origin[0];
            let dy = point[1] - origin[1];
            [dx * cos + dy * sin, -dx * sin + dy * cos]
        }
    }
}

fn apply_ucs_transform(entity: &mut Entity, origin: [f64; 2], rotation: f64, direction: UcsDirection) {
    match direction {
        UcsDirection::ToWorld => {
            apply_transform(entity, TransformOp::Rotate { angle: rotation, cx: 0.0, cy: 0.0 });
            entity.translate(origin);
        }
        UcsDirection::ToUcs => {
            entity.translate([-origin[0], -origin[1]]);
            apply_transform(entity, TransformOp::Rotate { angle: -rotation, cx: 0.0, cy: 0.0 });
        }
    }
}

/// Converts points and entities between world and user coordinates.
///
/// A UCS is an origin plus the angle of its X axis from the world X axis.
/// `direction: ToWorld` maps UCS values into the world system, and `ToUcs`
/// maps world values into the UCS. `points` are converted without touching the
/// drawing. `ids` rotate and translate the selected entities in place in one
/// history step, and a locked or frozen layer refuses with
/// `drawing/layer-locked` or `drawing/layer-frozen`. The response carries
/// `{ revision, direction, origin, rotation, points, entities }`, plus `drawing`
/// when the call mutated the document.
pub fn ucs(handle: u64, request: UcsRequest) -> Result<Value, ServiceError> {
    if !request.origin.iter().all(|value| value.is_finite()) {
        return Err(argument("`origin` must contain finite coordinates"));
    }
    if !request.rotation.is_finite() {
        return Err(argument("`rotation` must be a finite number of degrees"));
    }
    if request.points.is_empty() && request.ids.is_empty() {
        return Err(argument("pass `points`, `ids` or both"));
    }
    if !request.points.iter().all(|point| finite_point(*point)) {
        return Err(argument("`points` must contain finite coordinates"));
    }
    let direction = match request.direction {
        UcsDirection::ToWorld => "to-world",
        UcsDirection::ToUcs => "to-ucs",
    };
    let converted = request
        .points
        .iter()
        .map(|point| json!({ "from": point, "to": ucs_point(*point, request.origin, request.rotation, request.direction) }))
        .collect::<Vec<_>>();
    if request.ids.is_empty() {
        return store::read(handle, |session| {
            json!({
                "revision": session.drawing.revision,
                "direction": direction,
                "origin": request.origin,
                "rotation": request.rotation,
                "points": converted.clone(),
                "entities": [],
            })
        });
    }
    let applied = store::edit(handle, |drawing| {
        let mut seen = std::collections::HashSet::new();
        let mut applied = Vec::new();
        for id in &request.ids {
            if !seen.insert(id.as_str()) {
                continue;
            }
            let index = entity_index(drawing, id)?;
            let layer = drawing.entities[index].layer.clone();
            if let Some(error) = locked_layer(drawing, &layer) {
                return Err(error);
            }
            apply_ucs_transform(&mut drawing.entities[index], request.origin, request.rotation, request.direction);
            applied.push(id.clone());
        }
        drawing.revision += 1;
        Ok(applied)
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "direction": direction,
            "origin": request.origin,
            "rotation": request.rotation,
            "points": converted,
            "entities": applied,
        })
    })
}

/// Measures the drawing extent, path length and enclosed area.
///
/// The exact bounding box comes from every entity's own extent computation.
/// `length` sums line segments, polyline edges (including the closing edge of a
/// closed polyline) and the analytic length of arcs; ellipse length is sampled
/// at five-degree steps, so it is an approximation. `area` sums the shoelace
/// area of closed polylines, full circles and full ellipses; partial arcs and
/// ellipses contribute no area, and a self-intersecting polyline can cancel.
/// The response carries `{ bounds, width, height, entities, length, area }`.
pub fn measure(handle: u64, sheet: Option<String>) -> Result<Value, ServiceError> {
    require_sheet_absent(sheet.as_deref())?;
    store::read(handle, |session| {
        let drawing = &session.drawing;
        let (min, max, seen) = drawing_extents(drawing);
        let mut length = 0.0;
        let mut area = 0.0;
        for entity in &drawing.entities {
            let (entity_length, entity_area) = entity_length_area(&entity.kind);
            length += entity_length;
            area += entity_area;
        }
        let (width, height) = if seen { (max[0] - min[0], max[1] - min[1]) } else { (0.0, 0.0) };
        let (min_x, min_y) = if seen { (min[0], min[1]) } else { (0.0, 0.0) };
        let (max_x, max_y) = if seen { (max[0], max[1]) } else { (0.0, 0.0) };
        json!({
            "revision": drawing.revision,
            "bounds": { "minX": min_x, "minY": min_y, "maxX": max_x, "maxY": max_y },
            "width": width,
            "height": height,
            "entities": drawing.entities.len(),
            "length": length,
            "area": area,
        })
    })
}

/// Measures the path length and enclosed area of one or more entities.
///
/// Length and area use the same rules as [`measure`]: line segments, polyline
/// edges (including the closing edge of a closed polyline), hatch boundaries and
/// analytic arc lengths; ellipse length is sampled at five-degree steps. `area`
/// covers closed polylines, hatches, full circles and full ellipses, so a closed
/// polyline reports its perimeter as `length` and its enclosed area as `area`.
/// Repeated ids are measured once. A missing id reports `object/not-found`.
/// The response carries `{ revision, units, entities, totals }`, where each
/// entity is `{ id, layer, kind, closed, length, area }`.
pub fn measure_entities(handle: u64, ids: &[String]) -> Result<Value, ServiceError> {
    if ids.is_empty() {
        return Err(argument("pass at least one entity id to measure"));
    }
    store::read(handle, |session| -> Result<Value, ServiceError> {
        let mut entries = Vec::new();
        let mut total_length = 0.0;
        let mut total_area = 0.0;
        let mut seen = std::collections::HashSet::new();
        for id in ids {
            if !seen.insert(id.as_str()) {
                continue;
            }
            let entity = session
                .drawing
                .entities
                .iter()
                .find(|entity| &entity.id == id)
                .ok_or_else(|| ServiceError::new("object/not-found", format!("entity `{id}` does not exist")))?;
            let (length, area) = entity_length_area(&entity.kind);
            total_length += length;
            total_area += area;
            entries.push(json!({
                "id": entity.id,
                "layer": entity.layer,
                "kind": kind_name(&entity.kind),
                "closed": entity_closed(&entity.kind),
                "length": length,
                "area": area,
            }));
        }
        Ok(json!({
            "revision": session.drawing.revision,
            "units": session.drawing.units.as_str(),
            "entities": entries,
            "totals": { "entities": entries.len(), "length": total_length, "area": total_area },
        }))
    })?
}

/// Measures the distance between two points or the angle at three points.
///
/// Two points report `kind: "distance"` with `distance`, the signed `dx`/`dy`
/// offsets and the `bearing` in degrees counter-clockwise from the positive X
/// axis. Three points treat the second point as the vertex and report
/// `kind: "angle"` with the angle between the two legs in degrees, the leg
/// lengths `first`/`second` and the `span` between the first and third points.
/// The query never changes the drawing and the response carries
/// `{ revision, units, ... }`.
pub fn measure_points(handle: u64, points: &[[f64; 2]]) -> Result<Value, ServiceError> {
    if points.len() != 2 && points.len() != 3 {
        return Err(argument("pass exactly 2 points for a distance or 3 points for an angle"));
    }
    if !points.iter().all(|point| finite_point(*point)) {
        return Err(argument("`points` must contain finite coordinates"));
    }
    store::read(handle, |session| -> Result<Value, ServiceError> {
        let revision = session.drawing.revision;
        let units = session.drawing.units.as_str();
        if points.len() == 2 {
            let (from, to) = (points[0], points[1]);
            let dx = to[0] - from[0];
            let dy = to[1] - from[1];
            return Ok(json!({
                "revision": revision,
                "units": units,
                "kind": "distance",
                "points": 2,
                "from": from,
                "to": to,
                "distance": point_distance(from, to),
                "dx": dx,
                "dy": dy,
                "bearing": normalize_angle(dy.atan2(dx).to_degrees()),
            }));
        }
        let (first, vertex, third) = (points[0], points[1], points[2]);
        let leg_first = point_distance(first, vertex);
        let leg_second = point_distance(vertex, third);
        if leg_first < 1e-9 || leg_second < 1e-9 {
            return Err(argument("the angle needs two distinct legs from the vertex"));
        }
        let ux = first[0] - vertex[0];
        let uy = first[1] - vertex[1];
        let vx = third[0] - vertex[0];
        let vy = third[1] - vertex[1];
        let angle = (ux * vy - uy * vx).atan2(ux * vx + uy * vy).abs().to_degrees();
        Ok(json!({
            "revision": revision,
            "units": units,
            "kind": "angle",
            "points": 3,
            "vertex": vertex,
            "angle": angle,
            "first": leg_first,
            "second": leg_second,
            "span": point_distance(first, third),
        }))
    })?
}

/// Summarises entity count, path length and enclosed area per layer.
///
/// Length and area use the same rules as [`measure`]: analytic lines, arcs and
/// circles, five-degree ellipse sampling, and shoelace area only for closed
/// polylines, full circles and full ellipses. Annotations contribute nothing.
/// The response carries `{ revision, units, layers, totals }`, where each layer
/// is `{ id, name, visible, locked, entityCount, length, area }`.
pub fn stats(handle: u64) -> Result<Value, ServiceError> {
    store::read(handle, |session| {
        let drawing = &session.drawing;
        let mut layers = Vec::with_capacity(drawing.layers.len());
        for layer in &drawing.layers {
            let mut entity_count = 0;
            let mut length = 0.0;
            let mut area = 0.0;
            for entity in drawing.entities.iter().filter(|entity| entity.layer == layer.id) {
                let (entity_length, entity_area) = entity_length_area(&entity.kind);
                entity_count += 1;
                length += entity_length;
                area += entity_area;
            }
            layers.push(json!({
                "id": layer.id,
                "name": layer.name,
                "visible": layer.visible,
                "locked": layer.locked,
                "entityCount": entity_count,
                "length": length,
                "area": area,
            }));
        }
        let mut total_length = 0.0;
        let mut total_area = 0.0;
        for entity in &drawing.entities {
            let (entity_length, entity_area) = entity_length_area(&entity.kind);
            total_length += entity_length;
            total_area += entity_area;
        }
        json!({
            "revision": drawing.revision,
            "units": drawing.units.as_str(),
            "layers": layers,
            "totals": { "entities": drawing.entities.len(), "length": total_length, "area": total_area },
        })
    })
}

fn valid_color(value: &str) -> bool {
    let trimmed = value.trim();
    let Some(digits) = trimmed.strip_prefix('#')
    else {
        return false;
    };
    (digits.len() == 3 || digits.len() == 6) && digits.chars().all(|character| character.is_ascii_hexdigit())
}

/// Adds a layer and returns `{ revision, drawing, layer }`.
///
/// The layer id is the trimmed name, so a rename also renames the reference.
/// Names are unique case-insensitively; a duplicate reports `drawing/layer-exists`.
pub fn add_layer(handle: u64, name: &str, color: Option<&str>) -> Result<Value, ServiceError> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(argument("a layer needs a non-empty name"));
    }
    let color = color.map(str::trim).unwrap_or("#d0d7de").to_string();
    if !valid_color(&color) {
        return Err(argument(format!("`{color}` is not a #rgb or #rrggbb colour")));
    }
    store::edit(handle, |drawing| {
        if drawing.layers.iter().any(|layer| layer.id == name || layer.name.eq_ignore_ascii_case(&name)) {
            return Err(ServiceError::new("drawing/layer-exists", format!("a layer named `{name}` already exists")));
        }
        drawing.layers.push(Layer {
            id: name.clone(),
            name: name.clone(),
            color,
            visible: true,
            locked: false,
            frozen: false,
            plottable: true,
            linetype: CONTINUOUS_LINETYPE.to_string(),
            lineweight: 0.0,
        });
        drawing.revision += 1;
        Ok(())
    })?;
    store::read(handle, |session| {
        let layer = layer_json(&session.drawing, &name);
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "layer": layer,
        })
    })
}

/// Fields an update to one layer may change.
#[derive(Debug, Clone, Default)]
pub struct LayerUpdate {
    /// New display name; also becomes the layer id.
    pub name: Option<String>,
    /// New `#rgb` or `#rrggbb` colour.
    pub color: Option<String>,
    /// Whether the exporters paint the layer.
    pub visible: Option<bool>,
    /// Whether geometry on the layer is writable.
    pub locked: Option<bool>,
    /// Whether the layer is frozen and therefore hidden and read-only.
    pub frozen: Option<bool>,
    /// Whether plots and print exports include the layer.
    pub plottable: Option<bool>,
    /// Line type id assigned to the layer; it must already exist.
    pub linetype: Option<String>,
    /// Lineweight in millimetres, `0..=10`; `0` uses the exporter default.
    pub lineweight: Option<f64>,
}

/// Updates one layer and rewrites entity references when the name changes.
pub fn update_layer(handle: u64, id: &str, update: LayerUpdate) -> Result<Value, ServiceError> {
    if update.name.is_none()
        && update.color.is_none()
        && update.visible.is_none()
        && update.locked.is_none()
        && update.frozen.is_none()
        && update.plottable.is_none()
        && update.linetype.is_none()
        && update.lineweight.is_none()
    {
        return Err(argument("pass at least one of `name`, `color`, `visible`, `locked`, `frozen`, `plottable`, `linetype` or `lineweight`"));
    }
    let new_name = match update.name {
        Some(name) => {
            let trimmed = name.trim().to_string();
            if trimmed.is_empty() {
                return Err(argument("a layer needs a non-empty name"));
            }
            Some(trimmed)
        }
        None => None,
    };
    if let Some(color) = &update.color
        && !valid_color(color)
    {
        return Err(argument(format!("`{color}` is not a #rgb or #rrggbb colour")));
    }
    if let Some(lineweight) = update.lineweight
        && (!lineweight.is_finite() || !(0.0..=MAX_LINEWEIGHT).contains(&lineweight))
    {
        return Err(argument(format!("a lineweight must be between 0 and {MAX_LINEWEIGHT} millimetres")));
    }
    let linetype = update.linetype.as_ref().map(|name| name.trim().to_string());
    if linetype.as_ref().is_some_and(String::is_empty) {
        return Err(argument("a line type id cannot be empty"));
    }
    let target = id.to_string();
    let next_id = new_name.clone().unwrap_or_else(|| target.clone());
    store::edit(handle, |drawing| {
        let index = drawing
            .layers
            .iter()
            .position(|layer| layer.id == target)
            .ok_or_else(|| ServiceError::new("object/not-found", format!("layer `{target}` does not exist")))?;
        if let Some(name) = &new_name
            && drawing
                .layers
                .iter()
                .enumerate()
                .any(|(other, layer)| other != index && (layer.id == *name || layer.name.eq_ignore_ascii_case(name)))
        {
            return Err(ServiceError::new("drawing/layer-exists", format!("a layer named `{name}` already exists")));
        }
        if let Some(linetype) = &linetype
            && !drawing.linetypes.iter().any(|entry| entry.id == *linetype)
        {
            return Err(ServiceError::new("object/not-found", format!("line type `{linetype}` does not exist")));
        }
        let Some(layer) = drawing.layers.get_mut(index)
        else {
            return Err(ServiceError::new("object/not-found", format!("layer `{target}` does not exist")));
        };
        if let Some(name) = new_name {
            layer.id = name.clone();
            layer.name = name;
        }
        if let Some(color) = update.color {
            layer.color = color;
        }
        if let Some(visible) = update.visible {
            layer.visible = visible;
        }
        if let Some(locked) = update.locked {
            layer.locked = locked;
        }
        if let Some(frozen) = update.frozen {
            layer.frozen = frozen;
        }
        if let Some(plottable) = update.plottable {
            layer.plottable = plottable;
        }
        if let Some(linetype) = linetype {
            layer.linetype = linetype;
        }
        if let Some(lineweight) = update.lineweight {
            layer.lineweight = lineweight;
        }
        let renamed = layer.id.clone();
        for entity in &mut drawing.entities {
            if entity.layer == target {
                entity.layer = renamed.clone();
            }
        }
        drawing.revision += 1;
        Ok(())
    })?;
    store::read(handle, |session| {
        let layer = layer_json(&session.drawing, &next_id);
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "layer": layer,
        })
    })
}

/// Removes one empty layer and returns `{ revision, drawing, deleted }`.
///
/// A layer that any entity still references refuses with `drawing/layer-in-use`
/// and a message naming the layer and the reference count. Removing the last
/// remaining layer refuses with `drawing/layer-last`, so a drawing always keeps
/// at least one layer. The removal is a normal history step.
pub fn delete_layer(handle: u64, id: &str) -> Result<Value, ServiceError> {
    store::edit(handle, |drawing| {
        let index = drawing
            .layers
            .iter()
            .position(|layer| layer.id == id)
            .ok_or_else(|| ServiceError::new("object/not-found", format!("layer `{id}` does not exist")))?;
        if drawing.layers.len() <= 1 {
            return Err(ServiceError::new(
                "drawing/layer-last",
                format!("layer `{id}` is the last layer; a drawing must keep at least one layer"),
            ));
        }
        let count = drawing.entities.iter().filter(|entity| entity.layer == id).count();
        if count > 0 {
            return Err(ServiceError::new("drawing/layer-in-use", format!("layer `{id}` is referenced by {count} entity/entities")));
        }
        drawing.layers.remove(index);
        drawing.revision += 1;
        Ok(())
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "deleted": id,
        })
    })
}

/// Moves every entity of one layer onto another and removes the empty source.
///
/// The merge refuses when either layer is locked (`drawing/layer-locked`), when
/// a layer is missing (`object/not-found`), or when both names are the same.
/// The response carries `{ revision, drawing, from, to, moved }`; `moved` is
/// the number of entities whose layer reference changed. The merge is a normal
/// history step.
pub fn merge_layer(handle: u64, from: &str, to: &str) -> Result<Value, ServiceError> {
    if from == to {
        return Err(argument("`from` and `to` name the same layer"));
    }
    let moved = store::edit(handle, |drawing| {
        let from_index = drawing
            .layers
            .iter()
            .position(|layer| layer.id == from)
            .ok_or_else(|| ServiceError::new("object/not-found", format!("layer `{from}` does not exist")))?;
        if drawing.layer(to).is_none() {
            return Err(ServiceError::new("object/not-found", format!("layer `{to}` does not exist")));
        }
        if let Some(error) = locked_layer(drawing, from) {
            return Err(error);
        }
        if let Some(error) = locked_layer(drawing, to) {
            return Err(error);
        }
        let mut moved = 0;
        for entity in &mut drawing.entities {
            if entity.layer == from {
                entity.layer = to.to_string();
                moved += 1;
            }
        }
        drawing.layers.remove(from_index);
        drawing.revision += 1;
        Ok(moved)
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "from": from,
            "to": to,
            "moved": moved,
        })
    })
}

/// Fields used to create one dimension style. Omitted values keep the
/// `standard` defaults.
#[derive(Debug, Clone, Default)]
pub struct DimensionStyleRequest {
    /// Style name; it also becomes the style id.
    pub name: String,
    /// Overall scale applied to the style's lengths.
    pub scale: Option<f64>,
    /// Arrowhead size in drawing units.
    pub arrow_size: Option<f64>,
    /// Dimension text height in drawing units.
    pub text_height: Option<f64>,
    /// Extension line length beyond the dimension line.
    pub extension: Option<f64>,
    /// Gap between a measured point and its extension line.
    pub origin_offset: Option<f64>,
    /// Decimal places in a measured label, `0..=8`.
    pub precision: Option<u32>,
}

/// Fields an update to one dimension style may change.
#[derive(Debug, Clone, Default)]
pub struct DimensionStyleUpdate {
    /// New style name; also becomes the style id.
    pub name: Option<String>,
    /// New overall scale.
    pub scale: Option<f64>,
    /// New arrowhead size.
    pub arrow_size: Option<f64>,
    /// New text height.
    pub text_height: Option<f64>,
    /// New extension line length.
    pub extension: Option<f64>,
    /// New origin offset.
    pub origin_offset: Option<f64>,
    /// New decimal places, `0..=8`.
    pub precision: Option<u32>,
}

/// Lists the dimension styles of one drawing.
pub fn list_dim_styles(handle: u64) -> Result<Value, ServiceError> {
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "dimStyles": session.drawing.dim_styles,
        })
    })
}

/// Adds one dimension style and returns `{ revision, drawing, style }`.
///
/// Names are unique case-insensitively; a duplicate reports
/// `drawing/style-exists`. A `.ocad` document always keeps at least one style.
pub fn add_dim_style(handle: u64, request: DimensionStyleRequest) -> Result<Value, ServiceError> {
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err(argument("a dimension style needs a non-empty name"));
    }
    let style = DimensionStyle {
        id: name.clone(),
        name,
        scale: request.scale.unwrap_or(1.0),
        arrow_size: request.arrow_size.unwrap_or(2.5),
        text_height: request.text_height.unwrap_or(2.5),
        extension: request.extension.unwrap_or(1.25),
        origin_offset: request.origin_offset.unwrap_or(0.625),
        precision: request.precision.unwrap_or(2),
    };
    style.validate().map_err(argument)?;
    let id = style.id.clone();
    store::edit(handle, |drawing| {
        if drawing.dim_styles.iter().any(|existing| existing.id.eq_ignore_ascii_case(&style.id)) {
            return Err(ServiceError::new("drawing/style-exists", format!("a dimension style named `{}` already exists", style.name)));
        }
        drawing.dim_styles.push(style);
        drawing.revision += 1;
        Ok(())
    })?;
    read_dim_style(handle, &id)
}

/// Updates one dimension style and returns `{ revision, drawing, style }`.
pub fn update_dim_style(handle: u64, id: &str, update: DimensionStyleUpdate) -> Result<Value, ServiceError> {
    if update.name.is_none()
        && update.scale.is_none()
        && update.arrow_size.is_none()
        && update.text_height.is_none()
        && update.extension.is_none()
        && update.origin_offset.is_none()
        && update.precision.is_none()
    {
        return Err(argument("pass at least one dimension style field to update"));
    }
    let new_name = match &update.name {
        Some(name) => {
            let trimmed = name.trim().to_string();
            if trimmed.is_empty() {
                return Err(argument("a dimension style needs a non-empty name"));
            }
            Some(trimmed)
        }
        None => None,
    };
    let target = id.to_string();
    let next_id = new_name.clone().unwrap_or_else(|| target.clone());
    store::edit(handle, |drawing| {
        let index = drawing
            .dim_styles
            .iter()
            .position(|style| style.id == target)
            .ok_or_else(|| ServiceError::new("object/not-found", format!("dimension style `{target}` does not exist")))?;
        if let Some(name) = &new_name
            && drawing.dim_styles.iter().enumerate().any(|(other, style)| other != index && style.id.eq_ignore_ascii_case(name))
        {
            return Err(ServiceError::new("drawing/style-exists", format!("a dimension style named `{name}` already exists")));
        }
        let Some(style) = drawing.dim_styles.get_mut(index)
        else {
            return Err(ServiceError::new("object/not-found", format!("dimension style `{target}` does not exist")));
        };
        if let Some(name) = new_name {
            style.id = name.clone();
            style.name = name;
        }
        if let Some(scale) = update.scale {
            style.scale = scale;
        }
        if let Some(arrow_size) = update.arrow_size {
            style.arrow_size = arrow_size;
        }
        if let Some(text_height) = update.text_height {
            style.text_height = text_height;
        }
        if let Some(extension) = update.extension {
            style.extension = extension;
        }
        if let Some(origin_offset) = update.origin_offset {
            style.origin_offset = origin_offset;
        }
        if let Some(precision) = update.precision {
            style.precision = precision;
        }
        style.validate().map_err(argument)?;
        drawing.revision += 1;
        Ok(())
    })?;
    read_dim_style(handle, &next_id)
}

/// Removes one dimension style and returns `{ revision, drawing, deleted }`.
///
/// Removing the last remaining style refuses with `drawing/style-last`, so a
/// document always keeps one style.
pub fn delete_dim_style(handle: u64, id: &str) -> Result<Value, ServiceError> {
    store::edit(handle, |drawing| {
        let index = drawing
            .dim_styles
            .iter()
            .position(|style| style.id == id)
            .ok_or_else(|| ServiceError::new("object/not-found", format!("dimension style `{id}` does not exist")))?;
        if drawing.dim_styles.len() <= 1 {
            return Err(ServiceError::new(
                "drawing/style-last",
                format!("dimension style `{id}` is the last style; a drawing must keep at least one style"),
            ));
        }
        drawing.dim_styles.remove(index);
        drawing.revision += 1;
        Ok(())
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "deleted": id,
        })
    })
}

fn read_dim_style(handle: u64, id: &str) -> Result<Value, ServiceError> {
    store::read(handle, |session| {
        let style = session.drawing.dim_styles.iter().find(|style| style.id == id);
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "style": style,
        })
    })
}

/// Fields used to create one text style. Omitted values keep the `standard`
/// defaults.
#[derive(Debug, Clone, Default)]
pub struct TextStyleRequest {
    /// Style name; it also becomes the style id.
    pub name: String,
    /// Primary font file or family name.
    pub font: Option<String>,
    /// Fixed cap height; `0` leaves the height to each text entity.
    pub height: Option<f64>,
    /// Horizontal width factor.
    pub width_factor: Option<f64>,
    /// Oblique angle in degrees, `-85..=85`.
    pub oblique: Option<f64>,
}

/// Fields an update to one text style may change.
#[derive(Debug, Clone, Default)]
pub struct TextStyleUpdate {
    /// New style name; also becomes the style id.
    pub name: Option<String>,
    /// New primary font file or family name.
    pub font: Option<String>,
    /// New fixed cap height.
    pub height: Option<f64>,
    /// New horizontal width factor.
    pub width_factor: Option<f64>,
    /// New oblique angle in degrees.
    pub oblique: Option<f64>,
}

/// Lists the text styles of one drawing.
pub fn list_text_styles(handle: u64) -> Result<Value, ServiceError> {
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "textStyles": session.drawing.text_styles,
        })
    })
}

/// Adds one text style and returns `{ revision, drawing, style }`.
///
/// Names are unique case-insensitively; a duplicate reports
/// `drawing/style-exists`.
pub fn add_text_style(handle: u64, request: TextStyleRequest) -> Result<Value, ServiceError> {
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err(argument("a text style needs a non-empty name"));
    }
    let style = TextStyle {
        id: name.clone(),
        name,
        font: request.font.as_deref().map(str::trim).filter(|font| !font.is_empty()).unwrap_or("txt").to_string(),
        height: request.height.unwrap_or(0.0),
        width_factor: request.width_factor.unwrap_or(1.0),
        oblique: request.oblique.unwrap_or(0.0),
    };
    style.validate().map_err(argument)?;
    let id = style.id.clone();
    store::edit(handle, |drawing| {
        if drawing.text_styles.iter().any(|existing| existing.id.eq_ignore_ascii_case(&style.id)) {
            return Err(ServiceError::new("drawing/style-exists", format!("a text style named `{}` already exists", style.name)));
        }
        drawing.text_styles.push(style);
        drawing.revision += 1;
        Ok(())
    })?;
    read_text_style(handle, &id)
}

/// Updates one text style and returns `{ revision, drawing, style }`.
pub fn update_text_style(handle: u64, id: &str, update: TextStyleUpdate) -> Result<Value, ServiceError> {
    if update.name.is_none() && update.font.is_none() && update.height.is_none() && update.width_factor.is_none() && update.oblique.is_none() {
        return Err(argument("pass at least one text style field to update"));
    }
    let new_name = match &update.name {
        Some(name) => {
            let trimmed = name.trim().to_string();
            if trimmed.is_empty() {
                return Err(argument("a text style needs a non-empty name"));
            }
            Some(trimmed)
        }
        None => None,
    };
    let target = id.to_string();
    let next_id = new_name.clone().unwrap_or_else(|| target.clone());
    store::edit(handle, |drawing| {
        let index = drawing
            .text_styles
            .iter()
            .position(|style| style.id == target)
            .ok_or_else(|| ServiceError::new("object/not-found", format!("text style `{target}` does not exist")))?;
        if let Some(name) = &new_name
            && drawing.text_styles.iter().enumerate().any(|(other, style)| other != index && style.id.eq_ignore_ascii_case(name))
        {
            return Err(ServiceError::new("drawing/style-exists", format!("a text style named `{name}` already exists")));
        }
        let Some(style) = drawing.text_styles.get_mut(index)
        else {
            return Err(ServiceError::new("object/not-found", format!("text style `{target}` does not exist")));
        };
        if let Some(name) = new_name {
            style.id = name.clone();
            style.name = name;
        }
        if let Some(font) = update.font {
            style.font = font.trim().to_string();
        }
        if let Some(height) = update.height {
            style.height = height;
        }
        if let Some(width_factor) = update.width_factor {
            style.width_factor = width_factor;
        }
        if let Some(oblique) = update.oblique {
            style.oblique = oblique;
        }
        style.validate().map_err(argument)?;
        drawing.revision += 1;
        Ok(())
    })?;
    read_text_style(handle, &next_id)
}

/// Removes one text style and returns `{ revision, drawing, deleted }`.
///
/// Removing the last remaining style refuses with `drawing/style-last`.
pub fn delete_text_style(handle: u64, id: &str) -> Result<Value, ServiceError> {
    store::edit(handle, |drawing| {
        let index = drawing
            .text_styles
            .iter()
            .position(|style| style.id == id)
            .ok_or_else(|| ServiceError::new("object/not-found", format!("text style `{id}` does not exist")))?;
        if drawing.text_styles.len() <= 1 {
            return Err(ServiceError::new(
                "drawing/style-last",
                format!("text style `{id}` is the last style; a drawing must keep at least one style"),
            ));
        }
        drawing.text_styles.remove(index);
        drawing.revision += 1;
        Ok(())
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "deleted": id,
        })
    })
}

fn read_text_style(handle: u64, id: &str) -> Result<Value, ServiceError> {
    store::read(handle, |session| {
        let style = session.drawing.text_styles.iter().find(|style| style.id == id);
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "style": style,
        })
    })
}

/// Validates one line type dash pattern.
fn validate_linetype_pattern(pattern: &[f64]) -> Result<(), ServiceError> {
    if pattern.len() > LINETYPE_PATTERN_LIMIT {
        return Err(argument(format!("a line type pattern holds at most {LINETYPE_PATTERN_LIMIT} dashes")));
    }
    if pattern.iter().any(|dash| !dash.is_finite() || *dash < 0.0) {
        return Err(argument("line type dashes must be finite and non-negative"));
    }
    if !pattern.is_empty() && pattern.iter().all(|dash| *dash == 0.0) {
        return Err(argument("a line type pattern cannot be all zero dashes"));
    }
    Ok(())
}

/// Lists the line types of one drawing.
pub fn list_linetypes(handle: u64) -> Result<Value, ServiceError> {
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "linetypes": session.drawing.linetypes,
        })
    })
}

/// Fields used to create one line type.
#[derive(Debug, Clone, Default)]
pub struct LinetypeRequest {
    /// Line type name; it also becomes the id.
    pub name: String,
    /// Human-readable description.
    pub description: Option<String>,
    /// Dash lengths in drawing units; empty is a solid line.
    pub pattern: Option<Vec<f64>>,
}

/// Fields an update to one line type may change.
#[derive(Debug, Clone, Default)]
pub struct LinetypeUpdate {
    /// New name; it also becomes the id.
    pub name: Option<String>,
    /// New description.
    pub description: Option<String>,
    /// New dash pattern.
    pub pattern: Option<Vec<f64>>,
}

fn read_linetype(handle: u64, id: &str) -> Result<Value, ServiceError> {
    store::read(handle, |session| {
        let linetype = session.drawing.linetypes.iter().find(|linetype| linetype.id == id);
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "linetype": linetype,
        })
    })
}

/// Adds one line type and returns `{ revision, drawing, linetype }`.
pub fn add_linetype(handle: u64, request: LinetypeRequest) -> Result<Value, ServiceError> {
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err(argument("a line type needs a non-empty name"));
    }
    let description = request.description.map(|value| value.trim().to_string()).unwrap_or_default();
    let pattern = request.pattern.unwrap_or_default();
    validate_linetype_pattern(&pattern)?;
    store::edit(handle, |drawing| {
        if drawing.linetypes.iter().any(|linetype| linetype.id == name || linetype.name.eq_ignore_ascii_case(&name)) {
            return Err(ServiceError::new("drawing/linetype-exists", format!("a line type named `{name}` already exists")));
        }
        let linetype = Linetype { id: name.clone(), name: name.clone(), description, pattern };
        linetype.validate().map_err(argument)?;
        drawing.linetypes.push(linetype);
        drawing.revision += 1;
        Ok(())
    })?;
    read_linetype(handle, &name)
}

/// Updates one line type, renaming layer references when the name changes.
pub fn update_linetype(handle: u64, id: &str, update: LinetypeUpdate) -> Result<Value, ServiceError> {
    if update.name.is_none() && update.description.is_none() && update.pattern.is_none() {
        return Err(argument("pass at least one of `name`, `description` or `pattern`"));
    }
    let new_name = match update.name.as_deref() {
        Some(raw) => {
            let name = raw.trim().to_string();
            if name.is_empty() {
                return Err(argument("a line type needs a non-empty name"));
            }
            Some(name)
        }
        None => None,
    };
    if let Some(pattern) = &update.pattern {
        validate_linetype_pattern(pattern)?;
    }
    let target = id.to_string();
    let next_id = new_name.clone().unwrap_or_else(|| target.clone());
    store::edit(handle, |drawing| {
        let index = drawing
            .linetypes
            .iter()
            .position(|linetype| linetype.id == target)
            .ok_or_else(|| ServiceError::new("object/not-found", format!("line type `{target}` does not exist")))?;
        if let Some(name) = &new_name
            && drawing
                .linetypes
                .iter()
                .enumerate()
                .any(|(other, linetype)| other != index && (linetype.id == *name || linetype.name.eq_ignore_ascii_case(name)))
        {
            return Err(ServiceError::new("drawing/linetype-exists", format!("a line type named `{name}` already exists")));
        }
        let Some(linetype) = drawing.linetypes.get_mut(index)
        else {
            return Err(ServiceError::new("object/not-found", format!("line type `{target}` does not exist")));
        };
        if let Some(name) = new_name {
            linetype.id = name.clone();
            linetype.name = name;
        }
        if let Some(description) = update.description {
            linetype.description = description.trim().to_string();
        }
        if let Some(pattern) = update.pattern {
            linetype.pattern = pattern;
        }
        linetype.validate().map_err(argument)?;
        let renamed = linetype.id.clone();
        if renamed != target {
            for layer in &mut drawing.layers {
                if layer.linetype == target {
                    layer.linetype = renamed.clone();
                }
            }
        }
        drawing.revision += 1;
        Ok(())
    })?;
    read_linetype(handle, &next_id)
}

/// Removes one line type and returns `{ revision, drawing, deleted }`.
///
/// A line type any layer still references refuses with
/// `drawing/linetype-in-use`; removing the last line type refuses with
/// `drawing/linetype-last`.
pub fn delete_linetype(handle: u64, id: &str) -> Result<Value, ServiceError> {
    store::edit(handle, |drawing| {
        let index = drawing
            .linetypes
            .iter()
            .position(|linetype| linetype.id == id)
            .ok_or_else(|| ServiceError::new("object/not-found", format!("line type `{id}` does not exist")))?;
        if drawing.linetypes.len() <= 1 {
            return Err(ServiceError::new(
                "drawing/linetype-last",
                format!("line type `{id}` is the last one; a drawing must keep at least one line type"),
            ));
        }
        let count = drawing.layers.iter().filter(|layer| layer.linetype == id).count();
        if count > 0 {
            return Err(ServiceError::new("drawing/linetype-in-use", format!("line type `{id}` is referenced by {count} layer(s)")));
        }
        drawing.linetypes.remove(index);
        drawing.revision += 1;
        Ok(())
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "deleted": id,
        })
    })
}

/// Arguments of [`create_group`].
#[derive(Debug, Clone, Default)]
pub struct GroupCreateRequest {
    /// Group name; also becomes the group id.
    pub name: String,
    /// Member entity ids; repeated ids are kept once.
    pub entities: Vec<String>,
}

fn group_members(drawing: &Drawing, entities: &[String]) -> Result<Vec<String>, ServiceError> {
    let mut members = Vec::with_capacity(entities.len());
    let mut seen = std::collections::HashSet::new();
    for raw in entities {
        let id = raw.trim();
        if id.is_empty() {
            continue;
        }
        if drawing.entities.iter().all(|entity| entity.id != id) {
            return Err(ServiceError::new("object/not-found", format!("entity `{id}` does not exist")));
        }
        if seen.insert(id.to_string()) {
            members.push(id.to_string());
        }
    }
    Ok(members)
}

fn group_index(drawing: &Drawing, id: &str) -> Result<usize, ServiceError> {
    drawing
        .groups
        .iter()
        .position(|group| group.id == id)
        .ok_or_else(|| ServiceError::new("object/not-found", format!("entity group `{id}` does not exist")))
}

fn group_read(handle: u64, id: &str) -> Result<Value, ServiceError> {
    store::read(handle, |session| {
        let group = session.drawing.groups.iter().find(|group| group.id == id);
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "group": group,
        })
    })
}

fn detach_entity_from_groups(drawing: &mut Drawing, entity: &str) {
    for group in &mut drawing.groups {
        group.entities.retain(|member| member != entity);
    }
}

/// Lists the entity groups of one drawing.
pub fn list_groups(handle: u64) -> Result<Value, ServiceError> {
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "groups": session.drawing.groups,
        })
    })
}

/// Creates one entity group and returns `{ revision, drawing, group }`.
///
/// Names are unique case-insensitively and also become the group id. Every
/// member id must name an existing entity; repeated ids are kept once. A group
/// needs at least one member, and an existing name reports
/// `drawing/group-exists`.
pub fn create_group(handle: u64, request: GroupCreateRequest) -> Result<Value, ServiceError> {
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err(argument("an entity group needs a non-empty name"));
    }
    let group = store::edit(handle, |drawing| {
        if drawing.groups.iter().any(|existing| existing.id.eq_ignore_ascii_case(&name)) {
            return Err(ServiceError::new("drawing/group-exists", format!("an entity group named `{name}` already exists")));
        }
        let entities = group_members(drawing, &request.entities)?;
        if entities.is_empty() {
            return Err(argument("a group needs at least one entity"));
        }
        let group = EntityGroup { id: name.clone(), name: name.clone(), entities };
        drawing.groups.push(group.clone());
        drawing.revision += 1;
        Ok(group)
    })?;
    group_read(handle, &group.id)
}

/// Removes one entity group and returns `{ revision, drawing, deleted }`.
pub fn delete_group(handle: u64, id: &str) -> Result<Value, ServiceError> {
    store::edit(handle, |drawing| {
        let index = group_index(drawing, id)?;
        drawing.groups.remove(index);
        drawing.revision += 1;
        Ok(())
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "deleted": id,
        })
    })
}

/// Adds existing entities to one group and returns `{ revision, drawing, group }`.
///
/// Each id must name an existing entity; ids already in the group are ignored,
/// so an edit that adds nothing does not change the revision.
pub fn add_group_members(handle: u64, id: &str, entities: &[String]) -> Result<Value, ServiceError> {
    let target = id.to_string();
    let added = store::read(handle, |session| -> Result<usize, ServiceError> {
        let index = group_index(&session.drawing, &target)?;
        let members = group_members(&session.drawing, entities)?;
        Ok(members.iter().filter(|member| !session.drawing.groups[index].entities.contains(member)).count())
    })??;
    if added == 0 {
        return group_read(handle, &target);
    }
    store::edit(handle, |drawing| {
        let index = group_index(drawing, &target)?;
        for member in group_members(drawing, entities)? {
            if !drawing.groups[index].entities.contains(&member) {
                drawing.groups[index].entities.push(member);
            }
        }
        drawing.revision += 1;
        Ok(())
    })?;
    group_read(handle, &target)
}

/// Removes entities from one group and returns `{ revision, drawing, group }`.
///
/// Ids that are not members are ignored; removing nothing does not change the
/// revision. The group itself stays even when its last member is removed.
pub fn remove_group_members(handle: u64, id: &str, entities: &[String]) -> Result<Value, ServiceError> {
    let target = id.to_string();
    let removal = entities.iter().map(|entity| entity.trim()).filter(|entity| !entity.is_empty()).collect::<std::collections::HashSet<_>>();
    if removal.is_empty() {
        return Err(argument("pass at least one entity id to remove"));
    }
    let present = store::read(handle, |session| -> Result<bool, ServiceError> {
        let index = group_index(&session.drawing, &target)?;
        Ok(session.drawing.groups[index].entities.iter().any(|member| removal.contains(member.as_str())))
    })??;
    if !present {
        return group_read(handle, &target);
    }
    store::edit(handle, |drawing| {
        let index = group_index(drawing, &target)?;
        drawing.groups[index].entities.retain(|member| !removal.contains(member.as_str()));
        drawing.revision += 1;
        Ok(())
    })?;
    group_read(handle, &target)
}

/// Arguments of [`create_layout`].
#[derive(Debug, Clone, Default)]
pub struct LayoutRequest {
    /// Layout name; also becomes the layout id.
    pub name: String,
    /// Sheet width in millimetres; defaults to A3 landscape.
    pub width: Option<f64>,
    /// Sheet height in millimetres; defaults to A3 landscape.
    pub height: Option<f64>,
    /// Paper units per drawing unit; defaults to 1:1.
    pub scale: Option<f64>,
}

/// Fields an update to one layout may change.
#[derive(Debug, Clone, Default)]
pub struct LayoutUpdate {
    /// New layout name; also becomes the layout id.
    pub name: Option<String>,
    /// New sheet width in millimetres.
    pub width: Option<f64>,
    /// New sheet height in millimetres.
    pub height: Option<f64>,
    /// New paper units per drawing unit.
    pub scale: Option<f64>,
}

fn layout_index(drawing: &Drawing, id: &str) -> Result<usize, ServiceError> {
    drawing
        .layouts
        .iter()
        .position(|layout| layout.id == id)
        .ok_or_else(|| ServiceError::new("object/not-found", format!("layout `{id}` does not exist")))
}

/// One layout plus the view computed from the current model extents.
fn layout_json(drawing: &Drawing, layout: &Layout) -> Value {
    let (min, max, seen) = drawing_extents(drawing);
    let drawing_width = if seen { (max[0] - min[0]).max(0.0) } else { 0.0 };
    let drawing_height = if seen { (max[1] - min[1]).max(0.0) } else { 0.0 };
    let visible_width = layout.width / layout.scale;
    let visible_height = layout.height / layout.scale;
    let fits = !seen || (drawing_width <= visible_width + 1e-9 && drawing_height <= visible_height + 1e-9);
    let fit_scale = if drawing_width > 0.0 && drawing_height > 0.0 {
        (layout.width / drawing_width).min(layout.height / drawing_height)
    }
    else {
        layout.scale
    };
    json!({
        "id": layout.id,
        "name": layout.name,
        "width": layout.width,
        "height": layout.height,
        "scale": layout.scale,
        "view": {
            "center": [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0],
            "drawingWidth": drawing_width,
            "drawingHeight": drawing_height,
            "visibleWidth": visible_width,
            "visibleHeight": visible_height,
            "fits": fits,
            "fitScale": fit_scale,
        }
    })
}

fn layout_read(handle: u64, id: &str) -> Result<Value, ServiceError> {
    store::read(handle, |session| {
        let layout = session.drawing.layouts.iter().find(|layout| layout.id == id);
        let layout = layout.map(|layout| layout_json(&session.drawing, layout));
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "layout": layout,
        })
    })
}

/// Lists the paper-space layouts of one drawing with their computed view.
pub fn list_layouts(handle: u64) -> Result<Value, ServiceError> {
    store::read(handle, |session| {
        let layouts = session.drawing.layouts.iter().map(|layout| layout_json(&session.drawing, layout)).collect::<Vec<_>>();
        json!({
            "revision": session.drawing.revision,
            "layouts": layouts,
        })
    })
}

/// Creates one paper-space layout and returns `{ revision, drawing, layout }`.
///
/// Names are unique case-insensitively and also become the layout id. Omitted
/// sheet fields use an A3 landscape sheet at 1:1; a non-positive size or scale
/// reports `session/argument-invalid`, and an existing name reports
/// `drawing/layout-exists`.
pub fn create_layout(handle: u64, request: LayoutRequest) -> Result<Value, ServiceError> {
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err(argument("a layout needs a non-empty name"));
    }
    let mut layout = Layout::new(name);
    if let Some(width) = request.width {
        layout.width = width;
    }
    if let Some(height) = request.height {
        layout.height = height;
    }
    if let Some(scale) = request.scale {
        layout.scale = scale;
    }
    layout.validate().map_err(argument)?;
    let id = layout.id.clone();
    store::edit(handle, |drawing| {
        if drawing.layouts.iter().any(|existing| existing.id.eq_ignore_ascii_case(&layout.id)) {
            return Err(ServiceError::new("drawing/layout-exists", format!("a layout named `{}` already exists", layout.name)));
        }
        drawing.layouts.push(layout);
        drawing.revision += 1;
        Ok(())
    })?;
    layout_read(handle, &id)
}

/// Updates one paper-space layout and returns `{ revision, drawing, layout }`.
pub fn update_layout(handle: u64, id: &str, update: LayoutUpdate) -> Result<Value, ServiceError> {
    if update.name.is_none() && update.width.is_none() && update.height.is_none() && update.scale.is_none() {
        return Err(argument("pass at least one layout field to update"));
    }
    let new_name = match &update.name {
        Some(name) => {
            let trimmed = name.trim().to_string();
            if trimmed.is_empty() {
                return Err(argument("a layout needs a non-empty name"));
            }
            Some(trimmed)
        }
        None => None,
    };
    let target = new_name.clone().unwrap_or_else(|| id.to_string());
    store::edit(handle, |drawing| {
        let index = layout_index(drawing, id)?;
        let mut layout = drawing.layouts[index].clone();
        if let Some(name) = new_name {
            layout.id = name.clone();
            layout.name = name;
        }
        if let Some(width) = update.width {
            layout.width = width;
        }
        if let Some(height) = update.height {
            layout.height = height;
        }
        if let Some(scale) = update.scale {
            layout.scale = scale;
        }
        layout.validate().map_err(argument)?;
        if drawing.layouts.iter().enumerate().any(|(other, existing)| other != index && existing.id.eq_ignore_ascii_case(&layout.id)) {
            return Err(ServiceError::new("drawing/layout-exists", format!("a layout named `{}` already exists", layout.name)));
        }
        drawing.layouts[index] = layout;
        drawing.revision += 1;
        Ok(())
    })?;
    layout_read(handle, &target)
}

/// Removes one paper-space layout and returns `{ revision, drawing, deleted }`.
pub fn delete_layout(handle: u64, id: &str) -> Result<Value, ServiceError> {
    store::edit(handle, |drawing| {
        let index = layout_index(drawing, id)?;
        drawing.layouts.remove(index);
        drawing.revision += 1;
        Ok(())
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "deleted": id,
        })
    })
}

/// Converts a drawing between millimetres and inches.
///
/// Every coordinate, radius, ellipse axis, text height and dimension offset is
/// scaled about the origin by 25.4 (inch to mm) or 1 / 25.4 (mm to inch), and
/// the declared unit changes. A request for the unit the drawing already uses
/// changes nothing and does not add a history step. Explicit dimension labels
/// are kept as text, so the `units` check still reports a label that conflicts.
/// The response carries `{ revision, drawing, from, to, factor, converted }`.
pub fn convert_units(handle: u64, unit: &str) -> Result<Value, ServiceError> {
    let target = Units::parse(unit).ok_or_else(|| argument(format!("`{unit}` is not a drawing unit; use `mm` or `inch`")))?;
    let current = store::read(handle, |session| session.drawing.units)?;
    let factor = match (current, target) {
        (Units::Mm, Units::Inch) => 1.0 / 25.4,
        (Units::Inch, Units::Mm) => 25.4,
        _ => 1.0,
    };
    let converted = if current != target {
        store::edit(handle, |drawing| {
            for entity in &mut drawing.entities {
                apply_transform(entity, TransformOp::Scale { factor, cx: 0.0, cy: 0.0 });
            }
            drawing.units = target;
            drawing.revision += 1;
            Ok(())
        })?;
        true
    }
    else {
        false
    };
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "from": current.as_str(),
            "to": target.as_str(),
            "factor": factor,
            "converted": converted,
        })
    })
}

/// Lists every entity of one drawing.
pub fn list_entities(handle: u64) -> Result<Value, ServiceError> {
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "entities": session.drawing.entities,
        })
    })
}

/// Where [`reorder_entity`] moves one entity in the paint order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawOrderAction {
    /// Paint the entity after every other one.
    ToFront,
    /// Paint the entity before every other one.
    ToBack,
    /// Paint the entity one step later.
    Forward,
    /// Paint the entity one step earlier.
    Backward,
}

impl DrawOrderAction {
    /// Stable lowercase name used in JSON and the CLI.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ToFront => "to-front",
            Self::ToBack => "to-back",
            Self::Forward => "forward",
            Self::Backward => "backward",
        }
    }

    /// Parses an action name.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "to-front" | "top" | "front" => Some(Self::ToFront),
            "to-back" | "bottom" | "back" => Some(Self::ToBack),
            "forward" | "up" | "raise" => Some(Self::Forward),
            "backward" | "down" | "lower" => Some(Self::Backward),
            _ => None,
        }
    }
}

/// Moves one entity within the paint order.
///
/// The paint order runs back to front, so the last entity is drawn on top.
/// `to-front` and `to-back` move the entity to the end or the start of the
/// order, while `forward` and `backward` move it one step towards the end or
/// the start. An unknown id reports `object/not-found`. The response carries
/// `{ revision, drawing, entity, action, order }`, where `order` is the full
/// entity id sequence from back to front.
pub fn reorder_entity(handle: u64, entity: &str, action: DrawOrderAction) -> Result<Value, ServiceError> {
    let order = store::edit(handle, |drawing| {
        entity_index(drawing, entity)?;
        let mut order = drawing.ordered_entity_ids();
        let position = order
            .iter()
            .position(|id| id == entity)
            .ok_or_else(|| ServiceError::new("object/not-found", format!("entity `{entity}` does not exist")))?;
        let target = match action {
            DrawOrderAction::ToFront => order.len().saturating_sub(1),
            DrawOrderAction::ToBack => 0,
            DrawOrderAction::Forward => (position + 1).min(order.len().saturating_sub(1)),
            DrawOrderAction::Backward => position.saturating_sub(1),
        };
        if target != position {
            let id = order.remove(position);
            order.insert(target, id);
        }
        drawing.draw_order = order.clone();
        drawing.revision += 1;
        Ok(order)
    })?;
    store::read(handle, |session| {
        let entity = session.drawing.entities.iter().find(|candidate| candidate.id == entity);
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "entity": entity,
            "action": action.as_str(),
            "order": order,
        })
    })
}

/// How a rectangular selection treats entities that touch its boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionMode {
    /// Select only entities whose whole outline lies inside the rectangle.
    Window,
    /// Select entities inside the rectangle or crossing its boundary.
    Crossing,
}

impl SelectionMode {
    /// Stable lowercase name used in JSON and the CLI.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Window => "window",
            Self::Crossing => "crossing",
        }
    }
}

/// Selects entities with a rectangular window or crossing box.
///
/// A `window` selection keeps entities whose complete outline lies inside the
/// rectangle; a `crossing` selection also keeps entities whose outline crosses
/// its boundary. Curves are sampled at [`CURVE_STEP_DEGREES`], so a window test
/// is exact to that step. `layer` restricts the result to one layer id. The
/// query does not mutate the drawing and adds no history step; hidden and
/// locked layers remain selectable.
pub fn select(handle: u64, mode: SelectionMode, window: [[f64; 2]; 2], layer: Option<&str>) -> Result<Value, ServiceError> {
    if !finite_point(window[0]) || !finite_point(window[1]) {
        return Err(argument("the selection window needs two finite corners"));
    }
    let rect = SelectionRect::new(window[0], window[1]);
    if rect.width() <= 0.0 && rect.height() <= 0.0 {
        return Err(argument("the selection window needs a non-zero width or height"));
    }
    let layer = layer.map(str::trim).filter(|id| !id.is_empty());
    store::read(handle, |session| {
        if let Some(id) = layer
            && session.drawing.layer(id).is_none()
        {
            return Err(ServiceError::new("object/not-found", format!("layer `{id}` does not exist")));
        }
        let ids = session
            .drawing
            .entities
            .iter()
            .filter(|entity| layer.is_none_or(|id| entity.layer == id))
            .filter(|entity| entity_matches_selection(&entity.kind, mode, rect))
            .map(|entity| entity.id.clone())
            .collect::<Vec<_>>();
        Ok(json!({
            "revision": session.drawing.revision,
            "mode": mode.as_str(),
            "window": window,
            "layer": layer,
            "count": ids.len(),
            "ids": ids,
        }))
    })?
}

/// Axis-aligned rectangle normalised from two corners.
#[derive(Debug, Clone, Copy)]
struct SelectionRect {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}

impl SelectionRect {
    fn new(a: [f64; 2], b: [f64; 2]) -> Self {
        Self { min_x: a[0].min(b[0]), min_y: a[1].min(b[1]), max_x: a[0].max(b[0]), max_y: a[1].max(b[1]) }
    }

    fn width(&self) -> f64 {
        self.max_x - self.min_x
    }

    fn height(&self) -> f64 {
        self.max_y - self.min_y
    }

    fn contains(&self, point: [f64; 2]) -> bool {
        point[0] >= self.min_x && point[0] <= self.max_x && point[1] >= self.min_y && point[1] <= self.max_y
    }

    fn corners(&self) -> [[f64; 2]; 4] {
        [[self.min_x, self.min_y], [self.max_x, self.min_y], [self.max_x, self.max_y], [self.min_x, self.max_y]]
    }
}

/// Whether one entity belongs to a selection rectangle.
fn entity_matches_selection(kind: &EntityKind, mode: SelectionMode, rect: SelectionRect) -> bool {
    let (points, closed) = entity_outline(kind);
    if points.is_empty() {
        return false;
    }
    match mode {
        SelectionMode::Window => points.iter().all(|point| rect.contains(*point)),
        SelectionMode::Crossing => points.iter().any(|point| rect.contains(*point)) || outline_crosses_rect(&points, closed, rect),
    }
}

/// Outline of one entity, in drawing order, plus whether it closes back.
fn entity_outline(kind: &EntityKind) -> (Vec<[f64; 2]>, bool) {
    match kind {
        EntityKind::Line { from, to } => (vec![*from, *to], false),
        EntityKind::Circle { center, radius } => (sample_arc(*center, radius.abs(), 0.0, 360.0), false),
        EntityKind::Arc { center, radius, start_angle, end_angle } => (sample_arc(*center, radius.abs(), *start_angle, *end_angle), false),
        EntityKind::Ellipse { center, rx, ry, rotation, start_angle, end_angle } => {
            (sample_ellipse(*center, rx.abs(), ry.abs(), *rotation, *start_angle, *end_angle), false)
        }
        EntityKind::Point { at } => (vec![*at], false),
        EntityKind::Polyline { points, closed } => (points.clone(), *closed),
        EntityKind::Text { at, height, content } => {
            let size = height.abs();
            let width = size * 0.6 * content.chars().count() as f64;
            (vec![*at, [at[0] + width, at[1]], [at[0] + width, at[1] + size], [at[0], at[1] + size]], true)
        }
        EntityKind::Dimension { from, to, offset, .. } => {
            let (p1, p2) = offset_points(*from, *to, *offset);
            (vec![*from, p1, p2, *to], false)
        }
        EntityKind::RadialDimension { center, radius, angle, offset, .. } => {
            (vec![*center, radial_target(*center, *radius, *angle), radial_label(*center, *radius, *angle, *offset)], false)
        }
        EntityKind::OrdinateDimension { origin, feature, axis, offset, .. } => {
            let (base, guide) = ordinate_points(*origin, *feature, *axis, *offset);
            (vec![*feature, guide, base, *origin], false)
        }
        EntityKind::AngularDimension { center, first, second, offset, .. } => {
            let (start, sweep) = angular_sweep(*center, *first, *second);
            let mut points = vec![*first];
            points.extend(sample_arc(*center, offset.abs(), start, start + sweep));
            points.push(*second);
            (points, false)
        }
        EntityKind::Hatch { points, .. } => (points.clone(), true),
    }
}

/// Whether any outline edge crosses one rectangle edge.
fn outline_crosses_rect(points: &[[f64; 2]], closed: bool, rect: SelectionRect) -> bool {
    let corners = rect.corners();
    let mut edges = points.windows(2).map(|pair| (pair[0], pair[1])).collect::<Vec<_>>();
    if closed && points.len() >= 2 {
        edges.push((points[points.len() - 1], points[0]));
    }
    edges.iter().any(|(from, to)| (0..4).any(|index| segments_intersect(*from, *to, corners[index], corners[(index + 1) % 4])))
}

/// Twice the signed area of the triangle `a, b, c`.
fn orientation(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Whether two closed segments `a -> b` and `c -> d` intersect, touching included.
fn segments_intersect(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let o1 = orientation(a, b, c);
    let o2 = orientation(a, b, d);
    let o3 = orientation(c, d, a);
    let o4 = orientation(c, d, b);
    let opposite = |left: f64, right: f64| (left > 0.0 && right < 0.0) || (left < 0.0 && right > 0.0);
    if opposite(o1, o2) && opposite(o3, o4) {
        return true;
    }
    let zero = |value: f64| value.abs() < 1e-12;
    (zero(o1) && on_segment(a, b, c))
        || (zero(o2) && on_segment(a, b, d))
        || (zero(o3) && on_segment(c, d, a))
        || (zero(o4) && on_segment(c, d, b))
}

/// Whether a collinear point lies within the bounding box of a segment.
fn on_segment(a: [f64; 2], b: [f64; 2], point: [f64; 2]) -> bool {
    point[0] >= a[0].min(b[0]) && point[0] <= a[0].max(b[0]) && point[1] >= a[1].min(b[1]) && point[1] <= a[1].max(b[1])
}

/// Snap modes [`snap_points`] accepts.
pub const SNAP_MODES: &[&str] = &["endpoint", "midpoint", "center", "intersection"];

/// Most snap candidates one query returns.
const SNAP_LIMIT: usize = 1000;

/// Arguments of [`snap_points`].
#[derive(Debug, Clone, Default)]
pub struct SnapRequest {
    /// Modes to compute; an empty list uses every mode.
    pub modes: Vec<String>,
    /// Point the caller is probing; candidates are sorted by distance to it.
    pub point: Option<[f64; 2]>,
    /// Maximum distance from `point`; requires `point` and must be positive.
    pub tolerance: Option<f64>,
    /// Restrict the query to one layer id.
    pub layer: Option<String>,
    /// Restrict the query to these entity ids.
    pub ids: Option<Vec<String>>,
}

/// One computed snap candidate.
#[derive(Debug, Clone)]
struct SnapCandidate {
    kind: &'static str,
    point: [f64; 2],
    entities: Vec<String>,
}

impl SnapCandidate {
    fn new(kind: &'static str, point: [f64; 2], entity: &str) -> Self {
        Self { kind, point, entities: vec![entity.to_string()] }
    }
}

/// Adds `point` to `out` unless the same entity already reported it.
fn push_snap(out: &mut Vec<SnapCandidate>, kind: &'static str, entity: &str, point: [f64; 2]) {
    if out
        .iter()
        .any(|candidate| candidate.kind == kind && candidate.entities.first().is_some_and(|id| id == entity) && candidate.point == point)
    {
        return;
    }
    out.push(SnapCandidate::new(kind, point, entity));
}

/// Whether a segment intersection parameter lies on the segment.
fn within_segment(parameter: f64) -> bool {
    (-1e-9..=1.0 + 1e-9).contains(&parameter)
}

/// Intersection of two bounded segments, or `None` when they do not meet.
fn segment_intersection(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> Option<[f64; 2]> {
    let first = [b[0] - a[0], b[1] - a[1]];
    let second = [d[0] - c[0], d[1] - c[1]];
    let denominator = first[0] * second[1] - first[1] * second[0];
    if denominator.abs() < 1e-12 {
        return None;
    }
    let offset = [c[0] - a[0], c[1] - a[1]];
    let t = (offset[0] * second[1] - offset[1] * second[0]) / denominator;
    let u = (offset[0] * first[1] - offset[1] * first[0]) / denominator;
    if !within_segment(t) || !within_segment(u) {
        return None;
    }
    Some([a[0] + first[0] * t, a[1] + first[1] * t])
}

/// Midpoint of one polyline edge.
fn edge_midpoint(from: [f64; 2], to: [f64; 2]) -> [f64; 2] {
    [(from[0] + to[0]) * 0.5, (from[1] + to[1]) * 0.5]
}

/// Collects the endpoints, midpoints and centres one entity contributes.
fn entity_snap_points(entity: &Entity, modes: &[&str], out: &mut Vec<SnapCandidate>) {
    let endpoint = modes.contains(&"endpoint");
    let midpoint = modes.contains(&"midpoint");
    let center = modes.contains(&"center");
    match &entity.kind {
        EntityKind::Line { from, to } => {
            if endpoint {
                push_snap(out, "endpoint", &entity.id, *from);
                push_snap(out, "endpoint", &entity.id, *to);
            }
            if midpoint {
                push_snap(out, "midpoint", &entity.id, edge_midpoint(*from, *to));
            }
        }
        EntityKind::Circle { center: at, .. } => {
            if center {
                push_snap(out, "center", &entity.id, *at);
            }
        }
        EntityKind::Arc { center: at, radius, start_angle, end_angle } => {
            let sweep = arc_sweep(*start_angle, *end_angle);
            if endpoint {
                push_snap(out, "endpoint", &entity.id, arc_point(*at, radius.abs(), *start_angle));
                push_snap(out, "endpoint", &entity.id, arc_point(*at, radius.abs(), *start_angle + sweep));
            }
            if midpoint {
                push_snap(out, "midpoint", &entity.id, arc_point(*at, radius.abs(), *start_angle + sweep * 0.5));
            }
            if center {
                push_snap(out, "center", &entity.id, *at);
            }
        }
        EntityKind::Ellipse { center: at, rx, ry, rotation, start_angle, end_angle } => {
            let sweep = arc_sweep(*start_angle, *end_angle);
            if endpoint {
                push_snap(out, "endpoint", &entity.id, ellipse_point(*at, rx.abs(), ry.abs(), *rotation, *start_angle));
                push_snap(out, "endpoint", &entity.id, ellipse_point(*at, rx.abs(), ry.abs(), *rotation, *start_angle + sweep));
            }
            if midpoint {
                push_snap(out, "midpoint", &entity.id, ellipse_point(*at, rx.abs(), ry.abs(), *rotation, *start_angle + sweep * 0.5));
            }
            if center {
                push_snap(out, "center", &entity.id, *at);
            }
        }
        EntityKind::Point { at } => {
            if endpoint {
                push_snap(out, "endpoint", &entity.id, *at);
            }
        }
        EntityKind::Polyline { points, closed } => {
            if endpoint {
                for point in points {
                    push_snap(out, "endpoint", &entity.id, *point);
                }
            }
            if midpoint {
                for pair in points.windows(2) {
                    push_snap(out, "midpoint", &entity.id, edge_midpoint(pair[0], pair[1]));
                }
                if *closed
                    && let (Some(first), Some(last)) = (points.first(), points.last())
                    && points.len() > 2
                {
                    push_snap(out, "midpoint", &entity.id, edge_midpoint(*last, *first));
                }
            }
        }
        EntityKind::Hatch { points, .. } => {
            if endpoint {
                for point in points {
                    push_snap(out, "endpoint", &entity.id, *point);
                }
            }
            if midpoint {
                for index in 0..points.len() {
                    let next = (index + 1) % points.len();
                    push_snap(out, "midpoint", &entity.id, edge_midpoint(points[index], points[next]));
                }
            }
        }
        EntityKind::AngularDimension { center: vertex, first, second, offset, .. } => {
            if endpoint {
                push_snap(out, "endpoint", &entity.id, *first);
                push_snap(out, "endpoint", &entity.id, *second);
            }
            if center {
                push_snap(out, "center", &entity.id, *vertex);
            }
            if midpoint {
                let (start, sweep) = angular_sweep(*vertex, *first, *second);
                push_snap(out, "midpoint", &entity.id, arc_point(*vertex, offset.abs(), start + sweep / 2.0));
            }
        }
        EntityKind::Text { .. } | EntityKind::Dimension { .. } | EntityKind::RadialDimension { .. } | EntityKind::OrdinateDimension { .. } => {}
    }
}

/// Computes snap candidates (endpoints, midpoints, centres and line
/// intersections) without changing the drawing.
///
/// `modes` defaults to every mode. `point` sorts candidates by distance and
/// `tolerance` keeps only candidates within that distance; a tolerance without
/// a point is refused. `ids` and `layer` narrow the entity set, and an unknown
/// id or layer reports `object/not-found`. The response carries
/// `{ revision, modes, point, tolerance, count, truncated, points }`, where each
/// point is `{ kind, point, entities, distance? }`.
pub fn snap_points(handle: u64, request: SnapRequest) -> Result<Value, ServiceError> {
    let mut selected_modes: Vec<&'static str> = Vec::new();
    if request.modes.is_empty() {
        selected_modes.extend_from_slice(SNAP_MODES);
    }
    else {
        for raw in &request.modes {
            let mode = raw.trim().to_ascii_lowercase();
            let Some(mode) = SNAP_MODES.iter().copied().find(|candidate| *candidate == mode)
            else {
                return Err(argument(format!("`{raw}` is not a snap mode; use endpoint, midpoint, center or intersection")));
            };
            if !selected_modes.contains(&mode) {
                selected_modes.push(mode);
            }
        }
    }
    if let Some(point) = request.point
        && !finite_point(point)
    {
        return Err(argument("the snap point needs two finite coordinates"));
    }
    if let Some(tolerance) = request.tolerance {
        if request.point.is_none() {
            return Err(argument("a snap tolerance needs a point to measure from"));
        }
        if !tolerance.is_finite() || tolerance <= 0.0 {
            return Err(argument("the snap tolerance must be a positive finite distance"));
        }
    }
    let layer = request.layer.as_deref().map(str::trim).filter(|id| !id.is_empty());
    let ids = request
        .ids
        .as_deref()
        .map(|ids| ids.iter().map(|id| id.trim()).filter(|id| !id.is_empty()).map(str::to_string).collect::<std::collections::HashSet<_>>());
    if ids.as_ref().is_some_and(std::collections::HashSet::is_empty) {
        return Err(argument("pass at least one entity id"));
    }
    store::read(handle, |session| {
        let drawing = &session.drawing;
        if let Some(id) = layer
            && drawing.layer(id).is_none()
        {
            return Err(ServiceError::new("object/not-found", format!("layer `{id}` does not exist")));
        }
        if let Some(ids) = &ids {
            for id in ids {
                if !drawing.entities.iter().any(|entity| entity.id == *id) {
                    return Err(ServiceError::new("object/not-found", format!("entity `{id}` does not exist")));
                }
            }
        }
        let selected = drawing
            .entities
            .iter()
            .filter(|entity| layer.is_none_or(|id| entity.layer == id))
            .filter(|entity| ids.as_ref().is_none_or(|ids| ids.contains(&entity.id)))
            .collect::<Vec<_>>();

        let mut candidates = Vec::new();
        for entity in &selected {
            entity_snap_points(entity, &selected_modes, &mut candidates);
        }
        if selected_modes.contains(&"intersection") {
            for (index, first) in selected.iter().enumerate() {
                let Some((first_from, first_to)) = line_endpoints(&first.kind)
                else {
                    continue;
                };
                for second in selected.iter().skip(index + 1) {
                    let Some((second_from, second_to)) = line_endpoints(&second.kind)
                    else {
                        continue;
                    };
                    let Some(point) = segment_intersection(first_from, first_to, second_from, second_to)
                    else {
                        continue;
                    };
                    candidates.push(SnapCandidate { kind: "intersection", point, entities: vec![first.id.clone(), second.id.clone()] });
                }
            }
        }

        let mut points = if let Some(origin) = request.point {
            let tolerance = request.tolerance;
            candidates.retain(|candidate| tolerance.is_none_or(|limit| point_distance(origin, candidate.point) <= limit));
            candidates.sort_by(|a, b| {
                point_distance(origin, a.point)
                    .partial_cmp(&point_distance(origin, b.point))
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.kind.cmp(b.kind))
                    .then_with(|| a.entities.cmp(&b.entities))
            });
            candidates
                .into_iter()
                .map(|candidate| {
                    json!({
                        "kind": candidate.kind,
                        "point": candidate.point,
                        "entities": candidate.entities,
                        "distance": point_distance(origin, candidate.point),
                    })
                })
                .collect::<Vec<_>>()
        }
        else {
            candidates
                .into_iter()
                .map(|candidate| {
                    json!({
                        "kind": candidate.kind,
                        "point": candidate.point,
                        "entities": candidate.entities,
                    })
                })
                .collect::<Vec<_>>()
        };
        let truncated = points.len() > SNAP_LIMIT;
        points.truncate(SNAP_LIMIT);
        Ok(json!({
            "revision": drawing.revision,
            "modes": selected_modes,
            "point": request.point,
            "tolerance": request.tolerance,
            "count": points.len(),
            "truncated": truncated,
            "points": points,
        }))
    })?
}

/// Imports the declared ASCII DXF subset and returns `{ handle, drawing, ... }`.
///
/// The reader keeps the HEADER `$INSUNITS` value and the LAYER table, expands
/// INSERT references from BLOCKS, and reads LINE, CIRCLE, ARC, ELLIPSE, POINT,
/// TEXT, LWPOLYLINE (with bulge), POLYLINE/VERTEX/SEQEND (including a 3D
/// polyline projected onto the XY plane), SPLINE (from fit or control points)
/// and LEADER. The result carries `losses` with `{ code, severity, detail }`:
/// `notice` marks an approximation such as a flattened arc or a spline
/// polyline, `warning` marks content the drawing does not carry.
pub fn import_dxf(source: &Path, title: Option<String>) -> Result<Value, ServiceError> {
    let bytes = fs::read(source).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            ServiceError::new("drawing/not-found", format!("`{}` does not exist", source.display()))
        }
        else {
            ServiceError::new("drawing/io-error", format!("could not read `{}`: {error}", source.display()))
        }
    })?;
    let text = String::from_utf8(bytes).map_err(|_| invalid(format!("`{}` is not UTF-8 text", source.display())))?;
    let title =
        title.filter(|value| !value.trim().is_empty()).or_else(|| source.file_stem().and_then(|stem| stem.to_str()).map(str::to_string));
    let (drawing, losses) = import::dxf(&text, title)?;
    drawing.validate().map_err(|message| invalid(format!("`{}` imports an invalid drawing: {message}", source.display())))?;
    let imported = json!({ "entities": drawing.entities.len(), "layers": drawing.layers.len() });
    let handle = store::insert(drawing);
    store::read(handle, |session| {
        json!({
            "handle": handle,
            "source": source.display().to_string(),
            "drawing": session.summary(),
            "imported": imported,
            "losses": losses,
        })
    })
}

#[cfg(test)]
mod tests {
    use std::{
        env, fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = env::temp_dir().join(format!("open-cad-{name}-{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::SeqCst)));
            fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn handle_of(value: &Value) -> u64 {
        value["handle"].as_u64().expect("handle")
    }

    fn drawing_of(handle: u64) -> Drawing {
        store::read(handle, |session| session.drawing.clone()).expect("read drawing")
    }

    #[test]
    fn capability_matrix_matches_the_implemented_surface() {
        let matrix = capabilities();

        assert_eq!(matrix["project"], json!("partial"));
        assert_eq!(matrix["formats"]["svg"], json!("supported"));
        assert_eq!(matrix["formats"]["pdf"], json!("supported"));
        assert_eq!(matrix["formats"]["dxf"], json!("partial"));
        assert_eq!(matrix["formats"]["step"], json!("partial"), "STEP is export-only, so the format is partial");
        assert_eq!(matrix["formats"]["obj"], json!("partial"), "OBJ is export-only, so the format is partial");
        assert_eq!(matrix["formats"]["dwg"], json!("unsupported"));
        for check in ["units", "layers", "empty-layer", "geometry", "dimensions", "duplicate-entity", "zero-length", "open-contour"] {
            assert_eq!(matrix["checks"][check], json!("supported"), "check `{check}`");
        }
        for check in ["constraints", "references"] {
            assert_eq!(matrix["checks"][check], json!("unsupported"), "check `{check}`");
        }
        for operation in [
            "transform",
            "copy",
            "array",
            "explode",
            "offset",
            "fillet",
            "chamfer",
            "measure",
            "measure-points",
            "stats",
            "dimension-style",
            "text-style",
            "select",
            "ucs",
        ] {
            assert_eq!(matrix["operations"][operation], json!("supported"), "operation `{operation}`");
        }
        for operation in ["constraints", "kernel3d"] {
            assert_eq!(matrix["operations"][operation], json!("unsupported"), "operation `{operation}`");
        }
        for entity in ["radial-dimension", "ordinate-dimension", "angular-dimension", "hatch"] {
            assert_eq!(matrix["entities"][entity], json!("supported"), "entity `{entity}`");
        }
        assert_eq!(matrix["renderers"]["svg"], json!("supported"));
        for renderer in ["png", "pdf"] {
            assert_eq!(matrix["renderers"][renderer], json!("unsupported"), "renderer `{renderer}`");
        }
        assert_eq!(matrix["exportTargets"]["svg"], json!("supported"));
        assert_eq!(matrix["exportTargets"]["dxf"], json!("supported"));
        assert_eq!(matrix["exportTargets"]["pdf"], json!("supported"));
        assert_eq!(matrix["exportTargets"]["stl"], json!("supported"));
        assert_eq!(matrix["exportTargets"]["obj"], json!("supported"));
        assert_eq!(matrix["exportTargets"]["step"], json!("supported"));
        assert_eq!(matrix["formats"]["stl"], json!("unsupported"), "the STL reader stays unsupported");
        assert_eq!(matrix["formats"]["dwg"], json!("unsupported"));
    }

    #[test]
    fn create_add_save_open_round_trip() {
        let dir = TempDir::new("round-trip");
        let path = dir.path.join("plan.ocad");

        let created = create(CreateRequest { title: Some("Site plan".to_string()), units: Some("mm".to_string()), ..CreateRequest::default() })
            .expect("create");
        let handle = handle_of(&created);
        assert_eq!(created["drawing"]["units"], json!("mm"));
        assert_eq!(created["drawing"]["layers"].as_array().map(Vec::len), Some(2));

        let line = add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [100.0, 40.0]).expect("add line");
        assert_eq!(line["entity"]["id"], json!("entity-1"));
        let circle = add_circle(handle, DEFAULT_LAYER, [50.0, 50.0], 20.0).expect("add circle");
        assert_eq!(circle["entity"]["id"], json!("entity-2"));
        add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]], true).expect("add polyline");
        add_text(handle, DEFAULT_LAYER, [5.0, 5.0], 2.5, "Site").expect("add text");
        add_dimension(handle, DIMENSION_LAYER, [0.0, 0.0], [100.0, 0.0], 10.0, None).expect("add dimension");

        let listed = list_entities(handle).expect("list");
        assert_eq!(listed["entities"].as_array().map(Vec::len), Some(5));

        let before = summary(handle).expect("summary");
        let bytes = save(handle, &path, true).expect("save");
        assert!(bytes > 0);
        assert!(path.is_file());
        let saved_text = fs::read_to_string(&path).expect("read saved");
        assert!(saved_text.contains("\"format\": \"open-cad-drawing\""));
        assert!(saved_text.contains("\"version\": 1"));
        assert!(saved_text.contains("\"next_entity\""));
        close(handle).expect("close");

        let opened = open(&path).expect("open");
        assert_eq!(opened["drawing"]["revision"], before["revision"]);
        assert_eq!(opened["drawing"]["entities"], json!(5));
        let reopened = handle_of(&opened);

        let report = inspect(reopened, None).expect("inspect");
        assert_eq!(report["passed"], json!(true));
        assert_eq!(report["issues"].as_array().map(Vec::len), Some(0));
        assert!(report["checkerVersion"].is_string());

        let svg_path = dir.path.join("plan.svg");
        let svg_report = export(reopened, "svg", &svg_path).expect("export svg");
        assert_eq!(svg_report["format"], json!("svg"));
        assert_eq!(svg_report["editable"], json!(false));
        let svg = fs::read_to_string(&svg_path).expect("read svg");
        assert!(svg.contains("<svg"));
        assert!(svg.contains("viewBox="));
        assert!(svg.contains("data-revision="));

        let dxf_path = dir.path.join("plan.dxf");
        let dxf_report = export(reopened, "dxf", &dxf_path).expect("export dxf");
        assert_eq!(dxf_report["format"], json!("dxf"));
        assert!(dxf_report["losses"].as_array().is_some_and(|losses| !losses.is_empty()));
        let dxf = fs::read_to_string(&dxf_path).expect("read dxf");
        assert!(dxf.contains("HEADER"));
        assert!(dxf.contains("$INSUNITS"));
        assert!(dxf.contains("ENTITIES"));
        assert!(dxf.trim_end().ends_with("EOF"));

        let deleted = delete_entity(reopened, "entity-1").expect("delete");
        assert_eq!(deleted["deleted"], json!("entity-1"));
        assert_eq!(deleted["drawing"]["entities"], json!(4));
        assert!(delete_entity(reopened, "entity-1").is_err());

        close(reopened).expect("close reopened");
    }

    #[test]
    fn save_refuses_to_overwrite_without_the_flag() {
        let dir = TempDir::new("overwrite");
        let path = dir.path.join("plan.ocad");
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        save(handle, &path, false).expect("first save");
        let error = save(handle, &path, false).expect_err("second save without overwrite");
        assert_eq!(error.code, "drawing/overwrite-refused");
        save(handle, &path, true).expect("overwrite");
        close(handle).expect("close");
    }

    #[test]
    fn inspection_reports_real_findings() {
        let drawing: Drawing = serde_json::from_value(json!({
            "id": "drawing-1",
            "title": null,
            "revision": 3,
            "units": "mm",
            "layers": [
                { "id": "0", "name": "0", "color": "#d0d7de", "visible": true, "locked": false },
                { "id": "locked", "name": "locked", "color": "#0b7285", "visible": true, "locked": true }
            ],
            "entities": [
                { "id": "entity-1", "layer": "0", "kind": { "type": "line", "from": [0.0, 0.0], "to": [0.0, 0.0] } },
                { "id": "entity-1", "layer": "missing", "kind": { "type": "circle", "center": [0.0, 0.0], "radius": 0.0 } },
                { "id": "entity-3", "layer": "0", "kind": { "type": "polyline", "points": [[0.0, 0.0]], "closed": false } },
                { "id": "entity-4", "layer": "0", "kind": { "type": "dimension", "from": [1.0, 1.0], "to": [1.0, 1.0], "offset": 5.0, "text": null } },
                { "id": "entity-5", "layer": "locked", "kind": { "type": "text", "at": [0.0, 0.0], "height": 1.0, "content": "x" } }
            ],
            "next_entity": 6
        }))
        .expect("parse drawing");

        let report = inspection::run(&drawing, inspection::SUPPORTED_CHECKS);
        let codes = report.to_json()["issues"]
            .as_array()
            .expect("issues")
            .iter()
            .map(|issue| issue["code"].as_str().unwrap_or_default().to_string())
            .collect::<Vec<_>>();

        for expected in [
            "geometry/duplicate-id",
            "layers/missing-layer",
            "geometry/zero-length-line",
            "geometry/invalid-radius",
            "geometry/insufficient-points",
            "dimensions/degenerate",
            "layers/locked",
        ] {
            assert!(codes.iter().any(|code| code == expected), "expected `{expected}` in {codes:?}");
        }
        assert_eq!(report.to_json()["passed"], json!(false));
        let locked = report.to_json()["issues"]
            .as_array()
            .expect("issues")
            .iter()
            .find(|issue| issue["code"] == json!("layers/locked"))
            .cloned()
            .expect("locked finding");
        assert_eq!(locked["severity"], json!("warning"));
        assert_eq!(locked["recoverable"], json!(true));
        assert_eq!(locked["layerId"], json!("locked"));
        assert_eq!(locked["objectId"], json!("entity-5"));

        let dimension_only = inspection::run(&drawing, &["dimensions"]).to_json();
        let dimension_codes = dimension_only["issues"]
            .as_array()
            .expect("issues")
            .iter()
            .map(|issue| issue["code"].as_str().unwrap_or_default())
            .collect::<Vec<_>>();
        assert_eq!(dimension_codes, vec!["dimensions/degenerate"]);

        let mut non_finite = Drawing::empty(None, Units::Mm);
        non_finite.entities.push(Entity {
            id: "entity-nan".to_string(),
            layer: DEFAULT_LAYER.to_string(),
            kind: EntityKind::Line { from: [f64::NAN, 0.0], to: [1.0, 1.0] },
        });
        let report = inspection::run(&non_finite, inspection::SUPPORTED_CHECKS).to_json();
        assert!(report["issues"].as_array().expect("issues").iter().any(|issue| issue["code"] == json!("geometry/non-finite")));
    }

    #[test]
    fn open_validates_tag_version_and_document() {
        let dir = TempDir::new("invalid");
        let malformed = dir.path.join("malformed.ocad");
        fs::write(&malformed, "not json").expect("write");
        assert_eq!(open(&malformed).expect_err("malformed").code, "format/invalid");

        let future = dir.path.join("future.ocad");
        fs::write(&future, json!({ "format": FILE_FORMAT, "version": 2, "document": {} }).to_string()).expect("write");
        assert_eq!(open(&future).expect_err("future version").code, "format/unsupported-version");

        let duplicate = dir.path.join("duplicate.ocad");
        let document = json!({
            "format": FILE_FORMAT,
            "version": 1,
            "document": {
                "id": "drawing-1", "title": null, "revision": 1, "units": "mm",
                "layers": [{ "id": "0", "name": "0", "color": "#d0d7de", "visible": true, "locked": false }],
                "entities": [
                    { "id": "entity-1", "layer": "0", "kind": { "type": "line", "from": [0.0, 0.0], "to": [1.0, 1.0] } },
                    { "id": "entity-1", "layer": "0", "kind": { "type": "line", "from": [0.0, 0.0], "to": [2.0, 2.0] } }
                ],
                "next_entity": 2
            }
        });
        fs::write(&duplicate, document.to_string()).expect("write");
        assert_eq!(open(&duplicate).expect_err("duplicate ids").code, "format/invalid");

        assert_eq!(open(&dir.path.join("missing.ocad")).expect_err("missing").code, "drawing/not-found");
    }

    #[test]
    fn unimplemented_routes_keep_the_stable_code() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        assert_eq!(render(handle, "png").expect_err("png preview").code, NOT_IMPLEMENTED);
        assert_eq!(export(handle, "dwg", Path::new("plan.dwg")).expect_err("unknown").code, "format/unsupported");
        assert_eq!(inspect(handle, Some(vec!["constraints".to_string()])).expect_err("constraints").code, NOT_IMPLEMENTED);
        assert_eq!(inspect(handle, Some(vec!["references".to_string()])).expect_err("references").code, NOT_IMPLEMENTED);
        assert_eq!(summary(u64::MAX).expect_err("unknown handle").code, "drawing/not-found");
        close(handle).expect("close");
        assert_eq!(close(handle).expect_err("already closed").code, "drawing/not-found");
    }

    #[test]
    fn empty_drawing_exports_the_fallback_sheet() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        let report = inspect(handle, None).expect("inspect");
        assert_eq!(report["passed"], json!(true));
        let svg = exchange::svg(&store::read(handle, |session| session.drawing.clone()).expect("drawing"));
        assert!(svg.contains("viewBox=\"0 0 297 210\""));
        close(handle).expect("close");
    }

    #[test]
    fn undo_and_redo_restore_snapshots_and_bump_the_revision() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        assert_eq!(created["drawing"]["canUndo"], json!(false));
        assert_eq!(created["drawing"]["canRedo"], json!(false));

        add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("line");
        let second = add_circle(handle, DEFAULT_LAYER, [5.0, 5.0], 2.0).expect("circle");
        let revision = second["revision"].as_u64().expect("revision");
        assert_eq!(second["drawing"]["canUndo"], json!(true));

        let undone = undo(handle).expect("undo");
        assert_eq!(undone["drawing"]["entities"], json!(1));
        assert_eq!(undone["revision"], json!(revision + 1));
        assert_eq!(undone["canUndo"], json!(true));
        assert_eq!(undone["canRedo"], json!(true));

        let empty = undo(handle).expect("undo to empty");
        assert_eq!(empty["drawing"]["entities"], json!(0));
        assert_eq!(empty["canUndo"], json!(false));
        assert_eq!(undo(handle).expect_err("nothing to undo").code, "drawing/nothing-to-undo");

        let first = redo(handle).expect("redo");
        assert_eq!(first["drawing"]["entities"], json!(1));
        assert_eq!(first["canRedo"], json!(true));
        let restored = redo(handle).expect("redo circle");
        assert_eq!(restored["drawing"]["entities"], json!(2));
        assert_eq!(restored["canRedo"], json!(false));
        assert_eq!(redo(handle).expect_err("nothing to redo").code, "drawing/nothing-to-redo");

        close(handle).expect("close");
    }

    #[test]
    fn save_is_not_an_undo_step_and_history_is_bounded() {
        let dir = TempDir::new("history");
        let path = dir.path.join("plan.ocad");
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [1.0, 1.0]).expect("line");
        let before = summary(handle).expect("summary");
        save(handle, &path, true).expect("save");
        let after = summary(handle).expect("summary");
        assert_eq!(before["revision"], after["revision"]);
        assert_eq!(after["canUndo"], json!(true));
        undo(handle).expect("undo after save");
        assert_eq!(summary(handle).expect("summary")["entities"], json!(0));
        close(handle).expect("close");

        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        for index in 0..40 {
            add_line(handle, DEFAULT_LAYER, [f64::from(index), 0.0], [f64::from(index), 1.0]).expect("line");
        }
        for _ in 0..store::HISTORY_LIMIT {
            undo(handle).expect("undo within the history bound");
        }
        assert_eq!(summary(handle).expect("summary")["entities"], json!(8));
        assert_eq!(undo(handle).expect_err("history exhausted").code, "drawing/nothing-to-undo");
        close(handle).expect("close");
    }

    #[test]
    fn layer_crud_renames_references_and_locks_editing() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);

        let added = add_layer(handle, "Walls", Some("#a4262c")).expect("add layer");
        assert_eq!(added["layer"]["name"], json!("Walls"));
        assert_eq!(added["layer"]["color"], json!("#a4262c"));
        assert_eq!(add_layer(handle, "walls", None).expect_err("duplicate").code, "drawing/layer-exists");
        assert_eq!(add_layer(handle, "bad colour", Some("red")).expect_err("colour").code, "session/argument-invalid");

        let change = add_line(handle, "Walls", [0.0, 0.0], [10.0, 0.0]).expect("line on the new layer");
        assert_eq!(change["entity"]["layer"], json!("Walls"));

        let renamed =
            update_layer(handle, "Walls", LayerUpdate { name: Some("Structure".to_string()), ..LayerUpdate::default() }).expect("rename");
        assert_eq!(renamed["layer"]["id"], json!("Structure"));
        let listed = list_entities(handle).expect("list");
        assert_eq!(listed["entities"][0]["layer"], json!("Structure"));
        assert_eq!(summary(handle).expect("summary")["layers"][2]["name"], json!("Structure"));

        let revision = summary(handle).expect("summary")["revision"].as_u64().expect("revision");
        update_layer(handle, "Structure", LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock");
        assert_eq!(add_line(handle, "Structure", [0.0, 0.0], [1.0, 1.0]).expect_err("locked add").code, "drawing/layer-locked");
        assert_eq!(move_entity(handle, "entity-1", Some([1.0, 0.0]), None).expect_err("locked move").code, "drawing/layer-locked");
        assert_eq!(delete_entity(handle, "entity-1").expect_err("locked delete").code, "drawing/layer-locked");
        assert!(summary(handle).expect("summary")["revision"].as_u64().expect("revision") > revision);

        update_layer(handle, "Structure", LayerUpdate { visible: Some(false), ..LayerUpdate::default() }).expect("hide");
        let layer = &summary(handle).expect("summary")["layers"][2];
        assert_eq!(layer["visible"], json!(false));
        assert_eq!(layer["locked"], json!(true));
        let report = inspect(handle, Some(vec!["layers".to_string()])).expect("inspect layers");
        let codes =
            report["issues"].as_array().expect("issues").iter().map(|issue| issue["code"].as_str().unwrap_or_default()).collect::<Vec<_>>();
        assert!(codes.contains(&"layers/hidden"), "{codes:?}");
        assert!(codes.contains(&"layers/locked"), "{codes:?}");

        close(handle).expect("close");
    }

    #[test]
    fn layer_freeze_and_plot_flags_gate_writes_and_reach_dxf() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_layer(handle, "Walls", None).expect("add layer");
        add_line(handle, "Walls", [0.0, 0.0], [10.0, 0.0]).expect("line on the new layer");

        let updated =
            update_layer(handle, "Walls", LayerUpdate { frozen: Some(true), plottable: Some(false), ..LayerUpdate::default() }).expect("flags");
        assert_eq!(updated["layer"]["frozen"], json!(true));
        assert_eq!(updated["layer"]["plottable"], json!(false));
        assert_eq!(add_line(handle, "Walls", [0.0, 0.0], [1.0, 1.0]).expect_err("frozen add").code, "drawing/layer-frozen");
        assert_eq!(update_layer(handle, "Walls", LayerUpdate::default()).expect_err("empty update").code, "session/argument-invalid");

        let (text, _losses) = exchange::dxf(&drawing_of(handle));
        assert!(text.contains("AC1015"), "a non-plottable layer needs DXF 2000 flags");
        assert!(text.contains("2\nWalls\n70\n1\n"), "frozen layers carry DXF flag 1");
        assert!(text.contains("290\n0\n"), "non-plottable layers carry plot flag 0");

        update_layer(handle, "Walls", LayerUpdate { frozen: Some(false), plottable: Some(true), ..LayerUpdate::default() })
            .expect("clear flags");
        let layer = &summary(handle).expect("summary")["layers"][2];
        assert_eq!(layer["frozen"], json!(false));
        assert_eq!(layer["plottable"], json!(true));
        close(handle).expect("close");
    }

    #[test]
    fn ucs_converts_points_and_rotates_entities() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("line");

        let points = ucs(
            handle,
            UcsRequest { origin: [10.0, 0.0], rotation: 90.0, direction: UcsDirection::ToUcs, points: vec![[10.0, 10.0]], ids: Vec::new() },
        )
        .expect("point conversion");
        assert_eq!(points["direction"], json!("to-ucs"));
        let converted = &points["points"][0]["to"];
        assert!((converted[0].as_f64().expect("x") - 10.0).abs() < 1e-9);
        assert!(converted[1].as_f64().expect("y").abs() < 1e-9);

        let mutated = ucs(
            handle,
            UcsRequest {
                origin: [10.0, 0.0],
                rotation: 90.0,
                direction: UcsDirection::ToWorld,
                points: Vec::new(),
                ids: vec!["entity-1".to_string(), "entity-1".to_string()],
            },
        )
        .expect("entity conversion");
        assert_eq!(mutated["entities"], json!(["entity-1"]));
        assert_eq!(mutated["revision"], json!(3));
        let drawing = drawing_of(handle);
        let EntityKind::Line { from, to } = drawing.entities[0].kind
        else {
            panic!("expected a line");
        };
        assert!((from[0] - 10.0).abs() < 1e-9 && from[1].abs() < 1e-9, "{from:?}");
        assert!((to[0] - 10.0).abs() < 1e-9 && (to[1] - 10.0).abs() < 1e-9, "{to:?}");

        assert_eq!(
            ucs(handle, UcsRequest { origin: [0.0, 0.0], rotation: 0.0, direction: UcsDirection::ToWorld, points: vec![], ids: vec![] })
                .expect_err("empty request")
                .code,
            "session/argument-invalid"
        );
        update_layer(handle, DEFAULT_LAYER, LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock");
        assert_eq!(
            ucs(
                handle,
                UcsRequest {
                    origin: [0.0, 0.0],
                    rotation: 0.0,
                    direction: UcsDirection::ToWorld,
                    points: vec![],
                    ids: vec!["entity-1".to_string()],
                }
            )
            .expect_err("locked")
            .code,
            "drawing/layer-locked"
        );
        close(handle).expect("close");
    }

    #[test]
    fn dimension_styles_are_listed_added_updated_and_deleted() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        assert_eq!(created["drawing"]["dimStyles"][0]["name"], json!("standard"));
        assert_eq!(list_dim_styles(handle).expect("list")["dimStyles"].as_array().map(Vec::len), Some(1));

        let added = add_dim_style(
            handle,
            DimensionStyleRequest {
                name: "ARCH".to_string(),
                scale: Some(2.0),
                text_height: Some(3.5),
                precision: Some(0),
                ..DimensionStyleRequest::default()
            },
        )
        .expect("add");
        assert_eq!(added["style"]["id"], json!("ARCH"));
        assert_eq!(added["style"]["scale"], json!(2.0));
        assert_eq!(added["style"]["arrowSize"], json!(2.5));
        assert_eq!(added["style"]["originOffset"], json!(0.625));
        assert_eq!(added["style"]["precision"], json!(0));

        let duplicate = add_dim_style(handle, DimensionStyleRequest { name: "arch".to_string(), ..DimensionStyleRequest::default() })
            .expect_err("duplicate style");
        assert_eq!(duplicate.code, "drawing/style-exists");

        let updated = update_dim_style(
            handle,
            "ARCH",
            DimensionStyleUpdate { name: Some("ARCH-1".to_string()), scale: Some(3.0), ..DimensionStyleUpdate::default() },
        )
        .expect("update");
        assert_eq!(updated["style"]["id"], json!("ARCH-1"));
        assert_eq!(updated["style"]["scale"], json!(3.0));
        assert_eq!(updated["drawing"]["dimStyles"].as_array().map(Vec::len), Some(2));

        let invalid = update_dim_style(handle, "ARCH-1", DimensionStyleUpdate { precision: Some(9), ..DimensionStyleUpdate::default() })
            .expect_err("precision");
        assert_eq!(invalid.code, "session/argument-invalid");

        let deleted = delete_dim_style(handle, "ARCH-1").expect("delete");
        assert_eq!(deleted["deleted"], json!("ARCH-1"));
        assert_eq!(delete_dim_style(handle, "ARCH-1").expect_err("missing").code, "object/not-found");
        assert_eq!(delete_dim_style(handle, "standard").expect_err("last").code, "drawing/style-last");
        close(handle).expect("close");
    }

    #[test]
    fn dxf_round_trip_keeps_dimension_styles() {
        let dir = TempDir::new("dimstyle-dxf");
        let path = dir.path.join("plan.dxf");
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_dim_style(
            handle,
            DimensionStyleRequest {
                name: "ARCH".to_string(),
                scale: Some(2.0),
                arrow_size: Some(3.0),
                precision: Some(1),
                ..DimensionStyleRequest::default()
            },
        )
        .expect("add style");
        add_dimension(handle, DIMENSION_LAYER, [0.0, 0.0], [100.0, 0.0], 10.0, None).expect("dimension");
        export(handle, "dxf", &path).expect("export dxf");
        let dxf = fs::read_to_string(&path).expect("read dxf");
        assert!(dxf.contains("DIMSTYLE"));
        assert!(dxf.contains("ARCH"));

        let imported = import_dxf(&path, Some("Imported".to_string())).expect("import dxf");
        let styles = imported["drawing"]["dimStyles"].as_array().expect("dimStyles");
        let arch = styles.iter().find(|style| style["name"] == json!("ARCH")).expect("imported ARCH style");
        assert_eq!(arch["scale"], json!(2.0));
        assert_eq!(arch["arrowSize"], json!(3.0));
        assert_eq!(arch["precision"], json!(1));
        close(handle).expect("close");
    }

    #[test]
    fn text_styles_are_listed_added_updated_and_deleted() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        assert_eq!(created["drawing"]["textStyles"][0]["name"], json!("standard"));
        assert_eq!(created["drawing"]["textStyles"][0]["font"], json!("txt"));
        assert_eq!(list_text_styles(handle).expect("list")["textStyles"].as_array().map(Vec::len), Some(1));

        let added = add_text_style(
            handle,
            TextStyleRequest {
                name: "NOTES".to_string(),
                font: Some("romans.shx".to_string()),
                height: Some(2.5),
                width_factor: Some(0.8),
                oblique: Some(15.0),
            },
        )
        .expect("add");
        assert_eq!(added["style"]["id"], json!("NOTES"));
        assert_eq!(added["style"]["font"], json!("romans.shx"));
        assert_eq!(added["style"]["widthFactor"], json!(0.8));
        assert_eq!(added["style"]["oblique"], json!(15.0));

        let duplicate =
            add_text_style(handle, TextStyleRequest { name: "notes".to_string(), ..TextStyleRequest::default() }).expect_err("duplicate");
        assert_eq!(duplicate.code, "drawing/style-exists");

        let updated = update_text_style(
            handle,
            "NOTES",
            TextStyleUpdate { name: Some("NOTES-1".to_string()), oblique: Some(0.0), ..TextStyleUpdate::default() },
        )
        .expect("update");
        assert_eq!(updated["style"]["id"], json!("NOTES-1"));
        assert_eq!(updated["style"]["oblique"], json!(0.0));

        let invalid =
            update_text_style(handle, "NOTES-1", TextStyleUpdate { oblique: Some(120.0), ..TextStyleUpdate::default() }).expect_err("oblique");
        assert_eq!(invalid.code, "session/argument-invalid");

        let deleted = delete_text_style(handle, "NOTES-1").expect("delete");
        assert_eq!(deleted["deleted"], json!("NOTES-1"));
        assert_eq!(delete_text_style(handle, "standard").expect_err("last").code, "drawing/style-last");
        close(handle).expect("close");
    }

    #[test]
    fn dxf_round_trip_keeps_text_styles() {
        let dir = TempDir::new("textstyle-dxf");
        let path = dir.path.join("plan.dxf");
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_text_style(
            handle,
            TextStyleRequest {
                name: "NOTES".to_string(),
                font: Some("romans.shx".to_string()),
                height: Some(3.0),
                width_factor: Some(0.75),
                oblique: Some(10.0),
            },
        )
        .expect("add style");
        add_text(handle, DEFAULT_LAYER, [0.0, 0.0], 2.5, "Site").expect("text");
        export(handle, "dxf", &path).expect("export dxf");
        let dxf = fs::read_to_string(&path).expect("read dxf");
        assert!(dxf.contains("STYLE"));
        assert!(dxf.contains("romans.shx"));

        let imported = import_dxf(&path, Some("Imported".to_string())).expect("import dxf");
        let styles = imported["drawing"]["textStyles"].as_array().expect("textStyles");
        let notes = styles.iter().find(|style| style["name"] == json!("NOTES")).expect("imported NOTES style");
        assert_eq!(notes["font"], json!("romans.shx"));
        assert_eq!(notes["height"], json!(3.0));
        assert_eq!(notes["widthFactor"], json!(0.75));
        assert_eq!(notes["oblique"], json!(10.0));
        close(handle).expect("close");
    }

    #[test]
    fn linetypes_and_layer_lineweights_round_trip() {
        let dir = TempDir::new("linetypes");
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        let listed = list_linetypes(handle).expect("list");
        assert_eq!(listed["linetypes"].as_array().map(Vec::len), Some(1));
        assert_eq!(listed["linetypes"][0]["name"], json!("continuous"));

        let added = add_linetype(
            handle,
            LinetypeRequest { name: "DASHED".to_string(), description: Some("Dashed".to_string()), pattern: Some(vec![5.0, 2.0]) },
        )
        .expect("add linetype");
        assert_eq!(added["linetype"]["pattern"], json!([5.0, 2.0]));

        let layer = update_layer(
            handle,
            DEFAULT_LAYER,
            LayerUpdate { linetype: Some("DASHED".to_string()), lineweight: Some(0.5), ..LayerUpdate::default() },
        )
        .expect("assign linetype");
        assert_eq!(layer["layer"]["linetype"], json!("DASHED"));
        assert_eq!(layer["layer"]["lineweight"], json!(0.5));

        assert!(delete_linetype(handle, "continuous").is_err(), "a referenced line type cannot be removed");
        let renamed = update_linetype(handle, "DASHED", LinetypeUpdate { name: Some("DASH-1".to_string()), ..LinetypeUpdate::default() })
            .expect("rename");
        assert_eq!(renamed["linetype"]["name"], json!("DASH-1"));
        assert_eq!(renamed["drawing"]["layers"][0]["linetype"], json!("DASH-1"));

        let path = dir.path.join("plan.dxf");
        export(handle, "dxf", &path).expect("export dxf");
        let text = fs::read_to_string(&path).expect("read dxf");
        assert!(text.contains("LTYPE"));
        assert!(text.contains("DASH-1"));
        assert!(text.contains("370"));

        let imported = import_dxf(&path, None).expect("import dxf");
        let imported_handle = handle_of(&imported);
        let linetypes = list_linetypes(imported_handle).expect("list imported");
        let dashed = linetypes["linetypes"]
            .as_array()
            .and_then(|entries| entries.iter().find(|entry| entry["name"] == json!("DASH-1")))
            .cloned()
            .expect("imported line type");
        assert_eq!(dashed["pattern"], json!([5.0, 2.0]));
        let imported_layer = imported["drawing"]["layers"]
            .as_array()
            .and_then(|entries| entries.iter().find(|entry| entry["id"] == json!(DEFAULT_LAYER)))
            .cloned()
            .expect("imported layer");
        assert_eq!(imported_layer["lineweight"], json!(0.5));
        close(imported_handle).expect("close imported");
        close(handle).expect("close");
    }

    #[test]
    fn snap_points_report_endpoints_midpoints_centres_and_intersections() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        let first = add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("line");
        let second = add_line(handle, DEFAULT_LAYER, [5.0, -5.0], [5.0, 5.0]).expect("line");
        add_circle(handle, DEFAULT_LAYER, [20.0, 20.0], 2.0).expect("circle");

        let all = snap_points(handle, SnapRequest::default()).expect("snap");
        assert_eq!(all["count"], json!(8));
        let modes = all["modes"].as_array().expect("modes");
        assert_eq!(modes.len(), SNAP_MODES.len());

        let intersections =
            snap_points(handle, SnapRequest { modes: vec!["intersection".to_string()], ..SnapRequest::default() }).expect("intersect");
        assert_eq!(intersections["count"], json!(1));
        assert_eq!(intersections["points"][0]["point"], json!([5.0, 0.0]));
        assert_eq!(intersections["points"][0]["entities"], json!([first["entity"]["id"], second["entity"]["id"]]));

        let near = snap_points(
            handle,
            SnapRequest {
                modes: vec!["endpoint".to_string(), "midpoint".to_string()],
                point: Some([9.9, 0.0]),
                tolerance: Some(0.5),
                ..SnapRequest::default()
            },
        )
        .expect("near");
        assert_eq!(near["count"], json!(1));
        assert_eq!(near["points"][0]["point"], json!([10.0, 0.0]));
        assert_eq!(near["points"][0]["kind"], json!("endpoint"));

        let centres = snap_points(handle, SnapRequest { modes: vec!["center".to_string()], ..SnapRequest::default() }).expect("center");
        assert_eq!(centres["points"][0]["point"], json!([20.0, 20.0]));

        assert!(snap_points(handle, SnapRequest { modes: vec!["edge".to_string()], ..SnapRequest::default() }).is_err());
        assert!(snap_points(handle, SnapRequest { ids: Some(vec!["entity-99".to_string()]), ..SnapRequest::default() }).is_err());
        assert!(snap_points(handle, SnapRequest { tolerance: Some(1.0), ..SnapRequest::default() }).is_err());
        close(handle).expect("close");
    }

    #[test]
    fn drawing_properties_are_read_and_written() {
        let created = create(CreateRequest { title: Some("Site".to_string()), ..CreateRequest::default() }).expect("create");
        let handle = handle_of(&created);
        let initial = properties(handle).expect("properties");
        assert_eq!(initial["title"], json!("Site"));
        assert_eq!(initial["properties"]["author"], json!(""));

        let updated = update_properties(
            handle,
            PropertiesUpdate {
                title: Some("  Site plan  ".to_string()),
                author: Some("Design team".to_string()),
                company: Some("OpenCAD".to_string()),
                project: Some("Harbour".to_string()),
                ..PropertiesUpdate::default()
            },
        )
        .expect("update");
        assert_eq!(updated["title"], json!("Site plan"));
        assert_eq!(updated["drawing"]["title"], json!("Site plan"));
        assert_eq!(updated["drawing"]["properties"]["author"], json!("Design team"));
        assert_eq!(updated["properties"]["company"], json!("OpenCAD"));
        assert_eq!(updated["revision"], json!(2));

        let cleared =
            update_properties(handle, PropertiesUpdate { author: Some(String::new()), ..PropertiesUpdate::default() }).expect("clear");
        assert_eq!(cleared["properties"]["author"], json!(""));
        assert_eq!(cleared["properties"]["company"], json!("OpenCAD"));

        let without =
            update_properties(handle, PropertiesUpdate { title: Some(String::new()), ..PropertiesUpdate::default() }).expect("clear title");
        assert_eq!(without["title"], Value::Null);

        assert!(update_properties(handle, PropertiesUpdate::default()).is_err());
        assert!(
            update_properties(handle, PropertiesUpdate { author: Some("a".repeat(PROPERTY_VALUE_LIMIT + 1)), ..PropertiesUpdate::default() })
                .is_err()
        );
        close(handle).expect("close");
    }

    #[test]
    fn selection_matches_window_and_crossing_rectangles() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_layer(handle, "walls", None).expect("layer");
        let first = add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("line");
        let second = add_line(handle, "walls", [10.0, 10.0], [20.0, 20.0]).expect("line");
        add_circle(handle, DEFAULT_LAYER, [50.0, 50.0], 5.0).expect("circle");

        let window = select(handle, SelectionMode::Window, [[-1.0, -1.0], [11.0, 1.0]], None).expect("window");
        assert_eq!(window["mode"], json!("window"));
        assert_eq!(window["count"], json!(1));
        assert_eq!(window["ids"], json!([first["entity"]["id"]]));

        let crossing = select(handle, SelectionMode::Crossing, [[9.0, -1.0], [11.0, 11.0]], None).expect("crossing");
        assert_eq!(crossing["count"], json!(2), "{crossing}");
        assert_eq!(crossing["ids"][0], first["entity"]["id"]);
        assert_eq!(crossing["ids"][1], second["entity"]["id"]);

        let walls = select(handle, SelectionMode::Crossing, [[9.0, -1.0], [11.0, 11.0]], Some("walls")).expect("layer filter");
        assert_eq!(walls["count"], json!(1));
        assert_eq!(walls["ids"][0], second["entity"]["id"]);
        assert_eq!(walls["layer"], json!("walls"));

        let circle = select(handle, SelectionMode::Crossing, [[44.0, 44.0], [56.0, 56.0]], None).expect("circle");
        assert_eq!(circle["count"], json!(1));

        assert_eq!(
            select(handle, SelectionMode::Window, [[5.0, 5.0], [5.0, 5.0]], None).expect_err("degenerate").code,
            "session/argument-invalid"
        );
        assert_eq!(
            select(handle, SelectionMode::Window, [[0.0, 0.0], [1.0, 1.0]], Some("missing")).expect_err("missing layer").code,
            "object/not-found"
        );
        close(handle).expect("close");
    }

    #[test]
    fn entities_move_and_update_without_losing_identity() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("line");
        add_circle(handle, DEFAULT_LAYER, [5.0, 5.0], 2.0).expect("circle");
        add_text(handle, DEFAULT_LAYER, [1.0, 1.0], 2.5, "Old").expect("text");

        let moved = move_entity(handle, "entity-1", Some([10.0, 5.0]), None).expect("move line");
        assert_eq!(moved["entity"]["kind"]["from"], json!([10.0, 5.0]));
        assert_eq!(moved["entity"]["kind"]["to"], json!([20.0, 5.0]));

        let placed = move_entity(handle, "entity-2", None, Some([0.0, 0.0])).expect("absolute move");
        assert_eq!(placed["entity"]["kind"]["center"], json!([0.0, 0.0]));

        let text = update_entity(handle, "entity-3", Some("Hall".to_string()), Some(4.0), None).expect("update text");
        assert_eq!(text["entity"]["kind"]["content"], json!("Hall"));
        assert_eq!(text["entity"]["kind"]["height"], json!(4.0));

        let circle = update_entity(handle, "entity-2", None, None, Some(7.0)).expect("update radius");
        assert_eq!(circle["entity"]["kind"]["radius"], json!(7.0));

        assert_eq!(update_entity(handle, "entity-1", None, Some(2.0), None).expect_err("line height").code, "object/unsupported");
        assert_eq!(
            move_entity(handle, "entity-1", Some([1.0, 1.0]), Some([0.0, 0.0])).expect_err("ambiguous move").code,
            "session/argument-invalid"
        );
        update_layer(handle, DEFAULT_LAYER, LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock default layer");
        assert_eq!(update_entity(handle, "entity-3", Some("X".to_string()), None, None).expect_err("locked").code, "drawing/layer-locked");

        close(handle).expect("close");
    }

    #[test]
    fn polyline_vertices_and_closure_are_editable() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]], false).expect("polyline");

        let closed = update_polyline(handle, "entity-1", PolylineUpdate { closed: Some(true), ..PolylineUpdate::default() })
            .expect("close the polyline");
        assert_eq!(closed["entity"]["kind"]["closed"], json!(true));
        assert_eq!(closed["revision"], json!(3));

        let inserted = update_polyline(
            handle,
            "entity-1",
            PolylineUpdate { insert_at: Some(1), insert_point: Some([5.0, 0.0]), ..PolylineUpdate::default() },
        )
        .expect("insert a vertex");
        assert_eq!(inserted["entity"]["kind"]["points"], json!([[0.0, 0.0], [5.0, 0.0], [10.0, 0.0], [10.0, 10.0]]));

        let appended = update_polyline(
            handle,
            "entity-1",
            PolylineUpdate { insert_at: Some(4), insert_point: Some([0.0, 10.0]), ..PolylineUpdate::default() },
        )
        .expect("append a vertex");
        assert_eq!(appended["entity"]["kind"]["points"].as_array().map(Vec::len), Some(5));

        let removed =
            update_polyline(handle, "entity-1", PolylineUpdate { remove_at: Some(1), ..PolylineUpdate::default() }).expect("remove a vertex");
        assert_eq!(removed["entity"]["kind"]["points"].as_array().map(Vec::len), Some(4));

        let replaced = update_polyline(
            handle,
            "entity-1",
            PolylineUpdate {
                points: Some(vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]]),
                closed: Some(true),
                ..PolylineUpdate::default()
            },
        )
        .expect("replace the vertices");
        assert_eq!(replaced["entity"]["kind"]["points"], json!([[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]]));
        assert_eq!(replaced["entity"]["kind"]["closed"], json!(true));

        add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [1.0, 0.0]).expect("line");
        add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [1.0, 0.0]], false).expect("two-point polyline");
        assert_eq!(
            update_polyline(handle, "entity-2", PolylineUpdate { closed: Some(true), ..PolylineUpdate::default() })
                .expect_err("a line is not a polyline")
                .code,
            "object/unsupported"
        );
        assert_eq!(update_polyline(handle, "entity-1", PolylineUpdate::default()).expect_err("no edit").code, "session/argument-invalid");
        assert_eq!(
            update_polyline(handle, "entity-1", PolylineUpdate { insert_at: Some(0), ..PolylineUpdate::default() })
                .expect_err("insert without a point")
                .code,
            "session/argument-invalid"
        );
        assert_eq!(
            update_polyline(
                handle,
                "entity-1",
                PolylineUpdate { insert_at: Some(99), insert_point: Some([0.0, 0.0]), ..PolylineUpdate::default() }
            )
            .expect_err("index beyond the list")
            .code,
            "session/argument-invalid"
        );
        assert_eq!(
            update_polyline(handle, "entity-1", PolylineUpdate { points: Some(vec![[0.0, 0.0]]), ..PolylineUpdate::default() })
                .expect_err("too few vertices")
                .code,
            "session/argument-invalid"
        );
        assert_eq!(
            update_polyline(handle, "entity-3", PolylineUpdate { remove_at: Some(0), ..PolylineUpdate::default() })
                .expect_err("removing leaves one vertex")
                .code,
            "session/argument-invalid"
        );
        assert_eq!(
            update_polyline(handle, "entity-404", PolylineUpdate { closed: Some(true), ..PolylineUpdate::default() })
                .expect_err("missing entity")
                .code,
            "object/not-found"
        );
        update_layer(handle, DEFAULT_LAYER, LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock");
        assert_eq!(
            update_polyline(handle, "entity-1", PolylineUpdate { closed: Some(false), ..PolylineUpdate::default() })
                .expect_err("locked layer")
                .code,
            "drawing/layer-locked"
        );
        close(handle).expect("close");
    }

    #[test]
    fn rectangle_builds_a_closed_counter_clockwise_polyline() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        let before = drawing_of(handle).revision;

        let rectangle = add_rectangle(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 5.0]).expect("rectangle");
        assert_eq!(rectangle["entity"]["kind"]["type"], json!("polyline"));
        assert_eq!(rectangle["entity"]["kind"]["closed"], json!(true));
        assert_eq!(rectangle["entity"]["kind"]["points"], json!([[0.0, 0.0], [10.0, 0.0], [10.0, 5.0], [0.0, 5.0]]));
        assert_eq!(drawing_of(handle).revision, before + 1, "one revision for the whole rectangle");

        let measured = measure_entities(handle, &["entity-1".to_string()]).expect("measure");
        assert_eq!(measured["entities"][0]["closed"], json!(true));
        assert_eq!(measured["entities"][0]["area"], json!(50.0));
        assert_eq!(measured["entities"][0]["length"], json!(30.0));

        undo(handle).expect("undo the rectangle");
        assert_eq!(list_entities(handle).expect("list")["entities"].as_array().map(Vec::len), Some(0), "one undo step removes it");

        assert_eq!(add_rectangle(handle, DEFAULT_LAYER, [5.0, 0.0], [5.0, 5.0]).expect_err("zero width").code, "session/argument-invalid");
        assert_eq!(
            add_rectangle(handle, DEFAULT_LAYER, [10.0, 10.0], [0.0, 0.0]).expect_err("inverted corners").code,
            "session/argument-invalid"
        );
        assert_eq!(
            add_rectangle(handle, DEFAULT_LAYER, [0.0, 0.0], [f64::NAN, 5.0]).expect_err("non-finite corner").code,
            "session/argument-invalid"
        );
        assert_eq!(add_rectangle(handle, "missing", [0.0, 0.0], [5.0, 5.0]).expect_err("missing layer").code, "object/not-found");

        update_layer(handle, DEFAULT_LAYER, LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock");
        assert_eq!(add_rectangle(handle, DEFAULT_LAYER, [0.0, 0.0], [5.0, 5.0]).expect_err("locked layer").code, "drawing/layer-locked");
        close(handle).expect("close");
    }

    #[test]
    fn polygon_builds_a_closed_regular_polyline() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        let before = drawing_of(handle).revision;

        let triangle = add_polygon(handle, DEFAULT_LAYER, [0.0, 0.0], 10.0, 3, 0.0).expect("triangle");
        assert_eq!(triangle["entity"]["kind"]["type"], json!("polyline"));
        assert_eq!(triangle["entity"]["kind"]["closed"], json!(true));
        assert_eq!(drawing_of(handle).revision, before + 1, "one revision for the whole polygon");
        let points = triangle["entity"]["kind"]["points"].as_array().expect("points").clone();
        assert_eq!(points.len(), 3);
        let vertex = |index: usize| (points[index][0].as_f64().expect("x"), points[index][1].as_f64().expect("y"));
        assert!((vertex(0).0 - 10.0).abs() < 1e-9 && vertex(0).1.abs() < 1e-9, "the first vertex starts at rotation 0: {:?}", vertex(0));
        assert!((vertex(1).0 + 5.0).abs() < 1e-9, "the second vertex sits at 120 degrees: {:?}", vertex(1));
        assert!((vertex(1).1 - 8.660_254_037_844_387).abs() < 1e-9, "the second vertex sits at 120 degrees: {:?}", vertex(1));
        for point in &points {
            let x = point[0].as_f64().expect("x");
            let y = point[1].as_f64().expect("y");
            assert!(((x * x + y * y).sqrt() - 10.0).abs() < 1e-9, "every vertex sits on the radius: {point:?}");
        }

        let rotated = add_polygon(handle, DEFAULT_LAYER, [5.0, 5.0], 4.0, 4, 90.0).expect("rotated square");
        let corners = rotated["entity"]["kind"]["points"].as_array().expect("points").clone();
        assert_eq!(corners.len(), 4);
        let start = (corners[0][0].as_f64().expect("x"), corners[0][1].as_f64().expect("y"));
        assert!((start.0 - 5.0).abs() < 1e-9 && (start.1 - 9.0).abs() < 1e-9, "rotation places the first vertex: {start:?}");

        undo(handle).expect("undo the square");
        undo(handle).expect("undo the triangle");
        assert_eq!(list_entities(handle).expect("list")["entities"].as_array().map(Vec::len), Some(0), "each polygon is one undo step");

        assert_eq!(add_polygon(handle, DEFAULT_LAYER, [0.0, 0.0], 5.0, 2, 0.0).expect_err("too few sides").code, "session/argument-invalid");
        assert_eq!(
            add_polygon(handle, DEFAULT_LAYER, [0.0, 0.0], 5.0, MAX_POLYGON_SIDES + 1, 0.0).expect_err("too many sides").code,
            "session/argument-invalid"
        );
        assert_eq!(
            add_polygon(handle, DEFAULT_LAYER, [0.0, 0.0], 0.0, 4, 0.0).expect_err("non-positive radius").code,
            "session/argument-invalid"
        );
        assert_eq!(
            add_polygon(handle, DEFAULT_LAYER, [0.0, 0.0], 5.0, 4, f64::INFINITY).expect_err("non-finite rotation").code,
            "session/argument-invalid"
        );
        assert_eq!(
            add_polygon(handle, DEFAULT_LAYER, [f64::NAN, 0.0], 5.0, 4, 0.0).expect_err("non-finite centre").code,
            "session/argument-invalid"
        );

        update_layer(handle, DEFAULT_LAYER, LayerUpdate { frozen: Some(true), ..LayerUpdate::default() }).expect("freeze");
        assert_eq!(add_polygon(handle, DEFAULT_LAYER, [0.0, 0.0], 5.0, 4, 0.0).expect_err("frozen layer").code, "drawing/layer-frozen");
        close(handle).expect("close");
    }

    #[test]
    fn arc_radius_and_sweep_angles_are_editable() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_arc(handle, DEFAULT_LAYER, [0.0, 0.0], 5.0, 0.0, 90.0).expect("arc");

        let radius = update_arc(handle, "entity-1", Some(8.0), None, None).expect("update the radius");
        assert_eq!(radius["entity"]["kind"]["radius"], json!(8.0));
        assert_eq!(radius["entity"]["kind"]["start_angle"], json!(0.0));

        let angles = update_arc(handle, "entity-1", None, Some(45.0), Some(180.0)).expect("update the sweep");
        assert_eq!(angles["entity"]["kind"]["start_angle"], json!(45.0));
        assert_eq!(angles["entity"]["kind"]["end_angle"], json!(180.0));
        assert_eq!(angles["entity"]["kind"]["radius"], json!(8.0));

        let full = update_arc(handle, "entity-1", None, Some(90.0), Some(90.0)).expect("full turn");
        assert_eq!(full["entity"]["kind"]["start_angle"], json!(90.0));
        assert_eq!(full["entity"]["kind"]["end_angle"], json!(90.0));

        add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [1.0, 0.0]).expect("line");
        assert_eq!(update_arc(handle, "entity-2", Some(2.0), None, None).expect_err("a line is not an arc").code, "object/unsupported");
        assert_eq!(update_arc(handle, "entity-1", None, None, None).expect_err("no edit").code, "session/argument-invalid");
        assert_eq!(update_arc(handle, "entity-1", Some(0.0), None, None).expect_err("zero radius").code, "session/argument-invalid");
        assert_eq!(update_arc(handle, "entity-1", Some(f64::NAN), None, None).expect_err("non-finite radius").code, "session/argument-invalid");
        assert_eq!(
            update_arc(handle, "entity-1", None, Some(f64::INFINITY), None).expect_err("non-finite angle").code,
            "session/argument-invalid"
        );
        assert_eq!(update_arc(handle, "entity-404", Some(2.0), None, None).expect_err("missing entity").code, "object/not-found");
        update_layer(handle, DEFAULT_LAYER, LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock");
        assert_eq!(update_arc(handle, "entity-1", Some(2.0), None, None).expect_err("locked layer").code, "drawing/layer-locked");
        close(handle).expect("close");
    }

    #[test]
    fn entity_layer_assignment_preserves_geometry_and_respects_locks() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_layer(handle, "Walls", None).expect("add layer");
        add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("line");

        let assigned = assign_entity_layer(handle, "entity-1", "Walls").expect("assign the layer");
        assert_eq!(assigned["from"], json!(DEFAULT_LAYER));
        assert_eq!(assigned["to"], json!("Walls"));
        assert_eq!(assigned["moved"], json!(true));
        assert_eq!(assigned["entity"]["layer"], json!("Walls"));
        assert_eq!(assigned["entity"]["kind"]["from"], json!([0.0, 0.0]));
        assert_eq!(assigned["entity"]["kind"]["to"], json!([10.0, 0.0]));
        let revision = assigned["revision"].as_u64().expect("revision");
        let layers = assigned["drawing"]["layers"].as_array().expect("layers");
        assert_eq!(layers.iter().find(|layer| layer["id"] == json!("Walls")).expect("Walls")["entityCount"], json!(1));
        assert_eq!(layers.iter().find(|layer| layer["id"] == json!(DEFAULT_LAYER)).expect("default")["entityCount"], json!(0));

        let same = assign_entity_layer(handle, "entity-1", "Walls").expect("assign the same layer");
        assert_eq!(same["moved"], json!(false));
        assert_eq!(same["revision"], json!(revision));

        assert_eq!(assign_entity_layer(handle, "entity-1", "missing").expect_err("missing target").code, "object/not-found");
        assert_eq!(assign_entity_layer(handle, "entity-404", "Walls").expect_err("missing entity").code, "object/not-found");
        update_layer(handle, "Walls", LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock the target");
        assert_eq!(
            assign_entity_layer(handle, "entity-1", "Walls").expect_err("the entity is already on the locked layer").code,
            "drawing/layer-locked"
        );
        update_layer(handle, "Walls", LayerUpdate { locked: Some(false), ..LayerUpdate::default() }).expect("unlock the target");
        update_layer(handle, "Walls", LayerUpdate { frozen: Some(true), ..LayerUpdate::default() }).expect("freeze the target");
        assert_eq!(
            assign_entity_layer(handle, "entity-1", DEFAULT_LAYER).expect_err("the source layer is frozen").code,
            "drawing/layer-frozen"
        );
        close(handle).expect("close");
    }

    #[test]
    fn find_text_matches_case_insensitively_and_filters_by_layer() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_text(handle, DEFAULT_LAYER, [0.0, 0.0], 2.5, "Main Hall").expect("text");
        add_text(handle, DEFAULT_LAYER, [0.0, 4.0], 2.5, "Side hall").expect("text");
        add_text(handle, DEFAULT_LAYER, [0.0, 8.0], 2.5, "Room").expect("text");
        add_layer(handle, "Notes", None).expect("layer");

        let found = find_text(handle, "HALL", TextSearchOptions::default()).expect("find");
        assert_eq!(found["count"], json!(2));
        assert_eq!(found["matches"][0]["id"], json!("entity-1"));
        assert_eq!(found["matches"][1]["content"], json!("Side hall"));

        let exact = find_text(handle, "HALL", TextSearchOptions { match_case: true, layer: None }).expect("exact");
        assert_eq!(exact["count"], json!(0));
        let scoped = find_text(handle, "hall", TextSearchOptions { match_case: false, layer: Some("Notes".to_string()) }).expect("scoped");
        assert_eq!(scoped["count"], json!(0));
        assert_eq!(find_text(handle, "", TextSearchOptions::default()).expect_err("empty pattern").code, "session/argument-invalid");
        close(handle).expect("close");
    }

    #[test]
    fn replace_text_rewrites_every_match_in_one_history_step() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_text(handle, DEFAULT_LAYER, [0.0, 0.0], 2.5, "hall hall").expect("text");
        add_text(handle, DEFAULT_LAYER, [0.0, 4.0], 2.5, "Hall").expect("text");
        add_text(handle, DEFAULT_LAYER, [0.0, 8.0], 2.5, "Room").expect("text");

        let replaced = replace_text(handle, "hall", "lobby", TextSearchOptions::default()).expect("replace");
        assert_eq!(replaced["count"], json!(2));
        assert_eq!(replaced["occurrences"], json!(3));
        assert_eq!(replaced["revision"], json!(5));
        let drawing = drawing_of(handle);
        assert!(matches!(&drawing.entities[0].kind, EntityKind::Text { content, .. } if content == "lobby lobby"));
        assert!(matches!(&drawing.entities[1].kind, EntityKind::Text { content, .. } if content == "lobby"));

        undo(handle).expect("undo");
        let restored = drawing_of(handle);
        assert!(matches!(&restored.entities[0].kind, EntityKind::Text { content, .. } if content == "hall hall"));

        let before = restored.revision;
        let none = replace_text(handle, "zzz", "x", TextSearchOptions::default()).expect("no match");
        assert_eq!(none["count"], json!(0));
        assert_eq!(none["revision"], json!(before));

        update_layer(handle, DEFAULT_LAYER, LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock");
        assert_eq!(replace_text(handle, "hall", "x", TextSearchOptions::default()).expect_err("locked layer").code, "drawing/layer-locked");
        close(handle).expect("close");
    }

    #[test]
    fn groups_create_add_remove_and_prune_deleted_entities() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [1.0, 0.0]).expect("line");
        add_line(handle, DEFAULT_LAYER, [0.0, 1.0], [1.0, 1.0]).expect("line");
        add_line(handle, DEFAULT_LAYER, [0.0, 2.0], [1.0, 2.0]).expect("line");

        let group = create_group(
            handle,
            GroupCreateRequest { name: "Frame".to_string(), entities: vec!["entity-1".to_string(), "entity-2".to_string()] },
        )
        .expect("create group");
        assert_eq!(group["group"]["entities"], json!(["entity-1", "entity-2"]));
        assert_eq!(group["drawing"]["groups"], json!(1));
        assert_eq!(group["revision"], json!(5));

        assert_eq!(
            create_group(handle, GroupCreateRequest { name: "frame".to_string(), entities: vec!["entity-1".to_string()] },)
                .expect_err("duplicate name")
                .code,
            "drawing/group-exists"
        );
        assert_eq!(
            create_group(handle, GroupCreateRequest { name: "Missing".to_string(), entities: vec!["entity-9".to_string()] })
                .expect_err("unknown entity")
                .code,
            "object/not-found"
        );
        assert_eq!(
            create_group(handle, GroupCreateRequest { name: "Empty".to_string(), entities: Vec::new() }).expect_err("empty group").code,
            "session/argument-invalid"
        );

        let added = add_group_members(handle, "Frame", &["entity-3".to_string(), "entity-1".to_string()]).expect("add members");
        assert_eq!(added["group"]["entities"], json!(["entity-1", "entity-2", "entity-3"]));
        let before = added["revision"].as_u64().expect("revision");
        let noop = add_group_members(handle, "Frame", &["entity-1".to_string()]).expect("no-op add");
        assert_eq!(noop["revision"], json!(before));

        let removed = remove_group_members(handle, "Frame", &["entity-2".to_string()]).expect("remove member");
        assert_eq!(removed["group"]["entities"], json!(["entity-1", "entity-3"]));

        delete_entity(handle, "entity-1").expect("delete member");
        let listed = list_groups(handle).expect("list groups");
        assert_eq!(listed["groups"][0]["entities"], json!(["entity-3"]));

        let deleted = delete_group(handle, "Frame").expect("delete group");
        assert_eq!(deleted["deleted"], json!("Frame"));
        assert_eq!(deleted["drawing"]["groups"], json!(0));
        assert_eq!(delete_group(handle, "Frame").expect_err("missing group").code, "object/not-found");

        let mut drawing = Drawing::empty(None, Units::Mm);
        drawing.groups.push(EntityGroup { id: "G".to_string(), name: "G".to_string(), entities: vec!["entity-9".to_string()] });
        assert!(drawing.validate().is_err());
        close(handle).expect("close");
    }

    fn sample_dxf() -> String {
        [
            "0",
            "SECTION",
            "2",
            "HEADER",
            "9",
            "$INSUNITS",
            "70",
            "4",
            "0",
            "ENDSEC",
            "0",
            "SECTION",
            "2",
            "TABLES",
            "0",
            "TABLE",
            "2",
            "LAYER",
            "70",
            "2",
            "0",
            "LAYER",
            "2",
            "0",
            "70",
            "0",
            "62",
            "7",
            "0",
            "LAYER",
            "2",
            "WALLS",
            "70",
            "0",
            "62",
            "1",
            "0",
            "ENDTAB",
            "0",
            "ENDSEC",
            "0",
            "SECTION",
            "2",
            "ENTITIES",
            "0",
            "LINE",
            "8",
            "WALLS",
            "10",
            "0",
            "20",
            "0",
            "11",
            "10",
            "21",
            "0",
            "0",
            "CIRCLE",
            "8",
            "WALLS",
            "10",
            "5",
            "20",
            "5",
            "40",
            "2",
            "0",
            "TEXT",
            "8",
            "NOTES",
            "10",
            "1",
            "20",
            "1",
            "40",
            "2.5",
            "1",
            "Hall",
            "0",
            "LWPOLYLINE",
            "8",
            "WALLS",
            "90",
            "3",
            "70",
            "1",
            "10",
            "0",
            "20",
            "0",
            "10",
            "10",
            "20",
            "0",
            "10",
            "10",
            "20",
            "10",
            "0",
            "POLYLINE",
            "8",
            "WALLS",
            "70",
            "1",
            "0",
            "VERTEX",
            "8",
            "WALLS",
            "10",
            "0",
            "20",
            "0",
            "0",
            "VERTEX",
            "8",
            "WALLS",
            "10",
            "5",
            "20",
            "0",
            "0",
            "SEQEND",
            "8",
            "WALLS",
            "0",
            "SPLINE",
            "8",
            "WALLS",
            "0",
            "ENDSEC",
            "0",
            "EOF",
        ]
        .join("\n")
    }

    #[test]
    fn dxf_import_reads_the_declared_subset() {
        let dir = TempDir::new("dxf-import");
        let source = dir.path.join("site.dxf");
        fs::write(&source, sample_dxf()).expect("write dxf");

        let imported = import_dxf(&source, Some("Imported site".to_string())).expect("import");
        assert_eq!(imported["drawing"]["title"], json!("Imported site"));
        assert_eq!(imported["drawing"]["units"], json!("mm"));
        assert_eq!(imported["imported"]["entities"], json!(5));
        assert_eq!(imported["drawing"]["entities"], json!(5));
        assert_eq!(imported["drawing"]["canUndo"], json!(false));
        let losses = imported["losses"].as_array().expect("losses");
        let codes = losses.iter().map(|loss| loss["code"].as_str().unwrap_or_default()).collect::<Vec<_>>();
        assert!(codes.contains(&"dxf/unsupported-entity"), "{codes:?}");
        assert!(codes.contains(&"dxf/implicit-layer"), "{codes:?}");
        assert!(losses.iter().all(|loss| loss["severity"] == json!("warning") || loss["severity"] == json!("notice")));
        let walls = imported["drawing"]["layers"]
            .as_array()
            .expect("layers")
            .iter()
            .find(|layer| layer["name"] == json!("WALLS"))
            .cloned()
            .expect("walls layer");
        assert_eq!(walls["color"], json!("#ff0000"));
        assert_eq!(walls["entityCount"], json!(4));

        let handle = handle_of(&imported);
        let listed = list_entities(handle).expect("list");
        assert_eq!(listed["entities"][3]["kind"]["closed"], json!(true));
        assert_eq!(listed["entities"][4]["kind"]["type"], json!("polyline"));
        assert_eq!(listed["entities"][4]["kind"]["closed"], json!(true));
        assert_eq!(listed["entities"][4]["kind"]["points"], json!([[0.0, 0.0], [5.0, 0.0]]));
        let target = dir.path.join("imported.ocad");
        save(handle, &target, true).expect("save imported");
        close(handle).expect("close");
        let reopened = open(&target).expect("reopen");
        assert_eq!(reopened["drawing"]["entities"], json!(5));
        close(handle_of(&reopened)).expect("close reopened");

        let broken = dir.path.join("broken.dxf");
        fs::write(&broken, "not a dxf").expect("write");
        assert_eq!(import_dxf(&broken, None).expect_err("invalid dxf").code, "format/invalid");
        assert_eq!(import_dxf(&dir.path.join("missing.dxf"), None).expect_err("missing").code, "drawing/not-found");
    }

    #[test]
    fn pdf_export_writes_a_minimal_document() {
        let dir = TempDir::new("pdf");
        let path = dir.path.join("plan.pdf");
        let created = create(CreateRequest { title: Some("Sheet".to_string()), ..CreateRequest::default() }).expect("create");
        let handle = handle_of(&created);
        add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [100.0, 40.0]).expect("line");
        add_circle(handle, DEFAULT_LAYER, [50.0, 20.0], 10.0).expect("circle");
        add_text(handle, DEFAULT_LAYER, [5.0, 5.0], 2.5, "A(B)").expect("text");

        let report = export(handle, "pdf", &path).expect("export pdf");
        assert_eq!(report["format"], json!("pdf"));
        assert_eq!(report["editable"], json!(false));
        let pdf = fs::read_to_string(&path).expect("read pdf");
        assert!(pdf.starts_with("%PDF"));
        assert!(pdf.contains("/MediaBox [0 0 841.89 595.28]"));
        assert!(pdf.contains("/BaseFont /"));
        assert!(pdf.contains("xref"));
        assert!(pdf.contains("trailer"));
        assert!(pdf.contains("/Size"));
        assert!(pdf.contains("A\\(B\\)"));
        assert!(pdf.trim_end().ends_with("%%EOF"));
        close(handle).expect("close");
    }

    #[test]
    fn inspection_reports_layer_and_text_findings() {
        let mut drawing = Drawing::empty(None, Units::Mm);
        drawing.layers[0].visible = false;
        drawing.layers.push(Layer {
            id: "shadow".to_string(),
            name: "0".to_string(),
            color: "#000000".to_string(),
            visible: true,
            locked: false,
            frozen: false,
            plottable: true,
            linetype: CONTINUOUS_LINETYPE.to_string(),
            lineweight: 0.0,
        });
        drawing.entities.push(Entity {
            id: "entity-1".to_string(),
            layer: DEFAULT_LAYER.to_string(),
            kind: EntityKind::Text { at: [0.0, 0.0], height: 1.0, content: "  ".to_string() },
        });
        drawing.entities.push(Entity {
            id: "entity-2".to_string(),
            layer: DEFAULT_LAYER.to_string(),
            kind: EntityKind::Polyline { points: vec![[0.0, 0.0], [0.0, 0.0], [1.0, 1.0]], closed: false },
        });
        drawing.entities.push(Entity {
            id: "entity-3".to_string(),
            layer: DIMENSION_LAYER.to_string(),
            kind: EntityKind::Dimension { from: [0.0, 0.0], to: [5.0, 0.0], offset: 1.0, text: Some(String::new()) },
        });

        let report = inspection::run(&drawing, inspection::SUPPORTED_CHECKS).to_json();
        let codes =
            report["issues"].as_array().expect("issues").iter().map(|issue| issue["code"].as_str().unwrap_or_default()).collect::<Vec<_>>();
        for expected in
            ["layers/duplicate-name", "layers/hidden", "geometry/empty-text", "geometry/zero-length-segment", "dimensions/empty-text"]
        {
            assert!(codes.contains(&expected), "expected `{expected}` in {codes:?}");
        }
        assert_eq!(report["passed"], json!(false));
    }

    #[test]
    fn render_returns_a_read_only_svg_preview() {
        let created = create(CreateRequest { title: Some("Preview".to_string()), ..CreateRequest::default() }).expect("create");
        let handle = handle_of(&created);
        add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("line");
        add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [5.0, 0.0], [5.0, 5.0]], false).expect("polyline");

        let preview = render(handle, "svg").expect("render svg");
        assert_eq!(preview["format"], json!("svg"));
        assert_eq!(preview["revision"], json!(3));
        let svg = preview["svg"].as_str().expect("svg");
        assert!(svg.contains("<svg"), "the preview is an SVG document");
        assert!(svg.contains("<line"), "the line is drawn");
        assert!(svg.contains("<path"), "the polyline is drawn");
        assert_eq!(preview["width"], json!(12.0), "the 10 unit line plus the 1 unit pad on each side");
        assert!(preview["height"].as_f64().is_some_and(|height| height > 0.0));

        let again = render(handle, "svg").expect("render again");
        assert_eq!(again["svg"], preview["svg"], "rendering is deterministic");

        assert_eq!(render(handle, "png").expect_err("png preview").code, NOT_IMPLEMENTED);
        assert_eq!(render(handle, "pdf").expect_err("pdf preview").code, NOT_IMPLEMENTED);
        close(handle).expect("close");
    }

    #[test]
    fn dxf_writes_polylines_as_polyline_vertex_sequences() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]], true).expect("polyline");
        let drawing = store::read(handle, |session| session.drawing.clone()).expect("drawing");

        let (text, losses) = exchange::dxf(&drawing);
        assert!(text.contains("0\nPOLYLINE\n8\n0\n66\n1\n10\n0\n20\n0\n30\n0\n70\n1\n"), "closed POLYLINE header");
        assert_eq!(text.lines().filter(|line| line.trim() == "VERTEX").count(), 3);
        assert!(text.contains("0\nSEQEND\n8\n0\n"));
        assert!(!losses.iter().any(|loss| loss.code == "dxf/polyline-segments"), "polyline is no longer approximated");

        let (imported, _warnings) = import::dxf(&text, None).expect("import the exported DXF");
        let (points, closed) = imported
            .entities
            .iter()
            .find_map(|entity| match &entity.kind {
                EntityKind::Polyline { points, closed } => Some((points.clone(), *closed)),
                _ => None,
            })
            .expect("imported polyline");
        assert_eq!(points, vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]);
        assert!(closed);
        close(handle).expect("close");
    }

    #[test]
    fn layer_delete_removes_empty_layers_and_refuses_in_use() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_layer(handle, "Walls", None).expect("add layer");
        add_line(handle, "Walls", [0.0, 0.0], [10.0, 0.0]).expect("line");

        let error = delete_layer(handle, "Walls").expect_err("layer is in use");
        assert_eq!(error.code, "drawing/layer-in-use");
        assert!(error.message.contains("Walls"), "the message names the layer: {}", error.message);
        assert!(error.message.contains('1'), "the message counts the reference: {}", error.message);

        delete_entity(handle, "entity-1").expect("delete entity");
        let deleted = delete_layer(handle, "Walls").expect("delete empty layer");
        assert_eq!(deleted["deleted"], json!("Walls"));
        assert_eq!(deleted["drawing"]["layers"].as_array().map(Vec::len), Some(2));

        let restored = undo(handle).expect("undo the delete");
        assert_eq!(restored["drawing"]["layers"].as_array().map(Vec::len), Some(3));
        assert!(restored["drawing"]["layers"].as_array().is_some_and(|layers| layers.iter().any(|layer| layer["id"] == json!("Walls"))));
        let removed = redo(handle).expect("redo the delete");
        assert_eq!(removed["drawing"]["layers"].as_array().map(Vec::len), Some(2));

        delete_layer(handle, DIMENSION_LAYER).expect("delete the second layer");
        let last = delete_layer(handle, DEFAULT_LAYER).expect_err("the last layer stays");
        assert_eq!(last.code, "drawing/layer-last");
        assert_eq!(delete_layer(handle, "missing").expect_err("unknown layer").code, "object/not-found");
        close(handle).expect("close");
    }

    #[test]
    fn merge_layer_moves_references_and_removes_the_source() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_layer(handle, "Walls", None).expect("add Walls");
        add_layer(handle, "Structure", None).expect("add Structure");
        add_line(handle, "Walls", [0.0, 0.0], [10.0, 0.0]).expect("line");
        add_circle(handle, "Walls", [5.0, 5.0], 2.0).expect("circle");
        add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [0.0, 5.0]).expect("default line");

        let merged = merge_layer(handle, "Walls", "Structure").expect("merge");
        assert_eq!(merged["from"], json!("Walls"));
        assert_eq!(merged["to"], json!("Structure"));
        assert_eq!(merged["moved"], json!(2));
        assert_eq!(merged["drawing"]["layers"].as_array().map(Vec::len), Some(3));
        let layers = merged["drawing"]["layers"].as_array().expect("layers");
        assert!(layers.iter().all(|layer| layer["id"] != json!("Walls")));
        let structure = layers.iter().find(|layer| layer["id"] == json!("Structure")).expect("target layer");
        assert_eq!(structure["entityCount"], json!(2));

        let listed = list_entities(handle).expect("list");
        assert_eq!(listed["entities"][0]["layer"], json!("Structure"));
        assert_eq!(listed["entities"][1]["layer"], json!("Structure"));
        assert_eq!(listed["entities"][2]["layer"], json!(DEFAULT_LAYER));

        let restored = undo(handle).expect("undo the merge");
        assert!(restored["drawing"]["layers"].as_array().is_some_and(|layers| layers.iter().any(|layer| layer["id"] == json!("Walls"))));
        redo(handle).expect("redo the merge");

        assert_eq!(merge_layer(handle, "Walls", "Structure").expect_err("source is gone").code, "object/not-found");
        assert_eq!(merge_layer(handle, "Structure", "missing").expect_err("target is gone").code, "object/not-found");
        assert_eq!(merge_layer(handle, "Structure", "Structure").expect_err("same layer").code, "session/argument-invalid");
        update_layer(handle, "Structure", LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock");
        assert_eq!(merge_layer(handle, "Structure", DEFAULT_LAYER).expect_err("locked source").code, "drawing/layer-locked");
        assert_eq!(merge_layer(handle, DEFAULT_LAYER, "Structure").expect_err("locked target").code, "drawing/layer-locked");

        close(handle).expect("close");
    }

    #[test]
    fn convert_units_scales_geometry_and_round_trips() {
        let created = create(CreateRequest { units: Some("mm".to_string()), ..CreateRequest::default() }).expect("create");
        let handle = handle_of(&created);
        add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [25.4, 0.0]).expect("line");
        add_circle(handle, DEFAULT_LAYER, [25.4, 0.0], 12.7).expect("circle");
        add_text(handle, DEFAULT_LAYER, [0.0, 0.0], 2.54, "Hall").expect("text");
        add_dimension(handle, DIMENSION_LAYER, [0.0, 0.0], [25.4, 0.0], 2.54, None).expect("dimension");
        let revision = summary(handle).expect("summary")["revision"].as_u64().expect("revision");

        let inches = convert_units(handle, "inch").expect("convert to inches");
        assert_eq!(inches["from"], json!("mm"));
        assert_eq!(inches["to"], json!("inch"));
        assert_eq!(inches["converted"], json!(true));
        assert_eq!(inches["drawing"]["units"], json!("inch"));
        assert_eq!(inches["revision"], json!(revision + 1));
        let listed = list_entities(handle).expect("list");
        let entities = listed["entities"].as_array().expect("entities");
        let end = entities[0]["kind"]["to"][0].as_f64().expect("x");
        assert!((end - 1.0).abs() < 1e-12, "line length converts to inches: {end}");
        let radius = entities[1]["kind"]["radius"].as_f64().expect("radius");
        assert!((radius - 0.5).abs() < 1e-12, "circle radius converts to inches: {radius}");
        let height = entities[2]["kind"]["height"].as_f64().expect("height");
        assert!((height - 0.1).abs() < 1e-12, "text height converts to inches: {height}");
        let offset = entities[3]["kind"]["offset"].as_f64().expect("offset");
        assert!((offset - 0.1).abs() < 1e-12, "dimension offset scales: {offset}");

        let millimetres = convert_units(handle, "mm").expect("convert back");
        assert_eq!(millimetres["converted"], json!(true));
        let listed = list_entities(handle).expect("list");
        let entities = listed["entities"].as_array().expect("entities");
        let end = entities[0]["kind"]["to"][0].as_f64().expect("x");
        assert!((end - 25.4).abs() < 1e-9, "round trip returns the original length: {end}");
        let radius = entities[1]["kind"]["radius"].as_f64().expect("radius");
        assert!((radius - 12.7).abs() < 1e-9);

        let revision = summary(handle).expect("summary")["revision"].as_u64().expect("revision");
        let same = convert_units(handle, "mm").expect("same unit");
        assert_eq!(same["converted"], json!(false));
        assert_eq!(same["revision"], json!(revision), "a no-op conversion adds no revision");
        assert_eq!(convert_units(handle, "metre").expect_err("unknown unit").code, "session/argument-invalid");
        close(handle).expect("close");
    }

    #[test]
    fn duplicate_entity_copies_geometry_under_a_new_id() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 5.0]).expect("line");

        let copy = duplicate_entity(handle, "entity-1").expect("duplicate");
        assert_eq!(copy["entity"]["id"], json!("entity-2"));
        assert_eq!(copy["entity"]["layer"], json!(DEFAULT_LAYER));
        assert_eq!(copy["entity"]["kind"]["from"], json!([0.0, 0.0]));
        assert_eq!(copy["entity"]["kind"]["to"], json!([10.0, 5.0]));
        assert_eq!(copy["drawing"]["entities"], json!(2));
        assert_eq!(copy["revision"], json!(3));

        let listed = list_entities(handle).expect("list");
        assert_eq!(listed["entities"].as_array().map(Vec::len), Some(2));
        assert_eq!(listed["entities"][0]["kind"], listed["entities"][1]["kind"]);

        assert_eq!(duplicate_entity(handle, "entity-9").expect_err("unknown entity").code, "object/not-found");
        update_layer(handle, DEFAULT_LAYER, LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock");
        assert_eq!(duplicate_entity(handle, "entity-1").expect_err("locked layer").code, "drawing/layer-locked");
        close(handle).expect("close");
    }

    #[test]
    fn dxf_import_flattens_arcs_and_keeps_points() {
        let text = [
            "0",
            "SECTION",
            "2",
            "HEADER",
            "9",
            "$INSUNITS",
            "70",
            "4",
            "0",
            "ENDSEC",
            "0",
            "SECTION",
            "2",
            "TABLES",
            "0",
            "TABLE",
            "2",
            "LAYER",
            "70",
            "1",
            "0",
            "LAYER",
            "2",
            "ARC-LAYER",
            "70",
            "0",
            "62",
            "1",
            "0",
            "ENDTAB",
            "0",
            "ENDSEC",
            "0",
            "SECTION",
            "2",
            "ENTITIES",
            "0",
            "ARC",
            "8",
            "ARC-LAYER",
            "10",
            "0",
            "20",
            "0",
            "40",
            "10",
            "50",
            "0",
            "51",
            "90",
            "0",
            "POINT",
            "8",
            "ARC-LAYER",
            "10",
            "5",
            "20",
            "7",
            "0",
            "ENDSEC",
            "0",
            "EOF",
        ]
        .join("\n");

        let (drawing, losses) = import::dxf(&text, None).expect("import");
        assert_eq!(drawing.entities.len(), 2);
        let arc = &drawing.entities[0];
        match &arc.kind {
            EntityKind::Arc { center, radius, start_angle, end_angle } => {
                assert_eq!(*center, [0.0, 0.0]);
                assert!((*radius - 10.0).abs() < 1e-9);
                assert!((*start_angle - 0.0).abs() < 1e-9);
                assert!((*end_angle - 90.0).abs() < 1e-9);
            }
            other => panic!("ARC should stay analytic, got {other:?}"),
        }
        assert_eq!(arc.layer, "ARC-LAYER");
        assert!(matches!(drawing.entities[1].kind, EntityKind::Point { at } if at == [5.0, 7.0]));
        assert!(!losses.iter().any(|loss| loss.code == "dxf/flattened-arc"), "an analytic arc is not reported as lost: {losses:?}");
        assert!(!losses.iter().any(|loss| loss.code == "dxf/unsupported-entity"), "known entities are not skipped");
    }

    #[test]
    fn dxf_import_scales_metres_into_millimetres() {
        let text = [
            "0",
            "SECTION",
            "2",
            "HEADER",
            "9",
            "$INSUNITS",
            "70",
            "6",
            "0",
            "ENDSEC",
            "0",
            "SECTION",
            "2",
            "ENTITIES",
            "0",
            "LINE",
            "8",
            "0",
            "10",
            "1",
            "20",
            "0",
            "11",
            "2",
            "21",
            "0.5",
            "0",
            "ENDSEC",
            "0",
            "EOF",
        ]
        .join("\n");

        let (drawing, losses) = import::dxf(&text, None).expect("import");
        assert_eq!(drawing.units, Units::Mm);
        let EntityKind::Line { from, to } = &drawing.entities[0].kind
        else {
            panic!("LINE should stay a line");
        };
        assert_eq!(*from, [1000.0, 0.0]);
        assert_eq!(*to, [2000.0, 500.0]);
        assert!(
            losses.iter().any(|loss| loss.code == "dxf/assumed-units" && loss.detail.contains("metres")),
            "the metre conversion is stated: {losses:?}"
        );
    }

    #[test]
    fn dxf_import_flattens_ellipses() {
        let text = [
            "0",
            "SECTION",
            "2",
            "ENTITIES",
            "0",
            "ELLIPSE",
            "8",
            "0",
            "10",
            "0",
            "20",
            "0",
            "11",
            "10",
            "21",
            "0",
            "40",
            "0.5",
            "41",
            "0",
            "42",
            "6.283185307179586",
            "0",
            "ENDSEC",
            "0",
            "EOF",
        ]
        .join("\n");

        let (drawing, losses) = import::dxf(&text, None).expect("import");
        assert_eq!(drawing.entities.len(), 1);
        let EntityKind::Ellipse { center, rx, ry, rotation, start_angle, end_angle } = &drawing.entities[0].kind
        else {
            panic!("ELLIPSE should stay analytic");
        };
        assert_eq!(*center, [0.0, 0.0]);
        assert!((*rx - 10.0).abs() < 1e-9);
        assert!((*ry - 5.0).abs() < 1e-9);
        assert!(rotation.abs() < 1e-9);
        assert!(start_angle.abs() < 1e-9);
        assert!((*end_angle - 360.0).abs() < 1e-9);
        assert!(!losses.iter().any(|loss| loss.code == "dxf/flattened-ellipse"), "an analytic ellipse is not reported as lost: {losses:?}");
    }

    #[test]
    fn dxf_import_expands_blocks_with_insert_scale_and_rotation() {
        let text = [
            "0", "SECTION", "2", "BLOCKS", "0", "BLOCK", "8", "0", "2", "DOOR", "70", "0", "10", "0", "20", "0", "0", "LINE", "8", "0", "10",
            "0", "20", "0", "11", "1", "21", "0", "0", "ENDBLK", "8", "0", "0", "ENDSEC", "0", "SECTION", "2", "ENTITIES", "0", "INSERT", "8",
            "WALLS", "2", "DOOR", "10", "10", "20", "20", "41", "2", "42", "2", "50", "90", "0", "ENDSEC", "0", "EOF",
        ]
        .join("\n");

        let (drawing, losses) = import::dxf(&text, None).expect("import");
        assert_eq!(drawing.entities.len(), 1);
        let entity = &drawing.entities[0];
        assert_eq!(entity.layer, "WALLS");
        match &entity.kind {
            EntityKind::Line { from, to } => {
                assert!((from[0] - 10.0).abs() < 1e-9 && (from[1] - 20.0).abs() < 1e-9, "base point lands on the insertion: {from:?}");
                assert!((to[0] - 10.0).abs() < 1e-9 && (to[1] - 22.0).abs() < 1e-9, "scale and rotation apply: {to:?}");
            }
            other => panic!("the block LINE should expand, got {other:?}"),
        }
        assert!(
            losses.iter().any(|loss| loss.code == "dxf/expanded-block" && loss.severity == "notice"),
            "the expansion is a notice: {losses:?}"
        );
        assert!(losses.iter().any(|loss| loss.code == "dxf/implicit-layer"));
    }

    #[test]
    fn dxf_import_stops_recursive_blocks_at_the_depth_limit() {
        let text = [
            "0", "SECTION", "2", "BLOCKS", "0", "BLOCK", "8", "0", "2", "LOOP", "70", "0", "10", "0", "20", "0", "0", "INSERT", "8", "0", "2",
            "LOOP", "10", "0", "20", "0", "0", "ENDBLK", "8", "0", "0", "ENDSEC", "0", "SECTION", "2", "ENTITIES", "0", "INSERT", "8", "0",
            "2", "LOOP", "10", "0", "20", "0", "0", "ENDSEC", "0", "EOF",
        ]
        .join("\n");

        let (drawing, losses) = import::dxf(&text, None).expect("import");
        assert!(drawing.entities.is_empty(), "a self-referencing block expands no geometry");
        assert!(
            losses.iter().any(|loss| loss.code == "dxf/block-recursion" && loss.severity == "warning"),
            "recursion is reported: {losses:?}"
        );
        assert!(losses.iter().any(|loss| loss.code == "dxf/expanded-block"));
    }

    #[test]
    fn dxf_import_turns_lwpolyline_bulges_into_arcs() {
        let text = [
            "0",
            "SECTION",
            "2",
            "ENTITIES",
            "0",
            "LWPOLYLINE",
            "8",
            "0",
            "90",
            "2",
            "70",
            "0",
            "10",
            "0",
            "20",
            "0",
            "42",
            "1",
            "10",
            "10",
            "20",
            "0",
            "0",
            "ENDSEC",
            "0",
            "EOF",
        ]
        .join("\n");

        let (drawing, losses) = import::dxf(&text, None).expect("import");
        assert_eq!(drawing.entities.len(), 1);
        let EntityKind::Polyline { points, closed } = &drawing.entities[0].kind
        else {
            panic!("LWPOLYLINE should stay a polyline");
        };
        assert!(!closed);
        assert!(points.len() >= 10, "the bulge is sampled: {}", points.len());
        assert!(points.iter().any(|point| point[1].abs() > 1e-6), "the arc leaves the chord: {points:?}");
        assert!(
            losses.iter().any(|loss| loss.code == "dxf/assumed-units" && loss.severity == "notice"),
            "a missing header is a notice, not a silent default: {losses:?}"
        );
    }

    #[test]
    fn stl_export_extrudes_a_closed_profile() {
        let dir = TempDir::new("stl");
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [4.0, 0.0], [4.0, 2.0], [2.0, 2.0], [2.0, 4.0], [0.0, 4.0]], true)
            .expect("closed L profile");

        let path = dir.path.join("solid.stl");
        let report = export(handle, "stl", &path).expect("export stl");
        assert_eq!(report["format"], json!("stl"));
        assert_eq!(report["editable"], json!(false));
        assert_eq!(report["triangles"], json!(20), "4 cap triangles per side and 6 wall quads");
        assert!(report["losses"].as_array().is_some_and(Vec::is_empty));
        let stl = fs::read_to_string(&path).expect("read stl");
        assert_eq!(report["bytes"], json!(stl.len()));
        assert!(stl.starts_with("solid open-cad\n"));
        assert!(stl.trim_end().ends_with("endsolid"));
        assert_eq!(stl.matches("facet normal").count(), 20);
        assert_eq!(stl.matches("vertex").count(), 60);

        let thick = dir.path.join("thick.stl");
        let thick_report = export_with_options(handle, "stl", &thick, ExportOptions { thickness: Some(2.5) }).expect("export thick stl");
        assert_eq!(thick_report["triangles"], json!(20));
        let thick_text = fs::read_to_string(&thick).expect("read thick stl");
        assert!(thick_text.contains("2.5\n"), "the configured thickness reaches the vertices");
        close(handle).expect("close");
    }

    #[test]
    fn stl_export_reports_profiles_it_cannot_extrude() {
        let dir = TempDir::new("stl-open");
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0]], false).expect("open polyline");

        let path = dir.path.join("open.stl");
        let report = export(handle, "stl", &path).expect("export stl");
        assert_eq!(report["triangles"], json!(0));
        let losses = report["losses"].as_array().expect("losses");
        assert_eq!(losses.len(), 1);
        assert_eq!(losses[0]["code"], json!("stl/open-profile"));
        assert_eq!(losses[0]["severity"], json!("warning"));
        assert!(losses[0]["entityId"].as_str().is_some_and(|id| id.starts_with("entity-")));
        let stl = fs::read_to_string(&path).expect("read stl");
        assert!(stl.starts_with("solid open-cad\n"));
        assert!(stl.trim_end().ends_with("endsolid"));
        close(handle).expect("close");
    }

    #[test]
    fn obj_export_writes_a_triangulated_extrusion() {
        let dir = TempDir::new("obj");
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]], true).expect("closed square");
        add_polyline(handle, DEFAULT_LAYER, vec![[10.0, 0.0], [12.0, 0.0], [12.0, 2.0]], false).expect("open polyline");

        let path = dir.path.join("solid.obj");
        let report = export(handle, "obj", &path).expect("export obj");
        assert_eq!(report["format"], json!("obj"));
        assert_eq!(report["editable"], json!(false));
        assert_eq!(report["triangles"], json!(12), "2 cap triangles per side and 4 wall quads");
        let losses = report["losses"].as_array().expect("losses");
        assert_eq!(losses.len(), 1);
        assert_eq!(losses[0]["code"], json!("obj/open-profile"));
        let obj = fs::read_to_string(&path).expect("read obj");
        assert!(obj.starts_with("# OpenCAD OBJ extrusion\n"));
        assert_eq!(obj.matches("\nv ").count(), 36, "three vertices per triangle");
        assert_eq!(obj.matches("\nf ").count(), 12);
        assert!(obj.contains("f 1 2 3"));
        assert!(obj.trim_end().ends_with("# end of OpenCAD OBJ"));
        close(handle).expect("close");
    }

    #[test]
    fn step_export_writes_a_faceted_brep_for_a_closed_profile() {
        let dir = TempDir::new("step");
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [4.0, 0.0], [4.0, 2.0], [0.0, 2.0]], true).expect("closed rectangle");

        let path = dir.path.join("solid.step");
        let report = export(handle, "step", &path).expect("export step");
        assert_eq!(report["format"], json!("step"));
        assert_eq!(report["editable"], json!(false));
        assert_eq!(report["faces"], json!(12), "two cap triangles per side plus four wall quads");
        assert!(report["losses"].as_array().is_some_and(Vec::is_empty));

        let step = fs::read_to_string(&path).expect("read step");
        assert_eq!(report["bytes"], json!(step.len()));
        assert!(step.starts_with("ISO-10303-21;\n"), "the header must open the exchange structure");
        assert!(step.trim_end().ends_with("END-ISO-10303-21;"));
        assert!(step.contains("HEADER;"));
        assert!(step.contains("FILE_SCHEMA(('CONFIG_CONTROL_DESIGN'));"));
        assert!(step.contains("MANIFOLD_SOLID_BREP"));
        assert!(step.contains("CLOSED_SHELL"));
        assert!(step.contains("FACE_OUTER_BOUND"));
        assert_eq!(step.matches("ADVANCED_FACE").count(), 12);
        assert_eq!(step.matches("POLY_LOOP").count(), 12);
        assert_eq!(step.matches("CARTESIAN_POINT").count(), 37, "three vertices per face plus the shape origin");

        let second = dir.path.join("again.step");
        export(handle, "step", &second).expect("export step again");
        assert_eq!(fs::read(&path).expect("first"), fs::read(&second).expect("second"), "the projection is deterministic");
        close(handle).expect("close");
    }

    #[test]
    fn step_export_keeps_disjoint_profiles_as_separate_solids() {
        let dir = TempDir::new("step-disjoint");
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]], true).expect("first rectangle");
        add_polyline(handle, DEFAULT_LAYER, vec![[10.0, 0.0], [12.0, 0.0], [12.0, 2.0], [10.0, 2.0]], true).expect("second rectangle");

        let path = dir.path.join("pair.step");
        let report = export(handle, "step", &path).expect("export step");
        assert_eq!(report["faces"], json!(24));

        let step = fs::read_to_string(&path).expect("read step");
        assert_eq!(step.matches("MANIFOLD_SOLID_BREP").count(), 2, "each disjoint profile is its own solid");
        assert_eq!(step.matches("CLOSED_SHELL").count(), 2);
        assert_eq!(step.matches("ADVANCED_FACE").count(), 24);
        assert!(step.trim_end().ends_with("END-ISO-10303-21;"));
        close(handle).expect("close");
    }

    #[test]
    fn step_export_skips_open_profiles_instead_of_faking_a_solid() {
        let dir = TempDir::new("step-open");
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0]], false).expect("open polyline");

        let path = dir.path.join("open.step");
        let report = export(handle, "step", &path).expect("export step");
        assert_eq!(report["faces"], json!(0));
        let losses = report["losses"].as_array().expect("losses");
        assert_eq!(losses.len(), 1);
        assert_eq!(losses[0]["code"], json!("step/open-profile"));
        assert_eq!(losses[0]["severity"], json!("warning"));
        assert!(losses[0]["entityId"].as_str().is_some_and(|id| id.starts_with("entity-")));

        let step = fs::read_to_string(&path).expect("read step");
        assert!(step.starts_with("ISO-10303-21;\n"));
        assert!(step.trim_end().ends_with("END-ISO-10303-21;"));
        assert!(step.contains("SHAPE_REPRESENTATION"));
        assert!(step.contains("PRODUCT_DEFINITION_SHAPE"));
        assert!(!step.contains("MANIFOLD_SOLID_BREP"), "an open profile must not become a fabricated solid");
        assert!(!step.contains("CLOSED_SHELL"));
        close(handle).expect("close");
    }

    #[test]
    fn arc_and_ellipse_round_trip_and_reject_invalid_values() {
        let dir = TempDir::new("arc-ellipse");
        let path = dir.path.join("curves.ocad");
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);

        let arc = add_arc(handle, DEFAULT_LAYER, [1.0, 2.0], 5.0, 30.0, 120.0).expect("add arc");
        assert_eq!(arc["entity"]["kind"]["type"], json!("arc"));
        let ellipse = add_ellipse(handle, DEFAULT_LAYER, [0.0, 0.0], 8.0, 3.0, 15.0, 0.0, 360.0).expect("add ellipse");
        assert_eq!(ellipse["entity"]["kind"]["type"], json!("ellipse"));

        save(handle, &path, true).expect("save");
        let text = fs::read_to_string(&path).expect("read saved");
        let document: Value = serde_json::from_str(&text).expect("parse saved");
        assert_eq!(document["document"]["entities"][0]["kind"]["type"], json!("arc"));
        assert_eq!(document["document"]["entities"][1]["kind"]["type"], json!("ellipse"));
        close(handle).expect("close");

        let reopened = open(&path).expect("reopen");
        let reopened_handle = handle_of(&reopened);
        let drawing = drawing_of(reopened_handle);
        assert!(matches!(
            drawing.entities[0].kind,
            EntityKind::Arc { radius, start_angle, end_angle, .. } if (radius - 5.0).abs() < 1e-12 && (start_angle - 30.0).abs() < 1e-12 && (end_angle - 120.0).abs() < 1e-12
        ));
        assert!(matches!(
            drawing.entities[1].kind,
            EntityKind::Ellipse { rx, ry, rotation, .. } if (rx - 8.0).abs() < 1e-12 && (ry - 3.0).abs() < 1e-12 && (rotation - 15.0).abs() < 1e-12
        ));
        close(reopened_handle).expect("close reopened");

        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        assert_eq!(add_arc(handle, DEFAULT_LAYER, [0.0, 0.0], 0.0, 0.0, 90.0).expect_err("zero radius").code, "session/argument-invalid");
        assert_eq!(add_arc(handle, DEFAULT_LAYER, [0.0, 0.0], 1.0, f64::NAN, 90.0).expect_err("nan angle").code, "session/argument-invalid");
        assert_eq!(
            add_ellipse(handle, DEFAULT_LAYER, [0.0, 0.0], 1.0, -2.0, 0.0, 0.0, 360.0).expect_err("negative ry").code,
            "session/argument-invalid"
        );
        close(handle).expect("close");

        let kind: EntityKind = serde_json::from_value(json!({
            "type": "arc", "center": [0.0, 0.0], "radius": 4.0, "start_angle": 0.0, "end_angle": 90.0
        }))
        .expect("parse arc kind");
        assert_eq!(
            serde_json::to_value(&kind).expect("serialize"),
            json!({ "type": "arc", "center": [0.0, 0.0], "radius": 4.0, "start_angle": 0.0, "end_angle": 90.0 })
        );
        let invalid: Drawing = serde_json::from_value(json!({
            "id": "drawing-1",
            "title": null,
            "revision": 1,
            "units": "mm",
            "layers": [{ "id": "0", "name": "0", "color": "#d0d7de", "visible": true, "locked": false }],
            "entities": [{ "id": "entity-1", "layer": "0", "kind": { "type": "arc", "center": [0.0, 0.0], "radius": -1.0, "start_angle": 0.0, "end_angle": 90.0 } }],
            "next_entity": 2
        }))
        .expect("parse drawing");
        assert!(invalid.validate().is_err_and(|message| message.contains("non-positive radius")));
    }

    #[test]
    fn svg_arc_path_samples_the_radius() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_arc(handle, DEFAULT_LAYER, [0.0, 0.0], 10.0, 0.0, 90.0).expect("add arc");
        let svg = store::read(handle, |session| exchange::svg(&session.drawing)).expect("render");
        assert!(svg.contains("<path"), "the arc is a path: {svg}");
        assert!(svg.matches(" L").count() >= 2, "the quarter arc is sampled into segments: {svg}");
        assert!(!svg.contains(" A "), "the arc keeps the shared writer's sampled path: {svg}");
        close(handle).expect("close");
    }

    #[test]
    fn dxf_arc_round_trip_preserves_centre_radius_and_angles() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_arc(handle, DEFAULT_LAYER, [1.5, 2.25], 10.5, 30.0, 120.0).expect("add arc");
        let (text, _) = store::read(handle, |session| exchange::dxf(&session.drawing)).expect("export");
        assert!(text.contains("ARC"));
        let (imported, losses) = import::dxf(&text, None).expect("import");
        let EntityKind::Arc { center, radius, start_angle, end_angle } = imported.entities[0].kind
        else {
            panic!("ARC should import as an arc");
        };
        assert!((center[0] - 1.5).abs() < 1e-9 && (center[1] - 2.25).abs() < 1e-9);
        assert!((radius - 10.5).abs() < 1e-9);
        assert!((start_angle - 30.0).abs() < 1e-9);
        assert!((end_angle - 120.0).abs() < 1e-9);
        assert!(losses.iter().all(|loss| loss.code != "dxf/flattened-arc"));
        close(handle).expect("close");
    }

    #[test]
    fn rotation_transform_shifts_arc_angles() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_arc(handle, DEFAULT_LAYER, [0.0, 0.0], 5.0, 0.0, 90.0).expect("add arc");
        let result = transform(
            handle,
            TransformRequest {
                sheet: None,
                target: TransformTarget::Ids(vec!["entity-1".to_string()]),
                operation: TransformOp::Rotate { angle: 45.0, cx: 0.0, cy: 0.0 },
            },
        )
        .expect("transform");
        assert_eq!(result["affected"], json!(1));
        let drawing = drawing_of(handle);
        assert!(matches!(
            drawing.entities[0].kind,
            EntityKind::Arc { start_angle, end_angle, .. } if (start_angle - 45.0).abs() < 1e-12 && (end_angle - 135.0).abs() < 1e-12
        ));
        close(handle).expect("close");
    }

    #[test]
    fn copy_creates_the_requested_number_of_new_entities() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("add line");
        let result = copy(handle, CopyRequest { ids: vec!["entity-1".to_string()], dx: 10.0, dy: 0.0, count: 3 }).expect("copy");
        let created_ids =
            result["created"].as_array().expect("created").iter().map(|id| id.as_str().unwrap_or_default().to_string()).collect::<Vec<_>>();
        assert_eq!(created_ids.len(), 3);
        assert_eq!(created_ids.iter().collect::<std::collections::HashSet<_>>().len(), 3, "ids stay distinct");
        assert_eq!(result["drawing"]["entities"], json!(4));
        assert_eq!(result["revision"], json!(3));
        let drawing = drawing_of(handle);
        assert!(matches!(drawing.entities[3].kind, EntityKind::Line { from, .. } if from == [30.0, 0.0]));
        close(handle).expect("close");
    }

    #[test]
    fn measure_reports_the_exact_box_and_area_of_a_closed_square() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]], true).expect("add square");
        let measured = measure(handle, None).expect("measure");
        assert_eq!(measured["bounds"], json!({ "minX": 0.0, "minY": 0.0, "maxX": 10.0, "maxY": 10.0 }));
        assert_eq!(measured["width"], json!(10.0));
        assert_eq!(measured["height"], json!(10.0));
        assert_eq!(measured["entities"], json!(1));
        assert_eq!(measured["area"], json!(100.0));
        assert_eq!(measured["length"], json!(40.0));
        close(handle).expect("close");
    }

    #[test]
    fn measure_entities_reports_closed_polyline_area_and_perimeter() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]], true).expect("square");
        add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [3.0, 4.0]).expect("line");

        let measured =
            measure_entities(handle, &["entity-1".to_string(), "entity-2".to_string(), "entity-1".to_string()]).expect("measure entities");
        assert_eq!(measured["totals"], json!({ "entities": 2, "length": 45.0, "area": 100.0 }));
        assert_eq!(measured["entities"][0]["kind"], json!("polyline"));
        assert_eq!(measured["entities"][0]["closed"], json!(true));
        assert_eq!(measured["entities"][0]["length"], json!(40.0));
        assert_eq!(measured["entities"][0]["area"], json!(100.0));
        assert_eq!(measured["entities"][1]["closed"], json!(false));
        assert_eq!(measured["entities"][1]["length"], json!(5.0));
        assert_eq!(measured["entities"][1]["area"], json!(0.0));

        assert_eq!(measure_entities(handle, &[]).expect_err("empty ids").code, "session/argument-invalid");
        assert_eq!(measure_entities(handle, &["entity-9".to_string()]).expect_err("unknown id").code, "object/not-found");
        close(handle).expect("close");
    }

    #[test]
    fn measure_points_reports_distance_and_angle() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);

        let distance = measure_points(handle, &[[0.0, 0.0], [3.0, 4.0]]).expect("distance");
        assert_eq!(distance["kind"], json!("distance"));
        assert_eq!(distance["points"], json!(2));
        assert_eq!(distance["distance"], json!(5.0));
        assert_eq!(distance["dx"], json!(3.0));
        assert_eq!(distance["dy"], json!(4.0));
        assert_eq!(distance["bearing"], json!(53.13010235415598));

        let angle = measure_points(handle, &[[0.0, 10.0], [0.0, 0.0], [5.0, 0.0]]).expect("angle");
        assert_eq!(angle["kind"], json!("angle"));
        assert_eq!(angle["points"], json!(3));
        assert_eq!(angle["vertex"], json!([0.0, 0.0]));
        assert_eq!(angle["angle"], json!(90.0));
        assert_eq!(angle["first"], json!(10.0));
        assert_eq!(angle["second"], json!(5.0));
        assert_eq!(angle["span"], json!(11.180339887498949));

        assert_eq!(measure_points(handle, &[[0.0, 0.0]]).expect_err("one point").code, "session/argument-invalid");
        assert_eq!(measure_points(handle, &[[0.0, 0.0], [0.0, 0.0], [1.0, 1.0]]).expect_err("degenerate").code, "session/argument-invalid");
        close(handle).expect("close");
    }

    #[test]
    fn geometry_check_flags_a_zero_length_line() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_line(handle, DEFAULT_LAYER, [1.0, 1.0], [1.0, 1.0]).expect("add degenerate line");
        let report = inspect(handle, Some(vec!["geometry".to_string()])).expect("inspect");
        let codes =
            report["issues"].as_array().expect("issues").iter().map(|issue| issue["code"].as_str().unwrap_or_default()).collect::<Vec<_>>();
        assert!(codes.contains(&"geometry/zero-length-line"), "got {codes:?}");
        close(handle).expect("close");
    }

    #[test]
    fn array_fills_a_rectangular_grid_in_row_major_order() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [1.0, 0.0]).expect("line");
        let result = array(
            handle,
            ArrayRequest {
                ids: vec!["entity-1".to_string()],
                mode: ArrayMode::Rectangular { rows: 2, columns: 2, row_spacing: 5.0, column_spacing: 10.0 },
            },
        )
        .expect("array");
        assert_eq!(result["created"], json!(["entity-2", "entity-3", "entity-4"]));
        assert_eq!(result["count"], json!(3));
        assert_eq!(result["drawing"]["entities"], json!(4));
        assert_eq!(result["revision"], json!(3));
        let drawing = drawing_of(handle);
        assert!(matches!(drawing.entities[1].kind, EntityKind::Line { from: [10.0, 0.0], to: [11.0, 0.0] }));
        assert!(matches!(drawing.entities[2].kind, EntityKind::Line { from: [0.0, 5.0], to: [1.0, 5.0] }));
        assert!(matches!(drawing.entities[3].kind, EntityKind::Line { from: [10.0, 5.0], to: [11.0, 5.0] }));
        close(handle).expect("close");
    }

    #[test]
    fn array_places_polar_copies_counter_clockwise() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_line(handle, DEFAULT_LAYER, [10.0, 0.0], [20.0, 0.0]).expect("line");
        let result = array(
            handle,
            ArrayRequest { ids: vec!["entity-1".to_string()], mode: ArrayMode::Polar { center: [0.0, 0.0], count: 4, total_angle: 360.0 } },
        )
        .expect("array");
        assert_eq!(result["created"], json!(["entity-2", "entity-3", "entity-4"]));
        assert_eq!(result["count"], json!(3));
        let drawing = drawing_of(handle);
        let starts = drawing
            .entities
            .iter()
            .map(|entity| match entity.kind {
                EntityKind::Line { from, .. } => from,
                _ => panic!("every array item is a line"),
            })
            .collect::<Vec<_>>();
        assert!((starts[1][0]).abs() < 1e-9 && (starts[1][1] - 10.0).abs() < 1e-9, "{:?}", starts[1]);
        assert!((starts[2][0] + 10.0).abs() < 1e-9 && starts[2][1].abs() < 1e-9, "{:?}", starts[2]);
        assert!(starts[3][0].abs() < 1e-9 && (starts[3][1] + 10.0).abs() < 1e-9, "{:?}", starts[3]);
        close(handle).expect("close");
    }

    #[test]
    fn array_and_explode_validate_their_arguments() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [1.0, 0.0]).expect("line");
        let rectangular = |rows, columns, row_spacing, column_spacing| ArrayRequest {
            ids: vec!["entity-1".to_string()],
            mode: ArrayMode::Rectangular { rows, columns, row_spacing, column_spacing },
        };
        assert_eq!(
            array(handle, ArrayRequest { ids: Vec::new(), mode: ArrayMode::Polar { center: [0.0, 0.0], count: 2, total_angle: 90.0 } })
                .expect_err("empty ids")
                .code,
            "session/argument-invalid"
        );
        assert_eq!(array(handle, rectangular(1, 1, 1.0, 1.0)).expect_err("one position").code, "session/argument-invalid");
        assert_eq!(array(handle, rectangular(2, 2, f64::NAN, 1.0)).expect_err("nan spacing").code, "session/argument-invalid");
        let polar = |count, total_angle| ArrayRequest {
            ids: vec!["entity-1".to_string()],
            mode: ArrayMode::Polar { center: [0.0, 0.0], count, total_angle },
        };
        assert_eq!(array(handle, polar(1, 90.0)).expect_err("one item").code, "session/argument-invalid");
        assert_eq!(array(handle, polar(2, 0.0)).expect_err("zero angle").code, "session/argument-invalid");
        assert_eq!(
            array(handle, ArrayRequest { ids: vec!["entity-9".to_string()], mode: polar(2, 90.0).mode }).expect_err("unknown id").code,
            "object/not-found"
        );
        assert_eq!(explode(handle, "entity-9").expect_err("unknown id").code, "object/not-found");
        assert_eq!(explode(handle, "entity-1").expect_err("line is not a polyline").code, "object/unsupported");
        add_polyline(handle, DEFAULT_LAYER, vec![[1.0, 1.0]], false).expect("one-point polyline");
        assert_eq!(explode(handle, "entity-2").expect_err("degenerate polyline").code, "session/argument-invalid");
        update_layer(handle, DEFAULT_LAYER, LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock");
        assert_eq!(array(handle, rectangular(2, 2, 1.0, 1.0)).expect_err("locked array").code, "drawing/layer-locked");
        assert_eq!(explode(handle, "entity-2").expect_err("locked explode").code, "drawing/layer-locked");
        close(handle).expect("close");
    }

    #[test]
    fn explode_replaces_a_closed_polyline_with_its_edges() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]], true).expect("square");
        add_line(handle, DEFAULT_LAYER, [50.0, 50.0], [60.0, 60.0]).expect("marker");
        let result = explode(handle, "entity-1").expect("explode");
        assert_eq!(result["created"], json!(["entity-3", "entity-4", "entity-5", "entity-6"]));
        assert_eq!(result["count"], json!(4));
        assert_eq!(result["entity"], json!("entity-1"));
        assert_eq!(result["drawing"]["entities"], json!(5));
        assert_eq!(result["revision"], json!(4));
        let drawing = drawing_of(handle);
        assert!(matches!(drawing.entities[0].kind, EntityKind::Line { from: [0.0, 0.0], to: [10.0, 0.0] }));
        assert!(matches!(drawing.entities[3].kind, EntityKind::Line { from: [0.0, 10.0], to: [0.0, 0.0] }));
        assert!(matches!(drawing.entities[4].kind, EntityKind::Line { from: [50.0, 50.0], to: [60.0, 60.0] }));
        close(handle).expect("close");
    }

    #[test]
    fn dedicated_inspection_rules_flag_duplicates_and_zero_length() {
        let mut drawing = Drawing::empty(None, Units::Mm);
        drawing.entities.push(Entity {
            id: "entity-1".to_string(),
            layer: DEFAULT_LAYER.to_string(),
            kind: EntityKind::Line { from: [0.0, 0.0], to: [10.0, 0.0] },
        });
        drawing.entities.push(Entity {
            id: "entity-2".to_string(),
            layer: DEFAULT_LAYER.to_string(),
            kind: EntityKind::Line { from: [0.0, 0.0], to: [10.0, 0.0] },
        });
        drawing.entities.push(Entity {
            id: "entity-3".to_string(),
            layer: DEFAULT_LAYER.to_string(),
            kind: EntityKind::Line { from: [1.0, 1.0], to: [1.0, 1.0] },
        });
        drawing.entities.push(Entity {
            id: "entity-4".to_string(),
            layer: DEFAULT_LAYER.to_string(),
            kind: EntityKind::Polyline { points: vec![[2.0, 2.0], [2.0, 2.0]], closed: false },
        });
        drawing.next_entity = 5;
        let report = inspection::run(&drawing, &["duplicate-entity", "zero-length"]).to_json();
        let issues = report["issues"].as_array().expect("issues");
        let codes = issues.iter().map(|issue| issue["code"].as_str().unwrap_or_default()).collect::<Vec<_>>();
        assert!(codes.contains(&"inspection/duplicate-entity"), "got {codes:?}");
        assert_eq!(codes.iter().filter(|code| **code == "inspection/zero-length").count(), 2, "got {codes:?}");
        assert!(codes.iter().all(|code| !code.starts_with("geometry/")), "only the requested selectors ran: {codes:?}");
        let duplicate = issues.iter().find(|issue| issue["code"] == json!("inspection/duplicate-entity")).expect("duplicate issue");
        assert_eq!(duplicate["objectId"], json!("entity-2"));
        assert_eq!(duplicate["severity"], json!("warning"));
        assert_eq!(report["passed"], json!(false), "a zero-length entity is an error");
    }

    #[test]
    fn open_contour_check_flags_chains_that_do_not_close() {
        let entity = |id: &str, kind| Entity { id: id.to_string(), layer: DEFAULT_LAYER.to_string(), kind };
        let mut drawing = Drawing::empty(None, Units::Mm);
        drawing.entities.push(entity("entity-1", EntityKind::Line { from: [0.0, 0.0], to: [10.0, 0.0] }));
        drawing.entities.push(entity("entity-2", EntityKind::Line { from: [10.0, 0.0], to: [10.0, 10.0] }));
        drawing.entities.push(entity("entity-3", EntityKind::Line { from: [10.0, 10.0], to: [0.0, 10.0] }));
        drawing.entities.push(entity("entity-4", EntityKind::Line { from: [0.0, 10.0], to: [0.0, 0.0] }));
        drawing
            .entities
            .push(entity("entity-5", EntityKind::Polyline { points: vec![[20.0, 0.0], [24.0, 0.0], [24.0, 4.0], [20.0, 4.0]], closed: true }));
        let closed = inspection::run(&drawing, &["open-contour"]).to_json();
        assert_eq!(closed["issues"], json!([]), "closed loops are not flagged");
        assert_eq!(closed["passed"], json!(true));

        drawing.entities.push(entity("entity-6", EntityKind::Line { from: [40.0, 0.0], to: [45.0, 0.0] }));
        drawing.entities.push(entity("entity-7", EntityKind::Line { from: [45.0, 0.0], to: [45.0, 5.0] }));
        drawing.entities.push(entity("entity-8", EntityKind::Line { from: [45.0, 5.0], to: [40.0, 5.0] }));
        drawing.entities.push(entity("entity-9", EntityKind::Line { from: [60.0, 0.0], to: [65.0, 0.0] }));
        let report = inspection::run(&drawing, &["open-contour"]).to_json();
        let issues = report["issues"].as_array().expect("issues");
        let codes = issues.iter().map(|issue| issue["code"].as_str().unwrap_or_default()).collect::<Vec<_>>();
        assert_eq!(codes, ["inspection/open-contour"], "one open chain and one isolated line that is not a contour");
        assert_eq!(issues[0]["objectId"], json!("entity-6"));
        assert_eq!(issues[0]["severity"], json!("warning"));
        assert_eq!(report["passed"], json!(true), "an open contour is a warning");
    }

    #[test]
    fn dxf_export_import_round_trip_keeps_core_geometry() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("line");
        add_circle(handle, DEFAULT_LAYER, [5.0, 5.0], 2.0).expect("circle");
        add_arc(handle, DEFAULT_LAYER, [1.0, 2.0], 3.0, 30.0, 120.0).expect("arc");
        add_text(handle, DEFAULT_LAYER, [3.0, 4.0], 2.5, "Site").expect("text");
        add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]], true).expect("polyline");
        let drawing = drawing_of(handle);
        let (text, _) = exchange::dxf(&drawing);
        for marker in ["$INSUNITS", "TABLES", "LAYER", "ENTITIES", "LINE", "CIRCLE", "ARC", "TEXT", "POLYLINE"] {
            assert!(text.contains(marker), "export writes `{marker}`");
        }

        let (imported, _) = import::dxf(&text, None).expect("round trip import");
        assert_eq!(imported.entities.len(), 5);
        assert_eq!(imported.units, Units::Mm);
        let mut kinds = imported.entities.iter().map(|entity| kind_name(&entity.kind)).collect::<Vec<_>>();
        kinds.sort_unstable();
        assert_eq!(kinds, vec!["arc", "circle", "line", "polyline", "text"]);
        let line = imported.entities.iter().find_map(|entity| match &entity.kind {
            EntityKind::Line { from, to } => Some((*from, *to)),
            _ => None,
        });
        assert_eq!(line, Some(([0.0, 0.0], [10.0, 0.0])));
        let circle = imported.entities.iter().find_map(|entity| match &entity.kind {
            EntityKind::Circle { center, radius } => Some((*center, *radius)),
            _ => None,
        });
        assert_eq!(circle, Some(([5.0, 5.0], 2.0)));
        let arc = imported.entities.iter().find_map(|entity| match &entity.kind {
            EntityKind::Arc { center, radius, start_angle, end_angle } => Some((*center, *radius, *start_angle, *end_angle)),
            _ => None,
        });
        assert!(arc.is_some_and(|(center, radius, start_angle, end_angle)| {
            center == [1.0, 2.0] && (radius - 3.0).abs() < 1e-9 && (start_angle - 30.0).abs() < 1e-9 && (end_angle - 120.0).abs() < 1e-9
        }));
        let text = imported.entities.iter().find_map(|entity| match &entity.kind {
            EntityKind::Text { at, height, content } => Some((*at, *height, content.clone())),
            _ => None,
        });
        assert_eq!(text, Some(([3.0, 4.0], 2.5, "Site".to_string())));
        let polyline = imported.entities.iter().find_map(|entity| match &entity.kind {
            EntityKind::Polyline { points, closed } => Some((points.clone(), *closed)),
            _ => None,
        });
        assert_eq!(polyline, Some((vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]], true)));
        close(handle).expect("close");
    }

    #[test]
    fn hatch_round_trips_through_dxf_with_its_pattern_name() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        let hatch =
            add_hatch(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]], Some("ANSI31".to_string())).expect("hatch");
        assert_eq!(hatch["entity"]["kind"]["type"], json!("hatch"));
        assert_eq!(hatch["entity"]["kind"]["pattern"], json!("ANSI31"));
        let measured = measure(handle, None).expect("measure");
        assert_eq!(measured["area"], json!(16.0));
        assert_eq!(measured["length"], json!(16.0));

        let (text, losses) = exchange::dxf(&drawing_of(handle));
        assert!(text.contains("AC1015"), "a hatch needs a DXF version above R12");
        assert!(text.contains("0\nHATCH\n"));
        assert!(text.contains("2\nANSI31\n"));
        assert!(losses.iter().any(|loss| loss.code == "dxf/hatch-pattern"));

        let (imported, losses) = import::dxf(&text, None).expect("import the exported HATCH");
        assert_eq!(imported.entities.len(), 1, "{losses:?}");
        assert!(!losses.iter().any(|loss| loss.code == "dxf/unsupported-entity"), "{losses:?}");
        let EntityKind::Hatch { points, pattern } = &imported.entities[0].kind
        else {
            panic!("expected a hatch, got {:?}", imported.entities[0].kind);
        };
        assert_eq!(points.as_slice(), &[[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]]);
        assert_eq!(pattern, "ANSI31");
        assert!(losses.iter().any(|loss| loss.code == "dxf/hatch-boundary"));

        assert!(exchange::svg(&drawing_of(handle)).contains("data-open-cad"));

        let solid = create(CreateRequest::default()).expect("create solid drawing");
        let solid_handle = handle_of(&solid);
        add_hatch(solid_handle, DEFAULT_LAYER, vec![[10.0, 10.0], [12.0, 10.0], [11.0, 12.0]], None).expect("solid hatch");
        let (text, losses) = exchange::dxf(&drawing_of(solid_handle));
        assert!(!losses.iter().any(|loss| loss.code == "dxf/hatch-pattern"), "{losses:?}");
        assert!(text.contains("2\nSOLID\n"));
        close(solid_handle).expect("close solid drawing");

        assert_eq!(
            add_hatch(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [1.0, 1.0]], None).expect_err("too few points").code,
            "session/argument-invalid"
        );
        assert_eq!(
            add_hatch(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [1.0, 1.0], [f64::NAN, 2.0]], None).expect_err("non-finite").code,
            "session/argument-invalid"
        );
        close(handle).expect("close");
    }

    #[test]
    fn radial_and_ordinate_dimensions_round_trip_through_dxf() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        let radial = add_radial_dimension(handle, DIMENSION_LAYER, [0.0, 0.0], 5.0, 0.0, 1.5, false, None).expect("radial");
        assert_eq!(radial["entity"]["kind"]["type"], json!("radial_dimension"));
        assert_eq!(radial["entity"]["kind"]["radius"], json!(5.0));
        let diameter =
            add_radial_dimension(handle, DIMENSION_LAYER, [10.0, 0.0], 3.0, 90.0, 0.0, true, Some("DIA 6".to_string())).expect("diameter");
        assert_eq!(diameter["entity"]["kind"]["diameter"], json!(true));
        let ordinate = add_ordinate_dimension(handle, DIMENSION_LAYER, [0.0, 0.0], [12.0, 4.0], OrdinateAxis::X, 2.0, None).expect("ordinate");
        assert_eq!(ordinate["entity"]["kind"]["axis"], json!("x"));

        let drawing = drawing_of(handle);
        let kinds = drawing.entities.iter().map(|entity| kind_name(&entity.kind)).collect::<Vec<_>>();
        assert_eq!(kinds, vec!["radial dimension", "radial dimension", "ordinate dimension"]);
        let measured = measure(handle, None).expect("measure");
        assert_eq!(measured["length"], json!(0.0), "annotations contribute no length");
        assert_eq!(measured["area"], json!(0.0), "annotations contribute no area");

        let (text, _) = exchange::dxf(&drawing);
        assert!(text.contains("0\nDIMENSION\n"), "dimensions are written as DIMENSION records");
        assert!(text.contains("70\n4\n"), "a radial dimension uses type 4");
        assert!(text.contains("70\n3\n"), "a diameter dimension uses type 3");
        assert!(text.contains("70\n70\n"), "an X ordinate sets the type-6 bit and the X-datum bit");

        let (imported, losses) = import::dxf(&text, None).expect("import the exported DXF");
        assert_eq!(imported.entities.len(), 3, "{losses:?}");
        assert!(!losses.iter().any(|loss| loss.code == "dxf/unsupported-entity"), "{losses:?}");
        let EntityKind::RadialDimension { center, radius, angle, offset, diameter, text } = &imported.entities[0].kind
        else {
            panic!("expected a radial dimension, got {:?}", imported.entities[0].kind);
        };
        assert_eq!(*center, [0.0, 0.0]);
        assert!((radius - 5.0).abs() < 1e-9);
        assert!(angle.abs() < 1e-9);
        assert!((offset - 1.5).abs() < 1e-9);
        assert!(!diameter);
        assert_eq!(text, &None);
        let EntityKind::RadialDimension { center, angle, diameter, text, .. } = &imported.entities[1].kind
        else {
            panic!("expected a diameter dimension, got {:?}", imported.entities[1].kind);
        };
        assert!((center[0] - 10.0).abs() < 1e-9 && center[1].abs() < 1e-9);
        assert!((angle - 90.0).abs() < 1e-9);
        assert!(diameter);
        assert_eq!(text.as_deref(), Some("DIA 6"));
        let EntityKind::OrdinateDimension { origin, feature, axis, offset, .. } = &imported.entities[2].kind
        else {
            panic!("expected an ordinate dimension, got {:?}", imported.entities[2].kind);
        };
        assert_eq!(*origin, [0.0, 0.0]);
        assert_eq!(*feature, [12.0, 4.0]);
        assert_eq!(*axis, OrdinateAxis::X);
        assert!((offset - 2.0).abs() < 1e-9);

        transform(
            handle,
            TransformRequest {
                sheet: None,
                target: TransformTarget::Ids(vec!["entity-1".to_string()]),
                operation: TransformOp::Rotate { angle: 90.0, cx: 0.0, cy: 0.0 },
            },
        )
        .expect("rotate the radial dimension");
        let drawing = drawing_of(handle);
        assert!(matches!(
            drawing.entities[0].kind,
            EntityKind::RadialDimension { angle, .. } if (angle - 90.0).abs() < 1e-12
        ));
        close(handle).expect("close");
    }

    #[test]
    fn angular_dimensions_render_validate_and_round_trip() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        let added = add_angular_dimension(handle, DIMENSION_LAYER, [0.0, 0.0], [10.0, 0.0], [0.0, 10.0], 5.0, None).expect("angular dimension");
        assert_eq!(added["entity"]["kind"]["type"], json!("angular_dimension"));
        assert_eq!(added["entity"]["kind"]["offset"], json!(5.0));
        assert_eq!(
            add_angular_dimension(handle, DIMENSION_LAYER, [0.0, 0.0], [10.0, 0.0], [20.0, 0.0], 5.0, None).expect_err("collinear rays").code,
            "session/argument-invalid"
        );
        assert_eq!(
            add_angular_dimension(handle, DIMENSION_LAYER, [0.0, 0.0], [10.0, 0.0], [0.0, 10.0], 0.0, None).expect_err("zero radius").code,
            "session/argument-invalid"
        );

        let drawing = drawing_of(handle);
        assert_eq!(drawing.entities.len(), 1);
        assert_eq!(kind_name(&drawing.entities[0].kind), "angular dimension");
        let measured = measure(handle, None).expect("measure");
        assert_eq!(measured["length"], json!(0.0));
        assert_eq!(measured["area"], json!(0.0));

        let svg = exchange::svg(&drawing);
        assert!(svg.contains("90°"), "the SVG carries the measured angle: {svg}");
        let (text, losses) = exchange::dxf(&drawing);
        assert!(!losses.iter().any(|loss| loss.code == "dxf/unsupported-entity"), "{losses:?}");
        assert!(text.contains("70\n5\n"), "an angular dimension uses type 5");
        let (imported, losses) = import::dxf(&text, None).expect("import the exported DXF");
        assert!(!losses.iter().any(|loss| loss.code == "dxf/unsupported-entity"), "{losses:?}");
        let Some(EntityKind::AngularDimension { center, first, second, offset, .. }) =
            imported.entities.iter().map(|entity| &entity.kind).find(|kind| matches!(kind, EntityKind::AngularDimension { .. }))
        else {
            panic!("expected an imported angular dimension, got {:?}", imported.entities);
        };
        assert_eq!(*center, [0.0, 0.0]);
        assert_eq!(*first, [10.0, 0.0]);
        assert_eq!(*second, [0.0, 10.0]);
        assert!((offset - 5.0).abs() < 1e-9);

        let dir = TempDir::new("angular-dimension");
        let path = dir.path.join("angle.ocad");
        save(handle, &path, true).expect("save");
        close(handle).expect("close");
        let opened = open(&path).expect("open");
        let reopened = handle_of(&opened);
        let drawing = drawing_of(reopened);
        let EntityKind::AngularDimension { center, first, second, offset, .. } = &drawing.entities[0].kind
        else {
            panic!("expected an angular dimension, got {:?}", drawing.entities[0].kind);
        };
        assert_eq!(*center, [0.0, 0.0]);
        assert_eq!(*first, [10.0, 0.0]);
        assert_eq!(*second, [0.0, 10.0]);
        assert!((offset - 5.0).abs() < 1e-9);
        close(reopened).expect("close reopened");
    }

    #[test]
    fn radial_and_ordinate_dimensions_validate_and_inspect() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        assert_eq!(
            add_radial_dimension(handle, DIMENSION_LAYER, [0.0, 0.0], 0.0, 0.0, 0.0, false, None).expect_err("zero radius").code,
            "session/argument-invalid"
        );
        assert_eq!(
            add_radial_dimension(handle, DIMENSION_LAYER, [0.0, 0.0], 1.0, f64::NAN, 0.0, false, None).expect_err("nan angle").code,
            "session/argument-invalid"
        );

        let mut drawing = Drawing::empty(None, Units::Mm);
        drawing.entities.push(Entity {
            id: "entity-1".to_string(),
            layer: DIMENSION_LAYER.to_string(),
            kind: EntityKind::RadialDimension {
                center: [0.0, 0.0],
                radius: 5.0,
                angle: 0.0,
                offset: 0.0,
                diameter: false,
                text: Some("  ".to_string()),
            },
        });
        drawing.entities.push(Entity {
            id: "entity-2".to_string(),
            layer: DIMENSION_LAYER.to_string(),
            kind: EntityKind::OrdinateDimension {
                origin: [1.0, 2.0],
                feature: [1.0, 2.0],
                axis: OrdinateAxis::Y,
                offset: 0.0,
                text: Some("10 inch".to_string()),
            },
        });
        let report = inspection::run(&drawing, &["dimensions", "units"]).to_json();
        let codes =
            report["issues"].as_array().expect("issues").iter().map(|issue| issue["code"].as_str().unwrap_or_default()).collect::<Vec<_>>();
        for expected in ["dimensions/empty-text", "dimensions/degenerate", "units/dimension-label"] {
            assert!(codes.contains(&expected), "expected `{expected}` in {codes:?}");
        }
        assert_eq!(report["passed"], json!(false));
        close(handle).expect("close");
    }

    #[test]
    fn add_point_stores_the_anchor_and_respects_layer_flags() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        let point = add_point(handle, DEFAULT_LAYER, [5.0, 7.0]).expect("point");
        assert_eq!(point["entity"]["id"], json!("entity-1"));
        assert_eq!(point["revision"], json!(2));
        assert_eq!(point["entity"]["kind"]["type"], json!("point"));
        assert_eq!(point["entity"]["kind"]["at"], json!([5.0, 7.0]));
        assert_eq!(add_point(handle, DEFAULT_LAYER, [f64::NAN, 0.0]).expect_err("non-finite").code, "session/argument-invalid");

        add_layer(handle, "Structure", None).expect("layer");
        update_layer(handle, "Structure", LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock");
        assert_eq!(add_point(handle, "Structure", [0.0, 0.0]).expect_err("locked").code, "drawing/layer-locked");
        update_layer(handle, "Structure", LayerUpdate { locked: Some(false), frozen: Some(true), ..LayerUpdate::default() }).expect("freeze");
        assert_eq!(add_point(handle, "Structure", [0.0, 0.0]).expect_err("frozen").code, "drawing/layer-frozen");
        close(handle).expect("close");
    }

    #[test]
    fn offset_copies_circles_and_arcs_with_a_new_radius() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_circle(handle, DEFAULT_LAYER, [0.0, 0.0], 10.0).expect("circle");
        let grown = offset(handle, "entity-1", 2.0).expect("offset the circle outward");
        assert_eq!(grown["entity"]["kind"]["type"], json!("circle"));
        assert_eq!(grown["entity"]["kind"]["radius"], json!(12.0));
        assert_eq!(grown["source"], json!("entity-1"));
        assert_eq!(grown["revision"], json!(3));
        let shrunk = offset(handle, "entity-1", -4.0).expect("offset the circle inward");
        assert_eq!(shrunk["entity"]["kind"]["radius"], json!(6.0));
        assert_eq!(offset(handle, "entity-1", -10.0).expect_err("collapsing radius").code, "session/argument-invalid");
        assert_eq!(offset(handle, "entity-1", 0.0).expect_err("zero distance").code, "session/argument-invalid");

        add_arc(handle, DEFAULT_LAYER, [20.0, 0.0], 5.0, 0.0, 90.0).expect("arc");
        let arc = offset(handle, "entity-4", 1.0).expect("offset the arc");
        assert_eq!(arc["entity"]["kind"]["type"], json!("arc"));
        assert_eq!(arc["entity"]["kind"]["radius"], json!(6.0));
        assert_eq!(arc["entity"]["kind"]["start_angle"], json!(0.0));

        add_text(handle, DEFAULT_LAYER, [0.0, 0.0], 2.0, "note").expect("text");
        assert_eq!(offset(handle, "entity-6", 1.0).expect_err("text").code, "object/unsupported");
        assert_eq!(offset(handle, "entity-404", 1.0).expect_err("missing").code, "object/not-found");
        close(handle).expect("close");
    }

    #[test]
    fn offset_polylines_follow_mitered_normals() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        let open = add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]], false).expect("open polyline");
        assert_eq!(open["entity"]["kind"]["type"], json!("polyline"));
        let moved = offset(handle, "entity-1", 1.0).expect("offset the open polyline");
        assert_eq!(moved["entity"]["kind"]["points"], json!([[0.0, 1.0], [9.0, 1.0], [9.0, 10.0]]));
        assert_eq!(moved["entity"]["kind"]["closed"], json!(false));

        let closed = add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]], true).expect("closed polyline");
        assert_eq!(closed["entity"]["kind"]["closed"], json!(true));
        let inset = offset(handle, "entity-3", 1.0).expect("offset the closed polyline inward");
        assert_eq!(inset["entity"]["kind"]["points"], json!([[1.0, 1.0], [3.0, 1.0], [3.0, 3.0], [1.0, 3.0]]));
        close(handle).expect("close");
    }

    #[test]
    fn offset_lines_move_perpendicular_to_their_direction() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("line");
        let left = offset(handle, "entity-1", 2.0).expect("offset the line left");
        assert_eq!(left["entity"]["kind"]["type"], json!("line"));
        assert_eq!(left["entity"]["kind"]["from"], json!([0.0, 2.0]));
        assert_eq!(left["entity"]["kind"]["to"], json!([10.0, 2.0]));
        assert_eq!(left["source"], json!("entity-1"));

        let right = offset(handle, "entity-1", -3.0).expect("offset the line right");
        assert_eq!(right["entity"]["kind"]["from"], json!([0.0, -3.0]));
        assert_eq!(right["entity"]["kind"]["to"], json!([10.0, -3.0]));

        let zero = add_line(handle, DEFAULT_LAYER, [5.0, 5.0], [5.0, 5.0]).expect("zero-length line");
        let zero_id = zero["entity"]["id"].as_str().expect("id");
        assert_eq!(offset(handle, zero_id, 1.0).expect_err("zero-length").code, "session/argument-invalid");
        close(handle).expect("close");
    }

    #[test]
    fn offset_refuses_a_locked_layer() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_layer(handle, "Structure", Some("#ff0000")).expect("layer");
        add_circle(handle, "Structure", [0.0, 0.0], 4.0).expect("circle");
        update_layer(handle, "Structure", LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock");
        assert_eq!(offset(handle, "entity-1", 1.0).expect_err("locked").code, "drawing/layer-locked");
        close(handle).expect("close");
    }

    #[test]
    fn fillet_trims_two_lines_and_adds_a_tangent_arc() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("first line");
        add_line(handle, DEFAULT_LAYER, [10.0, 0.0], [10.0, 10.0]).expect("second line");
        let result = fillet(handle, "entity-1", "entity-2", 2.0).expect("fillet");
        assert_eq!(result["entity"]["kind"]["type"], json!("arc"));
        assert_eq!(result["entity"]["kind"]["radius"], json!(2.0));
        let center = result["entity"]["kind"]["center"].as_array().expect("center");
        assert!((center[0].as_f64().expect("x") - 8.0).abs() < 1e-9);
        assert!((center[1].as_f64().expect("y") - 2.0).abs() < 1e-9);
        assert!((result["entity"]["kind"]["start_angle"].as_f64().expect("start") + 90.0).abs() < 1e-9);
        assert!(result["entity"]["kind"]["end_angle"].as_f64().expect("end").abs() < 1e-9);
        assert_eq!(result["sources"], json!(["entity-1", "entity-2"]));
        assert_eq!(result["revision"], json!(4));

        let drawing = drawing_of(handle);
        let EntityKind::Line { from, to } = drawing.entities[0].kind
        else {
            panic!("expected a trimmed line");
        };
        assert_eq!((from, to), ([0.0, 0.0], [8.0, 0.0]));
        let EntityKind::Line { from, to } = drawing.entities[1].kind
        else {
            panic!("expected a trimmed line");
        };
        assert!((from[0] - 10.0).abs() < 1e-9 && (from[1] - 2.0).abs() < 1e-9, "trimmed from {from:?}");
        assert_eq!(to, [10.0, 10.0]);

        add_line(handle, DEFAULT_LAYER, [0.0, 5.0], [10.0, 5.0]).expect("parallel first");
        add_line(handle, DEFAULT_LAYER, [0.0, 8.0], [10.0, 8.0]).expect("parallel second");
        assert_eq!(fillet(handle, "entity-4", "entity-5", 1.0).expect_err("parallel").code, "session/argument-invalid");
        assert_eq!(fillet(handle, "entity-1", "entity-2", 11.0).expect_err("radius too large").code, "session/argument-invalid");
        assert_eq!(fillet(handle, "entity-1", "entity-1", 1.0).expect_err("same line").code, "session/argument-invalid");
        add_circle(handle, DEFAULT_LAYER, [0.0, 0.0], 3.0).expect("circle");
        assert_eq!(fillet(handle, "entity-1", "entity-6", 1.0).expect_err("circle").code, "object/unsupported");
        assert_eq!(fillet(handle, "entity-1", "entity-404", 1.0).expect_err("missing").code, "object/not-found");
        close(handle).expect("close");
    }

    #[test]
    fn chamfer_trims_two_lines_and_adds_the_bevel() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("first line");
        add_line(handle, DEFAULT_LAYER, [10.0, 0.0], [10.0, 10.0]).expect("second line");
        let result = chamfer(handle, "entity-1", "entity-2", 3.0, 4.0).expect("chamfer");
        assert_eq!(result["entity"]["kind"]["type"], json!("line"));
        assert_eq!(result["entity"]["kind"]["from"], json!([7.0, 0.0]));
        assert_eq!(result["entity"]["kind"]["to"], json!([10.0, 4.0]));
        assert_eq!(result["sources"], json!(["entity-1", "entity-2"]));

        let drawing = drawing_of(handle);
        let EntityKind::Line { from, to } = drawing.entities[0].kind
        else {
            panic!("expected a trimmed line");
        };
        assert_eq!((from, to), ([0.0, 0.0], [7.0, 0.0]));
        let EntityKind::Line { from, to } = drawing.entities[1].kind
        else {
            panic!("expected a trimmed line");
        };
        assert_eq!((from, to), ([10.0, 4.0], [10.0, 10.0]));

        assert_eq!(chamfer(handle, "entity-1", "entity-2", 8.0, 1.0).expect_err("distance too large").code, "session/argument-invalid");
        assert_eq!(chamfer(handle, "entity-1", "entity-2", 0.0, 1.0).expect_err("zero distance").code, "session/argument-invalid");
        add_text(handle, DEFAULT_LAYER, [0.0, 0.0], 2.0, "note").expect("text");
        assert_eq!(chamfer(handle, "entity-1", "entity-4", 1.0, 1.0).expect_err("text").code, "object/unsupported");
        close(handle).expect("close");
    }

    #[test]
    fn fillet_and_chamfer_refuse_a_locked_layer() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_layer(handle, "Structure", Some("#ff0000")).expect("layer");
        add_line(handle, "Structure", [0.0, 0.0], [10.0, 0.0]).expect("first line");
        add_line(handle, "Structure", [10.0, 0.0], [10.0, 10.0]).expect("second line");
        update_layer(handle, "Structure", LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock");
        assert_eq!(fillet(handle, "entity-1", "entity-2", 1.0).expect_err("locked fillet").code, "drawing/layer-locked");
        assert_eq!(chamfer(handle, "entity-1", "entity-2", 1.0, 1.0).expect_err("locked chamfer").code, "drawing/layer-locked");
        close(handle).expect("close");
    }

    #[test]
    fn stats_group_length_and_area_by_layer() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_layer(handle, "Walls", Some("#ff0000")).expect("layer");
        add_line(handle, "Walls", [0.0, 0.0], [3.0, 0.0]).expect("line");
        add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]], true).expect("square");
        add_circle(handle, DEFAULT_LAYER, [10.0, 10.0], 2.0).expect("circle");
        add_text(handle, DIMENSION_LAYER, [0.0, 0.0], 2.0, "note").expect("text");

        let summary = stats(handle).expect("stats");
        assert_eq!(summary["units"], json!("mm"));
        assert_eq!(summary["revision"], json!(6));
        assert_eq!(summary["totals"]["entities"], json!(4));
        let layers = summary["layers"].as_array().expect("layers");
        let walls = layers.iter().find(|layer| layer["id"] == json!("Walls")).expect("walls layer");
        assert_eq!(walls["entityCount"], json!(1));
        assert_eq!(walls["length"], json!(3.0));
        assert_eq!(walls["area"], json!(0.0));
        let default = layers.iter().find(|layer| layer["id"] == json!(DEFAULT_LAYER)).expect("default layer");
        assert_eq!(default["entityCount"], json!(2));
        assert_eq!(default["length"], json!(16.0), "square perimeter only");
        assert!((default["area"].as_f64().expect("area") - (16.0 + 4.0 * std::f64::consts::PI)).abs() < 1e-9, "square and circle area");
        let dimensions = layers.iter().find(|layer| layer["id"] == json!(DIMENSION_LAYER)).expect("dimension layer");
        assert_eq!(dimensions["entityCount"], json!(1));
        assert_eq!(dimensions["length"], json!(0.0));
        assert!((summary["totals"]["area"].as_f64().expect("total area") - (16.0 + 4.0 * std::f64::consts::PI)).abs() < 1e-9);
        close(handle).expect("close");
    }

    #[test]
    fn empty_layer_check_is_opt_in_and_warns_per_layer() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [1.0, 0.0]).expect("line");

        let default_report = inspect(handle, None).expect("default inspect");
        let default_codes = default_report["issues"]
            .as_array()
            .expect("issues")
            .iter()
            .map(|issue| issue["code"].as_str().unwrap_or_default())
            .collect::<Vec<_>>();
        assert!(!default_codes.contains(&"layers/empty"), "{default_codes:?}");

        let report = inspect(handle, Some(vec!["empty-layer".to_string()])).expect("empty-layer inspect");
        assert_eq!(report["passed"], json!(true), "empty layers are warnings");
        let issues = report["issues"].as_array().expect("issues");
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(issues[0]["code"], json!("layers/empty"));
        assert_eq!(issues[0]["layerId"], json!(DIMENSION_LAYER));
        assert_eq!(issues[0]["recoverable"], json!(true));
        close(handle).expect("close");
    }

    #[test]
    fn reorder_entity_changes_paint_order() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("first line");
        add_line(handle, DEFAULT_LAYER, [2.0, 0.0], [12.0, 0.0]).expect("second line");
        add_line(handle, DEFAULT_LAYER, [4.0, 0.0], [14.0, 0.0]).expect("third line");

        let moved = reorder_entity(handle, "entity-1", DrawOrderAction::ToFront).expect("to front");
        assert_eq!(moved["order"], json!(["entity-2", "entity-3", "entity-1"]));
        assert_eq!(drawing_of(handle).ordered_entity_ids(), vec!["entity-2", "entity-3", "entity-1"]);

        let moved = reorder_entity(handle, "entity-3", DrawOrderAction::ToBack).expect("to back");
        assert_eq!(moved["order"], json!(["entity-3", "entity-2", "entity-1"]));
        let moved = reorder_entity(handle, "entity-3", DrawOrderAction::Forward).expect("forward");
        assert_eq!(moved["order"], json!(["entity-2", "entity-3", "entity-1"]));
        let moved = reorder_entity(handle, "entity-3", DrawOrderAction::Backward).expect("backward");
        assert_eq!(moved["order"], json!(["entity-3", "entity-2", "entity-1"]));

        let svg = exchange::svg(&drawing_of(handle));
        let first = svg.find("<line").expect("first line element");
        let second = first + 1 + svg[first + 1..].find("<line").expect("second line element");
        let start_x = |index: usize| {
            svg[index..]
                .split("x1=\"")
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .and_then(|value| value.parse::<f64>().ok())
                .expect("line start x")
        };
        assert!(start_x(first) > start_x(second), "the front entity is painted last: {svg}");

        delete_entity(handle, "entity-3").expect("delete");
        assert_eq!(drawing_of(handle).ordered_entity_ids(), vec!["entity-2", "entity-1"]);
        close(handle).expect("close");
    }

    #[test]
    fn reorder_entity_rejects_an_unknown_entity() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("line");

        assert_eq!(reorder_entity(handle, "entity-404", DrawOrderAction::ToFront).expect_err("unknown entity").code, "object/not-found");
        assert!(DrawOrderAction::parse("sideways").is_none());
        assert_eq!(DrawOrderAction::parse(" Forward "), Some(DrawOrderAction::Forward));
        close(handle).expect("close");
    }

    #[test]
    fn layouts_create_update_list_and_delete() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [100.0, 0.0], [100.0, 50.0], [0.0, 50.0]], true).expect("square");

        let created_layout =
            create_layout(handle, LayoutRequest { name: "Sheet".to_string(), width: Some(200.0), height: Some(150.0), scale: None })
                .expect("create layout");
        assert_eq!(created_layout["drawing"]["layouts"], json!(1));
        assert_eq!(created_layout["layout"]["name"], json!("Sheet"));
        assert_eq!(created_layout["layout"]["width"], json!(200.0));
        assert_eq!(created_layout["layout"]["view"]["center"], json!([50.0, 25.0]));
        assert_eq!(created_layout["layout"]["view"]["drawingWidth"], json!(100.0));
        assert_eq!(created_layout["layout"]["view"]["visibleWidth"], json!(200.0));
        assert_eq!(created_layout["layout"]["view"]["fits"], json!(true));
        assert_eq!(created_layout["layout"]["view"]["fitScale"], json!(2.0));

        let updated = update_layout(handle, "Sheet", LayoutUpdate { width: Some(400.0), scale: Some(0.5), ..LayoutUpdate::default() })
            .expect("update layout");
        assert_eq!(updated["layout"]["width"], json!(400.0));
        assert_eq!(updated["layout"]["view"]["visibleWidth"], json!(800.0));
        assert_eq!(updated["layout"]["view"]["visibleHeight"], json!(300.0));
        assert_eq!(updated["layout"]["view"]["fitScale"], json!(3.0));

        let renamed =
            update_layout(handle, "Sheet", LayoutUpdate { name: Some("Site".to_string()), ..LayoutUpdate::default() }).expect("rename");
        assert_eq!(renamed["layout"]["id"], json!("Site"));
        let listed = list_layouts(handle).expect("list");
        assert_eq!(listed["layouts"].as_array().expect("layouts").len(), 1);
        assert_eq!(listed["layouts"][0]["name"], json!("Site"));

        let deleted = delete_layout(handle, "Site").expect("delete");
        assert_eq!(deleted["deleted"], json!("Site"));
        assert_eq!(list_layouts(handle).expect("list after delete")["layouts"], json!([]));
        close(handle).expect("close");
    }

    #[test]
    fn layouts_reject_duplicates_invalid_sizes_and_unknown_ids() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        create_layout(handle, LayoutRequest { name: "Sheet".to_string(), ..LayoutRequest::default() }).expect("layout");
        assert_eq!(
            create_layout(handle, LayoutRequest { name: "sheet".to_string(), ..LayoutRequest::default() }).expect_err("duplicate").code,
            "drawing/layout-exists"
        );
        assert_eq!(
            create_layout(handle, LayoutRequest { name: "Zero".to_string(), scale: Some(0.0), ..LayoutRequest::default() })
                .expect_err("zero scale")
                .code,
            "session/argument-invalid"
        );
        assert_eq!(
            update_layout(handle, "missing", LayoutUpdate { scale: Some(2.0), ..LayoutUpdate::default() }).expect_err("unknown").code,
            "object/not-found"
        );
        assert_eq!(update_layout(handle, "Sheet", LayoutUpdate::default()).expect_err("empty update").code, "session/argument-invalid");
        assert_eq!(delete_layout(handle, "missing").expect_err("unknown delete").code, "object/not-found");
        close(handle).expect("close");
    }

    #[test]
    fn dxf_r12_export_simplifies_entities_r12_cannot_carry() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_ellipse(handle, DEFAULT_LAYER, [20.0, 20.0], 10.0, 5.0, 0.0, 0.0, 360.0).expect("ellipse");
        add_hatch(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]], None).expect("hatch");
        add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [5.0, 5.0]).expect("line");

        let drawing = drawing_of(handle);
        let (text, losses) = exchange::dxf_r12(&drawing);
        assert!(text.contains("$ACADVER\n1\nAC1009\n"), "{text}");
        assert!(!text.contains("$INSUNITS"), "{text}");
        assert!(!text.contains("\nELLIPSE\n"), "{text}");
        assert!(!text.contains("\nHATCH\n"), "{text}");
        assert!(text.contains("\nPOLYLINE\n"), "{text}");
        let codes = losses.iter().map(|loss| loss.code.as_str()).collect::<Vec<_>>();
        for code in ["dxf/r12-units", "dxf/r12-ellipse", "dxf/r12-hatch"] {
            assert!(codes.contains(&code), "{codes:?}");
        }

        let (auto, _) = exchange::dxf(&drawing);
        assert!(auto.contains("\nELLIPSE\n"), "the default DXF target keeps analytic ellipses");
        assert!(auto.contains("AC1015"), "the default target upgrades for a hatch");
        close(handle).expect("close");
    }

    #[test]
    fn dxf_r12_omits_layer_flags_r12_cannot_write() {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_layer(handle, "Hidden", Some("#ff0000")).expect("layer");
        update_layer(handle, "Hidden", LayerUpdate { plottable: Some(false), lineweight: Some(0.5), ..LayerUpdate::default() })
            .expect("update layer");
        add_line(handle, "Hidden", [0.0, 0.0], [1.0, 0.0]).expect("line");

        let (text, losses) = exchange::dxf_r12(&drawing_of(handle));
        assert!(text.contains("0\nLAYER\n2\nHidden\n"), "{text}");
        assert!(!text.contains("\n370\n"), "{text}");
        assert!(!text.contains("\n290\n"), "{text}");
        let codes = losses.iter().map(|loss| loss.code.as_str()).collect::<Vec<_>>();
        assert!(codes.contains(&"dxf/r12-lineweight"), "{codes:?}");
        assert!(codes.contains(&"dxf/r12-plottable"), "{codes:?}");
        close(handle).expect("close");

        let empty = create(CreateRequest::default()).expect("empty create");
        let empty_handle = handle_of(&empty);
        let (text, losses) = exchange::dxf_r12(&drawing_of(empty_handle));
        assert!(text.contains("$ACADVER\n1\nAC1009\n"), "{text}");
        let codes = losses.iter().map(|loss| loss.code.as_str()).collect::<Vec<_>>();
        assert!(codes.contains(&"dxf/r12-units"), "{codes:?}");
        assert!(!codes.contains(&"dxf/r12-lineweight"), "{codes:?}");
        assert!(!codes.contains(&"dxf/r12-plottable"), "{codes:?}");
        close(empty_handle).expect("close");
    }
}

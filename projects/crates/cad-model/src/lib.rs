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
    ordinate_points, ordinate_value, radial_label, radial_target, sample_arc, sample_ellipse, sample_spline,
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
/// operations (transform, copy, rectangular/polar array, polyline explode,
/// offset, trim, extend, break, join, fillet, chamfer, align, distribute,
/// overkill, hatch-pattern, title-block, polyline-smooth, diff and measure);
/// constraint solving and a 3D kernel stay unsupported.
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
                "spline",
                "text",
                "dimension",
                "radial-dimension",
                "ordinate-dimension",
                "angular-dimension",
                "hatch",
            ],
            |name| match name {
                "line" | "circle" | "arc" | "ellipse" | "point" | "polyline" | "spline" | "text" | "dimension" | "radial-dimension"
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
                "trim",
                "extend",
                "break",
                "join",
                "fillet",
                "chamfer",
                "align",
                "distribute",
                "overkill",
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
                "hatch-pattern",
                "title-block",
                "polyline-smooth",
                "diff",
                "constraints",
                "kernel3d",
            ],
            |name| match name {
                "transform" | "copy" | "array" | "explode" | "offset" | "trim" | "extend" | "break" | "join" | "fillet" | "chamfer" | "align"
                | "distribute" | "overkill" | "measure" | "measure-entity" | "measure-points" | "stats" | "dimension-style" | "text-style" | "text-search"
                | "group" | "select" | "properties" | "snap" | "linetype" | "ucs" | "hatch-pattern" | "title-block" | "polyline-smooth" | "diff" => {
                    "supported"
                }
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

/// Reads and validates one `.ocad` project without opening a handle.
///
/// The file must be UTF-8 JSON with the `open-cad-drawing` tag and version 1.
/// Malformed content reports `format/invalid`; a future version reports
/// `format/unsupported-version`; neither path panics.
fn load_drawing(path: &Path) -> Result<Drawing, ServiceError> {
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
    Ok(drawing)
}

/// Opens a `.ocad` project and returns `{ handle, drawing }`.
///
/// The file must be UTF-8 JSON with the `open-cad-drawing` tag and version 1.
/// Malformed content reports `format/invalid`; a future version reports
/// `format/unsupported-version`; neither path panics.
pub fn open(path: &Path) -> Result<Value, ServiceError> {
    let drawing = load_drawing(path)?;
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

/// Adds a cubic spline to one layer.
///
/// The fit points are the points the curve passes through; at least two are
/// required. The curve is a uniform Catmull-Rom spline, so every fit point lies
/// on it, and `closed` joins the last span back to the first. A list with fewer
/// than two points or a non-finite coordinate reports
/// `session/argument-invalid`.
pub fn add_spline(handle: u64, layer: &str, points: Vec<[f64; 2]>, closed: bool) -> Result<Value, ServiceError> {
    require_finite(&points, "points")?;
    if points.len() < 2 {
        return Err(argument("a spline needs at least 2 fit points"));
    }
    push_entity(handle, layer, EntityKind::Spline { points, closed })
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
    /// Zero-based index of the vertex to move.
    pub move_at: Option<usize>,
    /// New position of the moved vertex, required together with `move_at`.
    pub move_to: Option<[f64; 2]>,
}

/// Edits the vertices or the closed flag of one polyline.
///
/// The caller passes a replacement `points` list, an `insert_at`/`insert_point`
/// pair, a `remove_at` index, a `move_at`/`move_to` pair or a new `closed` flag.
/// Replacing the list cannot be combined with a single-vertex edit and one call
/// edits at most one vertex. A polyline always keeps at least two vertices, and
/// an index outside the current list reports `session/argument-invalid`. A
/// non-polyline reports `object/unsupported` and a locked or frozen layer
/// reports `drawing/layer-locked` or `drawing/layer-frozen`. The response
/// carries `{ revision, drawing, entity }`.
pub fn update_polyline(handle: u64, entity: &str, update: PolylineUpdate) -> Result<Value, ServiceError> {
    let replacing = update.points.is_some();
    let inserting = update.insert_at.is_some();
    let removing = update.remove_at.is_some();
    let moving = update.move_at.is_some();
    if !replacing && update.closed.is_none() && !inserting && !removing && !moving {
        return Err(argument("pass at least one of `points`, `closed`, `insertAt`, `removeAt` or `moveAt`"));
    }
    if replacing && (inserting || removing || moving) {
        return Err(argument("replace the vertex list or edit one vertex, not both"));
    }
    if [inserting, removing, moving].into_iter().filter(|edit| *edit).count() > 1 {
        return Err(argument("insert, remove and move edit one vertex at a time"));
    }
    if inserting != update.insert_point.is_some() {
        return Err(argument("`insertAt` and `insertPoint` must be passed together"));
    }
    if moving != update.move_to.is_some() {
        return Err(argument("`moveAt` and `moveTo` must be passed together"));
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
    if let Some(point) = update.move_to {
        require_finite(&[point], "moveTo")?;
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
        if let Some(at) = update.move_at {
            if at >= points.len() {
                return Err(argument(format!("`moveAt` {at} is outside the polyline's {} vertices", points.len())));
            }
            points[at] = update.move_to.expect("the moved point is validated above");
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
                EntityKind::Spline { points, .. } => points.iter_mut().for_each(rotate),
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
                EntityKind::Spline { points, .. } => points.iter_mut().for_each(scale),
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
                EntityKind::Spline { points, .. } => points.iter_mut().for_each(mirror),
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
        EntityKind::Spline { .. } => "spline",
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

/// Direction an [`offset`] moves away from its source entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffsetSide {
    /// Offset to the left of the travel direction (lines and polylines).
    Left,
    /// Offset to the right of the travel direction (lines and polylines).
    Right,
    /// Increase the radius (circles and arcs).
    Outside,
    /// Decrease the radius (circles and arcs).
    Inside,
}

impl OffsetSide {
    /// Parses `left`, `right`, `outside` or `inside`.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            "outside" => Some(Self::Outside),
            "inside" => Some(Self::Inside),
            _ => None,
        }
    }

    /// Stable lowercase name used in JSON and the CLI.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Outside => "outside",
            Self::Inside => "inside",
        }
    }
}

/// Largest allowed ratio between a miter length and the offset distance.
const OFFSET_MITER_LIMIT: f64 = 4.0;

fn miter_limit() -> ServiceError {
    ServiceError::new("offset/miter-limit", "a polyline corner needs a miter longer than 4 times the offset distance")
}

fn invalid_side(entity: &str, kind: &str) -> ServiceError {
    let sides = if kind == "line" || kind == "polyline" { "`left` or `right`" } else { "`outside` or `inside`" };
    ServiceError::new("offset/invalid-side", format!("entity `{entity}` is a {kind}; use {sides}"))
}

fn linear_offset(entity: &str, distance: f64, side: OffsetSide, kind: &str) -> Result<f64, ServiceError> {
    match side {
        OffsetSide::Left => Ok(distance),
        OffsetSide::Right => Ok(-distance),
        _ => Err(invalid_side(entity, kind)),
    }
}

fn radial_offset(entity: &str, radius: f64, distance: f64, side: OffsetSide, kind: &str) -> Result<f64, ServiceError> {
    let delta = match side {
        OffsetSide::Outside => distance,
        OffsetSide::Inside => -distance,
        _ => return Err(invalid_side(entity, kind)),
    };
    let offset_radius = radius + delta;
    if !offset_radius.is_finite() || offset_radius <= 0.0 {
        return Err(ServiceError::new(
            "offset/invalid-distance",
            format!("offsetting {kind} `{entity}` by {distance} leaves a non-positive radius"),
        ));
    }
    Ok(clean_zero(offset_radius))
}

/// Normalises `-0` to `0` so exported values never carry a negative zero.
fn clean_zero(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
}

fn clean_point(point: [f64; 2]) -> [f64; 2] {
    [clean_zero(point[0]), clean_zero(point[1])]
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

/// Offsets a circle, arc, polyline or straight line by an explicit side.
///
/// `distance` must be positive and finite. `side` is `left` or `right` for a
/// line and polyline, measured along the travel direction, and `outside` or
/// `inside` for a circle and arc, which change only the radius. A line becomes
/// an equal-length parallel line; a polyline offsets each segment and joins
/// consecutive segments with a miter, refusing a corner whose miter exceeds
/// four times `distance` with `offset/miter-limit`. Ellipse, spline, text,
/// point, hatch and dimension entities report `offset/unsupported-entity`; an
/// unknown id reports `offset/not-found`; a radius at or below zero and a
/// non-finite result report `offset/invalid-distance`; a side that does not
/// match the entity kind reports `offset/invalid-side`. A locked layer reports
/// `drawing/layer-locked`. The source entity is not changed; the response
/// carries `{ revision, drawing, entity, source, side, distance }`.
pub fn offset_by_side(handle: u64, entity_id: &str, distance: f64, side: OffsetSide) -> Result<Value, ServiceError> {
    if !distance.is_finite() || distance <= 0.0 {
        return Err(ServiceError::new("offset/invalid-distance", "`distance` must be a positive finite number"));
    }
    let created = store::edit(handle, |drawing| {
        let index = drawing
            .entities
            .iter()
            .position(|candidate| candidate.id == entity_id)
            .ok_or_else(|| ServiceError::new("offset/not-found", format!("entity `{entity_id}` does not exist")))?;
        let source = drawing.entities[index].clone();
        if let Some(error) = locked_layer(drawing, &source.layer) {
            return Err(error);
        }
        let kind = match &source.kind {
            EntityKind::Circle { center, radius } => {
                EntityKind::Circle { center: clean_point(*center), radius: radial_offset(entity_id, *radius, distance, side, "circle")? }
            }
            EntityKind::Arc { center, radius, start_angle, end_angle } => EntityKind::Arc {
                center: clean_point(*center),
                radius: radial_offset(entity_id, *radius, distance, side, "arc")?,
                start_angle: clean_zero(*start_angle),
                end_angle: clean_zero(*end_angle),
            },
            EntityKind::Polyline { points, closed } => {
                let signed = linear_offset(entity_id, distance, side, "polyline")?;
                EntityKind::Polyline { points: offset_polyline_with(points, *closed, signed, true)?, closed: *closed }
            }
            EntityKind::Line { from, to } => {
                if side != OffsetSide::Left && side != OffsetSide::Right {
                    return Err(invalid_side(entity_id, "line"));
                }
                let normal =
                    segment_normal(*from, *to).ok_or_else(|| argument(format!("line `{entity_id}` has zero length and cannot be offset")))?;
                let signed = if side == OffsetSide::Left { distance } else { -distance };
                let shifted =
                    [[from[0] + normal[0] * signed, from[1] + normal[1] * signed], [to[0] + normal[0] * signed, to[1] + normal[1] * signed]];
                if !finite_point(shifted[0]) || !finite_point(shifted[1]) {
                    return Err(ServiceError::new(
                        "offset/invalid-distance",
                        format!("offsetting line `{entity_id}` produces a non-finite point"),
                    ));
                }
                EntityKind::Line { from: clean_point(shifted[0]), to: clean_point(shifted[1]) }
            }
            other => {
                return Err(ServiceError::new(
                    "offset/unsupported-entity",
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
            "side": side.as_str(),
            "distance": distance,
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

/// Intersections a straight target segment finds with one cutter.
enum CutterHits {
    /// The cutter is tangent, so the two roots collapse into one contact point.
    Tangent([f64; 2]),
    /// Distinct points where the target crosses the cutter, still bounded by the
    /// target segment.
    Points(Vec<[f64; 2]>),
    /// The cutter does not meet the target.
    None,
}

fn trim_degenerate(message: impl Into<String>) -> ServiceError {
    ServiceError::new("trim/degenerate", message)
}

fn cross(first: [f64; 2], second: [f64; 2]) -> f64 {
    first[0] * second[1] - first[1] * second[0]
}

/// Roots of the target segment against a full circle, keeping only the roots
/// that lie on the bounded segment.
fn line_circle_hits(from: [f64; 2], to: [f64; 2], center: [f64; 2], radius: f64) -> CutterHits {
    let delta = [to[0] - from[0], to[1] - from[1]];
    let offset = [from[0] - center[0], from[1] - center[1]];
    let quadratic = delta[0] * delta[0] + delta[1] * delta[1];
    if quadratic < 1e-24 || radius.abs() <= 1e-12 {
        return CutterHits::None;
    }
    let linear = 2.0 * (offset[0] * delta[0] + offset[1] * delta[1]);
    let constant = offset[0] * offset[0] + offset[1] * offset[1] - radius * radius;
    let discriminant = linear * linear - 4.0 * quadratic * constant;
    let tolerance = 1e-9 * quadratic * radius * radius;
    if discriminant < -tolerance {
        return CutterHits::None;
    }
    if discriminant <= tolerance {
        let parameter = -linear / (2.0 * quadratic);
        if !within_segment(parameter) {
            return CutterHits::None;
        }
        return CutterHits::Tangent([from[0] + delta[0] * parameter, from[1] + delta[1] * parameter]);
    }
    let root = discriminant.sqrt();
    let points = [(-linear - root) / (2.0 * quadratic), (-linear + root) / (2.0 * quadratic)]
        .into_iter()
        .filter(|parameter| within_segment(*parameter))
        .map(|parameter| [from[0] + delta[0] * parameter, from[1] + delta[1] * parameter])
        .collect::<Vec<_>>();
    if points.len() == 2 && distance(points[0], points[1]) <= 1e-9 * radius.abs().max(1.0) {
        return CutterHits::Tangent(points[0]);
    }
    CutterHits::Points(points)
}

/// Points where the target segment meets one line, circle or arc cutter.
fn line_cutter_hits(from: [f64; 2], to: [f64; 2], cutter: &EntityKind, cutter_id: &str) -> Result<Vec<[f64; 2]>, ServiceError> {
    let target_length = distance(from, to);
    if target_length <= 1e-12 {
        return Err(trim_degenerate("the target line has zero length"));
    }
    match cutter {
        EntityKind::Line { from: cutter_from, to: cutter_to } => {
            let cutter_length = distance(*cutter_from, *cutter_to);
            if cutter_length <= 1e-12 {
                return Err(trim_degenerate(format!("cutter `{cutter_id}` has zero length")));
            }
            let first = [to[0] - from[0], to[1] - from[1]];
            let second = [cutter_to[0] - cutter_from[0], cutter_to[1] - cutter_from[1]];
            let denominator = cross(first, second);
            let offset = [cutter_from[0] - from[0], cutter_from[1] - from[1]];
            if denominator.abs() <= 1e-12 * target_length * cutter_length {
                let perpendicular = (cross(offset, first) / target_length).abs();
                if perpendicular <= 1e-9 * target_length.max(1.0) {
                    return Err(trim_degenerate(format!("the line and cutter `{cutter_id}` are collinear")));
                }
                return Ok(Vec::new());
            }
            let along_target = cross(offset, second) / denominator;
            let along_cutter = cross(offset, first) / denominator;
            if within_segment(along_target) && within_segment(along_cutter) {
                Ok(vec![[from[0] + first[0] * along_target, from[1] + first[1] * along_target]])
            }
            else {
                Ok(Vec::new())
            }
        }
        EntityKind::Circle { center, radius } => match line_circle_hits(from, to, *center, radius.abs()) {
            CutterHits::Tangent(_) => Err(trim_degenerate(format!("cutter `{cutter_id}` is tangent to the target line"))),
            CutterHits::Points(points) => Ok(points),
            CutterHits::None => Ok(Vec::new()),
        },
        EntityKind::Arc { center, radius, start_angle, end_angle } => {
            let sweep = arc_sweep(*start_angle, *end_angle);
            let within_sweep =
                |point: [f64; 2]| angle_within_sweep(*start_angle, sweep, (point[1] - center[1]).atan2(point[0] - center[0]).to_degrees());
            match line_circle_hits(from, to, *center, radius.abs()) {
                CutterHits::Tangent(point) if within_sweep(point) => {
                    Err(trim_degenerate(format!("cutter `{cutter_id}` is tangent to the target line")))
                }
                CutterHits::Tangent(_) | CutterHits::None => Ok(Vec::new()),
                CutterHits::Points(points) => Ok(points.into_iter().filter(|point| within_sweep(*point)).collect()),
            }
        }
        other => Err(ServiceError::new("object/unsupported", format!("entity `{cutter_id}` is a {} and cannot cut a line", kind_name(other)))),
    }
}

/// Points where two full circles meet. Tangency collapses to one contact point,
/// while separation, containment and coincidence give no crossing.
fn circle_circle_hits(first: ([f64; 2], f64), second: ([f64; 2], f64)) -> CutterHits {
    let ((first_center, first_radius), (second_center, second_radius)) = (first, second);
    let delta = [second_center[0] - first_center[0], second_center[1] - first_center[1]];
    let separation = delta[0].hypot(delta[1]);
    let scale = (first_radius + second_radius).max(1.0);
    let tolerance = 1e-9 * scale;
    if separation < 1e-12 {
        return CutterHits::None;
    }
    let unit = [delta[0] / separation, delta[1] / separation];
    if separation >= first_radius + second_radius - tolerance {
        if separation <= first_radius + second_radius + tolerance {
            return CutterHits::Tangent([first_center[0] + unit[0] * first_radius, first_center[1] + unit[1] * first_radius]);
        }
        return CutterHits::None;
    }
    if separation <= (first_radius - second_radius).abs() + tolerance {
        if separation >= (first_radius - second_radius).abs() - tolerance {
            let sign = if first_radius >= second_radius { 1.0 } else { -1.0 };
            return CutterHits::Tangent([first_center[0] + unit[0] * first_radius * sign, first_center[1] + unit[1] * first_radius * sign]);
        }
        return CutterHits::None;
    }
    let along = (first_radius * first_radius - second_radius * second_radius + separation * separation) / (2.0 * separation);
    let height_squared = first_radius * first_radius - along * along;
    if height_squared <= 0.0 {
        return CutterHits::None;
    }
    let height = height_squared.sqrt();
    let base = [first_center[0] + unit[0] * along, first_center[1] + unit[1] * along];
    let offset = [-unit[1] * height, unit[0] * height];
    CutterHits::Points(vec![[base[0] + offset[0], base[1] + offset[1]], [base[0] - offset[0], base[1] - offset[1]]])
}

/// Points where the full circle `(center, radius)` meets one cutter. An arc
/// cutter only contributes the crossings on its own counter-clockwise sweep.
fn circle_cutter_hits(
    center: [f64; 2],
    radius: f64,
    cutter: &EntityKind,
    cutter_id: &str,
    target_kind: &str,
) -> Result<CutterHits, ServiceError> {
    match cutter {
        EntityKind::Line { from, to } => {
            if distance(*from, *to) <= 1e-12 {
                return Err(trim_degenerate(format!("cutter `{cutter_id}` has zero length")));
            }
            Ok(line_circle_hits(*from, *to, center, radius))
        }
        EntityKind::Circle { center: cutter_center, radius: cutter_radius } => {
            let cutter_radius = cutter_radius.abs();
            if cutter_radius <= 1e-12 {
                return Err(trim_degenerate(format!("cutter `{cutter_id}` has zero radius")));
            }
            if distance(*cutter_center, center) <= 1e-12 && (cutter_radius - radius).abs() <= 1e-9 * radius.max(1.0) {
                return Err(trim_degenerate(format!("cutter `{cutter_id}` is the target circle, so the cut is ambiguous")));
            }
            Ok(circle_circle_hits((center, radius), (*cutter_center, cutter_radius)))
        }
        EntityKind::Arc { center: cutter_center, radius: cutter_radius, start_angle, end_angle } => {
            let cutter_radius = cutter_radius.abs();
            if cutter_radius <= 1e-12 {
                return Err(trim_degenerate(format!("cutter `{cutter_id}` has zero radius")));
            }
            let sweep = arc_sweep(*start_angle, *end_angle);
            let within = |point: [f64; 2]| {
                angle_within_sweep(*start_angle, sweep, (point[1] - cutter_center[1]).atan2(point[0] - cutter_center[0]).to_degrees())
            };
            match circle_circle_hits((center, radius), (*cutter_center, cutter_radius)) {
                CutterHits::Tangent(point) if within(point) => Ok(CutterHits::Tangent(point)),
                CutterHits::Tangent(_) | CutterHits::None => Ok(CutterHits::None),
                CutterHits::Points(points) => Ok(CutterHits::Points(points.into_iter().filter(|point| within(*point)).collect())),
            }
        }
        other => {
            Err(ServiceError::new("object/unsupported", format!("entity `{cutter_id}` is a {} and cannot cut {target_kind}", kind_name(other))))
        }
    }
}

/// Trims an arc target and returns the kept `EntityKind::Arc`.
///
/// The target's full circle is cut, then only crossings on the arc sweep are
/// kept and measured as degrees travelled from the start angle. One crossing
/// keeps the longer remaining arc; two or more keep the longer outer portion,
/// discarding the span between the first two crossings.
fn trim_arc(target: &EntityKind, target_id: &str, cutter: &EntityKind, cutter_id: &str) -> Result<EntityKind, ServiceError> {
    let EntityKind::Arc { center, radius, start_angle, end_angle } = target
    else {
        unreachable!("trim_arc is only called with an arc target");
    };
    let sweep = arc_sweep(*start_angle, *end_angle);
    if radius.abs() <= 1e-12 {
        return Err(trim_degenerate(format!("target arc `{target_id}` has zero radius")));
    }
    let endpoint_tolerance = 1e-9 * sweep.max(1.0);
    let angular_tolerance = 1e-9_f64.to_degrees();
    let mut cuts = match circle_cutter_hits(*center, radius.abs(), cutter, cutter_id, "an arc")? {
        CutterHits::None => Vec::new(),
        CutterHits::Tangent(point) => vec![point],
        CutterHits::Points(points) => points,
    }
    .into_iter()
    .map(|point| normalize_angle((point[1] - center[1]).atan2(point[0] - center[0]).to_degrees() - start_angle))
    .filter(|travelled| sweep >= 360.0 - 1e-9 || *travelled <= sweep + angular_tolerance)
    .collect::<Vec<_>>();
    cuts.sort_by(|left, right| left.total_cmp(right));
    cuts.dedup_by(|left, right| (*left - *right).abs() <= angular_tolerance);
    if cuts.is_empty() {
        return Err(ServiceError::new("trim/no-intersection", format!("cutter `{cutter_id}` does not meet the target arc `{target_id}`")));
    }
    let (kept_start, kept_end) = if cuts.len() == 1 {
        let cut = cuts[0].clamp(0.0, sweep);
        let before = cut;
        let after = sweep - cut;
        if before.min(after) <= endpoint_tolerance {
            return Err(trim_degenerate("the intersection lies on a target arc endpoint, so there is nothing to trim"));
        }
        if before >= after { (0.0, cut) } else { (cut, sweep) }
    }
    else {
        let first = cuts[0].clamp(0.0, sweep);
        let second = cuts[1].clamp(0.0, sweep);
        let before = first;
        let after = sweep - second;
        if before.max(after) <= endpoint_tolerance {
            return Err(trim_degenerate("both intersections lie on the target arc endpoints, so there is nothing to trim"));
        }
        if before >= after { (0.0, first) } else { (second, sweep) }
    };
    Ok(EntityKind::Arc {
        center: *center,
        radius: *radius,
        start_angle: normalize_angle(start_angle + kept_start),
        end_angle: normalize_angle(start_angle + kept_end),
    })
}

/// Trims a circle target and returns the kept `EntityKind::Arc`.
///
/// A circle has no endpoints, so exactly two distinct crossings are required.
/// The shorter span between the crossings is removed and the result covers the
/// longer remaining sweep.
fn trim_circle(target: &EntityKind, target_id: &str, cutter: &EntityKind, cutter_id: &str) -> Result<EntityKind, ServiceError> {
    let EntityKind::Circle { center, radius } = target
    else {
        unreachable!("trim_circle is only called with a circle target");
    };
    if radius.abs() <= 1e-12 {
        return Err(trim_degenerate(format!("target circle `{target_id}` has zero radius")));
    }
    let angular_tolerance = 1e-9_f64.to_degrees();
    let mut angles = match circle_cutter_hits(*center, radius.abs(), cutter, cutter_id, "a circle")? {
        CutterHits::None => Vec::new(),
        CutterHits::Tangent(point) => vec![normalize_angle((point[1] - center[1]).atan2(point[0] - center[0]).to_degrees())],
        CutterHits::Points(points) => {
            points.into_iter().map(|point| normalize_angle((point[1] - center[1]).atan2(point[0] - center[0]).to_degrees())).collect()
        }
    };
    angles.sort_by(|left, right| left.total_cmp(right));
    angles.dedup_by(|left, right| (*left - *right).abs() <= angular_tolerance);
    if angles.len() > 1 && 360.0 - (angles[angles.len() - 1] - angles[0]) <= angular_tolerance {
        angles.pop();
    }
    if angles.is_empty() {
        return Err(ServiceError::new("trim/no-intersection", format!("cutter `{cutter_id}` does not meet the target circle `{target_id}`")));
    }
    if angles.len() < 2 {
        return Err(trim_degenerate("the cutter meets the target circle at a single point, so there is nothing to trim"));
    }
    let first = angles[0];
    let second = angles[1];
    let span = second - first;
    let (start_angle, end_angle) = if span >= 180.0 { (first, second) } else { (second, first) };
    Ok(EntityKind::Arc { center: *center, radius: *radius, start_angle, end_angle })
}

/// Trims a line, arc or circle target at a cutter, keeping the longer side.
///
/// The cutter may be a line, circle or arc. A line target keeps the longer of
/// the two sides for one intersection, and the longer outer segment for two, so
/// the span between the cut points is discarded. An arc target measures the
/// crossings along its sweep and applies the same policy. A circle target has
/// no endpoints, so exactly two distinct crossings are required; the shorter
/// span between them is removed and the result is an arc covering the longer
/// sweep. A target and cutter without a valid crossing report
/// `trim/no-intersection`; tangency, collinearity, a zero-length or zero-radius
/// input, an ambiguous circle cut and a cut that leaves nothing report
/// `trim/degenerate`. The response carries `{ revision, drawing, entity, cutter }`,
/// where `entity` is the trimmed target, possibly an arc that came from a circle.
pub fn trim(handle: u64, target_id: &str, cutter_id: &str) -> Result<Value, ServiceError> {
    if target_id == cutter_id {
        return Err(argument("a trim needs two different entities"));
    }
    let updated = store::edit(handle, |drawing| {
        let target_index = entity_index(drawing, target_id)?;
        let cutter_index = entity_index(drawing, cutter_id)?;
        let target = drawing.entities[target_index].kind.clone();
        if !matches!(target, EntityKind::Line { .. } | EntityKind::Arc { .. } | EntityKind::Circle { .. }) {
            return Err(ServiceError::new(
                "object/unsupported",
                format!("entity `{target_id}` is a {} and cannot be trimmed", kind_name(&target)),
            ));
        }
        if let Some(error) = locked_layer(drawing, &drawing.entities[target_index].layer) {
            return Err(error);
        }
        let cutter = drawing.entities[cutter_index].kind.clone();
        let trimmed = match &target {
            EntityKind::Line { from, to } => {
                let (from, to) = (*from, *to);
                let length = distance(from, to);
                let parameter =
                    |point: [f64; 2]| ((point[0] - from[0]) * (to[0] - from[0]) + (point[1] - from[1]) * (to[1] - from[1])) / (length * length);
                let mut hits =
                    line_cutter_hits(from, to, &cutter, cutter_id)?.into_iter().map(|point| (parameter(point), point)).collect::<Vec<_>>();
                hits.sort_by(|left, right| left.0.total_cmp(&right.0));
                hits.dedup_by(|left, right| distance(left.1, right.1) <= 1e-9 * length.max(1.0));
                if hits.is_empty() {
                    return Err(ServiceError::new(
                        "trim/no-intersection",
                        format!("cutter `{cutter_id}` does not meet the target line `{target_id}`"),
                    ));
                }
                let (new_from, new_to) = if hits.len() == 1 {
                    let cut = hits[0].0.clamp(0.0, 1.0);
                    let before = cut * length;
                    let after = (1.0 - cut) * length;
                    if before.min(after) <= 1e-9 * length {
                        return Err(trim_degenerate("the intersection lies on a target endpoint, so there is nothing to trim"));
                    }
                    if before >= after { (from, hits[0].1) } else { (hits[0].1, to) }
                }
                else {
                    let before = hits[0].0.clamp(0.0, 1.0) * length;
                    let after = (1.0 - hits[1].0.clamp(0.0, 1.0)) * length;
                    if before.max(after) <= 1e-9 * length {
                        return Err(trim_degenerate("both intersections lie on the target endpoints, so there is nothing to trim"));
                    }
                    if before >= after { (from, hits[0].1) } else { (hits[1].1, to) }
                };
                EntityKind::Line { from: new_from, to: new_to }
            }
            EntityKind::Arc { .. } => trim_arc(&target, target_id, &cutter, cutter_id)?,
            EntityKind::Circle { .. } => trim_circle(&target, target_id, &cutter, cutter_id)?,
            _ => unreachable!("the target kind was checked before the trim"),
        };
        drawing.entities[target_index].kind = trimmed;
        drawing.revision += 1;
        Ok(drawing.entities[target_index].clone())
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "entity": updated,
            "cutter": cutter_id,
        })
    })
}

/// Which endpoint of a line or arc [`extend`] moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtendSide {
    /// Extend past the `to` endpoint, in the `from`-to-`to` direction, or past
    /// the end angle counter-clockwise.
    End,
    /// Extend past the `from` endpoint, in the `to`-to-`from` direction, or past
    /// the start angle clockwise.
    Start,
}

impl ExtendSide {
    /// Parses `end` or `start`, ignoring case and surrounding whitespace.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "end" => Some(Self::End),
            "start" => Some(Self::Start),
            _ => None,
        }
    }

    /// Name used in JSON responses and the CLI.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::End => "end",
            Self::Start => "start",
        }
    }
}

/// Ray a line is extended along: its moving endpoint as the origin and the unit
/// direction it travels away from the line.
struct ExtendRay {
    /// Point the extension starts from, which is the endpoint being replaced.
    origin: [f64; 2],
    /// Unit direction away from the line.
    direction: [f64; 2],
    /// The endpoint that stays put.
    anchor: [f64; 2],
}

fn extend_ray(from: [f64; 2], to: [f64; 2], side: ExtendSide) -> Option<ExtendRay> {
    let (origin, anchor, tip) = match side {
        ExtendSide::End => (to, from, from),
        ExtendSide::Start => (from, to, to),
    };
    let direction = unit_direction(tip, origin)?;
    Some(ExtendRay { origin, direction, anchor })
}

/// Parameters along an extension ray where it meets a full circle.
fn ray_circle_parameters(ray: &ExtendRay, center: [f64; 2], radius: f64) -> Vec<f64> {
    let offset = [ray.origin[0] - center[0], ray.origin[1] - center[1]];
    let half = offset[0] * ray.direction[0] + offset[1] * ray.direction[1];
    let constant = offset[0] * offset[0] + offset[1] * offset[1] - radius * radius;
    let discriminant = half * half - constant;
    let tolerance = 1e-9 * radius.abs().max(1.0);
    if discriminant < -tolerance {
        return Vec::new();
    }
    if discriminant <= tolerance {
        return vec![-half];
    }
    let root = discriminant.sqrt();
    vec![-half - root, -half + root]
}

/// Parameter along an extension ray where it meets one bounded line segment.
fn ray_line_parameter(ray: &ExtendRay, from: [f64; 2], to: [f64; 2]) -> Option<f64> {
    let segment = [to[0] - from[0], to[1] - from[1]];
    let denominator = cross(ray.direction, segment);
    if denominator.abs() <= 1e-12 {
        return None;
    }
    let offset = [from[0] - ray.origin[0], from[1] - ray.origin[1]];
    let along_boundary = cross(offset, ray.direction) / denominator;
    if !within_segment(along_boundary) {
        return None;
    }
    Some(cross(offset, segment) / denominator)
}

/// Parameters where the extension ray meets one line, circle or arc boundary.
fn extend_boundary_parameters(ray: &ExtendRay, boundary: &EntityKind, boundary_id: &str) -> Result<Vec<f64>, ServiceError> {
    match boundary {
        EntityKind::Line { from, to } => Ok(ray_line_parameter(ray, *from, *to).into_iter().collect()),
        EntityKind::Circle { center, radius } => Ok(ray_circle_parameters(ray, *center, radius.abs())),
        EntityKind::Arc { center, radius, start_angle, end_angle } => {
            let sweep = arc_sweep(*start_angle, *end_angle);
            Ok(ray_circle_parameters(ray, *center, radius.abs())
                .into_iter()
                .filter(|parameter| {
                    let point = [ray.origin[0] + ray.direction[0] * parameter, ray.origin[1] + ray.direction[1] * parameter];
                    angle_within_sweep(*start_angle, sweep, (point[1] - center[1]).atan2(point[0] - center[0]).to_degrees())
                })
                .collect())
        }
        other => Err(ServiceError::new(
            "object/unsupported",
            format!("entity `{boundary_id}` is a {} and cannot bound an extension", kind_name(other)),
        )),
    }
}

/// Angles, in `[0, 360)` degrees around the target arc's centre, where the
/// target's full circle meets one line, circle or arc boundary. A line boundary
/// is its bounded segment, and an arc boundary contributes only the crossings
/// on its own counter-clockwise sweep.
fn extend_arc_angles(center: [f64; 2], radius: f64, boundary: &EntityKind, boundary_id: &str) -> Result<Vec<f64>, ServiceError> {
    let points = match boundary {
        EntityKind::Line { from, to } => match line_circle_hits(*from, *to, center, radius) {
            CutterHits::Tangent(point) => vec![point],
            CutterHits::Points(points) => points,
            CutterHits::None => Vec::new(),
        },
        EntityKind::Circle { center: boundary_center, radius: boundary_radius } => {
            match circle_circle_hits((center, radius), (*boundary_center, boundary_radius.abs())) {
                CutterHits::Tangent(point) => vec![point],
                CutterHits::Points(points) => points,
                CutterHits::None => Vec::new(),
            }
        }
        EntityKind::Arc { center: boundary_center, radius: boundary_radius, start_angle, end_angle } => {
            let sweep = arc_sweep(*start_angle, *end_angle);
            match circle_circle_hits((center, radius), (*boundary_center, boundary_radius.abs())) {
                CutterHits::Tangent(point) => retain_on_arc(vec![point], *boundary_center, *start_angle, sweep),
                CutterHits::Points(points) => retain_on_arc(points, *boundary_center, *start_angle, sweep),
                CutterHits::None => Vec::new(),
            }
        }
        other => {
            return Err(ServiceError::new(
                "object/unsupported",
                format!("entity `{boundary_id}` is a {} and cannot bound an extension", kind_name(other)),
            ));
        }
    };
    Ok(points.into_iter().map(|point| normalize_angle((point[1] - center[1]).atan2(point[0] - center[0]).to_degrees())).collect())
}

/// Extends one arc endpoint along its own circle until it meets the nearest
/// boundary crossing in the direction that grows the sweep.
///
/// `start` walks clockwise from the start angle and `end` walks counter-clockwise
/// from the end angle. Crossings at the moving endpoint are ignored, and a
/// crossing that would make the sweep reach a full turn is refused instead of
/// wrapping the arc.
fn extend_arc_target(
    drawing: &mut Drawing,
    target_index: usize,
    boundary_index: usize,
    boundary_id: &str,
    side: ExtendSide,
) -> Result<Entity, ServiceError> {
    let target = drawing.entities[target_index].clone();
    let EntityKind::Arc { center, radius, start_angle, end_angle } = target.kind
    else {
        return Err(ServiceError::new("object/unsupported", format!("entity `{}` is not a line or arc", target.id)));
    };
    if let Some(error) = locked_layer(drawing, &target.layer) {
        return Err(error);
    }
    let radius_abs = radius.abs();
    if radius_abs <= 1e-12 {
        return Err(argument(format!("arc `{}` has zero radius and cannot be extended", target.id)));
    }
    let sweep = arc_sweep(start_angle, end_angle);
    let tolerance = 1e-9 * sweep.max(1.0);
    let nearest = extend_arc_angles(center, radius_abs, &drawing.entities[boundary_index].kind, boundary_id)?
        .into_iter()
        .map(|angle| match side {
            ExtendSide::Start => normalize_angle(start_angle - angle),
            ExtendSide::End => normalize_angle(angle - end_angle),
        })
        .filter(|travel| *travel > tolerance)
        .min_by(f64::total_cmp);
    let Some(travel) = nearest
    else {
        return Err(ServiceError::new(
            "extend/no-intersection",
            format!("boundary `{boundary_id}` does not meet the extension of arc `{}`", target.id),
        ));
    };
    if sweep + travel >= 360.0 - tolerance {
        return Err(ServiceError::new(
            "extend/no-intersection",
            format!("extending arc `{}` to boundary `{boundary_id}` would wrap past a full circle", target.id),
        ));
    }
    drawing.entities[target_index].kind = match side {
        ExtendSide::Start => EntityKind::Arc { center, radius, start_angle: normalize_angle(start_angle - travel), end_angle },
        ExtendSide::End => EntityKind::Arc { center, radius, start_angle, end_angle: normalize_angle(end_angle + travel) },
    };
    drawing.revision += 1;
    Ok(drawing.entities[target_index].clone())
}

/// Extends one line or arc until it meets a line, circle or arc boundary.
///
/// The target must be a line or arc; the boundary may be a line, circle or arc.
/// `side` chooses which endpoint moves: for a line `end` keeps `from` and moves
/// `to`, while `start` keeps `to` and moves `from`. Only crossings ahead of the
/// moving endpoint count, and the nearest one becomes the new endpoint, so the
/// line grows and never folds back. A line boundary is its bounded drawn segment;
/// a circle or arc crossing must lie on the arc's counter-clockwise sweep.
///
/// An arc target travels along its own circle rather than a tangent ray: `end`
/// walks counter-clockwise from the end angle, `start` walks clockwise from the
/// start angle, and the first crossing with the boundary becomes the new
/// endpoint. The centre, radius and other endpoint stay put, and a candidate
/// that would push the sweep past a full turn is refused.
///
/// The response carries `{ revision, drawing, entity, boundary, side }`, where
/// `entity` is the extended line or arc. An extension that never meets the
/// boundary reports `extend/no-intersection`; a non-line, non-arc target or an
/// unsupported boundary reports `object/unsupported`; an unknown id reports
/// `object/not-found`; a locked target layer reports `drawing/layer-locked`; a
/// zero-length line, a zero-radius arc or the same id for target and boundary
/// reports `session/argument-invalid`.
pub fn extend(handle: u64, target_id: &str, boundary_id: &str, side: ExtendSide) -> Result<Value, ServiceError> {
    if target_id == boundary_id {
        return Err(argument("an extend needs two different entities"));
    }
    let updated = store::edit(handle, |drawing| {
        let target_index = entity_index(drawing, target_id)?;
        let boundary_index = entity_index(drawing, boundary_id)?;
        if matches!(drawing.entities[target_index].kind, EntityKind::Arc { .. }) {
            return extend_arc_target(drawing, target_index, boundary_index, boundary_id, side);
        }
        let (from, to) = line_endpoints(&drawing.entities[target_index].kind).ok_or_else(|| {
            ServiceError::new(
                "object/unsupported",
                format!("entity `{target_id}` is a {} and cannot be extended", kind_name(&drawing.entities[target_index].kind)),
            )
        })?;
        if let Some(error) = locked_layer(drawing, &drawing.entities[target_index].layer) {
            return Err(error);
        }
        let ray = extend_ray(from, to, side).ok_or_else(|| argument(format!("line `{target_id}` has zero length and cannot be extended")))?;
        let boundary = drawing.entities[boundary_index].kind.clone();
        let nearest = extend_boundary_parameters(&ray, &boundary, boundary_id)?
            .into_iter()
            .filter(|parameter| *parameter > 1e-9)
            .min_by(|left, right| left.total_cmp(right));
        let Some(parameter) = nearest
        else {
            return Err(ServiceError::new(
                "extend/no-intersection",
                format!("boundary `{boundary_id}` does not meet the extension of line `{target_id}`"),
            ));
        };
        let endpoint = [ray.origin[0] + ray.direction[0] * parameter, ray.origin[1] + ray.direction[1] * parameter];
        drawing.entities[target_index].kind = match side {
            ExtendSide::End => EntityKind::Line { from: ray.anchor, to: endpoint },
            ExtendSide::Start => EntityKind::Line { from: endpoint, to: ray.anchor },
        };
        drawing.revision += 1;
        Ok(drawing.entities[target_index].clone())
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "entity": updated,
            "boundary": boundary_id,
            "side": side.as_str(),
        })
    })
}

/// Arguments of [`break_entity`].
#[derive(Debug, Clone)]
pub struct BreakRequest {
    /// Identifier of the line or arc to split.
    pub id: String,
    /// Point where the entity is split; it must lie strictly inside the segment
    /// or the arc sweep.
    pub at: [f64; 2],
}

/// Splits an arc at an interior point and returns the two parts in order.
///
/// The point must lie on the arc's circle within `1e-9` scaled by the radius
/// (and at least 1), and its angle must fall inside the counter-clockwise sweep
/// away from both endpoints, using the same degree tolerance as the arc trim.
fn break_arc(
    center: [f64; 2],
    radius: f64,
    start_angle: f64,
    end_angle: f64,
    id: &str,
    at: [f64; 2],
) -> Result<(EntityKind, EntityKind), ServiceError> {
    let radius_abs = radius.abs();
    if radius_abs <= 1e-12 {
        return Err(ServiceError::new("break/not-on-entity", format!("arc `{id}` has zero radius and cannot be broken")));
    }
    let offset = [at[0] - center[0], at[1] - center[1]];
    let radial = offset[0].hypot(offset[1]) - radius_abs;
    if radial.abs() > 1e-9 * radius_abs.max(1.0) {
        return Err(ServiceError::new("break/not-on-entity", format!("the break point must lie on arc `{id}`")));
    }
    let sweep = arc_sweep(start_angle, end_angle);
    let angle = normalize_angle(offset[1].atan2(offset[0]).to_degrees());
    let travelled = normalize_angle(angle - start_angle);
    let endpoint_tolerance = 1e-9 * sweep.max(1.0);
    if !angle_within_sweep(start_angle, sweep, angle) || travelled <= endpoint_tolerance || travelled >= sweep - endpoint_tolerance {
        return Err(ServiceError::new(
            "break/not-on-entity",
            format!("the break point must lie strictly inside arc `{id}`, away from its endpoints"),
        ));
    }
    Ok((EntityKind::Arc { center, radius, start_angle, end_angle: angle }, EntityKind::Arc { center, radius, start_angle: angle, end_angle }))
}

/// Splits one line or arc at an interior point.
///
/// A line point must lie on the segment, away from both endpoints: the tolerance
/// is `1e-9` scaled by the segment length. A point that is close but not exactly
/// on the segment is projected onto it, so the two pieces stay collinear and
/// their lengths add up to the original. The source line is removed and
/// replaced, at the same list position, by two lines that keep its layer and
/// receive fresh deterministic ids.
///
/// An arc point must lie on the arc, away from both endpoints: the radial
/// tolerance is `1e-9` scaled by the radius and the angular endpoint tolerance
/// matches the arc trim. The source keeps the part from its start angle to the
/// cut angle, and a new arc appended with the next deterministic id covers the
/// cut angle to the original end angle. Both share the centre, radius and layer.
///
/// Either way the edit is one history step and one revision. The response
/// carries `{ revision, drawing, removed, created }`, where `removed` is the
/// source id and `created` holds the two resulting entities in order: for a line
/// from `from` to `at` and `at` to `to`, both with fresh ids, and for an arc the
/// two pieces, where the first reuses the source id.
///
/// A point on an endpoint or off the entity, a zero-length line and a zero-radius
/// arc report `break/not-on-entity`; a circle or another kind reports
/// `object/unsupported`; an unknown id reports `object/not-found`; a locked or
/// frozen layer reports `drawing/layer-locked` or `drawing/layer-frozen`;
/// non-finite coordinates report `session/argument-invalid`.
pub fn break_entity(handle: u64, request: BreakRequest) -> Result<Value, ServiceError> {
    require_finite(&[request.at], "at")?;
    let BreakRequest { id, at } = request;
    let created = store::edit(handle, |drawing| {
        let index = entity_index(drawing, &id)?;
        let source = drawing.entities[index].clone();
        if let Some(error) = locked_layer(drawing, &source.layer) {
            return Err(error);
        }
        if let EntityKind::Arc { center, radius, start_angle, end_angle } = source.kind {
            let (first, second) = break_arc(center, radius, start_angle, end_angle, &id, at)?;
            drawing.entities[index].kind = first;
            let second = Entity { id: drawing.take_entity_id(), layer: source.layer.clone(), kind: second };
            if let Some(position) = drawing.draw_order.iter().position(|candidate| candidate == &id) {
                drawing.draw_order.insert(position + 1, second.id.clone());
            }
            drawing.entities.push(second.clone());
            drawing.revision += 1;
            return Ok(vec![drawing.entities[index].clone(), second]);
        }
        let EntityKind::Line { from, to } = source.kind
        else {
            return Err(ServiceError::new("object/unsupported", format!("entity `{id}` is a {}, not a line", kind_name(&source.kind))));
        };
        let length = distance(from, to);
        if length <= 1e-12 {
            return Err(ServiceError::new("break/not-on-entity", format!("line `{id}` has zero length and cannot be broken")));
        }
        let delta = [to[0] - from[0], to[1] - from[1]];
        let along = ((at[0] - from[0]) * delta[0] + (at[1] - from[1]) * delta[1]) / length;
        let perpendicular = cross(delta, [at[0] - from[0], at[1] - from[1]]).abs() / length;
        let tolerance = 1e-9 * length.max(1.0);
        if perpendicular > tolerance || along <= tolerance || along >= length - tolerance {
            return Err(ServiceError::new(
                "break/not-on-entity",
                format!("the break point must lie strictly inside line `{id}`, away from its endpoints"),
            ));
        }
        let direction = [delta[0] / length, delta[1] / length];
        let split = [from[0] + direction[0] * along, from[1] + direction[1] * along];
        let first = Entity { id: drawing.take_entity_id(), layer: source.layer.clone(), kind: EntityKind::Line { from, to: split } };
        let second = Entity { id: drawing.take_entity_id(), layer: source.layer.clone(), kind: EntityKind::Line { from: split, to } };
        let ids = [first.id.clone(), second.id.clone()];
        drawing.entities.splice(index..=index, [first.clone(), second.clone()]);
        if let Some(position) = drawing.draw_order.iter().position(|candidate| candidate == &id) {
            drawing.draw_order.splice(position..=position, ids);
        }
        detach_entity_from_groups(drawing, &id);
        drawing.revision += 1;
        Ok(vec![first, second])
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "removed": id,
            "created": created,
        })
    })
}

/// Arguments of [`join_entities`].
#[derive(Debug, Clone)]
pub struct JoinRequest {
    /// Identifier of the first line.
    pub a: String,
    /// Identifier of the second line; one of its endpoints must meet `a`.
    pub b: String,
}

/// Merges two collinear lines that share one endpoint into a single line.
///
/// Both entities must be lines on the same layer, so colour, line type and
/// lineweight stay consistent. One endpoint of `a` must coincide with one
/// endpoint of `b`, and the two lines must be collinear and continue in opposite
/// directions from that shared point, so the union is one longer segment rather
/// than an overlap. Both tests use a tolerance of `1e-9` scaled by the longer
/// segment length, and at least 1.
///
/// The two sources are removed and replaced, at `a`'s list position, by one line
/// from `a`'s free endpoint to `b`'s free endpoint. The merged line keeps the
/// shared layer and receives a fresh deterministic id; both sources leave draw
/// order and entity groups. The edit is one history step and one revision.
///
/// The response carries `{ revision, drawing, removed, created }`, where
/// `removed` holds both source ids in `[a, b]` order and `created` holds the
/// merged line. Endpoints that do not meet, lines that are not collinear, or a
/// folded-back overlap report `join/not-adjacent`; a non-line or a layer
/// mismatch reports `object/unsupported`; an unknown id reports
/// `object/not-found`; a locked or frozen layer reports `drawing/layer-locked` or
/// `drawing/layer-frozen`; the same id twice or a zero-length line reports
/// `session/argument-invalid`.
pub fn join_entities(handle: u64, request: JoinRequest) -> Result<Value, ServiceError> {
    let JoinRequest { a, b } = request;
    if a == b {
        return Err(argument("a join needs two different lines"));
    }
    let created = store::edit(handle, |drawing| {
        let a_index = entity_index(drawing, &a)?;
        let b_index = entity_index(drawing, &b)?;
        let first = drawing.entities[a_index].clone();
        let second = drawing.entities[b_index].clone();
        let (a_from, a_to) = line_endpoints(&first.kind).ok_or_else(|| {
            ServiceError::new("object/unsupported", format!("entity `{a}` is a {} and cannot be joined", kind_name(&first.kind)))
        })?;
        let (b_from, b_to) = line_endpoints(&second.kind).ok_or_else(|| {
            ServiceError::new("object/unsupported", format!("entity `{b}` is a {} and cannot be joined", kind_name(&second.kind)))
        })?;
        if first.layer != second.layer {
            return Err(ServiceError::new("object/unsupported", format!("lines `{a}` and `{b}` are on different layers and cannot be joined")));
        }
        for entity in [&first, &second] {
            if let Some(error) = locked_layer(drawing, &entity.layer) {
                return Err(error);
            }
        }
        let a_length = distance(a_from, a_to);
        let b_length = distance(b_from, b_to);
        if a_length <= 1e-12 || b_length <= 1e-12 {
            return Err(argument("a zero-length line cannot be joined"));
        }
        let tolerance = 1e-9 * a_length.max(b_length).max(1.0);
        let a_points = [a_from, a_to];
        let b_points = [b_from, b_to];
        let mut shared: Option<(usize, usize)> = None;
        let mut best = f64::INFINITY;
        for (a_side, a_point) in a_points.iter().enumerate() {
            for (b_side, b_point) in b_points.iter().enumerate() {
                let gap = distance(*a_point, *b_point);
                if gap <= tolerance && gap < best {
                    best = gap;
                    shared = Some((a_side, b_side));
                }
            }
        }
        let Some((a_side, b_side)) = shared
        else {
            return Err(ServiceError::new("join/not-adjacent", format!("lines `{a}` and `{b}` do not meet at a shared endpoint")));
        };
        let point = a_points[a_side];
        let free_a = a_points[1 - a_side];
        let free_b = b_points[1 - b_side];
        let direction = unit_direction(point, free_a).ok_or_else(|| argument("a zero-length line cannot be joined"))?;
        if cross(direction, [free_b[0] - point[0], free_b[1] - point[1]]).abs() > tolerance {
            return Err(ServiceError::new("join/not-adjacent", format!("lines `{a}` and `{b}` are not collinear")));
        }
        let left = [free_a[0] - point[0], free_a[1] - point[1]];
        let right = [free_b[0] - point[0], free_b[1] - point[1]];
        if left[0] * right[0] + left[1] * right[1] >= 0.0 {
            return Err(ServiceError::new(
                "join/not-adjacent",
                format!("lines `{a}` and `{b}` fold back over each other at their shared endpoint"),
            ));
        }
        let id = drawing.take_entity_id();
        let merged = Entity { id: id.clone(), layer: first.layer.clone(), kind: EntityKind::Line { from: free_a, to: free_b } };
        drawing.entities.remove(b_index);
        let position = if b_index < a_index { a_index - 1 } else { a_index };
        drawing.entities[position] = merged.clone();
        if let Some(slot) = drawing.draw_order.iter().position(|candidate| candidate == &a) {
            drawing.draw_order.remove(slot);
            drawing.draw_order.insert(slot.min(drawing.draw_order.len()), id);
            drawing.draw_order.retain(|candidate| candidate != &b);
        }
        else if let Some(slot) = drawing.draw_order.iter().position(|candidate| candidate == &b) {
            drawing.draw_order[slot] = id;
        }
        detach_entity_from_groups(drawing, &a);
        detach_entity_from_groups(drawing, &b);
        drawing.revision += 1;
        Ok(merged)
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "removed": [a, b],
            "created": [created],
        })
    })
}

/// Edge or centreline an [`align`] applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignMode {
    /// Align left edges to the smallest `minX` of the selection.
    Left,
    /// Align horizontal centres to the midpoint of the selection's X span.
    Center,
    /// Align right edges to the largest `maxX` of the selection.
    Right,
    /// Align bottom edges to the smallest `minY` of the selection.
    Bottom,
    /// Align vertical centres to the midpoint of the selection's Y span.
    Middle,
    /// Align top edges to the largest `maxY` of the selection.
    Top,
}

impl AlignMode {
    /// Parses `left`, `center`, `right`, `bottom`, `middle` or `top`, ignoring
    /// case and surrounding whitespace.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "left" => Some(Self::Left),
            "center" => Some(Self::Center),
            "right" => Some(Self::Right),
            "bottom" => Some(Self::Bottom),
            "middle" => Some(Self::Middle),
            "top" => Some(Self::Top),
            _ => None,
        }
    }

    /// Name used in JSON responses and the CLI.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
            Self::Bottom => "bottom",
            Self::Middle => "middle",
            Self::Top => "top",
        }
    }
}

/// Arguments of [`align`].
#[derive(Debug, Clone)]
pub struct AlignRequest {
    /// Entity ids to align; repeated ids are applied once.
    pub ids: Vec<String>,
    /// Edge or centreline to align to.
    pub mode: AlignMode,
}

/// Axis a [`distribute`] spreads entities along.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistributeAxis {
    /// Spread along X by the horizontal edges of the bounding boxes.
    X,
    /// Spread along Y by the vertical edges of the bounding boxes.
    Y,
}

impl DistributeAxis {
    /// Parses `x` or `y`, ignoring case and surrounding whitespace.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "x" => Some(Self::X),
            "y" => Some(Self::Y),
            _ => None,
        }
    }

    /// Name used in JSON responses and the CLI.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::X => "x",
            Self::Y => "y",
        }
    }
}

/// Arguments of [`distribute`].
#[derive(Debug, Clone)]
pub struct DistributeRequest {
    /// Entity ids to distribute; repeated ids are applied once.
    pub ids: Vec<String>,
    /// Axis to spread along.
    pub axis: DistributeAxis,
}

/// Resolves ids to sorted, deduplicated entity indices.
fn selection_indices(drawing: &Drawing, ids: &[String]) -> Result<Vec<usize>, ServiceError> {
    let mut indices = Vec::with_capacity(ids.len());
    for id in ids {
        indices.push(entity_index(drawing, id)?);
    }
    indices.sort_unstable();
    indices.dedup();
    Ok(indices)
}

/// Axis-aligned bounding box of one entity from its finite bounds points.
fn entity_box(entity: &Entity) -> Option<([f64; 2], [f64; 2])> {
    let mut box_: Option<([f64; 2], [f64; 2])> = None;
    for point in entity.bounds_points() {
        if !finite_point(point) {
            continue;
        }
        box_ = Some(match box_ {
            None => ([point[0], point[1]], [point[0], point[1]]),
            Some((min, max)) => ([min[0].min(point[0]), min[1].min(point[1])], [max[0].max(point[0]), max[1].max(point[1])]),
        });
    }
    box_
}

/// Aligns a selection of entities to one bounding-box edge or centreline.
///
/// Each entity's box comes from the same sampled outline [`measure`] reports.
/// `left`, `right` and `bottom`/`top` align the extreme edges of every box to
/// the smallest or largest edge of the selection; `center` and `middle` align
/// the box centres to the midpoint of the selection's span, so the entity with
/// the widest box on each side stays put. Entities move by translation only;
/// ids, layers and geometry are otherwise untouched. Fewer than two distinct
/// ids report `align/insufficient-entities`; an unknown id reports
/// `object/not-found`; a locked or frozen layer reports `drawing/layer-locked`
/// or `drawing/layer-frozen`; an entity without finite bounds reports
/// `object/degenerate`. The edit is one history step and one revision, and the
/// response carries `{ revision, drawing, mode, affected, bounds }`.
pub fn align(handle: u64, request: AlignRequest) -> Result<Value, ServiceError> {
    if request.ids.is_empty() {
        return Err(argument("`ids` is empty"));
    }
    let mode = request.mode;
    let affected = store::edit(handle, |drawing| {
        let indices = selection_indices(drawing, &request.ids)?;
        if indices.len() < 2 {
            return Err(ServiceError::new("align/insufficient-entities", "aligning needs at least two distinct entities"));
        }
        for &index in &indices {
            if let Some(error) = locked_layer(drawing, &drawing.entities[index].layer) {
                return Err(error);
            }
        }
        let boxes = indices
            .iter()
            .map(|&index| {
                entity_box(&drawing.entities[index]).ok_or_else(|| {
                    ServiceError::new("object/degenerate", format!("entity `{}` has no finite bounds", drawing.entities[index].id))
                })
            })
            .collect::<Result<Vec<_>, ServiceError>>()?;
        let min_x = boxes.iter().map(|(min, _)| min[0]).fold(f64::INFINITY, f64::min);
        let max_x = boxes.iter().map(|(_, max)| max[0]).fold(f64::NEG_INFINITY, f64::max);
        let min_y = boxes.iter().map(|(min, _)| min[1]).fold(f64::INFINITY, f64::min);
        let max_y = boxes.iter().map(|(_, max)| max[1]).fold(f64::NEG_INFINITY, f64::max);
        for (index, (min, max)) in indices.iter().zip(&boxes) {
            let delta = match mode {
                AlignMode::Left => [min_x - min[0], 0.0],
                AlignMode::Right => [max_x - max[0], 0.0],
                AlignMode::Center => [(min_x + max_x) / 2.0 - (min[0] + max[0]) / 2.0, 0.0],
                AlignMode::Bottom => [0.0, min_y - min[1]],
                AlignMode::Top => [0.0, max_y - max[1]],
                AlignMode::Middle => [0.0, (min_y + max_y) / 2.0 - (min[1] + max[1]) / 2.0],
            };
            drawing.entities[*index].translate(delta);
        }
        drawing.revision += 1;
        Ok(indices.len())
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "mode": mode.as_str(),
            "affected": affected,
            "bounds": bounds_json(&session.drawing),
        })
    })
}

/// Spreads a selection of entities with equal gaps between bounding boxes.
///
/// Entities are ordered by the centre of their box along `axis`, ties keep
/// document order. The first and last entities stay put and every interior
/// entity is translated so the gap between consecutive boxes along the axis is
/// the same; the gap is `(span - total size) / (count - 1)` and may be negative
/// when the boxes overlap. Fewer than three distinct ids report
/// `distribute/insufficient-entities`; an unknown id reports
/// `object/not-found`; a locked or frozen layer reports `drawing/layer-locked`
/// or `drawing/layer-frozen`; an entity without finite bounds reports
/// `object/degenerate`. The edit is one history step and one revision, and the
/// response carries `{ revision, drawing, axis, affected, bounds }`.
pub fn distribute(handle: u64, request: DistributeRequest) -> Result<Value, ServiceError> {
    if request.ids.is_empty() {
        return Err(argument("`ids` is empty"));
    }
    let axis = request.axis;
    let affected = store::edit(handle, |drawing| {
        let indices = selection_indices(drawing, &request.ids)?;
        if indices.len() < 3 {
            return Err(ServiceError::new("distribute/insufficient-entities", "distributing needs at least three distinct entities"));
        }
        for &index in &indices {
            if let Some(error) = locked_layer(drawing, &drawing.entities[index].layer) {
                return Err(error);
            }
        }
        let boxes = indices
            .iter()
            .map(|&index| {
                entity_box(&drawing.entities[index]).ok_or_else(|| {
                    ServiceError::new("object/degenerate", format!("entity `{}` has no finite bounds", drawing.entities[index].id))
                })
            })
            .collect::<Result<Vec<_>, ServiceError>>()?;
        let coordinate = |(min, max): ([f64; 2], [f64; 2])| match axis {
            DistributeAxis::X => (min[0], max[0]),
            DistributeAxis::Y => (min[1], max[1]),
        };
        let mut order: Vec<usize> = (0..boxes.len()).collect();
        order.sort_by(|left, right| {
            let (a_min, a_max) = coordinate(boxes[*left]);
            let (b_min, b_max) = coordinate(boxes[*right]);
            let a_center = a_min + (a_max - a_min) / 2.0;
            let b_center = b_min + (b_max - b_min) / 2.0;
            a_center.partial_cmp(&b_center).unwrap_or(std::cmp::Ordering::Equal).then(left.cmp(right))
        });
        let (first_min, _) = coordinate(boxes[order[0]]);
        let (_, last_max) = coordinate(boxes[order[order.len() - 1]]);
        let total_size: f64 = order
            .iter()
            .map(|&position| {
                let (min, max) = coordinate(boxes[position]);
                max - min
            })
            .sum();
        let gap = (last_max - first_min - total_size) / f64::from(indices.len() as u32 - 1);
        let mut cursor = first_min;
        for position in order {
            let index = indices[position];
            let (min, max) = coordinate(boxes[position]);
            let delta = match axis {
                DistributeAxis::X => [cursor - min, 0.0],
                DistributeAxis::Y => [0.0, cursor - min],
            };
            drawing.entities[index].translate(delta);
            cursor += max - min + gap;
        }
        drawing.revision += 1;
        Ok(indices.len())
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "axis": axis.as_str(),
            "affected": affected,
            "bounds": bounds_json(&session.drawing),
        })
    })
}

/// Arguments of [`overkill`].
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OverkillRequest {
    /// Geometric tolerance in drawing units; defaults to `1e-9`.
    pub tolerance: Option<f64>,
    /// Whether collinear, overlapping or touching lines on one layer merge
    /// into their union; defaults to `false`.
    pub merge_overlapping: Option<bool>,
}

/// Whether [`overkill`] can compare one kind for exact duplication.
fn overkill_comparable(kind: &EntityKind) -> bool {
    matches!(
        kind,
        EntityKind::Line { .. }
            | EntityKind::Circle { .. }
            | EntityKind::Arc { .. }
            | EntityKind::Ellipse { .. }
            | EntityKind::Point { .. }
            | EntityKind::Polyline { .. }
            | EntityKind::Spline { .. }
    )
}

fn points_close(first: [f64; 2], second: [f64; 2], tolerance: f64) -> bool {
    distance(first, second) <= tolerance
}

fn numbers_close(first: f64, second: f64, tolerance: f64) -> bool {
    (first - second).abs() <= tolerance
}

/// Angular equality in degrees, tolerant of the `0`/`360` wrap.
fn angles_close(first: f64, second: f64, tolerance: f64) -> bool {
    let delta = (normalize_angle(first) - normalize_angle(second)).abs();
    delta <= tolerance || 360.0 - delta <= tolerance
}

/// Whether two same-kind entities describe the same geometry within
/// `tolerance`; a line also matches its reversed twin.
fn kinds_duplicate(first: &EntityKind, second: &EntityKind, tolerance: f64) -> bool {
    match (first, second) {
        (EntityKind::Line { from: a_from, to: a_to }, EntityKind::Line { from: b_from, to: b_to }) => {
            (points_close(*a_from, *b_from, tolerance) && points_close(*a_to, *b_to, tolerance))
                || (points_close(*a_from, *b_to, tolerance) && points_close(*a_to, *b_from, tolerance))
        }
        (EntityKind::Circle { center: a_center, radius: a_radius }, EntityKind::Circle { center: b_center, radius: b_radius }) => {
            points_close(*a_center, *b_center, tolerance) && numbers_close(*a_radius, *b_radius, tolerance)
        }
        (
            EntityKind::Arc { center: a_center, radius: a_radius, start_angle: a_start, end_angle: a_end },
            EntityKind::Arc { center: b_center, radius: b_radius, start_angle: b_start, end_angle: b_end },
        ) => {
            points_close(*a_center, *b_center, tolerance)
                && numbers_close(*a_radius, *b_radius, tolerance)
                && angles_close(*a_start, *b_start, tolerance)
                && angles_close(*a_end, *b_end, tolerance)
        }
        (
            EntityKind::Ellipse { center: a_center, rx: a_rx, ry: a_ry, rotation: a_rotation, start_angle: a_start, end_angle: a_end },
            EntityKind::Ellipse { center: b_center, rx: b_rx, ry: b_ry, rotation: b_rotation, start_angle: b_start, end_angle: b_end },
        ) => {
            points_close(*a_center, *b_center, tolerance)
                && numbers_close(*a_rx, *b_rx, tolerance)
                && numbers_close(*a_ry, *b_ry, tolerance)
                && angles_close(*a_rotation, *b_rotation, tolerance)
                && angles_close(*a_start, *b_start, tolerance)
                && angles_close(*a_end, *b_end, tolerance)
        }
        (EntityKind::Point { at: a_at }, EntityKind::Point { at: b_at }) => points_close(*a_at, *b_at, tolerance),
        (EntityKind::Polyline { points: a_points, closed: a_closed }, EntityKind::Polyline { points: b_points, closed: b_closed })
        | (EntityKind::Spline { points: a_points, closed: a_closed }, EntityKind::Spline { points: b_points, closed: b_closed }) => {
            a_closed == b_closed
                && a_points.len() == b_points.len()
                && std::iter::zip(a_points.iter(), b_points.iter()).all(|(a, b)| points_close(*a, *b, tolerance))
        }
        _ => false,
    }
}

/// Whether `point` lies on the infinite line through `anchor` with unit
/// `direction`, within `tolerance` — the same cross-product test
/// [`join_entities`] applies to its two free endpoints.
fn on_line(anchor: [f64; 2], direction: [f64; 2], point: [f64; 2], tolerance: f64) -> bool {
    cross(direction, [point[0] - anchor[0], point[1] - anchor[1]]).abs() <= tolerance
}

/// One planned merge: source ids in document order and the union endpoints.
struct OverkillMerge {
    sources: Vec<String>,
    from: [f64; 2],
    to: [f64; 2],
}

/// What one [`overkill`] call would change, computed before the edit.
struct OverkillPlan {
    removed: Vec<String>,
    merges: Vec<OverkillMerge>,
    skipped: usize,
    before: usize,
}

/// Computes the duplicate removals and, when asked, the collinear line merges.
fn plan_overkill(drawing: &Drawing, tolerance: f64, merge_overlapping: bool) -> Result<OverkillPlan, ServiceError> {
    let mut plan = OverkillPlan { removed: Vec::new(), merges: Vec::new(), skipped: 0, before: drawing.entities.len() };
    let mut kept: Vec<usize> = Vec::new();
    for (index, entity) in drawing.entities.iter().enumerate() {
        if !overkill_comparable(&entity.kind) {
            plan.skipped += 1;
            continue;
        }
        let duplicate = kept.iter().any(|&other| {
            let candidate = &drawing.entities[other];
            candidate.layer == entity.layer && kinds_duplicate(&candidate.kind, &entity.kind, tolerance)
        });
        if duplicate {
            if let Some(error) = locked_layer(drawing, &entity.layer) {
                return Err(error);
            }
            plan.removed.push(entity.id.clone());
        }
        else {
            kept.push(index);
        }
    }
    if !merge_overlapping {
        return Ok(plan);
    }
    let mut used = vec![false; drawing.entities.len()];
    for &first in &kept {
        if used[first] {
            continue;
        }
        let Some((from, first_to)) = line_endpoints(&drawing.entities[first].kind)
        else {
            continue;
        };
        let Some(direction) = unit_direction(from, first_to)
        else {
            continue;
        };
        let layer = drawing.entities[first].layer.clone();
        let mut group = vec![first];
        used[first] = true;
        for &other in &kept {
            if used[other] {
                continue;
            }
            let candidate = &drawing.entities[other];
            if candidate.layer != layer {
                continue;
            }
            let Some((c_from, c_to)) = line_endpoints(&candidate.kind)
            else {
                continue;
            };
            if on_line(from, direction, c_from, tolerance) && on_line(from, direction, c_to, tolerance) {
                group.push(other);
                used[other] = true;
            }
        }
        if group.len() < 2 {
            continue;
        }
        let mut spans: Vec<(f64, f64, usize)> = group
            .iter()
            .map(|&index| {
                let (a, b) = line_endpoints(&drawing.entities[index].kind).expect("group members are lines");
                let first_t = (a[0] - from[0]) * direction[0] + (a[1] - from[1]) * direction[1];
                let last_t = (b[0] - from[0]) * direction[0] + (b[1] - from[1]) * direction[1];
                (first_t.min(last_t), first_t.max(last_t), index)
            })
            .collect();
        spans.sort_by(|left, right| left.0.partial_cmp(&right.0).unwrap_or(std::cmp::Ordering::Equal).then(left.2.cmp(&right.2)));
        let mut runs: Vec<(f64, f64, Vec<usize>)> = Vec::new();
        for (low, high, index) in spans {
            match runs.last_mut() {
                Some(run) if low <= run.1 + tolerance => {
                    run.1 = run.1.max(high);
                    run.2.push(index);
                }
                _ => runs.push((low, high, vec![index])),
            }
        }
        for (low, high, indices) in runs {
            if indices.len() < 2 {
                continue;
            }
            if let Some(error) = locked_layer(drawing, &layer) {
                return Err(error);
            }
            let mut ids: Vec<(usize, String)> = indices.iter().map(|&index| (index, drawing.entities[index].id.clone())).collect();
            ids.sort();
            plan.merges.push(OverkillMerge {
                sources: ids.into_iter().map(|(_, id)| id).collect(),
                from: clean_point([from[0] + direction[0] * low, from[1] + direction[1] * low]),
                to: clean_point([from[0] + direction[0] * high, from[1] + direction[1] * high]),
            });
        }
    }
    Ok(plan)
}

/// Applies one [`plan_overkill`] result and returns each merge as sources to
/// the fresh id of its union line.
fn apply_overkill(drawing: &mut Drawing, plan: &OverkillPlan) -> Result<Vec<(Vec<String>, String)>, ServiceError> {
    for id in &plan.removed {
        let index = entity_index(drawing, id)?;
        drawing.entities.remove(index);
        drawing.forget_draw_order(id);
        detach_entity_from_groups(drawing, id);
    }
    let mut groups = Vec::with_capacity(plan.merges.len());
    for merge in &plan.merges {
        let layer = drawing.entities[entity_index(drawing, &merge.sources[0])?].layer.clone();
        let id = drawing.take_entity_id();
        let merged = Entity { id: id.clone(), layer, kind: EntityKind::Line { from: merge.from, to: merge.to } };
        let mut positions: Vec<usize> = Vec::with_capacity(merge.sources.len());
        for source in &merge.sources {
            positions.push(entity_index(drawing, source)?);
        }
        positions.sort_unstable();
        for &position in positions.iter().rev() {
            drawing.entities.remove(position);
        }
        drawing.entities.insert(positions[0], merged);
        if let Some(slot) = drawing.draw_order.iter().position(|candidate| merge.sources.iter().any(|source| source == candidate)) {
            drawing.draw_order.remove(slot);
            drawing.draw_order.insert(slot, id.clone());
        }
        for source in &merge.sources {
            drawing.forget_draw_order(source);
            detach_entity_from_groups(drawing, source);
        }
        groups.push((merge.sources.clone(), id));
    }
    drawing.revision += 1;
    Ok(groups)
}

/// Removes exact duplicate entities and optionally merges collinear lines.
///
/// An entity duplicates an earlier one when both sit on the same layer and
/// carry the same geometry within `tolerance` (default `1e-9` drawing units);
/// a line also matches its reversed twin. Only line, circle, arc, ellipse,
/// point, polyline and spline entities are compared; text, dimensions and
/// hatches are counted in `skipped` and never touched. When
/// `merge_overlapping` is set, lines on one layer that are collinear within
/// the same tolerance and whose spans overlap or touch merge into one union
/// line with a fresh deterministic id, reusing the collinearity test of
/// [`join_entities`].
///
/// The response carries `{ revision, drawing, removed, merged, skipped, before, after }`,
/// where `removed` lists the dropped duplicate ids, `merged` lists every group
/// as `{ sources, created }` and `before`/`after` count the entities around the
/// cleanup. A drawing with nothing to clean leaves the revision untouched. A
/// cleaning with changes is one history step and one revision, and a single
/// undo reverses the whole report. A non-finite or negative `tolerance`
/// reports `session/argument-invalid`; a duplicate or merge on a locked or
/// frozen layer reports `drawing/layer-locked` or `drawing/layer-frozen`.
pub fn overkill(handle: u64, request: OverkillRequest) -> Result<Value, ServiceError> {
    let tolerance = request.tolerance.unwrap_or(1e-9);
    if !tolerance.is_finite() || tolerance < 0.0 {
        return Err(argument("`tolerance` must be a finite non-negative number"));
    }
    let plan = store::read(handle, |session| plan_overkill(&session.drawing, tolerance, request.merge_overlapping.unwrap_or(false)))??;
    let groups = if plan.removed.is_empty() && plan.merges.is_empty() {
        Vec::new()
    }
    else {
        store::edit(handle, |drawing| apply_overkill(drawing, &plan))?
    };
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "removed": plan.removed,
            "merged": groups.iter().map(|(sources, id)| json!({ "sources": sources, "created": id })).collect::<Vec<_>>(),
            "skipped": plan.skipped,
            "before": plan.before,
            "after": session.drawing.entities.len(),
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
    offset_polyline_with(points, closed, distance, false)
}

fn offset_polyline_with(points: &[[f64; 2]], closed: bool, distance: f64, reject_long_miter: bool) -> Result<Vec<[f64; 2]>, ServiceError> {
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
    let mut moved = offset_vertices(unique, &normals, closed, distance, reject_long_miter)?;
    if repeated {
        moved.push(moved[0]);
    }
    Ok(moved)
}

fn offset_vertices(
    points: &[[f64; 2]],
    normals: &[[f64; 2]],
    closed: bool,
    distance: f64,
    reject_long_miter: bool,
) -> Result<Vec<[f64; 2]>, ServiceError> {
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
                let sum_length = (sum[0] * sum[0] + sum[1] * sum[1]).sqrt();
                if reject_long_miter {
                    if denominator <= 1e-12 || !denominator.is_finite() {
                        return Err(miter_limit());
                    }
                    let ratio = sum_length / denominator;
                    if !ratio.is_finite() || ratio > OFFSET_MITER_LIMIT {
                        return Err(miter_limit());
                    }
                    let factor = distance / denominator;
                    [sum[0] * factor, sum[1] * factor]
                }
                else if denominator.abs() < 1e-9 || sum_length < 1e-9 {
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
            return Err(if reject_long_miter {
                ServiceError::new("offset/invalid-distance", "the offset produces a non-finite vertex")
            }
            else {
                argument("the offset produces a non-finite vertex")
            });
        }
        moved.push(if reject_long_miter { clean_point(moved_point) } else { moved_point });
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
        EntityKind::Spline { closed, .. } => *closed,
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
        EntityKind::Spline { points, closed } => {
            let sampled = sample_spline(points, *closed);
            let area = if *closed && sampled.len() > 1 { polyline_area(&sampled) } else { 0.0 };
            (curve_length(&sampled), area)
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
/// analytic arc lengths; ellipse length is sampled at five-degree steps. A
/// spline is sampled at 16 segments per span for both length and area. `area`
/// covers closed polylines, closed splines, hatches, full circles and full
/// ellipses, so a closed polyline reports its perimeter as `length` and its
/// enclosed area as `area`.
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
        EntityKind::Spline { points, closed } => (sample_spline(points, *closed), *closed),
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
pub const SNAP_MODES: &[&str] = &["endpoint", "midpoint", "center", "quadrant", "nearest", "intersection", "perpendicular", "tangent"];

/// Modes an empty request selects.
///
/// `quadrant`, `nearest`, `perpendicular` and `tangent` are opt-in, so a query
/// that names no modes keeps the candidates it reported before those modes
/// existed.
pub const SNAP_DEFAULT_MODES: &[&str] = &["endpoint", "midpoint", "center", "intersection"];

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

/// Whether `angle` lies on the counter-clockwise sweep that starts at
/// `start_angle` and spans `sweep` degrees.
fn angle_within_sweep(start_angle: f64, sweep: f64, angle: f64) -> bool {
    sweep >= 360.0 - 1e-9 || normalize_angle(angle - start_angle) <= sweep + 1e-9
}

/// Closest point on the bounded segment `from -> to`.
fn segment_nearest(from: [f64; 2], to: [f64; 2], probe: [f64; 2]) -> [f64; 2] {
    let delta = [to[0] - from[0], to[1] - from[1]];
    let length_squared = delta[0] * delta[0] + delta[1] * delta[1];
    if length_squared < 1e-24 {
        return from;
    }
    let offset = [probe[0] - from[0], probe[1] - from[1]];
    let parameter = ((offset[0] * delta[0] + offset[1] * delta[1]) / length_squared).clamp(0.0, 1.0);
    [from[0] + delta[0] * parameter, from[1] + delta[1] * parameter]
}

/// Closest point on a point chain, one segment at a time.
fn polyline_nearest(points: &[[f64; 2]], closed: bool, probe: [f64; 2]) -> Option<[f64; 2]> {
    let mut edges = points.windows(2).map(|pair| (pair[0], pair[1])).collect::<Vec<_>>();
    if closed && points.len() > 2 {
        edges.push((points[points.len() - 1], points[0]));
    }
    edges
        .into_iter()
        .map(|(from, to)| segment_nearest(from, to, probe))
        .min_by(|left, right| point_distance(*left, probe).total_cmp(&point_distance(*right, probe)))
}

/// Closest point on a full circle; a probe at the centre resolves to the
/// quadrant on the positive X axis so the result stays deterministic.
fn circle_nearest(center: [f64; 2], radius: f64, probe: [f64; 2]) -> [f64; 2] {
    let offset = [probe[0] - center[0], probe[1] - center[1]];
    let length = offset[0].hypot(offset[1]);
    if length < 1e-12 {
        return [center[0] + radius, center[1]];
    }
    [center[0] + offset[0] / length * radius, center[1] + offset[1] / length * radius]
}

/// Closest point on an arc, clamped to its start and end angles.
fn arc_nearest(center: [f64; 2], radius: f64, start_angle: f64, sweep: f64, probe: [f64; 2]) -> [f64; 2] {
    let offset = [probe[0] - center[0], probe[1] - center[1]];
    let length = offset[0].hypot(offset[1]);
    if length < 1e-12 {
        return arc_point(center, radius, start_angle);
    }
    let probe_angle = offset[1].atan2(offset[0]).to_degrees();
    if angle_within_sweep(start_angle, sweep, probe_angle) {
        return arc_point(center, radius, probe_angle);
    }
    let start = arc_point(center, radius, start_angle);
    let end = arc_point(center, radius, start_angle + sweep);
    let angle = if point_distance(start, probe) <= point_distance(end, probe) { start_angle } else { start_angle + sweep };
    arc_point(center, radius, angle)
}

/// Foot of the perpendicular from `probe` to the bounded segment `from -> to`;
/// a projection outside the segment gives none.
fn segment_perpendicular(from: [f64; 2], to: [f64; 2], probe: [f64; 2]) -> Option<[f64; 2]> {
    let delta = [to[0] - from[0], to[1] - from[1]];
    let length_squared = delta[0] * delta[0] + delta[1] * delta[1];
    if length_squared < 1e-24 {
        return None;
    }
    let offset = [probe[0] - from[0], probe[1] - from[1]];
    let parameter = (offset[0] * delta[0] + offset[1] * delta[1]) / length_squared;
    if !within_segment(parameter) {
        return None;
    }
    Some([from[0] + delta[0] * parameter, from[1] + delta[1] * parameter])
}

/// The two points where the line through the centre and `probe` meets a full
/// circle; a probe at the centre gives none.
fn circle_perpendicular(center: [f64; 2], radius: f64, probe: [f64; 2]) -> Vec<[f64; 2]> {
    let offset = [probe[0] - center[0], probe[1] - center[1]];
    let length = offset[0].hypot(offset[1]);
    if length < 1e-12 {
        return Vec::new();
    }
    let unit = [offset[0] / length, offset[1] / length];
    vec![[center[0] + unit[0] * radius, center[1] + unit[1] * radius], [center[0] - unit[0] * radius, center[1] - unit[1] * radius]]
}

/// Perpendicular points of an arc: the two circle points whose angle lies on the
/// counter-clockwise sweep.
fn arc_perpendicular(center: [f64; 2], radius: f64, start_angle: f64, sweep: f64, probe: [f64; 2]) -> Vec<[f64; 2]> {
    let offset = [probe[0] - center[0], probe[1] - center[1]];
    let length = offset[0].hypot(offset[1]);
    if length < 1e-12 {
        return Vec::new();
    }
    let probe_angle = offset[1].atan2(offset[0]).to_degrees();
    [probe_angle, probe_angle + 180.0]
        .into_iter()
        .filter(|angle| angle_within_sweep(start_angle, sweep, *angle))
        .map(|angle| arc_point(center, radius, angle))
        .collect()
}

/// Directions, in degrees, of the tangent contact points from `probe` to a full
/// circle. A probe outside gives the two directions at `C -> P` plus and minus
/// `acos(r / d)`, a probe on the circle gives that one direction, and a probe
/// inside the circle or at its centre gives none.
fn circle_tangent_angles(center: [f64; 2], radius: f64, probe: [f64; 2]) -> Vec<f64> {
    if radius <= 1e-12 {
        return Vec::new();
    }
    let offset = [probe[0] - center[0], probe[1] - center[1]];
    let distance = offset[0].hypot(offset[1]);
    if distance < 1e-12 {
        return Vec::new();
    }
    let scale = distance.max(radius);
    if distance < radius - 1e-9 * scale {
        return Vec::new();
    }
    let base = offset[1].atan2(offset[0]).to_degrees();
    if distance <= radius + 1e-9 * scale {
        return vec![base];
    }
    let spread = (radius / distance).clamp(-1.0, 1.0).acos().to_degrees();
    vec![base + spread, base - spread]
}

/// Tangent points from `probe` to a full circle.
fn circle_tangent(center: [f64; 2], radius: f64, probe: [f64; 2]) -> Vec<[f64; 2]> {
    circle_tangent_angles(center, radius, probe).into_iter().map(|angle| arc_point(center, radius, angle)).collect()
}

/// Tangent points from `probe` to an arc: the circle contact points whose angle
/// lies on the counter-clockwise sweep.
fn arc_tangent(center: [f64; 2], radius: f64, start_angle: f64, sweep: f64, probe: [f64; 2]) -> Vec<[f64; 2]> {
    circle_tangent_angles(center, radius, probe)
        .into_iter()
        .filter(|angle| angle_within_sweep(start_angle, sweep, *angle))
        .map(|angle| arc_point(center, radius, angle))
        .collect()
}

/// Circle geometry of an entity, skipping a degenerate radius.
fn circle_geometry(kind: &EntityKind) -> Option<([f64; 2], f64)> {
    match kind {
        EntityKind::Circle { center, radius } if radius.abs() > 1e-12 => Some((*center, radius.abs())),
        _ => None,
    }
}

/// Circle geometry of an arc entity plus its counter-clockwise sweep, skipping
/// a degenerate radius.
fn arc_geometry(kind: &EntityKind) -> Option<(([f64; 2], f64), f64, f64)> {
    match kind {
        EntityKind::Arc { center, radius, start_angle, end_angle } if radius.abs() > 1e-12 => {
            Some(((*center, radius.abs()), *start_angle, arc_sweep(*start_angle, *end_angle)))
        }
        _ => None,
    }
}

/// Keeps only the points whose angle around `center` lies on the arc sweep.
fn retain_on_arc(points: Vec<[f64; 2]>, center: [f64; 2], start_angle: f64, sweep: f64) -> Vec<[f64; 2]> {
    points
        .into_iter()
        .filter(|point| angle_within_sweep(start_angle, sweep, (point[1] - center[1]).atan2(point[0] - center[0]).to_degrees()))
        .collect()
}

/// Points where a bounded segment meets a full circle; a tangent gives none.
fn segment_circle_intersections(from: [f64; 2], to: [f64; 2], center: [f64; 2], radius: f64) -> Vec<[f64; 2]> {
    let delta = [to[0] - from[0], to[1] - from[1]];
    let offset = [from[0] - center[0], from[1] - center[1]];
    let quadratic = delta[0] * delta[0] + delta[1] * delta[1];
    if quadratic < 1e-24 {
        return Vec::new();
    }
    let linear = 2.0 * (offset[0] * delta[0] + offset[1] * delta[1]);
    let constant = offset[0] * offset[0] + offset[1] * offset[1] - radius * radius;
    let discriminant = linear * linear - 4.0 * quadratic * constant;
    if discriminant <= 1e-9 * quadratic * radius * radius {
        return Vec::new();
    }
    let root = discriminant.sqrt();
    let mut points = Vec::new();
    for parameter in [(-linear - root) / (2.0 * quadratic), (-linear + root) / (2.0 * quadratic)] {
        if within_segment(parameter) {
            points.push([from[0] + delta[0] * parameter, from[1] + delta[1] * parameter]);
        }
    }
    points
}

/// Points where two full circles meet; tangency, coincidence and separation
/// give none.
fn circle_circle_intersections(first: ([f64; 2], f64), second: ([f64; 2], f64)) -> Vec<[f64; 2]> {
    let ((first_center, first_radius), (second_center, second_radius)) = (first, second);
    let delta = [second_center[0] - first_center[0], second_center[1] - first_center[1]];
    let distance = delta[0].hypot(delta[1]);
    let scale = (first_radius + second_radius).max(1e-12);
    if distance < 1e-12
        || distance <= (first_radius - second_radius).abs() + 1e-9 * scale
        || distance >= first_radius + second_radius - 1e-9 * scale
    {
        return Vec::new();
    }
    let along = (first_radius * first_radius - second_radius * second_radius + distance * distance) / (2.0 * distance);
    let height_squared = first_radius * first_radius - along * along;
    if height_squared <= 0.0 {
        return Vec::new();
    }
    let height = height_squared.sqrt();
    let base = [first_center[0] + delta[0] / distance * along, first_center[1] + delta[1] / distance * along];
    let offset = [-delta[1] / distance * height, delta[0] / distance * height];
    vec![[base[0] + offset[0], base[1] + offset[1]], [base[0] - offset[0], base[1] - offset[1]]]
}

/// Intersections one unordered entity pair contributes, in a deterministic
/// order: line to line, line to circle, circle to circle, then the arc pairs.
/// A crossing with an arc must lie on the counter-clockwise sweep of every arc
/// it touches.
fn pair_intersections(first: &EntityKind, second: &EntityKind) -> Vec<[f64; 2]> {
    let first_line = line_endpoints(first);
    let second_line = line_endpoints(second);
    if let (Some((first_from, first_to)), Some((second_from, second_to))) = (first_line, second_line) {
        return segment_intersection(first_from, first_to, second_from, second_to).into_iter().collect();
    }
    if let (Some((from, to)), Some((center, radius))) = (first_line, circle_geometry(second)) {
        return segment_circle_intersections(from, to, center, radius);
    }
    if let (Some((center, radius)), Some((from, to))) = (circle_geometry(first), second_line) {
        return segment_circle_intersections(from, to, center, radius);
    }
    if let (Some(first_circle), Some(second_circle)) = (circle_geometry(first), circle_geometry(second)) {
        return circle_circle_intersections(first_circle, second_circle);
    }
    if let (Some((from, to)), Some((second_circle, start_angle, sweep))) = (first_line, arc_geometry(second)) {
        let (center, radius) = second_circle;
        return retain_on_arc(segment_circle_intersections(from, to, center, radius), center, start_angle, sweep);
    }
    if let (Some((first_circle, start_angle, sweep)), Some((from, to))) = (arc_geometry(first), second_line) {
        let (center, radius) = first_circle;
        return retain_on_arc(segment_circle_intersections(from, to, center, radius), center, start_angle, sweep);
    }
    if let (Some((first_circle, start_angle, sweep)), Some(second_circle)) = (arc_geometry(first), circle_geometry(second)) {
        let (center, _) = first_circle;
        return retain_on_arc(circle_circle_intersections(first_circle, second_circle), center, start_angle, sweep);
    }
    if let (Some(first_circle), Some((second_circle, start_angle, sweep))) = (circle_geometry(first), arc_geometry(second)) {
        let (center, _) = second_circle;
        return retain_on_arc(circle_circle_intersections(first_circle, second_circle), center, start_angle, sweep);
    }
    if let (Some((first_circle, first_start, first_sweep)), Some((second_circle, second_start, second_sweep))) =
        (arc_geometry(first), arc_geometry(second))
    {
        let (first_center, _) = first_circle;
        let (second_center, _) = second_circle;
        let points = circle_circle_intersections(first_circle, second_circle);
        let on_first = retain_on_arc(points, first_center, first_start, first_sweep);
        return retain_on_arc(on_first, second_center, second_start, second_sweep);
    }
    Vec::new()
}

/// Collects the endpoints, midpoints, centres, quadrant points, nearest points,
/// perpendicular points and tangent points one entity contributes.
fn entity_snap_points(entity: &Entity, modes: &[&str], probe: Option<[f64; 2]>, out: &mut Vec<SnapCandidate>) {
    let endpoint = modes.contains(&"endpoint");
    let midpoint = modes.contains(&"midpoint");
    let center = modes.contains(&"center");
    let quadrant = modes.contains(&"quadrant");
    let nearest = modes.contains(&"nearest");
    let perpendicular = modes.contains(&"perpendicular");
    let tangent = modes.contains(&"tangent");
    match &entity.kind {
        EntityKind::Line { from, to } => {
            if endpoint {
                push_snap(out, "endpoint", &entity.id, *from);
                push_snap(out, "endpoint", &entity.id, *to);
            }
            if midpoint {
                push_snap(out, "midpoint", &entity.id, edge_midpoint(*from, *to));
            }
            if nearest && let Some(probe) = probe {
                push_snap(out, "nearest", &entity.id, segment_nearest(*from, *to, probe));
            }
            if perpendicular
                && let Some(probe) = probe
                && let Some(point) = segment_perpendicular(*from, *to, probe)
            {
                push_snap(out, "perpendicular", &entity.id, point);
            }
        }
        EntityKind::Circle { center: at, radius } => {
            let radius = radius.abs();
            if center {
                push_snap(out, "center", &entity.id, *at);
            }
            if radius <= 1e-12 {
                return;
            }
            if quadrant {
                for angle in [0.0, 90.0, 180.0, 270.0] {
                    push_snap(out, "quadrant", &entity.id, arc_point(*at, radius, angle));
                }
            }
            if nearest && let Some(probe) = probe {
                push_snap(out, "nearest", &entity.id, circle_nearest(*at, radius, probe));
            }
            if perpendicular && let Some(probe) = probe {
                for point in circle_perpendicular(*at, radius, probe) {
                    push_snap(out, "perpendicular", &entity.id, point);
                }
            }
            if tangent && let Some(probe) = probe {
                for point in circle_tangent(*at, radius, probe) {
                    push_snap(out, "tangent", &entity.id, point);
                }
            }
        }
        EntityKind::Arc { center: at, radius, start_angle, end_angle } => {
            let sweep = arc_sweep(*start_angle, *end_angle);
            let radius = radius.abs();
            if endpoint {
                push_snap(out, "endpoint", &entity.id, arc_point(*at, radius, *start_angle));
                push_snap(out, "endpoint", &entity.id, arc_point(*at, radius, *start_angle + sweep));
            }
            if midpoint {
                push_snap(out, "midpoint", &entity.id, arc_point(*at, radius, *start_angle + sweep * 0.5));
            }
            if center {
                push_snap(out, "center", &entity.id, *at);
            }
            if radius <= 1e-12 {
                return;
            }
            if quadrant {
                for angle in [0.0, 90.0, 180.0, 270.0] {
                    if angle_within_sweep(*start_angle, sweep, angle) {
                        push_snap(out, "quadrant", &entity.id, arc_point(*at, radius, angle));
                    }
                }
            }
            if nearest && let Some(probe) = probe {
                push_snap(out, "nearest", &entity.id, arc_nearest(*at, radius, *start_angle, sweep, probe));
            }
            if perpendicular && let Some(probe) = probe {
                for point in arc_perpendicular(*at, radius, *start_angle, sweep, probe) {
                    push_snap(out, "perpendicular", &entity.id, point);
                }
            }
            if tangent && let Some(probe) = probe {
                for point in arc_tangent(*at, radius, *start_angle, sweep, probe) {
                    push_snap(out, "tangent", &entity.id, point);
                }
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
            if nearest
                && let Some(probe) = probe
                && let Some(point) = polyline_nearest(points, *closed, probe)
            {
                push_snap(out, "nearest", &entity.id, point);
            }
            if perpendicular && let Some(probe) = probe {
                for pair in points.windows(2) {
                    if let Some(point) = segment_perpendicular(pair[0], pair[1], probe) {
                        push_snap(out, "perpendicular", &entity.id, point);
                    }
                }
                if *closed
                    && let (Some(first), Some(last)) = (points.first(), points.last())
                    && points.len() > 2
                    && let Some(point) = segment_perpendicular(*last, *first, probe)
                {
                    push_snap(out, "perpendicular", &entity.id, point);
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
        EntityKind::Spline { points, closed } => {
            let sampled = sample_spline(points, *closed);
            if endpoint {
                push_snap(out, "endpoint", &entity.id, *points.first().expect("a spline keeps at least two fit points"));
                if !*closed && let Some(last) = points.last() {
                    push_snap(out, "endpoint", &entity.id, *last);
                }
            }
            if midpoint && let Some(middle) = sampled.get(sampled.len() / 2) {
                push_snap(out, "midpoint", &entity.id, *middle);
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

/// Computes snap candidates without changing the drawing.
///
/// Modes are `endpoint`, `midpoint`, `center`, `quadrant`, `nearest`,
/// `intersection`, `perpendicular` and `tangent`. An empty list selects
/// `endpoint`, `midpoint`, `center` and `intersection`, the modes that predate
/// `quadrant`, `nearest`, `perpendicular` and `tangent`. `point` sorts
/// candidates by distance and `tolerance` keeps only candidates within that
/// distance; a tolerance without a point is refused, and so is `nearest`,
/// `perpendicular` or `tangent` without a point.
/// `ids` and `layer` narrow the entity set, and an unknown id or layer reports
/// `object/not-found`. The response carries
/// `{ revision, modes, point, tolerance, count, truncated, points }`, where each
/// point is `{ kind, point, entities, distance? }`.
pub fn snap_points(handle: u64, request: SnapRequest) -> Result<Value, ServiceError> {
    let mut selected_modes: Vec<&'static str> = Vec::new();
    if request.modes.is_empty() {
        selected_modes.extend_from_slice(SNAP_DEFAULT_MODES);
    }
    else {
        for raw in &request.modes {
            let mode = raw.trim().to_ascii_lowercase();
            let Some(mode) = SNAP_MODES.iter().copied().find(|candidate| *candidate == mode)
            else {
                return Err(argument(format!(
                    "`{raw}` is not a snap mode; use endpoint, midpoint, center, quadrant, nearest, intersection, perpendicular or tangent"
                )));
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
    if selected_modes.contains(&"nearest") && request.point.is_none() {
        return Err(argument("a nearest snap needs a point to measure from"));
    }
    if selected_modes.contains(&"perpendicular") && request.point.is_none() {
        return Err(argument("a perpendicular snap needs a point to measure from"));
    }
    if selected_modes.contains(&"tangent") && request.point.is_none() {
        return Err(argument("a tangent snap needs a point to measure from"));
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
            entity_snap_points(entity, &selected_modes, request.point, &mut candidates);
        }
        if selected_modes.contains(&"intersection") {
            for (index, first) in selected.iter().enumerate() {
                for second in selected.iter().skip(index + 1) {
                    for point in pair_intersections(&first.kind, &second.kind) {
                        candidates.push(SnapCandidate { kind: "intersection", point, entities: vec![first.id.clone(), second.id.clone()] });
                    }
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
                    .then_with(|| a.entities.cmp(&b.entities))
                    .then_with(|| a.kind.cmp(b.kind))
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

/// Arguments of [`hatch_pattern`].
#[derive(Debug, Clone)]
pub struct HatchPatternRequest {
    /// Identifier of the hatch entity to fill.
    pub id: String,
    /// Direction of the pattern lines in degrees; defaults to `0`.
    pub angle: Option<f64>,
    /// Perpendicular distance between adjacent pattern lines; defaults to `5`.
    pub spacing: Option<f64>,
    /// Adds a second family perpendicular to the first; defaults to `false`.
    pub cross: Option<bool>,
}

/// Maximum number of scanlines one pattern family may emit.
const HATCH_PATTERN_LINE_LIMIT: u64 = 100_000;

/// One clipped pattern segment as its two world-space endpoints.
type PatternSpan = ([f64; 2], [f64; 2]);

/// Scanline-fills one closed polygon with parallel segments at `angle` degrees.
///
/// The boundary is rotated by `-angle` so the pattern lines become horizontal,
/// then the scanline `v = k * spacing` for integer `k` (anchored to the world
/// origin) crosses each edge under a half-open rule that counts a shared
/// vertex exactly once. Levels within `1e-9` of the boundary are skipped, so
/// the boundary is not traced a second time and the rotation error of a
/// cardinal angle cannot add a coincident line. Crossings are sorted with a
/// total order and paired by the even-odd rule; every span longer than
/// `1e-12` is rotated back into world coordinates.
fn pattern_spans(points: &[[f64; 2]], angle: f64, spacing: f64) -> Result<Vec<PatternSpan>, ServiceError> {
    let (sin, cos) = angle.to_radians().sin_cos();
    let rotated: Vec<[f64; 2]> = points.iter().map(|point| [point[0] * cos + point[1] * sin, -point[0] * sin + point[1] * cos]).collect();
    let min_v = rotated.iter().map(|point| point[1]).fold(f64::INFINITY, f64::min);
    let max_v = rotated.iter().map(|point| point[1]).fold(f64::NEG_INFINITY, f64::max);
    let epsilon = 1e-9 * (max_v - min_v).abs().max(spacing);
    let first = (min_v / spacing).floor();
    let last = (max_v / spacing).ceil();
    let count = (last - first) as u64;
    if count > HATCH_PATTERN_LINE_LIMIT {
        return Err(ServiceError::new(
            "hatch/spacing-too-fine",
            format!("the pattern needs more than {HATCH_PATTERN_LINE_LIMIT} lines; increase `spacing`"),
        ));
    }
    let mut spans = Vec::new();
    for step in 0..=count {
        let level = (first + step as f64) * spacing;
        if level <= min_v + epsilon || level >= max_v - epsilon {
            continue;
        }
        let mut crossings: Vec<f64> = Vec::new();
        for index in 0..rotated.len() {
            let [u0, v0] = rotated[index];
            let [u1, v1] = rotated[(index + 1) % rotated.len()];
            if (v0 <= level && v1 > level) || (v1 <= level && v0 > level) {
                crossings.push(u0 + (level - v0) * (u1 - u0) / (v1 - v0));
            }
        }
        crossings.sort_by(f64::total_cmp);
        for pair in crossings.chunks(2) {
            if pair.len() == 2 && pair[1] - pair[0] > 1e-12 {
                spans.push((
                    [pair[0] * cos - level * sin, pair[0] * sin + level * cos],
                    [pair[1] * cos - level * sin, pair[1] * sin + level * cos],
                ));
            }
        }
    }
    Ok(spans)
}

/// Fills one hatch with pattern line segments clipped to its boundary.
///
/// The hatch keeps its boundary and pattern name; this operation adds the
/// pattern as real `line` entities on the hatch's layer, so every exporter
/// carries the fill. Lines are anchored to the world origin: the same request
/// on the same boundary always yields the same segments. `cross` adds a second
/// family perpendicular to the first. The edit is one history step and one
/// revision, and the response carries `{ revision, drawing, created }`.
///
/// An unknown id reports `object/not-found`; a non-hatch entity reports
/// `hatch/unsupported-entity`; a `SOLID` fill reports `hatch/solid-pattern`;
/// non-positive or non-finite spacing reports `hatch/invalid-spacing`; a
/// non-finite angle reports `hatch/invalid-angle`; a zero-area boundary reports
/// `hatch/degenerate-boundary`; a pattern beyond the line budget reports
/// `hatch/spacing-too-fine`; a locked or frozen layer reports
/// `drawing/layer-locked` or `drawing/layer-frozen`.
pub fn hatch_pattern(handle: u64, request: HatchPatternRequest) -> Result<Value, ServiceError> {
    let HatchPatternRequest { id, angle, spacing, cross } = request;
    let angle = angle.unwrap_or(0.0);
    if !angle.is_finite() {
        return Err(ServiceError::new("hatch/invalid-angle", "`angle` must be finite"));
    }
    let spacing = spacing.unwrap_or(5.0);
    if !spacing.is_finite() || spacing <= 0.0 {
        return Err(ServiceError::new("hatch/invalid-spacing", "`spacing` must be a positive finite number"));
    }
    let cross = cross.unwrap_or(false);
    let created = store::edit(handle, |drawing| {
        let index = entity_index(drawing, &id)?;
        let (points, pattern, layer) = match &drawing.entities[index].kind {
            EntityKind::Hatch { points, pattern } => (points.clone(), pattern.clone(), drawing.entities[index].layer.clone()),
            other => {
                return Err(ServiceError::new("hatch/unsupported-entity", format!("entity `{id}` is a {}, not a hatch", kind_name(other))));
            }
        };
        if let Some(error) = locked_layer(drawing, &layer) {
            return Err(error);
        }
        if pattern.eq_ignore_ascii_case("SOLID") {
            return Err(ServiceError::new("hatch/solid-pattern", format!("hatch `{id}` is a solid fill; pattern lines need a named pattern")));
        }
        if polyline_area(&points).abs() <= 1e-12 {
            return Err(ServiceError::new("hatch/degenerate-boundary", format!("hatch `{id}` has a zero-area boundary")));
        }
        let mut spans = pattern_spans(&points, angle, spacing)?;
        if cross {
            spans.extend(pattern_spans(&points, angle + 90.0, spacing)?);
        }
        let mut created = Vec::with_capacity(spans.len());
        for (from, to) in spans {
            let entity = Entity { id: drawing.take_entity_id(), layer: layer.clone(), kind: EntityKind::Line { from, to } };
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

/// Arguments of [`add_title_block`].
#[derive(Debug, Clone, Default)]
pub struct TitleBlockRequest {
    /// Layer that carries the frame and block; created when missing.
    pub layer: Option<String>,
    /// Sheet width; defaults to the A3 landscape width.
    pub width: Option<f64>,
    /// Sheet height; defaults to the A3 landscape height.
    pub height: Option<f64>,
    /// Distance from the sheet edge to the drawing frame; defaults to `10`.
    pub margin: Option<f64>,
    /// Height of the block text; defaults to `3.5`.
    pub text_height: Option<f64>,
    /// Title override; defaults to the drawing title.
    pub title: Option<String>,
    /// Drawing number override; defaults to the drawing id.
    pub number: Option<String>,
    /// Revision label override; defaults to the drawing revision.
    pub revision: Option<String>,
    /// Scale label; defaults to `1:1`.
    pub scale: Option<String>,
    /// Date label; defaults to an empty cell.
    pub date: Option<String>,
    /// Author override; defaults to the drawing author.
    pub author: Option<String>,
    /// Sheet label; defaults to `1`.
    pub sheet: Option<String>,
}

/// Default layer the generated title block is drawn on.
const TITLE_BLOCK_LAYER: &str = "titleblock";

/// Rows the generated block carries, top to bottom.
const TITLE_BLOCK_ROWS: [&str; 7] = ["Title", "Number", "Revision", "Scale", "Date", "Author", "Sheet"];

/// Adds a sheet frame and a title block filled from the drawing properties.
///
/// The sheet is a rectangle at the origin with the given `width` and `height`
/// (A3 landscape by default). The operation draws an outer border, an inner
/// frame inset by `margin`, and a seven-row block (Title, Number, Revision,
/// Scale, Date, Author, Sheet) in the bottom-right corner of the frame. Field
/// values default to the drawing title, id, revision, `1:1`, an empty date and
/// the drawing author; an override wins, and an empty override renders a blank
/// cell. Empty values emit no text entity, so only the grid stays. Everything
/// lands on `layer`, created with a neutral colour when missing, as `line` and
/// `text` entities with fresh deterministic ids. The edit is one history step
/// and one revision, and the response carries `{ revision, drawing, created }`.
///
/// Non-finite, zero or negative sheet dimensions report
/// `session/argument-invalid`; a sheet too small for the frame and block
/// reports `titleblock/invalid-sheet`; an override longer than the property
/// limit reports `session/argument-invalid`; a locked or frozen target layer
/// reports `drawing/layer-locked` or `drawing/layer-frozen`.
pub fn add_title_block(handle: u64, request: TitleBlockRequest) -> Result<Value, ServiceError> {
    let width = request.width.unwrap_or(DEFAULT_LAYOUT_WIDTH);
    let height = request.height.unwrap_or(DEFAULT_LAYOUT_HEIGHT);
    let margin = request.margin.unwrap_or(10.0);
    let text_height = request.text_height.unwrap_or(3.5);
    if !width.is_finite() || !height.is_finite() || !margin.is_finite() || !text_height.is_finite() {
        return Err(argument("`width`, `height`, `margin` and `textHeight` must be finite"));
    }
    if width <= 0.0 || height <= 0.0 || margin <= 0.0 || text_height <= 0.0 {
        return Err(argument("`width`, `height`, `margin` and `textHeight` must be positive"));
    }
    let overrides = [
        request.title.map(|raw| property_value(&raw, "title")).transpose()?,
        request.number.map(|raw| property_value(&raw, "number")).transpose()?,
        request.revision.map(|raw| property_value(&raw, "revision")).transpose()?,
        request.scale.map(|raw| property_value(&raw, "scale")).transpose()?,
        request.date.map(|raw| property_value(&raw, "date")).transpose()?,
        request.author.map(|raw| property_value(&raw, "author")).transpose()?,
        request.sheet.map(|raw| property_value(&raw, "sheet")).transpose()?,
    ];
    let layer_name = request.layer.map(|name| name.trim().to_string()).filter(|name| !name.is_empty());
    let row_height = text_height * 2.5;
    let block_height = row_height * TITLE_BLOCK_ROWS.len() as f64;
    let block_width = (width * 0.5).min(180.0);
    let label_width = block_width * 0.3;
    if block_width > width - 2.0 * margin || block_height > height - 2.0 * margin {
        return Err(ServiceError::new("titleblock/invalid-sheet", "the sheet is too small for the frame and title block"));
    }
    let created = store::edit(handle, |drawing| {
        let defaults = [
            drawing.title.clone().unwrap_or_default(),
            drawing.id.clone(),
            drawing.revision.to_string(),
            "1:1".to_string(),
            String::new(),
            drawing.properties.author.clone(),
            "1".to_string(),
        ];
        let values: Vec<String> =
            defaults.into_iter().zip(overrides).map(|(default, override_value)| override_value.unwrap_or(default)).collect();
        let layer = layer_name.clone().unwrap_or_else(|| TITLE_BLOCK_LAYER.to_string());
        if drawing.layer(&layer).is_none() {
            drawing.layers.push(Layer {
                id: layer.clone(),
                name: layer.clone(),
                color: "#d0d7de".to_string(),
                visible: true,
                locked: false,
                frozen: false,
                plottable: true,
                linetype: CONTINUOUS_LINETYPE.to_string(),
                lineweight: 0.0,
            });
        }
        if let Some(error) = locked_layer(drawing, &layer) {
            return Err(error);
        }
        let mut kinds: Vec<EntityKind> = Vec::new();
        kinds.extend_from_slice(&[
            EntityKind::Line { from: [0.0, 0.0], to: [width, 0.0] },
            EntityKind::Line { from: [width, 0.0], to: [width, height] },
            EntityKind::Line { from: [width, height], to: [0.0, height] },
            EntityKind::Line { from: [0.0, height], to: [0.0, 0.0] },
            EntityKind::Line { from: [margin, margin], to: [width - margin, margin] },
            EntityKind::Line { from: [width - margin, margin], to: [width - margin, height - margin] },
            EntityKind::Line { from: [width - margin, height - margin], to: [margin, height - margin] },
            EntityKind::Line { from: [margin, height - margin], to: [margin, margin] },
        ]);
        let bx0 = width - margin - block_width;
        let bx1 = width - margin;
        let by1 = margin + block_height;
        kinds.extend_from_slice(&[
            EntityKind::Line { from: [bx0, margin], to: [bx1, margin] },
            EntityKind::Line { from: [bx1, margin], to: [bx1, by1] },
            EntityKind::Line { from: [bx1, by1], to: [bx0, by1] },
            EntityKind::Line { from: [bx0, by1], to: [bx0, margin] },
        ]);
        for row in 1..TITLE_BLOCK_ROWS.len() {
            let y = by1 - row as f64 * row_height;
            kinds.push(EntityKind::Line { from: [bx0, y], to: [bx1, y] });
        }
        kinds.push(EntityKind::Line { from: [bx0 + label_width, margin], to: [bx0 + label_width, by1] });
        let inset = text_height * 0.5;
        for (row, (label, value)) in TITLE_BLOCK_ROWS.iter().zip(&values).enumerate() {
            let y = by1 - row as f64 * row_height + (row_height - text_height) / 2.0;
            kinds.push(EntityKind::Text { at: [bx0 + inset, y], height: text_height, content: (*label).to_string() });
            if !value.is_empty() {
                kinds.push(EntityKind::Text { at: [bx0 + label_width + inset, y], height: text_height, content: value.clone() });
            }
        }
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

/// Arguments of [`smooth_polyline`].
#[derive(Debug, Clone, Default)]
pub struct SmoothPolylineRequest {
    /// Identifier of the polyline to smooth.
    pub id: String,
    /// Corner-cutting passes; defaults to `2` and must be within `1..=8`.
    pub iterations: Option<u32>,
    /// Fraction each new corner is pulled towards its neighbour; defaults to
    /// `0.25` and must be strictly between `0` and `0.5`.
    pub ratio: Option<f64>,
}

/// Most vertices one smoothing request may produce.
const SMOOTH_VERTEX_LIMIT: u64 = 100_000;

/// One Chaikin corner-cutting pass over a vertex chain.
///
/// An open chain keeps its first and last vertex and replaces every segment
/// with the two points at `ratio` and `1 - ratio` along it. A closed chain
/// treats the last-to-first edge as one more segment and keeps no vertex.
/// The result is deterministic and preserves the winding order.
fn chaikin_pass(points: &[[f64; 2]], closed: bool, ratio: f64) -> Vec<[f64; 2]> {
    let mut result = Vec::with_capacity(points.len() * 2);
    if !closed {
        result.push(points[0]);
    }
    let segments = if closed { points.len() } else { points.len() - 1 };
    for index in 0..segments {
        let from = points[index];
        let to = points[(index + 1) % points.len()];
        let near = [from[0] + (to[0] - from[0]) * ratio, from[1] + (to[1] - from[1]) * ratio];
        let far = [from[0] + (to[0] - from[0]) * (1.0 - ratio), from[1] + (to[1] - from[1]) * (1.0 - ratio)];
        result.push(near);
        result.push(far);
    }
    if !closed {
        result.push(points[points.len() - 1]);
    }
    result
}

/// Rounds the corners of one polyline with Chaikin corner cutting.
///
/// Each pass replaces every segment by the two points `ratio` and
/// `1 - ratio` along it, so an open chain keeps its endpoints and a closed
/// chain keeps its closed flag and never duplicates the first vertex. The
/// entity keeps its id and layer, and the edit is one history step and one
/// revision. The response carries `{ revision, drawing, entity }`.
///
/// `iterations` defaults to `2` and `ratio` to `0.25`. An unknown id reports
/// `object/not-found`; a non-polyline reports
/// `polyline/unsupported-entity`; a chain with fewer than three vertices
/// reports `polyline/degenerate`; an iteration count outside `1..=8` reports
/// `polyline/invalid-iterations`; a ratio outside `(0, 0.5)` reports
/// `polyline/invalid-ratio`; a result beyond the vertex budget reports
/// `polyline/vertex-limit`; a locked or frozen layer reports
/// `drawing/layer-locked` or `drawing/layer-frozen`.
pub fn smooth_polyline(handle: u64, request: SmoothPolylineRequest) -> Result<Value, ServiceError> {
    let SmoothPolylineRequest { id, iterations, ratio } = request;
    let iterations = iterations.unwrap_or(2);
    if !(1..=8).contains(&iterations) {
        return Err(ServiceError::new("polyline/invalid-iterations", "`iterations` must be between 1 and 8"));
    }
    let ratio = ratio.unwrap_or(0.25);
    if !ratio.is_finite() || ratio <= 0.0 || ratio >= 0.5 {
        return Err(ServiceError::new("polyline/invalid-ratio", "`ratio` must be finite and strictly between 0 and 0.5"));
    }
    let entity = store::edit(handle, |drawing| {
        let index = entity_index(drawing, &id)?;
        let (points, closed, layer) = match &drawing.entities[index].kind {
            EntityKind::Polyline { points, closed } => (points.clone(), *closed, drawing.entities[index].layer.clone()),
            other => {
                return Err(ServiceError::new(
                    "polyline/unsupported-entity",
                    format!("entity `{id}` is a {}, not a polyline", kind_name(other)),
                ));
            }
        };
        if let Some(error) = locked_layer(drawing, &layer) {
            return Err(error);
        }
        if points.len() < 3 {
            return Err(ServiceError::new("polyline/degenerate", format!("polyline `{id}` needs at least 3 vertices to smooth")));
        }
        let mut smoothed = points;
        for _ in 0..iterations {
            if smoothed.len() as u64 * 2 > SMOOTH_VERTEX_LIMIT {
                return Err(ServiceError::new(
                    "polyline/vertex-limit",
                    format!("smoothing would exceed {SMOOTH_VERTEX_LIMIT} vertices; lower `iterations`"),
                ));
            }
            smoothed = chaikin_pass(&smoothed, closed, ratio);
        }
        drawing.entities[index].kind = EntityKind::Polyline { points: smoothed, closed };
        drawing.revision += 1;
        Ok(drawing.entities[index].clone())
    })?;
    store::read(handle, |session| {
        json!({
            "revision": session.drawing.revision,
            "drawing": session.summary(),
            "entity": entity,
        })
    })
}

/// Fields that differ between two versions of the same entity.
fn changed_entity_fields(before: &Entity, after: &Entity) -> Vec<&'static str> {
    let mut fields = Vec::new();
    if before.layer != after.layer {
        fields.push("layer");
    }
    if before.kind != after.kind {
        fields.push("geometry");
    }
    fields
}

/// Fields that differ between two versions of the same layer.
fn changed_layer_fields(before: &Layer, after: &Layer) -> Vec<&'static str> {
    let mut fields = Vec::new();
    if before.name != after.name {
        fields.push("name");
    }
    if before.color != after.color {
        fields.push("color");
    }
    if before.visible != after.visible {
        fields.push("visible");
    }
    if before.locked != after.locked {
        fields.push("locked");
    }
    if before.frozen != after.frozen {
        fields.push("frozen");
    }
    if before.plottable != after.plottable {
        fields.push("plottable");
    }
    if before.linetype != after.linetype {
        fields.push("linetype");
    }
    if before.lineweight != after.lineweight {
        fields.push("lineweight");
    }
    fields
}

/// Compact descriptor of one side of a [`diff`] call.
fn diff_side(drawing: &Drawing, path: &Path) -> Value {
    json!({
        "path": path.display().to_string(),
        "id": drawing.id,
        "title": drawing.title,
        "revision": drawing.revision,
        "units": drawing.units.as_str(),
        "entities": drawing.entities.len(),
        "layers": drawing.layers.len(),
    })
}

/// Drawing-level metadata differences between two projects.
fn diff_metadata(left: &Drawing, right: &Drawing) -> Vec<Value> {
    let mut entries = Vec::new();
    if left.title != right.title {
        entries.push(json!({ "field": "title", "left": left.title, "right": right.title }));
    }
    if left.units != right.units {
        entries.push(json!({ "field": "units", "left": left.units.as_str(), "right": right.units.as_str() }));
    }
    let properties = [
        ("author", &left.properties.author, &right.properties.author),
        ("description", &left.properties.description, &right.properties.description),
        ("company", &left.properties.company, &right.properties.company),
        ("project", &left.properties.project, &right.properties.project),
        ("keywords", &left.properties.keywords, &right.properties.keywords),
    ];
    for (field, before, after) in properties {
        if before != after {
            entries.push(json!({ "field": field, "left": before, "right": after }));
        }
    }
    entries
}

/// Compares two `.ocad` projects at entity level and returns a difference
/// report.
///
/// Entities and layers are matched by their stable id. The report carries
/// `added` (only in `right`), `removed` (only in `left`) and `changed`
/// entries with the differing `fields`, each in document order. Drawing-level
/// `metadata` covers the title, the unit and the declared properties. The
/// report is deterministic and does not modify either file; `identical` is
/// true when no entity, layer or metadata entry differs.
///
/// Revisions, dimension/text styles, line types, groups, layouts, draw order
/// and `nextEntity` are reported on each side but are not compared.
///
/// An unreadable, malformed or unsupported file reports the same codes as
/// [`open`]: `drawing/not-found`, `drawing/io-error`, `format/invalid` or
/// `format/unsupported-version`.
pub fn diff(left: &Path, right: &Path) -> Result<Value, ServiceError> {
    let left_drawing = load_drawing(left)?;
    let right_drawing = load_drawing(right)?;

    let left_entities = left_drawing.entities.iter().map(|entity| (entity.id.as_str(), entity)).collect::<std::collections::HashMap<_, _>>();
    let right_entities = right_drawing.entities.iter().map(|entity| (entity.id.as_str(), entity)).collect::<std::collections::HashMap<_, _>>();

    let added_entities = right_drawing.entities.iter().filter(|entity| !left_entities.contains_key(entity.id.as_str())).collect::<Vec<_>>();
    let removed_entities = left_drawing.entities.iter().filter(|entity| !right_entities.contains_key(entity.id.as_str())).collect::<Vec<_>>();
    let changed_entities = right_drawing
        .entities
        .iter()
        .filter_map(|entity| {
            left_entities
                .get(entity.id.as_str())
                .filter(|before| **before != entity)
                .map(|before| json!({ "id": entity.id, "fields": changed_entity_fields(before, entity), "left": before, "right": entity }))
        })
        .collect::<Vec<_>>();

    let left_layers = left_drawing.layers.iter().map(|layer| (layer.id.as_str(), layer)).collect::<std::collections::HashMap<_, _>>();
    let right_layers = right_drawing.layers.iter().map(|layer| (layer.id.as_str(), layer)).collect::<std::collections::HashMap<_, _>>();
    let added_layers = right_drawing.layers.iter().filter(|layer| !left_layers.contains_key(layer.id.as_str())).collect::<Vec<_>>();
    let removed_layers = left_drawing.layers.iter().filter(|layer| !right_layers.contains_key(layer.id.as_str())).collect::<Vec<_>>();
    let changed_layers = right_drawing
        .layers
        .iter()
        .filter_map(|layer| {
            left_layers
                .get(layer.id.as_str())
                .filter(|before| **before != layer)
                .map(|before| json!({ "id": layer.id, "fields": changed_layer_fields(before, layer), "left": before, "right": layer }))
        })
        .collect::<Vec<_>>();

    let metadata = diff_metadata(&left_drawing, &right_drawing);
    let identical = added_entities.is_empty()
        && removed_entities.is_empty()
        && changed_entities.is_empty()
        && added_layers.is_empty()
        && removed_layers.is_empty()
        && changed_layers.is_empty()
        && metadata.is_empty();

    Ok(json!({
        "identical": identical,
        "left": diff_side(&left_drawing, left),
        "right": diff_side(&right_drawing, right),
        "summary": {
            "added": added_entities.len(),
            "removed": removed_entities.len(),
            "changed": changed_entities.len(),
            "layersAdded": added_layers.len(),
            "layersRemoved": removed_layers.len(),
            "layersChanged": changed_layers.len(),
            "metadata": metadata.len(),
        },
        "entities": {
            "added": added_entities,
            "removed": removed_entities,
            "changed": changed_entities,
        },
        "layers": {
            "added": added_layers,
            "removed": removed_layers,
            "changed": changed_layers,
        },
        "metadata": metadata,
    }))
}

#[cfg(test)]
#[path = "../tests/unit/lib.rs"]
mod tests;

//! Thin Node-API surface for OpenCAD.
//!
//! Every export performs one orthogonal operation and returns a JSON string, so
//! the TypeScript layer owns argument parsing, help text and exit codes. The
//! drawing calls forward to `open-cad-drawing`, which owns the document store,
//! persistence, inspection, undo/redo history, layer and entity editing, the
//! ASCII DXF reader and SVG/DXF/PDF/STEP/STL export. Unimplemented routes keep
//! the stable `capability/not-implemented` code.
//!
//! Drawing handles come from the model crate's process-wide store, so a handle
//! returned by `drawing_create` or `drawing_open` stays valid for the lifetime
//! of the process until `drawing_close` releases it.
//!
//! The crate also hosts the WebView window that serves the built studio UI from
//! `projects/packages/open-cad/dist/gui`.

#![deny(missing_docs)]

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use open_cad_drawing::{
    ArrayMode, ArrayRequest, CopyRequest, CreateRequest, MirrorAxis, ServiceError, TransformOp, TransformRequest, TransformTarget,
};
use serde_json::{Value, json};
use wae::{
    CallContext, Error, JsValue, Result, function, optional_string_prop, register_module,
    webview::{DragRect, IpcHandler, IpcOutcome, IpcSink, OpenOptions, WebViewError, WebViewHost},
};

fn argument(ctx: &CallContext, index: usize) -> Result<JsValue> {
    ctx.args().get(index).copied().ok_or_else(|| Error::new("session/argument-invalid: a required argument is missing"))
}

fn text(ctx: &CallContext, index: usize) -> Result<String> {
    argument(ctx, index)?.as_string()
}

fn handle(ctx: &CallContext, index: usize) -> Result<u64> {
    let raw = argument(ctx, index)?.as_i64()?;
    u64::try_from(raw).map_err(|_| Error::new("session/argument-invalid: the drawing handle must be a non-negative integer"))
}

fn parse_request(raw: &str) -> Result<Value> {
    serde_json::from_str(raw).map_err(|error| Error::new(format!("session/argument-invalid: {error}")))
}

fn parse_checks(raw: &str) -> Result<Vec<String>> {
    serde_json::from_str(raw).map_err(|error| Error::new(format!("session/argument-invalid: {error}")))
}

fn fail(error: ServiceError) -> Error {
    Error::new(format!("{}: {}", error.code, error.message))
}

fn respond(ctx: &CallContext, value: &Value) -> Result<JsValue> {
    let text = value.to_string();
    ctx.env().string(&text)
}

/// Library version of the native binding.
#[function]
fn version(ctx: CallContext) -> Result<JsValue> {
    ctx.env().string(env!("CARGO_PKG_VERSION"))
}

/// Reports the format, check, renderer and export matrix of the engine.
///
/// The matrix comes from the same model crate the CLI uses, so neither entry
/// point can report different capabilities.
#[function]
fn capabilities(ctx: CallContext) -> Result<JsValue> {
    let matrix = open_cad_drawing::capabilities().to_string();
    ctx.env().string(&matrix)
}

/// Creates a drawing from a JSON request and returns `{ handle, drawing }`.
#[function]
fn drawing_create(ctx: CallContext) -> Result<JsValue> {
    let request = parse_request(&text(&ctx, 0)?)?;
    let created = open_cad_drawing::create(CreateRequest {
        title: request["title"].as_str().map(str::to_string),
        units: request["units"].as_str().map(str::to_string),
        template: request["template"].as_str().map(str::to_string),
    })
    .map_err(fail)?;
    respond(&ctx, &created)
}

/// Opens a project or exchange file and returns `{ handle, drawing }`.
#[function]
fn drawing_open(ctx: CallContext) -> Result<JsValue> {
    let path = text(&ctx, 0)?;
    let opened = open_cad_drawing::open(Path::new(&path)).map_err(fail)?;
    respond(&ctx, &opened)
}

/// Writes one open drawing to a project path.
#[function]
fn drawing_save(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let path = text(&ctx, 1)?;
    let overwrite = ctx.args().get(2).map(|value| value.as_bool()).transpose()?.unwrap_or(false);
    let report = call_save(handle, &path, overwrite).map_err(Error::new)?;
    respond(&ctx, &report)
}

/// Runs the requested checks on one open drawing.
#[function]
fn drawing_inspect(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let checks = match ctx.args().get(1) {
        Some(value) => Some(parse_checks(&value.as_string()?)?),
        None => None,
    };
    let report = open_cad_drawing::inspect(handle, checks).map_err(fail)?;
    respond(&ctx, &report)
}

/// Projects one open drawing into an exchange format.
#[function]
fn drawing_export(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let format = text(&ctx, 1)?;
    let path = text(&ctx, 2)?;
    let report = open_cad_drawing::export(handle, &format, Path::new(&path)).map_err(fail)?;
    respond(&ctx, &report)
}

/// Closes one drawing handle and releases its resources.
#[function]
fn drawing_close(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    open_cad_drawing::close(handle).map_err(fail)?;
    ctx.env().boolean(true)
}

/// Adds a line to one layer of an open drawing.
#[function]
fn drawing_add_line(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_add_line(handle, &request).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Adds a circle to one layer of an open drawing.
#[function]
fn drawing_add_circle(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_add_circle(handle, &request).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Adds a circular arc to one layer of an open drawing.
#[function]
fn drawing_add_arc(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_add_arc(handle, &request).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Adds a full or partial ellipse to one layer of an open drawing.
#[function]
fn drawing_add_ellipse(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_add_ellipse(handle, &request).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Adds a polyline to one layer of an open drawing.
#[function]
fn drawing_add_polyline(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_add_polyline(handle, &request).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Adds single-line text to one layer of an open drawing.
#[function]
fn drawing_add_text(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_add_text(handle, &request).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Adds a linear dimension to one layer of an open drawing.
#[function]
fn drawing_add_dimension(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_add_dimension(handle, &request).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Deletes one entity from an open drawing.
#[function]
fn drawing_delete_entity(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let entity = text(&ctx, 1)?;
    let result = call_delete_entity(handle, &entity).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Lists every entity of an open drawing.
#[function]
fn drawing_list_entities(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let result = call_list_entities(handle).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Restores the previous snapshot of an open drawing.
#[function]
fn drawing_undo(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let result = call_undo(handle).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Reapplies the snapshot the last undo replaced.
#[function]
fn drawing_redo(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let result = call_redo(handle).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Adds a layer to an open drawing.
#[function]
fn drawing_add_layer(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_add_layer(handle, &request).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Updates one layer of an open drawing.
#[function]
fn drawing_update_layer(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_update_layer(handle, &request).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Converts one open drawing between millimetres and inches.
#[function]
fn drawing_convert_units(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let unit = text(&ctx, 1)?;
    let result = open_cad_drawing::convert_units(handle, &unit).map_err(fail)?;
    respond(&ctx, &result)
}

/// Moves every entity of one layer onto another and drops the empty source.
#[function]
fn drawing_merge_layer(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let from = text(&ctx, 1)?;
    let to = text(&ctx, 2)?;
    let result = open_cad_drawing::merge_layer(handle, &from, &to).map_err(fail)?;
    respond(&ctx, &result)
}

/// Moves one entity of an open drawing.
#[function]
fn drawing_move_entity(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_move_entity(handle, &request).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Updates the editable payload of one entity.
#[function]
fn drawing_update_entity(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_update_entity(handle, &request).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Applies a translate/rotate/scale/mirror operation to a selection.
#[function]
fn drawing_transform(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_transform(handle, &request).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Duplicates entities with a cumulative offset.
#[function]
fn drawing_copy(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_copy(handle, &request).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Duplicates entities into a rectangular or polar array.
#[function]
fn drawing_array(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_array(handle, &request).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Splits one polyline into independent line entities.
#[function]
fn drawing_explode(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let entity = text(&ctx, 1)?;
    let result = open_cad_drawing::explode(handle, &entity).map_err(fail)?;
    respond(&ctx, &result)
}

/// Measures the bounds, path length and enclosed area of an open drawing.
#[function]
fn drawing_measure(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let sheet = match ctx.args().get(1) {
        Some(value) => Some(value.as_string()?),
        None => None,
    };
    let result = open_cad_drawing::measure(handle, sheet).map_err(fail)?;
    respond(&ctx, &result)
}

/// Imports the declared ASCII DXF subset into a new drawing.
#[function]
fn drawing_import_dxf(ctx: CallContext) -> Result<JsValue> {
    let request = parse_request(&text(&ctx, 0)?)?;
    let result = call_import_dxf(&request).map_err(Error::new)?;
    respond(&ctx, &result)
}

fn service_failure(error: ServiceError) -> String {
    format!("{}: {}", error.code, error.message)
}

fn call_create(request: &Value) -> std::result::Result<Value, String> {
    open_cad_drawing::create(CreateRequest {
        title: request["title"].as_str().map(str::to_string),
        units: request["units"].as_str().map(str::to_string),
        template: request["template"].as_str().map(str::to_string),
    })
    .map_err(service_failure)
}

fn call_open(path: &str) -> std::result::Result<Value, String> {
    open_cad_drawing::open(Path::new(path)).map_err(service_failure)
}

fn call_save(handle: u64, path: &str, overwrite: bool) -> std::result::Result<Value, String> {
    let bytes = open_cad_drawing::save(handle, Path::new(path), overwrite).map_err(service_failure)?;
    let summary = open_cad_drawing::summary(handle).map_err(service_failure)?;
    Ok(json!({ "path": path, "bytes": bytes, "revision": summary["revision"] }))
}

fn call_inspect(handle: u64, checks: Option<Vec<String>>) -> std::result::Result<Value, String> {
    open_cad_drawing::inspect(handle, checks).map_err(service_failure)
}

fn call_export(handle: u64, format: &str, path: &str) -> std::result::Result<Value, String> {
    open_cad_drawing::export(handle, format, Path::new(path)).map_err(service_failure)
}

fn call_close(handle: u64) -> std::result::Result<Value, String> {
    open_cad_drawing::close(handle).map_err(service_failure)?;
    Ok(json!({ "closed": true }))
}

fn request_layer(request: &Value) -> std::result::Result<String, String> {
    request.get("layer").and_then(Value::as_str).map(str::to_string).ok_or_else(|| "session/argument-invalid: `layer` is required".to_string())
}

fn request_point(request: &Value, key: &str) -> std::result::Result<[f64; 2], String> {
    let point =
        request.get(key).and_then(Value::as_array).ok_or_else(|| format!("session/argument-invalid: `{key}` must be a [x, y] array"))?;
    let x = point.first().and_then(Value::as_f64).ok_or_else(|| format!("session/argument-invalid: `{key}` needs two numbers"))?;
    let y = point.get(1).and_then(Value::as_f64).ok_or_else(|| format!("session/argument-invalid: `{key}` needs two numbers"))?;
    Ok([x, y])
}

fn request_number(request: &Value, key: &str) -> std::result::Result<f64, String> {
    request.get(key).and_then(Value::as_f64).ok_or_else(|| format!("session/argument-invalid: `{key}` must be a number"))
}

fn request_u32(request: &Value, key: &str) -> std::result::Result<u32, String> {
    let value =
        request.get(key).and_then(Value::as_u64).ok_or_else(|| format!("session/argument-invalid: `{key}` must be a positive integer"))?;
    u32::try_from(value).map_err(|_| format!("session/argument-invalid: `{key}` is too large"))
}

fn request_string(request: &Value, key: &str) -> std::result::Result<String, String> {
    request.get(key).and_then(Value::as_str).map(str::to_string).ok_or_else(|| format!("session/argument-invalid: `{key}` must be a string"))
}

fn request_points(request: &Value) -> std::result::Result<Vec<[f64; 2]>, String> {
    let points = request
        .get("points")
        .and_then(Value::as_array)
        .ok_or_else(|| "session/argument-invalid: `points` must be an array of [x, y]".to_string())?;
    points
        .iter()
        .map(|point| {
            let pair = point.as_array().ok_or_else(|| "session/argument-invalid: every point needs two numbers".to_string())?;
            let x =
                pair.first().and_then(Value::as_f64).ok_or_else(|| "session/argument-invalid: every point needs two numbers".to_string())?;
            let y = pair.get(1).and_then(Value::as_f64).ok_or_else(|| "session/argument-invalid: every point needs two numbers".to_string())?;
            Ok([x, y])
        })
        .collect()
}

fn call_add_line(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let layer = request_layer(request)?;
    let from = request_point(request, "from")?;
    let to = request_point(request, "to")?;
    open_cad_drawing::add_line(handle, &layer, from, to).map_err(service_failure)
}

fn call_add_circle(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let layer = request_layer(request)?;
    let center = request_point(request, "center")?;
    let radius = request_number(request, "radius")?;
    open_cad_drawing::add_circle(handle, &layer, center, radius).map_err(service_failure)
}

fn call_add_arc(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let layer = request_layer(request)?;
    let center = request_point(request, "center")?;
    let radius = request_number(request, "radius")?;
    let start_angle = request_number(request, "start_angle")?;
    let end_angle = request_number(request, "end_angle")?;
    open_cad_drawing::add_arc(handle, &layer, center, radius, start_angle, end_angle).map_err(service_failure)
}

fn call_add_ellipse(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let layer = request_layer(request)?;
    let center = request_point(request, "center")?;
    let rx = request_number(request, "rx")?;
    let ry = request_number(request, "ry")?;
    let rotation = request_number(request, "rotation")?;
    let start_angle = request_number(request, "start_angle")?;
    let end_angle = request_number(request, "end_angle")?;
    open_cad_drawing::add_ellipse(handle, &layer, center, rx, ry, rotation, start_angle, end_angle).map_err(service_failure)
}

fn call_add_polyline(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let layer = request_layer(request)?;
    let points = request_points(request)?;
    let closed = request.get("closed").and_then(Value::as_bool).unwrap_or(false);
    open_cad_drawing::add_polyline(handle, &layer, points, closed).map_err(service_failure)
}

fn call_add_text(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let layer = request_layer(request)?;
    let at = request_point(request, "at")?;
    let height = request_number(request, "height")?;
    let content = request_string(request, "content")?;
    open_cad_drawing::add_text(handle, &layer, at, height, &content).map_err(service_failure)
}

fn call_add_dimension(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let layer = request_layer(request)?;
    let from = request_point(request, "from")?;
    let to = request_point(request, "to")?;
    let offset = request_number(request, "offset")?;
    let text = request.get("text").and_then(Value::as_str).map(str::to_string);
    open_cad_drawing::add_dimension(handle, &layer, from, to, offset, text).map_err(service_failure)
}

fn call_delete_entity(handle: u64, entity: &str) -> std::result::Result<Value, String> {
    open_cad_drawing::delete_entity(handle, entity).map_err(service_failure)
}

fn call_list_entities(handle: u64) -> std::result::Result<Value, String> {
    open_cad_drawing::list_entities(handle).map_err(service_failure)
}

fn call_undo(handle: u64) -> std::result::Result<Value, String> {
    open_cad_drawing::undo(handle).map_err(service_failure)
}

fn call_redo(handle: u64) -> std::result::Result<Value, String> {
    open_cad_drawing::redo(handle).map_err(service_failure)
}

fn call_add_layer(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let name = request_string(request, "name")?;
    let color = request.get("color").and_then(Value::as_str);
    open_cad_drawing::add_layer(handle, &name, color).map_err(service_failure)
}

fn call_update_layer(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let layer = request_string(request, "layerId")?;
    let update = open_cad_drawing::LayerUpdate {
        name: request.get("name").and_then(Value::as_str).map(str::to_string),
        color: request.get("color").and_then(Value::as_str).map(str::to_string),
        visible: request.get("visible").and_then(Value::as_bool),
        locked: request.get("locked").and_then(Value::as_bool),
    };
    open_cad_drawing::update_layer(handle, &layer, update).map_err(service_failure)
}

fn call_move_entity(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let entity = request_string(request, "entityId")?;
    let target = if request.get("to").is_some() { Some(request_point(request, "to")?) } else { None };
    let offset = if request.get("dx").is_some() || request.get("dy").is_some() {
        let dx = request.get("dx").and_then(Value::as_f64).unwrap_or(0.0);
        let dy = request.get("dy").and_then(Value::as_f64).unwrap_or(0.0);
        Some([dx, dy])
    }
    else {
        None
    };
    open_cad_drawing::move_entity(handle, &entity, offset, target).map_err(service_failure)
}

fn call_update_entity(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let entity = request_string(request, "entityId")?;
    let content = request.get("content").and_then(Value::as_str).map(str::to_string);
    let height = request.get("height").and_then(Value::as_f64);
    let radius = request.get("radius").and_then(Value::as_f64);
    open_cad_drawing::update_entity(handle, &entity, content, height, radius).map_err(service_failure)
}

fn call_transform(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let sheet = request.get("sheet").and_then(Value::as_str).map(str::to_string);
    let target_value = request.get("target").ok_or_else(|| "session/argument-invalid: `target` is required".to_string())?;
    let target = if let Some(ids) = target_value.get("ids") {
        let ids = ids
            .as_array()
            .ok_or_else(|| "session/argument-invalid: `target.ids` must be an array".to_string())?
            .iter()
            .map(|id| id.as_str().map(str::to_string).ok_or_else(|| "session/argument-invalid: every id must be a string".to_string()))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        TransformTarget::Ids(ids)
    }
    else if let Some(layer) = target_value.get("layer") {
        TransformTarget::Layer(
            layer.as_str().ok_or_else(|| "session/argument-invalid: `target.layer` must be a string".to_string())?.to_string(),
        )
    }
    else if target_value.get("all").and_then(Value::as_bool) == Some(true) {
        TransformTarget::All
    }
    else {
        return Err("session/argument-invalid: `target` needs `ids`, `layer` or `all`".to_string());
    };
    let operation_value = request.get("operation").ok_or_else(|| "session/argument-invalid: `operation` is required".to_string())?;
    let operation = match operation_value.get("op").and_then(Value::as_str) {
        Some("translate") => TransformOp::Translate { dx: request_number(operation_value, "dx")?, dy: request_number(operation_value, "dy")? },
        Some("rotate") => TransformOp::Rotate {
            angle: request_number(operation_value, "angle")?,
            cx: request_number(operation_value, "cx")?,
            cy: request_number(operation_value, "cy")?,
        },
        Some("scale") => TransformOp::Scale {
            factor: request_number(operation_value, "factor")?,
            cx: request_number(operation_value, "cx")?,
            cy: request_number(operation_value, "cy")?,
        },
        Some("mirror") => {
            let axis = match operation_value.get("axis").and_then(Value::as_str) {
                Some("x") => MirrorAxis::X,
                Some("y") => MirrorAxis::Y,
                _ => return Err("session/argument-invalid: `operation.axis` must be `x` or `y`".to_string()),
            };
            TransformOp::Mirror { axis, at: request_number(operation_value, "at")? }
        }
        _ => return Err("session/argument-invalid: `operation.op` must be translate, rotate, scale or mirror".to_string()),
    };
    open_cad_drawing::transform(handle, TransformRequest { sheet, target, operation }).map_err(service_failure)
}

fn call_copy(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let ids = request
        .get("ids")
        .and_then(Value::as_array)
        .ok_or_else(|| "session/argument-invalid: `ids` must be an array".to_string())?
        .iter()
        .map(|id| id.as_str().map(str::to_string).ok_or_else(|| "session/argument-invalid: every id must be a string".to_string()))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let count = request
        .get("count")
        .and_then(Value::as_u64)
        .ok_or_else(|| "session/argument-invalid: `count` must be a positive integer".to_string())?;
    let count = u32::try_from(count).map_err(|_| "session/argument-invalid: `count` is too large".to_string())?;
    open_cad_drawing::copy(
        handle,
        CopyRequest {
            ids,
            dx: request.get("dx").and_then(Value::as_f64).unwrap_or(0.0),
            dy: request.get("dy").and_then(Value::as_f64).unwrap_or(0.0),
            count,
        },
    )
    .map_err(service_failure)
}

fn call_array(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let ids = request
        .get("ids")
        .and_then(Value::as_array)
        .ok_or_else(|| "session/argument-invalid: `ids` must be an array".to_string())?
        .iter()
        .map(|id| id.as_str().map(str::to_string).ok_or_else(|| "session/argument-invalid: every id must be a string".to_string()))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mode = match request.get("mode").and_then(Value::as_str) {
        Some("rectangular") => ArrayMode::Rectangular {
            rows: request_u32(request, "rows")?,
            columns: request_u32(request, "columns")?,
            row_spacing: request_number(request, "row_spacing")?,
            column_spacing: request_number(request, "column_spacing")?,
        },
        Some("polar") => ArrayMode::Polar {
            center: request_point(request, "center")?,
            count: request_u32(request, "count")?,
            total_angle: request_number(request, "total_angle")?,
        },
        Some(other) => return Err(format!("session/argument-invalid: unknown array mode `{other}`")),
        None => return Err("session/argument-invalid: `mode` must be `rectangular` or `polar`".to_string()),
    };
    open_cad_drawing::array(handle, ArrayRequest { ids, mode }).map_err(service_failure)
}

fn call_import_dxf(request: &Value) -> std::result::Result<Value, String> {
    let source = request_string(request, "source")?;
    let title = request.get("title").and_then(Value::as_str).map(str::to_string);
    open_cad_drawing::import_dxf(Path::new(&source), title).map_err(service_failure)
}

fn handle_of(value: &Value) -> Option<u64> {
    value.get("handle").and_then(Value::as_u64)
}

fn reply(id: Value, kind: &str, payload: Value) -> IpcOutcome {
    let mut body = payload;
    body["type"] = Value::String(kind.to_string());
    body["id"] = id;
    IpcOutcome::ReplyToPage(body.to_string())
}

fn failure(id: Value, kind: &str, message: String) -> IpcOutcome {
    reply(id, kind, json!({ "message": message }))
}

fn platform_name() -> &'static str {
    if cfg!(windows) {
        "win32"
    }
    else if cfg!(target_os = "macos") {
        "darwin"
    }
    else {
        "linux"
    }
}

/// Window actions the page cannot perform directly; the host applies them on the
/// thread that owns the window before the next message pump.
#[derive(Clone, Copy)]
enum WindowCommand {
    Minimize,
    ToggleMaximize,
}

static PENDING_DRAG_RECTS: Mutex<Option<Vec<DragRect>>> = Mutex::new(None);
static PENDING_WINDOW_COMMANDS: Mutex<Vec<WindowCommand>> = Mutex::new(Vec::new());

fn parse_drag_rects(value: &Value) -> Option<Vec<DragRect>> {
    let items = value.as_array()?;
    let mut rects = Vec::with_capacity(items.len());
    for item in items {
        let number = |key: &str| -> Option<i32> {
            let raw = item.get(key)?.as_i64()?;
            i32::try_from(raw).ok()
        };
        rects.push(DragRect { x: number("x")?, y: number("y")?, width: number("width")?, height: number("height")? });
    }
    Some(rects)
}

fn queue_window_command(command: WindowCommand) {
    if let Ok(mut queue) = PENDING_WINDOW_COMMANDS.lock() {
        queue.push(command);
    }
}

fn apply_window_commands(host: &mut WebViewHost) {
    if let Ok(mut pending) = PENDING_DRAG_RECTS.lock()
        && let Some(rects) = pending.take()
    {
        let _ = host.set_drag_regions(&rects);
    }
    let commands = PENDING_WINDOW_COMMANDS.lock().map(|mut queue| std::mem::take(&mut *queue)).unwrap_or_default();
    for command in commands {
        match command {
            WindowCommand::Minimize => {
                let _ = host.minimize();
            }
            WindowCommand::ToggleMaximize => {
                let _ = host.toggle_maximize();
            }
        }
    }
}

struct StudioIpcHandler {
    close_requested: Arc<AtomicBool>,
}

impl IpcHandler for StudioIpcHandler {
    fn on_ipc(&mut self, message: &str, _sink: &IpcSink) -> IpcOutcome {
        let Ok(value) = serde_json::from_str::<Value>(message)
        else {
            return IpcOutcome::Ignored;
        };
        let ty = value.get("type").and_then(Value::as_str).unwrap_or("");
        let id = value.get("id").cloned().unwrap_or(Value::Null);
        match ty {
            "close" => {
                self.close_requested.store(true, Ordering::SeqCst);
                IpcOutcome::CloseWindow
            }
            "ready" => reply(
                id,
                "readyResult",
                json!({
                    "version": env!("CARGO_PKG_VERSION"),
                    "platform": platform_name(),
                    "window": { "customChrome": cfg!(target_os = "windows") },
                }),
            ),
            "window.dragRegions" => {
                let Some(rects) = value.get("rects").and_then(parse_drag_rects)
                else {
                    return failure(id, "windowDragRegionsError", "a dragRegions request needs rects".to_string());
                };
                let count = rects.len();
                if let Ok(mut pending) = PENDING_DRAG_RECTS.lock() {
                    *pending = Some(rects);
                }
                reply(id, "windowDragRegionsResult", json!({ "count": count }))
            }
            "window.minimize" => {
                queue_window_command(WindowCommand::Minimize);
                reply(id, "windowMinimizeResult", json!({ "queued": true }))
            }
            "window.toggleMaximize" => {
                queue_window_command(WindowCommand::ToggleMaximize);
                reply(id, "windowToggleMaximizeResult", json!({ "queued": true }))
            }
            "window.close" => {
                self.close_requested.store(true, Ordering::SeqCst);
                IpcOutcome::CloseWindow
            }
            "capabilities" => reply(id, "capabilitiesResult", json!({ "capabilities": open_cad_drawing::capabilities() })),
            "drawing.create" => {
                let request = value.get("request").cloned().unwrap_or(Value::Null);
                match call_create(&request) {
                    Ok(result) => reply(id, "createResult", result),
                    Err(message) => failure(id, "createError", message),
                }
            }
            "drawing.open" => {
                let path = value.get("path").and_then(Value::as_str).unwrap_or("").trim().to_string();
                if path.is_empty() {
                    return failure(id, "openError", "an open request needs a project path".to_string());
                }
                match call_open(&path) {
                    Ok(result) => reply(id, "openResult", result),
                    Err(message) => failure(id, "openError", message),
                }
            }
            "drawing.save" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "saveError", "a save request needs a drawing handle".to_string());
                };
                let path = value.get("path").and_then(Value::as_str).unwrap_or("").trim().to_string();
                if path.is_empty() {
                    return failure(id, "saveError", "a save request needs a project path".to_string());
                }
                let overwrite = value.get("overwrite").and_then(Value::as_bool).unwrap_or(true);
                match call_save(handle, &path, overwrite) {
                    Ok(report) => reply(id, "saveResult", json!({ "report": report })),
                    Err(message) => failure(id, "saveError", message),
                }
            }
            "drawing.inspect" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "inspectError", "an inspect request needs a drawing handle".to_string());
                };
                let checks = value
                    .get("checks")
                    .and_then(Value::as_array)
                    .map(|items| items.iter().filter_map(Value::as_str).map(str::to_string).collect());
                match call_inspect(handle, checks) {
                    Ok(report) => reply(id, "inspectResult", json!({ "report": report })),
                    Err(message) => failure(id, "inspectError", message),
                }
            }
            "drawing.export" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "exportError", "an export request needs a drawing handle".to_string());
                };
                let format = value.get("format").and_then(Value::as_str).unwrap_or("").trim().to_string();
                let path = value.get("path").and_then(Value::as_str).unwrap_or("").trim().to_string();
                if format.is_empty() || path.is_empty() {
                    return failure(id, "exportError", "an export request needs a format and a target path".to_string());
                }
                match call_export(handle, &format, &path) {
                    Ok(report) => reply(id, "exportResult", json!({ "report": report })),
                    Err(message) => failure(id, "exportError", message),
                }
            }
            "drawing.close" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "closeError", "a close request needs a drawing handle".to_string());
                };
                match call_close(handle) {
                    Ok(result) => reply(id, "closeResult", result),
                    Err(message) => failure(id, "closeError", message),
                }
            }
            "drawing.addLine" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "addLineError", "an addLine request needs a drawing handle".to_string());
                };
                match call_add_line(handle, &value) {
                    Ok(result) => reply(id, "addLineResult", result),
                    Err(message) => failure(id, "addLineError", message),
                }
            }
            "drawing.addCircle" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "addCircleError", "an addCircle request needs a drawing handle".to_string());
                };
                match call_add_circle(handle, &value) {
                    Ok(result) => reply(id, "addCircleResult", result),
                    Err(message) => failure(id, "addCircleError", message),
                }
            }
            "drawing.addArc" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "addArcError", "an addArc request needs a drawing handle".to_string());
                };
                match call_add_arc(handle, &value) {
                    Ok(result) => reply(id, "addArcResult", result),
                    Err(message) => failure(id, "addArcError", message),
                }
            }
            "drawing.addEllipse" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "addEllipseError", "an addEllipse request needs a drawing handle".to_string());
                };
                match call_add_ellipse(handle, &value) {
                    Ok(result) => reply(id, "addEllipseResult", result),
                    Err(message) => failure(id, "addEllipseError", message),
                }
            }
            "drawing.addPolyline" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "addPolylineError", "an addPolyline request needs a drawing handle".to_string());
                };
                match call_add_polyline(handle, &value) {
                    Ok(result) => reply(id, "addPolylineResult", result),
                    Err(message) => failure(id, "addPolylineError", message),
                }
            }
            "drawing.addText" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "addTextError", "an addText request needs a drawing handle".to_string());
                };
                match call_add_text(handle, &value) {
                    Ok(result) => reply(id, "addTextResult", result),
                    Err(message) => failure(id, "addTextError", message),
                }
            }
            "drawing.addDimension" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "addDimensionError", "an addDimension request needs a drawing handle".to_string());
                };
                match call_add_dimension(handle, &value) {
                    Ok(result) => reply(id, "addDimensionResult", result),
                    Err(message) => failure(id, "addDimensionError", message),
                }
            }
            "drawing.deleteEntity" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "deleteEntityError", "a deleteEntity request needs a drawing handle".to_string());
                };
                let entity = value.get("entityId").and_then(Value::as_str).unwrap_or("").trim().to_string();
                if entity.is_empty() {
                    return failure(id, "deleteEntityError", "a deleteEntity request needs an entity id".to_string());
                }
                match call_delete_entity(handle, &entity) {
                    Ok(result) => reply(id, "deleteEntityResult", result),
                    Err(message) => failure(id, "deleteEntityError", message),
                }
            }
            "drawing.listEntities" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "listEntitiesError", "a listEntities request needs a drawing handle".to_string());
                };
                match call_list_entities(handle) {
                    Ok(result) => reply(id, "listEntitiesResult", result),
                    Err(message) => failure(id, "listEntitiesError", message),
                }
            }
            "drawing.undo" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "undoError", "an undo request needs a drawing handle".to_string());
                };
                match call_undo(handle) {
                    Ok(result) => reply(id, "undoResult", result),
                    Err(message) => failure(id, "undoError", message),
                }
            }
            "drawing.redo" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "redoError", "a redo request needs a drawing handle".to_string());
                };
                match call_redo(handle) {
                    Ok(result) => reply(id, "redoResult", result),
                    Err(message) => failure(id, "redoError", message),
                }
            }
            "drawing.addLayer" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "addLayerError", "an addLayer request needs a drawing handle".to_string());
                };
                let request = value.get("request").cloned().unwrap_or_else(|| value.clone());
                match call_add_layer(handle, &request) {
                    Ok(result) => reply(id, "addLayerResult", result),
                    Err(message) => failure(id, "addLayerError", message),
                }
            }
            "drawing.updateLayer" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "updateLayerError", "an updateLayer request needs a drawing handle".to_string());
                };
                let request = value.get("request").cloned().unwrap_or_else(|| value.clone());
                match call_update_layer(handle, &request) {
                    Ok(result) => reply(id, "updateLayerResult", result),
                    Err(message) => failure(id, "updateLayerError", message),
                }
            }
            "drawing.mergeLayer" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "mergeLayerError", "a mergeLayer request needs a drawing handle".to_string());
                };
                let request = value.get("request").cloned().unwrap_or_else(|| value.clone());
                let from = request.get("from").and_then(Value::as_str).unwrap_or("").trim().to_string();
                let to = request.get("to").and_then(Value::as_str).unwrap_or("").trim().to_string();
                if from.is_empty() || to.is_empty() {
                    return failure(id, "mergeLayerError", "a mergeLayer request needs `from` and `to` layer ids".to_string());
                }
                match open_cad_drawing::merge_layer(handle, &from, &to) {
                    Ok(result) => reply(id, "mergeLayerResult", result),
                    Err(error) => failure(id, "mergeLayerError", format!("{}: {}", error.code, error.message)),
                }
            }
            "drawing.convertUnits" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "convertUnitsError", "a convertUnits request needs a drawing handle".to_string());
                };
                let request = value.get("request").cloned().unwrap_or_else(|| value.clone());
                let unit = request.get("to").and_then(Value::as_str).unwrap_or("").trim().to_string();
                if unit.is_empty() {
                    return failure(id, "convertUnitsError", "a convertUnits request needs `to`: `mm` or `inch`".to_string());
                }
                match open_cad_drawing::convert_units(handle, &unit) {
                    Ok(result) => reply(id, "convertUnitsResult", result),
                    Err(error) => failure(id, "convertUnitsError", format!("{}: {}", error.code, error.message)),
                }
            }
            "drawing.moveEntity" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "moveEntityError", "a moveEntity request needs a drawing handle".to_string());
                };
                let request = value.get("request").cloned().unwrap_or_else(|| value.clone());
                match call_move_entity(handle, &request) {
                    Ok(result) => reply(id, "moveEntityResult", result),
                    Err(message) => failure(id, "moveEntityError", message),
                }
            }
            "drawing.updateEntity" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "updateEntityError", "an updateEntity request needs a drawing handle".to_string());
                };
                let request = value.get("request").cloned().unwrap_or_else(|| value.clone());
                match call_update_entity(handle, &request) {
                    Ok(result) => reply(id, "updateEntityResult", result),
                    Err(message) => failure(id, "updateEntityError", message),
                }
            }
            "drawing.transform" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "transformError", "a transform request needs a drawing handle".to_string());
                };
                let request = value.get("request").cloned().unwrap_or_else(|| value.clone());
                match call_transform(handle, &request) {
                    Ok(result) => reply(id, "transformResult", result),
                    Err(message) => failure(id, "transformError", message),
                }
            }
            "drawing.copy" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "copyError", "a copy request needs a drawing handle".to_string());
                };
                let request = value.get("request").cloned().unwrap_or_else(|| value.clone());
                match call_copy(handle, &request) {
                    Ok(result) => reply(id, "copyResult", result),
                    Err(message) => failure(id, "copyError", message),
                }
            }
            "drawing.array" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "arrayError", "an array request needs a drawing handle".to_string());
                };
                let request = value.get("request").cloned().unwrap_or_else(|| value.clone());
                match call_array(handle, &request) {
                    Ok(result) => reply(id, "arrayResult", result),
                    Err(message) => failure(id, "arrayError", message),
                }
            }
            "drawing.explode" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "explodeError", "an explode request needs a drawing handle".to_string());
                };
                let entity = value
                    .get("entity")
                    .or_else(|| value.get("request").and_then(|request| request.get("entity")))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                match open_cad_drawing::explode(handle, &entity) {
                    Ok(result) => reply(id, "explodeResult", result),
                    Err(error) => failure(id, "explodeError", format!("{}: {}", error.code, error.message)),
                }
            }
            "drawing.measure" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "measureError", "a measure request needs a drawing handle".to_string());
                };
                let sheet = value.get("sheet").and_then(Value::as_str).map(str::to_string);
                match open_cad_drawing::measure(handle, sheet) {
                    Ok(result) => reply(id, "measureResult", result),
                    Err(error) => failure(id, "measureError", format!("{}: {}", error.code, error.message)),
                }
            }
            "drawing.importDxf" => {
                let request = value.get("request").cloned().unwrap_or_else(|| value.clone());
                match call_import_dxf(&request) {
                    Ok(result) => reply(id, "importDxfResult", result),
                    Err(message) => failure(id, "importDxfError", message),
                }
            }
            _ => IpcOutcome::Ignored,
        }
    }
}

// One WebView host per process, owned by the thread that opened it.
thread_local! {
    static GUI: RefCell<Option<GuiState>> = const { RefCell::new(None) };
}

const GUI_CLOSED: &str = "gui_closed";
const GUI_HOST_EXITED: &str = "gui_host_exited";
const GUI_NOT_OPEN: &str = "gui_not_open";

struct GuiState {
    host: WebViewHost,
    backend: &'static str,
    opened_at: Instant,
    /// Set when the page asks the host to close, so a dead host is
    /// distinguishable from one that never processed the request.
    close_requested: Arc<AtomicBool>,
}

fn gui_status(
    state: &'static str,
    code: Option<&'static str>,
    backend: Option<&'static str>,
    uptime_ms: u64,
    message: Option<String>,
) -> Value {
    json!({
        "state": state,
        "code": code,
        "backend": backend,
        "platform": platform_name(),
        "uptimeMs": uptime_ms,
        "message": message,
    })
}

fn gui_open_result(ok: bool, status: Value, error: Option<&WebViewError>) -> Value {
    let (stage, hresult, attempted) = match error {
        Some(error) => {
            let attempted = match error {
                WebViewError::Host(host) => host.attempted.clone(),
                _ => Vec::new(),
            };
            (error.stage(), error.hresult(), attempted)
        }
        None => (None, None, Vec::new()),
    };
    json!({
        "ok": ok,
        "status": status,
        "stage": stage,
        "hresult": hresult,
        "attempted": attempted,
    })
}

/// Resolves the GUI content directory: explicit override, then the built tree.
fn resolve_gui_dir() -> Option<PathBuf> {
    if let Ok(value) = std::env::var("OPEN_CAD_GUI") {
        let path = PathBuf::from(value);
        if path.is_dir() {
            return Some(path);
        }
        if path.is_file() {
            return path.parent().map(Path::to_path_buf);
        }
    }

    let manifest_relative = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .join("projects")
        .join("packages")
        .join("open-cad")
        .join("dist")
        .join("gui");
    if manifest_relative.join("index.html").is_file() {
        return Some(manifest_relative);
    }

    if let Ok(cwd) = std::env::current_dir() {
        for candidate in [cwd.join("projects/packages/open-cad/dist/gui"), cwd.join("dist/gui"), cwd.join("gui")] {
            if candidate.join("index.html").is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Opens the OpenCAD studio window, or focuses the live one.
#[function(js_name = "openGui")]
fn open_gui(ctx: CallContext) -> Result<JsValue> {
    let mut title = "OpenCAD".to_string();
    let mut content_dir: Option<String> = None;
    let mut width = 1280i32;
    let mut height = 820i32;

    if let Some(arg) = ctx.args().first().copied()
        && !arg.is_undefined()?
    {
        if let Some(value) = optional_string_prop(arg, "title")? {
            title = value;
        }
        if let Some(value) = optional_string_prop(arg, "contentDir")? {
            content_dir = Some(value);
        }
        if let Ok(value) = arg.get_named("width")
            && let Ok(number) = value.as_i64()
        {
            width = number as i32;
        }
        if let Ok(value) = arg.get_named("height")
            && let Ok(number) = value.as_i64()
        {
            height = number as i32;
        }
    }

    let live = GUI.with(|slot| {
        let guard = slot.borrow();
        guard.as_ref().filter(|state| state.host.is_alive()).map(|state| (state.backend, state.opened_at.elapsed().as_millis() as u64))
    });
    if let Some((backend, uptime_ms)) = live {
        GUI.with(|slot| {
            if let Some(state) = slot.borrow_mut().as_mut() {
                let _ = state.host.focus();
            }
        });
        let status = gui_status("open", None, Some(backend), uptime_ms, None);
        return respond(&ctx, &gui_open_result(true, status, None));
    }
    GUI.with(|slot| {
        *slot.borrow_mut() = None;
    });

    let dir = match content_dir {
        Some(dir) => PathBuf::from(dir),
        None => match resolve_gui_dir() {
            Some(dir) => dir,
            None => {
                let status = gui_status(
                    "failed",
                    Some("gui_content_missing"),
                    None,
                    0,
                    Some("the WebView content directory is missing; build the studio UI first".to_string()),
                );
                return respond(&ctx, &gui_open_result(false, status, None));
            }
        },
    };

    let close_requested = Arc::new(AtomicBool::new(false));
    let mut options = OpenOptions::from_content_dir(dir.display().to_string(), title);
    options.width = width;
    options.height = height;
    options.accept_file_drop = true;
    // Windows draws its own window bar from the page, so the native title bar is
    // removed while resize borders and snap stay with the platform. Other
    // platforms keep the system decorations.
    options.decorations = !cfg!(target_os = "windows");

    match WebViewHost::open(options, Box::new(StudioIpcHandler { close_requested: Arc::clone(&close_requested) })) {
        Ok(host) => {
            let backend = host.backend_name();
            GUI.with(|slot| {
                *slot.borrow_mut() = Some(GuiState { host, backend, opened_at: Instant::now(), close_requested });
            });
            let status = gui_status("open", None, Some(backend), 0, None);
            respond(&ctx, &gui_open_result(true, status, None))
        }
        Err(error) => {
            let status = gui_status("failed", error.code(), None, 0, Some(error.to_string()));
            respond(&ctx, &gui_open_result(false, status, Some(&error)))
        }
    }
}

/// Pumps the host message loop once and reports the observed lifecycle state.
#[function(js_name = "pumpEvents")]
fn pump_events(ctx: CallContext) -> Result<JsValue> {
    let status = GUI.with(|slot| {
        let mut guard = slot.borrow_mut();
        let Some(state) = guard.as_mut()
        else {
            return gui_status("closed", Some(GUI_NOT_OPEN), None, 0, Some("no OpenCAD window is open".to_string()));
        };
        let backend = state.backend;
        let uptime_ms = state.opened_at.elapsed().as_millis() as u64;
        apply_window_commands(&mut state.host);
        if state.host.pump() {
            return gui_status("open", None, Some(backend), uptime_ms, None);
        }
        let requested = state.close_requested.swap(false, Ordering::SeqCst);
        *guard = None;
        if requested {
            gui_status("closed", Some(GUI_CLOSED), Some(backend), uptime_ms, None)
        }
        else {
            gui_status(
                "closed",
                Some(GUI_HOST_EXITED),
                Some(backend),
                uptime_ms,
                Some(format!("the {backend} webview host stopped without a page close request")),
            )
        }
    });
    respond(&ctx, &status)
}

/// Closes the studio window; the result distinguishes "already closed".
#[function(js_name = "closeGui")]
fn close_gui(ctx: CallContext) -> Result<JsValue> {
    let result = GUI.with(|slot| {
        let mut guard = slot.borrow_mut();
        let Some(state) = guard.as_mut()
        else {
            return json!({
                "closed": false,
                "status": gui_status("not_open", Some(GUI_NOT_OPEN), None, 0, Some("no OpenCAD window is open".to_string())),
            });
        };
        let backend = state.backend;
        let uptime_ms = state.opened_at.elapsed().as_millis() as u64;
        match state.host.close() {
            Ok(()) => {
                *guard = None;
                json!({ "closed": true, "status": gui_status("closed", None, Some(backend), uptime_ms, None) })
            }
            Err(error) => json!({
                "closed": false,
                "status": gui_status("open", Some("gui_close_failed"), Some(backend), uptime_ms, Some(error.to_string())),
            }),
        }
    });
    respond(&ctx, &result)
}

/// Reports whether the studio window is alive.
#[function(js_name = "isGuiOpen")]
fn is_gui_open(ctx: CallContext) -> Result<JsValue> {
    let open = GUI.with(|slot| slot.borrow().as_ref().is_some_and(|state| state.host.is_alive()));
    ctx.env().boolean(open)
}

register_module![
    version,
    capabilities,
    drawing_create,
    drawing_open,
    drawing_save,
    drawing_inspect,
    drawing_export,
    drawing_close,
    drawing_add_line,
    drawing_add_circle,
    drawing_add_arc,
    drawing_add_ellipse,
    drawing_add_polyline,
    drawing_add_text,
    drawing_add_dimension,
    drawing_delete_entity,
    drawing_list_entities,
    drawing_undo,
    drawing_redo,
    drawing_add_layer,
    drawing_update_layer,
    drawing_merge_layer,
    drawing_convert_units,
    drawing_move_entity,
    drawing_update_entity,
    drawing_transform,
    drawing_copy,
    drawing_array,
    drawing_explode,
    drawing_measure,
    drawing_import_dxf,
    open_gui,
    pump_events,
    close_gui,
    is_gui_open
];

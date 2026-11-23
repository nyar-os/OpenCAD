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
    AlignMode, AlignRequest, ArrayMode, ArrayRequest, BreakRequest, CopyRequest, CreateRequest, DistributeAxis, DistributeRequest,
    DrawOrderAction, ExtendSide, HatchPatternRequest, JoinRequest, LayoutRequest, LayoutUpdate, MirrorAxis, OffsetSide, OrdinateAxis,
    OverkillRequest, ServiceError, SmoothPolylineRequest, TitleBlockRequest, TransformOp, TransformRequest, TransformTarget, UcsDirection,
    UcsRequest,
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

/// Adds a point entity to one layer of an open drawing.
#[function]
fn drawing_add_point(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_add_point(handle, &request).map_err(Error::new)?;
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

/// Adds a cubic spline to one layer of an open drawing.
#[function]
fn drawing_add_spline(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_add_spline(handle, &request).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Adds an axis-aligned rectangle as one closed polyline to an open drawing.
#[function]
fn drawing_add_rectangle(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_add_rectangle(handle, &request).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Adds a regular polygon as one closed polyline to an open drawing.
#[function]
fn drawing_add_polygon(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_add_polygon(handle, &request).map_err(Error::new)?;
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

/// Adds a radial or diameter dimension to one layer of an open drawing.
#[function]
fn drawing_add_radial_dimension(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_add_radial_dimension(handle, &request).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Adds an ordinate dimension to one layer of an open drawing.
#[function]
fn drawing_add_ordinate_dimension(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_add_ordinate_dimension(handle, &request).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Adds an angular dimension to an open drawing.
#[function]
fn drawing_add_angular_dimension(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_add_angular_dimension(handle, &request).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Adds a hatch bounded by one closed polygon to an open drawing.
#[function]
fn drawing_add_hatch(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_add_hatch(handle, &request).map_err(Error::new)?;
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

/// Moves one entity within the paint order.
#[function]
fn drawing_reorder_entity(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let entity = text(&ctx, 1)?;
    let raw = text(&ctx, 2)?;
    let action = DrawOrderAction::parse(&raw).ok_or_else(|| {
        Error::new(format!("session/argument-invalid: `{raw}` is not a draw-order action; use `to-front`, `to-back`, `forward` or `backward`"))
    })?;
    let result = open_cad_drawing::reorder_entity(handle, &entity, action).map_err(fail)?;
    respond(&ctx, &result)
}

/// Lists the paper-space layouts of an open drawing.
#[function]
fn drawing_list_layouts(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let result = open_cad_drawing::list_layouts(handle).map_err(fail)?;
    respond(&ctx, &result)
}

/// Creates one paper-space layout.
#[function]
fn drawing_create_layout(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = open_cad_drawing::create_layout(
        handle,
        LayoutRequest {
            name: request_string(&request, "name").map_err(Error::new)?,
            width: optional_number(&request, "width"),
            height: optional_number(&request, "height"),
            scale: optional_number(&request, "scale"),
        },
    )
    .map_err(fail)?;
    respond(&ctx, &result)
}

/// Updates one paper-space layout.
#[function]
fn drawing_update_layout(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let layout_id = text(&ctx, 1)?;
    let request = parse_request(&text(&ctx, 2)?)?;
    let result = open_cad_drawing::update_layout(
        handle,
        &layout_id,
        LayoutUpdate {
            name: request.get("name").and_then(Value::as_str).map(str::to_string),
            width: optional_number(&request, "width"),
            height: optional_number(&request, "height"),
            scale: optional_number(&request, "scale"),
        },
    )
    .map_err(fail)?;
    respond(&ctx, &result)
}

/// Removes one paper-space layout.
#[function]
fn drawing_delete_layout(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let layout_id = text(&ctx, 1)?;
    let result = open_cad_drawing::delete_layout(handle, &layout_id).map_err(fail)?;
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

/// Reads the title and descriptive properties of an open drawing.
#[function]
fn drawing_properties(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let result = open_cad_drawing::properties(handle).map_err(fail)?;
    respond(&ctx, &result)
}

/// Updates the title and descriptive properties of an open drawing.
#[function]
fn drawing_update_properties(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let field = |key: &str| request[key].as_str().map(str::to_string);
    let result = open_cad_drawing::update_properties(
        handle,
        open_cad_drawing::PropertiesUpdate {
            title: field("title"),
            author: field("author"),
            description: field("description"),
            company: field("company"),
            project: field("project"),
            keywords: field("keywords"),
        },
    )
    .map_err(fail)?;
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

/// Removes one empty layer from an open drawing.
#[function]
fn drawing_delete_layer(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let layer = text(&ctx, 1)?;
    let result = open_cad_drawing::delete_layer(handle, &layer).map_err(fail)?;
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

/// Lists the dimension styles of an open drawing.
#[function]
fn drawing_list_dim_styles(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let result = open_cad_drawing::list_dim_styles(handle).map_err(fail)?;
    respond(&ctx, &result)
}

/// Adds one dimension style to an open drawing.
#[function]
fn drawing_add_dim_style(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_add_dim_style(handle, &request).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Updates one dimension style of an open drawing.
#[function]
fn drawing_update_dim_style(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_update_dim_style(handle, &request).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Removes one dimension style from an open drawing.
#[function]
fn drawing_delete_dim_style(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let style = text(&ctx, 1)?;
    let result = open_cad_drawing::delete_dim_style(handle, &style).map_err(fail)?;
    respond(&ctx, &result)
}

/// Lists the text styles of an open drawing.
#[function]
fn drawing_list_text_styles(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let result = open_cad_drawing::list_text_styles(handle).map_err(fail)?;
    respond(&ctx, &result)
}

/// Adds one text style to an open drawing.
#[function]
fn drawing_add_text_style(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_add_text_style(handle, &request).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Updates one text style of an open drawing.
#[function]
fn drawing_update_text_style(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_update_text_style(handle, &request).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Removes one text style from an open drawing.
#[function]
fn drawing_delete_text_style(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let style = text(&ctx, 1)?;
    let result = open_cad_drawing::delete_text_style(handle, &style).map_err(fail)?;
    respond(&ctx, &result)
}

/// Lists the line types of an open drawing.
#[function]
fn drawing_list_linetypes(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let result = open_cad_drawing::list_linetypes(handle).map_err(fail)?;
    respond(&ctx, &result)
}

/// Adds one line type to an open drawing.
#[function]
fn drawing_add_linetype(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let pattern = request["pattern"].as_array().map(|entries| entries.iter().filter_map(Value::as_f64).collect::<Vec<_>>());
    let result = open_cad_drawing::add_linetype(
        handle,
        open_cad_drawing::LinetypeRequest {
            name: request_string(&request, "name").map_err(Error::new)?,
            description: request["description"].as_str().map(str::to_string),
            pattern,
        },
    )
    .map_err(fail)?;
    respond(&ctx, &result)
}

/// Updates one line type of an open drawing.
#[function]
fn drawing_update_linetype(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let linetype = request_string(&request, "linetypeId").map_err(Error::new)?;
    let pattern = request["pattern"].as_array().map(|entries| entries.iter().filter_map(Value::as_f64).collect::<Vec<_>>());
    let result = open_cad_drawing::update_linetype(
        handle,
        &linetype,
        open_cad_drawing::LinetypeUpdate {
            name: request["name"].as_str().map(str::to_string),
            description: request["description"].as_str().map(str::to_string),
            pattern,
        },
    )
    .map_err(fail)?;
    respond(&ctx, &result)
}

/// Removes one line type from an open drawing.
#[function]
fn drawing_delete_linetype(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let linetype = text(&ctx, 1)?;
    let result = open_cad_drawing::delete_linetype(handle, &linetype).map_err(fail)?;
    respond(&ctx, &result)
}

/// Lists the entity groups of an open drawing.
#[function]
fn drawing_list_groups(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let result = open_cad_drawing::list_groups(handle).map_err(fail)?;
    respond(&ctx, &result)
}

/// Creates one entity group from existing entities.
#[function]
fn drawing_create_group(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_create_group(handle, &request).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Removes one entity group.
#[function]
fn drawing_delete_group(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let group = text(&ctx, 1)?;
    let result = open_cad_drawing::delete_group(handle, &group).map_err(fail)?;
    respond(&ctx, &result)
}

/// Adds existing entities to one group.
#[function]
fn drawing_add_group_members(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_group_members(handle, &request, true).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Removes entities from one group.
#[function]
fn drawing_remove_group_members(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_group_members(handle, &request, false).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Selects entities with a rectangular window or crossing box.
#[function]
fn drawing_select(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_select(handle, &request).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Computes snap candidates near one point of an open drawing.
#[function]
fn drawing_snap(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let modes = request["modes"]
        .as_array()
        .map(|entries| entries.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>())
        .unwrap_or_default();
    let point = match request.get("point").filter(|value| !value.is_null()) {
        Some(value) => {
            let array = value
                .as_array()
                .filter(|array| array.len() == 2)
                .ok_or_else(|| Error::new("session/argument-invalid: `point` must be an array of two numbers"))?;
            let x = array[0].as_f64().ok_or_else(|| Error::new("session/argument-invalid: `point` must be an array of two numbers"))?;
            let y = array[1].as_f64().ok_or_else(|| Error::new("session/argument-invalid: `point` must be an array of two numbers"))?;
            Some([x, y])
        }
        None => None,
    };
    let ids = request["ids"].as_array().map(|entries| entries.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>());
    let result = open_cad_drawing::snap_points(
        handle,
        open_cad_drawing::SnapRequest {
            modes,
            point,
            tolerance: request["tolerance"].as_f64(),
            layer: request["layer"].as_str().map(str::to_string),
            ids,
        },
    )
    .map_err(fail)?;
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

/// Edits the vertices or closed flag of one polyline.
#[function]
fn drawing_update_polyline(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_update_polyline(handle, &request).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Updates the radius or sweep angles of one arc.
#[function]
fn drawing_update_arc(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_update_arc(handle, &request).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Moves one entity onto another layer.
#[function]
fn drawing_assign_entity_layer(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_assign_entity_layer(handle, &request).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Finds text entities whose content contains a pattern.
#[function]
fn drawing_find_text(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_find_text(handle, &request).map_err(Error::new)?;
    respond(&ctx, &result)
}

/// Replaces a pattern in every matching text entity.
#[function]
fn drawing_replace_text(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = call_replace_text(handle, &request).map_err(Error::new)?;
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

/// Offsets a circle, arc, polyline or line into a new entity.
#[function]
fn drawing_offset(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let entity =
        request.get("entityId").and_then(Value::as_str).ok_or_else(|| Error::new("session/argument-invalid: `entityId` is required"))?;
    let distance = request_number(&request, "distance").map_err(Error::new)?;
    let result = open_cad_drawing::offset(handle, entity, distance).map_err(fail)?;
    respond(&ctx, &result)
}

/// Offsets a circle, arc, polyline or line by an explicit side.
#[function]
fn drawing_offset_by_side(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let entity =
        request.get("entityId").and_then(Value::as_str).ok_or_else(|| Error::new("session/argument-invalid: `entityId` is required"))?;
    let distance = request_number(&request, "distance").map_err(Error::new)?;
    let side = request
        .get("side")
        .and_then(Value::as_str)
        .and_then(OffsetSide::parse)
        .ok_or_else(|| Error::new("offset/invalid-side: `side` must be left, right, outside or inside"))?;
    let result = open_cad_drawing::offset_by_side(handle, entity, distance, side).map_err(fail)?;
    respond(&ctx, &result)
}

/// Fillets two lines with a tangent arc and trims both lines.
#[function]
fn drawing_fillet(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let (first, second) = corner_pair(&request).map_err(Error::new)?;
    let radius = request_number(&request, "radius").map_err(Error::new)?;
    let result = open_cad_drawing::fillet(handle, &first, &second, radius).map_err(fail)?;
    respond(&ctx, &result)
}

/// Chamfers two lines with a bevel and trims both lines.
#[function]
fn drawing_chamfer(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let (first, second) = corner_pair(&request).map_err(Error::new)?;
    let distance_a = request_number(&request, "distanceA").map_err(Error::new)?;
    let distance_b = request_number(&request, "distanceB").map_err(Error::new)?;
    let result = open_cad_drawing::chamfer(handle, &first, &second, distance_a, distance_b).map_err(fail)?;
    respond(&ctx, &result)
}

fn corner_pair(request: &Value) -> std::result::Result<(String, String), String> {
    let first = request.get("first").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let second = request.get("second").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    if first.is_empty() || second.is_empty() {
        return Err("session/argument-invalid: `first` and `second` entity ids are required".to_string());
    }
    Ok((first, second))
}

/// Trims a line, arc or circle target at a line, circle or arc cutter, keeping the longer side.
#[function]
fn drawing_trim(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let target = request.get("id").and_then(Value::as_str).unwrap_or_default().trim();
    let cutter = request.get("cutter").and_then(Value::as_str).unwrap_or_default().trim();
    if target.is_empty() || cutter.is_empty() {
        return Err(Error::new("session/argument-invalid: `id` and `cutter` entity ids are required"));
    }
    let result = open_cad_drawing::trim(handle, target, cutter).map_err(fail)?;
    respond(&ctx, &result)
}

/// Extends one line to its nearest line, circle or arc boundary.
#[function]
fn drawing_extend(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let target = request.get("id").and_then(Value::as_str).unwrap_or_default().trim();
    let boundary = request.get("boundary").or_else(|| request.get("to")).and_then(Value::as_str).unwrap_or_default().trim();
    if target.is_empty() || boundary.is_empty() {
        return Err(Error::new("session/argument-invalid: `id` and `boundary` entity ids are required"));
    }
    let side = match request.get("side").and_then(Value::as_str).map(str::trim) {
        None | Some("") => ExtendSide::End,
        Some(raw) => ExtendSide::parse(raw).ok_or_else(|| Error::new("session/argument-invalid: `side` must be end or start"))?,
    };
    let result = open_cad_drawing::extend(handle, target, boundary, side).map_err(fail)?;
    respond(&ctx, &result)
}

/// Splits one line or arc at an interior point.
#[function]
fn drawing_break(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let id = request.get("id").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    if id.is_empty() {
        return Err(Error::new("session/argument-invalid: `id` is required"));
    }
    let at = request_point(&request, "at").map_err(Error::new)?;
    let result = open_cad_drawing::break_entity(handle, BreakRequest { id, at }).map_err(fail)?;
    respond(&ctx, &result)
}

/// Merges two collinear lines that share an endpoint into one line.
#[function]
fn drawing_join(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let a = request.get("a").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let b = request.get("b").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    if a.is_empty() || b.is_empty() {
        return Err(Error::new("session/argument-invalid: `a` and `b` entity ids are required"));
    }
    let result = open_cad_drawing::join_entities(handle, JoinRequest { a, b }).map_err(fail)?;
    respond(&ctx, &result)
}

/// Aligns a selection of entities to one bounding-box edge or centreline.
#[function]
fn drawing_align(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let ids = request_ids(&request, "ids").map_err(Error::new)?;
    let mode = AlignMode::parse(request.get("mode").and_then(Value::as_str).unwrap_or_default())
        .ok_or_else(|| Error::new("session/argument-invalid: `mode` must be left, center, right, bottom, middle or top"))?;
    let result = open_cad_drawing::align(handle, AlignRequest { ids, mode }).map_err(fail)?;
    respond(&ctx, &result)
}

/// Spreads a selection of entities with equal gaps between bounding boxes.
#[function]
fn drawing_distribute(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let ids = request_ids(&request, "ids").map_err(Error::new)?;
    let axis = DistributeAxis::parse(request.get("axis").and_then(Value::as_str).unwrap_or_default())
        .ok_or_else(|| Error::new("session/argument-invalid: `axis` must be x or y"))?;
    let result = open_cad_drawing::distribute(handle, DistributeRequest { ids, axis }).map_err(fail)?;
    respond(&ctx, &result)
}

/// Removes exact duplicate entities and optionally merges collinear overlapping lines.
#[function]
fn drawing_overkill(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let request: OverkillRequest = serde_json::from_value(request).map_err(|error| Error::new(format!("session/argument-invalid: {error}")))?;
    let result = open_cad_drawing::overkill(handle, request).map_err(fail)?;
    respond(&ctx, &result)
}

/// Fills one hatch with pattern lines clipped to its boundary.
#[function]
fn drawing_hatch_pattern(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = open_cad_drawing::hatch_pattern(
        handle,
        HatchPatternRequest {
            id: request_string(&request, "id").map_err(Error::new)?,
            angle: optional_number(&request, "angle"),
            spacing: optional_number(&request, "spacing"),
            cross: request.get("cross").and_then(Value::as_bool),
        },
    )
    .map_err(fail)?;
    respond(&ctx, &result)
}

/// Adds a sheet frame and a title block filled from the drawing properties.
#[function]
fn drawing_title_block(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = open_cad_drawing::add_title_block(
        handle,
        TitleBlockRequest {
            layer: optional_string(&request, "layer"),
            width: optional_number(&request, "width"),
            height: optional_number(&request, "height"),
            margin: optional_number(&request, "margin"),
            text_height: optional_number(&request, "textHeight"),
            title: optional_string(&request, "title"),
            number: optional_string(&request, "number"),
            revision: optional_string(&request, "revision"),
            scale: optional_string(&request, "scale"),
            date: optional_string(&request, "date"),
            author: optional_string(&request, "author"),
            sheet: optional_string(&request, "sheet"),
        },
    )
    .map_err(fail)?;
    respond(&ctx, &result)
}

/// Rounds the corners of one polyline with Chaikin corner cutting.
#[function]
fn drawing_smooth_polyline(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let result = open_cad_drawing::smooth_polyline(
        handle,
        SmoothPolylineRequest {
            id: request_string(&request, "id").map_err(Error::new)?,
            iterations: request.get("iterations").and_then(Value::as_u64).map(|value| value.min(u32::MAX as u64) as u32),
            ratio: optional_number(&request, "ratio"),
        },
    )
    .map_err(fail)?;
    respond(&ctx, &result)
}

/// Compares two project files at entity level.
#[function]
fn drawing_diff(ctx: CallContext) -> Result<JsValue> {
    let left = text(&ctx, 0)?;
    let right = text(&ctx, 1)?;
    let result = open_cad_drawing::diff(Path::new(&left), Path::new(&right)).map_err(fail)?;
    respond(&ctx, &result)
}

/// Summarises entity count, path length and enclosed area per layer.
#[function]
fn drawing_stats(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let result = open_cad_drawing::stats(handle).map_err(fail)?;
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

/// Measures the path length and enclosed area of one or more entities.
#[function]
fn drawing_measure_entities(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let raw = text(&ctx, 1)?;
    let ids: Vec<String> = serde_json::from_str(&raw).map_err(|error| Error::new(format!("session/argument-invalid: {error}")))?;
    let result = open_cad_drawing::measure_entities(handle, &ids).map_err(fail)?;
    respond(&ctx, &result)
}

/// Measures the distance between two points or the angle at three points.
#[function]
fn drawing_measure_points(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let raw = text(&ctx, 1)?;
    let points: Vec<[f64; 2]> = serde_json::from_str(&raw).map_err(|error| Error::new(format!("session/argument-invalid: {error}")))?;
    let result = open_cad_drawing::measure_points(handle, &points).map_err(fail)?;
    respond(&ctx, &result)
}

/// Converts points and entities between world and user coordinates.
#[function]
fn drawing_ucs(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let request = parse_request(&text(&ctx, 1)?)?;
    let origin = request_point(&request, "origin").map_err(Error::new)?;
    let rotation = request.get("rotation").and_then(Value::as_f64).unwrap_or(0.0);
    let direction = match request.get("direction").and_then(Value::as_str) {
        Some("to-ucs") => UcsDirection::ToUcs,
        Some("to-world") | None => UcsDirection::ToWorld,
        Some(other) => return Err(Error::new(format!("session/argument-invalid: `{other}` is not a UCS direction"))),
    };
    let points = request.get("points").map(points_from_value).transpose().map_err(Error::new)?.unwrap_or_default();
    let ids = request
        .get("ids")
        .map(|value| {
            serde_json::from_value::<Vec<String>>(value.clone()).map_err(|error| Error::new(format!("session/argument-invalid: {error}")))
        })
        .transpose()?
        .unwrap_or_default();
    let result = open_cad_drawing::ucs(handle, UcsRequest { origin, rotation, direction, points, ids }).map_err(fail)?;
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

fn optional_string(request: &Value, key: &str) -> Option<String> {
    request.get(key).and_then(Value::as_str).map(str::to_string)
}

fn points_from_value(value: &Value) -> std::result::Result<Vec<[f64; 2]>, String> {
    let points = value.as_array().ok_or_else(|| "session/argument-invalid: `points` must be an array of [x, y]".to_string())?;
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

fn request_points(request: &Value) -> std::result::Result<Vec<[f64; 2]>, String> {
    let points = request.get("points").ok_or_else(|| "session/argument-invalid: `points` must be an array of [x, y]".to_string())?;
    points_from_value(points)
}

fn call_add_line(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let layer = request_layer(request)?;
    let from = request_point(request, "from")?;
    let to = request_point(request, "to")?;
    open_cad_drawing::add_line(handle, &layer, from, to).map_err(service_failure)
}

fn call_add_point(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let layer = request_layer(request)?;
    let at = request_point(request, "at")?;
    open_cad_drawing::add_point(handle, &layer, at).map_err(service_failure)
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

fn call_add_spline(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let layer = request_layer(request)?;
    let points = request_points(request)?;
    let closed = request.get("closed").and_then(Value::as_bool).unwrap_or(false);
    open_cad_drawing::add_spline(handle, &layer, points, closed).map_err(service_failure)
}

fn call_add_rectangle(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let layer = request_layer(request)?;
    let from = request_point(request, "from")?;
    let to = request_point(request, "to")?;
    open_cad_drawing::add_rectangle(handle, &layer, from, to).map_err(service_failure)
}

fn call_add_polygon(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let layer = request_layer(request)?;
    let center = request_point(request, "center")?;
    let radius = request_number(request, "radius")?;
    let sides = request_u32(request, "sides")?;
    let rotation = request.get("rotation").and_then(Value::as_f64).unwrap_or(0.0);
    open_cad_drawing::add_polygon(handle, &layer, center, radius, sides, rotation).map_err(service_failure)
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

fn call_add_radial_dimension(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let layer = request_layer(request)?;
    let center = request_point(request, "center")?;
    let radius = request_number(request, "radius")?;
    let angle = request_number(request, "angle")?;
    let offset = request.get("offset").and_then(Value::as_f64).unwrap_or(0.0);
    let diameter = request.get("diameter").and_then(Value::as_bool).unwrap_or(false);
    let text = request.get("text").and_then(Value::as_str).map(str::to_string);
    open_cad_drawing::add_radial_dimension(handle, &layer, center, radius, angle, offset, diameter, text).map_err(service_failure)
}

fn call_add_ordinate_dimension(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let layer = request_layer(request)?;
    let origin = request_point(request, "origin")?;
    let feature = request_point(request, "feature")?;
    let axis = match request.get("axis").and_then(Value::as_str) {
        Some("x") => OrdinateAxis::X,
        Some("y") => OrdinateAxis::Y,
        _ => return Err("session/argument-invalid: `axis` must be `x` or `y`".to_string()),
    };
    let offset = request.get("offset").and_then(Value::as_f64).unwrap_or(0.0);
    let text = request.get("text").and_then(Value::as_str).map(str::to_string);
    open_cad_drawing::add_ordinate_dimension(handle, &layer, origin, feature, axis, offset, text).map_err(service_failure)
}

fn call_add_angular_dimension(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let layer = request_layer(request)?;
    let center = request_point(request, "center")?;
    let first = request_point(request, "first")?;
    let second = request_point(request, "second")?;
    let offset = request.get("offset").and_then(Value::as_f64).unwrap_or(10.0);
    let text = request.get("text").and_then(Value::as_str).map(str::to_string);
    open_cad_drawing::add_angular_dimension(handle, &layer, center, first, second, offset, text).map_err(service_failure)
}

fn call_add_hatch(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let layer = request_layer(request)?;
    let points = request_points(request)?;
    let pattern = request.get("pattern").and_then(Value::as_str).map(str::to_string);
    open_cad_drawing::add_hatch(handle, &layer, points, pattern).map_err(service_failure)
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
        frozen: request.get("frozen").and_then(Value::as_bool),
        plottable: request.get("plottable").and_then(Value::as_bool),
        linetype: request.get("linetype").and_then(Value::as_str).map(str::to_string),
        lineweight: request.get("lineweight").and_then(Value::as_f64),
    };
    open_cad_drawing::update_layer(handle, &layer, update).map_err(service_failure)
}

fn optional_number(request: &Value, key: &str) -> Option<f64> {
    request.get(key).and_then(Value::as_f64)
}

fn optional_precision(request: &Value) -> std::result::Result<Option<u32>, String> {
    match request.get("precision") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let raw = value.as_u64().ok_or_else(|| "session/argument-invalid: `precision` must be an integer".to_string())?;
            u32::try_from(raw).map(Some).map_err(|_| "session/argument-invalid: `precision` is too large".to_string())
        }
    }
}

fn call_add_dim_style(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let style = open_cad_drawing::DimensionStyleRequest {
        name: request_string(request, "name")?,
        scale: optional_number(request, "scale"),
        arrow_size: optional_number(request, "arrowSize"),
        text_height: optional_number(request, "textHeight"),
        extension: optional_number(request, "extension"),
        origin_offset: optional_number(request, "originOffset"),
        precision: optional_precision(request)?,
    };
    open_cad_drawing::add_dim_style(handle, style).map_err(service_failure)
}

fn call_update_dim_style(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let style = request_string(request, "styleId")?;
    let update = open_cad_drawing::DimensionStyleUpdate {
        name: request.get("name").and_then(Value::as_str).map(str::to_string),
        scale: optional_number(request, "scale"),
        arrow_size: optional_number(request, "arrowSize"),
        text_height: optional_number(request, "textHeight"),
        extension: optional_number(request, "extension"),
        origin_offset: optional_number(request, "originOffset"),
        precision: optional_precision(request)?,
    };
    open_cad_drawing::update_dim_style(handle, &style, update).map_err(service_failure)
}

fn call_add_text_style(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let style = open_cad_drawing::TextStyleRequest {
        name: request_string(request, "name")?,
        font: request.get("font").and_then(Value::as_str).map(str::to_string),
        height: optional_number(request, "height"),
        width_factor: optional_number(request, "widthFactor"),
        oblique: optional_number(request, "oblique"),
    };
    open_cad_drawing::add_text_style(handle, style).map_err(service_failure)
}

fn call_update_text_style(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let style = request_string(request, "styleId")?;
    let update = open_cad_drawing::TextStyleUpdate {
        name: request.get("name").and_then(Value::as_str).map(str::to_string),
        font: request.get("font").and_then(Value::as_str).map(str::to_string),
        height: optional_number(request, "height"),
        width_factor: optional_number(request, "widthFactor"),
        oblique: optional_number(request, "oblique"),
    };
    open_cad_drawing::update_text_style(handle, &style, update).map_err(service_failure)
}

fn call_select(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let mode = match request.get("mode").and_then(Value::as_str).unwrap_or("window") {
        "window" => open_cad_drawing::SelectionMode::Window,
        "crossing" => open_cad_drawing::SelectionMode::Crossing,
        other => return Err(format!("session/argument-invalid: `{other}` is not a selection mode; use `window` or `crossing`")),
    };
    let window = request
        .get("window")
        .and_then(Value::as_array)
        .filter(|points| points.len() == 2)
        .ok_or_else(|| "session/argument-invalid: `window` must be two [x, y] corners".to_string())?;
    let corner = |value: &Value| -> std::result::Result<[f64; 2], String> {
        let pair = value
            .as_array()
            .filter(|pair| pair.len() == 2)
            .ok_or_else(|| "session/argument-invalid: every window corner needs two numbers".to_string())?;
        let x = pair[0].as_f64().ok_or_else(|| "session/argument-invalid: every window corner needs two numbers".to_string())?;
        let y = pair[1].as_f64().ok_or_else(|| "session/argument-invalid: every window corner needs two numbers".to_string())?;
        Ok([x, y])
    };
    let layer = request.get("layer").and_then(Value::as_str);
    open_cad_drawing::select(handle, mode, [corner(&window[0])?, corner(&window[1])?], layer).map_err(service_failure)
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

fn request_index(request: &Value, key: &str) -> std::result::Result<Option<usize>, String> {
    request
        .get(key)
        .map(|value| {
            let raw = value.as_u64().ok_or_else(|| format!("session/argument-invalid: `{key}` must be a non-negative integer"))?;
            usize::try_from(raw).map_err(|_| format!("session/argument-invalid: `{key}` is too large"))
        })
        .transpose()
}

fn call_update_polyline(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let entity = request_string(request, "entityId")?;
    let points = if request.get("points").is_some() { Some(request_points(request)?) } else { None };
    let closed = request.get("closed").and_then(Value::as_bool);
    let insert_at = request_index(request, "insertAt")?;
    let insert_point = if request.get("insertPoint").is_some() { Some(request_point(request, "insertPoint")?) } else { None };
    let remove_at = request_index(request, "removeAt")?;
    let move_at = request_index(request, "moveAt")?;
    let move_to = if request.get("moveTo").is_some() { Some(request_point(request, "moveTo")?) } else { None };
    open_cad_drawing::update_polyline(
        handle,
        &entity,
        open_cad_drawing::PolylineUpdate { points, closed, insert_at, insert_point, remove_at, move_at, move_to },
    )
    .map_err(service_failure)
}

fn call_update_arc(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let entity = request_string(request, "entityId")?;
    let radius = request.get("radius").and_then(Value::as_f64);
    let start_angle = request.get("startAngle").and_then(Value::as_f64);
    let end_angle = request.get("endAngle").and_then(Value::as_f64);
    open_cad_drawing::update_arc(handle, &entity, radius, start_angle, end_angle).map_err(service_failure)
}

fn call_assign_entity_layer(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let entity = request_string(request, "entityId")?;
    let layer = request_string(request, "layer")?;
    open_cad_drawing::assign_entity_layer(handle, &entity, &layer).map_err(service_failure)
}

fn text_search_options(request: &Value) -> open_cad_drawing::TextSearchOptions {
    open_cad_drawing::TextSearchOptions {
        match_case: request.get("matchCase").and_then(Value::as_bool).unwrap_or(false),
        layer: request.get("layer").and_then(Value::as_str).map(str::to_string),
    }
}

fn call_find_text(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let pattern = request_string(request, "pattern")?;
    open_cad_drawing::find_text(handle, &pattern, text_search_options(request)).map_err(service_failure)
}

fn call_replace_text(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let pattern = request_string(request, "pattern")?;
    let replacement = request.get("replacement").and_then(Value::as_str).unwrap_or("");
    open_cad_drawing::replace_text(handle, &pattern, replacement, text_search_options(request)).map_err(service_failure)
}

fn request_ids(request: &Value, key: &str) -> std::result::Result<Vec<String>, String> {
    let ids = request.get(key).and_then(Value::as_array).ok_or_else(|| format!("session/argument-invalid: `{key}` must be an array"))?;
    ids.iter()
        .map(|id| id.as_str().map(str::to_string).ok_or_else(|| format!("session/argument-invalid: every entry of `{key}` must be a string")))
        .collect()
}

fn call_create_group(handle: u64, request: &Value) -> std::result::Result<Value, String> {
    let name = request_string(request, "name")?;
    let entities = request_ids(request, "entities")?;
    open_cad_drawing::create_group(handle, open_cad_drawing::GroupCreateRequest { name, entities }).map_err(service_failure)
}

fn call_group_members(handle: u64, request: &Value, add: bool) -> std::result::Result<Value, String> {
    let group = request_string(request, "groupId")?;
    let entities = request_ids(request, "entities")?;
    let result = if add {
        open_cad_drawing::add_group_members(handle, &group, &entities)
    }
    else {
        open_cad_drawing::remove_group_members(handle, &group, &entities)
    };
    result.map_err(service_failure)
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
            "drawing.addSpline" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "addSplineError", "an addSpline request needs a drawing handle".to_string());
                };
                match call_add_spline(handle, &value) {
                    Ok(result) => reply(id, "addSplineResult", result),
                    Err(message) => failure(id, "addSplineError", message),
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
            "drawing.addRadialDimension" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "addRadialDimensionError", "an addRadialDimension request needs a drawing handle".to_string());
                };
                let request = value.get("request").cloned().unwrap_or_else(|| value.clone());
                match call_add_radial_dimension(handle, &request) {
                    Ok(result) => reply(id, "addRadialDimensionResult", result),
                    Err(message) => failure(id, "addRadialDimensionError", message),
                }
            }
            "drawing.addOrdinateDimension" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "addOrdinateDimensionError", "an addOrdinateDimension request needs a drawing handle".to_string());
                };
                let request = value.get("request").cloned().unwrap_or_else(|| value.clone());
                match call_add_ordinate_dimension(handle, &request) {
                    Ok(result) => reply(id, "addOrdinateDimensionResult", result),
                    Err(message) => failure(id, "addOrdinateDimensionError", message),
                }
            }
            "drawing.addAngularDimension" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "addAngularDimensionError", "an addAngularDimension request needs a drawing handle".to_string());
                };
                let request = value.get("request").cloned().unwrap_or_else(|| value.clone());
                match call_add_angular_dimension(handle, &request) {
                    Ok(result) => reply(id, "addAngularDimensionResult", result),
                    Err(message) => failure(id, "addAngularDimensionError", message),
                }
            }
            "drawing.addHatch" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "addHatchError", "an addHatch request needs a drawing handle".to_string());
                };
                let request = value.get("request").cloned().unwrap_or_else(|| value.clone());
                match call_add_hatch(handle, &request) {
                    Ok(result) => reply(id, "addHatchResult", result),
                    Err(message) => failure(id, "addHatchError", message),
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
            "drawing.offset" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "offsetError", "an offset request needs a drawing handle".to_string());
                };
                let entity = value
                    .get("entityId")
                    .or_else(|| value.get("entity"))
                    .or_else(|| value.get("request").and_then(|request| request.get("entityId")))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let Some(distance) = value.get("distance").and_then(Value::as_f64)
                else {
                    return failure(id, "offsetError", "an offset request needs a numeric `distance`".to_string());
                };
                match open_cad_drawing::offset(handle, entity, distance) {
                    Ok(result) => reply(id, "offsetResult", result),
                    Err(error) => failure(id, "offsetError", service_failure(error)),
                }
            }
            "drawing.offsetBySide" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "offsetError", "an offset request needs a drawing handle".to_string());
                };
                let request = value.get("request").cloned().unwrap_or_else(|| value.clone());
                let entity = value
                    .get("entityId")
                    .or_else(|| value.get("entity"))
                    .or_else(|| request.get("entityId"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let Some(distance) = request.get("distance").and_then(Value::as_f64)
                else {
                    return failure(id, "offsetError", "an offset request needs a numeric `distance`".to_string());
                };
                let Some(side) = request.get("side").and_then(Value::as_str).and_then(OffsetSide::parse)
                else {
                    return failure(id, "offsetError", "an offset request needs a `side` of left, right, outside or inside".to_string());
                };
                match open_cad_drawing::offset_by_side(handle, entity, distance, side) {
                    Ok(result) => reply(id, "offsetResult", result),
                    Err(error) => failure(id, "offsetError", service_failure(error)),
                }
            }
            "drawing.fillet" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "filletError", "a fillet request needs a drawing handle".to_string());
                };
                let request = value.get("request").cloned().unwrap_or_else(|| value.clone());
                let Some((first, second)) = corner_pair(&request).ok()
                else {
                    return failure(id, "filletError", "a fillet request needs `first` and `second` entity ids".to_string());
                };
                let Some(radius) = request.get("radius").and_then(Value::as_f64)
                else {
                    return failure(id, "filletError", "a fillet request needs a numeric `radius`".to_string());
                };
                match open_cad_drawing::fillet(handle, &first, &second, radius) {
                    Ok(result) => reply(id, "filletResult", result),
                    Err(error) => failure(id, "filletError", service_failure(error)),
                }
            }
            "drawing.chamfer" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "chamferError", "a chamfer request needs a drawing handle".to_string());
                };
                let request = value.get("request").cloned().unwrap_or_else(|| value.clone());
                let Some((first, second)) = corner_pair(&request).ok()
                else {
                    return failure(id, "chamferError", "a chamfer request needs `first` and `second` entity ids".to_string());
                };
                let Some(distance_a) = request.get("distanceA").and_then(Value::as_f64)
                else {
                    return failure(id, "chamferError", "a chamfer request needs a numeric `distanceA`".to_string());
                };
                let Some(distance_b) = request.get("distanceB").and_then(Value::as_f64)
                else {
                    return failure(id, "chamferError", "a chamfer request needs a numeric `distanceB`".to_string());
                };
                match open_cad_drawing::chamfer(handle, &first, &second, distance_a, distance_b) {
                    Ok(result) => reply(id, "chamferResult", result),
                    Err(error) => failure(id, "chamferError", service_failure(error)),
                }
            }
            "drawing.stats" => {
                let Some(handle) = handle_of(&value)
                else {
                    return failure(id, "statsError", "a stats request needs a drawing handle".to_string());
                };
                match open_cad_drawing::stats(handle) {
                    Ok(result) => reply(id, "statsResult", result),
                    Err(error) => failure(id, "statsError", service_failure(error)),
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
    drawing_add_point,
    drawing_add_circle,
    drawing_add_arc,
    drawing_add_ellipse,
    drawing_add_polyline,
    drawing_add_spline,
    drawing_add_rectangle,
    drawing_add_polygon,
    drawing_add_text,
    drawing_add_dimension,
    drawing_add_radial_dimension,
    drawing_add_ordinate_dimension,
    drawing_add_angular_dimension,
    drawing_add_hatch,
    drawing_delete_entity,
    drawing_list_entities,
    drawing_reorder_entity,
    drawing_list_layouts,
    drawing_create_layout,
    drawing_update_layout,
    drawing_delete_layout,
    drawing_undo,
    drawing_redo,
    drawing_properties,
    drawing_update_properties,
    drawing_add_layer,
    drawing_update_layer,
    drawing_delete_layer,
    drawing_list_dim_styles,
    drawing_add_dim_style,
    drawing_update_dim_style,
    drawing_delete_dim_style,
    drawing_list_text_styles,
    drawing_add_text_style,
    drawing_update_text_style,
    drawing_delete_text_style,
    drawing_list_linetypes,
    drawing_add_linetype,
    drawing_update_linetype,
    drawing_delete_linetype,
    drawing_list_groups,
    drawing_create_group,
    drawing_delete_group,
    drawing_add_group_members,
    drawing_remove_group_members,
    drawing_select,
    drawing_snap,
    drawing_merge_layer,
    drawing_convert_units,
    drawing_move_entity,
    drawing_update_entity,
    drawing_update_polyline,
    drawing_update_arc,
    drawing_assign_entity_layer,
    drawing_find_text,
    drawing_replace_text,
    drawing_transform,
    drawing_copy,
    drawing_array,
    drawing_explode,
    drawing_offset,
    drawing_offset_by_side,
    drawing_trim,
    drawing_extend,
    drawing_break,
    drawing_join,
    drawing_align,
    drawing_distribute,
    drawing_overkill,
    drawing_hatch_pattern,
    drawing_title_block,
    drawing_smooth_polyline,
    drawing_diff,
    drawing_fillet,
    drawing_chamfer,
    drawing_measure,
    drawing_measure_entities,
    drawing_measure_points,
    drawing_ucs,
    drawing_stats,
    drawing_import_dxf,
    open_gui,
    pump_events,
    close_gui,
    is_gui_open
];

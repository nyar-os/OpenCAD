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

fn assert_point(actual: &Value, expected: [f64; 2]) {
    let x = actual[0].as_f64().expect("x");
    let y = actual[1].as_f64().expect("y");
    assert!((x - expected[0]).abs() < 1e-9 && (y - expected[1]).abs() < 1e-9, "{actual:?} is not {expected:?}");
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
        "measure-points",
        "stats",
        "dimension-style",
        "text-style",
        "select",
        "ucs",
        "hatch-pattern",
        "title-block",
        "polyline-smooth",
        "diff",
        "quantity-table",
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
    let dimension_codes =
        dimension_only["issues"].as_array().expect("issues").iter().map(|issue| issue["code"].as_str().unwrap_or_default()).collect::<Vec<_>>();
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
fn layer_colours_round_trip_through_dxf_aci() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_layer(handle, "Walls", Some("#a4262c")).expect("add layer");

    let (text, losses) = store::read(handle, |session| exchange::dxf(&session.drawing)).expect("export dxf");
    assert!(losses.is_empty(), "every layer colour maps to an ACI: {:?}", losses.iter().map(|loss| loss.code.clone()).collect::<Vec<_>>());
    assert!(text.contains("0\nLAYER\n2\nWalls\n70\n0\n62\n1\n"), "the custom colour is written as ACI 1: {text}");
    assert!(text.contains("0\nLAYER\n2\n0\n70\n0\n62\n7\n"), "the near-white default layer is ACI 7: {text}");

    let (imported, import_losses) = import::dxf(&text, None).expect("import dxf");
    assert_eq!(imported.layer("Walls").expect("imported layer").color, "#ff0000", "ACI 1 maps back to the palette hex");
    assert!(
        import_losses.iter().all(|loss| loss.code != "dxf/layer-color"),
        "an ACI colour is not a loss: {:?}",
        import_losses.iter().map(|loss| loss.code.clone()).collect::<Vec<_>>()
    );
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

    let renamed = update_layer(handle, "Walls", LayerUpdate { name: Some("Structure".to_string()), ..LayerUpdate::default() }).expect("rename");
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
    let codes = report["issues"].as_array().expect("issues").iter().map(|issue| issue["code"].as_str().unwrap_or_default()).collect::<Vec<_>>();
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

    update_layer(handle, "Walls", LayerUpdate { frozen: Some(false), plottable: Some(true), ..LayerUpdate::default() }).expect("clear flags");
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
    let renamed =
        update_linetype(handle, "DASHED", LinetypeUpdate { name: Some("DASH-1".to_string()), ..LinetypeUpdate::default() }).expect("rename");
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
fn svg_and_pdf_draw_layer_lineweight_and_dash_pattern() {
    let dir = TempDir::new("stroke-style");
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_linetype(
        handle,
        LinetypeRequest { name: "DASHED".to_string(), description: Some("Dashed".to_string()), pattern: Some(vec![5.0, 2.0]) },
    )
    .expect("linetype");
    update_layer(handle, DEFAULT_LAYER, LayerUpdate { linetype: Some("DASHED".to_string()), lineweight: Some(0.8), ..LayerUpdate::default() })
        .expect("layer");
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [100.0, 0.0]).expect("line");

    let svg_path = dir.path.join("plan.svg");
    export(handle, "svg", &svg_path).expect("export svg");
    let svg = fs::read_to_string(&svg_path).expect("read svg");
    assert!(svg.contains(r#"stroke-width="0.8""#), "the layer lineweight is the SVG stroke width: {svg}");
    assert!(svg.contains(r#"stroke-dasharray="5 2""#), "the line type becomes an SVG dash array: {svg}");

    let pdf_path = dir.path.join("plan.pdf");
    export(handle, "pdf", &pdf_path).expect("export pdf");
    let pdf = fs::read_to_string(&pdf_path).expect("read pdf");
    let widths = pdf.lines().filter_map(|line| line.strip_suffix(" w").and_then(|value| value.parse::<f64>().ok())).collect::<Vec<_>>();
    assert!(
        widths.iter().any(|width| (*width - 0.7).abs() > 1e-9),
        "the PDF stroke width follows the layer lineweight instead of the default 0.7: {widths:?}"
    );
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
    assert_eq!(modes.len(), SNAP_DEFAULT_MODES.len());

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

/// Every candidate point of one snap response.
fn snap_candidate_points(value: &Value) -> Vec<[f64; 2]> {
    value["points"]
        .as_array()
        .expect("points")
        .iter()
        .map(|entry| [entry["point"][0].as_f64().expect("x"), entry["point"][1].as_f64().expect("y")])
        .collect()
}

/// Whether one snap response reports `expected` within a numeric tolerance.
fn snap_has_point(value: &Value, expected: [f64; 2]) -> bool {
    snap_candidate_points(value).iter().any(|point| (point[0] - expected[0]).abs() < 1e-9 && (point[1] - expected[1]).abs() < 1e-9)
}

#[test]
fn snap_reports_centres_quadrants_nearest_points_and_clips_arcs() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    let circle = add_circle(handle, DEFAULT_LAYER, [0.0, 0.0], 10.0).expect("circle");
    let circle_id = circle["entity"]["id"].clone();
    let arc = add_arc(handle, DEFAULT_LAYER, [100.0, 0.0], 10.0, 0.0, 90.0).expect("arc");
    let arc_id = arc["entity"]["id"].clone();
    let line = add_line(handle, DEFAULT_LAYER, [0.0, 20.0], [20.0, 20.0]).expect("line");
    let line_id = line["entity"]["id"].clone();
    let chain = add_polyline(handle, DEFAULT_LAYER, vec![[0.0, -20.0], [10.0, -20.0], [10.0, -30.0]], false).expect("polyline");
    let chain_id = chain["entity"]["id"].clone();

    let centres = snap_points(handle, SnapRequest { modes: vec!["center".to_string()], ..SnapRequest::default() }).expect("center");
    assert!(snap_has_point(&centres, [0.0, 0.0]));
    assert!(snap_has_point(&centres, [100.0, 0.0]));

    let quadrants = snap_points(handle, SnapRequest { modes: vec!["quadrant".to_string()], ..SnapRequest::default() }).expect("quadrant");
    assert_eq!(quadrants["count"], json!(6), "four circle quadrants and the two arc quadrants inside its sweep");
    assert!(snap_has_point(&quadrants, [10.0, 0.0]));
    assert!(snap_has_point(&quadrants, [0.0, 10.0]));
    assert!(snap_has_point(&quadrants, [110.0, 0.0]));
    assert!(snap_has_point(&quadrants, [100.0, 10.0]));
    assert!(!snap_has_point(&quadrants, [100.0, -10.0]), "an arc must not report a quadrant outside its sweep");

    let line_nearest = snap_points(
        handle,
        SnapRequest {
            modes: vec!["nearest".to_string()],
            point: Some([5.0, 21.0]),
            ids: Some(vec![line_id.as_str().unwrap().to_string()]),
            ..SnapRequest::default()
        },
    )
    .expect("line nearest");
    assert_eq!(line_nearest["count"], json!(1));
    assert!(snap_has_point(&line_nearest, [5.0, 20.0]));

    let arc_nearest = snap_points(
        handle,
        SnapRequest {
            modes: vec!["nearest".to_string()],
            point: Some([95.0, 15.0]),
            ids: Some(vec![arc_id.as_str().unwrap().to_string()]),
            ..SnapRequest::default()
        },
    )
    .expect("arc nearest");
    assert!(snap_has_point(&arc_nearest, [100.0, 10.0]), "the nearest point clamps to the arc end angle");

    let chain_nearest = snap_points(
        handle,
        SnapRequest {
            modes: vec!["nearest".to_string()],
            point: Some([15.0, -25.0]),
            ids: Some(vec![chain_id.as_str().unwrap().to_string()]),
            ..SnapRequest::default()
        },
    )
    .expect("polyline nearest");
    assert!(snap_has_point(&chain_nearest, [10.0, -25.0]), "a polyline nearest point is found segment by segment");

    let circle_nearest = snap_points(
        handle,
        SnapRequest {
            modes: vec!["nearest".to_string()],
            point: Some([20.0, 0.0]),
            ids: Some(vec![circle_id.as_str().unwrap().to_string()]),
            ..SnapRequest::default()
        },
    )
    .expect("circle nearest");
    assert!(snap_has_point(&circle_nearest, [10.0, 0.0]), "a circle nearest point sits on the radius towards the probe");

    assert!(
        snap_points(handle, SnapRequest { modes: vec!["nearest".to_string()], ..SnapRequest::default() }).is_err(),
        "nearest needs a probe point"
    );
    close(handle).expect("close");
}

#[test]
fn snap_intersections_cover_line_circle_and_circle_circle() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    let line = add_line(handle, DEFAULT_LAYER, [-5.0, 0.0], [5.0, 0.0]).expect("line");
    let circle = add_circle(handle, DEFAULT_LAYER, [0.0, 0.0], 2.0).expect("circle");
    let second = add_circle(handle, DEFAULT_LAYER, [6.0, 0.0], 5.0).expect("second circle");
    let outer = add_circle(handle, DEFAULT_LAYER, [0.0, 0.0], 5.0).expect("outer circle");

    let ids = |values: &[&Value]| Some(values.iter().map(|value| value["entity"]["id"].as_str().unwrap().to_string()).collect::<Vec<_>>());
    let line_circle =
        snap_points(handle, SnapRequest { modes: vec!["intersection".to_string()], ids: ids(&[&line, &circle]), ..SnapRequest::default() })
            .expect("line circle");
    assert_eq!(line_circle["count"], json!(2));
    assert!(snap_has_point(&line_circle, [-2.0, 0.0]));
    assert!(snap_has_point(&line_circle, [2.0, 0.0]));

    let circles =
        snap_points(handle, SnapRequest { modes: vec!["intersection".to_string()], ids: ids(&[&outer, &second]), ..SnapRequest::default() })
            .expect("circle circle");
    assert_eq!(circles["count"], json!(2));
    assert!(snap_has_point(&circles, [3.0, 4.0]));
    assert!(snap_has_point(&circles, [3.0, -4.0]));

    let tangent = add_line(handle, DEFAULT_LAYER, [-5.0, 1.0], [5.0, 1.0]).expect("tangent line");
    let unit = add_circle(handle, DEFAULT_LAYER, [0.0, 0.0], 1.0).expect("unit circle");
    let tangency =
        snap_points(handle, SnapRequest { modes: vec!["intersection".to_string()], ids: ids(&[&tangent, &unit]), ..SnapRequest::default() })
            .expect("tangent");
    assert_eq!(tangency["count"], json!(0), "a tangent contact is not an intersection candidate");

    let parallel = add_line(handle, DEFAULT_LAYER, [-5.0, 10.0], [5.0, 10.0]).expect("parallel");
    let parallel_run = snap_points(
        handle,
        SnapRequest { modes: vec!["intersection".to_string()], ids: ids(&[&tangent, &parallel]), ..SnapRequest::default() },
    )
    .expect("parallel");
    assert_eq!(parallel_run["count"], json!(0));

    let degenerate = add_circle(handle, DEFAULT_LAYER, [0.0, 0.0], 0.0).expect("degenerate circle");
    let degenerate_run =
        snap_points(handle, SnapRequest { modes: vec!["intersection".to_string()], ids: ids(&[&line, &degenerate]), ..SnapRequest::default() })
            .expect("degenerate");
    assert_eq!(degenerate_run["count"], json!(0), "a zero-radius circle contributes nothing");
    close(handle).expect("close");
}

#[test]
fn snap_intersections_cover_line_to_arc_pairs() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    let arc = add_arc(handle, DEFAULT_LAYER, [0.0, 0.0], 10.0, 0.0, 180.0).expect("arc");
    let line = add_line(handle, DEFAULT_LAYER, [-20.0, 5.0], [20.0, 5.0]).expect("line");
    let ids = |values: &[&Value]| Some(values.iter().map(|value| value["entity"]["id"].as_str().unwrap().to_string()).collect::<Vec<_>>());

    let crossings =
        snap_points(handle, SnapRequest { modes: vec!["intersection".to_string()], ids: ids(&[&line, &arc]), ..SnapRequest::default() })
            .expect("line arc");
    let offset = (100.0_f64 - 25.0).sqrt();
    assert_eq!(crossings["count"], json!(2));
    assert!(snap_has_point(&crossings, [-offset, 5.0]));
    assert!(snap_has_point(&crossings, [offset, 5.0]));

    let below = add_line(handle, DEFAULT_LAYER, [-20.0, -5.0], [20.0, -5.0]).expect("lower line");
    let outside =
        snap_points(handle, SnapRequest { modes: vec!["intersection".to_string()], ids: ids(&[&below, &arc]), ..SnapRequest::default() })
            .expect("lower line arc");
    assert_eq!(outside["count"], json!(0), "an arc only reports crossings inside its counter-clockwise sweep");
    close(handle).expect("close");
}

#[test]
fn snap_intersections_keep_arc_circle_crossings_inside_the_sweep() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    let arc = add_arc(handle, DEFAULT_LAYER, [0.0, 0.0], 10.0, 0.0, 90.0).expect("arc");
    let circle = add_circle(handle, DEFAULT_LAYER, [10.0, 0.0], 10.0).expect("circle");
    let ids = |values: &[&Value]| Some(values.iter().map(|value| value["entity"]["id"].as_str().unwrap().to_string()).collect::<Vec<_>>());

    let crossings =
        snap_points(handle, SnapRequest { modes: vec!["intersection".to_string()], ids: ids(&[&arc, &circle]), ..SnapRequest::default() })
            .expect("arc circle");
    assert_eq!(crossings["count"], json!(1), "the lower crossing lies outside the quarter arc");
    let height = (100.0_f64 - 25.0).sqrt();
    assert!(snap_has_point(&crossings, [5.0, height]));
    assert!(!snap_has_point(&crossings, [5.0, -height]));
    close(handle).expect("close");
}

#[test]
fn snap_intersections_cover_arc_to_arc_pairs() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    let first = add_arc(handle, DEFAULT_LAYER, [0.0, 0.0], 10.0, 0.0, 180.0).expect("first arc");
    let second = add_arc(handle, DEFAULT_LAYER, [10.0, 0.0], 10.0, 0.0, 180.0).expect("second arc");
    let ids = |values: &[&Value]| Some(values.iter().map(|value| value["entity"]["id"].as_str().unwrap().to_string()).collect::<Vec<_>>());

    let crossings =
        snap_points(handle, SnapRequest { modes: vec!["intersection".to_string()], ids: ids(&[&first, &second]), ..SnapRequest::default() })
            .expect("arc arc");
    assert_eq!(crossings["count"], json!(1), "the lower crossing lies outside both upper arcs");
    let height = (100.0_f64 - 25.0).sqrt();
    assert!(snap_has_point(&crossings, [5.0, height]));
    close(handle).expect("close");
}

#[test]
fn snap_intersections_handle_arcs_that_cross_zero_degrees() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    let arc = add_arc(handle, DEFAULT_LAYER, [0.0, 0.0], 10.0, 270.0, 90.0).expect("wrapping arc");
    let vertical = add_line(handle, DEFAULT_LAYER, [5.0, -20.0], [5.0, 20.0]).expect("vertical line");
    let horizontal = add_line(handle, DEFAULT_LAYER, [-20.0, 5.0], [20.0, 5.0]).expect("horizontal line");
    let ids = |values: &[&Value]| Some(values.iter().map(|value| value["entity"]["id"].as_str().unwrap().to_string()).collect::<Vec<_>>());
    let offset = (100.0_f64 - 25.0).sqrt();

    let both =
        snap_points(handle, SnapRequest { modes: vec!["intersection".to_string()], ids: ids(&[&vertical, &arc]), ..SnapRequest::default() })
            .expect("vertical arc");
    assert_eq!(both["count"], json!(2));
    assert!(snap_has_point(&both, [5.0, offset]));
    assert!(snap_has_point(&both, [5.0, -offset]));

    let single =
        snap_points(handle, SnapRequest { modes: vec!["intersection".to_string()], ids: ids(&[&horizontal, &arc]), ..SnapRequest::default() })
            .expect("horizontal arc");
    assert_eq!(single["count"], json!(1), "the left crossing lies outside the sweep from 270 to 90 degrees");
    assert!(snap_has_point(&single, [offset, 5.0]));
    close(handle).expect("close");
}

#[test]
fn snap_intersections_reject_arc_tangencies() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    let arc = add_arc(handle, DEFAULT_LAYER, [0.0, 0.0], 10.0, 0.0, 180.0).expect("arc");
    let tangent_line = add_line(handle, DEFAULT_LAYER, [-5.0, 10.0], [5.0, 10.0]).expect("tangent line");
    let tangent_circle = add_circle(handle, DEFAULT_LAYER, [20.0, 0.0], 10.0).expect("tangent circle");
    let tangent_arc = add_arc(handle, DEFAULT_LAYER, [20.0, 0.0], 10.0, 180.0, 360.0).expect("tangent arc");
    let ids = |values: &[&Value]| Some(values.iter().map(|value| value["entity"]["id"].as_str().unwrap().to_string()).collect::<Vec<_>>());

    for (label, other) in [("line", &tangent_line), ("circle", &tangent_circle), ("arc", &tangent_arc)] {
        let result =
            snap_points(handle, SnapRequest { modes: vec!["intersection".to_string()], ids: ids(&[&arc, other]), ..SnapRequest::default() })
                .expect(label);
        assert_eq!(result["count"], json!(0), "a tangency with a {label} is not an intersection candidate");
    }
    close(handle).expect("close");
}

#[test]
fn snap_intersections_with_arcs_sort_deterministically() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    let line = add_line(handle, DEFAULT_LAYER, [-60.0, 0.0], [60.0, 0.0]).expect("line");
    let left = add_arc(handle, DEFAULT_LAYER, [-20.0, 0.0], 10.0, 0.0, 180.0).expect("left arc");
    let right = add_arc(handle, DEFAULT_LAYER, [20.0, 0.0], 10.0, 0.0, 180.0).expect("right arc");
    let request = SnapRequest {
        modes: vec!["intersection".to_string()],
        point: Some([0.0, 0.0]),
        ids: Some(vec![
            line["entity"]["id"].as_str().unwrap().to_string(),
            left["entity"]["id"].as_str().unwrap().to_string(),
            right["entity"]["id"].as_str().unwrap().to_string(),
        ]),
        ..SnapRequest::default()
    };

    let ordered = snap_points(handle, request.clone()).expect("ordered");
    assert_eq!(ordered["count"], json!(4));
    let distances =
        ordered["points"].as_array().expect("points").iter().map(|entry| entry["distance"].as_f64().expect("distance")).collect::<Vec<_>>();
    assert!(distances.windows(2).all(|pair| pair[0] <= pair[1]), "candidates sort by distance: {distances:?}");

    let again = snap_points(handle, request).expect("again");
    assert_eq!(ordered["points"], again["points"]);
    close(handle).expect("close");
}

#[test]
fn snap_ordering_tolerance_and_defaults_stay_deterministic() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    let left = add_point(handle, DEFAULT_LAYER, [-5.0, 0.0]).expect("left");
    let right = add_point(handle, DEFAULT_LAYER, [5.0, 0.0]).expect("right");
    let circle = add_circle(handle, DEFAULT_LAYER, [0.0, 30.0], 10.0).expect("circle");

    let ordered = snap_points(handle, SnapRequest { modes: vec!["endpoint".to_string()], point: Some([0.0, 0.0]), ..SnapRequest::default() })
        .expect("ordered");
    assert_eq!(ordered["count"], json!(2));
    assert_eq!(ordered["points"][0]["entities"][0], left["entity"]["id"], "equal distances sort by entity id");
    assert_eq!(ordered["points"][1]["entities"][0], right["entity"]["id"]);
    let again = snap_points(handle, SnapRequest { modes: vec!["endpoint".to_string()], point: Some([0.0, 0.0]), ..SnapRequest::default() })
        .expect("again");
    assert_eq!(ordered["points"], again["points"]);

    let filtered = snap_points(
        handle,
        SnapRequest { modes: vec!["endpoint".to_string()], point: Some([0.0, 0.0]), tolerance: Some(4.9), ..SnapRequest::default() },
    )
    .expect("filtered");
    assert_eq!(filtered["count"], json!(0), "a tolerance below the nearest candidate drops every point");
    let kept = snap_points(
        handle,
        SnapRequest { modes: vec!["endpoint".to_string()], point: Some([0.0, 0.0]), tolerance: Some(5.1), ..SnapRequest::default() },
    )
    .expect("kept");
    assert_eq!(kept["count"], json!(2));

    let default = snap_points(handle, SnapRequest::default()).expect("default");
    let modes = default["modes"].as_array().expect("modes").iter().map(|value| value.as_str().expect("mode").to_string()).collect::<Vec<_>>();
    assert_eq!(modes, SNAP_DEFAULT_MODES.iter().map(|mode| (*mode).to_string()).collect::<Vec<_>>());
    assert!(!modes.contains(&"quadrant".to_string()), "quadrant is opt-in");
    assert!(!modes.contains(&"nearest".to_string()), "nearest is opt-in");
    let circle_endpoints = snap_points(
        handle,
        SnapRequest {
            modes: vec!["endpoint".to_string()],
            ids: Some(vec![circle["entity"]["id"].as_str().unwrap().to_string()]),
            ..SnapRequest::default()
        },
    )
    .expect("circle endpoints");
    assert_eq!(circle_endpoints["count"], json!(0), "a circle still reports no endpoints");
    close(handle).expect("close");
}

#[test]
fn snap_perpendicular_projects_lines_and_closed_polyline_edges() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    let line = add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("line");
    let line_id = line["entity"]["id"].as_str().expect("line id").to_string();

    let foot = snap_points(
        handle,
        SnapRequest {
            modes: vec!["perpendicular".to_string()],
            point: Some([5.0, 5.0]),
            ids: Some(vec![line_id.clone()]),
            ..SnapRequest::default()
        },
    )
    .expect("foot");
    assert_eq!(foot["count"], json!(1));
    assert_eq!(foot["points"][0]["kind"], json!("perpendicular"));
    assert!(snap_has_point(&foot, [5.0, 0.0]));

    let outside = snap_points(
        handle,
        SnapRequest { modes: vec!["perpendicular".to_string()], point: Some([15.0, 5.0]), ids: Some(vec![line_id]), ..SnapRequest::default() },
    )
    .expect("outside");
    assert_eq!(outside["count"], json!(0), "a projection outside the segment gives no candidate");

    let chain = add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 20.0], [10.0, 20.0], [10.0, 30.0]], true).expect("polyline");
    let chain_id = chain["entity"]["id"].as_str().expect("polyline id").to_string();
    let closing = snap_points(
        handle,
        SnapRequest {
            modes: vec!["perpendicular".to_string()],
            point: Some([12.0, 22.0]),
            ids: Some(vec![chain_id]),
            ..SnapRequest::default()
        },
    )
    .expect("closing edge");
    assert!(snap_has_point(&closing, [7.0, 27.0]), "the closing edge of a closed polyline contributes a perpendicular foot");
    close(handle).expect("close");
}

#[test]
fn snap_perpendicular_covers_circles_and_filters_arcs() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    let circle = add_circle(handle, DEFAULT_LAYER, [0.0, 0.0], 10.0).expect("circle");
    let circle_id = circle["entity"]["id"].as_str().expect("circle id").to_string();

    let points = snap_points(
        handle,
        SnapRequest {
            modes: vec!["perpendicular".to_string()],
            point: Some([20.0, 0.0]),
            ids: Some(vec![circle_id.clone()]),
            ..SnapRequest::default()
        },
    )
    .expect("circle perpendicular");
    assert_eq!(points["count"], json!(2));
    assert!(snap_has_point(&points, [10.0, 0.0]));
    assert!(snap_has_point(&points, [-10.0, 0.0]));

    let at_center = snap_points(
        handle,
        SnapRequest { modes: vec!["perpendicular".to_string()], point: Some([0.0, 0.0]), ids: Some(vec![circle_id]), ..SnapRequest::default() },
    )
    .expect("centre probe");
    assert_eq!(at_center["count"], json!(0), "a probe at the centre gives no perpendicular line");

    let arc = add_arc(handle, DEFAULT_LAYER, [0.0, 50.0], 10.0, 0.0, 90.0).expect("arc");
    let arc_id = arc["entity"]["id"].as_str().expect("arc id").to_string();
    let arc_points = snap_points(
        handle,
        SnapRequest {
            modes: vec!["perpendicular".to_string()],
            point: Some([20.0, 50.0]),
            ids: Some(vec![arc_id.clone()]),
            ..SnapRequest::default()
        },
    )
    .expect("arc perpendicular");
    assert_eq!(arc_points["count"], json!(1), "only the point inside the counter-clockwise sweep is reported");
    assert_eq!(arc_points["points"][0]["kind"], json!("perpendicular"));
    assert!(snap_has_point(&arc_points, [10.0, 50.0]));
    assert!(!snap_has_point(&arc_points, [-10.0, 50.0]), "the opposite point lies outside the 0-90 sweep");

    let arc_center = snap_points(
        handle,
        SnapRequest { modes: vec!["perpendicular".to_string()], point: Some([0.0, 50.0]), ids: Some(vec![arc_id]), ..SnapRequest::default() },
    )
    .expect("arc centre probe");
    assert_eq!(arc_center["count"], json!(0));
    close(handle).expect("close");
}

#[test]
fn snap_perpendicular_ordering_tolerance_and_missing_probe() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    let below = add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("below");
    let above = add_line(handle, DEFAULT_LAYER, [0.0, 10.0], [10.0, 10.0]).expect("above");

    let ordered =
        snap_points(handle, SnapRequest { modes: vec!["perpendicular".to_string()], point: Some([5.0, 5.0]), ..SnapRequest::default() })
            .expect("ordered");
    assert_eq!(ordered["count"], json!(2));
    assert_eq!(ordered["points"][0]["point"], json!([5.0, 0.0]));
    assert_eq!(ordered["points"][1]["point"], json!([5.0, 10.0]));
    assert_eq!(ordered["points"][0]["entities"][0], below["entity"]["id"], "equal distances sort by entity id");
    assert_eq!(ordered["points"][1]["entities"][0], above["entity"]["id"]);
    let again =
        snap_points(handle, SnapRequest { modes: vec!["perpendicular".to_string()], point: Some([5.0, 5.0]), ..SnapRequest::default() })
            .expect("again");
    assert_eq!(ordered["points"], again["points"]);

    let filtered = snap_points(
        handle,
        SnapRequest { modes: vec!["perpendicular".to_string()], point: Some([5.0, 5.0]), tolerance: Some(4.9), ..SnapRequest::default() },
    )
    .expect("filtered");
    assert_eq!(filtered["count"], json!(0), "a tolerance below the nearest candidate drops every point");
    let kept = snap_points(
        handle,
        SnapRequest { modes: vec!["perpendicular".to_string()], point: Some([5.0, 5.0]), tolerance: Some(5.1), ..SnapRequest::default() },
    )
    .expect("kept");
    assert_eq!(kept["count"], json!(2));

    assert!(
        snap_points(handle, SnapRequest { modes: vec!["perpendicular".to_string()], ..SnapRequest::default() }).is_err(),
        "perpendicular needs a probe point"
    );

    let default = snap_points(handle, SnapRequest::default()).expect("default");
    let modes = default["modes"].as_array().expect("modes").iter().map(|value| value.as_str().expect("mode").to_string()).collect::<Vec<_>>();
    assert!(!modes.contains(&"perpendicular".to_string()), "perpendicular is opt-in");
    close(handle).expect("close");
}

#[test]
fn snap_tangent_reports_contact_points_and_rejects_inside_probes() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    let circle = add_circle(handle, DEFAULT_LAYER, [0.0, 0.0], 10.0).expect("circle");
    let circle_id = circle["entity"]["id"].as_str().expect("circle id").to_string();
    let line = add_line(handle, DEFAULT_LAYER, [-20.0, 30.0], [20.0, 30.0]).expect("line");
    let line_id = line["entity"]["id"].as_str().expect("line id").to_string();

    let outside = snap_points(
        handle,
        SnapRequest {
            modes: vec!["tangent".to_string()],
            point: Some([20.0, 0.0]),
            ids: Some(vec![circle_id.clone()]),
            ..SnapRequest::default()
        },
    )
    .expect("circle tangent");
    assert_eq!(outside["count"], json!(2), "a probe outside the circle gives two contact points");
    assert_eq!(outside["points"][0]["kind"], json!("tangent"));
    for point in snap_candidate_points(&outside) {
        let radius = point[0].hypot(point[1]);
        assert!((radius - 10.0).abs() < 1e-9, "the contact point sits on the circle: {point:?}");
        let tangent = [20.0 - point[0], 0.0 - point[1]];
        assert!(
            (point[0] * tangent[0] + point[1] * tangent[1]).abs() < 1e-9,
            "the radius at the contact point is perpendicular to the tangent: {point:?}"
        );
    }
    assert!(snap_has_point(&outside, [5.0, 75.0_f64.sqrt()]));
    assert!(snap_has_point(&outside, [5.0, -75.0_f64.sqrt()]));

    let on_circle = snap_points(
        handle,
        SnapRequest {
            modes: vec!["tangent".to_string()],
            point: Some([10.0, 0.0]),
            ids: Some(vec![circle_id.clone()]),
            ..SnapRequest::default()
        },
    )
    .expect("on circle tangent");
    assert_eq!(on_circle["count"], json!(1), "a probe on the circle gives that point once");
    assert!(snap_has_point(&on_circle, [10.0, 0.0]));

    let inside = snap_points(
        handle,
        SnapRequest {
            modes: vec!["tangent".to_string()],
            point: Some([5.0, 0.0]),
            ids: Some(vec![circle_id.clone()]),
            ..SnapRequest::default()
        },
    )
    .expect("inside tangent");
    assert_eq!(inside["count"], json!(0), "a probe inside the circle has no tangent");

    let at_center = snap_points(
        handle,
        SnapRequest { modes: vec!["tangent".to_string()], point: Some([0.0, 0.0]), ids: Some(vec![circle_id]), ..SnapRequest::default() },
    )
    .expect("centre tangent");
    assert_eq!(at_center["count"], json!(0), "a probe at the centre has no tangent");

    let on_line = snap_points(
        handle,
        SnapRequest { modes: vec!["tangent".to_string()], point: Some([0.0, 20.0]), ids: Some(vec![line_id]), ..SnapRequest::default() },
    )
    .expect("line tangent");
    assert_eq!(on_line["count"], json!(0), "a line contributes no tangent candidate");
    close(handle).expect("close");
}

#[test]
fn snap_tangent_filters_arcs_and_needs_a_probe() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    let arc = add_arc(handle, DEFAULT_LAYER, [0.0, 0.0], 10.0, 0.0, 90.0).expect("arc");
    let arc_id = arc["entity"]["id"].as_str().expect("arc id").to_string();

    let points = snap_points(
        handle,
        SnapRequest { modes: vec!["tangent".to_string()], point: Some([20.0, 0.0]), ids: Some(vec![arc_id.clone()]), ..SnapRequest::default() },
    )
    .expect("arc tangent");
    assert_eq!(points["count"], json!(1), "only the contact point inside the sweep is reported");
    assert_eq!(points["points"][0]["kind"], json!("tangent"));
    assert!(snap_has_point(&points, [5.0, 75.0_f64.sqrt()]));
    assert!(!snap_has_point(&points, [5.0, -75.0_f64.sqrt()]), "the opposite contact point lies outside the 0-90 sweep");

    let opposite = add_arc(handle, DEFAULT_LAYER, [0.0, 0.0], 10.0, 180.0, 270.0).expect("opposite arc");
    let opposite_id = opposite["entity"]["id"].as_str().expect("arc id").to_string();
    let none = snap_points(
        handle,
        SnapRequest { modes: vec!["tangent".to_string()], point: Some([20.0, 0.0]), ids: Some(vec![opposite_id]), ..SnapRequest::default() },
    )
    .expect("opposite arc tangent");
    assert_eq!(none["count"], json!(0), "both contact points fall outside the 180-270 sweep");

    assert!(
        snap_points(handle, SnapRequest { modes: vec!["tangent".to_string()], ..SnapRequest::default() }).is_err(),
        "tangent needs a probe point"
    );

    let default = snap_points(handle, SnapRequest::default()).expect("default");
    let modes = default["modes"].as_array().expect("modes").iter().map(|value| value.as_str().expect("mode").to_string()).collect::<Vec<_>>();
    assert!(!modes.contains(&"tangent".to_string()), "tangent is opt-in");
    close(handle).expect("close");
}

#[test]
fn snap_tangent_tolerance_and_ordering_stay_deterministic() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    let first = add_circle(handle, DEFAULT_LAYER, [0.0, 0.0], 10.0).expect("first circle");
    let second = add_circle(handle, DEFAULT_LAYER, [0.0, 0.0], 10.0).expect("second circle");

    let ordered = snap_points(handle, SnapRequest { modes: vec!["tangent".to_string()], point: Some([20.0, 0.0]), ..SnapRequest::default() })
        .expect("ordered");
    assert_eq!(ordered["count"], json!(4));
    assert_eq!(ordered["points"][0]["entities"][0], first["entity"]["id"], "equal distances sort by entity id");
    assert_eq!(ordered["points"][1]["entities"][0], first["entity"]["id"]);
    assert_eq!(ordered["points"][2]["entities"][0], second["entity"]["id"]);
    assert_eq!(ordered["points"][3]["entities"][0], second["entity"]["id"]);
    let again = snap_points(handle, SnapRequest { modes: vec!["tangent".to_string()], point: Some([20.0, 0.0]), ..SnapRequest::default() })
        .expect("again");
    assert_eq!(ordered["points"], again["points"]);

    let filtered = snap_points(
        handle,
        SnapRequest { modes: vec!["tangent".to_string()], point: Some([20.0, 0.0]), tolerance: Some(17.3), ..SnapRequest::default() },
    )
    .expect("filtered");
    assert_eq!(filtered["count"], json!(0), "a tolerance below the nearest contact point drops every candidate");
    let kept = snap_points(
        handle,
        SnapRequest { modes: vec!["tangent".to_string()], point: Some([20.0, 0.0]), tolerance: Some(17.4), ..SnapRequest::default() },
    )
    .expect("kept");
    assert_eq!(kept["count"], json!(4));
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

    let cleared = update_properties(handle, PropertiesUpdate { author: Some(String::new()), ..PropertiesUpdate::default() }).expect("clear");
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

    assert_eq!(select(handle, SelectionMode::Window, [[5.0, 5.0], [5.0, 5.0]], None).expect_err("degenerate").code, "session/argument-invalid");
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

    let closed =
        update_polyline(handle, "entity-1", PolylineUpdate { closed: Some(true), ..PolylineUpdate::default() }).expect("close the polyline");
    assert_eq!(closed["entity"]["kind"]["closed"], json!(true));
    assert_eq!(closed["revision"], json!(3));

    let inserted =
        update_polyline(handle, "entity-1", PolylineUpdate { insert_at: Some(1), insert_point: Some([5.0, 0.0]), ..PolylineUpdate::default() })
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

    let moved =
        update_polyline(handle, "entity-1", PolylineUpdate { move_at: Some(1), move_to: Some([12.0, 3.0]), ..PolylineUpdate::default() })
            .expect("move a vertex");
    assert_eq!(moved["entity"]["kind"]["points"][1], json!([12.0, 3.0]));
    assert_eq!(moved["entity"]["kind"]["points"].as_array().map(Vec::len), Some(4));

    let replaced = update_polyline(
        handle,
        "entity-1",
        PolylineUpdate { points: Some(vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]]), closed: Some(true), ..PolylineUpdate::default() },
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
        update_polyline(handle, "entity-1", PolylineUpdate { move_at: Some(0), ..PolylineUpdate::default() })
            .expect_err("move without a point")
            .code,
        "session/argument-invalid"
    );
    assert_eq!(
        update_polyline(handle, "entity-1", PolylineUpdate { move_at: Some(99), move_to: Some([1.0, 1.0]), ..PolylineUpdate::default() })
            .expect_err("move index beyond the list")
            .code,
        "session/argument-invalid"
    );
    assert_eq!(
        update_polyline(
            handle,
            "entity-1",
            PolylineUpdate { remove_at: Some(0), move_at: Some(0), move_to: Some([1.0, 1.0]), ..PolylineUpdate::default() }
        )
        .expect_err("one vertex edit per call")
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
    assert_eq!(add_rectangle(handle, DEFAULT_LAYER, [10.0, 10.0], [0.0, 0.0]).expect_err("inverted corners").code, "session/argument-invalid");
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
    assert_eq!(add_polygon(handle, DEFAULT_LAYER, [0.0, 0.0], 0.0, 4, 0.0).expect_err("non-positive radius").code, "session/argument-invalid");
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
    assert_eq!(update_arc(handle, "entity-1", None, Some(f64::INFINITY), None).expect_err("non-finite angle").code, "session/argument-invalid");
    assert_eq!(update_arc(handle, "entity-404", Some(2.0), None, None).expect_err("missing entity").code, "object/not-found");
    update_layer(handle, DEFAULT_LAYER, LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock");
    assert_eq!(update_arc(handle, "entity-1", Some(2.0), None, None).expect_err("locked layer").code, "drawing/layer-locked");
    close(handle).expect("close");
}

#[test]
fn ellipse_fields_are_editable_in_place() {
    let dir = TempDir::new("ellipse-edit-round-trip");
    let path = dir.path.join("ellipse.ocad");
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_ellipse(handle, DEFAULT_LAYER, [10.0, 5.0], 20.0, 10.0, 0.0, 0.0, 360.0).expect("ellipse");

    let rx = update_ellipse(handle, "entity-1", EllipseUpdate { rx: Some(25.0), ..EllipseUpdate::default() }).expect("update rx");
    assert_eq!(rx["entity"]["kind"]["rx"], json!(25.0));
    assert_eq!(rx["entity"]["kind"]["ry"], json!(10.0), "untouched ry keeps its value");
    assert_eq!(rx["entity"]["kind"]["center"], json!([10.0, 5.0]));
    assert_eq!(rx["entity"]["id"], json!("entity-1"));
    assert_eq!(rx["entity"]["layer"], json!(DEFAULT_LAYER));
    assert_eq!(rx["revision"], json!(3), "one edit bumps the revision once");

    let ry = update_ellipse(handle, "entity-1", EllipseUpdate { ry: Some(4.0), ..EllipseUpdate::default() }).expect("update ry");
    assert_eq!(ry["entity"]["kind"]["ry"], json!(4.0));
    assert_eq!(ry["entity"]["kind"]["rx"], json!(25.0));

    let rotation =
        update_ellipse(handle, "entity-1", EllipseUpdate { rotation: Some(30.0), ..EllipseUpdate::default() }).expect("update rotation");
    assert_eq!(rotation["entity"]["kind"]["rotation"], json!(30.0));
    assert_eq!(rotation["entity"]["kind"]["start_angle"], json!(0.0));

    let sweep =
        update_ellipse(handle, "entity-1", EllipseUpdate { start_angle: Some(45.0), end_angle: Some(180.0), ..EllipseUpdate::default() })
            .expect("update the sweep");
    assert_eq!(sweep["entity"]["kind"]["start_angle"], json!(45.0));
    assert_eq!(sweep["entity"]["kind"]["end_angle"], json!(180.0));
    assert_eq!(sweep["entity"]["kind"]["rotation"], json!(30.0), "untouched rotation keeps its value");

    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [1.0, 0.0]).expect("line");
    assert_eq!(
        update_ellipse(handle, "entity-2", EllipseUpdate { rx: Some(2.0), ..EllipseUpdate::default() })
            .expect_err("a line is not an ellipse")
            .code,
        "object/unsupported"
    );
    assert_eq!(update_ellipse(handle, "entity-1", EllipseUpdate::default()).expect_err("no edit").code, "session/argument-invalid");
    assert_eq!(
        update_ellipse(handle, "entity-1", EllipseUpdate { rx: Some(0.0), ..EllipseUpdate::default() }).expect_err("zero rx").code,
        "session/argument-invalid"
    );
    assert_eq!(
        update_ellipse(handle, "entity-1", EllipseUpdate { ry: Some(-1.0), ..EllipseUpdate::default() }).expect_err("negative ry").code,
        "session/argument-invalid"
    );
    assert_eq!(
        update_ellipse(handle, "entity-1", EllipseUpdate { rx: Some(f64::NAN), ..EllipseUpdate::default() }).expect_err("non-finite rx").code,
        "session/argument-invalid"
    );
    assert_eq!(
        update_ellipse(handle, "entity-1", EllipseUpdate { rotation: Some(f64::INFINITY), ..EllipseUpdate::default() })
            .expect_err("non-finite rotation")
            .code,
        "session/argument-invalid"
    );
    assert_eq!(
        update_ellipse(handle, "entity-1", EllipseUpdate { end_angle: Some(f64::NAN), ..EllipseUpdate::default() })
            .expect_err("non-finite end angle")
            .code,
        "session/argument-invalid"
    );
    assert_eq!(
        update_ellipse(handle, "entity-404", EllipseUpdate { rx: Some(2.0), ..EllipseUpdate::default() }).expect_err("missing entity").code,
        "object/not-found"
    );

    save(handle, &path, true).expect("save");
    close(handle).expect("close");
    let opened = open(&path).expect("open");
    let reopened = handle_of(&opened);
    let drawing = drawing_of(reopened);
    assert_eq!(
        drawing.entities[0].kind,
        EntityKind::Ellipse { center: [10.0, 5.0], rx: 25.0, ry: 4.0, rotation: 30.0, start_angle: 45.0, end_angle: 180.0 }
    );
    update_layer(reopened, DEFAULT_LAYER, LayerUpdate { frozen: Some(true), ..LayerUpdate::default() }).expect("freeze");
    assert_eq!(
        update_ellipse(reopened, "entity-1", EllipseUpdate { rx: Some(2.0), ..EllipseUpdate::default() }).expect_err("frozen layer").code,
        "drawing/layer-frozen"
    );
    update_layer(reopened, DEFAULT_LAYER, LayerUpdate { frozen: Some(false), locked: Some(true), ..LayerUpdate::default() }).expect("lock");
    assert_eq!(
        update_ellipse(reopened, "entity-1", EllipseUpdate { rx: Some(2.0), ..EllipseUpdate::default() }).expect_err("locked layer").code,
        "drawing/layer-locked"
    );
    close(reopened).expect("close reopened");

    let again = create(CreateRequest::default()).expect("create again");
    let again_handle = handle_of(&again);
    add_ellipse(again_handle, DEFAULT_LAYER, [10.0, 5.0], 20.0, 10.0, 0.0, 0.0, 360.0).expect("ellipse again");
    let repeated = update_ellipse(again_handle, "entity-1", EllipseUpdate { rx: Some(25.0), ..EllipseUpdate::default() }).expect("rx again");
    assert_eq!(repeated["entity"]["kind"], rx["entity"]["kind"], "the same input yields the same geometry");
    close(again_handle).expect("close again");
}

#[test]
fn spline_fit_points_and_closure_are_editable() {
    let dir = TempDir::new("spline-edit-round-trip");
    let path = dir.path.join("spline.ocad");
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_spline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [10.0, 5.0], [20.0, 0.0]], false).expect("spline");

    let closed = update_spline(handle, "entity-1", SplineUpdate { closed: Some(true), ..SplineUpdate::default() }).expect("close the spline");
    assert_eq!(closed["entity"]["kind"]["closed"], json!(true));
    assert_eq!(closed["entity"]["id"], json!("entity-1"));
    assert_eq!(closed["entity"]["layer"], json!(DEFAULT_LAYER));
    assert_eq!(closed["revision"], json!(3), "one edit bumps the revision once");

    let inserted =
        update_spline(handle, "entity-1", SplineUpdate { insert_at: Some(1), insert_point: Some([5.0, 6.0]), ..SplineUpdate::default() })
            .expect("insert a fit point");
    assert_eq!(inserted["entity"]["kind"]["points"], json!([[0.0, 0.0], [5.0, 6.0], [10.0, 5.0], [20.0, 0.0]]));

    let appended =
        update_spline(handle, "entity-1", SplineUpdate { insert_at: Some(4), insert_point: Some([30.0, 5.0]), ..SplineUpdate::default() })
            .expect("append a fit point");
    assert_eq!(appended["entity"]["kind"]["points"].as_array().map(Vec::len), Some(5));

    let removed =
        update_spline(handle, "entity-1", SplineUpdate { remove_at: Some(1), ..SplineUpdate::default() }).expect("remove a fit point");
    assert_eq!(removed["entity"]["kind"]["points"].as_array().map(Vec::len), Some(4));

    let moved = update_spline(handle, "entity-1", SplineUpdate { move_at: Some(1), move_to: Some([12.0, 8.0]), ..SplineUpdate::default() })
        .expect("move a fit point");
    assert_eq!(moved["entity"]["kind"]["points"][1], json!([12.0, 8.0]));
    assert_eq!(moved["entity"]["kind"]["points"].as_array().map(Vec::len), Some(4));

    let replaced = update_spline(
        handle,
        "entity-1",
        SplineUpdate { points: Some(vec![[0.0, 0.0], [5.0, 5.0], [10.0, 0.0]]), closed: Some(false), ..SplineUpdate::default() },
    )
    .expect("replace the fit points");
    assert_eq!(replaced["entity"]["kind"]["points"], json!([[0.0, 0.0], [5.0, 5.0], [10.0, 0.0]]));
    assert_eq!(replaced["entity"]["kind"]["closed"], json!(false));

    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [1.0, 0.0]).expect("line");
    add_spline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [1.0, 1.0]], false).expect("two-point spline");
    assert_eq!(
        update_spline(handle, "entity-2", SplineUpdate { closed: Some(true), ..SplineUpdate::default() })
            .expect_err("a line is not a spline")
            .code,
        "object/unsupported"
    );
    assert_eq!(update_spline(handle, "entity-1", SplineUpdate::default()).expect_err("no edit").code, "session/argument-invalid");
    assert_eq!(
        update_spline(handle, "entity-1", SplineUpdate { insert_at: Some(0), ..SplineUpdate::default() })
            .expect_err("insert without a point")
            .code,
        "session/argument-invalid"
    );
    assert_eq!(
        update_spline(handle, "entity-1", SplineUpdate { insert_at: Some(99), insert_point: Some([0.0, 0.0]), ..SplineUpdate::default() })
            .expect_err("index beyond the list")
            .code,
        "session/argument-invalid"
    );
    assert_eq!(
        update_spline(handle, "entity-1", SplineUpdate { move_at: Some(0), ..SplineUpdate::default() }).expect_err("move without a point").code,
        "session/argument-invalid"
    );
    assert_eq!(
        update_spline(handle, "entity-1", SplineUpdate { move_at: Some(99), move_to: Some([1.0, 1.0]), ..SplineUpdate::default() })
            .expect_err("move index beyond the list")
            .code,
        "session/argument-invalid"
    );
    assert_eq!(
        update_spline(
            handle,
            "entity-1",
            SplineUpdate { remove_at: Some(0), move_at: Some(0), move_to: Some([1.0, 1.0]), ..SplineUpdate::default() }
        )
        .expect_err("one fit-point edit per call")
        .code,
        "session/argument-invalid"
    );
    assert_eq!(
        update_spline(handle, "entity-1", SplineUpdate { points: Some(vec![[0.0, 0.0]]), ..SplineUpdate::default() })
            .expect_err("too few fit points")
            .code,
        "session/argument-invalid"
    );
    assert_eq!(
        update_spline(handle, "entity-3", SplineUpdate { remove_at: Some(0), ..SplineUpdate::default() })
            .expect_err("removing leaves one fit point")
            .code,
        "session/argument-invalid"
    );
    assert_eq!(
        update_spline(handle, "entity-404", SplineUpdate { closed: Some(true), ..SplineUpdate::default() }).expect_err("missing entity").code,
        "object/not-found"
    );

    save(handle, &path, true).expect("save");
    close(handle).expect("close");
    let opened = open(&path).expect("open");
    let reopened = handle_of(&opened);
    let drawing = drawing_of(reopened);
    assert_eq!(drawing.entities[0].kind, EntityKind::Spline { points: vec![[0.0, 0.0], [5.0, 5.0], [10.0, 0.0]], closed: false });
    update_layer(reopened, DEFAULT_LAYER, LayerUpdate { frozen: Some(true), ..LayerUpdate::default() }).expect("freeze");
    assert_eq!(
        update_spline(reopened, "entity-1", SplineUpdate { closed: Some(true), ..SplineUpdate::default() }).expect_err("frozen layer").code,
        "drawing/layer-frozen"
    );
    update_layer(reopened, DEFAULT_LAYER, LayerUpdate { frozen: Some(false), locked: Some(true), ..LayerUpdate::default() }).expect("lock");
    assert_eq!(
        update_spline(reopened, "entity-1", SplineUpdate { closed: Some(true), ..SplineUpdate::default() }).expect_err("locked layer").code,
        "drawing/layer-locked"
    );
    close(reopened).expect("close reopened");

    let again = create(CreateRequest::default()).expect("create again");
    let again_handle = handle_of(&again);
    add_spline(again_handle, DEFAULT_LAYER, vec![[0.0, 0.0], [10.0, 5.0], [20.0, 0.0]], false).expect("spline again");
    let repeated =
        update_spline(again_handle, "entity-1", SplineUpdate { insert_at: Some(1), insert_point: Some([5.0, 6.0]), ..SplineUpdate::default() })
            .expect("insert again");
    assert_eq!(repeated["entity"]["kind"]["points"], inserted["entity"]["kind"]["points"], "the same input yields the same fit points");
    close(again_handle).expect("close again");
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
    assert_eq!(assign_entity_layer(handle, "entity-1", DEFAULT_LAYER).expect_err("the source layer is frozen").code, "drawing/layer-frozen");
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

    let group =
        create_group(handle, GroupCreateRequest { name: "Frame".to_string(), entities: vec!["entity-1".to_string(), "entity-2".to_string()] })
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
    let codes = report["issues"].as_array().expect("issues").iter().map(|issue| issue["code"].as_str().unwrap_or_default()).collect::<Vec<_>>();
    for expected in ["layers/duplicate-name", "layers/hidden", "geometry/empty-text", "geometry/zero-length-segment", "dimensions/empty-text"] {
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
        "0", "SECTION", "2", "BLOCKS", "0", "BLOCK", "8", "0", "2", "DOOR", "70", "0", "10", "0", "20", "0", "0", "LINE", "8", "0", "10", "0",
        "20", "0", "11", "1", "21", "0", "0", "ENDBLK", "8", "0", "0", "ENDSEC", "0", "SECTION", "2", "ENTITIES", "0", "INSERT", "8", "WALLS",
        "2", "DOOR", "10", "10", "20", "20", "41", "2", "42", "2", "50", "90", "0", "ENDSEC", "0", "EOF",
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
    assert!(losses.iter().any(|loss| loss.code == "dxf/expanded-block" && loss.severity == "notice"), "the expansion is a notice: {losses:?}");
    assert!(losses.iter().any(|loss| loss.code == "dxf/implicit-layer"));
}

#[test]
fn dxf_import_stops_recursive_blocks_at_the_depth_limit() {
    let text = [
        "0", "SECTION", "2", "BLOCKS", "0", "BLOCK", "8", "0", "2", "LOOP", "70", "0", "10", "0", "20", "0", "0", "INSERT", "8", "0", "2",
        "LOOP", "10", "0", "20", "0", "0", "ENDBLK", "8", "0", "0", "ENDSEC", "0", "SECTION", "2", "ENTITIES", "0", "INSERT", "8", "0", "2",
        "LOOP", "10", "0", "20", "0", "0", "ENDSEC", "0", "EOF",
    ]
    .join("\n");

    let (drawing, losses) = import::dxf(&text, None).expect("import");
    assert!(drawing.entities.is_empty(), "a self-referencing block expands no geometry");
    assert!(losses.iter().any(|loss| loss.code == "dxf/block-recursion" && loss.severity == "warning"), "recursion is reported: {losses:?}");
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
fn spline_entities_add_measure_and_round_trip_through_dxf() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    let points = vec![[0.0, 0.0], [10.0, 10.0], [20.0, 0.0], [30.0, 10.0]];
    let added = add_spline(handle, DEFAULT_LAYER, points.clone(), false).expect("add spline");
    assert_eq!(added["entity"]["kind"]["type"], json!("spline"));
    assert_eq!(added["entity"]["kind"]["points"], json!(points));
    assert_eq!(add_spline(handle, DEFAULT_LAYER, vec![[0.0, 0.0]], false).expect_err("one fit point").code, "session/argument-invalid");
    assert_eq!(
        add_spline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [f64::NAN, 0.0]], false).expect_err("nan fit point").code,
        "session/argument-invalid"
    );

    let measured = measure_entities(handle, &["entity-1".to_string()]).expect("measure spline");
    assert!(measured["entities"][0]["length"].as_f64().expect("length") > 30.0, "the sampled curve is longer than the chord");
    assert_eq!(measured["entities"][0]["area"], json!(0.0), "an open spline encloses no area");
    let selected = select(handle, SelectionMode::Window, [[-1.0, -1.0], [31.0, 20.0]], None).expect("select spline");
    assert!(
        selected["ids"].as_array().is_some_and(|ids| ids.contains(&json!("entity-1"))),
        "a window over the sampled curve selects the spline: {selected}"
    );

    let svg = store::read(handle, |session| exchange::svg(&session.drawing)).expect("render svg");
    assert!(svg.contains("<path"), "the spline is drawn as a path: {svg}");
    assert!(svg.matches(" L").count() >= 3 * 16, "each of the 3 spans is sampled: {svg}");

    let (text, losses) = store::read(handle, |session| exchange::dxf(&session.drawing)).expect("export dxf");
    assert!(text.contains("\nSPLINE\n"), "a spline is written as a SPLINE entity: {text}");
    assert!(text.contains("AC1015"), "a spline raises the DXF version: {text}");
    assert!(losses.is_empty(), "a spline writes without losses: {:?}", losses.iter().map(|loss| loss.code.clone()).collect::<Vec<_>>());
    let (imported, import_losses) = import::dxf(&text, None).expect("import dxf");
    let EntityKind::Spline { points: imported_points, closed } = &imported.entities[0].kind
    else {
        panic!("SPLINE should import as a spline, got {:?}", imported.entities[0].kind);
    };
    assert_eq!(imported_points, &points, "fit points round-trip exactly");
    assert!(!closed);
    assert!(import_losses.iter().any(|loss| loss.code == "dxf/spline-fit-points"));

    let square = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
    let closed_added = add_spline(handle, DEFAULT_LAYER, square, true).expect("closed spline");
    let closed_id = closed_added["entity"]["id"].as_str().expect("id").to_string();
    let closed_measured = measure_entities(handle, &[closed_id]).expect("measure closed spline");
    assert!(closed_measured["entities"][0]["area"].as_f64().expect("area") > 0.0);
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
    let codes = report["issues"].as_array().expect("issues").iter().map(|issue| issue["code"].as_str().unwrap_or_default()).collect::<Vec<_>>();
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
    let codes = report["issues"].as_array().expect("issues").iter().map(|issue| issue["code"].as_str().unwrap_or_default()).collect::<Vec<_>>();
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
fn offset_by_side_changes_only_the_radius_of_circles_and_arcs() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_circle(handle, DEFAULT_LAYER, [5.0, 0.0], 10.0).expect("circle");

    let grown = offset_by_side(handle, "entity-1", 2.0, OffsetSide::Outside).expect("offset the circle outward");
    assert_eq!(grown["entity"]["kind"]["radius"], json!(12.0));
    assert_eq!(grown["entity"]["kind"]["center"], json!([5.0, 0.0]));
    assert_eq!(grown["entity"]["id"], json!("entity-2"));
    assert_eq!(grown["entity"]["layer"], json!(DEFAULT_LAYER));
    assert_eq!(grown["side"], json!("outside"));
    assert_eq!(grown["distance"], json!(2.0));

    let shrunk = offset_by_side(handle, "entity-1", 4.0, OffsetSide::Inside).expect("offset the circle inward");
    assert_eq!(shrunk["entity"]["kind"]["radius"], json!(6.0));
    assert_eq!(shrunk["entity"]["id"], json!("entity-3"));
    assert_eq!(offset_by_side(handle, "entity-1", 10.0, OffsetSide::Inside).expect_err("collapsing radius").code, "offset/invalid-distance");
    assert_eq!(offset_by_side(handle, "entity-1", 0.0, OffsetSide::Outside).expect_err("zero distance").code, "offset/invalid-distance");
    assert_eq!(offset_by_side(handle, "entity-1", -1.0, OffsetSide::Outside).expect_err("negative distance").code, "offset/invalid-distance");
    assert_eq!(offset_by_side(handle, "entity-1", f64::NAN, OffsetSide::Outside).expect_err("nan distance").code, "offset/invalid-distance");
    assert_eq!(offset_by_side(handle, "entity-1", 1.0, OffsetSide::Left).expect_err("linear side on a circle").code, "offset/invalid-side");

    add_arc(handle, DEFAULT_LAYER, [20.0, 0.0], 5.0, 0.0, 90.0).expect("arc");
    let arc = offset_by_side(handle, "entity-4", 1.0, OffsetSide::Outside).expect("offset the arc");
    assert_eq!(arc["entity"]["kind"]["radius"], json!(6.0));
    assert_eq!(arc["entity"]["kind"]["center"], json!([20.0, 0.0]));
    assert_eq!(arc["entity"]["kind"]["start_angle"], json!(0.0));
    assert_eq!(arc["entity"]["kind"]["end_angle"], json!(90.0));

    let inset = offset_by_side(handle, "entity-4", 2.0, OffsetSide::Inside).expect("shrink the arc");
    assert_eq!(inset["entity"]["kind"]["radius"], json!(3.0));
    assert_eq!(inset["entity"]["kind"]["start_angle"], json!(0.0));
    assert_eq!(inset["entity"]["kind"]["end_angle"], json!(90.0));

    let drawing = drawing_of(handle);
    assert_eq!(drawing.entities[0].kind, EntityKind::Circle { center: [5.0, 0.0], radius: 10.0 });
    assert_eq!(drawing.entities[3].kind, EntityKind::Arc { center: [20.0, 0.0], radius: 5.0, start_angle: 0.0, end_angle: 90.0 });
    close(handle).expect("close");
}

#[test]
fn offset_by_side_moves_lines_and_polylines_perpendicular_to_their_direction() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("line");
    let left = offset_by_side(handle, "entity-1", 2.0, OffsetSide::Left).expect("offset the line left");
    assert_eq!(left["entity"]["kind"]["from"], json!([0.0, 2.0]));
    assert_eq!(left["entity"]["kind"]["to"], json!([10.0, 2.0]));
    let right = offset_by_side(handle, "entity-1", 3.0, OffsetSide::Right).expect("offset the line right");
    assert_eq!(right["entity"]["kind"]["from"], json!([0.0, -3.0]));
    assert_eq!(right["entity"]["kind"]["to"], json!([10.0, -3.0]));
    assert_eq!(offset_by_side(handle, "entity-1", 1.0, OffsetSide::Outside).expect_err("radial side on a line").code, "offset/invalid-side");

    add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]], false).expect("open polyline");
    let moved = offset_by_side(handle, "entity-4", 1.0, OffsetSide::Left).expect("offset the open polyline");
    assert_eq!(moved["entity"]["kind"]["points"], json!([[0.0, 1.0], [9.0, 1.0], [9.0, 10.0]]));

    add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]], true).expect("closed polyline");
    let inset = offset_by_side(handle, "entity-6", 1.0, OffsetSide::Left).expect("offset the closed polyline inward");
    assert_eq!(inset["entity"]["kind"]["points"], json!([[1.0, 1.0], [3.0, 1.0], [3.0, 3.0], [1.0, 3.0]]));
    assert_eq!(inset["entity"]["kind"]["closed"], json!(true));
    close(handle).expect("close");
}

#[test]
fn offset_by_side_refuses_a_miter_beyond_four_times_the_distance() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [10.0, 0.0], [0.0, 0.1]], false).expect("sharp polyline");
    assert_eq!(offset_by_side(handle, "entity-1", 1.0, OffsetSide::Left).expect_err("sharp miter").code, "offset/miter-limit");

    add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [10.0, 0.0], [0.0, -10.0]], false).expect("right angle");
    offset_by_side(handle, "entity-2", 1.0, OffsetSide::Left).expect("a right angle fits under the limit");
    close(handle).expect("close");
}

#[test]
fn offset_by_side_reports_unknown_and_unsupported_entities() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_text(handle, DEFAULT_LAYER, [0.0, 0.0], 2.0, "note").expect("text");
    add_ellipse(handle, DEFAULT_LAYER, [0.0, 0.0], 8.0, 3.0, 0.0, 0.0, 360.0).expect("ellipse");
    add_spline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [4.0, 2.0], [8.0, 0.0]], false).expect("spline");
    add_dimension(handle, DIMENSION_LAYER, [0.0, 0.0], [10.0, 0.0], 2.0, None).expect("dimension");

    for id in ["entity-1", "entity-2", "entity-3", "entity-4"] {
        assert_eq!(offset_by_side(handle, id, 1.0, OffsetSide::Left).expect_err("unsupported").code, "offset/unsupported-entity");
    }
    assert_eq!(offset_by_side(handle, "entity-404", 1.0, OffsetSide::Left).expect_err("missing").code, "offset/not-found");
    assert_eq!(drawing_of(handle).entities.len(), 4);
    close(handle).expect("close");
}

#[test]
fn offset_by_side_is_deterministic_and_survives_save_and_reopen() {
    let dir = TempDir::new("offset-side-round-trip");
    let path = dir.path.join("offset.ocad");
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("line");
    let first = offset_by_side(handle, "entity-1", 2.0, OffsetSide::Left).expect("first offset");
    assert_eq!(first["entity"]["id"], json!("entity-2"));
    let second = offset_by_side(handle, "entity-1", 2.0, OffsetSide::Left).expect("second offset");
    assert_eq!(second["entity"]["id"], json!("entity-3"), "ids are deterministic and never reused");

    save(handle, &path, true).expect("save");
    close(handle).expect("close");
    let opened = open(&path).expect("open");
    let reopened = handle_of(&opened);
    let drawing = drawing_of(reopened);
    assert_eq!(drawing.entities.len(), 3);
    assert_eq!(drawing.entities[1].kind, EntityKind::Line { from: [0.0, 2.0], to: [10.0, 2.0] });
    assert_eq!(drawing.entities[2].kind, EntityKind::Line { from: [0.0, 2.0], to: [10.0, 2.0] });
    close(reopened).expect("close reopened");

    let again = create(CreateRequest::default()).expect("create again");
    let again_handle = handle_of(&again);
    add_line(again_handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("line again");
    let repeated = offset_by_side(again_handle, "entity-1", 2.0, OffsetSide::Left).expect("offset again");
    assert_eq!(repeated["entity"]["id"], json!("entity-2"), "the same input yields the same id");
    close(again_handle).expect("close again");
}

#[test]
fn signed_and_side_offsets_agree() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("line");
    add_circle(handle, DEFAULT_LAYER, [5.0, 0.0], 10.0).expect("circle");
    add_arc(handle, DEFAULT_LAYER, [20.0, 0.0], 5.0, 0.0, 90.0).expect("arc");
    add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]], false).expect("open polyline");
    add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]], true).expect("closed polyline");

    let cases = [
        ("entity-1", 2.0, OffsetSide::Left),
        ("entity-1", -3.0, OffsetSide::Right),
        ("entity-2", 4.0, OffsetSide::Outside),
        ("entity-2", -4.0, OffsetSide::Inside),
        ("entity-3", 1.0, OffsetSide::Outside),
        ("entity-3", -1.0, OffsetSide::Inside),
        ("entity-4", 1.0, OffsetSide::Left),
        ("entity-4", -1.0, OffsetSide::Right),
        ("entity-5", 1.0, OffsetSide::Left),
        ("entity-5", -1.0, OffsetSide::Right),
    ];
    for (id, distance, side) in cases {
        let signed = offset(handle, id, distance).expect("signed offset");
        let by_side = offset_by_side(handle, id, distance.abs(), side).expect("side offset");
        assert_eq!(signed["entity"]["kind"], by_side["entity"]["kind"], "{id} with {side:?}");
        assert_eq!(signed["entity"]["layer"], by_side["entity"]["layer"], "{id} with {side:?}");
    }
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
fn trim_line_with_line_keeps_the_longer_side() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("target");
    add_line(handle, DEFAULT_LAYER, [3.0, -5.0], [3.0, 5.0]).expect("cutter");

    let result = trim(handle, "entity-1", "entity-2").expect("trim");
    assert_eq!(result["entity"]["id"], json!("entity-1"));
    assert_eq!(result["cutter"], json!("entity-2"));
    assert_eq!(result["revision"], json!(4));
    assert_eq!(result["entity"]["kind"]["type"], json!("line"));
    assert_point(&result["entity"]["kind"]["from"], [3.0, 0.0]);
    assert_point(&result["entity"]["kind"]["to"], [10.0, 0.0]);

    let drawing = drawing_of(handle);
    let EntityKind::Line { from, to } = drawing.entities[0].kind
    else {
        panic!("expected a trimmed line");
    };
    assert!((from[0] - 3.0).abs() < 1e-9 && from[1].abs() < 1e-9, "trimmed from {from:?}");
    assert!((to[0] - 10.0).abs() < 1e-9 && to[1].abs() < 1e-9, "trimmed to {to:?}");
    assert_eq!(drawing.entities[1].kind, EntityKind::Line { from: [3.0, -5.0], to: [3.0, 5.0] }, "the cutter stays put");
    close(handle).expect("close");
}

#[test]
fn trim_line_with_circle_keeps_the_longer_outer_segment() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("first target");
    add_circle(handle, DEFAULT_LAYER, [7.0, 0.0], 2.0).expect("two-crossing circle");
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("second target");
    add_circle(handle, DEFAULT_LAYER, [1.0, 0.0], 3.0).expect("one-crossing circle");

    let both = trim(handle, "entity-1", "entity-2").expect("trim across the circle");
    assert_point(&both["entity"]["kind"]["from"], [0.0, 0.0]);
    assert_point(&both["entity"]["kind"]["to"], [5.0, 0.0]);

    let one = trim(handle, "entity-3", "entity-4").expect("trim at the single crossing");
    assert_point(&one["entity"]["kind"]["from"], [4.0, 0.0]);
    assert_point(&one["entity"]["kind"]["to"], [10.0, 0.0]);
    close(handle).expect("close");
}

#[test]
fn trim_line_with_arc_keeps_the_longer_side_and_filters_the_sweep() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("first target");
    add_arc(handle, DEFAULT_LAYER, [7.0, 0.0], 2.0, 0.0, 180.0).expect("two-crossing arc");
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("second target");
    add_arc(handle, DEFAULT_LAYER, [5.0, 0.0], 3.0, 90.0, 270.0).expect("half-circle arc");

    let both = trim(handle, "entity-1", "entity-2").expect("trim across the arc");
    assert_point(&both["entity"]["kind"]["from"], [0.0, 0.0]);
    assert_point(&both["entity"]["kind"]["to"], [5.0, 0.0]);

    let one = trim(handle, "entity-3", "entity-4").expect("only the left crossing lies on the sweep");
    assert_point(&one["entity"]["kind"]["from"], [2.0, 0.0]);
    assert_point(&one["entity"]["kind"]["to"], [10.0, 0.0]);
    close(handle).expect("close");
}

#[test]
fn trim_reports_missing_intersections_and_degenerate_cutters() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("target");
    add_line(handle, DEFAULT_LAYER, [0.0, 4.0], [10.0, 4.0]).expect("parallel cutter");
    add_line(handle, DEFAULT_LAYER, [2.0, 0.0], [8.0, 0.0]).expect("collinear cutter");
    add_circle(handle, DEFAULT_LAYER, [5.0, 1.0], 1.0).expect("tangent circle");
    add_circle(handle, DEFAULT_LAYER, [5.0, 10.0], 1.0).expect("distant circle");
    add_line(handle, DEFAULT_LAYER, [0.0, -5.0], [0.0, 5.0]).expect("endpoint cutter");
    add_line(handle, DEFAULT_LAYER, [4.0, 4.0], [4.0, 4.0]).expect("zero-length target");
    add_text(handle, DEFAULT_LAYER, [0.0, 0.0], 2.0, "note").expect("text");

    assert_eq!(trim(handle, "entity-1", "entity-2").expect_err("parallel").code, "trim/no-intersection");
    assert_eq!(trim(handle, "entity-1", "entity-3").expect_err("collinear").code, "trim/degenerate");
    assert_eq!(trim(handle, "entity-1", "entity-4").expect_err("tangent").code, "trim/degenerate");
    assert_eq!(trim(handle, "entity-1", "entity-5").expect_err("distant").code, "trim/no-intersection");
    assert_eq!(trim(handle, "entity-1", "entity-6").expect_err("endpoint cut").code, "trim/degenerate");
    assert_eq!(trim(handle, "entity-7", "entity-2").expect_err("zero-length target").code, "trim/degenerate");
    assert_eq!(trim(handle, "entity-1", "entity-8").expect_err("text cutter").code, "object/unsupported");
    assert_eq!(trim(handle, "entity-8", "entity-1").expect_err("text target").code, "object/unsupported");
    assert_eq!(trim(handle, "entity-1", "entity-404").expect_err("missing cutter").code, "object/not-found");
    assert_eq!(trim(handle, "entity-1", "entity-1").expect_err("same entity").code, "session/argument-invalid");
    close(handle).expect("close");
}

#[test]
fn trim_refuses_a_locked_target_layer() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_layer(handle, "Structure", Some("#ff0000")).expect("layer");
    add_line(handle, "Structure", [0.0, 0.0], [10.0, 0.0]).expect("target");
    add_circle(handle, DEFAULT_LAYER, [5.0, 0.0], 2.0).expect("cutter");
    update_layer(handle, "Structure", LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock");
    assert_eq!(trim(handle, "entity-1", "entity-2").expect_err("locked trim").code, "drawing/layer-locked");
    close(handle).expect("close");
}

#[test]
fn trim_arc_with_line_keeps_the_longer_side_and_exact_angles() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_arc(handle, DEFAULT_LAYER, [0.0, 0.0], 2.0, 0.0, 180.0).expect("target arc");
    add_line(handle, DEFAULT_LAYER, [1.0, -5.0], [1.0, 5.0]).expect("cutter line");

    let result = trim(handle, "entity-1", "entity-2").expect("trim the arc");
    assert_eq!(result["entity"]["id"], json!("entity-1"));
    assert_eq!(result["cutter"], json!("entity-2"));
    assert_eq!(result["revision"], json!(4));
    assert_eq!(result["entity"]["kind"]["type"], json!("arc"));
    assert_point(&result["entity"]["kind"]["center"], [0.0, 0.0]);
    assert_eq!(result["entity"]["kind"]["radius"], json!(2.0));
    let result_start = result["entity"]["kind"]["start_angle"].as_f64().expect("start angle");
    assert!((result_start - 60.0).abs() < 1e-9, "start {result_start}");
    assert_eq!(result["entity"]["kind"]["end_angle"], json!(180.0));

    let EntityKind::Arc { center, radius, start_angle, end_angle } = drawing_of(handle).entities[0].kind.clone()
    else {
        panic!("the trimmed target must stay an arc");
    };
    assert_point(&json!(center), [0.0, 0.0]);
    assert!((radius - 2.0).abs() < 1e-9, "radius {radius}");
    assert!((start_angle - 60.0).abs() < 1e-9, "start {start_angle}");
    assert!((end_angle - 180.0).abs() < 1e-9, "end {end_angle}");
    close(handle).expect("close");
}

#[test]
fn trim_arc_with_circle_keeps_the_longer_outer_portion() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_arc(handle, DEFAULT_LAYER, [0.0, 0.0], 5.0, 0.0, 240.0).expect("target arc");
    add_circle(handle, DEFAULT_LAYER, [0.0, 3.0], 4.0).expect("cutter circle");

    let result = trim(handle, "entity-1", "entity-2").expect("trim the arc");
    assert_eq!(result["entity"]["kind"]["type"], json!("arc"));
    assert_point(&result["entity"]["kind"]["center"], [0.0, 0.0]);
    assert_eq!(result["entity"]["kind"]["radius"], json!(5.0));
    let start = result["entity"]["kind"]["start_angle"].as_f64().expect("start angle");
    let end = result["entity"]["kind"]["end_angle"].as_f64().expect("end angle");
    assert!((start - 143.130_102_354_155_98).abs() < 1e-9, "kept start {start}");
    assert!((end - 240.0).abs() < 1e-9, "kept end {end}");
    assert!((arc_sweep(start, end) - 96.869_897_645_844_02).abs() < 1e-9, "kept sweep");
    close(handle).expect("close");
}

#[test]
fn trim_arc_with_arc_filters_the_cutter_sweep() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_arc(handle, DEFAULT_LAYER, [0.0, 0.0], 2.0, 0.0, 180.0).expect("first target arc");
    add_arc(handle, DEFAULT_LAYER, [2.0, 0.0], 2.0, 0.0, 180.0).expect("upper cutter arc");
    add_arc(handle, DEFAULT_LAYER, [0.0, 0.0], 2.0, 0.0, 180.0).expect("second target arc");
    add_arc(handle, DEFAULT_LAYER, [2.0, 0.0], 2.0, 180.0, 360.0).expect("lower cutter arc");

    let kept = trim(handle, "entity-1", "entity-2").expect("only the upper crossing lies on both sweeps");
    assert_eq!(kept["entity"]["kind"]["type"], json!("arc"));
    let start = kept["entity"]["kind"]["start_angle"].as_f64().expect("start angle");
    assert!((start - 60.0).abs() < 1e-9, "start {start}");
    assert_eq!(kept["entity"]["kind"]["end_angle"], json!(180.0));

    assert_eq!(trim(handle, "entity-3", "entity-4").expect_err("the lower crossing misses the target sweep").code, "trim/no-intersection");
    close(handle).expect("close");
}

#[test]
fn trim_circle_with_line_becomes_an_arc_over_the_longer_sweep() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_circle(handle, DEFAULT_LAYER, [5.0, 0.0], 2.0).expect("target circle");
    add_line(handle, DEFAULT_LAYER, [4.0, -5.0], [4.0, 5.0]).expect("cutter line");

    let result = trim(handle, "entity-1", "entity-2").expect("trim the circle");
    assert_eq!(result["entity"]["id"], json!("entity-1"));
    assert_eq!(result["entity"]["kind"]["type"], json!("arc"));
    assert_point(&result["entity"]["kind"]["center"], [5.0, 0.0]);
    assert_eq!(result["entity"]["kind"]["radius"], json!(2.0));
    let start = result["entity"]["kind"]["start_angle"].as_f64().expect("start angle");
    let end = result["entity"]["kind"]["end_angle"].as_f64().expect("end angle");
    assert!((start - 240.0).abs() < 1e-9, "start {start}");
    assert!((end - 120.0).abs() < 1e-9, "end {end}");
    assert!((arc_sweep(start, end) - 240.0).abs() < 1e-9, "the removed span must be the shorter one");
    close(handle).expect("close");
}

#[test]
fn trim_arc_and_circle_report_missing_or_single_intersections() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_arc(handle, DEFAULT_LAYER, [0.0, 0.0], 2.0, 0.0, 180.0).expect("arc target");
    add_line(handle, DEFAULT_LAYER, [10.0, -5.0], [10.0, 5.0]).expect("distant line");
    add_circle(handle, DEFAULT_LAYER, [0.0, 0.0], 2.0).expect("circle target");
    add_circle(handle, DEFAULT_LAYER, [10.0, 0.0], 1.0).expect("distant circle");
    add_line(handle, DEFAULT_LAYER, [2.0, -5.0], [2.0, 5.0]).expect("tangent line");
    add_circle(handle, DEFAULT_LAYER, [3.0, 0.0], 1.0).expect("tangent circle");
    add_circle(handle, DEFAULT_LAYER, [0.0, 0.0], 0.0).expect("zero-radius target");
    add_line(handle, DEFAULT_LAYER, [0.0, -5.0], [0.0, 5.0]).expect("cutter");

    assert_eq!(trim(handle, "entity-1", "entity-2").expect_err("arc miss").code, "trim/no-intersection");
    assert_eq!(trim(handle, "entity-3", "entity-4").expect_err("circle miss").code, "trim/no-intersection");
    assert_eq!(trim(handle, "entity-3", "entity-5").expect_err("line tangent to circle").code, "trim/degenerate");
    assert_eq!(trim(handle, "entity-3", "entity-6").expect_err("circle tangent to circle").code, "trim/degenerate");
    assert_eq!(trim(handle, "entity-7", "entity-8").expect_err("zero-radius target").code, "trim/degenerate");
    close(handle).expect("close");
}

#[test]
fn trim_arc_refuses_a_cut_on_an_endpoint() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_arc(handle, DEFAULT_LAYER, [0.0, 0.0], 2.0, 0.0, 180.0).expect("target arc");
    add_line(handle, DEFAULT_LAYER, [2.0, -5.0], [2.0, 5.0]).expect("cutter tangent at the start angle");

    let error = trim(handle, "entity-1", "entity-2").expect_err("endpoint cut");
    assert_eq!(error.code, "trim/degenerate");
    assert!(error.message.contains("nothing to trim"), "message was {}", error.message);
    close(handle).expect("close");
}

#[test]
fn trim_refuses_a_locked_arc_target_layer() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_layer(handle, "Structure", Some("#ff0000")).expect("layer");
    add_arc(handle, "Structure", [0.0, 0.0], 5.0, 0.0, 180.0).expect("target arc");
    add_circle(handle, DEFAULT_LAYER, [3.0, 0.0], 2.0).expect("cutter");
    update_layer(handle, "Structure", LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock");
    assert_eq!(trim(handle, "entity-1", "entity-2").expect_err("locked arc trim").code, "drawing/layer-locked");
    close(handle).expect("close");
}

#[test]
fn extend_line_with_line_moves_the_near_endpoint_to_the_crossing() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("target");
    add_line(handle, DEFAULT_LAYER, [16.0, -5.0], [16.0, 5.0]).expect("boundary");

    let result = extend(handle, "entity-1", "entity-2", ExtendSide::End).expect("extend");
    assert_eq!(result["entity"]["id"], json!("entity-1"));
    assert_eq!(result["boundary"], json!("entity-2"));
    assert_eq!(result["side"], json!("end"));
    assert_eq!(result["revision"], json!(4));
    assert_eq!(result["entity"]["kind"]["type"], json!("line"));
    assert_point(&result["entity"]["kind"]["from"], [0.0, 0.0]);
    assert_point(&result["entity"]["kind"]["to"], [16.0, 0.0]);

    let drawing = drawing_of(handle);
    assert_eq!(drawing.entities[1].kind, EntityKind::Line { from: [16.0, -5.0], to: [16.0, 5.0] }, "the boundary stays put");
    close(handle).expect("close");
}

#[test]
fn extend_line_with_circle_stops_at_the_near_crossing() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("target");
    add_circle(handle, DEFAULT_LAYER, [16.0, 0.0], 2.0).expect("boundary");

    let result = extend(handle, "entity-1", "entity-2", ExtendSide::End).expect("extend to the circle");
    assert_point(&result["entity"]["kind"]["from"], [0.0, 0.0]);
    assert_point(&result["entity"]["kind"]["to"], [14.0, 0.0]);
    assert_eq!(result["boundary"], json!("entity-2"));
    close(handle).expect("close");
}

#[test]
fn extend_line_with_arc_filters_the_sweep() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("first target");
    add_arc(handle, DEFAULT_LAYER, [16.0, 0.0], 2.0, 0.0, 180.0).expect("right-half arc");
    add_line(handle, DEFAULT_LAYER, [0.0, 2.0], [10.0, 2.0]).expect("second target");
    add_arc(handle, DEFAULT_LAYER, [16.0, 2.0], 2.0, 45.0, 135.0).expect("upper arc outside the crossings");

    let extended = extend(handle, "entity-1", "entity-2", ExtendSide::End).expect("extend to the arc");
    assert_point(&extended["entity"]["kind"]["from"], [0.0, 0.0]);
    assert_point(&extended["entity"]["kind"]["to"], [14.0, 0.0]);

    assert_eq!(
        extend(handle, "entity-3", "entity-4", ExtendSide::End).expect_err("both crossings miss the sweep").code,
        "extend/no-intersection"
    );
    close(handle).expect("close");
}

#[test]
fn extend_start_side_moves_the_from_endpoint() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("target");
    add_line(handle, DEFAULT_LAYER, [-6.0, -5.0], [-6.0, 5.0]).expect("boundary");

    let result = extend(handle, "entity-1", "entity-2", ExtendSide::Start).expect("extend from the start");
    assert_eq!(result["side"], json!("start"));
    assert_point(&result["entity"]["kind"]["from"], [-6.0, 0.0]);
    assert_point(&result["entity"]["kind"]["to"], [10.0, 0.0]);
    close(handle).expect("close");
}

#[test]
fn extend_reports_missing_intersections_and_unsupported_entities() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("target");
    add_line(handle, DEFAULT_LAYER, [0.0, 4.0], [20.0, 4.0]).expect("parallel boundary");
    add_circle(handle, DEFAULT_LAYER, [5.0, 0.0], 2.0).expect("circle behind the target");
    add_line(handle, DEFAULT_LAYER, [6.0, -5.0], [6.0, 5.0]).expect("boundary before the endpoint");
    add_text(handle, DEFAULT_LAYER, [0.0, 0.0], 2.0, "note").expect("text");
    add_line(handle, DEFAULT_LAYER, [4.0, 4.0], [4.0, 4.0]).expect("zero-length target");

    assert_eq!(extend(handle, "entity-1", "entity-2", ExtendSide::End).expect_err("parallel").code, "extend/no-intersection");
    assert_eq!(extend(handle, "entity-1", "entity-3", ExtendSide::End).expect_err("behind").code, "extend/no-intersection");
    assert_eq!(extend(handle, "entity-1", "entity-4", ExtendSide::End).expect_err("before the endpoint").code, "extend/no-intersection");
    assert_eq!(extend(handle, "entity-5", "entity-1", ExtendSide::End).expect_err("text target").code, "object/unsupported");
    assert_eq!(extend(handle, "entity-1", "entity-5", ExtendSide::End).expect_err("text boundary").code, "object/unsupported");
    assert_eq!(extend(handle, "entity-1", "entity-404", ExtendSide::End).expect_err("missing boundary").code, "object/not-found");
    assert_eq!(extend(handle, "entity-1", "entity-1", ExtendSide::End).expect_err("same entity").code, "session/argument-invalid");
    assert_eq!(extend(handle, "entity-6", "entity-1", ExtendSide::End).expect_err("zero-length target").code, "session/argument-invalid");
    close(handle).expect("close");
}

#[test]
fn extend_refuses_a_locked_target_layer() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_layer(handle, "Structure", Some("#ff0000")).expect("layer");
    add_line(handle, "Structure", [0.0, 0.0], [10.0, 0.0]).expect("target");
    add_line(handle, DEFAULT_LAYER, [16.0, -5.0], [16.0, 5.0]).expect("boundary");
    update_layer(handle, "Structure", LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock");
    assert_eq!(extend(handle, "entity-1", "entity-2", ExtendSide::End).expect_err("locked extend").code, "drawing/layer-locked");
    close(handle).expect("close");
}

#[test]
fn extend_arc_start_to_line_walks_clockwise_from_the_start_angle() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_arc(handle, DEFAULT_LAYER, [0.0, 0.0], 10.0, 0.0, 90.0).expect("target arc");
    add_line(handle, DEFAULT_LAYER, [-20.0, -5.0], [20.0, -5.0]).expect("boundary line");

    let result = extend(handle, "entity-1", "entity-2", ExtendSide::Start).expect("extend the arc start");
    assert_eq!(result["entity"]["id"], json!("entity-1"));
    assert_eq!(result["boundary"], json!("entity-2"));
    assert_eq!(result["side"], json!("start"));
    assert_eq!(result["entity"]["kind"]["type"], json!("arc"));
    assert_point(&result["entity"]["kind"]["center"], [0.0, 0.0]);
    assert_eq!(result["entity"]["kind"]["radius"], json!(10.0));
    let start_angle = result["entity"]["kind"]["start_angle"].as_f64().expect("start angle");
    assert!((start_angle - 330.0).abs() < 1e-9, "start {start_angle}");
    assert_eq!(result["entity"]["kind"]["end_angle"], json!(90.0));
    close(handle).expect("close");
}

#[test]
fn extend_arc_end_to_line_walks_counter_clockwise_from_the_end_angle() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_arc(handle, DEFAULT_LAYER, [0.0, 0.0], 10.0, 0.0, 90.0).expect("target arc");
    add_line(handle, DEFAULT_LAYER, [-20.0, -5.0], [20.0, -5.0]).expect("boundary line");

    let result = extend(handle, "entity-1", "entity-2", ExtendSide::End).expect("extend the arc end");
    assert_eq!(result["entity"]["kind"]["type"], json!("arc"));
    assert_point(&result["entity"]["kind"]["center"], [0.0, 0.0]);
    assert_eq!(result["entity"]["kind"]["radius"], json!(10.0));
    assert_eq!(result["entity"]["kind"]["start_angle"], json!(0.0));
    let end_angle = result["entity"]["kind"]["end_angle"].as_f64().expect("end angle");
    assert!((end_angle - 210.0).abs() < 1e-9, "end {end_angle}");
    close(handle).expect("close");
}

#[test]
fn extend_arc_to_circle_stops_at_the_near_crossing() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    let boundary_center = [8.0 * 50f64.to_radians().cos(), 8.0 * 50f64.to_radians().sin()];
    let boundary_radius = (100.0 + 64.0 - 160.0 * 10f64.to_radians().cos()).sqrt();
    add_arc(handle, DEFAULT_LAYER, [0.0, 0.0], 10.0, 0.0, 10.0).expect("target arc");
    add_circle(handle, DEFAULT_LAYER, boundary_center, boundary_radius).expect("boundary circle");

    let result = extend(handle, "entity-1", "entity-2", ExtendSide::End).expect("extend to the circle");
    assert_eq!(result["entity"]["kind"]["type"], json!("arc"));
    assert_point(&result["entity"]["kind"]["center"], [0.0, 0.0]);
    assert_eq!(result["entity"]["kind"]["radius"], json!(10.0));
    assert_eq!(result["entity"]["kind"]["start_angle"], json!(0.0));
    let end_angle = result["entity"]["kind"]["end_angle"].as_f64().expect("end angle");
    assert!((end_angle - 40.0).abs() < 1e-6, "end {end_angle}");
    close(handle).expect("close");
}

#[test]
fn extend_arc_to_arc_filters_the_boundary_sweep() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    let boundary_center = [8.0 * 50f64.to_radians().cos(), 8.0 * 50f64.to_radians().sin()];
    let boundary_radius = (100.0 + 64.0 - 160.0 * 10f64.to_radians().cos()).sqrt();
    add_arc(handle, DEFAULT_LAYER, [0.0, 0.0], 10.0, 0.0, 10.0).expect("target arc");
    add_arc(handle, DEFAULT_LAYER, boundary_center, boundary_radius, 80.0, 110.0).expect("boundary arc covers only the far crossing");

    let result = extend(handle, "entity-1", "entity-2", ExtendSide::End).expect("extend to the arc boundary");
    assert_eq!(result["entity"]["kind"]["type"], json!("arc"));
    assert_point(&result["entity"]["kind"]["center"], [0.0, 0.0]);
    assert_eq!(result["entity"]["kind"]["radius"], json!(10.0));
    assert_eq!(result["entity"]["kind"]["start_angle"], json!(0.0));
    let end_angle = result["entity"]["kind"]["end_angle"].as_f64().expect("end angle");
    assert!((end_angle - 60.0).abs() < 1e-6, "end {end_angle}");
    close(handle).expect("close");
}

#[test]
fn extend_arc_ignores_a_crossing_behind_the_endpoint() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_arc(handle, DEFAULT_LAYER, [0.0, 0.0], 10.0, 0.0, 90.0).expect("target arc");
    add_line(handle, DEFAULT_LAYER, [-20.0, 5.0], [20.0, 5.0]).expect("boundary line crossing inside the sweep");

    let result = extend(handle, "entity-1", "entity-2", ExtendSide::End).expect("extend past the inner crossing");
    assert_eq!(result["entity"]["kind"]["start_angle"], json!(0.0));
    let end_angle = result["entity"]["kind"]["end_angle"].as_f64().expect("end angle");
    assert!((end_angle - 150.0).abs() < 1e-9, "the inner crossing at 30 degrees must not shorten the arc, got {end_angle}");
    close(handle).expect("close");
}

#[test]
fn extend_arc_across_zero_degrees_keeps_the_sweep_continuous() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_arc(handle, DEFAULT_LAYER, [0.0, 0.0], 10.0, 350.0, 10.0).expect("target arc across zero");
    add_line(handle, DEFAULT_LAYER, [10.0 * 30f64.to_radians().cos(), -20.0], [10.0 * 30f64.to_radians().cos(), 20.0]).expect("boundary line");

    let result = extend(handle, "entity-1", "entity-2", ExtendSide::End).expect("extend across zero");
    assert_eq!(result["entity"]["kind"]["start_angle"], json!(350.0));
    let end_angle = result["entity"]["kind"]["end_angle"].as_f64().expect("end angle");
    assert!((end_angle - 30.0).abs() < 1e-9, "end {end_angle}");
    close(handle).expect("close");
}

#[test]
fn extend_arc_refuses_a_full_sweep_and_reports_missing_boundaries() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    let tangent_x = 10.0 * 349f64.to_radians().cos();
    add_arc(handle, DEFAULT_LAYER, [0.0, 0.0], 10.0, 0.0, 350.0).expect("almost full arc");
    add_line(handle, DEFAULT_LAYER, [tangent_x, -20.0], [tangent_x, 20.0]).expect("boundary line");
    add_circle(handle, DEFAULT_LAYER, [100.0, 0.0], 1.0).expect("far circle");
    add_text(handle, DEFAULT_LAYER, [0.0, 0.0], 2.0, "note").expect("text");

    assert_eq!(
        extend(handle, "entity-1", "entity-2", ExtendSide::End).expect_err("the nearest crossing wraps a full turn").code,
        "extend/no-intersection"
    );
    assert_eq!(extend(handle, "entity-1", "entity-3", ExtendSide::End).expect_err("no crossing").code, "extend/no-intersection");
    assert_eq!(extend(handle, "entity-1", "entity-4", ExtendSide::End).expect_err("text boundary").code, "object/unsupported");
    assert_eq!(extend(handle, "entity-1", "entity-404", ExtendSide::Start).expect_err("missing boundary").code, "object/not-found");
    close(handle).expect("close");
}

#[test]
fn extend_refuses_a_locked_arc_target_layer() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_layer(handle, "Structure", Some("#ff0000")).expect("layer");
    add_arc(handle, "Structure", [0.0, 0.0], 10.0, 0.0, 90.0).expect("target arc");
    add_line(handle, DEFAULT_LAYER, [16.0, -5.0], [16.0, 5.0]).expect("boundary line");
    update_layer(handle, "Structure", LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock");
    assert_eq!(extend(handle, "entity-1", "entity-2", ExtendSide::End).expect_err("locked arc extend").code, "drawing/layer-locked");
    close(handle).expect("close");
}

#[test]
fn break_line_at_midpoint_creates_two_collinear_lines() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("line");

    let result = break_entity(handle, BreakRequest { id: "entity-1".to_string(), at: [4.0, 0.0] }).expect("break");
    assert_eq!(result["removed"], json!("entity-1"));
    assert_eq!(result["revision"], json!(3));
    let lines = result["created"].as_array().expect("created lines");
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["id"], json!("entity-2"));
    assert_eq!(lines[1]["id"], json!("entity-3"));
    assert_eq!(lines[0]["layer"], json!(DEFAULT_LAYER));
    assert_eq!(lines[1]["layer"], json!(DEFAULT_LAYER));
    assert_point(&lines[0]["kind"]["from"], [0.0, 0.0]);
    assert_point(&lines[0]["kind"]["to"], [4.0, 0.0]);
    assert_point(&lines[1]["kind"]["from"], [4.0, 0.0]);
    assert_point(&lines[1]["kind"]["to"], [10.0, 0.0]);

    let drawing = drawing_of(handle);
    assert_eq!(drawing.entities.iter().map(|entity| entity.id.as_str()).collect::<Vec<_>>(), ["entity-2", "entity-3"]);
    let total = drawing
        .entities
        .iter()
        .map(|entity| match entity.kind {
            EntityKind::Line { from, to } => distance(from, to),
            _ => panic!("break must only create lines"),
        })
        .sum::<f64>();
    assert!((total - 10.0).abs() < 1e-9, "the two pieces must add up to the original length, got {total}");

    let appended = add_line(handle, DEFAULT_LAYER, [20.0, 0.0], [30.0, 0.0]).expect("append");
    assert_eq!(appended["entity"]["id"], json!("entity-4"), "ids continue after the two new lines");
    assert_eq!(appended["revision"], json!(4));
    close(handle).expect("close");
}

#[test]
fn break_rejects_endpoints_outside_points_and_off_line_points() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("line");

    for at in [[0.0, 0.0], [10.0, 0.0], [-1.0, 0.0], [11.0, 0.0], [5.0, 0.01]] {
        assert_eq!(
            break_entity(handle, BreakRequest { id: "entity-1".to_string(), at }).expect_err("point off the interior").code,
            "break/not-on-entity",
            "point {at:?}"
        );
    }
    assert_eq!(
        break_entity(handle, BreakRequest { id: "entity-404".to_string(), at: [5.0, 0.0] }).expect_err("missing line").code,
        "object/not-found"
    );
    let drawing = drawing_of(handle);
    assert_eq!(drawing.entities.len(), 1, "a rejected break leaves the line untouched");
    assert_eq!(drawing.revision, 2);
    close(handle).expect("close");
}

#[test]
fn break_rejects_non_line_entities_and_locked_layers() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_circle(handle, DEFAULT_LAYER, [5.0, 0.0], 2.0).expect("circle");
    assert_eq!(
        break_entity(handle, BreakRequest { id: "entity-1".to_string(), at: [5.0, 0.0] }).expect_err("circle").code,
        "object/unsupported"
    );

    add_layer(handle, "Structure", Some("#ff0000")).expect("layer");
    add_line(handle, "Structure", [0.0, 0.0], [10.0, 0.0]).expect("line");
    update_layer(handle, "Structure", LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock");
    assert_eq!(
        break_entity(handle, BreakRequest { id: "entity-2".to_string(), at: [5.0, 0.0] }).expect_err("locked break").code,
        "drawing/layer-locked"
    );
    close(handle).expect("close");
}

#[test]
fn break_is_deterministic() {
    let run = || {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("line");
        let result = break_entity(handle, BreakRequest { id: "entity-1".to_string(), at: [2.5, 0.0] }).expect("break");
        close(handle).expect("close");
        result
    };

    let first = run();
    let second = run();
    assert_eq!(first["created"], second["created"]);
    assert_eq!(first["revision"], second["revision"]);
}

#[test]
fn break_arc_at_interior_point_splits_into_two_arcs() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_arc(handle, DEFAULT_LAYER, [0.0, 0.0], 5.0, 0.0, 180.0).expect("arc");

    let result = break_entity(handle, BreakRequest { id: "entity-1".to_string(), at: [0.0, 5.0] }).expect("break arc");
    assert_eq!(result["removed"], json!("entity-1"));
    assert_eq!(result["revision"], json!(3));
    let arcs = result["created"].as_array().expect("created arcs");
    assert_eq!(arcs.len(), 2);
    assert_eq!(arcs[0]["id"], json!("entity-1"), "the source keeps the first part");
    assert_eq!(arcs[1]["id"], json!("entity-2"));
    for arc in arcs {
        assert_eq!(arc["kind"]["type"], json!("arc"));
        assert_eq!(arc["layer"], json!(DEFAULT_LAYER));
        assert_point(&arc["kind"]["center"], [0.0, 0.0]);
        assert_eq!(arc["kind"]["radius"], json!(5.0));
    }
    assert_eq!(arcs[0]["kind"]["start_angle"], json!(0.0));
    assert_eq!(arcs[0]["kind"]["end_angle"], json!(90.0));
    assert_eq!(arcs[1]["kind"]["start_angle"], json!(90.0));
    assert_eq!(arcs[1]["kind"]["end_angle"], json!(180.0));

    let drawing = drawing_of(handle);
    assert_eq!(drawing.entities.iter().map(|entity| entity.id.as_str()).collect::<Vec<_>>(), ["entity-1", "entity-2"]);
    assert_eq!(drawing.revision, 3);

    let appended = add_arc(handle, DEFAULT_LAYER, [0.0, 0.0], 5.0, 180.0, 270.0).expect("append");
    assert_eq!(appended["entity"]["id"], json!("entity-3"), "ids continue after the split arc");
    assert_eq!(appended["revision"], json!(4));
    close(handle).expect("close");
}

#[test]
fn break_arc_keeps_the_wrapped_sweep_convention() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_arc(handle, DEFAULT_LAYER, [0.0, 0.0], 5.0, 350.0, 20.0).expect("arc");

    let at = arc_point([0.0, 0.0], 5.0, 5.0);
    let result = break_entity(handle, BreakRequest { id: "entity-1".to_string(), at }).expect("break arc");
    let arcs = result["created"].as_array().expect("created arcs");
    let first_start = arcs[0]["kind"]["start_angle"].as_f64().expect("start");
    let first_end = arcs[0]["kind"]["end_angle"].as_f64().expect("end");
    let second_start = arcs[1]["kind"]["start_angle"].as_f64().expect("start");
    let second_end = arcs[1]["kind"]["end_angle"].as_f64().expect("end");
    assert!((first_start - 350.0).abs() < 1e-9, "the first part keeps the source start angle, got {first_start}");
    assert!((first_end - 5.0).abs() < 1e-9, "the first part ends at the cut angle, got {first_end}");
    assert!((second_start - 5.0).abs() < 1e-9, "the second part starts at the cut angle, got {second_start}");
    assert!((second_end - 20.0).abs() < 1e-9, "the second part keeps the source end angle, got {second_end}");
    close(handle).expect("close");
}

#[test]
fn break_arc_rejects_endpoints_and_points_off_the_arc() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_arc(handle, DEFAULT_LAYER, [0.0, 0.0], 5.0, 0.0, 180.0).expect("arc");

    for at in [[5.0, 0.0], [-5.0, 0.0], [0.0, 4.9], [0.0, 5.1], [0.0, -5.0]] {
        assert_eq!(
            break_entity(handle, BreakRequest { id: "entity-1".to_string(), at }).expect_err("point off the interior").code,
            "break/not-on-entity",
            "point {at:?}"
        );
    }
    assert_eq!(
        break_entity(handle, BreakRequest { id: "entity-404".to_string(), at: [0.0, 5.0] }).expect_err("missing arc").code,
        "object/not-found"
    );
    let drawing = drawing_of(handle);
    assert_eq!(drawing.entities.len(), 1, "a rejected break leaves the arc untouched");
    assert_eq!(drawing.revision, 2);
    close(handle).expect("close");
}

#[test]
fn break_arc_rejects_zero_radius_and_locked_layers() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_arc(handle, DEFAULT_LAYER, [0.0, 0.0], 5.0, 0.0, 180.0).expect("arc");
    store::edit(handle, |drawing| {
        drawing.entities[0].kind = EntityKind::Arc { center: [0.0, 0.0], radius: 0.0, start_angle: 0.0, end_angle: 180.0 };
        Ok(())
    })
    .expect("force a zero-radius arc");
    assert_eq!(
        break_entity(handle, BreakRequest { id: "entity-1".to_string(), at: [0.0, 0.0] }).expect_err("zero radius").code,
        "break/not-on-entity"
    );

    add_layer(handle, "Structure", Some("#ff0000")).expect("layer");
    add_arc(handle, "Structure", [0.0, 0.0], 5.0, 0.0, 180.0).expect("arc");
    update_layer(handle, "Structure", LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock");
    assert_eq!(
        break_entity(handle, BreakRequest { id: "entity-2".to_string(), at: [0.0, 5.0] }).expect_err("locked break").code,
        "drawing/layer-locked"
    );
    close(handle).expect("close");
}

#[test]
fn join_horizontal_lines_into_one_segment() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [5.0, 0.0]).expect("first");
    add_line(handle, DEFAULT_LAYER, [5.0, 0.0], [10.0, 0.0]).expect("second");

    let result = join_entities(handle, JoinRequest { a: "entity-1".to_string(), b: "entity-2".to_string() }).expect("join");
    assert_eq!(result["removed"], json!(["entity-1", "entity-2"]));
    assert_eq!(result["revision"], json!(4), "a join is one revision bump");
    let merged = &result["created"][0];
    assert_eq!(merged["id"], json!("entity-3"));
    assert_eq!(merged["layer"], json!(DEFAULT_LAYER));
    assert_point(&merged["kind"]["from"], [0.0, 0.0]);
    assert_point(&merged["kind"]["to"], [10.0, 0.0]);
    assert_eq!(result["created"].as_array().expect("created").len(), 1);

    let drawing = drawing_of(handle);
    assert_eq!(drawing.entities.len(), 1, "both sources are removed");
    assert_eq!(drawing.entities[0].id, "entity-3");
    assert_eq!(drawing.revision, 4);

    let appended = add_line(handle, DEFAULT_LAYER, [20.0, 0.0], [30.0, 0.0]).expect("append");
    assert_eq!(appended["entity"]["id"], json!("entity-4"), "ids continue after the merged line");
    assert_eq!(appended["revision"], json!(5));
    close(handle).expect("close");
}

#[test]
fn join_slanted_lines_keeps_the_free_endpoints() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [3.0, 4.0]).expect("first");
    add_line(handle, DEFAULT_LAYER, [3.0, 4.0], [6.0, 8.0]).expect("second");

    let result = join_entities(handle, JoinRequest { a: "entity-1".to_string(), b: "entity-2".to_string() }).expect("join");
    let merged = &result["created"][0];
    assert_point(&merged["kind"]["from"], [0.0, 0.0]);
    assert_point(&merged["kind"]["to"], [6.0, 8.0]);
    let EntityKind::Line { from, to } = drawing_of(handle).entities[0].kind.clone()
    else {
        panic!("join must create a line");
    };
    assert!((distance(from, to) - 10.0).abs() < 1e-9, "the merged line keeps the summed length");
    close(handle).expect("close");
}

#[test]
fn join_accepts_a_reversed_second_line() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [5.0, 0.0]).expect("first");
    add_line(handle, DEFAULT_LAYER, [10.0, 0.0], [5.0, 0.0]).expect("second");

    let result = join_entities(handle, JoinRequest { a: "entity-1".to_string(), b: "entity-2".to_string() }).expect("join");
    let merged = &result["created"][0];
    assert_point(&merged["kind"]["from"], [0.0, 0.0]);
    assert_point(&merged["kind"]["to"], [10.0, 0.0]);
    close(handle).expect("close");
}

#[test]
fn join_rejects_lines_that_do_not_meet_or_are_not_collinear() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [5.0, 0.0]).expect("first");
    add_line(handle, DEFAULT_LAYER, [6.0, 0.0], [10.0, 0.0]).expect("gap");
    add_line(handle, DEFAULT_LAYER, [5.0, 0.0], [10.0, 5.0]).expect("bent");
    add_line(handle, DEFAULT_LAYER, [5.0, 0.0], [2.0, 0.0]).expect("folded");

    for b in ["entity-2", "entity-3", "entity-4"] {
        assert_eq!(
            join_entities(handle, JoinRequest { a: "entity-1".to_string(), b: b.to_string() }).expect_err("not adjacent").code,
            "join/not-adjacent",
            "second line {b}"
        );
    }
    assert_eq!(
        join_entities(handle, JoinRequest { a: "entity-1".to_string(), b: "entity-404".to_string() }).expect_err("missing line").code,
        "object/not-found"
    );
    assert_eq!(
        join_entities(handle, JoinRequest { a: "entity-1".to_string(), b: "entity-1".to_string() }).expect_err("same id").code,
        "session/argument-invalid"
    );
    let drawing = drawing_of(handle);
    assert_eq!(drawing.entities.len(), 4, "a rejected join leaves the lines untouched");
    assert_eq!(drawing.revision, 5, "a rejected join does not bump the revision");
    close(handle).expect("close");
}

#[test]
fn join_rejects_non_lines_and_cross_layer_pairs() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_circle(handle, DEFAULT_LAYER, [5.0, 0.0], 2.0).expect("circle");
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [5.0, 0.0]).expect("first");
    assert_eq!(
        join_entities(handle, JoinRequest { a: "entity-1".to_string(), b: "entity-2".to_string() }).expect_err("circle").code,
        "object/unsupported"
    );

    add_layer(handle, "Structure", Some("#ff0000")).expect("layer");
    add_line(handle, "Structure", [5.0, 0.0], [10.0, 0.0]).expect("second");
    assert_eq!(
        join_entities(handle, JoinRequest { a: "entity-2".to_string(), b: "entity-3".to_string() }).expect_err("layers").code,
        "object/unsupported"
    );
    close(handle).expect("close");
}

#[test]
fn join_refuses_a_locked_layer() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_layer(handle, "Structure", Some("#ff0000")).expect("layer");
    add_line(handle, "Structure", [0.0, 0.0], [5.0, 0.0]).expect("first");
    add_line(handle, "Structure", [5.0, 0.0], [10.0, 0.0]).expect("second");
    update_layer(handle, "Structure", LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock");
    assert_eq!(
        join_entities(handle, JoinRequest { a: "entity-1".to_string(), b: "entity-2".to_string() }).expect_err("locked join").code,
        "drawing/layer-locked"
    );
    close(handle).expect("close");
}

#[test]
fn join_is_deterministic() {
    let run = || {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [3.0, 4.0]).expect("first");
        add_line(handle, DEFAULT_LAYER, [3.0, 4.0], [6.0, 8.0]).expect("second");
        let result = join_entities(handle, JoinRequest { a: "entity-1".to_string(), b: "entity-2".to_string() }).expect("join");
        close(handle).expect("close");
        result
    };

    let first = run();
    let second = run();
    assert_eq!(first["created"], second["created"]);
    assert_eq!(first["removed"], second["removed"]);
    assert_eq!(first["revision"], second["revision"]);
}

#[test]
fn align_left_edges_share_the_smallest_min_x() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("long line");
    add_line(handle, DEFAULT_LAYER, [5.0, 5.0], [9.0, 5.0]).expect("short line");
    add_circle(handle, DEFAULT_LAYER, [20.0, 0.0], 2.0).expect("circle");

    let result = align(
        handle,
        AlignRequest { ids: vec!["entity-1".to_string(), "entity-2".to_string(), "entity-3".to_string()], mode: AlignMode::Left },
    )
    .expect("align left");
    assert_eq!(result["mode"], json!("left"));
    assert_eq!(result["affected"], json!(3));
    assert_eq!(result["revision"], json!(5), "an align is one revision bump");
    assert_eq!(result["bounds"]["minX"], json!(0.0));
    assert_eq!(result["bounds"]["maxX"], json!(10.0));

    let drawing = drawing_of(handle);
    assert_eq!(drawing.entities[1].kind, EntityKind::Line { from: [0.0, 5.0], to: [4.0, 5.0] }, "the short line moves to minX 0");
    assert_eq!(drawing.entities[2].kind, EntityKind::Circle { center: [2.0, 0.0], radius: 2.0 }, "the circle moves to minX 0");
    assert_eq!(drawing.revision, 5);
    close(handle).expect("close");
}

#[test]
fn align_center_and_middle_use_the_selection_midpoints() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("wide line");
    add_line(handle, DEFAULT_LAYER, [20.0, 0.0], [22.0, 0.0]).expect("narrow line");

    let centered = align(handle, AlignRequest { ids: vec!["entity-1".to_string(), "entity-2".to_string()], mode: AlignMode::Center })
        .expect("align center");
    assert_eq!(centered["revision"], json!(4));
    let drawing = drawing_of(handle);
    assert_eq!(drawing.entities[0].kind, EntityKind::Line { from: [6.0, 0.0], to: [16.0, 0.0] }, "centres meet at x 11");
    assert_eq!(drawing.entities[1].kind, EntityKind::Line { from: [10.0, 0.0], to: [12.0, 0.0] });

    add_circle(handle, DEFAULT_LAYER, [0.0, 0.0], 1.0).expect("small circle");
    add_circle(handle, DEFAULT_LAYER, [0.0, 8.0], 3.0).expect("large circle");
    let middled = align(handle, AlignRequest { ids: vec!["entity-3".to_string(), "entity-4".to_string()], mode: AlignMode::Middle })
        .expect("align middle");
    assert_eq!(middled["mode"], json!("middle"));
    let drawing = drawing_of(handle);
    assert_eq!(drawing.entities[2].kind, EntityKind::Circle { center: [0.0, 5.0], radius: 1.0 }, "centres meet at y 5");
    assert_eq!(drawing.entities[3].kind, EntityKind::Circle { center: [0.0, 5.0], radius: 3.0 });
    close(handle).expect("close");
}

#[test]
fn align_top_and_bottom_edges_share_the_extremes() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [5.0, 0.0]).expect("flat line");
    add_line(handle, DEFAULT_LAYER, [0.0, 4.0], [5.0, 8.0]).expect("tall line");

    let bottom = align(handle, AlignRequest { ids: vec!["entity-1".to_string(), "entity-2".to_string()], mode: AlignMode::Bottom })
        .expect("align bottom");
    assert_eq!(bottom["mode"], json!("bottom"));
    let drawing = drawing_of(handle);
    assert_eq!(drawing.entities[1].kind, EntityKind::Line { from: [0.0, 0.0], to: [5.0, 4.0] }, "bottom edges meet at y 0");

    let top =
        align(handle, AlignRequest { ids: vec!["entity-1".to_string(), "entity-2".to_string()], mode: AlignMode::Top }).expect("align top");
    assert_eq!(top["mode"], json!("top"));
    assert_eq!(top["revision"], json!(5), "the top align is the fifth revision");
    let drawing = drawing_of(handle);
    assert_eq!(drawing.entities[0].kind, EntityKind::Line { from: [0.0, 4.0], to: [5.0, 4.0] }, "top edges meet at y 4");
    assert_eq!(drawing.entities[1].kind, EntityKind::Line { from: [0.0, 0.0], to: [5.0, 4.0] }, "the tall line already ends at y 4");
    close(handle).expect("close");
}

#[test]
fn align_rejects_fewer_than_two_entities() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("line");

    assert_eq!(
        align(handle, AlignRequest { ids: vec!["entity-1".to_string()], mode: AlignMode::Left }).expect_err("one entity").code,
        "align/insufficient-entities"
    );
    assert_eq!(
        align(handle, AlignRequest { ids: vec!["entity-1".to_string(), "entity-1".to_string()], mode: AlignMode::Left })
            .expect_err("repeated id")
            .code,
        "align/insufficient-entities",
        "repeated ids deduplicate before the count"
    );
    assert_eq!(align(handle, AlignRequest { ids: vec![], mode: AlignMode::Left }).expect_err("empty ids").code, "session/argument-invalid");
    assert_eq!(
        align(handle, AlignRequest { ids: vec!["entity-1".to_string(), "entity-404".to_string()], mode: AlignMode::Left })
            .expect_err("missing id")
            .code,
        "object/not-found"
    );
    let drawing = drawing_of(handle);
    assert_eq!(drawing.revision, 2, "a rejected align does not bump the revision");
    close(handle).expect("close");
}

#[test]
fn align_refuses_locked_and_frozen_layers_and_degenerate_entities() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_layer(handle, "Structure", Some("#ff0000")).expect("layer");
    add_line(handle, "Structure", [0.0, 0.0], [5.0, 0.0]).expect("locked line");
    add_line(handle, DEFAULT_LAYER, [0.0, 10.0], [9.0, 10.0]).expect("line");
    update_layer(handle, "Structure", LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock");
    assert_eq!(
        align(handle, AlignRequest { ids: vec!["entity-1".to_string(), "entity-2".to_string()], mode: AlignMode::Right })
            .expect_err("locked align")
            .code,
        "drawing/layer-locked"
    );
    update_layer(handle, "Structure", LayerUpdate { locked: Some(false), frozen: Some(true), ..LayerUpdate::default() }).expect("freeze");
    assert_eq!(
        align(handle, AlignRequest { ids: vec!["entity-1".to_string(), "entity-2".to_string()], mode: AlignMode::Right })
            .expect_err("frozen align")
            .code,
        "drawing/layer-frozen"
    );

    update_layer(handle, "Structure", LayerUpdate { frozen: Some(false), ..LayerUpdate::default() }).expect("unfreeze");
    store::edit(handle, |drawing| {
        drawing.entities[0].kind = EntityKind::Line { from: [f64::NAN, f64::NAN], to: [f64::NAN, f64::NAN] };
        Ok(())
    })
    .expect("force a non-finite line");
    assert_eq!(
        align(handle, AlignRequest { ids: vec!["entity-1".to_string(), "entity-2".to_string()], mode: AlignMode::Right })
            .expect_err("degenerate align")
            .code,
        "object/degenerate"
    );
    assert_eq!(drawing_of(handle).revision, 7, "a rejected align does not bump the revision");
    close(handle).expect("close");
}

#[test]
fn distribute_equalizes_gaps_along_x() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [2.0, 0.0]).expect("first");
    add_line(handle, DEFAULT_LAYER, [3.0, 0.0], [9.0, 0.0]).expect("middle");
    add_line(handle, DEFAULT_LAYER, [20.0, 0.0], [24.0, 0.0]).expect("last");

    let result = distribute(
        handle,
        DistributeRequest { ids: vec!["entity-1".to_string(), "entity-2".to_string(), "entity-3".to_string()], axis: DistributeAxis::X },
    )
    .expect("distribute x");
    assert_eq!(result["axis"], json!("x"));
    assert_eq!(result["affected"], json!(3));
    assert_eq!(result["revision"], json!(5), "a distribute is one revision bump");
    assert_eq!(result["bounds"]["maxX"], json!(24.0), "the last entity stays put");

    let drawing = drawing_of(handle);
    assert_eq!(drawing.entities[0].kind, EntityKind::Line { from: [0.0, 0.0], to: [2.0, 0.0] }, "the first entity stays put");
    assert_eq!(drawing.entities[1].kind, EntityKind::Line { from: [8.0, 0.0], to: [14.0, 0.0] }, "gaps of 6 between the boxes");
    assert_eq!(drawing.entities[2].kind, EntityKind::Line { from: [20.0, 0.0], to: [24.0, 0.0] });
    close(handle).expect("close");
}

#[test]
fn distribute_along_y_orders_by_box_center() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [0.0, 2.0]).expect("bottom");
    add_line(handle, DEFAULT_LAYER, [0.0, 3.0], [0.0, 5.0]).expect("middle");
    add_line(handle, DEFAULT_LAYER, [0.0, 10.0], [0.0, 14.0]).expect("top");

    let result = distribute(
        handle,
        DistributeRequest { ids: vec!["entity-3".to_string(), "entity-1".to_string(), "entity-2".to_string()], axis: DistributeAxis::Y },
    )
    .expect("distribute y");
    assert_eq!(result["axis"], json!("y"));
    let drawing = drawing_of(handle);
    assert_eq!(drawing.entities[0].kind, EntityKind::Line { from: [0.0, 0.0], to: [0.0, 2.0] }, "the lowest box stays put");
    assert_eq!(drawing.entities[1].kind, EntityKind::Line { from: [0.0, 5.0], to: [0.0, 7.0] }, "equal gaps of 3");
    assert_eq!(drawing.entities[2].kind, EntityKind::Line { from: [0.0, 10.0], to: [0.0, 14.0] }, "the highest box stays put");
    close(handle).expect("close");
}

#[test]
fn distribute_rejects_fewer_than_three_entities() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [2.0, 0.0]).expect("first");
    add_line(handle, DEFAULT_LAYER, [3.0, 0.0], [9.0, 0.0]).expect("second");

    assert_eq!(
        distribute(handle, DistributeRequest { ids: vec!["entity-1".to_string(), "entity-2".to_string()], axis: DistributeAxis::X })
            .expect_err("two entities")
            .code,
        "distribute/insufficient-entities"
    );
    assert_eq!(
        distribute(
            handle,
            DistributeRequest { ids: vec!["entity-1".to_string(), "entity-2".to_string(), "entity-2".to_string()], axis: DistributeAxis::X }
        )
        .expect_err("deduplicated to two")
        .code,
        "distribute/insufficient-entities",
        "repeated ids deduplicate before the count"
    );
    assert_eq!(
        distribute(handle, DistributeRequest { ids: vec![], axis: DistributeAxis::X }).expect_err("empty ids").code,
        "session/argument-invalid"
    );
    assert_eq!(
        distribute(
            handle,
            DistributeRequest { ids: vec!["entity-1".to_string(), "entity-2".to_string(), "entity-404".to_string()], axis: DistributeAxis::X }
        )
        .expect_err("missing id")
        .code,
        "object/not-found"
    );
    assert_eq!(
        distribute(
            handle,
            DistributeRequest { ids: vec!["entity-1".to_string(), "entity-2".to_string(), "entity-3".to_string()], axis: DistributeAxis::X }
        )
        .expect_err("missing third id")
        .code,
        "object/not-found"
    );
    let drawing = drawing_of(handle);
    assert_eq!(drawing.revision, 3, "a rejected distribute does not bump the revision");
    close(handle).expect("close");
}

#[test]
fn distribute_refuses_locked_layers_and_degenerate_entities() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_layer(handle, "Structure", Some("#ff0000")).expect("layer");
    add_line(handle, "Structure", [0.0, 0.0], [2.0, 0.0]).expect("locked line");
    add_line(handle, DEFAULT_LAYER, [3.0, 0.0], [9.0, 0.0]).expect("middle");
    add_line(handle, DEFAULT_LAYER, [20.0, 0.0], [24.0, 0.0]).expect("last");
    update_layer(handle, "Structure", LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock");
    assert_eq!(
        distribute(
            handle,
            DistributeRequest { ids: vec!["entity-1".to_string(), "entity-2".to_string(), "entity-3".to_string()], axis: DistributeAxis::X }
        )
        .expect_err("locked distribute")
        .code,
        "drawing/layer-locked"
    );

    update_layer(handle, "Structure", LayerUpdate { locked: Some(false), ..LayerUpdate::default() }).expect("unlock");
    store::edit(handle, |drawing| {
        drawing.entities[2].kind = EntityKind::Line { from: [f64::NAN, f64::NAN], to: [f64::NAN, f64::NAN] };
        Ok(())
    })
    .expect("force a non-finite line");
    assert_eq!(
        distribute(
            handle,
            DistributeRequest { ids: vec!["entity-1".to_string(), "entity-2".to_string(), "entity-3".to_string()], axis: DistributeAxis::X }
        )
        .expect_err("degenerate distribute")
        .code,
        "object/degenerate"
    );
    close(handle).expect("close");
}

#[test]
fn align_and_distribute_are_single_history_steps() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("first");
    add_line(handle, DEFAULT_LAYER, [5.0, 5.0], [9.0, 5.0]).expect("second");

    let aligned =
        align(handle, AlignRequest { ids: vec!["entity-1".to_string(), "entity-2".to_string()], mode: AlignMode::Left }).expect("align");
    assert_eq!(aligned["revision"], json!(4));
    let undone = undo(handle).expect("undo the align");
    assert_eq!(undone["revision"], json!(5));
    assert_eq!(drawing_of(handle).entities[1].kind, EntityKind::Line { from: [5.0, 5.0], to: [9.0, 5.0] }, "undo restores the position");

    let redone = redo(handle).expect("redo the align");
    assert_eq!(redone["revision"], json!(6));
    assert_eq!(drawing_of(handle).entities[1].kind, EntityKind::Line { from: [0.0, 5.0], to: [4.0, 5.0] }, "redo reapplies the align");

    add_line(handle, DEFAULT_LAYER, [20.0, 0.0], [24.0, 0.0]).expect("third");
    let distributed = distribute(
        handle,
        DistributeRequest { ids: vec!["entity-1".to_string(), "entity-2".to_string(), "entity-3".to_string()], axis: DistributeAxis::X },
    )
    .expect("distribute");
    assert_eq!(distributed["revision"], json!(8));
    let undone = undo(handle).expect("undo the distribute");
    assert_eq!(undone["revision"], json!(9));
    assert_eq!(
        drawing_of(handle).entities[1].kind,
        EntityKind::Line { from: [0.0, 5.0], to: [4.0, 5.0] },
        "one undo reverses the whole distribute"
    );
    close(handle).expect("close");
}

#[test]
fn align_and_distribute_survive_save_and_reopen() {
    let dir = TempDir::new("align-distribute-round-trip");
    let path = dir.path.join("plan.ocad");
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [2.0, 0.0]).expect("first");
    add_line(handle, DEFAULT_LAYER, [3.0, 0.0], [9.0, 0.0]).expect("middle");
    add_line(handle, DEFAULT_LAYER, [20.0, 0.0], [24.0, 0.0]).expect("last");

    align(handle, AlignRequest { ids: vec!["entity-1".to_string(), "entity-2".to_string()], mode: AlignMode::Left }).expect("align");
    distribute(
        handle,
        DistributeRequest { ids: vec!["entity-1".to_string(), "entity-2".to_string(), "entity-3".to_string()], axis: DistributeAxis::X },
    )
    .expect("distribute");
    save(handle, &path, true).expect("save");
    close(handle).expect("close");

    let opened = open(&path).expect("open");
    let reopened = handle_of(&opened);
    let drawing = drawing_of(reopened);
    assert_eq!(drawing.entities[0].kind, EntityKind::Line { from: [0.0, 0.0], to: [2.0, 0.0] });
    assert_eq!(drawing.entities[1].kind, EntityKind::Line { from: [8.0, 0.0], to: [14.0, 0.0] }, "the distributed gap survives the round trip");
    assert_eq!(drawing.entities[2].kind, EntityKind::Line { from: [20.0, 0.0], to: [24.0, 0.0] });
    close(reopened).expect("close reopened");
}

#[test]
fn align_is_deterministic() {
    let run = |mode: AlignMode| {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("first");
        add_line(handle, DEFAULT_LAYER, [5.0, 5.0], [9.0, 5.0]).expect("second");
        add_circle(handle, DEFAULT_LAYER, [20.0, 0.0], 2.0).expect("third");
        let result = align(handle, AlignRequest { ids: vec!["entity-1".to_string(), "entity-2".to_string(), "entity-3".to_string()], mode })
            .expect("align");
        close(handle).expect("close");
        result
    };

    for mode in [AlignMode::Left, AlignMode::Center, AlignMode::Right, AlignMode::Top, AlignMode::Middle, AlignMode::Bottom] {
        let first = run(mode);
        let second = run(mode);
        assert_eq!(first["bounds"], second["bounds"], "mode {mode:?}");
        assert_eq!(first["revision"], second["revision"], "mode {mode:?}");
        assert_eq!(first["affected"], second["affected"], "mode {mode:?}");
    }
}

#[test]
fn overkill_removes_exact_duplicates_and_counts_skipped() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [5.0, 0.0]).expect("first");
    add_line(handle, DEFAULT_LAYER, [5.0, 0.0], [0.0, 0.0]).expect("reversed twin");
    add_circle(handle, DEFAULT_LAYER, [10.0, 0.0], 2.0).expect("circle");
    add_circle(handle, DEFAULT_LAYER, [10.0, 0.0], 2.0).expect("circle twin");
    add_text(handle, DEFAULT_LAYER, [1.0, 1.0], 2.0, "note").expect("text");
    add_text(handle, DEFAULT_LAYER, [1.0, 1.0], 2.0, "note").expect("text twin");

    let result = overkill(handle, OverkillRequest::default()).expect("overkill");
    assert_eq!(result["removed"], json!(["entity-2", "entity-4"]), "a reversed line and a circle twin are duplicates");
    assert_eq!(result["merged"], json!([]));
    assert_eq!(result["skipped"], json!(2), "the texts are counted and never touched");
    assert_eq!(result["before"], json!(6));
    assert_eq!(result["after"], json!(4));
    assert_eq!(result["revision"], json!(8), "one cleanup is one revision bump");
    let drawing = drawing_of(handle);
    assert_eq!(drawing.entities.len(), 4);
    assert!(drawing.entities.iter().any(|entity| matches!(entity.kind, EntityKind::Text { .. })), "the texts survive");
    close(handle).expect("close");
}

#[test]
fn overkill_merges_overlapping_and_touching_lines() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [5.0, 0.0]).expect("first");
    add_line(handle, DEFAULT_LAYER, [3.0, 0.0], [10.0, 0.0]).expect("overlapping");
    add_line(handle, DEFAULT_LAYER, [10.0, 0.0], [14.0, 0.0]).expect("touching");
    add_line(handle, DEFAULT_LAYER, [0.0, 5.0], [5.0, 5.0]).expect("parallel");

    let result = overkill(handle, OverkillRequest { tolerance: None, merge_overlapping: Some(true) }).expect("overkill merge");
    assert_eq!(result["removed"], json!([]));
    let merged = result["merged"].as_array().expect("merged").clone();
    assert_eq!(merged.len(), 1, "the three collinear spans form one union");
    assert_eq!(merged[0]["sources"], json!(["entity-1", "entity-2", "entity-3"]));
    assert_eq!(merged[0]["created"], json!("entity-5"));
    assert_eq!(result["before"], json!(4));
    assert_eq!(result["after"], json!(2));
    let drawing = drawing_of(handle);
    assert_eq!(drawing.entities[0].id, "entity-5");
    assert_eq!(drawing.entities[0].kind, EntityKind::Line { from: [0.0, 0.0], to: [14.0, 0.0] }, "the union keeps the outer endpoints");
    assert_eq!(drawing.entities[1].id, "entity-4", "the parallel line stays");
    close(handle).expect("close");
}

#[test]
fn overkill_leaves_a_clean_drawing_untouched() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [5.0, 0.0]).expect("first");
    add_line(handle, DEFAULT_LAYER, [6.0, 0.0], [10.0, 0.0]).expect("second");

    let result = overkill(handle, OverkillRequest { tolerance: None, merge_overlapping: Some(true) }).expect("clean overkill");
    assert_eq!(result["removed"], json!([]));
    assert_eq!(result["merged"], json!([]));
    assert_eq!(result["before"], json!(2));
    assert_eq!(result["after"], json!(2));
    assert_eq!(result["revision"], json!(3), "a no-op cleanup does not bump the revision");
    undo(handle).expect("undo the second line");
    undo(handle).expect("undo the first line");
    assert_eq!(undo(handle).expect_err("nothing to undo").code, "drawing/nothing-to-undo", "a no-op records no history step");
    close(handle).expect("close");
}

#[test]
fn overkill_honours_layers_and_tolerance() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_layer(handle, "Structure", Some("#ff0000")).expect("layer");
    add_circle(handle, DEFAULT_LAYER, [0.0, 0.0], 2.0).expect("circle");
    add_circle(handle, "Structure", [0.0, 0.0], 2.0).expect("cross-layer twin");
    add_line(handle, DEFAULT_LAYER, [0.0, 5.0], [5.0, 5.0]).expect("line");
    add_line(handle, DEFAULT_LAYER, [0.05, 5.0], [5.0, 5.0]).expect("near twin");

    let strict = overkill(handle, OverkillRequest { tolerance: Some(0.001), merge_overlapping: None }).expect("strict");
    assert_eq!(strict["removed"], json!([]), "cross-layer twins and near twins survive a tight tolerance");
    assert_eq!(strict["revision"], json!(6), "the strict pass is a no-op");

    let loose = overkill(handle, OverkillRequest { tolerance: Some(0.1), merge_overlapping: None }).expect("loose");
    assert_eq!(loose["removed"], json!(["entity-4"]), "the near twin falls inside the loose tolerance");
    assert_eq!(loose["after"], json!(3));
    close(handle).expect("close");
}

#[test]
fn overkill_refuses_locked_layers() {
    let duplicates = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&duplicates);
    add_layer(handle, "Structure", Some("#ff0000")).expect("layer");
    add_line(handle, "Structure", [0.0, 0.0], [5.0, 0.0]).expect("first");
    add_line(handle, "Structure", [0.0, 0.0], [5.0, 0.0]).expect("twin");
    update_layer(handle, "Structure", LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock");
    assert_eq!(overkill(handle, OverkillRequest::default()).expect_err("locked duplicate").code, "drawing/layer-locked");
    assert_eq!(drawing_of(handle).entities.len(), 2, "a refused cleanup leaves the drawing untouched");
    close(handle).expect("close");

    let overlaps = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&overlaps);
    add_layer(handle, "Structure", Some("#ff0000")).expect("layer");
    add_line(handle, "Structure", [0.0, 0.0], [5.0, 0.0]).expect("first");
    add_line(handle, "Structure", [3.0, 0.0], [10.0, 0.0]).expect("overlap");
    update_layer(handle, "Structure", LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock");
    assert_eq!(
        overkill(handle, OverkillRequest { tolerance: None, merge_overlapping: Some(true) }).expect_err("locked merge").code,
        "drawing/layer-locked"
    );
    close(handle).expect("close");
}

#[test]
fn overkill_rejects_a_bad_tolerance() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    assert_eq!(
        overkill(handle, OverkillRequest { tolerance: Some(-1.0), merge_overlapping: None }).expect_err("negative").code,
        "session/argument-invalid"
    );
    close(handle).expect("close");
}

#[test]
fn overkill_is_a_single_history_step() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [5.0, 0.0]).expect("first");
    add_line(handle, DEFAULT_LAYER, [5.0, 0.0], [0.0, 0.0]).expect("duplicate twin");
    add_line(handle, DEFAULT_LAYER, [3.0, 0.0], [10.0, 0.0]).expect("overlap");

    let result = overkill(handle, OverkillRequest { tolerance: None, merge_overlapping: Some(true) }).expect("overkill");
    assert_eq!(result["removed"], json!(["entity-2"]));
    assert_eq!(result["merged"][0]["created"], json!("entity-4"));
    assert_eq!(result["revision"], json!(5));

    let undone = undo(handle).expect("undo the cleanup");
    assert_eq!(undone["revision"], json!(6));
    assert_eq!(drawing_of(handle).entities.len(), 3, "one undo restores the duplicate and both sources");

    let redone = redo(handle).expect("redo the cleanup");
    assert_eq!(redone["revision"], json!(7));
    assert_eq!(drawing_of(handle).entities.len(), 1, "the duplicate is gone and the overlap is one union line");
    close(handle).expect("close");
}

#[test]
fn overkill_is_deterministic() {
    let run = || {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [5.0, 0.0]).expect("first");
        add_line(handle, DEFAULT_LAYER, [5.0, 0.0], [0.0, 0.0]).expect("twin");
        add_line(handle, DEFAULT_LAYER, [3.0, 0.0], [10.0, 0.0]).expect("overlap");
        let result = overkill(handle, OverkillRequest { tolerance: None, merge_overlapping: Some(true) }).expect("overkill");
        close(handle).expect("close");
        result
    };

    let first = run();
    let second = run();
    assert_eq!(first["removed"], second["removed"]);
    assert_eq!(first["merged"], second["merged"]);
    assert_eq!(first["revision"], second["revision"]);
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

    let summary = stats(handle, StatsGroup::Layer).expect("stats");
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
fn stats_groups_entities_by_type_colour_and_linetype() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_layer(handle, "Walls", Some("#ff0000")).expect("layer");
    add_line(handle, "Walls", [0.0, 0.0], [3.0, 0.0]).expect("line");
    add_circle(handle, DEFAULT_LAYER, [10.0, 10.0], 2.0).expect("circle");
    add_text(handle, DIMENSION_LAYER, [0.0, 0.0], 2.0, "note").expect("text");

    let by_type = stats(handle, StatsGroup::Type).expect("type stats");
    assert_eq!(by_type["groupBy"], json!("type"));
    let groups = by_type["groups"].as_array().expect("groups");
    assert_eq!(
        groups.iter().map(|group| group["key"].as_str().unwrap_or_default()).collect::<Vec<_>>(),
        vec!["circle", "line", "text"],
        "type groups are sorted by name"
    );
    assert_eq!(groups[1]["entityCount"], json!(1));
    assert_eq!(groups[1]["length"], json!(3.0));
    assert!(groups[1]["label"].is_string());

    let by_colour = stats(handle, StatsGroup::Color).expect("colour stats");
    let colours = by_colour["groups"].as_array().expect("colour groups");
    assert_eq!(
        colours.iter().map(|group| group["key"].as_str().unwrap_or_default()).collect::<Vec<_>>(),
        vec!["#0b7285", "#d0d7de", "#ff0000"]
    );
    assert_eq!(colours[2]["entityCount"], json!(1));
    assert_eq!(colours[2]["length"], json!(3.0));

    let by_linetype = stats(handle, StatsGroup::Linetype).expect("linetype stats");
    let linetypes = by_linetype["groups"].as_array().expect("linetype groups");
    assert_eq!(linetypes.len(), 1, "every default layer uses `continuous`");
    assert_eq!(linetypes[0]["key"], json!("continuous"));
    assert_eq!(linetypes[0]["entityCount"], json!(3));
    assert_eq!(by_linetype["layers"].as_array().map(Vec::len), Some(3), "the per-layer view stays available");
    close(handle).expect("close");
}

#[test]
fn quantity_table_draws_a_framed_table_with_totals() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_layer(handle, "Walls", Some("#ff0000")).expect("layer");
    add_line(handle, "Walls", [0.0, 0.0], [3.0, 0.0]).expect("line");
    add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]], true).expect("square");
    let before = summary(handle).expect("summary")["revision"].as_u64().unwrap_or_default();

    let result = add_quantity_table(handle, QuantityTableRequest::default()).expect("table");
    assert_eq!(result["groupBy"], json!("layer"));
    assert_eq!(result["revision"], json!(before + 1));
    let rows = result["rows"].as_array().expect("rows");
    assert_eq!(rows.len(), 3, "two default layers plus Walls");
    assert_eq!(rows[0]["label"], json!("0"));
    assert_eq!(rows[0]["entityCount"], json!(1));
    assert_eq!(rows[0]["length"], json!(16.0));
    assert_eq!(rows[1]["entityCount"], json!(0), "the empty dimensions layer is listed");
    assert_eq!(rows[2]["label"], json!("Walls"));
    assert_eq!(rows[2]["length"], json!(3.0));
    assert_eq!(result["totals"]["entities"], json!(2), "the table never counts itself");
    assert_eq!(result["totals"]["length"], json!(19.0));
    assert_eq!(result["totals"]["area"], json!(16.0));
    let entities = result["created"].as_array().expect("created");
    assert_eq!(entities[0]["kind"]["type"], json!("line"));
    assert_eq!(entities[0]["kind"]["from"], json!([0.0, 0.0]));
    assert_eq!(entities[0]["kind"]["to"], json!([210.0, 0.0]), "width is 60 * textHeight");
    assert_eq!(entities[1]["kind"]["to"], json!([210.0, 52.5]));
    assert!(entities.iter().all(|entity| entity["layer"] == json!("quantity")));
    let contents = entities.iter().filter_map(|entity| entity["kind"]["content"].as_str()).collect::<Vec<_>>();
    assert!(contents.contains(&"Quantity take-off"));
    assert!(contents.contains(&"Group"));
    assert!(contents.contains(&"Total"));
    assert!(contents.contains(&"19"));
    close(handle).expect("close");
}

#[test]
fn quantity_table_honours_origin_caption_and_type_grouping() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [2.0, 0.0]).expect("line");
    add_circle(handle, DEFAULT_LAYER, [0.0, 0.0], 1.0).expect("circle");
    let request = QuantityTableRequest {
        layer: Some("schedule".to_string()),
        group_by: StatsGroup::Type,
        origin: Some([100.0, 200.0]),
        text_height: Some(2.0),
        title: Some("Bill of quantities".to_string()),
    };
    let result = add_quantity_table(handle, request).expect("table");
    assert_eq!(result["groupBy"], json!("type"));
    assert_eq!(result["columns"], json!(["Group", "Count", "Length", "Area"]));
    let rows = result["rows"].as_array().expect("rows");
    assert_eq!(rows.iter().map(|row| row["label"].as_str().unwrap_or_default()).collect::<Vec<_>>(), vec!["circle", "line"]);
    let entities = result["created"].as_array().expect("created");
    assert_eq!(entities[0]["kind"]["from"], json!([100.0, 200.0]));
    assert_eq!(entities[0]["kind"]["to"], json!([220.0, 200.0]), "width is 60 * textHeight");
    assert!(entities.iter().all(|entity| entity["layer"] == json!("schedule")));
    let contents = entities.iter().filter_map(|entity| entity["kind"]["content"].as_str()).collect::<Vec<_>>();
    assert!(contents.contains(&"Bill of quantities"));
    assert!(contents.contains(&"circle"));
    close(handle).expect("close");
}

#[test]
fn quantity_table_rejects_bad_requests_and_protected_layers() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [1.0, 0.0]).expect("line");

    let bad_height = add_quantity_table(handle, QuantityTableRequest { text_height: Some(0.0), ..QuantityTableRequest::default() })
        .expect_err("zero height");
    assert_eq!(bad_height.code, "session/argument-invalid");
    let bad_origin = add_quantity_table(handle, QuantityTableRequest { origin: Some([f64::NAN, 0.0]), ..QuantityTableRequest::default() })
        .expect_err("non-finite origin");
    assert_eq!(bad_origin.code, "session/argument-invalid");

    add_layer(handle, "locked", None).expect("layer");
    update_layer(handle, "locked", LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock");
    let locked = add_quantity_table(handle, QuantityTableRequest { layer: Some("locked".to_string()), ..QuantityTableRequest::default() })
        .expect_err("locked layer");
    assert_eq!(locked.code, "drawing/layer-locked");

    add_layer(handle, "frozen", None).expect("layer");
    update_layer(handle, "frozen", LayerUpdate { frozen: Some(true), ..LayerUpdate::default() }).expect("freeze");
    let frozen = add_quantity_table(handle, QuantityTableRequest { layer: Some("frozen".to_string()), ..QuantityTableRequest::default() })
        .expect_err("frozen layer");
    assert_eq!(frozen.code, "drawing/layer-frozen");
    close(handle).expect("close");
}

#[test]
fn quantity_table_is_deterministic() {
    let run = || {
        let created = create(CreateRequest::default()).expect("create");
        let handle = handle_of(&created);
        add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [3.0, 0.0]).expect("line");
        add_circle(handle, DEFAULT_LAYER, [0.0, 0.0], 1.0).expect("circle");
        let result = add_quantity_table(handle, QuantityTableRequest::default()).expect("table");
        close(handle).expect("close");
        result
    };
    let first = run();
    let second = run();
    assert_eq!(first["created"], second["created"]);
    assert_eq!(first["rows"], second["rows"]);
    assert_eq!(first["totals"], second["totals"]);
}

#[test]
fn empty_layer_check_is_opt_in_and_warns_per_layer() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [1.0, 0.0]).expect("line");

    let default_report = inspect(handle, None).expect("default inspect");
    let default_codes =
        default_report["issues"].as_array().expect("issues").iter().map(|issue| issue["code"].as_str().unwrap_or_default()).collect::<Vec<_>>();
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

    let renamed = update_layout(handle, "Sheet", LayoutUpdate { name: Some("Site".to_string()), ..LayoutUpdate::default() }).expect("rename");
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

fn created_text(created: &Value) -> Vec<String> {
    created
        .as_array()
        .expect("created")
        .iter()
        .filter(|entity| entity["kind"]["type"] == json!("text"))
        .filter_map(|entity| entity["kind"]["content"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn hatch_pattern_fills_a_closed_boundary_with_lines() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_layer(handle, "Hatch", Some("#00ff00")).expect("layer");
    let hatch =
        add_hatch(handle, "Hatch", vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]], Some("ANSI31".to_string())).expect("hatch");
    let id = hatch["entity"]["id"].as_str().expect("id").to_string();

    let result = hatch_pattern(handle, HatchPatternRequest { id, angle: None, spacing: None, cross: None }).expect("hatch pattern");
    assert_eq!(result["revision"], json!(4), "one fill is one revision");
    let lines = result["created"].as_array().expect("created");
    assert_eq!(lines.len(), 1, "a 10x10 boundary with spacing 5 keeps one interior scanline");
    for line in lines {
        assert_eq!(line["kind"]["type"], json!("line"));
        assert_eq!(line["layer"], json!("Hatch"), "the pattern stays on the hatch layer");
        assert_eq!(line["kind"]["from"][1], line["kind"]["to"][1], "the default angle keeps pattern lines horizontal");
    }
    let drawing = drawing_of(handle);
    assert_eq!(drawing.entities.len(), 2, "the hatch boundary survives");
    assert_eq!(
        drawing.entities[0].kind,
        EntityKind::Hatch { points: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]], pattern: "ANSI31".to_string() }
    );
    close(handle).expect("close");
}

#[test]
fn hatch_pattern_honours_angle_spacing_and_cross() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_layer(handle, "Hatch", Some("#00ff00")).expect("layer");
    let hatch =
        add_hatch(handle, "Hatch", vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]], Some("ANSI31".to_string())).expect("hatch");
    let id = hatch["entity"]["id"].as_str().expect("id").to_string();

    let spaced =
        hatch_pattern(handle, HatchPatternRequest { id: id.clone(), angle: Some(0.0), spacing: Some(2.5), cross: None }).expect("spacing 2.5");
    assert_eq!(spaced["created"].as_array().map(Vec::len), Some(3), "three interior scanlines at spacing 2.5");

    let crossed = hatch_pattern(handle, HatchPatternRequest { id, angle: Some(0.0), spacing: Some(5.0), cross: Some(true) }).expect("cross");
    let crossed_lines = crossed["created"].as_array().expect("created");
    assert_eq!(crossed_lines.len(), 2, "cross adds a perpendicular family");
    let horizontal = crossed_lines.iter().filter(|line| line["kind"]["from"][1] == line["kind"]["to"][1]).count();
    assert_eq!(horizontal, 1, "one horizontal pattern line");
    assert_eq!(crossed_lines.len() - horizontal, 1, "one vertical pattern line");
    close(handle).expect("close");
}

#[test]
fn hatch_pattern_rejects_bad_requests_and_protected_layers() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_layer(handle, "Hatch", Some("#00ff00")).expect("layer");
    let hatch =
        add_hatch(handle, "Hatch", vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]], Some("ANSI31".to_string())).expect("hatch");
    let id = hatch["entity"]["id"].as_str().expect("id").to_string();

    assert_eq!(
        hatch_pattern(handle, HatchPatternRequest { id: id.clone(), angle: Some(0.0), spacing: Some(0.0), cross: None })
            .expect_err("zero spacing")
            .code,
        "hatch/invalid-spacing"
    );
    assert_eq!(
        hatch_pattern(handle, HatchPatternRequest { id: id.clone(), angle: Some(f64::INFINITY), spacing: None, cross: None })
            .expect_err("infinite angle")
            .code,
        "hatch/invalid-angle"
    );
    assert_eq!(
        hatch_pattern(handle, HatchPatternRequest { id: id.clone(), angle: None, spacing: Some(1e-9), cross: None })
            .expect_err("too fine")
            .code,
        "hatch/spacing-too-fine"
    );
    assert_eq!(
        hatch_pattern(handle, HatchPatternRequest { id: "entity-99".to_string(), angle: None, spacing: None, cross: None })
            .expect_err("unknown hatch")
            .code,
        "object/not-found"
    );
    let line = add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [1.0, 0.0]).expect("line");
    let line_id = line["entity"]["id"].as_str().expect("id").to_string();
    assert_eq!(
        hatch_pattern(handle, HatchPatternRequest { id: line_id, angle: None, spacing: None, cross: None }).expect_err("not a hatch").code,
        "hatch/unsupported-entity"
    );

    let solid = add_hatch(handle, "Hatch", vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], None).expect("solid hatch");
    let solid_id = solid["entity"]["id"].as_str().expect("id").to_string();
    assert_eq!(
        hatch_pattern(handle, HatchPatternRequest { id: solid_id, angle: None, spacing: None, cross: None }).expect_err("solid pattern").code,
        "hatch/solid-pattern"
    );

    let degenerate =
        add_hatch(handle, "Hatch", vec![[0.0, 0.0], [5.0, 0.0], [10.0, 0.0]], Some("ANSI31".to_string())).expect("degenerate hatch");
    let degenerate_id = degenerate["entity"]["id"].as_str().expect("id").to_string();
    assert_eq!(
        hatch_pattern(handle, HatchPatternRequest { id: degenerate_id, angle: None, spacing: None, cross: None }).expect_err("zero area").code,
        "hatch/degenerate-boundary"
    );

    update_layer(handle, "Hatch", LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock");
    assert_eq!(
        hatch_pattern(handle, HatchPatternRequest { id: id.clone(), angle: None, spacing: None, cross: None }).expect_err("locked layer").code,
        "drawing/layer-locked"
    );
    update_layer(handle, "Hatch", LayerUpdate { locked: Some(false), frozen: Some(true), ..LayerUpdate::default() }).expect("freeze");
    assert_eq!(
        hatch_pattern(handle, HatchPatternRequest { id, angle: None, spacing: None, cross: None }).expect_err("frozen layer").code,
        "drawing/layer-frozen"
    );
    assert_eq!(drawing_of(handle).entities.len(), 4, "refused requests never add entities");
    close(handle).expect("close");
}

#[test]
fn title_block_generates_a_framed_sheet_with_defaults() {
    let created = create(CreateRequest { title: Some("Site plan".to_string()), ..CreateRequest::default() }).expect("create");
    let handle = handle_of(&created);
    update_properties(handle, PropertiesUpdate { author: Some("Ada".to_string()), ..PropertiesUpdate::default() }).expect("author");

    let result = add_title_block(handle, TitleBlockRequest::default()).expect("title block");
    assert_eq!(result["revision"], json!(3), "one block is one revision");
    let created = result["created"].as_array().expect("created");
    assert_eq!(created.len(), 32, "19 grid lines and 13 default texts");
    let lines = created.iter().filter(|entity| entity["kind"]["type"] == json!("line")).count();
    assert_eq!(lines, 19, "outer border, inner frame, block frame, six row separators and one label divider");
    let texts = created_text(&result["created"]);
    for expected in ["Title", "Number", "Revision", "Scale", "Date", "Author", "Sheet", "Site plan", "drawing-1", "1:1", "Ada"] {
        assert!(texts.contains(&expected.to_string()), "missing `{expected}` in {texts:?}");
    }
    let drawing = drawing_of(handle);
    let layer = drawing.layer("titleblock").expect("titleblock layer");
    assert_eq!(layer.color, "#d0d7de");
    assert_eq!(drawing.entities.iter().filter(|entity| entity.layer == "titleblock").count(), created.len());
    let sheet = add_title_block(handle, TitleBlockRequest::default()).expect("second block");
    assert_eq!(sheet["created"].as_array().map(Vec::len), Some(created.len()), "the default sheet is deterministic");
    close(handle).expect("close");
}

#[test]
fn title_block_honours_overrides_and_a_custom_sheet() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    let result = add_title_block(
        handle,
        TitleBlockRequest {
            layer: Some("Sheet".to_string()),
            width: Some(200.0),
            height: Some(150.0),
            margin: Some(8.0),
            text_height: Some(2.5),
            title: Some("Pump skid".to_string()),
            number: Some("A-101".to_string()),
            revision: Some("C".to_string()),
            scale: Some("1:50".to_string()),
            date: Some("2026-11-23".to_string()),
            author: Some("Lin".to_string()),
            sheet: Some("2".to_string()),
        },
    )
    .expect("custom block");
    let texts = created_text(&result["created"]);
    for expected in ["Pump skid", "A-101", "C", "1:50", "2026-11-23", "Lin", "2"] {
        assert!(texts.contains(&expected.to_string()), "missing `{expected}` in {texts:?}");
    }
    let drawing = drawing_of(handle);
    assert!(drawing.layer("Sheet").is_some());
    assert!(drawing.entities.iter().all(|entity| entity.layer == "Sheet"));
    close(handle).expect("close");
}

#[test]
fn title_block_rejects_bad_sheets_and_protected_layers() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    assert_eq!(
        add_title_block(handle, TitleBlockRequest { width: Some(0.0), ..TitleBlockRequest::default() }).expect_err("zero width").code,
        "session/argument-invalid"
    );
    assert_eq!(
        add_title_block(handle, TitleBlockRequest { text_height: Some(f64::NAN), ..TitleBlockRequest::default() }).expect_err("nan text").code,
        "session/argument-invalid"
    );
    assert_eq!(
        add_title_block(
            handle,
            TitleBlockRequest { width: Some(100.0), height: Some(60.0), text_height: Some(10.0), ..TitleBlockRequest::default() }
        )
        .expect_err("tiny sheet")
        .code,
        "titleblock/invalid-sheet"
    );
    assert_eq!(
        add_title_block(handle, TitleBlockRequest { title: Some("x".repeat(PROPERTY_VALUE_LIMIT + 1)), ..TitleBlockRequest::default() })
            .expect_err("long title")
            .code,
        "session/argument-invalid"
    );

    add_layer(handle, "titleblock", Some("#ffffff")).expect("layer");
    update_layer(handle, "titleblock", LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock");
    assert_eq!(add_title_block(handle, TitleBlockRequest::default()).expect_err("locked layer").code, "drawing/layer-locked");
    update_layer(handle, "titleblock", LayerUpdate { locked: Some(false), frozen: Some(true), ..LayerUpdate::default() }).expect("freeze");
    assert_eq!(add_title_block(handle, TitleBlockRequest::default()).expect_err("frozen layer").code, "drawing/layer-frozen");
    assert_eq!(drawing_of(handle).entities.len(), 0, "refused requests never add entities");
    close(handle).expect("close");
}

#[test]
fn smooth_polyline_rounds_an_open_chain_and_keeps_its_identity() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_layer(handle, "Outline", Some("#ff0000")).expect("layer");
    let polyline = add_polyline(handle, "Outline", vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]], false).expect("polyline");
    let id = polyline["entity"]["id"].as_str().expect("id").to_string();

    let result = smooth_polyline(handle, SmoothPolylineRequest { id: id.clone(), iterations: Some(1), ratio: None }).expect("smooth");
    assert_eq!(result["revision"], json!(4), "one smoothing pass is one revision");
    let entity = &result["entity"];
    assert_eq!(entity["id"], json!(id));
    assert_eq!(entity["layer"], json!("Outline"), "the entity stays on its layer");
    assert_eq!(entity["kind"]["closed"], json!(false));
    let points = entity["kind"]["points"].as_array().expect("points");
    let expected = [[0.0, 0.0], [2.5, 0.0], [7.5, 0.0], [10.0, 2.5], [10.0, 7.5], [10.0, 10.0]];
    assert_eq!(points.len(), expected.len(), "one pass replaces every segment with two points");
    for (point, expected) in points.iter().zip(expected) {
        assert_point(point, expected);
    }

    let second = create(CreateRequest::default()).expect("second create");
    let second_handle = handle_of(&second);
    add_layer(second_handle, "Outline", Some("#ff0000")).expect("second layer");
    let second_polyline = add_polyline(second_handle, "Outline", vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]], false).expect("second polyline");
    let second_id = second_polyline["entity"]["id"].as_str().expect("id").to_string();
    let again =
        smooth_polyline(second_handle, SmoothPolylineRequest { id: second_id, iterations: Some(1), ratio: None }).expect("second smooth");
    assert_eq!(again["entity"], *entity, "the same input yields the same vertices");
    close(second_handle).expect("close second");
    close(handle).expect("close");
}

#[test]
fn smooth_polyline_keeps_a_closed_chain_closed_without_duplicating_the_first_vertex() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    let polyline = add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]], true).expect("polyline");
    let id = polyline["entity"]["id"].as_str().expect("id").to_string();

    let result = smooth_polyline(handle, SmoothPolylineRequest { id, iterations: Some(1), ratio: Some(0.25) }).expect("smooth");
    let points = result["entity"]["kind"]["points"].as_array().expect("points");
    let expected = [[2.5, 0.0], [7.5, 0.0], [10.0, 2.5], [10.0, 7.5], [7.5, 10.0], [2.5, 10.0], [0.0, 7.5], [0.0, 2.5]];
    assert_eq!(points.len(), expected.len(), "a closed chain keeps two points per edge and no fixed vertex");
    for (point, expected) in points.iter().zip(expected) {
        assert_point(point, expected);
    }
    assert_eq!(result["entity"]["kind"]["closed"], json!(true));
    close(handle).expect("close");
}

#[test]
fn smooth_polyline_rejects_bad_requests_and_protected_layers() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    let polyline = add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]], false).expect("polyline");
    let id = polyline["entity"]["id"].as_str().expect("id").to_string();
    let line = add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [1.0, 0.0]).expect("line");
    let line_id = line["entity"]["id"].as_str().expect("id").to_string();
    let short = add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [1.0, 0.0]], false).expect("short polyline");
    let short_id = short["entity"]["id"].as_str().expect("id").to_string();

    assert_eq!(
        smooth_polyline(handle, SmoothPolylineRequest { id: "entity-99".to_string(), ..SmoothPolylineRequest::default() })
            .expect_err("unknown id")
            .code,
        "object/not-found"
    );
    assert_eq!(
        smooth_polyline(handle, SmoothPolylineRequest { id: line_id, iterations: None, ratio: None }).expect_err("not a polyline").code,
        "polyline/unsupported-entity"
    );
    assert_eq!(
        smooth_polyline(handle, SmoothPolylineRequest { id: short_id, iterations: None, ratio: None }).expect_err("too few vertices").code,
        "polyline/degenerate"
    );
    assert_eq!(
        smooth_polyline(handle, SmoothPolylineRequest { id: id.clone(), iterations: Some(0), ratio: None }).expect_err("zero iterations").code,
        "polyline/invalid-iterations"
    );
    assert_eq!(
        smooth_polyline(handle, SmoothPolylineRequest { id: id.clone(), iterations: Some(9), ratio: None })
            .expect_err("too many iterations")
            .code,
        "polyline/invalid-iterations"
    );
    for ratio in [0.0, 0.5, f64::NAN, f64::INFINITY] {
        assert_eq!(
            smooth_polyline(handle, SmoothPolylineRequest { id: id.clone(), iterations: None, ratio: Some(ratio) })
                .expect_err("bad ratio")
                .code,
            "polyline/invalid-ratio"
        );
    }

    update_layer(handle, DEFAULT_LAYER, LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock");
    assert_eq!(
        smooth_polyline(handle, SmoothPolylineRequest { id: id.clone(), iterations: None, ratio: None }).expect_err("locked layer").code,
        "drawing/layer-locked"
    );
    update_layer(handle, DEFAULT_LAYER, LayerUpdate { locked: Some(false), frozen: Some(true), ..LayerUpdate::default() }).expect("freeze");
    assert_eq!(
        smooth_polyline(handle, SmoothPolylineRequest { id, iterations: None, ratio: None }).expect_err("frozen layer").code,
        "drawing/layer-frozen"
    );
    close(handle).expect("close");
}

#[test]
fn smooth_polyline_is_one_undoable_history_step() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    let polyline = add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]], false).expect("polyline");
    let id = polyline["entity"]["id"].as_str().expect("id").to_string();
    smooth_polyline(handle, SmoothPolylineRequest { id: id.clone(), iterations: Some(2), ratio: None }).expect("smooth");

    undo(handle).expect("undo");
    let restored = drawing_of(handle);
    assert_eq!(restored.entities.len(), 1);
    assert_eq!(restored.entities[0].kind, EntityKind::Polyline { points: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]], closed: false });
    close(handle).expect("close");
}

#[test]
fn polyline_chamfer_bevels_every_interior_vertex_of_an_open_chain() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_layer(handle, "Outline", Some("#00ff00")).expect("layer");
    let polyline = add_polyline(handle, "Outline", vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]], false).expect("polyline");
    let id = polyline["entity"]["id"].as_str().expect("id").to_string();
    let revision = drawing_of(handle).revision;

    let result = polyline_chamfer(handle, PolylineChamferRequest { id: id.clone(), distance: 2.0 }).expect("chamfer");
    assert_eq!(result["revision"], json!(revision + 1), "one chamfer is one revision");
    let entity = &result["entity"];
    assert_eq!(entity["id"], json!(id), "the entity keeps its id");
    assert_eq!(entity["layer"], json!("Outline"), "the entity keeps its layer");
    assert_eq!(entity["kind"]["closed"], json!(false), "an open chain stays open");
    let points = entity["kind"]["points"].as_array().expect("points");
    let expected = [[0.0, 0.0], [8.0, 0.0], [10.0, 2.0], [10.0, 10.0]];
    assert_eq!(points.len(), expected.len(), "one corner adds two points and removes the corner");
    for (point, expected) in points.iter().zip(expected) {
        assert_point(point, expected);
    }

    let second = create(CreateRequest::default()).expect("second create");
    let second_handle = handle_of(&second);
    add_layer(second_handle, "Outline", Some("#00ff00")).expect("second layer");
    let second_polyline = add_polyline(second_handle, "Outline", vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]], false).expect("second polyline");
    let second_id = second_polyline["entity"]["id"].as_str().expect("id").to_string();
    let again = polyline_chamfer(second_handle, PolylineChamferRequest { id: second_id, distance: 2.0 }).expect("second chamfer");
    assert_eq!(again["entity"], *entity, "the same input yields the same vertices");
    close(second_handle).expect("close second");
    close(handle).expect("close");
}

#[test]
fn polyline_chamfer_bevels_a_closed_chain_including_the_first_corner() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    let polyline = add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]], true).expect("polyline");
    let id = polyline["entity"]["id"].as_str().expect("id").to_string();

    let result = polyline_chamfer(handle, PolylineChamferRequest { id, distance: 2.0 }).expect("chamfer");
    let entity = &result["entity"];
    assert_eq!(entity["kind"]["closed"], json!(true), "a closed chain stays closed");
    let points = entity["kind"]["points"].as_array().expect("points");
    let expected = [[0.0, 2.0], [2.0, 0.0], [8.0, 0.0], [10.0, 2.0], [10.0, 8.0], [8.0, 10.0], [2.0, 10.0], [0.0, 8.0]];
    assert_eq!(points.len(), expected.len(), "every vertex of a closed chain keeps two points");
    for (point, expected) in points.iter().zip(expected) {
        assert_point(point, expected);
    }
    close(handle).expect("close");
}

#[test]
fn polyline_chamfer_rejects_short_segments_and_bad_requests_without_writing() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    let polyline = add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]], false).expect("polyline");
    let id = polyline["entity"]["id"].as_str().expect("id").to_string();
    let line = add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [20.0, 0.0]).expect("line");
    let line_id = line["entity"]["id"].as_str().expect("id").to_string();
    let short = add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [5.0, 0.0], [5.0, 10.0]], false).expect("short polyline");
    let short_id = short["entity"]["id"].as_str().expect("id").to_string();
    let flat = add_polyline(handle, DEFAULT_LAYER, vec![[0.0, 0.0], [10.0, 0.0]], false).expect("flat polyline");
    let flat_id = flat["entity"]["id"].as_str().expect("id").to_string();
    let before = drawing_of(handle);

    assert_eq!(
        polyline_chamfer(handle, PolylineChamferRequest { id: id.clone(), distance: 5.0 }).expect_err("half a segment").code,
        "session/argument-invalid",
        "a distance exactly half a segment cannot be beveled"
    );
    assert_eq!(
        polyline_chamfer(handle, PolylineChamferRequest { id: short_id, distance: 2.5 }).expect_err("short segment").code,
        "session/argument-invalid",
        "a segment equal to 2 * distance cannot be beveled"
    );
    for distance in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(
            polyline_chamfer(handle, PolylineChamferRequest { id: id.clone(), distance }).expect_err("bad distance").code,
            "session/argument-invalid"
        );
    }
    assert_eq!(
        polyline_chamfer(handle, PolylineChamferRequest { id: "entity-99".to_string(), distance: 1.0 }).expect_err("unknown id").code,
        "object/not-found"
    );
    assert_eq!(
        polyline_chamfer(handle, PolylineChamferRequest { id: line_id, distance: 1.0 }).expect_err("not a polyline").code,
        "polyline/unsupported-entity"
    );
    assert_eq!(
        polyline_chamfer(handle, PolylineChamferRequest { id: flat_id, distance: 1.0 }).expect_err("too few vertices").code,
        "polyline/degenerate"
    );
    assert_eq!(drawing_of(handle), before, "a refused chamfer writes nothing");

    update_layer(handle, DEFAULT_LAYER, LayerUpdate { locked: Some(true), ..LayerUpdate::default() }).expect("lock");
    assert_eq!(
        polyline_chamfer(handle, PolylineChamferRequest { id: id.clone(), distance: 1.0 }).expect_err("locked layer").code,
        "drawing/layer-locked"
    );
    update_layer(handle, DEFAULT_LAYER, LayerUpdate { locked: Some(false), frozen: Some(true), ..LayerUpdate::default() }).expect("freeze");
    assert_eq!(polyline_chamfer(handle, PolylineChamferRequest { id, distance: 1.0 }).expect_err("frozen layer").code, "drawing/layer-frozen");
    close(handle).expect("close");
}

#[test]
fn polyline_chamfer_is_one_undoable_step_and_survives_a_round_trip() {
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    let original = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]];
    let polyline = add_polyline(handle, DEFAULT_LAYER, original.clone(), false).expect("polyline");
    let id = polyline["entity"]["id"].as_str().expect("id").to_string();
    let chamfered = polyline_chamfer(handle, PolylineChamferRequest { id: id.clone(), distance: 3.0 }).expect("chamfer");
    assert_eq!(chamfered["entity"]["kind"]["points"].as_array().expect("points").len(), 4);

    undo(handle).expect("undo");
    let restored = drawing_of(handle);
    assert_eq!(restored.entities[0].kind, EntityKind::Polyline { points: original.clone(), closed: false });

    let revision = restored.revision;
    let again = polyline_chamfer(handle, PolylineChamferRequest { id: id.clone(), distance: 3.0 }).expect("chamfer again");
    assert_eq!(again["revision"], json!(revision + 1), "the chamfer bumps the revision once");

    let dir = TempDir::new("polyline-chamfer");
    let path = dir.path.join("bevel.ocad");
    save(handle, &path, true).expect("save");
    close(handle).expect("close");
    let opened = open(&path).expect("open");
    let reopened = handle_of(&opened);
    let drawing = drawing_of(reopened);
    let EntityKind::Polyline { points, closed } = &drawing.entities[0].kind
    else {
        panic!("expected a polyline, got {:?}", drawing.entities[0].kind);
    };
    assert!(!*closed);
    let expected = [[0.0, 0.0], [7.0, 0.0], [10.0, 3.0], [10.0, 10.0]];
    assert_eq!(points.len(), expected.len());
    for (point, expected) in points.iter().zip(expected) {
        assert!((point[0] - expected[0]).abs() < 1e-9 && (point[1] - expected[1]).abs() < 1e-9);
    }
    close(reopened).expect("close reopened");
}

#[test]
fn diff_reports_identical_projects() {
    let dir = TempDir::new("diff-identical");
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("line");
    let left = dir.path.join("left.ocad");
    let right = dir.path.join("right.ocad");
    save(handle, &left, false).expect("save left");
    save(handle, &right, false).expect("save right");

    let report = diff(&left, &right).expect("diff");
    assert_eq!(report["identical"], json!(true));
    assert_eq!(report["summary"]["added"], json!(0));
    assert_eq!(report["summary"]["removed"], json!(0));
    assert_eq!(report["summary"]["changed"], json!(0));
    assert_eq!(report["summary"]["metadata"], json!(0));
    assert_eq!(report["left"]["entities"], json!(1));
    assert_eq!(report["right"]["path"], json!(right.display().to_string()));
    close(handle).expect("close");
}

#[test]
fn diff_lists_added_removed_changed_and_metadata_entries() {
    let dir = TempDir::new("diff-changes");
    let left = dir.path.join("left.ocad");
    let right = dir.path.join("right.ocad");

    let created = create(CreateRequest { title: Some("Left plan".to_string()), ..CreateRequest::default() }).expect("create");
    let handle = handle_of(&created);
    add_line(handle, DEFAULT_LAYER, [0.0, 0.0], [10.0, 0.0]).expect("line");
    let circle = add_circle(handle, DEFAULT_LAYER, [5.0, 5.0], 2.0).expect("circle");
    let circle_id = circle["entity"]["id"].as_str().expect("id").to_string();
    save(handle, &left, false).expect("save left");

    delete_entity(handle, "entity-1").expect("delete line");
    move_entity(handle, &circle_id, Some([1.0, 0.0]), None).expect("move circle");
    add_line(handle, DEFAULT_LAYER, [0.0, 20.0], [10.0, 20.0]).expect("new line");
    update_layer(handle, DEFAULT_LAYER, LayerUpdate { color: Some("#ff0000".to_string()), ..LayerUpdate::default() }).expect("color");
    update_properties(handle, PropertiesUpdate { title: Some("Right plan".to_string()), ..PropertiesUpdate::default() }).expect("title");
    save(handle, &right, false).expect("save right");

    let report = diff(&left, &right).expect("diff");
    assert_eq!(report["identical"], json!(false));
    let summary = &report["summary"];
    assert_eq!(summary["added"], json!(1), "one new line");
    assert_eq!(summary["removed"], json!(1), "the first line is gone");
    assert_eq!(summary["changed"], json!(1), "the circle moved");
    assert_eq!(summary["layersChanged"], json!(1), "layer 0 changed colour");
    assert_eq!(summary["metadata"], json!(1), "the title changed");

    assert_eq!(report["entities"]["added"][0]["kind"]["type"], json!("line"));
    assert_eq!(report["entities"]["removed"][0]["id"], json!("entity-1"));
    let changed = &report["entities"]["changed"][0];
    assert_eq!(changed["id"], json!(circle_id));
    assert_eq!(changed["fields"], json!(["geometry"]));
    assert_eq!(report["layers"]["changed"][0]["fields"], json!(["color"]));
    assert_eq!(report["metadata"][0], json!({ "field": "title", "left": "Left plan", "right": "Right plan" }));
    close(handle).expect("close");
}

#[test]
fn diff_reports_unreadable_and_malformed_projects() {
    let dir = TempDir::new("diff-errors");
    let created = create(CreateRequest::default()).expect("create");
    let handle = handle_of(&created);
    let valid = dir.path.join("valid.ocad");
    save(handle, &valid, false).expect("save");

    assert_eq!(diff(&dir.path.join("missing.ocad"), &valid).expect_err("missing").code, "drawing/not-found");
    let junk = dir.path.join("junk.ocad");
    fs::write(&junk, b"{ not json").expect("write junk");
    assert_eq!(diff(&valid, &junk).expect_err("malformed").code, "format/invalid");
    close(handle).expect("close");
}

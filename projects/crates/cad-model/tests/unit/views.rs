use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

use super::*;
use crate::{CreateRequest, create, store};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn open_drawing() -> u64 {
    let _ = COUNTER.fetch_add(1, Ordering::SeqCst);
    let created = create(CreateRequest::default()).expect("create");
    created["handle"].as_u64().expect("handle")
}

fn created(result: &Value) -> Vec<Value> {
    result["created"].as_array().expect("created").clone()
}

fn kinds(result: &Value) -> Vec<String> {
    created(result).iter().map(|entity| entity["kind"]["type"].as_str().unwrap().to_string()).collect()
}

fn lines_on(result: &Value, layer: &str) -> Vec<([f64; 2], [f64; 2])> {
    created(result)
        .iter()
        .filter(|entity| entity["layer"] == layer && entity["kind"]["type"] == "line")
        .map(|entity| {
            let from = [entity["kind"]["from"][0].as_f64().unwrap(), entity["kind"]["from"][1].as_f64().unwrap()];
            let to = [entity["kind"]["to"][0].as_f64().unwrap(), entity["kind"]["to"][1].as_f64().unwrap()];
            (from, to)
        })
        .collect()
}

fn polylines_on(result: &Value, layer: &str) -> Vec<(Vec<[f64; 2]>, bool)> {
    created(result)
        .iter()
        .filter(|entity| entity["layer"] == layer && entity["kind"]["type"] == "polyline")
        .map(|entity| {
            let points = entity["kind"]["points"]
                .as_array()
                .unwrap()
                .iter()
                .map(|point| [point[0].as_f64().unwrap(), point[1].as_f64().unwrap()])
                .collect();
            let closed = entity["kind"]["closed"].as_bool().unwrap_or(false);
            (points, closed)
        })
        .collect()
}

fn circles_on(result: &Value, layer: &str) -> Vec<([f64; 2], f64)> {
    created(result)
        .iter()
        .filter(|entity| entity["layer"] == layer && entity["kind"]["type"] == "circle")
        .map(|entity| {
            let center = [entity["kind"]["center"][0].as_f64().unwrap(), entity["kind"]["center"][1].as_f64().unwrap()];
            let radius = entity["kind"]["radius"].as_f64().unwrap();
            (center, radius)
        })
        .collect()
}

fn texts_on(result: &Value, layer: &str) -> Vec<(String, [f64; 2])> {
    created(result)
        .iter()
        .filter(|entity| entity["layer"] == layer && entity["kind"]["type"] == "text")
        .map(|entity| {
            let content = entity["kind"]["content"].as_str().unwrap().to_string();
            let at = [entity["kind"]["at"][0].as_f64().unwrap(), entity["kind"]["at"][1].as_f64().unwrap()];
            (content, at)
        })
        .collect()
}

fn assert_close(actual: f64, expected: f64, tolerance: f64, label: &str) {
    assert!((actual - expected).abs() < tolerance, "{label}: expected {expected}, got {actual}");
}

#[test]
fn multiview_sheet_places_box_views_with_shared_centers() {
    let handle = open_drawing();
    let part = PartSpec { solids: vec![Feature3d::Box { min: [0.0, 0.0, 0.0], max: [20.0, 10.0, 30.0] }], voids: Vec::new() };
    let result = add_multiview_sheet(handle, MultiViewSheetRequest { part, ..Default::default() }).expect("sheet");
    let views = &result["views"];
    assert_close(views["front"]["width"].as_f64().unwrap(), 20.0, 1e-9, "front width");
    assert_close(views["front"]["height"].as_f64().unwrap(), 30.0, 1e-9, "front height");
    assert_close(views["top"]["width"].as_f64().unwrap(), 20.0, 1e-9, "top width");
    assert_close(views["top"]["height"].as_f64().unwrap(), 10.0, 1e-9, "top height");
    assert_close(views["left"]["width"].as_f64().unwrap(), 10.0, 1e-9, "left width");
    assert_close(views["left"]["height"].as_f64().unwrap(), 30.0, 1e-9, "left height");
    let front_center = [views["front"]["center"][0].as_f64().unwrap(), views["front"]["center"][1].as_f64().unwrap()];
    let top_center = [views["top"]["center"][0].as_f64().unwrap(), views["top"]["center"][1].as_f64().unwrap()];
    let left_center = [views["left"]["center"][0].as_f64().unwrap(), views["left"]["center"][1].as_f64().unwrap()];
    assert_close(top_center[0], front_center[0], 1e-9, "top/front x alignment");
    assert_close(left_center[1], front_center[1], 1e-9, "left/front z alignment");
    assert!(top_center[1] < front_center[1], "first angle places top below front");
    assert!(left_center[0] > front_center[0], "first angle places left to the right of front");
    let texts = texts_on(&result, "views");
    for label in ["FRONT", "TOP", "LEFT", "ISO", "FIRST ANGLE"] {
        assert!(texts.iter().any(|(content, _)| content == label), "missing label {label}");
    }
    let iso_width = views["iso"]["width"].as_f64().unwrap();
    let iso_height = views["iso"]["height"].as_f64().unwrap();
    assert!(iso_width > 20.0 && iso_width < 30.0, "iso width {iso_width}");
    assert!(iso_height > 40.0 && iso_height < 50.0, "iso height {iso_height}");
    store::read(handle, |session| {
        let _ = session;
    })
    .unwrap();
}

#[test]
fn multiview_sheet_third_angle_places_top_above_front() {
    let handle = open_drawing();
    let part = PartSpec { solids: vec![Feature3d::Box { min: [0.0, 0.0, 0.0], max: [20.0, 10.0, 30.0] }], voids: Vec::new() };
    let result = add_multiview_sheet(handle, MultiViewSheetRequest { part, projection: Some("third".to_string()), ..Default::default() })
        .expect("sheet");
    let views = &result["views"];
    let front_center = [views["front"]["center"][0].as_f64().unwrap(), views["front"]["center"][1].as_f64().unwrap()];
    let top_center = [views["top"]["center"][0].as_f64().unwrap(), views["top"]["center"][1].as_f64().unwrap()];
    let left_center = [views["left"]["center"][0].as_f64().unwrap(), views["left"]["center"][1].as_f64().unwrap()];
    assert!(top_center[1] > front_center[1], "third angle places top above front");
    assert!(left_center[0] < front_center[0], "third angle places left to the left of front");
    let texts = texts_on(&result, "views");
    assert!(texts.iter().any(|(content, _)| content == "THIRD ANGLE"));
    assert_eq!(result["projection"].as_str().unwrap(), "third");
}

#[test]
fn multiview_sheet_cylinder_along_x_projects_to_rectangles() {
    let handle = open_drawing();
    let part = PartSpec {
        solids: vec![Feature3d::Cylinder { axis: Axis3::X, center: [0.0, 5.0], from: 0.0, to: 60.0, radius: 10.0 }],
        voids: Vec::new(),
    };
    let result = add_multiview_sheet(handle, MultiViewSheetRequest { part, ..Default::default() }).expect("sheet");
    let views = &result["views"];
    assert_close(views["front"]["width"].as_f64().unwrap(), 60.0, 1e-9, "front width");
    assert_close(views["front"]["height"].as_f64().unwrap(), 20.0, 1e-9, "front height");
    assert_close(views["top"]["width"].as_f64().unwrap(), 60.0, 1e-9, "top width");
    assert_close(views["top"]["height"].as_f64().unwrap(), 20.0, 1e-9, "top height");
    assert_close(views["left"]["width"].as_f64().unwrap(), 20.0, 1e-9, "left width");
    assert_close(views["left"]["height"].as_f64().unwrap(), 20.0, 1e-9, "left height");
    let circles = circles_on(&result, "views");
    let part_circles: Vec<_> = circles.iter().filter(|(_, radius)| *radius > 5.0).collect();
    assert_eq!(part_circles.len(), 1, "one circle (left view); the symbol carries only small circles");
    assert_close(part_circles[0].1, 10.0, 1e-9, "left view radius");
}

#[test]
fn multiview_sheet_creates_hidden_layer_for_voids() {
    let handle = open_drawing();
    let part = PartSpec {
        solids: vec![Feature3d::Box { min: [0.0, 0.0, 0.0], max: [40.0, 40.0, 40.0] }],
        voids: vec![Feature3d::Cylinder { axis: Axis3::Z, center: [20.0, 20.0], from: 0.0, to: 40.0, radius: 10.0 }],
    };
    let result = add_multiview_sheet(handle, MultiViewSheetRequest { part, ..Default::default() }).expect("sheet");
    let hidden_polys = polylines_on(&result, "hidden");
    let hidden_circles = circles_on(&result, "hidden");
    assert!(!hidden_polys.is_empty() || !hidden_circles.is_empty(), "void outlines on hidden layer");
    let drawing = store::read(handle, |session| session.drawing.clone()).unwrap();
    assert!(drawing.linetypes.iter().any(|lt| lt.id == "hidden"));
    assert!(drawing.layer("hidden").is_some());
}

#[test]
fn multiview_sheet_is_deterministic() {
    let part = PartSpec { solids: vec![Feature3d::Box { min: [0.0, 0.0, 0.0], max: [20.0, 10.0, 30.0] }], voids: Vec::new() };
    let first = add_multiview_sheet(open_drawing(), MultiViewSheetRequest { part: part.clone(), ..Default::default() }).expect("first");
    let second = add_multiview_sheet(open_drawing(), MultiViewSheetRequest { part, ..Default::default() }).expect("second");
    assert_eq!(first["views"], second["views"]);
    assert_eq!(created(&first).len(), created(&second).len());
}

#[test]
fn multiview_sheet_rejects_invalid_requests() {
    let handle = open_drawing();
    let empty = MultiViewSheetRequest { part: PartSpec::default(), ..Default::default() };
    let err = add_multiview_sheet(handle, empty).unwrap_err();
    assert_eq!(err.code, "session/argument-invalid");
    let bad_projection = MultiViewSheetRequest {
        part: PartSpec { solids: vec![Feature3d::Box { min: [0.0, 0.0, 0.0], max: [1.0, 1.0, 1.0] }], voids: Vec::new() },
        projection: Some("second".to_string()),
        ..Default::default()
    };
    let err = add_multiview_sheet(handle, bad_projection).unwrap_err();
    assert_eq!(err.code, "session/argument-invalid");
    let bad_scale = MultiViewSheetRequest {
        part: PartSpec { solids: vec![Feature3d::Box { min: [0.0, 0.0, 0.0], max: [1.0, 1.0, 1.0] }], voids: Vec::new() },
        scale: Some(-1.0),
        ..Default::default()
    };
    let err = add_multiview_sheet(handle, bad_scale).unwrap_err();
    assert_eq!(err.code, "session/argument-invalid");
}

#[test]
fn section_view_hatches_box_at_45_degrees() {
    let handle = open_drawing();
    let part = PartSpec { solids: vec![Feature3d::Box { min: [0.0, 0.0, 0.0], max: [40.0, 40.0, 40.0] }], voids: Vec::new() };
    let result =
        add_section_view(handle, SectionViewRequest { part, axis: Some(Axis3::Z), at: Some(20.0), ..Default::default() }).expect("section");
    let lines = lines_on(&result, "views");
    let hatches: Vec<_> = lines.iter().filter(|(from, to)| (to[0] - from[0]).abs() > 1e-9 && (to[1] - from[1]).abs() > 1e-9).collect();
    assert!(!hatches.is_empty(), "hatch lines generated");
    for (from, to) in hatches {
        let dx = to[0] - from[0];
        let dy = to[1] - from[1];
        assert_close(dy.abs(), dx.abs(), 1e-9, "hatch angle 45°");
    }
    let indicator = lines.iter().filter(|(from, to)| (to[1] - from[1]).abs() <= 1e-9 && (to[0] - from[0]).abs() > 1e-9).count();
    assert_eq!(indicator, 1, "one horizontal cutting-plane indicator line");
    let texts = texts_on(&result, "views");
    assert!(texts.iter().any(|(content, _)| content == "SECTION A-A"));
}

#[test]
fn section_view_subtracts_void_regions() {
    let handle = open_drawing();
    let part = PartSpec {
        solids: vec![Feature3d::Box { min: [0.0, 0.0, 0.0], max: [40.0, 40.0, 40.0] }],
        voids: vec![Feature3d::Cylinder { axis: Axis3::Z, center: [20.0, 20.0], from: 0.0, to: 40.0, radius: 10.0 }],
    };
    let result =
        add_section_view(handle, SectionViewRequest { part, axis: Some(Axis3::Z), at: Some(20.0), ..Default::default() }).expect("section");
    let lines = lines_on(&result, "views");
    for (from, to) in lines {
        let mid = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
        let d = ((mid[0] - 20.0).powi(2) + (mid[1] - 20.0).powi(2)).sqrt();
        assert!(d >= 10.0 - 1e-6, "no hatch segment inside the bore (midpoint distance {d})");
    }
}

#[test]
fn section_view_reports_empty_when_plane_misses_part() {
    let handle = open_drawing();
    let part = PartSpec { solids: vec![Feature3d::Box { min: [0.0, 0.0, 0.0], max: [10.0, 10.0, 10.0] }], voids: Vec::new() };
    let err = add_section_view(handle, SectionViewRequest { part, axis: Some(Axis3::Z), at: Some(50.0), ..Default::default() }).unwrap_err();
    assert_eq!(err.code, "section/empty");
}

#[test]
fn section_view_rejects_invalid_requests() {
    let handle = open_drawing();
    let missing_axis = SectionViewRequest {
        part: PartSpec { solids: vec![Feature3d::Box { min: [0.0, 0.0, 0.0], max: [1.0, 1.0, 1.0] }], voids: Vec::new() },
        at: Some(0.0),
        ..Default::default()
    };
    let err = add_section_view(handle, missing_axis).unwrap_err();
    assert_eq!(err.code, "session/argument-invalid");
    let missing_at = SectionViewRequest {
        part: PartSpec { solids: vec![Feature3d::Box { min: [0.0, 0.0, 0.0], max: [1.0, 1.0, 1.0] }], voids: Vec::new() },
        axis: Some(Axis3::X),
        ..Default::default()
    };
    let err = add_section_view(handle, missing_at).unwrap_err();
    assert_eq!(err.code, "session/argument-invalid");
}

#[test]
fn leader_note_emits_polyline_arrow_and_text() {
    let handle = open_drawing();
    let result = add_leader_note(
        handle,
        LeaderNoteRequest { points: vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0]], text: "R5".to_string(), ..Default::default() },
    )
    .expect("leader");
    let kinds = kinds(&result);
    assert!(kinds.iter().any(|kind| kind == "polyline") && kinds.iter().any(|kind| kind == "line") && kinds.iter().any(|kind| kind == "text"));
    let polys = polylines_on(&result, "notes");
    assert_eq!(polys.len(), 1);
    assert_eq!(polys[0].0.len(), 3);
    assert!(!polys[0].1);
    let lines = lines_on(&result, "notes");
    assert_eq!(lines.len(), 2, "two arrowhead wings");
    let texts = texts_on(&result, "notes");
    assert_eq!(texts.len(), 1);
    assert_eq!(texts[0].0, "R5");
}

#[test]
fn leader_note_without_arrow_omits_wings() {
    let handle = open_drawing();
    let result = add_leader_note(
        handle,
        LeaderNoteRequest { points: vec![[0.0, 0.0], [20.0, 0.0]], text: "R5".to_string(), arrow: Some(false), ..Default::default() },
    )
    .expect("leader");
    let lines = lines_on(&result, "notes");
    assert!(lines.is_empty(), "no arrowhead wings");
}

#[test]
fn leader_note_rejects_invalid_requests() {
    let handle = open_drawing();
    let short = LeaderNoteRequest { points: vec![[0.0, 0.0]], text: "R5".to_string(), ..Default::default() };
    let err = add_leader_note(handle, short).unwrap_err();
    assert_eq!(err.code, "session/argument-invalid");
    let blank = LeaderNoteRequest { points: vec![[0.0, 0.0], [10.0, 0.0]], text: "   ".to_string(), ..Default::default() };
    let err = add_leader_note(handle, blank).unwrap_err();
    assert_eq!(err.code, "session/argument-invalid");
    let coincident = LeaderNoteRequest { points: vec![[0.0, 0.0], [0.0, 0.0], [10.0, 0.0]], text: "R5".to_string(), ..Default::default() };
    let err = add_leader_note(handle, coincident).unwrap_err();
    assert_eq!(err.code, "session/argument-invalid");
}

#[test]
fn capabilities_matrix_lists_the_new_operations() {
    let matrix = crate::capabilities();
    for name in ["multiview-sheet", "section-view", "leader-note"] {
        assert_eq!(matrix["operations"][name].as_str().unwrap(), "supported", "operation `{name}`");
    }
}

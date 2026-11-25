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
fn leader_note_arrow_wings_run_back_toward_the_leader() {
    let handle = open_drawing();
    let result = add_leader_note(
        handle,
        LeaderNoteRequest { points: vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0]], text: "R5".to_string(), ..Default::default() },
    )
    .expect("leader");
    let lines = lines_on(&result, "notes");
    assert_eq!(lines.len(), 2, "two arrowhead wings");
    for (from, to) in &lines {
        assert_close(from[0], 0.0, 1e-12, "wing starts at the tip");
        assert_close(from[1], 0.0, 1e-12, "wing starts at the tip");
        let dot = (to[0] - from[0]) * 1.0 + (to[1] - from[1]) * 0.0;
        assert!(dot > 0.0, "wing runs back toward the second point, got {to:?}");
        let length = ((to[0] - from[0]).powi(2) + (to[1] - from[1]).powi(2)).sqrt();
        assert_close(length, 3.5 * 1.5, 1e-9, "wing length");
    }
    let diagonal =
        add_leader_note(handle, LeaderNoteRequest { points: vec![[0.0, 0.0], [10.0, 10.0]], text: "C'BORE".to_string(), ..Default::default() })
            .expect("diagonal leader");
    for (from, to) in lines_on(&diagonal, "notes") {
        let dot = (to[0] - from[0]) + (to[1] - from[1]);
        assert!(dot > 0.0, "diagonal wing runs back along (1,1), got {to:?}");
    }
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
fn isometric_view_places_outlines_label_and_axis_indicator() {
    let handle = open_drawing();
    let part = PartSpec { solids: vec![Feature3d::Box { min: [0.0, 0.0, 0.0], max: [20.0, 10.0, 30.0] }], voids: Vec::new() };
    let result = add_isometric_view(handle, IsometricViewRequest { part, origin: Some([10.0, 20.0]), ..Default::default() }).expect("iso");
    let view = &result["view"];
    let width = view["width"].as_f64().unwrap();
    let height = view["height"].as_f64().unwrap();
    assert!(width > 20.0 && width < 30.0, "iso width {width}");
    assert!(height > 40.0 && height < 50.0, "iso height {height}");
    assert_close(view["center"][0].as_f64().unwrap(), 10.0 + width / 2.0, 1e-9, "placed centre x");
    assert_close(view["center"][1].as_f64().unwrap(), 20.0 + height / 2.0, 1e-9, "placed centre y");
    let polys = polylines_on(&result, "views");
    assert!(!polys.is_empty(), "iso outline");
    let (min, max) = polys[0].0.iter().fold(([f64::INFINITY, f64::INFINITY], [f64::NEG_INFINITY, f64::NEG_INFINITY]), |acc, p| {
        ([acc.0[0].min(p[0]), acc.0[1].min(p[1])], [acc.1[0].max(p[0]), acc.1[1].max(p[1])])
    });
    assert_close(min[0], 10.0, 1e-9, "outline lower-left x");
    assert_close(min[1], 20.0, 1e-9, "outline lower-left y");
    assert_close(max[0], 10.0 + width, 1e-9, "outline upper-right x");
    assert_close(max[1], 20.0 + height, 1e-9, "outline upper-right y");
    let texts = texts_on(&result, "views");
    let contents: Vec<&str> = texts.iter().map(|(content, _)| content.as_str()).collect();
    for label in ["ISOMETRIC", "X", "Y", "Z"] {
        assert!(contents.contains(&label), "missing text {label}");
    }
}

#[test]
fn isometric_view_hides_voids_and_rejects_invalid_requests() {
    let handle = open_drawing();
    let part = PartSpec {
        solids: vec![Feature3d::Box { min: [0.0, 0.0, 0.0], max: [40.0, 40.0, 40.0] }],
        voids: vec![Feature3d::Cylinder { axis: Axis3::Z, center: [20.0, 20.0], from: 0.0, to: 40.0, radius: 10.0 }],
    };
    let result = add_isometric_view(handle, IsometricViewRequest { part, ..Default::default() }).expect("iso");
    let hidden = polylines_on(&result, "hidden");
    assert!(!hidden.is_empty(), "void outline on the hidden layer");
    let bad_scale = IsometricViewRequest {
        part: PartSpec { solids: vec![Feature3d::Box { min: [0.0, 0.0, 0.0], max: [1.0, 1.0, 1.0] }], voids: Vec::new() },
        scale: Some(0.0),
        ..Default::default()
    };
    let err = add_isometric_view(handle, bad_scale).unwrap_err();
    assert_eq!(err.code, "session/argument-invalid");
    let err = add_isometric_view(handle, IsometricViewRequest::default()).unwrap_err();
    assert_eq!(err.code, "session/argument-invalid");
}

#[test]
fn exploded_assembly_balloons_number_components() {
    let handle = open_drawing();
    let body = PartSpec { solids: vec![Feature3d::Box { min: [0.0, 0.0, 0.0], max: [40.0, 20.0, 10.0] }], voids: Vec::new() };
    let plate = PartSpec { solids: vec![Feature3d::Box { min: [0.0, 0.0, 0.0], max: [40.0, 20.0, 2.0] }], voids: Vec::new() };
    let result = add_exploded_assembly(
        handle,
        ExplodedAssemblyRequest {
            components: vec![
                AssemblyComponent {
                    name: Some("BASE".to_string()),
                    part: body,
                    position: Some([0.0, 0.0, 0.0]),
                    explode: Some([0.0, 0.0, 0.0]),
                    quantity: Some(1),
                    material: Some("STEEL".to_string()),
                },
                AssemblyComponent {
                    name: Some("COVER".to_string()),
                    part: plate,
                    position: Some([0.0, 0.0, 10.0]),
                    explode: Some([0.0, 0.0, 30.0]),
                    quantity: Some(2),
                    material: Some("ALLOY".to_string()),
                },
            ],
            ..Default::default()
        },
    )
    .expect("exploded");
    let items = result["items"].as_array().expect("items");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["number"].as_u64().unwrap(), 1);
    assert_eq!(items[1]["name"].as_str().unwrap(), "COVER");
    assert!(items[1]["center"][1].as_f64().unwrap() > items[0]["center"][1].as_f64().unwrap(), "the cover explodes above the base");
    let circles = circles_on(&result, "views");
    assert_eq!(circles.len(), 2, "one balloon per component");
    assert!(
        (circles[0].0[0] - circles[1].0[0]).abs() > 1.0 && (circles[0].0[1] - circles[1].0[1]).abs() > 1.0,
        "balloons sit on a ring instead of one column"
    );
    let texts = texts_on(&result, "views");
    let contents: Vec<&str> = texts.iter().map(|(content, _)| content.as_str()).collect();
    for want in ["1", "2", "BASE", "COVER", "BILL OF MATERIALS", "ITEM", "PART", "QTY", "MATERIAL", "STEEL", "ALLOY"] {
        assert!(contents.contains(&want), "missing exploded-assembly text {want}");
    }
    let leaders = lines_on(&result, "views");
    assert!(leaders.len() > 2, "each balloon plus the BOM grid contributes lines");
}

#[test]
fn exploded_assembly_rejects_invalid_requests() {
    let handle = open_drawing();
    let err = add_exploded_assembly(handle, ExplodedAssemblyRequest::default()).unwrap_err();
    assert_eq!(err.code, "session/argument-invalid");
    let empty_part = ExplodedAssemblyRequest {
        components: vec![AssemblyComponent { part: PartSpec::default(), ..Default::default() }],
        ..Default::default()
    };
    let err = add_exploded_assembly(handle, empty_part).unwrap_err();
    assert_eq!(err.code, "session/argument-invalid");
    let bad_position = ExplodedAssemblyRequest {
        components: vec![AssemblyComponent {
            part: PartSpec { solids: vec![Feature3d::Box { min: [0.0, 0.0, 0.0], max: [1.0, 1.0, 1.0] }], voids: Vec::new() },
            position: Some([f64::NAN, 0.0, 0.0]),
            ..Default::default()
        }],
        ..Default::default()
    };
    let err = add_exploded_assembly(handle, bad_position).unwrap_err();
    assert_eq!(err.code, "session/argument-invalid");
}

#[test]
fn hole_table_lists_cylindrical_voids_with_quantities() {
    let handle = open_drawing();
    let part = PartSpec {
        solids: vec![Feature3d::Box { min: [0.0, 0.0, 0.0], max: [60.0, 40.0, 10.0] }],
        voids: vec![
            Feature3d::Cylinder { axis: Axis3::Z, center: [10.0, 10.0], from: 0.0, to: 10.0, radius: 3.0 },
            Feature3d::Cylinder { axis: Axis3::Z, center: [50.0, 10.0], from: 0.0, to: 10.0, radius: 3.0 },
            Feature3d::Cylinder { axis: Axis3::Z, center: [30.0, 30.0], from: 0.0, to: 5.0, radius: 5.0 },
        ],
    };
    let result = add_hole_table(handle, HoleTableRequest { part, ..Default::default() }).expect("hole table");
    let rows = result["rows"].as_array().expect("rows");
    assert_eq!(rows.len(), 3, "one row per cylindrical void");
    assert_close(rows[0]["diameter"].as_f64().unwrap(), 6.0, 1e-12, "diameter");
    assert_close(rows[0]["posA"].as_f64().unwrap(), 10.0, 1e-12, "pos A");
    assert_close(rows[2]["depth"].as_f64().unwrap(), 5.0, 1e-12, "depth");
    assert_eq!(rows[1]["axis"].as_str().unwrap(), "z");
    let texts = texts_on(&result, "holes");
    let contents: Vec<&str> = texts.iter().map(|(content, _)| content.as_str()).collect();
    for want in ["HOLE TABLE", "No.", "Axis", "Pos A", "Pos B", "Dia", "Depth", "1", "2", "3", "6", "50", "30", "10", "5"] {
        assert!(contents.contains(&want), "missing cell {want}");
    }
    let lines = lines_on(&result, "holes");
    assert!(!lines.is_empty(), "frame and separator lines");
}

#[test]
fn hole_table_reports_empty_without_cylinders() {
    let handle = open_drawing();
    let part = PartSpec {
        solids: vec![Feature3d::Box { min: [0.0, 0.0, 0.0], max: [10.0, 10.0, 10.0] }],
        voids: vec![Feature3d::Sphere { center: [5.0, 5.0, 5.0], radius: 2.0 }],
    };
    let err = add_hole_table(handle, HoleTableRequest { part, ..Default::default() }).unwrap_err();
    assert_eq!(err.code, "hole-table/empty");
}

#[test]
fn capabilities_matrix_lists_the_new_operations() {
    let matrix = crate::capabilities();
    for name in ["multiview-sheet", "section-view", "leader-note", "isometric-view", "exploded-assembly", "hole-table"] {
        assert_eq!(matrix["operations"][name].as_str().unwrap(), "supported", "operation `{name}`");
    }
}

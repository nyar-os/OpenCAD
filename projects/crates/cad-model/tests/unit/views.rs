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
    assert_eq!(indicator, 2, "one horizontal cutting-plane indicator split around its measured value");
    let texts = texts_on(&result, "views");
    assert!(texts.iter().any(|(content, _)| content == "SECTION A-A"));
    assert!(texts.iter().any(|(content, _)| content == "40"), "the indicator carries the measured width");
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
    assert_close(circles[0].0[0], circles[1].0[0], 1e-9, "balloons share one vertical column");
    assert!((circles[0].0[1] - circles[1].0[1]).abs() > 1.0, "balloons are spread along the column");
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
    for name in
        ["multiview-sheet", "section-view", "leader-note", "isometric-view", "exploded-assembly", "hole-table", "detail-view", "auxiliary-view"]
    {
        assert_eq!(matrix["operations"][name].as_str().unwrap(), "supported", "operation `{name}`");
    }
}

/// Column rules of the hole table: the left and right borders plus the inner
/// separators, sorted left to right.
fn hole_table_columns(result: &Value) -> Vec<f64> {
    let mut verticals: Vec<(f64, f64)> = lines_on(result, "holes")
        .into_iter()
        .filter(|(from, to)| (from[0] - to[0]).abs() < 1e-9)
        .map(|(from, to)| (from[0], (to[1] - from[1]).abs()))
        .collect();
    assert_eq!(verticals.len(), 7, "two borders and five separators");
    verticals.sort_by(|left, right| left.0.total_cmp(&right.0));
    verticals.into_iter().map(|(x, _)| x).collect()
}

#[test]
fn hole_table_keeps_every_cell_inside_its_column() {
    let handle = open_drawing();
    let part = PartSpec {
        solids: vec![Feature3d::Box { min: [0.0, 0.0, 0.0], max: [60.0, 40.0, 10.0] }],
        voids: vec![
            Feature3d::Cylinder { axis: Axis3::Z, center: [10.0, 10.0], from: 0.0, to: 10.0, radius: 3.0 },
            Feature3d::Cylinder { axis: Axis3::Z, center: [50.0, 10.0], from: 0.0, to: 10.0, radius: 3.0 },
            Feature3d::Cylinder { axis: Axis3::Z, center: [30.0, 30.0], from: 0.0, to: 5.0, radius: 5.0 },
        ],
    };
    let text_height = 4.0;
    let result =
        add_hole_table(handle, HoleTableRequest { part, text_height: Some(text_height), origin: Some([0.0, 0.0]), ..Default::default() })
            .expect("hole table");
    let columns = hole_table_columns(&result);
    let padding = text_height * CELL_PADDING;
    let cells: Vec<(String, [f64; 2], f64)> = created(&result)
        .iter()
        .filter(|entity| entity["layer"] == "holes" && entity["kind"]["type"] == "text")
        .map(|entity| {
            (
                entity["kind"]["content"].as_str().unwrap().to_string(),
                [entity["kind"]["at"][0].as_f64().unwrap(), entity["kind"]["at"][1].as_f64().unwrap()],
                entity["kind"]["height"].as_f64().unwrap(),
            )
        })
        .collect();
    for (content, at, height) in &cells {
        if content == "HOLE TABLE" {
            continue;
        }
        let width = text_width(content, *height);
        let index = columns.windows(2).position(|pair| at[0] + 1e-9 >= pair[0] && at[0] + width <= pair[1] + 1e-9);
        assert!(index.is_some(), "`{content}` at {at:?} (width {width}) escapes every column in {columns:?}");
    }
    // Derivative checks: headers are centred, numbers are flush right.
    let headers = ["No.", "Axis", "Pos A", "Pos B", "Dia", "Depth"];
    for (index, header) in headers.iter().enumerate() {
        let (_, at, height) = cells.iter().find(|(content, _, _)| content == header).expect("header");
        let center = at[0] + text_width(header, *height) / 2.0;
        assert_close(center, (columns[index] + columns[index + 1]) / 2.0, 1e-9, &format!("{header} centre"));
    }
    for (content, at, height) in cells.iter().filter(|(content, _, _)| content == "50" || content == "10") {
        let right = at[0] + text_width(content, *height);
        assert!(
            columns.iter().any(|edge| (right - (edge - padding)).abs() < 1e-9),
            "`{content}` right edge {right} is not flush with a column minus padding"
        );
    }
}

#[test]
fn exploded_balloon_leaders_avoid_the_other_components() {
    let handle = open_drawing();
    let plate = |height: f64| PartSpec { solids: vec![Feature3d::Box { min: [0.0, 0.0, 0.0], max: [40.0, 24.0, height] }], voids: Vec::new() };
    let components = vec![
        AssemblyComponent {
            name: Some(String::from("BASE")),
            part: plate(6.0),
            position: None,
            explode: None,
            quantity: Some(1),
            material: None,
        },
        AssemblyComponent {
            name: Some(String::from("GASKET")),
            part: plate(2.0),
            position: Some([0.0, 0.0, 6.0]),
            explode: Some([0.0, 0.0, 30.0]),
            quantity: Some(2),
            material: Some(String::from("RUBBER")),
        },
        AssemblyComponent {
            name: Some(String::from("COVER")),
            part: plate(8.0),
            position: Some([0.0, 0.0, 8.0]),
            explode: Some([0.0, 0.0, 60.0]),
            quantity: Some(1),
            material: Some(String::from("STEEL")),
        },
    ];
    let text_height = 3.5;
    let result = add_exploded_assembly(
        handle,
        ExplodedAssemblyRequest { components: components.clone(), text_height: Some(text_height), ..Default::default() },
    )
    .expect("exploded");
    let items = result["items"].as_array().expect("items");
    let leaders: Vec<Vec<[f64; 2]>> = polylines_on(&result, "views")
        .into_iter()
        .map(|(points, _)| points)
        .filter(|points| {
            points.len() == 4
                && items.iter().any(|item| {
                    (points[3][0] - item["center"][0].as_f64().unwrap()).abs() < 1e-9
                        && (points[3][1] - item["center"][1].as_f64().unwrap()).abs() < 1e-9
                })
        })
        .collect();
    assert_eq!(leaders.len(), components.len(), "one elbow leader per component");

    // Component boxes in placed coordinates, from the same projection and
    // placement the op uses.
    let placed_shapes: Vec<Vec<Shape2>> = components
        .iter()
        .map(|component| {
            let delta =
                [0, 1, 2].map(|axis| component.position.map(|p| p[axis]).unwrap_or(0.0) + component.explode.map(|e| e[axis]).unwrap_or(0.0));
            let shift2 = iso_point(&delta);
            component.part.solids.iter().flat_map(project_iso).map(|shape| shape.translated(shift2)).collect()
        })
        .collect();
    let projected: Vec<([f64; 2], [f64; 2])> = placed_shapes.iter().map(|shapes| shapes_bounds(shapes).expect("bounds")).collect();
    let group_min = projected.iter().fold([f64::INFINITY; 2], |acc, (min, _)| [acc[0].min(min[0]), acc[1].min(min[1])]);
    let shift = [-group_min[0], -group_min[1]];
    let boxes: Vec<([f64; 2], [f64; 2])> =
        projected.iter().map(|(min, max)| ([min[0] + shift[0], min[1] + shift[1]], [max[0] + shift[0], max[1] + shift[1]])).collect();

    let radius = text_height * 1.4;
    for (index, item) in items.iter().enumerate() {
        let center = [item["center"][0].as_f64().unwrap(), item["center"][1].as_f64().unwrap()];
        let leader = leaders
            .iter()
            .find(|points| (points[points.len() - 1][0] - center[0]).abs() < 1e-9 && (points[points.len() - 1][1] - center[1]).abs() < 1e-9);
        let leader = leader.unwrap_or_else(|| panic!("leader for item {index}"));
        assert_eq!(leader.len(), 4, "balloon edge, knee, knee turn and target");
        // The tip must land on its own outline, not in open space.
        let owner_shapes = &placed_shapes[index];
        let owner_bounds = projected[index];
        let local_target = [leader[3][0] - shift[0], leader[3][1] - shift[1]];
        let nearest = owner_shapes.iter().map(|shape| nearest_on_shape(shape, local_target)).min_by(|left, right| {
            ((left[0] - local_target[0]).powi(2) + (left[1] - local_target[1]).powi(2))
                .total_cmp(&((right[0] - local_target[0]).powi(2) + (right[1] - local_target[1]).powi(2)))
        });
        let nearest = nearest.expect("outline");
        let miss = ((nearest[0] - local_target[0]).powi(2) + (nearest[1] - local_target[1]).powi(2)).sqrt();
        assert!(miss < 1e-9, "leader {index} ends {miss} away from its outline (bounds {owner_bounds:?})");
        for segment in leader.windows(2).take(3) {
            for (other, (min, max)) in boxes.iter().enumerate() {
                if other == index {
                    continue;
                }
                assert!(
                    !segment_hits_rect(segment[0], segment[1], *min, *max, 1e-6),
                    "leader {index} segment {:?} crosses component {other}",
                    segment
                );
            }
        }
        // Leaders must not cross each other either.
        for (other_index, other) in leaders.iter().enumerate() {
            if other_index <= index {
                continue;
            }
            for left in leader.windows(2) {
                for right in other.windows(2) {
                    assert!(!segments_cross(left[0], left[1], right[0], right[1]), "leader {index} crosses leader {other_index}");
                }
            }
        }
        for (other, item) in items.iter().enumerate() {
            if other == index {
                continue;
            }
            let balloon = [item["balloon"][0].as_f64().unwrap(), item["balloon"][1].as_f64().unwrap()];
            let distance = distance_to_segment(balloon, leader[0], leader[1]).min(distance_to_segment(balloon, leader[1], leader[2]));
            assert!(distance > radius - 1e-9, "leader {index} runs through balloon {other} (distance {distance})");
        }
    }
}

/// Distance from `point` to the segment `from`-`to`.
fn distance_to_segment(point: [f64; 2], from: [f64; 2], to: [f64; 2]) -> f64 {
    let delta = [to[0] - from[0], to[1] - from[1]];
    let length = delta[0] * delta[0] + delta[1] * delta[1];
    let t = if length <= f64::EPSILON {
        0.0
    }
    else {
        (((point[0] - from[0]) * delta[0] + (point[1] - from[1]) * delta[1]) / length).clamp(0.0, 1.0)
    };
    let nearest = [from[0] + delta[0] * t, from[1] + delta[1] * t];
    ((point[0] - nearest[0]).powi(2) + (point[1] - nearest[1]).powi(2)).sqrt()
}

/// Whether two segments cross at a proper interior point.
fn segments_cross(a0: [f64; 2], a1: [f64; 2], b0: [f64; 2], b1: [f64; 2]) -> bool {
    let cross = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
    let d0 = cross(b0, b1, a0);
    let d1 = cross(b0, b1, a1);
    let d2 = cross(a0, a1, b0);
    let d3 = cross(a0, a1, b1);
    ((d0 > 1e-9 && d1 < -1e-9) || (d0 < -1e-9 && d1 > 1e-9)) && ((d2 > 1e-9 && d3 < -1e-9) || (d2 < -1e-9 && d3 > 1e-9))
}

#[test]
fn detail_view_clips_to_the_circle_and_annotates_the_scale() {
    let handle = open_drawing();
    let part = PartSpec {
        solids: vec![Feature3d::Box { min: [0.0, 0.0, 0.0], max: [40.0, 20.0, 10.0] }],
        voids: vec![Feature3d::Cylinder { axis: Axis3::Z, center: [20.0, 10.0], from: 0.0, to: 10.0, radius: 4.0 }],
    };
    let result = add_detail_view(
        handle,
        DetailViewRequest {
            part,
            center: Some([20.0, 10.0]),
            radius: Some(6.0),
            view: Some(String::from("top")),
            scale: Some(3.0),
            ..Default::default()
        },
    )
    .expect("detail");
    let circles = circles_on(&result, "views");
    assert!(circles.iter().any(|(_, radius)| (radius - 6.0).abs() < 1e-9), "callout circle");
    assert!(circles.iter().any(|(_, radius)| (radius - 18.0).abs() < 1e-9), "enlarged circle");
    let placed = result["detail"]["placed"].as_array().unwrap();
    let detail_center = [placed[0].as_f64().unwrap(), placed[1].as_f64().unwrap()];
    let texts = texts_on(&result, "views");
    assert!(texts.iter().any(|(content, _)| content == "DETAIL A"));
    assert!(texts.iter().any(|(content, _)| content == "SCALE 3:1"));
    for (points, _) in polylines_on(&result, "views") {
        // The base view is drawn at 1:1 on the left; only the runs right of the
        // gap belong to the enlarged detail.
        if points.iter().all(|point| point[0] < detail_center[0] - 18.0 - 1e-6) {
            continue;
        }
        for point in points {
            let offset = ((point[0] - detail_center[0]).powi(2) + (point[1] - detail_center[1]).powi(2)).sqrt();
            assert!(offset <= 18.0 + 1e-6, "detail geometry escapes its boundary at {offset}");
        }
    }
    let err = add_detail_view(
        handle,
        DetailViewRequest {
            part: PartSpec { solids: vec![Feature3d::Box { min: [0.0, 0.0, 0.0], max: [10.0, 10.0, 10.0] }], voids: Vec::new() },
            center: Some([500.0, 500.0]),
            radius: Some(2.0),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "detail-view/empty");
}

#[test]
fn auxiliary_view_turns_the_projection_plane_about_z() {
    let handle = open_drawing();
    let part = PartSpec { solids: vec![Feature3d::Box { min: [0.0, 0.0, 0.0], max: [40.0, 20.0, 10.0] }], voids: Vec::new() };
    let result = add_auxiliary_view(handle, AuxiliaryViewRequest { part, angle: Some(45.0), ..Default::default() }).expect("auxiliary view");
    let view = &result["view"];
    assert_close(view["width"].as_f64().unwrap(), (40.0 + 20.0) * std::f64::consts::FRAC_1_SQRT_2, 1e-9, "rotated width");
    assert_close(view["height"].as_f64().unwrap(), 10.0, 1e-9, "auxiliary height keeps Z");
    let texts = texts_on(&result, "views");
    assert!(texts.iter().any(|(content, _)| content == "VIEW A"));
    assert!(texts.iter().any(|(content, _)| content == "A"), "direction marker");
    let err = add_auxiliary_view(
        handle,
        AuxiliaryViewRequest {
            part: PartSpec { solids: vec![Feature3d::Box { min: [0.0, 0.0, 0.0], max: [1.0, 1.0, 1.0] }], voids: Vec::new() },
            plane: Some(String::from("side")),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "session/argument-invalid");
}

use super::*;

/// Builds a minimal ASCII DXF with optional BLOCKS content and the given
/// ENTITIES body, both written as raw group-code pairs.
fn document(blocks: &str, entities: &str) -> String {
    format!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nBLOCKS\n{blocks}\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n{entities}\n0\nENDSEC\n0\nEOF\n"
    )
}

fn codes(losses: &[Loss]) -> Vec<&str> {
    losses.iter().map(|loss| loss.code.as_str()).collect()
}

#[test]
fn layer_freeze_and_plot_flags_are_read() {
    let source = [
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
        "WALLS",
        "70",
        "1",
        "62",
        "1",
        "6",
        "continuous",
        "290",
        "0",
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
        "ENDSEC",
        "0",
        "EOF",
    ]
    .join("\n");
    let (drawing, _losses) = dxf(&source, None).expect("read layer flags");
    let layer = drawing.layer("WALLS").expect("layer");
    assert!(layer.frozen, "DXF flag 1 marks the layer frozen");
    assert!(!layer.plottable, "DXF code 290 marks the layer non-plottable");
}

#[test]
fn layer_true_colour_takes_precedence_over_the_aci_index() {
    let source = [
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
        "WALLS",
        "70",
        "0",
        "62",
        "1",
        "420",
        "8388736",
        "6",
        "continuous",
        "0",
        "LAYER",
        "2",
        "NOTES",
        "70",
        "0",
        "62",
        "2",
        "6",
        "continuous",
        "0",
        "ENDTAB",
        "0",
        "ENDSEC",
        "0",
        "SECTION",
        "2",
        "ENTITIES",
        "0",
        "ENDSEC",
        "0",
        "EOF",
    ]
    .join("\n");
    let (drawing, losses) = dxf(&source, None).expect("read layers");
    assert_eq!(drawing.layer("WALLS").expect("walls").color, "#800080", "code 420 overrides code 62");
    assert_eq!(drawing.layer("NOTES").expect("notes").color, "#ffff00", "without 420 the ACI index is mapped");
    assert!(!codes(&losses).contains(&"dxf/layer-color"), "{:?}", codes(&losses));
}

#[test]
fn hatch_import_reads_a_polyline_boundary_and_keeps_its_pattern() {
    let entities = [
        "0", "HATCH", "8", "FILL", "2", "ANSI31", "70", "0", "71", "0", "91", "1", "92", "1", "72", "0", "73", "1", "93", "3", "10", "0", "20",
        "0", "10", "4", "20", "0", "10", "4", "20", "4", "97", "0", "75", "0", "76", "1", "98", "0",
    ]
    .join("\n");
    let (drawing, losses) = dxf(&document("", &entities), None).expect("read hatch");

    assert_eq!(drawing.entities.len(), 1);
    let EntityKind::Hatch { points, pattern } = &drawing.entities[0].kind
    else {
        panic!("expected a hatch, got {:?}", drawing.entities[0].kind);
    };
    assert_eq!(points.as_slice(), &[[0.0, 0.0], [4.0, 0.0], [4.0, 4.0]]);
    assert_eq!(pattern, "ANSI31");
    assert_eq!(drawing.entities[0].layer, "FILL");
    assert!(codes(&losses).contains(&"dxf/hatch-boundary"));
    assert!(!codes(&losses).contains(&"dxf/unsupported-entity"), "{:?}", codes(&losses));
}

#[test]
fn hatch_import_samples_line_and_arc_edges_and_skips_unknown_edges() {
    let entities = [
        "0", "HATCH", "8", "FILL", "2", "SOLID", "70", "1", "91", "1", "92", "3", "93", "2", "72", "1", "10", "0", "20", "0", "11", "4", "21",
        "0", "72", "2", "10", "4", "20", "0", "40", "2", "50", "0", "51", "180", "73", "1", "97", "0", "75", "0", "76", "1", "98", "0",
    ]
    .join("\n");
    let (drawing, losses) = dxf(&document("", &entities), None).expect("read edge hatch");
    assert_eq!(drawing.entities.len(), 1, "{:?}", codes(&losses));
    let EntityKind::Hatch { points, .. } = &drawing.entities[0].kind
    else {
        panic!("expected a hatch, got {:?}", drawing.entities[0].kind);
    };
    assert!(points.len() > 4, "the arc edge is sampled: {points:?}");
    assert_eq!(points[0], [0.0, 0.0]);
    assert!((points.last().expect("points")[1]).abs() < 1e-9);

    let spline_edge = ["0", "HATCH", "8", "FILL", "2", "SOLID", "91", "1", "92", "3", "93", "1", "72", "4", "94", "3", "97", "0"].join("\n");
    let (drawing, losses) = dxf(&document("", &spline_edge), None).expect("read unsupported hatch");
    assert!(drawing.entities.is_empty());
    assert!(losses.iter().any(|loss| loss.code == "dxf/unsupported-entity" && loss.detail.contains("curved")));
}

#[test]
fn mleader_import_reads_leader_vertices_and_flattens_multiline_text() {
    let entities = [
        "0",
        "MLEADER",
        "8",
        "NOTES",
        "40",
        "2.5",
        "304",
        "CONTEXT_DATA{",
        "304",
        "LEADER{",
        "290",
        "1",
        "10",
        "0",
        "20",
        "0",
        "10",
        "10",
        "20",
        "5",
        "304",
        "}",
        "304",
        "TEXT{",
        "10",
        "99",
        "20",
        "99",
        "304",
        "{\\fArial|b0;\\Pfirst line\\Psecond line}",
        "304",
        "}",
        "304",
        "}",
    ]
    .join("\n");
    let (drawing, losses) = dxf(&document("", &entities), None).expect("read mleader");

    assert_eq!(drawing.entities.len(), 3, "{:?}", codes(&losses));
    let EntityKind::Polyline { points, closed } = &drawing.entities[0].kind
    else {
        panic!("expected a leader polyline, got {:?}", drawing.entities[0].kind);
    };
    assert_eq!(points.as_slice(), &[[0.0, 0.0], [10.0, 5.0]]);
    assert!(!closed);
    let EntityKind::Text { at, height, content } = &drawing.entities[1].kind
    else {
        panic!("expected a text line, got {:?}", drawing.entities[1].kind);
    };
    assert_eq!((*at, *height, content.as_str()), ([0.0, 0.0], 2.5, "first line"));
    let EntityKind::Text { at, content, .. } = &drawing.entities[2].kind
    else {
        panic!("expected a text line, got {:?}", drawing.entities[2].kind);
    };
    assert!((at[0] - 0.0).abs() < 1e-9 && (at[1] + 3.5).abs() < 1e-9, "stacked line at {at:?}");
    assert_eq!(content, "second line");
    assert!(codes(&losses).contains(&"dxf/mleader-leader"));
    assert!(codes(&losses).contains(&"dxf/mleader-text"));

    let textless =
        ["0", "MLEADER", "8", "NOTES", "304", "CONTEXT_DATA{", "304", "LEADER{", "10", "0", "20", "0", "10", "5", "20", "5", "304", "}"]
            .join("\n");
    let (drawing, losses) = dxf(&document("", &textless), None).expect("read textless mleader");
    assert_eq!(drawing.entities.len(), 1);
    assert!(losses.iter().any(|loss| loss.code == "dxf/mleader-text-missing" && loss.severity == "warning"));
}

#[test]
fn dimension_import_reads_aligned_and_rotated_references() {
    let entities = [
        "0",
        "DIMENSION",
        "8",
        "DIMENSIONS",
        "70",
        "1",
        "13",
        "0",
        "23",
        "0",
        "14",
        "10",
        "24",
        "0",
        "10",
        "5",
        "20",
        "3",
        "1",
        "W 10",
        "0",
        "DIMENSION",
        "8",
        "DIMENSIONS",
        "70",
        "0",
        "50",
        "90",
        "13",
        "0",
        "23",
        "0",
        "14",
        "0",
        "24",
        "10",
        "10",
        "2",
        "20",
        "5",
        "1",
        "<>",
    ]
    .join("\n");
    let (drawing, losses) = dxf(&document("", &entities), Some("dimensions".to_string())).expect("read dimensions");

    assert_eq!(drawing.entities.len(), 2);
    assert_eq!(drawing.entities[0].layer, "DIMENSIONS");
    let EntityKind::Dimension { from, to, offset, text, .. } = &drawing.entities[0].kind
    else {
        panic!("expected a dimension, got {:?}", drawing.entities[0].kind);
    };
    assert_eq!((*from, *to), ([0.0, 0.0], [10.0, 0.0]));
    assert!((offset - 3.0).abs() < 1e-9);
    assert_eq!(text.as_deref(), Some("W 10"));

    // A rotated dimension whose rotation follows its segment keeps the same
    // measured points, and `<>` means "use the measured value".
    let EntityKind::Dimension { offset, text, .. } = &drawing.entities[1].kind
    else {
        panic!("expected a rotated dimension, got {:?}", drawing.entities[1].kind);
    };
    assert!((offset + 2.0).abs() < 1e-9);
    assert_eq!(text, &None);
    assert!(!codes(&losses).contains(&"dxf/unsupported-entity"), "{:?}", codes(&losses));
}

#[test]
fn dimension_import_skips_types_the_model_cannot_carry() {
    let entities = [
        "0",
        "DIMENSION",
        "70",
        "2",
        "13",
        "0",
        "23",
        "0",
        "14",
        "10",
        "24",
        "0",
        "0",
        "DIMENSION",
        "70",
        "0",
        "50",
        "90",
        "13",
        "0",
        "23",
        "0",
        "14",
        "10",
        "24",
        "0",
        "10",
        "5",
        "20",
        "3",
    ]
    .join("\n");
    let (drawing, losses) = dxf(&document("", &entities), None).expect("read dimensions");

    assert!(drawing.entities.is_empty());
    let skipped = losses.iter().filter(|loss| loss.code == "dxf/unsupported-entity").collect::<Vec<_>>();
    assert_eq!(skipped.len(), 2, "{:?}", codes(&losses));
    assert!(skipped.iter().any(|loss| loss.detail.contains("angular")));
    assert!(skipped.iter().any(|loss| loss.detail.contains("rotated")));
    assert!(skipped.iter().all(|loss| loss.severity == "warning"));
}

#[test]
fn dimension_import_reports_a_missing_type_and_definition_point() {
    let entities = ["0", "DIMENSION", "13", "0", "23", "0", "14", "4", "24", "0"].join("\n");
    let (drawing, losses) = dxf(&document("", &entities), None).expect("read dimensions");

    let EntityKind::Dimension { offset, .. } = &drawing.entities[0].kind
    else {
        panic!("expected a dimension");
    };
    assert_eq!(*offset, 0.0, "a missing definition point keeps the measured segment");
    assert!(codes(&losses).contains(&"dxf/dimension-type"));
    assert!(codes(&losses).contains(&"dxf/dimension-offset"));
}

#[test]
fn attribute_values_follow_the_insert_layer() {
    let blocks = ["0", "BLOCK", "8", "0", "2", "TITLE", "70", "0", "10", "0", "20", "0", "0", "ENDBLK"].join("\n");
    let entities = [
        "0", "INSERT", "8", "TITLE", "2", "TITLE", "10", "0", "20", "0", "0", "ATTRIB", "8", "0", "10", "2", "20", "3", "40", "2.5", "1",
        "A-101", "0", "ATTRIB", "8", "NOTES", "10", "4", "20", "3", "40", "2.5", "1", "NOTE", "0", "ATTRIB", "8", "0", "10", "6", "20", "3",
        "40", "2.5", "1", "", "0", "SEQEND", "8", "0",
    ]
    .join("\n");
    let (drawing, losses) = dxf(&document(&blocks, &entities), None).expect("read attributes");

    assert_eq!(drawing.entities.len(), 2);
    assert_eq!(drawing.entities[0].layer, "TITLE");
    let EntityKind::Text { at, height, content } = &drawing.entities[0].kind
    else {
        panic!("expected text, got {:?}", drawing.entities[0].kind);
    };
    assert_eq!((*at, *height, content.as_str()), ([2.0, 3.0], 2.5, "A-101"));
    assert_eq!(drawing.entities[1].layer, "NOTES");
    assert!(codes(&losses).contains(&"dxf/attribute-text"));
    assert!(codes(&losses).contains(&"dxf/attribute-empty"));
    assert_eq!(
        losses.iter().find(|loss| loss.code == "dxf/attribute-text").map(|loss| loss.detail.as_str()),
        Some("2 block attribute value(s) were imported as text")
    );
    assert_eq!(
        losses.iter().find(|loss| loss.code == "dxf/attribute-empty").map(|loss| loss.detail.as_str()),
        Some("1 block attribute(s) had no value and were skipped")
    );
}

#[test]
fn spline_import_prefers_fit_points_and_falls_back_to_control_points() {
    let entities = [
        "0", "SPLINE", "8", "WALLS", "70", "0", "71", "3", "10", "0", "20", "0", "10", "5", "20", "10", "10", "10", "20", "0", "11", "0", "21",
        "0", "11", "5", "21", "5", "11", "10", "21", "0", "0", "SPLINE", "8", "WALLS", "70", "1", "10", "2", "20", "2", "10", "8", "20", "2",
        "10", "8", "20", "8", "0", "SPLINE", "8", "WALLS",
    ]
    .join("\n");
    let (drawing, losses) = dxf(&document("", &entities), None).expect("read splines");

    assert_eq!(drawing.entities.len(), 2);
    let EntityKind::Spline { points, closed } = &drawing.entities[0].kind
    else {
        panic!("expected a spline, got {:?}", drawing.entities[0].kind);
    };
    assert_eq!(points.as_slice(), &[[0.0, 0.0], [5.0, 5.0], [10.0, 0.0]]);
    assert!(!closed, "fit points of an open spline keep the chain open");
    let EntityKind::Spline { points, closed } = &drawing.entities[1].kind
    else {
        panic!("expected a spline, got {:?}", drawing.entities[1].kind);
    };
    assert_eq!(points.as_slice(), &[[2.0, 2.0], [8.0, 2.0], [8.0, 8.0]]);
    assert!(closed, "a closed SPLINE keeps the closed flag");
    assert!(codes(&losses).contains(&"dxf/spline-fit-points"));
    assert!(codes(&losses).contains(&"dxf/spline-control-polygon"));
    assert!(codes(&losses).contains(&"dxf/unsupported-entity"), "a SPLINE without points is still skipped");
}

#[test]
fn leader_import_becomes_an_open_polyline() {
    let entities =
        ["0", "LEADER", "8", "NOTES", "3", "STANDARD", "76", "3", "10", "0", "20", "0", "10", "4", "20", "2", "10", "8", "20", "2"].join("\n");
    let (drawing, losses) = dxf(&document("", &entities), None).expect("read leader");

    assert_eq!(drawing.entities.len(), 1);
    let EntityKind::Polyline { points, closed } = &drawing.entities[0].kind
    else {
        panic!("expected a leader polyline, got {:?}", drawing.entities[0].kind);
    };
    assert_eq!(points.as_slice(), &[[0.0, 0.0], [4.0, 2.0], [8.0, 2.0]]);
    assert!(!closed);
    assert!(codes(&losses).contains(&"dxf/leader-polyline"));
    assert!(!codes(&losses).contains(&"dxf/unsupported-entity"), "{:?}", codes(&losses));
}

#[test]
fn three_dimensional_polyline_is_projected_onto_the_plane() {
    let entities = [
        "0", "POLYLINE", "8", "WALLS", "70", "8", "0", "VERTEX", "8", "WALLS", "10", "0", "20", "0", "30", "5", "0", "VERTEX", "8", "WALLS",
        "10", "10", "20", "0", "30", "6", "0", "SEQEND", "8", "WALLS",
    ]
    .join("\n");
    let (drawing, losses) = dxf(&document("", &entities), None).expect("read 3D polyline");

    assert_eq!(drawing.entities.len(), 1);
    let EntityKind::Polyline { points, .. } = &drawing.entities[0].kind
    else {
        panic!("expected a projected polyline, got {:?}", drawing.entities[0].kind);
    };
    assert_eq!(points.as_slice(), &[[0.0, 0.0], [10.0, 0.0]]);
    assert!(codes(&losses).contains(&"dxf/projected-3d"));
}

#[test]
fn svg_reads_basic_shapes() {
    let source = "<svg xmlns=\"http://www.w3.org/2000/svg\">\
        <line x1=\"0\" y1=\"0\" x2=\"10\" y2=\"5\"/>\
        <circle cx=\"5\" cy=\"5\" r=\"2\"/>\
        <ellipse cx=\"0\" cy=\"0\" rx=\"4\" ry=\"2\"/>\
        <rect x=\"1\" y=\"1\" width=\"4\" height=\"2\"/>\
        <polyline points=\"0,0 1,2 3,4\"/>\
        <polygon points=\"0,0 2,0 2,2\"/>\
    </svg>";
    let (drawing, losses) = svg(source, Some("Plan".to_string())).expect("read svg");

    assert_eq!(drawing.title.as_deref(), Some("Plan"));
    assert_eq!(drawing.entities.len(), 6);
    let EntityKind::Line { from, to } = &drawing.entities[0].kind
    else {
        panic!("expected a line")
    };
    assert_eq!((*from, *to), ([0.0, 0.0], [10.0, 5.0]));
    let EntityKind::Circle { center, radius } = &drawing.entities[1].kind
    else {
        panic!("expected a circle")
    };
    assert_eq!((*center, *radius), ([5.0, 5.0], 2.0));
    let EntityKind::Ellipse { center, rx, ry, rotation, start_angle, end_angle } = &drawing.entities[2].kind
    else {
        panic!("expected an ellipse")
    };
    assert_eq!((*center, *rx, *ry, *rotation, *start_angle, *end_angle), ([0.0, 0.0], 4.0, 2.0, 0.0, 0.0, 360.0));
    let EntityKind::Polyline { points, closed } = &drawing.entities[3].kind
    else {
        panic!("expected a rect polyline")
    };
    assert_eq!(points.as_slice(), &[[1.0, 1.0], [5.0, 1.0], [5.0, 3.0], [1.0, 3.0]]);
    assert!(closed);
    let EntityKind::Polyline { points, closed } = &drawing.entities[4].kind
    else {
        panic!("expected a polyline")
    };
    assert_eq!(points.as_slice(), &[[0.0, 0.0], [1.0, 2.0], [3.0, 4.0]]);
    assert!(!closed);
    let EntityKind::Polyline { points, closed } = &drawing.entities[5].kind
    else {
        panic!("expected a polygon")
    };
    assert_eq!(points.as_slice(), &[[0.0, 0.0], [2.0, 0.0], [2.0, 2.0]]);
    assert!(closed);
    assert!(drawing.entities.iter().all(|entity| entity.layer == "0"));
    assert_eq!(codes(&losses), ["svg/assumed-units"]);
}

#[test]
fn svg_rect_rounding_is_reported() {
    let (drawing, losses) = svg("<svg><rect x=\"0\" y=\"0\" width=\"10\" height=\"10\" rx=\"2\"/></svg>", None).expect("read rounded rect");
    assert_eq!(drawing.entities.len(), 1);
    assert!(codes(&losses).contains(&"svg/rect-corner"), "{:?}", codes(&losses));
}

#[test]
fn svg_path_moves_lines_and_closes() {
    let source = "<svg><path d=\"M0,0 L10,0 L10,10 Z M20,20 L25,25\"/></svg>";
    let (drawing, _losses) = svg(source, None).expect("read path");
    assert_eq!(drawing.entities.len(), 2);
    let EntityKind::Polyline { points, closed } = &drawing.entities[0].kind
    else {
        panic!("expected a closed subpath")
    };
    assert_eq!(points.as_slice(), &[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]);
    assert!(closed);
    let EntityKind::Polyline { points, closed } = &drawing.entities[1].kind
    else {
        panic!("expected an open subpath")
    };
    assert_eq!(points.as_slice(), &[[20.0, 20.0], [25.0, 25.0]]);
    assert!(!closed);
}

#[test]
fn svg_path_relative_commands_match_absolute() {
    let source = "<svg><path d=\"M1,1 l2,0 h3 v2\"/><path d=\"M1,1 L3,1 H6 V3\"/></svg>";
    let (drawing, _losses) = svg(source, None).expect("read relative path");
    assert_eq!(drawing.entities.len(), 2);
    let EntityKind::Polyline { points, .. } = &drawing.entities[0].kind
    else {
        panic!("expected a polyline")
    };
    let EntityKind::Polyline { points: absolute, .. } = &drawing.entities[1].kind
    else {
        panic!("expected a polyline")
    };
    assert_eq!(points.as_slice(), &[[1.0, 1.0], [3.0, 1.0], [6.0, 1.0], [6.0, 3.0]]);
    assert_eq!(points, absolute);
}

#[test]
fn svg_path_curves_are_flattened() {
    let source = "<svg><path d=\"M0,0 C10,0 10,10 0,10\"/><path d=\"M0,0 Q10,0 10,10\"/><path d=\"M0,0 C4,0 6,2 10,2 S16,4 20,4\"/></svg>";
    let (drawing, losses) = svg(source, None).expect("read curved paths");
    assert_eq!(drawing.entities.len(), 3);
    for entity in &drawing.entities {
        let EntityKind::Polyline { points, closed } = &entity.kind
        else {
            panic!("expected a flattened polyline")
        };
        assert!(points.len() > 2, "a curve keeps its samples");
        assert!(!closed);
    }
    let EntityKind::Polyline { points, .. } = &drawing.entities[0].kind
    else {
        panic!("expected a polyline")
    };
    assert_eq!((points.len(), points[points.len() - 1]), (17, [0.0, 10.0]));
    let EntityKind::Polyline { points, .. } = &drawing.entities[1].kind
    else {
        panic!("expected a polyline")
    };
    assert_eq!((points.len(), points[points.len() - 1]), (17, [10.0, 10.0]));
    let EntityKind::Polyline { points, .. } = &drawing.entities[2].kind
    else {
        panic!("expected a polyline")
    };
    assert_eq!((points.len(), points[points.len() - 1]), (33, [20.0, 4.0]));
    let flattened = losses.iter().find(|loss| loss.code == "svg/path-curve-flattened").expect("flattened loss");
    assert_eq!(flattened.severity, "notice");
    assert!(flattened.detail.contains('4'), "{}", flattened.detail);
}

#[test]
fn svg_path_arc_command_truncates_the_path() {
    let source = "<svg><path d=\"M0,0 L5,5 A2,2 0 0 1 10,10 L20,20\"/></svg>";
    let (drawing, losses) = svg(source, None).expect("read truncated path");
    assert_eq!(drawing.entities.len(), 1);
    let EntityKind::Polyline { points, .. } = &drawing.entities[0].kind
    else {
        panic!("expected a polyline")
    };
    assert_eq!(points.as_slice(), &[[0.0, 0.0], [5.0, 5.0]]);
    let truncated = losses.iter().find(|loss| loss.code == "svg/path-unsupported-command").expect("truncated loss");
    assert_eq!(truncated.severity, "warning");
}

#[test]
fn svg_text_becomes_a_text_entity() {
    let source = "<svg><text x=\"3\" y=\"4\" font-size=\"12\">Site <tspan>t</tspan> plan</text><text>hello</text></svg>";
    let (drawing, _losses) = svg(source, None).expect("read text");
    assert_eq!(drawing.entities.len(), 2);
    let EntityKind::Text { at, height, content } = &drawing.entities[0].kind
    else {
        panic!("expected text")
    };
    assert_eq!((*at, *height, content.as_str()), ([3.0, 4.0], 12.0, "Site t plan"));
    let EntityKind::Text { at, height, content } = &drawing.entities[1].kind
    else {
        panic!("expected text")
    };
    assert_eq!((*at, *height, content.as_str()), ([0.0, 0.0], 16.0, "hello"));
}

#[test]
fn svg_transforms_and_unsupported_elements_are_reported() {
    let source = "<svg><g transform=\"translate(1,1)\"><line x1=\"0\" y1=\"0\" x2=\"1\" y2=\"1\"/></g>\
        <image href=\"a.png\"/><use href=\"#b\"/><defs><circle cx=\"0\" cy=\"0\" r=\"5\"/></defs></svg>";
    let (drawing, losses) = svg(source, None).expect("read svg with losses");
    assert_eq!(drawing.entities.len(), 1, "defs content is not imported");
    let EntityKind::Line { from, to } = &drawing.entities[0].kind
    else {
        panic!("expected a line")
    };
    assert_eq!((*from, *to), ([0.0, 0.0], [1.0, 1.0]), "transforms are not applied");
    let codes = codes(&losses);
    assert!(codes.contains(&"svg/transform"), "{codes:?}");
    assert_eq!(codes.iter().filter(|code| **code == "svg/element-unsupported").count(), 2, "{codes:?}");
    assert!(losses.iter().any(|loss| loss.code == "svg/element-unsupported" && loss.detail.contains("image")));
    assert!(losses.iter().any(|loss| loss.code == "svg/element-unsupported" && loss.detail.contains("use")));
}

#[test]
fn svg_degenerate_geometry_is_skipped() {
    let source = "<svg><circle cx=\"1\" cy=\"1\"/><line/><line x2=\"4\" y2=\"3\"/></svg>";
    let (drawing, losses) = svg(source, None).expect("read svg");
    assert_eq!(drawing.entities.len(), 1);
    let EntityKind::Line { from, to } = &drawing.entities[0].kind
    else {
        panic!("expected a line")
    };
    assert_eq!((*from, *to), ([0.0, 0.0], [4.0, 3.0]));
    let invalid = losses.iter().find(|loss| loss.code == "svg/element-invalid").expect("invalid loss");
    assert_eq!(invalid.severity, "warning");
    assert!(invalid.detail.contains('2'), "{}", invalid.detail);
}

#[test]
fn svg_rejects_non_svg_text_and_imports_an_empty_deck() {
    let error = svg("0\nSECTION\n2\nENTITIES\n0\nENDSEC\n0\nEOF\n", None).expect_err("not svg");
    assert_eq!(error.code, "format/invalid");
    let (drawing, losses) = svg("<svg></svg>", None).expect("empty svg");
    assert_eq!(drawing.entities.len(), 0);
    assert_eq!(codes(&losses), ["svg/assumed-units"]);
}

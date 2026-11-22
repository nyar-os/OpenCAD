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
    let EntityKind::Dimension { from, to, offset, text } = &drawing.entities[0].kind
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

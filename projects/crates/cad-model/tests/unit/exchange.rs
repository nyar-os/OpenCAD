use super::*;

#[test]
fn light_default_layer_colour_is_darkened_for_export() {
    let original = color_of("#d0d7de");
    let dark = export_color(original);
    assert!(luma(original) > LIGHT_INK_LIMIT, "the viewport default must be light");
    assert!(luma(dark) < 0.5, "`{}` is not legible on white", dark.to_css());
}

#[test]
fn white_is_darkened_for_export() {
    let dark = export_color(Color::WHITE);
    assert!(luma(dark) < 0.5, "`{}` is not legible on white", dark.to_css());
}

#[test]
fn dark_layer_colours_are_kept_untouched() {
    let teal = color_of("#0b7285");
    assert_eq!(export_color(teal), teal);
    assert_eq!(export_color(Color::BLACK), Color::BLACK);
}

#[test]
fn lightened_colour_keeps_its_channel_order() {
    let dark = export_color(color_of("#d0d7de"));
    assert!(dark.blue() >= dark.green() && dark.green() >= dark.red(), "`{}` lost the cool grey hue", dark.to_css());
}

#[test]
fn every_dimension_kind_lowers_solid_arrowheads() {
    let mut drawing = Drawing::empty(None, Units::Mm);
    drawing.entities.push(Entity {
        id: "entity-1".to_string(),
        layer: "dimensions".to_string(),
        kind: EntityKind::Dimension { from: [0.0, 0.0], to: [100.0, 0.0], offset: 10.0, text: None, style: None },
    });
    drawing.entities.push(Entity {
        id: "entity-2".to_string(),
        layer: "dimensions".to_string(),
        kind: EntityKind::RadialDimension { center: [50.0, 50.0], radius: 20.0, angle: 45.0, offset: 5.0, diameter: false, text: None },
    });
    drawing.entities.push(Entity {
        id: "entity-3".to_string(),
        layer: "dimensions".to_string(),
        kind: EntityKind::OrdinateDimension { origin: [0.0, 0.0], feature: [30.0, 10.0], axis: OrdinateAxis::X, offset: 20.0, text: None },
    });
    drawing.entities.push(Entity {
        id: "entity-4".to_string(),
        layer: "dimensions".to_string(),
        kind: EntityKind::AngularDimension { center: [0.0, 0.0], first: [10.0, 0.0], second: [0.0, 10.0], offset: 5.0, text: None },
    });
    let frame = Frame::of(&drawing);
    let map = |point: [f64; 2]| frame.map(point);
    let projection = Projection {
        map: &map,
        scale: 1.0,
        stroke: 0.5,
        lineweight_scale: 1.0,
        label_offset: 1.0,
        label_size: 1.75,
        dim_styles: &drawing.dim_styles,
    };
    let flat = build_drawing(&drawing, FlatSize::new(frame.width, frame.height), &projection);
    let arrows = flat.elements.iter().filter(|element| matches!(&element.shape, Shape::Polygon { .. })).collect::<Vec<_>>();
    assert_eq!(arrows.len(), 6, "two linear, one radial, one ordinate and two angular arrowheads");
    assert!(
        arrows.iter().all(|element| element.style.fill.is_some() && element.style.stroke.is_none()),
        "every arrowhead is a solid outline-free triangle"
    );
    let dots = flat.elements.iter().filter(|element| matches!(&element.shape, Shape::Circle { .. })).count();
    assert_eq!(dots, 1, "the radius dimension marks its centre");
}

#[test]
fn dimension_arrow_follows_the_referenced_style() {
    let mut drawing = Drawing::empty(None, Units::Mm);
    drawing.dim_styles.push(DimensionStyle { id: "big".to_string(), name: "big".to_string(), arrow_size: 10.0, ..DimensionStyle::standard() });
    drawing.entities.push(Entity {
        id: "entity-1".to_string(),
        layer: "dimensions".to_string(),
        kind: EntityKind::Dimension { from: [0.0, 0.0], to: [100.0, 0.0], offset: 10.0, text: None, style: None },
    });
    drawing.entities.push(Entity {
        id: "entity-2".to_string(),
        layer: "dimensions".to_string(),
        kind: EntityKind::Dimension { from: [0.0, -20.0], to: [100.0, -20.0], offset: 10.0, text: None, style: Some("big".to_string()) },
    });
    let frame = Frame::of(&drawing);
    let map = |point: [f64; 2]| frame.map(point);
    let projection = Projection {
        map: &map,
        scale: 1.0,
        stroke: 0.5,
        lineweight_scale: 1.0,
        label_offset: 1.0,
        label_size: 1.75,
        dim_styles: &drawing.dim_styles,
    };
    let flat = build_drawing(&drawing, FlatSize::new(frame.width, frame.height), &projection);
    let widths = flat
        .elements
        .iter()
        .filter_map(|element| {
            let Shape::Polygon { points } = &element.shape
            else {
                return None;
            };
            let xs = points.iter().map(|point| point.x).collect::<Vec<_>>();
            Some(xs.iter().fold(f64::MIN, |acc, x| acc.max(*x)) - xs.iter().fold(f64::MAX, |acc, x| acc.min(*x)))
        })
        .collect::<Vec<_>>();
    assert_eq!(widths.len(), 4, "both dimensions draw two arrowheads each");
    assert!(widths[..2].iter().all(|width| (width - 2.5).abs() < 1e-6), "the default style keeps the standard arrow: {widths:?}");
    assert!(widths[2..].iter().all(|width| (width - 10.0).abs() < 1e-6), "the referenced style sizes the arrow: {widths:?}");
}

#[test]
fn svg_export_draws_sheet_frame_and_title_block() {
    let mut drawing = Drawing::empty(Some("Bracket".to_string()), Units::Mm);
    drawing.properties.author = "Ada".to_string();
    drawing.entities.push(Entity {
        id: "entity-1".to_string(),
        layer: "0".to_string(),
        kind: EntityKind::Line { from: [0.0, 0.0], to: [100.0, 40.0] },
    });
    let svg = svg(&drawing);
    assert!(svg.contains("Units: mm"), "the title block declares the units: {svg}");
    assert!(svg.contains("Rev 1"), "the title block reports the revision: {svg}");
    assert!(svg.contains("Ada"), "the title block reports the author: {svg}");
    assert!(svg.matches("<path").count() >= 3, "the borders and the title block are closed paths: {svg}");
}

#[test]
fn pdf_export_draws_sheet_frame_and_title_block() {
    let mut drawing = Drawing::empty(Some("Bracket".to_string()), Units::Mm);
    drawing.properties.author = "Ada".to_string();
    drawing.entities.push(Entity {
        id: "entity-1".to_string(),
        layer: "0".to_string(),
        kind: EntityKind::Line { from: [0.0, 0.0], to: [100.0, 40.0] },
    });
    let (pdf, losses) = pdf(&drawing);
    assert!(pdf.starts_with("%PDF-1.7"), "a PDF header: {pdf}");
    assert!(pdf.contains("(Bracket) Tj"), "the title block reports the title: {pdf}");
    assert!(pdf.contains("(Units: mm) Tj"), "the title block declares the units: {pdf}");
    assert!(pdf.contains("(Rev 1) Tj"), "the title block reports the revision: {pdf}");
    assert!(pdf.contains("(Ada) Tj"), "the title block reports the author: {pdf}");
    assert!(losses.is_empty(), "ASCII sheet text needs no encoding loss: {losses:?}");
    // Two borders, the block frame and its three rules, plus the entity itself.
    let strokes = pdf.matches("\nS\n").count();
    assert!(strokes >= 7, "the content stream carries the frame strokes and the entity: {strokes}");
}

#[test]
fn pdf_geometry_stays_clear_of_the_title_block() {
    let mut drawing = Drawing::empty(Some("Bracket".to_string()), Units::Mm);
    drawing.entities.push(Entity {
        id: "entity-1".to_string(),
        layer: "0".to_string(),
        kind: EntityKind::Line { from: [0.0, 0.0], to: [100.0, 40.0] },
    });
    let (flat, layout) = pdf_page(&drawing);
    let block = layout.block.expect("the A4 sheet fits a title block");
    let Shape::Line { from, to } = &flat.elements[0].shape
    else {
        panic!("the entity is the first element");
    };
    for point in [from, to] {
        assert!(point.y < block.top, "{point:?} reaches into the title block whose top is {}", block.top);
        assert!(
            point.x > layout.inner && point.x < PDF_PAGE_WIDTH - layout.inner,
            "{point:?} crosses a vertical inner border at {}",
            layout.inner
        );
        assert!(
            point.y > layout.inner && point.y < PDF_PAGE_HEIGHT - layout.inner,
            "{point:?} crosses a horizontal inner border at {}",
            layout.inner
        );
    }
}

#[test]
fn pdf_export_keeps_dimension_text_legible() {
    let mut drawing = Drawing::empty(Some("Large plate".to_string()), Units::Mm);
    drawing.entities.push(Entity {
        id: "entity-1".to_string(),
        layer: "0".to_string(),
        kind: EntityKind::Dimension { from: [0.0, 0.0], to: [300.0, 200.0], offset: 10.0, text: None, style: None },
    });
    let (pdf, _) = pdf(&drawing);
    let sizes: Vec<f64> =
        pdf.split("/F1 ").filter_map(|chunk| chunk.split(" Tf").next()).filter_map(|size| size.trim().parse::<f64>().ok()).collect();
    assert!(!sizes.is_empty(), "the sheet writes text: {pdf}");
    let smallest = sizes.iter().copied().fold(f64::INFINITY, f64::min);
    assert!(smallest >= 7.0, "dimension text stays legible after fitting: {smallest} pt in {sizes:?}");
}

#[test]
fn pdf_export_handles_an_empty_drawing() {
    let (pdf, losses) = pdf(&Drawing::empty(None, Units::Mm));
    assert!(pdf.starts_with("%PDF-1.7"), "a PDF header: {pdf}");
    assert!(pdf.contains("(Untitled drawing) Tj"), "the frame falls back to a placeholder title: {pdf}");
    assert!(losses.is_empty(), "a frame-only sheet needs no loss: {losses:?}");
}

/// Parses a generated IGES deck back into `(entity type, parameter data)` pairs,
/// checking the fixed card layout, the section order and the pointer bookkeeping.
fn parse_iges_deck(content: &str) -> Vec<(usize, String)> {
    let lines: Vec<&str> = content.lines().collect();
    assert!(!lines.is_empty(), "the deck carries cards");
    let mut sections: Vec<(char, Vec<&str>)> = Vec::new();
    for line in &lines {
        assert!(line.is_ascii(), "cards are ASCII: {line:?}");
        assert_eq!(line.chars().count(), 80, "every card is exactly 80 columns: {line:?}");
        let letter = line.as_bytes()[72] as char;
        assert!(matches!(letter, 'S' | 'G' | 'D' | 'P' | 'T'), "unknown section `{letter}`");
        let number = line[73..].trim().parse::<usize>().expect("section sequence number");
        if sections.last().is_some_and(|(current, _)| *current == letter) {
            let cards = &mut sections.last_mut().expect("section").1;
            assert_eq!(number, cards.len() + 1, "sequence numbers count lines within the section");
            cards.push(line);
        }
        else {
            assert_eq!(number, 1, "a new section starts at line 1");
            sections.push((letter, vec![line]));
        }
    }
    let letters: Vec<char> = sections.iter().map(|(letter, _)| *letter).collect();
    assert_eq!(letters.first(), Some(&'S'), "the deck opens with the start section");
    assert_eq!(letters.last(), Some(&'T'), "the deck closes with the terminate section");
    let order = ['S', 'G', 'D', 'P', 'T'];
    let mut cursor = 0usize;
    for letter in &letters {
        let position = order[cursor..]
            .iter()
            .position(|current| current == letter)
            .unwrap_or_else(|| panic!("sections must follow the order {order:?}: {letters:?}"));
        cursor += position + 1;
    }
    let section = |letter: char| sections.iter().find(|(current, _)| *current == letter).map(|(_, cards)| cards.clone()).unwrap_or_default();
    let directory = section('D');
    let parameter = section('P');

    let mut entities: Vec<(usize, String)> = Vec::new();
    let mut card = 0usize;
    let mut parameter_card = 0usize;
    while card < directory.len() {
        let pointer = directory[card][64..72].trim().parse::<i64>().expect("parameter pointer");
        assert_eq!(pointer as usize, card + 2, "the first directory line points at the structure line of its entry");
        let structure = directory[card + 1];
        let entity_type = structure[..8].trim().parse::<usize>().expect("entity type");
        let structure_pointer = structure[8..16].trim().parse::<i64>().expect("structure pointer");
        assert_eq!(structure_pointer.unsigned_abs() as usize, card + 1, "the structure line points back at the entry");
        let line_count = structure[16..23].trim().parse::<usize>().expect("line count");
        let mut data = String::new();
        for offset in 0..line_count {
            let back = parameter[parameter_card + offset][64..72].trim().parse::<i64>().expect("parameter back pointer");
            assert_eq!(back.unsigned_abs() as usize, card + 1, "every parameter card points back at its directory entry");
            data.push_str(parameter[parameter_card + offset][..64].trim_end());
        }
        parameter_card += line_count;
        entities.push((entity_type, data));
        card += 2;
    }
    assert_eq!(card, directory.len(), "directory cards cover complete entries");
    assert_eq!(parameter_card, parameter.len(), "every parameter card belongs to an entry");

    let terminate_line = section('T')[0];
    let terminate: Vec<&str> = terminate_line[..72].trim_end().split(',').map(str::trim).collect();
    assert_eq!(terminate[0].parse::<usize>().expect("entity count"), entities.len(), "the terminate section counts entities");
    assert_eq!(terminate[2].parse::<usize>().expect("D line count"), directory.len(), "the terminate section counts directory lines");
    assert_eq!(terminate[16].parse::<usize>().expect("P1"), parameter.len(), "the terminate section points at the last parameter line");
    entities
}

/// Evaluates one rational or non-rational Type 126 curve at `t` in `[0, 1]`.
///
/// The writer emits clamped knot vectors whose interior knots repeat `degree`
/// times, so every knot span is one Bezier piece and span `k` uses control
/// points `k * degree ..= k * degree + degree`.
fn iges_bspline_point(data: &str, t: f64) -> [f64; 2] {
    let fields: Vec<f64> = data[4..data.len() - 1].split(',').map(|field| field.trim().parse().expect("numeric parameter")).collect();
    let degree = fields[0] as usize;
    let n = fields[1] as usize;
    let rational = fields[5] as usize == 1;
    let mut controls = vec![[0.0, 0.0]; n + 1];
    controls[0] = [fields[8], fields[9]];
    controls[n] = [fields[11], fields[12]];
    let mut cursor = 14usize;
    for control in controls.iter_mut().take(n).skip(1) {
        *control = [fields[cursor], fields[cursor + 1]];
        cursor += 3;
    }
    let weights: Vec<f64> = if rational { (0..=n).map(|index| fields[cursor + index]).collect() } else { vec![1.0; n + 1] };
    let segments = n / degree;
    let span = ((t * segments as f64).floor().min(segments as f64 - 1.0)) as usize;
    let local = t * segments as f64 - span as f64;
    let one = 1.0 - local;
    let basis: Vec<f64> = match degree {
        2 => vec![one * one, 2.0 * local * one, local * local],
        3 => vec![one * one * one, 3.0 * local * one * one, 3.0 * local * local * one, local * local * local],
        other => panic!("unexpected degree {other}"),
    };
    let mut point = [0.0, 0.0];
    let mut weight_sum = 0.0;
    for (handle, basis) in basis.iter().enumerate() {
        let index = span * degree + handle;
        point[0] += basis * weights[index] * controls[index][0];
        point[1] += basis * weights[index] * controls[index][1];
        weight_sum += basis * weights[index];
    }
    [point[0] / weight_sum, point[1] / weight_sum]
}

fn iges_entity(id: &str, kind: EntityKind) -> Entity {
    Entity { id: id.to_string(), layer: "0".to_string(), kind }
}

#[test]
fn iges_deck_is_a_well_formed_deterministic_projection() {
    let mut drawing = Drawing::empty(Some("IGES".to_string()), Units::Mm);
    drawing.entities.push(iges_entity("line", EntityKind::Line { from: [1.0, 2.0], to: [3.0, 4.0] }));
    drawing.entities.push(iges_entity("circle", EntityKind::Circle { center: [0.0, 0.0], radius: 5.0 }));
    drawing.entities.push(iges_entity("arc", EntityKind::Arc { center: [0.0, 0.0], radius: 5.0, start_angle: 0.0, end_angle: 90.0 }));
    drawing.entities.push(iges_entity("polyline", EntityKind::Polyline { points: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]], closed: true }));
    drawing
        .entities
        .push(iges_entity("spline", EntityKind::Spline { points: vec![[0.0, 0.0], [1.0, 2.0], [2.0, 0.0], [3.0, 2.0]], closed: false }));
    drawing.entities.push(iges_entity("text", EntityKind::Text { at: [1.0, 1.0], height: 2.0, content: "NOTE".to_string() }));
    drawing
        .entities
        .push(iges_entity("dimension", EntityKind::Dimension { from: [0.0, 0.0], to: [10.0, 0.0], offset: 2.0, text: None, style: None }));

    let module = iges(&drawing);
    let entities = parse_iges_deck(&module.content);
    assert_eq!(module.entities, entities.len());
    // line + circle + arc + 3 closed polyline segments + spline = 7 IGES entities.
    assert_eq!(module.entities, 7, "annotations are not written: {entities:?}");
    assert!(entities.iter().all(|(kind, data)| data.starts_with(&format!("{kind},")) && data.ends_with(';')));
    assert_eq!(entities[0], (110, "110,1,2,0,3,4,0;".to_string()));
    assert_eq!(entities[1], (112, "112,0,0,0,5,0,0,5;".to_string()));
    assert_eq!(iges(&drawing).content, module.content, "the deck is deterministic");
    assert_eq!(module.losses.len(), 1, "one aggregated annotation loss: {:?}", module.losses);
    assert_eq!(module.losses[0].code, "iges/unsupported-entity");
}

#[test]
fn iges_arc_is_an_exact_rational_quadratic_bspline() {
    let mut drawing = Drawing::empty(Some("IGES".to_string()), Units::Mm);
    drawing.entities.push(iges_entity("arc", EntityKind::Arc { center: [0.0, 0.0], radius: 5.0, start_angle: 0.0, end_angle: 90.0 }));
    let entities = parse_iges_deck(&iges(&drawing).content);
    assert_eq!(entities.len(), 1);
    let (kind, data) = &entities[0];
    assert_eq!(*kind, 126, "a partial arc becomes a B-spline");
    let start = iges_bspline_point(data, 0.0);
    let middle = iges_bspline_point(data, 0.5);
    let end = iges_bspline_point(data, 1.0);
    for point in [start, middle, end] {
        let distance = (point[0] * point[0] + point[1] * point[1]).sqrt();
        assert!((distance - 5.0).abs() < 1e-3, "the curve stays on the circle: {point:?} -> {distance}");
    }
    assert!((start[0] - 5.0).abs() < 1e-9 && start[1].abs() < 1e-9, "starts at 0 degrees: {start:?}");
    assert!(middle[0] > 0.0 && middle[1] > 0.0, "sweeps the first quadrant: {middle:?}");
    assert!(end[0].abs() < 1e-3 && (end[1] - 5.0).abs() < 1e-3, "ends at 90 degrees: {end:?}");
}

#[test]
fn iges_long_arc_splits_into_quarter_turn_segments() {
    let mut drawing = Drawing::empty(Some("IGES".to_string()), Units::Mm);
    drawing.entities.push(iges_entity("long-arc", EntityKind::Arc { center: [2.0, 3.0], radius: 4.0, start_angle: 10.0, end_angle: 300.0 }));
    let entities = parse_iges_deck(&iges(&drawing).content);
    let (kind, data) = &entities[0];
    assert_eq!(*kind, 126);
    let fields: Vec<f64> = data[4..data.len() - 1].split(',').map(|field| field.trim().parse().expect("numeric")).collect();
    assert_eq!(fields[0], 2.0, "quadratic");
    assert_eq!(fields[1], 8.0, "a 290 degree sweep splits into four quarter turns with 2 * 4 + 1 control points");
    for t in [0.0, 0.17, 0.5, 0.83, 1.0] {
        let point = iges_bspline_point(data, t);
        let distance = ((point[0] - 2.0) * (point[0] - 2.0) + (point[1] - 3.0) * (point[1] - 3.0)).sqrt();
        assert!((distance - 4.0).abs() < 1e-3, "every sampled point lies on the arc: t={t} -> {point:?}");
    }
}

#[test]
fn iges_full_sweep_arc_becomes_a_circle() {
    let mut drawing = Drawing::empty(Some("IGES".to_string()), Units::Mm);
    drawing.entities.push(iges_entity("closed-arc", EntityKind::Arc { center: [1.0, 1.0], radius: 2.0, start_angle: 0.0, end_angle: 0.0 }));
    let entities = parse_iges_deck(&iges(&drawing).content);
    assert_eq!(entities[0].0, 112, "a zero-length sweep means the full circle");
    assert_eq!(entities[0].1, "112,1,1,0,3,1,0,2;");
}

#[test]
fn iges_spline_interpolates_its_fit_points() {
    let mut drawing = Drawing::empty(Some("IGES".to_string()), Units::Mm);
    let points = vec![[0.0, 0.0], [1.0, 2.0], [2.0, 0.0], [3.0, 2.0]];
    drawing.entities.push(iges_entity("spline", EntityKind::Spline { points: points.clone(), closed: false }));
    let entities = parse_iges_deck(&iges(&drawing).content);
    let (kind, data) = &entities[0];
    assert_eq!(*kind, 126);
    let fields: Vec<f64> = data[4..data.len() - 1].split(',').map(|field| field.trim().parse().expect("numeric")).collect();
    assert_eq!(fields[0], 3.0, "cubic");
    assert_eq!(fields[5], 0.0, "non-rational");
    assert_eq!(fields[1], 9.0, "three spans give 3 * 3 + 1 control points");
    for (index, expected) in points.iter().enumerate() {
        let t = index as f64 / (points.len() - 1) as f64;
        let point = iges_bspline_point(data, t);
        assert!(
            (point[0] - expected[0]).abs() < 1e-3 && (point[1] - expected[1]).abs() < 1e-3,
            "the curve passes through fit point {index}: {point:?} != {expected:?}"
        );
    }
}

#[test]
fn iges_full_ellipse_is_analytic_and_partial_ellipses_are_flattened() {
    let mut drawing = Drawing::empty(Some("IGES".to_string()), Units::Mm);
    drawing.entities.push(iges_entity(
        "full-ellipse",
        EntityKind::Ellipse { center: [0.0, 0.0], rx: 4.0, ry: 2.0, rotation: 0.0, start_angle: 0.0, end_angle: 0.0 },
    ));
    let entities = parse_iges_deck(&iges(&drawing).content);
    assert_eq!(entities[0].0, 114);
    assert_eq!(entities[0].1, "114,0,0,0,4,0,0,2,0.5;");

    let mut drawing = Drawing::empty(Some("IGES".to_string()), Units::Mm);
    drawing.entities.push(iges_entity(
        "half-ellipse",
        EntityKind::Ellipse { center: [0.0, 0.0], rx: 4.0, ry: 2.0, rotation: 0.0, start_angle: 0.0, end_angle: 180.0 },
    ));
    let module = iges(&drawing);
    assert_eq!(module.losses.len(), 1, "the flattening is reported: {:?}", module.losses);
    assert_eq!(module.losses[0].code, "iges/ellipse-polyline");
    let entities = parse_iges_deck(&module.content);
    assert!(entities.iter().all(|(kind, _)| *kind == 110), "the partial ellipse becomes line segments");
    assert!(entities.len() > 2);
}

#[test]
fn iges_empty_drawing_is_a_valid_deck() {
    let module = iges(&Drawing::empty(None, Units::Mm));
    assert_eq!(module.entities, 0);
    assert!(module.losses.is_empty(), "nothing is dropped: {:?}", module.losses);
    assert!(parse_iges_deck(&module.content).is_empty());
}

#[test]
fn iges_layer_info_loss_appears_only_with_several_layers() {
    let mut drawing = Drawing::empty(Some("IGES".to_string()), Units::Mm);
    drawing.entities.push(iges_entity("first", EntityKind::Line { from: [0.0, 0.0], to: [1.0, 1.0] }));
    assert!(iges(&drawing).losses.iter().all(|loss| loss.code != "iges/layer-info"), "the built-in layer pair carries no loss");
    drawing.layers.push(crate::Layer { id: "extra".to_string(), name: "extra".to_string(), ..crate::Layer::default_geometry() });
    drawing.entities.push(iges_entity("second", EntityKind::Line { from: [1.0, 1.0], to: [2.0, 2.0] }));
    let module = iges(&drawing);
    assert!(module.losses.iter().any(|loss| loss.code == "iges/layer-info"), "custom layer styling is dropped");
}

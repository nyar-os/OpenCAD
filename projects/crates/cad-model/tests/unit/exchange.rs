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
    let projection =
        Projection { map: &map, scale: 1.0, stroke: 0.5, lineweight_scale: 1.0, label_offset: 1.0, label_size: 1.75, dim_styles: &drawing.dim_styles };
    let flat = build_drawing(&drawing, FlatSize::new(frame.width, frame.height), &projection);
    let arrows = flat.elements.iter().filter(|element| matches!(&element.shape, Shape::Polygon { .. })).collect::<Vec<_>>();
    assert_eq!(arrows.len(), 6, "two linear, one radial, one ordinate and two angular arrowheads");
    assert!(arrows.iter().all(|element| element.style.fill.is_some() && element.style.stroke.is_none()), "every arrowhead is a solid outline-free triangle");
    let dots = flat.elements.iter().filter(|element| matches!(&element.shape, Shape::Circle { .. })).count();
    assert_eq!(dots, 1, "the radius dimension marks its centre");
}

#[test]
fn dimension_arrow_follows_the_referenced_style() {
    let mut drawing = Drawing::empty(None, Units::Mm);
    drawing.dim_styles.push(DimensionStyle {
        id: "big".to_string(),
        name: "big".to_string(),
        arrow_size: 10.0,
        ..DimensionStyle::standard()
    });
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
    let projection =
        Projection { map: &map, scale: 1.0, stroke: 0.5, lineweight_scale: 1.0, label_offset: 1.0, label_size: 1.75, dim_styles: &drawing.dim_styles };
    let flat = build_drawing(&drawing, FlatSize::new(frame.width, frame.height), &projection);
    let widths = flat
        .elements
        .iter()
        .filter_map(|element| {
            let Shape::Polygon { points } = &element.shape else {
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

//! Preview driver: renders every generated drawing kind to SVG and PDF.
//!
//! The output directory defaults to `views-preview`; pass another path as the
//! first argument. Every drawing is built from the declarative part model so
//! the same deterministic input can be re-exported while tuning the layout.

use std::path::Path;

use open_cad_drawing::{
    AssemblyComponent, Axis3, CreateRequest, ExplodedAssemblyRequest, Feature3d, HoleTableRequest, IsometricViewRequest, LeaderNoteRequest,
    MultiViewSheetRequest, PartSpec, SectionViewRequest, TitleBlockRequest, add_circle, add_exploded_assembly, add_hole_table,
    add_isometric_view, add_leader_note, add_line, add_multiview_sheet, add_section_view, add_text, add_title_block, create, export,
};

fn bracket() -> PartSpec {
    PartSpec {
        solids: vec![
            Feature3d::Box { min: [0.0, 0.0, 0.0], max: [80.0, 50.0, 20.0] },
            Feature3d::Cylinder { axis: Axis3::Z, center: [20.0, 25.0], from: 20.0, to: 42.0, radius: 14.0 },
        ],
        voids: vec![
            Feature3d::Cylinder { axis: Axis3::Z, center: [20.0, 25.0], from: 0.0, to: 42.0, radius: 7.0 },
            Feature3d::Cylinder { axis: Axis3::Z, center: [10.0, 10.0], from: 0.0, to: 20.0, radius: 4.0 },
            Feature3d::Cylinder { axis: Axis3::Z, center: [70.0, 10.0], from: 0.0, to: 20.0, radius: 4.0 },
            Feature3d::Cylinder { axis: Axis3::Z, center: [10.0, 40.0], from: 0.0, to: 20.0, radius: 4.0 },
            Feature3d::Cylinder { axis: Axis3::Z, center: [70.0, 40.0], from: 0.0, to: 20.0, radius: 4.0 },
        ],
    }
}

fn flange() -> PartSpec {
    let mut voids = vec![Feature3d::Cylinder { axis: Axis3::Z, center: [0.0, 0.0], from: 0.0, to: 12.0, radius: 20.0 }];
    for step in 0..6 {
        let angle = (step * 60) as f64;
        let radians = angle.to_radians();
        voids.push(Feature3d::Cylinder {
            axis: Axis3::Z,
            center: [45.0 * radians.cos(), 45.0 * radians.sin()],
            from: 0.0,
            to: 12.0,
            radius: 6.0,
        });
    }
    PartSpec { solids: vec![Feature3d::Cylinder { axis: Axis3::Z, center: [0.0, 0.0], from: 0.0, to: 12.0, radius: 60.0 }], voids }
}

fn assembly() -> Vec<AssemblyComponent> {
    let base = PartSpec { solids: vec![Feature3d::Box { min: [0.0, 0.0, 0.0], max: [80.0, 50.0, 10.0] }], voids: Vec::new() };
    let gasket = PartSpec { solids: vec![Feature3d::Box { min: [0.0, 0.0, 0.0], max: [80.0, 50.0, 2.0] }], voids: Vec::new() };
    let cover = PartSpec {
        solids: vec![Feature3d::Box { min: [0.0, 0.0, 0.0], max: [80.0, 50.0, 12.0] }],
        voids: vec![Feature3d::Cylinder { axis: Axis3::Z, center: [40.0, 25.0], from: 0.0, to: 12.0, radius: 14.0 }],
    };
    vec![
        AssemblyComponent {
            name: Some("BASE".to_string()),
            part: base,
            position: Some([0.0; 3]),
            explode: Some([0.0; 3]),
            quantity: Some(1),
            material: Some("STEEL".to_string()),
        },
        AssemblyComponent {
            name: Some("GASKET".to_string()),
            part: gasket,
            position: Some([0.0, 0.0, 10.0]),
            explode: Some([0.0, 0.0, 40.0]),
            quantity: Some(1),
            material: Some("RUBBER".to_string()),
        },
        AssemblyComponent {
            name: Some("COVER".to_string()),
            part: cover,
            position: Some([0.0, 0.0, 12.0]),
            explode: Some([0.0, 0.0, 80.0]),
            quantity: Some(1),
            material: Some("STEEL".to_string()),
        },
    ]
}

fn export_both(handle: u64, out: &Path, stem: &str) -> Result<(), Box<dyn std::error::Error>> {
    for format in ["svg", "pdf"] {
        let target = out.join(format!("{stem}.{format}"));
        let report = export(handle, format, &target)?;
        println!("{stem}.{format}: {report}");
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args().nth(1).unwrap_or_else(|| "views-preview".into());
    let out = Path::new(&out);
    std::fs::create_dir_all(out)?;

    let created = create(CreateRequest { title: Some("OpenCAD bracket".into()), units: Some("mm".into()), template: None })?;
    let handle = created["handle"].as_u64().expect("handle");
    add_multiview_sheet(handle, MultiViewSheetRequest { part: bracket(), projection: Some("first".to_string()), ..Default::default() })?;
    export_both(handle, out, "multiview-bracket")?;

    let created = create(CreateRequest { title: Some("OpenCAD flange section".into()), units: Some("mm".into()), template: None })?;
    let handle = created["handle"].as_u64().expect("handle");
    add_section_view(handle, SectionViewRequest { part: flange(), axis: Some(Axis3::Y), at: Some(0.0), ..Default::default() })?;
    export_both(handle, out, "section-flange")?;

    let created = create(CreateRequest { title: Some("OpenCAD leader note".into()), units: Some("mm".into()), template: None })?;
    let handle = created["handle"].as_u64().expect("handle");
    add_circle(handle, "0", [60.0, 60.0], 40.0)?;
    add_circle(handle, "0", [60.0, 60.0], 12.0)?;
    add_line(handle, "dimensions", [10.0, 60.0], [110.0, 60.0])?;
    add_line(handle, "dimensions", [60.0, 10.0], [60.0, 110.0])?;
    add_leader_note(
        handle,
        LeaderNoteRequest { points: vec![[28.28, 31.72], [18.0, 22.0], [4.0, 22.0]], text: "R40".to_string(), ..Default::default() },
    )?;
    add_leader_note(
        handle,
        LeaderNoteRequest { points: vec![[68.49, 51.51], [88.0, 34.0], [102.0, 34.0]], text: "BORE 24".to_string(), ..Default::default() },
    )?;
    export_both(handle, out, "leader-notes")?;

    let created = create(CreateRequest { title: Some("OpenCAD isometric".into()), units: Some("mm".into()), template: None })?;
    let handle = created["handle"].as_u64().expect("handle");
    add_isometric_view(handle, IsometricViewRequest { part: bracket(), ..Default::default() })?;
    export_both(handle, out, "isometric-bracket")?;

    let created = create(CreateRequest { title: Some("OpenCAD exploded".into()), units: Some("mm".into()), template: None })?;
    let handle = created["handle"].as_u64().expect("handle");
    add_exploded_assembly(handle, ExplodedAssemblyRequest { components: assembly(), ..Default::default() })?;
    export_both(handle, out, "exploded-cover")?;

    let created = create(CreateRequest { title: Some("OpenCAD bracket holes".into()), units: Some("mm".into()), template: None })?;
    let handle = created["handle"].as_u64().expect("handle");
    add_hole_table(handle, HoleTableRequest { part: bracket(), ..Default::default() })?;
    export_both(handle, out, "hole-table")?;

    let created = create(CreateRequest { title: Some("OpenCAD A4 sheet".into()), units: Some("mm".into()), template: None })?;
    let handle = created["handle"].as_u64().expect("handle");
    add_circle(handle, "0", [120.0, 110.0], 40.0)?;
    add_text(handle, "0", [52.0, 170.0], 5.0, "SHEET FRAME AND TITLE BLOCK")?;
    add_title_block(
        handle,
        TitleBlockRequest {
            format: Some("a4".to_string()),
            title: Some("Bracket assembly".to_string()),
            number: Some("A-101".to_string()),
            revision: Some("B".to_string()),
            scale: Some("1:2".to_string()),
            sheet: Some("1 / 1".to_string()),
            ..Default::default()
        },
    )?;
    export_both(handle, out, "sheet-a4")?;

    let created = create(CreateRequest { title: Some("OpenCAD A3 sheet".into()), units: Some("mm".into()), template: None })?;
    let handle = created["handle"].as_u64().expect("handle");
    add_circle(handle, "0", [180.0, 150.0], 60.0)?;
    add_text(handle, "0", [110.0, 250.0], 6.0, "A3 SHEET WITH REVISION BLOCK")?;
    add_title_block(
        handle,
        TitleBlockRequest {
            format: Some("a3".to_string()),
            title: Some("Gearbox housing".to_string()),
            number: Some("A3-204".to_string()),
            revision: Some("C".to_string()),
            scale: Some("1:1".to_string()),
            projection: Some("third".to_string()),
            ..Default::default()
        },
    )?;
    export_both(handle, out, "sheet-a3")?;

    Ok(())
}

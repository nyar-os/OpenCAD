//! Temporary driver: builds the demo flange and exports every supported format.

use std::path::Path;

use open_cad_drawing::{
    CreateRequest, PropertiesUpdate, add_circle, add_line, add_radial_dimension, add_text, create, export, update_properties,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args().nth(1).unwrap_or_else(|| "export-all".into());
    let out = Path::new(&out);
    std::fs::create_dir_all(out)?;

    let created = create(CreateRequest { title: Some("OpenCAD demo flange".into()), units: Some("mm".into()), template: None })?;
    let handle = created["handle"].as_u64().expect("handle");
    update_properties(
        handle,
        PropertiesUpdate {
            title: Some("OpenCAD demo flange".into()),
            author: Some("ACME Engineering".into()),
            company: Some("ACME".into()),
            project: Some("Flange 120".into()),
            ..Default::default()
        },
    )?;

    let center = [60.0, 60.0];
    let plate_radius = 60.0;
    let bore_radius = 20.0;
    let bolt_circle_radius = 45.0;
    let hole_radius = 6.0;

    add_circle(handle, "0", center, plate_radius)?;
    add_circle(handle, "0", center, bore_radius)?;
    for step in 0..6 {
        let angle = (step * 60) as f64;
        let radians = angle.to_radians();
        let at = [center[0] + bolt_circle_radius * radians.cos(), center[1] + bolt_circle_radius * radians.sin()];
        add_circle(handle, "0", at, hole_radius)?;
    }
    add_line(handle, "dimensions", [0.0, 60.0], [120.0, 60.0])?;
    add_line(handle, "dimensions", [60.0, 0.0], [60.0, 120.0])?;
    add_radial_dimension(handle, "dimensions", center, plate_radius, 45.0, 12.0, true, None)?;
    add_radial_dimension(handle, "dimensions", center, bore_radius, 135.0, 12.0, true, None)?;
    add_radial_dimension(handle, "dimensions", center, bolt_circle_radius, 150.0, 12.0, true, None)?;
    add_text(handle, "dimensions", [4.0, 130.0], 3.0, "FLANGE DIA 120 / 6 HOLES DIA 12 ON BOLT CIRCLE DIA 90")?;

    for format in ["svg", "dxf", "dxf-r12", "pdf", "stl", "obj", "step"] {
        let target = out.join(format!("demo-flange.{format}"));
        match export(handle, format, &target) {
            Ok(report) => println!("OK {format}: {report}"),
            Err(err) => println!("ERR {format}: {}", err.message),
        }
    }
    Ok(())
}

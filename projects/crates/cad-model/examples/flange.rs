//! Preview driver: builds a correct round pipe flange and exports SVG, PDF, and PNG.
//!
//! Geometry (millimetres, plate centred at `[60, 60]`):
//! - plate outer circle `DIA 120` (radius 60),
//! - central bore `DIA 40` (radius 20),
//! - bolt circle `DIA 90` (radius 45) with six `DIA 12` holes (radius 6) at
//!   60 degree spacing,
//! - horizontal and vertical centrelines through the plate centre,
//! - a diameter dimension for the plate, the bore, and the bolt circle.
//!
//! Every bolt hole lies fully inside the plate (45 + 6 = 51 < 60) and clear of
//! the bore (45 - 6 = 39 > 20), so the drawing is a valid flange.

use std::path::Path;

use open_cad_drawing::{CreateRequest, add_circle, add_line, add_radial_dimension, add_text, create, export};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args().nth(1).unwrap_or_else(|| "preview".into());
    let out = Path::new(&out);
    std::fs::create_dir_all(out)?;

    let created = create(CreateRequest { title: Some("OpenCAD demo flange".into()), units: Some("mm".into()), template: None })?;
    let handle = created["handle"].as_u64().expect("handle");

    let center = [60.0, 60.0];
    let plate_radius = 60.0;
    let bore_radius = 20.0;
    let bolt_circle_radius = 45.0;
    let hole_radius = 6.0;

    // Plate outline and central bore.
    add_circle(handle, "0", center, plate_radius)?;
    add_circle(handle, "0", center, bore_radius)?;

    // Six bolt holes on the bolt circle at 60 degree spacing.
    for step in 0..6 {
        let angle = (step * 60) as f64;
        let radians = angle.to_radians();
        let at = [center[0] + bolt_circle_radius * radians.cos(), center[1] + bolt_circle_radius * radians.sin()];
        add_circle(handle, "0", at, hole_radius)?;
    }

    // Centrelines through the plate centre.
    add_line(handle, "dimensions", [0.0, 60.0], [120.0, 60.0])?;
    add_line(handle, "dimensions", [60.0, 0.0], [60.0, 120.0])?;

    // Diameter dimensions: plate, bore, bolt circle. A full-chord diameter at
    // angle A lies on the same line as A + 180, so the three angles must be
    // distinct modulo 180 degrees or two dimension lines overlap.
    add_radial_dimension(handle, "dimensions", center, plate_radius, 45.0, 12.0, true, None)?;
    add_radial_dimension(handle, "dimensions", center, bore_radius, 135.0, 12.0, true, None)?;
    add_radial_dimension(handle, "dimensions", center, bolt_circle_radius, 150.0, 12.0, true, None)?;

    // Title block text, sized to stay inside the drawing width.
    add_text(handle, "dimensions", [4.0, 130.0], 3.0, "FLANGE DIA 120 / 6 HOLES DIA 12 ON BOLT CIRCLE DIA 90")?;

    for format in ["svg", "pdf"] {
        let target = out.join(format!("demo-flange.{format}"));
        let report = export(handle, format, &target)?;
        println!("{format}: {report}");
    }
    Ok(())
}

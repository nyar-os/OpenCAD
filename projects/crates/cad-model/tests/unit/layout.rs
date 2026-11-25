//! Hard layout checks over the exported SVG sheet.
//!
//! Each generated drawing is parsed back out of its SVG and checked against the
//! fixed page contract: a 297 x 210 mm A4 landscape sheet, every geometry and
//! text bound inside the inner border, table text inside its cell, geometry
//! clear of the title block, and enough of the drawable area covered that the
//! sheet is not a small drawing in a corner.

use crate::{
    AssemblyComponent, AuxiliaryViewRequest, Axis3, CreateRequest, DetailViewRequest, ExplodedAssemblyRequest, Feature3d, HoleTableRequest,
    IsometricViewRequest, LeaderNoteRequest, MultiViewSheetRequest, PartSpec, SectionViewRequest, TitleBlockRequest, add_auxiliary_view,
    add_circle, add_detail_view, add_exploded_assembly, add_hole_table, add_isometric_view, add_leader_note, add_line, add_multiview_sheet,
    add_section_view, add_text, add_title_block, create, schema::Drawing,
};

const PAGE_WIDTH: f64 = 297.0;
const PAGE_HEIGHT: f64 = 210.0;
const INNER_MARGIN: f64 = 10.0;
/// Allowed slack for the inner border and table cells, in millimetres.
const TOLERANCE: f64 = 1.0;
/// Drawable box of the sheet in millimetres: the inner border minus the 4 mm fit
/// gap, with the title block band excluded.
const AREA_LEFT: f64 = 14.0;
const AREA_TOP: f64 = 14.0;
const AREA_WIDTH: f64 = 269.0;
const AREA_HEIGHT: f64 = 158.0;
/// Top edge of the title block, so no drawing content may reach below it.
const TITLE_BLOCK_TOP: f64 = 176.0;
/// Ink used for the sheet frame and title block; those shapes are furniture
/// rather than drawing content.
const FURNITURE_INK: &str = "#1f2933";
/// Approximate glyph advance used to estimate a rendered text width.
const GLYPH_FACTOR: f64 = 0.6;

#[derive(Debug, Clone)]
struct TextItem {
    left: f64,
    right: f64,
    baseline: f64,
    size: f64,
    content: String,
}

#[derive(Debug, Clone, Copy)]
struct Box2 {
    min: [f64; 2],
    max: [f64; 2],
}

#[derive(Debug, Default)]
struct Parsed {
    view_box: [f64; 4],
    width: f64,
    height: f64,
    segments: Vec<([f64; 2], [f64; 2])>,
    boxes: Vec<Box2>,
    texts: Vec<TextItem>,
    /// Bounds of the sheet furniture, for the page-level check.
    furniture: Vec<Box2>,
}

impl Parsed {
    /// Bounding box of the drawing content, leaving out the sheet furniture.
    fn content_bounds(&self) -> Option<Box2> {
        let mut bounds: Option<Box2> = None;
        let mut grow = |min: [f64; 2], max: [f64; 2]| {
            let item = Box2 { min, max };
            bounds = Some(match bounds {
                Some(current) => Box2 {
                    min: [current.min[0].min(item.min[0]), current.min[1].min(item.min[1])],
                    max: [current.max[0].max(item.max[0]), current.max[1].max(item.max[1])],
                },
                None => item,
            });
        };
        for (from, to) in &self.segments {
            grow([from[0].min(to[0]), from[1].min(to[1])], [from[0].max(to[0]), from[1].max(to[1])]);
        }
        for item in &self.boxes {
            grow(item.min, item.max);
        }
        for text in &self.texts {
            grow([text.left, text.baseline - text.size], [text.right, text.baseline + text.size * 0.3]);
        }
        bounds
    }

    /// X of the nearest vertical rule to the right of `text` that vertically
    /// covers the text band, or the inner border when the row has no rule.
    fn next_vertical_rule(&self, text: &TextItem) -> f64 {
        let top = text.baseline - text.size * 1.05;
        let bottom = text.baseline + text.size * 0.3;
        let mut boundary = PAGE_WIDTH - INNER_MARGIN;
        for (from, to) in &self.segments {
            if (from[0] - to[0]).abs() > 1e-9 || from[0] <= text.left + 1e-9 {
                continue;
            }
            let low = from[1].min(to[1]);
            let high = from[1].max(to[1]);
            if high < top || low > bottom {
                continue;
            }
            boundary = boundary.min(from[0]);
        }
        boundary
    }
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')? + start;
    Some(&tag[start..end])
}

fn number_attr(tag: &str, name: &str) -> Option<f64> {
    attr(tag, name)?.parse().ok()
}

fn unescape(value: &str) -> String {
    value.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

fn path_segments(data: &str) -> Vec<([f64; 2], [f64; 2])> {
    // The writer glues commands to their first number (`M61.01 125.4`), so
    // separate letters from numbers before tokenising.
    let spaced: String =
        data.chars().flat_map(|character| if character.is_alphabetic() { vec![' ', character, ' '] } else { vec![character] }).collect();
    let mut segments = Vec::new();
    let mut command = 'M';
    let mut current: Option<[f64; 2]> = None;
    let mut tokens = spaced.split(|character: char| character.is_whitespace() || character == ',').filter(|token| !token.is_empty());
    while let Some(token) = tokens.next() {
        if token.chars().all(char::is_alphabetic) {
            command = token.chars().next().unwrap_or('M');
            if command == 'Z' || command == 'z' {
                current = None;
            }
            continue;
        }
        let Some(x) = token.parse::<f64>().ok()
        else {
            continue;
        };
        let Some(y) = tokens.next().and_then(|value| value.parse::<f64>().ok())
        else {
            break;
        };
        let point = [x, y];
        match command {
            'M' => current = Some(point),
            'L' => {
                if let Some(previous) = current {
                    segments.push((previous, point));
                }
                current = Some(point);
            }
            'Z' | 'z' => current = None,
            _ => current = Some(point),
        }
    }
    segments
}

fn is_furniture(tag: &str) -> bool {
    [attr(tag, "stroke"), attr(tag, "fill")].into_iter().flatten().any(|color| color.eq_ignore_ascii_case(FURNITURE_INK))
}

fn push_box(parsed: &mut Parsed, tag: &str, min: [f64; 2], max: [f64; 2]) {
    if is_furniture(tag) {
        parsed.furniture.push(Box2 { min, max });
    }
    else {
        parsed.boxes.push(Box2 { min, max });
    }
}

fn parse(svg: &str) -> Parsed {
    let root = &svg[..svg.find('>').unwrap_or(svg.len())];
    let view_box_text = attr(root, "viewBox").unwrap_or("0 0 0 0");
    let numbers: Vec<f64> = view_box_text.split_whitespace().filter_map(|value| value.parse().ok()).collect();
    let mut view_box = [0.0; 4];
    view_box[..numbers.len().min(4)].copy_from_slice(&numbers[..numbers.len().min(4)]);
    let mut parsed = Parsed {
        view_box,
        width: number_attr(root, "width").unwrap_or(0.0),
        height: number_attr(root, "height").unwrap_or(0.0),
        ..Parsed::default()
    };

    let mut index = 0usize;
    while let Some(offset) = svg[index..].find('<') {
        let start = index + offset;
        let Some(end_offset) = svg[start..].find('>')
        else {
            break;
        };
        let end = start + end_offset;
        let tag = &svg[start + 1..end];
        index = end + 1;
        if tag.starts_with('/') || tag.starts_with('!') || tag.starts_with('?') {
            continue;
        }
        let name = tag.split_whitespace().next().unwrap_or("");
        match name {
            "text" => {
                let close = svg[index..].find("</text>").map_or(svg.len(), |value| index + value);
                let content = unescape(&svg[index..close]);
                index = (close + "</text>".len()).min(svg.len());
                let Some(x) = number_attr(tag, "x")
                else {
                    continue;
                };
                let Some(baseline) = number_attr(tag, "y")
                else {
                    continue;
                };
                let size = number_attr(tag, "font-size").unwrap_or(0.0);
                let width = content.chars().count() as f64 * size * GLYPH_FACTOR;
                let (left, right) = match attr(tag, "text-anchor") {
                    Some("middle") => (x - width / 2.0, x + width / 2.0),
                    Some("end") => (x - width, x),
                    _ => (x, x + width),
                };
                let item = TextItem { left, right, baseline, size, content };
                if is_furniture(tag) {
                    parsed.furniture.push(Box2 { min: [left, baseline - size], max: [right, baseline + size * 0.3] });
                }
                else {
                    parsed.texts.push(item);
                }
            }
            "line" => {
                let (Some(x1), Some(y1), Some(x2), Some(y2)) =
                    (number_attr(tag, "x1"), number_attr(tag, "y1"), number_attr(tag, "x2"), number_attr(tag, "y2"))
                else {
                    continue;
                };
                if is_furniture(tag) {
                    parsed.furniture.push(Box2 { min: [x1.min(x2), y1.min(y2)], max: [x1.max(x2), y1.max(y2)] });
                }
                else {
                    parsed.segments.push(([x1, y1], [x2, y2]));
                }
            }
            "path" => {
                let Some(data) = attr(tag, "d")
                else {
                    continue;
                };
                let segments = path_segments(data);
                if is_furniture(tag) {
                    if let (Some(min_x), Some(max_x), Some(min_y), Some(max_y)) = (
                        segments.iter().flat_map(|(from, to)| [from[0], to[0]]).reduce(f64::min),
                        segments.iter().flat_map(|(from, to)| [from[0], to[0]]).reduce(f64::max),
                        segments.iter().flat_map(|(from, to)| [from[1], to[1]]).reduce(f64::min),
                        segments.iter().flat_map(|(from, to)| [from[1], to[1]]).reduce(f64::max),
                    ) {
                        parsed.furniture.push(Box2 { min: [min_x, min_y], max: [max_x, max_y] });
                    }
                }
                else {
                    parsed.segments.extend(segments);
                }
            }
            "ellipse" => {
                let (Some(cx), Some(cy), Some(rx), Some(ry)) =
                    (number_attr(tag, "cx"), number_attr(tag, "cy"), number_attr(tag, "rx"), number_attr(tag, "ry"))
                else {
                    continue;
                };
                push_box(&mut parsed, tag, [cx - rx, cy - ry], [cx + rx, cy + ry]);
            }
            "rect" => {
                let (Some(x), Some(y), Some(width), Some(height)) =
                    (number_attr(tag, "x"), number_attr(tag, "y"), number_attr(tag, "width"), number_attr(tag, "height"))
                else {
                    continue;
                };
                push_box(&mut parsed, tag, [x, y], [x + width, y + height]);
            }
            _ => {}
        }
    }
    parsed
}

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
        let radians = (step * 60) as f64 * std::f64::consts::PI / 180.0;
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

fn drawing_of(build: impl FnOnce(u64)) -> Drawing {
    let created = create(CreateRequest { units: Some("mm".to_string()), ..CreateRequest::default() }).expect("create");
    let handle = created["handle"].as_u64().expect("handle");
    build(handle);
    crate::store::read(handle, |session| session.drawing.clone()).expect("read drawing")
}

/// Every drawing exported through the default SVG path, one per generated kind
/// plus a plain drawing.
fn fixtures() -> Vec<(&'static str, Drawing)> {
    let mut fixtures = Vec::new();
    fixtures.push((
        "isometric",
        drawing_of(|handle| {
            add_isometric_view(handle, IsometricViewRequest { part: bracket(), ..Default::default() }).expect("isometric");
        }),
    ));
    fixtures.push((
        "multiview",
        drawing_of(|handle| {
            add_multiview_sheet(handle, MultiViewSheetRequest { part: bracket(), projection: Some("first".to_string()), ..Default::default() })
                .expect("multiview");
        }),
    ));
    fixtures.push((
        "exploded",
        drawing_of(|handle| {
            add_exploded_assembly(handle, ExplodedAssemblyRequest { components: assembly(), ..Default::default() }).expect("exploded");
        }),
    ));
    fixtures.push((
        "detail",
        drawing_of(|handle| {
            add_detail_view(
                handle,
                DetailViewRequest {
                    part: bracket(),
                    view: Some("top".to_string()),
                    center: Some([20.0, 25.0]),
                    radius: Some(24.0),
                    scale: Some(2.0),
                    ..Default::default()
                },
            )
            .expect("detail");
        }),
    ));
    fixtures.push((
        "auxiliary",
        drawing_of(|handle| {
            add_auxiliary_view(handle, AuxiliaryViewRequest { part: bracket(), angle: Some(45.0), ..Default::default() }).expect("auxiliary");
        }),
    ));
    fixtures.push((
        "section",
        drawing_of(|handle| {
            add_section_view(handle, SectionViewRequest { part: flange(), axis: Some(Axis3::Y), at: Some(0.0), ..Default::default() })
                .expect("section");
        }),
    ));
    fixtures.push((
        "hole-table",
        drawing_of(|handle| {
            add_hole_table(handle, HoleTableRequest { part: bracket(), ..Default::default() }).expect("hole table");
        }),
    ));
    fixtures.push((
        "leader-notes",
        drawing_of(|handle| {
            add_circle(handle, "0", [60.0, 60.0], 40.0).expect("outer circle");
            add_circle(handle, "0", [60.0, 60.0], 12.0).expect("bore");
            add_line(handle, "dimensions", [10.0, 60.0], [110.0, 60.0]).expect("horizontal");
            add_line(handle, "dimensions", [60.0, 10.0], [60.0, 110.0]).expect("vertical");
            add_leader_note(
                handle,
                LeaderNoteRequest { points: vec![[28.28, 31.72], [18.0, 22.0], [4.0, 22.0]], text: "R40".to_string(), ..Default::default() },
            )
            .expect("leader");
        }),
    ));
    fixtures.push((
        "plain",
        drawing_of(|handle| {
            add_line(handle, "0", [0.0, 0.0], [120.0, 80.0]).expect("line");
            add_text(handle, "0", [10.0, 90.0], 5.0, "PLAIN DRAWING").expect("text");
        }),
    ));
    fixtures
}

#[test]
fn generated_drawings_use_the_fixed_a4_sheet() {
    for (name, drawing) in fixtures() {
        let svg = crate::exchange::svg(&drawing);
        let parsed = parse(&svg);
        assert_eq!(parsed.view_box, [0.0, 0.0, PAGE_WIDTH, PAGE_HEIGHT], "{name}: the page is A4 landscape");
        assert!((parsed.width - PAGE_WIDTH * 8.0).abs() < 1e-6, "{name}: the pixel width follows the page millimetres");
        assert!((parsed.height - PAGE_HEIGHT * 8.0).abs() < 1e-6, "{name}: the pixel height follows the page millimetres");

        let bounds = parsed.content_bounds().unwrap_or_else(|| panic!("{name}: the drawing has content"));
        let left = INNER_MARGIN - TOLERANCE;
        let right = PAGE_WIDTH - INNER_MARGIN + TOLERANCE;
        let top = INNER_MARGIN - TOLERANCE;
        let bottom = PAGE_HEIGHT - INNER_MARGIN + TOLERANCE;
        assert!(bounds.min[0] >= left, "{name}: x {} escapes the inner border {left}", bounds.min[0]);
        assert!(bounds.max[0] <= right, "{name}: x {} escapes the inner border {right}", bounds.max[0]);
        assert!(bounds.min[1] >= top, "{name}: y {} escapes the inner border {top}", bounds.min[1]);
        assert!(bounds.max[1] <= bottom, "{name}: y {} escapes the inner border {bottom}", bounds.max[1]);
        assert!(bounds.max[1] <= TITLE_BLOCK_TOP + TOLERANCE, "{name}: y {} runs into the title block", bounds.max[1]);

        // The fitted drawing must use the drawable area instead of sitting in a
        // corner of an oversized page. A legitimately wide or tall view cannot
        // fill both axes, so require one filled axis and enough total area.
        let cover_width = (bounds.max[0] - bounds.min[0]) / AREA_WIDTH;
        let cover_height = (bounds.max[1] - bounds.min[1]) / AREA_HEIGHT;
        assert!(cover_width.max(cover_height) >= 0.9, "{name}: the drawing fills neither page axis ({cover_width:.2} x {cover_height:.2})");
        assert!(cover_width * cover_height >= 0.35, "{name}: the drawing covers only {:.2} of the drawable area", cover_width * cover_height);
        let centered_x = (bounds.min[0] - AREA_LEFT) - (AREA_LEFT + AREA_WIDTH - bounds.max[0]);
        let centered_y = (bounds.min[1] - AREA_TOP) - (AREA_TOP + AREA_HEIGHT - bounds.max[1]);
        assert!(centered_x.abs() <= 2.0, "{name}: the drawing is {centered_x:.2} mm off centre horizontally");
        assert!(centered_y.abs() <= 2.0, "{name}: the drawing is {centered_y:.2} mm off centre vertically");

        // Every text stays inside the table cell or band to its right.
        for text in &parsed.texts {
            let boundary = parsed.next_vertical_rule(text);
            assert!(
                text.right <= boundary + 0.2,
                "{name}: `{}` runs {:.2} mm past the cell edge {boundary:.2}",
                text.content,
                text.right - boundary
            );
        }

        // Even the sheet furniture stays on the paper.
        for item in &parsed.furniture {
            assert!(item.min[0] >= -TOLERANCE && item.max[0] <= PAGE_WIDTH + TOLERANCE, "{name}: furniture leaves the page horizontally");
            assert!(item.min[1] >= -TOLERANCE && item.max[1] <= PAGE_HEIGHT + TOLERANCE, "{name}: furniture leaves the page vertically");
        }
    }
}

#[test]
fn authored_a3_sheet_keeps_its_paper_size() {
    let drawing = drawing_of(|handle| {
        add_circle(handle, "0", [180.0, 150.0], 60.0).expect("circle");
        add_text(handle, "0", [110.0, 250.0], 6.0, "A3 SHEET").expect("text");
        add_title_block(handle, TitleBlockRequest { format: Some("a3".to_string()), title: Some("Housing".to_string()), ..Default::default() })
            .expect("title block");
    });
    let parsed = parse(&crate::exchange::svg(&drawing));
    assert_eq!(parsed.view_box, [0.0, 0.0, 420.0, 297.0], "an explicit A3 sheet keeps A3 landscape");
}

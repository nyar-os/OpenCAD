//! Deterministic raster output for the PNG export target.
//!
//! The rasteriser consumes the same flat drawing the PDF writer lowers onto,
//! so shape geometry, sheet framing and text placement are shared. Strokes,
//! fills and a built-in 5x7 bitmap font paint into an 8-bit RGB buffer, and
//! [`encode_png`] writes a self-contained PNG with uncompressed `deflate`
//! blocks. No compression library, no clock and no hash iteration are involved,
//! so identical input produces identical bytes.

use document_drawing::{Color, Drawing as FlatDrawing, Shape, Style};

/// Largest image edge the rasteriser accepts, in pixels.
const MAX_EDGE: usize = 8192;

/// Largest pixel count the rasteriser accepts.
const MAX_PIXELS: usize = 32_000_000;

/// One rendered PNG plus what the raster could not draw.
pub(crate) struct RasterReport {
    /// Encoded PNG bytes.
    pub(crate) content: Vec<u8>,
    /// Image width in pixels.
    pub(crate) width: usize,
    /// Image height in pixels.
    pub(crate) height: usize,
    /// Characters without a bitmap glyph.
    pub(crate) missing_glyphs: usize,
}

/// Renders a flat drawing to PNG at `dpi`.
///
/// `dpi` is pixels per inch, so the pixel scale is `dpi / 72`. Non-finite or
/// non-positive values and images beyond [`MAX_EDGE`] or [`MAX_PIXELS`] are
/// rejected with a message the caller maps to `session/argument-invalid`.
pub(crate) fn render(drawing: &FlatDrawing, dpi: f64) -> Result<RasterReport, String> {
    if !dpi.is_finite() || dpi <= 0.0 {
        return Err(String::from("`dpi` must be a positive finite number"));
    }
    let scale = dpi / 72.0;
    let width = (drawing.size.width * scale).round().max(1.0) as usize;
    let height = (drawing.size.height * scale).round().max(1.0) as usize;
    if width > MAX_EDGE || height > MAX_EDGE {
        return Err(format!("`dpi` produces a {width}x{height} image; the limit is {MAX_EDGE} pixels per edge"));
    }
    if width.saturating_mul(height) > MAX_PIXELS {
        return Err(format!("`dpi` produces a {width}x{height} image; the limit is {MAX_PIXELS} pixels"));
    }
    let mut canvas = Canvas::new(width, height);
    let mut missing_glyphs = 0;
    for element in &drawing.elements {
        paint(&mut canvas, &element.shape, &element.style, scale, &mut missing_glyphs);
    }
    Ok(RasterReport { content: encode_png(width, height, &canvas.pixels), width, height, missing_glyphs })
}

struct Paint {
    stroke: Option<[u8; 3]>,
    fill: Option<[u8; 3]>,
    width: f64,
    dash: Option<[f64; 2]>,
}

fn paint(canvas: &mut Canvas, shape: &Shape, style: &Style, scale: f64, missing: &mut usize) {
    let paint = Paint {
        stroke: style.stroke.map(channels),
        fill: style.fill.map(channels),
        width: (style.width * scale).max(1.0),
        dash: style.dash.map(|[on, off]| [on * scale, off * scale]),
    };
    match shape {
        Shape::Line { from, to } => {
            if let Some(color) = paint.stroke {
                canvas.stroke_path(&[point(*from, scale), point(*to, scale)], false, paint.width, color, paint.dash);
            }
        }
        Shape::Polyline { points } => {
            let path = points.iter().map(|value| point(*value, scale)).collect::<Vec<_>>();
            if let Some(color) = paint.stroke {
                canvas.stroke_path(&path, false, paint.width, color, paint.dash);
            }
        }
        Shape::Polygon { points } => {
            let path = points.iter().map(|value| point(*value, scale)).collect::<Vec<_>>();
            if let Some(color) = paint.fill {
                canvas.fill_polygon(&path, color);
            }
            if let Some(color) = paint.stroke {
                canvas.stroke_path(&path, true, paint.width, color, paint.dash);
            }
        }
        Shape::Circle { center, radius } => ellipse(canvas, *center, *radius, *radius, scale, &paint),
        Shape::Ellipse { center, radii } => ellipse(canvas, *center, radii.width, radii.height, scale, &paint),
        Shape::Arc { center, radius, start_degrees, end_degrees } => {
            if let Some(color) = paint.stroke {
                let path = arc_points(*center, *radius, *start_degrees, *end_degrees, scale);
                canvas.stroke_path(&path, false, paint.width, color, paint.dash);
            }
        }
        Shape::Text { at, text, size } => {
            if let Some(color) = paint.fill.or(paint.stroke) {
                canvas.draw_text(point(*at, scale), text, size * scale, color, missing);
            }
        }
    }
}

fn ellipse(canvas: &mut Canvas, center: document_drawing::Point, rx: f64, ry: f64, scale: f64, paint: &Paint) {
    let center = point(center, scale);
    let rx = rx * scale;
    let ry = ry * scale;
    let segments = ((rx.max(ry) * 0.7).ceil() as usize).clamp(24, 256);
    let path = (0..segments)
        .map(|index| {
            let angle = std::f64::consts::TAU * index as f64 / segments as f64;
            [center[0] + rx * angle.cos(), center[1] + ry * angle.sin()]
        })
        .collect::<Vec<_>>();
    if let Some(color) = paint.fill {
        canvas.fill_polygon(&path, color);
    }
    if let Some(color) = paint.stroke {
        canvas.stroke_path(&path, true, paint.width, color, paint.dash);
    }
}

fn arc_points(center: document_drawing::Point, radius: f64, start: f64, end: f64, scale: f64) -> Vec<[f64; 2]> {
    let center = point(center, scale);
    let radius = radius * scale;
    let sweep = (end - start).abs();
    let segments = ((sweep / 5.0).ceil() as usize).max(2);
    (0..=segments)
        .map(|index| {
            let angle = (start + (end - start) * index as f64 / segments as f64).to_radians();
            [center[0] + radius * angle.cos(), center[1] + radius * angle.sin()]
        })
        .collect()
}

fn point(value: document_drawing::Point, scale: f64) -> [f64; 2] {
    [value.x * scale, value.y * scale]
}

fn channels(color: Color) -> [u8; 3] {
    [color.red(), color.green(), color.blue()]
}

struct Canvas {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

impl Canvas {
    fn new(width: usize, height: usize) -> Self {
        Self { width, height, pixels: vec![255; width * height * 3] }
    }

    fn set(&mut self, x: i64, y: i64, color: [u8; 3]) {
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
            return;
        }
        let offset = (y as usize * self.width + x as usize) * 3;
        self.pixels[offset..offset + 3].copy_from_slice(&color);
    }

    fn stamp(&mut self, center: [f64; 2], radius: f64, color: [u8; 3]) {
        if radius <= 0.75 {
            self.set(center[0].round() as i64, center[1].round() as i64, color);
            return;
        }
        let reach = radius.ceil() as i64;
        let cx = center[0].round() as i64;
        let cy = center[1].round() as i64;
        let limit = radius * radius;
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                if (dx * dx + dy * dy) as f64 <= limit {
                    self.set(cx + dx, cy + dy, color);
                }
            }
        }
    }

    fn stroke_path(&mut self, points: &[[f64; 2]], closed: bool, width: f64, color: [u8; 3], dash: Option<[f64; 2]>) {
        if points.len() < 2 {
            return;
        }
        let radius = (width * 0.5).max(0.5);
        let mut travelled = 0.0;
        let count = if closed { points.len() } else { points.len() - 1 };
        for index in 0..count {
            travelled = self.stroke_segment(points[index], points[(index + 1) % points.len()], radius, color, dash, travelled);
        }
    }

    fn stroke_segment(&mut self, from: [f64; 2], to: [f64; 2], radius: f64, color: [u8; 3], dash: Option<[f64; 2]>, travelled: f64) -> f64 {
        let span = [to[0] - from[0], to[1] - from[1]];
        let length = (span[0] * span[0] + span[1] * span[1]).sqrt();
        if length <= f64::EPSILON {
            return travelled;
        }
        let steps = (length * 2.0).ceil().max(1.0) as usize;
        let period = dash.map(|[on, off]| on + off).filter(|value| *value > 0.0);
        for index in 0..=steps {
            let t = index as f64 / steps as f64;
            let distance = travelled + length * t;
            if let (Some(period), Some([on, _])) = (period, dash)
                && distance % period >= on
            {
                continue;
            }
            self.stamp([from[0] + span[0] * t, from[1] + span[1] * t], radius, color);
        }
        travelled + length
    }

    fn fill_polygon(&mut self, points: &[[f64; 2]], color: [u8; 3]) {
        if points.len() < 3 {
            return;
        }
        let mut min_y = f64::INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for point in points {
            min_y = min_y.min(point[1]);
            max_y = max_y.max(point[1]);
        }
        let start = min_y.floor().max(0.0) as i64;
        let end = max_y.ceil().min(self.height as f64) as i64;
        let mut crossings: Vec<f64> = Vec::new();
        for y in start..end {
            let scanline = y as f64 + 0.5;
            crossings.clear();
            for index in 0..points.len() {
                let a = points[index];
                let b = points[(index + 1) % points.len()];
                if (a[1] <= scanline && b[1] > scanline) || (b[1] <= scanline && a[1] > scanline) {
                    let t = (scanline - a[1]) / (b[1] - a[1]);
                    crossings.push(a[0] + (b[0] - a[0]) * t);
                }
            }
            crossings.sort_by(f64::total_cmp);
            for pair in crossings.as_chunks::<2>().0 {
                let start_x = pair[0].ceil() as i64;
                let end_x = pair[1].floor() as i64;
                for x in start_x..=end_x {
                    self.set(x, y, color);
                }
            }
        }
    }

    fn draw_text(&mut self, at: [f64; 2], text: &str, size: f64, color: [u8; 3], missing: &mut usize) {
        if size < 3.0 {
            return;
        }
        let cell = (size / 9.0).floor().max(1.0) as i64;
        let glyph_height = 7 * cell;
        let baseline = at[1].round() as i64;
        let top = baseline - glyph_height;
        let mut cursor = at[0].round() as i64;
        for character in text.chars() {
            match glyph(character) {
                Some(rows) => {
                    for (row, bits) in rows.iter().enumerate() {
                        for (column, bit) in bits.chars().enumerate() {
                            if bit != '1' {
                                continue;
                            }
                            let x = cursor + column as i64 * cell;
                            let y = top + row as i64 * cell;
                            for dy in 0..cell {
                                for dx in 0..cell {
                                    self.set(x + dx, y + dy, color);
                                }
                            }
                        }
                    }
                }
                None => *missing += 1,
            }
            cursor += 6 * cell;
        }
    }
}

/// Returns the 5x7 bitmap rows for one character, `None` when unsupported.
fn glyph(character: char) -> Option<[&'static str; 7]> {
    let upper = character.to_ascii_uppercase();
    let rows = match upper {
        'A' => ["01110", "10001", "10001", "11111", "10001", "10001", "10001"],
        'B' => ["11110", "10001", "10001", "11110", "10001", "10001", "11110"],
        'C' => ["01110", "10001", "10000", "10000", "10000", "10001", "01110"],
        'D' => ["11110", "10001", "10001", "10001", "10001", "10001", "11110"],
        'E' => ["11111", "10000", "10000", "11110", "10000", "10000", "11111"],
        'F' => ["11111", "10000", "10000", "11110", "10000", "10000", "10000"],
        'G' => ["01110", "10001", "10000", "10111", "10001", "10001", "01110"],
        'H' => ["10001", "10001", "10001", "11111", "10001", "10001", "10001"],
        'I' => ["01110", "00100", "00100", "00100", "00100", "00100", "01110"],
        'J' => ["00111", "00010", "00010", "00010", "00010", "10010", "01100"],
        'K' => ["10001", "10010", "10100", "11000", "10100", "10010", "10001"],
        'L' => ["10000", "10000", "10000", "10000", "10000", "10000", "11111"],
        'M' => ["10001", "11011", "10101", "10101", "10001", "10001", "10001"],
        'N' => ["10001", "11001", "10101", "10011", "10001", "10001", "10001"],
        'O' => ["01110", "10001", "10001", "10001", "10001", "10001", "01110"],
        'P' => ["11110", "10001", "10001", "11110", "10000", "10000", "10000"],
        'Q' => ["01110", "10001", "10001", "10001", "10101", "10010", "01101"],
        'R' => ["11110", "10001", "10001", "11110", "10100", "10010", "10001"],
        'S' => ["01111", "10000", "10000", "01110", "00001", "00001", "11110"],
        'T' => ["11111", "00100", "00100", "00100", "00100", "00100", "00100"],
        'U' => ["10001", "10001", "10001", "10001", "10001", "10001", "01110"],
        'V' => ["10001", "10001", "10001", "10001", "10001", "01010", "00100"],
        'W' => ["10001", "10001", "10001", "10101", "10101", "11011", "10001"],
        'X' => ["10001", "10001", "01010", "00100", "01010", "10001", "10001"],
        'Y' => ["10001", "10001", "01010", "00100", "00100", "00100", "00100"],
        'Z' => ["11111", "00001", "00010", "00100", "01000", "10000", "11111"],
        '0' => ["01110", "10001", "10011", "10101", "11001", "10001", "01110"],
        '1' => ["00100", "01100", "00100", "00100", "00100", "00100", "01110"],
        '2' => ["01110", "10001", "00001", "00010", "00100", "01000", "11111"],
        '3' => ["11110", "00001", "00001", "01110", "00001", "00001", "11110"],
        '4' => ["00010", "00110", "01010", "10010", "11111", "00010", "00010"],
        '5' => ["11111", "10000", "10000", "11110", "00001", "00001", "11110"],
        '6' => ["01110", "10000", "10000", "11110", "10001", "10001", "01110"],
        '7' => ["11111", "00001", "00010", "00100", "01000", "01000", "01000"],
        '8' => ["01110", "10001", "10001", "01110", "10001", "10001", "01110"],
        '9' => ["01110", "10001", "10001", "01111", "00001", "00001", "01110"],
        ' ' => ["00000", "00000", "00000", "00000", "00000", "00000", "00000"],
        '-' => ["00000", "00000", "00000", "11111", "00000", "00000", "00000"],
        '_' => ["00000", "00000", "00000", "00000", "00000", "00000", "11111"],
        '.' => ["00000", "00000", "00000", "00000", "00000", "00110", "00110"],
        ',' => ["00000", "00000", "00000", "00000", "00100", "00100", "01000"],
        ':' => ["00000", "00100", "00100", "00000", "00100", "00100", "00000"],
        ';' => ["00000", "00100", "00100", "00000", "00100", "00100", "01000"],
        '/' => ["00001", "00010", "00010", "00100", "01000", "01000", "10000"],
        '(' => ["00010", "00100", "01000", "01000", "01000", "00100", "00010"],
        ')' => ["01000", "00100", "00010", "00010", "00010", "00100", "01000"],
        '+' => ["00000", "00100", "00100", "11111", "00100", "00100", "00000"],
        '=' => ["00000", "00000", "11111", "00000", "11111", "00000", "00000"],
        '#' => ["01010", "01010", "11111", "01010", "11111", "01010", "01010"],
        '\'' => ["00100", "00100", "01000", "00000", "00000", "00000", "00000"],
        '"' => ["01010", "01010", "00000", "00000", "00000", "00000", "00000"],
        '*' => ["00000", "10101", "01110", "11111", "01110", "10101", "00000"],
        '?' => ["01110", "10001", "00001", "00010", "00100", "00000", "00100"],
        '!' => ["00100", "00100", "00100", "00100", "00100", "00000", "00100"],
        '%' => ["11001", "11010", "00100", "00100", "01000", "01011", "10011"],
        '&' => ["01100", "10010", "10100", "01000", "10101", "10010", "01101"],
        '@' => ["01110", "10001", "10111", "10101", "10111", "10000", "01110"],
        _ => return None,
    };
    Some(rows)
}

/// Encodes one 8-bit RGB buffer as a PNG with stored `deflate` blocks.
pub(crate) fn encode_png(width: usize, height: usize, pixels: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(height * (1 + width * 3));
    for row in 0..height {
        raw.push(0);
        let offset = row * width * 3;
        raw.extend_from_slice(&pixels[offset..offset + width * 3]);
    }
    let mut zlib = Vec::with_capacity(raw.len() + raw.len() / 65535 * 5 + 6);
    zlib.extend_from_slice(&[0x78, 0x01]);
    let mut chunks = raw.chunks(65535).peekable();
    while let Some(chunk) = chunks.next() {
        let last = chunks.peek().is_none();
        zlib.push(u8::from(last));
        let length = chunk.len() as u16;
        zlib.extend_from_slice(&length.to_le_bytes());
        zlib.extend_from_slice(&(!length).to_le_bytes());
        zlib.extend_from_slice(chunk);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = Vec::with_capacity(zlib.len() + 57);
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    push_chunk(&mut out, b"IHDR", &ihdr);
    push_chunk(&mut out, b"IDAT", &zlib);
    push_chunk(&mut out, b"IEND", &[]);
    out
}

fn push_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = Crc32::new();
    crc.update(kind);
    crc.update(data);
    out.extend_from_slice(&crc.finish().to_be_bytes());
}

fn adler32(bytes: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &byte in bytes {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

struct Crc32 {
    value: u32,
}

impl Crc32 {
    fn new() -> Self {
        Self { value: 0xffff_ffff }
    }

    fn update(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.value ^= byte as u32;
            for _ in 0..8 {
                self.value = if self.value & 1 == 1 { (self.value >> 1) ^ 0xedb8_8320 } else { self.value >> 1 };
            }
        }
    }

    fn finish(self) -> u32 {
        !self.value
    }
}

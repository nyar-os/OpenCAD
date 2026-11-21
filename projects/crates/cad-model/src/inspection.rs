//! Inspection of a drawing document.
//!
//! Every finding points at a layer or an entity; no finding is a score. A
//! drawing with no error-level finding passes, warnings are reported without
//! failing the run.

use std::collections::HashMap;

use boost_check::{CheckReport, Issue, Severity};
use serde_json::{Value, json};

use crate::schema::{Drawing, EntityKind, Units};

/// Checks this build executes.
pub(crate) const SUPPORTED_CHECKS: &[&str] =
    &["units", "layers", "empty-layer", "geometry", "polylines", "dimensions", "duplicate-entity", "zero-length", "open-contour"];

/// Checks a request without an explicit selection runs.
///
/// `empty-layer` is opt-in because every new drawing starts with a reserved
/// but unused `dimensions` layer, and flagging that in every default run would
/// be noise rather than a finding.
pub(crate) const DEFAULT_CHECKS: &[&str] =
    &["units", "layers", "geometry", "polylines", "dimensions", "duplicate-entity", "zero-length", "open-contour"];

/// Result of one inspection run.
#[derive(Debug, Clone)]
pub(crate) struct Report {
    revision: u64,
    issues: CheckReport,
}

impl Report {
    /// JSON shape returned by `inspect`.
    pub(crate) fn to_json(&self) -> Value {
        let issues = self
            .issues
            .issues
            .iter()
            .map(|issue| {
                json!({
                    "code": issue.code,
                    "severity": issue.severity.as_str(),
                    "revision": self.revision,
                    "layerId": issue.details.get("layer"),
                    "objectId": issue.details.get("object"),
                    "recoverable": issue.details.get("recoverable").is_some_and(|value| value == "true"),
                    "message": issue.message,
                })
            })
            .collect::<Vec<_>>();
        json!({
            "revision": self.revision,
            "checkerVersion": env!("CARGO_PKG_VERSION"),
            "passed": self.issues.is_ok(),
            "issues": issues,
        })
    }
}

struct Collector<'a> {
    drawing: &'a Drawing,
    issues: CheckReport,
    seen_ids: HashMap<&'a str, usize>,
    geometry: bool,
    layers: bool,
    units: bool,
    polylines: bool,
    dimensions: bool,
}

impl<'a> Collector<'a> {
    fn push(&mut self, code: &str, severity: Severity, layer: Option<&str>, entity: Option<&str>, recoverable: bool, message: String) {
        let mut issue = Issue::new(code, severity, message).with_detail("recoverable", recoverable.to_string());
        if let Some(layer) = layer {
            issue = issue.with_detail("layer", layer);
        }
        if let Some(entity) = entity {
            issue = issue.with_detail("object", entity);
        }
        self.issues.push(issue);
    }

    fn check_ids(&mut self) {
        for entity in &self.drawing.entities {
            let count = self.seen_ids.entry(entity.id.as_str()).or_insert(0);
            *count += 1;
        }
        let duplicates = self.seen_ids.iter().filter(|(_, count)| **count > 1).map(|(id, _)| *id).collect::<Vec<_>>();
        for id in duplicates {
            self.push(
                "geometry/duplicate-id",
                Severity::Error,
                self.drawing.entities.iter().find(|entity| entity.id == id).map(|entity| entity.layer.as_str()),
                Some(id),
                false,
                format!("entity id `{id}` is used more than once"),
            );
        }
    }

    /// Flags entities whose layer and geometry are byte-for-byte identical.
    ///
    /// Ids stay unique, so this is a warning about duplicated drawing content
    /// rather than a schema error. The first occurrence is named in the message.
    /// The check runs under the `geometry` rule and, with the
    /// `duplicate-entity` selector, also reports the dedicated
    /// `inspection/duplicate-entity` code.
    fn check_duplicate_entities(&mut self, code: &str) {
        let mut seen: HashMap<String, &'a str> = HashMap::new();
        for entity in &self.drawing.entities {
            let Ok(kind) = serde_json::to_string(&entity.kind)
            else {
                continue;
            };
            let key = format!("{}\u{0}{kind}", entity.layer);
            if let Some(first) = seen.get(&key).copied() {
                self.push(
                    code,
                    Severity::Warning,
                    Some(&entity.layer),
                    Some(&entity.id),
                    true,
                    format!("entity `{}` duplicates the geometry of `{first}` on the same layer", entity.id),
                );
            }
            else {
                seen.insert(key, entity.id.as_str());
            }
        }
    }

    /// Flags lines and polylines whose whole length collapses to one point.
    ///
    /// A line is degenerate when both ends coincide; a polyline is degenerate
    /// when it has fewer than two vertices or every edge has zero length.
    fn check_zero_length(&mut self) {
        for entity in &self.drawing.entities {
            let degenerate = match &entity.kind {
                EntityKind::Line { from, to } => from == to,
                EntityKind::Polyline { points, .. } => points.len() < 2 || points.windows(2).all(|pair| pair[0] == pair[1]),
                _ => false,
            };
            if degenerate {
                let name = if matches!(entity.kind, EntityKind::Line { .. }) { "line" } else { "polyline" };
                self.push(
                    "inspection/zero-length",
                    Severity::Error,
                    Some(&entity.layer),
                    Some(&entity.id),
                    true,
                    format!("{name} `{}` has zero length", entity.id),
                );
            }
        }
    }

    /// Flags chains of lines and open polylines that do not close back on
    /// themselves.
    ///
    /// Every line and every open polyline contributes one edge per segment.
    /// Points closer than one millionth of a drawing unit are treated as the
    /// same vertex, so chained entities connect. A connected chain is closed
    /// when every vertex joins exactly two edges. Closed polylines, curves and
    /// annotations are ignored, and a single isolated line is not a contour.
    fn check_open_contours(&mut self) {
        let mut graph = ContourGraph::default();
        let mut edges = Vec::new();
        for (index, entity) in self.drawing.entities.iter().enumerate() {
            match &entity.kind {
                EntityKind::Line { from, to } => {
                    if let Some((vertex, _)) = graph.edge(*from, *to) {
                        edges.push((index, vertex));
                    }
                }
                EntityKind::Polyline { points, closed } => {
                    if *closed {
                        continue;
                    }
                    for pair in points.windows(2) {
                        if let Some((vertex, _)) = graph.edge(pair[0], pair[1]) {
                            edges.push((index, vertex));
                        }
                    }
                }
                _ => {}
            }
        }
        if edges.len() < 2 {
            return;
        }

        let mut order = Vec::new();
        let mut segments: HashMap<usize, usize> = HashMap::new();
        let mut first_entity: HashMap<usize, usize> = HashMap::new();
        for (index, vertex) in edges {
            let root = graph.root(vertex);
            if !segments.contains_key(&root) {
                order.push(root);
                first_entity.insert(root, index);
            }
            *segments.entry(root).or_default() += 1;
        }
        let mut loose: HashMap<usize, usize> = HashMap::new();
        for vertex in 0..graph.parent.len() {
            let root = graph.root(vertex);
            if graph.degree[vertex] != 2 {
                *loose.entry(root).or_default() += 1;
            }
        }

        let findings = order
            .into_iter()
            .filter_map(|root| {
                let count = segments.get(&root).copied().unwrap_or(0);
                let loose = loose.get(&root).copied().unwrap_or(0);
                (count >= 2 && loose > 0).then(|| (first_entity[&root], count, loose))
            })
            .collect::<Vec<_>>();
        for (index, count, loose) in findings {
            let entity = &self.drawing.entities[index];
            self.push(
                "inspection/open-contour",
                Severity::Warning,
                Some(&entity.layer),
                Some(&entity.id),
                true,
                format!("contour starting at `{}` joins {count} segment(s) and has {loose} loose or branching vertex(es)", entity.id),
            );
        }
    }

    fn check_layer_names(&mut self) {
        let mut seen: HashMap<&str, &str> = HashMap::new();
        let mut duplicates = Vec::new();
        for layer in &self.drawing.layers {
            if seen.insert(layer.name.as_str(), layer.id.as_str()).is_some() {
                duplicates.push((layer.id.clone(), layer.name.clone()));
            }
        }
        for (id, name) in duplicates {
            self.push("layers/duplicate-name", Severity::Error, Some(&id), None, true, format!("layer id `{id}` reuses the name `{name}`"));
        }
    }

    /// Flags layers that carry no entities.
    ///
    /// A warning rather than an error: an empty layer is a drawing-organisation
    /// hint, not invalid geometry. The finding names the layer, not an entity.
    fn check_empty_layers(&mut self) {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for entity in &self.drawing.entities {
            *counts.entry(entity.layer.as_str()).or_default() += 1;
        }
        for layer in &self.drawing.layers {
            if counts.get(layer.id.as_str()).copied().unwrap_or(0) == 0 {
                self.push("layers/empty", Severity::Warning, Some(&layer.id), None, true, format!("layer `{}` has no entities", layer.name));
            }
        }
    }

    fn check_entities(&mut self) {
        let locked = self.drawing.layers.iter().filter(|layer| layer.locked).map(|layer| layer.id.as_str()).collect::<Vec<_>>();
        for entity in &self.drawing.entities {
            if self.layers && self.drawing.layer(&entity.layer).is_none() {
                self.push(
                    "layers/missing-layer",
                    Severity::Error,
                    Some(&entity.layer),
                    Some(&entity.id),
                    true,
                    format!("entity `{}` references layer `{}`, which does not exist", entity.id, entity.layer),
                );
            }
            if self.geometry && !entity.is_finite() {
                self.push(
                    "geometry/non-finite",
                    Severity::Error,
                    Some(&entity.layer),
                    Some(&entity.id),
                    false,
                    format!("entity `{}` carries a non-finite coordinate", entity.id),
                );
            }
            match &entity.kind {
                EntityKind::Line { from, to } => {
                    if self.geometry && from == to {
                        self.push(
                            "geometry/zero-length-line",
                            Severity::Error,
                            Some(&entity.layer),
                            Some(&entity.id),
                            true,
                            format!("line `{}` starts and ends at the same point", entity.id),
                        );
                    }
                }
                EntityKind::Point { .. } => {}
                EntityKind::Circle { radius, .. } => {
                    if self.geometry && *radius <= 0.0 {
                        self.push(
                            "geometry/invalid-radius",
                            Severity::Error,
                            Some(&entity.layer),
                            Some(&entity.id),
                            true,
                            format!("circle `{}` has a non-positive radius ({radius})", entity.id),
                        );
                    }
                }
                EntityKind::Arc { radius, .. } => {
                    if self.geometry && *radius <= 0.0 {
                        self.push(
                            "geometry/invalid-radius",
                            Severity::Error,
                            Some(&entity.layer),
                            Some(&entity.id),
                            true,
                            format!("arc `{}` has a non-positive radius ({radius})", entity.id),
                        );
                    }
                }
                EntityKind::Ellipse { rx, ry, .. } => {
                    if self.geometry && (*rx <= 0.0 || *ry <= 0.0) {
                        self.push(
                            "geometry/invalid-radius",
                            Severity::Error,
                            Some(&entity.layer),
                            Some(&entity.id),
                            true,
                            format!("ellipse `{}` has a non-positive radius (rx {rx}, ry {ry})", entity.id),
                        );
                    }
                }
                EntityKind::Polyline { points, closed } => {
                    if self.geometry && points.len() < 2 {
                        self.push(
                            "geometry/insufficient-points",
                            Severity::Error,
                            Some(&entity.layer),
                            Some(&entity.id),
                            true,
                            format!("polyline `{}` has {} point(s); at least 2 are required", entity.id, points.len()),
                        );
                    }
                    else if self.geometry && points.windows(2).any(|pair| pair[0] == pair[1]) {
                        self.push(
                            "geometry/zero-length-segment",
                            Severity::Warning,
                            Some(&entity.layer),
                            Some(&entity.id),
                            true,
                            format!("polyline `{}` repeats a vertex, which draws a zero-length segment", entity.id),
                        );
                    }
                    if self.polylines && points.len() < 2 {
                        self.push(
                            "polylines/insufficient-points",
                            Severity::Error,
                            Some(&entity.layer),
                            Some(&entity.id),
                            true,
                            format!("polyline `{}` has {} point(s); a polyline needs at least 2", entity.id, points.len()),
                        );
                    }
                    if self.polylines && !*closed {
                        self.push(
                            "polylines/open",
                            Severity::Warning,
                            Some(&entity.layer),
                            Some(&entity.id),
                            true,
                            format!("polyline `{}` is open; its area is not measured and it cannot be extruded", entity.id),
                        );
                    }
                }
                EntityKind::Dimension { from, to, text, .. } => {
                    if self.dimensions && from == to {
                        self.push(
                            "dimensions/degenerate",
                            Severity::Error,
                            Some(&entity.layer),
                            Some(&entity.id),
                            true,
                            format!("dimension `{}` measures a zero-length segment", entity.id),
                        );
                    }
                    if self.dimensions && text.as_deref().is_some_and(|label| label.trim().is_empty()) {
                        self.push(
                            "dimensions/empty-text",
                            Severity::Warning,
                            Some(&entity.layer),
                            Some(&entity.id),
                            true,
                            format!("dimension `{}` carries an empty label; the measured distance is used", entity.id),
                        );
                    }
                    if self.units
                        && let Some(label) = text
                        && label_conflicts_with_units(label, self.drawing.units)
                    {
                        self.push(
                            "units/dimension-label",
                            Severity::Warning,
                            Some(&entity.layer),
                            Some(&entity.id),
                            true,
                            format!(
                                "dimension `{}` labels `{}` in a unit that conflicts with the drawing unit `{}`",
                                entity.id,
                                label.trim(),
                                self.drawing.units.as_str()
                            ),
                        );
                    }
                }
                EntityKind::RadialDimension { radius, text, .. } => {
                    if self.dimensions && *radius <= 0.0 {
                        self.push(
                            "dimensions/invalid-radius",
                            Severity::Error,
                            Some(&entity.layer),
                            Some(&entity.id),
                            true,
                            format!("radial dimension `{}` has a non-positive radius ({radius})", entity.id),
                        );
                    }
                    if self.dimensions && text.as_deref().is_some_and(|label| label.trim().is_empty()) {
                        self.push(
                            "dimensions/empty-text",
                            Severity::Warning,
                            Some(&entity.layer),
                            Some(&entity.id),
                            true,
                            format!("radial dimension `{}` carries an empty label; the measured value is used", entity.id),
                        );
                    }
                    if self.units
                        && let Some(label) = text
                        && label_conflicts_with_units(label, self.drawing.units)
                    {
                        self.push(
                            "units/dimension-label",
                            Severity::Warning,
                            Some(&entity.layer),
                            Some(&entity.id),
                            true,
                            format!(
                                "radial dimension `{}` labels `{}` in a unit that conflicts with the drawing unit `{}`",
                                entity.id,
                                label.trim(),
                                self.drawing.units.as_str()
                            ),
                        );
                    }
                }
                EntityKind::OrdinateDimension { origin, feature, text, .. } => {
                    if self.dimensions && origin == feature {
                        self.push(
                            "dimensions/degenerate",
                            Severity::Error,
                            Some(&entity.layer),
                            Some(&entity.id),
                            true,
                            format!("ordinate dimension `{}` measures zero distance", entity.id),
                        );
                    }
                    if self.dimensions && text.as_deref().is_some_and(|label| label.trim().is_empty()) {
                        self.push(
                            "dimensions/empty-text",
                            Severity::Warning,
                            Some(&entity.layer),
                            Some(&entity.id),
                            true,
                            format!("ordinate dimension `{}` carries an empty label; the measured value is used", entity.id),
                        );
                    }
                    if self.units
                        && let Some(label) = text
                        && label_conflicts_with_units(label, self.drawing.units)
                    {
                        self.push(
                            "units/dimension-label",
                            Severity::Warning,
                            Some(&entity.layer),
                            Some(&entity.id),
                            true,
                            format!(
                                "ordinate dimension `{}` labels `{}` in a unit that conflicts with the drawing unit `{}`",
                                entity.id,
                                label.trim(),
                                self.drawing.units.as_str()
                            ),
                        );
                    }
                }
                EntityKind::Text { content, .. } => {
                    if self.geometry && content.trim().is_empty() {
                        self.push(
                            "geometry/empty-text",
                            Severity::Warning,
                            Some(&entity.layer),
                            Some(&entity.id),
                            true,
                            format!("text `{}` has no content", entity.id),
                        );
                    }
                }
            }
            if self.layers && locked.contains(&entity.layer.as_str()) {
                self.push(
                    "layers/locked",
                    Severity::Warning,
                    Some(&entity.layer),
                    Some(&entity.id),
                    true,
                    format!("entity `{}` sits on locked layer `{}`", entity.id, entity.layer),
                );
            }
            if self.layers && self.drawing.layer(&entity.layer).is_some_and(|layer| !layer.visible) {
                self.push(
                    "layers/hidden",
                    Severity::Warning,
                    Some(&entity.layer),
                    Some(&entity.id),
                    true,
                    format!("entity `{}` sits on hidden layer `{}` and is not exported", entity.id, entity.layer),
                );
            }
        }
    }
}

/// Vertex key shared by points closer than one millionth of a drawing unit.
fn vertex_key(point: [f64; 2]) -> (i64, i64) {
    const SCALE: f64 = 1_000_000.0;
    ((point[0] * SCALE).round() as i64, (point[1] * SCALE).round() as i64)
}

/// Undirected graph of the line segments that form line and polyline chains.
#[derive(Default)]
struct ContourGraph {
    vertices: HashMap<(i64, i64), usize>,
    parent: Vec<usize>,
    degree: Vec<usize>,
}

impl ContourGraph {
    fn vertex(&mut self, point: [f64; 2]) -> Option<usize> {
        if !point[0].is_finite() || !point[1].is_finite() {
            return None;
        }
        if let Some(index) = self.vertices.get(&vertex_key(point)) {
            return Some(*index);
        }
        let index = self.parent.len();
        self.vertices.insert(vertex_key(point), index);
        self.parent.push(index);
        self.degree.push(0);
        Some(index)
    }

    /// Adds one undirected edge and returns its two vertex indices.
    ///
    /// A zero-length edge is rejected, so it cannot make a chain look closed.
    fn edge(&mut self, from: [f64; 2], to: [f64; 2]) -> Option<(usize, usize)> {
        let (from, to) = (self.vertex(from)?, self.vertex(to)?);
        if from == to {
            return None;
        }
        self.degree[from] += 1;
        self.degree[to] += 1;
        let (from_root, to_root) = (self.root(from), self.root(to));
        if from_root != to_root {
            self.parent[to_root] = from_root;
        }
        Some((from, to))
    }

    fn root(&mut self, index: usize) -> usize {
        let mut root = index;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        let mut current = index;
        while self.parent[current] != current {
            let next = self.parent[current];
            self.parent[current] = root;
            current = next;
        }
        root
    }
}

/// Whether a dimension label names a unit that conflicts with the document.
///
/// The check is deliberately conservative: it only looks at ASCII unit words
/// and the inch symbol in a dimension's explicit text. A label without a unit
/// is never a conflict.
fn label_conflicts_with_units(label: &str, units: Units) -> bool {
    let lower = label.to_ascii_lowercase();
    let tokens = lower.split(|character: char| !character.is_ascii_alphanumeric()).filter(|token| !token.is_empty()).collect::<Vec<_>>();
    let has = |names: &[&str]| tokens.iter().any(|token| names.contains(token));
    match units {
        Units::Mm => has(&["inch", "inches", "ft", "foot", "feet"]) || lower.contains('"'),
        Units::Inch => has(&[
            "mm",
            "millimeter",
            "millimeters",
            "millimetre",
            "millimetres",
            "cm",
            "centimeter",
            "centimeters",
            "centimetre",
            "centimetres",
            "m",
            "meter",
            "meters",
            "metre",
            "metres",
        ]),
    }
}

/// Runs the requested supported checks.
pub(crate) fn run(drawing: &Drawing, checks: &[&str]) -> Report {
    let geometry = checks.contains(&"geometry");
    let layers = checks.contains(&"layers");
    let empty_layers = checks.contains(&"empty-layer");
    let units = checks.contains(&"units");
    let polylines = checks.contains(&"polylines");
    let dimensions = checks.contains(&"dimensions");
    let duplicate_entity = checks.contains(&"duplicate-entity");
    let zero_length = checks.contains(&"zero-length");
    let open_contour = checks.contains(&"open-contour");
    let mut collector =
        Collector { drawing, issues: CheckReport::new(), seen_ids: HashMap::new(), geometry, layers, units, polylines, dimensions };
    if geometry {
        collector.check_ids();
        collector.check_duplicate_entities("geometry/duplicate-entity");
    }
    if duplicate_entity {
        collector.check_duplicate_entities("inspection/duplicate-entity");
    }
    if zero_length {
        collector.check_zero_length();
    }
    if open_contour {
        collector.check_open_contours();
    }
    if layers {
        collector.check_layer_names();
    }
    if empty_layers {
        collector.check_empty_layers();
    }
    collector.check_entities();
    Report { revision: drawing.revision, issues: collector.issues }
}

//! Inspection of a drawing document.
//!
//! Every finding points at a layer or an entity; no finding is a score. A
//! drawing with no error-level finding passes, warnings are reported without
//! failing the run.

use std::collections::HashMap;

use serde::Serialize;
use serde_json::{Value, json};

use crate::schema::{Drawing, EntityKind, Units};

/// Checks this build executes.
pub(crate) const SUPPORTED_CHECKS: &[&str] = &["units", "layers", "geometry", "polylines", "dimensions"];

/// Severity of one finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Severity {
    /// The drawing is valid but something deserves attention.
    Warning,
    /// The drawing breaks an invariant.
    Error,
}

impl Severity {
    fn is_error(self) -> bool {
        self == Self::Error
    }
}

/// One finding.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Issue {
    /// Stable finding code.
    pub(crate) code: String,
    /// `warning` or `error`.
    pub(crate) severity: Severity,
    /// Revision the finding belongs to.
    pub(crate) revision: u64,
    /// Layer locator.
    pub(crate) layer_id: Option<String>,
    /// Entity locator.
    pub(crate) object_id: Option<String>,
    /// Whether a caller can correct the finding without rebuilding the file.
    pub(crate) recoverable: bool,
    /// Human-readable message.
    pub(crate) message: String,
}

/// Result of one inspection run.
#[derive(Debug, Clone)]
pub(crate) struct Report {
    revision: u64,
    issues: Vec<Issue>,
}

impl Report {
    /// JSON shape returned by `inspect`.
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "revision": self.revision,
            "checkerVersion": env!("CARGO_PKG_VERSION"),
            "passed": self.passed(),
            "issues": self.issues,
        })
    }

    fn passed(&self) -> bool {
        !self.issues.iter().any(|issue| issue.severity.is_error())
    }
}

struct Collector<'a> {
    drawing: &'a Drawing,
    issues: Vec<Issue>,
    seen_ids: HashMap<&'a str, usize>,
    geometry: bool,
    layers: bool,
    units: bool,
    polylines: bool,
    dimensions: bool,
}

impl<'a> Collector<'a> {
    fn push(&mut self, code: &str, severity: Severity, layer: Option<&str>, entity: Option<&str>, recoverable: bool, message: String) {
        self.issues.push(Issue {
            code: code.to_string(),
            severity,
            revision: self.drawing.revision,
            layer_id: layer.map(str::to_string),
            object_id: entity.map(str::to_string),
            recoverable,
            message,
        });
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
    fn check_duplicate_entities(&mut self) {
        let mut seen: HashMap<String, &'a str> = HashMap::new();
        for entity in &self.drawing.entities {
            let Ok(kind) = serde_json::to_string(&entity.kind)
            else {
                continue;
            };
            let key = format!("{}\u{0}{kind}", entity.layer);
            if let Some(first) = seen.get(&key).copied() {
                self.push(
                    "geometry/duplicate-entity",
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
    let units = checks.contains(&"units");
    let polylines = checks.contains(&"polylines");
    let dimensions = checks.contains(&"dimensions");
    let mut collector = Collector { drawing, issues: Vec::new(), seen_ids: HashMap::new(), geometry, layers, units, polylines, dimensions };
    if geometry {
        collector.check_ids();
        collector.check_duplicate_entities();
    }
    if layers {
        collector.check_layer_names();
    }
    collector.check_entities();
    Report { revision: drawing.revision, issues: collector.issues }
}

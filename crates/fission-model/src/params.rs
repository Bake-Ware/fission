//! User parameters (Fusion's Change Parameters) and the expressions that
//! tie feature values and sketch dimensions to them.
//!
//! A feature keeps plain numbers in its fields; when a value was typed as an
//! expression that names a parameter (`width/2`), the text is kept in
//! [`Feature::exprs`](crate::Feature) under the field's key, and
//! [`Document::apply_expressions`] re-evaluates it whenever parameters change.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::expr::{evaluate_with, names_in, Quantity};
use crate::{Document, FeatureKind, PatternKind};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    pub expression: String,
    #[serde(default = "length")]
    pub quantity: Quantity,
    #[serde(default)]
    pub comment: String,
}

fn length() -> Quantity {
    Quantity::Length
}

/// Key for a sketch dimension's expression: the constraint's index.
pub fn dim_key(constraint: usize) -> String {
    format!("dim:{constraint}")
}

impl FeatureKind {
    /// Every numeric value a parameter can drive: (key, label, quantity, field).
    pub fn values_mut(&mut self) -> Vec<(&'static str, &'static str, Quantity, &mut f64)> {
        use Quantity::{Angle, Length};
        match self {
            FeatureKind::Sketch { offset, .. } | FeatureKind::Mirror { offset, .. } => vec![("offset", "Offset", Length, offset)],
            FeatureKind::Plane { offset, angle, .. } => vec![("offset", "Offset", Length, offset), ("angle", "Angle", Angle, angle)],
            FeatureKind::Extrude { distance, .. } => vec![("distance", "Distance", Length, distance)],
            FeatureKind::Hole { diameter, depth, kind, .. } => {
                let mut v = vec![("diameter", "Diameter", Length, diameter), ("depth", "Depth", Length, depth)];
                match kind {
                    crate::HoleKind::Simple => {}
                    crate::HoleKind::Counterbore { diameter, depth } => {
                        v.push(("cb_diameter", "Counterbore diameter", Length, diameter));
                        v.push(("cb_depth", "Counterbore depth", Length, depth));
                    }
                    crate::HoleKind::Countersink { diameter, angle } => {
                        v.push(("cs_diameter", "Countersink diameter", Length, diameter));
                        v.push(("cs_angle", "Countersink angle", Angle, angle));
                    }
                }
                v
            }
            FeatureKind::Revolve { degrees, .. } => vec![("degrees", "Angle", Angle, degrees)],
            FeatureKind::Fillet { radius, .. } => vec![("radius", "Radius", Length, radius)],
            FeatureKind::Chamfer { distance, .. } => vec![("distance", "Distance", Length, distance)],
            FeatureKind::Shell { thickness, .. } => vec![("thickness", "Thickness", Length, thickness)],
            FeatureKind::Sphere { diameter, offset, .. } => vec![("diameter", "Diameter", Length, diameter), ("offset", "Plane offset", Length, offset)],
            FeatureKind::Torus { diameter, section, offset, .. } => vec![
                ("diameter", "Ring diameter", Length, diameter),
                ("section", "Tube diameter", Length, section),
                ("offset", "Plane offset", Length, offset),
            ],
            FeatureKind::Thread { pitch, .. } => vec![("pitch", "Pitch", Length, pitch)],
            FeatureKind::Coil { diameter, pitch, turns, section, offset, .. } => vec![
                ("diameter", "Diameter", Length, diameter),
                ("pitch", "Pitch", Length, pitch),
                ("turns", "Revolutions", crate::Quantity::Count, turns),
                ("section", "Section size", Length, section),
                ("offset", "Plane offset", Length, offset),
            ],
            FeatureKind::OffsetFace { distance, .. } => vec![("distance", "Distance", Length, distance)],
            FeatureKind::Draft { degrees, offset, .. } => vec![("degrees", "Angle", Angle, degrees), ("offset", "Plane offset", Length, offset)],
            FeatureKind::Split { offset, .. } => vec![("offset", "Plane offset", Length, offset)],
            FeatureKind::Scale { factor, .. } => vec![("factor", "Scale factor", crate::Quantity::Count, factor)],
            FeatureKind::Move { translate, degrees, .. } => {
                let [x, y, z] = translate;
                vec![("x", "X distance", Length, x), ("y", "Y distance", Length, y), ("z", "Z distance", Length, z), ("degrees", "Angle", Angle, degrees)]
            }
            FeatureKind::Joint { motion, value, min, max, .. } => {
                let q = if *motion == crate::JointMotion::Revolute { Angle } else { Length };
                vec![("value", "Value", q, value), ("min", "Minimum", q, min), ("max", "Maximum", q, max)]
            }
            FeatureKind::Pattern { pattern: PatternKind::Rectangular { spacing, second, .. }, .. } => {
                let mut v = vec![("spacing", "Spacing", Length, spacing)];
                if let Some((_, _, s2)) = second {
                    v.push(("spacing2", "Second spacing", Length, s2));
                }
                v
            }
            FeatureKind::Pattern { pattern: PatternKind::Circular { degrees, .. }, .. } => vec![("degrees", "Total angle", Angle, degrees)],
            FeatureKind::ImportStep { .. } | FeatureKind::Axis { .. } | FeatureKind::Point { .. } | FeatureKind::Unsupported { .. } | FeatureKind::DeleteFace { .. } | FeatureKind::Combine { .. } | FeatureKind::Loft { .. } | FeatureKind::Sweep { .. } => {
                Vec::new()
            }
        }
    }

    /// The value typing goes into while the feature's dialog is open.
    pub fn primary_value(&mut self) -> Option<(&'static str, &'static str, Quantity, &mut f64)> {
        let pick = match self {
            FeatureKind::Move { .. } => "x",
            FeatureKind::Plane { angle, .. } if *angle != 0.0 => "angle",
            _ => "",
        };
        let mut values = self.values_mut();
        let at = values.iter().position(|v| pick.is_empty() || v.0 == pick)?;
        Some(values.swap_remove(at))
    }
}

impl Document {
    /// Every user parameter's value, in mm or degrees. Parameters may refer
    /// to each other in any order; cycles and unknown names are errors.
    pub fn parameter_values(&self) -> Result<BTreeMap<String, f64>, String> {
        let mut values: BTreeMap<String, f64> = BTreeMap::new();
        let mut pending: Vec<&crate::params::Parameter> = self.parameters.iter().collect();
        while !pending.is_empty() {
            let before = pending.len();
            let mut error = None;
            pending.retain(|p| {
                if !names_in(&p.expression).iter().all(|n| values.contains_key(n)) {
                    return true;
                }
                match evaluate_with(&p.expression, p.quantity, &|n| values.get(n).copied()) {
                    Ok(v) => {
                        values.insert(p.name.clone(), v);
                        false
                    }
                    Err(e) => {
                        error.get_or_insert(format!("{}: {e}", p.name));
                        false
                    }
                }
            });
            if let Some(e) = error {
                return Err(e);
            }
            if pending.len() == before {
                let names: Vec<&str> = pending.iter().map(|p| p.name.as_str()).collect();
                return Err(format!("parameters refer to each other or to unknown names: {}", names.join(", ")));
            }
        }
        Ok(values)
    }

    /// Evaluate a value typed into a box, with the document's parameters.
    pub fn evaluate(&self, text: &str, quantity: Quantity) -> Result<f64, String> {
        let values = self.parameter_values()?;
        evaluate_with(text, quantity, &|n| values.get(n).copied())
    }

    /// Re-evaluate every stored expression into its field (and re-solve
    /// sketches whose dimensions changed). Returns problems to report.
    pub fn apply_expressions(&mut self) -> Vec<String> {
        let values = match self.parameter_values() {
            Ok(v) => v,
            Err(e) => return vec![e],
        };
        let lookup = |n: &str| values.get(n).copied();
        let mut errors = Vec::new();
        for feature in &mut self.features {
            if feature.exprs.is_empty() {
                continue;
            }
            let exprs = feature.exprs.clone();
            let mut sketch_changed = false;
            for (key, text) in &exprs {
                if let Some(k) = key.strip_prefix("dim:").and_then(|k| k.parse::<usize>().ok()) {
                    let FeatureKind::Sketch { sketch, .. } = &mut feature.kind else { continue };
                    // Circles are dimensioned by diameter, stored by radius.
                    let half = matches!(sketch.constraints.get(k), Some(crate::sketch2d::Constraint::Radius(c, _))
                        if matches!(sketch.curves.get(*c), Some(crate::sketch2d::Curve::Circle { .. })));
                    let Some(v) = sketch.constraints.get_mut(k).and_then(|c| c.value_mut()) else { continue };
                    match evaluate_with(text, Quantity::Length, &lookup).map(|x| if half { x / 2.0 } else { x }) {
                        Ok(new) if (*v - new).abs() > 1e-12 => {
                            *v = new;
                            sketch_changed = true;
                        }
                        Ok(_) => {}
                        Err(e) => errors.push(format!("{} dimension: {e}", feature.name)),
                    }
                    continue;
                }
                let mut fields = feature.kind.values_mut();
                if let Some((_, label, quantity, field)) = fields.iter_mut().find(|f| f.0 == key) {
                    match evaluate_with(text, *quantity, &lookup) {
                        Ok(v) => **field = v,
                        Err(e) => errors.push(format!("{} {}: {e}", feature.name, label.to_lowercase())),
                    }
                }
            }
            if sketch_changed {
                if let FeatureKind::Sketch { sketch, .. } = &mut feature.kind {
                    if let Err(e) = sketch.solve() {
                        errors.push(format!("{}: {e}", feature.name));
                    }
                }
            }
        }
        errors
    }
}

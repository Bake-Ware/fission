//! Loft and Sweep: starting them, picking their profiles and path in the
//! viewport, and drawing what's picked.

use eframe::egui::{self, Pos2, Stroke};
use fission_model::{interior_point, FeatureKind, Operation, PathRef, Section, Sketch};

use super::sketch::{curve_points, ray_on_plane};
use super::theme;
use crate::app::FissionApp;

/// A loft or sweep's sketches to draw, each with its picked points, and the
/// sweep's path.
pub type LoftSweepPicks = (Vec<(usize, Vec<[f64; 2]>)>, Option<PathRef>);

/// Screen distance for picking a path curve, in points.
const PATH_PICK_PX: f32 = 8.0;

impl FissionApp {
    fn operation_for_new(&self) -> Operation {
        if self.model_bodies.is_empty() { Operation::NewBody } else { Operation::Join }
    }

    /// Sketches with regions, applied and unsuppressed (any may be a loft section).
    fn sketches_with_regions(&self) -> Vec<usize> {
        self.regen_sketches
            .iter()
            .enumerate()
            .filter(|(_, o)| o.as_ref().is_some_and(|o| !o.regions.is_empty()))
            .map(|(i, _)| i)
            .collect()
    }

    /// CREATE ▸ Loft. Starts with every unused sketch's single profile, in
    /// timeline order; click profiles to add or remove them.
    pub fn new_loft(&mut self) {
        if self.sketch.is_some() {
            self.finish_sketch();
        }
        let outputs = self.doc.sketch_outputs();
        let sections: Vec<Section> = self
            .open_sketches()
            .into_iter()
            .filter_map(|sk| {
                let regions = &outputs.get(sk)?.as_ref()?.regions;
                (regions.len() == 1).then(|| Section { sketch: sk, point: interior_point(&regions[0]) })
            })
            .collect();
        let operation = self.operation_for_new();
        self.create(FeatureKind::Loft { sections, operation });
    }

    /// CREATE ▸ Sweep. The newest sketch with a profile is swept; the path
    /// is the first open sketch without regions (or click one).
    pub fn new_sweep(&mut self) {
        if self.sketch.is_some() {
            self.finish_sketch();
        }
        let outputs = self.doc.sketch_outputs();
        let open = self.open_sketches();
        let profile_sketch = open.iter().rev().copied().find(|&sk| outputs.get(sk).and_then(Option::as_ref).is_some_and(|o| !o.regions.is_empty()));
        let path = open
            .iter()
            .copied()
            .find(|&sk| outputs.get(sk).and_then(Option::as_ref).is_some_and(|o| o.regions.is_empty() && !o.sketch.curves.is_empty()))
            .map(|sk| PathRef { sketch: sk, curve: 0 });
        let profile = match profile_sketch {
            Some(sk) => {
                let regions = &outputs[sk].as_ref().unwrap().regions;
                let points = if regions.len() == 1 { vec![interior_point(&regions[0])] } else { Vec::new() };
                Sketch::Profiles { sketch: sk, points }
            }
            None => {
                self.notify("Sweep needs a sketch with a closed profile", true);
                return;
            }
        };
        let operation = self.operation_for_new();
        self.create(FeatureKind::Sweep { profile, path, operation });
    }

    /// While a loft or sweep dialog is open: its sketches with the points to
    /// highlight, and the sweep path.
    pub fn editing_loft_sweep(&self) -> Option<LoftSweepPicks> {
        let feature = self.doc.features.get(self.editing.as_ref()?.index)?;
        match &feature.kind {
            FeatureKind::Loft { sections, .. } => {
                let mut out: Vec<(usize, Vec<[f64; 2]>)> = Vec::new();
                for sk in self.sketches_with_regions() {
                    let points = sections.iter().filter(|s| s.sketch == sk).map(|s| s.point).collect();
                    out.push((sk, points));
                }
                Some((out, None))
            }
            FeatureKind::Sweep { profile: Sketch::Profiles { sketch, points }, path, .. } => {
                let mut out = vec![(*sketch, points.clone())];
                if let Some(p) = path {
                    if p.sketch != *sketch {
                        out.push((p.sketch, Vec::new()));
                    }
                }
                // Other sketches stay visible so a path can be clicked.
                for sk in self.open_sketches() {
                    if !out.iter().any(|(s, _)| *s == sk) {
                        out.push((sk, Vec::new()));
                    }
                }
                Some((out, *path))
            }
            _ => None,
        }
    }

    /// A click while a loft or sweep is being edited. Returns true if it
    /// picked something.
    pub fn pick_loft_sweep(&mut self) -> bool {
        let Some(index) = self.editing.as_ref().map(|e| e.index) else { return false };
        let Some(ray) = self.viewport.pointer_ray() else { return false };
        // The nearest region under the pointer, in any sketch with regions.
        let region_hit = self
            .sketches_with_regions()
            .into_iter()
            .filter_map(|sk| {
                let out = self.regen_sketches.get(sk)?.as_ref()?;
                let p = ray_on_plane(&ray, &out.plane)?;
                let region = out.region_at(p)?;
                let world = out.plane.to_world(p);
                let d = glam::Vec3::new(world[0] as f32, world[1] as f32, world[2] as f32).distance(ray.origin);
                Some((sk, interior_point(region), region.clone(), d))
            })
            .min_by(|a, b| a.3.total_cmp(&b.3));
        let path_hit = self.path_curve_under_pointer();
        match &mut self.doc.features[index].kind {
            FeatureKind::Loft { sections, .. } => {
                let Some((sk, point, region, _)) = region_hit else { return false };
                match sections.iter().position(|s| s.sketch == sk && region.contains(s.point)) {
                    Some(i) => {
                        sections.remove(i);
                    }
                    None => sections.push(Section { sketch: sk, point }),
                }
            }
            FeatureKind::Sweep { profile: Sketch::Profiles { sketch, points }, path, .. } => {
                // A path curve near the pointer wins; else a profile region.
                if let Some(hit) = path_hit {
                    *path = Some(hit);
                } else if let Some((_, point, region, _)) = region_hit.filter(|h| h.0 == *sketch) {
                    match points.iter().position(|q| region.contains(*q)) {
                        Some(i) => {
                            points.remove(i);
                        }
                        None => points.push(point),
                    }
                } else {
                    return false;
                }
            }
            _ => return false,
        }
        self.changed();
        true
    }

    /// A sketch curve within a few pixels of the pointer (not in the
    /// sweep's own profile sketch).
    fn path_curve_under_pointer(&self) -> Option<PathRef> {
        let index = self.editing.as_ref()?.index;
        let profile_sketch = match &self.doc.features.get(index)?.kind {
            FeatureKind::Sweep { profile: Sketch::Profiles { sketch, .. }, .. } => Some(*sketch),
            _ => None,
        };
        let pointer = self.viewport.pointer.map(|p| self.viewport.rect.min + egui::vec2(p.x, p.y))?;
        let mut best: Option<(PathRef, f32)> = None;
        for (sk, out) in self.regen_sketches.iter().enumerate() {
            let Some(out) = out else { continue };
            if Some(sk) == profile_sketch && !out.regions.is_empty() {
                continue;
            }
            for c in 0..out.sketch.curves.len() {
                let pts: Vec<Pos2> = curve_points(&out.sketch, c)
                    .into_iter()
                    .filter_map(|p| {
                        let w = out.plane.to_world(p);
                        self.viewport.to_screen(glam::Vec3::new(w[0] as f32, w[1] as f32, w[2] as f32))
                    })
                    .collect();
                for w in pts.windows(2) {
                    let d = seg_dist(pointer, w[0], w[1]);
                    if d < PATH_PICK_PX && best.is_none_or(|(_, b)| d < b) {
                        best = Some((PathRef { sketch: sk, curve: c }, d));
                    }
                }
            }
        }
        best.map(|(p, _)| p)
    }

    /// Draw the sweep's path on top of the sketches.
    pub fn draw_sweep_path(&self, ui: &egui::Ui, path: PathRef) {
        let Some(out) = self.regen_sketches.get(path.sketch).and_then(Option::as_ref) else { return };
        if path.curve >= out.sketch.curves.len() {
            return;
        }
        let painter = ui.painter_at(self.viewport.rect);
        let stroke = Stroke::new(3.0, theme::accent());
        for (c, _) in out.sketch.chain(path.curve).0 {
            let pts: Vec<Pos2> = curve_points(&out.sketch, c)
                .into_iter()
                .filter_map(|p| {
                    let w = out.plane.to_world(p);
                    self.viewport.to_screen(glam::Vec3::new(w[0] as f32, w[1] as f32, w[2] as f32))
                })
                .collect();
            painter.line(pts, stroke);
        }
    }
}

fn seg_dist(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t).distance(p)
}

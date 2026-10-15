//! Move and Joint dialogs in the viewport. While one is open its bodies
//! follow the dialog live (rigidly moved meshes, no regeneration until OK),
//! and a gizmo drags them: arrows translate along X/Y/Z, rings rotate.
//! Drags snap: lengths to a round step that suits the zoom, angles to 5°;
//! Shift snaps coarser (10× the step, 15°), Ctrl moves freely.

use eframe::egui::{self, Color32, Pos2, Stroke};
use fission_kernel::Mesh;
use fission_model::{Axis, FeatureKind, JointMotion, Rigid};
use glam::{DVec3, Vec3};

use crate::app::FissionApp;
use crate::motion::moved_mesh;
use crate::ui::theme;
use crate::viewport::DisplayBody;

/// Arrow length and ring radius, in screen points.
const ARROW: f64 = 90.0;
const RING: f64 = 70.0;
/// How close the pointer must be to grab a handle, in points.
const GRAB: f32 = 7.0;
/// Snap steps for lengths (mm), the finest one at least this many points apart.
const LENGTH_STEPS: [f64; 12] = [0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0];
const STEP_POINTS: f64 = 6.0;
const SNAP_DEG: f64 = 5.0;
const SNAP_DEG_COARSE: f64 = 15.0;

/// What a drag snaps to.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Snap {
    mm: Option<f64>,
    deg: Option<f64>,
}

impl Snap {
    /// `wpp`: mm per screen point at the gizmo.
    fn new(wpp: f64, coarse: bool, free: bool) -> Snap {
        if free {
            return Snap { mm: None, deg: None };
        }
        let fine = LENGTH_STEPS.iter().copied().find(|s| s / wpp >= STEP_POINTS).unwrap_or(1000.0);
        if coarse {
            Snap { mm: Some(fine * 10.0), deg: Some(SNAP_DEG_COARSE) }
        } else {
            Snap { mm: Some(fine), deg: Some(SNAP_DEG) }
        }
    }

    fn length(&self, v: f64) -> f64 {
        self.mm.map_or(v, |s| round_to(v, s))
    }

    fn angle(&self, v: f64) -> f64 {
        self.deg.map_or(v, |s| round_to(v, s))
    }
}

fn round_to(v: f64, step: f64) -> f64 {
    // Rounded through the step's decimals, so 0.1 steps don't print as 0.30000000000000004.
    let r = (v / step).round() * step;
    (r * 1e6).round() / 1e6
}

/// The model just before the open Move/Joint (what the dialog moves).
pub struct RigidBase {
    bodies: Vec<(String, usize, Mesh)>,
    /// Which bodies moved and whether they were copied, last time the
    /// preview was laid out (unchanged: only the moved meshes are replaced).
    layout: Option<(Vec<usize>, bool)>,
}

#[derive(Clone, Copy, PartialEq)]
enum Handle {
    /// Translate along `dir`: a world axis for Move, a slider's direction.
    Arrow { dir: DVec3, color: Color32 },
    /// Rotate about the line through `center` along `dir`. `origin_axis`:
    /// a Move ring about a world axis through the bodies, which also
    /// translates so they turn in place.
    Ring { center: DVec3, dir: DVec3, origin_axis: Option<Axis>, color: Color32 },
}

pub struct GizmoDrag {
    handle: Handle,
    start: FeatureKind,
    /// The bodies' center before the feature, and where it was at the start.
    base_pivot: DVec3,
    pivot: DVec3,
    /// Arrow: where along it the drag started. Ring: screen angle so far.
    along: f64,
    turned: f64,
    last_angle: f64,
    /// A ring seen nearly edge-on turns by how far the pointer moves along
    /// its tangent where it was grabbed: (unit screen tangent, points per
    /// radian, press position). Going round its squashed outline would be erratic.
    tangent: Option<(egui::Vec2, f64, Pos2)>,
}

/// The rigid motion an open Move or Joint applies, the bodies it moves (by
/// creating feature; empty: the last), and whether Move copies them.
fn rigid_of(kind: &FeatureKind, sketches: &[Option<fission_model::SketchOutput>]) -> Option<(Rigid, Vec<usize>, bool)> {
    match kind {
        FeatureKind::Move { bodies, translate, axis, degrees, copy } => {
            let (origin, dir) = if *degrees == 0.0 { ([0.0; 3], [0.0, 0.0, 1.0]) } else { axis.resolve(sketches).ok()? };
            Some((Rigid { origin, dir, degrees: *degrees, by: *translate }, bodies.clone(), *copy))
        }
        FeatureKind::Joint { bodies, motion, axis, value, .. } => {
            let (origin, dir) = axis.resolve(sketches).ok()?;
            Some((Rigid::joint(*motion, origin, dir, *value), bodies.clone(), false))
        }
        _ => None,
    }
}

fn v3(p: [f64; 3]) -> DVec3 {
    DVec3::from(p)
}

/// Ray parameter's closest point on the line `a + s·d`: s (d unit).
fn closest_on_line(a: DVec3, d: DVec3, ray: &crate::viewport::Ray) -> Option<f64> {
    let (o, e) = (ray.origin.as_dvec3(), ray.dir.as_dvec3());
    let b = d.dot(e);
    let denom = 1.0 - b * b;
    if denom < 1e-4 {
        return None;
    }
    let w = a - o;
    Some((b * e.dot(w) - d.dot(w)) / denom)
}

fn seg_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t - p).length()
}

fn axis_color(k: usize) -> Color32 {
    [Color32::from_rgb(222, 64, 64), Color32::from_rgb(64, 170, 72), Color32::from_rgb(56, 110, 232)][k]
}

impl FissionApp {
    /// Is the open dialog a Move or Joint (whose bodies follow it live)?
    pub fn editing_rigid(&self) -> bool {
        let kind = self.editing.as_ref().and_then(|e| self.doc.features.get(e.index)).map(|f| &f.kind);
        matches!(kind, Some(FeatureKind::Move { .. } | FeatureKind::Joint { .. }))
    }

    /// After regenerating with a Move/Joint open: keep the model before it,
    /// which the preview moves.
    pub fn set_rigid_base(&mut self) {
        self.rigid_base = None;
        self.gizmo_drag = None;
        if self.editing_rigid() {
            let bodies = self.bodies.iter().map(|b| (b.name.clone(), b.created_by, b.mesh.clone())).collect();
            self.rigid_base = Some(RigidBase { bodies, layout: None });
            self.refresh_rigid_preview();
        }
    }

    /// Open a new Move/Joint without rebuilding. When it goes at the end of
    /// the timeline, the model before it is what's on screen already;
    /// otherwise this is `create`, which rebuilds.
    pub fn create_rigid(&mut self, kind: FeatureKind) {
        self.end_joint_preview();
        let appending = self.doc.marker >= self.doc.features.len();
        if self.editing.is_some() || self.needs_regen || self.sketch.is_some() || !appending || self.bodies.is_empty() {
            return self.create(kind);
        }
        let before = self.doc.clone();
        let index = self.doc.push(kind);
        self.editing = Some(crate::app::Edit { index, before, is_new: true });
        self.selected_feature = Some(index);
        self.edit_preview = false;
        self.edit_base = self.model_bodies.clone();
        let n = self.doc.features.len();
        self.status.resize(n, fission_model::FeatureStatus::Ok);
        self.import_parts.resize(n, 0);
        self.import_part_faces.resize(n, Vec::new());
        self.set_rigid_base();
    }

    /// Close an open Move/Joint without rebuilding, when the bodies on screen
    /// are already the answer: OK on one that nothing later builds on (and
    /// whose values follow no expressions), or Cancel on a new one, which
    /// brings back the model from before it. False: rebuild as usual.
    pub fn finish_rigid_fast(&mut self, keep: bool) -> bool {
        let (Some(edit), Some(base)) = (&self.editing, &self.rigid_base) else { return false };
        // Something asked for a rebuild (typing a value into the heads-up box, say).
        if self.needs_regen {
            return false;
        }
        let index = edit.index;
        let Some(feature) = self.doc.features.get(index) else { return false };
        if keep {
            let Some((_, mut moving, _)) = rigid_of(&feature.kind, &self.regen_sketches) else { return false };
            if moving.is_empty() {
                moving.extend(base.bodies.last().map(|b| b.1));
            }
            if feature.suppressed || !feature.exprs.is_empty() || self.status.get(index).is_some_and(|s| *s != fission_model::FeatureStatus::Ok) {
                return false;
            }
            if !self.doc.nothing_later_builds_on(index, &moving) {
                return false;
            }
            let Some(edit) = self.editing.take() else { return false };
            if self.doc != edit.before {
                self.history.record(edit.before);
            }
        } else {
            let appended = edit.is_new && edit.before.features.len() == index && edit.before.marker >= index;
            if !appended {
                return false;
            }
            let Some(edit) = self.editing.take() else { return false };
            self.doc = edit.before;
            self.selected_feature = None;
            let base = self.rigid_base.take().map(|b| b.bodies).unwrap_or_default();
            self.bodies = base
                .into_iter()
                .map(|(name, created_by, mesh)| DisplayBody { visible: !self.hidden.contains(&created_by), name, mesh, created_by })
                .collect();
            let n = self.doc.features.len();
            self.status.truncate(n);
            self.import_parts.truncate(n);
            self.import_part_faces.truncate(n);
        }
        self.model_bodies = self.bodies.iter().map(|b| (b.name.clone(), b.created_by)).collect();
        self.rigid_base = None;
        self.gizmo_drag = None;
        self.needs_regen = false;
        self.viewport.bodies_changed(&self.bodies);
        true
    }

    /// Show the open Move/Joint's bodies where its values put them.
    pub fn refresh_rigid_preview(&mut self) {
        let Some(index) = self.editing.as_ref().map(|e| e.index) else { return };
        let Some(kind) = self.doc.features.get(index).map(|f| f.kind.clone()) else { return };
        let Some(base) = &mut self.rigid_base else { return };
        let (rigid, mut moving, copy) = rigid_of(&kind, &self.regen_sketches).unwrap_or((Rigid::IDENTITY, Vec::new(), false));
        if moving.is_empty() {
            moving.extend(base.bodies.last().map(|b| b.1));
        }
        let layout = Some((moving.clone(), copy));
        if base.layout == layout {
            // Same bodies: only the moved meshes change.
            let mut k = 0;
            for (_, owner, mesh) in base.bodies.iter() {
                if moving.contains(owner) {
                    let at = if copy { base.bodies.len() + k } else { self.bodies.iter().position(|b| b.created_by == *owner).unwrap_or(0) };
                    if let Some(b) = self.bodies.get_mut(at) {
                        b.mesh = moved_mesh(mesh, &rigid);
                    }
                    k += 1;
                }
            }
        } else {
            let visible = |owner: usize| !self.hidden.contains(&owner);
            let mut shown = Vec::new();
            let mut copies = Vec::new();
            for (name, owner, mesh) in &base.bodies {
                let moved = moving.contains(owner);
                let body = |name: String, mesh: Mesh, created_by: usize| DisplayBody { visible: visible(created_by), name, mesh, created_by };
                if moved && copy {
                    shown.push(body(name.clone(), mesh.clone(), *owner));
                    copies.push(body(format!("{name} (copy)"), moved_mesh(mesh, &rigid), index));
                } else if moved {
                    shown.push(body(name.clone(), moved_mesh(mesh, &rigid), *owner));
                } else {
                    shown.push(body(name.clone(), mesh.clone(), *owner));
                }
            }
            shown.extend(copies);
            self.bodies = shown;
            base.layout = layout;
        }
        self.viewport.bodies_changed(&self.bodies);
    }

    /// The gizmo's handles, and the moved bodies' center before the feature.
    fn gizmo_handles(&self) -> Option<(Vec<Handle>, DVec3, DVec3)> {
        let kind = &self.doc.features.get(self.editing.as_ref()?.index)?.kind;
        let base = self.rigid_base.as_ref()?;
        let (rigid, mut moving, _) = rigid_of(kind, &self.regen_sketches)?;
        if moving.is_empty() {
            moving.extend(base.bodies.last().map(|b| b.1));
        }
        let (mut lo, mut hi) = (DVec3::INFINITY, DVec3::NEG_INFINITY);
        for (_, owner, mesh) in &base.bodies {
            if moving.contains(owner) {
                for p in &mesh.positions {
                    lo = lo.min(Vec3::from(*p).as_dvec3());
                    hi = hi.max(Vec3::from(*p).as_dvec3());
                }
            }
        }
        if !lo.x.is_finite() {
            return None;
        }
        let base_pivot = (lo + hi) / 2.0;
        let pivot = v3(rigid.point(base_pivot.into()));
        let mut handles = Vec::new();
        // A ring about a line: centered where it passes nearest the bodies.
        let line_ring = |origin: DVec3, dir: DVec3, color| {
            let center = origin + dir * (pivot - origin).dot(dir);
            Handle::Ring { center, dir, origin_axis: None, color }
        };
        match kind {
            FeatureKind::Move { axis, degrees, translate, .. } => {
                for k in 0..3 {
                    handles.push(Handle::Arrow { dir: DVec3::AXES[k], color: axis_color(k) });
                }
                let world = [Axis::X, Axis::Y, Axis::Z];
                match world.iter().position(|a| a == axis) {
                    // Any world axis until it's turned; then only that one (a
                    // second rotation can't be written as one axis and angle).
                    Some(k) => {
                        for (j, a) in world.iter().enumerate() {
                            if *degrees == 0.0 || j == k {
                                handles.push(Handle::Ring { center: pivot, dir: DVec3::AXES[j], origin_axis: Some(*a), color: axis_color(j) });
                            }
                        }
                    }
                    None => {
                        if let Ok((origin, dir)) = axis.resolve(&self.regen_sketches) {
                            // Rotation happens first, so the line has moved with the translation.
                            handles.push(line_ring(v3(origin) + v3(*translate), v3(dir), theme::accent()));
                        }
                    }
                }
            }
            FeatureKind::Joint { motion, axis, .. } => {
                let (origin, dir) = axis.resolve(&self.regen_sketches).ok()?;
                handles.push(match motion {
                    JointMotion::Revolute => line_ring(v3(origin), v3(dir), theme::accent()),
                    JointMotion::Slider => Handle::Arrow { dir: v3(dir), color: theme::accent() },
                });
            }
            _ => return None,
        }
        Some((handles, base_pivot, pivot))
    }

    /// Draw the gizmo and drag with it. Returns true if the pointer is on it
    /// (so a click there doesn't pick a body).
    pub fn rigid_gizmo(&mut self, ui: &egui::Ui, events: &crate::viewport::ViewportEvents) -> bool {
        self.viewport.hover_blocked = false;
        if self.rigid_base.is_none() {
            return false;
        }
        let Some((handles, base_pivot, pivot)) = self.gizmo_handles() else { return false };
        let vp = &self.viewport;
        let cam = &vp.camera;
        // World size of a screen point at the gizmo's depth.
        let depth = (pivot - cam.eye().as_dvec3()).dot(cam.forward().as_dvec3()).max(1e-6);
        let wpp = 2.0 * depth * (cam.fov_y as f64 / 2.0).tan() / vp.size().y as f64;
        let screen = |p: DVec3| vp.to_screen(p.as_vec3());

        // Each handle as a screen polyline.
        let shapes: Vec<Vec<Pos2>> = handles
            .iter()
            .map(|h| match *h {
                Handle::Arrow { dir, .. } => {
                    let a = pivot + dir * (wpp * 14.0);
                    [a, pivot + dir * (wpp * ARROW)].iter().filter_map(|&p| screen(p)).collect()
                }
                Handle::Ring { center, dir, .. } => {
                    let u = dir.any_orthonormal_vector();
                    let v = dir.cross(u);
                    (0..=64)
                        .filter_map(|i| {
                            let a = i as f64 / 64.0 * std::f64::consts::TAU;
                            screen(center + (u * a.cos() + v * a.sin()) * (wpp * RING))
                        })
                        .collect()
                }
            })
            .collect();
        let (pointer, press, snap) = ui.input(|i| (i.pointer.latest_pos(), i.pointer.press_origin(), Snap::new(wpp, i.modifiers.shift, i.modifiers.command)));
        let near = |p: Pos2| {
            shapes
                .iter()
                .enumerate()
                .filter_map(|(k, s)| s.windows(2).map(|w| seg_distance(p, w[0], w[1])).reduce(f32::min).map(|d| (k, d)))
                .filter(|&(_, d)| d < GRAB)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(k, _)| k)
        };
        let hovered = pointer.filter(|p| vp.rect.contains(*p)).and_then(near);

        if events.primary_drag_started && self.gizmo_drag.is_none() {
            if let Some(k) = press.and_then(near) {
                let kind = self.doc.features[self.editing.as_ref().unwrap().index].kind.clone();
                let mut drag = GizmoDrag { handle: handles[k], start: kind, base_pivot, pivot, along: 0.0, turned: 0.0, last_angle: 0.0, tangent: None };
                match handles[k] {
                    Handle::Arrow { dir, .. } => {
                        let ray = press.map(|p| cam.ray(glam::Vec2::new(p.x - vp.rect.min.x, p.y - vp.rect.min.y), vp.size()));
                        drag.along = ray.and_then(|r| closest_on_line(pivot, dir, &r)).unwrap_or(0.0);
                    }
                    Handle::Ring { center, dir, .. } => {
                        if let (Some(c), Some(p)) = (screen(center), press) {
                            drag.last_angle = ((p.y - c.y) as f64).atan2((p.x - c.x) as f64);
                            let ray = cam.ray(glam::Vec2::new(p.x - vp.rect.min.x, p.y - vp.rect.min.y), vp.size());
                            if ray.dir.as_dvec3().dot(dir).abs() < 0.35 {
                                let (u, v) = (dir.any_orthonormal_vector(), dir.cross(dir.any_orthonormal_vector()));
                                let r = wpp * RING;
                                let at = |a: f64| center + (u * a.cos() + v * a.sin()) * r;
                                let grab = (0..128)
                                    .map(|i| i as f64 / 128.0 * std::f64::consts::TAU)
                                    .filter_map(|a| screen(at(a)).map(|q| (a, (q - p).length())))
                                    .min_by(|x, y| x.1.total_cmp(&y.1));
                                if let Some((a, _)) = grab {
                                    // Positive turns move a point along d × (p - center).
                                    let step = 0.01;
                                    if let (Some(s0), Some(s1)) = (screen(at(a)), screen(at(a + step))) {
                                        let t = s1 - s0;
                                        if t.length() > 1e-3 {
                                            drag.tangent = Some((t.normalized(), t.length() as f64 / step, p));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                self.gizmo_drag = Some(drag);
            }
        }

        let mut label = None;
        if let (Some(drag), Some(p)) = (&mut self.gizmo_drag, pointer) {
            let vp = &self.viewport;
            let index = self.editing.as_ref().map_or(0, |e| e.index);
            let mut kind = drag.start.clone();
            match drag.handle {
                Handle::Arrow { dir, .. } => {
                    let ray = vp.camera.ray(glam::Vec2::new(p.x - vp.rect.min.x, p.y - vp.rect.min.y), vp.size());
                    if let Some(s) = closest_on_line(drag.pivot, dir, &ray) {
                        let by = s - drag.along;
                        match &mut kind {
                            FeatureKind::Move { translate, .. } => {
                                let k = (0..3).find(|&k| DVec3::AXES[k] == dir).unwrap_or(0);
                                let t = snap.length(translate[k] + by);
                                translate[k] = t;
                                label = Some(format!("{} {t:.2} mm", ["X", "Y", "Z"][k]));
                            }
                            FeatureKind::Joint { value, min, max, .. } => {
                                *value = snap.length(*value + by).clamp(*min, *max);
                                label = Some(format!("{:.2} mm", *value));
                            }
                            _ => {}
                        }
                    }
                }
                Handle::Ring { center, dir, origin_axis, .. } => {
                    if let Some(c) = vp.to_screen(center.as_vec3()) {
                        let a = ((p.y - c.y) as f64).atan2((p.x - c.x) as f64);
                        let mut step = a - drag.last_angle;
                        step -= (step / std::f64::consts::TAU).round() * std::f64::consts::TAU;
                        drag.turned += step;
                        drag.last_angle = a;
                    }
                    // Counterclockwise on screen is positive about an axis facing the viewer
                    // (screen y points down, so that's a decreasing angle).
                    let facing = dir.dot(vp.camera.eye().as_dvec3() - center) >= 0.0;
                    let turned = match drag.tangent {
                        Some((t, per_radian, from)) => ((p - from).dot(t) as f64 / per_radian).to_degrees(),
                        None => -drag.turned.to_degrees() * if facing { 1.0 } else { -1.0 },
                    };
                    let snap = |d: f64| snap.angle(d);
                    match &mut kind {
                        FeatureKind::Move { axis, degrees, translate, .. } => {
                            if let Some(a) = origin_axis {
                                let from = if *axis == a { *degrees } else { 0.0 };
                                *axis = a;
                                *degrees = snap(from + turned);
                                // Turn in place: the center stays where it was.
                                let r = Rigid { origin: [0.0; 3], dir: dir.into(), degrees: *degrees, by: [0.0; 3] };
                                *translate = (drag.pivot - v3(r.point(drag.base_pivot.into()))).into();
                            } else {
                                *degrees = snap(*degrees + turned);
                            }
                            label = Some(format!("{:.1}°", *degrees));
                        }
                        FeatureKind::Joint { value, min, max, .. } => {
                            *value = snap(*value + turned).clamp(*min, *max);
                            label = Some(format!("{:.1}°", *value));
                        }
                        _ => {}
                    }
                }
            }
            if let Some(f) = self.doc.features.get_mut(index) {
                if f.kind != kind {
                    // Values the drag changed no longer follow their expressions.
                    let mut old = std::mem::replace(&mut f.kind, kind);
                    let old: Vec<(&str, f64)> = old.values_mut().into_iter().map(|(k, _, _, v)| (k, *v)).collect();
                    for (k, _, _, v) in f.kind.values_mut() {
                        if old.iter().any(|(o, ov)| *o == k && ov != v) {
                            f.exprs.remove(k);
                        }
                    }
                    self.refresh_rigid_preview();
                }
            }
        }
        let dragging = self.gizmo_drag.as_ref().map(|d| d.handle);
        if events.primary_drag_stopped || !ui.input(|i| i.pointer.primary_down()) {
            self.gizmo_drag = None;
        }

        // Draw: hovered or dragged handles light up.
        let painter = ui.painter_at(self.viewport.rect);
        for (k, (h, s)) in handles.iter().zip(&shapes).enumerate() {
            let hot = hovered == Some(k) || dragging.is_some_and(|d| same_handle(&d, h));
            let (color, ring) = match *h {
                Handle::Arrow { color, .. } => (color, false),
                Handle::Ring { color, .. } => (color, true),
            };
            let color = if hot { Color32::from_rgb(250, 200, 40) } else { color };
            let width = if hot { 3.5 } else { 2.5 };
            painter.add(egui::Shape::line(s.clone(), Stroke::new(width, color)));
            if !ring && s.len() == 2 {
                // Arrowhead.
                let (a, b) = (s[0], s[1]);
                let d = (b - a).normalized();
                let n = egui::vec2(-d.y, d.x);
                let tip = b + d * 12.0;
                painter.add(egui::Shape::convex_polygon(vec![tip, b + n * 5.5, b - n * 5.5], color, Stroke::NONE));
            }
        }
        if let Some(c) = self.viewport.to_screen(pivot.as_vec3()) {
            painter.circle_filled(c, 4.0, theme::accent());
        }
        if let (Some(text), Some(p)) = (label, pointer) {
            let galley = painter.layout_no_wrap(text, egui::FontId::proportional(12.0), theme::text());
            let rect = egui::Rect::from_min_size(p + egui::vec2(16.0, 14.0), galley.size()).expand(4.0);
            painter.rect_filled(rect, 3.0, theme::palette().panel);
            painter.galley(rect.min + egui::vec2(4.0, 4.0), galley, theme::text());
        }
        if hovered.is_some() || dragging.is_some() {
            ui.ctx().set_cursor_icon(if dragging.is_some() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
        }
        let on = hovered.is_some() || dragging.is_some();
        // Faces under a handle don't light up as if they'd be picked.
        self.viewport.hover_blocked = on;
        on
    }
}

/// The same handle across frames (its position follows the bodies).
fn same_handle(a: &Handle, b: &Handle) -> bool {
    match (a, b) {
        (Handle::Arrow { dir: x, .. }, Handle::Arrow { dir: y, .. }) => x == y,
        (Handle::Ring { dir: x, origin_axis: p, .. }, Handle::Ring { dir: y, origin_axis: q, .. }) => x == y && p == q,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewport::Ray;

    #[test]
    fn closest_point_on_a_line_under_a_ray() {
        // Looking down -Z at x = 5: the X axis's nearest point is s = 5.
        let ray = Ray { origin: Vec3::new(5.0, 0.0, 10.0), dir: Vec3::new(0.0, 0.0, -1.0) };
        let s = closest_on_line(DVec3::ZERO, DVec3::X, &ray).unwrap();
        assert!((s - 5.0).abs() < 1e-6, "{s}");
        // Along the line itself: no answer.
        let along = Ray { origin: Vec3::new(0.0, 0.0, 0.0), dir: Vec3::X };
        assert!(closest_on_line(DVec3::ZERO, DVec3::X, &along).is_none());
    }

    #[test]
    fn snaps_to_round_steps_that_suit_the_zoom() {
        // 0.05 mm per point: 0.5 mm is the first step 6 points apart.
        let s = Snap::new(0.05, false, false);
        assert_eq!(s.mm, Some(0.5));
        assert_eq!(s.length(12.34), 12.5);
        assert_eq!(s.angle(37.0), 35.0);
        let coarse = Snap::new(0.05, true, false);
        assert_eq!((coarse.mm, coarse.angle(37.0)), (Some(5.0), 30.0));
        assert_eq!(Snap::new(0.05, false, true).length(12.34), 12.34, "Ctrl: free");
        assert_eq!(Snap::new(0.01, false, false).length(0.33), 0.3);
    }

    #[test]
    fn turning_in_place_keeps_the_center() {
        // A body centered at (10, 0, 0), turned 90° about Z through its center.
        let (base, pivot) = (DVec3::new(10.0, 0.0, 0.0), DVec3::new(10.0, 0.0, 0.0));
        let r = Rigid { origin: [0.0; 3], dir: [0.0, 0.0, 1.0], degrees: 90.0, by: [0.0; 3] };
        let by: [f64; 3] = (pivot - v3(r.point(base.into()))).into();
        let moved = Rigid { by, ..r };
        assert!((v3(moved.point(base.into())) - pivot).length() < 1e-9);
    }
}

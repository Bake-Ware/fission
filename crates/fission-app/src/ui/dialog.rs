//! The feature dialog on the right of the viewport. Every edit regenerates
//! immediately so the model previews live; Cancel restores the snapshot.

use eframe::egui::{self, Rect, RichText};
use fission_model::{Axis, EdgeRef, FeatureKind, FeatureStatus, HoleKind, JointMotion, Operation, OriginPlane, PatternKind, Sketch};

use super::theme;
use crate::app::FissionApp;

pub fn show(app: &mut FissionApp, ctx: &egui::Context, viewport: Rect) {
    let Some(index) = app.editing.as_ref().map(|e| e.index) else { return };
    if index >= app.doc.features.len() {
        app.editing = None;
        return;
    }

    let title = app.doc.features[index].kind.type_name().to_uppercase();
    let mut close = None;
    let mut changed = false;

    egui::Window::new(RichText::new(title).strong())
        .id(egui::Id::new("feature_dialog"))
        .collapsible(false)
        .resizable(false)
        .default_width(250.0)
        .fixed_pos(egui::pos2(viewport.max.x - 270.0, viewport.min.y + 120.0))
        .show(ctx, |ui| {
            // Lines a revolve can turn about: those in its profile's sketch.
            let axis_lines = revolve_lines(&app.doc, index);
            // Construction axes made before this feature.
            let axes: Vec<(Axis, String)> = app
                .doc
                .construction_axes()
                .into_iter()
                .filter(|(i, ..)| *i < index)
                .map(|(i, origin, dir)| (Axis::Line { feature: i, origin, dir }, app.doc.features[i].name.clone()))
                .collect();
            let feature = &mut app.doc.features[index];
            egui::Grid::new("feature_fields").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut feature.name);
                ui.end_row();
                // Values that follow an expression, before the user touches them.
                let bound: Vec<(String, f64)> = feature
                    .kind
                    .values_mut()
                    .into_iter()
                    .filter(|(k, ..)| feature.exprs.contains_key(*k))
                    .map(|(k, _, _, v)| (k.to_string(), *v))
                    .collect();
                changed |= fields(ui, &mut feature.kind, &mut app.edit_preview, &app.edit_base, &axis_lines, &axes);
                // Dragging or typing a plain number into one of them unlinks it.
                for (k, old) in bound {
                    let now = feature.kind.values_mut().into_iter().find(|(key, ..)| *key == k).map(|v| *v.3);
                    if now.is_some_and(|v| (v - old).abs() > 1e-12) {
                        feature.exprs.remove(&k);
                    }
                }
                for (k, e) in feature.exprs.iter().filter(|(k, _)| !k.starts_with("dim:")) {
                    ui.label(RichText::new("fx").small().color(theme::accent()));
                    ui.label(RichText::new(format!("{k} = {e}")).small());
                    ui.end_row();
                }
                if matches!(
                    feature.kind,
                    FeatureKind::Extrude { operation, .. }
                    | FeatureKind::Revolve { operation, .. }
                    | FeatureKind::Loft { operation, .. }
                    | FeatureKind::Sweep { operation, .. }
                    | FeatureKind::Sphere { operation, .. }
                    | FeatureKind::Torus { operation, .. }
                    | FeatureKind::Coil { operation, .. }
                    | FeatureKind::ImportStep { operation, .. } if operation != Operation::NewBody
                ) {
                    changed |= target_field(ui, &mut feature.target, &app.edit_base);
                }
            });

            if let Some(FeatureStatus::Error(message)) = app.status.get(index) {
                ui.add_space(4.0);
                ui.label(RichText::new(format!("⚠ {message}")).color(theme::error()));
            }

            ui.add_space(6.0);
            ui.separator();
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        close = Some(false);
                    }
                    let ok = egui::Button::new(RichText::new("OK").color(egui::Color32::WHITE)).fill(theme::accent());
                    if ui.add(ok).clicked() {
                        close = Some(true);
                    }
                });
            });
        });

    if changed {
        app.changed();
    }
    if let Some(keep) = close {
        app.finish_edit(keep);
    }
}

/// Which body a Join/Cut/Intersect changes. Lists bodies that exist before
/// the edited feature.
fn target_field(ui: &mut egui::Ui, target: &mut Option<usize>, bodies: &[(String, usize)]) -> bool {
    let mut changed = false;
    let label = |t: &Option<usize>| match t {
        None => "Last body".to_string(),
        Some(owner) => bodies.iter().find(|(_, o)| o == owner).map_or("(missing body)".into(), |(n, _)| n.clone()),
    };
    ui.label("Objects");
    egui::ComboBox::from_id_salt("target").selected_text(label(target)).show_ui(ui, |ui| {
        changed |= ui.selectable_value(target, None, "Last body").changed();
        for (name, owner) in bodies {
            changed |= ui.selectable_value(target, Some(*owner), name).changed();
        }
    });
    ui.end_row();
    changed
}

fn edge_fields(ui: &mut egui::Ui, edges: &mut Vec<EdgeRef>, preview: &mut bool) -> bool {
    let mut changed = false;
    ui.label("Edges");
    ui.horizontal(|ui| {
        if edges.is_empty() {
            ui.label(RichText::new("All edges (click edges to choose)").color(theme::muted()));
        } else {
            ui.label(format!("{} selected", edges.len()));
            if ui.small_button("Clear").clicked() {
                edges.clear();
                changed = true;
            }
        }
    });
    ui.end_row();
    ui.label("");
    if ui.checkbox(preview, "Preview result").on_hover_text("Off: pick edges on the body before this feature").changed() {
        changed = true;
    }
    ui.end_row();
    changed
}

fn picked_label(ui: &mut egui::Ui, label: &str, count: usize, none: &str, list_clear: &mut bool) {
    ui.label(label);
    ui.horizontal(|ui| {
        if count == 0 {
            ui.label(RichText::new(none).color(theme::muted()));
        } else {
            ui.label(format!("{count} selected"));
            if ui.small_button("Clear").clicked() {
                *list_clear = true;
            }
        }
    });
    ui.end_row();
}

fn preview_row(ui: &mut egui::Ui, preview: &mut bool) -> bool {
    ui.label("");
    let changed = ui.checkbox(preview, "Preview result").on_hover_text("Off: pick on the model before this feature").changed();
    ui.end_row();
    changed
}

fn body_name(bases: &[(String, usize)], owner: usize) -> String {
    bases.iter().find(|(_, o)| *o == owner).map_or("(missing body)".into(), |(n, _)| n.clone())
}

/// An axis's name: construction axes by their feature's name.
fn axis_name(axis: Axis, axes: &[(Axis, String)]) -> String {
    match axis {
        Axis::Line { feature, .. } => axes
            .iter()
            .find(|(a, _)| matches!(a, Axis::Line { feature: f, .. } if *f == feature))
            .map_or(axis.label(), |(_, n)| n.clone()),
        _ => axis.label(),
    }
}

fn axis_combo(ui: &mut egui::Ui, id: &str, axis: &mut Axis, axes: &[(Axis, String)]) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(id).selected_text(axis_name(*axis, axes)).show_ui(ui, |ui| {
        for a in Axis::ALL {
            changed |= ui.selectable_value(axis, a, a.label()).changed();
        }
        for (a, name) in axes {
            changed |= ui.selectable_value(axis, *a, name).changed();
        }
    });
    changed
}

/// The line curves (index, construction) in a revolve's profile sketch.
fn revolve_lines(doc: &fission_model::Document, index: usize) -> Vec<(usize, usize, bool)> {
    let Some(FeatureKind::Revolve { sketch: Sketch::Profiles { sketch, .. }, .. }) = doc.features.get(index).map(|f| &f.kind)
    else {
        return Vec::new();
    };
    let Some(FeatureKind::Sketch { sketch: sk, .. }) = doc.features.get(*sketch).map(|f| &f.kind) else { return Vec::new() };
    sk.curves
        .iter()
        .enumerate()
        .filter(|(_, c)| matches!(c, fission_model::sketch2d::Curve::Line { .. }))
        .map(|(i, c)| (*sketch, i, c.construction()))
        .collect()
}

fn fields(
    ui: &mut egui::Ui,
    kind: &mut FeatureKind,
    preview: &mut bool,
    bases: &[(String, usize)],
    axis_lines: &[(usize, usize, bool)],
    axes: &[(Axis, String)],
) -> bool {
    let mut changed = false;
    match kind {
        FeatureKind::Draft { faces, plane, offset, degrees, flip } => {
            let mut clear = false;
            picked_label(ui, "Faces", faces.len(), "Click faces to draft", &mut clear);
            if clear {
                faces.clear();
                changed = true;
            }
            changed |= preview_row(ui, preview);
            ui.label("Pull plane");
            egui::ComboBox::from_id_salt("draft_plane").selected_text(plane.label()).show_ui(ui, |ui| {
                for p in OriginPlane::ALL {
                    changed |= ui.selectable_value(plane, p, p.label()).changed();
                }
            });
            ui.end_row();
            ui.label("Plane offset");
            changed |= length(ui, offset, None);
            ui.end_row();
            ui.label("Angle");
            changed |= ui.add(egui::DragValue::new(degrees).speed(0.5).range(-89.0..=89.0).suffix(" deg")).changed();
            ui.end_row();
            ui.label("Direction");
            changed |= ui.checkbox(flip, "Flip").changed();
            ui.end_row();
        }
        FeatureKind::Thread { face, size, pitch, left_hand } => {
            ui.label("Face");
            ui.label(RichText::new(format!("{} (click a round face to change)", body_name(bases, face.body))).color(theme::muted()));
            ui.end_row();
            changed |= preview_row(ui, preview);
            ui.label("Size");
            let name = fission_model::thread::designation(*size, *pitch);
            egui::ComboBox::from_id_salt("thread_size").selected_text(name).show_ui(ui, |ui| {
                for (d, p) in fission_model::thread::ISO_COARSE {
                    let label = fission_model::thread::designation(d, p);
                    if ui.selectable_label(*size == d && *pitch == p, label).clicked() {
                        (*size, *pitch) = (d, p);
                        changed = true;
                    }
                }
            });
            ui.end_row();
            ui.label("Pitch");
            changed |= length(ui, pitch, Some(0.05));
            ui.end_row();
            ui.label("Direction");
            ui.horizontal(|ui| {
                changed |= ui.selectable_value(left_hand, false, "Right hand").changed();
                changed |= ui.selectable_value(left_hand, true, "Left hand").changed();
            });
            ui.end_row();
        }
        FeatureKind::Coil { plane, offset, center, diameter, pitch, turns, section, left_hand, operation } => {
            changed |= placement_fields(ui, plane, offset, center);
            ui.label("Diameter");
            changed |= length(ui, diameter, Some(0.01));
            ui.end_row();
            ui.label("Pitch");
            changed |= length(ui, pitch, Some(0.01));
            ui.end_row();
            ui.label("Revolutions");
            changed |= ui.add(egui::DragValue::new(turns).speed(0.05).range(0.1..=500.0).max_decimals(3)).changed();
            ui.end_row();
            ui.label("Section size");
            changed |= length(ui, section, Some(0.01));
            ui.end_row();
            ui.label("Rotation");
            ui.horizontal(|ui| {
                changed |= ui.selectable_value(left_hand, false, "Clockwise").changed();
                changed |= ui.selectable_value(left_hand, true, "Counter-clockwise").changed();
            });
            ui.end_row();
            changed |= operation_field(ui, operation);
        }
        FeatureKind::Unsupported { kind, note } => {
            ui.label("From Fusion");
            ui.label(kind.as_str());
            ui.end_row();
            ui.label("");
            ui.label(RichText::new(if note.is_empty() { "Not replayed yet: the model continues without it." } else { note.as_str() }).color(theme::muted()));
            ui.end_row();
        }
        FeatureKind::DeleteFace { faces } => {
            let mut clear = false;
            picked_label(ui, "Faces", faces.len(), "Click faces to delete", &mut clear);
            if clear {
                faces.clear();
                changed = true;
            }
            changed |= preview_row(ui, preview);
        }
        FeatureKind::OffsetFace { faces, distance } => {
            let mut clear = false;
            picked_label(ui, "Faces", faces.len(), "Click faces to move", &mut clear);
            if clear {
                faces.clear();
                changed = true;
            }
            changed |= preview_row(ui, preview);
            ui.label("Distance");
            changed |= length(ui, distance, None);
            ui.end_row();
        }
        FeatureKind::Shell { thickness, faces } => {
            let mut clear = false;
            picked_label(ui, "Faces", faces.len(), "Closed (click faces to open)", &mut clear);
            if clear {
                faces.clear();
                changed = true;
            }
            changed |= preview_row(ui, preview);
            ui.label("Thickness");
            changed |= length(ui, thickness, Some(0.01));
            ui.end_row();
        }
        FeatureKind::Combine { target, tools, operation, keep_tools } => {
            ui.label("Target body");
            egui::ComboBox::from_id_salt("combine_target").selected_text(body_name(bases, *target)).show_ui(ui, |ui| {
                for (name, owner) in bases {
                    changed |= ui.selectable_value(target, *owner, name).changed();
                }
            });
            ui.end_row();
            let mut clear = false;
            picked_label(ui, "Tool bodies", tools.len(), "Click bodies", &mut clear);
            if clear {
                tools.clear();
                changed = true;
            }
            changed |= preview_row(ui, preview);
            ui.label("Operation");
            egui::ComboBox::from_id_salt("combine_op").selected_text(operation.label()).show_ui(ui, |ui| {
                for op in [Operation::Join, Operation::Cut, Operation::Intersect] {
                    changed |= ui.selectable_value(operation, op, op.label()).changed();
                }
            });
            ui.end_row();
            ui.label("");
            changed |= ui.checkbox(keep_tools, "Keep tools").changed();
            ui.end_row();
        }
        FeatureKind::Move { bodies, translate, axis, degrees, copy } => {
            let mut clear = false;
            picked_label(ui, "Bodies", bodies.len(), "Last body (click to choose)", &mut clear);
            if clear {
                bodies.clear();
                changed = true;
            }
            changed |= preview_row(ui, preview);
            for (label, k) in [("X distance", 0), ("Y distance", 1), ("Z distance", 2)] {
                ui.label(label);
                changed |= length(ui, &mut translate[k], None);
                ui.end_row();
            }
            ui.label("Rotate about");
            changed |= axis_combo(ui, "move_axis", axis, axes);
            ui.end_row();
            ui.label("Angle");
            changed |= ui.add(egui::DragValue::new(degrees).speed(1.0).range(-360.0..=360.0).suffix(" deg")).changed();
            ui.end_row();
            ui.label("");
            changed |= ui.checkbox(copy, "Create copy").changed();
            ui.end_row();
        }
        FeatureKind::Joint { bodies, motion, axis, value, min, max } => {
            let mut clear = false;
            picked_label(ui, "Moving bodies", bodies.len(), "Last body (click to choose)", &mut clear);
            if clear {
                bodies.clear();
                changed = true;
            }
            changed |= preview_row(ui, preview);
            ui.label("Motion");
            egui::ComboBox::from_id_salt("joint_motion").selected_text(motion.label()).show_ui(ui, |ui| {
                for m in [JointMotion::Revolute, JointMotion::Slider] {
                    changed |= ui.selectable_value(motion, m, m.label()).changed();
                }
            });
            ui.end_row();
            ui.label(if *motion == JointMotion::Revolute { "Turn about" } else { "Slide along" });
            changed |= axis_combo(ui, "joint_axis", axis, axes);
            ui.end_row();
            let revolute = *motion == JointMotion::Revolute;
            let row = |ui: &mut egui::Ui, label: &str, v: &mut f64| {
                ui.label(label);
                let changed = if revolute {
                    ui.add(egui::DragValue::new(v).speed(1.0).range(-720.0..=720.0).suffix(" deg")).changed()
                } else {
                    length(ui, v, None)
                };
                ui.end_row();
                changed
            };
            changed |= row(ui, "Minimum", min);
            changed |= row(ui, "Maximum", max);
            changed |= row(ui, "Value", value);
            if *min > *max {
                std::mem::swap(min, max);
            }
            *value = value.clamp(*min, *max);
        }
        FeatureKind::Split { body, plane, offset } => {
            ui.label("Body");
            ui.horizontal(|ui| match body {
                Some(owner) => {
                    ui.label(body_name(bases, *owner));
                    if ui.small_button("Clear").clicked() {
                        *body = None;
                        changed = true;
                    }
                }
                None => {
                    ui.label(RichText::new("Last body (click to choose)").color(theme::muted()));
                }
            });
            ui.end_row();
            changed |= preview_row(ui, preview);
            ui.label("Splitting plane");
            egui::ComboBox::from_id_salt("split_plane").selected_text(plane.label()).show_ui(ui, |ui| {
                for p in OriginPlane::ALL {
                    changed |= ui.selectable_value(plane, p, p.label()).changed();
                }
            });
            ui.end_row();
            ui.label("Plane offset");
            changed |= length(ui, offset, None);
            ui.end_row();
            ui.label("");
            ui.label(RichText::new("Or click a planar face to split along it").color(theme::muted()));
            ui.end_row();
        }
        FeatureKind::Scale { bodies, center, factor } => {
            let mut clear = false;
            picked_label(ui, "Bodies", bodies.len(), "Last body (click to choose)", &mut clear);
            if clear {
                bodies.clear();
                changed = true;
            }
            changed |= preview_row(ui, preview);
            ui.label("Point");
            ui.horizontal(|ui| {
                for c in center.iter_mut() {
                    changed |= ui.add(egui::DragValue::new(c).speed(0.5).max_decimals(3)).changed();
                }
            });
            ui.end_row();
            ui.label("Scale factor");
            changed |= ui.add(egui::DragValue::new(factor).speed(0.01).range(0.001..=1000.0).max_decimals(4)).changed();
            ui.end_row();
        }
        FeatureKind::Mirror { bodies, plane, offset, join } => {
            let mut clear = false;
            picked_label(ui, "Bodies", bodies.len(), "Last body (click to choose)", &mut clear);
            if clear {
                bodies.clear();
                changed = true;
            }
            changed |= preview_row(ui, preview);
            ui.label("Mirror plane");
            egui::ComboBox::from_id_salt("mirror_plane").selected_text(plane.label()).show_ui(ui, |ui| {
                for p in OriginPlane::ALL {
                    changed |= ui.selectable_value(plane, p, p.label()).changed();
                }
            });
            ui.end_row();
            ui.label("Plane offset");
            changed |= length(ui, offset, None);
            ui.end_row();
            ui.label("");
            changed |= ui.checkbox(join, "Join to original").changed();
            ui.end_row();
        }
        FeatureKind::Pattern { bodies, pattern, join } => {
            let mut clear = false;
            picked_label(ui, "Bodies", bodies.len(), "Last body (click to choose)", &mut clear);
            if clear {
                bodies.clear();
                changed = true;
            }
            changed |= preview_row(ui, preview);
            match pattern {
                PatternKind::Rectangular { dir, count, spacing, second } => {
                    ui.label("Direction");
                    changed |= axis_combo(ui, "pat_dir", dir, axes);
                    ui.end_row();
                    ui.label("Quantity");
                    changed |= ui.add(egui::DragValue::new(count).range(1..=200)).changed();
                    ui.end_row();
                    ui.label("Spacing");
                    changed |= length(ui, spacing, None);
                    ui.end_row();
                    ui.label("");
                    let mut two = second.is_some();
                    if ui.checkbox(&mut two, "Second direction").changed() {
                        *second = two.then_some((Axis::Y, 2, *spacing));
                        changed = true;
                    }
                    ui.end_row();
                    if let Some((dir2, count2, spacing2)) = second {
                        ui.label("Direction 2");
                        changed |= axis_combo(ui, "pat_dir2", dir2, axes);
                        ui.end_row();
                        ui.label("Quantity 2");
                        changed |= ui.add(egui::DragValue::new(count2).range(1..=200)).changed();
                        ui.end_row();
                        ui.label("Spacing 2");
                        changed |= length(ui, spacing2, None);
                        ui.end_row();
                    }
                }
                PatternKind::Circular { axis, count, degrees } => {
                    ui.label("Axis");
                    changed |= axis_combo(ui, "pat_axis", axis, axes);
                    ui.end_row();
                    ui.label("Quantity");
                    changed |= ui.add(egui::DragValue::new(count).range(2..=360)).changed();
                    ui.end_row();
                    ui.label("Total angle");
                    changed |= ui.add(egui::DragValue::new(degrees).speed(1.0).range(-360.0..=360.0).suffix(" deg")).changed();
                    ui.end_row();
                }
            }
            ui.label("");
            changed |= ui.checkbox(join, "Join to original").changed();
            ui.end_row();
        }
        // Sketches are edited in sketch mode, not in a dialog.
        FeatureKind::Loft { sections, operation } => {
            ui.label("Profiles");
            ui.horizontal(|ui| {
                if sections.len() < 2 {
                    ui.label(RichText::new(format!("{} picked: click profiles in order", sections.len())).color(theme::error()));
                } else {
                    ui.label(format!("{} profiles", sections.len()));
                }
                if !sections.is_empty() && ui.small_button("Clear").clicked() {
                    sections.clear();
                    changed = true;
                }
            });
            ui.end_row();
            changed |= operation_field(ui, operation);
        }
        FeatureKind::Sweep { profile, path, operation } => {
            changed |= sketch_fields(ui, profile);
            ui.label("Path");
            ui.horizontal(|ui| match path {
                Some(p) => {
                    ui.label(format!("Sketch curve {}", p.curve + 1));
                    if ui.small_button("Clear").clicked() {
                        *path = None;
                        changed = true;
                    }
                }
                None => {
                    ui.label(RichText::new("Click a sketch line or arc").color(theme::error()));
                }
            });
            ui.end_row();
            changed |= operation_field(ui, operation);
        }
        FeatureKind::Plane { base, offset, angle } => {
            ui.label("Plane");
            egui::ComboBox::from_id_salt("plane_base").selected_text(base.label()).show_ui(ui, |ui| {
                for p in OriginPlane::ALL {
                    changed |= ui.selectable_value(base, p, p.label()).changed();
                }
            });
            ui.end_row();
            ui.label("Distance");
            changed |= length(ui, offset, None);
            ui.end_row();
            ui.label("Angle");
            changed |= ui.add(egui::DragValue::new(angle).speed(1.0).range(-360.0..=360.0).suffix(" deg")).changed();
            ui.end_row();
        }
        FeatureKind::Sketch { .. } => {}
        FeatureKind::Sphere { plane, offset, center, diameter, operation } => {
            changed |= placement_fields(ui, plane, offset, center);
            ui.label("Diameter");
            changed |= length(ui, diameter, Some(0.01));
            ui.end_row();
            changed |= operation_field(ui, operation);
        }
        FeatureKind::Torus { plane, offset, center, diameter, section, operation } => {
            changed |= placement_fields(ui, plane, offset, center);
            ui.label("Ring diameter");
            changed |= length(ui, diameter, Some(0.01));
            ui.end_row();
            ui.label("Tube diameter");
            changed |= length(ui, section, Some(0.01));
            ui.end_row();
            changed |= operation_field(ui, operation);
        }
        FeatureKind::Point { pos } => {
            for (label, k) in [("X", 0), ("Y", 1), ("Z", 2)] {
                ui.label(label);
                changed |= length(ui, &mut pos[k], None);
                ui.end_row();
            }
        }
        FeatureKind::Axis { origin, dir } => {
            let v = |p: &[f64; 3]| format!("({:.2}, {:.2}, {:.2})", p[0], p[1], p[2]);
            ui.label("Through");
            ui.label(v(origin));
            ui.end_row();
            ui.label("Direction");
            ui.label(v(dir));
            ui.end_row();
        }
        FeatureKind::Hole { flip, diameter, depth, through, kind, tip_angle, .. } => {
            ui.label("Placement");
            ui.label(RichText::new("Click a face to move the hole").small().color(theme::muted()));
            ui.end_row();
            ui.label("Type");
            egui::ComboBox::from_id_salt("hole_kind").selected_text(kind.label()).show_ui(ui, |ui| {
                let d = *diameter;
                for k in [
                    HoleKind::Simple,
                    HoleKind::Counterbore { diameter: d * 1.8, depth: d * 0.6 },
                    HoleKind::Countersink { diameter: d * 2.0, angle: 90.0 },
                ] {
                    let is = std::mem::discriminant(kind) == std::mem::discriminant(&k);
                    if ui.selectable_label(is, k.label()).clicked() && !is {
                        *kind = k;
                        changed = true;
                    }
                }
            });
            ui.end_row();
            ui.label("Diameter");
            changed |= length(ui, diameter, Some(0.01));
            ui.end_row();
            match kind {
                HoleKind::Simple => {}
                HoleKind::Counterbore { diameter, depth } => {
                    ui.label("Counterbore Ø");
                    changed |= length(ui, diameter, Some(0.01));
                    ui.end_row();
                    ui.label("Counterbore depth");
                    changed |= length(ui, depth, Some(0.01));
                    ui.end_row();
                }
                HoleKind::Countersink { diameter, angle } => {
                    ui.label("Countersink Ø");
                    changed |= length(ui, diameter, Some(0.01));
                    ui.end_row();
                    ui.label("Countersink angle");
                    changed |= ui.add(egui::DragValue::new(angle).speed(1.0).range(1.0..=179.0).suffix(" deg")).changed();
                    ui.end_row();
                }
            }
            ui.label("Extents");
            changed |= ui.selectable_value(through, false, "Distance").changed();
            ui.end_row();
            ui.label("");
            changed |= ui.selectable_value(through, true, "All").changed();
            ui.end_row();
            if !*through {
                ui.label("Depth");
                changed |= length(ui, depth, Some(0.01));
                ui.end_row();
                ui.label("Drill point");
                let mut angled = *tip_angle > 0.0;
                if ui.checkbox(&mut angled, "118°").changed() {
                    *tip_angle = if angled { 118.0 } else { 0.0 };
                    changed = true;
                }
                ui.end_row();
            }
            ui.label("Direction");
            changed |= ui.checkbox(flip, "Flip").changed();
            ui.end_row();
        }
        FeatureKind::Extrude { sketch, distance, symmetric, operation } => {
            changed |= sketch_fields(ui, sketch);
            ui.label("Direction");
            changed |= ui.selectable_value(symmetric, false, "One Side").changed();
            ui.end_row();
            ui.label("");
            changed |= ui.selectable_value(symmetric, true, "Symmetric").changed();
            ui.end_row();
            ui.label(if *symmetric { "Half distance" } else { "Distance" });
            changed |= length(ui, distance, None);
            ui.end_row();
            changed |= operation_field(ui, operation);
        }
        FeatureKind::Revolve { sketch, axis, degrees, operation } => {
            changed |= sketch_fields(ui, sketch);
            ui.label("Axis");
            egui::ComboBox::from_id_salt("axis").selected_text(axis_name(*axis, axes)).show_ui(ui, |ui| {
                // Construction lines first: they're what axes are usually drawn as.
                let mut lines = axis_lines.to_vec();
                lines.sort_by_key(|&(_, _, construction)| !construction);
                for (sketch, curve, construction) in lines {
                    let a = Axis::SketchLine { sketch, curve };
                    let label = if construction { format!("{} (construction)", a.label()) } else { a.label() };
                    changed |= ui.selectable_value(axis, a, label).changed();
                }
                for a in Axis::ALL {
                    changed |= ui.selectable_value(axis, a, format!("{} axis", a.label())).changed();
                }
                for (a, name) in axes {
                    changed |= ui.selectable_value(axis, *a, name).changed();
                }
            });
            ui.end_row();
            ui.label("Angle");
            changed |= ui
                .add(egui::DragValue::new(degrees).speed(1.0).range(-360.0..=360.0).suffix(" deg"))
                .changed();
            ui.end_row();
            changed |= operation_field(ui, operation);
        }
        FeatureKind::Fillet { radius, edges } => {
            changed |= edge_fields(ui, edges, preview);
            ui.label("Radius");
            changed |= length(ui, radius, Some(0.01));
            ui.end_row();
        }
        FeatureKind::ImportStep { path, operation, .. } => {
            ui.label("File");
            ui.horizontal(|ui| {
                changed |= ui.add(egui::TextEdit::singleline(path).desired_width(150.0)).lost_focus();
                if ui.button("…").on_hover_text("Choose a STEP file").clicked() {
                    if let Some(file) = rfd::FileDialog::new().add_filter("STEP", &["step", "stp"]).pick_file() {
                        *path = file.display().to_string();
                        changed = true;
                    }
                }
            });
            ui.end_row();
            changed |= operation_field(ui, operation);
        }
        FeatureKind::Chamfer { distance, edges } => {
            changed |= edge_fields(ui, edges, preview);
            ui.label("Distance");
            changed |= length(ui, distance, Some(0.01));
            ui.end_row();
        }
    }
    changed
}

/// Plane, offset along it, and a center point on it.
fn placement_fields(ui: &mut egui::Ui, plane: &mut OriginPlane, offset: &mut f64, center: &mut [f64; 2]) -> bool {
    let mut changed = false;
    ui.label("Plane");
    egui::ComboBox::from_id_salt("plane").selected_text(plane.label()).show_ui(ui, |ui| {
        for p in OriginPlane::ALL {
            changed |= ui.selectable_value(plane, p, p.label()).changed();
        }
    });
    ui.end_row();
    ui.label("Plane offset");
    changed |= length(ui, offset, None);
    ui.end_row();
    ui.label("Center");
    ui.horizontal(|ui| {
        changed |= length(ui, &mut center[0], None);
        changed |= length(ui, &mut center[1], None);
    });
    ui.end_row();
    changed
}

fn sketch_fields(ui: &mut egui::Ui, sketch: &mut Sketch) -> bool {
    let mut changed = false;
    if let Sketch::Profiles { points, .. } = sketch {
        ui.label("Profile");
        ui.horizontal(|ui| {
            if points.is_empty() {
                ui.label(RichText::new("Click regions in the sketch").color(theme::error()));
            } else {
                ui.label(format!("{} selected", points.len()));
                if ui.small_button("Clear").clicked() {
                    points.clear();
                    changed = true;
                }
            }
        });
        ui.end_row();
        return changed;
    }
    let is_rect = matches!(sketch, Sketch::Rect { .. });
    ui.label("Profile");
    ui.horizontal(|ui| {
        if ui.selectable_label(is_rect, "Rectangle").clicked() && !is_rect {
            if let Sketch::Circle { plane, offset, center, radius } = *sketch {
                *sketch = Sketch::Rect { plane, offset, center, size: [radius * 2.0, radius * 2.0] };
                changed = true;
            }
        }
        if ui.selectable_label(!is_rect, "Circle").clicked() && is_rect {
            if let Sketch::Rect { plane, offset, center, size } = *sketch {
                *sketch = Sketch::Circle { plane, offset, center, radius: size[0].min(size[1]) / 2.0 };
                changed = true;
            }
        }
    });
    ui.end_row();

    let (plane, offset, center) = match sketch {
        Sketch::Rect { plane, offset, center, .. } | Sketch::Circle { plane, offset, center, .. } => {
            (plane, offset, center)
        }
        Sketch::Profiles { .. } => unreachable!("handled above"),
    };
    changed |= placement_fields(ui, plane, offset, center);

    match sketch {
        Sketch::Rect { size, .. } => {
            ui.label("Size");
            ui.horizontal(|ui| {
                changed |= length(ui, &mut size[0], Some(0.01));
                changed |= length(ui, &mut size[1], Some(0.01));
            });
            ui.end_row();
        }
        Sketch::Circle { radius, .. } => {
            ui.label("Radius");
            changed |= length(ui, radius, Some(0.01));
            ui.end_row();
        }
        Sketch::Profiles { .. } => {}
    }
    changed
}

fn operation_field(ui: &mut egui::Ui, operation: &mut Operation) -> bool {
    let mut changed = false;
    ui.label("Operation");
    egui::ComboBox::from_id_salt("operation").selected_text(operation.label()).show_ui(ui, |ui| {
        for op in Operation::ALL {
            changed |= ui.selectable_value(operation, op, op.label()).changed();
        }
    });
    ui.end_row();
    changed
}

fn length(ui: &mut egui::Ui, value: &mut f64, min: Option<f64>) -> bool {
    let mut drag = egui::DragValue::new(value).speed(0.1).suffix(" mm").max_decimals(3);
    if let Some(min) = min {
        drag = drag.range(min..=1e6);
    }
    ui.add(drag).changed()
}

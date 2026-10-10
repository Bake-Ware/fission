use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use fission_model::{FeatureKind, FeatureStatus};

use super::icons::{self, Icon};
use super::theme;
use crate::app::FissionApp;
use crate::viewport::Pick;

const CHIP: f32 = 26.0;
const GAP: f32 = 6.0;

pub fn icon_for(kind: &FeatureKind) -> Icon {
    match kind {
        FeatureKind::Sketch { .. } => Icon::Sketch,
        FeatureKind::Plane { .. } => Icon::OffsetPlane,
        FeatureKind::Axis { .. } => Icon::Axis,
        FeatureKind::Point { .. } => Icon::Point,
        FeatureKind::Sphere { .. } => Icon::Sphere,
        FeatureKind::Torus { .. } => Icon::Torus,
        FeatureKind::Loft { .. } => Icon::Loft,
        FeatureKind::Sweep { .. } => Icon::Sweep,
        FeatureKind::Extrude { sketch: fission_model::Sketch::Circle { .. }, .. } => Icon::ExtrudeCylinder,
        FeatureKind::Extrude { .. } => Icon::ExtrudeBox,
        FeatureKind::Hole { .. } => Icon::ExtrudeCylinder,
        FeatureKind::Revolve { .. } => Icon::Revolve,
        FeatureKind::Fillet { .. } => Icon::Fillet,
        FeatureKind::Chamfer { .. } => Icon::Chamfer,
        FeatureKind::ImportStep { .. } => Icon::ExportStep,
        FeatureKind::Shell { .. } => Icon::Shell,
        FeatureKind::Draft { .. } => Icon::Draft,
        FeatureKind::Combine { .. } => Icon::Combine,
        FeatureKind::Move { .. } => Icon::Move,
        FeatureKind::Mirror { .. } => Icon::Mirror,
        FeatureKind::Scale { .. } => Icon::Scale,
        FeatureKind::Split { .. } => Icon::Split,
        FeatureKind::OffsetFace { .. } => Icon::OffsetFace,
        FeatureKind::DeleteFace { .. } => Icon::DeleteFace,
        FeatureKind::Unsupported { .. } => Icon::Unsupported,
        FeatureKind::Thread { .. } => Icon::Thread,
        FeatureKind::Coil { .. } => Icon::Coil,
        FeatureKind::Pattern { pattern: fission_model::PatternKind::Circular { .. }, .. } => Icon::PatternCirc,
        FeatureKind::Pattern { .. } => Icon::PatternRect,
    }
}

pub fn show(app: &mut FissionApp, ui: &mut egui::Ui) {
    egui::Panel::bottom("timeline")
        .exact_size(44.0)
        .frame(
            egui::Frame::NONE
                .fill(theme::PANEL)
                .stroke(Stroke::new(1.0, theme::BORDER))
                .inner_margin(egui::Margin::symmetric(8, 6)),
        )
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                playback(app, ui);
                ui.add_space(8.0);
                chips(app, ui);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| info(app, ui));
            });
        });
}

fn playback(app: &mut FissionApp, ui: &mut egui::Ui) {
    let len = app.doc.features.len();
    let marker = app.doc.marker;
    for (label, tip, target) in [
        ("⏮", "Go to beginning", 0),
        ("⏴", "Step back", marker.saturating_sub(1)),
        ("⏵", "Step forward", (marker + 1).min(len)),
        ("⏭", "Go to end", len),
    ] {
        let button = egui::Button::new(RichText::new(label).size(13.0)).min_size(Vec2::new(22.0, 22.0));
        if ui.add(button).on_hover_text(tip).clicked() && target != marker {
            app.finish_edit(true);
            app.commit(|doc| doc.marker = target);
        }
    }
}

fn chips(app: &mut FissionApp, ui: &mut egui::Ui) {
    let n = app.doc.features.len();
    let width = n as f32 * (CHIP + GAP) + GAP;
    let (strip, strip_response) = ui.allocate_exact_size(Vec2::new(width.max(60.0), CHIP + 4.0), Sense::hover());
    let painter = ui.painter_at(strip.expand(2.0));
    painter.rect_filled(strip, 4.0, Color32::from_rgb(232, 234, 238));

    let chip_rect = |i: usize| {
        Rect::from_min_size(Pos2::new(strip.min.x + GAP + i as f32 * (CHIP + GAP), strip.min.y + 2.0), Vec2::splat(CHIP))
    };
    // x of the gap before feature `i` (i == n is after the last).
    let gap_x = |i: usize| strip.min.x + GAP / 2.0 + i as f32 * (CHIP + GAP);

    let mut action = None;
    for i in 0..n {
        let rect = chip_rect(i);
        let id = ui.id().with(("chip", i));
        let response = ui.interact(rect, id, Sense::click());
        let feature = &app.doc.features[i];
        let status = app.status.get(i).cloned().unwrap_or(FeatureStatus::Ok);
        let inactive = i >= app.doc.marker || feature.suppressed;

        let selected = app.selected_feature == Some(i) || app.editing.as_ref().is_some_and(|e| e.index == i);
        let (fill, stroke) = match (&status, selected) {
            (FeatureStatus::Error(_), _) => (Color32::from_rgb(253, 228, 226), Stroke::new(1.5, theme::ERROR)),
            (_, true) => (Color32::from_rgb(214, 234, 250), Stroke::new(1.5, theme::ACCENT)),
            _ if response.hovered() => (Color32::WHITE, Stroke::new(1.0, theme::ACCENT.linear_multiply(0.6))),
            _ => (Color32::WHITE, Stroke::new(1.0, theme::BORDER)),
        };
        painter.rect_filled(rect, 3.0, fill);
        painter.rect_stroke(rect, 3.0, stroke, egui::StrokeKind::Inside);
        let icon_color = if inactive { Color32::from_gray(175) } else { theme::ICON };
        icons::paint(&painter, rect.shrink(4.0), icon_for(&feature.kind), icon_color);
        if feature.suppressed {
            painter.line_segment([rect.left_bottom(), rect.right_top()], Stroke::new(1.2, theme::MUTED));
        }

        let tip = match &status {
            FeatureStatus::Error(message) => format!("{}\n⚠ {message}", feature.name),
            FeatureStatus::Suppressed => format!("{} (suppressed)", feature.name),
            FeatureStatus::RolledBack => format!("{} (rolled back)", feature.name),
            FeatureStatus::Ok => feature.name.clone(),
        };
        let response = response.on_hover_text(tip);
        if response.clicked() {
            app.selected_feature = Some(i);
        }
        if response.double_clicked() {
            action = Some(ChipAction::Edit(i));
        }
        response.context_menu(|ui| {
            if ui.button("Edit Feature").clicked() {
                action = Some(ChipAction::Edit(i));
            }
            let suppress = if app.doc.features[i].suppressed { "Unsuppress Feature" } else { "Suppress Feature" };
            if ui.button(suppress).clicked() {
                action = Some(ChipAction::ToggleSuppress(i));
            }
            if ui.button("Roll History Marker Here").clicked() {
                action = Some(ChipAction::Marker(i + 1));
            }
            ui.separator();
            if ui.add_enabled(i > 0, egui::Button::new("Move Earlier")).clicked() {
                action = Some(ChipAction::Move(i, i - 1));
            }
            if ui.add_enabled(i + 1 < n, egui::Button::new("Move Later")).clicked() {
                action = Some(ChipAction::Move(i, i + 1));
            }
            ui.separator();
            if ui.button(RichText::new("Delete").color(theme::ERROR)).clicked() {
                action = Some(ChipAction::Delete(i));
            }
        });
    }

    // The history marker: drag it between features to roll the model back.
    let marker_x = gap_x(app.doc.marker.min(n));
    let handle = Rect::from_center_size(Pos2::new(marker_x, strip.center().y), Vec2::new(10.0, CHIP + 8.0));
    let marker_response = ui
        .interact(handle, ui.id().with("history_marker"), Sense::drag())
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
        .on_hover_text("History marker: drag to roll back");
    if marker_response.drag_started() {
        app.finish_edit(true);
        app.marker_drag = Some(app.doc.clone());
    }
    if marker_response.dragged() {
        if let Some(pos) = ui.ctx().pointer_latest_pos() {
            let target = (0..=n)
                .min_by(|&a, &b| (gap_x(a) - pos.x).abs().total_cmp(&(gap_x(b) - pos.x).abs()))
                .unwrap_or(n);
            if target != app.doc.marker {
                app.doc.marker = target;
                app.changed();
            }
        }
    }
    if marker_response.drag_stopped() {
        if let Some(before) = app.marker_drag.take() {
            if before != app.doc {
                app.history.record(before);
            }
        }
    }
    let active = marker_response.hovered() || marker_response.dragged();
    let color = if active { theme::ACCENT } else { Color32::from_rgb(90, 96, 106) };
    painter.vline(marker_x, strip.y_range().expand(1.0), Stroke::new(3.0, color));
    let top = strip.min.y - 1.0;
    painter.add(egui::Shape::convex_polygon(
        vec![Pos2::new(marker_x - 5.0, top), Pos2::new(marker_x + 5.0, top), Pos2::new(marker_x, top + 6.0)],
        color,
        Stroke::NONE,
    ));
    let _ = strip_response;

    match action {
        Some(ChipAction::Edit(i)) => app.begin_edit(i),
        Some(ChipAction::ToggleSuppress(i)) => {
            app.finish_edit(true);
            app.commit(|doc| doc.features[i].suppressed = !doc.features[i].suppressed);
        }
        Some(ChipAction::Marker(m)) => {
            app.finish_edit(true);
            app.commit(|doc| doc.marker = m);
        }
        Some(ChipAction::Move(from, to)) => {
            app.finish_edit(true);
            app.commit(|doc| doc.reorder(from, to));
            app.selected_feature = Some(to);
        }
        Some(ChipAction::Delete(i)) => app.delete_feature(i),
        None => {}
    }
}

enum ChipAction {
    Edit(usize),
    ToggleSuppress(usize),
    Marker(usize),
    Move(usize, usize),
    Delete(usize),
}

fn info(app: &FissionApp, ui: &mut egui::Ui) {
    let tris: usize = app.bodies.iter().filter(|b| b.visible).map(|b| b.mesh.triangle_count()).sum();
    ui.label(RichText::new(format!("{:.0} ms · {tris} tris", app.regen_ms)).small().color(theme::MUTED))
        .on_hover_text("Last regeneration time and displayed triangles");
    let shown = app.viewport.hovered.or(app.viewport.selection.last().copied());
    let Some(pick) = shown else { return };
    let name = app.bodies.get(pick.body()).map_or("?", |b| b.name.as_str());
    let text = match pick {
        Pick::Face(hit) => {
            let (p, n) = (hit.point, hit.normal);
            format!(
                "{name} · Face {} · ({:.2}, {:.2}, {:.2}) · n=({:.2}, {:.2}, {:.2})",
                hit.face, p.x, p.y, p.z, n.x, n.y, n.z
            )
        }
        Pick::Edge(e) => {
            let p = e.point;
            format!("{name} · Edge {} · ({:.2}, {:.2}, {:.2})", e.edge, p.x, p.y, p.z)
        }
    };
    let count = app.viewport.selection.len();
    let text = if count > 1 { format!("{text} · {count} selected") } else { text };
    ui.separator();
    ui.label(RichText::new(text).small());
}

pub mod browser;
pub mod appearance;
pub mod canvas;
pub mod commands;
pub mod construct;
pub mod dialog;
pub mod hud;
pub mod icons;
pub mod loft;
pub mod measure;
pub mod params;
pub mod section;
pub mod sketch;
pub mod theme;
pub mod timeline;
pub mod toolbar;

use eframe::egui::{self, Rect, RichText};

use crate::app::FissionApp;

/// Text drawn over the viewport: toasts and the empty-document hint.
pub fn overlay(app: &FissionApp, ui: &mut egui::Ui, viewport: Rect) {
    let painter = ui.painter_at(viewport);
    if app.doc.features.is_empty() {
        painter.text(
            viewport.center(),
            egui::Align2::CENTER_CENTER,
            "Empty design. Press E to extrude a rectangle, or use CREATE above.",
            egui::FontId::proportional(14.0),
            theme::muted(),
        );
    }

    if let Some(toast) = &app.toast {
        let pos = egui::pos2(viewport.center().x, viewport.max.y - 18.0);
        let color = if toast.error { theme::error() } else { theme::text() };
        let galley = painter.layout_no_wrap(toast.text.clone(), egui::FontId::proportional(13.0), color);
        let rect = Rect::from_center_size(pos, galley.size() + egui::vec2(20.0, 10.0));
        painter.rect_filled(rect, 4.0, egui::Color32::from_white_alpha(235));
        painter.rect_stroke(rect, 4.0, egui::Stroke::new(1.0, theme::border()), egui::StrokeKind::Inside);
        painter.galley(rect.center() - galley.size() / 2.0, galley, color);
    }

    // A rebuild long enough to notice: say so at the top of the viewport.
    match app.rebuilding_for() {
        Some(t) if t > std::time::Duration::from_millis(250) => {
            let rect = Rect::from_center_size(egui::pos2(viewport.center().x, viewport.min.y + 22.0), egui::vec2(124.0, 26.0));
            painter.rect_filled(rect, 4.0, theme::panel());
            painter.rect_stroke(rect, 4.0, egui::Stroke::new(1.0, theme::border()), egui::StrokeKind::Inside);
            ui.put(Rect::from_center_size(rect.left_center() + egui::vec2(16.0, 0.0), egui::vec2(14.0, 14.0)), egui::Spinner::new().size(14.0));
            painter.text(rect.left_center() + egui::vec2(30.0, 0.0), egui::Align2::LEFT_CENTER, "Rebuilding…", egui::FontId::proportional(13.0), theme::text());
        }
        Some(_) => ui.ctx().request_repaint_after(std::time::Duration::from_millis(260)),
        None => {}
    }

    // Sketch status: active tool, constraint state, last message.
    if let Some(session) = &app.sketch {
        let dof = if session.dof == 0 { "Fully constrained".to_string() } else { format!("{} degrees of freedom", session.dof) };
        let mut text = format!("{} · {dof}", session.tool.label());
        if let Some(msg) = session.message.as_deref().or(session.prompt()) {
            text = format!("{text} · {msg}");
        }
        painter.text(
            viewport.left_bottom() + egui::vec2(10.0, -26.0),
            egui::Align2::LEFT_BOTTOM,
            text,
            egui::FontId::proportional(13.0),
            if session.message.is_some() { theme::error() } else { theme::text() },
        );
    }

    // Mouse help in the bottom-left, like Fusion's navigation hint.
    let hint = "Middle-drag: pan · Shift+Middle or Right-drag: orbit · Wheel: zoom · Middle double-click: fit";
    painter.text(
        viewport.left_bottom() + egui::vec2(10.0, -8.0),
        egui::Align2::LEFT_BOTTOM,
        RichText::new(hint).text(),
        egui::FontId::proportional(11.0),
        theme::muted(),
    );
}

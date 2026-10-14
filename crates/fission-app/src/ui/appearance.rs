//! MODIFY ▸ Appearance (`A`): color the selected bodies (or the last one).

use eframe::egui::{self, ecolor, Color32, RichText, Stroke};

use super::theme;
use crate::app::FissionApp;

/// Generic finishes (plain sRGB colors; no textures).
const SWATCHES: &[(&str, [f32; 3])] = &[
    ("Aluminum", [0.78, 0.80, 0.82]),
    ("Steel", [0.55, 0.57, 0.60]),
    ("Brass", [0.80, 0.66, 0.32]),
    ("Copper", [0.78, 0.46, 0.30]),
    ("Black plastic", [0.12, 0.12, 0.13]),
    ("White plastic", [0.92, 0.92, 0.90]),
    ("Gray plastic", [0.50, 0.51, 0.53]),
    ("Red", [0.80, 0.18, 0.16]),
    ("Orange", [0.93, 0.50, 0.15]),
    ("Yellow", [0.95, 0.80, 0.20]),
    ("Green", [0.25, 0.62, 0.30]),
    ("Blue", [0.20, 0.42, 0.80]),
    ("Purple", [0.52, 0.32, 0.72]),
    ("Wood", [0.62, 0.45, 0.28]),
    ("Rubber", [0.20, 0.20, 0.22]),
    ("Glass", [0.70, 0.85, 0.90]),
];

pub struct AppearanceDialog {
    /// Bodies to color, by creating feature. Empty: the last body.
    pub targets: Vec<usize>,
    /// sRGB, like the swatches and `Feature::color`.
    pub custom: [f32; 3],
}

fn to_color32(c: [f32; 3]) -> Color32 {
    let [r, g, b] = c.map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8);
    Color32::from_rgb(r, g, b)
}

pub fn show(app: &mut FissionApp, ctx: &egui::Context) {
    let Some(dialog) = app.appearance.as_mut() else { return };
    let targets = if dialog.targets.is_empty() {
        app.model_bodies.last().map(|b| vec![b.1]).unwrap_or_default()
    } else {
        dialog.targets.clone()
    };
    let names: Vec<String> = targets
        .iter()
        .filter_map(|t| app.model_bodies.iter().find(|b| b.1 == *t).map(|b| b.0.clone()))
        .collect();
    let mut open = true;
    let mut apply: Option<Option<[f32; 3]>> = None;
    egui::Window::new(RichText::new("APPEARANCE").strong())
        .id(egui::Id::new("appearance"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(260.0)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            if names.is_empty() {
                ui.label(RichText::new("No body to color").color(theme::error()));
                return;
            }
            ui.label(RichText::new(format!("Applies to: {}", names.join(", "))).small().color(theme::muted()));
            ui.add_space(4.0);
            egui::Grid::new("swatches").num_columns(4).spacing([6.0, 6.0]).show(ui, |ui| {
                for (i, (name, rgb)) in SWATCHES.iter().enumerate() {
                    let (rect, response) = ui.allocate_exact_size(egui::vec2(52.0, 30.0), egui::Sense::click());
                    ui.painter().rect_filled(rect, 3.0, to_color32(*rgb));
                    if response.hovered() {
                        ui.painter().rect_stroke(rect, 3.0, Stroke::new(2.0, theme::accent()), egui::StrokeKind::Outside);
                    }
                    if response.on_hover_text(*name).clicked() {
                        apply = Some(Some(*rgb));
                    }
                    if i % 4 == 3 {
                        ui.end_row();
                    }
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Custom");
                // egui's float picker edits linear RGB; colors are kept as sRGB.
                let mut linear = dialog.custom.map(ecolor::linear_from_gamma);
                if ui.color_edit_button_rgb(&mut linear).changed() {
                    dialog.custom = linear.map(ecolor::gamma_from_linear);
                }
                if ui.button("Apply").clicked() {
                    apply = Some(Some(dialog.custom));
                }
                if ui.button("Default").on_hover_text("Back to the default gray").clicked() {
                    apply = Some(None);
                }
            });
        });
    if let Some(color) = apply {
        app.commit(|doc| {
            for &t in &targets {
                if let Some(f) = doc.features.get_mut(t) {
                    f.color = color;
                }
            }
        });
    }
    if !open {
        app.appearance = None;
    }
}

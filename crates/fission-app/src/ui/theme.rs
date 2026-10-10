use eframe::egui::{self, Color32, CornerRadius, Stroke};

pub const ACCENT: Color32 = Color32::from_rgb(6, 150, 215);
pub const PANEL: Color32 = Color32::from_rgb(244, 245, 247);
pub const TOOLBAR: Color32 = Color32::from_rgb(250, 250, 251);
pub const BORDER: Color32 = Color32::from_rgb(214, 217, 222);
pub const TEXT: Color32 = Color32::from_rgb(45, 48, 54);
pub const MUTED: Color32 = Color32::from_rgb(120, 125, 133);
pub const ERROR: Color32 = Color32::from_rgb(214, 59, 50);
pub const ICON: Color32 = Color32::from_rgb(70, 76, 86);

/// Light, flat, compact: close to Fusion's default look.
pub fn apply(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::light();
    visuals.panel_fill = PANEL;
    visuals.window_fill = Color32::WHITE;
    visuals.window_stroke = Stroke::new(1.0, BORDER);
    visuals.window_corner_radius = CornerRadius::same(4);
    visuals.menu_corner_radius = CornerRadius::same(3);
    visuals.extreme_bg_color = Color32::WHITE;
    visuals.selection.bg_fill = ACCENT.linear_multiply(0.35);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.hyperlink_color = ACCENT;
    visuals.override_text_color = Some(TEXT);
    for w in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        w.corner_radius = CornerRadius::same(3);
    }
    visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(236, 238, 241);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(222, 236, 248);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT.linear_multiply(0.6));
    visuals.widgets.active.weak_bg_fill = Color32::from_rgb(200, 226, 246);
    ctx.set_visuals(visuals);

    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(6.0, 4.0);
        style.spacing.button_padding = egui::vec2(6.0, 3.0);
        style.spacing.interact_size.y = 20.0;
        style.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(13.0));
        style.text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(13.0));
        style.text_styles.insert(egui::TextStyle::Small, egui::FontId::proportional(10.5));
    });
}

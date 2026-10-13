use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{self, Color32, CornerRadius, Stroke};

/// The UI's colors. Two palettes: light (close to Fusion's default look) and
/// dark. Read them through the functions below, which follow the current mode.
pub struct Palette {
    pub accent: Color32,
    pub panel: Color32,
    pub toolbar: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub error: Color32,
    pub icon: Color32,
    /// Dialogs, chips, the document tab, text fields.
    pub surface: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub button: Color32,
    /// Hover behind the green Finish Sketch check.
    pub ok_hover: Color32,
    pub strip: Color32,
    pub chip_selected: Color32,
    pub chip_error: Color32,
    pub icon_off: Color32,
    pub marker: Color32,
    /// Sketch curves when fully constrained, and sketch points.
    pub ink: Color32,
    /// Curves of sketches other than the one being edited.
    pub ink_inactive: Color32,
    pub label_bg: Color32,
    /// View cube: lightest face shade, outline, labels, home icon.
    pub cube_shade: u8,
    pub cube_stroke: Color32,
    pub cube_text: Color32,
    /// Viewport background gradient (linear RGB).
    pub bg_top: [f32; 3],
    pub bg_bottom: [f32; 3],
}

pub const LIGHT: Palette = Palette {
    accent: Color32::from_rgb(6, 150, 215),
    panel: Color32::from_rgb(244, 245, 247),
    toolbar: Color32::from_rgb(250, 250, 251),
    border: Color32::from_rgb(214, 217, 222),
    text: Color32::from_rgb(45, 48, 54),
    muted: Color32::from_rgb(120, 125, 133),
    error: Color32::from_rgb(214, 59, 50),
    icon: Color32::from_rgb(70, 76, 86),
    surface: Color32::WHITE,
    hover: Color32::from_rgb(222, 236, 248),
    pressed: Color32::from_rgb(200, 226, 246),
    button: Color32::from_rgb(236, 238, 241),
    ok_hover: Color32::from_rgb(222, 240, 226),
    strip: Color32::from_rgb(232, 234, 238),
    chip_selected: Color32::from_rgb(214, 234, 250),
    chip_error: Color32::from_rgb(253, 228, 226),
    icon_off: Color32::from_gray(175),
    marker: Color32::from_rgb(90, 96, 106),
    ink: Color32::from_rgb(20, 20, 20),
    ink_inactive: Color32::from_rgb(70, 90, 120),
    label_bg: Color32::from_rgba_premultiplied(220, 220, 220, 220),
    cube_shade: 200,
    cube_stroke: Color32::from_gray(110),
    cube_text: Color32::from_gray(60),
    bg_top: [0.97, 0.97, 0.98],
    bg_bottom: [0.74, 0.76, 0.79],
};

pub const DARK: Palette = Palette {
    accent: Color32::from_rgb(38, 170, 235),
    panel: Color32::from_rgb(34, 37, 43),
    toolbar: Color32::from_rgb(40, 43, 50),
    border: Color32::from_rgb(62, 67, 77),
    text: Color32::from_rgb(222, 226, 232),
    muted: Color32::from_rgb(140, 147, 158),
    error: Color32::from_rgb(240, 96, 86),
    icon: Color32::from_rgb(196, 202, 212),
    surface: Color32::from_rgb(46, 50, 58),
    hover: Color32::from_rgb(38, 66, 92),
    pressed: Color32::from_rgb(30, 86, 128),
    button: Color32::from_rgb(54, 58, 67),
    ok_hover: Color32::from_rgb(36, 74, 48),
    strip: Color32::from_rgb(28, 30, 35),
    chip_selected: Color32::from_rgb(30, 74, 110),
    chip_error: Color32::from_rgb(92, 38, 36),
    icon_off: Color32::from_gray(100),
    marker: Color32::from_rgb(170, 176, 186),
    ink: Color32::from_rgb(235, 238, 242),
    ink_inactive: Color32::from_rgb(140, 160, 190),
    label_bg: Color32::from_rgba_premultiplied(30, 33, 39, 220),
    cube_shade: 120,
    cube_stroke: Color32::from_gray(150),
    cube_text: Color32::from_gray(235),
    bg_top: [0.20, 0.22, 0.26],
    bg_bottom: [0.06, 0.07, 0.09],
};

static IS_DARK: AtomicBool = AtomicBool::new(false);

pub fn is_dark() -> bool {
    IS_DARK.load(Ordering::Relaxed)
}

pub fn palette() -> &'static Palette {
    if is_dark() { &DARK } else { &LIGHT }
}

pub fn accent() -> Color32 {
    palette().accent
}
pub fn panel() -> Color32 {
    palette().panel
}
pub fn toolbar() -> Color32 {
    palette().toolbar
}
pub fn border() -> Color32 {
    palette().border
}
pub fn text() -> Color32 {
    palette().text
}
pub fn muted() -> Color32 {
    palette().muted
}
pub fn error() -> Color32 {
    palette().error
}
pub fn icon() -> Color32 {
    palette().icon
}

/// Where the choice is kept between runs.
fn settings_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))?;
    Some(base.join("fission/theme"))
}

/// The saved mode, else the system's (dark if egui reports a dark OS theme).
/// `FISSION_THEME=dark|light` overrides both.
pub fn init(ctx: &egui::Context) {
    let saved = std::env::var("FISSION_THEME")
        .ok()
        .or_else(|| settings_path().and_then(|p| std::fs::read_to_string(p).ok()))
        .map(|s| s.trim().eq_ignore_ascii_case("dark"));
    let dark = saved.unwrap_or_else(|| ctx.system_theme() == Some(egui::Theme::Dark));
    IS_DARK.store(dark, Ordering::Relaxed);
    apply(ctx);
}

/// Pick the mode without a UI (headless rendering), and without saving it.
pub fn use_dark(dark: bool) {
    IS_DARK.store(dark, Ordering::Relaxed);
}

/// Switch modes and remember the choice.
pub fn set_dark(ctx: &egui::Context, dark: bool) {
    IS_DARK.store(dark, Ordering::Relaxed);
    apply(ctx);
    if let Some(path) = settings_path() {
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        let _ = std::fs::write(path, if dark { "dark\n" } else { "light\n" });
    }
}

/// Flat and compact, in the current mode's colors.
pub fn apply(ctx: &egui::Context) {
    let p = palette();
    let mut visuals = if is_dark() { egui::Visuals::dark() } else { egui::Visuals::light() };
    visuals.panel_fill = p.panel;
    visuals.window_fill = p.surface;
    visuals.window_stroke = Stroke::new(1.0, p.border);
    visuals.window_corner_radius = CornerRadius::same(4);
    visuals.menu_corner_radius = CornerRadius::same(3);
    visuals.extreme_bg_color = p.surface;
    visuals.faint_bg_color = p.toolbar;
    visuals.selection.bg_fill = p.accent.linear_multiply(0.35);
    visuals.selection.stroke = Stroke::new(1.0, p.accent);
    visuals.hyperlink_color = p.accent;
    visuals.override_text_color = Some(p.text);
    for w in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        w.corner_radius = CornerRadius::same(3);
    }
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    visuals.widgets.inactive.weak_bg_fill = p.button;
    visuals.widgets.inactive.bg_fill = p.button;
    visuals.widgets.hovered.weak_bg_fill = p.hover;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, p.accent.linear_multiply(0.6));
    visuals.widgets.active.weak_bg_fill = p.pressed;
    visuals.widgets.open.weak_bg_fill = p.hover;
    ctx.set_visuals_of(egui::Theme::Light, visuals.clone());
    ctx.set_visuals_of(egui::Theme::Dark, visuals);
    ctx.set_theme(if is_dark() { egui::Theme::Dark } else { egui::Theme::Light });

    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(6.0, 4.0);
        style.spacing.button_padding = egui::vec2(6.0, 3.0);
        style.spacing.interact_size.y = 20.0;
        style.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(13.0));
        style.text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(13.0));
        style.text_styles.insert(egui::TextStyle::Small, egui::FontId::proportional(10.5));
    });
}

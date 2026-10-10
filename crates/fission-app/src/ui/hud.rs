//! The heads-up value box: while a command is active, just type a number
//! (or an expression like `10+5` or `1 in`) and press Enter, as in Fusion.
//! In a feature dialog it sets the main value and accepts the feature; while
//! sketching it sets the length, diameter, or size of what's being drawn.

use eframe::egui::{self, RichText};
use fission_model::names_in;

use super::sketch::Tool;
use super::theme;
use crate::app::FissionApp;

pub struct Hud {
    pub text: String,
    pub error: Option<String>,
    fresh: bool,
}

impl Hud {
    pub fn new(first: String) -> Hud {
        Hud { text: first, error: None, fresh: true }
    }
}

/// What typing would set right now, if anything.
fn target_label(app: &mut FissionApp) -> Option<&'static str> {
    if let Some(s) = &app.sketch {
        if s.tool == Tool::Offset {
            return s.offset_curve.map(|_| "Offset distance");
        }
        if s.pending.is_empty() {
            return None;
        }
        return match s.tool {
            Tool::Line => Some("Length"),
            Tool::Circle => Some("Diameter"),
            Tool::Rectangle => Some("Width, height"),
            Tool::Polygon => Some("Radius"),
            Tool::Slot if s.pending.len() == 2 => Some("Width"),
            _ => None,
        };
    }
    let index = app.editing.as_ref()?.index;
    let kind = &mut app.doc.features.get_mut(index)?.kind;
    kind.primary_value().map(|(_, label, _, _)| label)
}

pub fn wants_typed_value(app: &mut FissionApp) -> bool {
    target_label(app).is_some()
}

/// Apply typed text as if entered in the box (for scripted checks).
pub fn commit_text(app: &mut FissionApp, text: &str) -> Result<(), String> {
    commit(app, text)
}

/// Apply the typed text. Returns an error to show in the box.
fn commit(app: &mut FissionApp, text: &str) -> Result<(), String> {
    if app.sketch.is_some() {
        return super::sketch::apply_typed(app, text);
    }
    let index = app.editing.as_ref().ok_or("no command is active")?.index;
    let quantity = {
        let kind = &mut app.doc.features.get_mut(index).ok_or("no command is active")?.kind;
        kind.primary_value().ok_or("this command has no typed value")?.2
    };
    // Parameters are allowed: `width / 2`.
    let value = app.doc.evaluate(text, quantity)?;
    let feature = &mut app.doc.features[index];
    let (key, _, _, field) = feature.kind.primary_value().unwrap();
    *field = value;
    // Keep the expression if it names parameters, so the value follows them.
    if names_in(text).is_empty() {
        feature.exprs.remove(key);
    } else {
        feature.exprs.insert(key.to_string(), text.trim().to_string());
    }
    app.changed();
    // Enter accepts the command, like Fusion.
    app.finish_edit(true);
    Ok(())
}

pub fn show(app: &mut FissionApp, ctx: &egui::Context) {
    let Some(label) = (if app.hud.is_some() { target_label(app) } else { None }) else {
        app.hud = None;
        return;
    };
    let pos = ctx.pointer_latest_pos().unwrap_or(app.viewport.rect.center()) + egui::vec2(18.0, 14.0);
    let mut submit = false;
    let mut cancel = false;
    let hud = app.hud.as_mut().unwrap();
    egui::Area::new(egui::Id::new("value_hud")).fixed_pos(pos).order(egui::Order::Foreground).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).small().color(theme::muted()));
                let edit = egui::TextEdit::singleline(&mut hud.text).desired_width(90.0).id(egui::Id::new("value_hud_text"));
                let response = ui.add(edit);
                if hud.fresh {
                    // Keep the first typed character and put the caret after it.
                    response.request_focus();
                    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), response.id) {
                        let end = egui::text::CCursor::new(hud.text.chars().count());
                        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
                        state.store(ui.ctx(), response.id);
                    }
                    hud.fresh = false;
                }
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    submit = true;
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });
            if let Some(err) = &hud.error {
                ui.label(RichText::new(err).small().color(theme::error()));
            }
        });
    });
    if cancel {
        app.hud = None;
    } else if submit {
        let text = app.hud.as_ref().unwrap().text.clone();
        match commit(app, &text) {
            Ok(()) => app.hud = None,
            Err(e) => {
                if let Some(h) = &mut app.hud {
                    h.error = Some(e);
                }
            }
        }
    }
}

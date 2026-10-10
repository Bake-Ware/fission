//! MODIFY ▸ Change Parameters: the user parameter table, and the model
//! values that follow an expression.

use eframe::egui::{self, RichText};
use fission_model::{Parameter, Quantity};

use super::theme;
use crate::app::FissionApp;

/// Open state: the document as it was before the current edit (for undo).
pub struct ParamsDialog {
    pub before: fission_model::Document,
}

fn unit(q: Quantity) -> &'static str {
    match q {
        Quantity::Length => "mm",
        Quantity::Angle => "deg",
        Quantity::Count => "",
    }
}

pub fn show(app: &mut FissionApp, ctx: &egui::Context) {
    if app.params_dialog.is_none() {
        return;
    }
    let values = app.doc.parameter_values();
    let mut open = true;
    let mut commit = false;
    let mut remove = None;
    egui::Window::new(RichText::new("PARAMETERS").strong())
        .id(egui::Id::new("parameters"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(560.0)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("User Parameters").strong());
                if ui.small_button("+").on_hover_text("Add a parameter").clicked() {
                    let n = (1..).find(|n| !app.doc.parameters.iter().any(|p| p.name == format!("param{n}"))).unwrap();
                    app.doc.parameters.push(Parameter {
                        name: format!("param{n}"),
                        expression: "10 mm".into(),
                        quantity: Quantity::Length,
                        comment: String::new(),
                    });
                    commit = true;
                }
            });
            egui::Grid::new("param_grid").num_columns(6).striped(true).spacing([8.0, 4.0]).show(ui, |ui| {
                for h in ["Name", "Unit", "Expression", "Value", "Comment", ""] {
                    ui.label(RichText::new(h).small().color(theme::MUTED));
                }
                ui.end_row();
                for (i, p) in app.doc.parameters.iter_mut().enumerate() {
                    commit |= ui.add_sized([100.0, 20.0], egui::TextEdit::singleline(&mut p.name)).lost_focus();
                    egui::ComboBox::from_id_salt(("param_unit", i)).width(50.0).selected_text(unit(p.quantity)).show_ui(ui, |ui| {
                        for q in [Quantity::Length, Quantity::Angle, Quantity::Count] {
                            commit |= ui.selectable_value(&mut p.quantity, q, unit(q)).changed();
                        }
                    });
                    commit |= ui.add_sized([140.0, 20.0], egui::TextEdit::singleline(&mut p.expression)).lost_focus();
                    match values.as_ref().map(|v| v.get(&p.name).copied()) {
                        Ok(Some(v)) => ui.label(format!("{v:.3} {}", unit(p.quantity))),
                        _ => ui.label(RichText::new("—").color(theme::ERROR)),
                    };
                    ui.add_sized([120.0, 20.0], egui::TextEdit::singleline(&mut p.comment));
                    if ui.small_button("−").on_hover_text("Delete this parameter").clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
            if let Err(e) = &values {
                ui.label(RichText::new(e).color(theme::ERROR));
            }

            // Feature values tied to expressions.
            let bound: Vec<String> = app
                .doc
                .features
                .iter()
                .flat_map(|f| f.exprs.iter().map(move |(k, e)| format!("{} · {k} = {e}", f.name)))
                .collect();
            if !bound.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new("Model values following parameters").strong());
                for b in bound {
                    ui.label(RichText::new(b).small());
                }
            }
            ui.add_space(4.0);
            ui.label(
                RichText::new("Use a name in any value box: type `width / 2` while extruding, or in a sketch dimension.")
                    .small()
                    .color(theme::MUTED),
            );
        });
    if let Some(i) = remove {
        app.doc.parameters.remove(i);
        commit = true;
    }
    if commit {
        if let Some(d) = app.params_dialog.as_mut() {
            if d.before != app.doc {
                let before = std::mem::replace(&mut d.before, app.doc.clone());
                app.history.record(before);
            }
        }
        app.changed();
    }
    if !open {
        app.params_dialog = None;
    }
}

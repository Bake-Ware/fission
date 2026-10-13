use eframe::egui::{self, RichText, Stroke};

use super::icons::{self, Icon};
use super::theme;
use crate::app::FissionApp;

pub fn show(app: &mut FissionApp, ui: &mut egui::Ui) {
    egui::Panel::left("browser")
        .resizable(true)
        .default_size(220.0)
        .frame(
            egui::Frame::NONE
                .fill(theme::panel())
                .stroke(Stroke::new(1.0, theme::border()))
                .inner_margin(egui::Margin::symmetric(8, 6)),
        )
        .show(ui, |ui| {
            ui.label(RichText::new("BROWSER").small().strong().color(theme::muted()));
            ui.add_space(2.0);
            egui::ScrollArea::vertical().show(ui, |ui| tree(app, ui));
        });
}

/// An imported body's parts (a Fusion design's bodies, a STEP assembly's
/// solids), each with its own visibility.
fn parts(app: &mut FissionApp, ui: &mut egui::Ui, feature: usize) {
    let count = app.import_parts.get(feature).copied().unwrap_or(0);
    let Some(fission_model::FeatureKind::ImportStep { hidden_parts, .. }) = app.doc.features.get(feature).map(|f| &f.kind) else {
        return;
    };
    if count < 2 {
        return;
    }
    let hidden = hidden_parts.clone();
    ui.indent(("parts", feature), |ui| {
        egui::CollapsingHeader::new(format!("Parts ({} of {count} shown)", count - hidden.len().min(count)))
            .id_salt(("parts", feature))
            .show(ui, |ui| {
                let mut toggle = None;
                ui.horizontal(|ui| {
                    if ui.small_button("Show all").clicked() {
                        toggle = Some(None);
                    }
                });
                for p in 0..count {
                    ui.horizontal(|ui| {
                        let visible = !hidden.contains(&p);
                        let eye = if visible { Icon::Eye } else { Icon::EyeOff };
                        if icons::button(ui, eye, 16.0, if visible { "Hide" } else { "Show" }).clicked() {
                            toggle = Some(Some((p, !visible)));
                        }
                        let text = RichText::new(format!("Part {}", p + 1));
                        ui.label(if visible { text } else { text.color(theme::muted()) });
                    });
                }
                match toggle {
                    Some(Some((p, v))) => app.set_import_part_visible(feature, p, v),
                    Some(None) => app.commit(|doc| {
                        if let Some(fission_model::FeatureKind::ImportStep { hidden_parts, .. }) =
                            doc.features.get_mut(feature).map(|f| &mut f.kind)
                        {
                            hidden_parts.clear();
                        }
                    }),
                    None => {}
                }
            });
    });
}

/// A slider per joint: dragging moves its bodies live, releasing keeps the
/// value. The play button swings it between its limits.
fn joints(app: &mut FissionApp, ui: &mut egui::Ui) {
    let joints = app.doc.joints();
    if joints.is_empty() {
        return;
    }
    egui::CollapsingHeader::new(format!("Joints ({})", joints.len())).id_salt("joints").default_open(true).show(ui, |ui| {
        for joint in joints {
            let preview = app.joint_preview.as_ref().filter(|p| p.feature == joint.feature);
            let playing = preview.is_some_and(|p| p.playing.is_some());
            let mut value = preview.map_or(joint.value, |p| p.value);
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                icons::paint(ui.painter(), rect, Icon::Joint, theme::icon());
                let response = ui.selectable_label(app.selected_feature == Some(joint.feature), &joint.name);
                if response.clicked() {
                    app.selected_feature = Some(joint.feature);
                }
                if response.double_clicked() {
                    app.begin_edit(joint.feature);
                }
                let label = if playing { "Stop" } else { "Play" };
                if ui.small_button(label).on_hover_text("Swing between the limits").clicked() {
                    app.toggle_joint_play(joint.feature);
                }
            });
            let suffix = if joint.motion == fission_model::JointMotion::Revolute { "deg" } else { "mm" };
            let range = joint.min.min(joint.max)..=joint.max.max(joint.min);
            // Fixed widths: a row that changes width while playing makes the
            // panel re-lay itself out every frame.
            let response = ui
                .horizontal(|ui| {
                    ui.spacing_mut().slider_width = 110.0;
                    let r = ui.add_enabled(!playing, egui::Slider::new(&mut value, range).show_value(false));
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(64.0, 16.0), egui::Sense::hover());
                    ui.painter().text(rect.right_center(), egui::Align2::RIGHT_CENTER, format!("{value:.1} {suffix}"), egui::FontId::proportional(12.0), theme::text());
                    r
                })
                .inner;
            if response.changed() {
                app.preview_joint(joint.feature, value);
            }
            // A drag commits on release; a typed or clicked value at once.
            if response.drag_stopped() || (response.changed() && !response.dragged()) {
                app.commit_joint_preview();
            }
        }
    });
}

fn tree(app: &mut FissionApp, ui: &mut egui::Ui) {
    egui::CollapsingHeader::new(RichText::new(&app.doc.name).strong())
        .id_salt("doc_root")
        .default_open(true)
        .show(ui, |ui| {
            egui::CollapsingHeader::new("Document Settings").id_salt("settings").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut app.doc.name);
                });
                ui.label(RichText::new("Units: mm").color(theme::muted()));
            });

            egui::CollapsingHeader::new("Origin").id_salt("origin").show(ui, |ui| {
                for (name, color) in [("X", egui::Color32::from_rgb(217, 51, 51)), ("Y", egui::Color32::from_rgb(51, 166, 64)), ("Z", egui::Color32::from_rgb(51, 102, 217))] {
                    ui.label(RichText::new(format!("{name} axis")).color(color));
                }
                ui.label("XY plane");
                ui.label("XZ plane");
                ui.label("YZ plane");
            });

            let count = app.bodies.len();
            egui::CollapsingHeader::new(format!("Bodies ({count})"))
                .id_salt("bodies")
                .default_open(true)
                .show(ui, |ui| {
                    if count == 0 {
                        ui.label(RichText::new("No bodies").color(theme::muted()));
                    }
                    for i in 0..count {
                        ui.horizontal(|ui| {
                            let visible = app.bodies[i].visible;
                            let eye = if visible { Icon::Eye } else { Icon::EyeOff };
                            if icons::button(ui, eye, 18.0, if visible { "Hide" } else { "Show" }).clicked() {
                                app.set_body_visible(app.bodies[i].created_by, !visible);
                            }
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                            icons::paint(ui.painter(), rect, Icon::Body, theme::icon());
                            let faces = app.bodies[i].mesh.face_ids.iter().max().map_or(0, |m| m + 1);
                            let selected = app.viewport.selection.iter().any(|p| p.body() == i);
                            let response = ui
                                .selectable_label(selected, &app.bodies[i].name)
                                .on_hover_text(format!("{faces} faces, {} triangles", app.bodies[i].mesh.triangle_count()));
                            let created_by = app.bodies[i].created_by;
                            response.context_menu(|ui| {
                                if ui.button("Appearance…").clicked() {
                                    app.appearance = Some(crate::ui::appearance::AppearanceDialog {
                                        targets: vec![created_by],
                                        custom: app.doc.features.get(created_by).and_then(|f| f.color).unwrap_or([0.6, 0.6, 0.65]),
                                    });
                                    ui.close();
                                }
                                if ui.button(if visible { "Hide" } else { "Show" }).clicked() {
                                    app.set_body_visible(created_by, !visible);
                                    ui.close();
                                }
                            });
                        });
                        parts(app, ui, app.bodies[i].created_by);
                    }
                });

            joints(app, ui);

            egui::CollapsingHeader::new(format!("Features ({})", app.doc.features.len()))
                .id_salt("features")
                .show(ui, |ui| {
                    let mut edit = None;
                    for (i, feature) in app.doc.features.iter().enumerate() {
                        let selected = app.selected_feature == Some(i);
                        let mut text = RichText::new(&feature.name);
                        if i >= app.doc.marker || feature.suppressed {
                            text = text.color(theme::muted());
                        }
                        if matches!(app.status.get(i), Some(fission_model::FeatureStatus::Error(_))) {
                            text = text.color(theme::error());
                        }
                        let response = ui.selectable_label(selected, text);
                        if response.clicked() {
                            app.selected_feature = Some(i);
                        }
                        if response.double_clicked() {
                            edit = Some(i);
                        }
                    }
                    if let Some(i) = edit {
                        app.begin_edit(i);
                    }
                });
        });
}

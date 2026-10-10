//! INSPECT ▸ Section Analysis: cut the view along a plane to see inside.
//! A view setting, like Fusion's analysis items; it isn't in the timeline.

use eframe::egui::{self, RichText};
use fission_model::OriginPlane;

use crate::app::FissionApp;

pub struct Section {
    pub plane: OriginPlane,
    pub offset: f64,
    /// Keep the other side.
    pub flip: bool,
}

impl Section {
    /// (normal, d): the viewport cuts away where normal·p > d.
    pub fn clip(&self) -> glam::Vec4 {
        let p = self.plane.plane(self.offset);
        let s = if self.flip { -1.0 } else { 1.0 };
        let n = glam::DVec3::from_array(p.normal) * s;
        let d = n.dot(glam::DVec3::from_array(p.origin));
        glam::Vec4::new(n.x as f32, n.y as f32, n.z as f32, d as f32)
    }
}

pub fn show(app: &mut FissionApp, ctx: &egui::Context, viewport: egui::Rect) {
    let Some(section) = app.section.as_mut() else {
        app.viewport.section = None;
        return;
    };
    let mut close = false;
    egui::Window::new(RichText::new("SECTION ANALYSIS").strong())
        .id(egui::Id::new("section"))
        .collapsible(false)
        .resizable(false)
        .default_width(240.0)
        .fixed_pos(egui::pos2(viewport.max.x - 270.0, viewport.min.y + 120.0))
        .show(ctx, |ui| {
            egui::Grid::new("section_fields").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label("Plane");
                egui::ComboBox::from_id_salt("section_plane").selected_text(section.plane.label()).show_ui(ui, |ui| {
                    for p in OriginPlane::ALL {
                        ui.selectable_value(&mut section.plane, p, p.label());
                    }
                });
                ui.end_row();
                ui.label("Distance");
                ui.add(egui::DragValue::new(&mut section.offset).speed(0.5).suffix(" mm"));
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut section.flip, "Flip side");
                ui.end_row();
            });
            ui.add_space(4.0);
            if ui.button("Close").clicked() {
                close = true;
            }
        });
    app.viewport.section = Some(section.clip());
    if close {
        app.section = None;
        app.viewport.section = None;
    }
}

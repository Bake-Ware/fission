//! INSPECT ▸ Measure: click one or two faces or edges and read off lengths,
//! areas, volumes, distances and angles, like Fusion's measure box.

use eframe::egui::{self, Color32, RichText, Stroke};
use glam::Vec3;

use super::theme;
use crate::app::FissionApp;
use crate::viewport::Pick;

#[derive(Default)]
pub struct Measure {
    /// Up to two picks; a third click starts over from it.
    pub picks: Vec<Pick>,
}

impl Measure {
    pub fn add(&mut self, pick: Pick) {
        if self.picks.len() >= 2 {
            self.picks.clear();
        }
        self.picks.push(pick);
    }
}

/// A picked edge's polyline, in model space.
fn edge_polyline(app: &FissionApp, body: usize, edge: u32) -> Option<&[[f32; 3]]> {
    app.bodies.get(body)?.mesh.edges.get(edge as usize).map(Vec::as_slice)
}

fn polyline_length(pts: &[[f32; 3]]) -> f32 {
    pts.windows(2).map(|w| (Vec3::from(w[1]) - Vec3::from(w[0])).length()).sum()
}

/// Straight edges have a direction; curved ones don't.
fn edge_direction(pts: &[[f32; 3]]) -> Option<Vec3> {
    let (a, b) = (Vec3::from(*pts.first()?), Vec3::from(*pts.last()?));
    let d = (b - a).normalize_or_zero();
    let straight = pts.iter().all(|p| (Vec3::from(*p) - a).cross(d).length() < 1e-3 * (b - a).length().max(1.0));
    (straight && d != Vec3::ZERO).then_some(d)
}

fn describe(app: &FissionApp, pick: &Pick) -> String {
    let name = app.bodies.get(pick.body()).map_or("?", |b| b.name.as_str());
    match pick {
        Pick::Face(h) => format!("{name} · face {}", h.face),
        Pick::Edge(e) => format!("{name} · edge {}", e.edge),
    }
}

fn point(pick: &Pick) -> Vec3 {
    match pick {
        Pick::Face(h) => h.point,
        Pick::Edge(e) => e.point,
    }
}

fn mm(v: f32) -> String {
    format!("{v:.3} mm")
}

/// Rows of (label, value) for the current picks.
pub fn results(app: &FissionApp, m: &Measure) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    match m.picks.as_slice() {
        [] => {}
        [one] => match one {
            Pick::Edge(e) => {
                if let Some(pts) = edge_polyline(app, e.body, e.edge) {
                    out.push(("Length", mm(polyline_length(pts))));
                }
            }
            Pick::Face(h) => {
                let mesh = app.bodies.get(h.body).map(|b| &b.mesh);
                if let Some(area) = mesh.and_then(|m| m.face_areas.get(h.face as usize)) {
                    out.push(("Area", format!("{area:.3} mm²")));
                }
                let p = h.point;
                out.push(("Point", format!("({:.3}, {:.3}, {:.3})", p.x, p.y, p.z)));
                let n = h.normal;
                out.push(("Normal", format!("({:.3}, {:.3}, {:.3})", n.x, n.y, n.z)));
                // The whole body, as Fusion shows when a body is selected.
                if let Some(m) = mesh.filter(|m| m.volume > 0.0) {
                    out.push(("Body volume", format!("{:.3} mm³", m.volume)));
                    out.push(("Body area", format!("{:.3} mm²", m.face_areas.iter().sum::<f64>())));
                }
            }
        },
        [a, b] => {
            let (pa, pb) = (point(a), point(b));
            out.push(("Distance", mm((pb - pa).length())));
            let d = pb - pa;
            out.push(("ΔX ΔY ΔZ", format!("{:.3}, {:.3}, {:.3}", d.x, d.y, d.z)));
            let dir = |p: &Pick| match p {
                Pick::Face(h) => Some(h.normal),
                Pick::Edge(e) => edge_polyline(app, e.body, e.edge).and_then(edge_direction),
            };
            if let (Some(u), Some(v)) = (dir(a), dir(b)) {
                let angle = u.dot(v).clamp(-1.0, 1.0).acos().to_degrees();
                // Face normals: report the angle between the faces.
                out.push(("Angle", format!("{angle:.2}°")));
                if let (Pick::Face(fa), Pick::Face(_)) = (a, b) {
                    if u.cross(v).length() < 1e-4 {
                        out.push(("Face to face", mm(d.dot(fa.normal).abs())));
                    }
                }
            }
        }
        _ => {}
    }
    out
}

pub fn show(app: &mut FissionApp, ctx: &egui::Context, viewport: egui::Rect) {
    let Some(m) = app.measure.as_ref() else { return };
    let rows = results(app, m);
    let labels: Vec<String> = m.picks.iter().map(|p| describe(app, p)).collect();
    let points: Vec<Vec3> = m.picks.iter().map(point).collect();
    let mut close = false;
    egui::Window::new(RichText::new("MEASURE").strong())
        .id(egui::Id::new("measure"))
        .collapsible(false)
        .resizable(false)
        .default_width(240.0)
        .fixed_pos(egui::pos2(viewport.max.x - 270.0, viewport.min.y + 120.0))
        .show(ctx, |ui| {
            if labels.is_empty() {
                ui.label(RichText::new("Click a face or an edge.").color(theme::muted()));
            }
            for (i, l) in labels.iter().enumerate() {
                ui.label(RichText::new(format!("{}: {l}", i + 1)).small().color(theme::muted()));
            }
            egui::Grid::new("measure_rows").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                for (k, v) in &rows {
                    ui.label(*k);
                    ui.label(RichText::new(v).strong());
                    ui.end_row();
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("Clear").clicked() {
                    if let Some(m) = &mut app.measure {
                        m.picks.clear();
                    }
                }
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
        });
    // A line between the two picks, as Fusion draws.
    if let [a, b] = points.as_slice() {
        if let (Some(sa), Some(sb)) = (app.viewport.to_screen(*a), app.viewport.to_screen(*b)) {
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("measure_line")));
            painter.line_segment([sa, sb], Stroke::new(2.0, theme::accent()));
            for s in [sa, sb] {
                painter.circle_filled(s, 4.0, Color32::WHITE);
                painter.circle_stroke(s, 4.0, Stroke::new(1.5, theme::accent()));
            }
        }
    }
    if close {
        app.measure = None;
    }
}

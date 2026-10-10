mod camera;
mod pick;
mod renderer;

pub use camera::{Camera, Ray, StandardView};
pub use pick::{EdgeHit, Hit, Pick};

use eframe::egui::{self, Color32, PointerButton, Pos2, Rect, Sense, Stroke, Vec2 as EVec2};
use eframe::egui_wgpu::RenderState;
use fission_kernel::Mesh;
use glam::{Vec2, Vec3, Vec4Swizzles};

use renderer::{Frame, ViewportRenderer};

/// One body's display data.
pub struct DisplayBody {
    pub name: String,
    pub mesh: Mesh,
    pub visible: bool,
    /// Index of the feature that created the body (its stable identity).
    pub created_by: usize,
}

/// Entities an open dialog has picked, by display body index.
#[derive(Default)]
pub struct Highlights {
    pub edges: Vec<(usize, u32)>,
    pub faces: Vec<(usize, u32)>,
    pub bodies: Vec<usize>,
}

/// What the user did in the viewport this frame. The app owns the selection,
/// so commands (like an open Fillet dialog) can consume clicks.
#[derive(Default)]
pub struct ViewportEvents {
    /// Left click on an entity; `toggle` when Shift or Ctrl was held.
    pub clicked: Option<(Pick, bool)>,
    /// Left click on empty space.
    pub clicked_empty: bool,
    /// Raw left-button state, for tools that do their own picking (sketching).
    pub primary_clicked: bool,
    pub primary_double_clicked: bool,
    pub primary_drag_started: bool,
    pub primary_dragging: bool,
    pub primary_drag_stopped: bool,
    /// Shift or Ctrl held.
    pub toggle: bool,
}

pub struct Viewport {
    pub camera: Camera,
    renderer: ViewportRenderer,
    pub hovered: Option<Pick>,
    pub selection: Vec<Pick>,
    /// Off while sketching: the sketch tools pick 2D geometry themselves.
    pub pick_enabled: bool,
    /// Screen rect and pointer (viewport-local) from the last frame.
    pub rect: Rect,
    pub pointer: Option<Vec2>,
    meshes_dirty: bool,
    /// Where the grid goes (the ground, or the open sketch's plane) and its
    /// spacing; the renderer rebuilds when this changes.
    pub grid: (fission_kernel::Plane, f64),
    /// INSPECT ▸ Section Analysis: cut away where normal·p > d.
    pub section: Option<glam::Vec4>,
    /// Appearance per display body (alpha 0 = default).
    pub body_colors: Vec<[f32; 4]>,
    grid_built: Option<(fission_kernel::Plane, f64)>,
}

impl Viewport {
    pub fn new(rs: &RenderState) -> Self {
        Viewport {
            camera: Camera::default(),
            renderer: ViewportRenderer::new(rs),
            hovered: None,
            selection: Vec::new(),
            pick_enabled: true,
            rect: Rect::NOTHING,
            pointer: None,
            meshes_dirty: true,
            grid: (fission_kernel::Plane::XY, 10.0),
            grid_built: None,
            section: None,
            body_colors: Vec::new(),
        }
    }

    /// Mark meshes for re-upload and drop selected entities that no longer exist.
    pub fn bodies_changed(&mut self, bodies: &[DisplayBody]) {
        self.meshes_dirty = true;
        self.hovered = None;
        self.selection.retain(|p| match p {
            Pick::Face(h) => bodies.get(h.body).is_some_and(|b| b.mesh.face_ids.contains(&h.face)),
            Pick::Edge(e) => bodies.get(e.body).is_some_and(|b| (e.edge as usize) < b.mesh.edges.len()),
        });
    }

    /// Toggle or replace the selection with `pick`.
    pub fn select(&mut self, pick: Pick, toggle: bool) {
        if toggle {
            match self.selection.iter().position(|p| p.same_entity(&pick)) {
                Some(i) => {
                    self.selection.remove(i);
                }
                None => self.selection.push(pick),
            }
        } else {
            self.selection = vec![pick];
        }
    }

    pub fn selected_face(&self) -> Option<&Hit> {
        self.selection.iter().find_map(Pick::face)
    }

    /// The model edge under the pointer, even while picking is off (for
    /// sketch Project).
    pub fn edge_under_pointer(&self, bodies: &[DisplayBody]) -> Option<EdgeHit> {
        let visible = bodies.iter().enumerate().filter(|(_, b)| b.visible).map(|(i, b)| (i, &b.mesh));
        match pick::pick_any_clipped(&self.camera, self.pointer?, self.size(), visible, self.section)? {
            Pick::Edge(e) => Some(e),
            Pick::Face(_) => None,
        }
    }

    pub fn size(&self) -> Vec2 {
        Vec2::new(self.rect.width().max(1.0), self.rect.height().max(1.0))
    }

    /// World point to screen position (egui points).
    pub fn to_screen(&self, p: Vec3) -> Option<Pos2> {
        self.camera.project(p, self.size()).map(|q| Pos2::new(self.rect.min.x + q.x, self.rect.min.y + q.y))
    }

    /// The ray under the pointer, if it's over the viewport.
    pub fn pointer_ray(&self) -> Option<Ray> {
        self.pointer.map(|p| self.camera.ray(p, self.size()))
    }

    /// World units per screen point at the camera target's depth.
    pub fn world_per_point(&self) -> f32 {
        2.0 * self.camera.distance * (self.camera.fov_y / 2.0).tan() / self.size().y
    }

    pub fn fit_all(&mut self, bodies: &[DisplayBody]) {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for body in bodies.iter().filter(|b| b.visible) {
            for &p in &body.mesh.positions {
                min = min.min(p.into());
                max = max.max(p.into());
            }
        }
        if min.x.is_finite() {
            self.camera.fit(min, max);
        } else {
            self.camera.fit(Vec3::splat(-50.0), Vec3::splat(50.0));
        }
    }

    /// Draw the viewport. `extra` highlights what an open dialog has picked
    /// (on top of the selection).
    pub fn show(&mut self, ui: &mut egui::Ui, rs: &RenderState, bodies: &[DisplayBody], extra: &Highlights) -> ViewportEvents {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        self.rect = rect;
        if self.meshes_dirty {
            let visible = bodies.iter().enumerate().filter(|(_, b)| b.visible).map(|(i, b)| (i, &b.mesh));
            self.renderer.set_meshes(rs, visible);
            self.meshes_dirty = false;
        }
        if self.grid_built != Some(self.grid) {
            let (plane, spacing) = self.grid;
            // Cover about the same area whatever the spacing.
            let half_cells = ((250.0 / spacing).round() as i32).clamp(25, 200);
            self.renderer.set_grid(rs, &plane, spacing, half_cells);
            self.grid_built = Some(self.grid);
        }

        let events = self.handle_input(ui, &response, rect, bodies);

        let ppp = ui.ctx().pixels_per_point();
        let size_px = [(rect.width() * ppp).round() as u32, (rect.height() * ppp).round() as u32];
        let size = Vec2::new(rect.width(), rect.height());
        let frame = Frame {
            view_proj: self.camera.view_proj(size),
            eye: self.camera.eye(),
            hovered: self.hovered.as_ref().and_then(Pick::face).map_or(0, Hit::id),
            selected: self
                .selection
                .iter()
                .filter_map(Pick::face)
                .map(Hit::id)
                .chain(extra.faces.iter().map(|&(b, f)| pick::pack_id(b, f)))
                .collect(),
            body_mask: extra.bodies.iter().filter(|&&b| b < 32).fold(0, |m, &b| m | (1 << b)),
            clip: self.section,
            colors: self.body_colors.clone(),
        };
        let texture = self.renderer.render(rs, size_px, &frame);
        ui.painter().image(
            texture,
            rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );

        self.edge_overlay(ui, rect, bodies, &extra.edges);
        if self.view_cube(ui, rect) {
            // Home fits the model too, as in Fusion.
            self.fit_all(bodies);
        }
        events
    }

    /// Hovered and selected edges, drawn thick over the render (1px wgpu
    /// lines are too thin to read as a selection).
    fn edge_overlay(&self, ui: &egui::Ui, rect: Rect, bodies: &[DisplayBody], extra: &[(usize, u32)]) {
        let size = Vec2::new(rect.width(), rect.height());
        let painter = ui.painter_at(rect);
        let draw = |body: usize, edge: u32, stroke: Stroke| {
            let Some(polyline) = bodies.get(body).filter(|b| b.visible).and_then(|b| b.mesh.edges.get(edge as usize))
            else {
                return;
            };
            let points: Vec<Pos2> = polyline
                .iter()
                .filter_map(|&p| self.camera.project(p.into(), size))
                .map(|p| Pos2::new(rect.min.x + p.x, rect.min.y + p.y))
                .collect();
            painter.line(points, stroke);
        };
        let selected = Stroke::new(3.0, Color32::from_rgb(6, 150, 215));
        for p in &self.selection {
            if let Pick::Edge(e) = p {
                draw(e.body, e.edge, selected);
            }
        }
        for &(body, edge) in extra {
            draw(body, edge, selected);
        }
        if let Some(Pick::Edge(e)) = self.hovered {
            draw(e.body, e.edge, Stroke::new(3.0, Color32::from_rgb(120, 190, 245)));
        }
    }

    fn handle_input(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: Rect,
        bodies: &[DisplayBody],
    ) -> ViewportEvents {
        let mut events = ViewportEvents::default();
        let size = Vec2::new(rect.width(), rect.height());
        let (shift, delta) = ui.input(|i| (i.modifiers.shift, i.pointer.delta()));
        let delta = Vec2::new(delta.x, delta.y);

        // Fusion: middle-drag pans, shift+middle orbits. Right-drag also orbits
        // for mice and trackpads without a usable middle button.
        let orbiting = response.dragged_by(PointerButton::Secondary)
            || (response.dragged_by(PointerButton::Middle) && shift);
        if orbiting {
            self.camera.orbit(delta);
        } else if response.dragged_by(PointerButton::Middle) {
            self.camera.pan(delta, rect.height());
        }

        events.toggle = ui.input(|i| i.modifiers.shift || i.modifiers.command);
        events.primary_clicked = response.clicked_by(PointerButton::Primary);
        events.primary_double_clicked = response.double_clicked_by(PointerButton::Primary);
        events.primary_drag_started = response.drag_started_by(PointerButton::Primary);
        events.primary_dragging = response.dragged_by(PointerButton::Primary);
        events.primary_drag_stopped = response.drag_stopped_by(PointerButton::Primary);
        self.pointer = ui
            .ctx()
            .pointer_latest_pos()
            .filter(|p| rect.contains(*p) || response.dragged())
            .map(|p| Vec2::new(p.x - rect.min.x, p.y - rect.min.y));

        if let Some(pos) = response.hover_pos() {
            let local = Vec2::new(pos.x - rect.min.x, pos.y - rect.min.y);
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.camera.zoom_at((-scroll * 0.0025).exp(), local, size);
            }
            if !self.pick_enabled {
                self.hovered = None;
            } else if !response.dragged() {
                let visible = bodies.iter().enumerate().filter(|(_, b)| b.visible).map(|(i, b)| (i, &b.mesh));
                self.hovered = pick::pick_any_clipped(&self.camera, local, size, visible, self.section);
            }
        } else {
            self.hovered = None;
        }

        if response.clicked_by(PointerButton::Primary) && self.pick_enabled {
            let toggle = events.toggle;
            match self.hovered {
                Some(pick) => events.clicked = Some((pick, toggle)),
                None => events.clicked_empty = true,
            }
        }
        if response.double_clicked_by(PointerButton::Middle) {
            self.fit_all(bodies);
        }
        events
    }

    /// Clickable orientation cube in the top-right corner, like Fusion's ViewCube.
    /// Draw and handle the view cube. Returns true when Home was clicked.
    fn view_cube(&mut self, ui: &mut egui::Ui, viewport: Rect) -> bool {
        let size = 92.0;
        let rect = Rect::from_min_size(Pos2::new(viewport.max.x - size - 12.0, viewport.min.y + 12.0), EVec2::splat(size));
        // Dragging the cube orbits, like Fusion's ViewCube; a click picks a face.
        let response = ui.interact(rect, ui.id().with("view_cube"), Sense::click_and_drag());
        if response.dragged() {
            let d = response.drag_delta();
            self.camera.orbit(Vec2::new(d.x, d.y));
        }
        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(if response.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
        }
        let painter = ui.painter_at(rect);
        let center = rect.center();
        let scale = size * 0.27;
        let rot = self.camera.rotation();

        let faces: [(StandardView, &str, Vec3, Vec3, Vec3); 6] = [
            (StandardView::Top, "TOP", Vec3::Z, Vec3::X, Vec3::Y),
            (StandardView::Bottom, "BOTTOM", -Vec3::Z, Vec3::X, -Vec3::Y),
            (StandardView::Front, "FRONT", -Vec3::Y, Vec3::X, Vec3::Z),
            (StandardView::Back, "BACK", Vec3::Y, -Vec3::X, Vec3::Z),
            (StandardView::Right, "RIGHT", Vec3::X, Vec3::Y, Vec3::Z),
            (StandardView::Left, "LEFT", -Vec3::X, -Vec3::Y, Vec3::Z),
        ];

        let to_screen = |p: Vec3| {
            let v = (rot * p.extend(0.0)).xyz();
            (Pos2::new(center.x + v.x * scale, center.y - v.y * scale), v.z)
        };

        let mut visible: Vec<_> = faces
            .iter()
            .filter_map(|&(view, label, n, u, v)| {
                let (_, depth) = to_screen(n);
                // Facing the viewer once rotated into view space (+Z toward camera).
                (depth > 0.05).then(|| {
                    let corners = [n - u - v, n + u - v, n + u + v, n - u + v].map(|p| to_screen(p).0);
                    (depth, view, label, corners)
                })
            })
            .collect();
        visible.sort_by(|a, b| a.0.total_cmp(&b.0));

        let hover = response.hover_pos();
        let mut clicked = None;
        for (depth, view, label, corners) in &visible {
            let inside = hover.is_some_and(|p| point_in_quad(p, corners));
            let shade = (200.0 + 55.0 * depth) as u8;
            let fill = if inside { Color32::from_rgb(150, 200, 245) } else { Color32::from_gray(shade) };
            painter.add(egui::Shape::convex_polygon(corners.to_vec(), fill, Stroke::new(1.0, Color32::from_gray(110))));
            if *depth > 0.3 {
                let mid = corners.iter().fold(EVec2::ZERO, |acc, p| acc + p.to_vec2()) / 4.0;
                painter.text(
                    mid.to_pos2(),
                    egui::Align2::CENTER_CENTER,
                    *label,
                    egui::FontId::proportional(9.0),
                    Color32::from_gray(60),
                );
            }
            if inside && response.clicked() {
                clicked = Some(*view);
            }
        }

        // Home button under the cube.
        let home = Rect::from_center_size(Pos2::new(rect.min.x + 8.0, rect.min.y + 8.0), EVec2::splat(16.0));
        let home_resp = ui.interact(home, ui.id().with("view_home"), Sense::click()).on_hover_text("Home view");
        draw_home_icon(&painter, home, home_resp.hovered());
        if home_resp.clicked() {
            clicked = Some(StandardView::Home);
        }

        if let Some(view) = clicked {
            self.camera.set_view(view);
        }
        clicked == Some(StandardView::Home)
    }
}

fn point_in_quad(p: Pos2, quad: &[Pos2; 4]) -> bool {
    let mut sign = 0.0f32;
    for i in 0..4 {
        let a = quad[i];
        let b = quad[(i + 1) % 4];
        let cross = (b - a).x * (p - a).y - (b - a).y * (p - a).x;
        if cross != 0.0 {
            if sign != 0.0 && cross.signum() != sign {
                return false;
            }
            sign = cross.signum();
        }
    }
    true
}

fn draw_home_icon(painter: &egui::Painter, rect: Rect, hovered: bool) {
    let color = if hovered { Color32::from_rgb(6, 150, 215) } else { Color32::from_gray(90) };
    let c = rect.center();
    let s = rect.width() / 2.0 - 2.0;
    let roof = [Pos2::new(c.x - s, c.y), Pos2::new(c.x, c.y - s), Pos2::new(c.x + s, c.y)];
    painter.line(roof.to_vec(), Stroke::new(1.5, color));
    let body = Rect::from_min_max(Pos2::new(c.x - s * 0.65, c.y), Pos2::new(c.x + s * 0.65, c.y + s));
    painter.rect_stroke(body, 0.0, Stroke::new(1.5, color), egui::StrokeKind::Inside);
}

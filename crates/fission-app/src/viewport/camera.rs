use glam::camera::rh::{proj::directx::perspective, view::look_at_mat4};
use glam::{Mat4, Vec2, Vec3};

/// Z-up orbit camera.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub target: Vec3,
    /// Radians around +Z, measured from +X.
    pub yaw: f32,
    /// Radians above the XY plane.
    pub pitch: f32,
    pub distance: f32,
    pub fov_y: f32,
}

impl Default for Camera {
    fn default() -> Self {
        let mut camera = Camera { target: Vec3::ZERO, yaw: 0.0, pitch: 0.0, distance: 200.0, fov_y: 30f32.to_radians() };
        camera.set_view(StandardView::Home);
        camera
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StandardView {
    Home,
    Top,
    Bottom,
    Front,
    Back,
    Right,
    Left,
}

impl StandardView {
    pub fn parse(name: &str) -> Option<StandardView> {
        Some(match name.to_ascii_lowercase().as_str() {
            "home" | "iso" => StandardView::Home,
            "top" => StandardView::Top,
            "bottom" => StandardView::Bottom,
            "front" => StandardView::Front,
            "back" => StandardView::Back,
            "right" => StandardView::Right,
            "left" => StandardView::Left,
            _ => return None,
        })
    }

    /// (yaw, pitch) in degrees. Front looks along +Y, so the eye sits at -Y.
    fn angles(self) -> (f32, f32) {
        match self {
            StandardView::Home => (-60.0, 30.0),
            StandardView::Top => (-90.0, 89.9),
            StandardView::Bottom => (-90.0, -89.9),
            StandardView::Front => (-90.0, 0.0),
            StandardView::Back => (90.0, 0.0),
            StandardView::Right => (0.0, 0.0),
            StandardView::Left => (180.0, 0.0),
        }
    }
}

pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
}

impl Camera {
    pub fn set_view(&mut self, view: StandardView) {
        let (yaw, pitch) = view.angles();
        self.yaw = yaw.to_radians();
        self.pitch = pitch.to_radians();
    }

    /// Look straight at a plane with this normal (it faces the viewer).
    pub fn look_along(&mut self, normal: Vec3) {
        let n = normal.normalize_or_zero();
        self.pitch = n.z.clamp(-1.0, 1.0).asin();
        if n.x.abs() > 1e-6 || n.y.abs() > 1e-6 {
            self.yaw = n.y.atan2(n.x);
        }
    }

    pub fn forward(&self) -> Vec3 {
        -Vec3::new(self.pitch.cos() * self.yaw.cos(), self.pitch.cos() * self.yaw.sin(), self.pitch.sin())
    }

    pub fn eye(&self) -> Vec3 {
        self.target - self.forward() * self.distance
    }

    fn right(&self) -> Vec3 {
        self.forward().cross(Vec3::Z).try_normalize().unwrap_or(Vec3::X)
    }

    fn up(&self) -> Vec3 {
        self.right().cross(self.forward())
    }

    pub fn view(&self) -> Mat4 {
        look_at_mat4(self.eye(), self.target, self.up())
    }

    pub fn proj(&self, aspect: f32) -> Mat4 {
        // Near/far scale with distance so large and tiny parts both get depth precision.
        perspective(self.fov_y, aspect.max(1e-3), self.distance * 0.01, self.distance * 100.0)
    }

    pub fn view_proj(&self, size: Vec2) -> Mat4 {
        self.proj(size.x / size.y) * self.view()
    }

    /// Ray through a pixel, `pos` in viewport-local pixels.
    pub fn ray(&self, pos: Vec2, size: Vec2) -> Ray {
        let ndc = Vec2::new(pos.x / size.x * 2.0 - 1.0, 1.0 - pos.y / size.y * 2.0);
        let inv = self.view_proj(size).inverse();
        let near = inv.project_point3(ndc.extend(0.0));
        let far = inv.project_point3(ndc.extend(1.0));
        Ray { origin: near, dir: (far - near).normalize() }
    }

    pub fn orbit(&mut self, delta: Vec2) {
        let speed = 0.008;
        self.yaw -= delta.x * speed;
        self.pitch = (self.pitch + delta.y * speed).clamp(-1.5705, 1.5705);
    }

    /// Pan so the point under the cursor follows it.
    pub fn pan(&mut self, delta: Vec2, viewport_height: f32) {
        let world_per_px = 2.0 * self.distance * (self.fov_y / 2.0).tan() / viewport_height.max(1.0);
        self.target += (-self.right() * delta.x + self.up() * delta.y) * world_per_px;
    }

    /// Zoom by `factor` (< 1 is in), keeping the point under the cursor fixed.
    pub fn zoom_at(&mut self, factor: f32, pos: Vec2, size: Vec2) {
        let ray = self.ray(pos, size);
        // Point on the plane through the target facing the camera.
        let t = (self.target - ray.origin).dot(self.forward()) / ray.dir.dot(self.forward());
        let anchor = ray.origin + ray.dir * t;
        let new_distance = (self.distance * factor).clamp(0.05, 1e6);
        let applied = new_distance / self.distance;
        self.target = anchor + (self.target - anchor) * applied;
        self.distance = new_distance;
    }

    /// Frame a bounding box, keeping the current direction.
    pub fn fit(&mut self, min: Vec3, max: Vec3) {
        let radius = ((max - min).length() / 2.0).max(1.0);
        self.target = (min + max) / 2.0;
        self.distance = radius / (self.fov_y / 2.0).sin() * 1.1;
    }

    /// Frame `points` tightly in a view of this aspect (width / height),
    /// keeping the current direction, with `margin` (0.05 = 5%) to spare.
    pub fn fit_points(&mut self, points: &[Vec3], aspect: f32, margin: f32) {
        if points.is_empty() {
            return;
        }
        let (min, max) = points.iter().fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |(lo, hi), &p| (lo.min(p), hi.max(p)));
        self.target = (min + max) / 2.0;
        let (right, up, back) = (self.right(), self.up(), -self.forward());
        let tan_y = (self.fov_y / 2.0).tan() * (1.0 - margin);
        let tan_x = tan_y * aspect;
        // Distance from the target at which every point is inside the frustum.
        let need = |target: Vec3, p: &Vec3| {
            let d = *p - target;
            let depth = d.dot(back);
            (depth + d.dot(right).abs() / tan_x).max(depth + d.dot(up).abs() / tan_y)
        };
        let fit = |target: Vec3| points.iter().map(|p| need(target, p)).fold(0.0f32, f32::max).max(1.0);
        let mut distance = fit(self.target);
        // The view is centred on the box, not on the points' screen extent:
        // recentre on their projected middle, then refit once.
        for _ in 0..2 {
            let (mut lo, mut hi) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
            for p in points {
                let d = *p - self.target;
                let z = distance - d.dot(back);
                let q = Vec2::new(d.dot(right), d.dot(up)) / z.max(1e-3);
                lo = lo.min(q);
                hi = hi.max(q);
            }
            let mid = (lo + hi) / 2.0;
            self.target += (right * mid.x + up * mid.y) * distance;
            distance = fit(self.target);
        }
        self.distance = distance;
    }

    /// World point to viewport-local pixels; None if behind the camera.
    pub fn project(&self, p: Vec3, size: Vec2) -> Option<Vec2> {
        let clip = self.view_proj(size) * p.extend(1.0);
        if clip.w <= 0.0 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(Vec2::new((ndc.x + 1.0) / 2.0 * size.x, (1.0 - ndc.y) / 2.0 * size.y))
    }

    /// Rotation-only view matrix, for drawing the view cube.
    pub fn rotation(&self) -> Mat4 {
        look_at_mat4(-self.forward(), Vec3::ZERO, self.up())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn center_ray_hits_target() {
        let camera = Camera::default();
        let size = Vec2::new(800.0, 600.0);
        let ray = camera.ray(size / 2.0, size);
        let closest = ray.origin + ray.dir * (camera.target - ray.origin).dot(ray.dir);
        assert!(closest.distance(camera.target) < 1e-2);
    }

    #[test]
    fn project_inverts_ray() {
        let camera = Camera::default();
        let size = Vec2::new(800.0, 600.0);
        let px = Vec2::new(123.0, 456.0);
        let ray = camera.ray(px, size);
        let back = camera.project(ray.origin + ray.dir * 150.0, size).unwrap();
        assert!(back.distance(px) < 0.1, "{back}");
    }

    #[test]
    fn zoom_keeps_cursor_point_fixed() {
        let mut camera = Camera::default();
        let size = Vec2::new(800.0, 600.0);
        let px = Vec2::new(600.0, 200.0);
        let before = camera.ray(px, size);
        let t = (camera.target - before.origin).dot(camera.forward()) / before.dir.dot(camera.forward());
        let anchor = before.origin + before.dir * t;
        camera.zoom_at(0.5, px, size);
        let after = camera.project(anchor, size).unwrap();
        assert!(after.distance(px) < 0.5, "{after}");
    }

    #[test]
    fn front_view_looks_along_plus_y() {
        let mut camera = Camera::default();
        camera.set_view(StandardView::Front);
        assert!(camera.forward().distance(Vec3::Y) < 1e-4);
    }
}

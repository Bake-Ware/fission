use fission_kernel::Mesh;
use glam::{Vec2, Vec3, Vec4};

use super::camera::{Camera, Ray};

/// Something under the cursor or in the selection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pick {
    Face(Hit),
    Edge(EdgeHit),
}

impl Pick {
    pub fn body(&self) -> usize {
        match self {
            Pick::Face(h) => h.body,
            Pick::Edge(e) => e.body,
        }
    }

    /// Same B-rep entity, ignoring where on it the cursor was.
    pub fn same_entity(&self, other: &Pick) -> bool {
        match (self, other) {
            (Pick::Face(a), Pick::Face(b)) => a.body == b.body && a.face == b.face,
            (Pick::Edge(a), Pick::Edge(b)) => a.body == b.body && a.edge == b.edge,
            _ => false,
        }
    }

    pub fn face(&self) -> Option<&Hit> {
        match self {
            Pick::Face(h) => Some(h),
            Pick::Edge(_) => None,
        }
    }
}

/// A B-rep edge under the cursor: index into `Mesh::edges` of `body`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeHit {
    pub body: usize,
    pub edge: u32,
    pub point: Vec3,
}

/// Screen-space edge pick radius, in points.
const EDGE_PICK_PX: f32 = 7.0;

/// Edges win over faces within a few pixels, as in Fusion, but only if
/// they're not hidden behind the face under the cursor.
#[cfg(test)]
pub fn pick_any<'a>(
    camera: &Camera,
    cursor: Vec2,
    size: Vec2,
    meshes: impl IntoIterator<Item = (usize, &'a Mesh)> + Clone,
) -> Option<Pick> {
    pick_any_clipped(camera, cursor, size, meshes, None)
}

/// Is `p` cut away by a section plane (normal·p > d)?
fn clipped(clip: Option<Vec4>, p: Vec3) -> bool {
    clip.is_some_and(|c| c.truncate().dot(p) > c.w)
}

/// Like [`pick_any`], ignoring what a section plane `clip` (normal, d) cuts away.
pub fn pick_any_clipped<'a>(
    camera: &Camera,
    cursor: Vec2,
    size: Vec2,
    meshes: impl IntoIterator<Item = (usize, &'a Mesh)> + Clone,
    clip: Option<Vec4>,
) -> Option<Pick> {
    let ray = camera.ray(cursor, size);
    let face = pick_clipped(&ray, meshes.clone(), clip);
    let occlusion_slack = camera.distance * 0.01;

    let mut best: Option<(f32, EdgeHit)> = None;
    for (body, mesh) in meshes {
        for (edge, polyline) in mesh.edges.iter().enumerate() {
            for seg in polyline.windows(2) {
                let (a, b) = (Vec3::from(seg[0]), Vec3::from(seg[1]));
                let (Some(sa), Some(sb)) = (camera.project(a, size), camera.project(b, size)) else { continue };
                let (px, t) = point_segment(cursor, sa, sb);
                if px > EDGE_PICK_PX || best.is_some_and(|(d, _)| px >= d) {
                    continue;
                }
                let point = a.lerp(b, t);
                if clipped(clip, point) {
                    continue;
                }
                let along_ray = (point - ray.origin).dot(ray.dir);
                if face.is_some_and(|f| f.distance < along_ray - occlusion_slack) {
                    continue;
                }
                best = Some((px, EdgeHit { body, edge: edge as u32, point }));
            }
        }
    }
    match (best, face) {
        (Some((_, edge)), _) => Some(Pick::Edge(edge)),
        (None, Some(face)) => Some(Pick::Face(face)),
        (None, None) => None,
    }
}

/// Distance from `p` to segment ab, and the segment parameter of the closest point.
fn point_segment(p: Vec2, a: Vec2, b: Vec2) -> (f32, f32) {
    let ab = b - a;
    let len2 = ab.length_squared();
    let t = if len2 == 0.0 { 0.0 } else { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) };
    (p.distance(a + ab * t), t)
}

/// A B-rep face under the cursor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub body: usize,
    pub face: u32,
    pub point: Vec3,
    pub normal: Vec3,
    pub distance: f32,
}

impl Hit {
    /// Packed id the shader compares against: body in the high 16 bits.
    pub fn id(&self) -> u32 {
        pack_id(self.body, self.face)
    }
}

pub fn pack_id(body: usize, face: u32) -> u32 {
    ((body as u32 + 1) << 16) | (face & 0xffff)
}

/// Exact ray cast against every triangle. Meshes here are small (tens of
/// thousands of triangles), so brute force is fast enough and never misses.
#[cfg(test)]
pub fn pick<'a>(ray: &Ray, meshes: impl IntoIterator<Item = (usize, &'a Mesh)>) -> Option<Hit> {
    pick_clipped(ray, meshes, None)
}

fn pick_clipped<'a>(ray: &Ray, meshes: impl IntoIterator<Item = (usize, &'a Mesh)>, clip: Option<Vec4>) -> Option<Hit> {
    let mut best: Option<Hit> = None;
    for (body, mesh) in meshes {
        for (t, tri) in mesh.indices.chunks_exact(3).enumerate() {
            let [a, b, c] = [0, 1, 2].map(|i| Vec3::from(mesh.positions[tri[i] as usize]));
            if let Some(distance) = intersect(ray, a, b, c) {
                if clipped(clip, ray.origin + ray.dir * distance) {
                    continue;
                }
                if best.is_none_or(|h| distance < h.distance) {
                    let mut normal = (b - a).cross(c - a).normalize_or_zero();
                    if normal.dot(ray.dir) > 0.0 {
                        normal = -normal;
                    }
                    best = Some(Hit {
                        body,
                        face: mesh.face_ids[t],
                        point: ray.origin + ray.dir * distance,
                        normal,
                        distance,
                    });
                }
            }
        }
    }
    best
}

/// Möller–Trumbore, two-sided.
fn intersect(ray: &Ray, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let e1 = b - a;
    let e2 = c - a;
    let p = ray.dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = ray.origin - a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = ray.dir.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 0.0).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(z: f32, face: u32) -> Mesh {
        Mesh {
            positions: vec![[-1.0, -1.0, z], [1.0, -1.0, z], [1.0, 1.0, z], [-1.0, 1.0, z]],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            indices: vec![0, 1, 2, 0, 2, 3],
            face_ids: vec![face, face],
            edges: vec![],
            ..Default::default()
        }
    }

    #[test]
    fn picks_nearest_face() {
        let ray = Ray { origin: Vec3::new(0.2, 0.3, 10.0), dir: -Vec3::Z };
        let low = quad(0.0, 1);
        let high = quad(2.0, 7);
        let hit = pick(&ray, [(0, &low), (3, &high)]).unwrap();
        assert_eq!((hit.body, hit.face), (3, 7));
        assert!((hit.distance - 8.0).abs() < 1e-4);
        assert!(hit.normal.distance(Vec3::Z) < 1e-4);
    }

    #[test]
    fn misses_outside() {
        let ray = Ray { origin: Vec3::new(5.0, 0.0, 10.0), dir: -Vec3::Z };
        assert!(pick(&ray, [(0, &quad(0.0, 0))]).is_none());
    }

    #[test]
    fn edge_wins_near_its_line_and_hidden_edges_dont() {
        // Looking straight down (pitch a hair under 90 degrees).
        let camera = Camera { target: Vec3::ZERO, yaw: -std::f32::consts::FRAC_PI_2, pitch: std::f32::consts::FRAC_PI_2 - 1e-4, distance: 10.0, fov_y: 0.5 };
        let size = Vec2::new(400.0, 400.0);
        let mut top = quad(1.0, 3);
        top.edges = vec![vec![[-1.0, 0.0, 1.0], [1.0, 0.0, 1.0]]];
        let mut buried = quad(-1.0, 4);
        buried.edges = vec![vec![[-1.0, 0.3, -1.0], [1.0, 0.3, -1.0]]];

        let edge_px = camera.project(Vec3::new(0.0, 0.0, 1.0), size).unwrap();
        let near = pick_any(&camera, edge_px + Vec2::new(0.0, 3.0), size, [(0, &top), (1, &buried)]);
        assert!(matches!(near, Some(Pick::Edge(EdgeHit { body: 0, edge: 0, .. }))), "{near:?}");

        // Over the buried edge, the top face is in the way.
        let buried_px = camera.project(Vec3::new(0.0, 0.3, -1.0), size).unwrap();
        let hidden = pick_any(&camera, buried_px, size, [(0, &top), (1, &buried)]);
        assert!(matches!(hidden, Some(Pick::Face(Hit { body: 0, .. }))), "{hidden:?}");
    }

    #[test]
    fn ids_are_unique_per_body_and_never_zero() {
        assert_ne!(pack_id(0, 0), 0);
        assert_ne!(pack_id(0, 1), pack_id(1, 0));
    }
}

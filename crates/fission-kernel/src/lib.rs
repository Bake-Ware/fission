//! Kernel-agnostic geometry interface.
//!
//! The app and the feature timeline only talk to [`Kernel`]. The first backend
//! is OpenCascade (`fission-occt`); a pure-Rust backend can slot in later
//! without touching the app.

use std::io::{self, Write};

use serde::{Deserialize, Serialize};
use std::path::Path;

pub type Vec3 = [f64; 3];

/// A plane in 3D space that sketches live on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plane {
    pub origin: Vec3,
    pub normal: Vec3,
    /// Direction of the sketch's local +X axis.
    pub x_dir: Vec3,
}

impl Plane {
    pub const XY: Plane = Plane { origin: [0.0; 3], normal: [0.0, 0.0, 1.0], x_dir: [1.0, 0.0, 0.0] };
    pub const XZ: Plane = Plane { origin: [0.0; 3], normal: [0.0, -1.0, 0.0], x_dir: [1.0, 0.0, 0.0] };
    pub const YZ: Plane = Plane { origin: [0.0; 3], normal: [1.0, 0.0, 0.0], x_dir: [0.0, 1.0, 0.0] };

    pub fn y_dir(&self) -> Vec3 {
        cross(self.normal, self.x_dir)
    }

    /// Map a 2D sketch point onto this plane.
    pub fn to_world(&self, p: [f64; 2]) -> Vec3 {
        let y = self.y_dir();
        std::array::from_fn(|i| self.origin[i] + self.x_dir[i] * p[0] + y[i] * p[1])
    }
}

/// A piece of a 3D path (for sweeps): end to end, in order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg3 {
    Line(Vec3, Vec3),
    /// Through `mid`.
    Arc { start: Vec3, mid: Vec3, end: Vec3 },
}

/// A closed 2D region on a plane, the input to extrude/revolve.
#[derive(Clone, Debug, PartialEq)]
pub enum Profile {
    Polygon { plane: Plane, points: Vec<[f64; 2]> },
    Circle { plane: Plane, center: [f64; 2], radius: f64 },
    /// A sketch region: ordered outer loop (counter-clockwise) and hole loops.
    Region { plane: Plane, outer: Vec<Seg2>, holes: Vec<Vec<Seg2>> },
}

/// One piece of a profile loop, in plane coordinates.
#[derive(Clone, Debug, PartialEq)]
pub enum Seg2 {
    Line([f64; 2], [f64; 2]),
    /// Three-point arc from `start` through `mid` to `end`.
    Arc { start: [f64; 2], mid: [f64; 2], end: [f64; 2] },
    Circle { center: [f64; 2], radius: f64 },
}

impl Seg2 {
    pub fn reversed(&self) -> Seg2 {
        match *self {
            Seg2::Line(a, b) => Seg2::Line(b, a),
            Seg2::Arc { start, mid, end } => Seg2::Arc { start: end, mid, end: start },
            Seg2::Circle { center, radius } => Seg2::Circle { center, radius },
        }
    }
}

impl Profile {
    pub fn plane(&self) -> &Plane {
        match self {
            Profile::Polygon { plane, .. } | Profile::Circle { plane, .. } | Profile::Region { plane, .. } => plane,
        }
    }

    pub fn plane_mut(&mut self) -> &mut Plane {
        match self {
            Profile::Polygon { plane, .. } | Profile::Circle { plane, .. } | Profile::Region { plane, .. } => plane,
        }
    }

    pub fn rect(plane: Plane, min: [f64; 2], max: [f64; 2]) -> Self {
        let points = vec![min, [max[0], min[1]], max, [min[0], max[1]]];
        Profile::Polygon { plane, points }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BooleanOp {
    Union,
    Subtract,
    Intersect,
}

/// Which edges a fillet/chamfer applies to.
#[derive(Clone, Debug, PartialEq)]
pub enum EdgeSelection {
    All,
    /// Edges identified geometrically (see [`EdgeHint`]). A backend errors
    /// if a hint no longer matches any edge.
    Near(Vec<EdgeHint>),
}

/// A geometric edge reference. Edge indices change on every regeneration;
/// midpoint and direction mostly don't, or change predictably when an
/// upstream dimension changes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgeHint {
    pub mid: Vec3,
    /// Unit tangent at the midpoint (sign doesn't matter).
    pub dir: Vec3,
}

impl EdgeHint {
    pub fn from_polyline(points: &[[f32; 3]]) -> Option<EdgeHint> {
        let mid = polyline_midpoint(points)?;
        // Tangent from the segment containing the midpoint (nearest segment).
        let seg = points
            .windows(2)
            .min_by(|a, b| seg_dist(mid, a).total_cmp(&seg_dist(mid, b)))?;
        let d = std::array::from_fn(|k| (seg[1][k] - seg[0][k]) as f64);
        let len = dist(d, [0.0; 3]);
        let dir = if len == 0.0 { [0.0; 3] } else { d.map(|x| x / len) };
        Some(EdgeHint { mid, dir })
    }
}

fn seg_dist(p: Vec3, seg: &[[f32; 3]]) -> f64 {
    let a = p_of(seg[0]);
    let b = p_of(seg[1]);
    let ab: Vec3 = std::array::from_fn(|k| b[k] - a[k]);
    let len2 = ab.iter().map(|x| x * x).sum::<f64>();
    let t = if len2 == 0.0 { 0.0 } else { ((0..3).map(|k| (p[k] - a[k]) * ab[k]).sum::<f64>() / len2).clamp(0.0, 1.0) };
    dist(p, std::array::from_fn(|k| a[k] + ab[k] * t))
}

/// How close (mm) an edge's midpoint must be to count as the same edge.
pub const EDGE_MATCH_TOLERANCE: f64 = 1e-3;

/// Resolve hints against candidate edges, returning one distinct candidate
/// index per hint. Exact midpoint match first; otherwise the nearest parallel
/// edge within `max_drift` (so references survive upstream resizes). Errors
/// with the number of hints that matched nothing.
pub fn resolve_edges(hints: &[EdgeHint], candidates: &[EdgeHint], max_drift: f64) -> std::result::Result<Vec<usize>, usize> {
    let pairs = |h: &[EdgeHint]| h.iter().map(|e| (e.mid, e.dir)).collect::<Vec<_>>();
    resolve(&pairs(hints), &pairs(candidates), max_drift, false)
}

/// A geometric face reference: center of mass and outward normal there.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FaceHint {
    pub center: Vec3,
    pub normal: Vec3,
}

/// Like [`resolve_edges`] for faces; normals must point the same way.
pub fn resolve_faces(hints: &[FaceHint], candidates: &[FaceHint], max_drift: f64) -> std::result::Result<Vec<usize>, usize> {
    let pairs = |h: &[FaceHint]| h.iter().map(|f| (f.center, f.normal)).collect::<Vec<_>>();
    resolve(&pairs(hints), &pairs(candidates), max_drift, true)
}

/// Shared matcher: (position, direction) pairs; `signed` = direction sign matters.
fn resolve(
    hints: &[(Vec3, Vec3)],
    candidates: &[(Vec3, Vec3)],
    max_drift: f64,
    signed: bool,
) -> std::result::Result<Vec<usize>, usize> {
    let mut used = vec![false; candidates.len()];
    let mut out = Vec::with_capacity(hints.len());
    let mut lost = 0;
    for hint in hints {
        let free = |i: &usize| !used[*i];
        let (hint_pos, hint_dir) = *hint;
        let exact = (0..candidates.len())
            .filter(free)
            .find(|&i| dist(candidates[i].0, hint_pos) < EDGE_MATCH_TOLERANCE);
        let parallel = || {
            (0..candidates.len())
                .filter(free)
                .filter(|&i| {
                    let d = candidates[i].1;
                    let dot = d[0] * hint_dir[0] + d[1] * hint_dir[1] + d[2] * hint_dir[2];
                    if signed { dot > 0.999 } else { dot.abs() > 0.999 }
                })
                .map(|i| (i, dist(candidates[i].0, hint_pos)))
                .filter(|&(_, d)| d < max_drift)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i)
        };
        match exact.or_else(parallel) {
            Some(i) => {
                used[i] = true;
                out.push(i);
            }
            None => lost += 1,
        }
    }
    if lost == 0 { Ok(out) } else { Err(lost) }
}

/// Arc-length midpoint of a polyline: the shared edge signature, so the
/// viewport (which sees meshes) and backends (which see B-rep) agree.
pub fn polyline_midpoint(points: &[[f32; 3]]) -> Option<Vec3> {
    let p = |i: usize| points[i].map(|x| x as f64);
    let lengths: Vec<f64> = points.windows(2).map(|w| dist(p_of(w[0]), p_of(w[1]))).collect();
    let total: f64 = lengths.iter().sum();
    if points.is_empty() {
        return None;
    }
    if total == 0.0 {
        return Some(p(0));
    }
    let mut remaining = total / 2.0;
    for (i, &len) in lengths.iter().enumerate() {
        if remaining <= len {
            let t = if len == 0.0 { 0.0 } else { remaining / len };
            let (a, b) = (p(i), p(i + 1));
            return Some(std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t));
        }
        remaining -= len;
    }
    Some(p(points.len() - 1))
}

fn p_of(v: [f32; 3]) -> Vec3 {
    v.map(|x| x as f64)
}

pub fn dist(a: Vec3, b: Vec3) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// Triangle mesh for display and mesh export. `face_ids[t]` is the B-rep face
/// triangle `t` came from, so the viewport can pick and highlight whole faces.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    pub face_ids: Vec<u32>,
    /// B-rep edges as polylines (each edge once), for drawing and picking.
    pub edges: Vec<Vec<[f32; 3]>>,
    /// Per face id: the face's center of mass and normal there (for stable
    /// face references; same values the backend matches against).
    pub face_hints: Vec<FaceHint>,
    /// Faces that couldn't be triangulated (shown as holes in the display).
    #[serde(default)]
    pub skipped_faces: u32,
    /// Per face id: its exact area in mm² (empty if the backend doesn't say).
    #[serde(default)]
    pub face_areas: Vec<f64>,
    /// The body's exact volume in mm³ (0 for sheets or if unknown).
    #[serde(default)]
    pub volume: f64,
}

impl Mesh {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Binary STL. Lives here rather than in a backend so every kernel gets it.
    pub fn write_stl(&self, mut out: impl Write) -> io::Result<()> {
        out.write_all(&[0u8; 80])?;
        out.write_all(&(self.triangle_count() as u32).to_le_bytes())?;
        for tri in self.indices.chunks_exact(3) {
            let [a, b, c] = [0, 1, 2].map(|i| self.positions[tri[i] as usize]);
            let n = normalize(cross(sub(b, a), sub(c, a)));
            for v in [n, a, b, c] {
                for x in v {
                    out.write_all(&x.to_le_bytes())?;
                }
            }
            out.write_all(&[0, 0])?;
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum KernelError {
    #[error("{op} failed: {reason}")]
    Operation { op: &'static str, reason: String },
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub type Result<T> = std::result::Result<T, KernelError>;

/// The operations the feature timeline needs from a geometry kernel.
pub trait Kernel {
    /// A kernel-owned solid body. Cheap to clone (backends share or copy as needed).
    type Solid: Clone;

    fn extrude(&self, profile: &Profile, distance: f64) -> Result<Self::Solid>;

    /// A solid through profiles in order (Fusion's Loft). Uses each
    /// profile's outer boundary.
    fn loft(&self, sections: &[Profile]) -> Result<Self::Solid> {
        let _ = sections;
        Err(KernelError::Operation { op: "loft", reason: "not supported by this kernel".into() })
    }

    /// Sweep a profile along a connected path (Fusion's Sweep).
    fn sweep(&self, profile: &Profile, path: &[Seg3]) -> Result<Self::Solid> {
        let _ = (profile, path);
        Err(KernelError::Operation { op: "sweep", reason: "not supported by this kernel".into() })
    }
    fn revolve(&self, profile: &Profile, axis_origin: Vec3, axis_dir: Vec3, degrees: f64) -> Result<Self::Solid>;
    fn boolean(&self, a: &Self::Solid, b: &Self::Solid, op: BooleanOp) -> Result<Self::Solid>;
    fn fillet(&self, solid: &Self::Solid, edges: EdgeSelection, radius: f64) -> Result<Self::Solid>;
    fn chamfer(&self, solid: &Self::Solid, edges: EdgeSelection, distance: f64) -> Result<Self::Solid>;

    /// Hollow a solid to `thickness`, opening the given faces (none = closed void).
    fn shell(&self, solid: &Self::Solid, thickness: f64, open: &[FaceHint]) -> Result<Self::Solid>;
    /// Tilt `faces` by `degrees` (Fusion's Draft): they stay put where they
    /// cross the neutral plane through `origin` square to `pull`, and a
    /// positive angle narrows the solid going along `pull`.
    fn draft(&self, solid: &Self::Solid, faces: &[FaceHint], origin: Vec3, pull: Vec3, degrees: f64) -> Result<Self::Solid>;
    /// Move `faces` `distance` along their outward normals (Fusion's Press
    /// Pull / Offset Face): negative pushes them into the material.
    fn offset_faces(&self, solid: &Self::Solid, faces: &[FaceHint], distance: f64) -> Result<Self::Solid>;
    /// Remove `faces` and heal the gap from the faces around them (Fusion's
    /// Delete on faces): holes fill in, fillets go sharp.
    fn delete_faces(&self, solid: &Self::Solid, faces: &[FaceHint]) -> Result<Self::Solid>;
    /// Cut an ISO metric thread of `pitch` into a cylindrical face: outside
    /// for a shaft, inside for a hole, along the face's length.
    fn thread(&self, solid: &Self::Solid, face: &FaceHint, pitch: f64, left_hand: bool) -> Result<Self::Solid>;
    /// A spring: a round section of diameter `section` swept `turns` times
    /// round a helix of `radius` about the axis through `origin` along
    /// `axis`, starting toward `x_dir` and rising `pitch` per turn.
    #[allow(clippy::too_many_arguments)]
    fn coil(&self, origin: Vec3, axis: Vec3, x_dir: Vec3, radius: f64, pitch: f64, turns: f64, left_hand: bool, section: f64) -> Result<Self::Solid>;
    fn translate(&self, solid: &Self::Solid, by: Vec3) -> Self::Solid;
    /// Grow or shrink evenly about `center`.
    fn scale(&self, solid: &Self::Solid, center: Vec3, factor: f64) -> Self::Solid;
    fn rotate(&self, solid: &Self::Solid, origin: Vec3, axis: Vec3, degrees: f64) -> Self::Solid;
    /// Group solids into one body without a boolean (fast, and avoids
    /// boolean crashes for copies that are never fused).
    fn compound(&self, solids: &[Self::Solid]) -> Self::Solid;
    /// Reflect across the plane through `origin` with `normal`.
    fn mirror(&self, solid: &Self::Solid, origin: Vec3, normal: Vec3) -> Self::Solid;

    fn tessellate(&self, solid: &Self::Solid, tolerance: f64) -> Result<Mesh>;
    fn face_count(&self, solid: &Self::Solid) -> usize;
    /// How many separate solids a body holds.
    fn solid_count(&self, _solid: &Self::Solid) -> usize {
        1
    }
    /// Enclosed volume, in mm³.
    fn volume(&self, solid: &Self::Solid) -> f64;

    fn export_step(&self, solids: &[Self::Solid], path: &Path) -> Result<()>;
    fn import_step(&self, path: &Path) -> Result<Self::Solid>;

    /// The separate bodies in a file (an assembly's parts, a Fusion
    /// design's bodies), to show or hide one by one. Defaults to the
    /// whole file as one part.
    fn import_parts(&self, path: &Path) -> Result<Vec<Self::Solid>> {
        Ok(vec![self.import_step(path)?])
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len == 0.0 { v } else { v.map(|x| x / len) }
}

fn cross<T: Copy + std::ops::Mul<Output = T> + std::ops::Sub<Output = T>>(a: [T; 3], b: [T; 3]) -> [T; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midpoint_is_by_arc_length() {
        let line = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 3.0, 0.0]];
        assert_eq!(polyline_midpoint(&line), Some([1.0, 1.0, 0.0]));
        assert_eq!(polyline_midpoint(&[]), None);
    }

    #[test]
    fn edge_refs_survive_a_resize_but_not_removal() {
        let x = |y: f64, z: f64| EdgeHint { mid: [0.0, y, z], dir: [1.0, 0.0, 0.0] };
        let z_edge = EdgeHint { mid: [5.0, 5.0, 2.0], dir: [0.0, 0.0, 1.0] };
        // Stored against a 10 mm tall box; now it's 12 mm tall.
        let stored = [x(0.0, 10.0), z_edge];
        let now = [x(0.0, 0.0), x(0.0, 12.0), EdgeHint { mid: [5.0, 5.0, 2.4], dir: [0.0, 0.0, -1.0] }];
        assert_eq!(resolve_edges(&stored, &now, 5.0), Ok(vec![1, 2]));
        // With only the bottom edge left, both references are lost.
        assert_eq!(resolve_edges(&stored, &now[..1], 5.0), Err(2));
    }

    #[test]
    fn plane_maps_sketch_points() {
        assert_eq!(Plane::XY.to_world([2.0, 3.0]), [2.0, 3.0, 0.0]);
        assert_eq!(Plane::XZ.to_world([2.0, 3.0]), [2.0, 0.0, 3.0]);
    }

    #[test]
    fn stl_has_header_count_and_records() {
        let mesh = Mesh {
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            indices: vec![0, 1, 2],
            face_ids: vec![0],
            edges: vec![],
            face_hints: vec![],
            skipped_faces: 0,
            ..Default::default()
        };
        let mut buf = Vec::new();
        mesh.write_stl(&mut buf).unwrap();
        assert_eq!(buf.len(), 80 + 4 + 50);
        assert_eq!(u32::from_le_bytes(buf[80..84].try_into().unwrap()), 1);
        // Facet normal is +Z.
        assert_eq!(f32::from_le_bytes(buf[92..96].try_into().unwrap()), 1.0);
    }
}

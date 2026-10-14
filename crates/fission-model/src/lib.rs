//! Parametric document: an ordered feature timeline that regenerates into bodies.
//!
//! Generic over [`Kernel`], so the timeline logic is testable without OCCT.

use fission_kernel::{BooleanOp, EdgeHint, EdgeSelection, FaceHint, Kernel, Plane, Profile, Seg2};
pub use fission_sketch as sketch2d;
use serde::{Deserialize, Serialize};

mod expr;
mod params;
mod history;
pub mod thread;
pub mod f3d;
pub mod motion;
pub use expr::{evaluate, evaluate_with, names_in, Quantity};
pub use params::{dim_key, Parameter};
pub use history::History;

/// Planes a sketch can sit on: the origin planes (Z is up), or any other
/// plane, such as a slanted face the sketch was started on.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum OriginPlane {
    XY,
    XZ,
    YZ,
    Custom { origin: [f64; 3], normal: [f64; 3], x_dir: [f64; 3] },
}

impl OriginPlane {
    pub const ALL: [OriginPlane; 3] = [OriginPlane::XY, OriginPlane::XZ, OriginPlane::YZ];

    pub fn label(self) -> &'static str {
        match self {
            OriginPlane::XY => "XY",
            OriginPlane::XZ => "XZ",
            OriginPlane::YZ => "YZ",
            OriginPlane::Custom { .. } => "Face",
        }
    }

    /// This plane turned `degrees` about its X direction through its
    /// origin (right-handed: positive tips the normal toward -Y).
    pub fn tilted(self, degrees: f64) -> OriginPlane {
        if degrees == 0.0 {
            return self;
        }
        let p = self.plane(0.0);
        let (s, c) = degrees.to_radians().sin_cos();
        let (n, x) = (p.normal, p.x_dir);
        let x_cross_n = [x[1] * n[2] - x[2] * n[1], x[2] * n[0] - x[0] * n[2], x[0] * n[1] - x[1] * n[0]];
        let normal = std::array::from_fn(|k| n[k] * c + x_cross_n[k] * s);
        OriginPlane::Custom { origin: p.origin, normal, x_dir: x }
    }

    /// The plane, shifted `offset` along its normal.
    pub fn plane(self, offset: f64) -> Plane {
        let mut plane = match self {
            OriginPlane::XY => Plane::XY,
            OriginPlane::XZ => Plane::XZ,
            OriginPlane::YZ => Plane::YZ,
            OriginPlane::Custom { origin, normal, x_dir } => {
                return Plane { origin: std::array::from_fn(|k| origin[k] + normal[k] * offset), normal, x_dir };
            }
        };
        plane.origin = plane.normal.map(|n| n * offset);
        plane
    }
}

/// A parametric closed sketch profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Sketch {
    Rect { plane: OriginPlane, offset: f64, center: [f64; 2], size: [f64; 2] },
    Circle { plane: OriginPlane, offset: f64, center: [f64; 2], radius: f64 },
    /// Regions of a Sketch feature, each named by a point inside it (region
    /// order changes as dimensions change; "the region around this point"
    /// mostly doesn't).
    Profiles { sketch: usize, points: Vec<[f64; 2]> },
}

impl Sketch {
    /// The kernel profiles this resolves to, given the sketches built so far.
    pub fn profiles(&self, sketches: &[Option<SketchOutput>]) -> Result<Vec<Profile>, String> {
        match self {
            Sketch::Profiles { sketch, points } => {
                let out = sketches
                    .get(*sketch)
                    .and_then(Option::as_ref)
                    .ok_or("the referenced sketch is missing or comes later in the timeline")?;
                if points.is_empty() {
                    return Err("no profile selected".into());
                }
                points
                    .iter()
                    .map(|&p| {
                        out.region_at(p)
                            .map(|r| region_profile(out.plane, r))
                            .ok_or_else(|| "a selected profile no longer exists; reselect it".to_string())
                    })
                    .collect()
            }
            simple => Ok(vec![simple.simple_profile()]),
        }
    }

    fn simple_profile(&self) -> Profile {
        match *self {
            Sketch::Rect { plane, offset, center, size } => {
                let half = [size[0] / 2.0, size[1] / 2.0];
                Profile::rect(
                    plane.plane(offset),
                    [center[0] - half[0], center[1] - half[1]],
                    [center[0] + half[0], center[1] + half[1]],
                )
            }
            Sketch::Circle { plane, offset, center, radius } => {
                Profile::Circle { plane: plane.plane(offset), center, radius }
            }
            Sketch::Profiles { .. } => unreachable!("handled in profiles()"),
        }
    }
}

/// Solve-check a Sketch feature: None if it isn't one.
fn sketch_output(kind: &FeatureKind) -> Option<Result<SketchOutput, String>> {
    let FeatureKind::Sketch { plane, offset, sketch } = kind else { return None };
    // Re-check the stored geometry; it was solved when edited.
    let mut check = sketch.clone();
    Some(
        check
            .solve()
            .map(|report| {
                let plane = plane.plane(*offset);
                let lines = sketch
                    .curves
                    .iter()
                    .map(|c| match *c {
                        sketch2d::Curve::Line { a, b, .. } => {
                            Some([plane.to_world(sketch.points[a]), plane.to_world(sketch.points[b])])
                        }
                        _ => None,
                    })
                    .collect();
                SketchOutput { plane, regions: sketch.regions(), lines, dof: report.dof, sketch: sketch.clone() }
            })
            .map_err(|e| e.to_string()),
    )
}

/// A solved sketch as regeneration saw it: its plane and closed regions.
#[derive(Clone, Debug)]
pub struct SketchOutput {
    pub plane: Plane,
    pub regions: Vec<sketch2d::Region>,
    /// Each curve's end points in model space when it's a line (for axes).
    pub lines: Vec<Option<[[f64; 3]; 2]>>,
    /// Remaining degrees of freedom (0 = fully constrained).
    pub dof: usize,
    /// The solved sketch itself (for paths).
    pub sketch: sketch2d::Sketch,
}

impl SketchOutput {
    /// The connected chain through `curve`, as a 3D path in order (a lone
    /// circle is two half arcs).
    pub fn path(&self, curve: usize) -> Result<Vec<fission_kernel::Seg3>, String> {
        use fission_kernel::Seg3;
        let sk = &self.sketch;
        if curve >= sk.curves.len() {
            return Err("the path curve no longer exists".into());
        }
        let w = |p: [f64; 2]| self.plane.to_world(p);
        let (chain, _) = sk.chain(curve);
        let mut out = Vec::new();
        for (c, rev) in chain {
            match sk.curves[c] {
                sketch2d::Curve::Line { a, b, .. } => {
                    let (a, b) = (w(sk.points[a]), w(sk.points[b]));
                    out.push(if rev { Seg3::Line(b, a) } else { Seg3::Line(a, b) });
                }
                sketch2d::Curve::Arc { center, start, end, .. } => {
                    let (c2, s2, e2) = (sk.points[center], sk.points[start], sk.points[end]);
                    let r = sketch2d::dist(c2, s2);
                    let a0 = (s2[1] - c2[1]).atan2(s2[0] - c2[0]);
                    let a1 = (e2[1] - c2[1]).atan2(e2[0] - c2[0]);
                    let mid_a = a0 + (a1 - a0).rem_euclid(std::f64::consts::TAU) / 2.0;
                    let mid = w([c2[0] + r * mid_a.cos(), c2[1] + r * mid_a.sin()]);
                    let (s3, e3) = (w(s2), w(e2));
                    out.push(if rev { Seg3::Arc { start: e3, mid, end: s3 } } else { Seg3::Arc { start: s3, mid, end: e3 } });
                }
                sketch2d::Curve::Circle { center, radius, .. } => {
                    let c2 = sk.points[center];
                    let at = |t: f64| w([c2[0] + radius * t.cos(), c2[1] + radius * t.sin()]);
                    use std::f64::consts::{FRAC_PI_2, PI};
                    out.push(Seg3::Arc { start: at(0.0), mid: at(FRAC_PI_2), end: at(PI) });
                    out.push(Seg3::Arc { start: at(PI), mid: at(3.0 * FRAC_PI_2), end: at(0.0) });
                }
            }
        }
        Ok(out)
    }

    /// The region containing `p` (regions don't overlap: holes are excluded).
    pub fn region_at(&self, p: [f64; 2]) -> Option<&sketch2d::Region> {
        self.regions.iter().filter(|r| r.contains(p)).min_by(|a, b| a.area.total_cmp(&b.area))
    }
}

fn region_profile(plane: Plane, region: &sketch2d::Region) -> Profile {
    let convert = |l: &sketch2d::Loop| -> Vec<Seg2> {
        l.segments
            .iter()
            .map(|s| match *s {
                sketch2d::Segment::Line(a, b) => Seg2::Line(a, b),
                sketch2d::Segment::Arc { start, mid, end } => Seg2::Arc { start, mid, end },
                sketch2d::Segment::Circle { center, radius } => Seg2::Circle { center, radius },
            })
            .collect()
    };
    Profile::Region { plane, outer: convert(&region.outer), holes: region.holes.iter().map(convert).collect() }
}

/// A point inside a region (for referencing it), preferring its centroid.
pub fn interior_point(region: &sketch2d::Region) -> [f64; 2] {
    let poly = region.outer.polygon();
    let n = poly.len().max(1) as f64;
    let centroid = [poly.iter().map(|p| p[0]).sum::<f64>() / n, poly.iter().map(|p| p[1]).sum::<f64>() / n];
    if region.contains(centroid) {
        return centroid;
    }
    // Otherwise scan a grid over the bounding box.
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for p in &poly {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    for i in 1..32 {
        for j in 1..32 {
            let p = [lo[0] + (hi[0] - lo[0]) * i as f64 / 32.0, lo[1] + (hi[1] - lo[1]) * j as f64 / 32.0];
            if region.contains(p) {
                return p;
            }
        }
    }
    centroid
}

/// How a new solid combines with the existing target body (Fusion's "Operation").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operation {
    NewBody,
    Join,
    Cut,
    Intersect,
}

impl Operation {
    pub const ALL: [Operation; 4] = [Operation::NewBody, Operation::Join, Operation::Cut, Operation::Intersect];

    pub fn label(self) -> &'static str {
        match self {
            Operation::NewBody => "New Body",
            Operation::Join => "Join",
            Operation::Cut => "Cut",
            Operation::Intersect => "Intersect",
        }
    }

    fn boolean(self) -> Option<BooleanOp> {
        match self {
            Operation::NewBody => None,
            Operation::Join => Some(BooleanOp::Union),
            Operation::Cut => Some(BooleanOp::Subtract),
            Operation::Intersect => Some(BooleanOp::Intersect),
        }
    }
}

/// The top of a hole.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum HoleKind {
    Simple,
    /// A wider flat-bottomed recess, for a socket-head screw.
    Counterbore { diameter: f64, depth: f64 },
    /// A cone, for a flat-head screw; `angle` is the included angle.
    Countersink { diameter: f64, angle: f64 },
}

impl HoleKind {
    pub fn label(self) -> &'static str {
        match self {
            HoleKind::Simple => "Simple",
            HoleKind::Counterbore { .. } => "Counterbore",
            HoleKind::Countersink { .. } => "Countersink",
        }
    }
}

/// How deep a through hole goes, in mm: past anything a model holds.
const THROUGH_DEPTH: f64 = 10_000.0;

/// A hole's half cross-section as (radius, height) points, height 0 at the
/// face and negative going in; it starts a little above the face so the cut
/// doesn't share the face's plane.
pub fn hole_section(diameter: f64, depth: f64, kind: HoleKind, tip_angle: f64) -> Result<Vec<[f64; 2]>, String> {
    let r = diameter / 2.0;
    if r <= 0.0 {
        return Err("diameter must be positive".into());
    }
    if depth <= 0.0 {
        return Err("depth must be positive".into());
    }
    let above = 0.01_f64.max(diameter * 1e-3);
    let mut pts = vec![[0.0, above]];
    match kind {
        HoleKind::Simple => pts.extend([[r, above]]),
        HoleKind::Counterbore { diameter: cd, depth: cdepth } => {
            if cd <= diameter {
                return Err("the counterbore must be wider than the hole".into());
            }
            if cdepth <= 0.0 || cdepth >= depth {
                return Err("the counterbore depth must be between 0 and the hole depth".into());
            }
            pts.extend([[cd / 2.0, above], [cd / 2.0, -cdepth], [r, -cdepth]]);
        }
        HoleKind::Countersink { diameter: cd, angle } => {
            if cd <= diameter {
                return Err("the countersink must be wider than the hole".into());
            }
            if !(1.0..179.0).contains(&angle) {
                return Err("the countersink angle must be between 1° and 179°".into());
            }
            let sink = (cd / 2.0 - r) / (angle.to_radians() / 2.0).tan();
            if sink >= depth {
                return Err("the countersink is deeper than the hole".into());
            }
            pts.extend([[cd / 2.0, above], [cd / 2.0, 0.0], [r, -sink]]);
        }
    }
    pts.push([r, -depth]);
    if tip_angle > 0.0 && tip_angle < 180.0 {
        pts.push([0.0, -depth - r / (tip_angle.to_radians() / 2.0).tan()]);
    } else {
        pts.push([0.0, -depth]);
    }
    Ok(pts)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum FeatureKind {
    /// A 2D sketch on a plane. Makes no body; extrudes reference its regions.
    Sketch { plane: OriginPlane, offset: f64, sketch: sketch2d::Sketch },
    /// A construction plane: `base` tilted `angle` degrees about its X
    /// direction (the hinge line through its origin), then moved `offset`
    /// along the tilted normal. Makes no body; sketches can be started on it.
    Plane {
        base: OriginPlane,
        offset: f64,
        #[serde(default)]
        angle: f64,
    },
    /// Tilt faces of one body (Fusion's Draft): hinged where they cross the
    /// plane, leaning in by `degrees` going along its normal (or against it
    /// with `flip`).
    /// Move faces along their normals (Press Pull / Offset Face); the
    /// faces around them stretch to follow. Applies to the first face's body.
    OffsetFace {
        faces: Vec<FaceRef>,
        distance: f64,
    },
    /// A modeled ISO metric thread cut into a cylindrical face (outside on a
    /// shaft, inside a hole). `size` is the nominal diameter, for its name;
    /// the geometry follows the face.
    Thread {
        face: FaceRef,
        size: f64,
        pitch: f64,
        #[serde(default)]
        left_hand: bool,
    },
    /// A spring around the normal through `center` of a plane: `diameter`
    /// across the coil's centerline, round wire of `section` diameter.
    Coil {
        plane: OriginPlane,
        offset: f64,
        center: [f64; 2],
        diameter: f64,
        pitch: f64,
        turns: f64,
        section: f64,
        #[serde(default)]
        left_hand: bool,
        operation: Operation,
    },
    /// Delete faces and heal (holes fill, fillets go sharp). Applies to the
    /// first face's body.
    DeleteFace { faces: Vec<FaceRef> },
    Draft {
        #[serde(default)]
        faces: Vec<FaceRef>,
        plane: OriginPlane,
        offset: f64,
        degrees: f64,
        #[serde(default)]
        flip: bool,
    },
    /// Cut a body in two along a plane (offset along its normal). The part
    /// behind the plane keeps the body; the part in front becomes a new
    /// body. `body: None` splits the last body.
    Split {
        #[serde(default)]
        body: Option<usize>,
        plane: OriginPlane,
        offset: f64,
    },
    /// Grow or shrink bodies evenly about a point (empty `bodies`: the last).
    Scale {
        #[serde(default)]
        bodies: Vec<usize>,
        center: [f64; 3],
        factor: f64,
    },
    /// A construction axis: a line through `origin` along `dir`. Makes no
    /// body; revolves, moves and circular patterns can turn about it.
    Axis { origin: [f64; 3], dir: [f64; 3] },
    /// A sphere centred on `center` of a plane (CREATE ▸ Sphere).
    Sphere { plane: OriginPlane, offset: f64, center: [f64; 2], diameter: f64, operation: Operation },
    /// A ring around the plane's normal through `center`: `diameter` across
    /// the ring's centerline, `section` across the tube (CREATE ▸ Torus).
    Torus { plane: OriginPlane, offset: f64, center: [f64; 2], diameter: f64, section: f64, operation: Operation },
    /// A construction point. Makes no body.
    Point { pos: [f64; 3] },
    /// A reference image on a plane (Fusion's Canvas), for tracing over.
    /// Makes no body. `crop` keeps that part of the image (left, top,
    /// right, bottom as fractions of its width and height), shown `size` mm
    /// across (width and height scale independently), centred on `center`,
    /// turned `rotation` degrees in the plane.
    Canvas {
        path: String,
        plane: OriginPlane,
        #[serde(default)]
        offset: f64,
        #[serde(default)]
        center: [f64; 2],
        size: [f64; 2],
        #[serde(default)]
        rotation: f64,
        #[serde(default = "full_crop")]
        crop: [f64; 4],
        #[serde(default = "half")]
        opacity: f64,
    },
    /// A feature from another program's timeline that Fission can't replay
    /// yet (Fusion's name for it, and why). It does nothing and shows as an
    /// error, so the imported timeline stays complete.
    Unsupported { kind: String, note: String },
    Extrude { sketch: Sketch, distance: f64, symmetric: bool, operation: Operation },
    /// A drilled hole at `center` on a face's plane, cut into the target body.
    Hole {
        plane: OriginPlane,
        offset: f64,
        center: [f64; 2],
        /// Drill along the plane's normal instead of against it.
        #[serde(default)]
        flip: bool,
        diameter: f64,
        depth: f64,
        /// All the way through (`depth` is ignored).
        #[serde(default)]
        through: bool,
        kind: HoleKind,
        /// Included angle of the drill point at the bottom; 0 is flat.
        #[serde(default)]
        tip_angle: f64,
    },
    Revolve { sketch: Sketch, axis: Axis, degrees: f64, operation: Operation },
    /// Empty `edges` means every edge of the last body.
    Fillet {
        radius: f64,
        #[serde(default)]
        edges: Vec<EdgeRef>,
    },
    Chamfer {
        distance: f64,
        #[serde(default)]
        edges: Vec<EdgeRef>,
    },
    /// A body read from a STEP file, re-read on every regeneration.
    ImportStep {
        path: String,
        operation: Operation,
        /// Parts of the file left out of the body (hidden in the browser).
        #[serde(default)]
        hidden_parts: Vec<usize>,
    },
    /// Hollow a body inward; `faces` are left open (empty: a closed void).
    Shell {
        thickness: f64,
        #[serde(default)]
        faces: Vec<FaceRef>,
    },
    /// Boolean between existing bodies (Fusion's Combine).
    Combine { target: usize, tools: Vec<usize>, operation: Operation, keep_tools: bool },
    /// Move (or copy) bodies: rotate about an origin axis, then translate.
    Move {
        #[serde(default)]
        bodies: Vec<usize>,
        translate: [f64; 3],
        axis: Axis,
        degrees: f64,
        copy: bool,
    },
    /// Lets bodies move relative to the rest of the design, like Fusion's
    /// as-built joint: a hinge turning `value` degrees about `axis`, or a
    /// slide `value` mm along it, kept within `min`..=`max`. The model shows
    /// the joint at `value`; Animate and Interference sweep it.
    Joint {
        #[serde(default)]
        bodies: Vec<usize>,
        motion: JointMotion,
        axis: Axis,
        value: f64,
        min: f64,
        max: f64,
    },
    /// Mirror bodies across an origin plane (offset along its normal).
    Mirror {
        #[serde(default)]
        bodies: Vec<usize>,
        plane: OriginPlane,
        offset: f64,
        join: bool,
    },
    Pattern {
        #[serde(default)]
        bodies: Vec<usize>,
        pattern: PatternKind,
        join: bool,
    },
    /// A solid through two or more sketch regions, in order.
    Loft { sections: Vec<Section>, operation: Operation },
    /// A sketch profile swept along the chain of sketch curves through `path`.
    Sweep { profile: Sketch, path: Option<PathRef>, operation: Operation },
}

/// How a joint moves its bodies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum JointMotion {
    /// Turns about the axis (value in degrees).
    Revolute,
    /// Slides along the axis (value in mm).
    Slider,
}

impl JointMotion {
    pub fn label(self) -> &'static str {
        match self {
            JointMotion::Revolute => "Revolute",
            JointMotion::Slider => "Slider",
        }
    }
}

/// A rigid motion: rotate `degrees` about the line through `origin` along
/// unit `dir`, then translate by `by`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rigid {
    pub origin: [f64; 3],
    pub dir: [f64; 3],
    pub degrees: f64,
    pub by: [f64; 3],
}

impl Rigid {
    pub const IDENTITY: Rigid = Rigid { origin: [0.0; 3], dir: [0.0, 0.0, 1.0], degrees: 0.0, by: [0.0; 3] };

    /// A joint's motion `value` away from where it was built.
    pub fn joint(motion: JointMotion, origin: [f64; 3], dir: [f64; 3], value: f64) -> Rigid {
        match motion {
            JointMotion::Revolute => Rigid { origin, dir, degrees: value, by: [0.0; 3] },
            JointMotion::Slider => Rigid { by: dir.map(|d| d * value), ..Rigid::IDENTITY },
        }
    }

    /// Rotate a direction (no translation).
    pub fn turn(&self, v: [f64; 3]) -> [f64; 3] {
        if self.degrees == 0.0 {
            return v;
        }
        // Rodrigues: v cos + (k × v) sin + k (k·v)(1 - cos)
        let k = self.dir;
        let (s, c) = self.degrees.to_radians().sin_cos();
        let kxv = [k[1] * v[2] - k[2] * v[1], k[2] * v[0] - k[0] * v[2], k[0] * v[1] - k[1] * v[0]];
        let kv = k[0] * v[0] + k[1] * v[1] + k[2] * v[2];
        std::array::from_fn(|i| v[i] * c + kxv[i] * s + k[i] * kv * (1.0 - c))
    }

    pub fn point(&self, p: [f64; 3]) -> [f64; 3] {
        let local = std::array::from_fn(|i| p[i] - self.origin[i]);
        let r = self.turn(local);
        std::array::from_fn(|i| r[i] + self.origin[i] + self.by[i])
    }
}

/// A joint as the UI and the sweep tools see it.
#[derive(Clone, Debug, PartialEq)]
pub struct JointInfo {
    /// The Joint feature's index.
    pub feature: usize,
    pub name: String,
    pub motion: JointMotion,
    /// Moved bodies by creating feature (resolved: never empty once regenerated).
    pub bodies: Vec<usize>,
    pub axis: Axis,
    pub value: f64,
    pub min: f64,
    pub max: f64,
}

impl JointInfo {
    pub fn clamp(&self, v: f64) -> f64 {
        v.clamp(self.min.min(self.max), self.max.max(self.min))
    }
}

fn full_crop() -> [f64; 4] {
    [0.0, 0.0, 1.0, 1.0]
}

fn half() -> f64 {
    0.5
}

/// A canvas as the viewport draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasInfo {
    pub feature: usize,
    pub path: String,
    /// Corners in model space: bottom-left, bottom-right, top-right, top-left.
    pub corners: [[f64; 3]; 4],
    /// Image coordinates (0..1, y down) of the same corners.
    pub uv: [[f64; 2]; 4],
    pub opacity: f64,
}

/// Change a canvas's crop, keeping what stays visible the same size and in
/// the same place on its plane (cropping trims the picture; it doesn't
/// stretch it). Crop edges are clamped to the image and kept in order.
pub fn recrop_canvas(kind: &mut FeatureKind, crop: [f64; 4]) {
    let FeatureKind::Canvas { center, size, rotation, crop: old, .. } = kind else { return };
    let min = 0.01;
    let mut c = crop.map(|v| v.clamp(0.0, 1.0));
    if c[2] - c[0] < min {
        c[2] = (c[0] + min).min(1.0);
        c[0] = c[2] - min;
    }
    if c[3] - c[1] < min {
        c[3] = (c[1] + min).min(1.0);
        c[1] = c[3] - min;
    }
    let (ow, oh) = (old[2] - old[0], old[3] - old[1]);
    if ow <= 0.0 || oh <= 0.0 {
        *old = c;
        return;
    }
    // mm per image unit along the canvas's own x and y.
    let (sx, sy) = (size[0] / ow, size[1] / oh);
    let dx = ((c[0] + c[2]) - (old[0] + old[2])) / 2.0 * sx;
    let dy = -((c[1] + c[3]) - (old[1] + old[3])) / 2.0 * sy; // image y runs down
    let (s, co) = rotation.to_radians().sin_cos();
    center[0] += dx * co - dy * s;
    center[1] += dx * s + dy * co;
    *size = [(c[2] - c[0]) * sx, (c[3] - c[1]) * sy];
    *old = c;
}

/// A sketch region, by a point inside it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Section {
    pub sketch: usize,
    pub point: [f64; 2],
}

/// A sketch curve (its whole connected chain is the path).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PathRef {
    pub sketch: usize,
    pub curve: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PatternKind {
    /// `count` copies along `dir`, `spacing` apart (including the original);
    /// optionally a second direction.
    Rectangular { dir: Axis, count: u32, spacing: f64, second: Option<(Axis, u32, f64)> },
    /// `count` copies spread over `degrees` around an origin axis (360 = full circle).
    Circular { axis: Axis, count: u32, degrees: f64 },
}

/// A picked face: owning body (creating feature) and geometric signature.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FaceRef {
    pub body: usize,
    pub center: [f64; 3],
    pub normal: [f64; 3],
}

impl FaceRef {
    pub fn hint(&self) -> FaceHint {
        FaceHint { center: self.center, normal: self.normal }
    }
}

/// A picked edge: which body (by the index of the feature that created it)
/// and its geometric signature, re-resolved on every regeneration.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgeRef {
    pub body: usize,
    pub mid: [f64; 3],
    pub dir: [f64; 3],
}

impl EdgeRef {
    pub fn hint(&self) -> EdgeHint {
        EdgeHint { mid: self.mid, dir: self.dir }
    }
}

/// Rotation axes: the origin axes, a line in a sketch (any line, usually a
/// construction line), as Fusion's revolve allows, or a construction axis.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Axis {
    X,
    Y,
    Z,
    SketchLine { sketch: usize, curve: usize },
    /// A construction axis, by the feature that made it. Those have no
    /// parameters, so the line is copied in rather than looked up.
    Line { feature: usize, origin: [f64; 3], dir: [f64; 3] },
}

impl Axis {
    pub const ALL: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];

    pub fn label(self) -> String {
        match self {
            Axis::X => "X".into(),
            Axis::Y => "Y".into(),
            Axis::Z => "Z".into(),
            Axis::SketchLine { curve, .. } => format!("Sketch line {}", curve + 1),
            Axis::Line { .. } => "Construction axis".into(),
        }
    }

    /// Rewrite the feature index this axis refers to (see `Document::remap`).
    fn remap(&mut self, map: &impl Fn(&mut usize)) {
        match self {
            Axis::SketchLine { sketch, .. } => map(sketch),
            Axis::Line { feature, .. } => map(feature),
            Axis::X | Axis::Y | Axis::Z => {}
        }
    }

    /// Direction of an origin axis (a sketch line's comes from `resolve`).
    pub fn dir(self) -> [f64; 3] {
        match self {
            Axis::X => [1.0, 0.0, 0.0],
            Axis::Y => [0.0, 1.0, 0.0],
            Axis::Z | Axis::SketchLine { .. } => [0.0, 0.0, 1.0],
            Axis::Line { dir, .. } => dir,
        }
    }

    /// A point on the axis and its direction, in model space.
    pub fn resolve(self, sketches: &[Option<SketchOutput>]) -> Result<([f64; 3], [f64; 3]), String> {
        let Axis::SketchLine { sketch, curve } = self else {
            let origin = if let Axis::Line { origin, .. } = self { origin } else { [0.0; 3] };
            return Ok((origin, self.dir()));
        };
        let out = sketches.get(sketch).and_then(Option::as_ref).ok_or("the axis sketch is missing or not solved")?;
        let [a, b] = out.lines.get(curve).copied().flatten().ok_or("the axis line no longer exists")?;
        let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if len < 1e-9 {
            return Err("the axis line has no length".into());
        }
        Ok((a, [d[0] / len, d[1] / len, d[2] / len]))
    }
}

impl FeatureKind {
    pub fn type_name(&self) -> &'static str {
        match self {
            FeatureKind::Sketch { .. } => "Sketch",
            FeatureKind::Loft { .. } => "Loft",
            FeatureKind::Sweep { .. } => "Sweep",
            FeatureKind::Plane { .. } => "Plane",
            FeatureKind::Axis { .. } => "Axis",
            FeatureKind::Point { .. } => "Point",
            FeatureKind::Canvas { .. } => "Canvas",
            FeatureKind::Unsupported { .. } => "Unsupported",
            FeatureKind::Sphere { .. } => "Sphere",
            FeatureKind::Torus { .. } => "Torus",
            FeatureKind::Extrude { .. } => "Extrude",
            FeatureKind::Hole { .. } => "Hole",
            FeatureKind::Revolve { .. } => "Revolve",
            FeatureKind::Fillet { .. } => "Fillet",
            FeatureKind::Chamfer { .. } => "Chamfer",
            FeatureKind::ImportStep { .. } => "Import",
            FeatureKind::Shell { .. } => "Shell",
            FeatureKind::Draft { .. } => "Draft",
            FeatureKind::OffsetFace { .. } => "Offset Face",
            FeatureKind::DeleteFace { .. } => "Delete Face",
            FeatureKind::Thread { .. } => "Thread",
            FeatureKind::Coil { .. } => "Coil",
            FeatureKind::Combine { .. } => "Combine",
            FeatureKind::Move { .. } => "Move",
            FeatureKind::Joint { .. } => "Joint",
            FeatureKind::Mirror { .. } => "Mirror",
            FeatureKind::Scale { .. } => "Scale",
            FeatureKind::Split { .. } => "Split Body",
            FeatureKind::Pattern { .. } => "Pattern",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Feature {
    pub name: String,
    pub kind: FeatureKind,
    #[serde(default)]
    pub suppressed: bool,
    /// Body a Join/Cut/Intersect applies to, by the index of the feature that
    /// created it. None means the most recent body.
    #[serde(default)]
    pub target: Option<usize>,
    /// Values typed as expressions naming parameters, by field key (see
    /// `FeatureKind::values_mut`; sketch dimensions use `dim:<index>`).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub exprs: std::collections::BTreeMap<String, String>,
    /// Appearance of the body this feature creates: sRGB 0..1, as color
    /// pickers and swatches show it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[f32; 3]>,
    /// Name of the body this feature creates (default: Body<n>).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub name: String,
    pub features: Vec<Feature>,
    /// Features at index >= `marker` are rolled back (not applied), like
    /// dragging Fusion's timeline marker.
    pub marker: usize,
    /// User parameters, usable by name in any value.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<Parameter>,
}

impl Default for Document {
    fn default() -> Self {
        Document { name: "Untitled".into(), features: Vec::new(), marker: 0, parameters: Vec::new() }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DocError {
    #[error("couldn't read document: {0}")]
    Parse(#[from] ron::error::SpannedError),
    #[error("couldn't write document: {0}")]
    Write(#[from] ron::Error),
}

impl Document {
    /// A small part that exercises every feature type.
    pub fn sample() -> Self {
        let mut doc = Document { name: "Bracket".into(), ..Default::default() };
        doc.push(FeatureKind::Extrude {
            sketch: Sketch::Rect { plane: OriginPlane::XY, offset: 0.0, center: [0.0, 0.0], size: [60.0, 40.0] },
            distance: 12.0,
            symmetric: false,
            operation: Operation::NewBody,
        });
        doc.push(FeatureKind::Extrude {
            sketch: Sketch::Rect { plane: OriginPlane::XY, offset: 12.0, center: [-20.0, 0.0], size: [20.0, 40.0] },
            distance: 25.0,
            symmetric: false,
            operation: Operation::Join,
        });
        doc.push(FeatureKind::Extrude {
            sketch: Sketch::Circle { plane: OriginPlane::XY, offset: 0.0, center: [15.0, 0.0], radius: 8.0 },
            distance: 12.0,
            symmetric: false,
            operation: Operation::Cut,
        });
        doc.push(FeatureKind::Fillet { radius: 2.0, edges: Vec::new() });
        doc
    }

    /// Insert a feature at the marker (Fusion inserts at the marker, then moves past it).
    pub fn push(&mut self, kind: FeatureKind) -> usize {
        let n = self.features.iter().filter(|f| f.kind.type_name() == kind.type_name()).count() + 1;
        let name = format!("{}{}", kind.type_name(), n);
        let at = self.marker.min(self.features.len());
        self.remap(|i| Some(if i >= at { i + 1 } else { i }));
        self.features.insert(at, Feature { name, kind, suppressed: false, target: None, exprs: Default::default(), color: None, body_name: None });
        self.marker = at + 1;
        at
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.features.len() {
            self.features.remove(index);
            self.remap(|i| match i.cmp(&index) {
                std::cmp::Ordering::Less => Some(i),
                std::cmp::Ordering::Equal => None,
                std::cmp::Ordering::Greater => Some(i - 1),
            });
            if self.marker > index {
                self.marker -= 1;
            }
        }
    }

    /// Move a feature to a new position, keeping the marker after the same set of features.
    pub fn reorder(&mut self, from: usize, to: usize) {
        if from >= self.features.len() || to >= self.features.len() || from == to {
            return;
        }
        let feature = self.features.remove(from);
        self.features.insert(to, feature);
        self.remap(|i| {
            Some(if i == from {
                to
            } else if from < to && i > from && i <= to {
                i - 1
            } else if to < from && i >= to && i < from {
                i + 1
            } else {
                i
            })
        });
    }

    /// Rewrite every feature-index reference (target bodies, edge owners,
    /// sketch profiles) after features move. `None` means the referenced
    /// feature was deleted; those references dangle and regeneration reports it.
    fn remap(&mut self, f: impl Fn(usize) -> Option<usize>) {
        let map = |i: &mut usize| *i = f(*i).unwrap_or(usize::MAX);
        for feature in &mut self.features {
            if let Some(t) = &mut feature.target {
                map(t);
            }
            match &mut feature.kind {
                FeatureKind::Fillet { edges, .. } | FeatureKind::Chamfer { edges, .. } => {
                    edges.iter_mut().for_each(|e| map(&mut e.body))
                }
                FeatureKind::Extrude { sketch: Sketch::Profiles { sketch, .. }, .. } => map(sketch),
                FeatureKind::Revolve { sketch, axis, .. } => {
                    if let Sketch::Profiles { sketch, .. } = sketch {
                        map(sketch);
                    }
                    axis.remap(&map);
                }
                FeatureKind::Shell { faces, .. }
                | FeatureKind::Draft { faces, .. }
                | FeatureKind::OffsetFace { faces, .. }
                | FeatureKind::DeleteFace { faces } => {
                    faces.iter_mut().for_each(|f| map(&mut f.body))
                }
                FeatureKind::Combine { target, tools, .. } => {
                    map(target);
                    tools.iter_mut().for_each(map);
                }
                FeatureKind::Move { bodies, axis, .. } | FeatureKind::Joint { bodies, axis, .. } => {
                    bodies.iter_mut().for_each(map);
                    axis.remap(&map);
                }
                FeatureKind::Pattern { bodies, pattern, .. } => {
                    bodies.iter_mut().for_each(map);
                    match pattern {
                        PatternKind::Circular { axis, .. } => axis.remap(&map),
                        PatternKind::Rectangular { dir, second, .. } => {
                            dir.remap(&map);
                            if let Some((d, ..)) = second {
                                d.remap(&map);
                            }
                        }
                    }
                }
                FeatureKind::Mirror { bodies, .. } | FeatureKind::Scale { bodies, .. } => bodies.iter_mut().for_each(map),
                FeatureKind::Split { body: Some(b), .. } => map(b),
                FeatureKind::Thread { face, .. } => map(&mut face.body),
                FeatureKind::Loft { sections, .. } => sections.iter_mut().for_each(|s| map(&mut s.sketch)),
                FeatureKind::Sweep { profile, path, .. } => {
                    if let Sketch::Profiles { sketch, .. } = profile {
                        map(sketch);
                    }
                    if let Some(p) = path {
                        map(&mut p.sketch);
                    }
                }
                _ => {}
            }
        }
    }

    /// Construction points that are applied and unsuppressed: (feature, position).
    pub fn construction_points(&self) -> Vec<(usize, [f64; 3])> {
        self.features
            .iter()
            .enumerate()
            .take(self.marker)
            .filter(|(_, f)| !f.suppressed)
            .filter_map(|(i, f)| match f.kind {
                FeatureKind::Point { pos } => Some((i, pos)),
                _ => None,
            })
            .collect()
    }

    /// Construction axes that are applied and unsuppressed: (feature, origin, dir).
    pub fn construction_axes(&self) -> Vec<(usize, [f64; 3], [f64; 3])> {
        self.features
            .iter()
            .enumerate()
            .take(self.marker)
            .filter(|(_, f)| !f.suppressed)
            .filter_map(|(i, f)| match f.kind {
                FeatureKind::Axis { origin, dir } => Some((i, origin, dir)),
                _ => None,
            })
            .collect()
    }

    /// Canvases that are applied and unsuppressed, with their corners placed.
    pub fn canvases(&self) -> Vec<CanvasInfo> {
        let mut out = Vec::new();
        for (i, f) in self.features.iter().enumerate().take(self.marker) {
            let FeatureKind::Canvas { path, plane, offset, center, size, rotation, crop, opacity } = &f.kind else { continue };
            if f.suppressed {
                continue;
            }
            let p = plane.plane(*offset);
            let (s, c) = rotation.to_radians().sin_cos();
            let (hw, hh) = (size[0] / 2.0, size[1] / 2.0);
            let at = |x: f64, y: f64| p.to_world([center[0] + x * c - y * s, center[1] + x * s + y * c]);
            let [l, t, r, b] = *crop;
            out.push(CanvasInfo {
                feature: i,
                path: path.clone(),
                corners: [at(-hw, -hh), at(hw, -hh), at(hw, hh), at(-hw, hh)],
                uv: [[l, b], [r, b], [r, t], [l, t]],
                opacity: opacity.clamp(0.0, 1.0),
            });
        }
        out
    }

    /// True when `self` and `other` differ at most in appearance, so they
    /// build the same bodies and need no regeneration between them.
    pub fn same_except_appearance(&self, other: &Document) -> bool {
        let plain = |d: &Document| {
            let mut d = d.clone();
            d.features.iter_mut().for_each(|f| f.color = None);
            d
        };
        self.features.len() == other.features.len() && plain(self) == plain(other)
    }

    /// Joints that are applied and unsuppressed, in timeline order. A joint
    /// with no bodies listed moves the last body before it, as regeneration
    /// does; `bodies` says which (by creating feature).
    pub fn joints(&self) -> Vec<JointInfo> {
        let mut out = Vec::new();
        for (i, f) in self.features.iter().enumerate().take(self.marker) {
            let FeatureKind::Joint { bodies, motion, axis, value, min, max } = &f.kind else { continue };
            if f.suppressed {
                continue;
            }
            let bodies = if bodies.is_empty() { self.last_body_before(i).into_iter().collect() } else { bodies.clone() };
            out.push(JointInfo { feature: i, name: f.name.clone(), motion: *motion, bodies, axis: *axis, value: *value, min: *min, max: *max });
        }
        out
    }

    /// The joint called `name` (case-insensitive), or the only joint when
    /// `name` is empty.
    pub fn joint(&self, name: &str) -> Result<JointInfo, String> {
        let joints = self.joints();
        if name.is_empty() {
            return match joints.len() {
                1 => Ok(joints.into_iter().next().unwrap()),
                0 => Err("the design has no joints".into()),
                _ => Err(format!(
                    "the design has several joints; name one: {}",
                    joints.iter().map(|j| j.name.as_str()).collect::<Vec<_>>().join(", ")
                )),
            };
        }
        joints.into_iter().find(|j| j.name.eq_ignore_ascii_case(name)).ok_or_else(|| format!("no joint named '{name}'"))
    }

    /// Set a joint's value (clamped to its limits). When the value is tied to
    /// a single user parameter, the parameter changes instead, so the
    /// design stays parametric.
    pub fn set_joint_value(&mut self, feature: usize, value: f64) {
        let Some(f) = self.features.get_mut(feature) else { return };
        let FeatureKind::Joint { value: v, min, max, motion, .. } = &mut f.kind else { return };
        let value = value.clamp(min.min(*max), max.max(*min));
        *v = value;
        let unit = if *motion == JointMotion::Revolute { "deg" } else { "mm" };
        if let Some(expr) = f.exprs.get("value").cloned() {
            let name = expr.trim();
            match self.parameters.iter_mut().find(|p| p.name == name) {
                Some(p) => p.expression = format!("{value} {unit}"),
                None => {
                    self.features[feature].exprs.remove("value");
                }
            }
        }
    }

    /// The body-creating feature regeneration would treat as "the last body"
    /// just before feature `before` (a guess without a kernel: the latest
    /// feature that makes a new body).
    fn last_body_before(&self, before: usize) -> Option<usize> {
        self.features[..before.min(self.features.len())].iter().enumerate().rev().find_map(|(i, f)| {
            let new_body = match &f.kind {
                FeatureKind::Extrude { operation, .. }
                | FeatureKind::Revolve { operation, .. }
                | FeatureKind::Sphere { operation, .. }
                | FeatureKind::Torus { operation, .. }
                | FeatureKind::Loft { operation, .. }
                | FeatureKind::Sweep { operation, .. }
                | FeatureKind::Coil { operation, .. }
                | FeatureKind::ImportStep { operation, .. } => *operation == Operation::NewBody,
                FeatureKind::Split { .. } | FeatureKind::Mirror { join: false, .. } | FeatureKind::Pattern { join: false, .. } => true,
                FeatureKind::Move { copy, .. } => *copy,
                _ => false,
            };
            (new_body && !f.suppressed).then_some(i)
        })
    }

    /// Solved sketches (applied, unsuppressed), without a kernel: sketches are
    /// pure 2D, so the UI can have them even when bodies come from elsewhere.
    /// Construction planes that are applied and unsuppressed: (feature, base, offset).
    pub fn construction_planes(&self) -> Vec<(usize, OriginPlane, f64)> {
        self.features
            .iter()
            .enumerate()
            .take(self.marker)
            .filter(|(_, f)| !f.suppressed)
            .filter_map(|(i, f)| match f.kind {
                FeatureKind::Plane { base, offset, angle } => Some((i, base.tilted(angle), offset)),
                _ => None,
            })
            .collect()
    }

    pub fn sketch_outputs(&self) -> Vec<Option<SketchOutput>> {
        self.features
            .iter()
            .enumerate()
            .map(|(i, f)| {
                if i >= self.marker || f.suppressed {
                    return None;
                }
                sketch_output(&f.kind).and_then(Result::ok)
            })
            .collect()
    }

    pub fn to_ron(&self) -> Result<String, DocError> {
        Ok(ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())?)
    }

    pub fn from_ron(text: &str) -> Result<Self, DocError> {
        let mut doc: Document = ron::from_str(text)?;
        doc.marker = doc.marker.min(doc.features.len());
        Ok(doc)
    }

    /// Replay the timeline up to the marker.
    pub fn regenerate<K: Kernel>(&self, kernel: &K) -> Regen<K::Solid> {
        let mut bodies: Vec<Body<K::Solid>> = Vec::new();
        let mut status = Vec::with_capacity(self.features.len());
        let mut sketches: Vec<Option<SketchOutput>> = vec![None; self.features.len()];
        let mut parts = vec![0; self.features.len()];
        let mut part_faces = vec![Vec::new(); self.features.len()];

        for (i, feature) in self.features.iter().enumerate() {
            if i >= self.marker {
                status.push(FeatureStatus::RolledBack);
                continue;
            }
            if feature.suppressed {
                status.push(FeatureStatus::Suppressed);
                continue;
            }
            if let FeatureKind::Plane { .. } | FeatureKind::Axis { .. } | FeatureKind::Point { .. } | FeatureKind::Canvas { .. } = &feature.kind {
                status.push(FeatureStatus::Ok);
                continue;
            }
            if let FeatureKind::Unsupported { kind, note } = &feature.kind {
                let why = if note.is_empty() { String::new() } else { format!(": {note}") };
                status.push(FeatureStatus::Error(format!("Fusion's {kind} isn't replayed yet{why}")));
                continue;
            }
            if let FeatureKind::Sketch { .. } = &feature.kind {
                match sketch_output(&feature.kind) {
                    Some(Ok(out)) => {
                        sketches[i] = Some(out);
                        status.push(FeatureStatus::Ok);
                    }
                    Some(Err(e)) => status.push(FeatureStatus::Error(e)),
                    None => unreachable!(),
                }
                continue;
            }
            match apply(kernel, feature, &mut bodies, &sketches, (&mut parts, &mut part_faces), i) {
                Ok(()) => status.push(FeatureStatus::Ok),
                Err(message) => status.push(FeatureStatus::Error(message)),
            }
        }
        for body in &mut bodies {
            if let Some(name) = self.features.get(body.created_by).and_then(|f| f.body_name.clone()) {
                body.name = name;
            }
        }
        Regen { bodies, status, parts, part_faces, sketches }
    }
}

/// Where imports record their parts: counts, and (part, face count) lists.
type PartTally<'a> = (&'a mut [usize], &'a mut [Vec<(usize, usize)>]);

fn apply<K: Kernel>(
    kernel: &K,
    feature: &Feature,
    bodies: &mut Vec<Body<K::Solid>>,
    sketches: &[Option<SketchOutput>],
    (part_counts, part_faces): PartTally<'_>,
    index: usize,
) -> Result<(), String> {
    let err = |e: fission_kernel::KernelError| e.to_string();
    let target = feature.target;
    match &feature.kind {
        FeatureKind::Extrude { sketch, distance, symmetric, operation } => {
            if *distance == 0.0 {
                return Err("distance must be non-zero".into());
            }
            let length = if *symmetric { distance * 2.0 } else { *distance };
            let tools = sketch
                .profiles(sketches)?
                .into_iter()
                .map(|mut profile| {
                    if *symmetric {
                        shift_profile(&mut profile, -distance);
                    }
                    kernel.extrude(&profile, length).map_err(err)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let tool = union_all(kernel, tools)?;
            combine(kernel, bodies, tool, *operation, target, index)
        }
        FeatureKind::Hole { plane, offset, center, flip, diameter, depth, through, kind, tip_angle } => {
            let depth = if *through { THROUGH_DEPTH } else { *depth };
            // No drill point on a through hole: it would only lengthen the tool.
            let tip = if *through { 0.0 } else { *tip_angle };
            let section = hole_section(*diameter, depth, *kind, tip)?;
            let p = plane.plane(*offset);
            // `up` points out of the material, against the drilling.
            let up: [f64; 3] = std::array::from_fn(|k| if *flip { -p.normal[k] } else { p.normal[k] });
            let u = p.x_dir;
            let normal = [u[1] * up[2] - u[2] * up[1], u[2] * up[0] - u[0] * up[2], u[0] * up[1] - u[1] * up[0]];
            let origin = p.to_world(*center);
            let profile = Profile::Polygon { plane: Plane { origin, normal, x_dir: u }, points: section };
            let tool = kernel.revolve(&profile, origin, up, 360.0).map_err(err)?;
            combine(kernel, bodies, tool, Operation::Cut, target, index)
        }
        FeatureKind::Revolve { sketch, axis, degrees, operation } => {
            if *degrees == 0.0 {
                return Err("angle must be non-zero".into());
            }
            let tools = sketch
                .profiles(sketches)?
                .iter()
                .map(|profile| {
                    let (origin, dir) = axis.resolve(sketches)?;
                    kernel.revolve(profile, origin, dir, *degrees).map_err(err)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let tool = union_all(kernel, tools)?;
            combine(kernel, bodies, tool, *operation, target, index)
        }
        FeatureKind::Sphere { plane, offset, center, diameter, operation } => {
            if *diameter <= 0.0 {
                return Err("the diameter must be positive".into());
            }
            let r = diameter / 2.0;
            // A half disc standing on the axis, turned all the way round.
            let half = vec![Seg2::Arc { start: [0.0, -r], mid: [r, 0.0], end: [0.0, r] }, Seg2::Line([0.0, r], [0.0, -r])];
            let tool = revolve_about_normal(kernel, plane.plane(*offset), *center, half)?;
            combine(kernel, bodies, tool, *operation, target, index)
        }
        FeatureKind::Torus { plane, offset, center, diameter, section, operation } => {
            let (big, small) = (diameter / 2.0, section / 2.0);
            if small <= 0.0 || big <= small {
                return Err("the tube must be thinner than the ring is wide".into());
            }
            let tube = vec![Seg2::Circle { center: [big, 0.0], radius: small }];
            let tool = revolve_about_normal(kernel, plane.plane(*offset), *center, tube)?;
            combine(kernel, bodies, tool, *operation, target, index)
        }
        FeatureKind::Loft { sections, operation } => {
            if sections.len() < 2 {
                return Err("pick at least two profiles to loft between".into());
            }
            let profiles = sections
                .iter()
                .map(|s| {
                    let out = sketches.get(s.sketch).and_then(Option::as_ref).ok_or("a loft profile's sketch is missing")?;
                    let region = out.region_at(s.point).ok_or("a loft profile no longer exists")?;
                    Ok(region_profile(out.plane, region))
                })
                .collect::<Result<Vec<_>, String>>()?;
            let tool = kernel.loft(&profiles).map_err(err)?;
            combine(kernel, bodies, tool, *operation, target, index)
        }
        FeatureKind::Sweep { profile, path, operation } => {
            let path = path.ok_or("pick a path: click a sketch line or arc")?;
            let out = sketches.get(path.sketch).and_then(Option::as_ref).ok_or("the path's sketch is missing")?;
            let segs = out.path(path.curve)?;
            let tools = profile
                .profiles(sketches)?
                .iter()
                .map(|p| kernel.sweep(p, &segs).map_err(err))
                .collect::<Result<Vec<_>, _>>()?;
            let tool = union_all(kernel, tools)?;
            combine(kernel, bodies, tool, *operation, target, index)
        }
        FeatureKind::Fillet { radius, edges } => {
            for_edges(bodies, edges, "fillet", |solid, sel| kernel.fillet(solid, sel, *radius).map_err(err))
        }
        FeatureKind::Chamfer { distance, edges } => {
            for_edges(bodies, edges, "chamfer", |solid, sel| kernel.chamfer(solid, sel, *distance).map_err(err))
        }
        FeatureKind::ImportStep { path, operation, hidden_parts } => {
            let parts = kernel.import_parts(std::path::Path::new(path)).map_err(err)?;
            part_counts[index] = parts.len();
            let (indices, shown): (Vec<usize>, Vec<K::Solid>) =
                parts.into_iter().enumerate().filter(|(i, _)| !hidden_parts.contains(i)).unzip();
            part_faces[index] = indices.iter().zip(&shown).map(|(&i, s)| (i, kernel.face_count(s))).collect();
            let tool = match shown.len() {
                0 => return Err("every part of the import is hidden".into()),
                1 => shown.into_iter().next().unwrap(),
                _ => kernel.compound(&shown),
            };
            combine(kernel, bodies, tool, *operation, target, index)
        }
        FeatureKind::Sketch { .. }
        | FeatureKind::Plane { .. }
        | FeatureKind::Axis { .. }
        | FeatureKind::Point { .. }
        | FeatureKind::Canvas { .. }
        | FeatureKind::Unsupported { .. } => {
            unreachable!("handled in regenerate")
        }
        FeatureKind::Thread { face, pitch, left_hand, .. } => {
            let body = find_body(bodies, face.body)?;
            body.solid = kernel.thread(&body.solid, &face.hint(), *pitch, *left_hand).map_err(err)?;
            Ok(())
        }
        FeatureKind::Coil { plane, offset, center, diameter, pitch, turns, section, left_hand, operation } => {
            let p = plane.plane(*offset);
            let tool = kernel
                .coil(p.to_world(*center), p.normal, p.x_dir, diameter / 2.0, *pitch, *turns, *left_hand, *section)
                .map_err(err)?;
            combine(kernel, bodies, tool, *operation, target, index)
        }
        FeatureKind::DeleteFace { faces } => {
            let owner = faces.first().ok_or("select faces to delete")?.body;
            let body = find_body(bodies, owner)?;
            let hints: Vec<FaceHint> = faces.iter().filter(|f| f.body == owner).map(FaceRef::hint).collect();
            body.solid = kernel.delete_faces(&body.solid, &hints).map_err(err)?;
            Ok(())
        }
        FeatureKind::OffsetFace { faces, distance } => {
            let owner = faces.first().ok_or("select faces to move")?.body;
            let body = find_body(bodies, owner)?;
            let hints: Vec<FaceHint> = faces.iter().filter(|f| f.body == owner).map(FaceRef::hint).collect();
            body.solid = kernel.offset_faces(&body.solid, &hints, *distance).map_err(err)?;
            Ok(())
        }
        FeatureKind::Draft { faces, plane, offset, degrees, flip } => {
            let owner = faces.first().ok_or("select faces to draft")?.body;
            let body = find_body(bodies, owner)?;
            let hints: Vec<FaceHint> = faces.iter().filter(|f| f.body == owner).map(FaceRef::hint).collect();
            let p = plane.plane(*offset);
            let pull = if *flip { p.normal.map(|n| -n) } else { p.normal };
            body.solid = kernel.draft(&body.solid, &hints, p.origin, pull, *degrees).map_err(err)?;
            Ok(())
        }
        FeatureKind::Shell { thickness, faces } => {
            let owner = match faces.first() {
                Some(f) => f.body,
                None => bodies.last().ok_or("no body to shell")?.created_by,
            };
            let body = find_body(bodies, owner)?;
            let hints: Vec<FaceHint> = faces.iter().filter(|f| f.body == owner).map(FaceRef::hint).collect();
            body.solid = kernel.shell(&body.solid, *thickness, &hints).map_err(err)?;
            Ok(())
        }
        FeatureKind::Combine { target, tools, operation, keep_tools } => {
            let op = operation.boolean().ok_or("Combine needs Join, Cut or Intersect")?;
            if tools.is_empty() {
                return Err("select tool bodies to combine".into());
            }
            if tools.contains(target) {
                return Err("a body can't be combined with itself".into());
            }
            let tool_solids: Vec<K::Solid> =
                tools.iter().map(|t| find_body(bodies, *t).map(|b| b.solid.clone())).collect::<Result<_, _>>()?;
            let body = find_body(bodies, *target)?;
            for tool in &tool_solids {
                body.solid = kernel.boolean(&body.solid, tool, op).map_err(err)?;
            }
            if !keep_tools {
                bodies.retain(|b| !tools.contains(&b.created_by));
            }
            Ok(())
        }
        FeatureKind::Move { bodies: which, translate, axis, degrees, copy } => {
            let owners = body_list(bodies, which)?;
            for owner in owners {
                let body = find_body(bodies, owner)?;
                let mut moved = body.solid.clone();
                if *degrees != 0.0 {
                    let (origin, dir) = axis.resolve(sketches)?;
                    moved = kernel.rotate(&moved, origin, dir, *degrees);
                }
                if *translate != [0.0; 3] {
                    moved = kernel.translate(&moved, *translate);
                }
                if *copy {
                    let name = format!("{} (copy)", body.name);
                    bodies.push(Body { name, solid: moved, created_by: index });
                } else {
                    body.solid = moved;
                }
            }
            Ok(())
        }
        FeatureKind::Joint { bodies: which, motion, axis, value, min, max } => {
            if min > max {
                return Err("the joint's lower limit is above its upper limit".into());
            }
            if value < min || value > max {
                return Err(format!("the joint's value {value} is outside its limits {min}..{max}"));
            }
            let (origin, dir) = axis.resolve(sketches)?;
            for owner in body_list(bodies, which)? {
                let body = find_body(bodies, owner)?;
                body.solid = match motion {
                    _ if *value == 0.0 => continue,
                    JointMotion::Revolute => kernel.rotate(&body.solid, origin, dir, *value),
                    JointMotion::Slider => kernel.translate(&body.solid, dir.map(|d| d * value)),
                };
            }
            Ok(())
        }
        FeatureKind::Scale { bodies: which, center, factor } => {
            if !(*factor > 0.0 && factor.is_finite()) {
                return Err("the scale factor must be positive".into());
            }
            for owner in body_list(bodies, which)? {
                let body = find_body(bodies, owner)?;
                body.solid = kernel.scale(&body.solid, *center, *factor);
            }
            Ok(())
        }
        FeatureKind::Split { body, plane, offset } => {
            let owner = match body {
                Some(b) => *b,
                None => bodies.last().ok_or("no body to split")?.created_by,
            };
            // Everything in front of the plane, as a slab far bigger than any part.
            let p = plane.plane(*offset);
            let l = THROUGH_DEPTH;
            let square = vec![[-l, -l], [l, -l], [l, l], [-l, l]];
            let front = kernel.extrude(&Profile::Polygon { plane: p, points: square }, l).map_err(err)?;
            let target = find_body(bodies, owner)?;
            let ahead = kernel.boolean(&target.solid, &front, BooleanOp::Intersect).map_err(err)?;
            let behind = kernel.boolean(&target.solid, &front, BooleanOp::Subtract).map_err(err)?;
            let tiny = 1e-9 * kernel.volume(&target.solid).abs().max(1.0);
            if kernel.volume(&ahead) <= tiny || kernel.volume(&behind) <= tiny {
                return Err("the splitting plane doesn't cut through the body".into());
            }
            target.solid = behind;
            bodies.push(Body { name: format!("Body{}", bodies.len() + 1), solid: ahead, created_by: index });
            Ok(())
        }
        FeatureKind::Mirror { bodies: which, plane, offset, join } => {
            let p = plane.plane(*offset);
            let owners = body_list(bodies, which)?;
            let mut copies = Vec::new();
            for owner in &owners {
                let body = find_body(bodies, *owner)?;
                let mirrored = kernel.mirror(&body.solid, p.origin, p.normal);
                if *join {
                    body.solid = kernel.boolean(&body.solid, &mirrored, BooleanOp::Union).map_err(err)?;
                } else {
                    copies.push(mirrored);
                }
            }
            if !copies.is_empty() {
                let solid = if copies.len() == 1 { copies.remove(0) } else { kernel.compound(&copies) };
                bodies.push(Body { name: format!("Body{}", bodies.len() + 1), solid, created_by: index });
            }
            Ok(())
        }
        FeatureKind::Pattern { bodies: which, pattern, join } => {
            let offsets = pattern_transforms(pattern, sketches)?;
            let owners = body_list(bodies, which)?;
            let mut copies = Vec::new();
            for owner in &owners {
                let body = find_body(bodies, *owner)?;
                let mut instances = Vec::new();
                for t in &offsets {
                    instances.push(apply_transform(kernel, &body.solid, t));
                }
                if *join {
                    let mut solid = body.solid.clone();
                    for inst in &instances {
                        solid = kernel.boolean(&solid, inst, BooleanOp::Union).map_err(err)?;
                    }
                    body.solid = solid;
                } else {
                    copies.extend(instances);
                }
            }
            if !copies.is_empty() {
                // Copies stay separate solids in one body: no boolean needed.
                let solid = if copies.len() == 1 { copies.remove(0) } else { kernel.compound(&copies) };
                bodies.push(Body { name: format!("Body{}", bodies.len() + 1), solid, created_by: index });
            }
            Ok(())
        }
    }
}

/// Revolve a loop drawn in the half plane standing on `plane` (x running out
/// along the plane's x, y along its normal) once round the normal through
/// `center`.
fn revolve_about_normal<K: Kernel>(kernel: &K, plane: Plane, center: [f64; 2], outer: Vec<Seg2>) -> Result<K::Solid, String> {
    let origin = plane.to_world(center);
    let (n, u) = (plane.normal, plane.x_dir);
    // x_dir × normal' = y, so normal' = u × n makes the profile's y the plane normal.
    let side = [u[1] * n[2] - u[2] * n[1], u[2] * n[0] - u[0] * n[2], u[0] * n[1] - u[1] * n[0]];
    let profile = Profile::Region { plane: Plane { origin, normal: side, x_dir: u }, outer, holes: Vec::new() };
    kernel.revolve(&profile, origin, n, 360.0).map_err(|e| e.to_string())
}

fn find_body<S>(bodies: &mut [Body<S>], owner: usize) -> Result<&mut Body<S>, String> {
    bodies.iter_mut().find(|b| b.created_by == owner).ok_or_else(|| "a referenced body no longer exists".to_string())
}

/// The bodies a feature applies to: the listed ones, or the last body.
fn body_list<S>(bodies: &[Body<S>], which: &[usize]) -> Result<Vec<usize>, String> {
    if which.is_empty() {
        Ok(vec![bodies.last().ok_or("no body to apply this to")?.created_by])
    } else {
        Ok(which.to_vec())
    }
}

/// A pattern instance placement: translate, or rotate about an origin axis.
enum Placement {
    Translate([f64; 3]),
    /// About the line through the first point along the second.
    Rotate([f64; 3], [f64; 3], f64),
}

/// Placements for every instance except the original.
fn pattern_transforms(pattern: &PatternKind, sketches: &[Option<SketchOutput>]) -> Result<Vec<Placement>, String> {
    match *pattern {
        PatternKind::Rectangular { dir, count, spacing, second } => {
            if count == 0 || second.is_some_and(|(_, c, _)| c == 0) {
                return Err("pattern count must be at least 1".into());
            }
            let (dir2, count2, spacing2) = second.unwrap_or((dir, 1, 0.0));
            let mut out = Vec::new();
            for i in 0..count {
                for j in 0..count2 {
                    if i == 0 && j == 0 {
                        continue;
                    }
                    let (a, b) = (dir.dir(), dir2.dir());
                    let t = std::array::from_fn(|k| a[k] * spacing * i as f64 + b[k] * spacing2 * j as f64);
                    out.push(Placement::Translate(t));
                }
            }
            Ok(out)
        }
        PatternKind::Circular { axis, count, degrees } => {
            if count < 2 {
                return Err("circular pattern needs at least 2 instances".into());
            }
            // A full circle spaces instances evenly without doubling the original.
            let full = (degrees.abs() - 360.0).abs() < 1e-9;
            let step = if full { degrees / count as f64 } else { degrees / (count - 1) as f64 };
            let (origin, dir) = axis.resolve(sketches)?;
            Ok((1..count).map(|i| Placement::Rotate(origin, dir, step * i as f64)).collect())
        }
    }
}

fn apply_transform<K: Kernel>(kernel: &K, solid: &K::Solid, t: &Placement) -> K::Solid {
    match *t {
        Placement::Translate(by) => kernel.translate(solid, by),
        Placement::Rotate(origin, axis, deg) => kernel.rotate(solid, origin, axis, deg),
    }
}

/// Several profiles extruded at once act as one tool.
fn union_all<K: Kernel>(kernel: &K, tools: Vec<K::Solid>) -> Result<K::Solid, String> {
    let mut tools = tools.into_iter();
    let first = tools.next().ok_or("no profile to build from")?;
    tools.try_fold(first, |acc, t| kernel.boolean(&acc, &t, BooleanOp::Union).map_err(|e| e.to_string()))
}

/// Apply an edge op per body: all edges of the last body when `edges` is
/// empty, else each referenced body with its own edges.
fn for_edges<S>(
    bodies: &mut [Body<S>],
    edges: &[EdgeRef],
    what: &str,
    mut op: impl FnMut(&S, EdgeSelection) -> Result<S, String>,
) -> Result<(), String> {
    if edges.is_empty() {
        let body = bodies.last_mut().ok_or(format!("no body to {what}"))?;
        body.solid = op(&body.solid, EdgeSelection::All)?;
        return Ok(());
    }
    let mut owners: Vec<usize> = edges.iter().map(|e| e.body).collect();
    owners.dedup();
    owners.sort_unstable();
    owners.dedup();
    for owner in owners {
        let body = bodies
            .iter_mut()
            .find(|b| b.created_by == owner)
            .ok_or("the body these edges were on no longer exists")?;
        let hints = edges.iter().filter(|e| e.body == owner).map(EdgeRef::hint).collect();
        body.solid = op(&body.solid, EdgeSelection::Near(hints))?;
    }
    Ok(())
}

fn shift_profile(profile: &mut Profile, by: f64) {
    let plane = profile.plane_mut();
    for i in 0..3 {
        plane.origin[i] += plane.normal[i] * by;
    }
}

/// Join/Cut/Intersect apply to `target` (a body's creating feature), else
/// the most recent body.
fn combine<K: Kernel>(
    kernel: &K,
    bodies: &mut Vec<Body<K::Solid>>,
    tool: K::Solid,
    operation: Operation,
    target: Option<usize>,
    index: usize,
) -> Result<(), String> {
    let body = match target {
        Some(t) => Some(bodies.iter_mut().find(|b| b.created_by == t).ok_or("the target body no longer exists")?),
        None => bodies.last_mut(),
    };
    match (operation.boolean(), body) {
        (None, _) => {
            bodies.push(Body { name: format!("Body{}", bodies.len() + 1), solid: tool, created_by: index });
            Ok(())
        }
        (Some(op), Some(target)) => {
            let result = kernel.boolean(&target.solid, &tool, op).map_err(|e| e.to_string())?;
            // As in Fusion, a join that touches nothing makes a new body.
            if operation == Operation::Join && kernel.solid_count(&result) >= kernel.solid_count(&target.solid) + kernel.solid_count(&tool) {
                bodies.push(Body { name: format!("Body{}", bodies.len() + 1), solid: tool, created_by: index });
                return Ok(());
            }
            target.solid = result;
            Ok(())
        }
        (Some(_), None) => Err(format!("{} needs an existing body", operation.label())),
    }
}

#[derive(Clone, Debug)]
pub struct Body<S> {
    pub name: String,
    pub solid: S,
    /// Index of the feature that created this body.
    pub created_by: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum FeatureStatus {
    Ok,
    Error(String),
    Suppressed,
    RolledBack,
}

pub struct Regen<S> {
    pub bodies: Vec<Body<S>>,
    pub status: Vec<FeatureStatus>,
    /// Per feature: how many parts an import brought in (0 otherwise).
    pub parts: Vec<usize>,
    /// Per feature: the shown parts of an import, in body face order, as
    /// (part index, face count), so a picked face maps back to its part.
    pub part_faces: Vec<Vec<(usize, usize)>>,
    /// Per feature: the solved sketch, for Sketch features that regenerated.
    pub sketches: Vec<Option<SketchOutput>>,
}

#[cfg(test)]
mod tests;

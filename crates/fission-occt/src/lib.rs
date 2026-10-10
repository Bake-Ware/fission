//! OpenCascade backend for [`fission_kernel::Kernel`].

mod shim;

/// Check a result with BRepCheck; heal with ShapeFix if needed; error if it
/// still isn't a valid shape. OCCT algorithms sometimes report success and
/// return broken geometry, which then fails or crashes later operations.
fn validated(op: &'static str, shape: Shape) -> Result<Shape> {
    if shim::ffi::shape_is_valid(shape.ffi()) {
        return Ok(shape);
    }
    let fixed = shim::ffi::shape_fix(shape.ffi()).map_err(|e| op_err(op, e.what().to_string()))?;
    if shim::ffi::shape_is_valid(&fixed) {
        return Ok(Shape::from_ffi(fixed));
    }
    Err(op_err(op, "the result isn't valid geometry".into()))
}

/// Build the exact geometry of a Fusion `.f3d` (all bodies, as one compound).
pub fn import_f3d(path: &Path) -> Result<Solid> {
    let mut parts = import_f3d_parts(path)?;
    Ok(match parts.len() {
        1 => parts.remove(0),
        _ => Rc::new(Shape::from(opencascade::primitives::Compound::from_shapes(parts.iter().map(|p| &**p)))),
    })
}

/// Each closed body in an .f3d as its own part (open sheets are left out
/// unless there's nothing else), turned Z-up.
pub fn import_f3d_parts(path: &Path) -> Result<Vec<Solid>> {
    let model = fission_f3d::read_f3d(path).map_err(|e| op_err("import", e.to_string()))?;
    parts_of_model(&model)
}

/// Build the bodies of a decoded B-rep blob (Fusion's Y-up turned Z-up).
pub fn parts_of_model(model: &fission_f3d::Model) -> Result<Vec<Solid>> {
    let (mut solids, mut sheets) = (Vec::new(), Vec::new());
    let (mut built, mut failed) = (0, 0);
    let trace = std::env::var_os("FISSION_F3D_TRACE").is_some();
    for (i, body) in model.bodies.iter().enumerate() {
        if trace {
            eprintln!("f3d body {i}: {} faces", body.faces.len());
        }
        let mut stats = shim::ffi::BuildStats { faces_built: 0, faces_failed: 0, solid: false };
        let shape = shim::ffi::build_asm_body(&asm_body(body), &mut stats).map_err(|e| op_err("import", e.what().to_string()))?;
        built += stats.faces_built;
        failed += stats.faces_failed;
        if stats.faces_built > 0 {
            // Topologically closed bodies are parts even if OCCT's sewing
            // didn't quite make a solid of them.
            if body.closed || stats.solid { solids.push(Shape::from_ffi(shape)) } else { sheets.push(Shape::from_ffi(shape)) }
        }
    }
    if built == 0 {
        return Err(op_err("import", format!("no faces could be built ({} unsupported kinds: {:?})", model.unsupported.len(), model.unsupported)));
    }
    if failed > 0 || trace {
        eprintln!(
            "f3d import: {built} faces built, {failed} skipped; {} closed bodies, {} sheets; unsupported: {:?}",
            solids.len(),
            sheets.len(),
            model.unsupported
        );
    }
    // Open sheets in a design's B-rep are mostly construction leftovers
    // (sketch profiles, split tools); keep them only if there's nothing else.
    let parts = if solids.is_empty() || std::env::var_os("FISSION_F3D_SHEETS").is_some() {
        solids.into_iter().chain(sheets).collect()
    } else {
        solids
    };
    // Fusion is Y-up; Fission is Z-up. Turn +90 degrees about X so parts sit
    // the way they do in Fusion.
    Ok(parts.into_iter().map(|p| Rc::new(p.rotated(DVec3::X, std::f64::consts::FRAC_PI_2))).collect())
}

fn asm_spline(b: &fission_f3d::BSpline) -> shim::ffi::AsmSpline {
    shim::ffi::AsmSpline {
        degree: b.degree as i32,
        knots: b.knots.clone(),
        mults: b.mults.iter().map(|&m| m as i32).collect(),
        poles: b.poles.iter().flatten().copied().collect(),
        weights: b.weights.clone().unwrap_or_default(),
    }
}

fn no_spline() -> shim::ffi::AsmSpline {
    shim::ffi::AsmSpline { degree: 0, knots: Vec::new(), mults: Vec::new(), poles: Vec::new(), weights: Vec::new() }
}

fn asm_body(body: &fission_f3d::Body) -> shim::ffi::AsmBody {
    use fission_f3d::{Curve, Surface};
    use shim::ffi::{AsmCoedge, AsmEdge, AsmFace, AsmLoop};
    let edges = body
        .edges
        .iter()
        .map(|e| {
            let (kind, a, b, c, ratio) = match e.curve {
                Curve::Line { origin, dir } => (0, origin, dir, [0.0; 3], 1.0),
                Curve::Ellipse { center, normal, major, ratio } => (1, center, normal, major, ratio),
                Curve::BSpline(_) => (2, [0.0; 3], [0.0; 3], [0.0; 3], 1.0),
                Curve::Unsupported(_) => (255, [0.0; 3], [0.0; 3], [0.0; 3], 1.0),
            };
            let spline = match &e.curve {
                Curve::BSpline(b) => asm_spline(b),
                _ => no_spline(),
            };
            AsmEdge { start: e.start, end: e.end, kind, spline, a, b, c, ratio, reversed: e.reversed }
        })
        .collect();
    let faces = body
        .faces
        .iter()
        .map(|f| {
            let mut out = AsmFace {
                kind: 255,
                spline: no_spline(),
                p: [0.0; 3],
                n: [0.0; 3],
                u: [0.0; 3],
                r1: 0.0,
                r2: 0.0,
                sin: 0.0,
                cos: 1.0,
                reversed: f.reversed,
                loops: f
                    .loops
                    .iter()
                    .map(|l| AsmLoop { coedges: l.iter().map(|&(edge, reversed)| AsmCoedge { edge, reversed }).collect() })
                    .collect(),
            };
            match &f.surface {
                Surface::Extrusion { profile, dir } => {
                    (out.kind, out.n) = (4, *dir);
                    out.spline = asm_spline(profile);
                }
                &Surface::Plane { origin, normal, u_dir } => {
                    (out.kind, out.p, out.n, out.u) = (0, origin, normal, u_dir);
                }
                &Surface::Cone { center, axis, major, ratio, sin_half_angle, cos_half_angle } => {
                    (out.kind, out.p, out.n, out.u, out.r1) = (1, center, axis, major, ratio);
                    (out.sin, out.cos) = (sin_half_angle, cos_half_angle);
                }
                &Surface::Sphere { center, radius, u_dir, pole } => {
                    (out.kind, out.p, out.r1, out.n, out.u) = (2, center, radius, pole, u_dir);
                }
                &Surface::Torus { center, normal, major_radius, minor_radius, u_dir } => {
                    (out.kind, out.p, out.n, out.r1, out.r2, out.u) = (3, center, normal, major_radius, minor_radius, u_dir);
                }
                Surface::Unsupported(_) => {}
            }
            out
        })
        .collect();
    let transform = body
        .transform
        .map(|t| t.rows.iter().flatten().copied().chain(t.translation).chain([t.scale]).collect())
        .unwrap_or_default();
    shim::ffi::AsmBody { vertices: body.vertices.iter().flatten().copied().collect(), edges, faces, transform }
}

/// Diagnostics for kernel-lab: shim boolean with a given fuzzy value.
pub fn shim_union(a: &Solid, b: &Solid, fuzzy: f64) -> bool {
    shim::ffi::boolean_op(a.ffi(), b.ffi(), 0, fuzzy).is_ok()
}

/// Diagnostics for kernel-lab: is this solid valid per BRepCheck, and after ShapeFix?
pub fn validity(solid: &Solid) -> (bool, bool) {
    let valid = shim::ffi::shape_is_valid(solid.ffi());
    let fixed = shim::ffi::shape_fix(solid.ffi()).map(|f| shim::ffi::shape_is_valid(&f)).unwrap_or(false);
    (valid, fixed)
}

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::rc::Rc;

use fission_kernel::{
    resolve_edges, resolve_faces, BooleanOp, EdgeHint, EdgeSelection, FaceHint, Kernel, KernelError, Mesh, Profile, Result, Seg2,
    Vec3,
};
use glam::{dvec3, DVec3};
use opencascade::angle::Angle;
use opencascade::primitives::{Edge, Face, Shape, Wire};

/// OCCT shapes aren't `Clone`; share them instead. Every op returns a new shape.
pub type Solid = Rc<Shape>;

#[derive(Default)]
pub struct Occt;

impl Occt {
    /// The solid's faces matching `faces`, as one compound for the shim.
    fn picked_faces(&self, op: &'static str, solid: &Solid, faces: &[FaceHint]) -> Result<Shape> {
        let all: Vec<Face> = solid.faces().collect();
        let candidates: Vec<FaceHint> = all.iter().map(face_hint).collect();
        let picked = resolve_faces(faces, &candidates, 0.25 * diagonal_of(solid))
            .map_err(|lost| op_err(op, format!("{lost} face reference(s) lost; reselect the faces")))?;
        let chosen: Vec<Shape> = picked.into_iter().map(|i| Shape::from(&all[i])).collect();
        Ok(Shape::from(opencascade::primitives::Compound::from_shapes(chosen.iter())))
    }
}

impl Kernel for Occt {
    type Solid = Solid;

    fn extrude(&self, profile: &Profile, distance: f64) -> Result<Solid> {
        if !distance.is_finite() || distance == 0.0 {
            return Err(op_err("extrude", "distance must be non-zero".into()));
        }
        guarded_result("extrude", || {
            let n = v(profile.plane().normal);
            Ok(Shape::from(profile_face(profile)?.extrude(n * distance)))
        })
    }

    fn revolve(&self, profile: &Profile, axis_origin: Vec3, axis_dir: Vec3, degrees: f64) -> Result<Solid> {
        if !degrees.is_finite() || degrees == 0.0 {
            return Err(op_err("revolve", "angle must be non-zero".into()));
        }
        guarded_result("revolve", || {
            let angle = (degrees.abs() < 360.0).then_some(Angle::Degrees(degrees));
            Ok(Shape::from(profile_face(profile)?.revolve(v(axis_origin), v(axis_dir), angle)))
        })
    }

    fn boolean(&self, a: &Solid, b: &Solid, op: BooleanOp) -> Result<Solid> {
        guarded_result("boolean", || {
            match op {
                BooleanOp::Union => a.try_union(b),
                BooleanOp::Subtract => a.try_subtract(b),
                BooleanOp::Intersect => a.try_intersect(b),
            }
            // Merge coplanar faces left by the boolean (like Fusion does), so
            // later fillets don't hit seam edges between them.
            .map(|shape| shape.clean())
            .map_err(|e| op_err("boolean", e.to_string()))
            .and_then(|shape| validated("boolean", shape))
        })
    }

    fn fillet(&self, solid: &Solid, edges: EdgeSelection, radius: f64) -> Result<Solid> {
        if !positive(radius) {
            return Err(op_err("fillet", "radius must be positive".into()));
        }
        let chosen = select_edges("fillet", solid, &edges)?;
        guarded_result("fillet", || {
            solid
                .try_fillet_edges(radius, chosen.iter())
                .map_err(|_| op_err("fillet", format!("radius {radius} mm is too large for these edges")))
                .and_then(|shape| validated("fillet", shape))
        })
    }

    fn chamfer(&self, solid: &Solid, edges: EdgeSelection, distance: f64) -> Result<Solid> {
        if !positive(distance) {
            return Err(op_err("chamfer", "distance must be positive".into()));
        }
        let chosen = select_edges("chamfer", solid, &edges)?;
        guarded_result("chamfer", || {
            solid
                .try_chamfer_edges(distance, chosen.iter())
                .map_err(|_| op_err("chamfer", format!("distance {distance} mm is too large for these edges")))
                .and_then(|shape| validated("chamfer", shape))
        })
    }

    fn shell(&self, solid: &Solid, thickness: f64, open: &[FaceHint]) -> Result<Solid> {
        if !positive(thickness) {
            return Err(op_err("shell", "thickness must be positive".into()));
        }
        let faces: Vec<Face> = solid.faces().collect();
        let candidates: Vec<FaceHint> = faces.iter().map(face_hint).collect();
        let max_drift = 0.25 * diagonal_of(solid);
        let picked = resolve_faces(open, &candidates, max_drift)
            .map_err(|lost| op_err("shell", format!("{lost} face reference(s) lost; reselect the faces")))?;
        let mut faces: Vec<Option<Face>> = faces.into_iter().map(Some).collect();
        let chosen: Vec<Face> = picked.into_iter().filter_map(|i| faces[i].take()).collect();
        // Shell inward, like Fusion's default. OCCT can report success on an
        // impossible wall and return junk, so check the result removed some
        // material but not all of it.
        let too_thick = || op_err("shell", format!("can't shell to {thickness} mm; try a thinner wall"));
        let before = solid.volume();
        guarded_result("shell", || {
            let shelled = solid.try_hollow(-thickness, chosen.iter()).map_err(|_| too_thick())?;
            let after = shelled.volume();
            if !(after.is_finite() && after > 1e-9 && after < before) {
                return Err(too_thick());
            }
            validated("shell", shelled).map_err(|_| {
                op_err("shell", "OpenCascade couldn't build a valid shell here; try other faces or a thinner wall".into())
            })
        })
    }

    fn draft(&self, solid: &Solid, faces: &[FaceHint], origin: Vec3, pull: Vec3, degrees: f64) -> Result<Solid> {
        if !degrees.is_finite() || degrees.abs() >= 89.0 {
            return Err(op_err("draft", "the angle must be less than 89°".into()));
        }
        let all: Vec<Face> = solid.faces().collect();
        let candidates: Vec<FaceHint> = all.iter().map(face_hint).collect();
        let picked = resolve_faces(faces, &candidates, 0.25 * diagonal_of(solid))
            .map_err(|lost| op_err("draft", format!("{lost} face reference(s) lost; reselect the faces")))?;
        let chosen: Vec<Shape> = picked.into_iter().map(|i| Shape::from(&all[i])).collect();
        let compound = Shape::from(opencascade::primitives::Compound::from_shapes(chosen.iter()));
        // OCCT's positive angle already leans faces inward along the pull,
        // which is Fusion's positive draft.
        let shape = shim::ffi::draft_shape(
            solid.ffi(),
            compound.ffi(),
            pull[0],
            pull[1],
            pull[2],
            origin[0],
            origin[1],
            origin[2],
            degrees.to_radians(),
        )
        .map_err(|e| op_err("draft", e.what().to_string()))?;
        validated("draft", Shape::from_ffi(shape)).map(Rc::new)
    }

    fn thread(&self, solid: &Solid, face: &FaceHint, pitch: f64, left_hand: bool) -> Result<Solid> {
        let chosen = self.picked_faces("thread", solid, std::slice::from_ref(face))?;
        let shape = shim::ffi::thread_shape(solid.ffi(), chosen.ffi(), pitch, left_hand).map_err(|e| op_err("thread", e.what().to_string()))?;
        validated("thread", Shape::from_ffi(shape)).map(Rc::new)
    }

    fn coil(&self, o: Vec3, d: Vec3, x: Vec3, radius: f64, pitch: f64, turns: f64, left_hand: bool, section: f64) -> Result<Solid> {
        let shape = shim::ffi::coil_shape(o[0], o[1], o[2], d[0], d[1], d[2], x[0], x[1], x[2], radius, pitch, turns, left_hand, section)
            .map_err(|e| op_err("coil", e.what().to_string()))?;
        validated("coil", Shape::from_ffi(shape)).map(Rc::new)
    }

    fn delete_faces(&self, solid: &Solid, faces: &[FaceHint]) -> Result<Solid> {
        let chosen = self.picked_faces("delete face", solid, faces)?;
        let shape = shim::ffi::remove_faces_shape(solid.ffi(), chosen.ffi()).map_err(|e| op_err("delete face", e.what().to_string()))?;
        validated("delete face", Shape::from_ffi(shape)).map(Rc::new)
    }

    fn offset_faces(&self, solid: &Solid, faces: &[FaceHint], distance: f64) -> Result<Solid> {
        if !distance.is_finite() {
            return Err(op_err("offset face", "the distance isn't a number".into()));
        }
        if distance == 0.0 {
            return Ok(solid.clone());
        }
        let compound = self.picked_faces("offset face", solid, faces)?;
        let shape = shim::ffi::offset_faces_shape(solid.ffi(), compound.ffi(), distance)
            .map_err(|e| op_err("offset face", e.what().to_string()))?;
        validated("offset face", Shape::from_ffi(shape).clean()).map(Rc::new)
    }

    fn compound(&self, solids: &[Solid]) -> Solid {
        Rc::new(Shape::from(opencascade::primitives::Compound::from_shapes(solids.iter().map(|s| s.as_ref()))))
    }

    fn translate(&self, solid: &Solid, by: Vec3) -> Solid {
        Rc::new(solid.translated(v(by)))
    }

    fn rotate(&self, solid: &Solid, origin: Vec3, axis: Vec3, degrees: f64) -> Solid {
        // `rotated` turns about an axis through the world origin; shift around it.
        let o = v(origin);
        Rc::new(solid.translated(-o).rotated(v(axis), degrees.to_radians()).translated(o))
    }

    fn scale(&self, solid: &Solid, center: Vec3, factor: f64) -> Solid {
        Rc::new(solid.scaled(v(center), factor))
    }

    fn mirror(&self, solid: &Solid, origin: Vec3, normal: Vec3) -> Solid {
        // Plane reflection = half-turn about the normal, then point reflection
        // (scale -1), both through `origin`.
        let o = v(origin);
        let at_origin = solid.translated(-o);
        Rc::new(at_origin.mirrored(DVec3::ZERO, v(normal)).scaled(DVec3::ZERO, -1.0).translated(o))
    }

    fn tessellate(&self, solid: &Solid, tolerance: f64) -> Result<Mesh> {
        let mut mesh = Mesh::default();
        // Mesh the whole solid first so neighbouring faces share their edge
        // points (meshing faces alone leaves cracks along curved seams, e.g.
        // threads), then read it back face by face so each triangle keeps
        // its B-rep face id for picking.
        let _ = opencascade::mesh::Mesher::try_new(solid, tolerance);
        let mut skipped = 0;
        for (face_id, face) in solid.faces().enumerate() {
            // Skip a face that won't triangulate instead of losing the body.
            let Ok(part) = Shape::from(&face).mesh_with_tolerance(tolerance) else {
                skipped += 1;
                mesh.face_hints.push(FaceHint { center: [f64::NAN; 3], normal: [0.0; 3] });
                continue;
            };
            let base = mesh.positions.len() as u32;
            mesh.positions.extend(part.vertices.iter().map(|p| p.as_vec3().to_array()));
            mesh.normals.extend(part.normals.iter().map(|n| n.as_vec3().to_array()));
            mesh.indices.extend(part.indices.iter().map(|&i| base + i as u32));
            mesh.face_ids.extend(std::iter::repeat_n(face_id as u32, part.indices.len() / 3));
            mesh.face_hints.push(face_hint(&face));
        }
        mesh.edges = unique_edges(solid).into_iter().map(|(_, polyline)| polyline).collect();
        mesh.skipped_faces = skipped;
        mesh.face_areas = solid.faces().map(|f| f.surface_area()).collect();
        mesh.volume = solid.volume();
        Ok(mesh)
    }

    fn volume(&self, solid: &Solid) -> f64 {
        solid.volume()
    }

    fn face_count(&self, solid: &Solid) -> usize {
        solid.faces().count()
    }

    fn solid_count(&self, solid: &Solid) -> usize {
        shim::ffi::solid_count(solid.ffi())
    }

    fn export_step(&self, solids: &[Solid], path: &Path) -> Result<()> {
        Shape::write_all_step(solids.iter().map(|s| s.as_ref()), path)
            .map_err(|e| op_err("export_step", format!("{e:?}")))
    }

    fn import_step(&self, path: &Path) -> Result<Solid> {
        // Fusion archives go through our own reader; everything else is STEP.
        if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("f3d")) {
            return import_f3d(path);
        }
        Shape::read_step(path).map(Rc::new).map_err(|e| op_err("import_step", format!("{e:?}")))
    }

    fn loft(&self, sections: &[Profile]) -> Result<Solid> {
        let wires: Vec<Shape> = sections.iter().map(|p| profile_outer_wire(p).map(Shape::from)).collect::<Result<_>>()?;
        let compound = Shape::from(opencascade::primitives::Compound::from_shapes(wires.iter()));
        let shape = shim::ffi::loft_shape(compound.ffi(), false).map_err(|e| op_err("loft", e.what().to_string()))?;
        validated("loft", Shape::from_ffi(shape)).map(Rc::new)
    }

    fn sweep(&self, profile: &Profile, path: &[fission_kernel::Seg3]) -> Result<Solid> {
        // Outer boundary first, then the holes, as plain wires.
        let mut wires = vec![Shape::from(profile_outer_wire(profile)?)];
        if let Profile::Region { plane, holes, .. } = profile {
            for h in holes {
                wires.push(Shape::from(loop_wire(plane, h)?));
            }
        }
        let wires = Shape::from(opencascade::primitives::Compound::from_shapes(wires.iter()));
        let spine = Shape::from(path_wire(path)?);
        let shape = shim::ffi::pipe_shape(wires.ffi(), spine.ffi()).map_err(|e| op_err("sweep", e.what().to_string()))?;
        validated("sweep", Shape::from_ffi(shape)).map(Rc::new)
    }

    fn import_parts(&self, path: &Path) -> Result<Vec<Solid>> {
        if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("f3d")) {
            return import_f3d_parts(path);
        }
        // A STEP assembly: one part per solid.
        let whole = self.import_step(path)?;
        let n = shim::ffi::solid_count(whole.ffi());
        let solids: Vec<Solid> = (0..n)
            .filter_map(|i| shim::ffi::solid_at(whole.ffi(), i).ok())
            .map(|s| Rc::new(Shape::from_ffi(s)))
            .collect();
        Ok(if solids.len() > 1 { solids } else { vec![whole] })
    }
}

/// Each B-rep edge once, with its display polyline. OCCT's explorer visits a
/// shared edge once per adjacent face, so dedupe by midpoint.
fn unique_edges(solid: &Shape) -> Vec<(Edge, Vec<[f32; 3]>)> {
    let mut out: Vec<(Edge, Vec<[f32; 3]>, EdgeHint)> = Vec::new();
    for edge in solid.edges() {
        let polyline: Vec<[f32; 3]> = edge.approximation_segments().map(|p| p.as_vec3().to_array()).collect();
        let Some(hint) = EdgeHint::from_polyline(&polyline) else { continue };
        if out.iter().any(|(_, _, h)| fission_kernel::dist(h.mid, hint.mid) < 1e-6) {
            continue;
        }
        out.push((edge, polyline, hint));
    }
    out.into_iter().map(|(e, p, _)| (e, p)).collect()
}

fn select_edges(op: &'static str, solid: &Shape, selection: &EdgeSelection) -> Result<Vec<Edge>> {
    let edges = unique_edges(solid);
    match selection {
        EdgeSelection::All => Ok(edges.into_iter().map(|(e, _)| e).collect()),
        EdgeSelection::Near(hints) => {
            let candidates: Vec<EdgeHint> = edges.iter().filter_map(|(_, p)| EdgeHint::from_polyline(p)).collect();
            // Let references follow edges up to a quarter of the part's size.
            let max_drift = 0.25 * diagonal(edges.iter().flat_map(|(_, p)| p.iter()));
            let picked = resolve_edges(hints, &candidates, max_drift)
                .map_err(|lost| op_err(op, format!("{lost} edge reference(s) lost; reselect the edges")))?;
            let mut edges: Vec<Option<Edge>> = edges.into_iter().map(|(e, _)| Some(e)).collect();
            Ok(picked.into_iter().filter_map(|i| edges[i].take()).collect())
        }
    }
}

/// Mesh tolerance for face signatures. Fixed (not the display tolerance) so
/// picking and resolution always compute identical hints.
const FACE_HINT_TOLERANCE: f64 = 0.05;

/// Area-weighted centroid and outward normal of a face's triangulation.
/// Computed from the mesh because projecting the center of mass onto a
/// curved surface (Face::normal_at) can fail and throw.
fn face_hint(face: &Face) -> FaceHint {
    let Ok(mesh) = Shape::from(face).mesh_with_tolerance(FACE_HINT_TOLERANCE) else {
        return FaceHint { center: [0.0; 3], normal: [0.0; 3] };
    };
    let (mut area, mut center, mut normal) = (0.0, DVec3::ZERO, DVec3::ZERO);
    for tri in mesh.indices.chunks_exact(3) {
        let [a, b, c] = [0, 1, 2].map(|i| mesh.vertices[tri[i]]);
        let cross = (b - a).cross(c - a);
        let w = cross.length() / 2.0;
        area += w;
        center += (a + b + c) / 3.0 * w;
        normal += cross;
    }
    if area > 0.0 {
        center /= area;
    }
    FaceHint { center: center.to_array(), normal: normal.normalize_or_zero().to_array() }
}

fn diagonal_of(solid: &Shape) -> f64 {
    let faces: Vec<FaceHint> = solid.faces().map(|f| face_hint(&f)).collect();
    let pts: Vec<[f32; 3]> = faces.iter().map(|h| h.center.map(|x| x as f32)).collect();
    diagonal(pts.iter())
}

fn diagonal<'a>(points: impl Iterator<Item = &'a [f32; 3]>) -> f64 {
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for p in points {
        for k in 0..3 {
            min[k] = min[k].min(p[k] as f64);
            max[k] = max[k].max(p[k] as f64);
        }
    }
    if min[0].is_finite() { fission_kernel::dist(min, max) } else { 0.0 }
}

/// Validate before building: OCCT throws (and so aborts) on degenerate input.
/// A profile's outer boundary as a wire (for lofts).
fn profile_outer_wire(profile: &Profile) -> Result<Wire> {
    match profile {
        Profile::Polygon { plane, points } => {
            if points.len() < 3 {
                return Err(op_err("profile", "profile has no area".into()));
            }
            Wire::from_ordered_points(points.iter().map(|&p| v(plane.to_world(p)))).map_err(|e| op_err("profile", format!("{e:?}")))
        }
        Profile::Circle { plane, center, radius } => {
            if !positive(*radius) {
                return Err(op_err("profile", "circle radius must be positive".into()));
            }
            Ok(Wire::from_edges([&Edge::circle(v(plane.to_world(*center)), v(plane.normal), *radius)]))
        }
        Profile::Region { plane, outer, .. } => loop_wire(plane, outer),
    }
}

/// A connected path as a wire.
fn path_wire(path: &[fission_kernel::Seg3]) -> Result<Wire> {
    use fission_kernel::Seg3;
    if path.is_empty() {
        return Err(op_err("sweep", "the path is empty".into()));
    }
    let edges: Vec<Edge> = path
        .iter()
        .map(|s| match *s {
            Seg3::Line(a, b) => {
                if fission_kernel::dist(a, b) < 1e-9 {
                    return Err(op_err("sweep", "zero-length line in the path".into()));
                }
                Ok(Edge::segment(v(a), v(b)))
            }
            Seg3::Arc { start, mid, end } => Ok(Edge::arc(v(start), v(mid), v(end))),
        })
        .collect::<Result<_>>()?;
    Ok(Wire::from_edges(edges.iter()))
}

fn profile_face(profile: &Profile) -> Result<Face> {
    let wire = match profile {
        Profile::Polygon { plane, points } => {
            if points.len() < 3 || polygon_area(points).abs() < 1e-9 {
                return Err(op_err("profile", "profile has no area".into()));
            }
            // from_ordered_points closes the loop itself; repeating the first
            // point would add a zero-length edge, which OCCT throws on.
            let world: Vec<DVec3> = points.iter().map(|&p| v(plane.to_world(p))).collect();
            Wire::from_ordered_points(world).map_err(|e| op_err("profile", format!("{e:?}")))?
        }
        Profile::Circle { plane, center, radius } => {
            if !positive(*radius) {
                return Err(op_err("profile", "circle radius must be positive".into()));
            }
            let edge = Edge::circle(v(plane.to_world(*center)), v(plane.normal), *radius);
            Wire::from_edges([&edge])
        }
        Profile::Region { plane, outer, holes } => {
            if outer.is_empty() {
                return Err(op_err("profile", "region has no boundary".into()));
            }
            let outer = loop_wire(plane, outer)?;
            // MakeFace wants holes wound opposite to the outer boundary.
            let holes: Vec<Wire> = holes
                .iter()
                .map(|h| {
                    let reversed: Vec<Seg2> = h.iter().rev().map(Seg2::reversed).collect();
                    loop_wire(plane, &reversed)
                })
                .collect::<Result<_>>()?;
            return Ok(Face::from_wire_with_holes(&outer, &holes));
        }
    };
    Ok(Face::from_wire(&wire))
}

fn loop_wire(plane: &fission_kernel::Plane, segments: &[Seg2]) -> Result<Wire> {
    let w = |p: [f64; 2]| v(plane.to_world(p));
    let mut edges = Vec::with_capacity(segments.len());
    for seg in segments {
        match *seg {
            Seg2::Line(a, b) => {
                if fission_kernel::dist(plane.to_world(a), plane.to_world(b)) < 1e-9 {
                    return Err(op_err("profile", "zero-length line in profile".into()));
                }
                edges.push(Edge::segment(w(a), w(b)));
            }
            Seg2::Arc { start, mid, end } => {
                // Collinear points make GC_MakeArcOfCircle fail (and throw).
                let area2 = (mid[0] - start[0]) * (end[1] - start[1]) - (end[0] - start[0]) * (mid[1] - start[1]);
                if area2.abs() < 1e-12 {
                    return Err(op_err("profile", "degenerate arc in profile".into()));
                }
                edges.push(Edge::arc(w(start), w(mid), w(end)));
            }
            Seg2::Circle { center, radius } => {
                if !positive(radius) {
                    return Err(op_err("profile", "circle radius must be positive".into()));
                }
                edges.push(Edge::circle(w(center), v(plane.normal), radius));
            }
        }
    }
    Ok(Wire::from_edges(edges.iter()))
}

fn polygon_area(points: &[[f64; 2]]) -> f64 {
    let n = points.len();
    (0..n).map(|i| points[i][0] * points[(i + 1) % n][1] - points[(i + 1) % n][0] * points[i][1]).sum::<f64>() / 2.0
}

/// C++ exceptions can't be caught here (they abort across cxx), which is why
/// ops validate input and check IsDone. This only catches Rust panics from the
/// binding layer.
fn guarded_result(op: &'static str, f: impl FnOnce() -> Result<Shape>) -> Result<Solid> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(shape)) => Ok(Rc::new(shape)),
        Ok(Err(e)) => Err(e),
        Err(panic) => {
            let reason = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "kernel panic".into());
            Err(op_err(op, reason))
        }
    }
}

fn op_err(op: &'static str, reason: String) -> KernelError {
    KernelError::Operation { op, reason }
}

fn v(a: Vec3) -> DVec3 {
    dvec3(a[0], a[1], a[2])
}

/// Finite and greater than zero (so NaN is rejected too).
fn positive(x: f64) -> bool {
    x.is_finite() && x > 0.0
}

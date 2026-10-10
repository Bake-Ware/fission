//! Fission's own OCCT calls. Unlike the opencascade-sys bindings these return
//! `Result`: the C++ side catches OCCT exceptions and rethrows them as
//! std::exception, which cxx turns into `Err` instead of aborting.

#[cxx::bridge(namespace = "fission")]
pub mod ffi {
    /// One imported B-rep body, flattened for C++ (see fission-f3d's Model).
    struct AsmBody {
        /// x, y, z per vertex.
        vertices: Vec<f64>,
        edges: Vec<AsmEdge>,
        faces: Vec<AsmFace>,
        /// Row-major 3x3 then translation then scale; empty for identity.
        transform: Vec<f64>,
    }

    /// A B-spline curve: clamped knots with multiplicities, xyz poles,
    /// weights (empty when non-rational).
    struct AsmSpline {
        degree: i32,
        knots: Vec<f64>,
        mults: Vec<i32>,
        poles: Vec<f64>,
        weights: Vec<f64>,
    }

    struct AsmEdge {
        start: usize,
        end: usize,
        /// 0 line (a = origin, b = direction), 1 ellipse (a = center,
        /// b = normal, c = major axis vector, ratio), 2 B-spline (spline),
        /// 255 unsupported.
        kind: u8,
        spline: AsmSpline,
        a: [f64; 3],
        b: [f64; 3],
        c: [f64; 3],
        ratio: f64,
        reversed: bool,
    }

    struct AsmCoedge {
        edge: usize,
        reversed: bool,
    }

    struct AsmLoop {
        coedges: Vec<AsmCoedge>,
    }

    struct AsmFace {
        /// 0 plane (p, n, u), 1 cone (p = center, n = axis, u = major
        /// vector, r1 = ratio, sin, cos), 2 sphere (p, r1 = radius, n = pole,
        /// u), 3 torus (p, n, r1 = major, r2 = minor, u), 4 extrusion of
        /// `spline` along n, 255 unsupported.
        kind: u8,
        spline: AsmSpline,
        p: [f64; 3],
        n: [f64; 3],
        u: [f64; 3],
        r1: f64,
        r2: f64,
        sin: f64,
        cos: f64,
        reversed: bool,
        loops: Vec<AsmLoop>,
    }

    struct BuildStats {
        faces_built: u32,
        faces_failed: u32,
        solid: bool,
    }

    unsafe extern "C++" {
        include!("fission-occt/src/shim.hxx");

        #[namespace = ""]
        type TopoDS_Shape = opencascade::opencascade_sys::topo_ds::TopoDS_Shape;

        /// BRepCheck_Analyzer: is the topology and geometry consistent?
        fn shape_is_valid(shape: &TopoDS_Shape) -> bool;

        /// ShapeFix_Shape healing pass.
        fn shape_fix(shape: &TopoDS_Shape) -> Result<UniquePtr<TopoDS_Shape>>;

        /// Boolean (0 fuse, 1 cut, 2 common) with IsDone/HasErrors checks.
        /// `fuzzy` > 0 sets OCCT's fuzzy tolerance (for near-coincident geometry).
        fn boolean_op(a: &TopoDS_Shape, b: &TopoDS_Shape, op: i32, fuzzy: f64) -> Result<UniquePtr<TopoDS_Shape>>;

        /// Build a body from imported topology: shared vertices and edges,
        /// faces on analytic surfaces, healed and sewn; a solid if it closes.
        fn build_asm_body(body: &AsmBody, stats: &mut BuildStats) -> Result<UniquePtr<TopoDS_Shape>>;

        /// Tilt `faces` (a compound of the solid's own faces) by `radians`
        /// against the pull direction (dx, dy, dz), hinged where they cross
        /// the neutral plane through (px, py, pz) square to it.
        #[allow(clippy::too_many_arguments)]
        fn draft_shape(
            solid: &TopoDS_Shape,
            faces: &TopoDS_Shape,
            dx: f64,
            dy: f64,
            dz: f64,
            px: f64,
            py: f64,
            pz: f64,
            radians: f64,
        ) -> Result<UniquePtr<TopoDS_Shape>>;

        /// Move `faces` (a compound of the solid's own faces) `distance`
        /// along their outward normals; the faces around them follow.
        fn offset_faces_shape(solid: &TopoDS_Shape, faces: &TopoDS_Shape, distance: f64) -> Result<UniquePtr<TopoDS_Shape>>;

        /// Remove `faces` (a compound of the solid's own faces) and heal: the
        /// faces around them extend to close the gap.
        fn remove_faces_shape(solid: &TopoDS_Shape, faces: &TopoDS_Shape) -> Result<UniquePtr<TopoDS_Shape>>;

        /// A circular section of diameter `section` swept along a helix of
        /// `radius` around the axis through (ox, oy, oz) along (dx, dy, dz),
        /// starting toward (xx, xy, xz), for `turns` turns of `pitch`.
        #[allow(clippy::too_many_arguments)]
        fn coil_shape(
            ox: f64,
            oy: f64,
            oz: f64,
            dx: f64,
            dy: f64,
            dz: f64,
            xx: f64,
            xy: f64,
            xz: f64,
            radius: f64,
            pitch: f64,
            turns: f64,
            left: bool,
            section: f64,
        ) -> Result<UniquePtr<TopoDS_Shape>>;

        /// Cut an ISO metric thread of `pitch` into the cylindrical face in
        /// `faces` (outside for a shaft, inside for a hole), along its length.
        fn thread_shape(solid: &TopoDS_Shape, faces: &TopoDS_Shape, pitch: f64, left: bool) -> Result<UniquePtr<TopoDS_Shape>>;

        /// ThruSections through the wires of `sections` (a compound, in order).
        fn loft_shape(sections: &TopoDS_Shape, ruled: bool) -> Result<UniquePtr<TopoDS_Shape>>;

        /// Sweep a profile along a wire: `profile` is a compound of wires,
        /// the outer boundary first, then holes. The profile is kept square
        /// to the path (corrected Frenet frame) and mitred at sharp corners.
        fn pipe_shape(profile: &TopoDS_Shape, spine: &TopoDS_Shape) -> Result<UniquePtr<TopoDS_Shape>>;

        /// Solids inside a shape (an assembly's parts), counted and fetched.
        fn solid_count(shape: &TopoDS_Shape) -> usize;
        fn solid_at(shape: &TopoDS_Shape, index: usize) -> Result<UniquePtr<TopoDS_Shape>>;
    }
}

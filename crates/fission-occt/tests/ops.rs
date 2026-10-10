use fission_kernel::{BooleanOp, EdgeSelection, Kernel, Plane, Profile};
use fission_occt::Occt;

fn plate(k: &Occt) -> fission_occt::Solid {
    let profile = Profile::rect(Plane::XY, [-20.0, -15.0], [20.0, 15.0]);
    k.extrude(&profile, 10.0).unwrap()
}

#[test]
fn extruded_rect_is_a_box() {
    let k = Occt;
    assert_eq!(k.face_count(&plate(&k)), 6);
}

#[test]
fn hole_then_fillet() {
    let k = Occt;
    let hole = Profile::Circle { plane: Plane::XY, center: [0.0, 0.0], radius: 5.0 };
    let hole = k.extrude(&hole, 10.0).unwrap();
    let part = k.boolean(&plate(&k), &hole, BooleanOp::Subtract).unwrap();
    // Box faces + cylindrical hole wall.
    assert_eq!(k.face_count(&part), 7);

    let filleted = k.fillet(&part, EdgeSelection::All, 1.0).unwrap();
    assert!(k.face_count(&filleted) > 7);
}

#[test]
fn tessellation_tags_every_triangle_with_a_face() {
    let k = Occt;
    let solid = plate(&k);
    let mesh = k.tessellate(&solid, 0.1).unwrap();
    assert_eq!(mesh.face_ids.len(), mesh.triangle_count());
    let faces: std::collections::BTreeSet<_> = mesh.face_ids.iter().copied().collect();
    assert_eq!(faces.len(), 6);
    assert!(mesh.indices.iter().all(|&i| (i as usize) < mesh.positions.len()));
}

#[test]
fn oversized_fillet_is_an_error_not_a_crash() {
    let k = Occt;
    assert!(k.fillet(&plate(&k), EdgeSelection::All, 50.0).is_err());
}

#[test]
fn step_round_trip_and_stl() {
    let k = Occt;
    let dir = std::env::temp_dir().join("fission-occt-test");
    std::fs::create_dir_all(&dir).unwrap();

    let step = dir.join("plate.step");
    k.export_step(&[plate(&k)], &step).unwrap();
    assert_eq!(k.face_count(&k.import_step(&step).unwrap()), 6);

    let mesh = k.tessellate(&plate(&k), 0.1).unwrap();
    let mut stl = Vec::new();
    mesh.write_stl(&mut stl).unwrap();
    assert_eq!(stl.len(), 84 + 50 * mesh.triangle_count());
}

#[test]
fn flush_join_merges_faces_so_fillet_works() {
    let k = Occt;
    let base = k.extrude(&Profile::rect(Plane::XY, [-30.0, -20.0], [30.0, 20.0]), 12.0).unwrap();
    let mut top_plane = Plane::XY;
    top_plane.origin[2] = 12.0;
    let tower = k.extrude(&Profile::rect(top_plane, [-30.0, -20.0], [-10.0, 20.0]), 25.0).unwrap();
    let joined = k.boolean(&base, &tower, BooleanOp::Union).unwrap();
    // An L-shaped prism: 8 faces once coplanar sides are merged.
    assert_eq!(k.face_count(&joined), 8);
    assert!(k.fillet(&joined, EdgeSelection::All, 2.0).is_ok());
}

#[test]
fn sample_document_regenerates_cleanly() {
    use fission_model::{Document, FeatureStatus};
    let k = Occt;
    let regen = Document::sample().regenerate(&k);
    assert!(regen.status.iter().all(|s| *s == FeatureStatus::Ok), "{:?}", regen.status);
    assert_eq!(regen.bodies.len(), 1);
    let mesh = k.tessellate(&regen.bodies[0].solid, 0.05).unwrap();
    let z_max = mesh.positions.iter().map(|p| p[2]).fold(f32::MIN, f32::max);
    assert!((z_max - 37.0).abs() < 1e-3, "top of the tower should be at 12 + 25 mm, got {z_max}");
}

#[test]
fn fillet_one_picked_edge_and_survive_resize() {
    use fission_kernel::EdgeHint;
    let k = Occt;
    let solid = plate(&k); // 40 x 30 x 10, z from 0 to 10
    let mesh = k.tessellate(&solid, 0.1).unwrap();
    assert_eq!(mesh.edges.len(), 12, "a box has 12 distinct edges");

    // Pick the top edge running along X at y = +15.
    let top_edge = mesh
        .edges
        .iter()
        .filter_map(|p| EdgeHint::from_polyline(p))
        .find(|h| (h.mid[1] - 15.0).abs() < 1e-6 && (h.mid[2] - 10.0).abs() < 1e-6)
        .expect("top +Y edge");
    let one = k.fillet(&solid, EdgeSelection::Near(vec![top_edge]), 2.0).unwrap();
    // One rounded edge adds exactly one face.
    assert_eq!(k.face_count(&one), 7);

    // Same reference against a taller plate (edge moved from z=10 to z=14).
    let taller = k.extrude(&Profile::rect(Plane::XY, [-20.0, -15.0], [20.0, 15.0]), 14.0).unwrap();
    assert_eq!(k.face_count(&k.fillet(&taller, EdgeSelection::Near(vec![top_edge]), 2.0).unwrap()), 7);

    // A reference to geometry that's gone errors instead of guessing.
    let far = EdgeHint { mid: [500.0, 0.0, 0.0], dir: [0.0, 0.0, 1.0] };
    assert!(k.fillet(&solid, EdgeSelection::Near(vec![far]), 2.0).is_err());
}

#[test]
fn region_with_arcs_and_a_hole_extrudes() {
    use fission_kernel::Seg2;
    let k = Occt;
    // A 20 x 10 slot with round ends and a round hole in the middle.
    let outer = vec![
        Seg2::Line([0.0, 0.0], [20.0, 0.0]),
        Seg2::Arc { start: [20.0, 0.0], mid: [25.0, 5.0], end: [20.0, 10.0] },
        Seg2::Line([20.0, 10.0], [0.0, 10.0]),
        Seg2::Arc { start: [0.0, 10.0], mid: [-5.0, 5.0], end: [0.0, 0.0] },
    ];
    let hole = vec![Seg2::Circle { center: [10.0, 5.0], radius: 2.0 }];
    let profile = Profile::Region { plane: Plane::XY, outer, holes: vec![hole] };
    let solid = k.extrude(&profile, 3.0).unwrap();
    // Top + bottom + 2 flat sides + 2 round ends + hole wall.
    assert_eq!(k.face_count(&solid), 7);
    let mesh = k.tessellate(&solid, 0.05).unwrap();
    let x_max = mesh.positions.iter().map(|p| p[0]).fold(f32::MIN, f32::max);
    assert!((x_max - 25.0).abs() < 0.05, "round end reaches x = 25, got {x_max}");
}

fn bounds(k: &Occt, s: &fission_occt::Solid) -> ([f32; 3], [f32; 3]) {
    let m = k.tessellate(s, 0.1).unwrap();
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for p in &m.positions {
        for i in 0..3 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    (lo, hi)
}

#[test]
fn shell_open_top_makes_a_tray() {
    let k = Occt;
    let solid = plate(&k);
    let mesh = k.tessellate(&solid, 0.1).unwrap();
    // The top face (normal +Z at z = 10).
    let top = *mesh.face_hints.iter().find(|h| h.normal[2] > 0.99 && (h.center[2] - 10.0).abs() < 1e-6).unwrap();
    let tray = k.shell(&solid, 2.0, &[top]).unwrap();
    // 5 outer + 5 inner walls, plus the rim around the opening.
    assert_eq!(k.face_count(&tray), 11);
    // Outer size unchanged: shelling goes inward.
    let (lo, hi) = bounds(&k, &tray);
    assert!((hi[0] - lo[0] - 40.0).abs() < 0.01 && (hi[2] - 10.0).abs() < 0.01);
    assert!(k.shell(&solid, 50.0, &[top]).is_err(), "too thick is an error, not a crash");
}

#[test]
fn mirror_rotate_translate() {
    let k = Occt;
    let solid = k.extrude(&Profile::rect(Plane::XY, [10.0, 0.0], [20.0, 5.0]), 3.0).unwrap();
    // Mirror across the YZ plane: x 10..20 becomes -20..-10.
    let (lo, hi) = bounds(&k, &k.mirror(&solid, [0.0; 3], [1.0, 0.0, 0.0]));
    assert!((lo[0] + 20.0).abs() < 1e-3 && (hi[0] + 10.0).abs() < 1e-3, "{lo:?} {hi:?}");
    assert_eq!(k.face_count(&k.mirror(&solid, [0.0; 3], [1.0, 0.0, 0.0])), 6);
    // Quarter turn about Z through the origin: x 10..20 becomes y 10..20.
    let (lo, hi) = bounds(&k, &k.rotate(&solid, [0.0; 3], [0.0, 0.0, 1.0], 90.0));
    assert!((lo[1] - 10.0).abs() < 1e-3 && (hi[1] - 20.0).abs() < 1e-3, "{lo:?} {hi:?}");
    let (lo, _) = bounds(&k, &k.translate(&solid, [0.0, 0.0, 7.0]));
    assert!((lo[2] - 7.0).abs() < 1e-3);
}

#[test]
fn volume_of_a_box() {
    let k = Occt;
    assert!((k.volume(&plate(&k)) - 40.0 * 30.0 * 10.0).abs() < 1e-6);
}

#[test]
fn meshes_carry_exact_areas_and_volume() {
    let k = Occt;
    let cyl = k.extrude(&Profile::Circle { plane: Plane::XY, center: [0.0, 0.0], radius: 5.0 }, 10.0).unwrap();
    let mesh = k.tessellate(&cyl, 0.05).unwrap();
    let pi = std::f64::consts::PI;
    assert!((mesh.volume - pi * 25.0 * 10.0).abs() < 1e-6, "{}", mesh.volume);
    let total: f64 = mesh.face_areas.iter().sum();
    assert!((total - (2.0 * pi * 25.0 + 2.0 * pi * 5.0 * 10.0)).abs() < 1e-6, "{total}");
}

#[test]
fn imports_fusion_f3d_files() {
    let dir = std::env::var_os("FISSION_SAMPLES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/f3d"));
    let k = Occt;
    for f in ["flathex.f3d", "clouds.f3d", "camera.f3d"] {
        let path = dir.join(f);
        if !path.exists() {
            continue;
        }
        let solid = k.import_step(&path).unwrap();
        let mesh = k.tessellate(&solid, 0.1).unwrap();
        eprintln!("{}: {} faces, {} tris, volume {:.1} mm3", path.display(), k.face_count(&solid), mesh.triangle_count(), k.volume(&solid));
        assert!(k.face_count(&solid) > 10);
        assert!(mesh.triangle_count() > 0);
    }
}

#[test]
fn revolve_about_an_offset_axis() {
    // A 10x10 square 5..15 mm from an axis through x=5 along Y: a ring with
    // mean radius 10, so the volume is 2*pi*10*100 (Pappus).
    let k = Occt;
    let square = Profile::rect(Plane::XY, [10.0, 0.0], [20.0, 10.0]);
    let ring = k.revolve(&square, [5.0, 0.0, 0.0], [0.0, 1.0, 0.0], 360.0).unwrap();
    let expected = 2.0 * std::f64::consts::PI * 10.0 * 100.0;
    assert!((k.volume(&ring) - expected).abs() < 1e-3 * expected, "{}", k.volume(&ring));
}

#[test]
fn extrude_from_a_slanted_plane() {
    // A 45-degree plane through (0, 0, 5): a 10x4 rectangle pushed 3 mm
    // along the plane's normal has the same volume as anywhere else.
    let k = Occt;
    let h = std::f64::consts::FRAC_1_SQRT_2;
    let plane = Plane { origin: [0.0, 0.0, 5.0], normal: [0.0, -h, h], x_dir: [1.0, 0.0, 0.0] };
    let profile = Profile::rect(plane, [0.0, 0.0], [10.0, 4.0]);
    let solid = k.extrude(&profile, 3.0).unwrap();
    assert!((k.volume(&solid) - 120.0).abs() < 1e-6, "{}", k.volume(&solid));
    // It rose along the normal: the far corner is at z = 5 + 4h + 3h.
    let mesh = k.tessellate(&solid, 0.1).unwrap();
    let top = mesh.positions.iter().map(|p| p[2]).fold(f32::MIN, f32::max) as f64;
    assert!((top - (5.0 + 7.0 * h)).abs() < 1e-4, "{top}");
}

#[test]
fn loft_between_a_square_and_a_smaller_square() {
    // A frustum: 20x20 at z=0 to 10x10 at z=10. Volume h/3 (A1 + A2 + sqrt(A1 A2)).
    let k = Occt;
    let mut top = Plane::XY;
    top.origin = [0.0, 0.0, 10.0];
    let a = Profile::rect(Plane::XY, [-10.0, -10.0], [10.0, 10.0]);
    let b = Profile::rect(top, [-5.0, -5.0], [5.0, 5.0]);
    let solid = k.loft(&[a, b]).unwrap();
    let expected = 10.0 / 3.0 * (400.0 + 100.0 + 200.0);
    assert!((k.volume(&solid) - expected).abs() < 1e-3 * expected, "{}", k.volume(&solid));
}

#[test]
fn sweep_a_circle_along_an_l_shaped_path() {
    use fission_kernel::Seg3;
    // A 2 mm radius circle on XY, swept up +Z, round a 90-degree bend, then
    // along +X.
    let k = Occt;
    let circle = Profile::Circle { plane: Plane::XY, center: [0.0, 0.0], radius: 2.0 };
    let r = 10.0;
    let s = std::f64::consts::FRAC_1_SQRT_2;
    let path = [
        Seg3::Line([0.0, 0.0, 0.0], [0.0, 0.0, 20.0]),
        Seg3::Arc { start: [0.0, 0.0, 20.0], mid: [r - r * s, 0.0, 20.0 + r * s], end: [r, 0.0, 30.0] },
        Seg3::Line([r, 0.0, 30.0], [r + 15.0, 0.0, 30.0]),
    ];
    let solid = k.sweep(&circle, &path).unwrap();
    // Tube volume = area * path length (Pappus holds for the bend too).
    let length = 20.0 + std::f64::consts::FRAC_PI_2 * r + 15.0;
    let expected = std::f64::consts::PI * 4.0 * length;
    assert!((k.volume(&solid) - expected).abs() < 1e-2 * expected, "{} vs {expected}", k.volume(&solid));
}

#[test]
fn sweep_round_a_sharp_corner_stays_round() {
    use fission_kernel::Seg3;
    // Up 20, then 15 along +X with a sharp corner: both arms keep the
    // circle's 2 mm radius (a flattened arm would have far less volume).
    let k = Occt;
    let circle = Profile::Circle { plane: Plane::XY, center: [0.0, 0.0], radius: 2.0 };
    let path = [Seg3::Line([0.0, 0.0, 0.0], [0.0, 0.0, 20.0]), Seg3::Line([0.0, 0.0, 20.0], [15.0, 0.0, 20.0])];
    let solid = k.sweep(&circle, &path).unwrap();
    let area = std::f64::consts::PI * 4.0;
    let v = k.volume(&solid);
    assert!(v > area * 33.0 && v < area * 36.0, "{v} vs about {}", area * 35.0);
}

#[test]
fn holes_remove_the_right_volume() {
    use fission_model::{Document, FeatureKind, FeatureStatus, HoleKind, Operation, OriginPlane, Sketch};
    use std::f64::consts::PI;
    let k = Occt;
    let plate_volume = 40.0 * 30.0 * 10.0;
    let removed = |kind: HoleKind, depth: f64, through: bool, tip_angle: f64| {
        let mut doc = Document::default();
        doc.push(FeatureKind::Extrude {
            sketch: Sketch::Rect { plane: OriginPlane::XY, offset: 0.0, center: [0.0, 0.0], size: [40.0, 30.0] },
            distance: 10.0,
            symmetric: false,
            operation: Operation::NewBody,
        });
        doc.push(FeatureKind::Hole {
            plane: OriginPlane::XY,
            offset: 10.0,
            center: [5.0, 0.0],
            flip: false,
            diameter: 6.0,
            depth,
            through,
            kind,
            tip_angle,
        });
        let regen = doc.regenerate(&k);
        assert!(regen.status.iter().all(|s| *s == FeatureStatus::Ok), "{:?}", regen.status);
        plate_volume - k.volume(&regen.bodies[0].solid)
    };
    let close = |a: f64, b: f64| assert!((a - b).abs() < 1e-3 * b, "{a} vs {b}");
    close(removed(HoleKind::Simple, 4.0, false, 0.0), PI * 9.0 * 4.0);
    let h = 3.0 / 59f64.to_radians().tan();
    close(removed(HoleKind::Simple, 4.0, false, 118.0), PI * 9.0 * 4.0 + PI * 9.0 * h / 3.0);
    close(removed(HoleKind::Counterbore { diameter: 10.0, depth: 2.0 }, 0.0, true, 118.0), PI * 9.0 * 10.0 + PI * 16.0 * 2.0);
    // 90° countersink from 12 mm down to 6 mm: 3 mm deep.
    close(removed(HoleKind::Countersink { diameter: 12.0, angle: 90.0 }, 0.0, true, 0.0), PI * 9.0 * 10.0 + PI * (36.0 + 18.0 + 9.0) - PI * 27.0);
}

#[test]
fn circular_pattern_turns_about_a_construction_axis() {
    use fission_model::{Axis, Document, FeatureKind, FeatureStatus, Operation, OriginPlane, PatternKind, Sketch};
    let k = Occt;
    let mut doc = Document::default();
    let body = doc.push(FeatureKind::Extrude {
        sketch: Sketch::Rect { plane: OriginPlane::XY, offset: 0.0, center: [25.0, 0.0], size: [10.0, 10.0] },
        distance: 5.0,
        symmetric: false,
        operation: Operation::NewBody,
    });
    // Vertical axis through (20, 0): a half turn lands the copy on x 10..20.
    let axis = doc.push(FeatureKind::Axis { origin: [20.0, 0.0, 0.0], dir: [0.0, 0.0, 1.0] });
    doc.push(FeatureKind::Pattern {
        bodies: vec![body],
        pattern: PatternKind::Circular { axis: Axis::Line { feature: axis, origin: [20.0, 0.0, 0.0], dir: [0.0, 0.0, 1.0] }, count: 2, degrees: 360.0 },
        join: true,
    });
    let regen = doc.regenerate(&k);
    assert!(regen.status.iter().all(|s| *s == FeatureStatus::Ok), "{:?}", regen.status);
    let (lo, hi) = bounds(&k, &regen.bodies[0].solid);
    assert!((lo[0] - 10.0).abs() < 1e-3 && (hi[0] - 30.0).abs() < 1e-3, "{lo:?} {hi:?}");
    assert_eq!(doc.construction_axes().len(), 1);
}

#[test]
fn draft_narrows_the_part_along_the_pull() {
    let k = Occt;
    let solid = plate(&k); // 40 x 30 x 10, z from 0 to 10
    let mesh = k.tessellate(&solid, 0.1).unwrap();
    let walls: Vec<_> = mesh.face_hints.iter().filter(|h| h.normal[2].abs() < 1e-6).copied().collect();
    assert_eq!(walls.len(), 4);
    let drafted = k.draft(&solid, &walls, [0.0; 3], [0.0, 0.0, 1.0], 5.0).unwrap();
    // The bottom stays 40 x 30; each wall leans in by 10·tan 5° at the top.
    let t = 10.0 * 5f64.to_radians().tan();
    let (a0, a1) = (40.0 * 30.0, (40.0 - 2.0 * t) * (30.0 - 2.0 * t));
    let frustum = 10.0 / 3.0 * (a0 + a1 + (a0 * a1).sqrt());
    let v = k.volume(&drafted);
    assert!((v - frustum).abs() < 1e-3 * frustum, "{v} vs {frustum}");
    let (lo, hi) = bounds(&k, &drafted);
    assert!((lo[0] + 20.0).abs() < 1e-3 && (hi[0] - 20.0).abs() < 1e-3);
}

#[test]
fn scale_grows_volume_by_the_cube() {
    let k = Occt;
    let doubled = k.scale(&plate(&k), [0.0; 3], 2.0);
    assert!((k.volume(&doubled) - 8.0 * 40.0 * 30.0 * 10.0).abs() < 1e-6);
}

#[test]
fn split_body_cuts_one_body_into_two() {
    use fission_model::{Document, FeatureKind, FeatureStatus, Operation, OriginPlane, Sketch};
    let k = Occt;
    let split = |plane: OriginPlane, offset: f64| {
        let mut doc = Document::default();
        doc.push(FeatureKind::Extrude {
            sketch: Sketch::Rect { plane: OriginPlane::XY, offset: 0.0, center: [0.0, 0.0], size: [40.0, 30.0] },
            distance: 10.0,
            symmetric: false,
            operation: Operation::NewBody,
        });
        doc.push(FeatureKind::Split { body: None, plane, offset });
        doc.regenerate(&k)
    };
    // 5 mm right of center: 25 mm of the 40 stay behind, 15 go ahead.
    let regen = split(OriginPlane::YZ, 5.0);
    assert!(regen.status.iter().all(|s| *s == FeatureStatus::Ok), "{:?}", regen.status);
    let v: Vec<f64> = regen.bodies.iter().map(|b| k.volume(&b.solid)).collect();
    assert_eq!(v.len(), 2);
    assert!((v[0] - 25.0 * 30.0 * 10.0).abs() < 1e-6 && (v[1] - 15.0 * 30.0 * 10.0).abs() < 1e-6, "{v:?}");
    let (lo, _) = bounds(&k, &regen.bodies[1].solid);
    assert!((lo[0] - 5.0).abs() < 1e-4);

    // A slanted plane through the middle halves it.
    let n = [1.0 / 2f64.sqrt(), 0.0, 1.0 / 2f64.sqrt()];
    let regen = split(OriginPlane::Custom { origin: [0.0, 0.0, 5.0], normal: n, x_dir: [0.0, 1.0, 0.0] }, 0.0);
    let v: Vec<f64> = regen.bodies.iter().map(|b| k.volume(&b.solid)).collect();
    assert!((v[0] - 6000.0).abs() < 1e-3 && (v[1] - 6000.0).abs() < 1e-3, "{v:?}");

    // A plane that misses the body is an error, not an empty body.
    let regen = split(OriginPlane::XY, 50.0);
    assert!(matches!(regen.status[1], FeatureStatus::Error(_)));
    assert_eq!(regen.bodies.len(), 1);
}

#[test]
fn offset_face_moves_a_face_and_its_neighbours_follow() {
    let k = Occt;
    let solid = plate(&k); // 40 x 30 x 10
    let mesh = k.tessellate(&solid, 0.1).unwrap();
    let top = *mesh.face_hints.iter().find(|h| h.normal[2] > 0.99).unwrap();
    // Pull the top up 5: the walls grow to meet it.
    let raised = k.offset_faces(&solid, &[top], 5.0).unwrap();
    assert!((k.volume(&raised) - 40.0 * 30.0 * 15.0).abs() < 1e-6, "{}", k.volume(&raised));
    assert_eq!(k.face_count(&raised), 6);
    // Push a side in by 4.
    let side = *mesh.face_hints.iter().find(|h| h.normal[0] > 0.99).unwrap();
    let narrowed = k.offset_faces(&solid, &[side], -4.0).unwrap();
    assert!((k.volume(&narrowed) - 36.0 * 30.0 * 10.0).abs() < 1e-6, "{}", k.volume(&narrowed));
    let (_, hi) = bounds(&k, &narrowed);
    assert!((hi[0] - 16.0).abs() < 1e-4);

    // With every edge rounded, OCCT's offset once returned a solid more than
    // twice the size without complaint; the volume change must be sane.
    let rounded = k.fillet(&solid, EdgeSelection::All, 2.0).unwrap();
    let top = *k.tessellate(&rounded, 0.1).unwrap().face_hints.iter().find(|h| h.normal[2] > 0.99).unwrap();
    let pulled = k.offset_faces(&rounded, &[top], 5.0).unwrap();
    let grown = k.volume(&pulled) - k.volume(&rounded);
    assert!(grown > 0.9 * 36.0 * 26.0 * 5.0 && grown < 40.0 * 30.0 * 5.0 * 1.01, "{grown}");

    // A hole's wall offset outward into the hole shrinks it.
    let hole = k.extrude(&Profile::Circle { plane: Plane::XY, center: [0.0, 0.0], radius: 5.0 }, 10.0).unwrap();
    let drilled = k.boolean(&solid, &hole, BooleanOp::Subtract).unwrap();
    let mesh = k.tessellate(&drilled, 0.1).unwrap();
    let wall = *mesh.face_hints.iter().find(|h| h.normal[2].abs() < 1e-6 && h.center[0].abs() < 6.0 && h.center[1].abs() < 6.0).unwrap();
    let smaller = k.offset_faces(&drilled, &[wall], 2.0).unwrap();
    let expect = 40.0 * 30.0 * 10.0 - std::f64::consts::PI * 9.0 * 10.0;
    assert!((k.volume(&smaller) - expect).abs() < 1e-3, "{} vs {expect}", k.volume(&smaller));
}

#[test]
fn spheres_and_tori_have_their_textbook_volumes() {
    use fission_model::{Document, FeatureKind, FeatureStatus, Operation, OriginPlane};
    use std::f64::consts::PI;
    let k = Occt;
    let mut doc = Document::default();
    doc.push(FeatureKind::Sphere { plane: OriginPlane::XZ, offset: 3.0, center: [4.0, 2.0], diameter: 20.0, operation: Operation::NewBody });
    doc.push(FeatureKind::Torus { plane: OriginPlane::XY, offset: 0.0, center: [50.0, 0.0], diameter: 30.0, section: 8.0, operation: Operation::NewBody });
    let regen = doc.regenerate(&k);
    assert!(regen.status.iter().all(|s| *s == FeatureStatus::Ok), "{:?}", regen.status);
    let v: Vec<f64> = regen.bodies.iter().map(|b| k.volume(&b.solid)).collect();
    assert!((v[0] - 4.0 / 3.0 * PI * 1000.0).abs() < 1e-3 * v[0], "{v:?}");
    assert!((v[1] - 2.0 * PI * PI * 15.0 * 16.0).abs() < 1e-3 * v[1], "{v:?}");
    // The sphere sits at (4, 2) on XZ moved 3 along its normal (-Y).
    let (lo, hi) = bounds(&k, &regen.bodies[0].solid);
    let c: Vec<f32> = (0..3).map(|i| (lo[i] + hi[i]) / 2.0).collect();
    assert!((c[0] - 4.0).abs() < 1e-3 && (c[1] + 3.0).abs() < 1e-3 && (c[2] - 2.0).abs() < 1e-3, "{c:?}");
    let (lo, hi) = bounds(&k, &regen.bodies[1].solid);
    // Tessellated, so the ring comes up a chord short of 38.
    assert!((hi[0] - lo[0] - 38.0).abs() < 0.2 && (hi[2] - lo[2] - 8.0).abs() < 1e-3, "{lo:?} {hi:?}");
}

#[test]
fn deleting_faces_fills_holes_and_sharpens_fillets() {
    let k = Occt;
    let solid = plate(&k);
    let full = 40.0 * 30.0 * 10.0;
    let hole = k.extrude(&Profile::Circle { plane: Plane::XY, center: [5.0, 0.0], radius: 3.0 }, 10.0).unwrap();
    let drilled = k.boolean(&solid, &hole, BooleanOp::Subtract).unwrap();
    let mesh = k.tessellate(&drilled, 0.1).unwrap();
    let wall = *mesh.face_hints.iter().find(|h| h.normal[2].abs() < 1e-6 && (h.center[0] - 5.0).abs() < 4.0 && h.center[1].abs() < 4.0).unwrap();
    let filled = k.delete_faces(&drilled, &[wall]).unwrap();
    assert!((k.volume(&filled) - full).abs() < 1e-6, "{}", k.volume(&filled));
    assert_eq!(k.face_count(&filled), 6);

    // Round the top +Y edge, then delete the round: sharp again.
    let mesh = k.tessellate(&solid, 0.1).unwrap();
    let edge = mesh
        .edges
        .iter()
        .filter_map(|p| fission_kernel::EdgeHint::from_polyline(p))
        .find(|h| (h.mid[1] - 15.0).abs() < 1e-6 && (h.mid[2] - 10.0).abs() < 1e-6)
        .unwrap();
    let rounded = k.fillet(&solid, EdgeSelection::Near(vec![edge]), 2.0).unwrap();
    let mesh = k.tessellate(&rounded, 0.1).unwrap();
    let round = *mesh.face_hints.iter().find(|h| h.normal.iter().all(|n| n.abs() < 0.99)).unwrap();
    let sharp = k.delete_faces(&rounded, &[round]).unwrap();
    assert!((k.volume(&sharp) - full).abs() < 1e-6, "{}", k.volume(&sharp));
    assert_eq!(k.face_count(&sharp), 6);

    // With every edge rounded, one round can't go alone (OCCT returns the
    // solid unchanged): its tangent chain goes, so the box is sharp again.
    let rounded = k.fillet(&solid, EdgeSelection::All, 2.0).unwrap();
    let mesh = k.tessellate(&rounded, 0.1).unwrap();
    let round = *mesh.face_hints.iter().find(|h| h.normal.iter().all(|n| n.abs() < 0.99)).unwrap();
    let sharp = k.delete_faces(&rounded, &[round]).unwrap();
    assert!((k.volume(&sharp) - full).abs() < 1e-6 && k.face_count(&sharp) == 6);
    // A face nothing can grow to replace is an error, not a silent no-op.
    let top = *mesh.face_hints.iter().find(|h| h.normal[2] > 0.99).unwrap();
    assert!(k.delete_faces(&rounded, &[top]).is_err());
}

#[test]
fn coils_and_threads_follow_a_helix() {
    use std::f64::consts::PI;
    let k = Occt;
    // A screw motion keeps volume like a revolve: turns · 2πR · section area.
    let spring = k.coil([0.0; 3], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0], 10.0, 6.0, 3.0, false, 4.0).unwrap();
    let expect = 3.0 * 2.0 * PI * 10.0 * PI * 4.0;
    assert!((k.volume(&spring) - expect).abs() < 1e-3 * expect, "{} vs {expect}", k.volume(&spring));
    let (lo, hi) = bounds(&k, &spring);
    assert!((hi[2] - lo[2] - (18.0 + 4.0)).abs() < 0.05, "{lo:?} {hi:?}");
    assert!(k.coil([0.0; 3], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0], 10.0, 3.0, 3.0, false, 4.0).is_err(), "overlapping turns");

    // An M10 shaft, 20 long, pitch 1.5: the thread takes material from the
    // outside but leaves the core (the root diameter) and the far end alone.
    let shaft = k.extrude(&Profile::Circle { plane: Plane::XY, center: [0.0, 0.0], radius: 5.0 }, 20.0).unwrap();
    let mesh = k.tessellate(&shaft, 0.1).unwrap();
    let side = *mesh.face_hints.iter().find(|h| h.normal[2].abs() < 1e-6).unwrap();
    let threaded = k.thread(&shaft, &side, 1.5, false).unwrap();
    let (full, cut) = (k.volume(&shaft), k.volume(&threaded));
    let core = PI * (5.0f64 - 0.6134 * 1.5).powi(2) * 20.0;
    assert!(cut < full * 0.95 && cut > core, "{cut} of {full}, core {core}");
    let (lo, hi) = bounds(&k, &threaded);
    assert!(lo[2].abs() < 1e-4 && (hi[2] - 20.0).abs() < 1e-4);
    // 25 long at 1.5 once came back empty from OCCT's boolean: the cut checks
    // what it removed and retries.
    let long = k.extrude(&Profile::Circle { plane: Plane::XY, center: [0.0, 0.0], radius: 5.0 }, 25.0).unwrap();
    let side = *k.tessellate(&long, 0.1).unwrap().face_hints.iter().find(|h| h.normal[2].abs() < 1e-6).unwrap();
    let ratio = k.volume(&k.thread(&long, &side, 1.5, false).unwrap()) / k.volume(&long);
    assert!((ratio - cut / full).abs() < 0.01, "{ratio} vs {}", cut / full);
    // A tapped hole: the thread cuts outward into the plate.
    let hole = k.extrude(&Profile::Circle { plane: Plane::XY, center: [0.0, 0.0], radius: 4.0 }, 10.0).unwrap();
    let plate = k.boolean(&plate(&k), &hole, BooleanOp::Subtract).unwrap();
    let mesh = k.tessellate(&plate, 0.1).unwrap();
    let wall = *mesh.face_hints.iter().find(|h| h.normal[2].abs() < 1e-6 && h.center[0].abs() < 5.0 && h.center[1].abs() < 5.0).unwrap();
    let tapped = k.thread(&plate, &wall, 1.25, false).unwrap();
    assert!(k.volume(&tapped) < k.volume(&plate) - 10.0);
    let (lo, hi) = bounds(&k, &tapped);
    assert!((lo[0] + 20.0).abs() < 1e-4 && (hi[0] - 20.0).abs() < 1e-4, "the plate's outside is untouched");
}

/// Every mesh edge is shared by exactly one other triangle running the
/// other way (welding vertices by position), so an STL of it is printable.
fn assert_watertight(mesh: &fission_kernel::Mesh) {
    use std::collections::HashMap;
    let key = |i: u32| mesh.positions[i as usize].map(|c| (c * 1e4).round() as i64);
    let mut edges: HashMap<_, i32> = HashMap::new();
    for t in mesh.indices.chunks_exact(3) {
        let k = [key(t[0]), key(t[1]), key(t[2])];
        if k[0] == k[1] || k[1] == k[2] || k[0] == k[2] {
            continue;
        }
        for (a, b) in [(k[0], k[1]), (k[1], k[2]), (k[2], k[0])] {
            *edges.entry((a, b)).or_default() += 1;
        }
    }
    let open = edges.iter().filter(|&(&(a, b), &n)| edges.get(&(b, a)) != Some(&n)).count();
    assert_eq!(open, 0, "{open} unpaired edges");
}

#[test]
fn threaded_shaft_tessellates_watertight() {
    // Meshing faces one at a time left cracks along the helical seams.
    let k = Occt;
    let shaft = k.extrude(&Profile::Circle { plane: Plane::XY, center: [0.0, 0.0], radius: 5.0 }, 25.0).unwrap();
    let side = *k.tessellate(&shaft, 0.1).unwrap().face_hints.iter().find(|h| h.normal[2].abs() < 1e-6).unwrap();
    let threaded = k.thread(&shaft, &side, 1.5, false).unwrap();
    assert_watertight(&k.tessellate(&plate(&k), 0.01).unwrap());
    assert_watertight(&k.tessellate(&threaded, 0.01).unwrap());
}

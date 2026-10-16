use super::*;
use fission_kernel::{KernelError, Mesh, Result, Vec3};
use std::path::Path;

/// Kernel that records the ops applied to each solid as a string.
struct Fake;

impl Kernel for Fake {
    type Solid = String;

    fn extrude(&self, profile: &Profile, distance: f64) -> Result<String> {
        let z = profile.plane().origin[2];
        Ok(format!("extrude({distance}@{z})"))
    }
    fn revolve(&self, _: &Profile, origin: Vec3, dir: Vec3, degrees: f64) -> Result<String> {
        Ok(format!("revolve({degrees}@{origin:?}/{dir:?})"))
    }
    fn loft(&self, sections: &[Profile]) -> Result<String> {
        let zs: Vec<String> = sections.iter().map(|p| format!("{}", p.plane().origin[2])).collect();
        Ok(format!("loft({})", zs.join(",")))
    }
    fn sweep(&self, _: &Profile, path: &[fission_kernel::Seg3]) -> Result<String> {
        Ok(format!("sweep({})", path.len()))
    }
    fn boolean(&self, a: &String, b: &String, op: BooleanOp) -> Result<String> {
        Ok(format!("{op:?}({a},{b})"))
    }
    fn fillet(&self, s: &String, sel: EdgeSelection, radius: f64) -> Result<String> {
        if radius > 10.0 {
            return Err(KernelError::Operation { op: "fillet", reason: "too big".into() });
        }
        match sel {
            EdgeSelection::All => Ok(format!("fillet({s})")),
            EdgeSelection::Near(h) => Ok(format!("fillet{}({s})", h.len())),
        }
    }
    fn chamfer(&self, s: &String, _: EdgeSelection, _: f64) -> Result<String> {
        Ok(format!("chamfer({s})"))
    }
    fn shell(&self, s: &String, t: f64, open: &[fission_kernel::FaceHint]) -> Result<String> {
        Ok(format!("shell{t}/{}({s})", open.len()))
    }
    fn draft(&self, s: &String, faces: &[fission_kernel::FaceHint], _: Vec3, _: Vec3, degrees: f64) -> Result<String> {
        Ok(format!("draft{degrees}/{}({s})", faces.len()))
    }
    fn offset_faces(&self, s: &String, faces: &[fission_kernel::FaceHint], distance: f64) -> Result<String> {
        Ok(format!("offset{distance}/{}({s})", faces.len()))
    }
    fn thread(&self, s: &String, _: &fission_kernel::FaceHint, pitch: f64, left: bool) -> Result<String> {
        Ok(format!("thread{pitch}{}({s})", if left { "L" } else { "" }))
    }
    fn coil(&self, _: Vec3, _: Vec3, _: Vec3, radius: f64, pitch: f64, turns: f64, _: bool, section: f64) -> Result<String> {
        Ok(format!("coil{radius}/{pitch}/{turns}/{section}"))
    }
    fn delete_faces(&self, s: &String, faces: &[fission_kernel::FaceHint]) -> Result<String> {
        Ok(format!("delete{}({s})", faces.len()))
    }
    fn scale(&self, s: &String, _: Vec3, factor: f64) -> String {
        format!("S{factor}({s})")
    }
    fn translate(&self, s: &String, by: Vec3) -> String {
        format!("T{by:?}({s})")
    }
    fn rotate(&self, s: &String, _: Vec3, _: Vec3, degrees: f64) -> String {
        format!("R{degrees}({s})")
    }
    fn mirror(&self, s: &String, _: Vec3, n: Vec3) -> String {
        format!("M{n:?}({s})")
    }
    fn compound(&self, solids: &[String]) -> String {
        format!("C({})", solids.join(","))
    }
    fn volume(&self, _: &String) -> f64 {
        1.0
    }
    fn tessellate(&self, _: &String, _: f64) -> Result<Mesh> {
        Ok(Mesh::default())
    }
    fn face_count(&self, _: &String) -> usize {
        0
    }
    fn export_step(&self, _: &[String], _: &Path) -> Result<()> {
        Ok(())
    }
    fn import_step(&self, _: &Path) -> Result<String> {
        Ok("step".into())
    }
}

#[test]
fn sample_regenerates_in_order() {
    let regen = Document::sample().regenerate(&Fake);
    assert!(regen.status.iter().all(|s| *s == FeatureStatus::Ok));
    assert_eq!(regen.bodies.len(), 1);
    assert_eq!(
        regen.bodies[0].solid,
        "fillet(Subtract(Union(extrude(12@0),extrude(25@12)),extrude(12@0)))"
    );
}

#[test]
fn marker_rolls_back_later_features() {
    let mut doc = Document::sample();
    doc.marker = 1;
    let regen = doc.regenerate(&Fake);
    assert_eq!(regen.bodies[0].solid, "extrude(12@0)");
    assert_eq!(regen.status[1..], [FeatureStatus::RolledBack, FeatureStatus::RolledBack, FeatureStatus::RolledBack]);
}

#[test]
fn failing_feature_is_reported_and_skipped() {
    let mut doc = Document::sample();
    doc.features[3].kind = FeatureKind::Fillet { radius: 50.0, edges: vec![] };
    doc.push(FeatureKind::Chamfer { distance: 1.0, edges: vec![] });
    let regen = doc.regenerate(&Fake);
    assert!(matches!(regen.status[3], FeatureStatus::Error(_)));
    assert_eq!(regen.status[4], FeatureStatus::Ok);
    assert!(regen.bodies[0].solid.starts_with("chamfer(Subtract("));
}

#[test]
fn suppressed_feature_is_skipped() {
    let mut doc = Document::sample();
    doc.features[2].suppressed = true;
    let regen = doc.regenerate(&Fake);
    assert_eq!(regen.status[2], FeatureStatus::Suppressed);
    assert!(!regen.bodies[0].solid.contains("Subtract"));
}

#[test]
fn join_without_body_errors() {
    let mut doc = Document::default();
    doc.push(FeatureKind::Extrude {
        sketch: Sketch::Circle { plane: OriginPlane::XY, offset: 0.0, center: [0.0; 2], radius: 1.0 },
        distance: 1.0,
        symmetric: false,
        operation: Operation::Cut,
    });
    let regen = doc.regenerate(&Fake);
    assert_eq!(regen.status[0], FeatureStatus::Error("Cut needs an existing body".into()));
}

#[test]
fn symmetric_extrude_starts_below_plane() {
    let mut doc = Document::default();
    doc.push(FeatureKind::Extrude {
        sketch: Sketch::Rect { plane: OriginPlane::XY, offset: 0.0, center: [0.0; 2], size: [1.0, 1.0] },
        distance: 5.0,
        symmetric: true,
        operation: Operation::NewBody,
    });
    assert_eq!(doc.regenerate(&Fake).bodies[0].solid, "extrude(10@-5)");
}

#[test]
fn push_inserts_at_marker_and_names_by_type() {
    let mut doc = Document::sample();
    doc.marker = 2;
    let at = doc.push(FeatureKind::Chamfer { distance: 1.0, edges: vec![] });
    assert_eq!(at, 2);
    assert_eq!(doc.marker, 3);
    assert_eq!(doc.features[2].name, "Chamfer1");
    assert_eq!(doc.features[0].name, "Extrude1");
    assert_eq!(doc.features[1].name, "Extrude2");
}

#[test]
fn remove_keeps_marker_consistent() {
    let mut doc = Document::sample();
    doc.marker = 2;
    doc.remove(0);
    assert_eq!(doc.marker, 1);
    doc.remove(3);
    assert_eq!(doc.features.len(), 3);
}

#[test]
fn ron_round_trip() {
    let doc = Document::sample();
    let text = doc.to_ron().unwrap();
    assert_eq!(Document::from_ron(&text).unwrap(), doc);
}

#[test]
fn undo_redo() {
    let mut history = History::default();
    let mut doc = Document::sample();
    let before = doc.clone();
    doc.remove(0);
    history.record(before.clone());
    assert!(history.undo(&mut doc));
    assert_eq!(doc, before);
    assert!(history.redo(&mut doc));
    assert_eq!(doc.features.len(), 3);
    assert!(!history.redo(&mut doc));
}

#[test]
fn offset_planes_follow_normals() {
    assert_eq!(OriginPlane::XY.plane(5.0).origin, [0.0, 0.0, 5.0]);
    assert_eq!(OriginPlane::XZ.plane(5.0).origin, [0.0, -5.0, 0.0]);
    assert_eq!(OriginPlane::YZ.plane(5.0).origin, [5.0, 0.0, 0.0]);
}

#[test]
fn import_step_creates_a_body() {
    let mut doc = Document::default();
    doc.push(FeatureKind::ImportStep { path: "part.step".into(), operation: Operation::NewBody, hidden_parts: vec![] });
    let regen = doc.regenerate(&Fake);
    assert_eq!(regen.bodies[0].solid, "step");
    assert_eq!(doc.features[0].name, "Import1");
}

#[test]
fn fillet_edges_go_to_their_own_bodies() {
    let mut doc = Document::default();
    let rect = |offset| Sketch::Rect { plane: OriginPlane::XY, offset, center: [0.0; 2], size: [1.0, 1.0] };
    for z in [0.0, 5.0] {
        doc.push(FeatureKind::Extrude { sketch: rect(z), distance: 1.0, symmetric: false, operation: Operation::NewBody });
    }
    let edge = |body| EdgeRef { body, mid: [0.0; 3], dir: [1.0, 0.0, 0.0] };
    doc.push(FeatureKind::Fillet { radius: 0.1, edges: vec![edge(0), edge(0)] });
    let regen = doc.regenerate(&Fake);
    // Two edges on the first body; the second body is untouched.
    assert_eq!(regen.bodies[0].solid, "fillet2(extrude(1@0))");
    assert_eq!(regen.bodies[1].solid, "extrude(1@5)");
}

#[test]
fn target_picks_the_body_to_cut() {
    let mut doc = Document::default();
    let rect = |offset| Sketch::Rect { plane: OriginPlane::XY, offset, center: [0.0; 2], size: [1.0, 1.0] };
    for z in [0.0, 5.0] {
        doc.push(FeatureKind::Extrude { sketch: rect(z), distance: 1.0, symmetric: false, operation: Operation::NewBody });
    }
    let cut = doc.push(FeatureKind::Extrude { sketch: rect(0.0), distance: 2.0, symmetric: false, operation: Operation::Cut });
    doc.features[cut].target = Some(0);
    let regen = doc.regenerate(&Fake);
    assert!(regen.bodies[0].solid.starts_with("Subtract("));
    assert_eq!(regen.bodies[1].solid, "extrude(1@5)");

    doc.features[cut].target = Some(7);
    assert_eq!(doc.regenerate(&Fake).status[cut], FeatureStatus::Error("the target body no longer exists".into()));
}

#[test]
fn old_files_without_new_fields_still_load() {
    let text = r#"(name: "Old", features: [(name: "Fillet1", kind: Fillet(radius: 1.0))], marker: 1)"#;
    let doc = Document::from_ron(text).unwrap();
    assert_eq!(doc.features[0].kind, FeatureKind::Fillet { radius: 1.0, edges: vec![] });
    assert_eq!(doc.features[0].target, None);
}

fn sketch_with_ring() -> sketch2d::Sketch {
    let mut s = sketch2d::Sketch::default();
    s.add_rectangle([0.0, 0.0], [40.0, 20.0]);
    s.add_circle([20.0, 10.0], 5.0);
    s
}

#[test]
fn extrude_sketch_profiles() {
    let mut doc = Document::default();
    let sk = doc.push(FeatureKind::Sketch { plane: OriginPlane::XY, offset: 3.0, sketch: sketch_with_ring() });
    // The plate (point near a corner), not the disk.
    doc.push(FeatureKind::Extrude {
        sketch: Sketch::Profiles { sketch: sk, points: vec![[2.0, 2.0]] },
        distance: 5.0,
        symmetric: false,
        operation: Operation::NewBody,
    });
    let regen = doc.regenerate(&Fake);
    assert_eq!(regen.status, vec![FeatureStatus::Ok, FeatureStatus::Ok]);
    assert_eq!(regen.bodies.len(), 1);
    assert_eq!(regen.bodies[0].solid, "extrude(5@3)", "on the sketch plane, 3 mm up");
    let out = regen.sketches[sk].as_ref().unwrap();
    assert_eq!(out.regions.len(), 2);
    assert_eq!(out.dof, 4 + 3, "free rectangle + free circle");
}

#[test]
fn two_profiles_extrude_as_one_tool() {
    let mut doc = Document::default();
    let sk = doc.push(FeatureKind::Sketch { plane: OriginPlane::XY, offset: 0.0, sketch: sketch_with_ring() });
    doc.push(FeatureKind::Extrude {
        sketch: Sketch::Profiles { sketch: sk, points: vec![[2.0, 2.0], [20.0, 10.0]] },
        distance: 1.0,
        symmetric: false,
        operation: Operation::NewBody,
    });
    assert_eq!(doc.regenerate(&Fake).bodies[0].solid, "Union(extrude(1@0),extrude(1@0))");
}

#[test]
fn lost_profile_is_an_error() {
    let mut doc = Document::default();
    let sk = doc.push(FeatureKind::Sketch { plane: OriginPlane::XY, offset: 0.0, sketch: sketch_with_ring() });
    doc.push(FeatureKind::Extrude {
        sketch: Sketch::Profiles { sketch: sk, points: vec![[100.0, 100.0]] },
        distance: 1.0,
        symmetric: false,
        operation: Operation::NewBody,
    });
    assert!(matches!(&doc.regenerate(&Fake).status[1], FeatureStatus::Error(m) if m.contains("no longer exists")));
}

#[test]
fn references_follow_deletes_reorders_and_inserts() {
    let mut doc = Document::default();
    doc.push(FeatureKind::Chamfer { distance: 1.0, edges: vec![] }); // 0: filler
    let sk = doc.push(FeatureKind::Sketch { plane: OriginPlane::XY, offset: 0.0, sketch: sketch_with_ring() }); // 1
    let ex = doc.push(FeatureKind::Extrude {
        sketch: Sketch::Profiles { sketch: sk, points: vec![[2.0, 2.0]] },
        distance: 1.0,
        symmetric: false,
        operation: Operation::NewBody,
    }); // 2
    let profile_sketch = |doc: &Document, i: usize| match &doc.features[i].kind {
        FeatureKind::Extrude { sketch: Sketch::Profiles { sketch, .. }, .. } => *sketch,
        _ => panic!(),
    };
    assert_eq!(profile_sketch(&doc, ex), 1);

    doc.remove(0);
    assert_eq!(profile_sketch(&doc, 1), 0, "delete before shifts the reference down");

    doc.marker = 0;
    doc.push(FeatureKind::Chamfer { distance: 1.0, edges: vec![] });
    assert_eq!(profile_sketch(&doc, 2), 1, "insert before shifts it up");

    doc.reorder(1, 0);
    assert_eq!(profile_sketch(&doc, 2), 0, "reorder moves it with the sketch");

    doc.remove(0);
    assert_eq!(profile_sketch(&doc, 1), usize::MAX, "deleting the sketch leaves a dangling reference");
    doc.marker = doc.features.len();
    assert!(matches!(doc.regenerate(&Fake).status[1], FeatureStatus::Error(_)));
}

fn two_boxes() -> Document {
    let mut doc = Document::default();
    for z in [0.0, 5.0] {
        doc.push(FeatureKind::Extrude {
            sketch: Sketch::Rect { plane: OriginPlane::XY, offset: z, center: [0.0; 2], size: [1.0, 1.0] },
            distance: 1.0,
            symmetric: false,
            operation: Operation::NewBody,
        });
    }
    doc
}

#[test]
fn combine_cuts_and_consumes_tools() {
    let mut doc = two_boxes();
    doc.push(FeatureKind::Combine { target: 0, tools: vec![1], operation: Operation::Cut, keep_tools: false });
    let regen = doc.regenerate(&Fake);
    assert_eq!(regen.bodies.len(), 1);
    assert_eq!(regen.bodies[0].solid, "Subtract(extrude(1@0),extrude(1@5))");

    doc.features[2].kind = FeatureKind::Combine { target: 0, tools: vec![0], operation: Operation::Join, keep_tools: true };
    assert!(matches!(doc.regenerate(&Fake).status[2], FeatureStatus::Error(_)));
}

#[test]
fn shell_mirror_move_pattern() {
    let mut doc = two_boxes();
    doc.push(FeatureKind::Shell { thickness: 0.2, faces: vec![] });
    let r = doc.regenerate(&Fake);
    assert_eq!(r.bodies[1].solid, "shell0.2/0(extrude(1@5))", "shells the last body");

    let mut doc = two_boxes();
    doc.push(FeatureKind::Mirror { bodies: vec![0], plane: OriginPlane::YZ, offset: 0.0, join: false });
    let r = doc.regenerate(&Fake);
    assert_eq!(r.bodies.len(), 3, "mirror makes a new body");
    assert_eq!(r.bodies[2].solid, "M[1.0, 0.0, 0.0](extrude(1@0))");

    let mut doc = two_boxes();
    doc.push(FeatureKind::Move { bodies: vec![1], translate: [0.0, 0.0, 2.0], axis: Axis::Z, degrees: 90.0, copy: false });
    assert_eq!(doc.regenerate(&Fake).bodies[1].solid, "T[0.0, 0.0, 2.0](R90(extrude(1@5)))");

    let mut doc = two_boxes();
    doc.push(FeatureKind::Pattern {
        bodies: vec![0],
        pattern: PatternKind::Circular { axis: Axis::Z, count: 4, degrees: 360.0 },
        join: true,
    });
    let s = doc.regenerate(&Fake).bodies[0].solid.clone();
    for angle in ["R90(", "R180(", "R270("] {
        assert!(s.contains(angle), "{s}");
    }
    assert!(!s.contains("R360("), "a full circle doesn't duplicate the original");
}

#[test]
fn rectangular_pattern_in_two_directions_makes_count_minus_one_copies() {
    let ts = pattern_transforms(&PatternKind::Rectangular { dir: Axis::X, count: 3, spacing: 10.0, second: Some((Axis::Y, 2, 5.0)) }, &[]).unwrap();
    assert_eq!(ts.len(), 3 * 2 - 1);
}

#[test]
fn deleting_a_body_used_by_combine_leaves_an_error() {
    let mut doc = two_boxes();
    doc.push(FeatureKind::Combine { target: 0, tools: vec![1], operation: Operation::Join, keep_tools: false });
    doc.remove(1);
    assert!(matches!(doc.regenerate(&Fake).status[1], FeatureStatus::Error(_)));
}

#[test]
fn revolve_about_a_sketch_line() {
    // A rectangle beside a construction line at x = 5 on a sketch 2 mm up.
    let mut sketch = sketch2d::Sketch::default();
    sketch.add_rectangle([10.0, 0.0], [20.0, 10.0]);
    let a = sketch.add_point([5.0, 0.0]);
    let b = sketch.add_point([5.0, 10.0]);
    sketch.curves.push(sketch2d::Curve::Line { a, b, construction: true });
    let axis_curve = sketch.curves.len() - 1;
    let mut doc = Document::default();
    let sk = doc.push(FeatureKind::Sketch { plane: OriginPlane::XY, offset: 2.0, sketch });
    doc.push(FeatureKind::Revolve {
        sketch: Sketch::Profiles { sketch: sk, points: vec![[15.0, 5.0]] },
        axis: Axis::SketchLine { sketch: sk, curve: axis_curve },
        degrees: 180.0,
        operation: Operation::NewBody,
    });
    let regen = doc.regenerate(&Fake);
    assert_eq!(regen.status, vec![FeatureStatus::Ok, FeatureStatus::Ok]);
    // The construction line doesn't make a region of its own.
    assert_eq!(regen.sketches[sk].as_ref().unwrap().regions.len(), 1);
    assert_eq!(regen.bodies[0].solid, "revolve(180@[5.0, 0.0, 2.0]/[0.0, 1.0, 0.0])");

    // Moving the sketch later in the timeline keeps the axis pointing at it.
    doc.push(FeatureKind::Sketch { plane: OriginPlane::XY, offset: 0.0, sketch: sketch2d::Sketch::default() });
    doc.reorder(2, 0);
    let FeatureKind::Revolve { axis: Axis::SketchLine { sketch, .. }, .. } = doc.features[2].kind else { panic!() };
    assert_eq!(sketch, sk + 1);
}

#[test]
fn construction_planes_follow_the_marker_and_suppression() {
    let mut doc = Document::default();
    let p = doc.push(FeatureKind::Plane { base: OriginPlane::XY, offset: 20.0, angle: 0.0 });
    let sk = doc.push(FeatureKind::Sketch { plane: OriginPlane::XY, offset: 20.0, sketch: sketch_with_ring() });
    doc.push(FeatureKind::Extrude {
        sketch: Sketch::Profiles { sketch: sk, points: vec![[2.0, 2.0]] },
        distance: 5.0,
        symmetric: false,
        operation: Operation::NewBody,
    });
    let regen = doc.regenerate(&Fake);
    assert_eq!(regen.status, vec![FeatureStatus::Ok; 3]);
    assert_eq!(regen.bodies[0].solid, "extrude(5@20)");
    assert_eq!(doc.construction_planes(), vec![(p, OriginPlane::XY, 20.0)]);
    assert_eq!(doc.features[p].name, "Plane1");
    doc.features[p].suppressed = true;
    assert!(doc.construction_planes().is_empty());
}

fn param(name: &str, expression: &str) -> Parameter {
    Parameter { name: name.into(), expression: expression.into(), quantity: crate::Quantity::Length, comment: String::new() }
}

#[test]
fn parameters_drive_feature_values_and_sketch_dimensions() {
    // Out of order on purpose: wall depends on width.
    let mut doc = Document { parameters: vec![param("wall", "width / 20"), param("width", "40 mm")], ..Default::default() };
    let mut sketch = sketch2d::Sketch::default();
    let [bottom, ..] = sketch.add_rectangle([0.0, 0.0], [10.0, 10.0]);
    sketch.constraints.push(sketch2d::Constraint::Length(bottom, 10.0));
    let dim = sketch.constraints.len() - 1;
    let sk = doc.push(FeatureKind::Sketch { plane: OriginPlane::XY, offset: 0.0, sketch });
    doc.features[sk].exprs.insert(dim_key(dim), "width".into());
    let ex = doc.push(FeatureKind::Extrude {
        sketch: Sketch::Profiles { sketch: sk, points: vec![[1.0, 1.0]] },
        distance: 1.0,
        symmetric: false,
        operation: Operation::NewBody,
    });
    doc.features[ex].exprs.insert("distance".into(), "wall * 3".into());

    assert!(doc.apply_expressions().is_empty());
    let FeatureKind::Extrude { distance, .. } = doc.features[ex].kind else { panic!() };
    assert_eq!(distance, 6.0);
    let FeatureKind::Sketch { sketch, .. } = &doc.features[sk].kind else { panic!() };
    assert!((sketch.line_length(bottom) - 40.0).abs() < 1e-6, "the sketch re-solved to the new width");

    // Change a parameter: everything follows.
    doc.parameters[1].expression = "50".into();
    assert!(doc.apply_expressions().is_empty());
    let FeatureKind::Extrude { distance, .. } = doc.features[ex].kind else { panic!() };
    assert_eq!(distance, 7.5);

    // Survives save and load.
    let back = Document::from_ron(&doc.to_ron().unwrap()).unwrap();
    assert_eq!(back.parameters, doc.parameters);
    assert_eq!(back.features[ex].exprs, doc.features[ex].exprs);
}

#[test]
fn parameter_cycles_are_reported() {
    let doc = Document { parameters: vec![param("a", "b + 1"), param("b", "a * 2")], ..Default::default() };
    let err = doc.parameter_values().unwrap_err();
    assert!(err.contains("a") && err.contains("b"), "{err}");
}

#[test]
fn loft_joins_regions_of_two_sketches() {
    let mut doc = Document::default();
    let rect = |s: f64| {
        let mut sk = sketch2d::Sketch::default();
        sk.add_rectangle([-s, -s], [s, s]);
        sk
    };
    let a = doc.push(FeatureKind::Sketch { plane: OriginPlane::XY, offset: 0.0, sketch: rect(10.0) });
    let p = doc.push(FeatureKind::Plane { base: OriginPlane::XY, offset: 30.0, angle: 0.0 });
    let b = doc.push(FeatureKind::Sketch { plane: OriginPlane::XY, offset: 30.0, sketch: rect(4.0) });
    doc.push(FeatureKind::Loft {
        sections: vec![Section { sketch: a, point: [0.0, 0.0] }, Section { sketch: b, point: [0.0, 0.0] }],
        operation: Operation::NewBody,
    });
    let regen = doc.regenerate(&Fake);
    assert!(regen.status.iter().all(|s| *s == FeatureStatus::Ok), "{:?}", regen.status);
    assert_eq!(regen.bodies[0].solid, "loft(0,30)");
    // Inserting a feature before the sketches keeps the loft pointing at them.
    doc.marker = 0;
    doc.push(FeatureKind::Plane { base: OriginPlane::XZ, offset: 5.0, angle: 0.0 });
    doc.marker = doc.features.len();
    let FeatureKind::Loft { sections, .. } = &doc.features[4].kind else { panic!() };
    assert_eq!((sections[0].sketch, sections[1].sketch), (a + 1, b + 1));
    let _ = p;
}

#[test]
fn sweep_follows_the_chain_through_a_curve() {
    let mut doc = Document::default();
    let profile = doc.push(FeatureKind::Sketch { plane: OriginPlane::XY, offset: 0.0, sketch: {
        let mut s = sketch2d::Sketch::default();
        s.add_circle([0.0, 0.0], 2.0);
        s
    } });
    // An L of two lines sharing a point, on XZ: up Z, then along X.
    let mut path = sketch2d::Sketch::default();
    let o = path.add_point([0.0, 0.0]);
    let up = path.add_point([0.0, 20.0]);
    let side = path.add_point([15.0, 20.0]);
    path.curves.push(sketch2d::Curve::Line { a: o, b: up, construction: false });
    path.curves.push(sketch2d::Curve::Line { a: up, b: side, construction: false });
    let ps = doc.push(FeatureKind::Sketch { plane: OriginPlane::XZ, offset: 0.0, sketch: path });
    doc.push(FeatureKind::Sweep {
        profile: Sketch::Profiles { sketch: profile, points: vec![[0.0, 0.0]] },
        path: Some(PathRef { sketch: ps, curve: 1 }),
        operation: Operation::NewBody,
    });
    let regen = doc.regenerate(&Fake);
    assert_eq!(regen.status[2], FeatureStatus::Ok);
    assert_eq!(regen.bodies[0].solid, "sweep(2)", "both lines of the chain");
    let segs = regen.sketches[ps].as_ref().unwrap().path(1).unwrap();
    assert_eq!(segs[0], fission_kernel::Seg3::Line([0.0, 0.0, 0.0], [0.0, 0.0, 20.0]));
}

#[test]
fn tilted_planes_turn_about_their_x_direction() {
    let p = OriginPlane::XY.tilted(90.0).plane(5.0);
    assert!((p.normal[1] + 1.0).abs() < 1e-9 && p.normal[2].abs() < 1e-9, "{:?}", p.normal);
    assert_eq!(p.x_dir, [1.0, 0.0, 0.0]);
    assert!((p.origin[1] + 5.0).abs() < 1e-9);
    assert_eq!(OriginPlane::XZ.tilted(0.0), OriginPlane::XZ);
    let mut doc = Document::default();
    let i = doc.push(FeatureKind::Plane { base: OriginPlane::XY, offset: 0.0, angle: 30.0 });
    let (_, base, _) = doc.construction_planes()[0];
    let n = base.plane(0.0).normal;
    assert!((n[2] - 30f64.to_radians().cos()).abs() < 1e-9, "{n:?}");
    // Typing while the dialog is open sets the angle.
    assert_eq!(doc.features[i].kind.primary_value().map(|v| v.0), Some("angle"));
}

#[test]
fn construction_axis_references_follow_deletes() {
    let mut doc = Document::default();
    doc.push(FeatureKind::Plane { base: OriginPlane::XY, offset: 5.0, angle: 0.0 });
    let a = doc.push(FeatureKind::Axis { origin: [1.0, 2.0, 3.0], dir: [0.0, 0.0, 1.0] });
    let m = doc.push(FeatureKind::Move {
        bodies: vec![],
        translate: [0.0; 3],
        axis: Axis::Line { feature: a, origin: [1.0, 2.0, 3.0], dir: [0.0, 0.0, 1.0] },
        degrees: 90.0,
        copy: false,
    });
    doc.remove(0);
    let FeatureKind::Move { axis: Axis::Line { feature, .. }, .. } = doc.features[m - 1].kind else { panic!() };
    assert_eq!(feature, a - 1);
    assert_eq!(Axis::Line { feature, origin: [1.0, 2.0, 3.0], dir: [0.0, 0.0, 1.0] }.resolve(&[]).unwrap(), ([1.0, 2.0, 3.0], [0.0, 0.0, 1.0]));
}

#[test]
fn draft_goes_to_the_faces_body() {
    let mut doc = two_boxes();
    let face = |body| FaceRef { body, center: [0.5, 0.0, 0.5], normal: [1.0, 0.0, 0.0] };
    let d = doc.push(FeatureKind::Draft { faces: vec![face(0), face(0)], plane: OriginPlane::XY, offset: 0.0, degrees: 3.0, flip: false });
    let r = doc.regenerate(&Fake);
    assert_eq!(r.bodies[0].solid, "draft3/2(extrude(1@0))", "the first body, not the last");
    // Nothing picked is an error, not a silent no-op.
    doc.features[d].kind = FeatureKind::Draft { faces: vec![], plane: OriginPlane::XY, offset: 0.0, degrees: 3.0, flip: false };
    assert!(matches!(doc.regenerate(&Fake).status[d], FeatureStatus::Error(_)));
}

#[test]
fn scale_defaults_to_the_last_body_and_rejects_zero() {
    let mut doc = two_boxes();
    let s = doc.push(FeatureKind::Scale { bodies: vec![], center: [0.0; 3], factor: 0.5 });
    assert_eq!(doc.regenerate(&Fake).bodies[1].solid, "S0.5(extrude(1@5))");
    doc.features[s].kind = FeatureKind::Scale { bodies: vec![0], center: [0.0; 3], factor: 0.0 };
    assert!(matches!(doc.regenerate(&Fake).status[s], FeatureStatus::Error(_)));
}

#[test]
fn face_edits_go_to_the_faces_body() {
    let mut doc = two_boxes();
    let face = |body| FaceRef { body, center: [0.5, 0.0, 0.5], normal: [1.0, 0.0, 0.0] };
    doc.push(FeatureKind::OffsetFace { faces: vec![face(0)], distance: 2.0 });
    doc.push(FeatureKind::DeleteFace { faces: vec![face(1), face(1)] });
    let r = doc.regenerate(&Fake);
    assert!(r.status.iter().all(|s| *s == FeatureStatus::Ok), "{:?}", r.status);
    assert_eq!(r.bodies[0].solid, "offset2/1(extrude(1@0))");
    assert_eq!(r.bodies[1].solid, "delete2(extrude(1@5))");
    // Deleting the first box leaves the offset dangling, and re-points the delete.
    doc.remove(0);
    let FeatureKind::DeleteFace { faces } = &doc.features[2].kind else { panic!() };
    assert_eq!(faces[0].body, 0);
    assert!(matches!(doc.regenerate(&Fake).status[1], FeatureStatus::Error(_)));
}

#[test]
fn split_keeps_one_side_and_adds_the_other() {
    let mut doc = two_boxes();
    let s = doc.push(FeatureKind::Split { body: Some(0), plane: OriginPlane::YZ, offset: 0.0 });
    let r = doc.regenerate(&Fake);
    assert_eq!(r.bodies.len(), 3);
    assert!(r.bodies[0].solid.starts_with("Subtract(extrude(1@0)"), "{}", r.bodies[0].solid);
    assert!(r.bodies[2].solid.starts_with("Intersect(extrude(1@0)"));
    assert_eq!(r.bodies[2].created_by, s);
    // Its body reference follows a reorder.
    doc.remove(1);
    assert!(matches!(doc.features[1].kind, FeatureKind::Split { body: Some(0), .. }));
}

#[test]
fn construction_points_make_no_bodies() {
    let mut doc = two_boxes();
    let p = doc.push(FeatureKind::Point { pos: [1.0, 2.0, 3.0] });
    let r = doc.regenerate(&Fake);
    assert_eq!(r.bodies.len(), 2);
    assert_eq!(r.status[p], FeatureStatus::Ok);
    assert_eq!(doc.construction_points(), vec![(p, [1.0, 2.0, 3.0])]);
    doc.features[p].suppressed = true;
    assert!(doc.construction_points().is_empty());
}

fn hinge(doc: &mut Document, value: f64) -> usize {
    doc.push(FeatureKind::Joint {
        bodies: vec![1],
        motion: JointMotion::Revolute,
        axis: Axis::X,
        value,
        min: -180.0,
        max: 10.0,
    })
}

#[test]
fn joint_turns_its_bodies_within_limits() {
    let mut doc = two_boxes();
    hinge(&mut doc, -90.0);
    let r = doc.regenerate(&Fake);
    assert_eq!(r.bodies[0].solid, "extrude(1@0)", "the other body stays put");
    assert_eq!(r.bodies[1].solid, "R-90(extrude(1@5))");

    let mut doc = two_boxes();
    let j = hinge(&mut doc, 12.0);
    let r = doc.regenerate(&Fake);
    assert!(matches!(&r.status[j], FeatureStatus::Error(e) if e.contains("outside its limits")), "{:?}", r.status[j]);

    let mut doc = two_boxes();
    doc.push(FeatureKind::Joint { bodies: vec![], motion: JointMotion::Slider, axis: Axis::Z, value: 3.0, min: 0.0, max: 5.0 });
    assert_eq!(doc.regenerate(&Fake).bodies[1].solid, "T[0.0, 0.0, 3.0](extrude(1@5))", "a slider with no bodies moves the last one");
    assert_eq!(doc.joints()[0].bodies, vec![1]);
}

#[test]
fn joint_value_set_through_its_parameter() {
    let mut doc = two_boxes();
    let j = hinge(&mut doc, 0.0);
    doc.parameters.push(Parameter { name: "fold".into(), expression: "-30 deg".into(), quantity: Quantity::Angle, comment: String::new() });
    doc.features[j].exprs.insert("value".into(), "fold".into());
    assert!(doc.apply_expressions().is_empty());
    assert_eq!(doc.joint("").unwrap().value, -30.0);

    doc.set_joint_value(j, -500.0);
    assert_eq!(doc.parameters[0].expression, "-180 deg", "clamped, and written to the parameter");
    assert!(doc.apply_expressions().is_empty());
    assert_eq!(doc.joint("joint1").unwrap().value, -180.0);
}

#[test]
fn joint_survives_ron_and_reorders() {
    let mut doc = two_boxes();
    hinge(&mut doc, 5.0);
    doc.reorder(0, 1);
    let FeatureKind::Joint { bodies, .. } = &doc.features[2].kind else { panic!() };
    assert_eq!(bodies, &vec![0], "body references follow the reorder");
    let back = Document::from_ron(&doc.to_ron().unwrap()).unwrap();
    assert_eq!(back, doc);
}

#[test]
fn rigid_rotation_about_an_offset_axis() {
    let r = Rigid::joint(JointMotion::Revolute, [0.0, -33.0, 18.9], [1.0, 0.0, 0.0], 90.0);
    let p = r.point([5.0, -33.0, 28.9]); // 10 above the axis
    let want = [5.0, -43.0, 18.9]; // turned +90 about +X: +Z goes to -Y
    for k in 0..3 {
        assert!((p[k] - want[k]).abs() < 1e-9, "{p:?}");
    }
    let s = Rigid::joint(JointMotion::Slider, [0.0; 3], [0.0, 1.0, 0.0], 4.0);
    assert_eq!(s.point([1.0, 1.0, 1.0]), [1.0, 5.0, 1.0]);
}

#[test]
fn canvas_places_its_cropped_corners_on_the_plane() {
    let mut doc = Document::default();
    doc.push(FeatureKind::Canvas {
        path: "ref.png".into(),
        plane: OriginPlane::XZ,
        offset: 0.0,
        center: [10.0, 5.0],
        size: [40.0, 20.0],
        rotation: 0.0,
        crop: [0.25, 0.0, 0.75, 0.5],
        opacity: 0.5,
    });
    let r = doc.regenerate(&Fake);
    assert_eq!(r.status, vec![FeatureStatus::Ok]);
    assert!(r.bodies.is_empty(), "a canvas makes no body");
    let c = &doc.canvases()[0];
    let xz = OriginPlane::XZ.plane(0.0);
    assert_eq!(c.corners[0], xz.to_world([-10.0, -5.0]));
    assert_eq!(c.corners[2], xz.to_world([30.0, 15.0]));
    assert_eq!(c.uv[0], [0.25, 0.5], "bottom-left shows the crop's left edge, bottom edge");
    assert_eq!(c.uv[2], [0.75, 0.0]);

    let back = Document::from_ron(&doc.to_ron().unwrap()).unwrap();
    assert_eq!(back, doc);
    let minimal = r#"(name: "c", features: [(name: "Canvas1", kind: Canvas(path: "a.jpg", plane: XY, size: (10.0, 10.0)))], marker: 1)"#;
    let FeatureKind::Canvas { crop, opacity, .. } = Document::from_ron(minimal).unwrap().features[0].kind.clone() else { panic!() };
    assert_eq!((crop, opacity), ([0.0, 0.0, 1.0, 1.0], 0.5), "defaults: whole image, half opacity");
}

#[test]
fn recropping_a_canvas_trims_without_moving_or_stretching() {
    let mut kind = FeatureKind::Canvas {
        path: "a.png".into(),
        plane: OriginPlane::XY,
        offset: 0.0,
        center: [0.0, 0.0],
        size: [100.0, 50.0],
        rotation: 0.0,
        crop: [0.0, 0.0, 1.0, 1.0],
        opacity: 0.5,
    };
    // Keep the right half and the top half.
    recrop_canvas(&mut kind, [0.5, 0.0, 1.0, 0.5]);
    let FeatureKind::Canvas { center, size, crop, .. } = &kind else { panic!() };
    assert_eq!(*size, [50.0, 25.0]);
    assert_eq!(*center, [25.0, 12.5], "the right/top quarter stays where it was");
    assert_eq!(*crop, [0.5, 0.0, 1.0, 0.5]);
    // Crop edges can't cross.
    recrop_canvas(&mut kind, [0.9, 0.4, 0.2, 0.1]);
    let FeatureKind::Canvas { crop, .. } = &kind else { panic!() };
    assert!(crop[2] > crop[0] && crop[3] > crop[1], "{crop:?}");
}

#[test]
fn appearance_alone_is_not_a_geometry_change() {
    let doc = Document::sample();
    let mut colored = doc.clone();
    colored.features[1].color = Some([0.8, 0.2, 0.2]);
    assert!(colored.same_except_appearance(&doc));
    let mut moved = colored.clone();
    moved.marker -= 1;
    assert!(!moved.same_except_appearance(&doc));
    let mut renamed = colored.clone();
    renamed.features[0].name = "Other".into();
    assert!(!renamed.same_except_appearance(&doc));
}

#[test]
fn a_last_joint_moves_without_regenerating() {
    let mut doc = two_boxes();
    let j = hinge(&mut doc, -90.0);
    doc.push(FeatureKind::Axis { origin: [0.0; 3], dir: [1.0, 0.0, 0.0] });
    let mut moved = doc.clone();
    moved.set_joint_value(j, -30.0);
    assert!(moved.only_joint_moved(&doc, j));

    let mut other = moved.clone();
    other.features[0].suppressed = true;
    assert!(!other.only_joint_moved(&doc, j), "anything else changing needs a rebuild");

    // A later joint on the same body: rotations don't commute.
    let mut two = doc.clone();
    hinge(&mut two, 0.0);
    let mut two_moved = two.clone();
    two_moved.set_joint_value(j, -30.0);
    assert!(!two_moved.only_joint_moved(&two, j));

    // A later feature building on the moved body.
    let mut cut = doc.clone();
    cut.push(FeatureKind::Extrude {
        sketch: Sketch::Rect { plane: OriginPlane::XY, offset: 0.0, center: [0.0, 0.0], size: [1.0, 1.0] },
        distance: 1.0,
        symmetric: false,
        operation: Operation::Cut,
    });
    let mut cut_moved = cut.clone();
    cut_moved.set_joint_value(j, -30.0);
    assert!(!cut_moved.only_joint_moved(&cut, j));
}

#[test]
fn cached_regeneration_matches_a_fresh_one() {
    let summary = |r: &Regen<String>| {
        let bodies: Vec<_> = r.bodies.iter().map(|b| (b.name.clone(), b.solid.clone(), b.created_by)).collect();
        (bodies, r.status.clone())
    };
    let mut cache = RegenCache::default();
    let mut doc = Document::sample();
    let check = |doc: &Document, cache: &mut RegenCache<String>| {
        assert_eq!(summary(&doc.regenerate_cached(&Fake, cache)), summary(&doc.regenerate(&Fake)));
    };
    check(&doc, &mut cache);
    assert_eq!(cache.len(), 4);
    check(&doc, &mut cache);

    // An edit in the middle, a failing feature, and back.
    let FeatureKind::Extrude { distance, .. } = &mut doc.features[1].kind else { panic!() };
    *distance = 30.0;
    check(&doc, &mut cache);
    let before_edit = cache.len();
    let FeatureKind::Fillet { radius, .. } = &mut doc.features[3].kind else { panic!() };
    *radius = 40.0;
    check(&doc, &mut cache);
    let FeatureKind::Fillet { radius, .. } = &mut doc.features[3].kind else { panic!() };
    *radius = 2.0;
    check(&doc, &mut cache);
    assert_eq!(cache.len(), before_edit + 1, "undoing an edit finds the old model");

    // Rolling back keeps the later steps for rolling forward again.
    let kept = cache.len();
    doc.marker = 2;
    check(&doc, &mut cache);
    doc.marker = 4;
    check(&doc, &mut cache);
    assert_eq!(cache.len(), kept, "nothing new was built");

    // Suppressing, and renaming (which doesn't rebuild anything).
    doc.features[2].suppressed = true;
    check(&doc, &mut cache);
    doc.features[0].body_name = Some("Base".into());
    check(&doc, &mut cache);
    assert_eq!(doc.regenerate_cached(&Fake, &mut cache).bodies[0].name, "Base");
}


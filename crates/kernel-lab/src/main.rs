// Scratch experiments against the real kernel: `cargo run --release -p kernel-lab`.
// With no arguments: validity of the sample's bodies per BRepCheck (and after
// ShapeFix). `kernel-lab probe x` threads shafts of a few lengths.
// `kernel-lab showcase out.fission` writes a design using the newer
// features (thread, coil, sphere, torus, split, offset/delete face, construction
// points, and a sketch drawn with 3-point/center shapes and Mirror), for
// checking them by eye.
use fission_kernel::Kernel;
use fission_model::sketch2d::{Constraint, Sketch as Sketch2d};
use fission_model::{Document, FaceRef, FeatureKind, Operation, OriginPlane, Sketch};
use fission_occt::Occt;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let [_, cmd, out] = args.as_slice() {
        if cmd == "bracket" {
            bracket_probe();
            return;
        }
        if cmd == "probe" {
            thread_probe();
            return;
        }
        if cmd == "showcase" {
            let doc = showcase();
            std::fs::write(out, doc.to_ron().unwrap()).unwrap();
            let regen = doc.regenerate(&Occt);
            for (f, s) in doc.features.iter().zip(&regen.status) {
                println!("{:<14} {s:?}", f.name);
            }
            for b in &regen.bodies {
                let mesh = Occt.tessellate(&b.solid, 0.05);
                let tris = mesh.as_ref().map_or(0, |m| m.indices.len() / 3);
                println!("{:<6} volume {:>10.2}  faces {:>3}  triangles {tris}  valid {:?}", b.name, Occt.volume(&b.solid), Occt.face_count(&b.solid), fission_occt::validity(&b.solid));
            }
            return;
        }
    }
    let k = Occt;
    for marker in 1..=4 {
        let mut doc = Document::sample();
        doc.marker = marker;
        let body = doc.regenerate(&k).bodies.remove(0).solid;
        let (valid, fixed) = fission_occt::validity(&body);
        println!("marker {marker}: valid={valid} after_fix={fixed}");
    }
}

/// A face of the body `owner` made, chosen by `pick` from the current model.
fn face_of(doc: &Document, owner: usize, pick: impl Fn(&fission_kernel::FaceHint) -> bool) -> FaceRef {
    let regen = doc.regenerate(&Occt);
    let body = regen.bodies.iter().find(|b| b.created_by == owner).expect("body");
    let mesh = Occt.tessellate(&body.solid, 0.1).unwrap();
    let hint = *mesh.face_hints.iter().find(|h| pick(h)).expect("face");
    FaceRef { body: owner, center: hint.center, normal: hint.normal }
}

fn showcase() -> Document {
    let mut doc = Document::default();
    let rect = |center: [f64; 2], size: [f64; 2]| Sketch::Rect { plane: OriginPlane::XY, offset: 0.0, center, size };

    // An M10 bolt shank, threaded.
    let shaft = doc.push(FeatureKind::Extrude {
        sketch: Sketch::Circle { plane: OriginPlane::XY, offset: 0.0, center: [0.0, 0.0], radius: 5.0 },
        distance: 25.0,
        symmetric: false,
        operation: Operation::NewBody,
    });
    let side = face_of(&doc, shaft, |h| h.normal[2].abs() < 1e-6);
    doc.push(FeatureKind::Thread { face: side, size: 10.0, pitch: 1.5, left_hand: false });

    // A plate with a tapped M8 hole and one hole healed shut with Delete.
    let plate = doc.push(FeatureKind::Extrude { sketch: rect([40.0, 0.0], [40.0, 30.0]), distance: 8.0, symmetric: false, operation: Operation::NewBody });
    for x in [32.0, 48.0] {
        doc.push(FeatureKind::Hole {
            plane: OriginPlane::XY,
            offset: 8.0,
            center: [x, 0.0],
            flip: false,
            diameter: 6.8,
            depth: 8.0,
            through: true,
            kind: fission_model::HoleKind::Simple,
            tip_angle: 0.0,
        });
        let hole = doc.features.len() - 1;
        doc.features[hole].target = Some(plate);
    }
    let wall = |x: f64| move |h: &fission_kernel::FaceHint| h.normal[2].abs() < 1e-6 && (h.center[0] - x).abs() < 4.0 && h.center[1].abs() < 4.0;
    let healed = face_of(&doc, plate, wall(48.0));
    doc.push(FeatureKind::DeleteFace { faces: vec![healed] });
    // Pull the plate's +X end out 6.
    let end = face_of(&doc, plate, |h| h.normal[0] > 0.99);
    doc.push(FeatureKind::OffsetFace { faces: vec![end], distance: 6.0 });
    // Threads last: OCCT's offset and defeaturing can crash on helical faces.
    let tapped = face_of(&doc, plate, wall(32.0));
    doc.push(FeatureKind::Thread { face: tapped, size: 8.0, pitch: 1.25, left_hand: false });

    // A spring, a ball and a ring.
    doc.push(FeatureKind::Coil {
        plane: OriginPlane::XY,
        offset: 0.0,
        center: [0.0, 40.0],
        diameter: 20.0,
        pitch: 5.0,
        turns: 5.0,
        section: 2.0,
        left_hand: false,
        operation: Operation::NewBody,
    });
    doc.push(FeatureKind::Sphere { plane: OriginPlane::XY, offset: 10.0, center: [-35.0, 0.0], diameter: 20.0, operation: Operation::NewBody });
    doc.push(FeatureKind::Torus { plane: OriginPlane::XY, offset: 4.0, center: [40.0, 40.0], diameter: 28.0, section: 8.0, operation: Operation::NewBody });

    // A block split down the middle, the far half nudged away.
    let block = doc.push(FeatureKind::Extrude { sketch: rect([-35.0, 40.0], [24.0, 16.0]), distance: 12.0, symmetric: false, operation: Operation::NewBody });
    let split = doc.push(FeatureKind::Split { body: Some(block), plane: OriginPlane::YZ, offset: -35.0 });
    doc.push(FeatureKind::Move { bodies: vec![split], translate: [4.0, 0.0, 0.0], axis: fission_model::Axis::Z, degrees: 0.0, copy: false });

    // A sketch drawn with the new tools.
    let mut sk = Sketch2d::default();
    let axis = sk.add_line([0.0, -70.0], [0.0, -40.0]);
    sk.curves[axis].set_construction(true);
    let (a0, a1) = sk.line_points(axis);
    sk.constraints.extend([Constraint::Fixed(a0), Constraint::Fixed(a1)]);
    let (p, q) = (sk.add_point([6.0, -66.0]), sk.add_point([18.0, -60.0]));
    let tilted = sk.add_three_point_rectangle(p, q, [12.0, -54.0]);
    let c = sk.add_point([-25.0, -55.0]);
    sk.add_center_rectangle(c, [-19.0, -51.0]);
    let circle = sk.add_circle_through([[10.0, -45.0], [16.0, -45.0], [13.0, -42.0]]).unwrap();
    let (s, e) = (sk.add_point([4.0, -48.0]), sk.add_point([4.0, -40.0]));
    let arc = sk.add_arc_through(s, e, [9.0, -44.0]).unwrap();
    let mut mirrored = tilted.to_vec();
    mirrored.extend([circle, arc]);
    sk.mirror(&mirrored, axis);
    sk.solve().unwrap();
    doc.push(FeatureKind::Sketch { plane: OriginPlane::XY, offset: 0.0, sketch: sk });

    doc.push(FeatureKind::Point { pos: [0.0, 0.0, 25.0] });
    doc
}

/// Thread a shaft at a few lengths and pitches (`kernel-lab probe x`).
fn thread_probe() {
    let k = Occt;
    for (len, pitch) in [(20.0, 1.5), (25.0, 1.5), (24.0, 1.5), (25.0, 1.25), (21.0, 1.5)] {
        let shaft = k
            .extrude(&fission_kernel::Profile::Circle { plane: fission_kernel::Plane::XY, center: [0.0, 0.0], radius: 5.0 }, len)
            .unwrap();
        let mesh = k.tessellate(&shaft, 0.1).unwrap();
        let side = *mesh.face_hints.iter().find(|h| h.normal[2].abs() < 1e-6).unwrap();
        let v = k.thread(&shaft, &side, pitch, false).map(|t| k.volume(&t));
        println!("len {len} pitch {pitch}: faces {} -> {v:?} of {}", k.face_count(&shaft), k.volume(&shaft));
    }
}

/// The sample bracket: thread its hole, then offset the upright's top
/// (`kernel-lab bracket x`), timing each rebuild.
fn bracket_probe() {
    let mut doc = Document::sample();
    let owner = 0;
    {
        let mut plain = Document::sample();
        let top = face_of(&plain, owner, |h| h.normal[2] > 0.99 && h.center[2] > 36.0);
        let before = Occt.volume(&plain.regenerate(&Occt).bodies[0].solid);
        plain.push(FeatureKind::OffsetFace { faces: vec![top], distance: 5.0 });
        let t = std::time::Instant::now();
        let r = plain.regenerate(&Occt);
        println!("offset alone: {:?} in {:?}, volume {before} -> {}", r.status.last(), t.elapsed(), Occt.volume(&r.bodies[0].solid));
    }
    let regen = doc.regenerate(&Occt);
    let mesh = Occt.tessellate(&regen.bodies[0].solid, 0.1).unwrap();
    let wall = mesh.face_hints[39];
    for h in mesh.face_hints.iter().filter(|h| h.normal[2].abs() < 1e-6 && h.normal[0].abs() > 0.05 && h.normal[0].abs() < 0.95) {
        let r = Occt.delete_faces(&regen.bodies[0].solid, &[*h]);
        println!("delete round {h:?}: {:?}", r.map(|s| (Occt.face_count(&s), Occt.volume(&s))));
    }
    let round = mesh.face_hints[19];
    let sharp = Occt.delete_faces(&regen.bodies[0].solid, &[round]);
    println!("delete fillet 19 {round:?}: {:?}", sharp.map(|s| (Occt.face_count(&s), Occt.volume(&s), Occt.face_count(&regen.bodies[0].solid), Occt.volume(&regen.bodies[0].solid))));
    println!("hole wall hint {wall:?}");
    doc.push(FeatureKind::Thread { face: FaceRef { body: owner, center: wall.center, normal: wall.normal }, size: 8.0, pitch: 1.25, left_hand: false });
    let t = std::time::Instant::now();
    let r = doc.regenerate(&Occt);
    println!("thread: {:?} in {:?}, volume {}", r.status.last(), t.elapsed(), Occt.volume(&r.bodies[0].solid));
    let top = face_of(&doc, owner, |h| h.normal[2] > 0.99 && h.center[2] > 36.0);
    doc.push(FeatureKind::OffsetFace { faces: vec![top], distance: 5.0 });
    let t = std::time::Instant::now();
    let r = doc.regenerate(&Occt);
    println!("offset: {:?} in {:?}, volume {}", r.status.last(), t.elapsed(), Occt.volume(&r.bodies[0].solid));
}

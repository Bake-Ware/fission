//! Debug: an imported sketch's points, curves and regions.
//! `f3d_sketch file.f3d SketchName`
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let t = fission_f3d::read_timeline(std::path::Path::new(&a[1])).unwrap();
    let import = fission_model::f3d::document(&t, "x", &fission_occt::Occt);
    for f in &import.doc.features {
        if f.name != a[2] {
            continue;
        }
        if let fission_model::FeatureKind::Sketch { sketch, .. } = &f.kind {
            for (i, c) in sketch.curves.iter().enumerate() {
                let pts: Vec<[f64; 2]> = c.points().iter().map(|&p| sketch.points[p]).collect();
                println!("curve {i}: {c:?} {pts:?}");
            }
            if let Some(keep) = a.get(3) {
                let keep: Vec<usize> = keep.split(',').map(|k| k.parse().unwrap()).collect();
                let mut sk = sketch.clone();
                for (i, c) in sk.curves.iter_mut().enumerate() {
                    if !keep.contains(&i) {
                        c.set_construction(true);
                    }
                }
                for r in sk.regions() {
                    println!("subset region area {:.4} holes {}", r.area, r.holes.len());
                }
            }
            for r in sketch.regions() {
                println!("region area {:.4} holes {} outer segs {}", r.area, r.holes.len(), r.outer.segments.len());
            }
        }
    }
}

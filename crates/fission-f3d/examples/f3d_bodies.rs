//! Per-body summary of an .f3d: face count, bounding box (mm).
fn main() {
    let path = std::env::args().nth(1).expect("usage: f3d_bodies FILE.f3d");
    let model = fission_f3d::read_f3d(std::path::Path::new(&path)).unwrap();
    let mut hist = std::collections::BTreeMap::new();
    for (i, b) in model.bodies.iter().enumerate() {
        *hist.entry(b.faces.len().min(20)).or_insert(0) += 1;
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        for v in &b.vertices {
            for k in 0..3 {
                lo[k] = lo[k].min(v[k]);
                hi[k] = hi[k].max(v[k]);
            }
        }
        if b.faces.len() >= 20 || i < 5 {
            println!("body {i}: {} faces, {} verts, size {:.1} x {:.1} x {:.1}, transform {}", b.faces.len(), b.vertices.len(), hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2], b.transform.is_some());
        }
    }
    let closed = model.bodies.iter().filter(|b| b.closed).count();
    println!("closed bodies {closed}, open {}", model.bodies.len() - closed);
    println!("faces-per-body histogram (20 = 20+): {hist:?}");
}

//! Pairwise overlap volumes of a STEP file's solids: `overlap file.step`.
use fission_kernel::{BooleanOp, Kernel};
use fission_occt::Occt;
fn main() {
    let path = std::env::args().nth(1).expect("usage: overlap FILE.step");
    let k = Occt;
    let parts = k.import_parts(std::path::Path::new(&path)).unwrap();
    for (i, p) in parts.iter().enumerate() {
        println!("solid {i}: volume {:.1} faces {} valid {:?}", k.volume(p), k.face_count(p), fission_occt::validity(p));
    }
    for i in 0..parts.len() {
        for j in i + 1..parts.len() {
            let v = k.boolean(&parts[i], &parts[j], BooleanOp::Intersect).map(|s| k.volume(&s));
            println!("overlap {i}&{j}: {v:?}");
        }
    }
}

//! Print what an .f3d holds and which geometry types aren't supported yet.
//! Usage: cargo run --release -p fission-f3d --example f3d_info -- FILE.f3d

fn main() {
    let path = std::env::args().nth(1).expect("usage: f3d_info FILE.f3d");
    match fission_f3d::read_f3d(std::path::Path::new(&path)) {
        Ok(model) => {
            println!("bodies {}  faces {}  supported {}", model.bodies.len(), model.face_count(), model.supported_face_count());
            for (kind, n) in &model.unsupported {
                println!("  unsupported {kind}: {n}");
            }
        }
        Err(e) => println!("error: {e}"),
    }
}

//! Print an .f3d's timeline: `cargo run -p fission-f3d --example f3d_timeline -- file.f3d`
use fission_f3d::design::FeatureData;

fn main() {
    for path in std::env::args().skip(1) {
        let t = match fission_f3d::read_timeline(std::path::Path::new(&path)) {
            Ok(t) => t,
            Err(e) => {
                println!("{path}: {e}");
                continue;
            }
        };
        println!("{path}: {} parameters, {} features", t.parameters.len(), t.features.len());
        for p in &t.parameters {
            println!("  param {:<6} = {:<14} {:<22} {}", p.name, p.expression, p.role, p.value);
        }
        for f in &t.features {
            match &f.data {
                FeatureData::Sketch(s) => {
                    println!("  {:<16} sketch data {} curves {} matrix {:?}", f.name, s.data, s.curves.len(), &s.matrix[..12]);
                    if std::env::var_os("CURVES").is_some() {
                        for c in &s.curves {
                            println!("      {c:?}");
                        }
                    }
                }
                FeatureData::Extrude(e) => println!("  {:<16} id {} {:?} parts {:?} dir {} dist {:?} against {:?} sketch {} profiles {:?}", f.name, f.id, e.operation, e.participants, e.direction, e.distance, e.against, e.sketch, e.regions.iter().map(|r| r.iter().map(Vec::len).collect::<Vec<_>>()).collect::<Vec<_>>()),
                FeatureData::Unsupported => println!("  {:<16} (not decoded) id {}", f.name, f.id),
                other => println!("  {:<16} {other:?}", f.name),
            }
        }
    }
}

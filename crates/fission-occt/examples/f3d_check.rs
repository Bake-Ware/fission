//! Import .f3d timelines and check them against Fusion's own bodies:
//! `cargo run --release -p fission-occt --example f3d_check -- files...`
//! For each file: features replayed / skipped, and the volume Fission's
//! rebuild gives against the volume of the B-rep Fusion saved.
use fission_kernel::Kernel;
use fission_occt::Occt;

fn main() {
    let k = Occt;
    let (mut full, mut exact) = (0, 0);
    let files: Vec<String> = std::env::args().skip(1).collect();
    for path in &files {
        let p = std::path::Path::new(path);
        let name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let t = match fission_f3d::read_timeline(p) {
            Ok(t) => t,
            Err(e) => {
                println!("{name}: no timeline ({e})");
                continue;
            }
        };
        let import = fission_model::f3d::document(&t, &name, &k);
        let regen = import.doc.regenerate(&k);
        let ours: f64 = regen.bodies.iter().map(|b| k.volume(&b.solid)).sum();
        let errors: Vec<String> = import
            .doc
            .features
            .iter()
            .zip(&regen.status)
            .filter_map(|(f, s)| match s {
                fission_model::FeatureStatus::Error(e) => Some(format!("{}: {e}", f.name)),
                _ => None,
            })
            .collect();
        if std::env::var_os("VERBOSE").is_some() {
            // Volume after each feature.
            let mut d = import.doc.clone();
            for i in 0..d.features.len() {
                d.marker = i + 1;
                let r = d.regenerate(&k);
                let v: f64 = r.bodies.iter().map(|b| k.volume(&b.solid)).sum();
                println!("  {:<16} {:>14.1}  bodies {}  {:?}", d.features[i].name, v, r.bodies.len(), r.status[i]);
            }
        }
        if std::env::var_os("BODIES").is_some() {
            let mut theirs: Vec<f64> = k.import_parts(p).map(|parts| parts.iter().map(|s| k.volume(s)).collect()).unwrap_or_default();
            theirs.sort_by(|a, b| b.total_cmp(a));
            let mut ours: Vec<(f64, String)> = regen.bodies.iter().map(|b| (k.volume(&b.solid), import.doc.features[b.created_by].name.clone())).collect();
            ours.sort_by(|a, b| b.0.total_cmp(&a.0));
            println!("  fusion bodies {theirs:.1?}");
            println!("  fusion final  {:.1?}", t.bodies);
            println!("  our bodies    {ours:.1?}");
        }
        // Fusion's own measure of the bodies it ends with, else the B-rep.
        let stored: Option<f64> = t.bodies.iter().copied().sum();
        let theirs: f64 = match stored {
            Some(v) if !t.bodies.is_empty() => v,
            _ => k.import_parts(p).map(|parts| parts.iter().map(|s| k.volume(s)).sum()).unwrap_or(f64::NAN),
        };
        let ratio = ours / theirs;
        let complete = import.skipped.is_empty() && errors.is_empty();
        full += complete as usize;
        let ok = complete && (ratio - 1.0).abs() < 1e-4;
        exact += ok as usize;
        println!(
            "{name}: {} features, skipped {:?}, errors {:?}, notes {:?}, volume {ours:.3} / {theirs:.3} = {ratio:.5}{}",
            import.doc.features.len(),
            import.skipped,
            errors,
            import.notes,
            if ok { " OK" } else { "" }
        );
    }
    println!("{} files: {full} fully replayed, {exact} match Fusion's volume", files.len());
}

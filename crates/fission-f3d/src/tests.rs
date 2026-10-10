use super::*;

/// Real Fusion designs (not redistributable, so tests skip when they're
/// absent). They're looked up in `$FISSION_SAMPLES`, else `out/f3d`.
fn local(name: &str) -> Option<std::path::PathBuf> {
    let dir = std::env::var_os("FISSION_SAMPLES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/f3d"));
    let p = dir.join(name);
    p.exists().then_some(p)
}

const FLATHEX: &str = "flathex.f3d";
const CLOUDS: &str = "clouds.f3d";

#[test]
fn flathex_screw_is_fully_analytic() {
    let Some(path) = local(FLATHEX) else { return };
    let model = read_f3d(&path).unwrap();
    assert!(!model.bodies.is_empty());
    assert_eq!(model.supported_face_count(), model.face_count(), "unsupported: {:?}", model.unsupported);
    // Every coedge points at a real edge, every edge at real vertices.
    for body in &model.bodies {
        for face in &body.faces {
            for lp in &face.loops {
                assert!(!lp.is_empty());
                for &(e, _) in lp {
                    let edge = &body.edges[e];
                    assert!(edge.start < body.vertices.len() && edge.end < body.vertices.len());
                }
            }
        }
    }
    // Sizes are in mm: the screw's plane at z = 0.235 cm.
    let has_mm_plane = model.bodies.iter().flat_map(|b| &b.faces).any(|f| {
        matches!(f.surface, Surface::Plane { origin, .. } if (origin[2] - 2.35).abs() < 1e-9)
    });
    assert!(has_mm_plane);
}

#[test]
fn clouds_box_reads_with_zstd() {
    let Some(path) = local(CLOUDS) else { return };
    let model = read_f3d(&path).unwrap();
    assert!(model.face_count() > 300, "{}", model.face_count());
    assert_eq!(model.supported_face_count(), model.face_count(), "unsupported: {:?}", model.unsupported);
}

#[test]
fn missing_geometry_is_an_error() {
    let dir = std::env::temp_dir().join("fission-f3d-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("empty.f3d");
    let file = std::fs::File::create(&path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    zip.start_file("Manifest.dat", zip::write::SimpleFileOptions::default()).unwrap();
    std::io::Write::write_all(&mut zip, b"x").unwrap();
    zip.finish().unwrap();
    assert!(matches!(read_f3d(&path), Err(F3dError::NoGeometry)));
}

/// de Boor evaluation of a clamped B-spline at `t`.
fn eval(b: &BSpline, t: f64) -> P3 {
    let knots: Vec<f64> = b.knots.iter().zip(&b.mults).flat_map(|(&k, &m)| std::iter::repeat_n(k, m as usize)).collect();
    let p = b.degree as usize;
    let n = b.poles.len();
    let k = (p..n).rfind(|&i| knots[i] <= t).unwrap_or(p).min(n - 1);
    let w = |i: usize| b.weights.as_ref().map_or(1.0, |w| w[i]);
    let mut d: Vec<[f64; 4]> =
        (0..=p).map(|j| { let q = b.poles[j + k - p]; let wi = w(j + k - p); [q[0] * wi, q[1] * wi, q[2] * wi, wi] }).collect();
    for r in 1..=p {
        for j in (r..=p).rev() {
            let i = j + k - p;
            let den = knots[i + p - r + 1] - knots[i];
            let a = if den.abs() < 1e-15 { 0.0 } else { (t - knots[i]) / den };
            let prev = d[j - 1];
            for (c, v) in d[j].iter_mut().enumerate() {
                *v = (1.0 - a) * prev[c] + a * *v;
            }
        }
    }
    [d[p][0] / d[p][3], d[p][1] / d[p][3], d[p][2] / d[p][3]]
}

fn dist_to_curve(b: &BSpline, q: P3) -> f64 {
    let (a, z) = (b.knots[0], *b.knots.last().unwrap());
    (0..=2000)
        .map(|i| eval(b, a + (z - a) * i as f64 / 2000.0))
        .map(|p| ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt())
        .fold(f64::INFINITY, f64::min)
}

const CAMERA: &str = "camera.f3d";

#[test]
fn camera_housing_splines_pass_through_their_vertices() {
    let Some(path) = local(CAMERA) else { return };
    let model = read_f3d(&path).unwrap();
    assert_eq!(model.supported_face_count(), model.face_count(), "unsupported: {:?}", model.unsupported);
    let (mut checked, mut off) = (0, Vec::new());
    for body in &model.bodies {
        for e in &body.edges {
            if let Curve::BSpline(b) = &e.curve {
                checked += 1;
                let d = dist_to_curve(b, body.vertices[e.start]).max(dist_to_curve(b, body.vertices[e.end]));
                if d > 0.01 {
                    off.push(d);
                }
            }
        }
    }
    assert!(checked > 100, "{checked}");
    assert!(off.is_empty(), "{} of {checked} spline edges miss their vertices: {:?}", off.len(), &off[..off.len().min(10)]);
}

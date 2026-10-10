//! ISO metric thread sizes, for picking a thread from a face's diameter.

/// Nominal diameter and coarse pitch, in mm (ISO 261/262).
pub const ISO_COARSE: [(f64, f64); 16] = [
    (1.6, 0.35),
    (2.0, 0.4),
    (2.5, 0.45),
    (3.0, 0.5),
    (4.0, 0.7),
    (5.0, 0.8),
    (6.0, 1.0),
    (8.0, 1.25),
    (10.0, 1.5),
    (12.0, 1.75),
    (14.0, 2.0),
    (16.0, 2.0),
    (20.0, 2.5),
    (24.0, 3.0),
    (30.0, 3.5),
    (36.0, 4.0),
];

/// The size that fits a cylinder of `diameter`: a shaft is the nominal size;
/// a hole is drilled to about the minor diameter (nominal − 1.0825·pitch).
pub fn iso_size_for(diameter: f64, internal: bool) -> (f64, f64) {
    let fit = |&(d, p): &(f64, f64)| if internal { d - 1.0825 * p } else { d };
    *ISO_COARSE.iter().min_by(|a, b| (fit(a) - diameter).abs().total_cmp(&(fit(b) - diameter).abs())).unwrap()
}

/// "M10x1.5".
pub fn designation(size: f64, pitch: f64) -> String {
    format!("M{}x{}", trim(size), trim(pitch))
}

fn trim(v: f64) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_come_from_shafts_and_tap_drills() {
        assert_eq!(iso_size_for(10.0, false), (10.0, 1.5));
        assert_eq!(iso_size_for(9.9, false), (10.0, 1.5));
        // M8's tap drill is 6.8 mm.
        assert_eq!(iso_size_for(6.8, true), (8.0, 1.25));
        assert_eq!(designation(10.0, 1.5), "M10x1.5");
        assert_eq!(designation(1.6, 0.35), "M1.6x0.35");
    }
}

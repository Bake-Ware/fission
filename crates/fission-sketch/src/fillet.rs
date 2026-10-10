//! Sketch fillet: round the corner where two lines meet with a tangent arc,
//! as Fusion's sketch Fillet does.

use std::f64::consts::{PI, TAU};

use crate::{dist, Constraint, Curve, Sketch, P2};

impl Sketch {
    /// The two lines with an end at `corner` (the point itself or one lying
    /// on top of it), as (line, end is `b`).
    pub fn corner_lines(&self, corner: usize) -> Option<[(usize, bool); 2]> {
        let at = self.points[corner];
        let mut found = Vec::new();
        for (i, c) in self.curves.iter().enumerate() {
            if let Curve::Line { a, b, .. } = *c {
                if dist(self.points[a], at) < 1e-9 {
                    found.push((i, false));
                } else if dist(self.points[b], at) < 1e-9 {
                    found.push((i, true));
                }
            }
        }
        (found.len() == 2).then(|| [found[0], found[1]])
    }

    /// A sensible starting radius: a quarter of the shorter line.
    pub fn default_fillet_radius(&self, corner: usize) -> Option<f64> {
        let [(l1, _), (l2, _)] = self.corner_lines(corner)?;
        Some(self.line_length(l1).min(self.line_length(l2)) * 0.25)
    }

    /// Round the corner at `corner` with an arc of `radius`. Returns the
    /// arc's curve index and the index of its radius dimension.
    pub fn fillet_corner(&mut self, corner: usize, radius: f64) -> Result<(usize, usize), String> {
        if radius.is_nan() || radius <= 0.0 {
            return Err("fillet radius must be positive".into());
        }
        let [(l1, end1), (l2, end2)] = self.corner_lines(corner).ok_or("a fillet needs exactly two lines meeting at the corner")?;
        let c = self.points[corner];
        // Unit directions from the corner along each line.
        let away = |line: usize, end_b: bool| -> (P2, f64) {
            let (a, b) = self.line_points(line);
            let far = if end_b { self.points[a] } else { self.points[b] };
            let len = dist(c, far);
            ([(far[0] - c[0]) / len, (far[1] - c[1]) / len], len)
        };
        let ((u1, len1), (u2, len2)) = (away(l1, end1), away(l2, end2));
        let cos = (u1[0] * u2[0] + u1[1] * u2[1]).clamp(-1.0, 1.0);
        let theta = cos.acos();
        if theta < 1e-6 || (PI - theta) < 1e-6 {
            return Err("the lines are parallel at the corner".into());
        }
        // Tangent points sit `d` from the corner; the center is along the bisector.
        let d = radius / (theta / 2.0).tan();
        if d >= len1 || d >= len2 {
            return Err("the radius is too big for these lines".into());
        }
        let t1 = [c[0] + u1[0] * d, c[1] + u1[1] * d];
        let t2 = [c[0] + u2[0] * d, c[1] + u2[1] * d];
        let bis = [u1[0] + u2[0], u1[1] + u2[1]];
        let bl = (bis[0] * bis[0] + bis[1] * bis[1]).sqrt();
        let h = radius / (theta / 2.0).sin();
        let center = [c[0] + bis[0] / bl * h, c[1] + bis[1] / bl * h];

        let (p1, p2, pc) = (self.add_point(t1), self.add_point(t2), self.add_point(center));
        for (line, end_b, p) in [(l1, end1, p1), (l2, end2, p2)] {
            if let Curve::Line { a, b, .. } = &mut self.curves[line] {
                if end_b { *b = p } else { *a = p }
            }
        }
        // Counter-clockwise the short way round.
        let ang = |p: P2| (p[1] - center[1]).atan2(p[0] - center[0]);
        let (start, end) = if (ang(t2) - ang(t1)).rem_euclid(TAU) <= PI { (p1, p2) } else { (p2, p1) };
        self.curves.push(Curve::Arc { center: pc, start, end, construction: false });
        let arc = self.curves.len() - 1;
        self.constraints.push(Constraint::Tangent(l1, arc));
        self.constraints.push(Constraint::Tangent(l2, arc));
        self.constraints.push(Constraint::Radius(arc, radius));
        let dim = self.constraints.len() - 1;
        // The old corner goes, and with it the constraints that pinned it.
        self.remove_unused_points();
        let dim = self.constraints.iter().position(|k| *k == Constraint::Radius(arc, radius)).unwrap_or(dim);
        Ok((arc, dim))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangle_corner_gets_a_tangent_arc() {
        let mut s = Sketch::default();
        let [bottom, right, ..] = s.add_rectangle([0.0, 0.0], [20.0, 10.0]);
        let corner = s.line_points(bottom).1; // (20, 0)
        let (arc, _) = s.fillet_corner(corner, 3.0).unwrap();
        s.solve().unwrap();
        assert!((s.radius(arc) - 3.0).abs() < 1e-6);
        let Curve::Arc { center, start, end, .. } = s.curves[arc] else { panic!() };
        assert!(dist(s.points[center], [17.0, 3.0]) < 1e-6);
        // Lines now end where the arc starts and stops.
        assert!(dist(s.points[s.line_points(bottom).1], [17.0, 0.0]) < 1e-6);
        assert!(dist(s.points[s.line_points(right).0], [20.0, 3.0]) < 1e-6);
        assert!([start, end].contains(&s.line_points(bottom).1));
        // Still one closed region, now with the rounded corner.
        assert_eq!(s.regions().len(), 1);
        let area = s.regions()[0].area;
        let expected = 200.0 - (9.0 - PI * 9.0 / 4.0);
        assert!((area - expected).abs() < 0.05, "{area} vs {expected}");
    }

    #[test]
    fn too_big_a_radius_is_refused() {
        let mut s = Sketch::default();
        let [bottom, ..] = s.add_rectangle([0.0, 0.0], [4.0, 4.0]);
        let corner = s.line_points(bottom).1;
        assert!(s.fillet_corner(corner, 10.0).is_err());
        assert_eq!(s.curves.len(), 4, "unchanged");
    }
}

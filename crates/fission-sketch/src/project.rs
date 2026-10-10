//! Project: copy a model edge, already flattened onto the sketch plane,
//! into the sketch as fixed geometry, like Fusion's Project. Straight
//! edges become lines, circular ones circles or arcs, anything else a
//! chain of short lines.

use crate::shapes::circle_through;
use crate::{dist, Constraint, Curve, Sketch, P2};

impl Sketch {
    /// Add the flattened edge `pts` and fix it in place. Ends that land on
    /// an existing point reuse it, so projected edges join into profiles.
    /// Returns the new curves (empty if the edge flattens to a point).
    pub fn project_polyline(&mut self, pts: &[P2]) -> Vec<usize> {
        let mut clean: Vec<P2> = Vec::with_capacity(pts.len());
        for &p in pts {
            if clean.last().is_none_or(|&q| dist(p, q) > 1e-9) {
                clean.push(p);
            }
        }
        let pts = clean;
        let extent = pts_extent(&pts);
        let tol = 1e-4 * extent.max(1.0);
        if extent < tol {
            return Vec::new();
        }
        let closed = pts.len() > 2 && dist(pts[0], *pts.last().unwrap()) < tol;

        // Straight: everything within tol of the line through the two
        // farthest points (also an edge-on circle, which flattens to a line).
        let q = farthest(&pts, pts[0]);
        let r = farthest(&pts, q);
        if pts.iter().all(|&p| line_dist(p, q, r) < tol) {
            let (a, b) = (self.fixed_point(q, tol), self.fixed_point(r, tol));
            self.curves.push(Curve::Line { a, b, construction: false });
            return vec![self.curves.len() - 1];
        }

        let n = pts.len();
        if let Some((c, radius)) = circle_through(pts[0], pts[n / 3], pts[2 * n / 3]) {
            if pts.iter().all(|&p| (dist(p, c) - radius).abs() < tol) {
                let center = self.fixed_point(c, tol);
                if closed {
                    self.curves.push(Curve::Circle { center, radius, construction: false });
                    self.constraints.push(Constraint::Radius(self.curves.len() - 1, radius));
                    return vec![self.curves.len() - 1];
                }
                // Arcs run counter-clockwise from start to end.
                let (mut s, mut e) = (pts[0], pts[n - 1]);
                let m = pts[n / 2];
                if cross(sub(s, c), sub(m, c)) < 0.0 {
                    std::mem::swap(&mut s, &mut e);
                }
                let on = |p: P2| {
                    let d = sub(p, c);
                    let k = radius / dist(p, c);
                    [c[0] + d[0] * k, c[1] + d[1] * k]
                };
                let (start, end) = (self.fixed_point(on(s), tol), self.fixed_point(on(e), tol));
                self.curves.push(Curve::Arc { center, start, end, construction: false });
                return vec![self.curves.len() - 1];
            }
        }

        // Anything else (an ellipse from a tilted circle, a spline): lines.
        let ids: Vec<usize> = pts.iter().map(|&p| self.fixed_point(p, tol)).collect();
        let mut out = Vec::new();
        for w in ids.windows(2) {
            if w[0] != w[1] {
                self.curves.push(Curve::Line { a: w[0], b: w[1], construction: false });
                out.push(self.curves.len() - 1);
            }
        }
        out
    }

    /// An existing point within `tol` of `p`, or a new one; either way fixed.
    fn fixed_point(&mut self, p: P2, tol: f64) -> usize {
        let found = (0..self.points.len()).find(|&i| dist(self.points[i], p) < tol);
        let i = found.unwrap_or_else(|| self.add_point(p));
        if !self.constraints.iter().any(|c| matches!(c, Constraint::Fixed(j) if *j == i)) {
            self.constraints.push(Constraint::Fixed(i));
        }
        i
    }
}

fn pts_extent(pts: &[P2]) -> f64 {
    let Some(&first) = pts.first() else { return 0.0 };
    let q = farthest(pts, first);
    dist(q, farthest(pts, q))
}

fn farthest(pts: &[P2], from: P2) -> P2 {
    pts.iter().copied().max_by(|a, b| dist(*a, from).total_cmp(&dist(*b, from))).unwrap_or(from)
}

fn sub(a: P2, b: P2) -> P2 {
    [a[0] - b[0], a[1] - b[1]]
}

fn cross(a: P2, b: P2) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn line_dist(p: P2, a: P2, b: P2) -> f64 {
    let len = dist(a, b);
    if len == 0.0 { dist(p, a) } else { cross(sub(b, a), sub(p, a)).abs() / len }
}

//! More ways to draw, as in Fusion's CREATE menu: 3-point and center
//! rectangles, 2- and 3-point circles, 3-point arcs, and Mirror.

use std::collections::HashMap;
use std::f64::consts::TAU;

use crate::{dist, Constraint, Curve, Sketch, P2};

impl Sketch {
    /// A rectangle with one side from point `a` to point `b` and the
    /// opposite side through `p`, at any angle. Corners are shared points;
    /// one perpendicular and two parallel constraints keep it rectangular.
    /// Returns its lines (a→b first, then round the loop).
    pub fn add_three_point_rectangle(&mut self, a: usize, b: usize, p: P2) -> [usize; 4] {
        let (pa, pb) = (self.points[a], self.points[b]);
        let len = dist(pa, pb).max(1e-9);
        let n = [-(pb[1] - pa[1]) / len, (pb[0] - pa[0]) / len];
        let h = (p[0] - pa[0]) * n[0] + (p[1] - pa[1]) * n[1];
        let c = self.add_point([pb[0] + n[0] * h, pb[1] + n[1] * h]);
        let d = self.add_point([pa[0] + n[0] * h, pa[1] + n[1] * h]);
        let mut lines = [0; 4];
        for (k, (s, e)) in [(a, b), (b, c), (c, d), (d, a)].into_iter().enumerate() {
            self.curves.push(Curve::Line { a: s, b: e, construction: false });
            lines[k] = self.curves.len() - 1;
        }
        self.constraints.push(Constraint::Perpendicular(lines[0], lines[1]));
        self.constraints.push(Constraint::Parallel(lines[0], lines[2]));
        self.constraints.push(Constraint::Parallel(lines[1], lines[3]));
        lines
    }

    /// An axis-aligned rectangle centred on point `center` with a corner at
    /// `corner`. A construction diagonal with the center at its midpoint
    /// keeps it centred. Returns the lines (bottom, right, top, left).
    pub fn add_center_rectangle(&mut self, center: usize, corner: P2) -> [usize; 4] {
        let c = self.points[center];
        let lines = self.add_rectangle([2.0 * c[0] - corner[0], 2.0 * c[1] - corner[1]], corner);
        let (from, to) = (self.line_points(lines[0]).0, self.line_points(lines[2]).0);
        self.curves.push(Curve::Line { a: from, b: to, construction: true });
        self.constraints.push(Constraint::Midpoint(center, self.curves.len() - 1));
        lines
    }

    /// A circle through three points (any of them sketch points the circle
    /// should stay on). None if they're in a line.
    pub fn add_circle_through(&mut self, pts: [P2; 3]) -> Option<usize> {
        let (c, r) = circle_through(pts[0], pts[1], pts[2])?;
        let center = self.add_point(c);
        self.curves.push(Curve::Circle { center, radius: r, construction: false });
        Some(self.curves.len() - 1)
    }

    /// An arc from point `start` to point `end` that passes through `via`.
    /// None if the three are in a line.
    pub fn add_arc_through(&mut self, start: usize, end: usize, via: P2) -> Option<usize> {
        let (s, e) = (self.points[start], self.points[end]);
        let (c, _) = circle_through(s, e, via)?;
        let angle = |p: P2| (p[1] - c[1]).atan2(p[0] - c[0]);
        // Arcs run counter-clockwise: swap the ends if `via` lies clockwise.
        let ccw = |from: P2, to: P2| (angle(to) - angle(from)).rem_euclid(TAU);
        let (start, end) = if ccw(s, via) < ccw(s, e) { (start, end) } else { (end, start) };
        let center = self.add_point(c);
        self.curves.push(Curve::Arc { center, start, end, construction: false });
        Some(self.curves.len() - 1)
    }

    /// Mirror `curves` across the line `about`, like Fusion's sketch Mirror:
    /// each copy's points are tied to the originals with Symmetric
    /// constraints (circles also Equal), so edits to the originals carry
    /// over. Points already on the line are shared. Returns the copies.
    pub fn mirror(&mut self, curves: &[usize], about: usize) -> Vec<usize> {
        let Curve::Line { a, b, .. } = self.curves[about] else { return Vec::new() };
        let (la, lb) = (self.points[a], self.points[b]);
        let len = dist(la, lb);
        if len < 1e-9 {
            return Vec::new();
        }
        let u = [(lb[0] - la[0]) / len, (lb[1] - la[1]) / len];
        let reflect = |p: P2| {
            let d = [p[0] - la[0], p[1] - la[1]];
            let t = d[0] * u[0] + d[1] * u[1];
            let foot = [la[0] + u[0] * t, la[1] + u[1] * t];
            [2.0 * foot[0] - p[0], 2.0 * foot[1] - p[1]]
        };
        let tol = 1e-6 * len.max(1.0);
        let mut map: HashMap<usize, usize> = HashMap::new();
        let mut out = Vec::new();
        let mut seen = Vec::new();
        for &c in curves {
            if c == about || c >= self.curves.len() || seen.contains(&c) {
                continue;
            }
            seen.push(c);
            let curve = self.curves[c].clone();
            for p in curve.points() {
                if map.contains_key(&p) {
                    continue;
                }
                let q = reflect(self.points[p]);
                let copy = if dist(q, self.points[p]) < tol {
                    p
                } else {
                    let copy = self.add_point(q);
                    self.constraints.push(Constraint::Symmetric(p, copy, about));
                    copy
                };
                map.insert(p, copy);
            }
            let m = |p: usize| map[&p];
            // Reflection turns counter-clockwise arcs clockwise: swap ends.
            self.curves.push(match curve {
                Curve::Line { a, b, construction } => Curve::Line { a: m(a), b: m(b), construction },
                Curve::Circle { center, radius, construction } => Curve::Circle { center: m(center), radius, construction },
                Curve::Arc { center, start, end, construction } => Curve::Arc { center: m(center), start: m(end), end: m(start), construction },
            });
            let copy = self.curves.len() - 1;
            if matches!(curve, Curve::Circle { .. }) {
                self.constraints.push(Constraint::Equal(c, copy));
            }
            out.push(copy);
        }
        out
    }
}

/// The circle through three points, if they aren't in a line.
pub fn circle_through(a: P2, b: P2, c: P2) -> Option<(P2, f64)> {
    let d = 2.0 * (a[0] * (b[1] - c[1]) + b[0] * (c[1] - a[1]) + c[0] * (a[1] - b[1]));
    let scale = [a, b, c].iter().map(|p| dist(*p, a)).fold(0.0, f64::max).max(1e-9);
    if d.abs() < 1e-9 * scale * scale {
        return None;
    }
    let sq = |p: P2| p[0] * p[0] + p[1] * p[1];
    let x = (sq(a) * (b[1] - c[1]) + sq(b) * (c[1] - a[1]) + sq(c) * (a[1] - b[1])) / d;
    let y = (sq(a) * (c[0] - b[0]) + sq(b) * (a[0] - c[0]) + sq(c) * (b[0] - a[0])) / d;
    Some(([x, y], dist([x, y], a)))
}

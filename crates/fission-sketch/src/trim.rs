//! Trim, as in Fusion: clicking a curve removes the piece between the
//! nearest intersections on either side of the click (or the whole curve
//! when nothing crosses it). Cut ends stay attached to the curve that cut
//! them.

use std::f64::consts::TAU;

use crate::{dist, Constraint, Curve, Sketch, P2};

const EPS: f64 = 1e-7;

/// A curve's geometry, resolved from its points.
#[derive(Clone, Copy)]
enum Shape {
    Segment(P2, P2),
    /// Center, radius, start angle and CCW sweep (TAU for a full circle).
    Round(P2, f64, f64, f64),
}

impl Sketch {
    fn shape(&self, c: usize) -> Shape {
        let p = |i: usize| self.points[i];
        match self.curves[c] {
            Curve::Line { a, b, .. } => Shape::Segment(p(a), p(b)),
            Curve::Circle { center, radius, .. } => Shape::Round(p(center), radius, 0.0, TAU),
            Curve::Arc { center, start, end, .. } => {
                let (c, s, e) = (p(center), p(start), p(end));
                let (a0, a1) = (angle(c, s), angle(c, e));
                let sweep = (a1 - a0).rem_euclid(TAU);
                Shape::Round(c, dist(c, s), a0, if sweep < EPS { TAU } else { sweep })
            }
        }
    }

    /// Trim `curve` at the piece nearest `at`. Returns false when nothing
    /// changed (no such curve).
    pub fn trim(&mut self, curve: usize, at: P2) -> bool {
        if curve >= self.curves.len() {
            return false;
        }
        let (shape, full_circle, cuts) = self.cuts(curve);
        let t = param(shape, at);

        if full_circle {
            if cuts.len() < 2 {
                self.delete_curve(curve);
                return true;
            }
            // The clicked piece runs from the last cut before the click to
            // the first after it (wrapping); keep the rest as an arc.
            let before = cuts.iter().rev().find(|c| c.0 < t).copied().unwrap_or(*cuts.last().unwrap());
            let after = cuts.iter().find(|c| c.0 > t).copied().unwrap_or(cuts[0]);
            let Curve::Circle { center, construction, .. } = self.curves[curve] else { unreachable!() };
            let start = self.cut_point(shape, after);
            let end = self.cut_point(shape, before);
            self.curves[curve] = Curve::Arc { center, start, end, construction };
            return true;
        }

        let before = cuts.iter().rev().find(|c| c.0 < t).copied();
        let after = cuts.iter().find(|c| c.0 > t).copied();
        match (before, after) {
            (None, None) => self.delete_curve(curve),
            // Clicked the first piece: the curve now starts at the cut.
            (None, Some(cut)) => {
                let q = self.cut_point(shape, cut);
                match &mut self.curves[curve] {
                    Curve::Line { a, .. } => *a = q,
                    Curve::Arc { start, .. } => *start = q,
                    Curve::Circle { .. } => unreachable!(),
                }
                self.drop_length_constraints(curve);
                self.remove_unused_points();
            }
            // Clicked the last piece: it now ends at the cut.
            (Some(cut), None) => {
                let q = self.cut_point(shape, cut);
                match &mut self.curves[curve] {
                    Curve::Line { b, .. } => *b = q,
                    Curve::Arc { end, .. } => *end = q,
                    Curve::Circle { .. } => unreachable!(),
                }
                self.drop_length_constraints(curve);
                self.remove_unused_points();
            }
            // A middle piece: split into the part before and the part after.
            (Some(lo), Some(hi)) => {
                let q1 = self.cut_point(shape, lo);
                let q2 = self.cut_point(shape, hi);
                let new = match &mut self.curves[curve] {
                    Curve::Line { b, construction, .. } => {
                        let rest = Curve::Line { a: q2, b: *b, construction: *construction };
                        *b = q1;
                        rest
                    }
                    Curve::Arc { center, end, construction, .. } => {
                        let rest = Curve::Arc { center: *center, start: q2, end: *end, construction: *construction };
                        *end = q1;
                        rest
                    }
                    Curve::Circle { .. } => unreachable!(),
                };
                self.curves.push(new);
                let new = self.curves.len() - 1;
                self.drop_length_constraints(curve);
                // The second piece keeps the first one's direction or circle.
                let copies: Vec<Constraint> = self
                    .constraints
                    .iter()
                    .filter_map(|k| match *k {
                        Constraint::Horizontal(l) if l == curve => Some(Constraint::Horizontal(new)),
                        Constraint::Vertical(l) if l == curve => Some(Constraint::Vertical(new)),
                        Constraint::Parallel(a, b) if a == curve => Some(Constraint::Parallel(new, b)),
                        Constraint::Parallel(a, b) if b == curve => Some(Constraint::Parallel(a, new)),
                        Constraint::Perpendicular(a, b) if a == curve => Some(Constraint::Perpendicular(new, b)),
                        Constraint::Perpendicular(a, b) if b == curve => Some(Constraint::Perpendicular(a, new)),
                        _ => None,
                    })
                    .collect();
                self.constraints.extend(copies);
                match self.curves[curve] {
                    Curve::Line { .. } => self.constraints.push(Constraint::Parallel(curve, new)),
                    _ => self.constraints.push(Constraint::Equal(curve, new)),
                }
                self.remove_unused_points();
            }
        }
        true
    }

    /// Where other curves cross `curve`, as sorted parameters along it.
    fn cuts(&self, curve: usize) -> (Shape, bool, Vec<(f64, usize)>) {
        let shape = self.shape(curve);
        let full_circle = matches!(self.curves[curve], Curve::Circle { .. });
        let mut cuts: Vec<(f64, usize)> = Vec::new();
        for other in 0..self.curves.len() {
            if other == curve {
                continue;
            }
            for x in intersections(shape, self.shape(other)) {
                let t = param(shape, x);
                let inside = full_circle || (t > EPS && t < span(shape) - EPS);
                if inside && !cuts.iter().any(|&(u, _)| (u - t).abs() < 1e-6) {
                    cuts.push((t, other));
                }
            }
        }
        cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
        (shape, full_circle, cuts)
    }

    /// The piece `trim(curve, at)` would remove, as a polyline (for the
    /// red hover preview).
    pub fn trim_preview(&self, curve: usize, at: P2) -> Vec<P2> {
        if curve >= self.curves.len() {
            return Vec::new();
        }
        let (shape, full_circle, cuts) = self.cuts(curve);
        let t = param(shape, at);
        let (lo, hi) = if full_circle {
            if cuts.len() < 2 {
                (0.0, TAU)
            } else {
                let lo = cuts.iter().rev().find(|c| c.0 < t).unwrap_or(cuts.last().unwrap()).0;
                let hi = cuts.iter().find(|c| c.0 > t).unwrap_or(&cuts[0]).0;
                (lo, if hi < lo { hi + TAU } else { hi })
            }
        } else {
            let lo = cuts.iter().rev().find(|c| c.0 < t).map_or(0.0, |c| c.0);
            let hi = cuts.iter().find(|c| c.0 > t).map_or(span(shape), |c| c.0);
            (lo, hi)
        };
        let n = match shape {
            Shape::Segment(..) => 1,
            Shape::Round(..) => 48,
        };
        (0..=n).map(|i| eval(shape, lo + (hi - lo) * i as f64 / n as f64)).collect()
    }

    /// A new point where `cut` crosses `shape`, held on the cutting curve.
    fn cut_point(&mut self, shape: Shape, (t, cutter): (f64, usize)) -> usize {
        let q = self.add_point(eval(shape, t));
        self.constraints.push(match self.curves[cutter] {
            Curve::Line { .. } => Constraint::PointOnLine(q, cutter),
            _ => Constraint::PointOnCircle(q, cutter),
        });
        q
    }

    /// Length-like constraints no longer describe a trimmed curve.
    fn drop_length_constraints(&mut self, curve: usize) {
        self.constraints.retain(|k| {
            !matches!(*k,
                Constraint::Length(l, _) | Constraint::Midpoint(_, l) if l == curve)
                && !matches!(*k, Constraint::Equal(a, b) if (a == curve || b == curve)
                    && matches!(self.curves[curve], Curve::Line { .. }))
        });
    }
}

/// What `extend(curve, at)` would do: move end point `point` to `to`, on
/// curve `onto`, adding `piece` (for the preview).
struct Extension {
    point: usize,
    to: P2,
    onto: usize,
    piece: Vec<P2>,
}

impl Sketch {
    /// Extend `curve` from its end nearest `at` to the first curve in the
    /// way, like Fusion's Extend: lines along their direction, arcs around
    /// their circle. Only free ends extend. Returns false if nothing is hit.
    pub fn extend(&mut self, curve: usize, at: P2) -> bool {
        let Some(x) = self.extension(curve, at) else { return false };
        self.points[x.point] = x.to;
        // A pin on the old end would pull it back.
        self.constraints.retain(|k| !matches!(*k, Constraint::Fixed(p) if p == x.point));
        self.drop_length_constraints(curve);
        self.constraints.push(match self.curves[x.onto] {
            Curve::Line { .. } => Constraint::PointOnLine(x.point, x.onto),
            _ => Constraint::PointOnCircle(x.point, x.onto),
        });
        true
    }

    /// The piece `extend(curve, at)` would add, as a polyline.
    pub fn extend_preview(&self, curve: usize, at: P2) -> Vec<P2> {
        self.extension(curve, at).map(|x| x.piece).unwrap_or_default()
    }

    fn extension(&self, curve: usize, at: P2) -> Option<Extension> {
        let p = |i: usize| self.points[i];
        let near = |i: usize, j: usize| if dist(p(i), at) <= dist(p(j), at) { (i, j) } else { (j, i) };
        let (point, from_start) = match *self.curves.get(curve)? {
            Curve::Line { a, b, .. } => (near(a, b).0, false),
            Curve::Arc { start, end, .. } => {
                let (n, _) = near(start, end);
                (n, n == start)
            }
            Curve::Circle { .. } => return None,
        };
        // Connected ends (shared, or coincident with another curve's point) stay put.
        let others_use = |q: usize| self.curves.iter().enumerate().any(|(k, c)| k != curve && c.points().contains(&q));
        let joined = self.constraints.iter().any(|k| match *k {
            Constraint::Coincident(a, b) => (a == point && others_use(b)) || (b == point && others_use(a)),
            _ => false,
        });
        if others_use(point) || joined {
            return None;
        }
        let shape = self.shape(curve);
        // How far past the end each crossing is (parameter units), if ahead.
        let ahead = |x: P2| -> Option<f64> {
            match shape {
                Shape::Segment(..) => {
                    let Curve::Line { a, b, .. } = self.curves[curve] else { return None };
                    let (e, o) = near(a, b);
                    let t = param(Shape::Segment(p(o), p(e)), x);
                    (t > 1.0 + EPS).then_some(t - 1.0)
                }
                Shape::Round(c, _, a0, sweep) => {
                    let d = if from_start { (a0 - angle(c, x)).rem_euclid(TAU) } else { (angle(c, x) - a0 - sweep).rem_euclid(TAU) };
                    (d > EPS && d < TAU - sweep - EPS).then_some(d)
                }
            }
        };
        let mut best: Option<(f64, P2, usize)> = None;
        for other in (0..self.curves.len()).filter(|&k| k != curve) {
            let os = self.shape(other);
            let carrier = match (shape, os) {
                (Shape::Segment(a, b), Shape::Segment(r, s)) => line_line(a, b, r, s).into_iter().collect(),
                (Shape::Segment(a, b), Shape::Round(c, r, ..)) => line_circle(a, b, c, r),
                (Shape::Round(c, r, ..), Shape::Segment(a, b)) => line_circle(a, b, c, r),
                (Shape::Round(c1, r1, ..), Shape::Round(c2, r2, ..)) => circle_circle(c1, r1, c2, r2),
            };
            for x in carrier.into_iter().filter(|&x| on(os, x)) {
                if let Some(d) = ahead(x) {
                    if best.is_none_or(|b| d < b.0) {
                        best = Some((d, x, other));
                    }
                }
            }
        }
        let (d, to, onto) = best?;
        let piece = match shape {
            Shape::Segment(..) => vec![p(point), to],
            Shape::Round(c, r, a0, sweep) => {
                let (from, dir) = if from_start { (a0, -1.0) } else { (a0 + sweep, 1.0) };
                (0..=24).map(|i| eval(Shape::Round(c, r, from + dir * d * i as f64 / 24.0, 0.0), 0.0)).collect()
            }
        };
        Some(Extension { point, to, onto, piece })
    }
}

fn angle(c: P2, p: P2) -> f64 {
    (p[1] - c[1]).atan2(p[0] - c[0])
}

/// Parameter length: 1 for a segment, the sweep for a round curve.
fn span(s: Shape) -> f64 {
    match s {
        Shape::Segment(..) => 1.0,
        Shape::Round(_, _, _, sweep) => sweep,
    }
}

/// Parameter of the point on `s` nearest `p`.
fn param(s: Shape, p: P2) -> f64 {
    match s {
        Shape::Segment(a, b) => {
            let d = [b[0] - a[0], b[1] - a[1]];
            let len2 = (d[0] * d[0] + d[1] * d[1]).max(1e-18);
            ((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / len2
        }
        Shape::Round(c, _, a0, _) => (angle(c, p) - a0).rem_euclid(TAU),
    }
}

fn eval(s: Shape, t: f64) -> P2 {
    match s {
        Shape::Segment(a, b) => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t],
        Shape::Round(c, r, a0, _) => [c[0] + r * (a0 + t).cos(), c[1] + r * (a0 + t).sin()],
    }
}

/// Is `p` (on the curve's carrier) within the curve's extent?
fn on(s: Shape, p: P2) -> bool {
    let t = param(s, p);
    match s {
        Shape::Segment(..) => (-EPS..=1.0 + EPS).contains(&t),
        Shape::Round(_, _, _, sweep) => sweep >= TAU - EPS || t <= sweep + EPS || t >= TAU - EPS,
    }
}

fn intersections(a: Shape, b: Shape) -> Vec<P2> {
    let raw = match (a, b) {
        (Shape::Segment(p, q), Shape::Segment(r, s)) => line_line(p, q, r, s).into_iter().collect(),
        (Shape::Segment(p, q), Shape::Round(c, rad, ..)) | (Shape::Round(c, rad, ..), Shape::Segment(p, q)) => {
            line_circle(p, q, c, rad)
        }
        (Shape::Round(c1, r1, ..), Shape::Round(c2, r2, ..)) => circle_circle(c1, r1, c2, r2),
    };
    raw.into_iter().filter(|&x| on(a, x) && on(b, x)).collect()
}

fn line_line(p: P2, q: P2, r: P2, s: P2) -> Option<P2> {
    let d1 = [q[0] - p[0], q[1] - p[1]];
    let d2 = [s[0] - r[0], s[1] - r[1]];
    let den = d1[0] * d2[1] - d1[1] * d2[0];
    if den.abs() < 1e-12 {
        return None;
    }
    let t = ((r[0] - p[0]) * d2[1] - (r[1] - p[1]) * d2[0]) / den;
    Some([p[0] + d1[0] * t, p[1] + d1[1] * t])
}

fn line_circle(p: P2, q: P2, c: P2, r: f64) -> Vec<P2> {
    let d = [q[0] - p[0], q[1] - p[1]];
    let f = [p[0] - c[0], p[1] - c[1]];
    let a = d[0] * d[0] + d[1] * d[1];
    if a < 1e-18 {
        return Vec::new();
    }
    let b = 2.0 * (f[0] * d[0] + f[1] * d[1]);
    let k = f[0] * f[0] + f[1] * f[1] - r * r;
    let disc = b * b - 4.0 * a * k;
    if disc < 0.0 {
        return Vec::new();
    }
    let root = disc.sqrt();
    let mut out = vec![];
    for t in [(-b - root) / (2.0 * a), (-b + root) / (2.0 * a)] {
        let x = [p[0] + d[0] * t, p[1] + d[1] * t];
        if !out.iter().any(|&y| dist(x, y) < 1e-9) {
            out.push(x);
        }
    }
    out
}

fn circle_circle(c1: P2, r1: f64, c2: P2, r2: f64) -> Vec<P2> {
    let d = dist(c1, c2);
    if d < 1e-12 || d > r1 + r2 + 1e-9 || d < (r1 - r2).abs() - 1e-9 {
        return Vec::new();
    }
    let a = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
    let h = (r1 * r1 - a * a).max(0.0).sqrt();
    let m = [c1[0] + a * (c2[0] - c1[0]) / d, c1[1] + a * (c2[1] - c1[1]) / d];
    let off = [-(c2[1] - c1[1]) * h / d, (c2[0] - c1[0]) * h / d];
    let x1 = [m[0] + off[0], m[1] + off[1]];
    let x2 = [m[0] - off[0], m[1] - off[1]];
    if h < 1e-9 { vec![x1] } else { vec![x1, x2] }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plus sign: horizontal line crossed by a vertical one at x = 0.
    fn plus() -> Sketch {
        let mut s = Sketch::default();
        s.add_line([-10.0, 0.0], [10.0, 0.0]);
        s.add_line([0.0, -10.0], [0.0, 10.0]);
        s
    }

    #[test]
    fn trims_the_clicked_end_of_a_line() {
        let mut s = plus();
        assert!(s.trim(0, [5.0, 0.0]));
        let (a, b) = s.line_points(0);
        assert_eq!(s.points[a], [-10.0, 0.0]);
        assert!(dist(s.points[b], [0.0, 0.0]) < 1e-9);
        // The new end stays on the vertical line.
        assert!(s.constraints.contains(&Constraint::PointOnLine(b, 1)));
        s.solve().unwrap();
    }

    #[test]
    fn extend_reaches_the_next_line_or_circle() {
        // A short line pointing at a vertical wall, and an arc that can
        // swing round onto the same wall.
        let mut s = Sketch::default();
        s.add_line([0.0, 0.0], [5.0, 0.0]);
        s.add_line([10.0, -10.0], [10.0, 10.0]);
        assert_eq!(s.extend_preview(0, [4.0, 0.0]), vec![[5.0, 0.0], [10.0, 0.0]]);
        assert!(s.extend(0, [4.0, 0.0]));
        let (_, b) = s.line_points(0);
        assert!(dist(s.points[b], [10.0, 0.0]) < 1e-9);
        assert!(s.constraints.contains(&Constraint::PointOnLine(b, 1)));
        s.solve().unwrap();
        // Nothing beyond the other end.
        assert!(!s.extend(0, [0.5, 0.0]));

        // A quarter arc around (0, 6), radius 3, from (3, 6) CCW to (0, 9).
        let c = s.add_point([0.0, 6.0]);
        let st = s.add_point([3.0, 6.0]);
        let en = s.add_point([0.0, 9.0]);
        s.curves.push(Curve::Arc { center: c, start: st, end: en, construction: false });
        let arc = s.curves.len() - 1;
        // Radius 3 around (0, 6) never reaches y = 0, so nothing to hit.
        assert!(!s.extend(arc, [2.9, 6.5]));
        s.add_line([-5.0, 4.0], [5.0, 4.0]);
        // Clockwise from (3, 6) the circle meets y = 4 at x = +sqrt(5).
        assert!(s.extend(arc, [2.9, 6.5]));
        assert!(dist(s.points[st], [5f64.sqrt(), 4.0]) < 1e-9, "{:?}", s.points[st]);
        // A connected end doesn't extend.
        let mut r = Sketch::default();
        r.add_rectangle([0.0, 0.0], [4.0, 4.0]);
        r.add_line([10.0, -10.0], [10.0, 10.0]);
        assert!(!r.extend(0, [3.9, 0.0]));
    }

    #[test]
    fn a_curve_nothing_crosses_is_deleted() {
        let mut s = Sketch::default();
        s.add_line([0.0, 0.0], [5.0, 0.0]);
        s.trim(0, [1.0, 0.0]);
        assert!(s.curves.is_empty() && s.points.is_empty());
    }

    #[test]
    fn middle_piece_splits_the_line() {
        let mut s = plus();
        s.add_line([4.0, -10.0], [4.0, 10.0]);
        s.constraints.push(Constraint::Horizontal(0));
        s.trim(0, [2.0, 0.0]);
        assert_eq!(s.curves.len(), 4);
        assert!((s.line_length(0) - 10.0).abs() < 1e-9, "left part -10..0");
        assert!((s.line_length(3) - 6.0).abs() < 1e-9, "right part 4..10");
        assert!(s.constraints.contains(&Constraint::Horizontal(3)));
        s.solve().unwrap();
    }

    #[test]
    fn circle_cut_twice_becomes_an_arc() {
        let mut s = Sketch::default();
        s.add_circle([0.0, 0.0], 5.0);
        s.add_line([-10.0, 0.0], [10.0, 0.0]);
        // Click the top half: the bottom half stays, from (-5,0) round to (5,0).
        s.trim(0, [0.0, 5.0]);
        let Curve::Arc { start, end, .. } = s.curves[0] else { panic!("{:?}", s.curves[0]) };
        assert!(dist(s.points[start], [-5.0, 0.0]) < 1e-9);
        assert!(dist(s.points[end], [5.0, 0.0]) < 1e-9);
        s.solve().unwrap();
    }

    #[test]
    fn rectangle_corner_overhang_trims_back_to_the_corner() {
        // A rectangle with a line poking out past its right side.
        let mut s = Sketch::default();
        s.add_rectangle([0.0, 0.0], [10.0, 10.0]);
        s.add_line([5.0, 5.0], [15.0, 5.0]);
        let overhang = 4;
        s.trim(overhang, [13.0, 5.0]);
        let (_, b) = s.line_points(overhang);
        assert!(dist(s.points[b], [10.0, 5.0]) < 1e-9);
        assert_eq!(s.regions().len(), 1, "the dangling line inside adds no region");
        s.solve().unwrap();
    }
}

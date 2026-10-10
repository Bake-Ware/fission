//! Closed-region ("profile") detection.
//!
//! Curves are split wherever they cross or touch another curve, then the
//! pieces form a planar graph (vertices merged by position). Walking its
//! faces (always taking the next edge clockwise from where we came in)
//! yields every bounded region counter-clockwise; circles nothing touches
//! are loops of their own. Loops inside other loops become holes, so a
//! circle inside a rectangle gives two profiles: the plate with a hole, and
//! the disk; two crossing rectangles give three.

use std::f64::consts::TAU;

use crate::{dist, Curve, Sketch, P2};

#[derive(Clone, Debug, PartialEq)]
pub enum Segment {
    Line(P2, P2),
    /// Three-point arc from `start` through `mid` to `end`.
    Arc { start: P2, mid: P2, end: P2 },
    Circle { center: P2, radius: f64 },
}

/// A closed chain of segments, counter-clockwise for outer boundaries.
#[derive(Clone, Debug, PartialEq)]
pub struct Loop {
    pub segments: Vec<Segment>,
}

/// A profile: one outer loop minus its holes.
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub outer: Loop,
    pub holes: Vec<Loop>,
    pub area: f64,
}

impl Loop {
    /// Polygon approximation (arcs sampled), for area and containment tests.
    pub fn polygon(&self) -> Vec<P2> {
        let mut out = Vec::new();
        for seg in &self.segments {
            match *seg {
                Segment::Line(a, _) => out.push(a),
                Segment::Arc { start, mid, end } => {
                    if let Some((c, r, a0, sweep)) = arc_params(start, mid, end) {
                        for i in 0..64 {
                            let t = a0 + sweep * i as f64 / 64.0;
                            out.push([c[0] + r * t.cos(), c[1] + r * t.sin()]);
                        }
                    } else {
                        out.push(start);
                    }
                }
                Segment::Circle { center, radius } => {
                    for i in 0..128 {
                        let t = TAU * i as f64 / 128.0;
                        out.push([center[0] + radius * t.cos(), center[1] + radius * t.sin()]);
                    }
                }
            }
        }
        out
    }

    pub fn signed_area(&self) -> f64 {
        signed_area(&self.polygon())
    }
}

impl Region {
    /// Is `p` inside the profile (in the outer loop, outside every hole)?
    pub fn contains(&self, p: P2) -> bool {
        point_in_polygon(p, &self.outer.polygon()) && !self.holes.iter().any(|h| point_in_polygon(p, &h.polygon()))
    }
}

/// Center, radius, start angle, and signed sweep of the arc through three points.
pub fn arc_params(start: P2, mid: P2, end: P2) -> Option<(P2, f64, f64, f64)> {
    let (ax, ay, bx, by, cx, cy) = (start[0], start[1], mid[0], mid[1], end[0], end[1]);
    let d = 2.0 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by));
    if d.abs() < 1e-12 {
        return None;
    }
    let a2 = ax * ax + ay * ay;
    let b2 = bx * bx + by * by;
    let c2 = cx * cx + cy * cy;
    let ux = (a2 * (by - cy) + b2 * (cy - ay) + c2 * (ay - by)) / d;
    let uy = (a2 * (cx - bx) + b2 * (ax - cx) + c2 * (bx - ax)) / d;
    let c = [ux, uy];
    let angle = |p: P2| (p[1] - uy).atan2(p[0] - ux);
    let (t0, t1, tm) = (angle(start), angle(end), angle(mid));
    let ccw = |from: f64, to: f64| (to - from).rem_euclid(TAU);
    // Go the way that passes through `mid`.
    let sweep = if ccw(t0, tm) <= ccw(t0, t1) { ccw(t0, t1) } else { ccw(t0, t1) - TAU };
    Some((c, dist(c, start), t0, sweep))
}

fn signed_area(poly: &[P2]) -> f64 {
    let n = poly.len();
    (0..n).map(|i| poly[i][0] * poly[(i + 1) % n][1] - poly[(i + 1) % n][0] * poly[i][1]).sum::<f64>() / 2.0
}

pub fn point_in_polygon(p: P2, poly: &[P2]) -> bool {
    let mut inside = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + n - 1) % n]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
    }
    inside
}

/// A non-construction curve as plain geometry.
#[derive(Clone, Copy, Debug)]
enum Shape {
    Line(P2, P2),
    /// Counter-clockwise from `t0` to `t1` (t1 > t0); a full circle when
    /// the sweep is TAU.
    Arc { c: P2, r: f64, t0: f64, t1: f64 },
}

impl Shape {
    fn at(&self, t: f64) -> P2 {
        match *self {
            Shape::Line(a, b) => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t],
            Shape::Arc { c, r, .. } => [c[0] + r * t.cos(), c[1] + r * t.sin()],
        }
    }

    fn range(&self) -> (f64, f64) {
        match *self {
            Shape::Line(..) => (0.0, 1.0),
            Shape::Arc { t0, t1, .. } => (t0, t1),
        }
    }

    fn is_full_circle(&self) -> bool {
        matches!(*self, Shape::Arc { t0, t1, .. } if (t1 - t0 - TAU).abs() < 1e-12)
    }

    /// The parameter of `p` if it lies on the curve (strictly inside for
    /// lines and arcs, anywhere on a full circle).
    fn param_of(&self, p: P2, tol: f64) -> Option<f64> {
        match *self {
            Shape::Line(a, b) => {
                let d = [b[0] - a[0], b[1] - a[1]];
                let len2 = d[0] * d[0] + d[1] * d[1];
                let t = ((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / len2;
                let slack = tol / len2.sqrt();
                ((-slack..=1.0 + slack).contains(&t) && dist(self.at(t), p) < tol).then_some(t.clamp(0.0, 1.0))
            }
            Shape::Arc { c, r, t0, t1 } => {
                if (dist(c, p) - r).abs() > tol {
                    return None;
                }
                let mut t = (p[1] - c[1]).atan2(p[0] - c[0]);
                while t < t0 - 1e-12 {
                    t += TAU;
                }
                while t > t0 + TAU + 1e-12 {
                    t -= TAU;
                }
                (t <= t1 + 1e-12).then_some(t)
            }
        }
    }
}

/// Points where two curves cross (candidates; each is checked against both).
fn crossings(a: &Shape, b: &Shape) -> Vec<P2> {
    let circle = |s: &Shape| match *s {
        Shape::Arc { c, r, .. } => Some((c, r)),
        Shape::Line(..) => None,
    };
    match (a, b) {
        (Shape::Line(p, q), Shape::Line(u, v)) => {
            let (d1, d2) = ([q[0] - p[0], q[1] - p[1]], [v[0] - u[0], v[1] - u[1]]);
            let den = d1[0] * d2[1] - d1[1] * d2[0];
            if den.abs() < 1e-12 {
                return Vec::new();
            }
            let t = ((u[0] - p[0]) * d2[1] - (u[1] - p[1]) * d2[0]) / den;
            vec![[p[0] + d1[0] * t, p[1] + d1[1] * t]]
        }
        (Shape::Line(p, q), _) | (_, Shape::Line(p, q)) => {
            let (c, r) = circle(if matches!(a, Shape::Line(..)) { b } else { a }).unwrap();
            let d = [q[0] - p[0], q[1] - p[1]];
            let f = [p[0] - c[0], p[1] - c[1]];
            let (qa, qb, qc) = (d[0] * d[0] + d[1] * d[1], 2.0 * (f[0] * d[0] + f[1] * d[1]), f[0] * f[0] + f[1] * f[1] - r * r);
            let disc = qb * qb - 4.0 * qa * qc;
            if disc < -1e-12 * qa * r * r {
                return Vec::new();
            }
            let root = disc.max(0.0).sqrt();
            [(-qb - root) / (2.0 * qa), (-qb + root) / (2.0 * qa)].iter().map(|&t| [p[0] + d[0] * t, p[1] + d[1] * t]).collect()
        }
        _ => {
            let ((c1, r1), (c2, r2)) = (circle(a).unwrap(), circle(b).unwrap());
            let d = dist(c1, c2);
            if d < 1e-12 || d > r1 + r2 + 1e-9 || d < (r1 - r2).abs() - 1e-9 {
                return Vec::new();
            }
            let along = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
            let h = (r1 * r1 - along * along).max(0.0).sqrt();
            let u = [(c2[0] - c1[0]) / d, (c2[1] - c1[1]) / d];
            let m = [c1[0] + u[0] * along, c1[1] + u[1] * along];
            vec![[m[0] - u[1] * h, m[1] + u[0] * h], [m[0] + u[1] * h, m[1] - u[0] * h]]
        }
    }
}

struct HalfEdge {
    from: usize,
    to: usize,
    /// Direction leaving `from`, as an angle.
    angle: f64,
    /// Signed curvature along the way (left turns positive): breaks ties
    /// between curves that leave a vertex in the same direction (tangent).
    curvature: f64,
    segment: Segment,
}

pub(crate) fn regions(sketch: &Sketch) -> Vec<Region> {
    let pt = |i: usize| sketch.points[i];
    let mut shapes: Vec<Shape> = Vec::new();
    for curve in sketch.curves.iter().filter(|c| !c.construction()) {
        match *curve {
            Curve::Line { a, b, .. } => {
                if dist(pt(a), pt(b)) > 1e-9 {
                    shapes.push(Shape::Line(pt(a), pt(b)));
                }
            }
            Curve::Arc { center, start, end, .. } => {
                let (c, s, e) = (pt(center), pt(start), pt(end));
                let r = dist(c, s);
                if r > 1e-9 {
                    let t0 = (s[1] - c[1]).atan2(s[0] - c[0]);
                    let mut t1 = (e[1] - c[1]).atan2(e[0] - c[0]);
                    if t1 <= t0 {
                        t1 += TAU;
                    }
                    shapes.push(Shape::Arc { c, r, t0, t1 });
                }
            }
            Curve::Circle { center, radius, .. } => {
                if radius > 1e-9 {
                    let t0 = 0.0;
                    shapes.push(Shape::Arc { c: pt(center), r: radius, t0, t1: TAU });
                }
            }
        }
    }
    // Scale-aware tolerance for "on the curve".
    let size = sketch.points.iter().fold(1.0f64, |m, p| m.max(p[0].abs()).max(p[1].abs()));
    let tol = 1e-7 * size;

    // Where each curve gets cut: crossings with other curves, and other
    // curves' ends touching it.
    let mut cuts: Vec<Vec<f64>> = shapes
        .iter()
        .map(|s| {
            let (a, b) = s.range();
            if s.is_full_circle() { Vec::new() } else { vec![a, b] }
        })
        .collect();
    let ends = |s: &Shape| -> Vec<P2> {
        if s.is_full_circle() { Vec::new() } else { let (a, b) = s.range(); vec![s.at(a), s.at(b)] }
    };
    for i in 0..shapes.len() {
        for j in 0..shapes.len() {
            if i == j {
                continue;
            }
            let mut candidates = ends(&shapes[j]);
            if i < j {
                candidates.extend(crossings(&shapes[i], &shapes[j]));
            } else {
                candidates.extend(crossings(&shapes[j], &shapes[i]));
            }
            for p in candidates {
                if shapes[j].param_of(p, tol).is_none() && !ends(&shapes[j]).iter().any(|e| dist(*e, p) < tol) {
                    continue;
                }
                if let Some(t) = shapes[i].param_of(p, tol) {
                    cuts[i].push(t);
                }
            }
        }
    }

    // Vertices by position.
    let mut verts: Vec<P2> = Vec::new();
    let mut vertex = |p: P2| -> usize {
        match verts.iter().position(|v| dist(*v, p) < tol * 10.0) {
            Some(i) => i,
            None => {
                verts.push(p);
                verts.len() - 1
            }
        }
    };

    let mut halves: Vec<HalfEdge> = Vec::new();
    let mut loops: Vec<Loop> = Vec::new();
    // Each loop's connected component (a loop can only be a hole in a loop
    // it isn't joined to).
    let mut comps: Vec<usize> = Vec::new();
    for (shape, ts) in shapes.iter().zip(cuts.iter_mut()) {
        if shape.is_full_circle() {
            for t in ts.iter_mut() {
                *t = t.rem_euclid(TAU);
            }
        }
        // Cuts closer than the tolerance are one cut (tangent curves give
        // two crossings a hair apart).
        let step = match *shape {
            Shape::Line(a, b) => tol * 10.0 / dist(a, b).max(1e-12),
            Shape::Arc { r, .. } => tol * 10.0 / r.max(1e-12),
        };
        ts.sort_by(f64::total_cmp);
        if !shape.is_full_circle() {
            // Keep the curve's own ends exactly.
            let (lo, hi) = shape.range();
            ts.retain(|t| *t == lo || *t == hi || (*t - lo > step && hi - *t > step));
        }
        ts.dedup_by(|a, b| (*a - *b).abs() < step);
        if shape.is_full_circle() && ts.len() > 1 && (ts[0] + TAU - ts[ts.len() - 1]).abs() < 1e-9 {
            ts.pop();
        }
        if shape.is_full_circle() && ts.len() < 2 {
            // Nothing (or one point) touches it: a loop of its own.
            if let Shape::Arc { c, r, .. } = *shape {
                loops.push(Loop { segments: vec![Segment::Circle { center: c, radius: r }] });
                comps.push(usize::MAX - loops.len());
            }
            continue;
        }
        // A full circle cut somewhere: start at the first cut.
        let pieces: Vec<(f64, f64)> = if shape.is_full_circle() {
            let mut v: Vec<(f64, f64)> = ts.windows(2).map(|w| (w[0], w[1])).collect();
            v.push((ts[ts.len() - 1], ts[0] + TAU));
            v
        } else {
            ts.windows(2).map(|w| (w[0], w[1])).collect()
        };
        for (t0, t1) in pieces {
            let (s, e) = (shape.at(t0), shape.at(t1));
            if dist(s, e) < tol && !matches!(shape, Shape::Arc { .. }) {
                continue;
            }
            let (from, to) = (vertex(s), vertex(e));
            if from == to {
                continue;
            }
            match *shape {
                Shape::Line(..) => {
                    let ang = |a: P2, b: P2| (b[1] - a[1]).atan2(b[0] - a[0]);
                    halves.push(HalfEdge { from, to, angle: ang(s, e), curvature: 0.0, segment: Segment::Line(s, e) });
                    halves.push(HalfEdge { from: to, to: from, angle: ang(e, s), curvature: 0.0, segment: Segment::Line(e, s) });
                }
                Shape::Arc { r, .. } => {
                    let mid = shape.at((t0 + t1) / 2.0);
                    // CCW tangents: leaving start at t0 + 90°, leaving end (going back) at t1 - 90°.
                    halves.push(HalfEdge { from, to, angle: t0 + TAU / 4.0, curvature: 1.0 / r, segment: Segment::Arc { start: s, mid, end: e } });
                    halves.push(HalfEdge { from: to, to: from, angle: t1 - TAU / 4.0, curvature: -1.0 / r, segment: Segment::Arc { start: e, mid, end: s } });
                }
            }
        }
    }

    // Outgoing half-edges per vertex, sorted counter-clockwise by angle.
    let mut outgoing: std::collections::HashMap<usize, Vec<usize>> = Default::default();
    for (i, h) in halves.iter().enumerate() {
        outgoing.entry(h.from).or_default().push(i);
    }
    for list in outgoing.values_mut() {
        // Counter-clockwise by direction; for the same direction, the one
        // bending left (more counter-clockwise just after) goes later.
        let key = |h: &HalfEdge| {
            let a = h.angle.rem_euclid(TAU);
            if TAU - a < 1e-9 { 0.0 } else { a }
        };
        list.sort_by(|&a, &b| {
            let (ka, kb) = (key(&halves[a]), key(&halves[b]));
            if (ka - kb).abs() < 1e-9 { halves[a].curvature.total_cmp(&halves[b].curvature) } else { ka.total_cmp(&kb) }
        });
    }
    let twin = |i: usize| i ^ 1;

    let mut component: Vec<usize> = (0..verts.len()).collect();
    fn find(c: &mut [usize], mut i: usize) -> usize {
        while c[i] != i {
            c[i] = c[c[i]];
            i = c[i];
        }
        i
    }
    for h in &halves {
        let (a, b) = (find(&mut component, h.from), find(&mut component, h.to));
        component[a] = b;
    }
    let mut visited = vec![false; halves.len()];
    for start in 0..halves.len() {
        if visited[start] {
            continue;
        }
        let mut segments = Vec::new();
        let mut h = start;
        let mut ok = true;
        loop {
            visited[h] = true;
            segments.push(halves[h].segment.clone());
            // At the end vertex, take the edge just clockwise of the way back.
            let around = &outgoing[&halves[h].to];
            let k = around.iter().position(|&o| o == twin(h)).unwrap();
            h = around[(k + around.len() - 1) % around.len()];
            if h == start {
                break;
            }
            if visited[h] || segments.len() > halves.len() {
                ok = false;
                break;
            }
        }
        if ok {
            loops.push(Loop { segments });
            comps.push(find(&mut component, halves[start].from));
        }
    }

    // Bounded faces come out counter-clockwise (positive area); the
    // unbounded outside of each component is clockwise, and dangling edges
    // enclose nothing.
    let mut loops: Vec<(Loop, f64, Vec<P2>, usize)> = loops
        .into_iter()
        .zip(comps)
        .map(|(l, comp)| {
            let poly = l.polygon();
            let area = signed_area(&poly);
            (l, area, poly, comp)
        })
        .filter(|(_, area, _, _)| *area > 1e-9)
        .collect();
    loops.sort_by(|a, b| b.1.total_cmp(&a.1));

    // Parent = the smallest larger loop, from another component, containing
    // this loop (most of its vertices: one may touch the parent).
    let within = |child: &[P2], parent: &[P2]| child.iter().filter(|&&p| point_in_polygon(p, parent)).count() * 2 > child.len();
    let parent: Vec<Option<usize>> = (0..loops.len())
        .map(|i| (0..i).rev().find(|&j| loops[j].3 != loops[i].3 && loops[j].1 > loops[i].1 && within(&loops[i].2, &loops[j].2)))
        .collect();

    loops
        .iter()
        .enumerate()
        .map(|(i, (outer, area, _, _))| {
            let holes: Vec<Loop> = (0..loops.len()).filter(|&j| parent[j] == Some(i)).map(|j| loops[j].0.clone()).collect();
            let hole_area: f64 = (0..loops.len()).filter(|&j| parent[j] == Some(i)).map(|j| loops[j].1).sum();
            Region { outer: outer.clone(), holes, area: area - hole_area }
        })
        .collect()
}

//! Sketch offset: copy a chain of connected curves (or a circle) at a
//! distance, joining the copies at their corners, like Fusion's Offset.

use std::f64::consts::TAU;

use crate::{dist, Constraint, Curve, Sketch, P2};

const JOIN: f64 = 1e-7;

impl Sketch {
    fn ends(&self, c: usize) -> Option<(P2, P2)> {
        let p = |i: usize| self.points[i];
        match self.curves[c] {
            Curve::Line { a, b, .. } => Some((p(a), p(b))),
            Curve::Arc { start, end, .. } => Some((p(start), p(end))),
            Curve::Circle { .. } => None,
        }
    }

    /// The curves connected end to end with `start`, in order, each with
    /// whether it runs backwards along the chain; and whether the chain
    /// closes. Stops at branches.
    pub fn chain(&self, start: usize) -> (Vec<(usize, bool)>, bool) {
        let Some((s0, e0)) = self.ends(start) else { return (vec![(start, false)], true) };
        let construction = self.curves[start].construction();
        let mut chain = vec![(start, false)];
        let next = |at: P2, used: &[(usize, bool)]| -> Option<(usize, bool)> {
            let found: Vec<(usize, bool)> = (0..self.curves.len())
                .filter(|&c| self.curves[c].construction() == construction && !used.iter().any(|u| u.0 == c))
                .filter_map(|c| {
                    let (s, e) = self.ends(c)?;
                    if dist(s, at) < JOIN {
                        Some((c, false))
                    } else if dist(e, at) < JOIN {
                        Some((c, true))
                    } else {
                        None
                    }
                })
                .collect();
            (found.len() == 1).then(|| found[0])
        };
        // Forward from the start curve's end.
        let mut at = e0;
        while let Some((c, rev)) = next(at, &chain) {
            chain.push((c, rev));
            let (s, e) = self.ends(c).unwrap();
            at = if rev { s } else { e };
            if dist(at, s0) < JOIN {
                return (chain, true);
            }
        }
        // Backward from its start.
        let mut at = s0;
        while let Some((c, rev)) = next(at, &chain) {
            // Walking backwards, a curve that starts here runs against the chain.
            chain.insert(0, (c, !rev));
            let (s, e) = self.ends(c).unwrap();
            at = if rev { e } else { s };
        }
        (chain, false)
    }

    /// Offset the chain through `curve` by `distance`: outward for a closed
    /// chain or a circle, to the left of the first curve for an open one
    /// (negative goes the other way). Returns the new curves.
    pub fn offset_chain(&mut self, curve: usize, distance: f64) -> Result<Vec<usize>, String> {
        if distance == 0.0 || !distance.is_finite() {
            return Err("offset distance must be non-zero".into());
        }
        if curve >= self.curves.len() {
            return Err("no such curve".into());
        }
        let construction = self.curves[curve].construction();
        if let Curve::Circle { center, radius, .. } = self.curves[curve] {
            let r = radius + distance;
            if r <= 0.0 {
                return Err("the offset is bigger than the circle".into());
            }
            let c = self.add_point(self.points[center]);
            self.curves.push(Curve::Circle { center: c, radius: r, construction });
            let new = self.curves.len() - 1;
            self.constraints.push(Constraint::Concentric(curve, new));
            return Ok(vec![new]);
        }
        let (chain, closed) = self.chain(curve);
        // Outward on a counter-clockwise loop is to the right of travel.
        let left = if closed && self.chain_area(&chain) > 0.0 { -distance } else { distance };

        // Each curve's offset copy, as start/end and (for arcs) center/radius.
        let mut pieces: Vec<Piece> = Vec::new();
        for &(c, rev) in &chain {
            let (s, e) = self.ends(c).unwrap();
            let (s, e) = if rev { (e, s) } else { (s, e) };
            pieces.push(match self.curves[c] {
                Curve::Line { .. } => {
                    let len = dist(s, e).max(1e-12);
                    let n = [-(e[1] - s[1]) / len * left, (e[0] - s[0]) / len * left];
                    Piece::Line([s[0] + n[0], s[1] + n[1]], [e[0] + n[0], e[1] + n[1]])
                }
                Curve::Arc { center, .. } => {
                    let cpt = self.points[center];
                    let r = dist(cpt, s);
                    // Travelling counter-clockwise, left is toward the center.
                    let r2 = if rev { r + left } else { r - left };
                    if r2 <= 0.0 {
                        return Err("the offset is bigger than an arc in the chain".into());
                    }
                    let at = |p: P2| [cpt[0] + (p[0] - cpt[0]) * r2 / r, cpt[1] + (p[1] - cpt[1]) * r2 / r];
                    Piece::Arc { center: cpt, radius: r2, s: at(s), e: at(e) }
                }
                Curve::Circle { .. } => unreachable!(),
            });
        }
        // Corners: where neighbouring copies meet (they already do at tangent joints).
        let n = pieces.len();
        let joints = if closed { n } else { n.saturating_sub(1) };
        for j in 0..joints {
            let (i, k) = (j, (j + 1) % n);
            let (a_end, b_start) = (pieces[i].end(), pieces[k].start());
            if dist(a_end, b_start) < 1e-9 {
                continue;
            }
            let guess = [(a_end[0] + b_start[0]) / 2.0, (a_end[1] + b_start[1]) / 2.0];
            let meet = carrier_meet(&pieces[i], &pieces[k], guess).unwrap_or(guess);
            pieces[i].set_end(meet);
            pieces[k].set_start(meet);
        }
        // A copied line that flipped direction means the offset ate it.
        for (piece, &(c, rev)) in pieces.iter().zip(&chain) {
            if let (Piece::Line(s, e), Some((os, oe))) = (piece, self.ends(c)) {
                let (os, oe) = if rev { (oe, os) } else { (os, oe) };
                let dot = (e[0] - s[0]) * (oe[0] - os[0]) + (e[1] - s[1]) * (oe[1] - os[1]);
                if dot <= 0.0 {
                    return Err("the offset is too big for this shape".into());
                }
            }
        }

        // Build: neighbouring pieces share their joint point.
        let mut first_point = None;
        let mut prev_end: Option<usize> = None;
        let mut new_curves = Vec::new();
        for (idx, (piece, &(orig, rev))) in pieces.iter().zip(&chain).enumerate() {
            let s = match prev_end {
                Some(p) => p,
                None => self.add_point(piece.start()),
            };
            first_point.get_or_insert(s);
            let e = if closed && idx == n - 1 { first_point.unwrap() } else { self.add_point(piece.end()) };
            let curve = match *piece {
                Piece::Line(..) => Curve::Line { a: s, b: e, construction },
                Piece::Arc { center, .. } => {
                    let c = self.add_point(center);
                    // Arcs are stored counter-clockwise.
                    if rev { Curve::Arc { center: c, start: e, end: s, construction } } else { Curve::Arc { center: c, start: s, end: e, construction } }
                }
            };
            self.curves.push(curve);
            let new = self.curves.len() - 1;
            self.constraints.push(match piece {
                Piece::Line(..) => Constraint::Parallel(orig, new),
                Piece::Arc { .. } => Constraint::Concentric(orig, new),
            });
            new_curves.push(new);
            prev_end = Some(e);
        }
        Ok(new_curves)
    }

    /// Signed area of a closed chain (positive = counter-clockwise).
    fn chain_area(&self, chain: &[(usize, bool)]) -> f64 {
        let mut poly = Vec::new();
        for &(c, rev) in chain {
            let (s, e) = self.ends(c).unwrap();
            let (s, e) = if rev { (e, s) } else { (s, e) };
            if let Curve::Arc { center, .. } = self.curves[c] {
                let cpt = self.points[center];
                let r = dist(cpt, s);
                let a0 = (s[1] - cpt[1]).atan2(s[0] - cpt[0]);
                let a1 = (e[1] - cpt[1]).atan2(e[0] - cpt[0]);
                let sweep = if rev { -(a0 - a1).rem_euclid(TAU) } else { (a1 - a0).rem_euclid(TAU) };
                for i in 0..16 {
                    let t = a0 + sweep * i as f64 / 16.0;
                    poly.push([cpt[0] + r * t.cos(), cpt[1] + r * t.sin()]);
                }
            } else {
                poly.push(s);
            }
        }
        let n = poly.len();
        (0..n).map(|i| poly[i][0] * poly[(i + 1) % n][1] - poly[(i + 1) % n][0] * poly[i][1]).sum::<f64>() / 2.0
    }
}

#[derive(Clone, Copy, Debug)]
enum Piece {
    Line(P2, P2),
    Arc { center: P2, radius: f64, s: P2, e: P2 },
}

impl Piece {
    fn start(&self) -> P2 {
        match *self {
            Piece::Line(s, _) | Piece::Arc { s, .. } => s,
        }
    }
    fn end(&self) -> P2 {
        match *self {
            Piece::Line(_, e) | Piece::Arc { e, .. } => e,
        }
    }
    fn set_start(&mut self, p: P2) {
        match self {
            Piece::Line(s, _) | Piece::Arc { s, .. } => *s = p,
        }
    }
    fn set_end(&mut self, p: P2) {
        match self {
            Piece::Line(_, e) | Piece::Arc { e, .. } => *e = p,
        }
    }
}

/// Where the carriers (infinite line / full circle) of two pieces cross,
/// nearest to `guess`.
fn carrier_meet(a: &Piece, b: &Piece, guess: P2) -> Option<P2> {
    let candidates: Vec<P2> = match (*a, *b) {
        (Piece::Line(p, q), Piece::Line(r, s)) => {
            let d1 = [q[0] - p[0], q[1] - p[1]];
            let d2 = [s[0] - r[0], s[1] - r[1]];
            let den = d1[0] * d2[1] - d1[1] * d2[0];
            if den.abs() < 1e-12 {
                return None;
            }
            let t = ((r[0] - p[0]) * d2[1] - (r[1] - p[1]) * d2[0]) / den;
            vec![[p[0] + d1[0] * t, p[1] + d1[1] * t]]
        }
        (Piece::Line(p, q), Piece::Arc { center, radius, .. }) | (Piece::Arc { center, radius, .. }, Piece::Line(p, q)) => {
            let d = [q[0] - p[0], q[1] - p[1]];
            let f = [p[0] - center[0], p[1] - center[1]];
            let aa = d[0] * d[0] + d[1] * d[1];
            let bb = 2.0 * (f[0] * d[0] + f[1] * d[1]);
            let cc = f[0] * f[0] + f[1] * f[1] - radius * radius;
            let disc = bb * bb - 4.0 * aa * cc;
            if aa < 1e-18 || disc < 0.0 {
                return None;
            }
            [(-bb - disc.sqrt()) / (2.0 * aa), (-bb + disc.sqrt()) / (2.0 * aa)]
                .into_iter()
                .map(|t| [p[0] + d[0] * t, p[1] + d[1] * t])
                .collect()
        }
        (Piece::Arc { center: c1, radius: r1, .. }, Piece::Arc { center: c2, radius: r2, .. }) => {
            let d = dist(c1, c2);
            if d < 1e-12 || d > r1 + r2 || d < (r1 - r2).abs() {
                return None;
            }
            let a = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
            let h = (r1 * r1 - a * a).max(0.0).sqrt();
            let m = [c1[0] + a * (c2[0] - c1[0]) / d, c1[1] + a * (c2[1] - c1[1]) / d];
            let o = [-(c2[1] - c1[1]) * h / d, (c2[0] - c1[0]) * h / d];
            vec![[m[0] + o[0], m[1] + o[1]], [m[0] - o[0], m[1] - o[1]]]
        }
    };
    candidates.into_iter().min_by(|x, y| dist(*x, guess).total_cmp(&dist(*y, guess)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangle_offsets_outward_with_sharp_corners() {
        let mut s = Sketch::default();
        let [bottom, ..] = s.add_rectangle([0.0, 0.0], [20.0, 10.0]);
        let (chain, closed) = s.chain(bottom);
        assert_eq!(chain.len(), 4);
        assert!(closed);
        let new = s.offset_chain(bottom, 2.0).unwrap();
        assert_eq!(new.len(), 4);
        s.solve().unwrap();
        let regions = s.regions();
        assert_eq!(regions.len(), 2, "the frame between the loops, and the inside");
        let outer = regions.iter().map(|r| r.outer.signed_area().abs()).fold(0.0, f64::max);
        assert!((outer - 24.0 * 14.0).abs() < 1e-6, "{outer}");
    }

    #[test]
    fn negative_offset_goes_inside() {
        let mut s = Sketch::default();
        let [bottom, ..] = s.add_rectangle([0.0, 0.0], [20.0, 10.0]);
        let new = s.offset_chain(bottom, -2.0).unwrap();
        let (a, _) = s.line_points(new[0]);
        assert!(dist(s.points[a], [2.0, 2.0]) < 1e-9, "{:?}", s.points[a]);
        assert!(s.offset_chain(bottom, -6.0).is_err(), "can't shrink a 10 mm wide box by 6 each side");
    }

    #[test]
    fn circle_offset_is_concentric() {
        let mut s = Sketch::default();
        let c = s.add_circle([1.0, 2.0], 5.0);
        let new = s.offset_chain(c, 1.5).unwrap();
        s.solve().unwrap();
        assert!((s.radius(new[0]) - 6.5).abs() < 1e-9);
    }

    #[test]
    fn rounded_corner_offsets_to_a_bigger_arc() {
        let mut s = Sketch::default();
        let [bottom, ..] = s.add_rectangle([0.0, 0.0], [20.0, 10.0]);
        let corner = s.line_points(bottom).1;
        let (arc, _) = s.fillet_corner(corner, 3.0).unwrap();
        s.solve().unwrap();
        let new = s.offset_chain(bottom, 1.0).unwrap();
        assert_eq!(new.len(), 5);
        s.solve().unwrap();
        let grown = new.iter().find(|&&c| matches!(s.curves[c], Curve::Arc { .. })).unwrap();
        assert!((s.radius(*grown) - (s.radius(arc) + 1.0)).abs() < 1e-6);
    }
}

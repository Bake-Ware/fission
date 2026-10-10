//! 2D parametric sketches: geometry, constraints, a solver, and profile
//! (closed region) detection. Pure Rust and kernel-independent; the model
//! turns profiles into kernel `Profile`s.

use serde::{Deserialize, Serialize};

mod profiles;
mod solver;
mod trim;
mod fillet;
mod offset;
mod project;
mod shapes;

pub use profiles::{Loop, Region, Segment};
pub use shapes::circle_through;
pub use solver::{SolveError, SolveReport};

pub type P2 = [f64; 2];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Curve {
    Line { a: usize, b: usize, #[serde(default)] construction: bool },
    Circle { center: usize, radius: f64, #[serde(default)] construction: bool },
    /// Counter-clockwise from `start` to `end` around `center`. The solver
    /// keeps both ends at the same distance from the center.
    Arc { center: usize, start: usize, end: usize, #[serde(default)] construction: bool },
}

impl Curve {
    pub fn construction(&self) -> bool {
        match *self {
            Curve::Line { construction, .. } | Curve::Circle { construction, .. } | Curve::Arc { construction, .. } => {
                construction
            }
        }
    }

    pub fn set_construction(&mut self, value: bool) {
        match self {
            Curve::Line { construction, .. } | Curve::Circle { construction, .. } | Curve::Arc { construction, .. } => {
                *construction = value
            }
        }
    }

    /// Points this curve uses.
    pub fn points(&self) -> Vec<usize> {
        match *self {
            Curve::Line { a, b, .. } => vec![a, b],
            Curve::Circle { center, .. } => vec![center],
            Curve::Arc { center, start, end, .. } => vec![center, start, end],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Constraint {
    Coincident(usize, usize),
    /// Line is parallel to the sketch X axis.
    Horizontal(usize),
    Vertical(usize),
    /// Point doesn't move.
    Fixed(usize),
    /// Point-to-point distance.
    Distance(usize, usize, f64),
    /// Signed X/Y offset between two points (b - a).
    HorizontalDistance(usize, usize, f64),
    VerticalDistance(usize, usize, f64),
    /// Line length.
    Length(usize, f64),
    /// Circle or arc radius.
    Radius(usize, f64),
    Parallel(usize, usize),
    Perpendicular(usize, usize),
    /// Two lines of equal length, or two circles/arcs of equal radius.
    Equal(usize, usize),
    PointOnLine(usize, usize),
    PointOnCircle(usize, usize),
    Midpoint(usize, usize),
    /// Line tangent to a circle/arc.
    Tangent(usize, usize),
    Concentric(usize, usize),
    /// Two points mirror images of each other across a line.
    Symmetric(usize, usize, usize),
}

impl Constraint {
    /// Short label for UI glyphs.
    pub fn label(&self) -> &'static str {
        match self {
            Constraint::Coincident(..) => "Coincident",
            Constraint::Horizontal(..) => "Horizontal",
            Constraint::Vertical(..) => "Vertical",
            Constraint::Fixed(..) => "Fix",
            Constraint::Distance(..) => "Distance",
            Constraint::HorizontalDistance(..) => "Horizontal distance",
            Constraint::VerticalDistance(..) => "Vertical distance",
            Constraint::Length(..) => "Length",
            Constraint::Radius(..) => "Radius",
            Constraint::Parallel(..) => "Parallel",
            Constraint::Perpendicular(..) => "Perpendicular",
            Constraint::Equal(..) => "Equal",
            Constraint::PointOnLine(..) => "On line",
            Constraint::PointOnCircle(..) => "On circle",
            Constraint::Midpoint(..) => "Midpoint",
            Constraint::Tangent(..) => "Tangent",
            Constraint::Concentric(..) => "Concentric",
            Constraint::Symmetric(..) => "Symmetric",
        }
    }

    /// Dimensions carry a value the user edits; the rest are geometric.
    pub fn value_mut(&mut self) -> Option<&mut f64> {
        match self {
            Constraint::Distance(_, _, v)
            | Constraint::HorizontalDistance(_, _, v)
            | Constraint::VerticalDistance(_, _, v)
            | Constraint::Length(_, v)
            | Constraint::Radius(_, v) => Some(v),
            _ => None,
        }
    }
}

/// A sketch on some plane: 2D points, curves through them, and constraints.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Sketch {
    pub points: Vec<P2>,
    pub curves: Vec<Curve>,
    pub constraints: Vec<Constraint>,
}

impl Sketch {
    pub fn add_point(&mut self, p: P2) -> usize {
        self.points.push(p);
        self.points.len() - 1
    }

    pub fn add_line(&mut self, a: P2, b: P2) -> usize {
        let a = self.add_point(a);
        let b = self.add_point(b);
        self.curves.push(Curve::Line { a, b, construction: false });
        self.curves.len() - 1
    }

    pub fn add_circle(&mut self, center: P2, radius: f64) -> usize {
        let center = self.add_point(center);
        self.curves.push(Curve::Circle { center, radius, construction: false });
        self.curves.len() - 1
    }

    pub fn add_arc(&mut self, center: P2, start: P2, end: P2) -> usize {
        let (c, s, e) = (self.add_point(center), self.add_point(start), self.add_point(end));
        self.curves.push(Curve::Arc { center: c, start: s, end: e, construction: false });
        self.curves.len() - 1
    }

    /// Axis-aligned rectangle from two corners, constrained like Fusion's
    /// two-point rectangle: four lines, corners coincident, two horizontal,
    /// two vertical. Returns the line indices (bottom, right, top, left).
    pub fn add_rectangle(&mut self, a: P2, b: P2) -> [usize; 4] {
        let corners = [[a[0], a[1]], [b[0], a[1]], [b[0], b[1]], [a[0], b[1]]];
        let lines: Vec<usize> = (0..4).map(|i| self.add_line(corners[i], corners[(i + 1) % 4])).collect();
        for i in 0..4 {
            let end = self.line_points(lines[i]).1;
            let start = self.line_points(lines[(i + 1) % 4]).0;
            self.constraints.push(Constraint::Coincident(end, start));
        }
        self.constraints.push(Constraint::Horizontal(lines[0]));
        self.constraints.push(Constraint::Horizontal(lines[2]));
        self.constraints.push(Constraint::Vertical(lines[1]));
        self.constraints.push(Constraint::Vertical(lines[3]));
        [lines[0], lines[1], lines[2], lines[3]]
    }

    /// A regular polygon inscribed in a construction circle around point
    /// `center`, with one vertex at `vertex`, like Fusion's inscribed
    /// polygon: sides of equal length, vertices on the circle. Returns
    /// (the circle, the side lines).
    pub fn add_polygon(&mut self, center: usize, vertex: P2, sides: usize) -> (usize, Vec<usize>) {
        let sides = sides.clamp(3, 64);
        let c = self.points[center];
        let r = dist(c, vertex).max(1e-6);
        let a0 = (vertex[1] - c[1]).atan2(vertex[0] - c[0]);
        self.curves.push(Curve::Circle { center, radius: r, construction: true });
        let circle = self.curves.len() - 1;
        let corners: Vec<usize> = (0..sides)
            .map(|i| {
                let a = a0 + std::f64::consts::TAU * i as f64 / sides as f64;
                self.add_point([c[0] + r * a.cos(), c[1] + r * a.sin()])
            })
            .collect();
        let mut lines = Vec::with_capacity(sides);
        for i in 0..sides {
            self.curves.push(Curve::Line { a: corners[i], b: corners[(i + 1) % sides], construction: false });
            lines.push(self.curves.len() - 1);
            self.constraints.push(Constraint::PointOnCircle(corners[i], circle));
        }
        for &l in &lines[1..] {
            self.constraints.push(Constraint::Equal(lines[0], l));
        }
        (circle, lines)
    }

    /// A center-to-center slot of `width` between points `c1` and `c2`:
    /// two half arcs around the centers joined by two tangent lines, with a
    /// construction line between the centers. Returns (top, bottom, arc at
    /// c1, arc at c2).
    pub fn add_slot(&mut self, c1: usize, c2: usize, width: f64) -> [usize; 4] {
        let (a, b) = (self.points[c1], self.points[c2]);
        let len = dist(a, b).max(1e-9);
        let u = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
        let n = [-u[1], u[0]];
        let r = (width / 2.0).max(1e-6);
        let at = |p: P2, s: f64| [p[0] + n[0] * r * s, p[1] + n[1] * r * s];
        let (p1, p2, p3, p4) = (self.add_point(at(a, 1.0)), self.add_point(at(b, 1.0)), self.add_point(at(b, -1.0)), self.add_point(at(a, -1.0)));
        self.curves.push(Curve::Line { a: p1, b: p2, construction: false });
        let top = self.curves.len() - 1;
        // Counter-clockwise arcs bulging away from the other center.
        self.curves.push(Curve::Arc { center: c2, start: p3, end: p2, construction: false });
        let arc2 = self.curves.len() - 1;
        self.curves.push(Curve::Line { a: p3, b: p4, construction: false });
        let bottom = self.curves.len() - 1;
        self.curves.push(Curve::Arc { center: c1, start: p1, end: p4, construction: false });
        let arc1 = self.curves.len() - 1;
        self.curves.push(Curve::Line { a: c1, b: c2, construction: true });
        for line in [top, bottom] {
            for arc in [arc1, arc2] {
                self.constraints.push(Constraint::Tangent(line, arc));
            }
        }
        self.constraints.push(Constraint::Equal(arc1, arc2));
        [top, bottom, arc1, arc2]
    }

    pub fn line_points(&self, line: usize) -> (usize, usize) {
        match self.curves[line] {
            Curve::Line { a, b, .. } => (a, b),
            _ => panic!("curve {line} is not a line"),
        }
    }

    /// Radius of a circle or arc, from the current geometry.
    pub fn radius(&self, curve: usize) -> f64 {
        match self.curves[curve] {
            Curve::Circle { radius, .. } => radius,
            Curve::Arc { center, start, .. } => dist(self.points[center], self.points[start]),
            Curve::Line { .. } => 0.0,
        }
    }

    pub fn line_length(&self, line: usize) -> f64 {
        let (a, b) = self.line_points(line);
        dist(self.points[a], self.points[b])
    }

    /// Move the geometry to satisfy every constraint, staying close to the
    /// current positions.
    pub fn solve(&mut self) -> Result<SolveReport, SolveError> {
        solver::solve(self, None)
    }

    /// Solve while pulling `point` toward `target` (for dragging): other
    /// geometry follows only as the constraints require.
    pub fn solve_drag(&mut self, point: usize, target: P2) -> Result<SolveReport, SolveError> {
        solver::solve(self, Some((point, target)))
    }

    /// Closed regions bounded by non-construction curves, ready to extrude.
    pub fn regions(&self) -> Vec<Region> {
        profiles::regions(self)
    }

    /// Remove a curve and any constraints that mention it or its now-unused points.
    pub fn delete_curve(&mut self, curve: usize) {
        if curve >= self.curves.len() {
            return;
        }
        self.curves.remove(curve);
        let fix_curve = |c: usize| if c > curve { Some(c - 1) } else if c == curve { None } else { Some(c) };
        self.constraints.retain_mut(|k| {
            use Constraint::*;
            let curves: &mut [&mut usize] = &mut match k {
                Horizontal(l) | Vertical(l) | Length(l, _) | Radius(l, _) => vec![l],
                Parallel(a, b) | Perpendicular(a, b) | Equal(a, b) | Tangent(a, b) | Concentric(a, b) => vec![a, b],
                PointOnLine(_, l) | PointOnCircle(_, l) | Midpoint(_, l) | Symmetric(_, _, l) => vec![l],
                _ => vec![],
            };
            for c in curves.iter_mut() {
                match fix_curve(**c) {
                    Some(new) => **c = new,
                    None => return false,
                }
            }
            true
        });
        self.remove_unused_points();
    }

    /// Drop points no curve uses, with the constraints that mention them.
    pub fn remove_unused_points(&mut self) {
        let mut used = vec![false; self.points.len()];
        for c in &self.curves {
            for p in c.points() {
                used[p] = true;
            }
        }
        let mut remap = vec![usize::MAX; self.points.len()];
        let mut kept = Vec::new();
        for (i, p) in self.points.iter().enumerate() {
            if used[i] {
                remap[i] = kept.len();
                kept.push(*p);
            }
        }
        self.points = kept;
        for c in &mut self.curves {
            match c {
                Curve::Line { a, b, .. } => {
                    *a = remap[*a];
                    *b = remap[*b];
                }
                Curve::Circle { center, .. } => *center = remap[*center],
                Curve::Arc { center, start, end, .. } => {
                    *center = remap[*center];
                    *start = remap[*start];
                    *end = remap[*end];
                }
            }
        }
        self.constraints.retain_mut(|k| {
            use Constraint::*;
            let pts: Vec<&mut usize> = match k {
                Coincident(a, b) | Distance(a, b, _) | HorizontalDistance(a, b, _) | VerticalDistance(a, b, _) | Symmetric(a, b, _) => vec![a, b],
                Fixed(p) | PointOnLine(p, _) | PointOnCircle(p, _) | Midpoint(p, _) => vec![p],
                _ => vec![],
            };
            for p in pts {
                if remap[*p] == usize::MAX {
                    return false;
                }
                *p = remap[*p];
            }
            true
        });
    }
}

pub fn dist(a: P2, b: P2) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

#[cfg(test)]
mod tests;

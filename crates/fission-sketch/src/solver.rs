//! Constraint solver: damped least squares (Levenberg–Marquardt) over point
//! coordinates and circle radii, with a numeric Jacobian. Sketches are small
//! (tens to a few hundred unknowns), so dense linear algebra is plenty.
//!
//! Every residual is in millimetres, so mixed constraints balance. Starting
//! from the current geometry and damping steps keeps the solution close to
//! what the user drew when the sketch is under-constrained.

use crate::{dist, Constraint, Curve, Sketch, P2};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SolveReport {
    pub iterations: usize,
    /// Remaining degrees of freedom (0 = fully constrained).
    pub dof: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SolveError {
    /// The constraints can't all hold at once; geometry is left as it was.
    Conflicting { residual: f64 },
}

impl std::fmt::Display for SolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SolveError::Conflicting { residual } => {
                write!(f, "constraints conflict (off by {residual:.3} mm)")
            }
        }
    }
}

const TOLERANCE: f64 = 1e-7;
const MAX_ITERATIONS: usize = 200;
/// How hard a dragged point pulls compared to real constraints.
const DRAG_WEIGHT: f64 = 1e-3;

struct Layout {
    /// Index of each circle's radius unknown (after the point coordinates).
    radius_var: Vec<Option<usize>>,
    n: usize,
}

impl Layout {
    fn new(sketch: &Sketch) -> Layout {
        let mut n = sketch.points.len() * 2;
        let radius_var = sketch
            .curves
            .iter()
            .map(|c| match c {
                Curve::Circle { .. } => {
                    n += 1;
                    Some(n - 1)
                }
                _ => None,
            })
            .collect();
        Layout { radius_var, n }
    }

    fn pack(&self, sketch: &Sketch) -> Vec<f64> {
        let mut x: Vec<f64> = sketch.points.iter().flat_map(|p| [p[0], p[1]]).collect();
        x.resize(self.n, 0.0);
        for (curve, var) in sketch.curves.iter().zip(&self.radius_var) {
            if let (Curve::Circle { radius, .. }, Some(v)) = (curve, var) {
                x[*v] = *radius;
            }
        }
        x
    }

    fn unpack(&self, x: &[f64], sketch: &mut Sketch) {
        for (i, p) in sketch.points.iter_mut().enumerate() {
            *p = [x[2 * i], x[2 * i + 1]];
        }
        for (curve, var) in sketch.curves.iter_mut().zip(&self.radius_var) {
            if let (Curve::Circle { radius, .. }, Some(v)) = (curve, var) {
                *radius = x[*v];
            }
        }
    }
}

/// Residuals of all constraints (plus implicit arc ones), and optionally the
/// weighted drag pull last.
fn residuals(sketch: &Sketch, layout: &Layout, x: &[f64], origin: &[f64], drag: Option<(usize, P2)>, out: &mut Vec<f64>) {
    out.clear();
    let p = |i: usize| [x[2 * i], x[2 * i + 1]];
    let line = |l: usize| match sketch.curves[l] {
        Curve::Line { a, b, .. } => (p(a), p(b)),
        _ => (p(0), p(0)),
    };
    let center = |c: usize| match sketch.curves[c] {
        Curve::Circle { center, .. } | Curve::Arc { center, .. } => p(center),
        Curve::Line { a, .. } => p(a),
    };
    let radius = |c: usize| match (&sketch.curves[c], layout.radius_var[c]) {
        (Curve::Circle { .. }, Some(v)) => x[v],
        (Curve::Arc { center, start, .. }, _) => dist(p(*center), p(*start)),
        _ => 0.0,
    };
    let sub = |a: P2, b: P2| [a[0] - b[0], a[1] - b[1]];
    let cross = |a: P2, b: P2| a[0] * b[1] - a[1] * b[0];
    let dot = |a: P2, b: P2| a[0] * b[0] + a[1] * b[1];
    let len = |a: P2| (a[0] * a[0] + a[1] * a[1]).sqrt().max(1e-12);

    for k in &sketch.constraints {
        match *k {
            Constraint::Coincident(a, b) => {
                let d = sub(p(a), p(b));
                out.extend(d);
            }
            Constraint::Horizontal(l) => {
                let (a, b) = line(l);
                out.push(a[1] - b[1]);
            }
            Constraint::Vertical(l) => {
                let (a, b) = line(l);
                out.push(a[0] - b[0]);
            }
            Constraint::Fixed(i) => {
                out.push((x[2 * i] - origin[2 * i]) * 10.0);
                out.push((x[2 * i + 1] - origin[2 * i + 1]) * 10.0);
            }
            Constraint::Distance(a, b, d) => out.push(dist(p(a), p(b)) - d),
            Constraint::HorizontalDistance(a, b, d) => out.push(p(b)[0] - p(a)[0] - d),
            Constraint::VerticalDistance(a, b, d) => out.push(p(b)[1] - p(a)[1] - d),
            Constraint::Length(l, d) => {
                let (a, b) = line(l);
                out.push(dist(a, b) - d);
            }
            Constraint::Radius(c, r) => out.push(radius(c) - r),
            Constraint::Parallel(l1, l2) | Constraint::Perpendicular(l1, l2) => {
                let (a1, b1) = line(l1);
                let (a2, b2) = line(l2);
                let (d1, d2) = (sub(b1, a1), sub(b2, a2));
                let scale = (len(d1) + len(d2)) / 2.0;
                let v = if matches!(k, Constraint::Parallel(..)) { cross(d1, d2) } else { dot(d1, d2) };
                out.push(v / (len(d1) * len(d2)) * scale);
            }
            Constraint::Equal(c1, c2) => match (&sketch.curves[c1], &sketch.curves[c2]) {
                (Curve::Line { .. }, Curve::Line { .. }) => {
                    let (a1, b1) = line(c1);
                    let (a2, b2) = line(c2);
                    out.push(dist(a1, b1) - dist(a2, b2));
                }
                _ => out.push(radius(c1) - radius(c2)),
            },
            Constraint::PointOnLine(pt, l) => {
                let (a, b) = line(l);
                let d = sub(b, a);
                out.push(cross(d, sub(p(pt), a)) / len(d));
            }
            Constraint::PointOnCircle(pt, c) => out.push(dist(p(pt), center(c)) - radius(c)),
            Constraint::Midpoint(pt, l) => {
                let (a, b) = line(l);
                out.push(p(pt)[0] - (a[0] + b[0]) / 2.0);
                out.push(p(pt)[1] - (a[1] + b[1]) / 2.0);
            }
            Constraint::Tangent(l, c) => {
                let (a, b) = line(l);
                let d = sub(b, a);
                out.push((cross(d, sub(center(c), a)) / len(d)).abs() - radius(c));
            }
            Constraint::Concentric(c1, c2) => {
                out.extend(sub(center(c1), center(c2)));
            }
            Constraint::Symmetric(a, b, l) => {
                // The midpoint on the line, and a→b across it.
                let (la, lb) = line(l);
                let d = sub(lb, la);
                let mid = [(p(a)[0] + p(b)[0]) / 2.0, (p(a)[1] + p(b)[1]) / 2.0];
                out.push(cross(d, sub(mid, la)) / len(d));
                out.push(dot(d, sub(p(b), p(a))) / len(d));
            }
        }
    }
    // An arc's ends stay on one circle.
    for c in &sketch.curves {
        if let Curve::Arc { center, start, end, .. } = *c {
            out.push(dist(p(center), p(end)) - dist(p(center), p(start)));
        }
    }
    if let Some((i, target)) = drag {
        out.push((x[2 * i] - target[0]) * DRAG_WEIGHT);
        out.push((x[2 * i + 1] - target[1]) * DRAG_WEIGHT);
    }
}

fn norm(v: &[f64]) -> f64 {
    v.iter().map(|x| x * x).sum::<f64>().sqrt()
}

fn jacobian(
    sketch: &Sketch,
    layout: &Layout,
    x: &mut [f64],
    origin: &[f64],
    drag: Option<(usize, P2)>,
    m: usize,
) -> Vec<Vec<f64>> {
    let mut jac = vec![vec![0.0; layout.n]; m];
    let (mut plus, mut minus) = (Vec::with_capacity(m), Vec::with_capacity(m));
    for j in 0..layout.n {
        let h = 1e-7 * x[j].abs().max(1.0);
        let keep = x[j];
        x[j] = keep + h;
        residuals(sketch, layout, x, origin, drag, &mut plus);
        x[j] = keep - h;
        residuals(sketch, layout, x, origin, drag, &mut minus);
        x[j] = keep;
        for i in 0..m {
            jac[i][j] = (plus[i] - minus[i]) / (2.0 * h);
        }
    }
    jac
}

/// Solve (A + λ·diag) dx = b by Gaussian elimination with partial pivoting.
#[allow(clippy::needless_range_loop)] // row operations index two rows at once
fn solve_linear(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for col in 0..n {
        let pivot = (col..n).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[pivot][col].abs() < 1e-300 {
            return None;
        }
        a.swap(col, pivot);
        b.swap(col, pivot);
        for row in col + 1..n {
            let f = a[row][col] / a[col][col];
            if f != 0.0 {
                for k in col..n {
                    a[row][k] -= f * a[col][k];
                }
                b[row] -= f * b[col];
            }
        }
    }
    let mut x = vec![0.0; n];
    for row in (0..n).rev() {
        let s: f64 = (row + 1..n).map(|k| a[row][k] * x[k]).sum();
        x[row] = (b[row] - s) / a[row][row];
    }
    Some(x)
}

/// Numerical rank by elimination with a relative tolerance.
#[allow(clippy::needless_range_loop)]
fn rank(mut a: Vec<Vec<f64>>) -> usize {
    let (rows, cols) = (a.len(), a.first().map_or(0, Vec::len));
    let scale = a.iter().flatten().fold(0.0f64, |m, v| m.max(v.abs())).max(1e-12);
    let mut r = 0;
    for col in 0..cols {
        let Some(pivot) = (r..rows).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs())) else { break };
        if a[pivot][col].abs() <= 1e-9 * scale {
            continue;
        }
        a.swap(r, pivot);
        for row in r + 1..rows {
            let f = a[row][col] / a[r][col];
            for k in col..cols {
                a[row][k] -= f * a[r][k];
            }
        }
        r += 1;
        if r == rows {
            break;
        }
    }
    r
}

pub(crate) fn solve(sketch: &mut Sketch, drag: Option<(usize, P2)>) -> Result<SolveReport, SolveError> {
    let layout = Layout::new(sketch);
    let origin = layout.pack(sketch);
    let mut x = origin.clone();
    let mut r = Vec::new();
    residuals(sketch, &layout, &x, &origin, drag, &mut r);
    let m = r.len();
    let constraint_rows = m - if drag.is_some() { 2 } else { 0 };

    let mut lambda = 1e-3;
    let mut iterations = 0;
    let mut cost = norm(&r);
    let mut trial = Vec::new();
    // Without a drag, stop once constraints hold. With one, keep pulling until
    // the weighted pull stops improving.
    while iterations < MAX_ITERATIONS && (drag.is_some() || norm(&r[..constraint_rows]) > TOLERANCE) {
        iterations += 1;
        let jac = jacobian(sketch, &layout, &mut x, &origin, drag, m);
        // Normal equations: (JᵀJ + λ diag(JᵀJ) + λ I) dx = -Jᵀr
        let n = layout.n;
        let mut jtj = vec![vec![0.0; n]; n];
        let mut jtr = vec![0.0; n];
        for i in 0..m {
            for a in 0..n {
                if jac[i][a] == 0.0 {
                    continue;
                }
                jtr[a] -= jac[i][a] * r[i];
                for b in 0..n {
                    jtj[a][b] += jac[i][a] * jac[i][b];
                }
            }
        }
        let mut improved = false;
        for _ in 0..12 {
            let mut damped = jtj.clone();
            for d in 0..n {
                damped[d][d] += lambda * (jtj[d][d] + 1.0);
            }
            let Some(dx) = solve_linear(damped, jtr.clone()) else {
                lambda *= 10.0;
                continue;
            };
            let candidate: Vec<f64> = x.iter().zip(&dx).map(|(a, d)| a + d).collect();
            residuals(sketch, &layout, &candidate, &origin, drag, &mut trial);
            let new_cost = norm(&trial);
            if new_cost < cost {
                let gain = cost - new_cost;
                x = candidate;
                std::mem::swap(&mut r, &mut trial);
                cost = new_cost;
                if drag.is_some() && gain < 1e-12 && norm(&r[..constraint_rows]) <= TOLERANCE {
                    break;
                }
                lambda = (lambda / 10.0).max(1e-12);
                improved = true;
                break;
            }
            lambda *= 10.0;
        }
        if !improved {
            break;
        }
    }

    let residual = norm(&r[..constraint_rows]);
    if residual > 1e-5 {
        return Err(SolveError::Conflicting { residual });
    }
    layout.unpack(&x, sketch);

    // Degrees of freedom: unknowns minus the rank of the constraint Jacobian.
    let jac = jacobian(sketch, &layout, &mut x, &origin, None, constraint_rows);
    let dof = layout.n - rank(jac);
    Ok(SolveReport { iterations, dof })
}

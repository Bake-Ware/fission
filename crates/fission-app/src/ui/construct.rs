//! CONSTRUCT ▸ planes, axes and points found from picked model geometry
//! (edges, their ends, faces), and drawing axes and points in the viewport.

use eframe::egui::{self, Color32, Stroke};
use fission_kernel::Mesh;
use glam::{DMat3, DVec3};

use super::theme;
use crate::app::FissionApp;

/// A point on the line and its unit direction.
pub type Line = (DVec3, DVec3);

fn dv(p: [f32; 3]) -> DVec3 {
    glam::Vec3::from(p).as_dvec3()
}

/// A straight edge's line, or a circular edge's axis (through its center,
/// along its normal).
pub fn edge_axis(pts: &[[f32; 3]]) -> Option<Line> {
    straight_edge(pts).or_else(|| circle_axis(pts))
}

/// A straight edge's line (through its midpoint), or None if it bends.
pub fn straight_edge(pts: &[[f32; 3]]) -> Option<Line> {
    let pts: Vec<DVec3> = pts.iter().copied().map(dv).collect();
    let (a, b) = (*pts.first()?, *pts.last()?);
    let size = pts.iter().map(|p| (*p - a).length()).fold(0.0, f64::max);
    let tol = 1e-3 * size.max(1.0);
    let straight = size > 1e-9 && (b - a).length() > tol && pts.iter().all(|p| (*p - a).cross((b - a).normalize()).length() < tol);
    straight.then(|| ((a + b) / 2.0, (b - a).normalize()))
}

/// A circular edge's center and normal, or None if it isn't round.
pub fn circle_axis(pts: &[[f32; 3]]) -> Option<Line> {
    let pts: Vec<DVec3> = pts.iter().copied().map(dv).collect();
    let a = *pts.first()?;
    let size = pts.iter().map(|p| (*p - a).length()).fold(0.0, f64::max);
    if size < 1e-9 {
        return None;
    }
    let tol = 1e-3 * size.max(1.0);
    // Newell's method: the normal of the (implicitly closed) polygon.
    let mut n = DVec3::ZERO;
    for (p, q) in pts.iter().zip(pts.iter().cycle().skip(1)) {
        n += p.cross(*q);
    }
    let n = n.try_normalize()?;
    let center = circle_center(&pts, n)?;
    let r = (pts[0] - center).reject_from(n).length();
    let round = pts.iter().all(|p| ((*p - center).reject_from(n).length() - r).abs() < tol);
    round.then_some((center, n))
}

/// A cylindrical face's axis, from its tessellation: every normal is
/// perpendicular to the axis, and the points lie on a circle around it.
pub fn cylinder_axis(points: &[DVec3], normals: &[DVec3]) -> Option<Line> {
    let n0 = normals.first()?.try_normalize()?;
    let other = normals.iter().max_by(|a, b| n0.cross(**a).length().total_cmp(&n0.cross(**b).length()))?;
    let dir = n0.cross(*other).try_normalize()?;
    if n0.cross(*other).length() < 0.1 || normals.iter().any(|n| n.normalize_or_zero().dot(dir).abs() > 0.02) {
        return None;
    }
    let center = circle_center(points, dir)?;
    // Center the line along the face.
    let mid = points.iter().map(|p| (*p - center).dot(dir)).sum::<f64>() / points.len() as f64;
    Some((center + dir * mid, dir))
}

/// Where two non-parallel planes meet.
pub fn planes_axis((p1, n1): Line, (p2, n2): Line) -> Option<Line> {
    let dir = n1.cross(n2);
    if dir.length() < 1e-3 {
        return None;
    }
    let dir = dir.normalize();
    // n1·x = n1·p1, n2·x = n2·p2, and the point nearest the picks along dir.
    let rows = DMat3::from_cols(n1, n2, dir).transpose();
    let rhs = DVec3::new(n1.dot(p1), n2.dot(p2), dir.dot((p1 + p2) / 2.0));
    (rows.determinant().abs() > 1e-12).then(|| (rows.inverse() * rhs, dir))
}

/// The end of an edge nearest `click` (a vertex, since picks are of edges).
pub fn nearest_end(pts: &[[f32; 3]], click: DVec3) -> Option<DVec3> {
    let (a, b) = (dv(*pts.first()?), dv(*pts.last()?));
    Some(if (a - click).length() <= (b - click).length() { a } else { b })
}

/// Where two lines cross (the midpoint of their closest approach, if they
/// come within a hair of each other).
pub fn lines_meet((p1, d1): Line, (p2, d2): Line) -> Option<DVec3> {
    let n = d1.cross(d2);
    if n.length() < 1e-6 {
        return None;
    }
    // p1 + s·d1 and p2 + t·d2 closest where the gap is square to both.
    let w = p2 - p1;
    let s = w.cross(d2).dot(n) / n.length_squared();
    let t = w.cross(d1).dot(n) / n.length_squared();
    let (a, b) = (p1 + d1 * s, p2 + d2 * t);
    let scale = (p1 - p2).length().max(1.0);
    ((a - b).length() < 1e-3 * scale).then(|| (a + b) / 2.0)
}

/// Where a line goes through a plane (point, normal).
pub fn line_meets_plane((p, d): Line, (o, n): Line) -> Option<DVec3> {
    let along = d.dot(n);
    (along.abs() > 1e-9).then(|| p + d * ((o - p).dot(n) / along))
}

/// The point on all three planes.
pub fn three_planes_meet(a: Line, b: Line, c: Line) -> Option<DVec3> {
    let rows = DMat3::from_cols(a.1, b.1, c.1).transpose();
    let rhs = DVec3::new(a.1.dot(a.0), b.1.dot(b.0), c.1.dot(c.0));
    (rows.determinant().abs() > 1e-9).then(|| rows.inverse() * rhs)
}

/// The plane (point, unit normal) through three points not in a line.
pub fn plane_through_points(a: DVec3, b: DVec3, c: DVec3) -> Option<Line> {
    let n = (b - a).cross(c - a);
    let scale = (b - a).length().max((c - a).length()).max(1e-9);
    (n.length() > 1e-6 * scale * scale).then(|| ((a + b + c) / 3.0, n.normalize()))
}

/// The plane holding two straight edges (parallel, or crossing).
pub fn plane_through_lines((p1, d1): Line, (p2, d2): Line) -> Option<Line> {
    let w = p2 - p1;
    let n = if d1.cross(d2).length() < 1e-6 { d1.cross(w) } else { d1.cross(d2) };
    let n = n.try_normalize()?;
    // Skew lines don't share a plane.
    (w.dot(n).abs() < 1e-3 * w.length().max(1.0)).then(|| ((p1 + p2) / 2.0, n))
}

/// Least-squares circle center of points seen along `n` (Kåsa fit), in 3D.
fn circle_center(points: &[DVec3], n: DVec3) -> Option<DVec3> {
    if points.len() < 3 {
        return None;
    }
    let mean = points.iter().copied().sum::<DVec3>() / points.len() as f64;
    let u = n.any_orthonormal_vector();
    let v = n.cross(u);
    let (mut m, mut rhs) = (DMat3::ZERO, DVec3::ZERO);
    for p in points {
        let d = *p - mean;
        let (x, y) = (d.dot(u), d.dot(v));
        let row = DVec3::new(x, y, 1.0);
        m += DMat3::from_cols(row * x, row * y, row);
        rhs -= row * (x * x + y * y);
    }
    if m.determinant().abs() < 1e-18 {
        return None;
    }
    let s = m.inverse() * rhs;
    Some(mean + u * (-s.x / 2.0) + v * (-s.y / 2.0))
}

/// A face's tessellated points and normals.
pub fn face_samples(mesh: &Mesh, face: u32) -> (Vec<DVec3>, Vec<DVec3>) {
    let mut seen = std::collections::BTreeSet::new();
    for (t, tri) in mesh.indices.chunks_exact(3).enumerate() {
        if mesh.face_ids.get(t) == Some(&face) {
            seen.extend(tri.iter().copied());
        }
    }
    seen.into_iter().map(|i| (dv(mesh.positions[i as usize]), dv(mesh.normals[i as usize]))).unzip()
}

/// Construction points: small orange dots with their names.
pub fn draw_construction_points(app: &FissionApp, ui: &egui::Ui) {
    let painter = ui.painter_at(app.viewport.rect);
    for (i, pos) in app.doc.construction_points() {
        if let Some(at) = app.viewport.to_screen(DVec3::from_array(pos).as_vec3()) {
            painter.circle(at, 3.5, Color32::from_rgb(210, 140, 50), Stroke::new(1.0, Color32::WHITE));
            painter.text(at + egui::vec2(6.0, -4.0), egui::Align2::LEFT_BOTTOM, &app.doc.features[i].name, egui::FontId::proportional(12.0), theme::TEXT);
        }
    }
}

/// Construction axes, dashed orange with their names, as Fusion draws them.
pub fn draw_construction_axes(app: &FissionApp, ui: &egui::Ui) {
    let painter = ui.painter_at(app.viewport.rect);
    let half = app.viewport.camera.distance as f64 * 0.3;
    for (i, origin, dir) in app.doc.construction_axes() {
        let (o, d) = (DVec3::from_array(origin), DVec3::from_array(dir));
        let ends = [o - d * half, o + d * half].map(|p| app.viewport.to_screen(p.as_vec3()));
        if let [Some(a), Some(b)] = ends {
            painter.add(egui::Shape::dashed_line(&[a, b], Stroke::new(1.5, Color32::from_rgb(210, 140, 50)), 8.0, 5.0));
            painter.text(b, egui::Align2::LEFT_BOTTOM, &app.doc.features[i].name, egui::FontId::proportional(12.0), theme::TEXT);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::TAU;

    fn close(a: DVec3, b: DVec3) -> bool {
        (a - b).length() < 1e-6
    }

    #[test]
    fn edges_give_their_line_or_circle_axis() {
        let (o, d) = edge_axis(&[[0.0, 0.0, 0.0], [5.0, 0.0, 0.0], [10.0, 0.0, 0.0]]).unwrap();
        assert!(close(o, DVec3::new(5.0, 0.0, 0.0)) && close(d, DVec3::X));
        // A circle of radius 3 around (1, 2) at z = 4.
        let ring: Vec<[f32; 3]> = (0..=32)
            .map(|k| {
                let t = TAU * k as f64 / 32.0;
                [(1.0 + 3.0 * t.cos()) as f32, (2.0 + 3.0 * t.sin()) as f32, 4.0]
            })
            .collect();
        let (c, n) = edge_axis(&ring).unwrap();
        assert!((c - DVec3::new(1.0, 2.0, 4.0)).length() < 1e-4, "{c}");
        assert!(n.cross(DVec3::Z).length() < 1e-6);
    }

    #[test]
    fn cylinder_faces_give_their_axis() {
        // Radius 5 around the vertical line through (10, -3), z 0..8.
        let (mut pts, mut ns) = (Vec::new(), Vec::new());
        for k in 0..24 {
            let t = TAU * k as f64 / 24.0;
            let radial = DVec3::new(t.cos(), t.sin(), 0.0);
            for z in [0.0, 8.0] {
                pts.push(DVec3::new(10.0, -3.0, z) + radial * 5.0);
                ns.push(radial);
            }
        }
        let (o, d) = cylinder_axis(&pts, &ns).unwrap();
        assert!(d.cross(DVec3::Z).length() < 1e-9);
        assert!(close(o, DVec3::new(10.0, -3.0, 4.0)), "{o}");
        // A flat face isn't a cylinder.
        assert!(cylinder_axis(&pts, &vec![DVec3::Z; pts.len()]).is_none());
    }

    #[test]
    fn points_and_planes_from_picked_geometry() {
        let x = (DVec3::new(0.0, 0.0, 5.0), DVec3::X);
        let y = (DVec3::new(3.0, 1.0, 5.0), DVec3::Y);
        assert!(close(lines_meet(x, y).unwrap(), DVec3::new(3.0, 0.0, 5.0)));
        // Skew lines (one lifted) don't meet.
        assert!(lines_meet(x, (DVec3::new(3.0, 1.0, 6.0), DVec3::Y)).is_none());
        let floor = (DVec3::ZERO, DVec3::Z);
        assert!(close(line_meets_plane((DVec3::new(1.0, 2.0, 7.0), DVec3::Z), floor).unwrap(), DVec3::new(1.0, 2.0, 0.0)));
        let corner = three_planes_meet((DVec3::new(4.0, 0.0, 0.0), DVec3::X), (DVec3::new(0.0, -2.0, 0.0), DVec3::Y), floor);
        assert!(close(corner.unwrap(), DVec3::new(4.0, -2.0, 0.0)));

        let (o, n) = plane_through_points(DVec3::ZERO, DVec3::X, DVec3::Y).unwrap();
        assert!(n.cross(DVec3::Z).length() < 1e-9 && o.z.abs() < 1e-12);
        assert!(plane_through_points(DVec3::ZERO, DVec3::X, DVec3::X * 2.0).is_none());
        // Two parallel edges of a box's top face: the top plane.
        let (_, n) = plane_through_lines(x, (DVec3::new(0.0, 7.0, 5.0), DVec3::X)).unwrap();
        assert!(n.cross(DVec3::Z).length() < 1e-9);
        assert!(plane_through_lines(x, (DVec3::new(3.0, 1.0, 6.0), DVec3::Y)).is_none());

        let edge = [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]];
        assert!(close(nearest_end(&edge, DVec3::new(8.0, 1.0, 0.0)).unwrap(), DVec3::new(10.0, 0.0, 0.0)));
        assert!(circle_axis(&edge).is_none() && straight_edge(&edge).is_some());
    }

    #[test]
    fn two_planes_meet_in_a_line() {
        // Top of a box at z = 10 and its side at x = 20: the edge between them.
        let (o, d) = planes_axis((DVec3::new(3.0, 1.0, 10.0), DVec3::Z), (DVec3::new(20.0, 5.0, 2.0), DVec3::X)).unwrap();
        assert!(d.cross(DVec3::Y).length() < 1e-9);
        assert!((o.x - 20.0).abs() < 1e-9 && (o.z - 10.0).abs() < 1e-9 && (o.y - 3.0).abs() < 1e-9, "{o}");
        assert!(planes_axis((DVec3::ZERO, DVec3::Z), (DVec3::ONE, DVec3::Z)).is_none());
    }
}

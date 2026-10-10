use super::*;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-5
}

fn rect_sketch() -> (Sketch, [usize; 4]) {
    let mut s = Sketch::default();
    let lines = s.add_rectangle([0.0, 0.0], [40.0, 20.0]);
    (s, lines)
}

#[test]
fn rectangle_keeps_shape_and_takes_dimensions() {
    let (mut s, [bottom, right, top, left]) = rect_sketch();
    // Make it 60 x 25 with its corner pinned at the origin.
    let corner = s.line_points(bottom).0;
    s.constraints.push(Constraint::Fixed(corner));
    s.constraints.push(Constraint::Length(bottom, 60.0));
    s.constraints.push(Constraint::Length(left, 25.0));
    let report = s.solve().unwrap();
    assert!(close(s.line_length(bottom), 60.0));
    assert!(close(s.line_length(top), 60.0), "top follows via horizontals + coincidences");
    assert!(close(s.line_length(right), 25.0));
    assert!(close(s.points[corner][0], 0.0) && close(s.points[corner][1], 0.0));
    assert_eq!(report.dof, 0, "fixed corner + two dimensions fully constrain a rectangle");
}

#[test]
fn unconstrained_rectangle_has_four_dof() {
    // Position (2) + width + height.
    let (mut s, _) = rect_sketch();
    assert_eq!(s.solve().unwrap().dof, 4);
}

#[test]
fn conflicting_dimensions_are_reported_and_leave_geometry_alone() {
    let (mut s, [bottom, _, top, _]) = rect_sketch();
    s.constraints.push(Constraint::Length(bottom, 10.0));
    s.constraints.push(Constraint::Length(top, 30.0));
    let before = s.points.clone();
    assert!(matches!(s.solve(), Err(SolveError::Conflicting { .. })));
    assert_eq!(s.points, before);
}

#[test]
fn circle_radius_and_tangent_line() {
    let mut s = Sketch::default();
    let c = s.add_circle([0.0, 0.0], 5.0);
    let l = s.add_line([-10.0, 8.0], [10.0, 9.0]);
    s.constraints.push(Constraint::Radius(c, 7.0));
    s.constraints.push(Constraint::Horizontal(l));
    s.constraints.push(Constraint::Tangent(l, c));
    let center = 0;
    s.constraints.push(Constraint::Fixed(center));
    s.solve().unwrap();
    assert!(close(s.radius(c), 7.0));
    let (a, b) = s.line_points(l);
    assert!(close(s.points[a][1], s.points[b][1]));
    assert!(close(s.points[a][1].abs(), 7.0), "line sits on the circle");
}

#[test]
fn arc_ends_stay_on_one_circle() {
    let mut s = Sketch::default();
    let arc = s.add_arc([0.0, 0.0], [10.0, 0.0], [0.0, 12.0]);
    s.solve().unwrap();
    let Curve::Arc { center, start, end, .. } = s.curves[arc] else { unreachable!() };
    assert!(close(dist(s.points[center], s.points[start]), dist(s.points[center], s.points[end])));
}

#[test]
fn drag_moves_free_geometry_and_respects_constraints() {
    let (mut s, [bottom, ..]) = rect_sketch();
    let (a, b) = s.line_points(bottom);
    s.constraints.push(Constraint::Fixed(a));
    s.solve().unwrap();
    // Pull the bottom-right corner up and out; the bottom edge must stay horizontal.
    s.solve_drag(b, [50.0, 5.0]).unwrap();
    assert!(close(s.points[a][1], s.points[b][1]), "still horizontal");
    assert!(s.points[b][0] > 45.0, "followed the drag in x: {:?}", s.points[b]);
    assert!(close(s.points[a][0], 0.0), "fixed corner stayed put");
}

#[test]
fn rectangle_is_one_region() {
    let (s, _) = rect_sketch();
    let regions = s.regions();
    assert_eq!(regions.len(), 1);
    assert!(close(regions[0].area, 800.0));
    assert!(regions[0].outer.signed_area() > 0.0, "outer loops are counter-clockwise");
}

#[test]
fn circle_inside_rectangle_makes_a_ring_and_a_disk() {
    let (mut s, _) = rect_sketch();
    s.add_circle([20.0, 10.0], 5.0);
    let mut regions = s.regions();
    regions.sort_by(|a, b| b.area.total_cmp(&a.area));
    assert_eq!(regions.len(), 2);
    assert_eq!(regions[0].holes.len(), 1, "the plate has the circle as a hole");
    let disk = std::f64::consts::PI * 25.0;
    assert!((regions[0].area - (800.0 - disk)).abs() < 1.0, "{}", regions[0].area);
    assert!((regions[1].area - disk).abs() < 1.0);
    assert!(regions[0].contains([2.0, 2.0]) && !regions[0].contains([20.0, 10.0]));
    assert!(regions[1].contains([20.0, 10.0]));
}

#[test]
fn a_line_across_a_rectangle_splits_it() {
    // Outline with vertices at the split, plus the splitting line.
    let mut s = Sketch::default();
    let pts = [[0.0, 0.0], [15.0, 0.0], [40.0, 0.0], [40.0, 20.0], [15.0, 20.0], [0.0, 20.0]];
    for i in 0..6 {
        s.add_line(pts[i], pts[(i + 1) % 6]);
    }
    s.add_line([15.0, 0.0], [15.0, 20.0]);
    let mut regions = s.regions();
    regions.sort_by(|a, b| a.area.total_cmp(&b.area));
    assert_eq!(regions.len(), 2, "{regions:?}");
    assert!(close(regions[0].area, 300.0) && close(regions[1].area, 500.0));
}

#[test]
fn slot_made_of_lines_and_arcs_is_one_region() {
    let mut s = Sketch::default();
    s.add_line([0.0, 0.0], [20.0, 0.0]);
    s.add_arc([20.0, 5.0], [20.0, 0.0], [20.0, 10.0]);
    s.add_line([20.0, 10.0], [0.0, 10.0]);
    s.add_arc([0.0, 5.0], [0.0, 10.0], [0.0, 0.0]);
    let regions = s.regions();
    assert_eq!(regions.len(), 1, "{regions:?}");
    let expected = 200.0 + std::f64::consts::PI * 25.0;
    assert!((regions[0].area - expected).abs() < 0.5, "{}", regions[0].area);
}

#[test]
fn construction_and_dangling_lines_make_no_regions() {
    let (mut s, lines) = rect_sketch();
    for l in lines {
        s.curves[l].set_construction(true);
    }
    s.add_line([100.0, 0.0], [120.0, 0.0]);
    assert!(s.regions().is_empty());
}

#[test]
fn delete_curve_drops_its_constraints() {
    let (mut s, [bottom, ..]) = rect_sketch();
    s.constraints.push(Constraint::Length(bottom, 50.0));
    s.delete_curve(bottom);
    assert_eq!(s.curves.len(), 3);
    assert!(!s.constraints.iter().any(|k| matches!(k, Constraint::Length(..))));
    // Indices still valid: solving works.
    s.solve().unwrap();
}

#[test]
fn polygon_stays_regular_when_its_radius_changes() {
    let mut s = Sketch::default();
    let c = s.add_point([0.0, 0.0]);
    s.constraints.push(Constraint::Fixed(c));
    let (circle, lines) = s.add_polygon(c, [10.0, 0.0], 6);
    assert_eq!(lines.len(), 6);
    assert_eq!(s.regions().len(), 1, "the hexagon is one profile (the circle is construction)");
    s.constraints.push(Constraint::Radius(circle, 20.0));
    s.solve().unwrap();
    for &l in &lines {
        assert!((s.line_length(l) - 20.0).abs() < 1e-6, "a regular hexagon's side equals its radius");
    }
}

#[test]
fn slot_is_one_profile_with_the_right_area() {
    let mut s = Sketch::default();
    let a = s.add_point([0.0, 0.0]);
    let b = s.add_point([30.0, 0.0]);
    s.constraints.push(Constraint::Fixed(a));
    s.constraints.push(Constraint::Fixed(b));
    let [_, _, arc1, _] = s.add_slot(a, b, 10.0);
    s.constraints.push(Constraint::Radius(arc1, 5.0));
    s.solve().unwrap();
    let regions = s.regions();
    assert_eq!(regions.len(), 1);
    let expected = 30.0 * 10.0 + std::f64::consts::PI * 25.0;
    assert!((regions[0].area - expected).abs() < 0.5, "{} vs {expected}", regions[0].area);
}

#[test]
fn projected_edges_become_fixed_lines_circles_and_arcs() {
    use std::f64::consts::{PI, TAU};
    let mut s = Sketch::default();
    // Four box edges, each as a two-point polyline: one closed profile.
    let corners = [[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]];
    for i in 0..4 {
        assert_eq!(s.project_polyline(&[corners[i], corners[(i + 1) % 4]]).len(), 1);
    }
    assert_eq!(s.points.len(), 4, "shared corners are reused");
    assert_eq!(s.regions().len(), 1);

    // A tessellated hole edge: a closed circle.
    let ring: Vec<[f64; 2]> = (0..=24).map(|k| {
        let t = TAU * k as f64 / 24.0;
        [10.0 + 3.0 * t.cos(), 5.0 + 3.0 * t.sin()]
    }).collect();
    let c = s.project_polyline(&ring);
    assert!(matches!(s.curves[c[0]], Curve::Circle { radius, .. } if (radius - 3.0).abs() < 1e-9));

    // A clockwise quarter arc is stored counter-clockwise.
    let arc: Vec<[f64; 2]> = (0..=8).map(|k| {
        let t = PI / 2.0 - PI / 2.0 * k as f64 / 8.0;
        [40.0 + 5.0 * t.cos(), 5.0 * t.sin()]
    }).collect();
    let a = s.project_polyline(&arc)[0];
    let Curve::Arc { start, end, .. } = s.curves[a] else { panic!("not an arc") };
    assert!(dist(s.points[start], [45.0, 0.0]) < 1e-9 && dist(s.points[end], [40.0, 5.0]) < 1e-9);

    // An ellipse (a tilted circle seen flat) falls back to lines.
    let ellipse: Vec<[f64; 2]> = (0..=16).map(|k| {
        let t = TAU * k as f64 / 16.0;
        [60.0 + 4.0 * t.cos(), 2.0 * t.sin()]
    }).collect();
    assert_eq!(s.project_polyline(&ellipse).len(), 16);
    // An edge seen end-on adds nothing.
    assert!(s.project_polyline(&[[1.0, 1.0], [1.0, 1.0]]).is_empty());

    // Everything is fixed: nothing left to solve for but the circle's
    // center, which is fixed too.
    assert_eq!(s.solve().unwrap().dof, 0);
}

#[test]
fn three_point_and_center_rectangles_stay_rectangles() {
    // Tilted: a 30° base edge, then the far side 10 mm off it.
    let mut s = Sketch::default();
    let (a, b) = (s.add_point([0.0, 0.0]), s.add_point([30f64.to_radians().cos() * 20.0, 30f64.to_radians().sin() * 20.0]));
    let lines = s.add_three_point_rectangle(a, b, [-5.0, 75f64.sqrt()]);
    let regions = s.regions();
    assert_eq!(regions.len(), 1);
    assert!(close(regions[0].area, 20.0 * 10.0), "{}", regions[0].area);
    // Dragging a corner keeps right angles: 5 degrees of freedom.
    assert_eq!(s.solve().unwrap().dof, 5);
    let far = s.line_points(lines[2]).0;
    s.solve_drag(far, [3.0, 20.0]).unwrap();
    let d = |l: usize| {
        let (p, q) = s.line_points(l);
        [s.points[q][0] - s.points[p][0], s.points[q][1] - s.points[p][1]]
    };
    let (u, v) = (d(lines[0]), d(lines[1]));
    assert!((u[0] * v[0] + u[1] * v[1]).abs() < 1e-5);

    // Centred on a fixed point: corners stay symmetric about it.
    let mut s = Sketch::default();
    let c = s.add_point([10.0, 5.0]);
    s.constraints.push(Constraint::Fixed(c));
    let lines = s.add_center_rectangle(c, [16.0, 9.0]);
    assert!(close(s.regions()[0].area, 12.0 * 8.0));
    let corner = s.line_points(lines[1]).1;
    s.solve_drag(corner, [20.0, 12.0]).unwrap();
    let bottom_left = s.points[s.line_points(lines[0]).0];
    assert!(close(bottom_left[0], 0.0) && close(bottom_left[1], -2.0), "{bottom_left:?}");
}

#[test]
fn circles_and_arcs_through_three_points() {
    let mut s = Sketch::default();
    let c = s.add_circle_through([[5.0, 0.0], [0.0, 5.0], [-5.0, 0.0]]).unwrap();
    assert!(close(s.radius(c), 5.0));
    assert!(s.add_circle_through([[0.0, 0.0], [1.0, 1.0], [2.0, 2.0]]).is_none());

    // Through the top: counter-clockwise from (5,0) to (-5,0).
    let (p, q) = (s.add_point([5.0, 0.0]), s.add_point([-5.0, 0.0]));
    let a = s.add_arc_through(p, q, [0.0, 5.0]).unwrap();
    assert!(matches!(s.curves[a], Curve::Arc { start, end, .. } if start == p && end == q));
    // Through the bottom: the ends swap so it still runs counter-clockwise.
    let a = s.add_arc_through(p, q, [0.0, -5.0]).unwrap();
    assert!(matches!(s.curves[a], Curve::Arc { start, end, .. } if start == q && end == p));
    // A big arc (more than half a circle) through a point near one end.
    let (p, q) = (s.add_point([10.0, 0.0]), s.add_point([0.0, 10.0]));
    let a = s.add_arc_through(p, q, [10.0 * 0.6f64.cos(), -10.0 * 0.6f64.sin()]).unwrap();
    assert!(matches!(s.curves[a], Curve::Arc { start, end, .. } if start == q && end == p));
}

#[test]
fn mirrored_curves_follow_the_originals() {
    // An L of two lines and a circle, mirrored across the Y axis.
    let mut s = Sketch::default();
    let axis = s.add_line([0.0, -10.0], [0.0, 30.0]);
    let (a0, a1) = s.line_points(axis);
    s.constraints.extend([Constraint::Fixed(a0), Constraint::Fixed(a1)]);
    let (top, side) = (s.add_line([0.0, 20.0], [15.0, 20.0]), s.add_line([15.0, 20.0], [15.0, 0.0]));
    let circle = s.add_circle([8.0, 8.0], 3.0);
    let copies = s.mirror(&[top, side, circle, axis], axis);
    assert_eq!(copies.len(), 3);
    // The point already on the axis is shared, not copied.
    let (t0, _) = s.line_points(top);
    let (m0, m1) = s.line_points(copies[0]);
    assert_eq!(m0, t0);
    assert!(dist(s.points[m1], [-15.0, 20.0]) < 1e-9);
    assert!(dist(s.points[s.curves[copies[2]].points()[0]], [-8.0, 8.0]) < 1e-9);
    s.solve().unwrap();

    // Move an original: its mirror image moves too.
    let (_, end) = s.line_points(side);
    s.constraints.push(Constraint::Fixed(end));
    s.points[end] = [25.0, -4.0];
    s.constraints.push(Constraint::Radius(circle, 5.0));
    s.solve().unwrap();
    let (_, mirrored_end) = s.line_points(copies[1]);
    assert!(dist(s.points[mirrored_end], [-25.0, -4.0]) < 1e-5, "{:?}", s.points[mirrored_end]);
    assert!(close(s.radius(copies[2]), 5.0));

    // Deleting the axis drops the symmetry.
    let before = s.constraints.len();
    s.delete_curve(axis);
    assert!(s.constraints.len() < before);
    assert!(!s.constraints.iter().any(|k| matches!(k, Constraint::Symmetric(..))));
}

#[test]
fn crossing_curves_split_into_regions_like_fusion() {
    // Two overlapping rectangles that share no points: three regions.
    let mut s = Sketch::default();
    s.add_rectangle([0.0, 0.0], [20.0, 10.0]);
    s.add_rectangle([10.0, 5.0], [30.0, 15.0]);
    let mut areas: Vec<f64> = s.regions().iter().map(|r| r.area).collect();
    areas.sort_by(f64::total_cmp);
    assert_eq!(areas.len(), 3, "{areas:?}");
    assert!(close(areas[0], 50.0) && close(areas[1], 150.0) && close(areas[2], 150.0), "{areas:?}");

    // A line straight through a circle: two half disks.
    let mut s = Sketch::default();
    s.add_circle([0.0, 0.0], 10.0);
    s.add_line([-20.0, 0.0], [20.0, 0.0]);
    let areas: Vec<f64> = s.regions().iter().map(|r| r.area).collect();
    assert_eq!(areas.len(), 2, "{areas:?}");
    let half = std::f64::consts::PI * 100.0 / 2.0;
    assert!(areas.iter().all(|a| (a - half).abs() < 0.05 * half), "{areas:?}");

    // A line ending on another's middle (a T) splits the rectangle it crosses.
    let mut s = Sketch::default();
    s.add_rectangle([0.0, 0.0], [20.0, 10.0]);
    s.add_line([10.0, 0.0], [10.0, 10.0]);
    assert_eq!(s.regions().len(), 2);
}

#[test]
fn a_rounded_slot_from_an_import_is_a_region() {
    // Three arcs and a line from a real Fusion sketch (one end arc is short).
    let mut s = Sketch::default();
    let c6 = s.add_point([-150.40408205773457, -10.0]);
    let p8 = s.add_point([-159.34835396773371, -1.999999999999994]);
    let p9 = s.add_point([-162.16163282309384, -7.599999999999999]);
    let c4 = s.add_point([-170.0, -6.000000000000014]);
    let p10 = s.add_point([-177.8383671769062, -7.600000000000005]);
    let c2 = s.add_point([-189.59591794226537, -9.999999999999917]);
    let p11 = s.add_point([-180.65164603226629, -1.999999999999993]);
    s.curves.push(Curve::Arc { center: c6, start: p8, end: p9, construction: false });
    s.curves.push(Curve::Arc { center: c4, start: p10, end: p9, construction: false });
    s.curves.push(Curve::Arc { center: c2, start: p10, end: p11, construction: false });
    s.curves.push(Curve::Line { a: p11, b: p8, construction: false });
    let r = s.regions();
    assert_eq!(r.len(), 1, "{r:?}");

    // Its offset, a little inside.
    let mut s = Sketch::default();
    let c6 = s.add_point([-150.40408205773457, -10.0]);
    let p12 = s.add_point([-161.35853320783787, -2.9999999999999827]);
    let p13 = s.add_point([-163.1414287202071, -7.400000000000001]);
    let c4 = s.add_point([-170.0, -6.000000000000014]);
    let p14 = s.add_point([-176.85857127979293, -7.400000000000003]);
    let c2 = s.add_point([-189.59591794226537, -9.999999999999917]);
    let p15 = s.add_point([-178.64146679216213, -2.999999999999994]);
    s.curves.push(Curve::Arc { center: c6, start: p12, end: p13, construction: false });
    s.curves.push(Curve::Arc { center: c4, start: p14, end: p13, construction: false });
    s.curves.push(Curve::Arc { center: c2, start: p14, end: p15, construction: false });
    s.curves.push(Curve::Line { a: p15, b: p12, construction: false });
    let r = s.regions();
    assert_eq!(r.len(), 1, "{r:?}");
}

#[test]
fn a_circle_touching_a_line_makes_the_right_regions() {
    // A circle tangent to the right side of a rectangle, from inside.
    let mut s = Sketch::default();
    s.add_rectangle([0.0, 0.0], [10.0, 10.0]);
    s.add_circle([8.0, 5.0], 2.0);
    let mut areas: Vec<f64> = s.regions().iter().map(|r| r.area).collect();
    areas.sort_by(f64::total_cmp);
    let disk = std::f64::consts::PI * 4.0;
    assert_eq!(areas.len(), 2, "{areas:?}");
    assert!((areas[0] - disk).abs() < 0.05 && (areas[1] - (100.0 - disk)).abs() < 0.05, "{areas:?}");
}

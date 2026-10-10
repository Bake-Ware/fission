//! Sketch mode: the SKETCH contextual tab, 2D tools, dimensions, constraint
//! commands, and drawing sketches over the viewport.
//!
//! Sketch geometry lives in the document (a `FeatureKind::Sketch`); tools edit
//! it directly and re-solve. Bodies aren't regenerated while sketching, only
//! when the sketch is finished.

use eframe::egui::{self, Color32, Pos2, Stroke};
use fission_kernel::Plane;
use fission_model::sketch2d::{self, Constraint, Curve, Region};
use fission_model::{Document, FeatureKind, OriginPlane};
use glam::Vec3;

use super::theme;
use crate::app::FissionApp;
use crate::viewport::{Ray, StandardView};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Line,
    Rectangle,
    Circle,
    Arc,
    Dimension,
    Trim,
    Fillet,
    Offset,
    Polygon,
    Slot,
    Project,
    Extend,
    Rectangle3,
    CenterRectangle,
    Circle2,
    Circle3,
    Arc3,
    Mirror,
}

impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Line => "Line",
            Tool::Rectangle => "2-Point Rectangle",
            Tool::Circle => "Center Diameter Circle",
            Tool::Arc => "Center Point Arc",
            Tool::Dimension => "Sketch Dimension",
            Tool::Trim => "Trim",
            Tool::Fillet => "Fillet",
            Tool::Offset => "Offset",
            Tool::Polygon => "Inscribed Polygon",
            Tool::Slot => "Center to Center Slot",
            Tool::Project => "Project",
            Tool::Extend => "Extend",
            Tool::Rectangle3 => "3-Point Rectangle",
            Tool::CenterRectangle => "Center Rectangle",
            Tool::Circle2 => "2-Point Circle",
            Tool::Circle3 => "3-Point Circle",
            Tool::Arc3 => "3-Point Arc",
            Tool::Mirror => "Mirror",
        }
    }

    pub fn hotkey(self) -> Option<&'static str> {
        match self {
            Tool::Line => Some("L"),
            Tool::Rectangle => Some("R"),
            Tool::Circle => Some("C"),
            Tool::Dimension => Some("D"),
            Tool::Trim => Some("T"),
            Tool::Offset => Some("O"),
            Tool::Project => Some("P"),
            _ => None,
        }
    }
}

/// Something in the sketch under the cursor or selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Point(usize),
    Curve(usize),
}

pub struct Session {
    pub feature: usize,
    pub before: Document,
    pub tool: Tool,
    /// Points placed so far by the active tool (sketch point indices).
    pub pending: Vec<usize>,
    pub selection: Vec<Item>,
    pub hovered: Option<Item>,
    pub dragging: Option<usize>,
    /// Dimension being typed: constraint index and its text.
    pub editing_dim: Option<(usize, String)>,
    /// Remaining degrees of freedom after the last solve.
    pub dof: usize,
    pub message: Option<String>,
    /// Where the next click would land, and why (for the preview and glyph).
    pub snap: Option<Snap>,
    /// Snap to the sketch grid.
    pub grid_snap: bool,
    /// Sketch states to step back to with Ctrl+Z while sketching, and forward.
    pub undo: Vec<sketch2d::Sketch>,
    pub redo: Vec<sketch2d::Sketch>,
    /// The last edit only placed a tool's first point: the next edit (the
    /// curve it starts) shares its undo step, and Esc removes the point.
    pub lone_start: bool,
    /// The piece Trim would remove under the cursor (drawn red).
    pub trim_preview: Vec<[f64; 2]>,
    /// The curve whose chain Offset will copy once a distance is typed.
    pub offset_curve: Option<usize>,
    /// Sides for the Polygon tool.
    pub polygon_sides: usize,
    /// The model edge Project would copy, flattened onto the sketch plane.
    pub project_preview: Vec<[f64; 2]>,
    /// Mirror has its curves and is waiting for the mirror line.
    pub mirror_pick_line: bool,
}

impl Session {
    /// What the active tool wants next, for the status line.
    pub fn prompt(&self) -> Option<&'static str> {
        let n = self.pending.len();
        Some(match self.tool {
            Tool::Mirror if self.mirror_pick_line => "click the mirror line",
            Tool::Mirror => "click curves to mirror, then press Enter",
            Tool::Rectangle3 if n == 0 => "click the first corner",
            Tool::Rectangle3 if n == 1 => "click the end of the first side",
            Tool::Rectangle3 => "click to set the width",
            Tool::CenterRectangle if n == 0 => "click the center",
            Tool::CenterRectangle => "click a corner",
            Tool::Circle2 if n == 0 => "click one end of the diameter",
            Tool::Circle2 => "click the other end",
            Tool::Arc3 if n == 0 => "click the start",
            Tool::Arc3 if n == 1 => "click the end",
            Tool::Circle3 if n < 2 => "click points on the circle",
            Tool::Circle3 => "click the third point",
            Tool::Arc3 => "click a point the arc passes through",
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SnapKind {
    Point(usize),
    Origin,
    Midpoint(usize),
    OnCurve(usize),
    Horizontal,
    Vertical,
    Grid,
    Free,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snap {
    pub pos: [f64; 2],
    pub kind: SnapKind,
    /// The point being drawn from (for alignment guides).
    pub anchor: Option<[f64; 2]>,
}

/// A "nice" grid step (1, 2 or 5 x 10^n mm) about `min` or larger.
pub fn grid_step(min: f64) -> f64 {
    let min = min.max(1e-6);
    let base = 10f64.powf(min.log10().floor());
    [1.0, 2.0, 5.0, 10.0].into_iter().map(|m| m * base).find(|&s| s >= min).unwrap_or(base * 10.0)
}

/// Nearest point on a curve to `p`, with its distance.
fn nearest_on_curve(sketch: &sketch2d::Sketch, c: usize, p: [f64; 2]) -> ([f64; 2], f64) {
    match sketch.curves[c] {
        Curve::Line { a, b, .. } => {
            let (a, b) = (sketch.points[a], sketch.points[b]);
            let ab = [b[0] - a[0], b[1] - a[1]];
            let len2 = ab[0] * ab[0] + ab[1] * ab[1];
            let t = if len2 == 0.0 { 0.0 } else { (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / len2).clamp(0.0, 1.0) };
            let q = [a[0] + ab[0] * t, a[1] + ab[1] * t];
            (q, sketch2d::dist(p, q))
        }
        Curve::Circle { center, radius, .. } => {
            let c0 = sketch.points[center];
            let d = sketch2d::dist(p, c0).max(1e-12);
            let q = [c0[0] + (p[0] - c0[0]) * radius / d, c0[1] + (p[1] - c0[1]) * radius / d];
            (q, (d - radius).abs())
        }
        Curve::Arc { .. } => {
            // Sampled; arcs are short enough that this is plenty accurate for snapping.
            curve_points(sketch, c)
                .into_iter()
                .map(|q| (q, sketch2d::dist(p, q)))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap_or((p, f64::INFINITY))
        }
    }
}

/// Where a click at `cursor` should land. Priority, as in Fusion: existing
/// points, the origin, midpoints, curves, horizontal/vertical alignment with
/// the point being drawn from, then the grid.
pub fn snap(
    sketch: &sketch2d::Sketch,
    cursor: [f64; 2],
    tol: f64,
    anchor: Option<[f64; 2]>,
    grid: Option<f64>,
    skip_point: Option<usize>,
) -> Snap {
    let make = |pos, kind| Snap { pos, kind, anchor };
    let point = (0..sketch.points.len())
        .filter(|&i| Some(i) != skip_point)
        .map(|i| (i, sketch2d::dist(sketch.points[i], cursor)))
        .filter(|&(_, d)| d < tol)
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((i, _)) = point {
        return make(sketch.points[i], SnapKind::Point(i));
    }
    if sketch2d::dist(cursor, [0.0, 0.0]) < tol {
        return make([0.0, 0.0], SnapKind::Origin);
    }
    for (c, curve) in sketch.curves.iter().enumerate() {
        if let Curve::Line { a, b, .. } = *curve {
            let (pa, pb) = (sketch.points[a], sketch.points[b]);
            let mid = [(pa[0] + pb[0]) / 2.0, (pa[1] + pb[1]) / 2.0];
            if sketch2d::dist(cursor, mid) < tol {
                return make(mid, SnapKind::Midpoint(c));
            }
        }
    }
    let on_curve = (0..sketch.curves.len())
        .map(|c| (c, nearest_on_curve(sketch, c, cursor)))
        .filter(|(_, (_, d))| *d < tol)
        .min_by(|a, b| a.1 .1.total_cmp(&b.1 .1));
    if let Some((c, (q, _))) = on_curve {
        return make(q, SnapKind::OnCurve(c));
    }
    let round = |v: f64| grid.map_or(v, |g| (v / g).round() * g);
    if let Some(a) = anchor {
        if (cursor[1] - a[1]).abs() < tol && (cursor[0] - a[0]).abs() > tol {
            return make([round(cursor[0]), a[1]], SnapKind::Horizontal);
        }
        if (cursor[0] - a[0]).abs() < tol && (cursor[1] - a[1]).abs() > tol {
            return make([a[0], round(cursor[1])], SnapKind::Vertical);
        }
    }
    match grid {
        Some(_) => make([round(cursor[0]), round(cursor[1])], SnapKind::Grid),
        None => make(cursor, SnapKind::Free),
    }
}

impl Session {
    fn push_undo(&mut self, state: sketch2d::Sketch) {
        if std::mem::take(&mut self.lone_start) {
            // Same step as the start point already recorded.
            self.redo.clear();
            return;
        }
        if self.undo.last() != Some(&state) {
            self.undo.push(state);
            if self.undo.len() > 200 {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
    }
}

/// Snap distance for points and curves, in screen points.
const SNAP_PX: f32 = 9.0;

impl FissionApp {
    /// Start a new sketch: on the selected planar face, else ask for a plane.
    pub fn new_sketch(&mut self) {
        self.finish_edit(true);
        if let Some((plane, offset, _)) = self.plane_from_selection() {
            self.begin_new_sketch(plane, offset);
        } else {
            self.picking_sketch_plane = true;
            self.notify("Select a plane for the sketch: XY, XZ or YZ", false);
        }
    }

    pub fn begin_new_sketch(&mut self, plane: OriginPlane, offset: f64) {
        if std::mem::take(&mut self.picking_sketch_plane) {
            self.toast = None; // the "select a plane" hint
        }
        let before = self.doc.clone();
        let index = self.doc.push(FeatureKind::Sketch { plane, offset, sketch: sketch2d::Sketch::default() });
        self.enter_sketch(index, before);
    }

    /// Edit an existing sketch feature.
    pub fn edit_sketch(&mut self, index: usize) {
        self.finish_edit(true);
        let before = self.doc.clone();
        self.enter_sketch(index, before);
    }

    fn enter_sketch(&mut self, index: usize, before: Document) {
        let Some(FeatureKind::Sketch { plane, .. }) = self.doc.features.get(index).map(|f| &f.kind) else { return };
        match *plane {
            OriginPlane::XY => self.viewport.camera.set_view(StandardView::Top),
            OriginPlane::XZ => self.viewport.camera.set_view(StandardView::Front),
            OriginPlane::YZ => self.viewport.camera.set_view(StandardView::Right),
            OriginPlane::Custom { normal, .. } => {
                self.viewport.camera.look_along(glam::Vec3::new(normal[0] as f32, normal[1] as f32, normal[2] as f32))
            }
        }
        self.viewport.pick_enabled = false;
        self.viewport.selection.clear();
        self.selected_feature = Some(index);
        let dof = self.sketch_ref(index).map_or(0, |s| s.clone().solve().map_or(0, |r| r.dof));
        self.sketch = Some(Session {
            feature: index,
            before,
            tool: Tool::Line,
            pending: Vec::new(),
            selection: Vec::new(),
            hovered: None,
            dragging: None,
            editing_dim: None,
            dof,
            message: None,
            snap: None,
            grid_snap: true,
            undo: Vec::new(),
            redo: Vec::new(),
            lone_start: false,
            trim_preview: Vec::new(),
            offset_curve: None,
            polygon_sides: 6,
            project_preview: Vec::new(),
            mirror_pick_line: false,
        });
        self.changed();
    }

    pub fn finish_sketch(&mut self) {
        if let Some(session) = self.sketch.take() {
            if self.doc != session.before {
                self.history.record(session.before);
            }
            self.viewport.pick_enabled = true;
            self.changed();
        }
    }

    #[allow(dead_code)] // for a Cancel button / scripted use
    pub fn cancel_sketch(&mut self) {
        if let Some(session) = self.sketch.take() {
            self.doc = session.before;
            self.viewport.pick_enabled = true;
            self.changed();
        }
    }

    pub fn sketch_ref(&self, index: usize) -> Option<&sketch2d::Sketch> {
        match self.doc.features.get(index).map(|f| &f.kind) {
            Some(FeatureKind::Sketch { sketch, .. }) => Some(sketch),
            _ => None,
        }
    }

    fn sketch_mut(&mut self, index: usize) -> Option<&mut sketch2d::Sketch> {
        match self.doc.features.get_mut(index).map(|f| &mut f.kind) {
            Some(FeatureKind::Sketch { sketch, .. }) => Some(sketch),
            _ => None,
        }
    }

    pub fn sketch_plane(&self, index: usize) -> Option<Plane> {
        match self.doc.features.get(index).map(|f| &f.kind) {
            Some(FeatureKind::Sketch { plane, offset, .. }) => Some(plane.plane(*offset)),
            _ => None,
        }
    }

    pub fn set_sketch_tool(&mut self, tool: Tool) {
        self.drop_lone_start();
        if let Some(s) = &mut self.sketch {
            s.tool = tool;
            s.pending.clear();
            s.message = None;
            s.project_preview.clear();
            s.mirror_pick_line = tool == Tool::Mirror && s.selection.iter().any(|i| matches!(i, Item::Curve(_)));
        }
    }

    /// Remove a first point the active tool placed but never used (Esc or a
    /// tool change before the second click).
    pub fn drop_lone_start(&mut self) {
        let Some(session) = &mut self.sketch else { return };
        if !std::mem::take(&mut session.lone_start) {
            return;
        }
        let index = session.feature;
        session.pending.clear();
        if let Some(state) = session.undo.pop() {
            if let Some(sketch) = self.sketch_mut(index) {
                *sketch = state;
            }
            self.changed();
        }
    }

    /// Step the open sketch back (or forward) one edit. Returns false when
    /// there's nothing to step to.
    pub fn sketch_undo(&mut self, redo: bool) -> bool {
        let Some(session) = &mut self.sketch else { return false };
        let index = session.feature;
        let Some(state) = (if redo { session.redo.pop() } else { session.undo.pop() }) else { return false };
        session.pending.clear();
        session.lone_start = false;
        session.selection.clear();
        session.hovered = None;
        session.dragging = None;
        session.editing_dim = None;
        let Some(sketch) = self.sketch_mut(index) else { return false };
        let current = std::mem::replace(sketch, state);
        let dof = sketch.clone().solve().map_or(0, |r| r.dof);
        let session = self.sketch.as_mut().unwrap();
        if redo { session.undo.push(current) } else { session.redo.push(current) }
        session.dof = dof;
        self.changed();
        true
    }

    /// Re-solve after an edit. On conflict, roll the sketch back to `undo`.
    pub(crate) fn resolve_sketch(&mut self, undo: sketch2d::Sketch) -> bool {
        let Some(index) = self.sketch.as_ref().map(|s| s.feature) else { return false };
        let Some(sketch) = self.sketch_mut(index) else { return false };
        match sketch.solve() {
            Ok(report) => {
                let changed = *sketch != undo;
                if let Some(s) = &mut self.sketch {
                    s.dof = report.dof;
                    s.message = None;
                    if changed {
                        s.push_undo(undo);
                    }
                }
                true
            }
            Err(e) => {
                *sketch = undo;
                if let Some(s) = &mut self.sketch {
                    s.message = Some(format!("{e}; change undone"));
                }
                false
            }
        }
    }

    /// Apply a constraint command to the sketch selection (CONSTRAINTS panel).
    pub fn apply_constraint(&mut self, kind: ConstraintCmd) {
        let Some(session) = &self.sketch else { return };
        let (index, selection) = (session.feature, session.selection.clone());
        let Some(sketch) = self.sketch_ref(index) else { return };
        let undo = sketch.clone();
        let points: Vec<usize> = selection.iter().filter_map(|i| if let Item::Point(p) = i { Some(*p) } else { None }).collect();
        let curves: Vec<usize> = selection.iter().filter_map(|i| if let Item::Curve(c) = i { Some(*c) } else { None }).collect();
        let is_line = |c: usize| matches!(sketch.curves[c], Curve::Line { .. });
        let lines: Vec<usize> = curves.iter().copied().filter(|&c| is_line(c)).collect();
        let rounds: Vec<usize> = curves.iter().copied().filter(|&c| !is_line(c)).collect();

        let mut added: Vec<Constraint> = Vec::new();
        match kind {
            ConstraintCmd::HorizontalVertical => {
                for &l in &lines {
                    let (a, b) = sketch.line_points(l);
                    let d = [sketch.points[b][0] - sketch.points[a][0], sketch.points[b][1] - sketch.points[a][1]];
                    added.push(if d[0].abs() >= d[1].abs() { Constraint::Horizontal(l) } else { Constraint::Vertical(l) });
                }
            }
            ConstraintCmd::Coincident => match (points.as_slice(), curves.as_slice()) {
                ([a, b], []) => added.push(Constraint::Coincident(*a, *b)),
                ([p], [c]) if is_line(*c) => added.push(Constraint::PointOnLine(*p, *c)),
                ([p], [c]) => added.push(Constraint::PointOnCircle(*p, *c)),
                _ => {}
            },
            ConstraintCmd::Tangent => {
                if let ([l], [c]) = (lines.as_slice(), rounds.as_slice()) {
                    added.push(Constraint::Tangent(*l, *c));
                }
            }
            ConstraintCmd::Equal => {
                if let [a, b] = curves.as_slice() {
                    if is_line(*a) == is_line(*b) {
                        added.push(Constraint::Equal(*a, *b));
                    }
                }
            }
            ConstraintCmd::Parallel | ConstraintCmd::Perpendicular => {
                if let [a, b] = lines.as_slice() {
                    added.push(if kind == ConstraintCmd::Parallel {
                        Constraint::Parallel(*a, *b)
                    } else {
                        Constraint::Perpendicular(*a, *b)
                    });
                }
            }
            ConstraintCmd::Fix => {
                let mut pts = points.clone();
                for &c in &curves {
                    pts.extend(sketch.curves[c].points());
                }
                added.extend(pts.into_iter().map(Constraint::Fixed));
            }
            ConstraintCmd::Midpoint => {
                if let ([p], [l]) = (points.as_slice(), lines.as_slice()) {
                    added.push(Constraint::Midpoint(*p, *l));
                }
            }
            ConstraintCmd::Concentric => {
                if let [a, b] = rounds.as_slice() {
                    added.push(Constraint::Concentric(*a, *b));
                }
            }
        }
        if added.is_empty() {
            if let Some(s) = &mut self.sketch {
                s.message = Some(format!("{}: {}", kind.label(), kind.needs()));
            }
            return;
        }
        if let Some(sketch) = self.sketch_mut(index) {
            sketch.constraints.extend(added);
        }
        self.resolve_sketch(undo);
    }

    pub fn toggle_construction(&mut self) {
        let Some(session) = &self.sketch else { return };
        let (index, selection) = (session.feature, session.selection.clone());
        let Some(sketch) = self.sketch_mut(index) else { return };
        let undo = sketch.clone();
        for item in selection {
            if let Item::Curve(c) = item {
                let now = sketch.curves[c].construction();
                sketch.curves[c].set_construction(!now);
            }
        }
        self.resolve_sketch(undo);
    }

    pub fn delete_sketch_selection(&mut self) {
        let Some(session) = &mut self.sketch else { return };
        let index = session.feature;
        let mut curves: Vec<usize> =
            session.selection.iter().filter_map(|i| if let Item::Curve(c) = i { Some(*c) } else { None }).collect();
        session.selection.clear();
        session.hovered = None;
        curves.sort_unstable();
        curves.dedup();
        if let Some(sketch) = self.sketch_mut(index) {
            for c in curves.into_iter().rev() {
                sketch.delete_curve(c);
            }
        }
        let undo = self.sketch_ref(index).cloned().unwrap_or_default();
        self.resolve_sketch(undo);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConstraintCmd {
    HorizontalVertical,
    Coincident,
    Tangent,
    Equal,
    Parallel,
    Perpendicular,
    Fix,
    Midpoint,
    Concentric,
}

impl ConstraintCmd {
    pub const ALL: [ConstraintCmd; 9] = [
        ConstraintCmd::HorizontalVertical,
        ConstraintCmd::Coincident,
        ConstraintCmd::Tangent,
        ConstraintCmd::Equal,
        ConstraintCmd::Parallel,
        ConstraintCmd::Perpendicular,
        ConstraintCmd::Fix,
        ConstraintCmd::Midpoint,
        ConstraintCmd::Concentric,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ConstraintCmd::HorizontalVertical => "Horizontal/Vertical",
            ConstraintCmd::Coincident => "Coincident",
            ConstraintCmd::Tangent => "Tangent",
            ConstraintCmd::Equal => "Equal",
            ConstraintCmd::Parallel => "Parallel",
            ConstraintCmd::Perpendicular => "Perpendicular",
            ConstraintCmd::Fix => "Fix",
            ConstraintCmd::Midpoint => "Midpoint",
            ConstraintCmd::Concentric => "Concentric",
        }
    }

    /// Toolbar icon (drawn, since the default font lacks most constraint glyphs).
    pub fn icon(self) -> super::icons::Icon {
        use super::icons::Icon;
        match self {
            ConstraintCmd::HorizontalVertical => Icon::ConHV,
            ConstraintCmd::Coincident => Icon::ConCoincident,
            ConstraintCmd::Tangent => Icon::ConTangent,
            ConstraintCmd::Equal => Icon::ConEqual,
            ConstraintCmd::Parallel => Icon::ConParallel,
            ConstraintCmd::Perpendicular => Icon::ConPerpendicular,
            ConstraintCmd::Fix => Icon::ConFix,
            ConstraintCmd::Midpoint => Icon::ConMidpoint,
            ConstraintCmd::Concentric => Icon::ConConcentric,
        }
    }

    fn needs(self) -> &'static str {
        match self {
            ConstraintCmd::HorizontalVertical => "select one or more lines",
            ConstraintCmd::Coincident => "select two points, or a point and a curve",
            ConstraintCmd::Tangent => "select a line and a circle or arc",
            ConstraintCmd::Equal => "select two lines, or two circles/arcs",
            ConstraintCmd::Parallel | ConstraintCmd::Perpendicular => "select two lines",
            ConstraintCmd::Fix => "select points or curves",
            ConstraintCmd::Midpoint => "select a point and a line",
            ConstraintCmd::Concentric => "select two circles or arcs",
        }
    }
}

/// Where the pointer ray meets the plane, in plane coordinates.
pub fn ray_on_plane(ray: &Ray, plane: &Plane) -> Option<[f64; 2]> {
    let n = Vec3::new(plane.normal[0] as f32, plane.normal[1] as f32, plane.normal[2] as f32);
    let o = Vec3::new(plane.origin[0] as f32, plane.origin[1] as f32, plane.origin[2] as f32);
    let denom = ray.dir.dot(n);
    if denom.abs() < 1e-6 {
        return None;
    }
    let t = (o - ray.origin).dot(n) / denom;
    if t < 0.0 {
        return None;
    }
    let hit = ray.origin + ray.dir * t - o;
    let x = Vec3::new(plane.x_dir[0] as f32, plane.x_dir[1] as f32, plane.x_dir[2] as f32);
    let y = n.cross(x);
    Some([hit.dot(x) as f64, hit.dot(y) as f64])
}

fn world(plane: &Plane, p: [f64; 2]) -> Vec3 {
    let w = plane.to_world(p);
    Vec3::new(w[0] as f32, w[1] as f32, w[2] as f32)
}

/// Sample a curve as plane points, for drawing and hit tests.
pub(crate) fn curve_points(sketch: &sketch2d::Sketch, c: usize) -> Vec<[f64; 2]> {
    match sketch.curves[c] {
        Curve::Line { a, b, .. } => vec![sketch.points[a], sketch.points[b]],
        Curve::Circle { center, radius, .. } => {
            let c = sketch.points[center];
            (0..=64).map(|i| {
                let t = std::f64::consts::TAU * i as f64 / 64.0;
                [c[0] + radius * t.cos(), c[1] + radius * t.sin()]
            })
            .collect()
        }
        Curve::Arc { center, start, end, .. } => {
            let (c, s, e) = (sketch.points[center], sketch.points[start], sketch.points[end]);
            let r = sketch2d::dist(c, s);
            let t0 = (s[1] - c[1]).atan2(s[0] - c[0]);
            let mut t1 = (e[1] - c[1]).atan2(e[0] - c[0]);
            if t1 <= t0 {
                t1 += std::f64::consts::TAU;
            }
            (0..=48).map(|i| {
                let t = t0 + (t1 - t0) * i as f64 / 48.0;
                [c[0] + r * t.cos(), c[1] + r * t.sin()]
            })
            .collect()
        }
    }
}

fn seg_dist(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let len2 = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if len2 == 0.0 { 0.0 } else { (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / len2).clamp(0.0, 1.0) };
    sketch2d::dist(p, [a[0] + ab[0] * t, a[1] + ab[1] * t])
}

/// Nearest point within `tol` (plane units), else nearest curve within `tol`.
fn hit_item(sketch: &sketch2d::Sketch, p: [f64; 2], tol: f64, skip_point: Option<usize>) -> Option<Item> {
    let point = (0..sketch.points.len())
        .filter(|&i| Some(i) != skip_point)
        .map(|i| (i, sketch2d::dist(sketch.points[i], p)))
        .filter(|&(_, d)| d < tol)
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((i, _)) = point {
        return Some(Item::Point(i));
    }
    hit_curve(sketch, p, tol).map(Item::Curve)
}

/// The curve within `tol` of `p`, ignoring points (for Trim).
fn hit_curve(sketch: &sketch2d::Sketch, p: [f64; 2], tol: f64) -> Option<usize> {
    (0..sketch.curves.len())
        .map(|c| {
            let pts = curve_points(sketch, c);
            let d = pts.windows(2).map(|w| seg_dist(p, w[0], w[1])).fold(f64::INFINITY, f64::min);
            (c, d)
        })
        .filter(|&(_, d)| d < tol)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(c, _)| c)
}

/// Per-frame sketch input: tools, selection, dragging. Called with the
/// viewport's events while a sketch session is active.
pub fn handle_input(app: &mut FissionApp, events: &crate::viewport::ViewportEvents) {
    let Some(session) = &app.sketch else { return };
    let (Some(plane), Some(ray)) = (app.sketch_plane(session.feature), app.viewport.pointer_ray()) else { return };
    let Some(cursor) = ray_on_plane(&ray, &plane) else { return };
    let wpp = app.viewport.world_per_point() as f64;
    let tol = SNAP_PX as f64 * wpp;
    let grid = app.sketch.as_ref().is_some_and(|s| s.grid_snap).then(|| grid_step(wpp * 12.0));
    if session.tool == Tool::Project {
        project_input(app, events, &plane);
        return;
    }
    handle_at(app, events, cursor, tol, grid);
}

/// Project: hover a model edge to see it flattened onto the sketch, click
/// to copy it in as fixed geometry.
fn project_input(app: &mut FissionApp, events: &crate::viewport::ViewportEvents, plane: &Plane) {
    let flat: Vec<[f64; 2]> = app
        .viewport
        .edge_under_pointer(&app.bodies)
        .and_then(|e| app.bodies[e.body].mesh.edges.get(e.edge as usize))
        .map(|polyline| polyline.iter().map(|&p| to_plane(plane, p)).collect())
        .unwrap_or_default();
    if events.primary_clicked && !flat.is_empty() {
        let Some(index) = app.sketch.as_ref().map(|s| s.feature) else { return };
        let Some(sketch) = app.sketch_mut(index) else { return };
        let undo = sketch.clone();
        if sketch.project_polyline(&flat).is_empty() {
            if let Some(s) = &mut app.sketch {
                s.message = Some("That edge is end-on to the sketch plane".into());
            }
        } else {
            app.resolve_sketch(undo);
        }
    }
    if let Some(s) = &mut app.sketch {
        s.project_preview = flat;
    }
}

/// A model point dropped onto the plane, in plane coordinates.
fn to_plane(plane: &Plane, p: [f32; 3]) -> [f64; 2] {
    let d: [f64; 3] = std::array::from_fn(|k| p[k] as f64 - plane.origin[k]);
    let dot = |v: [f64; 3]| d[0] * v[0] + d[1] * v[1] + d[2] * v[2];
    [dot(plane.x_dir), dot(plane.y_dir())]
}

/// The tool logic for a pointer at `cursor` (plane coordinates), with `tol`
/// as the snap distance. Separate from the ray math so scripted clicks
/// (FISSION_DEMO) exercise the same code as real ones.
pub fn handle_at(
    app: &mut FissionApp,
    events: &crate::viewport::ViewportEvents,
    cursor: [f64; 2],
    tol: f64,
    grid: Option<f64>,
) {
    let Some(session) = &app.sketch else { return };
    let index = session.feature;
    let Some(sketch) = app.sketch_ref(index) else { return };
    let undo = sketch.clone();
    let dragging = session.dragging;
    let tool = session.tool;
    let mirror_pick_line = session.mirror_pick_line;
    let hovered = if matches!(tool, Tool::Trim | Tool::Offset | Tool::Extend | Tool::Mirror) {
        hit_curve(sketch, cursor, tol).map(Item::Curve)
    } else {
        hit_item(sketch, cursor, tol, dragging)
    };
    let trim_preview = match hovered {
        Some(Item::Curve(c)) if tool == Tool::Trim => sketch.trim_preview(c, cursor),
        Some(Item::Curve(c)) if tool == Tool::Extend => sketch.extend_preview(c, cursor),
        _ => Vec::new(),
    };
    let anchor = session.pending.last().map(|&p| sketch.points[p]);
    let drawing = !matches!(tool, Tool::Select | Tool::Dimension | Tool::Trim | Tool::Fillet | Tool::Offset | Tool::Extend | Tool::Mirror);
    let snapped = drawing.then(|| snap(sketch, cursor, tol, anchor, grid, None));
    let snap_point = match snapped.map(|s| s.kind) {
        Some(SnapKind::Point(i)) => Some(i),
        _ => None,
    };
    let at = snapped.map_or(cursor, |s| s.pos);
    if let Some(s) = &mut app.sketch {
        s.hovered = hovered;
        s.snap = snapped;
        s.trim_preview = trim_preview;
    }

    // Dragging a point in Select re-solves with the point pulled to the cursor.
    if tool == Tool::Select {
        if events.primary_drag_started {
            if let Some(Item::Point(p)) = hovered {
                if let Some(s) = &mut app.sketch {
                    s.push_undo(undo.clone());
                    s.dragging = Some(p);
                }
            }
        }
        if let Some(p) = dragging {
            if events.primary_dragging {
                if let Some(sketch) = app.sketch_mut(index) {
                    let undo = sketch.clone();
                    if sketch.solve_drag(p, cursor).is_err() {
                        *sketch = undo;
                    }
                }
            }
            if events.primary_drag_stopped {
                if let Some(s) = &mut app.sketch {
                    s.dragging = None;
                }
            }
        }
    }

    if !events.primary_clicked {
        return;
    }
    // Use the snapped-to point, or make one there with the constraint the
    // snap implies.
    let place = |app: &mut FissionApp| -> usize {
        if let Some(i) = snap_point {
            return i;
        }
        let Some(sketch) = app.sketch_mut(index) else { return 0 };
        let p = sketch.add_point(at);
        match snapped.map(|s| s.kind) {
            Some(SnapKind::Origin) => sketch.constraints.push(Constraint::Fixed(p)),
            Some(SnapKind::Midpoint(l)) => sketch.constraints.push(Constraint::Midpoint(p, l)),
            Some(SnapKind::OnCurve(c)) => {
                let k = if matches!(sketch.curves[c], Curve::Line { .. }) {
                    Constraint::PointOnLine(p, c)
                } else {
                    Constraint::PointOnCircle(p, c)
                };
                sketch.constraints.push(k);
            }
            _ => {}
        }
        p
    };

    match tool {
        // Handled before the plane math (see `project_input`).
        Tool::Project => {}
        Tool::Select => {
            let toggle = events.toggle;
            if let Some(s) = &mut app.sketch {
                match hovered {
                    Some(item) if toggle => match s.selection.iter().position(|i| *i == item) {
                        Some(k) => {
                            s.selection.remove(k);
                        }
                        None => s.selection.push(item),
                    },
                    Some(item) => s.selection = vec![item],
                    None => s.selection.clear(),
                }
            }
        }
        Tool::Line => {
            let p = place(app);
            let last = app.sketch.as_ref().and_then(|s| s.pending.last().copied());
            match last {
                // Double-click (or clicking the same point) ends the chain.
                Some(prev) if prev == p || events.primary_double_clicked => {
                    if let Some(s) = &mut app.sketch {
                        s.pending.clear();
                    }
                }
                Some(prev) => {
                    if let Some(sketch) = app.sketch_mut(index) {
                        sketch.curves.push(Curve::Line { a: prev, b: p, construction: false });
                        let l = sketch.curves.len() - 1;
                        // Auto horizontal/vertical within 2 degrees, like Fusion's inference.
                        let (a, b) = (sketch.points[prev], sketch.points[p]);
                        let angle = (b[1] - a[1]).atan2(b[0] - a[0]).to_degrees().rem_euclid(180.0);
                        if !(2.0..=178.0).contains(&angle) {
                            sketch.constraints.push(Constraint::Horizontal(l));
                        } else if (angle - 90.0).abs() < 2.0 {
                            sketch.constraints.push(Constraint::Vertical(l));
                        }
                    }
                    // Closing a loop on an existing point ends the chain.
                    let closed = snap_point.is_some();
                    if let Some(s) = &mut app.sketch {
                        s.pending = if closed { Vec::new() } else { vec![p] };
                    }
                    app.resolve_sketch(undo);
                }
                None => start_point(app, undo, p),
            }
        }
        Tool::Rectangle => {
            let first = app.sketch.as_ref().and_then(|s| s.pending.first().copied());
            match first {
                None => {
                    let p = place(app);
                    start_point(app, undo, p);
                }
                Some(a) => {
                    if let Some(sketch) = app.sketch_mut(index) {
                        let corner = sketch.points[a];
                        if sketch2d::dist(corner, at) > 1e-6 {
                            let [bottom, ..] = sketch.add_rectangle(corner, at);
                            // Tie the first corner to the clicked point.
                            let start = sketch.line_points(bottom).0;
                            sketch.constraints.push(Constraint::Coincident(start, a));
                        }
                    }
                    if let Some(s) = &mut app.sketch {
                        s.pending.clear();
                    }
                    app.resolve_sketch(undo);
                }
            }
        }
        Tool::Slot => {
            let pending = app.sketch.as_ref().map(|s| s.pending.clone()).unwrap_or_default();
            if pending.len() < 2 {
                let p = place(app);
                start_point(app, undo, p);
            } else {
                // Third click: the width is twice its distance from the center line.
                if let Some(sketch) = app.sketch_mut(index) {
                    let width = 2.0 * slot_half_width(sketch.points[pending[0]], sketch.points[pending[1]], at);
                    if width > 1e-6 {
                        sketch.add_slot(pending[0], pending[1], width);
                    }
                }
                if let Some(s) = &mut app.sketch {
                    s.pending.clear();
                }
                app.resolve_sketch(undo);
            }
        }
        Tool::Polygon => {
            let first = app.sketch.as_ref().and_then(|s| s.pending.first().copied());
            match first {
                None => {
                    let p = place(app);
                    start_point(app, undo, p);
                }
                Some(center) => {
                    let sides = app.sketch.as_ref().map_or(6, |s| s.polygon_sides);
                    if let Some(sketch) = app.sketch_mut(index) {
                        if sketch2d::dist(sketch.points[center], at) > 1e-6 {
                            sketch.add_polygon(center, at, sides);
                        }
                    }
                    if let Some(s) = &mut app.sketch {
                        s.pending.clear();
                    }
                    app.resolve_sketch(undo);
                }
            }
        }
        Tool::Circle => {
            let first = app.sketch.as_ref().and_then(|s| s.pending.first().copied());
            match first {
                None => {
                    let p = place(app);
                    start_point(app, undo, p);
                }
                Some(center) => {
                    if let Some(sketch) = app.sketch_mut(index) {
                        let r = sketch2d::dist(sketch.points[center], at);
                        if r > 1e-6 {
                            sketch.curves.push(Curve::Circle { center, radius: r, construction: false });
                        }
                    }
                    if let Some(s) = &mut app.sketch {
                        s.pending.clear();
                    }
                    app.resolve_sketch(undo);
                }
            }
        }
        Tool::Arc => {
            let pending = app.sketch.as_ref().map(|s| s.pending.clone()).unwrap_or_default();
            if pending.len() < 2 {
                let p = place(app);
                start_point(app, undo, p);
            } else {
                let (center, start) = (pending[0], pending[1]);
                if let Some(sketch) = app.sketch_mut(index) {
                    // End point goes on the arc's circle, in the cursor's direction.
                    let (c, s0) = (sketch.points[center], sketch.points[start]);
                    let r = sketch2d::dist(c, s0);
                    let d = sketch2d::dist(c, at).max(1e-9);
                    let e = [c[0] + (at[0] - c[0]) * r / d, c[1] + (at[1] - c[1]) * r / d];
                    let end = match snap_point {
                        Some(i) => i,
                        None => sketch.add_point(e),
                    };
                    sketch.curves.push(Curve::Arc { center, start, end, construction: false });
                }
                if let Some(s) = &mut app.sketch {
                    s.pending.clear();
                }
                app.resolve_sketch(undo);
            }
        }
        Tool::Rectangle3 | Tool::Circle3 | Tool::Arc3 => {
            let pending = app.sketch.as_ref().map(|s| s.pending.clone()).unwrap_or_default();
            if pending.len() < 2 {
                let p = place(app);
                start_point(app, undo, p);
            } else {
                // Third click: the far side, or a third point on the curve.
                let third = (tool == Tool::Circle3).then(|| place(app));
                if let Some(sketch) = app.sketch_mut(index) {
                    let (a, b) = (pending[0], pending[1]);
                    match tool {
                        Tool::Rectangle3 => {
                            if slot_half_width(sketch.points[a], sketch.points[b], at) > 1e-6 {
                                sketch.add_three_point_rectangle(a, b, at);
                            }
                        }
                        Tool::Arc3 => {
                            sketch.add_arc_through(a, b, at);
                        }
                        _ => {
                            let on = [a, b, third.unwrap_or(b)];
                            if let Some(c) = sketch.add_circle_through(on.map(|p| sketch.points[p])) {
                                circle_on_points(sketch, c, &on);
                            }
                        }
                    }
                }
                if let Some(s) = &mut app.sketch {
                    s.pending.clear();
                }
                app.resolve_sketch(undo);
            }
        }
        Tool::CenterRectangle | Tool::Circle2 => {
            let first = app.sketch.as_ref().and_then(|s| s.pending.first().copied());
            match first {
                None => {
                    let p = place(app);
                    start_point(app, undo, p);
                }
                Some(a) => {
                    let second = (tool == Tool::Circle2).then(|| place(app));
                    if let Some(sketch) = app.sketch_mut(index) {
                        let pa = sketch.points[a];
                        if sketch2d::dist(pa, at) > 1e-6 {
                            if let Some(b) = second {
                                // The two clicks are the ends of a diameter.
                                let pb = sketch.points[b];
                                let center = sketch.add_point([(pa[0] + pb[0]) / 2.0, (pa[1] + pb[1]) / 2.0]);
                                sketch.curves.push(Curve::Circle { center, radius: sketch2d::dist(pa, pb) / 2.0, construction: false });
                                let c = sketch.curves.len() - 1;
                                circle_on_points(sketch, c, &[a, b]);
                            } else {
                                sketch.add_center_rectangle(a, at);
                            }
                        }
                    }
                    if let Some(s) = &mut app.sketch {
                        s.pending.clear();
                    }
                    app.resolve_sketch(undo);
                }
            }
        }
        Tool::Mirror => {
            let Some(Item::Curve(c)) = hovered else { return };
            if !mirror_pick_line {
                if let Some(s) = &mut app.sketch {
                    match s.selection.iter().position(|i| *i == Item::Curve(c)) {
                        Some(k) => {
                            s.selection.remove(k);
                        }
                        None => s.selection.push(Item::Curve(c)),
                    }
                }
                return;
            }
            let is_line = app.sketch_ref(index).is_some_and(|sk| matches!(sk.curves[c], Curve::Line { .. }));
            if !is_line {
                if let Some(s) = &mut app.sketch {
                    s.message = Some("the mirror line must be a line".into());
                }
                return;
            }
            let objects: Vec<usize> = app.sketch.as_ref().map_or(Vec::new(), |s| {
                s.selection.iter().filter_map(|i| if let Item::Curve(k) = i { Some(*k) } else { None }).collect()
            });
            if let Some(sketch) = app.sketch_mut(index) {
                sketch.mirror(&objects, c);
            }
            if let Some(s) = &mut app.sketch {
                s.selection.clear();
                s.mirror_pick_line = false;
                s.tool = Tool::Select;
            }
            app.resolve_sketch(undo);
        }
        Tool::Trim => {
            if let Some(Item::Curve(c)) = hovered {
                if let Some(sketch) = app.sketch_mut(index) {
                    sketch.trim(c, cursor);
                }
                if let Some(s) = &mut app.sketch {
                    s.hovered = None;
                    s.selection.clear();
                    s.trim_preview.clear();
                }
                app.resolve_sketch(undo);
            }
        }
        Tool::Extend => {
            let Some(Item::Curve(c)) = hovered else { return };
            let extended = app.sketch_mut(index).is_some_and(|sketch| sketch.extend(c, cursor));
            if let Some(s) = &mut app.sketch {
                s.trim_preview.clear();
                if !extended {
                    s.message = Some("Nothing in the way to extend to (or that end is connected)".into());
                }
            }
            if extended {
                app.resolve_sketch(undo);
            }
        }
        Tool::Offset => {
            let Some(Item::Curve(c)) = hovered else { return };
            let chain = app.sketch_ref(index).map(|sk| sk.chain(c).0).unwrap_or_default();
            if let Some(s) = &mut app.sketch {
                s.offset_curve = Some(c);
                s.selection = chain.into_iter().map(|(c, _)| Item::Curve(c)).collect();
            }
            // Ask for the distance right away.
            app.hud = Some(super::hud::Hud::new(String::new()));
        }
        Tool::Fillet => {
            let Some(Item::Point(p)) = hovered else { return };
            let Some(sketch) = app.sketch_mut(index) else { return };
            let Some(r) = sketch.default_fillet_radius(p) else {
                if let Some(s) = &mut app.sketch {
                    s.message = Some("Click a corner where two lines meet".into());
                }
                return;
            };
            match sketch.fillet_corner(p, grid_step(r * 0.5)) {
                Ok((_, dim)) => {
                    if app.resolve_sketch(undo) {
                        // Straight into the radius, like Fusion's fillet box.
                        let value = display_value(app_sketch(&app.doc, index), dim);
                        if let Some(s) = &mut app.sketch {
                            s.hovered = None;
                            s.editing_dim = Some((dim, format!("{value:.2}")));
                        }
                    }
                }
                Err(e) => {
                    if let Some(s) = &mut app.sketch {
                        s.message = Some(e);
                    }
                }
            }
        }
        Tool::Dimension => {
            let pending = app.sketch.as_ref().map(|s| s.pending.clone()).unwrap_or_default();
            let new = match (hovered, pending.as_slice()) {
                (Some(Item::Curve(c)), []) => {
                    let sketch = app.sketch_ref(index).unwrap();
                    Some(match sketch.curves[c] {
                        Curve::Line { .. } => Constraint::Length(c, sketch.line_length(c)),
                        _ => Constraint::Radius(c, sketch.radius(c)),
                    })
                }
                (Some(Item::Point(p)), []) => {
                    if let Some(s) = &mut app.sketch {
                        s.pending = vec![p];
                    }
                    None
                }
                (Some(Item::Point(q)), [p]) if *p != q => {
                    let sketch = app.sketch_ref(index).unwrap();
                    Some(Constraint::Distance(*p, q, sketch2d::dist(sketch.points[*p], sketch.points[q])))
                }
                _ => {
                    if let Some(s) = &mut app.sketch {
                        s.pending.clear();
                    }
                    None
                }
            };
            if let Some(k) = new {
                let at = app.sketch_mut(index).map(|sketch| {
                    sketch.constraints.push(k);
                    sketch.constraints.len() - 1
                });
                // Open the value editor on the new dimension, like Fusion.
                if app.resolve_sketch(undo) {
                    if let Some(at) = at {
                        let value = display_value(app_sketch(&app.doc, index), at);
                        if let Some(s) = &mut app.sketch {
                            s.pending.clear();
                            s.editing_dim = Some((at, format!("{value:.2}")));
                        }
                    }
                }
            }
        }
    }
}

/// Distance from `p` to the line through `a` and `b`.
fn slot_half_width(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> f64 {
    let d = sketch2d::dist(a, b).max(1e-9);
    (((b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])) / d).abs()
}

/// Keep a circle drawn through clicked points on them: points other curves
/// use stay on the circle; points made just for the clicks go away.
fn circle_on_points(sketch: &mut sketch2d::Sketch, circle: usize, points: &[usize]) {
    let mut points = points.to_vec();
    points.dedup();
    for p in points {
        sketch.constraints.push(Constraint::PointOnCircle(p, circle));
    }
    sketch.remove_unused_points();
}

fn session_pending_len(app: &FissionApp) -> usize {
    app.sketch.as_ref().map_or(0, |s| s.pending.len())
}

/// A tool's first click(s): add the point to the pending list and record an
/// undo step, which the curve it starts will share.
fn start_point(app: &mut FissionApp, undo: sketch2d::Sketch, p: usize) {
    let index = app.sketch.as_ref().map_or(0, |s| s.feature);
    let changed = app.sketch_ref(index).is_some_and(|sketch| *sketch != undo);
    let Some(s) = &mut app.sketch else { return };
    s.pending.push(p);
    if changed {
        s.push_undo(undo);
        s.lone_start = true;
    }
}

fn app_sketch(doc: &Document, index: usize) -> &sketch2d::Sketch {
    match &doc.features[index].kind {
        FeatureKind::Sketch { sketch, .. } => sketch,
        _ => unreachable!(),
    }
}

/// The number a dimension shows: circles as diameters (like Fusion), the rest as is.
fn display_value(sketch: &sketch2d::Sketch, constraint: usize) -> f64 {
    match sketch.constraints[constraint] {
        Constraint::Radius(c, r) if matches!(sketch.curves[c], Curve::Circle { .. }) => r * 2.0,
        Constraint::Distance(_, _, v)
        | Constraint::HorizontalDistance(_, _, v)
        | Constraint::VerticalDistance(_, _, v)
        | Constraint::Length(_, v)
        | Constraint::Radius(_, v) => v,
        _ => 0.0,
    }
}

/// Set a dimension from its displayed value and re-solve.
pub fn set_dimension(app: &mut FissionApp, index: usize, constraint: usize, shown: f64) {
    let Some(sketch) = app.sketch_ref(index) else { return };
    let undo = sketch.clone();
    let is_circle = matches!(sketch.constraints[constraint], Constraint::Radius(c, _) if matches!(sketch.curves[c], Curve::Circle { .. }));
    let value = if is_circle { shown / 2.0 } else { shown };
    if let Some(sketch) = app.sketch_mut(index) {
        if let Some(v) = sketch.constraints[constraint].value_mut() {
            *v = value;
        }
    }
    app.resolve_sketch(undo);
}

/// Where a dimension's label sits, in plane coordinates.
fn dimension_anchor(sketch: &sketch2d::Sketch, k: &Constraint) -> Option<[f64; 2]> {
    let mid = |a: [f64; 2], b: [f64; 2]| [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    match *k {
        Constraint::Length(l, _) => {
            let (a, b) = sketch.line_points(l);
            Some(mid(sketch.points[a], sketch.points[b]))
        }
        Constraint::Distance(a, b, _) | Constraint::HorizontalDistance(a, b, _) | Constraint::VerticalDistance(a, b, _) => {
            Some(mid(sketch.points[a], sketch.points[b]))
        }
        Constraint::Radius(c, _) => {
            let center = match sketch.curves[c] {
                Curve::Circle { center, .. } | Curve::Arc { center, .. } => sketch.points[center],
                Curve::Line { a, .. } => sketch.points[a],
            };
            let r = sketch.radius(c) * std::f64::consts::FRAC_1_SQRT_2;
            Some([center[0] + r, center[1] + r])
        }
        _ => None,
    }
}

/// Draw a sketch over the viewport. `active` = being edited (tools, labels, colors).
pub fn draw(app: &mut FissionApp, ui: &mut egui::Ui, index: usize, active: bool, highlight_regions: &[[f64; 2]]) {
    let (Some(plane), Some(sketch)) = (app.sketch_plane(index), app.sketch_ref(index).cloned()) else { return };
    let viewport = &app.viewport;
    let painter = ui.painter_at(viewport.rect);
    let to_screen = |p: [f64; 2]| viewport.to_screen(world(&plane, p));
    let session = app.sketch.as_ref().filter(|s| active && s.feature == index);

    // Profiles: filled when highlighted (picked for an extrude) or while sketching.
    let regions = sketch.regions();
    for region in &regions {
        let picked = highlight_regions.iter().any(|&p| region.contains(p));
        if active || picked {
            let fill = if picked {
                Color32::from_rgba_unmultiplied(6, 150, 215, 90)
            } else {
                Color32::from_rgba_unmultiplied(255, 190, 90, 45)
            };
            fill_region(&painter, region, &to_screen, fill);
        }
    }

    let fully = session.is_some_and(|s| s.dof == 0);
    for (c, curve) in sketch.curves.iter().enumerate() {
        let pts: Vec<Pos2> = curve_points(&sketch, c).into_iter().filter_map(to_screen).collect();
        let selected = session.is_some_and(|s| s.selection.contains(&Item::Curve(c)));
        let hovered = session.is_some_and(|s| s.hovered == Some(Item::Curve(c)));
        let color = if selected {
            theme::accent()
        } else if hovered {
            Color32::from_rgb(120, 190, 245)
        } else if curve.construction() {
            Color32::from_rgb(230, 140, 40)
        } else if !active {
            theme::palette().ink_inactive
        } else if fully {
            theme::palette().ink
        } else {
            Color32::from_rgb(40, 110, 200)
        };
        let width = if selected || hovered { 2.5 } else { 1.5 };
        if curve.construction() {
            painter.add(egui::Shape::dashed_line(&pts, Stroke::new(width, color), 6.0, 4.0));
        } else {
            painter.line(pts, Stroke::new(width, color));
        }
    }

    if let Some(session) = session.filter(|_| active) {
        // Trim's piece to remove in red; Extend's piece to add in blue.
        let pts: Vec<Pos2> = session.trim_preview.iter().copied().filter_map(to_screen).collect();
        if pts.len() > 1 {
            if session.tool == Tool::Extend {
                painter.add(egui::Shape::dashed_line(&pts, Stroke::new(3.0, theme::accent()), 6.0, 3.0));
            } else {
                painter.line(pts, Stroke::new(3.0, Color32::from_rgb(220, 40, 40)));
            }
        }
        // Projected geometry previews in purple, as Fusion draws it.
        let pts: Vec<Pos2> = session.project_preview.iter().copied().filter_map(to_screen).collect();
        if pts.len() > 1 {
            painter.line(pts, Stroke::new(3.0, Color32::from_rgb(150, 70, 200)));
        }
    }

    if !active {
        return;
    }
    let session = session.unwrap();
    for (i, &p) in sketch.points.iter().enumerate() {
        let Some(s) = to_screen(p) else { continue };
        let selected = session.selection.contains(&Item::Point(i));
        let hovered = session.hovered == Some(Item::Point(i));
        let size = if hovered || selected { 4.0 } else { 2.5 };
        let color = if selected { theme::accent() } else if hovered { Color32::from_rgb(120, 190, 245) } else { theme::palette().ink };
        painter.rect_filled(egui::Rect::from_center_size(s, egui::vec2(size * 2.0, size * 2.0)), 1.0, color);
    }

    // Snap glyph and alignment guide where the next click would land.
    if let Some(snap) = session.snap {
        if let Some(s) = to_screen(snap.pos) {
            let glyph = Stroke::new(1.5, Color32::from_rgb(230, 120, 20));
            match snap.kind {
                SnapKind::Point(_) | SnapKind::Origin => {
                    painter.rect_stroke(egui::Rect::from_center_size(s, egui::vec2(11.0, 11.0)), 0.0, glyph, egui::StrokeKind::Middle);
                }
                SnapKind::Midpoint(_) => {
                    painter.add(egui::Shape::convex_polygon(
                        vec![s + egui::vec2(0.0, -7.0), s + egui::vec2(6.0, 4.0), s + egui::vec2(-6.0, 4.0)],
                        Color32::TRANSPARENT,
                        glyph,
                    ));
                }
                SnapKind::OnCurve(_) => {
                    painter.line_segment([s + egui::vec2(-5.0, -5.0), s + egui::vec2(5.0, 5.0)], glyph);
                    painter.line_segment([s + egui::vec2(-5.0, 5.0), s + egui::vec2(5.0, -5.0)], glyph);
                }
                SnapKind::Horizontal | SnapKind::Vertical => {
                    if let Some(a) = snap.anchor.and_then(to_screen) {
                        painter.add(egui::Shape::dashed_line(&[a, s], Stroke::new(1.0, Color32::from_rgb(230, 120, 20)), 4.0, 3.0));
                    }
                    let label = if snap.kind == SnapKind::Horizontal { "—" } else { "|" };
                    painter.text(s + egui::vec2(10.0, -10.0), egui::Align2::LEFT_BOTTOM, label, egui::FontId::proportional(12.0), Color32::from_rgb(230, 120, 20));
                }
                SnapKind::Grid => {
                    painter.circle_stroke(s, 3.0, Stroke::new(1.0, Color32::from_rgb(230, 120, 20)));
                }
                SnapKind::Free => {}
            }
        }
    }

    // Rubber band for the active tool.
    if let (Some(ray), Some(&first)) = (app.viewport.pointer_ray(), session.pending.first()) {
        let raw = ray_on_plane(&ray, &plane);
        if let Some(cursor) = session.snap.map(|s| s.pos).or(raw) {
            let preview = Stroke::new(1.5, Color32::from_rgb(40, 110, 200).linear_multiply(0.7));
            let last = *session.pending.last().unwrap();
            match session.tool {
                Tool::Line | Tool::Dimension => {
                    if let (Some(a), Some(b)) = (to_screen(sketch.points[last]), to_screen(cursor)) {
                        painter.add(egui::Shape::dashed_line(&[a, b], preview, 5.0, 3.0));
                    }
                }
                Tool::Rectangle => {
                    let a = sketch.points[first];
                    let corners = [a, [cursor[0], a[1]], cursor, [a[0], cursor[1]], a];
                    let pts: Vec<Pos2> = corners.into_iter().filter_map(to_screen).collect();
                    painter.add(egui::Shape::dashed_line(&pts, preview, 5.0, 3.0));
                }
                Tool::Slot => {
                    let a = sketch.points[first];
                    if session.pending.len() == 1 {
                        if let (Some(p), Some(q)) = (to_screen(a), to_screen(cursor)) {
                            painter.add(egui::Shape::dashed_line(&[p, q], preview, 5.0, 3.0));
                        }
                    } else {
                        let b = sketch.points[last];
                        let r = slot_half_width(a, b, cursor);
                        let d = sketch2d::dist(a, b).max(1e-9);
                        let (ux, uy) = ((b[0] - a[0]) / d, (b[1] - a[1]) / d);
                        let base = uy.atan2(ux);
                        let mut outline = Vec::new();
                        // Around the far end, then back round the near end.
                        for (c, from) in [(b, base - std::f64::consts::FRAC_PI_2), (a, base + std::f64::consts::FRAC_PI_2)] {
                            for i in 0..=24 {
                                let t = from + std::f64::consts::PI * i as f64 / 24.0;
                                outline.push([c[0] + r * t.cos(), c[1] + r * t.sin()]);
                            }
                        }
                        outline.push(outline[0]);
                        let pts: Vec<Pos2> = outline.into_iter().filter_map(to_screen).collect();
                        painter.add(egui::Shape::dashed_line(&pts, preview, 5.0, 3.0));
                    }
                }
                Tool::Polygon => {
                    let c = sketch.points[first];
                    let n = session.polygon_sides.clamp(3, 64);
                    let r = sketch2d::dist(c, cursor);
                    let a0 = (cursor[1] - c[1]).atan2(cursor[0] - c[0]);
                    let pts: Vec<Pos2> = (0..=n)
                        .filter_map(|i| {
                            let t = a0 + std::f64::consts::TAU * i as f64 / n as f64;
                            to_screen([c[0] + r * t.cos(), c[1] + r * t.sin()])
                        })
                        .collect();
                    painter.add(egui::Shape::dashed_line(&pts, preview, 5.0, 3.0));
                }
                Tool::Circle | Tool::Arc => {
                    let c = sketch.points[first];
                    let r = sketch2d::dist(c, cursor);
                    let pts: Vec<Pos2> = (0..=48)
                        .filter_map(|i| {
                            let t = std::f64::consts::TAU * i as f64 / 48.0;
                            to_screen([c[0] + r * t.cos(), c[1] + r * t.sin()])
                        })
                        .collect();
                    painter.add(egui::Shape::dashed_line(&pts, preview, 5.0, 3.0));
                }
                Tool::Rectangle3 | Tool::Circle3 | Tool::Arc3 if session.pending.len() == 1 => {
                    if let (Some(a), Some(b)) = (to_screen(sketch.points[first]), to_screen(cursor)) {
                        painter.add(egui::Shape::dashed_line(&[a, b], preview, 5.0, 3.0));
                    }
                }
                Tool::Rectangle3 => {
                    let (a, b) = (sketch.points[first], sketch.points[last]);
                    let d = sketch2d::dist(a, b).max(1e-9);
                    let n = [-(b[1] - a[1]) / d, (b[0] - a[0]) / d];
                    let h = (cursor[0] - a[0]) * n[0] + (cursor[1] - a[1]) * n[1];
                    let corners = [a, b, [b[0] + n[0] * h, b[1] + n[1] * h], [a[0] + n[0] * h, a[1] + n[1] * h], a];
                    let pts: Vec<Pos2> = corners.into_iter().filter_map(to_screen).collect();
                    painter.add(egui::Shape::dashed_line(&pts, preview, 5.0, 3.0));
                }
                Tool::CenterRectangle => {
                    let c = sketch.points[first];
                    let a = [2.0 * c[0] - cursor[0], 2.0 * c[1] - cursor[1]];
                    let corners = [a, [cursor[0], a[1]], cursor, [a[0], cursor[1]], a];
                    let pts: Vec<Pos2> = corners.into_iter().filter_map(to_screen).collect();
                    painter.add(egui::Shape::dashed_line(&pts, preview, 5.0, 3.0));
                }
                Tool::Circle2 | Tool::Circle3 | Tool::Arc3 => {
                    let (a, b) = (sketch.points[first], sketch.points[last]);
                    let arc = match session.tool {
                        Tool::Circle2 => {
                            let c = [(a[0] + cursor[0]) / 2.0, (a[1] + cursor[1]) / 2.0];
                            Some((c, sketch2d::dist(a, cursor) / 2.0, 0.0, std::f64::consts::TAU))
                        }
                        _ => sketch2d::circle_through(a, b, cursor).map(|(c, r)| {
                            if session.tool == Tool::Circle3 {
                                return (c, r, 0.0, std::f64::consts::TAU);
                            }
                            // From the start through the cursor to the end.
                            let ang = |p: [f64; 2]| (p[1] - c[1]).atan2(p[0] - c[0]);
                            let ccw = |from: f64, to: f64| (to - from).rem_euclid(std::f64::consts::TAU);
                            let sweep = ccw(ang(a), ang(b));
                            let sweep = if ccw(ang(a), ang(cursor)) < sweep { sweep } else { sweep - std::f64::consts::TAU };
                            (c, r, ang(a), sweep)
                        }),
                    };
                    if let Some((c, r, from, sweep)) = arc {
                        let pts: Vec<Pos2> = (0..=64)
                            .filter_map(|i| {
                                let t = from + sweep * i as f64 / 64.0;
                                to_screen([c[0] + r * t.cos(), c[1] + r * t.sin()])
                            })
                            .collect();
                        painter.add(egui::Shape::dashed_line(&pts, preview, 5.0, 3.0));
                    }
                }
                Tool::Select | Tool::Trim | Tool::Fillet | Tool::Offset | Tool::Project | Tool::Extend | Tool::Mirror => {}
            }
        }
    }

    // Mirror: the copies the hovered line would make.
    if let (Tool::Mirror, true, Some(Item::Curve(l))) = (session.tool, session.mirror_pick_line, session.hovered) {
        if let Curve::Line { a, b, .. } = sketch.curves[l] {
            let (la, lb) = (sketch.points[a], sketch.points[b]);
            let d = sketch2d::dist(la, lb).max(1e-9);
            let u = [(lb[0] - la[0]) / d, (lb[1] - la[1]) / d];
            let reflect = |p: [f64; 2]| {
                let t = (p[0] - la[0]) * u[0] + (p[1] - la[1]) * u[1];
                [2.0 * (la[0] + u[0] * t) - p[0], 2.0 * (la[1] + u[1] * t) - p[1]]
            };
            let preview = Stroke::new(1.5, Color32::from_rgb(40, 110, 200).linear_multiply(0.7));
            for item in &session.selection {
                if let Item::Curve(c) = *item {
                    let pts: Vec<Pos2> = curve_points(&sketch, c).into_iter().map(reflect).filter_map(to_screen).collect();
                    painter.add(egui::Shape::dashed_line(&pts, preview, 5.0, 3.0));
                }
            }
        }
    }

    // Dimension labels; double-click one to edit it. Ones driven by a
    // parameter expression read "fx:", as in Fusion.
    let exprs = app.doc.features.get(index).map(|f| f.exprs.clone()).unwrap_or_default();
    let mut edit_request = None;
    for (k, constraint) in sketch.constraints.iter().enumerate() {
        let Some(anchor) = dimension_anchor(&sketch, constraint) else { continue };
        let Some(pos) = to_screen(anchor) else { continue };
        if session.editing_dim.as_ref().is_some_and(|(e, _)| *e == k) {
            continue;
        }
        let value = display_value(&sketch, k);
        let prefix = match constraint {
            Constraint::Radius(c, _) if matches!(sketch.curves[*c], Curve::Circle { .. }) => "⌀",
            Constraint::Radius(..) => "R",
            _ => "",
        };
        let expr = exprs.get(&fission_model::dim_key(k));
        let text = if expr.is_some() { format!("fx: {prefix}{value:.2}") } else { format!("{prefix}{value:.2}") };
        let galley = painter.layout_no_wrap(text, egui::FontId::proportional(12.0), theme::palette().ink);
        let rect = egui::Rect::from_center_size(pos + egui::vec2(0.0, -12.0), galley.size() + egui::vec2(8.0, 4.0));
        let response = ui.interact(rect, ui.id().with(("dim", index, k)), egui::Sense::click());
        let bg = if response.hovered() { theme::palette().hover } else { theme::palette().label_bg };
        painter.rect_filled(rect, 2.0, bg);
        painter.galley(rect.center() - galley.size() / 2.0, galley, theme::palette().ink);
        if response.double_clicked() || response.clicked() {
            edit_request = Some((k, expr.cloned().unwrap_or_else(|| format!("{value:.2}"))));
        }
    }
    if let Some(req) = edit_request {
        if let Some(s) = &mut app.sketch {
            s.editing_dim = Some(req);
        }
    }

    // The value editor for the dimension being typed.
    let editing = app.sketch.as_ref().and_then(|s| s.editing_dim.clone());
    if let Some((k, mut text)) = editing {
        let Some(anchor) = sketch.constraints.get(k).and_then(|c| dimension_anchor(&sketch, c)) else {
            if let Some(s) = &mut app.sketch {
                s.editing_dim = None;
            }
            return;
        };
        let Some(pos) = to_screen(anchor) else { return };
        let mut commit = false;
        let mut cancel = false;
        egui::Area::new(egui::Id::new(("dim_edit", index, k)))
            .fixed_pos(pos + egui::vec2(-35.0, -24.0))
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    let edit = ui.add(egui::TextEdit::singleline(&mut text).desired_width(70.0));
                    edit.request_focus();
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        commit = true;
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        cancel = true;
                    }
                });
            });
        if commit {
            // Numbers, units, arithmetic and parameter names all work here.
            match app.doc.evaluate(text.trim(), fission_model::Quantity::Length) {
                Ok(v) if v > 0.0 => {
                    set_dimension(app, index, k, v);
                    let key = fission_model::dim_key(k);
                    if let Some(f) = app.doc.features.get_mut(index) {
                        if fission_model::names_in(&text).is_empty() {
                            f.exprs.remove(&key);
                        } else {
                            f.exprs.insert(key, text.trim().to_string());
                        }
                    }
                }
                Ok(_) => app.notify("A dimension must be positive", true),
                Err(e) => app.notify(e, true),
            }
            if let Some(s) = &mut app.sketch {
                s.editing_dim = None;
            }
        } else if cancel {
            if let Some(s) = &mut app.sketch {
                s.editing_dim = None;
            }
        } else if let Some(s) = &mut app.sketch {
            s.editing_dim = Some((k, text));
        }
    }
}

/// Fill a region with holes in screen space by scanlines (even-odd), which
/// handles any shape and view angle without triangulating.
fn fill_region(painter: &egui::Painter, region: &Region, to_screen: &impl Fn([f64; 2]) -> Option<Pos2>, fill: Color32) {
    let mut loops: Vec<Vec<Pos2>> = vec![region.outer.polygon().into_iter().filter_map(to_screen).collect()];
    for hole in &region.holes {
        loops.push(hole.polygon().into_iter().filter_map(to_screen).collect());
    }
    let (min_y, max_y) = loops.iter().flatten().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), p| (lo.min(p.y), hi.max(p.y)));
    if !min_y.is_finite() {
        return;
    }
    let step = 2.0;
    let mut mesh = egui::Mesh::default();
    let mut y = min_y.floor();
    while y <= max_y {
        let yc = y + step / 2.0;
        let mut xs: Vec<f32> = Vec::new();
        for l in &loops {
            for i in 0..l.len() {
                let (a, b) = (l[i], l[(i + 1) % l.len()]);
                if (a.y > yc) != (b.y > yc) {
                    xs.push(a.x + (yc - a.y) / (b.y - a.y) * (b.x - a.x));
                }
            }
        }
        xs.sort_by(f32::total_cmp);
        for pair in xs.chunks(2) {
            if let [x0, x1] = pair {
                mesh.add_colored_rect(egui::Rect::from_min_max(Pos2::new(*x0, y), Pos2::new(*x1, y + step)), fill);
            }
        }
        y += step;
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// Origin planes offered when starting a sketch with nothing selected.
/// The planes a new sketch can go on: the origin planes, then the
/// document's construction planes; (plane, offset, label, half size).
fn sketchable_planes(app: &FissionApp) -> Vec<(OriginPlane, f64, String, f32)> {
    let size = app.viewport.camera.distance * 0.25;
    let mut out: Vec<(OriginPlane, f64, String, f32)> =
        OriginPlane::ALL.into_iter().map(|op| (op, 0.0, op.label().to_string(), size)).collect();
    for (i, base, offset) in app.doc.construction_planes() {
        out.push((base, offset, app.doc.features[i].name.clone(), size * 0.8));
    }
    out
}

fn plane_quad(app: &FissionApp, plane: &Plane, half: f32) -> Vec<Pos2> {
    let s = half as f64;
    [[-s, -s], [s, -s], [s, s], [-s, s]].into_iter().filter_map(|p| app.viewport.to_screen(world(plane, p))).collect()
}

/// Construction planes, drawn the way Fusion shows them: translucent orange
/// with their name.
pub fn draw_construction_planes(app: &FissionApp, ui: &mut egui::Ui) {
    let painter = ui.painter_at(app.viewport.rect);
    for (op, offset, label, half) in sketchable_planes(app).into_iter().skip(3) {
        let corners = plane_quad(app, &op.plane(offset), half);
        if corners.len() == 4 {
            let fill = Color32::from_rgba_unmultiplied(230, 160, 60, 30);
            painter.add(egui::Shape::convex_polygon(corners.clone(), fill, Stroke::new(1.0, Color32::from_rgb(210, 140, 50))));
            painter.text(corners[2], egui::Align2::LEFT_BOTTOM, label, egui::FontId::proportional(12.0), theme::text());
        }
    }
}

pub fn draw_plane_picker(app: &mut FissionApp, ui: &mut egui::Ui, clicked: bool) {
    let planes = sketchable_planes(app);
    let painter = ui.painter_at(app.viewport.rect);
    let ray = app.viewport.pointer_ray();
    let mut hovered: Option<(usize, f32)> = None;
    for (k, (op, offset, _, half)) in planes.iter().enumerate() {
        let plane = op.plane(*offset);
        let local = ray.as_ref().and_then(|r| ray_on_plane(r, &plane));
        if let Some(p) = local {
            // Construction planes are centered on their origin, like the origin planes.
            if p[0].abs() as f32 <= *half && p[1].abs() as f32 <= *half {
                let hit = world(&plane, p);
                let d = ray.as_ref().map_or(0.0, |r| (hit - r.origin).length());
                if hovered.is_none_or(|(_, best)| d < best) {
                    hovered = Some((k, d));
                }
            }
        }
    }
    for (k, (op, offset, label, half)) in planes.iter().enumerate() {
        let corners = plane_quad(app, &op.plane(*offset), *half);
        let is_hover = hovered.is_some_and(|(h, _)| h == k);
        let fill = if is_hover {
            Color32::from_rgba_unmultiplied(6, 150, 215, 90)
        } else {
            Color32::from_rgba_unmultiplied(230, 160, 60, 40)
        };
        if corners.len() == 4 {
            painter.add(egui::Shape::convex_polygon(corners.clone(), fill, Stroke::new(1.0, Color32::from_rgb(200, 130, 40))));
            painter.text(corners[2], egui::Align2::LEFT_BOTTOM, label, egui::FontId::proportional(12.0), theme::text());
        }
    }
    if clicked {
        if let Some((k, _)) = hovered {
            let (op, offset, ..) = planes[k];
            app.begin_new_sketch(op, offset);
        }
    }
}

/// A value typed while drawing: line length, circle diameter, or rectangle
/// "width, height" (also `40x20`; a single value sets the width). The shape
/// follows the cursor's direction, and the value becomes a dimension.
pub fn apply_typed(app: &mut FissionApp, text: &str) -> Result<(), String> {
    use fission_model::Quantity;
    let doc_for_eval = app.doc.clone();
    let evaluate = |text: &str, q: Quantity| doc_for_eval.evaluate(text, q);
    let session = app.sketch.as_ref().ok_or("not sketching")?;
    let (index, tool) = (session.feature, session.tool);
    if tool == Tool::Offset {
        let curve = session.offset_curve.ok_or("click a curve to offset")?;
        let distance = evaluate(text, Quantity::Length)?;
        let undo = app.sketch_ref(index).ok_or("not sketching")?.clone();
        app.sketch_mut(index).unwrap().offset_chain(curve, distance)?;
        if let Some(s) = &mut app.sketch {
            s.offset_curve = None;
            s.selection.clear();
        }
        app.resolve_sketch(undo);
        return Ok(());
    }
    let first = *session.pending.first().ok_or("click a start point first")?;
    let last = *session.pending.last().unwrap();
    let toward = session.snap.map(|s| s.pos);
    let sketch = app.sketch_ref(index).ok_or("not sketching")?;
    let undo = sketch.clone();
    let from = sketch.points[last];
    let aim = toward.unwrap_or([from[0] + 1.0, from[1]]);
    let unit = |a: [f64; 2], b: [f64; 2]| {
        let d = [b[0] - a[0], b[1] - a[1]];
        let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
        if len < 1e-9 { [1.0, 0.0] } else { [d[0] / len, d[1] / len] }
    };
    // New dimensions typed as expressions with parameter names.
    let mut bound: Vec<(usize, String)> = Vec::new();
    let mut bind = |constraint: usize, text: &str| {
        if !fission_model::names_in(text).is_empty() {
            bound.push((constraint, text.trim().to_string()));
        }
    };
    match tool {
        Tool::Line => {
            let length = evaluate(text, Quantity::Length)?;
            if length <= 0.0 {
                return Err("length must be positive".into());
            }
            let d = unit(from, aim);
            let sketch = app.sketch_mut(index).unwrap();
            let end = sketch.add_point([from[0] + d[0] * length, from[1] + d[1] * length]);
            sketch.curves.push(Curve::Line { a: last, b: end, construction: false });
            let l = sketch.curves.len() - 1;
            if d[1].abs() < 1e-9 {
                sketch.constraints.push(Constraint::Horizontal(l));
            } else if d[0].abs() < 1e-9 {
                sketch.constraints.push(Constraint::Vertical(l));
            }
            sketch.constraints.push(Constraint::Length(l, length));
            bind(sketch.constraints.len() - 1, text);
            if let Some(s) = &mut app.sketch {
                s.pending = vec![end];
            }
        }
        Tool::Slot => {
            if session_pending_len(app) < 2 {
                return Err("click the second center first".into());
            }
            let width = evaluate(text, Quantity::Length)?;
            if width <= 0.0 {
                return Err("width must be positive".into());
            }
            let sketch = app.sketch_mut(index).unwrap();
            let [_, _, arc1, _] = sketch.add_slot(first, last, width);
            sketch.constraints.push(Constraint::Radius(arc1, width / 2.0));
            if let Some(s) = &mut app.sketch {
                s.pending.clear();
            }
        }
        Tool::Polygon => {
            let radius = evaluate(text, Quantity::Length)?;
            if radius <= 0.0 {
                return Err("radius must be positive".into());
            }
            let sides = app.sketch.as_ref().map_or(6, |s| s.polygon_sides);
            let c = sketch.points[first];
            let d = unit(c, aim);
            let sketch = app.sketch_mut(index).unwrap();
            let (circle, _) = sketch.add_polygon(first, [c[0] + d[0] * radius, c[1] + d[1] * radius], sides);
            sketch.constraints.push(Constraint::Radius(circle, radius));
            bind(sketch.constraints.len() - 1, text);
            if let Some(s) = &mut app.sketch {
                s.pending.clear();
            }
        }
        Tool::Circle => {
            let diameter = evaluate(text, Quantity::Length)?;
            if diameter <= 0.0 {
                return Err("diameter must be positive".into());
            }
            let sketch = app.sketch_mut(index).unwrap();
            sketch.curves.push(Curve::Circle { center: first, radius: diameter / 2.0, construction: false });
            let c = sketch.curves.len() - 1;
            sketch.constraints.push(Constraint::Radius(c, diameter / 2.0));
            bind(sketch.constraints.len() - 1, text);
            if let Some(s) = &mut app.sketch {
                s.pending.clear();
            }
        }
        Tool::Rectangle => {
            // "W,H"; "WxH" too when no parameter names are involved (they
            // may contain an x), and `*` stays multiplication.
            let seps: &[char] = if text.contains(',') || !fission_model::names_in(text).is_empty() { &[','] } else { &['x', 'X'] };
            let parts: Vec<&str> = text.split(seps).map(str::trim).filter(|p| !p.is_empty()).collect();
            let corner = sketch.points[first];
            let width = evaluate(parts.first().ok_or("enter a width")?, Quantity::Length)?;
            let height = match parts.get(1) {
                Some(h) => evaluate(h, Quantity::Length)?,
                None => (aim[1] - corner[1]).abs().max(1e-3),
            };
            if width <= 0.0 || height <= 0.0 {
                return Err("sizes must be positive".into());
            }
            let sx = if aim[0] < corner[0] { -1.0 } else { 1.0 };
            let sy = if aim[1] < corner[1] { -1.0 } else { 1.0 };
            let sketch = app.sketch_mut(index).unwrap();
            let [bottom, _, _, left] = sketch.add_rectangle(corner, [corner[0] + sx * width, corner[1] + sy * height]);
            let start = sketch.line_points(bottom).0;
            sketch.constraints.push(Constraint::Coincident(start, first));
            sketch.constraints.push(Constraint::Length(bottom, width));
            bind(sketch.constraints.len() - 1, parts[0]);
            if parts.len() > 1 {
                sketch.constraints.push(Constraint::Length(left, height));
                bind(sketch.constraints.len() - 1, parts[1]);
            }
            if let Some(s) = &mut app.sketch {
                s.pending.clear();
            }
        }
        _ => return Err("type a value while drawing a line, circle or rectangle".into()),
    }
    if app.resolve_sketch(undo) {
        if let Some(f) = app.doc.features.get_mut(index) {
            for (k, text) in bound {
                f.exprs.insert(fission_model::dim_key(k), text);
            }
        }
        Ok(())
    } else {
        Err(app.sketch.as_ref().and_then(|s| s.message.clone()).unwrap_or_else(|| "that conflicts with the sketch".into()))
    }
}

#[cfg(test)]
mod snap_tests {
    use super::*;

    fn square() -> sketch2d::Sketch {
        let mut s = sketch2d::Sketch::default();
        s.add_line([10.0, 10.0], [30.0, 10.0]);
        s.add_circle([50.0, 50.0], 5.0);
        s
    }

    #[test]
    fn snap_priority() {
        let s = square();
        let tol = 1.0;
        assert_eq!(snap(&s, [10.4, 10.3], tol, None, None, None).kind, SnapKind::Point(0));
        assert_eq!(snap(&s, [0.3, -0.2], tol, None, None, None).pos, [0.0, 0.0]);
        assert_eq!(snap(&s, [20.2, 10.4], tol, None, None, None).kind, SnapKind::Midpoint(0));
        let on = snap(&s, [25.0, 10.6], tol, None, None, None);
        assert_eq!(on.kind, SnapKind::OnCurve(0));
        assert_eq!(on.pos, [25.0, 10.0]);
        assert_eq!(snap(&s, [55.6, 50.0], tol, None, None, None).kind, SnapKind::OnCurve(1));
        // Alignment with the point being drawn from, rounded to the grid along it.
        let h = snap(&s, [73.3, 30.4], tol, Some([60.0, 30.0]), Some(5.0), None);
        assert_eq!((h.kind, h.pos), (SnapKind::Horizontal, [75.0, 30.0]));
        let g = snap(&s, [73.3, 37.9], tol, None, Some(5.0), None);
        assert_eq!((g.kind, g.pos), (SnapKind::Grid, [75.0, 40.0]));
    }

    #[test]
    fn grid_steps_are_nice_numbers() {
        assert_eq!(grid_step(0.7), 1.0);
        assert_eq!(grid_step(1.3), 2.0);
        assert_eq!(grid_step(3.0), 5.0);
        assert_eq!(grid_step(12.0), 20.0);
    }
}

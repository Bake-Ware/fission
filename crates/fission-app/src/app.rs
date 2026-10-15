use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};
use eframe::egui_wgpu::RenderState;
use fission_kernel::{EdgeHint, Kernel, Mesh, EDGE_MATCH_TOLERANCE};
use fission_model::{
    interior_point, Axis, Document, EdgeRef, FaceRef, FeatureKind, FeatureStatus, HoleKind, History, Operation, OriginPlane, PatternKind, Sketch,
};
use fission_occt::{Occt, Solid};

use crate::worker;

use crate::ui;
use crate::viewport::{DisplayBody, Highlights, Pick, Viewport};

/// Finer deflection for STL export.
const EXPORT_TOLERANCE: f64 = 0.01;

/// A dialog open on one feature. `before` restores the document on Cancel.
pub struct Edit {
    pub index: usize,
    pub before: Document,
    pub is_new: bool,
}

pub struct Toast {
    pub text: String,
    pub error: bool,
    pub until: Instant,
}

pub struct FissionApp {
    /// The geometry kernel, in a crash-isolated worker process.
    pub kernel: worker::Kernel,
    pub doc: Document,
    pub history: History,
    pub path: Option<PathBuf>,
    saved: Document,

    /// Bodies of the full timeline (what gets exported): name and creating feature.
    pub model_bodies: Vec<(String, usize)>,
    /// What the viewport shows: the full model, or while editing an edge
    /// feature, the model just before it (so its edges can be picked).
    pub bodies: Vec<DisplayBody>,
    /// Bodies hidden in the browser, by creating feature.
    pub hidden: HashSet<usize>,
    /// While editing: bodies that exist before the edited feature (name, creating feature).
    pub edit_base: Vec<(String, usize)>,
    /// Show the edited fillet/chamfer's result instead of the pickable base.
    pub edit_preview: bool,
    /// Solved sketches from the last full regeneration, per feature.
    pub regen_sketches: Vec<Option<fission_model::SketchOutput>>,
    pub status: Vec<FeatureStatus>,
    pub regen_ms: f32,
    pub(crate) needs_regen: bool,
    first_regen: bool,

    pub viewport: Viewport,
    render_state: RenderState,
    pub editing: Option<Edit>,
    pub selected_feature: Option<usize>,
    pub toast: Option<Toast>,
    /// Per feature: how many parts an import brought in (from the last regen).
    pub import_parts: Vec<usize>,
    /// INSPECT ▸ Measure, while open.
    pub measure: Option<ui::measure::Measure>,
    pub params_dialog: Option<ui::params::ParamsDialog>,
    pub section: Option<ui::section::Section>,
    pub appearance: Option<ui::appearance::AppearanceDialog>,
    pub import_part_faces: Vec<Vec<(usize, usize)>>,
    /// Document as it was when a history-marker drag started.
    pub marker_drag: Option<Document>,
    /// FISSION_DEMO `menu:<PANEL>`: open that toolbar dropdown once at startup.
    pub demo_menu: Option<String>,
    /// Active sketch editing session (the SKETCH contextual tab).
    pub sketch: Option<ui::sketch::Session>,
    /// Waiting for the user to click an origin plane for a new sketch.
    pub picking_sketch_plane: bool,
    /// Value being typed into the active command (Fusion-style heads-up box).
    pub hud: Option<ui::hud::Hud>,
    /// A joint being scrubbed or played (display only, until committed).
    pub joint_preview: Option<crate::motion::JointPreview>,
    /// Decoded reference images, by path.
    pub canvas_store: ui::canvas::CanvasStore,
    /// With a Move/Joint dialog open: the model before it, which follows the dialog.
    pub rigid_base: Option<crate::gizmo::RigidBase>,
    pub gizmo_drag: Option<crate::gizmo::GizmoDrag>,
}

impl FissionApp {
    pub fn new(cc: &eframe::CreationContext<'_>, path: Option<PathBuf>) -> Self {
        ui::theme::init(&cc.egui_ctx);
        let render_state = cc.wgpu_render_state.clone().expect("fission needs the wgpu renderer");

        let mut app = FissionApp {
            kernel: worker::Kernel::new(),
            doc: Document::sample(),
            history: History::default(),
            path: None,
            saved: Document::sample(),
            model_bodies: Vec::new(),
            bodies: Vec::new(),
            hidden: HashSet::new(),
            edit_base: Vec::new(),
            edit_preview: false,
            regen_sketches: Vec::new(),
            status: Vec::new(),
            regen_ms: 0.0,
            needs_regen: true,
            first_regen: true,
            viewport: Viewport::new(&render_state),
            render_state,
            editing: None,
            selected_feature: None,
            toast: None,
            import_parts: Vec::new(),
            measure: None,
            params_dialog: None,
            section: None,
            appearance: None,
            import_part_faces: Vec::new(),
            marker_drag: None,
            demo_menu: None,
            sketch: None,
            picking_sketch_plane: false,
            hud: None,
            joint_preview: None,
            canvas_store: Default::default(),
            rigid_base: None,
            gizmo_drag: None,
        };
        if let Some(path) = path {
            app.open_path(&path);
        }
        if let Ok(steps) = std::env::var("FISSION_DEMO") {
            app.run_demo(&steps);
        }
        app
    }

    /// Put the UI into a given state for screenshots and manual checks, e.g.
    /// `FISSION_DEMO="edit:3,select-top"`. Steps: `edit:<i>`, `marker:<i>`,
    /// `suppress:<i>`, `break-fillet`, `view:<top|front|right|home>`, `menu:<create|modify|export>`,
    /// `pick-edge:<n>`, `pick-face:<n>` (toggle-click on the first body), `preview`,
    /// `joint:<name>=<value>`, `joint-play:<name>`.
    fn run_demo(&mut self, steps: &str) {
        self.regenerate();
        for step in steps.split(',').map(str::trim) {
            let (name, arg) = step.split_once(':').unwrap_or((step, ""));
            let index = arg.parse::<usize>().ok();
            match (name, index) {
                ("edit", Some(i)) => self.begin_edit(i),
                ("marker", Some(i)) => self.commit(|doc| doc.marker = i.min(doc.features.len())),
                ("suppress", Some(i)) if i < self.doc.features.len() => {
                    self.commit(|doc| doc.features[i].suppressed = true)
                }
                ("menu", _) => self.demo_menu = Some(arg.to_uppercase()),
                // Click edge/face <n> of the first shown body, as a toggle click.
                ("pick-edge", Some(n)) => {
                    let point = self.bodies.first().and_then(|b| b.mesh.edges.get(n)).and_then(|p| p.first()).copied();
                    if let Some(p) = point {
                        let hit = crate::viewport::EdgeHit { body: 0, edge: n as u32, point: p.into() };
                        self.handle_click(Pick::Edge(hit), true);
                    }
                }
                ("pick-face", Some(n)) => {
                    let mesh = self.bodies.first().map(|b| &b.mesh);
                    let tri = mesh.and_then(|m| m.face_ids.iter().position(|&f| f == n as u32));
                    if let (Some(mesh), Some(t)) = (mesh, tri) {
                        let a = glam::Vec3::from(mesh.positions[mesh.indices[t * 3] as usize]);
                        let b = glam::Vec3::from(mesh.positions[mesh.indices[t * 3 + 1] as usize]);
                        let c = glam::Vec3::from(mesh.positions[mesh.indices[t * 3 + 2] as usize]);
                        let normal = (b - a).cross(c - a).normalize_or_zero();
                        let hit = crate::viewport::Hit { body: 0, face: n as u32, point: (a + b + c) / 3.0, normal, distance: 0.0 };
                        self.handle_click(Pick::Face(hit), true);
                    }
                }
                ("preview", _) => {
                    self.edit_preview = true;
                    self.changed();
                }
                // Sketching: `sketch:xy`, `tool:line|rect|circle|arc|dim|select`,
                // `sk:x;y` click (double: `sk2:x;y`), `sel:x;y` toggle-select,
                // `constrain:<label>`, `dim:<k>;<value>`, `finish`, `pick:x;y` (profile).
                ("sketch", _) => {
                    let plane = match arg {
                        "xz" => OriginPlane::XZ,
                        "yz" => OriginPlane::YZ,
                        _ => OriginPlane::XY,
                    };
                    self.begin_new_sketch(plane, 0.0);
                }
                ("tool", _) => {
                    use ui::sketch::Tool;
                    let tool = match arg {
                        "line" => Tool::Line,
                        "rect" => Tool::Rectangle,
                        "circle" => Tool::Circle,
                        "arc" => Tool::Arc,
                        "dim" => Tool::Dimension,
                        _ => Tool::Select,
                    };
                    self.set_sketch_tool(tool);
                }
                ("sk" | "sk2" | "sel", _) => {
                    if let Some(p) = demo_point(arg) {
                        if name == "sel" {
                            self.set_sketch_tool(ui::sketch::Tool::Select);
                        }
                        let events = crate::viewport::ViewportEvents {
                            primary_clicked: true,
                            primary_double_clicked: name == "sk2",
                            toggle: name == "sel",
                            ..Default::default()
                        };
                        ui::sketch::handle_at(self, &events, p, 0.5, None);
                        if let Some(s) = &mut self.sketch {
                            s.editing_dim = None;
                        }
                    }
                }
                ("hover", _) => {
                    if let Some(p) = demo_point(arg) {
                        let events = crate::viewport::ViewportEvents::default();
                        ui::sketch::handle_at(self, &events, p, 1.5, Some(5.0));
                    }
                }
                // Type into the active command; `type:40,` commits like Enter,
                // `typing:40` leaves the box open for a screenshot.
                ("typing", _) => self.hud = Some(ui::hud::Hud::new(arg.to_string())),
                ("type", _) => {
                    if let Err(e) = ui::hud::commit_text(self, arg) {
                        log::warn!("typed '{arg}': {e}");
                    }
                }
                ("constrain", _) => {
                    if let Some(cmd) = ui::sketch::ConstraintCmd::ALL.into_iter().find(|c| c.label().eq_ignore_ascii_case(arg)) {
                        self.apply_constraint(cmd);
                    }
                }
                ("dim", _) => {
                    let parts: Vec<f64> = arg.split(';').filter_map(|x| x.trim().parse().ok()).collect();
                    if let ([k, v], Some(sk)) = (parts.as_slice(), self.sketch.as_ref().map(|s| s.feature)) {
                        ui::sketch::set_dimension(self, sk, *k as usize, *v);
                    }
                }
                ("finish", _) => self.finish_sketch(),
                ("new", _) => self.new_document(),
                ("import", _) => {
                    self.import_step_path(Path::new(arg));
                    self.first_regen = true;
                }
                // Run a toolbar command by its label, e.g. `cmd:shell`.
                ("cmd", _) => {
                    let all = ui::commands::SOLID_PANELS.iter().flat_map(|p| p.dropdown.iter());
                    let mut found = None;
                    for entry in all {
                        if let ui::commands::Entry::Cmd(c) = entry {
                            if c.label().eq_ignore_ascii_case(arg) {
                                found = Some(*c);
                            }
                        }
                        if let ui::commands::Entry::Submenu(_, items) = entry {
                            for item in *items {
                                if let ui::commands::Entry::Cmd(c) = item {
                                    if c.label().eq_ignore_ascii_case(arg) {
                                        found = Some(*c);
                                    }
                                }
                            }
                        }
                    }
                    match found {
                        Some(c) => c.run(self),
                        None => log::warn!("no command '{arg}'"),
                    }
                }
                // Click the highest upward-facing face of the first shown body.
                ("pick-top", _) => {
                    let best = self.bodies.first().and_then(|b| {
                        b.mesh
                            .face_hints
                            .iter()
                            .enumerate()
                            .filter(|(_, h)| h.normal[2] > 0.99)
                            .max_by(|a, b| a.1.center[2].total_cmp(&b.1.center[2]))
                            .map(|(i, h)| (i, *h))
                    });
                    if let Some((face, hint)) = best {
                        let c = hint.center.map(|x| x as f32);
                        let hit = crate::viewport::Hit { body: 0, face: face as u32, point: c.into(), normal: glam::Vec3::Z, distance: 0.0 };
                        self.handle_click(Pick::Face(hit), true);
                    }
                }
                ("extrude", _) => self.new_extrude(),
                ("pick", _) => {
                    // Profile pick at a sketch-plane point for the extrude being edited.
                    if let (Some(p), Some((sk, _))) = (demo_point(arg), self.editing_profiles()) {
                        self.pick_profile_at(sk, p);
                    }
                }
                ("break-fillet", _) => self.commit(|doc| {
                    if let Some(FeatureKind::Fillet { radius, .. }) =
                        doc.features.iter_mut().map(|f| &mut f.kind).find(|k| matches!(k, FeatureKind::Fillet { .. }))
                    {
                        *radius = 40.0;
                    }
                }),
                // Show a joint at a value without committing it: `joint:Hinge=-90`
                // (no name: the only joint). `joint-play:Hinge` starts it swinging.
                ("joint" | "joint-play", _) => {
                    let (joint, value) = arg.split_once('=').unwrap_or((arg, ""));
                    match self.doc.joint(joint) {
                        Ok(j) if name == "joint-play" => self.toggle_joint_play(j.feature),
                        Ok(j) => match value.trim().parse::<f64>() {
                            Ok(v) => self.preview_joint(j.feature, v),
                            Err(_) => log::warn!("joint step '{arg}' needs =<value>"),
                        },
                        Err(e) => log::warn!("{e}"),
                    }
                }
                // Insert a reference image: `canvas:<path>`.
                ("canvas", _) => self.insert_canvas(Path::new(arg)),
                ("view", _) => {
                    use crate::viewport::StandardView::*;
                    let view = match arg {
                        "top" => Top,
                        "front" => Front,
                        "right" => Right,
                        _ => Home,
                    };
                    self.viewport.camera.set_view(view);
                }
                _ => log::warn!("unknown FISSION_DEMO step '{step}'"),
            }
            if self.needs_regen {
                self.regenerate();
            }
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.doc != self.saved
    }

    /// Call after any change to `doc`; regeneration happens before the next draw.
    pub fn changed(&mut self) {
        self.needs_regen = true;
    }

    /// Apply a change as one undoable step.
    pub fn commit(&mut self, change: impl FnOnce(&mut Document)) {
        let before = self.doc.clone();
        change(&mut self.doc);
        if self.doc != before {
            self.changed_from(&before);
            self.history.record(before);
        }
    }

    /// `doc` was `before` until now. Appearance alone doesn't change any
    /// body, and a full regeneration can take seconds, so it's skipped then:
    /// the viewport reads colors from `doc` every frame.
    fn changed_from(&mut self, before: &Document) {
        if !self.doc.same_except_appearance(before) {
            self.changed();
        }
    }

    fn regenerate(&mut self) {
        let start = Instant::now();
        // While editing, also build the model as it was just before the
        // feature: its bodies are the dialog's targets, and pick features
        // pick on it.
        let base_marker = self.editing.as_ref().map(|e| e.index);
        // Parameters first: expressions write their values into the features.
        let problems = self.doc.apply_expressions();
        if let Some(p) = problems.first() {
            self.notify(p.clone(), true);
        }
        let result = match self.kernel.regenerate(&self.doc, base_marker) {
            Ok(result) => result,
            Err(e) => {
                self.notify(e, true);
                self.needs_regen = false;
                return;
            }
        };
        for e in &result.errors {
            self.notify(e.clone(), true);
        }
        self.status = result.status;
        self.import_parts = result.parts;
        self.import_part_faces = result.part_faces;
        self.regen_sketches = self.doc.sketch_outputs();
        self.model_bodies = result.bodies.iter().map(|b| (b.name.clone(), b.created_by)).collect();
        self.edit_base = result.base.iter().flatten().map(|b| (b.name.clone(), b.created_by)).collect();

        let shown = match result.base {
            Some(base) if self.editing_pick_feature() && !self.edit_preview => base,
            _ => result.bodies,
        };
        self.bodies = shown
            .into_iter()
            .map(|b| DisplayBody {
                visible: !self.hidden.contains(&b.created_by),
                name: b.name,
                mesh: b.mesh,
                created_by: b.created_by,
            })
            .collect();
        self.regen_ms = start.elapsed().as_secs_f32() * 1000.0;
        // New meshes replace any joint preview's.
        self.joint_preview = None;
        self.refresh_canvases();
        // A Move/Joint dialog shows its bodies moved from here on, without rebuilding.
        self.set_rigid_base();
        self.viewport.bodies_changed(&self.bodies);
        if self.first_regen {
            self.viewport.fit_all(&self.bodies);
            self.first_regen = false;
        }
        self.needs_regen = false;
    }

    pub fn set_body_visible(&mut self, created_by: usize, visible: bool) {
        if visible {
            self.hidden.remove(&created_by);
        } else {
            self.hidden.insert(created_by);
        }
        for body in self.bodies.iter_mut().filter(|b| b.created_by == created_by) {
            body.visible = visible;
        }
        self.viewport.bodies_changed(&self.bodies);
        self.refresh_canvases();
    }

    /// Sketches that are applied, unsuppressed, and not yet used by a later
    /// feature: Fusion shows these and offers their profiles.
    pub fn open_sketches(&self) -> Vec<usize> {
        let used: Vec<usize> = self
            .doc
            .features
            .iter()
            .flat_map(|f| match &f.kind {
                FeatureKind::Extrude { sketch: Sketch::Profiles { sketch, .. }, .. }
                | FeatureKind::Revolve { sketch: Sketch::Profiles { sketch, .. }, .. } => vec![*sketch],
                FeatureKind::Loft { sections, .. } => sections.iter().map(|s| s.sketch).collect(),
                FeatureKind::Sweep { profile, path, .. } => {
                    let mut v: Vec<usize> = path.iter().map(|p| p.sketch).collect();
                    if let Sketch::Profiles { sketch, .. } = profile {
                        v.push(*sketch);
                    }
                    v
                }
                _ => Vec::new(),
            })
            .collect();
        (0..self.doc.features.len().min(self.doc.marker))
            .filter(|&i| {
                let f = &self.doc.features[i];
                matches!(f.kind, FeatureKind::Sketch { .. }) && !f.suppressed && !used.contains(&i)
            })
            .collect()
    }

    /// While editing an extrude/revolve from sketch profiles: (sketch, picked points).
    pub fn editing_profiles(&self) -> Option<(usize, Vec<[f64; 2]>)> {
        let feature = self.doc.features.get(self.editing.as_ref()?.index)?;
        match &feature.kind {
            FeatureKind::Extrude { sketch: Sketch::Profiles { sketch, points }, .. }
            | FeatureKind::Revolve { sketch: Sketch::Profiles { sketch, points }, .. } => Some((*sketch, points.clone())),
            _ => None,
        }
    }

    /// Toggle the profile under the pointer in the extrude being edited.
    fn pick_profile(&mut self) -> bool {
        let Some((sk, _)) = self.editing_profiles() else { return false };
        let (Some(plane), Some(ray)) = (self.sketch_plane(sk), self.viewport.pointer_ray()) else { return false };
        let Some(p) = ui::sketch::ray_on_plane(&ray, &plane) else { return false };
        self.pick_profile_at(sk, p)
    }

    fn pick_profile_at(&mut self, sk: usize, p: [f64; 2]) -> bool {
        let Some(out) = self.regen_sketches.get(sk).and_then(Option::as_ref) else { return false };
        let Some(region) = out.region_at(p).cloned() else { return false };
        let index = self.editing.as_ref().unwrap().index;
        if let FeatureKind::Extrude { sketch: Sketch::Profiles { points, .. }, .. }
        | FeatureKind::Revolve { sketch: Sketch::Profiles { points, .. }, .. } = &mut self.doc.features[index].kind
        {
            match points.iter().position(|q| region.contains(*q)) {
                Some(i) => {
                    points.remove(i);
                }
                None => points.push(interior_point(&region)),
            }
        }
        self.changed();
        true
    }

    /// Does the open dialog pick edges, faces or bodies on the model before
    /// the feature (so the viewport shows that model while editing)?
    pub fn editing_pick_feature(&self) -> bool {
        self.editing.as_ref().and_then(|e| self.doc.features.get(e.index)).is_some_and(|f| {
            matches!(
                f.kind,
                FeatureKind::Fillet { .. }
                    | FeatureKind::Chamfer { .. }
                    | FeatureKind::Shell { .. }
                    | FeatureKind::Draft { .. }
                    | FeatureKind::OffsetFace { .. }
                    | FeatureKind::DeleteFace { .. }
                    | FeatureKind::Thread { .. }
                    | FeatureKind::Combine { .. }
                    | FeatureKind::Move { .. }
                    | FeatureKind::Joint { .. }
                    | FeatureKind::Mirror { .. }
                    | FeatureKind::Scale { .. }
                    | FeatureKind::Split { .. }
                    | FeatureKind::Pattern { .. }
            )
        })
    }

    fn edited_kind_mut(&mut self) -> Option<&mut FeatureKind> {
        let index = self.editing.as_ref()?.index;
        self.doc.features.get_mut(index).map(|f| &mut f.kind)
    }

    /// Display index of the body created by `owner`.
    fn display_body(&self, owner: usize) -> Option<usize> {
        self.bodies.iter().position(|b| b.created_by == owner)
    }

    /// What the open dialog has picked, for highlighting.
    pub fn highlights(&self) -> Highlights {
        let mut h = Highlights { edges: self.edited_edges(), ..Default::default() };
        // Measure's picks stay lit.
        for pick in self.measure.iter().flat_map(|m| &m.picks) {
            match pick {
                Pick::Face(f) => h.faces.push((f.body, f.face)),
                Pick::Edge(e) => h.edges.push((e.body, e.edge)),
            }
        }
        let Some(feature) = self.editing.as_ref().and_then(|e| self.doc.features.get(e.index)) else { return h };
        match &feature.kind {
            FeatureKind::Shell { faces, .. }
            | FeatureKind::Draft { faces, .. }
            | FeatureKind::OffsetFace { faces, .. }
            | FeatureKind::DeleteFace { faces } => {
                for f in faces {
                    if let Some(bi) = self.display_body(f.body) {
                        let hit = self.bodies[bi].mesh.face_hints.iter().position(|hint| {
                            fission_kernel::dist(hint.center, f.center) < EDGE_MATCH_TOLERANCE
                        });
                        if let Some(face) = hit {
                            h.faces.push((bi, face as u32));
                        }
                    }
                }
            }
            FeatureKind::Combine { tools, .. } => h.bodies = tools.iter().filter_map(|&t| self.display_body(t)).collect(),
            FeatureKind::Split { body: Some(b), .. } => h.bodies = self.display_body(*b).into_iter().collect(),
            FeatureKind::Thread { face, .. } => {
                if let Some(bi) = self.display_body(face.body) {
                    let hints = &self.bodies[bi].mesh.face_hints;
                    if let Some(f) = hints.iter().position(|hint| fission_kernel::dist(hint.center, face.center) < EDGE_MATCH_TOLERANCE) {
                        h.faces.push((bi, f as u32));
                    }
                }
            }
            FeatureKind::Move { bodies, .. }
            | FeatureKind::Joint { bodies, .. }
            | FeatureKind::Mirror { bodies, .. }
            | FeatureKind::Pattern { bodies, .. }
            | FeatureKind::Scale { bodies, .. } => {
                h.bodies = bodies.iter().filter_map(|&b| self.display_body(b)).collect()
            }
            _ => {}
        }
        h
    }

    /// A stable reference to a displayed edge.
    fn edge_ref(&self, body: usize, edge: u32) -> Option<EdgeRef> {
        let display = self.bodies.get(body)?;
        let hint = EdgeHint::from_polyline(display.mesh.edges.get(edge as usize)?)?;
        Some(EdgeRef { body: display.created_by, mid: hint.mid, dir: hint.dir })
    }

    /// Edges of the feature being edited, as (display body, edge) for highlighting.
    pub fn edited_edges(&self) -> Vec<(usize, u32)> {
        let Some(feature) = self.editing.as_ref().and_then(|e| self.doc.features.get(e.index)) else {
            return Vec::new();
        };
        let (FeatureKind::Fillet { edges, .. } | FeatureKind::Chamfer { edges, .. }) = &feature.kind else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for r in edges {
            for (bi, body) in self.bodies.iter().enumerate().filter(|(_, b)| b.created_by == r.body) {
                let hit = body.mesh.edges.iter().position(|p| {
                    EdgeHint::from_polyline(p).is_some_and(|h| fission_kernel::dist(h.mid, r.mid) < EDGE_MATCH_TOLERANCE)
                });
                if let Some(ei) = hit {
                    out.push((bi, ei as u32));
                }
            }
        }
        out
    }

    /// Route a viewport click: an open Fillet/Chamfer dialog takes edges,
    /// otherwise it changes the selection.
    pub fn handle_click(&mut self, pick: Pick, toggle: bool) {
        if let Some(m) = &mut self.measure {
            m.add(pick);
            return;
        }
        if self.place_hole(pick) {
            return;
        }
        if self.editing_pick_feature() && !self.edit_preview && self.pick_into_feature(pick) {
            if self.editing_rigid() {
                self.refresh_rigid_preview();
            } else {
                self.changed();
            }
            return;
        }
        if self.editing_pick_feature() && !self.edit_preview {
            if let Pick::Edge(e) = pick {
                let Some(new) = self.edge_ref(e.body, e.edge) else { return };
                let index = self.editing.as_ref().unwrap().index;
                if let FeatureKind::Fillet { edges, .. } | FeatureKind::Chamfer { edges, .. } =
                    &mut self.doc.features[index].kind
                {
                    match edges.iter().position(|r| fission_kernel::dist(r.mid, new.mid) < EDGE_MATCH_TOLERANCE) {
                        Some(i) => {
                            edges.remove(i);
                        }
                        None => edges.push(new),
                    }
                }
                self.changed();
            }
            return;
        }
        self.viewport.select(pick, toggle);
    }

    /// Faces and bodies a dialog collects by clicking (edges are handled in
    /// `handle_click`). Returns whether the pick was used.
    fn pick_into_feature(&mut self, pick: Pick) -> bool {
        let body_owner = self.bodies.get(pick.body()).map(|b| b.created_by);
        let edited = self.editing.as_ref().map_or(usize::MAX, |e| e.index);
        let face_ref = match pick {
            Pick::Face(hit) => self.bodies.get(hit.body).and_then(|b| {
                let hint = b.mesh.face_hints.get(hit.face as usize)?;
                Some(FaceRef { body: b.created_by, center: hint.center, normal: hint.normal })
            }),
            Pick::Edge(_) => None,
        };
        let editing_thread = matches!(self.edited_kind_mut(), Some(FeatureKind::Thread { .. }));
        let thread_size = match pick {
            Pick::Face(hit) if editing_thread => self.bodies.get(hit.body).and_then(|b| thread_fit(&b.mesh, hit.face)),
            _ => None,
        };
        let toggle = |list: &mut Vec<usize>, owner: usize| match list.iter().position(|&b| b == owner) {
            Some(i) => {
                list.remove(i);
            }
            None => list.push(owner),
        };
        match self.edited_kind_mut() {
            Some(
                FeatureKind::Shell { faces, .. }
                | FeatureKind::Draft { faces, .. }
                | FeatureKind::OffsetFace { faces, .. }
                | FeatureKind::DeleteFace { faces },
            ) => {
                let Some(new) = face_ref else { return false };
                match faces.iter().position(|f| fission_kernel::dist(f.center, new.center) < EDGE_MATCH_TOLERANCE) {
                    Some(i) => {
                        faces.remove(i);
                    }
                    None => faces.push(new),
                }
                true
            }
            Some(FeatureKind::Thread { face, size, pitch, .. }) => {
                let (Some(new), Some(fit)) = (face_ref, thread_size) else { return false };
                *face = new;
                (*size, *pitch) = fit;
                true
            }
            // The first click picks the body; later ones split along a face.
            Some(FeatureKind::Split { body, plane, offset }) => {
                let (Some(owner), Pick::Face(hit)) = (body_owner, pick) else { return false };
                if body.is_none() {
                    *body = Some(owner);
                } else {
                    (*plane, *offset, _) = plane_through(hit.point.as_dvec3(), hit.normal.as_dvec3());
                }
                true
            }
            Some(FeatureKind::Combine { target, tools, .. }) => {
                let Some(owner) = body_owner else { return false };
                if owner != *target {
                    toggle(tools, owner);
                }
                true
            }
            Some(FeatureKind::Move { bodies, .. } | FeatureKind::Joint { bodies, .. }) => {
                // A copy the preview shows belongs to the Move itself.
                let Some(owner) = body_owner.filter(|&o| o < edited) else { return true };
                toggle(bodies, owner);
                true
            }
            Some(
                FeatureKind::Mirror { bodies, .. }
                | FeatureKind::Pattern { bodies, .. }
                | FeatureKind::Scale { bodies, .. },
            ) => {
                let Some(owner) = body_owner else { return false };
                toggle(bodies, owner);
                true
            }
            _ => false,
        }
    }

    /// Bodies (by creating feature) under the current selection.
    fn selected_bodies(&self) -> Vec<usize> {
        let mut out: Vec<usize> =
            self.viewport.selection.iter().filter_map(|p| self.bodies.get(p.body()).map(|b| b.created_by)).collect();
        out.dedup();
        out
    }

    /// The selected faces, as references that survive a rebuild.
    fn selected_face_refs(&self) -> Vec<FaceRef> {
        self.viewport
            .selection
            .iter()
            .filter_map(|p| {
                let hit = p.face()?;
                let b = self.bodies.get(hit.body)?;
                let hint = b.mesh.face_hints.get(hit.face as usize)?;
                Some(FaceRef { body: b.created_by, center: hint.center, normal: hint.normal })
            })
            .collect()
    }

    pub fn new_shell(&mut self) {
        let faces = self.selected_face_refs();
        self.create(FeatureKind::Shell { thickness: 1.0, faces });
    }

    /// MODIFY ▸ Draft: the selected faces, pulled up from XY, 2° to start.
    pub fn new_draft(&mut self) {
        let faces = self.selected_face_refs();
        self.create(FeatureKind::Draft { faces, plane: OriginPlane::XY, offset: 0.0, degrees: 2.0, flip: false });
    }

    /// MODIFY ▸ Offset Face: the selected faces, out 5 mm to start.
    pub fn new_offset_face(&mut self) {
        let faces = self.selected_face_refs();
        self.create(FeatureKind::OffsetFace { faces, distance: 5.0 });
    }

    /// CREATE ▸ Thread: on the selected round face, sized from its diameter.
    pub fn new_thread(&mut self) {
        let picked = self.viewport.selected_face().copied().and_then(|hit| {
            let body = self.bodies.get(hit.body)?;
            let hint = body.mesh.face_hints.get(hit.face as usize)?;
            let fit = thread_fit(&body.mesh, hit.face)?;
            Some((FaceRef { body: body.created_by, center: hint.center, normal: hint.normal }, fit))
        });
        match picked {
            Some((face, (size, pitch))) => self.create(FeatureKind::Thread { face, size, pitch, left_hand: false }),
            None => self.notify("Select a cylindrical face (a shaft or a hole), then Thread", true),
        }
    }

    /// CREATE ▸ Coil: a spring at the origin, or standing on the selected face.
    pub fn new_coil(&mut self) {
        let (plane, offset, center, operation) = self.primitive_placement();
        self.create_on_face(FeatureKind::Coil {
            plane,
            offset,
            center,
            diameter: 20.0,
            pitch: 5.0,
            turns: 5.0,
            section: 2.0,
            left_hand: false,
            operation,
        });
    }

    /// MODIFY ▸ Delete (faces): heal the selected faces away.
    pub fn new_delete_face(&mut self) {
        let faces = self.selected_face_refs();
        self.create(FeatureKind::DeleteFace { faces });
    }

    /// MODIFY ▸ Press Pull (Q): what it does depends on the selection, as in
    /// Fusion: edges get a fillet, faces move, and otherwise it extrudes.
    pub fn press_pull(&mut self) {
        if self.viewport.selection.iter().any(|p| matches!(p, Pick::Edge(_))) {
            self.new_fillet();
        } else if self.viewport.selection.iter().any(|p| matches!(p, Pick::Face(_))) {
            self.new_offset_face();
        } else {
            self.new_extrude();
        }
    }

    pub fn new_combine(&mut self) {
        let mut picked = self.selected_bodies();
        if picked.len() < 2 {
            // Default: the last body cuts into the one before it.
            picked = self.model_bodies.iter().rev().take(2).map(|(_, o)| *o).rev().collect();
        }
        if picked.len() < 2 {
            self.notify("Combine needs two bodies", true);
            return;
        }
        let target = picked[0];
        self.create(FeatureKind::Combine { target, tools: picked[1..].to_vec(), operation: Operation::Join, keep_tools: false });
    }

    pub fn new_move(&mut self) {
        let bodies = self.selected_bodies();
        self.create(FeatureKind::Move { bodies, translate: [10.0, 0.0, 0.0], axis: Axis::Z, degrees: 0.0, copy: false });
    }

    /// ASSEMBLE ▸ Joint: the selected bodies (or the last) hinge about the
    /// latest construction axis, or X.
    pub fn new_joint(&mut self) {
        let bodies = self.selected_bodies();
        let axis = match self.doc.construction_axes().last() {
            Some(&(feature, origin, dir)) => Axis::Line { feature, origin, dir },
            None => Axis::X,
        };
        self.create(FeatureKind::Joint { bodies, motion: fission_model::JointMotion::Revolute, axis, value: 0.0, min: -90.0, max: 90.0 });
    }

    /// MODIFY ▸ Scale: the selected bodies (or the last), about the origin.
    pub fn new_scale(&mut self) {
        let bodies = self.selected_bodies();
        self.create(FeatureKind::Scale { bodies, center: [0.0; 3], factor: 2.0 });
    }

    /// MODIFY ▸ Split Body: the selected body (or the last), along a second
    /// selected planar face, or YZ.
    pub fn new_split(&mut self) {
        let body = self.selected_bodies().first().copied();
        let faces: Vec<_> = self.viewport.selection.iter().filter_map(|p| if let Pick::Face(h) = p { Some(*h) } else { None }).collect();
        let (plane, offset) = match faces.get(1) {
            Some(hit) => {
                let (plane, offset, _) = plane_through(hit.point.as_dvec3(), hit.normal.as_dvec3());
                (plane, offset)
            }
            None => (OriginPlane::YZ, 0.0),
        };
        self.create(FeatureKind::Split { body, plane, offset });
    }

    pub fn new_mirror(&mut self) {
        let bodies = self.selected_bodies();
        self.create(FeatureKind::Mirror { bodies, plane: OriginPlane::YZ, offset: 0.0, join: false });
    }

    pub fn new_pattern(&mut self, circular: bool) {
        let bodies = self.selected_bodies();
        let pattern = if circular {
            PatternKind::Circular { axis: Axis::Z, count: 6, degrees: 360.0 }
        } else {
            PatternKind::Rectangular { dir: Axis::X, count: 3, spacing: 30.0, second: None }
        };
        self.create(FeatureKind::Pattern { bodies, pattern, join: false });
    }

    /// Selected edges as stable references (for creating a fillet from a preselection).
    fn selected_edge_refs(&self) -> Vec<EdgeRef> {
        self.viewport
            .selection
            .iter()
            .filter_map(|p| match p {
                Pick::Edge(e) => self.edge_ref(e.body, e.edge),
                Pick::Face(_) => None,
            })
            .collect()
    }

    pub fn new_fillet(&mut self) {
        let edges = self.selected_edge_refs();
        self.create(FeatureKind::Fillet { radius: 1.0, edges });
    }

    pub fn new_chamfer(&mut self) {
        let edges = self.selected_edge_refs();
        self.create(FeatureKind::Chamfer { distance: 1.0, edges });
    }

    pub fn notify(&mut self, text: impl Into<String>, error: bool) {
        let text = text.into();
        if error {
            log::error!("{text}");
        } else {
            log::info!("{text}");
        }
        self.toast = Some(Toast { text, error, until: Instant::now() + Duration::from_secs(if error { 8 } else { 4 }) });
    }

    // ---- feature editing ----

    pub fn begin_edit(&mut self, index: usize) {
        if matches!(self.doc.features.get(index).map(|f| &f.kind), Some(FeatureKind::Sketch { .. })) {
            self.edit_sketch(index);
            return;
        }
        if index < self.doc.features.len() {
            self.finish_edit(true);
            self.editing = Some(Edit { index, before: self.doc.clone(), is_new: false });
            self.selected_feature = Some(index);
            self.edit_preview = false;
            self.changed();
        }
    }

    pub fn create(&mut self, kind: FeatureKind) {
        self.finish_edit(true);
        let before = self.doc.clone();
        let index = self.doc.push(kind);
        self.editing = Some(Edit { index, before, is_new: true });
        self.selected_feature = Some(index);
        self.edit_preview = false;
        self.changed();
    }

    /// Close the open dialog, keeping (OK) or discarding (Cancel) its changes.
    pub fn finish_edit(&mut self, keep: bool) {
        if let Some(edit) = self.editing.take() {
            // The viewport may have been showing the pre-feature model.
            self.changed();
            if keep {
                if self.doc != edit.before {
                    self.history.record(edit.before);
                }
            } else {
                self.doc = edit.before;
                if edit.is_new {
                    self.selected_feature = None;
                }
                self.changed();
            }
        }
    }

    pub fn new_extrude(&mut self) {
        if self.sketch.is_some() {
            self.finish_sketch();
        }
        // Prefer the newest sketch nothing uses yet, like Fusion's profile pick.
        if let Some(sk) = self.open_sketches().last().copied() {
            // Fresh from the document: finishing the sketch just now hasn't
            // regenerated yet.
            let outputs = self.doc.sketch_outputs();
            let regions = outputs.get(sk).and_then(Option::as_ref).map(|o| o.regions.clone()).unwrap_or_default();
            if !regions.is_empty() {
                let points = if regions.len() == 1 { vec![interior_point(&regions[0])] } else { Vec::new() };
                let operation = if self.model_bodies.is_empty() { Operation::NewBody } else { Operation::Join };
                self.create(FeatureKind::Extrude {
                    sketch: Sketch::Profiles { sketch: sk, points },
                    distance: 10.0,
                    symmetric: false,
                    operation,
                });
                return;
            }
        }
        // Sketch on the selected planar face when there is one, else on XY.
        let (plane, offset, outward) = self.plane_from_selection().unwrap_or((OriginPlane::XY, 0.0, 1.0));
        let operation = if self.model_bodies.is_empty() { Operation::NewBody } else { Operation::Join };
        self.create(FeatureKind::Extrude {
            sketch: Sketch::Rect { plane, offset, center: [0.0, 0.0], size: [20.0, 20.0] },
            distance: 10.0 * outward,
            symmetric: false,
            operation,
        });
    }

    /// HOLE: drilled into the selected face where it was clicked, or up
    /// through everything from the origin; click a face while the dialog is
    /// open to move it there.
    pub fn new_hole(&mut self) {
        if self.model_bodies.is_empty() {
            self.notify("Hole needs a body to cut into", true);
            return;
        }
        let hit = self.viewport.selected_face().copied();
        let (plane, offset, center, flip, through) = match hit {
            Some(hit) => {
                let (plane, offset, center, flip) = hole_placement(&hit);
                (plane, offset, center, flip, false)
            }
            None => (OriginPlane::XY, 0.0, [0.0, 0.0], true, true),
        };
        let target = hit.and_then(|h| self.bodies.get(h.body)).map(|b| b.created_by);
        self.create(FeatureKind::Hole {
            plane,
            offset,
            center,
            flip,
            diameter: 6.0,
            depth: 10.0,
            through,
            kind: HoleKind::Simple,
            tip_angle: 118.0,
        });
        if let Some(i) = self.editing.as_ref().map(|e| e.index) {
            self.doc.features[i].target = target;
            self.changed();
        }
    }

    /// While a Hole dialog is open, a face click moves the hole there.
    /// Returns whether it did.
    fn place_hole(&mut self, pick: Pick) -> bool {
        let Pick::Face(hit) = pick else { return false };
        let Some(index) = self.editing.as_ref().map(|e| e.index) else { return false };
        let owner = self.bodies.get(hit.body).map(|b| b.created_by);
        let feature = &mut self.doc.features[index];
        let FeatureKind::Hole { plane, offset, center, flip, .. } = &mut feature.kind else { return false };
        (*plane, *offset, *center, *flip) = hole_placement(&hit);
        feature.target = owner;
        self.changed();
        true
    }

    /// CONSTRUCT ▸ Offset Plane: 10 mm out from the selected planar face,
    /// or above XY.
    pub fn new_offset_plane(&mut self) {
        let (base, offset) = match self.plane_from_selection() {
            Some((plane, offset, outward)) => (plane, offset + 10.0 * outward),
            None => (OriginPlane::XY, 10.0),
        };
        self.create(FeatureKind::Plane { base, offset, angle: 0.0 });
    }

    /// CONSTRUCT ▸ Plane at Angle: hinged on the selected straight edge
    /// (starting upright through it), or on the X axis from XY; 45° to begin.
    pub fn new_plane_at_angle(&mut self) {
        let edge = self.viewport.selection.iter().find_map(|p| match p {
            Pick::Edge(e) => Some(*e),
            Pick::Face(_) => None,
        });
        let base = match edge {
            Some(e) => {
                let Some(pts) = self.bodies.get(e.body).and_then(|b| b.mesh.edges.get(e.edge as usize)) else { return };
                let (a, b) = (glam::Vec3::from(pts[0]).as_dvec3(), glam::Vec3::from(*pts.last().unwrap()).as_dvec3());
                let straight = pts.iter().all(|p| (glam::Vec3::from(*p).as_dvec3() - a).cross(b - a).length() < 1e-3 * (b - a).length_squared().max(1.0));
                if (b - a).length() < 1e-6 || !straight {
                    self.notify("Plane at Angle needs a straight edge", true);
                    return;
                }
                let d = (b - a).normalize();
                let up = if d.z.abs() < 0.9 { glam::DVec3::Z } else { glam::DVec3::Y };
                OriginPlane::Custom { origin: ((a + b) / 2.0).to_array(), normal: d.cross(up).normalize().to_array(), x_dir: d.to_array() }
            }
            None => OriginPlane::XY,
        };
        self.create(FeatureKind::Plane { base, offset: 0.0, angle: 45.0 });
    }

    /// CONSTRUCT ▸ Midplane: halfway between the two selected parallel faces.
    pub fn new_midplane(&mut self) {
        let faces: Vec<_> = self.viewport.selection.iter().filter_map(Pick::face).copied().collect();
        let [a, b] = faces.as_slice() else {
            self.notify("Select two parallel faces, then Midplane", true);
            return;
        };
        let (n, pa, pb) = (a.normal.as_dvec3(), a.point.as_dvec3(), b.point.as_dvec3());
        if n.cross(b.normal.as_dvec3()).length() > 1e-3 {
            self.notify("Midplane needs two parallel faces", true);
            return;
        }
        let mid = pa + n * ((pb - pa).dot(n) / 2.0);
        let (base, offset, _) = plane_through(mid, n);
        self.create(FeatureKind::Plane { base, offset, angle: 0.0 });
    }

    /// CONSTRUCT ▸ Axis Through Cylinder: the axis of the selected round
    /// face, or of a circular edge.
    pub fn new_axis_through_cylinder(&mut self) {
        let line = self.viewport.selection.iter().find_map(|p| match p {
            Pick::Face(h) => {
                let (pts, normals) = ui::construct::face_samples(&self.bodies.get(h.body)?.mesh, h.face);
                ui::construct::cylinder_axis(&pts, &normals)
            }
            Pick::Edge(e) => {
                ui::construct::edge_axis(self.bodies.get(e.body)?.mesh.edges.get(e.edge as usize)?)
            }
        });
        self.create_axis(line, "Select a cylindrical face or a circular edge, then Axis Through Cylinder");
    }

    /// CONSTRUCT ▸ Axis Through Edge: along the selected straight edge.
    pub fn new_axis_through_edge(&mut self) {
        let line = self.viewport.selection.iter().find_map(|p| match p {
            Pick::Edge(e) => ui::construct::edge_axis(self.bodies.get(e.body)?.mesh.edges.get(e.edge as usize)?),
            Pick::Face(_) => None,
        });
        self.create_axis(line, "Select an edge, then Axis Through Edge");
    }

    /// CONSTRUCT ▸ Axis Through Two Planes: where two selected flat faces meet.
    pub fn new_axis_through_two_planes(&mut self) {
        let faces: Vec<_> = self.viewport.selection.iter().filter_map(Pick::face).collect();
        let line = match faces.as_slice() {
            [a, b] => ui::construct::planes_axis(
                (a.point.as_dvec3(), a.normal.as_dvec3()),
                (b.point.as_dvec3(), b.normal.as_dvec3()),
            ),
            _ => None,
        };
        self.create_axis(line, "Select two faces that aren't parallel, then Axis Through Two Planes");
    }

    /// Selected edges' polylines, with where each was clicked.
    fn selected_edges(&self) -> Vec<(&[[f32; 3]], glam::DVec3)> {
        self.viewport
            .selection
            .iter()
            .filter_map(|p| match p {
                Pick::Edge(e) => Some((self.bodies.get(e.body)?.mesh.edges.get(e.edge as usize)?.as_slice(), e.point.as_dvec3())),
                Pick::Face(_) => None,
            })
            .collect()
    }

    /// Selected straight edges as lines.
    fn selected_lines(&self) -> Vec<ui::construct::Line> {
        self.selected_edges().into_iter().filter_map(|(pts, _)| ui::construct::straight_edge(pts)).collect()
    }

    /// Corners: the end of each selected edge nearest where it was clicked.
    fn selected_vertices(&self) -> Vec<glam::DVec3> {
        self.selected_edges().into_iter().filter_map(|(pts, click)| ui::construct::nearest_end(pts, click)).collect()
    }

    /// Selected faces as (clicked point, normal there).
    fn selected_face_planes(&self) -> Vec<ui::construct::Line> {
        self.viewport.selection.iter().filter_map(Pick::face).map(|h| (h.point.as_dvec3(), h.normal.as_dvec3())).collect()
    }

    fn create_plane(&mut self, plane: Option<ui::construct::Line>, hint: &str) {
        match plane {
            Some((p, n)) => {
                let (base, offset, _) = plane_through(p, n);
                self.create(FeatureKind::Plane { base, offset, angle: 0.0 });
            }
            None => self.notify(hint, true),
        }
    }

    fn create_point(&mut self, pos: Option<glam::DVec3>, hint: &str) {
        match pos {
            // `+ 0.0` keeps -0 out of the dialog.
            Some(p) => self.create(FeatureKind::Point { pos: p.to_array().map(|v| v + 0.0) }),
            None => self.notify(hint, true),
        }
    }

    /// CONSTRUCT ▸ Tangent Plane: touching the selected face where clicked.
    pub fn new_tangent_plane(&mut self) {
        let plane = self.selected_face_planes().first().copied();
        self.create_plane(plane, "Click a round face where the plane should touch it, then Tangent Plane");
    }

    pub fn new_plane_through_two_edges(&mut self) {
        let plane = match self.selected_lines().as_slice() {
            [a, b] => ui::construct::plane_through_lines(*a, *b),
            _ => None,
        };
        self.create_plane(plane, "Select two straight edges that lie in one plane, then Plane Through Two Edges");
    }

    pub fn new_plane_through_three_points(&mut self) {
        let plane = match self.selected_vertices().as_slice() {
            [a, b, c] => ui::construct::plane_through_points(*a, *b, *c),
            _ => None,
        };
        self.create_plane(plane, "Select three edges near the corners the plane goes through");
    }

    pub fn new_axis_perpendicular(&mut self) {
        let line = self.selected_face_planes().first().map(|&(p, n)| (p, n.normalize()));
        self.create_axis(line, "Click a face where the axis should go through it, then Axis Perpendicular at Point");
    }

    pub fn new_axis_through_two_points(&mut self) {
        let line = match self.selected_vertices().as_slice() {
            [a, b] if (*b - *a).length() > 1e-6 => Some(((*a + *b) / 2.0, (*b - *a).normalize())),
            _ => None,
        };
        self.create_axis(line, "Select two edges near the corners the axis goes through");
    }

    pub fn new_point_at_vertex(&mut self) {
        let pos = self.selected_vertices().first().copied();
        self.create_point(pos, "Select an edge near the corner, then Point at Vertex");
    }

    pub fn new_point_through_two_edges(&mut self) {
        let pos = match self.selected_lines().as_slice() {
            [a, b] => ui::construct::lines_meet(*a, *b),
            _ => None,
        };
        self.create_point(pos, "Select two straight edges whose lines cross");
    }

    pub fn new_point_through_three_planes(&mut self) {
        let pos = match self.selected_face_planes().as_slice() {
            [a, b, c] => ui::construct::three_planes_meet(*a, *b, *c),
            _ => None,
        };
        self.create_point(pos, "Select three flat faces whose planes meet in a point");
    }

    pub fn new_point_at_center(&mut self) {
        let pos = self.selected_edges().into_iter().find_map(|(pts, _)| ui::construct::circle_axis(pts)).map(|(c, _)| c);
        self.create_point(pos, "Select a circular edge, then Point at Center");
    }

    pub fn new_point_at_edge_and_plane(&mut self) {
        let (lines, planes) = (self.selected_lines(), self.selected_face_planes());
        let pos = match (lines.as_slice(), planes.as_slice()) {
            ([l], [p]) => ui::construct::line_meets_plane(*l, *p),
            _ => None,
        };
        self.create_point(pos, "Select a straight edge and a flat face, then Point at Edge and Plane");
    }

    fn create_axis(&mut self, line: Option<ui::construct::Line>, hint: &str) {
        match line {
            Some((origin, dir)) => self.create(FeatureKind::Axis { origin: origin.to_array(), dir: dir.to_array() }),
            None => self.notify(hint, true),
        }
    }

    /// Appearance for the selected bodies (or the last body).
    pub fn open_appearance(&mut self) {
        let mut targets: Vec<usize> =
            self.viewport.selection.iter().filter_map(|p| self.bodies.get(p.body()).map(|b| b.created_by)).collect();
        targets.sort_unstable();
        targets.dedup();
        self.appearance = Some(ui::appearance::AppearanceDialog { targets, custom: [0.6, 0.6, 0.65] });
    }

    pub fn toggle_section(&mut self) {
        self.section = match self.section {
            Some(_) => None,
            None => Some(ui::section::Section { plane: OriginPlane::XZ, offset: 0.0, flip: false }),
        };
    }

    pub fn open_parameters(&mut self) {
        self.finish_edit(true);
        self.params_dialog = Some(ui::params::ParamsDialog { before: self.doc.clone() });
    }

    pub fn start_measure(&mut self) {
        self.finish_edit(true);
        self.measure = Some(ui::measure::Measure::default());
        self.notify("Measure: click faces or edges", false);
    }

    pub fn new_box(&mut self) {
        self.create(FeatureKind::Extrude {
            sketch: Sketch::Rect { plane: OriginPlane::XY, offset: 0.0, center: [0.0, 0.0], size: [20.0, 20.0] },
            distance: 20.0,
            symmetric: false,
            operation: Operation::NewBody,
        });
    }

    pub fn new_cylinder(&mut self) {
        self.create(FeatureKind::Extrude {
            sketch: Sketch::Circle { plane: OriginPlane::XY, offset: 0.0, center: [0.0, 0.0], radius: 10.0 },
            distance: 20.0,
            symmetric: false,
            operation: Operation::NewBody,
        });
    }

    /// Where a primitive goes: on the selected face at the click, or the
    /// origin on XY. Joins what it touches when placed on a face.
    fn primitive_placement(&self) -> (OriginPlane, f64, [f64; 2], Operation) {
        match self.viewport.selected_face() {
            Some(hit) => {
                let (plane, offset, center, _) = hole_placement(hit);
                (plane, offset, center, Operation::Join)
            }
            None => (OriginPlane::XY, 0.0, [0.0, 0.0], Operation::NewBody),
        }
    }

    pub fn new_sphere(&mut self) {
        let (plane, offset, center, operation) = self.primitive_placement();
        self.create_on_face(FeatureKind::Sphere { plane, offset, center, diameter: 20.0, operation });
    }

    pub fn new_torus(&mut self) {
        let (plane, offset, center, operation) = self.primitive_placement();
        self.create_on_face(FeatureKind::Torus { plane, offset, center, diameter: 40.0, section: 10.0, operation });
    }

    /// Create a feature that joins the body of the selected face.
    fn create_on_face(&mut self, kind: FeatureKind) {
        let owner = self.viewport.selected_face().and_then(|h| self.bodies.get(h.body)).map(|b| b.created_by);
        self.create(kind);
        if let (Some(owner), Some(index)) = (owner, self.editing.as_ref().map(|e| e.index)) {
            self.doc.features[index].target = Some(owner);
            self.changed();
        }
    }

    pub fn new_revolve(&mut self) {
        if self.sketch.is_some() {
            self.finish_sketch();
        }
        let operation = if self.model_bodies.is_empty() { Operation::NewBody } else { Operation::Join };
        // Like extrude: the newest unused sketch's profile, about one of its
        // lines (a construction line when there is one).
        if let Some(sk) = self.open_sketches().last().copied() {
            let outputs = self.doc.sketch_outputs();
            let regions = outputs.get(sk).and_then(Option::as_ref).map(|o| o.regions.clone()).unwrap_or_default();
            if !regions.is_empty() {
                let points = if regions.len() == 1 { vec![interior_point(&regions[0])] } else { Vec::new() };
                let axis = self.default_revolve_axis(sk);
                self.create(FeatureKind::Revolve {
                    sketch: Sketch::Profiles { sketch: sk, points },
                    axis,
                    degrees: 360.0,
                    operation,
                });
                return;
            }
        }
        self.create(FeatureKind::Revolve {
            sketch: Sketch::Rect { plane: OriginPlane::XZ, offset: 0.0, center: [20.0, 10.0], size: [8.0, 20.0] },
            axis: fission_model::Axis::Z,
            degrees: 360.0,
            operation,
        });
    }

    /// A construction line in the sketch, else an origin axis in its plane.
    fn default_revolve_axis(&self, sk: usize) -> fission_model::Axis {
        use fission_model::{sketch2d::Curve, Axis};
        let Some(FeatureKind::Sketch { plane, sketch, .. }) = self.doc.features.get(sk).map(|f| &f.kind) else {
            return Axis::Z;
        };
        let construction =
            sketch.curves.iter().position(|c| matches!(c, Curve::Line { construction: true, .. }));
        if let Some(curve) = construction {
            return Axis::SketchLine { sketch: sk, curve };
        }
        match plane {
            OriginPlane::XY => Axis::Y,
            OriginPlane::XZ | OriginPlane::YZ | OriginPlane::Custom { .. } => Axis::Z,
        }
    }

    /// If the selected face is planar and parallel to an origin plane, return
    /// that plane, its offset, and +1/-1 for whether the face points along the
    /// plane normal (so new features can go outward or inward).
    pub(crate) fn plane_from_selection(&self) -> Option<(OriginPlane, f64, f64)> {
        let hit = *self.viewport.selected_face()?;
        Some(plane_through(hit.point.as_dvec3(), hit.normal.as_dvec3()))
    }

    pub fn delete_feature(&mut self, index: usize) {
        self.finish_edit(true);
        self.commit(|doc| doc.remove(index));
        self.selected_feature = None;
    }

    // ---- files ----

    pub fn new_document(&mut self) {
        self.finish_edit(true);
        let before = std::mem::take(&mut self.doc);
        self.history.record(before);
        self.saved = self.doc.clone();
        self.path = None;
        self.selected_feature = None;
        self.first_regen = true;
        self.changed();
    }

    pub fn open_dialog(&mut self) {
        let file = rfd::FileDialog::new()
            .add_filter("Designs", &["fission", "f3d", "step", "stp"])
            .add_filter("Fission design", &["fission"])
            .add_filter("Fusion archive", &["f3d"])
            .add_filter("STEP", &["step", "stp"])
            .pick_file();
        if let Some(path) = file {
            self.open_path(&path);
        }
    }

    pub fn open_path(&mut self, path: &Path) {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if ext == "f3d" && std::env::var_os("FISSION_F3D_BREP").is_none() {
            // Fusion's timeline, replayed as Fission features.
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Imported");
            let name = stem.trim_start_matches('_').split('.').next().unwrap_or(stem).to_string();
            // Converted here, with rebuilds through the crash-proof worker.
            let converted = fission_f3d::read_timeline(path).map_err(|e| e.to_string()).map(|t| {
                let kernel = &mut self.kernel;
                let mut rebuild = |doc: &Document| -> Vec<(usize, Mesh)> {
                    match kernel.regenerate(doc, None) {
                        Ok(r) => r.bodies.into_iter().map(|b| (b.created_by, b.mesh)).collect(),
                        Err(_) => Vec::new(),
                    }
                };
                let import = fission_model::f3d::document_with(&t, &name, &mut rebuild);
                (import.doc, import.skipped, import.notes)
            });
            match converted {
                Ok((doc, skipped, notes)) if !doc.features.is_empty() => {
                    for n in &notes {
                        log::info!("f3d import: {n}");
                    }
                    self.finish_edit(false);
                    self.history = History::default();
                    self.doc = doc;
                    self.saved = self.doc.clone();
                    self.path = None;
                    self.selected_feature = None;
                    self.first_regen = true;
                    self.changed();
                    let total = self.doc.features.len();
                    if skipped.is_empty() {
                        self.notify(format!("Opened {name}: all {total} timeline features"), false);
                    } else {
                        self.notify(format!("Opened {name}: {} of {total} features replay; not yet: {}", total - skipped.len(), skipped.join(", ")), false);
                    }
                    return;
                }
                Ok(_) => {}
                Err(e) => log::warn!("f3d timeline: {e}; importing its bodies instead"),
            }
        }
        if matches!(ext.as_str(), "f3d" | "step" | "stp") {
            // Someone else's format: a new design holding it as an import.
            self.new_document();
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Imported");
            self.doc.name = stem.trim_start_matches('_').split('.').next().unwrap_or(stem).to_string();
            self.saved = self.doc.clone();
            self.import_step_path(path);
            self.first_regen = true;
            return;
        }
        let result = std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|text| Document::from_ron(&text).map_err(|e| e.to_string()));
        match result {
            Ok(doc) => {
                self.finish_edit(false);
                self.history = History::default();
                self.saved = doc.clone();
                self.doc = doc;
                self.path = Some(path.to_path_buf());
                remember_last_file(path);
                self.selected_feature = None;
                self.first_regen = true;
                self.changed();
                self.notify(format!("Opened {}", path.display()), false);
            }
            Err(e) => self.notify(format!("Couldn't open {}: {e}", path.display()), true),
        }
    }

    pub fn save(&mut self, save_as: bool) {
        self.finish_edit(true);
        let path = match (&self.path, save_as) {
            (Some(path), false) => Some(path.clone()),
            _ => rfd::FileDialog::new()
                .add_filter("Fission design", &["fission"])
                .set_file_name(format!("{}.fission", self.doc.name))
                .save_file(),
        };
        let Some(path) = path else { return };
        let result = self.doc.to_ron().map_err(|e| e.to_string()).and_then(|text| {
            std::fs::write(&path, text).map_err(|e| e.to_string())
        });
        match result {
            Ok(()) => {
                self.saved = self.doc.clone();
                self.notify(format!("Saved {}", path.display()), false);
                remember_last_file(&path);
                self.path = Some(path);
            }
            Err(e) => self.notify(format!("Couldn't save: {e}"), true),
        }
    }

    /// Add a STEP or Fusion .f3d file to the timeline as an Import feature.
    pub fn import_step_dialog(&mut self) {
        let file = rfd::FileDialog::new()
            .add_filter("CAD files", &["step", "stp", "f3d"])
            .add_filter("STEP", &["step", "stp"])
            .add_filter("Fusion 360 archive", &["f3d"])
            .pick_file();
        if let Some(path) = file {
            self.import_step_path(&path);
        }
    }

    /// The import part a picked face belongs to, as (feature, part).
    pub fn part_of_face(&self, body: usize, face: u32) -> Option<(usize, usize)> {
        let feature = self.bodies.get(body)?.created_by;
        let mut first = 0usize;
        for &(part, faces) in self.import_part_faces.get(feature)? {
            if (face as usize) < first + faces {
                return Some((feature, part));
            }
            first += faces;
        }
        None
    }

    /// V, as in Fusion: hide what's selected. A face on an imported body
    /// hides just its part; anything else hides the whole body.
    pub fn hide_selection(&mut self) {
        let picks = std::mem::take(&mut self.viewport.selection);
        let mut parts: Vec<(usize, usize)> = Vec::new();
        let mut bodies = Vec::new();
        for pick in &picks {
            let Some(body) = self.bodies.get(pick.body()) else { continue };
            let has_parts = self.import_part_faces.get(body.created_by).is_some_and(|p| p.len() > 1);
            match pick.face().and_then(|h| self.part_of_face(h.body, h.face)) {
                Some(fp) if has_parts => parts.push(fp),
                // An edge on a multi-part import: leave the body alone.
                _ if has_parts => {}
                _ => bodies.push(body.created_by),
            }
        }
        if !parts.is_empty() {
            self.commit(|doc| {
                for (feature, part) in parts {
                    if let Some(FeatureKind::ImportStep { hidden_parts, .. }) = doc.features.get_mut(feature).map(|f| &mut f.kind) {
                        if !hidden_parts.contains(&part) {
                            hidden_parts.push(part);
                            hidden_parts.sort_unstable();
                        }
                    }
                }
            });
        }
        for b in bodies {
            self.set_body_visible(b, false);
        }
    }

    /// Show or hide one part of an import (it leaves or rejoins the body).
    pub fn set_import_part_visible(&mut self, feature: usize, part: usize, visible: bool) {
        self.commit(|doc| {
            if let Some(FeatureKind::ImportStep { hidden_parts, .. }) = doc.features.get_mut(feature).map(|f| &mut f.kind) {
                hidden_parts.retain(|&p| p != part);
                if !visible {
                    hidden_parts.push(part);
                    hidden_parts.sort_unstable();
                }
            }
        });
    }

    pub fn import_step_path(&mut self, path: &Path) {
        let operation = Operation::NewBody;
        self.finish_edit(true);
        self.commit(|doc| {
            doc.push(FeatureKind::ImportStep { path: path.display().to_string(), operation, hidden_parts: Vec::new() });
        });
    }

    pub fn export_dialog(&mut self, extension: &str) {
        let file = rfd::FileDialog::new()
            .add_filter(extension.to_uppercase(), &[extension])
            .set_file_name(format!("{}.{extension}", self.doc.name))
            .save_file();
        if let Some(path) = file {
            let hidden: Vec<usize> = self.hidden.iter().copied().collect();
            match self.kernel.export(&self.doc, &hidden, &path) {
                Ok(()) => self.notify(format!("Exported {}", path.display()), false),
                Err(e) => self.notify(format!("Export failed: {e}"), true),
            }
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let cmd = |key| KeyboardShortcut::new(Modifiers::COMMAND, key);
        let cmd_shift = |key| KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, key);
        let plain = |key| KeyboardShortcut::new(Modifiers::NONE, key);
        let typing = ctx.egui_wants_keyboard_input();

        let pressed = |s: KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(&s));
        if self.sketch.is_some() && (pressed(cmd_shift(Key::Z)) || pressed(cmd(Key::Y))) {
            self.sketch_undo(true);
        } else if self.sketch.is_some() && pressed(cmd(Key::Z)) {
            self.sketch_undo(false);
        } else if pressed(cmd_shift(Key::Z)) || pressed(cmd(Key::Y)) {
            self.finish_edit(true);
            let before = self.doc.clone();
            if self.history.redo(&mut self.doc) {
                self.changed_from(&before);
            }
        } else if pressed(cmd(Key::Z)) {
            self.finish_edit(true);
            let before = self.doc.clone();
            if self.history.undo(&mut self.doc) {
                self.changed_from(&before);
            }
        }
        if pressed(cmd_shift(Key::S)) {
            self.save(true);
        } else if pressed(cmd(Key::S)) {
            self.save(false);
        }
        if pressed(cmd(Key::O)) {
            self.open_dialog();
        }
        if pressed(cmd(Key::N)) {
            self.new_document();
        }
        if typing {
            return;
        }
        // Typing while a command is active opens the value box. In a feature
        // dialog anything typed goes there (so parameter names work, and
        // letters don't fire shortcuts); while drawing, a number starts it.
        if self.hud.is_none() && ui::hud::wants_typed_value(self) {
            let in_dialog = self.editing.is_some() && self.sketch.is_none();
            let starts = |c: char| in_dialog && (c.is_ascii_alphanumeric() || c == '_') || c.is_ascii_digit() || ".-+(,".contains(c);
            let started = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Text(t) if t.starts_with(starts) => Some(t.clone()),
                    _ => None,
                })
            });
            if let Some(text) = started {
                // The key that started typing isn't also a shortcut.
                ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Key { pressed: true, .. })));
                self.hud = Some(ui::hud::Hud::new(text));
                return;
            }
        }
        if self.sketch.is_some() {
            use ui::sketch::Tool;
            for (key, tool) in [(Key::L, Tool::Line), (Key::R, Tool::Rectangle), (Key::C, Tool::Circle), (Key::D, Tool::Dimension), (Key::T, Tool::Trim), (Key::O, Tool::Offset), (Key::P, Tool::Project)] {
                if pressed(plain(key)) {
                    self.set_sketch_tool(tool);
                }
            }
            if pressed(plain(Key::X)) {
                self.toggle_construction();
            }
            // Mirror: Enter moves from picking curves to picking the line.
            if pressed(plain(Key::Enter)) {
                let s = self.sketch.as_mut().unwrap();
                if s.tool == Tool::Mirror && s.selection.iter().any(|i| matches!(i, ui::sketch::Item::Curve(_))) {
                    s.mirror_pick_line = true;
                }
            }
            if pressed(plain(Key::Delete)) {
                self.delete_sketch_selection();
            }
            if pressed(plain(Key::Escape)) {
                // Esc backs out one step: pending points, then the tool.
                let s = self.sketch.as_mut().unwrap();
                if s.editing_dim.is_some() {
                    s.editing_dim = None;
                } else if s.lone_start {
                    self.drop_lone_start();
                } else if !s.pending.is_empty() {
                    s.pending.clear();
                } else if s.tool != Tool::Select {
                    s.tool = Tool::Select;
                } else {
                    s.selection.clear();
                }
            }
            if pressed(KeyboardShortcut::new(Modifiers::COMMAND, Key::Enter)) {
                self.finish_sketch();
            }
            if pressed(plain(Key::E)) {
                self.new_extrude();
            }
            return;
        }
        if self.picking_sketch_plane && pressed(plain(Key::Escape)) {
            self.picking_sketch_plane = false;
        }
        // Fusion's single-key shortcuts.
        if pressed(plain(Key::E)) {
            self.new_extrude();
        }
        if pressed(plain(Key::Q)) {
            self.press_pull();
        }
        if pressed(plain(Key::H)) {
            self.new_hole();
        }
        if pressed(plain(Key::F)) {
            self.new_fillet();
        }
        if pressed(plain(Key::A)) {
            self.open_appearance();
        }
        if pressed(plain(Key::V)) && !self.viewport.selection.is_empty() {
            self.hide_selection();
        }
        if self.measure.is_some() && pressed(plain(Key::Escape)) {
            self.measure = None;
        }
        if pressed(plain(Key::Escape)) {
            if self.editing.is_some() {
                self.finish_edit(false);
            } else {
                self.viewport.selection.clear();
                self.selected_feature = None;
            }
        }
        if pressed(plain(Key::Enter)) && self.editing.is_some() {
            self.finish_edit(true);
        }
        if pressed(plain(Key::Delete)) {
            if let Some(index) = self.selected_feature {
                self.delete_feature(index);
            }
        }
        if pressed(plain(Key::F6)) {
            self.viewport.fit_all(&self.bodies);
        }
    }

    fn update_title(&self, ctx: &egui::Context) {
        let title = format!("{}{} — Fission", self.doc.name, if self.is_dirty() { " •" } else { "" });
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
    }
}

impl eframe::App for FissionApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.shortcuts(&ctx);
        if self.needs_regen {
            self.regenerate();
        }
        if self.tick_joint_play() {
            ctx.request_repaint();
        }
        self.update_title(&ctx);

        ui::toolbar::show(self, ui);
        ui::timeline::show(self, ui);
        ui::browser::show(self, ui);

        let rs = self.render_state.clone();
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            let rect = ui.max_rect();
            let extra = self.highlights();
            // The grid lies on the open sketch's plane, at its snap spacing.
            self.viewport.grid = match self.sketch.as_ref().and_then(|s| self.sketch_plane(s.feature).map(|p| (p, s.grid_snap))) {
                Some((plane, snapping)) => {
                    let wpp = self.viewport.world_per_point() as f64;
                    (plane, if snapping { ui::sketch::grid_step(wpp * 12.0) } else { 10.0 })
                }
                None => (fission_kernel::Plane::XY, 10.0),
            };
            self.viewport.body_colors = self
                .bodies
                .iter()
                .map(|b| crate::viewport::appearance(self.doc.features.get(b.created_by).and_then(|f| f.color)))
                .collect();
            let events = self.viewport.show(ui, &rs, &self.bodies, &extra);

            // Sketches over the model: the active one with tools, otherwise
            // the open (unused) ones, and the one an extrude is picking from.
            if let Some(active) = self.sketch.as_ref().map(|s| s.feature) {
                ui::sketch::handle_input(self, &events);
                ui::sketch::draw(self, ui, active, true, &[]);
            } else {
                let picking = self.editing_profiles();
                let loft_sweep = self.editing_loft_sweep();
                let shown: Vec<usize> = loft_sweep.iter().flat_map(|(s, _)| s.iter().map(|(sk, _)| *sk)).collect();
                for sk in self.open_sketches() {
                    if picking.as_ref().is_none_or(|(p, _)| *p != sk) && !shown.contains(&sk) {
                        ui::sketch::draw(self, ui, sk, false, &[]);
                    }
                }
                if let Some((sk, points)) = &picking {
                    ui::sketch::draw(self, ui, *sk, false, points);
                }
                if let Some((sketches, path)) = &loft_sweep {
                    for (sk, points) in sketches {
                        ui::sketch::draw(self, ui, *sk, false, points);
                    }
                    if let Some(p) = path {
                        self.draw_sweep_path(ui, *p);
                    }
                }
                if !self.picking_sketch_plane {
                    ui::sketch::draw_construction_planes(self, ui);
                    ui::construct::draw_construction_axes(self, ui);
                    ui::construct::draw_construction_points(self, ui);
                }
                if self.picking_sketch_plane {
                    ui::sketch::draw_plane_picker(self, ui, events.primary_clicked);
                } else if events.primary_clicked && picking.is_some() && self.pick_profile() {
                    // Click went to profile selection.
                } else if events.primary_clicked && loft_sweep.is_some() && self.pick_loft_sweep() {
                    // Click picked a loft profile or the sweep's path.
                } else if self.rigid_gizmo(ui, &events) {
                    // The pointer is on the Move/Joint gizmo.
                } else if let Some((pick, toggle)) = events.clicked {
                    self.handle_click(pick, toggle);
                } else if events.clicked_empty && self.editing.is_none() {
                    self.viewport.selection.clear();
                }
            }
            ui::dialog::show(self, &ctx, rect);
            ui::measure::show(self, &ctx, rect);
            ui::params::show(self, &ctx);
            ui::section::show(self, &ctx, rect);
            ui::appearance::show(self, &ctx);
            ui::hud::show(self, &ctx);
            ui::overlay(self, ui, rect);
        });

        if self.toast.as_ref().is_some_and(|t| Instant::now() > t.until) {
            self.toast = None;
        }
    }
}

/// Where the last opened or saved design's path is kept.
fn last_file_record() -> Option<PathBuf> {
    let state = match std::env::var_os("XDG_STATE_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".local/state"),
    };
    Some(state.join("fission").join("last-file"))
}

fn remember_last_file(path: &Path) {
    let Some(record) = last_file_record() else { return };
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let written = record.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&record, path.to_string_lossy().as_bytes()));
    if let Err(e) = written {
        log::warn!("can't remember the last file in {}: {e}", record.display());
    }
}

/// The design to reopen on a launch without one, if it still exists.
pub fn last_file() -> Option<PathBuf> {
    let path = PathBuf::from(std::fs::read_to_string(last_file_record()?).ok()?.trim());
    path.is_file().then_some(path)
}

fn demo_point(arg: &str) -> Option<[f64; 2]> {
    let v: Vec<f64> = arg.split(';').filter_map(|x| x.trim().parse().ok()).collect();
    (v.len() == 2).then(|| [v[0], v[1]])
}

/// Export visible bodies by file extension. Shared by the GUI and the CLI.
pub fn export(kernel: &Occt, bodies: &[(String, Solid)], path: &Path) -> Result<(), String> {
    if bodies.is_empty() {
        return Err("nothing to export".into());
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let meshes = || -> Result<Vec<(String, Mesh)>, String> {
        bodies
            .iter()
            .map(|(name, solid)| kernel.tessellate(solid, EXPORT_TOLERANCE).map(|m| (name.clone(), m)).map_err(|e| e.to_string()))
            .collect()
    };
    match ext.as_str() {
        "3mf" => crate::export::write_3mf(path, &meshes()?),
        "obj" => crate::export::write_obj(path, &meshes()?),
        "stl" => {
            let mut combined = Mesh::default();
            for (_, solid) in bodies {
                let mesh = kernel.tessellate(solid, EXPORT_TOLERANCE).map_err(|e| e.to_string())?;
                let base = combined.positions.len() as u32;
                combined.positions.extend(mesh.positions);
                combined.indices.extend(mesh.indices.iter().map(|i| base + i));
            }
            let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
            combined.write_stl(std::io::BufWriter::new(file)).map_err(|e| e.to_string())
        }
        "step" | "stp" => {
            let chosen: Vec<Solid> = bodies.iter().map(|(_, s)| s.clone()).collect();
            kernel.export_step(&chosen, path).map_err(|e| e.to_string())
        }
        other => Err(format!("unsupported export format '.{other}' (use .stl, .3mf, .obj or .step)")),
    }
}

/// Where a hole clicked at `hit` goes: the face's plane and offset, the
/// click in plane coordinates, and whether drilling runs along the normal.
/// The ISO size (nominal, pitch) for a cylindrical face: its diameter is the
/// nominal size on a shaft, about the tap drill in a hole.
fn thread_fit(mesh: &fission_kernel::Mesh, face: u32) -> Option<(f64, f64)> {
    let (pts, normals) = ui::construct::face_samples(mesh, face);
    let (origin, dir) = ui::construct::cylinder_axis(&pts, &normals)?;
    let radial = |p: glam::DVec3| (p - origin).reject_from(dir);
    let r = pts.iter().map(|p| radial(*p).length()).sum::<f64>() / pts.len() as f64;
    // A hole's normals point at the axis.
    let internal = pts.iter().zip(&normals).map(|(p, n)| radial(*p).dot(*n)).sum::<f64>() < 0.0;
    Some(fission_model::thread::iso_size_for(2.0 * r, internal))
}

fn hole_placement(hit: &crate::viewport::Hit) -> (OriginPlane, f64, [f64; 2], bool) {
    let (plane, offset, outward) = plane_through(hit.point.as_dvec3(), hit.normal.as_dvec3());
    let p = plane.plane(offset);
    let d = hit.point.as_dvec3() - glam::DVec3::from_array(p.origin);
    let center = [d.dot(glam::DVec3::from_array(p.x_dir)), d.dot(glam::DVec3::from_array(p.y_dir()))];
    (plane, offset, center, outward < 0.0)
}

/// The plane through `p` with normal `n`: an origin plane and offset when
/// `n` is along an axis, else its own plane with X as level as possible.
/// The third value is +1 or -1: whether `n` points along the plane's normal.
pub(crate) fn plane_through(p: glam::DVec3, n: glam::DVec3) -> (OriginPlane, f64, f64) {
    let aligned = |v: f64| v.abs() > 0.999;
    let (plane, offset, along) = if aligned(n.z) {
        (OriginPlane::XY, p.z, n.z)
    } else if aligned(n.y) {
        // XZ's normal is -Y.
        (OriginPlane::XZ, -p.y, -n.y)
    } else if aligned(n.x) {
        (OriginPlane::YZ, p.x, n.x)
    } else {
        // A slanted face: its own plane, with sketch X as level as possible.
        let level = glam::DVec3::Z.cross(n);
        let x = if level.length() > 1e-6 { level.normalize() } else { glam::DVec3::X };
        return (OriginPlane::Custom { origin: p.to_array(), normal: n.to_array(), x_dir: x.to_array() }, 0.0, 1.0);
    };
    // `+ 0.0` turns -0 (from XZ's flipped normal) into 0 for display.
    (plane, offset + 0.0, along.signum())
}

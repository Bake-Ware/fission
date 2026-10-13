//! Joints in the editor: scrubbing a joint moves its bodies' display meshes
//! rigidly (no regeneration), and the value is committed on release.

use std::time::Instant;

use fission_kernel::Mesh;
use fission_model::{JointInfo, Rigid};

use crate::app::FissionApp;

/// A joint being dragged or played, shown without regenerating.
pub struct JointPreview {
    pub feature: usize,
    pub value: f64,
    /// The moved bodies' meshes at the joint's built value, by display index.
    base: Vec<(usize, Mesh)>,
    /// Playing back and forth through the limits since this instant.
    pub playing: Option<Instant>,
}

/// `mesh` after a rigid motion.
pub fn moved_mesh(mesh: &Mesh, t: &Rigid) -> Mesh {
    let point = |p: [f32; 3]| t.point(p.map(f64::from)).map(|x| x as f32);
    let mut out = mesh.clone();
    out.positions.iter_mut().for_each(|p| *p = point(*p));
    out.normals.iter_mut().for_each(|n| *n = t.turn(n.map(f64::from)).map(|x| x as f32));
    out.edges.iter_mut().flatten().for_each(|p| *p = point(*p));
    for h in &mut out.face_hints {
        h.center = t.point(h.center);
        h.normal = t.turn(h.normal);
    }
    out
}

/// Seconds to sweep once from one limit to the other.
const SWEEP_SECONDS: f64 = 2.5;

impl FissionApp {
    pub fn joint_info(&self, feature: usize) -> Option<JointInfo> {
        self.doc.joints().into_iter().find(|j| j.feature == feature)
    }

    /// The motion from a joint's built value to `value`.
    fn joint_motion(&self, joint: &JointInfo, value: f64) -> Option<Rigid> {
        let (origin, dir) = joint.axis.resolve(&self.regen_sketches).ok()?;
        Some(Rigid::joint(joint.motion, origin, dir, value - joint.value))
    }

    /// Show joint `feature` at `value` (clamped to its limits).
    pub fn preview_joint(&mut self, feature: usize, value: f64) {
        if self.joint_preview.as_ref().is_some_and(|p| p.feature != feature) {
            self.end_joint_preview();
        }
        let Some(joint) = self.joint_info(feature) else { return };
        let value = joint.clamp(value);
        let Some(motion) = self.joint_motion(&joint, value) else { return };
        let preview = self.joint_preview.get_or_insert_with(|| JointPreview {
            feature,
            value,
            base: self
                .bodies
                .iter()
                .enumerate()
                .filter(|(_, b)| joint.bodies.contains(&b.created_by))
                .map(|(i, b)| (i, b.mesh.clone()))
                .collect(),
            playing: None,
        });
        preview.value = value;
        for (i, mesh) in &preview.base {
            if let Some(body) = self.bodies.get_mut(*i) {
                body.mesh = moved_mesh(mesh, &motion);
            }
        }
        self.viewport.bodies_changed(&self.bodies);
    }

    /// Put the bodies back where the model has them.
    pub fn end_joint_preview(&mut self) {
        if let Some(preview) = self.joint_preview.take() {
            for (i, mesh) in preview.base {
                if let Some(body) = self.bodies.get_mut(i) {
                    body.mesh = mesh;
                }
            }
            self.viewport.bodies_changed(&self.bodies);
        }
    }

    /// Keep the previewed value: the joint (or the parameter driving it)
    /// takes it, as one undoable step.
    pub fn commit_joint_preview(&mut self) {
        let Some(preview) = &self.joint_preview else { return };
        let (feature, value) = (preview.feature, preview.value);
        let built = self.joint_info(feature).map(|j| j.value);
        if built == Some(value) {
            self.end_joint_preview();
            return;
        }
        // The preview's meshes already show the new value; regeneration replaces them.
        self.joint_preview = None;
        self.commit(|doc| doc.set_joint_value(feature, value));
    }

    pub fn toggle_joint_play(&mut self, feature: usize) {
        if self.joint_preview.as_ref().is_some_and(|p| p.feature == feature && p.playing.is_some()) {
            self.end_joint_preview();
            return;
        }
        let Some(joint) = self.joint_info(feature) else { return };
        self.preview_joint(feature, joint.value);
        if let Some(p) = &mut self.joint_preview {
            p.playing = Some(Instant::now());
        }
    }

    /// Advance a playing joint; true while one plays.
    pub fn tick_joint_play(&mut self) -> bool {
        let Some((feature, start)) = self.joint_preview.as_ref().and_then(|p| Some((p.feature, p.playing?))) else {
            return false;
        };
        let Some(joint) = self.joint_info(feature) else {
            self.end_joint_preview();
            return false;
        };
        let value = pingpong(&joint, start.elapsed().as_secs_f64() / SWEEP_SECONDS);
        self.preview_joint(feature, value);
        if let Some(p) = &mut self.joint_preview {
            p.playing = Some(start);
        }
        true
    }

    /// INSPECT ▸ Interference: sweep every joint through its limits.
    pub fn check_interference(&mut self) {
        let joints = self.doc.joints();
        if joints.is_empty() {
            self.notify("No joints to check: add one with ASSEMBLE ▸ Joint", true);
            return;
        }
        let mut lines = Vec::new();
        let mut collided = false;
        for joint in &joints {
            let values = fission_model::motion::steps(joint.min, joint.max, 37);
            match self.kernel.sweep(&self.doc, &joint.name, &values, INTERFERENCE_TOLERANCE) {
                Ok(sweep) => {
                    let worst = sweep.samples.iter().max_by(|a, b| a.overlap.total_cmp(&b.overlap));
                    match worst.filter(|s| s.overlap > INTERFERENCE_TOLERANCE) {
                        None => lines.push(format!("{}: clear through {}..{}", joint.name, joint.min, joint.max)),
                        Some(s) => {
                            collided = true;
                            let (a, b, _) = &s.pairs[0];
                            let clear = sweep.clear.map_or("collides where it is".to_string(), |(lo, hi)| format!("clear {lo:.1}..{hi:.1}"));
                            lines.push(format!("{}: {clear}; {a} hits {b} ({:.1} mm³ at {:.1})", joint.name, s.overlap, s.value));
                        }
                    }
                }
                Err(e) => {
                    collided = true;
                    lines.push(format!("{}: {e}", joint.name));
                }
            }
        }
        self.notify(lines.join("\n"), collided);
    }
}

/// Overlaps up to this many mm³ are faces touching, not a collision.
pub const INTERFERENCE_TOLERANCE: f64 = 0.01;

/// Back and forth between the limits: `t` sweeps per unit, starting at the
/// built value heading toward the farther limit.
pub fn pingpong(joint: &JointInfo, t: f64) -> f64 {
    let (lo, hi) = (joint.min.min(joint.max), joint.max.max(joint.min));
    let span = hi - lo;
    if span <= 0.0 {
        return lo;
    }
    // Phase 0..2 over one round trip, offset so phase 0 is the built value.
    let start = (joint.value - lo) / span;
    let phase = (start + t).rem_euclid(2.0);
    lo + span * if phase <= 1.0 { phase } else { 2.0 - phase }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fission_model::{Axis, JointMotion};

    #[test]
    fn pingpong_starts_at_the_built_value_and_turns_at_the_limits() {
        let j = JointInfo { feature: 0, name: "J".into(), motion: JointMotion::Revolute, bodies: vec![], axis: Axis::X, value: 0.0, min: -180.0, max: 10.0 };
        assert!((pingpong(&j, 0.0) - 0.0).abs() < 1e-9);
        let near_top = pingpong(&j, 10.0 / 190.0);
        assert!((near_top - 10.0).abs() < 1e-9, "{near_top}");
        assert!((pingpong(&j, 10.0 / 190.0 + 1.0) + 180.0).abs() < 1e-9);
    }

    #[test]
    fn moved_mesh_turns_normals_and_edges() {
        let mesh = Mesh {
            positions: vec![[0.0, 0.0, 10.0]],
            normals: vec![[0.0, 0.0, 1.0]],
            edges: vec![vec![[0.0, 0.0, 10.0]]],
            ..Default::default()
        };
        let t = Rigid::joint(JointMotion::Revolute, [0.0; 3], [1.0, 0.0, 0.0], 90.0);
        let m = moved_mesh(&mesh, &t);
        assert!((m.positions[0][1] + 10.0).abs() < 1e-5 && m.positions[0][2].abs() < 1e-5);
        assert!((m.normals[0][1] + 1.0).abs() < 1e-5);
        assert_eq!(m.edges[0][0], m.positions[0]);
    }
}

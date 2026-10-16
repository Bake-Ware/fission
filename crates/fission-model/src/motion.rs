//! Sweeping a joint through its range: where it collides with the rest of
//! the design.
//!
//! The design regenerates once, at the joint's built value; each sample then
//! moves the joint's bodies rigidly and intersects them with every other
//! body. No per-sample regeneration, so a sweep costs one boolean per
//! overlapping pair and sample.

use fission_kernel::{BooleanOp, Kernel};

use crate::{Document, FeatureStatus, JointInfo, JointMotion, Regen, Rigid};

/// One sample of a sweep.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Sample {
    pub value: f64,
    /// Total overlap of moving with static bodies, in mm³.
    pub overlap: f64,
    /// Overlapping pairs: (moving body, static body, mm³).
    pub pairs: Vec<(String, String, f64)>,
}

/// What a sweep found.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Sweep {
    pub joint: String,
    pub samples: Vec<Sample>,
    /// The clear stretch of samples around the joint's built value (None if
    /// it already collides there): the effective limits.
    pub clear: Option<(f64, f64)>,
}

impl Sweep {
    pub fn collides(&self, tolerance: f64) -> bool {
        self.samples.iter().any(|s| s.overlap > tolerance)
    }
}

/// Evenly spaced values from `from` to `to`, both included.
pub fn steps(from: f64, to: f64, count: usize) -> Vec<f64> {
    let count = count.max(2);
    (0..count).map(|i| from + (to - from) * i as f64 / (count - 1) as f64).collect()
}

type Bounds = Option<([f64; 3], [f64; 3])>;

/// Axis-aligned box of a solid from a coarse mesh, grown by `pad`.
fn bounds<K: Kernel>(kernel: &K, solid: &K::Solid, pad: f64) -> Bounds {
    let mesh = kernel.tessellate(solid, 0.5).ok()?;
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in &mesh.positions {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k] as f64 - pad);
            hi[k] = hi[k].max(p[k] as f64 + pad);
        }
    }
    lo[0].is_finite().then_some((lo, hi))
}

/// The box around `b` after a rigid motion (its moved corners' box).
fn moved_bounds(b: Bounds, t: &Rigid) -> Bounds {
    let (lo, hi) = b?;
    let mut out_lo = [f64::INFINITY; 3];
    let mut out_hi = [f64::NEG_INFINITY; 3];
    for i in 0..8 {
        let c = std::array::from_fn(|k| if i >> k & 1 == 0 { lo[k] } else { hi[k] });
        let p = t.point(c);
        for k in 0..3 {
            out_lo[k] = out_lo[k].min(p[k]);
            out_hi[k] = out_hi[k].max(p[k]);
        }
    }
    Some((out_lo, out_hi))
}

/// Unknown boxes might touch anything.
fn may_touch(a: Bounds, b: Bounds) -> bool {
    match (a, b) {
        (Some((alo, ahi)), Some((blo, bhi))) => (0..3).all(|k| alo[k] <= bhi[k] && blo[k] <= ahi[k]),
        _ => true,
    }
}

impl Document {
    /// Sweep `joint` through `values` and measure how much its bodies
    /// overlap the others at each one. Overlaps at or below `tolerance` mm³
    /// (touching faces) count as clear.
    pub fn sweep_joint<K: Kernel>(&self, kernel: &K, joint: &JointInfo, values: &[f64], tolerance: f64) -> Result<Sweep, String> {
        self.sweep_joint_in(kernel, &self.regenerate(kernel), joint, values, tolerance)
    }

    /// `sweep_joint` on an already regenerated model.
    pub fn sweep_joint_in<K: Kernel>(&self, kernel: &K, regen: &Regen<K::Solid>, joint: &JointInfo, values: &[f64], tolerance: f64) -> Result<Sweep, String> {
        if let Some(FeatureStatus::Error(e)) = regen.status.get(joint.feature) {
            return Err(format!("{}: {e}", joint.name));
        }
        let sketches = &regen.sketches;
        let (origin, dir) = joint.axis.resolve(sketches)?;
        let (moving, fixed): (Vec<_>, Vec<_>) = regen.bodies.iter().partition(|b| joint.bodies.contains(&b.created_by));
        if moving.is_empty() {
            return Err(format!("{} moves no bodies", joint.name));
        }
        let pad = 0.6;
        let moving_bounds: Vec<Bounds> = moving.iter().map(|b| bounds(kernel, &b.solid, pad)).collect();
        let fixed_bounds: Vec<Bounds> = fixed.iter().map(|b| bounds(kernel, &b.solid, pad)).collect();

        let mut samples = Vec::with_capacity(values.len());
        for &value in values {
            let t = Rigid::joint(joint.motion, origin, dir, value - joint.value);
            let mut sample = Sample { value, overlap: 0.0, pairs: Vec::new() };
            for (m, mb) in moving.iter().zip(&moving_bounds) {
                let mb = moved_bounds(*mb, &t);
                let mut solid = None;
                for (f, fb) in fixed.iter().zip(&fixed_bounds) {
                    if !may_touch(mb, *fb) {
                        continue;
                    }
                    let moved = solid.get_or_insert_with(|| match joint.motion {
                        _ if t.degrees == 0.0 && t.by == [0.0; 3] => m.solid.clone(),
                        JointMotion::Revolute => kernel.rotate(&m.solid, origin, dir, t.degrees),
                        JointMotion::Slider => kernel.translate(&m.solid, t.by),
                    });
                    let overlap = match kernel.boolean(moved, &f.solid, BooleanOp::Intersect) {
                        Ok(common) => kernel.volume(&common).abs(),
                        Err(e) => return Err(format!("{} against {} at {value}: {e}", m.name, f.name)),
                    };
                    if overlap > tolerance {
                        sample.overlap += overlap;
                        sample.pairs.push((m.name.clone(), f.name.clone(), overlap));
                    }
                }
            }
            samples.push(sample);
        }
        let clear = clear_range(&samples, joint.value, tolerance);
        Ok(Sweep { joint: joint.name.clone(), samples, clear })
    }
}

/// The run of clear samples around `home` (the sample nearest it).
fn clear_range(samples: &[Sample], home: f64, tolerance: f64) -> Option<(f64, f64)> {
    let mut order: Vec<usize> = (0..samples.len()).collect();
    order.sort_by(|&a, &b| samples[a].value.total_cmp(&samples[b].value));
    let near = |k: usize| (samples[order[k]].value - home).abs();
    let start = (0..order.len()).min_by(|&a, &b| near(a).total_cmp(&near(b)))?;
    let clear = |k: usize| samples[order[k]].overlap <= tolerance;
    if !clear(start) {
        return None;
    }
    let (mut lo, mut hi) = (start, start);
    while lo > 0 && clear(lo - 1) {
        lo -= 1;
    }
    while hi + 1 < order.len() && clear(hi + 1) {
        hi += 1;
    }
    Some((samples[order[lo]].value, samples[order[hi]].value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(value: f64, overlap: f64) -> Sample {
        Sample { value, overlap, pairs: Vec::new() }
    }

    #[test]
    fn clear_range_stops_at_the_first_contact_each_way() {
        let s: Vec<Sample> = [(-20.0, 5.0), (-10.0, 0.0), (0.0, 0.0), (10.0, 0.0), (20.0, 3.0), (30.0, 0.0)]
            .iter()
            .map(|&(v, o)| sample(v, o))
            .collect();
        assert_eq!(clear_range(&s, 1.0, 0.01), Some((-10.0, 10.0)));
        assert_eq!(clear_range(&s, 19.0, 0.01), None, "collides at the built value");
    }

    #[test]
    fn steps_include_both_ends() {
        assert_eq!(steps(10.0, -180.0, 20).len(), 20);
        assert_eq!(steps(0.0, 1.0, 3), vec![0.0, 0.5, 1.0]);
    }

    #[test]
    fn moved_box_follows_a_quarter_turn() {
        let b = Some(([0.0, 0.0, 0.0], [1.0, 2.0, 3.0]));
        let t = Rigid::joint(JointMotion::Revolute, [0.0; 3], [0.0, 0.0, 1.0], 90.0);
        let (lo, hi) = moved_bounds(b, &t).unwrap();
        assert!((lo[0] + 2.0).abs() < 1e-9 && (hi[1] - 1.0).abs() < 1e-9 && (hi[2] - 3.0).abs() < 1e-9, "{lo:?} {hi:?}");
    }
}

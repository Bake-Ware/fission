//! A Fusion timeline (decoded by `fission_f3d::design`) as a Fission
//! document: parameters, sketches and the features we can replay.
//!
//! Fusion stores centimetres and radians; Fission uses millimetres and
//! degrees.

use std::collections::HashMap;

use fission_f3d::design::{self, FeatureData, SketchCurve, Timeline};

use fission_kernel::{Kernel, Mesh};

use crate::sketch2d::{self, Curve};
use crate::{Axis, Document, EdgeRef, FeatureKind, Operation, OriginPlane, Parameter, Quantity, Sketch};

const CM: f64 = 10.0;

/// Fusion's model space is Y-up; Fission's is Z-up. Turn +90° about X, as
/// the B-rep import does, so both put parts the same way up.
fn up(p: [f64; 3]) -> [f64; 3] {
    [p[0], -p[2], p[1]]
}

fn up_geom(g: design::EdgeGeom) -> design::EdgeGeom {
    match g {
        design::EdgeGeom::Line { point, dir } => design::EdgeGeom::Line { point: up(point), dir: up(dir) },
        design::EdgeGeom::Circle { center, normal, radius } => design::EdgeGeom::Circle { center: up(center), normal: up(normal), radius },
    }
}

/// The converted document, and what couldn't be carried over.
pub struct Import {
    pub doc: Document,
    /// Features that changed the model but aren't replayed (by Fusion name).
    pub skipped: Vec<String>,
    /// Other problems worth showing.
    pub notes: Vec<String>,
}

/// Features that don't change any body (safe to leave out).
fn is_reference_only(kind: &str) -> bool {
    matches!(kind, "Plane" | "Joint Origin")
}

/// Rebuild a document and mesh its bodies: (creating feature, mesh).
pub type Rebuild<'a> = dyn FnMut(&Document) -> Vec<(usize, Mesh)> + 'a;

/// Convert a timeline with a kernel in this process.
pub fn document<K: Kernel>(t: &Timeline, name: &str, kernel: &K) -> Import {
    let mut rebuild = |doc: &Document| -> Vec<(usize, Mesh)> {
        let regen = doc.regenerate(kernel);
        regen.bodies.iter().filter_map(|b| Some((b.created_by, kernel.tessellate(&b.solid, 0.05).ok()?))).collect()
    };
    document_with(t, name, &mut rebuild)
}

/// Convert a timeline. `rebuild` builds the model so far, to turn Fusion's
/// picks of model edges into references to ours.
pub fn document_with(t: &Timeline, name: &str, rebuild: &mut Rebuild<'_>) -> Import {
    let mut doc = Document { name: name.to_string(), ..Default::default() };
    let mut skipped = Vec::new();
    let mut notes = Vec::new();

    // User parameters, and the model parameters their expressions or
    // features name.
    for p in t.parameters.iter().filter(|p| p.is_user()) {
        doc.parameters.push(Parameter { name: p.name.clone(), expression: p.expression.clone(), quantity: quantity(&p.unit), comment: String::new() });
    }

    // Fusion feature id -> our feature index; sketch data id -> (feature
    // index, Fusion curve id -> our curve index).
    let mut feature_at: HashMap<u64, usize> = HashMap::new();
    let mut sketches: HashMap<u64, (usize, HashMap<u64, usize>)> = HashMap::new();
    for f in &t.features {
        match &f.data {
            FeatureData::Sketch(s) => {
                let (plane, offset) = plane_of(&s.matrix);
                let (sketch, curve_map) = sketch_of(s);
                let index = push(&mut doc, FeatureKind::Sketch { plane, offset, sketch }, &f.name);
                feature_at.insert(f.id, index);
                sketches.insert(s.data, (index, curve_map));
            }
            FeatureData::Extrude(e) => match extrude(&doc, t, e, &sketches, &feature_at) {
                Ok((kind, target, expr)) => {
                    let index = push(&mut doc, kind, &f.name);
                    doc.features[index].target = target;
                    if let Some(expr) = expr {
                        doc.features[index].exprs.insert("distance".into(), expr);
                    }
                    feature_at.insert(f.id, index);
                }
                Err(e) => {
                    notes.push(format!("{}: {e}", f.name));
                    skipped.push(f.name.clone());
                    placeholder(&mut doc, f, &e);
                }
            },
            FeatureData::Fillet(fl) => match fillet(&doc, t, fl, rebuild) {
                Ok(kind) => {
                    let index = push(&mut doc, kind, &f.name);
                    if let Some(p) = fl.radius.as_ref().and_then(|n| t.parameter(n)).filter(|p| !crate::names_in(&p.expression).is_empty()) {
                        doc.features[index].exprs.insert("radius".into(), p.expression.clone());
                    }
                    feature_at.insert(f.id, index);
                }
                Err(e) => {
                    notes.push(format!("{}: {e}", f.name));
                    skipped.push(f.name.clone());
                    placeholder(&mut doc, f, &e);
                }
            },
            FeatureData::Combine(c) => {
                let target = c.target.and_then(|t| feature_at.get(&t)).copied();
                let tools: Option<Vec<usize>> = c.tools.iter().map(|t| feature_at.get(t).copied()).collect();
                let operation = match c.operation {
                    design::Operation::Join => Some(Operation::Join),
                    design::Operation::Cut => Some(Operation::Cut),
                    design::Operation::Intersect => Some(Operation::Intersect),
                    _ => None,
                };
                match (target, tools, operation) {
                    (Some(target), Some(tools), Some(operation)) if !tools.is_empty() => {
                        let index = push(&mut doc, FeatureKind::Combine { target, tools, operation, keep_tools: c.keep_tools }, &f.name);
                        feature_at.insert(f.id, index);
                    }
                    _ => {
                        notes.push(format!("{}: its bodies weren't imported", f.name));
                        skipped.push(f.name.clone());
                        placeholder(&mut doc, f, "its bodies weren't imported");
                    }
                }
            }
            FeatureData::Revolve(r) => match revolve(&doc, t, r, &sketches, &feature_at) {
                Ok((kind, target)) => {
                    let index = push(&mut doc, kind, &f.name);
                    doc.features[index].target = target;
                    feature_at.insert(f.id, index);
                }
                Err(e) => {
                    notes.push(format!("{}: {e}", f.name));
                    skipped.push(f.name.clone());
                    placeholder(&mut doc, f, &e);
                }
            },
            FeatureData::SplitBody(sp) => match split(&doc, sp, &sketches, &feature_at) {
                Ok(kind) => {
                    let index = push(&mut doc, kind, &f.name);
                    feature_at.insert(f.id, index);
                }
                Err(e) => {
                    notes.push(format!("{}: {e}", f.name));
                    skipped.push(f.name.clone());
                    placeholder(&mut doc, f, &e);
                }
            },
            FeatureData::Unsupported => {
                if !is_reference_only(&f.kind) {
                    skipped.push(f.name.clone());
                    placeholder(&mut doc, f, "");
                }
            }
        }
    }
    // Model parameters that user expressions or feature expressions name.
    let named: Vec<String> = doc.features.iter().flat_map(|f| f.exprs.values().flat_map(|e| crate::names_in(e))).collect();
    for n in named {
        if doc.parameters.iter().any(|p| p.name == n) {
            continue;
        }
        if let Some(p) = t.parameter(&n) {
            doc.parameters.push(Parameter { name: p.name.clone(), expression: p.expression.clone(), quantity: quantity(&p.unit), comment: p.role.clone() });
        }
    }
    doc.marker = doc.features.len();
    Import { doc, skipped, notes }
}

/// Keep a feature we can't replay in the timeline, as a do-nothing marker.
fn placeholder(doc: &mut Document, f: &design::Feature, note: &str) {
    push(doc, FeatureKind::Unsupported { kind: f.kind.clone(), note: note.to_string() }, &f.name);
}

fn push(doc: &mut Document, kind: FeatureKind, name: &str) -> usize {
    let index = doc.push(kind);
    doc.features[index].name = name.to_string();
    index
}

fn quantity(unit: &str) -> Quantity {
    match unit {
        "deg" | "rad" => Quantity::Angle,
        "" => Quantity::Count,
        _ => Quantity::Length,
    }
}

/// The sketch plane: an origin plane and offset when it is one, else the
/// plane itself.
fn plane_of(m: &[f64; 16]) -> (OriginPlane, f64) {
    let x = up([m[0], m[4], m[8]]);
    let n = up([m[2], m[6], m[10]]);
    let o = up([m[3] * CM, m[7] * CM, m[11] * CM]);
    let same = |a: [f64; 3], b: [f64; 3]| (0..3).all(|k| (a[k] - b[k]).abs() < 1e-9);
    for plane in OriginPlane::ALL {
        let p = plane.plane(0.0);
        if same(p.normal, n) && same(p.x_dir, x) {
            // The origin must lie on the normal line through the world origin.
            let along: f64 = (0..3).map(|k| o[k] * n[k]).sum();
            if same(std::array::from_fn(|k| n[k] * along), o) {
                return (plane, along + 0.0);
            }
        }
    }
    (OriginPlane::Custom { origin: o, normal: n, x_dir: x }, 0.0)
}

/// The sketch's curves as fixed geometry, with Fusion curve id -> curve index.
fn sketch_of(s: &design::Sketch) -> (sketch2d::Sketch, HashMap<u64, usize>) {
    let mut sk = sketch2d::Sketch::default();
    let mut map = HashMap::new();
    let point = |sk: &mut sketch2d::Sketch, p: [f64; 2]| -> usize {
        match sk.points.iter().position(|q| sketch2d::dist(*q, p) < 1e-6) {
            Some(i) => i,
            None => sk.add_point(p),
        }
    };
    let flat = |p: [f64; 3]| [p[0] * CM, p[1] * CM];
    for c in &s.curves {
        match *c {
            SketchCurve::Line { a, b, construction, .. } => {
                let (a, b) = (point(&mut sk, flat(a)), point(&mut sk, flat(b)));
                if a == b {
                    continue;
                }
                sk.curves.push(Curve::Line { a, b, construction });
            }
            SketchCurve::Arc { center, normal, x_dir, radius, start, end, construction, .. } => {
                let r = radius * CM;
                let c = point(&mut sk, flat(center));
                let sweep = end - start;
                if (sweep.abs() - std::f64::consts::TAU).abs() < 1e-9 {
                    sk.curves.push(Curve::Circle { center: c, radius: r, construction });
                } else {
                    // y = normal x x_dir, in the sketch plane.
                    let y = [normal[1] * x_dir[2] - normal[2] * x_dir[1], normal[2] * x_dir[0] - normal[0] * x_dir[2], normal[0] * x_dir[1] - normal[1] * x_dir[0]];
                    let at = |t: f64| [(center[0] + radius * (t.cos() * x_dir[0] + t.sin() * y[0])) * CM, (center[1] + radius * (t.cos() * x_dir[1] + t.sin() * y[1])) * CM];
                    let (p0, p1) = (point(&mut sk, at(start)), point(&mut sk, at(end)));
                    // Ours run counter-clockwise about +Z.
                    let (s0, e0) = if normal[2] >= 0.0 { (p0, p1) } else { (p1, p0) };
                    sk.curves.push(Curve::Arc { center: c, start: s0, end: e0, construction });
                }
            }
        }
        let id = match c {
            SketchCurve::Line { id, .. } | SketchCurve::Arc { id, .. } => *id,
        };
        map.insert(id, sk.curves.len() - 1);
    }
    (sk, map)
}

/// Which of our curves a region's boundary runs along.
fn region_curves(sk: &sketch2d::Sketch, region: &sketch2d::Region) -> Vec<usize> {
    let mut out = Vec::new();
    for lp in std::iter::once(&region.outer).chain(&region.holes) {
        for seg in &lp.segments {
            // Points along the piece: it lies on the curve that has them all.
            let probes: Vec<[f64; 2]> = match *seg {
                sketch2d::Segment::Line(a, b) => [0.25, 0.5, 0.75].iter().map(|t| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]).collect(),
                sketch2d::Segment::Arc { start, mid, end } => vec![start, mid, end],
                sketch2d::Segment::Circle { center, radius } => vec![[center[0] + radius, center[1]], [center[0] - radius, center[1]]],
            };
            let real = |c: usize| !sk.curves[c].construction();
            if let Some(c) = (0..sk.curves.len()).find(|&c| real(c) && probes.iter().all(|p| on_curve(sk, c, *p))) {
                if !out.contains(&c) {
                    out.push(c);
                }
            }
        }
    }
    out.sort_unstable();
    out
}

fn on_curve(sk: &sketch2d::Sketch, c: usize, p: [f64; 2]) -> bool {
    let tol = 1e-6;
    match sk.curves[c] {
        Curve::Line { a, b, .. } => {
            let (a, b) = (sk.points[a], sk.points[b]);
            let len = sketch2d::dist(a, b);
            let cross = ((b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])).abs() / len.max(1e-12);
            let t = ((p[0] - a[0]) * (b[0] - a[0]) + (p[1] - a[1]) * (b[1] - a[1])) / (len * len).max(1e-12);
            cross < tol && (-1e-9..=1.0 + 1e-9).contains(&t)
        }
        Curve::Circle { center, radius, .. } => (sketch2d::dist(sk.points[center], p) - radius).abs() < tol,
        Curve::Arc { center, start, end, .. } => {
            let c = sk.points[center];
            let r = sketch2d::dist(c, sk.points[start]);
            if (sketch2d::dist(c, p) - r).abs() > tol {
                return false;
            }
            let ang = |q: [f64; 2]| (q[1] - c[1]).atan2(q[0] - c[0]);
            let span = (ang(sk.points[end]) - ang(sk.points[start])).rem_euclid(std::f64::consts::TAU);
            (ang(p) - ang(sk.points[start])).rem_euclid(std::f64::consts::TAU) <= span + 1e-9
        }
    }
}

/// A point inside a region (not in its holes).
fn inside(region: &sketch2d::Region) -> Option<[f64; 2]> {
    let poly = region.outer.polygon();
    let n = poly.len() as f64;
    let c = [poly.iter().map(|p| p[0]).sum::<f64>() / n, poly.iter().map(|p| p[1]).sum::<f64>() / n];
    if region.contains(c) {
        return Some(c);
    }
    let (lo, hi) = poly.iter().fold(([f64::MAX; 2], [f64::MIN; 2]), |(lo, hi), p| ([lo[0].min(p[0]), lo[1].min(p[1])], [hi[0].max(p[0]), hi[1].max(p[1])]));
    for k in 1..64 {
        for i in 0..k {
            for j in 0..k {
                let p = [lo[0] + (hi[0] - lo[0]) * (i as f64 + 0.5) / k as f64, lo[1] + (hi[1] - lo[1]) * (j as f64 + 0.5) / k as f64];
                if region.contains(p) {
                    return Some(p);
                }
            }
        }
    }
    None
}

type Converted = (FeatureKind, Option<usize>, Option<String>);

type SketchMap = HashMap<u64, (usize, HashMap<u64, usize>)>;

/// Points inside our regions matching Fusion's profiles (by bounding curves).
fn profile_points(doc: &Document, sketches: &SketchMap, sketch: u64, profiles: &[design::Region]) -> Result<(usize, Vec<[f64; 2]>), String> {
    let (sketch_index, curve_map) = sketches.get(&sketch).ok_or("its sketch wasn't imported")?;
    let FeatureKind::Sketch { sketch: sk, .. } = &doc.features[*sketch_index].kind else { unreachable!() };
    let regions = sk.regions();
    let ours: Vec<Vec<usize>> = regions.iter().map(|r| region_curves(sk, r)).collect();
    let mut points = Vec::new();
    for profile in profiles {
        let mut want: Vec<usize> = profile.iter().flatten().filter_map(|(c, _)| curve_map.get(c).copied()).collect();
        want.sort_unstable();
        want.dedup();
        // The region bounded by exactly those curves, else the closest: a
        // curve that only touches (tangent at a point) is listed by Fusion
        // but bounds nothing here.
        let score = |i: usize| {
            let common = ours[i].iter().filter(|c| want.contains(c)).count() as f64;
            common / (ours[i].len() + want.len()) as f64 * 2.0
        };
        let hit = (0..regions.len())
            .filter(|&i| !ours[i].is_empty() && ours[i].iter().all(|c| want.contains(c)) && score(i) >= 0.75)
            .max_by(|&a, &b| score(a).total_cmp(&score(b)).then(regions[b].area.total_cmp(&regions[a].area)));
        if hit.is_none() && std::env::var_os("FISSION_F3D_DEBUG").is_some() {
            eprintln!("want {want:?} (fusion {profile:?}); ours {ours:?}; areas {:?}", regions.iter().map(|r| (r.area, r.holes.len())).collect::<Vec<_>>());
        }
        let i = hit.ok_or_else(|| format!("couldn't find a profile bounded by curves {want:?}"))?;
        points.push(inside(&regions[i]).ok_or("a profile has no inside")?);
    }
    Ok((*sketch_index, points))
}

fn extrude(doc: &Document, t: &Timeline, e: &design::Extrude, sketches: &SketchMap, feature_at: &HashMap<u64, usize>) -> Result<Converted, String> {
    let (sketch_index, points) = profile_points(doc, sketches, e.sketch, &e.regions)?;
    let sketch_index = &sketch_index;
    let param = |name: &Option<String>| name.as_ref().and_then(|n| t.parameter(n));
    let along = param(&e.distance).ok_or("no distance")?;
    if param(&e.taper).is_some_and(|p| p.value.abs() > 1e-12) {
        return Err("tapered extrudes aren't supported yet".into());
    }
    let mut symmetric = false;
    let mut distance = along.value * CM * e.direction;
    if let Some(against) = param(&e.against) {
        // Two sides: equal halves are a symmetric extrude.
        if (against.value + along.value / 2.0).abs() < 1e-9 || (against.value.abs() - along.value.abs()).abs() < 1e-9 {
            symmetric = true;
            if (against.value + along.value / 2.0).abs() < 1e-9 {
                distance /= 2.0;
            }
        } else {
            return Err("two-sided extrudes with unequal sides aren't supported yet".into());
        }
    }
    let operation = match e.operation {
        design::Operation::Join => Operation::Join,
        design::Operation::Cut => Operation::Cut,
        design::Operation::Intersect => Operation::Intersect,
        design::Operation::NewBody => Operation::NewBody,
        design::Operation::Other(k) => return Err(format!("unknown extrude operation {k}")),
    };
    // A join Fusion turned into a body of its own.
    let operation = if e.new_body && operation == Operation::Join { Operation::NewBody } else { operation };
    let target = e.participants.first().and_then(|p| feature_at.get(p)).copied().filter(|_| operation != Operation::NewBody);
    let expr = (!crate::names_in(&along.expression).is_empty() && !symmetric).then(|| {
        if e.direction < 0.0 { format!("-({})", along.expression) } else { along.expression.clone() }
    });
    Ok((FeatureKind::Extrude { sketch: Sketch::Profiles { sketch: *sketch_index, points }, distance, symmetric, operation }, target, expr))
}

fn kernel_operation(op: design::Operation) -> Result<Operation, String> {
    match op {
        design::Operation::Join => Ok(Operation::Join),
        design::Operation::Cut => Ok(Operation::Cut),
        design::Operation::Intersect => Ok(Operation::Intersect),
        design::Operation::NewBody => Ok(Operation::NewBody),
        design::Operation::Other(k) => Err(format!("unknown operation {k}")),
    }
}

fn revolve(doc: &Document, t: &Timeline, r: &design::Revolve, sketches: &SketchMap, feature_at: &HashMap<u64, usize>) -> Result<(FeatureKind, Option<usize>), String> {
    let (sketch_index, points) = profile_points(doc, sketches, r.sketch, &r.regions)?;
    let axis = r.axis.as_ref().ok_or("no axis")?;
    let axis = match sketches.get(&axis.sketch).and_then(|(i, map)| Some((*i, *map.get(&axis.curve)?))) {
        Some((sketch, curve)) => Axis::SketchLine { sketch, curve },
        None => match axis.line {
            Some(design::EdgeGeom::Line { point, dir }) => {
                let (point, dir) = (up(point), up(dir));
                let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
                Axis::Line { feature: sketch_index, origin: point.map(|v| v * CM), dir: dir.map(|v| v / len) }
            }
            _ => return Err("its axis isn't a sketch line".into()),
        },
    };
    let angle = r.angle.as_ref().and_then(|n| t.parameter(n)).ok_or("no angle")?;
    let degrees = angle.value.to_degrees();
    let target = r.participants.first().and_then(|p| feature_at.get(p)).copied();
    Ok((FeatureKind::Revolve { sketch: Sketch::Profiles { sketch: sketch_index, points }, axis, degrees, operation: kernel_operation(r.operation)? }, target))
}

/// Split along a sketch line: the plane through it, square to its sketch.
fn split(doc: &Document, sp: &design::SplitBody, sketches: &SketchMap, feature_at: &HashMap<u64, usize>) -> Result<FeatureKind, String> {
    let body = sp.body.and_then(|b| feature_at.get(&b)).copied().ok_or("its body wasn't imported")?;
    if let (None, Some((origin, normal))) = (&sp.tool, sp.plane) {
        let (origin, normal) = (up(origin).map(|v| v * CM), up(normal));
        // Any direction in the plane will do for its x axis.
        let a = if normal[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
        let x = [a[1] * normal[2] - a[2] * normal[1], a[2] * normal[0] - a[0] * normal[2], a[0] * normal[1] - a[1] * normal[0]];
        let len = (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt();
        return Ok(FeatureKind::Split { body: Some(body), plane: OriginPlane::Custom { origin, normal, x_dir: x.map(|v| v / len) }, offset: 0.0 });
    }
    let tool = sp.tool.as_ref().ok_or("only splitting by a sketch line or plane is supported")?;
    let Some(design::EdgeGeom::Line { point, dir }) = tool.line.map(up_geom) else { return Err("the splitting line wasn't cached".into()) };
    let (sketch_index, _) = sketches.get(&tool.sketch).ok_or("the splitting line's sketch wasn't imported")?;
    let FeatureKind::Sketch { plane, offset, .. } = &doc.features[*sketch_index].kind else { unreachable!() };
    let n = plane.plane(*offset).normal;
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    let d = dir.map(|v| v / len);
    let normal = [d[1] * n[2] - d[2] * n[1], d[2] * n[0] - d[0] * n[2], d[0] * n[1] - d[1] * n[0]];
    Ok(FeatureKind::Split { body: Some(body), plane: OriginPlane::Custom { origin: point.map(|v| v * CM), normal, x_dir: d }, offset: 0.0 })
}

/// Rebuild what we have, and find the edges of it lying on Fusion's cached
/// edge geometry.
fn fillet(doc: &Document, t: &Timeline, fl: &design::Fillet, rebuild: &mut Rebuild<'_>) -> Result<FeatureKind, String> {
    let radius = fl.radius.as_ref().and_then(|n| t.parameter(n)).ok_or("no radius")?.value * CM;
    let edges = resolve_edges(doc, &fl.edges, rebuild)?;
    Ok(FeatureKind::Fillet { radius, edges })
}

fn resolve_edges(doc: &Document, picks: &[design::EdgeGeom], rebuild: &mut Rebuild<'_>) -> Result<Vec<EdgeRef>, String> {
    let mut out: Vec<EdgeRef> = Vec::new();
    let meshes = rebuild(doc);
    let mut lost = 0;
    for pick in picks {
        let before = out.len();
        let pick = &up_geom(*pick);
        for (owner, mesh) in &meshes {
            for poly in &mesh.edges {
                if poly.len() < 2 || !on_pick(poly, pick) {
                    continue;
                }
                if let Some(h) = fission_kernel::EdgeHint::from_polyline(poly) {
                    if !out.iter().any(|e| e.body == *owner && fission_kernel::dist(e.mid, h.mid) < 1e-6) {
                        out.push(EdgeRef { body: *owner, mid: h.mid, dir: h.dir });
                    }
                }
            }
        }
        lost += (out.len() == before) as usize;
    }
    if out.is_empty() {
        return Err("none of its edges were found in the rebuilt model".into());
    }
    if lost > 0 {
        return Err(format!("{lost} of its {} edges weren't found in the rebuilt model", picks.len()));
    }
    Ok(out)
}

/// Does a tessellated edge (mm) lie on a cached edge (cm)?
fn on_pick(poly: &[[f32; 3]], pick: &design::EdgeGeom) -> bool {
    let tol = 1e-3;
    let pts = poly.iter().map(|p| [p[0] as f64 / CM, p[1] as f64 / CM, p[2] as f64 / CM]);
    match *pick {
        design::EdgeGeom::Line { point, dir } => {
            let len2 = dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2];
            pts.into_iter().all(|p| {
                let d = [p[0] - point[0], p[1] - point[1], p[2] - point[2]];
                let t = (d[0] * dir[0] + d[1] * dir[1] + d[2] * dir[2]) / len2;
                let off = [d[0] - dir[0] * t, d[1] - dir[1] * t, d[2] - dir[2] * t];
                (off[0] * off[0] + off[1] * off[1] + off[2] * off[2]).sqrt() < tol && (-1e-6..=1.0 + 1e-6).contains(&t)
            })
        }
        design::EdgeGeom::Circle { center, normal, radius } => pts.into_iter().all(|p| {
            let d = [p[0] - center[0], p[1] - center[1], p[2] - center[2]];
            let along = d[0] * normal[0] + d[1] * normal[1] + d[2] * normal[2];
            let r = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2] - along * along).max(0.0).sqrt();
            along.abs() < tol && (r - radius).abs() < tol
        }),
    }
}

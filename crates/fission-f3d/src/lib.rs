//! Reading Fusion 360 `.f3d` archives (clean-room, from the files alone).
//!
//! An `.f3d` is a zip (deflate or zstd entries). Exact geometry lives in
//! `FusionAssetName[Active]/Breps.BlobParts/BREP.<uuid>.smb`: ShapeManager
//! binary B-reps, a sibling of the ACIS SAB format (see [`sab`]). This crate
//! turns them into a kernel-neutral topology model ([`Model`]) with analytic
//! geometry: planes, cylinders/cones, spheres, tori, lines, circles/ellipses,
//! B-spline edge curves and extruded-spline surfaces. Other procedural
//! spline surfaces are reported as unsupported. The parametric timeline
//! (the design segment's `MetaStream.dat`/`BulkStream.dat`) is read by
//! [`design`].

use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::Path;

pub mod design;
pub mod sab;

use sab::{Fields, Record, Value};

#[derive(Debug, thiserror::Error)]
pub enum F3dError {
    #[error("couldn't read the file: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a readable .f3d archive: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("unexpected B-rep data: {0}")]
    Format(String),
    #[error("B-rep data ends early")]
    Truncated,
    #[error("the file has no B-rep geometry")]
    NoGeometry,
}

pub type P3 = [f64; 3];

#[derive(Clone, Debug, PartialEq)]
pub enum Surface {
    Plane { origin: P3, normal: P3, u_dir: P3 },
    /// Cylinder when `sin_half_angle` is 0. `major` is the radius vector at
    /// `center`; `ratio` < 1 makes it elliptical.
    Cone { center: P3, axis: P3, major: P3, ratio: f64, sin_half_angle: f64, cos_half_angle: f64 },
    Sphere { center: P3, radius: f64, u_dir: P3, pole: P3 },
    Torus { center: P3, normal: P3, major_radius: f64, minor_radius: f64, u_dir: P3 },
    /// A profile curve swept along a direction.
    Extrusion { profile: BSpline, dir: P3 },
    Unsupported(String),
}

/// A B-spline curve. Knot multiplicities follow the usual clamped
/// convention (ends = degree + 1), not ACIS's (ends = degree).
#[derive(Clone, Debug, PartialEq)]
pub struct BSpline {
    pub degree: u32,
    pub knots: Vec<f64>,
    pub mults: Vec<u32>,
    pub poles: Vec<P3>,
    /// Rational weights, one per pole.
    pub weights: Option<Vec<f64>>,
}

impl BSpline {
    /// The same curve run the other way.
    pub fn reversed(&self) -> BSpline {
        let (a, b) = (self.knots[0], *self.knots.last().unwrap());
        BSpline {
            degree: self.degree,
            knots: self.knots.iter().rev().map(|k| a + b - k).collect(),
            mults: self.mults.iter().rev().copied().collect(),
            poles: self.poles.iter().rev().copied().collect(),
            weights: self.weights.as_ref().map(|w| w.iter().rev().copied().collect()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Curve {
    Line { origin: P3, dir: P3 },
    /// Circle when `ratio` is 1. `major` is the semi-major axis vector.
    Ellipse { center: P3, normal: P3, major: P3, ratio: f64 },
    BSpline(BSpline),
    Unsupported(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    pub start: usize,
    pub end: usize,
    pub curve: Curve,
    /// The edge runs against its curve's direction.
    pub reversed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub surface: Surface,
    /// The face normal is opposite to the surface normal.
    pub reversed: bool,
    /// Loops of (edge index, coedge runs against the edge).
    pub loops: Vec<Vec<(usize, bool)>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    /// Rows of the rotation/scale matrix applied to points: p' = M·p·scale + t.
    pub rows: [P3; 3],
    pub translation: P3,
    pub scale: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Body {
    pub vertices: Vec<P3>,
    pub edges: Vec<Edge>,
    pub faces: Vec<Face>,
    pub transform: Option<Transform>,
    /// Every coedge has a partner: the faces close up (a solid, not a sheet).
    pub closed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Model {
    pub bodies: Vec<Body>,
    /// Geometry types seen but not converted, with counts.
    pub unsupported: BTreeMap<String, usize>,
}

impl Model {
    pub fn face_count(&self) -> usize {
        self.bodies.iter().map(|b| b.faces.len()).sum()
    }

    pub fn supported_face_count(&self) -> usize {
        self.bodies.iter().flat_map(|b| &b.faces).filter(|f| !matches!(f.surface, Surface::Unsupported(_))).count()
    }
}

/// Read every B-rep blob in an `.f3d`.
pub fn read_f3d(path: &Path) -> Result<Model, F3dError> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path)?)?;
    let mut model = Model::default();
    let names: Vec<String> = archive
        .file_names()
        .filter_map(|n| n.ok().map(|n| n.into_owned()))
        .filter(|n| n.contains("Breps.BlobParts/") && n.ends_with(".smb"))
        .collect();
    if names.is_empty() {
        return Err(F3dError::NoGeometry);
    }
    for name in names {
        let mut data = Vec::new();
        archive.by_name(&name)?.read_to_end(&mut data)?;
        let part = read_blob(&data)?;
        model.bodies.extend(part.bodies);
        for (k, n) in part.unsupported {
            *model.unsupported.entry(k).or_default() += n;
        }
    }
    Ok(model)
}

/// The design's timeline: parameters, sketches and features, in order.
pub fn read_timeline(path: &Path) -> Result<design::Timeline, F3dError> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path)?)?;
    let metas: Vec<String> = archive.file_names().filter_map(|n| n.ok().map(|n| n.into_owned())).filter(|n| n.ends_with("/MetaStream.dat")).collect();
    for meta_name in metas {
        let mut meta = Vec::new();
        archive.by_name(&meta_name)?.read_to_end(&mut meta)?;
        let mut bulk = Vec::new();
        archive.by_name(&meta_name.replace("MetaStream.dat", "BulkStream.dat"))?.read_to_end(&mut bulk)?;
        // The design segment is the one with a timeline (older files name
        // their segments differently).
        if let Some(seg) = design::Segment::parse(&meta, bulk) {
            if !seg.objects(design::Kind::Timeline).is_empty() {
                return Ok(design::timeline(&seg));
            }
        }
    }
    Err(F3dError::Format("no design segment".into()))
}

/// Convert one `.smb` blob.
pub fn read_blob(data: &[u8]) -> Result<Model, F3dError> {
    let (header, records) = sab::parse(data)?;
    // Subtype blocks (`{ name ... }`) are numbered in file order; later data
    // can point back at one with `ref n` instead of repeating it.
    let mut subtypes = Vec::new();
    for (ri, r) in records.iter().enumerate() {
        for (vi, w) in r.values.windows(2).enumerate() {
            if let [Value::Open, Value::Id(name)] = w {
                if name != "ref" {
                    subtypes.push((ri, vi));
                }
            }
        }
    }
    Builder { records: &records, subtypes, scale: header.mm_per_unit, model: Model::default() }.build()
}

struct Builder<'a> {
    records: &'a [Record],
    /// (record, value index of the `Open`) for each subtype block.
    subtypes: Vec<(usize, usize)>,
    /// Model units to mm.
    scale: f64,
    model: Model,
}

/// Every entity starts with attribute pointer, history index, and one more
/// ShapeManager pointer; the fields we want come after.
const PREFIX: usize = 3;

impl Builder<'_> {
    fn rec(&self, i: usize) -> Result<&Record, F3dError> {
        self.records.get(i).ok_or_else(|| F3dError::Format(format!("dangling pointer {i}")))
    }

    fn fields(&self, i: usize, expect: &str) -> Result<Fields<'_>, F3dError> {
        let r = self.rec(i)?;
        if !r.is(expect) {
            return Err(F3dError::Format(format!("record {i} is {:?}, expected {expect}", r.kind)));
        }
        let mut f = Fields::new(r);
        f.skip(PREFIX);
        Ok(f)
    }

    fn mm(&self, p: P3) -> P3 {
        p.map(|x| x * self.scale)
    }

    fn build(mut self) -> Result<Model, F3dError> {
        // Bodies are the roots; walk each one's lumps -> shells -> faces.
        let roots: Vec<usize> = (0..self.records.len()).filter(|&i| self.records[i].is("body")).collect();
        for b in roots {
            let body = self.body(b)?;
            if !body.faces.is_empty() {
                self.model.bodies.push(body);
            }
        }
        Ok(self.model)
    }

    fn body(&mut self, index: usize) -> Result<Body, F3dError> {
        let mut f = self.fields(index, "body")?;
        let lump = f.ptr();
        let _wire = f.ptr();
        let transform = f.ptr();
        let mut body = Body { transform: transform.map(|t| self.transform(t)).transpose()?, closed: true, ..Default::default() };
        let mut vertex_index: HashMap<usize, usize> = HashMap::new();
        let mut edge_index: HashMap<usize, usize> = HashMap::new();

        let mut lump = lump;
        let mut guard = 0;
        while let Some(l) = lump {
            let mut lf = self.fields(l, "lump")?;
            let next_lump = lf.ptr();
            let mut shell = lf.ptr();
            while let Some(s) = shell {
                let mut sf = self.fields(s, "shell")?;
                let next_shell = sf.ptr();
                let _subshell = sf.ptr();
                let mut face = sf.ptr();
                while let Some(fi) = face {
                    let (next, built) = self.face(fi, &mut body, &mut vertex_index, &mut edge_index)?;
                    body.faces.push(built);
                    face = next;
                    guard += 1;
                    if guard > 10_000_000 {
                        return Err(F3dError::Format("face list doesn't terminate".into()));
                    }
                }
                shell = next_shell;
            }
            lump = next_lump;
        }
        Ok(body)
    }

    fn transform(&self, index: usize) -> Result<Transform, F3dError> {
        // Transforms carry only attribute + history before their data.
        let r = self.rec(index)?;
        if !r.is("transform") {
            return Err(F3dError::Format(format!("record {index} is {:?}, expected transform", r.kind)));
        }
        let mut f = Fields::new(r);
        f.skip(2);
        let x = f.point().ok_or(F3dError::Truncated)?;
        let y = f.point().ok_or(F3dError::Truncated)?;
        let z = f.point().ok_or(F3dError::Truncated)?;
        let t = f.point().ok_or(F3dError::Truncated)?;
        let scale = f.double().unwrap_or(1.0);
        // ACIS stores the images of the axes; points transform as p·M (row vectors).
        let rows = [[x[0], y[0], z[0]], [x[1], y[1], z[1]], [x[2], y[2], z[2]]];
        Ok(Transform { rows, translation: self.mm(t), scale })
    }

    #[allow(clippy::type_complexity)]
    fn face(
        &mut self,
        index: usize,
        body: &mut Body,
        vertex_index: &mut HashMap<usize, usize>,
        edge_index: &mut HashMap<usize, usize>,
    ) -> Result<(Option<usize>, Face), F3dError> {
        let mut f = self.fields(index, "face")?;
        let next = f.ptr();
        let first_loop = f.ptr();
        let _shell = f.ptr();
        let _subshell = f.ptr();
        let surface = f.ptr();
        let mut reversed = f.logical().unwrap_or(false);
        let surface = match surface {
            Some(s) => {
                let (surface, flipped) = self.surface(s)?;
                reversed ^= flipped;
                surface
            }
            None => Surface::Unsupported("none".into()),
        };

        let mut loops = Vec::new();
        let mut lp = first_loop;
        while let Some(l) = lp {
            let mut lf = self.fields(l, "loop")?;
            let next_loop = lf.ptr();
            let first = lf.ptr();
            let mut coedges = Vec::new();
            let mut ce = first;
            let mut steps = 0;
            while let Some(c) = ce {
                let mut cf = self.fields(c, "coedge")?;
                let next_ce = cf.ptr();
                let _prev = cf.ptr();
                let partner = cf.ptr();
                if partner.is_none() || partner == Some(c) {
                    body.closed = false;
                }
                let edge = cf.ptr().ok_or_else(|| F3dError::Format(format!("coedge {c} has no edge")))?;
                let ce_reversed = cf.logical().unwrap_or(false);
                let e = match edge_index.get(&edge) {
                    Some(&e) => e,
                    None => {
                        let built = self.edge(edge, body, vertex_index)?;
                        body.edges.push(built);
                        edge_index.insert(edge, body.edges.len() - 1);
                        body.edges.len() - 1
                    }
                };
                coedges.push((e, ce_reversed));
                ce = next_ce.filter(|&n| Some(n) != first);
                steps += 1;
                if steps > 1_000_000 {
                    return Err(F3dError::Format("coedge loop doesn't close".into()));
                }
            }
            loops.push(coedges);
            lp = next_loop;
        }
        Ok((next, Face { surface, reversed, loops }))
    }

    fn vertex(&self, index: usize, body: &mut Body, vertex_index: &mut HashMap<usize, usize>) -> Result<usize, F3dError> {
        if let Some(&v) = vertex_index.get(&index) {
            return Ok(v);
        }
        let mut f = self.fields(index, "vertex")?;
        let _edge = f.ptr();
        // An int sits between the edge and the point pointer in ASM vertices.
        let point = match f.ptr() {
            Some(p) => Some(p),
            None => f.ptr(),
        };
        let point = point.ok_or_else(|| F3dError::Format(format!("vertex {index} has no point")))?;
        let mut pf = self.fields(point, "point")?;
        let p = self.mm(pf.point().ok_or(F3dError::Truncated)?);
        body.vertices.push(p);
        vertex_index.insert(index, body.vertices.len() - 1);
        Ok(body.vertices.len() - 1)
    }

    fn edge(&mut self, index: usize, body: &mut Body, vertex_index: &mut HashMap<usize, usize>) -> Result<Edge, F3dError> {
        let mut f = self.fields(index, "edge")?;
        let start = f.ptr().ok_or_else(|| F3dError::Format(format!("edge {index} has no start")))?;
        let _t0 = f.double();
        let end = f.ptr().ok_or_else(|| F3dError::Format(format!("edge {index} has no end")))?;
        let _t1 = f.double();
        let _coedge = f.ptr();
        let curve = f.ptr();
        let reversed = f.logical().unwrap_or(false);
        let curve = match curve {
            Some(c) => self.curve(c)?,
            None => Curve::Unsupported("none".into()),
        };
        let start = self.vertex(start, body, vertex_index)?;
        let end = self.vertex(end, body, vertex_index)?;
        Ok(Edge { start, end, curve, reversed })
    }

    fn unsupported(&mut self, kind: String) -> String {
        *self.model.unsupported.entry(kind.clone()).or_default() += 1;
        kind
    }

    /// Follow `ref n` to the block it names; (record, value index of `Open`).
    fn resolve_block(&self, rec: usize, open: usize) -> Option<(usize, usize)> {
        let vals = &self.records.get(rec)?.values;
        match (vals.get(open)?, vals.get(open + 1)?, vals.get(open + 2)) {
            (Value::Open, Value::Id(name), Some(Value::Int(n))) if name == "ref" => {
                let &(r, o) = self.subtypes.get(*n as usize)?;
                self.resolve_block(r, o)
            }
            (Value::Open, Value::Id(_), _) => Some((rec, open)),
            _ => None,
        }
    }

    /// The first B-spline curve inside the block at (`rec`, `open`): the
    /// exact or approximating 3D curve of an intcurve.
    fn block_bs3(&self, rec: usize, open: usize) -> Option<BSpline> {
        let (rec, open) = self.resolve_block(rec, open)?;
        let vals = &self.records[rec].values;
        let end = block_end(vals, open)?;
        let at = (open..end).find(|&i| matches!(&vals[i], Value::Id(n) if n == "nubs" || n == "nurbs"))?;
        bs3(vals, at, self.scale)
    }

    fn surface(&mut self, index: usize) -> Result<(Surface, bool), F3dError> {
        let r = self.rec(index)?;
        if r.subtype() == Some("spline") {
            let flipped = matches!(r.values.get(PREFIX), Some(Value::True));
            return Ok((self.spline_surface(index, PREFIX + 1), flipped));
        }
        let sub = r.subtype().unwrap_or("").to_string();
        let mut f = Fields::new(r);
        f.skip(PREFIX);
        let missing = || F3dError::Format(format!("{sub} surface {index} is incomplete"));
        Ok((match sub.as_str() {
            "plane" => Surface::Plane {
                origin: self.mm(f.point().ok_or_else(missing)?),
                normal: f.point().ok_or_else(missing)?,
                u_dir: f.point().ok_or_else(missing)?,
            },
            "cone" => {
                let center = self.mm(f.point().ok_or_else(missing)?);
                let axis = f.point().ok_or_else(missing)?;
                let major = self.mm(f.point().ok_or_else(missing)?);
                let ratio = f.double().ok_or_else(missing)?;
                f.bound();
                f.bound();
                let sin = f.double().ok_or_else(missing)?;
                let cos = f.double().ok_or_else(missing)?;
                Surface::Cone { center, axis, major, ratio, sin_half_angle: sin, cos_half_angle: cos }
            }
            "sphere" => Surface::Sphere {
                center: self.mm(f.point().ok_or_else(missing)?),
                radius: f.double().ok_or_else(missing)? * self.scale,
                u_dir: f.point().ok_or_else(missing)?,
                pole: f.point().ok_or_else(missing)?,
            },
            "torus" => Surface::Torus {
                center: self.mm(f.point().ok_or_else(missing)?),
                normal: f.point().ok_or_else(missing)?,
                major_radius: f.double().ok_or_else(missing)? * self.scale,
                minor_radius: f.double().ok_or_else(missing)? * self.scale,
                u_dir: f.point().ok_or_else(missing)?,
            },
            other => Surface::Unsupported(self.unsupported(format!("{other} surface"))),
        }, false))
    }

    /// A spline surface's subtype block at (`rec`, `open`).
    fn spline_surface(&mut self, rec: usize, open: usize) -> Surface {
        let Some((rec, open)) = self.resolve_block(rec, open) else {
            return Surface::Unsupported(self.unsupported("spline surface without data".into()));
        };
        let vals = &self.records[rec].values;
        let name = match &vals[open + 1] {
            Value::Id(n) => n.clone(),
            _ => String::new(),
        };
        match name.as_str() {
            // Extrusion: { cyl_spl_sur version intcurve reversed { curve } range range dir ... }
            "cyl_spl_sur" => {
                let curve_rev = matches!(vals.get(open + 4), Some(Value::True));
                let curve_open = open + 5;
                let profile = self.block_bs3(rec, curve_open);
                let dir = block_end(vals, curve_open)
                    .and_then(|e| vals[e..].iter().find_map(|v| if let Value::Vec(d) = v { Some(*d) } else { None }));
                match (profile, dir) {
                    (Some(p), Some(dir)) => Surface::Extrusion { profile: if curve_rev { p.reversed() } else { p }, dir },
                    _ => Surface::Unsupported(self.unsupported("cyl_spl_sur without a curve".into())),
                }
            }
            other => Surface::Unsupported(self.unsupported(format!("{other} spline surface"))),
        }
    }

    fn curve(&mut self, index: usize) -> Result<Curve, F3dError> {
        let r = self.rec(index)?;
        let sub = r.subtype().unwrap_or("").to_string();
        let mut f = Fields::new(r);
        f.skip(PREFIX);
        let missing = || F3dError::Format(format!("{sub} curve {index} is incomplete"));
        Ok(match sub.as_str() {
            "straight" => Curve::Line { origin: self.mm(f.point().ok_or_else(missing)?), dir: f.point().ok_or_else(missing)? },
            "ellipse" => Curve::Ellipse {
                center: self.mm(f.point().ok_or_else(missing)?),
                normal: f.point().ok_or_else(missing)?,
                major: self.mm(f.point().ok_or_else(missing)?),
                ratio: f.double().ok_or_else(missing)?,
            },
            "intcurve" => {
                let reversed = matches!(r.values.get(PREFIX), Some(Value::True));
                match self.block_bs3(index, PREFIX + 1) {
                    Some(b) => Curve::BSpline(if reversed { b.reversed() } else { b }),
                    None => Curve::Unsupported(self.unsupported("intcurve without a B-spline".into())),
                }
            }
            other => Curve::Unsupported(self.unsupported(format!("{other} curve"))),
        })
    }
}

/// Index of the `Close` matching the `Open` at `open`.
fn block_end(vals: &[Value], open: usize) -> Option<usize> {
    let mut depth = 0;
    for (i, v) in vals.iter().enumerate().skip(open) {
        match v {
            Value::Open => depth += 1,
            Value::Close => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// A `nubs`/`nurbs` B-spline curve at `at`: degree, closure, knot count,
/// (knot, multiplicity) pairs, then x y z (w) per pole. ACIS end
/// multiplicities are the degree; we return them clamped (degree + 1).
fn bs3(vals: &[Value], at: usize, scale: f64) -> Option<BSpline> {
    let rational = matches!(vals.get(at)?, Value::Id(n) if n == "nurbs");
    let int = |i: usize| match vals.get(i)? {
        Value::Int(n) => Some(*n as i64),
        Value::Enum(n) => Some(*n as i64),
        _ => None,
    };
    let dbl = |i: usize| match vals.get(i)? {
        Value::Double(d) => Some(*d),
        Value::Int(n) => Some(*n as f64),
        _ => None,
    };
    let degree = int(at + 1)? as u32;
    let _closure = int(at + 2)?;
    let count = int(at + 3)? as usize;
    let mut i = at + 4;
    let (mut knots, mut mults) = (Vec::with_capacity(count), Vec::with_capacity(count));
    for _ in 0..count {
        knots.push(dbl(i)?);
        mults.push(int(i + 1)? as u32);
        i += 2;
    }
    if degree == 0 || knots.len() < 2 {
        return None;
    }
    let total: u32 = mults.iter().sum();
    let n_poles = (total + 1).checked_sub(degree)? as usize;
    let stride = if rational { 4 } else { 3 };
    let (mut poles, mut weights) = (Vec::with_capacity(n_poles), Vec::new());
    for _ in 0..n_poles {
        poles.push([dbl(i)? * scale, dbl(i + 1)? * scale, dbl(i + 2)? * scale]);
        if rational {
            weights.push(dbl(i + 3)?);
        }
        i += stride;
    }
    // Clamp: ACIS leaves the ends one short.
    if mults[0] == degree {
        mults[0] += 1;
    }
    let last = mults.len() - 1;
    if mults[last] == degree {
        mults[last] += 1;
    }
    Some(BSpline { degree, knots, mults, poles, weights: rational.then_some(weights) })
}

#[cfg(test)]
mod tests;

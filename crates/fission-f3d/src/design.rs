//! Fusion's parametric design (the timeline) from an `.f3d`'s design
//! segment: `MetaStream.dat` and `BulkStream.dat`.
//!
//! Format notes, all observed from files (clean-room):
//! - MetaStream: `[u32 n][segment type]`, a few header fields, then class
//!   records `[lps class GUID][lps parent GUID or ""][u32 version][lps module]
//!   [u32 count][count x u64 object id]` (lps = u32 length + ASCII), then
//!   further tables.
//! - BulkStream: chunks `[lps "<256 + class index>"][u64 object id][fields]`,
//!   one chunk per class level of an object (most derived first). A
//!   reference to another object is `01` + u64 id + two bytes; strings are
//!   `[u32 n][n UTF-16 units]`; numbers are little-endian f64 in centimetres
//!   and radians.
//!
//! Classes are recognised by their GUIDs, which identify the format; the
//! names here are ours.

use std::collections::HashMap;
use std::ops::Range;

/// The classes we decode, by GUID.
const KNOWN: &[(&str, Kind)] = &[
    ("8DA771B7-52ED-42FC-94E2-2353FE141373", Kind::SketchFeature),
    ("44A64366-4BD3-4B24-881A-F94C206E8F2D", Kind::SketchData),
    ("C2CEDAE7-1716-47C1-B7B1-07B70081D0FB", Kind::Point),
    ("DCA267ED-D615-4934-B64F-AD805E8003E2", Kind::Line),
    ("F0130424-8B7E-4092-93C9-1CA807482534", Kind::Arc),
    ("7B6C3C2D-4096-4E6D-B225-699E8C2D9355", Kind::Matrix),
    ("DD405BC2-D673-44F0-8833-5CB2A1C186C7", Kind::Extrude),
    ("E3849A15-2FC6-42A0-AF3A-2F1D7273B406", Kind::Revolve),
    ("7A6A3D31-BE74-4E19-9115-20642944C2E4", Kind::Parameter),
    ("2F4C1849-1A5A-4F6C-A086-8DD445CBF94B", Kind::Timeline),
    ("4BD53E5A-0B3E-45E5-AE8B-02044306485A", Kind::RegionRef),
    ("0D57BD2F-D09B-43FC-AD57-1E89A118C453", Kind::RegionRecipe),
    ("A07D5F17-68CB-464D-9935-BF68E98A865F", Kind::Fillet),
    ("2A94257F-2020-4B19-9A68-103A2672F1B7", Kind::Combine),
    ("E8063467-BC05-4688-8196-AECB90F0EFEE", Kind::Shell),
    ("65CA3206-E33A-471E-8E40-2C72B57C6181", Kind::SplitBody),
    ("584D9526-EF22-4F6F-9FE1-5FCF1F3CE575", Kind::Thread),
    ("11F1A5CE-2B57-4476-8480-6994621493C9", Kind::CircularPattern),
    ("AE12AD3F-BB81-4B59-9D34-A599E58E4BF6", Kind::RectangularPattern),
    ("F1451EC8-11A4-4952-84ED-5AB7C13E2E83", Kind::SketchProject),
    ("F757D611-217B-4B72-9C41-B617BDCE43DA", Kind::Chamfer),
    ("1C037A07-4A15-43F6-ABFC-BBF61B9038D4", Kind::Hole),
    ("F4163A07-9E71-4C46-BD92-ACE5B5B40BED", Kind::Draft),
    ("64E40C8E-E564-469C-8690-5C4E77C1B02C", Kind::Move),
    ("02E8AA49-08B8-439C-A8C7-221BE20143D8", Kind::JointOrigin),
    ("8D424E29-DE23-4B1F-A3AD-84C52A497D1A", Kind::WorkPlane),
    ("D869265F-F339-4751-A066-91D30F81FF08", Kind::ConstructionPlane),
    ("9716F783-676D-42E0-93F9-EBF273E7C035", Kind::BodyRef),
    ("D26351F0-5940-4D23-AA20-2C35475A6D9E", Kind::Body),
    ("50AEE8B4-9456-4DD7-847D-6F6C909BBB9D", Kind::CachedLine),
    ("B0B3FA59-C374-444F-B424-24AA5DE41645", Kind::CachedArc),
    ("5662F619-2281-4AD9-93DC-1FCFDA1062E7", Kind::BrepRef),
    ("E2CEFD18-D755-4E09-8E7F-953A2F6D43F8", Kind::CurveRecipe),
    ("5A1BF548-241F-46FD-9FB5-E4B05126EB9D", Kind::EntityRef),
    ("2CA5A1CD-C99B-4C9B-91AF-57989148E841", Kind::ModelRef),
    ("CD57BC48-50EC-47DC-975A-FB6DEA72F4DA", Kind::SolidList),
    ("D3937028-C20C-4E65-B010-94AD418A5C20", Kind::Solid),
    ("CC54ECAD-6DE8-4DE1-803D-BBE0E751975B", Kind::CachedPlane),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    SketchFeature,
    SketchData,
    Point,
    Line,
    Arc,
    Matrix,
    Extrude,
    Revolve,
    Parameter,
    Timeline,
    RegionRef,
    RegionRecipe,
    Fillet,
    Combine,
    Shell,
    SplitBody,
    Thread,
    CircularPattern,
    RectangularPattern,
    SketchProject,
    Chamfer,
    Hole,
    Draft,
    Move,
    JointOrigin,
    WorkPlane,
    ConstructionPlane,
    BodyRef,
    Body,
    CachedLine,
    CachedArc,
    BrepRef,
    CurveRecipe,
    EntityRef,
    ModelRef,
    /// A component's current bodies.
    SolidList,
    /// A body's display and mass properties.
    Solid,
    /// A picked planar face's plane.
    CachedPlane,
}

impl Kind {
    /// What Fusion's timeline calls this kind of feature.
    pub fn label(self) -> &'static str {
        match self {
            Kind::SketchFeature => "Sketch",
            Kind::Extrude => "Extrude",
            Kind::Revolve => "Revolve",
            Kind::Fillet => "Fillet",
            Kind::Combine => "Combine",
            Kind::Shell => "Shell",
            Kind::SplitBody => "Split Body",
            Kind::Thread => "Thread",
            Kind::CircularPattern => "Circular Pattern",
            Kind::RectangularPattern => "Rectangular Pattern",
            Kind::SketchProject => "Project",
            Kind::Chamfer => "Chamfer",
            Kind::Hole => "Hole",
            Kind::Draft => "Draft",
            Kind::Move => "Move",
            Kind::JointOrigin => "Joint Origin",
            Kind::ConstructionPlane => "Plane",
            _ => "Feature",
        }
    }
}

/// One class in the segment's table.
#[derive(Clone, Debug)]
pub struct Class {
    pub guid: String,
    pub parent: String,
    pub version: u32,
    pub kind: Option<Kind>,
}

/// A design segment's objects, split into their chunks.
pub struct Segment {
    pub classes: Vec<Class>,
    bulk: Vec<u8>,
    /// Object id -> its chunks (class index, byte range in `bulk`).
    chunks: HashMap<u64, Vec<(usize, Range<usize>)>>,
    /// Object id -> its class (the most derived one).
    class_of: HashMap<u64, usize>,
    /// The timeline's entries, in order (after the fragment it opens with).
    listed: Vec<u64>,
}

fn read_u32(d: &[u8], p: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(p..p + 4)?.try_into().ok()?))
}

fn read_u64(d: &[u8], p: usize) -> Option<u64> {
    Some(u64::from_le_bytes(d.get(p..p + 8)?.try_into().ok()?))
}

fn read_f64(d: &[u8], p: usize) -> Option<f64> {
    Some(f64::from_le_bytes(d.get(p..p + 8)?.try_into().ok()?))
}

/// `[u32 n][n bytes]` as ASCII.
fn lps(d: &[u8], p: usize) -> Option<(&[u8], usize)> {
    let n = read_u32(d, p)? as usize;
    Some((d.get(p + 4..p + 4 + n)?, p + 4 + n))
}

/// `[u32 n][n UTF-16 units]`.
fn lps16(d: &[u8], p: usize) -> Option<(String, usize)> {
    let n = read_u32(d, p)? as usize;
    if n > 4096 {
        return None;
    }
    let bytes = d.get(p + 4..p + 4 + 2 * n)?;
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Some((String::from_utf16_lossy(&units), p + 4 + 2 * n))
}

fn is_guid(s: &[u8]) -> bool {
    (30..=40).contains(&s.len()) && s.iter().filter(|&&c| c == b'-').count() >= 4 && s.iter().all(|c| c.is_ascii_hexdigit() || *c == b'-')
}

impl Segment {
    pub fn parse(meta: &[u8], bulk: Vec<u8>) -> Option<Segment> {
        let known: HashMap<&str, Kind> = KNOWN.iter().copied().collect();
        // The first class record: the first GUID after the header.
        let mut p = (0x40..meta.len().saturating_sub(40)).find(|&p| {
            read_u32(meta, p) == Some(36) && meta.get(p + 4..p + 40).is_some_and(is_guid)
        })?;
        let mut classes = Vec::new();
        let mut class_of = HashMap::new();
        while let Some((g, q)) = lps(meta, p) {
            if !is_guid(g) {
                break;
            }
            let (parent, q) = lps(meta, q)?;
            let version = read_u32(meta, q)?;
            let (_module, q) = lps(meta, q + 4)?;
            let count = read_u32(meta, q)? as usize;
            let mut q = q + 4;
            let index = classes.len();
            for _ in 0..count {
                class_of.insert(read_u64(meta, q)?, index);
                q += 8;
            }
            let guid = String::from_utf8_lossy(g).to_ascii_uppercase();
            let kind = known.get(guid.as_str()).copied();
            classes.push(Class { guid, parent: String::from_utf8_lossy(parent).to_ascii_uppercase(), version, kind });
            p = q;
        }
        if classes.is_empty() {
            return None;
        }
        // Chunk headers: "\x03\0\0\0" + three digits + u64 id.
        let mut heads: Vec<(usize, usize, usize, u64)> = Vec::new();
        let mut i = 0;
        while i + 15 <= bulk.len() {
            if bulk[i..i + 4] == [3, 0, 0, 0] && bulk[i + 4..i + 7].iter().all(u8::is_ascii_digit) {
                let tag = std::str::from_utf8(&bulk[i + 4..i + 7]).ok().and_then(|s| s.parse::<usize>().ok());
                let id = read_u64(&bulk, i + 7);
                if let (Some(tag), Some(id)) = (tag, id) {
                    if tag >= 256 && tag - 256 < classes.len() && id < 50_000_000 {
                        heads.push((i, i + 15, tag - 256, id));
                        i += 15;
                        continue;
                    }
                }
            }
            i += 1;
        }
        let mut chunks: HashMap<u64, Vec<(usize, Range<usize>)>> = HashMap::new();
        for (k, &(_, body, class, id)) in heads.iter().enumerate() {
            let end = heads.get(k + 1).map_or(bulk.len(), |h| h.0);
            chunks.entry(id).or_default().push((class, body..end));
        }
        let mut seg = Segment { classes, bulk, chunks, class_of, listed: Vec::new() };
        if let Some(body) = seg.objects(Kind::Timeline).first().and_then(|&id| seg.body(id)) {
            // The list opens with the design fragment it belongs to.
            seg.listed = seg.refs(body).into_iter().map(|(_, id)| id).skip(1).collect();
        }
        Some(seg)
    }

    /// Debug: the class GUID of an object.
    pub fn guid_of(&self, id: u64) -> Option<&str> {
        Some(self.classes.get(*self.class_of.get(&id)?)?.guid.as_str())
    }

    /// The version of the object's (most derived) class.
    pub fn version_of(&self, id: u64) -> Option<u32> {
        Some(self.classes.get(*self.class_of.get(&id)?)?.version)
    }

    pub fn kind_of(&self, id: u64) -> Option<Kind> {
        self.classes.get(*self.class_of.get(&id)?)?.kind
    }

    /// Ids of every object whose class is `kind`.
    pub fn objects(&self, kind: Kind) -> Vec<u64> {
        let mut ids: Vec<u64> = self.class_of.iter().filter(|(_, &c)| self.classes[c].kind == Some(kind)).map(|(&id, _)| id).collect();
        ids.sort_unstable();
        ids
    }

    /// The body of the object's most-derived chunk.
    pub fn body(&self, id: u64) -> Option<&[u8]> {
        let (_, r) = self.chunks.get(&id)?.first()?;
        Some(&self.bulk[r.clone()])
    }

    /// References (object ids) in a body, in order.
    pub fn refs(&self, body: &[u8]) -> Vec<(usize, u64)> {
        let mut out = Vec::new();
        let mut p = 0;
        while p + 11 <= body.len() {
            if body[p] == 1 && body[p + 9] == 0 && body[p + 10] == 0 {
                if let Some(id) = read_u64(body, p + 1) {
                    if id > 0 && self.class_of.contains_key(&id) {
                        out.push((p, id));
                        p += 11;
                        continue;
                    }
                }
            }
            p += 1;
        }
        out
    }

    fn refs_of(&self, body: &[u8], kind: Kind) -> Vec<u64> {
        self.refs(body).into_iter().map(|(_, id)| id).filter(|&id| self.kind_of(id) == Some(kind)).collect()
    }
}

/// A named value; sketch dimensions and feature inputs are parameters too.
#[derive(Clone, Debug, PartialEq)]
pub struct Parameter {
    pub name: String,
    pub expression: String,
    /// "User Parameter", or what it drives ("AlongDistance", "Linear Dimension-2"...).
    pub role: String,
    pub unit: String,
    /// Internal units: centimetres, radians, or a plain number.
    pub value: f64,
    /// The feature-side object the parameter drives, if any.
    pub owner: Option<u64>,
}

impl Parameter {
    pub fn is_user(&self) -> bool {
        self.owner.is_none()
    }
}

/// A curve in a sketch, in the sketch's own coordinates (centimetres).
#[derive(Clone, Debug, PartialEq)]
pub enum SketchCurve {
    Line { id: u64, a: [f64; 3], b: [f64; 3], construction: bool },
    /// Counter-clockwise about `normal` from `start` to `end` (radians,
    /// measured from `x_dir`); a full circle when the sweep is 2π.
    Arc { id: u64, center: [f64; 3], normal: [f64; 3], x_dir: [f64; 3], radius: f64, start: f64, end: f64, construction: bool },
}

impl SketchCurve {
    pub fn construction(&self) -> bool {
        match self {
            SketchCurve::Line { construction, .. } | SketchCurve::Arc { construction, .. } => *construction,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sketch {
    /// The sketch data object (profiles refer to it).
    pub data: u64,
    /// Row-major 4x4: sketch coordinates to model coordinates.
    pub matrix: [f64; 16],
    pub curves: Vec<SketchCurve>,
}

/// A profile: loops of (curve id, piece id), the outer loop first.
pub type Region = Vec<Vec<(u64, u64)>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Join,
    Cut,
    Intersect,
    NewBody,
    Other(u8),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Extrude {
    pub sketch: u64,
    pub regions: Vec<Region>,
    pub operation: Operation,
    /// +1 along the sketch normal, -1 against it.
    pub direction: f64,
    /// Parameter names (look them up in `Timeline::parameters`).
    pub distance: Option<String>,
    pub against: Option<String>,
    pub taper: Option<String>,
    /// The features that made the bodies a join or cut applies to.
    pub participants: Vec<u64>,
    /// It made a body of its own: a join that touched nothing ends up as a
    /// new body in Fusion.
    pub new_body: bool,
}

/// Model geometry a feature picked (an edge), as Fusion cached it, in
/// centimetres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EdgeGeom {
    /// An edge on the line through `point` along `dir`.
    /// An edge on the segment from `point` to `point + dir`.
    Line { point: [f64; 3], dir: [f64; 3] },
    /// An edge on this circle.
    Circle { center: [f64; 3], normal: [f64; 3], radius: f64 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fillet {
    pub radius: Option<String>,
    pub edges: Vec<EdgeGeom>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Combine {
    /// Creating features of the target and tool bodies.
    pub target: Option<u64>,
    pub tools: Vec<u64>,
    pub operation: Operation,
    pub keep_tools: bool,
}

/// A rotation axis: a sketch curve (sketch data id, curve id), with the
/// line Fusion cached for it.
#[derive(Clone, Debug, PartialEq)]
pub struct CurveRef {
    pub sketch: u64,
    pub curve: u64,
    pub line: Option<EdgeGeom>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Revolve {
    pub sketch: u64,
    pub regions: Vec<Region>,
    pub operation: Operation,
    pub angle: Option<String>,
    pub axis: Option<CurveRef>,
    pub participants: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SplitBody {
    /// Creating feature of the body split.
    pub body: Option<u64>,
    /// The splitting tool, when it's a sketch line.
    pub tool: Option<CurveRef>,
    /// Or when it's a planar face or construction plane: origin and normal,
    /// in centimetres.
    pub plane: Option<([f64; 3], [f64; 3])>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FeatureData {
    Sketch(Sketch),
    Extrude(Extrude),
    Fillet(Fillet),
    Combine(Combine),
    Revolve(Revolve),
    SplitBody(SplitBody),
    /// Present in the timeline but not decoded yet.
    Unsupported,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Feature {
    pub id: u64,
    pub name: String,
    /// Fusion's kind, or the class GUID when we don't know it.
    pub kind: String,
    pub data: FeatureData,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Timeline {
    pub parameters: Vec<Parameter>,
    pub features: Vec<Feature>,
    /// The bodies the design ends with, as Fusion measured them: volume in
    /// mm³ (None when the file doesn't store it).
    pub bodies: Vec<Option<f64>>,
}

impl Timeline {
    pub fn parameter(&self, name: &str) -> Option<&Parameter> {
        self.parameters.iter().find(|p| p.name == name)
    }
}

/// Decode the timeline of a design segment.
pub fn timeline(seg: &Segment) -> Timeline {
    let parameters = seg.objects(Kind::Parameter).into_iter().filter_map(|id| parameter(seg, id)).collect::<Vec<_>>();
    let mut features = Vec::new();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for &id in &seg.listed {
        let kind = seg.kind_of(id);
        let label = kind.map_or_else(|| seg.classes[seg.class_of[&id]].guid.clone(), |k| k.label().to_string());
        let n = counts.entry(label.clone()).or_default();
        *n += 1;
        let data = match kind {
            Some(Kind::SketchFeature) => sketch(seg, id).map_or(FeatureData::Unsupported, FeatureData::Sketch),
            Some(Kind::Extrude) => extrude(seg, id, &parameters).map_or(FeatureData::Unsupported, FeatureData::Extrude),
            Some(Kind::Fillet) => fillet(seg, id, &parameters).map_or(FeatureData::Unsupported, FeatureData::Fillet),
            Some(Kind::Combine) => combine(seg, id).map_or(FeatureData::Unsupported, FeatureData::Combine),
            Some(Kind::Revolve) => revolve(seg, id, &parameters).map_or(FeatureData::Unsupported, FeatureData::Revolve),
            Some(Kind::SplitBody) => split_body(seg, id).map_or(FeatureData::Unsupported, FeatureData::SplitBody),
            _ => FeatureData::Unsupported,
        };
        features.push(Feature { id, name: format!("{label}{n}"), kind: label, data });
    }
    Timeline { parameters, features, bodies: final_bodies(seg) }
}

/// The first body list's bodies, with the volume each stores after its
/// GUID (cm³). Older files store 0 there.
fn final_bodies(seg: &Segment) -> Vec<Option<f64>> {
    let Some(list) = seg.objects(Kind::SolidList).first().and_then(|&id| seg.body(id)) else { return Vec::new() };
    let mut ids: Vec<u64> = Vec::new();
    for b in seg.refs_of(list, Kind::Solid) {
        if !ids.contains(&b) {
            ids.push(b);
        }
    }
    ids.into_iter()
        .map(|b| {
            let body = seg.body(b)?;
            let at = (20..body.len().saturating_sub(80)).find(|&p| read_u32(body, p) == Some(36) && lps16(body, p).is_some_and(|(g, _)| g.len() == 36 && g.matches('-').count() == 4))?;
            let v = read_f64(body, at + 76)?;
            (v > 0.0 && v.is_finite()).then_some(v * 1000.0)
        })
        .collect()
}

/// Whether an object is a timeline entry.
fn is_feature(seg: &Segment, id: u64) -> bool {
    seg.listed.contains(&id)
}

fn parameter(seg: &Segment, id: u64) -> Option<Parameter> {
    let b = seg.body(id)?;
    // [header][u32 index][reference to what it drives, or 00][expression]
    // [gap][role][gap][unit][name][f64 value]. The header and gaps grew in
    // later versions.
    let (index_at, gap1, gap2) = if seg.version_of(id)? >= 7 { (16, 9, 4) } else { (11, 5, 0) };
    let mut p = index_at + 4;
    let owner = if b.get(p) == Some(&1) {
        let o = read_u64(b, p + 1);
        p += 11;
        o
    } else {
        p += 1;
        None
    };
    let (expression, q) = lps16(b, p)?;
    let (role, q) = lps16(b, q + gap1)?;
    let (unit, q) = lps16(b, q + gap2)?;
    let (name, q) = lps16(b, q)?;
    let value = read_f64(b, q)?;
    Some(Parameter { name, expression, role, unit, value, owner })
}

/// Where a named attribute's u64 value ends (`[lps name][lps type][u64]`).
fn after_attribute(b: &[u8], name: &[u8]) -> Option<usize> {
    let at = b.windows(name.len()).position(|w| w == name)?;
    let (_, q) = lps(b, at + name.len())?;
    Some(q + 8)
}

fn point(seg: &Segment, id: u64) -> Option<[f64; 3]> {
    let b = seg.body(id)?;
    let p = after_attribute(b, b"pt_tag")? + 11 + 8;
    Some([read_f64(b, p)?, read_f64(b, p + 8)?, read_f64(b, p + 16)?])
}

fn curve_id(b: &[u8]) -> Option<u64> {
    let at = b.windows(14).position(|w| w == b"crv_primary_id")?;
    let (_, q) = lps(b, at + 14)?;
    read_u64(b, q)
}

fn sketch(seg: &Segment, id: u64) -> Option<Sketch> {
    let feature = seg.body(id)?;
    let data = *seg.refs_of(feature, Kind::SketchData).first()?;
    let matrix = match seg.refs_of(feature, Kind::Matrix).first().and_then(|&m| seg.body(m)) {
        Some(b) => {
            let mut m = [0.0; 16];
            for (k, v) in m.iter_mut().enumerate() {
                *v = read_f64(b, 49 + 8 * k).unwrap_or(0.0);
            }
            if is_rigid(&m) {
                m
            } else {
                identity()
            }
        }
        None => identity(),
    };
    // The data object's second chunk lists the sketch's entities.
    let chunks = seg.chunks.get(&data)?;
    let (_, r) = chunks.get(1)?;
    let list = &seg.bulk[r.clone()];
    let mut curves = Vec::new();
    for (_, e) in seg.refs(list) {
        match seg.kind_of(e) {
            Some(Kind::Line) => {
                let b = seg.body(e)?;
                let ends = seg.refs_of(b, Kind::Point);
                if let (Some(&s), Some(&t), Some(cid)) = (ends.first(), ends.get(1), curve_id(b)) {
                    if let (Some(pa), Some(pb)) = (point(seg, s), point(seg, t)) {
                        curves.push(SketchCurve::Line { id: cid, a: pa, b: pb, construction: construction(seg, e, b) });
                    }
                }
            }
            Some(Kind::Arc) => {
                if let Some(c) = arc(seg, e) {
                    curves.push(c);
                }
            }
            _ => {}
        }
    }
    Some(Sketch { data, matrix, curves })
}

fn arc(seg: &Segment, id: u64) -> Option<SketchCurve> {
    let b = seg.body(id)?;
    let cid = curve_id(b)?;
    // After crv_secondary_id's value: center, normal, x direction, radius,
    // start and end angles.
    let p = after_attribute(b, b"crv_secondary_id")?;
    let v = |k: usize| read_f64(b, p + 8 * k);
    let center = [v(0)?, v(1)?, v(2)?];
    let normal = [v(3)?, v(4)?, v(5)?];
    let x_dir = [v(6)?, v(7)?, v(8)?];
    let (radius, start, end) = (v(9)?, v(10)?, v(11)?);
    let construction = construction(seg, id, b);
    (radius > 0.0).then_some(SketchCurve::Arc { id: cid, center, normal, x_dir, radius, start, end, construction })
}

/// Whether a sketch curve is construction geometry. Its flags follow its
/// last point reference; the curve base class gained a byte in version 9.
/// (Observed in version 8 files, where profiles leave these lines out.)
fn construction(seg: &Segment, id: u64, b: &[u8]) -> bool {
    let Some(&(at, _)) = seg.refs(b).iter().filter(|(_, r)| seg.kind_of(*r) == Some(Kind::Point)).last() else { return false };
    let flags = at + 11;
    let _ = id;
    let curve_version = seg.classes.iter().find(|c| c.guid == CURVE_BASE).map_or(9, |c| c.version);
    // Line style: 0 normal, 1 construction, 2 centerline. Construction is
    // left out of profiles; a centerline bounds them (it's usually the
    // revolve axis along a profile's edge). Version 9 inserted a byte before it.
    let at = if curve_version >= 9 { flags + 7 } else { flags + 6 };
    b.get(at) == Some(&1)
}

/// The curve base class all sketch curves derive from.
const CURVE_BASE: &str = "9FF71F46-205F-484B-A2B5-A245D0D8FE33";

/// A rotation plus translation, last row 0 0 0 1 (anything else is how an
/// identity placement is stored).
fn is_rigid(m: &[f64; 16]) -> bool {
    let row = |r: usize| [m[4 * r], m[4 * r + 1], m[4 * r + 2]];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let ok = m.iter().all(|v| v.is_finite() && v.abs() < 1e7) && m[12] == 0.0 && m[13] == 0.0 && m[14] == 0.0 && m[15] == 1.0;
    ok && (0..3).all(|i| (0..3).all(|j| (dot(row(i), row(j)) - if i == j { 1.0 } else { 0.0 }).abs() < 1e-9))
}

fn identity() -> [f64; 16] {
    let mut m = [0.0; 16];
    for k in 0..4 {
        m[k * 5] = 1.0;
    }
    m
}

/// The profiles a region reference picks: each is loops of (curve id,
/// piece id). Returns the sketch data object they're in too.
fn profiles_of(seg: &Segment, region_ref: u64) -> Option<(u64, Vec<Region>)> {
    let rb = seg.body(region_ref)?;
    let recipe = *seg.refs_of(rb, Kind::RegionRecipe).first()?;
    // The sketch data object's id, written as text.
    let sketch = (0..rb.len().saturating_sub(4)).find_map(|p| {
        let (s, _) = lps16(rb, p)?;
        (!s.is_empty() && s.len() < 12 && s.chars().all(|c| c.is_ascii_digit())).then(|| s.parse().ok()).flatten()
    })?;
    let b = seg.body(recipe)?;
    // [6 bytes][ref back] [u32 profiles], each [u32 loops] {[u32 pieces]
    // pieces [u8 flag]} [u32 0]; a piece is [u32 3][u64 curve][u64 piece]
    // [u32][u32][u32][u64], 40 bytes.
    let mut p = 6 + 11;
    let nprofiles = read_u32(b, p)? as usize;
    p += 4;
    let mut out = Vec::new();
    for _ in 0..nprofiles.min(1024) {
        let nloops = read_u32(b, p)? as usize;
        p += 4;
        let mut loops: Region = Vec::new();
        for _ in 0..nloops.min(1024) {
            let pieces = read_u32(b, p)? as usize;
            p += 4;
            let mut lp = Vec::new();
            for _ in 0..pieces.min(4096) {
                if read_u32(b, p)? != 3 {
                    return None;
                }
                lp.push((read_u64(b, p + 4)?, read_u64(b, p + 12)?));
                p += 40;
            }
            p += 1;
            loops.push(lp);
        }
        p += 4;
        out.push(loops);
    }
    Some((sketch, out))
}

fn extrude(seg: &Segment, id: u64, params: &[Parameter]) -> Option<Extrude> {
    let b = seg.body(id)?;
    let operation = operation(*b.get(13)?);
    let direction = read_f64(b, 47).filter(|d| d.abs() == 1.0).unwrap_or(1.0);
    let mut regions = Vec::new();
    let mut sketch = None;
    for r in seg.refs_of(b, Kind::RegionRef) {
        if let Some((s, profiles)) = profiles_of(seg, r) {
            sketch = Some(s);
            regions.extend(profiles);
        }
    }
    let role = |role: &str| params.iter().find(|p| p.owner.is_some_and(|o| feature_of(seg, o) == Some(id)) && p.role == role).map(|p| p.name.clone());
    let participants = participants(seg, b);
    let new_body = creates_body(seg, id);
    Some(Extrude { sketch: sketch?, regions, operation, direction, distance: role("AlongDistance"), against: role("AgainstDistance"), taper: role("TaperAngle"), participants, new_body })
}

/// Whether feature `id` created a body (some DcBody names it as creator).
fn creates_body(seg: &Segment, id: u64) -> bool {
    seg.objects(Kind::Body).into_iter().any(|body| {
        seg.body(body).and_then(|bb| seg.refs(bb).into_iter().map(|(_, r)| r).find(|&r| is_feature(seg, r))) == Some(id)
    })
}

/// The creating features of the bodies a feature's body references point at.
fn participants(seg: &Segment, b: &[u8]) -> Vec<u64> {
    let mut out = Vec::new();
    for r in seg.refs_of(b, Kind::BodyRef) {
        let Some(rb) = seg.body(r) else { continue };
        for body in seg.refs_of(rb, Kind::Body) {
            let Some(bb) = seg.body(body) else { continue };
            if let Some(&creator) = seg.refs(bb).iter().map(|(_, id)| id).find(|&&id| is_feature(seg, id) && seg.kind_of(id).is_some_and(|k| k != Kind::Body)) {
                if !out.contains(&creator) {
                    out.push(creator);
                }
            }
        }
    }
    out
}

/// The feature a parameter's owner object belongs to (its first reference
/// to a feature).
fn feature_of(seg: &Segment, owner: u64) -> Option<u64> {
    let b = seg.body(owner)?;
    seg.refs(b).into_iter().map(|(_, id)| id).find(|&id| matches!(seg.kind_of(id), Some(k) if !matches!(k, Kind::Parameter)) && is_feature(seg, id))
}

fn operation(byte: u8) -> Operation {
    match byte {
        1 => Operation::Join,
        2 => Operation::Cut,
        3 => Operation::Intersect,
        4 => Operation::NewBody,
        k => Operation::Other(k),
    }
}

/// The parameter driving `role` for feature `id`.
fn role_param(seg: &Segment, params: &[Parameter], id: u64, role: &str) -> Option<String> {
    params.iter().find(|p| p.role == role && p.owner.is_some_and(|o| feature_of(seg, o) == Some(id))).map(|p| p.name.clone())
}

/// Cached edge geometry: a line (point, direction) or a circle (center,
/// normal, x direction, radius, start, end), from byte 6.
fn edge_geom(seg: &Segment, id: u64) -> Option<EdgeGeom> {
    let b = seg.body(id)?;
    let v = |k: usize| read_f64(b, 6 + 8 * k);
    match seg.kind_of(id)? {
        Kind::CachedLine => Some(EdgeGeom::Line { point: [v(0)?, v(1)?, v(2)?], dir: [v(3)?, v(4)?, v(5)?] }),
        Kind::CachedArc => Some(EdgeGeom::Circle { center: [v(0)?, v(1)?, v(2)?], normal: [v(3)?, v(4)?, v(5)?], radius: v(9)? }),
        _ => None,
    }
}

/// The cached geometry of each B-rep reference a feature lists.
fn edges_of(seg: &Segment, b: &[u8]) -> Vec<EdgeGeom> {
    let mut out = Vec::new();
    for r in seg.refs_of(b, Kind::BrepRef) {
        let Some(rb) = seg.body(r) else { continue };
        if let Some(g) = seg.refs(rb).into_iter().find_map(|(_, c)| edge_geom(seg, c)) {
            if !out.contains(&g) {
                out.push(g);
            }
        }
    }
    out
}

fn fillet(seg: &Segment, id: u64, params: &[Parameter]) -> Option<Fillet> {
    let b = seg.body(id)?;
    Some(Fillet { radius: role_param(seg, params, id, "Radius"), edges: edges_of(seg, b) })
}

/// The creator of the body behind a body reference (DRBody -> DcBody -> feature).
fn creator_of_body_ref(seg: &Segment, r: u64) -> Option<u64> {
    let rb = seg.body(r)?;
    let body = *seg.refs_of(rb, Kind::Body).first()?;
    let bb = seg.body(body)?;
    seg.refs(bb).into_iter().map(|(_, id)| id).find(|&id| is_feature(seg, id))
}

fn combine(seg: &Segment, id: u64) -> Option<Combine> {
    let b = seg.body(id)?;
    // The target and tools, in order, through model references.
    let mut bodies = Vec::new();
    for m in seg.refs_of(b, Kind::ModelRef) {
        let Some(mb) = seg.body(m) else { continue };
        for r in seg.refs_of(mb, Kind::BodyRef) {
            if let Some(c) = creator_of_body_ref(seg, r) {
                if !bodies.contains(&c) {
                    bodies.push(c);
                }
            }
        }
    }
    let operation = match *b.get(6)? {
        0 => Operation::Join,
        1 => Operation::Cut,
        2 => Operation::Intersect,
        k => Operation::Other(k),
    };
    // The target is listed last (the body later features keep using).
    let target = bodies.pop();
    Some(Combine { target, tools: bodies, operation, keep_tools: b.get(14) == Some(&1) })
}

/// A sketch curve picked as an axis or tool: [14] sketch data id, [22]
/// curve id in its recipe, plus the line Fusion cached.
fn curve_ref(seg: &Segment, entity_ref: u64) -> Option<CurveRef> {
    let eb = seg.body(entity_ref)?;
    let recipe = *seg.refs_of(eb, Kind::CurveRecipe).first()?;
    let rb = seg.body(recipe)?;
    let line = seg.refs(eb).into_iter().find_map(|(_, c)| edge_geom(seg, c));
    Some(CurveRef { sketch: read_u64(rb, 14)?, curve: read_u64(rb, 22)?, line })
}

fn revolve(seg: &Segment, id: u64, params: &[Parameter]) -> Option<Revolve> {
    let b = seg.body(id)?;
    let mut regions = Vec::new();
    let mut sketch = None;
    for r in seg.refs_of(b, Kind::RegionRef) {
        if let Some((s, profiles)) = profiles_of(seg, r) {
            sketch = Some(s);
            regions.extend(profiles);
        }
    }
    let axis = seg.refs_of(b, Kind::EntityRef).into_iter().find_map(|e| curve_ref(seg, e));
    Some(Revolve {
        sketch: sketch?,
        regions,
        operation: operation(*b.get(10)?),
        angle: role_param(seg, params, id, "AlongAngle"),
        axis,
        participants: participants(seg, b),
    })
}

fn split_body(seg: &Segment, id: u64) -> Option<SplitBody> {
    let b = seg.body(id)?;
    // The body split, through its body reference (a DcBody listed directly
    // is the extra body the split made).
    let body = seg.refs_of(b, Kind::BodyRef).into_iter().find_map(|r| creator_of_body_ref(seg, r));
    let tool = seg.refs_of(b, Kind::EntityRef).into_iter().find_map(|e| curve_ref(seg, e));
    let plane = seg.refs_of(b, Kind::EntityRef).into_iter().find_map(|e| {
        let eb = seg.body(e)?;
        cached_plane(seg, *seg.refs_of(eb, Kind::CachedPlane).first()?)
    });
    Some(SplitBody { body, tool, plane })
}

/// A cached plane: origin, then x and y directions, as f64 from byte 7.
fn cached_plane(seg: &Segment, id: u64) -> Option<([f64; 3], [f64; 3])> {
    let b = seg.body(id)?;
    let v = |k: usize| read_f64(b, 7 + 8 * k);
    let [o, x, y] = [0, 3, 6].map(|k| [v(k), v(k + 1), v(k + 2)]);
    let (o, x, y) = (o.map(Option::unwrap_or_default), x.map(Option::unwrap_or_default), y.map(Option::unwrap_or_default));
    let n = [x[1] * y[2] - x[2] * y[1], x[2] * y[0] - x[0] * y[2], x[0] * y[1] - x[1] * y[0]];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    (len > 0.5).then(|| (o, n.map(|c| c / len)))
}

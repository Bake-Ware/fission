//! Tokenizer for ShapeManager's binary B-rep blobs ("ASM BinaryFile4"),
//! the ACIS SAB family: a 15-byte magic, four little-endian u32s, then a
//! stream of tagged values. A record is one entity (`body`, `face`,
//! `plane-surface`, ...) ending at the `#` tag; pointers are record indices.

use crate::F3dError;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i32),
    Double(f64),
    Str(String),
    /// Record index; -1 is null.
    Ptr(i32),
    /// Entity type name (`body`, `surface`, ...).
    Id(String),
    /// Subtype prefix (`plane` in `plane-surface`).
    Sub(String),
    True,
    False,
    Pos([f64; 3]),
    Vec([f64; 3]),
    Enum(i32),
    Par([f64; 2]),
    Open,
    Close,
    /// Long string payloads we don't need (length only).
    Blob(usize),
}

#[derive(Clone, Debug, Default)]
pub struct Record {
    /// Full type, subtypes first: e.g. `["plane", "surface"]`.
    pub kind: Vec<String>,
    pub values: Vec<Value>,
}

impl Record {
    pub fn is(&self, name: &str) -> bool {
        self.kind.last().is_some_and(|k| k == name)
    }

    pub fn subtype(&self) -> Option<&str> {
        (self.kind.len() > 1).then(|| self.kind[0].as_str())
    }
}

pub struct Header {
    pub version: u32,
    /// Millimetres per model unit (Fusion stores centimetres: 10).
    pub mm_per_unit: f64,
}

/// Parse a blob into its header and records (record 0 is `asmheader`).
pub fn parse(data: &[u8]) -> Result<(Header, Vec<Record>), F3dError> {
    const MAGIC: &[u8] = b"ASM BinaryFile4";
    if !data.starts_with(MAGIC) {
        return Err(F3dError::Format("not an ASM binary B-rep".into()));
    }
    let u32_at = |i: usize| data.get(i..i + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    let version = u32_at(MAGIC.len()).ok_or(F3dError::Truncated)?;
    let mut pos = MAGIC.len() + 16;

    // Header values: product, version string, date, then units and tolerances.
    let mut header_values = Vec::new();
    while header_values.iter().filter(|v| matches!(v, Value::Double(_))).count() < 3 {
        header_values.push(read(data, &mut pos)?);
    }
    let mm_per_unit = header_values.iter().find_map(|v| if let Value::Double(d) = v { Some(*d) } else { None }).unwrap_or(1.0);

    let mut records = Vec::new();
    let mut current = Record::default();
    while pos < data.len() {
        match read(data, &mut pos)? {
            // Type names lead a record; later ones name nested data
            // (e.g. a spline's subtype) and stay in place.
            Value::Id(name) if current.values.is_empty() => current.kind.push(name),
            v @ Value::Id(_) => current.values.push(v),
            Value::Sub(name) if current.values.is_empty() => current.kind.push(name),
            Value::Sub(name) => current.values.push(Value::Sub(name)),
            v @ Value::Close if current.values.is_empty() && current.kind.is_empty() => current.values.push(v),
            Value::Close => current.values.push(Value::Close),
            other if is_end(&other) => {
                records.push(std::mem::take(&mut current));
            }
            other => current.values.push(other),
        }
    }
    Ok((Header { version, mm_per_unit }, records))
}

fn is_end(v: &Value) -> bool {
    matches!(v, Value::Enum(i32::MIN))
}

fn read(data: &[u8], pos: &mut usize) -> Result<Value, F3dError> {
    let take = |pos: &mut usize, n: usize| -> Result<&[u8], F3dError> {
        let s = data.get(*pos..*pos + n).ok_or(F3dError::Truncated)?;
        *pos += n;
        Ok(s)
    };
    let f64s = |b: &[u8]| -> Vec<f64> { b.chunks_exact(8).map(|c| f64::from_le_bytes(c.try_into().unwrap())).collect() };
    let tag = take(pos, 1)?[0];
    Ok(match tag {
        0x04 => Value::Int(i32::from_le_bytes(take(pos, 4)?.try_into().unwrap())),
        0x06 => Value::Double(f64s(take(pos, 8)?)[0]),
        0x07 | 0x0D | 0x0E => {
            let n = take(pos, 1)?[0] as usize;
            let s = String::from_utf8_lossy(take(pos, n)?).into_owned();
            match tag {
                0x07 => Value::Str(s),
                0x0D => Value::Id(s),
                _ => Value::Sub(s),
            }
        }
        0x08 => {
            let n = u16::from_le_bytes(take(pos, 2)?.try_into().unwrap()) as usize;
            Value::Str(String::from_utf8_lossy(take(pos, n)?).into_owned())
        }
        0x09 => {
            let n = u32::from_le_bytes(take(pos, 4)?.try_into().unwrap()) as usize;
            take(pos, n)?;
            Value::Blob(n)
        }
        0x0A => Value::True,
        0x0B => Value::False,
        0x0C => Value::Ptr(i32::from_le_bytes(take(pos, 4)?.try_into().unwrap())),
        0x0F => Value::Open,
        0x10 => Value::Close,
        // End of record: reuse an impossible enum as the marker.
        0x11 => Value::Enum(i32::MIN),
        0x13 => {
            let v = f64s(take(pos, 24)?);
            Value::Pos([v[0], v[1], v[2]])
        }
        0x14 => {
            let v = f64s(take(pos, 24)?);
            Value::Vec([v[0], v[1], v[2]])
        }
        0x15 => Value::Enum(i32::from_le_bytes(take(pos, 4)?.try_into().unwrap())),
        0x16 => {
            let v = f64s(take(pos, 16)?);
            Value::Par([v[0], v[1]])
        }
        other => return Err(F3dError::Format(format!("unknown tag 0x{other:02x} at byte {}", *pos - 1))),
    })
}

/// Sequential field reader over a record's values.
pub struct Fields<'a> {
    values: &'a [Value],
    at: usize,
}

impl<'a> Fields<'a> {
    pub fn new(record: &'a Record) -> Self {
        Fields { values: &record.values, at: 0 }
    }

    fn next(&mut self) -> Option<&'a Value> {
        let v = self.values.get(self.at);
        self.at += 1;
        v
    }

    pub fn skip(&mut self, n: usize) -> &mut Self {
        self.at += n;
        self
    }

    pub fn ptr(&mut self) -> Option<usize> {
        match self.next()? {
            Value::Ptr(p) if *p >= 0 => Some(*p as usize),
            _ => None,
        }
    }

    pub fn double(&mut self) -> Option<f64> {
        match self.next()? {
            Value::Double(d) => Some(*d),
            Value::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    pub fn point(&mut self) -> Option<[f64; 3]> {
        match self.next()? {
            Value::Pos(p) | Value::Vec(p) => Some(*p),
            _ => None,
        }
    }

    pub fn logical(&mut self) -> Option<bool> {
        match self.next()? {
            Value::True => Some(true),
            Value::False => Some(false),
            _ => None,
        }
    }

    /// An optional parameter bound: `F`, or `T` followed by a value.
    pub fn bound(&mut self) -> Option<Option<f64>> {
        match self.next()? {
            Value::False => Some(None),
            Value::True => Some(self.double()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(body: &[u8]) -> Vec<u8> {
        let mut d = b"ASM BinaryFile4".to_vec();
        d.extend(22700u32.to_le_bytes());
        d.extend([0u8; 12]);
        // Header: product string, units 10, two tolerances.
        d.extend([0x07, 1, b'x']);
        for v in [10.0f64, 1e-6, 1e-10] {
            d.push(0x06);
            d.extend(v.to_le_bytes());
        }
        d.extend(body);
        d
    }

    #[test]
    fn parses_records_and_header() {
        let mut body = vec![0x0D, 4];
        body.extend(b"body");
        body.extend([0x0C]);
        body.extend((-1i32).to_le_bytes());
        body.push(0x11);
        body.extend([0x0E, 5]);
        body.extend(b"plane");
        body.extend([0x0D, 7]);
        body.extend(b"surface");
        body.push(0x13);
        for v in [1.0f64, 2.0, 3.0] {
            body.extend(v.to_le_bytes());
        }
        body.push(0x0A);
        body.push(0x11);
        let (h, records) = parse(&blob(&body)).unwrap();
        assert_eq!(h.version, 22700);
        assert_eq!(h.mm_per_unit, 10.0);
        assert_eq!(records.len(), 2);
        assert!(records[0].is("body"));
        assert_eq!(records[1].kind, ["plane", "surface"]);
        assert_eq!(records[1].subtype(), Some("plane"));
        let mut f = Fields::new(&records[1]);
        assert_eq!(f.point(), Some([1.0, 2.0, 3.0]));
        assert_eq!(f.logical(), Some(true));
    }

    #[test]
    fn rejects_unknown_tags_and_bad_magic() {
        assert!(parse(b"nope").is_err());
        assert!(parse(&blob(&[0xEE])).is_err());
    }
}

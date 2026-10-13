//! The geometry kernel runs in a child process (`fission-app kernel-worker`).
//!
//! OCCT has crash bugs (segfaults, not exceptions) on some valid inputs. In a
//! worker, a crash costs a restart instead of the user's session: the client
//! notices the dead connection, bisects the timeline to find the feature that
//! takes the kernel down, marks it failed, and rebuilds without it.
//!
//! Transport is loopback TCP with length-prefixed postcard frames. Not stdio,
//! because OCCT prints to stdout.

use std::io::{self, BufReader, BufWriter, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use fission_kernel::{Kernel as _, Mesh};
use fission_model::{Document, FeatureStatus};
use fission_occt::Occt;
use serde::{Deserialize, Serialize};

/// Linear deflection for display meshes, in mm.
pub const DISPLAY_TOLERANCE: f64 = 0.05;

#[derive(Serialize, Deserialize)]
enum Request {
    /// Regenerate `doc`; with `base_marker`, also the model up to that feature.
    Regen { doc: String, base_marker: Option<usize> },
    /// Regenerate and export bodies whose creating feature isn't in `hidden`.
    Export { doc: String, hidden: Vec<usize>, path: String },
    /// Sweep a joint (by name; empty: the only one) through `values`.
    Sweep { doc: String, joint: String, values: Vec<f64>, tolerance: f64 },
}

#[derive(Clone, Serialize, Deserialize)]
pub struct MeshBody {
    pub name: String,
    pub created_by: usize,
    pub mesh: Mesh,
}

#[derive(Serialize, Deserialize)]
enum Reply {
    Regen { status: Vec<FeatureStatus>, parts: Vec<usize>, part_faces: Vec<Vec<(usize, usize)>>, bodies: Vec<MeshBody>, base: Option<Vec<MeshBody>>, errors: Vec<String> },
    Export(Result<(), String>),
    Sweep(Result<fission_model::motion::Sweep, String>),
}

pub struct RegenResult {
    pub status: Vec<FeatureStatus>,
    /// Per feature: parts an import brought in.
    pub parts: Vec<usize>,
    pub part_faces: Vec<Vec<(usize, usize)>>,
    pub bodies: Vec<MeshBody>,
    pub base: Option<Vec<MeshBody>>,
    pub errors: Vec<String>,
}

fn write_frame(w: &mut impl Write, value: &impl Serialize) -> io::Result<()> {
    let bytes = postcard::to_stdvec(value).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    w.write_all(&(bytes.len() as u32).to_le_bytes())?;
    w.write_all(&bytes)?;
    w.flush()
}

fn read_frame<T: for<'de> Deserialize<'de>>(r: &mut impl Read) -> io::Result<T> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let mut buf = vec![0u8; u32::from_le_bytes(len) as usize];
    r.read_exact(&mut buf)?;
    postcard::from_bytes(&buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

// ---- worker side ----

/// `fission-app kernel-worker <port>`: serve kernel requests until the app hangs up.
pub fn run_worker(port: &str) -> ExitCode {
    let Ok(stream) = TcpStream::connect(("127.0.0.1", port.parse::<u16>().unwrap_or(0))) else {
        return ExitCode::FAILURE;
    };
    let _ = stream.set_nodelay(true);
    let mut reader = BufReader::new(stream.try_clone().expect("clone socket"));
    let mut writer = BufWriter::new(stream);
    let kernel = Occt;
    while let Ok(request) = read_frame::<Request>(&mut reader) {
        let reply = match request {
            Request::Regen { doc, base_marker } => regen(&kernel, &doc, base_marker),
            Request::Export { doc, hidden, path } => Reply::Export(export(&kernel, &doc, &hidden, Path::new(&path))),
            Request::Sweep { doc, joint, values, tolerance } => Reply::Sweep(parse(&doc).and_then(|doc| {
                let joint = doc.joint(&joint)?;
                doc.sweep_joint(&kernel, &joint, &values, tolerance)
            })),
        };
        if write_frame(&mut writer, &reply).is_err() {
            break;
        }
    }
    ExitCode::SUCCESS
}

fn parse(doc: &str) -> Result<Document, String> {
    Document::from_ron(doc).map_err(|e| e.to_string())
}

fn mesh_bodies(kernel: &Occt, bodies: Vec<fission_model::Body<fission_occt::Solid>>, errors: &mut Vec<String>) -> Vec<MeshBody> {
    bodies
        .into_iter()
        .map(|b| {
            let mesh = kernel.tessellate(&b.solid, DISPLAY_TOLERANCE).unwrap_or_else(|e| {
                errors.push(format!("{}: {e}", b.name));
                Mesh::default()
            });
            if mesh.skipped_faces > 0 {
                errors.push(format!("{}: {} face(s) couldn't be displayed", b.name, mesh.skipped_faces));
            }
            MeshBody { name: b.name, created_by: b.created_by, mesh }
        })
        .collect()
}

fn regen(kernel: &Occt, doc: &str, base_marker: Option<usize>) -> Reply {
    let doc = match parse(doc) {
        Ok(doc) => doc,
        Err(e) => return Reply::Regen { status: vec![], parts: vec![], part_faces: vec![], bodies: vec![], base: None, errors: vec![e] },
    };
    let mut errors = Vec::new();
    let full = doc.regenerate(kernel);
    let bodies = mesh_bodies(kernel, full.bodies, &mut errors);
    let base = base_marker.map(|m| {
        let mut d = doc.clone();
        d.marker = m.min(d.features.len());
        mesh_bodies(kernel, d.regenerate(kernel).bodies, &mut errors)
    });
    Reply::Regen { status: full.status, parts: full.parts, part_faces: full.part_faces, bodies, base, errors }
}

fn export(kernel: &Occt, doc: &str, hidden: &[usize], path: &Path) -> Result<(), String> {
    let doc = parse(doc)?;
    let regen = doc.regenerate(kernel);
    let bodies: Vec<_> =
        regen.bodies.iter().filter(|b| !hidden.contains(&b.created_by)).map(|b| (b.name.clone(), b.solid.clone())).collect();
    crate::app::export(kernel, &bodies, path)
}

// ---- app side ----

/// The worker died mid-request (most likely an OCCT crash).
#[derive(Debug)]
pub struct Crashed;

pub struct KernelClient {
    child: Child,
    reader: BufReader<TcpStream>,
    writer: BufWriter<TcpStream>,
}

impl KernelClient {
    pub fn spawn() -> io::Result<KernelClient> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let mut child = Command::new(std::env::current_exe()?)
            .arg("kernel-worker")
            .arg(port.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()?;
        // Wait for the worker to connect, but not forever.
        listener.set_nonblocking(true)?;
        let deadline = Instant::now() + Duration::from_secs(10);
        let stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    if Instant::now() > deadline || child.try_wait()?.is_some() {
                        let _ = child.kill();
                        return Err(io::Error::new(io::ErrorKind::TimedOut, "kernel worker didn't start"));
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => return Err(e),
            }
        };
        stream.set_nonblocking(false)?;
        let _ = stream.set_nodelay(true);
        Ok(KernelClient { child, reader: BufReader::new(stream.try_clone()?), writer: BufWriter::new(stream) })
    }

    fn call(&mut self, request: &Request) -> Result<Reply, Crashed> {
        if write_frame(&mut self.writer, request).is_err() {
            return Err(Crashed);
        }
        read_frame(&mut self.reader).map_err(|_| Crashed)
    }

    fn regen_once(&mut self, doc: &Document, base_marker: Option<usize>) -> Result<RegenResult, Crashed> {
        let text = doc.to_ron().map_err(|_| Crashed)?;
        match self.call(&Request::Regen { doc: text, base_marker })? {
            Reply::Regen { status, parts, part_faces, bodies, base, errors } => {
                Ok(RegenResult { status, parts, part_faces, bodies, base, errors })
            }
            _ => Err(Crashed),
        }
    }

    pub fn export(&mut self, doc: &Document, hidden: &[usize], path: &Path) -> Result<Result<(), String>, Crashed> {
        let text = doc.to_ron().map_err(|_| Crashed)?;
        match self.call(&Request::Export { doc: text, hidden: hidden.to_vec(), path: path.display().to_string() })? {
            Reply::Export(result) => Ok(result),
            _ => Err(Crashed),
        }
    }
}

impl KernelClient {
    pub fn sweep(&mut self, doc: &Document, joint: &str, values: &[f64], tolerance: f64) -> Result<Result<fission_model::motion::Sweep, String>, Crashed> {
        let text = doc.to_ron().map_err(|_| Crashed)?;
        match self.call(&Request::Sweep { doc: text, joint: joint.into(), values: values.to_vec(), tolerance })? {
            Reply::Sweep(result) => Ok(result),
            _ => Err(Crashed),
        }
    }
}

impl Drop for KernelClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Owns the worker and survives its crashes.
pub struct Kernel {
    client: Option<KernelClient>,
    /// Features found to crash the kernel: index plus a hash of the timeline
    /// up to and including it. Kernel results are deterministic, so a crasher
    /// stays skipped until something at or before it changes.
    crashers: Vec<(usize, u64)>,
}

fn prefix_hash(doc: &Document, index: usize) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let end = (index + 1).min(doc.features.len());
    ron::to_string(&doc.features[..end]).unwrap_or_default().hash(&mut h);
    h.finish()
}

const CRASH_MESSAGE: &str = "the geometry kernel crashed on this feature (an OpenCascade bug); it's skipped";

impl Kernel {
    pub fn new() -> Kernel {
        Kernel { client: None, crashers: Vec::new() }
    }

    fn client(&mut self) -> Result<&mut KernelClient, String> {
        if self.client.is_none() {
            self.client = Some(KernelClient::spawn().map_err(|e| format!("can't start the kernel worker: {e}"))?);
        }
        Ok(self.client.as_mut().unwrap())
    }

    /// Crashers that still apply to `doc`.
    fn active_crashers(&self, doc: &Document) -> Vec<usize> {
        self.crashers.iter().filter(|&&(i, h)| i < doc.features.len() && prefix_hash(doc, i) == h).map(|&(i, _)| i).collect()
    }

    /// `doc` with known crashers suppressed.
    fn safe_doc(&self, doc: &Document) -> Document {
        let mut d = doc.clone();
        for i in self.active_crashers(doc) {
            d.features[i].suppressed = true;
        }
        d
    }

    fn try_regen(&mut self, doc: &Document, base_marker: Option<usize>) -> Result<RegenResult, Crashed> {
        let result = self.client().map_err(|_| Crashed)?.regen_once(doc, base_marker);
        if result.is_err() {
            self.client = None; // dead; respawn next time
        }
        result
    }

    /// Regenerate in the worker. If it crashes, find the feature responsible
    /// (the shortest crashing timeline prefix), skip it, and try again.
    pub fn regenerate(&mut self, doc: &Document, base_marker: Option<usize>) -> Result<RegenResult, String> {
        for _ in 0..8 {
            let safe = self.safe_doc(doc);
            match self.try_regen(&safe, base_marker) {
                Ok(mut result) => {
                    for i in self.active_crashers(doc) {
                        if let Some(s) = result.status.get_mut(i) {
                            *s = FeatureStatus::Error(CRASH_MESSAGE.into());
                        }
                    }
                    return Ok(result);
                }
                Err(Crashed) => {
                    let culprit = self.find_crasher(&safe, base_marker)?;
                    log::warn!("kernel crashed on feature {culprit}; skipping it");
                    self.crashers.push((culprit, prefix_hash(doc, culprit)));
                }
            }
        }
        Err("the geometry kernel keeps crashing".into())
    }

    /// Binary search for the shortest prefix that crashes; its last feature is the culprit.
    fn find_crasher(&mut self, doc: &Document, base_marker: Option<usize>) -> Result<usize, String> {
        let end = doc.marker.min(doc.features.len());
        let crashes = |k: &mut Kernel, m: usize| {
            let mut d = doc.clone();
            d.marker = m;
            k.try_regen(&d, base_marker.map(|b| b.min(m))).is_err()
        };
        if !crashes(self, end) {
            // Only the base model crashed; search over it instead.
            let base = base_marker.ok_or("the kernel crashed but no feature reproduces it")?;
            let mut d = doc.clone();
            d.marker = base;
            return self.find_crasher(&d, None);
        }
        let (mut ok, mut bad) = (0usize, end); // prefix `ok` works, prefix `bad` crashes
        while bad - ok > 1 {
            let mid = (ok + bad) / 2;
            if crashes(self, mid) {
                bad = mid;
            } else {
                ok = mid;
            }
        }
        Ok(bad - 1)
    }

    /// Sweep a joint for interference (see `Document::sweep_joint`).
    pub fn sweep(&mut self, doc: &Document, joint: &str, values: &[f64], tolerance: f64) -> Result<fission_model::motion::Sweep, String> {
        let safe = self.safe_doc(doc);
        match self.client()?.sweep(&safe, joint, values, tolerance) {
            Ok(result) => result,
            Err(Crashed) => {
                self.client = None;
                Err("the geometry kernel crashed while checking interference".into())
            }
        }
    }

    pub fn export(&mut self, doc: &Document, hidden: &[usize], path: &Path) -> Result<(), String> {
        let safe = self.safe_doc(doc);
        match self.client()?.export(&safe, hidden, path) {
            Ok(result) => result,
            Err(Crashed) => {
                self.client = None;
                Err("the geometry kernel crashed while exporting".into())
            }
        }
    }
}

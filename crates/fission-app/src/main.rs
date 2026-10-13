mod animate;
mod app;
mod export;
mod motion;
mod png;
mod ui;
mod viewport;
mod worker;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use fission_model::Document;

const USAGE: &str = "\
usage:
  fission-app [design.fission]          open the editor
  fission-app export <design|design.f3d|--sample> <out.stl|out.3mf|out.obj|out.step>
                                        regenerate a design and export it headless
  fission-app interfere <design> [--joint NAME] [--from A] [--to B] [--steps N]
                        [--tolerance MM3] [--json]
                                        sweep a joint (default: its limits) and report
                                        where its bodies hit the rest; exit 3 if they do
  fission-app animate <design> --out <dir|out.gif|out.mp4> [--joint NAME] [--from A]
                      [--to B] [--frames N] [--pingpong] [--view home|front|top|right|...]
                      [--size WxH] [--fps N] [--dark]
                                        render a joint sweep headless (gif/mp4 need ffmpeg)
  fission-app install-desktop           add a launcher + icon to ~/.local/share";

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn"))
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("export") => match (args.get(1), args.get(2)) {
            (Some(input), Some(output)) => export_cli(input, Path::new(output)),
            _ => usage_error(),
        },
        Some("interfere") => interfere_cli(&args[1..]),
        Some("animate") => animate_cli(&args[1..]),
        Some("install-desktop") => install_desktop(),
        Some("kernel-worker") => worker::run_worker(args.get(1).map_or("", String::as_str)),
        Some("-h" | "--help") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(path) if !path.starts_with('-') => run_gui(Some(PathBuf::from(path))),
        Some(_) => usage_error(),
        None => run_gui(None),
    }
}

fn usage_error() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

fn run_gui(path: Option<PathBuf>) -> ExitCode {
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Fission")
            .with_app_id("fission")
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([800.0, 500.0])
            .with_icon(app_icon()),
        ..Default::default()
    };
    let result = eframe::run_native("Fission", options, Box::new(|cc| Ok(Box::new(app::FissionApp::new(cc, path)))));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("fission: {e}");
            ExitCode::FAILURE
        }
    }
}

/// A cube split in two along a glowing cut, on a dark rounded tile
/// (`assets/icon.svg`, rasterized to raw RGBA).
fn app_icon() -> eframe::egui::IconData {
    const N: u32 = 256;
    let rgba = include_bytes!("../assets/icon-256.rgba").to_vec();
    debug_assert_eq!(rgba.len(), (N * N * 4) as usize);
    eframe::egui::IconData { rgba, width: N, height: N }
}

/// Wayland compositors take the window icon from a .desktop file matching the
/// app id ("fission"), so write one plus the icon as a PNG.
fn install_desktop() -> ExitCode {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        eprintln!("fission: HOME is not set");
        return ExitCode::FAILURE;
    };
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            eprintln!("fission: can't find own path: {e}");
            return ExitCode::FAILURE;
        }
    };
    let icon = app_icon();
    let icon_path = home.join(".local/share/icons/hicolor/256x256/apps/fission.png");
    let desktop_path = home.join(".local/share/applications/fission.desktop");
    let desktop = format!(
        "[Desktop Entry]\nType=Application\nName=Fission\nComment=Parametric CAD\nExec={} %f\nIcon=fission\n\
         Terminal=false\nCategories=Graphics;Engineering;3DGraphics;\nMimeType=application/x-fission;\n\
         StartupWMClass=fission\n",
        exe.display()
    );
    let result = (|| -> std::io::Result<()> {
        // Earlier versions wrote a 64 px icon that would shadow this one.
        let _ = std::fs::remove_file(home.join(".local/share/icons/hicolor/64x64/apps/fission.png"));
        std::fs::create_dir_all(icon_path.parent().unwrap())?;
        std::fs::write(&icon_path, png::encode_rgba(icon.width, icon.height, &icon.rgba))?;
        std::fs::create_dir_all(desktop_path.parent().unwrap())?;
        std::fs::write(&desktop_path, desktop)
    })();
    match result {
        Ok(()) => {
            println!("wrote {}\nwrote {}", desktop_path.display(), icon_path.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("fission: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Read a design (.fission, a Fusion .f3d, or `--sample`) with its
/// parameters applied.
fn load_design(input: &str, kernel: &mut worker::Kernel) -> Result<Document, String> {
    let mut doc = if input == "--sample" {
        Document::sample()
    } else if input.to_ascii_lowercase().ends_with(".f3d") {
        // A Fusion design: replay its timeline.
        let timeline = fission_f3d::read_timeline(Path::new(input)).map_err(|e| format!("{input}: {e}"))?;
        let name = Path::new(input).file_stem().and_then(|s| s.to_str()).unwrap_or("Imported");
        let mut rebuild = |doc: &Document| match kernel.regenerate(doc, None) {
            Ok(r) => r.bodies.into_iter().map(|b| (b.created_by, b.mesh)).collect(),
            Err(_) => Vec::new(),
        };
        fission_model::f3d::document_with(&timeline, name, &mut rebuild).doc
    } else {
        let text = std::fs::read_to_string(input).map_err(|e| format!("can't read {input}: {e}"))?;
        Document::from_ron(&text).map_err(|e| format!("{input}: {e}"))?
    };
    for problem in doc.apply_expressions() {
        eprintln!("fission: {problem}");
    }
    Ok(doc)
}

/// `--name value` options after a positional design argument.
struct Flags {
    design: Option<String>,
    values: std::collections::HashMap<String, String>,
    switches: Vec<String>,
}

fn flags(args: &[String], switches: &[&str]) -> Result<Flags, String> {
    let mut out = Flags { design: None, values: Default::default(), switches: Vec::new() };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if let Some(name) = a.strip_prefix("--").filter(|&n| n != "sample") {
            if switches.contains(&name) {
                out.switches.push(name.into());
            } else {
                let v = it.next().ok_or(format!("--{name} needs a value"))?;
                out.values.insert(name.into(), v.clone());
            }
        } else if out.design.is_none() {
            out.design = Some(a.clone());
        } else {
            return Err(format!("unexpected argument '{a}'"));
        }
    }
    Ok(out)
}

impl Flags {
    fn num<T: std::str::FromStr>(&self, name: &str) -> Result<Option<T>, String> {
        self.values.get(name).map(|v| v.parse().map_err(|_| format!("--{name}: '{v}' isn't a number"))).transpose()
    }
    fn str(&self, name: &str) -> &str {
        self.values.get(name).map_or("", String::as_str)
    }
    fn on(&self, name: &str) -> bool {
        self.switches.iter().any(|s| s == name)
    }
}

fn fail(message: impl std::fmt::Display) -> ExitCode {
    eprintln!("fission: {message}");
    ExitCode::FAILURE
}

fn interfere_cli(args: &[String]) -> ExitCode {
    let run = || -> Result<ExitCode, String> {
        let f = flags(args, &["json"])?;
        let design = f.design.clone().ok_or("interfere needs a design")?;
        let mut kernel = worker::Kernel::new();
        let doc = load_design(&design, &mut kernel)?;
        let joint = doc.joint(f.str("joint"))?;
        let from = f.num("from")?.unwrap_or(joint.min);
        let to = f.num("to")?.unwrap_or(joint.max);
        let steps = f.num("steps")?.unwrap_or(25usize);
        let tolerance = f.num("tolerance")?.unwrap_or(motion::INTERFERENCE_TOLERANCE);
        let values = fission_model::motion::steps(from, to, steps);
        let sweep = kernel.sweep(&doc, &joint.name, &values, tolerance)?;
        let collides = sweep.collides(tolerance);
        if f.on("json") {
            println!("{}", serde_json_like(&sweep, collides));
        } else {
            let unit = if joint.motion == fission_model::JointMotion::Revolute { "deg" } else { "mm" };
            println!("{} ({}, limits {} to {} {unit}, built at {}): {} steps from {from} to {to}", joint.name, joint.motion.label().to_lowercase(), joint.min, joint.max, joint.value, values.len());
            for s in &sweep.samples {
                let pairs: Vec<String> = s.pairs.iter().map(|(a, b, v)| format!("{a} x {b} {v:.2}")).collect();
                let mark = if s.overlap > tolerance { "HIT  " } else { "clear" };
                println!("  {:>9.3} {unit}  {mark} {:>10.2} mm3  {}", s.value, s.overlap, pairs.join("; "));
            }
            match sweep.clear {
                Some((lo, hi)) => println!("clear from {lo} to {hi} {unit}"),
                None => println!("collides at its built value"),
            }
            if collides {
                let worst = sweep.samples.iter().max_by(|a, b| a.overlap.total_cmp(&b.overlap)).unwrap();
                println!("worst overlap {:.2} mm3 at {} {unit}", worst.overlap, worst.value);
            } else {
                println!("no interference");
            }
        }
        Ok(if collides { ExitCode::from(3) } else { ExitCode::SUCCESS })
    };
    run().unwrap_or_else(fail)
}

/// The sweep as JSON (hand-written: the app has no JSON dependency).
fn serde_json_like(sweep: &fission_model::motion::Sweep, collides: bool) -> String {
    let q = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let samples: Vec<String> = sweep
        .samples
        .iter()
        .map(|s| {
            let pairs: Vec<String> = s.pairs.iter().map(|(a, b, v)| format!("{{\"moving\":{},\"fixed\":{},\"overlap\":{v}}}", q(a), q(b))).collect();
            format!("{{\"value\":{},\"overlap\":{},\"pairs\":[{}]}}", s.value, s.overlap, pairs.join(","))
        })
        .collect();
    let clear = sweep.clear.map_or("null".into(), |(lo, hi)| format!("[{lo},{hi}]"));
    format!("{{\"joint\":{},\"collides\":{collides},\"clear\":{clear},\"samples\":[{}]}}", q(&sweep.joint), samples.join(","))
}

fn animate_cli(args: &[String]) -> ExitCode {
    let run = || -> Result<ExitCode, String> {
        let f = flags(args, &["pingpong", "dark"])?;
        let design = f.design.clone().ok_or("animate needs a design")?;
        let out = f.values.get("out").ok_or("animate needs --out <dir|file.gif|file.mp4>")?;
        let size = match f.values.get("size") {
            Some(s) => {
                let (w, h) = s.split_once(['x', 'X']).ok_or("--size is WxH, e.g. 960x640")?;
                [w.parse().map_err(|_| "--size width")?, h.parse().map_err(|_| "--size height")?]
            }
            None => [960, 640],
        };
        let view = match f.values.get("view") {
            Some(v) => viewport::StandardView::parse(v).ok_or(format!("unknown view '{v}'"))?,
            None => viewport::StandardView::Home,
        };
        let opts = animate::Options {
            joint: f.str("joint").into(),
            from: f.num("from")?,
            to: f.num("to")?,
            frames: f.num("frames")?.unwrap_or(30),
            pingpong: f.on("pingpong"),
            view,
            size,
            fps: f.num("fps")?.unwrap_or(15),
            dark: f.on("dark"),
            out: PathBuf::from(out),
        };
        let mut kernel = worker::Kernel::new();
        let doc = load_design(&design, &mut kernel)?;
        println!("{}", animate::run(&doc, &mut kernel, &opts)?);
        Ok(ExitCode::SUCCESS)
    };
    run().unwrap_or_else(fail)
}

fn export_cli(input: &str, output: &Path) -> ExitCode {
    // All kernel work goes through the worker process, so a design that
    // crashes OpenCascade loses only the crashing feature, as in the editor.
    let mut kernel = worker::Kernel::new();
    let doc = match load_design(input, &mut kernel) {
        Ok(doc) => doc,
        Err(e) => return fail(e),
    };
    let regen = match kernel.regenerate(&doc, None) {
        Ok(regen) => regen,
        Err(e) => {
            eprintln!("fission: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut failed = false;
    for (feature, status) in doc.features.iter().zip(&regen.status) {
        if let fission_model::FeatureStatus::Error(message) = status {
            eprintln!("fission: {}: {message}", feature.name);
            failed = true;
        }
    }
    match kernel.export(&doc, &[], output) {
        Ok(()) => {
            println!("exported {} bodies to {}", regen.bodies.len(), output.display());
            if failed { ExitCode::from(3) } else { ExitCode::SUCCESS }
        }
        Err(e) => {
            eprintln!("fission: {e}");
            ExitCode::FAILURE
        }
    }
}

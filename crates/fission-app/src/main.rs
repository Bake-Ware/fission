mod app;
mod export;
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

/// A shaded isometric cube on a rounded blue tile, drawn procedurally.
fn app_icon() -> eframe::egui::IconData {
    const N: usize = 64;
    let mut rgba = vec![0u8; N * N * 4];
    let c = N as f32 / 2.0;
    for y in 0..N {
        for x in 0..N {
            let (px, py) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
            // Rounded-square tile.
            let q = (px.abs() - 26.0).max(0.0).hypot((py.abs() - 26.0).max(0.0));
            if q > 5.0 {
                continue;
            }
            let mut color = [6u8, 150, 215];
            // Pointy-top hexagon (circumradius 1) split at its center into
            // three rhombi: an isometric cube with a lit top.
            let (u, v) = (px / 20.0, py / 20.0);
            let inside = u.abs() <= 0.866 && v.abs() + 0.577 * u.abs() <= 1.0;
            if inside {
                color = if v <= -0.577 * u.abs() {
                    [235, 242, 250]
                } else if u < 0.0 {
                    [190, 205, 222]
                } else {
                    [150, 168, 190]
                };
            }
            let i = (y * N + x) * 4;
            rgba[i..i + 4].copy_from_slice(&[color[0], color[1], color[2], 255]);
        }
    }
    eframe::egui::IconData { rgba, width: N as u32, height: N as u32 }
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
    let icon_path = home.join(".local/share/icons/hicolor/64x64/apps/fission.png");
    let desktop_path = home.join(".local/share/applications/fission.desktop");
    let desktop = format!(
        "[Desktop Entry]\nType=Application\nName=Fission\nComment=Parametric CAD\nExec={} %f\nIcon=fission\n\
         Terminal=false\nCategories=Graphics;Engineering;3DGraphics;\nMimeType=application/x-fission;\n\
         StartupWMClass=fission\n",
        exe.display()
    );
    let result = (|| -> std::io::Result<()> {
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

fn export_cli(input: &str, output: &Path) -> ExitCode {
    // All kernel work goes through the worker process, so a design that
    // crashes OpenCascade loses only the crashing feature, as in the editor.
    let mut kernel = worker::Kernel::new();
    let mut doc = if input == "--sample" {
        Document::sample()
    } else if input.to_ascii_lowercase().ends_with(".f3d") {
        // A Fusion design: replay its timeline.
        let timeline = match fission_f3d::read_timeline(Path::new(input)) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("fission: {input}: {e}");
                return ExitCode::FAILURE;
            }
        };
        let name = Path::new(input).file_stem().and_then(|s| s.to_str()).unwrap_or("Imported");
        let mut rebuild = |doc: &Document| match kernel.regenerate(doc, None) {
            Ok(r) => r.bodies.into_iter().map(|b| (b.created_by, b.mesh)).collect(),
            Err(_) => Vec::new(),
        };
        fission_model::f3d::document_with(&timeline, name, &mut rebuild).doc
    } else {
        let text = match std::fs::read_to_string(input) {
            Ok(text) => text,
            Err(e) => {
                eprintln!("fission: can't read {input}: {e}");
                return ExitCode::FAILURE;
            }
        };
        match Document::from_ron(&text) {
            Ok(doc) => doc,
            Err(e) => {
                eprintln!("fission: {input}: {e}");
                return ExitCode::FAILURE;
            }
        }
    };

    for problem in doc.apply_expressions() {
        eprintln!("fission: {problem}");
    }
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

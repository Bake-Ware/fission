//! `fission-app animate`: render a joint sweep headless, one frame per
//! value, through the same renderer the viewport uses (no window).

use std::path::{Path, PathBuf};
use std::process::Command;

use eframe::wgpu;
use fission_model::{motion, Document, Rigid};
use glam::Vec3;

use crate::motion::moved_mesh;
use crate::viewport::{Camera, Frame, StandardView, ViewportRenderer};
use crate::worker;

pub struct Options {
    pub joint: String,
    pub from: Option<f64>,
    pub to: Option<f64>,
    pub frames: usize,
    /// Go there and back (the return leg skips both ends).
    pub pingpong: bool,
    pub view: StandardView,
    pub size: [u32; 2],
    pub fps: u32,
    pub dark: bool,
    /// A directory (PNG frames), or a .gif / .mp4 (needs ffmpeg).
    pub out: PathBuf,
}

/// A GPU device with no window.
fn gpu() -> Result<(wgpu::Device, wgpu::Queue), String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .map_err(|e| format!("no GPU adapter for headless rendering: {e}"))?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).map_err(|e| format!("can't open the GPU: {e}"))
}

pub fn run(doc: &Document, kernel: &mut worker::Kernel, opts: &Options) -> Result<String, String> {
    let joint = doc.joint(&opts.joint)?;
    let regen = kernel.regenerate(doc, None)?;
    if let Some(fission_model::FeatureStatus::Error(e)) = regen.status.get(joint.feature) {
        return Err(format!("{}: {e}", joint.name));
    }
    let (origin, dir) = joint.axis.resolve(&doc.sketch_outputs())?;
    let from = opts.from.unwrap_or(joint.min);
    let to = opts.to.unwrap_or(joint.max);
    let mut values = motion::steps(from, to, opts.frames);
    if opts.pingpong && values.len() > 2 {
        let back: Vec<f64> = values[1..values.len() - 1].iter().rev().copied().collect();
        values.extend(back);
    }
    let motion_at = |v: f64| Rigid::joint(joint.motion, origin, dir, v - joint.value);
    let moving = |created_by: usize| joint.bodies.contains(&created_by);

    // One camera for the whole sweep, framing everything it passes through.
    let mut points = Vec::new();
    for body in &regen.bodies {
        let corners = box_corners(&body.mesh.positions);
        if moving(body.created_by) {
            for &v in &values {
                let t = motion_at(v);
                points.extend(corners.iter().map(|&c| Vec3::from(t.point(c.map(f64::from)).map(|x| x as f32))));
            }
        } else {
            points.extend(corners.iter().map(|&c| Vec3::from(c)));
        }
    }
    let aspect = opts.size[0] as f32 / opts.size[1] as f32;
    let mut camera = Camera::default();
    camera.set_view(opts.view);
    camera.fit_points(&points, aspect, 0.06);

    crate::ui::theme::use_dark(opts.dark);
    let (device, queue) = gpu()?;
    let mut renderer = ViewportRenderer::new(&device);
    let colors: Vec<[f32; 4]> =
        regen.bodies.iter().map(|b| crate::viewport::appearance(doc.features.get(b.created_by).and_then(|f| f.color))).collect();

    let video = matches!(extension(&opts.out).as_str(), "gif" | "mp4" | "webm");
    let frames_dir = if video { opts.out.with_extension("frames") } else { opts.out.clone() };
    std::fs::create_dir_all(&frames_dir).map_err(|e| format!("{}: {e}", frames_dir.display()))?;
    let size = glam::Vec2::new(opts.size[0] as f32, opts.size[1] as f32);
    for (i, &v) in values.iter().enumerate() {
        let t = motion_at(v);
        let meshes: Vec<_> = regen
            .bodies
            .iter()
            .map(|b| if moving(b.created_by) { std::borrow::Cow::Owned(moved_mesh(&b.mesh, &t)) } else { std::borrow::Cow::Borrowed(&b.mesh) })
            .collect();
        renderer.set_meshes(&device, &queue, meshes.iter().enumerate().map(|(k, m)| (k, m.as_ref())));
        let frame = Frame {
            view_proj: camera.view_proj(size),
            eye: camera.eye(),
            hovered: 0,
            selected: Vec::new(),
            body_mask: 0,
            clip: None,
            colors: colors.clone(),
        };
        let rgba = renderer.render_pixels(&device, &queue, opts.size, &frame);
        let path = frames_dir.join(format!("f{i:04}.png"));
        std::fs::write(&path, crate::png::encode_rgba(opts.size[0], opts.size[1], &rgba)).map_err(|e| format!("{}: {e}", path.display()))?;
    }

    let summary = format!("{}: {} frames, {from} to {to}{}", joint.name, values.len(), if opts.pingpong { " and back" } else { "" });
    if !video {
        return Ok(format!("{summary} in {}", frames_dir.display()));
    }
    encode_video(&frames_dir, &opts.out, opts.fps)?;
    let _ = std::fs::remove_dir_all(&frames_dir);
    Ok(format!("{summary} -> {}", opts.out.display()))
}

fn extension(p: &Path) -> String {
    p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

/// The 8 corners of a point cloud's box.
fn box_corners(points: &[[f32; 3]]) -> Vec<[f32; 3]> {
    if points.is_empty() {
        return Vec::new();
    }
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for p in points {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    (0..8).map(|i| std::array::from_fn(|k| if i >> k & 1 == 0 { lo[k] } else { hi[k] })).collect()
}

fn encode_video(frames: &Path, out: &Path, fps: u32) -> Result<(), String> {
    let input = frames.join("f%04d.png");
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-y", "-loglevel", "error", "-framerate", &fps.to_string(), "-i"]).arg(&input);
    match extension(out).as_str() {
        "gif" => cmd.args(["-vf", "split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=sierra2_4a", "-loop", "0"]),
        _ => cmd.args(["-movflags", "faststart", "-pix_fmt", "yuv420p", "-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2"]),
    };
    let status = cmd.arg(out).status().map_err(|e| format!("can't run ffmpeg (install it, or give a directory for PNG frames): {e}"))?;
    if !status.success() {
        return Err(format!("ffmpeg failed; the frames are in {}", frames.display()));
    }
    Ok(())
}

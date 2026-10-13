//! INSERT ▸ Canvas: reference images on planes, for tracing over.
//!
//! Images are decoded once per path and shared with the renderer; the
//! dialog shows the whole image with the crop box, which can be dragged.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke};
use fission_model::{FeatureKind, FeatureStatus, OriginPlane};

use super::theme;
use crate::app::FissionApp;
use crate::viewport::CanvasImage;

/// Larger images are scaled down to this many pixels on their long side
/// (GPU texture limits, and memory).
const MAX_SIDE: u32 = 4096;

#[derive(Default)]
pub struct CanvasStore {
    images: HashMap<PathBuf, Result<Arc<CanvasImage>, String>>,
    previews: HashMap<PathBuf, egui::TextureHandle>,
}

impl CanvasStore {
    /// The decoded image at `path` (loaded the first time it's asked for).
    pub fn image(&mut self, path: &Path) -> Result<Arc<CanvasImage>, String> {
        self.images.entry(path.to_path_buf()).or_insert_with(|| load(path).map(Arc::new)).clone()
    }

    /// Re-read an image from disk next time (after Change Image, or edits to the file).
    pub fn forget(&mut self, path: &Path) {
        self.images.remove(path);
        self.previews.remove(path);
    }

    fn preview(&mut self, ctx: &egui::Context, path: &Path) -> Option<egui::TextureHandle> {
        if let Some(t) = self.previews.get(path) {
            return Some(t.clone());
        }
        let image = self.image(path).ok()?;
        let color = egui::ColorImage::from_rgba_unmultiplied([image.size[0] as usize, image.size[1] as usize], &image.rgba);
        let handle = ctx.load_texture(format!("canvas:{}", path.display()), color, egui::TextureOptions::LINEAR);
        self.previews.insert(path.to_path_buf(), handle.clone());
        Some(handle)
    }
}

fn load(path: &Path) -> Result<CanvasImage, String> {
    let image = image::open(path).map_err(|e| format!("can't read the image {}: {e}", path.display()))?;
    let image = if image.width().max(image.height()) > MAX_SIDE {
        image.resize(MAX_SIDE, MAX_SIDE, image::imageops::FilterType::Triangle)
    } else {
        image
    };
    let rgba = image.to_rgba8();
    Ok(CanvasImage { size: [rgba.width(), rgba.height()], rgba: rgba.into_raw() })
}

impl FissionApp {
    /// A canvas's image path, relative ones taken from the design's folder.
    pub fn canvas_path(&self, path: &str) -> PathBuf {
        let p = PathBuf::from(path);
        match (&self.path, p.is_relative()) {
            (Some(design), true) => design.parent().map_or(p.clone(), |d| d.join(&p)),
            _ => p,
        }
    }

    /// Hand the current canvases to the viewport, and mark ones whose image
    /// won't load. Call after every regeneration.
    pub fn refresh_canvases(&mut self) {
        let mut draws = Vec::new();
        for c in self.doc.canvases() {
            let path = self.canvas_path(&c.path);
            match self.canvas_store.image(&path) {
                Ok(image) => {
                    if self.hidden.contains(&c.feature) {
                        continue;
                    }
                    draws.push(crate::viewport::CanvasEntry {
                        key: path.display().to_string(),
                        image,
                        corners: c.corners.map(|p| p.map(|v| v as f32)),
                        uv: c.uv.map(|p| p.map(|v| v as f32)),
                        opacity: c.opacity as f32,
                    });
                }
                Err(e) => {
                    if let Some(s) = self.status.get_mut(c.feature) {
                        *s = FeatureStatus::Error(e);
                    }
                }
            }
        }
        self.viewport.set_canvases(draws);
    }

    /// INSERT ▸ Canvas: pick an image and place it on the selected planar
    /// face, or on XY, 100 mm wide.
    pub fn new_canvas(&mut self) {
        let Some(file) = rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg"]).pick_file() else { return };
        self.insert_canvas(&file);
    }

    pub fn insert_canvas(&mut self, file: &Path) {
        let image = match self.canvas_store.image(file) {
            Ok(image) => image,
            Err(e) => return self.notify(e, true),
        };
        let (plane, offset, center) = match self.viewport.selected_face() {
            Some(hit) => {
                let (p, n) = (hit.point.as_dvec3(), hit.normal.as_dvec3());
                let (plane, offset, _) = crate::app::plane_through(p, n);
                let pl = plane.plane(offset);
                let d = p - glam::DVec3::from(pl.origin);
                (plane, offset, [d.dot(pl.x_dir.into()), d.dot(pl.y_dir().into())])
            }
            None => (OriginPlane::XY, 0.0, [0.0, 0.0]),
        };
        let width = 100.0;
        let height = width * image.size[1] as f64 / image.size[0].max(1) as f64;
        self.create(FeatureKind::Canvas {
            path: file.display().to_string(),
            plane,
            offset,
            center: center.map(|v| v + 0.0),
            size: [width, height],
            rotation: 0.0,
            crop: [0.0, 0.0, 1.0, 1.0],
            opacity: 0.5,
        });
    }
}

/// Below the canvas dialog's fields: the whole image with the crop box.
/// Drag on it to crop to a new box; Reset shows the whole image again.
/// Returns true if the feature changed.
pub fn crop_editor(app: &mut FissionApp, ui: &mut egui::Ui, index: usize) -> bool {
    let Some(FeatureKind::Canvas { path, crop, .. }) = app.doc.features.get(index).map(|f| f.kind.clone()) else { return false };
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(Path::new(&path).file_name().map_or(path.clone(), |n| n.to_string_lossy().into_owned())).small().color(theme::muted()));
        if ui.small_button("Change…").clicked() {
            if let Some(file) = rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg"]).pick_file() {
                app.canvas_store.forget(&file);
                if let Some(FeatureKind::Canvas { path, .. }) = app.doc.features.get_mut(index).map(|f| &mut f.kind) {
                    *path = file.display().to_string();
                }
                changed = true;
            }
        }
        if ui.small_button("Reset crop").clicked() {
            if let Some(f) = app.doc.features.get_mut(index) {
                fission_model::recrop_canvas(&mut f.kind, [0.0, 0.0, 1.0, 1.0]);
            }
            changed = true;
        }
    });
    let full = app.canvas_path(&path);
    let Some(texture) = app.canvas_store.preview(ui.ctx(), &full) else { return changed };
    let [w, h] = texture.size();
    let width = ui.available_width().min(240.0);
    let size = egui::vec2(width, width * h as f32 / w.max(1) as f32);
    let (rect, response) = ui.allocate_exact_size(size, Sense::drag());
    let painter = ui.painter_at(rect);
    painter.image(texture.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
    let to_screen = |u: f64, v: f64| Pos2::new(rect.min.x + u as f32 * rect.width(), rect.min.y + v as f32 * rect.height());
    let to_uv = |p: Pos2| [((p.x - rect.min.x) / rect.width()).clamp(0.0, 1.0) as f64, ((p.y - rect.min.y) / rect.height()).clamp(0.0, 1.0) as f64];

    // While dragging, the box follows the pointer; it's applied on release.
    let id = response.id.with("crop_drag");
    let mut shown = crop;
    if response.drag_started() {
        if let Some(p) = response.interact_pointer_pos() {
            ui.memory_mut(|m| m.data.insert_temp(id, to_uv(p)));
        }
    }
    let start: Option<[f64; 2]> = ui.memory(|m| m.data.get_temp(id));
    if let (Some(a), Some(p)) = (start, response.interact_pointer_pos()) {
        let b = to_uv(p);
        shown = [a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])];
        if response.drag_stopped() {
            ui.memory_mut(|m| m.data.remove::<[f64; 2]>(id));
            if shown[2] - shown[0] > 0.01 && shown[3] - shown[1] > 0.01 {
                if let Some(f) = app.doc.features.get_mut(index) {
                    fission_model::recrop_canvas(&mut f.kind, shown);
                }
                changed = true;
            }
        }
    }
    // Dim what's cropped away.
    let keep = Rect::from_min_max(to_screen(shown[0], shown[1]), to_screen(shown[2], shown[3]));
    let dim = Color32::from_black_alpha(150);
    for r in [
        Rect::from_min_max(rect.min, Pos2::new(rect.max.x, keep.min.y)),
        Rect::from_min_max(Pos2::new(rect.min.x, keep.max.y), rect.max),
        Rect::from_min_max(Pos2::new(rect.min.x, keep.min.y), Pos2::new(keep.min.x, keep.max.y)),
        Rect::from_min_max(Pos2::new(keep.max.x, keep.min.y), Pos2::new(rect.max.x, keep.max.y)),
    ] {
        painter.rect_filled(r, 0.0, dim);
    }
    painter.rect_stroke(keep, 0.0, Stroke::new(1.5, theme::accent()), egui::StrokeKind::Middle);
    ui.label(RichText::new("Drag on the image to crop").small().color(theme::muted()));
    changed
}

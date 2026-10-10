use bytemuck::{Pod, Zeroable};
use eframe::egui_wgpu::RenderState;
use eframe::wgpu::{self, util::DeviceExt};
use fission_kernel::{Mesh, Plane};
use glam::{Mat4, Vec3};

use super::pick::pack_id;

const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SAMPLES: u32 = 4;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    ids: [u32; 16],
    bodies: [u32; 4],
    clip: [f32; 4],
    // Background gradient top and bottom; theme.x is 1 in dark mode.
    bg_top: [f32; 4],
    bg_bottom: [f32; 4],
    theme: [f32; 4],
    colors: [[f32; 4]; 32],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FaceVertex {
    pos: [f32; 3],
    normal: [f32; 3],
    id: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct LineVertex {
    pub pos: [f32; 3],
    pub color: [f32; 4],
}

pub struct Frame {
    pub view_proj: Mat4,
    pub eye: Vec3,
    pub hovered: u32,
    /// Face ids to highlight; only the first 15 are shown.
    pub selected: Vec<u32>,
    /// Bodies (display index bitmask) to tint as selected.
    pub body_mask: u32,
    /// Section plane (normal, d): cut away where normal·p > d.
    pub clip: Option<glam::Vec4>,
    /// Appearance per display body (alpha 0 = default gray).
    pub colors: Vec<[f32; 4]>,
}

struct Targets {
    size: [u32; 2],
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
    resolve: wgpu::TextureView,
}

struct Geometry {
    vertices: wgpu::Buffer,
    indices: Option<wgpu::Buffer>,
    count: u32,
}

/// Renders the 3D scene into an offscreen texture that egui shows as an image.
/// Owning the target lets us pick MSAA and depth independently of the window.
pub struct ViewportRenderer {
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    background: wgpu::RenderPipeline,
    faces_pipeline: wgpu::RenderPipeline,
    lines_pipeline: wgpu::RenderPipeline,
    targets: Option<Targets>,
    texture_id: Option<eframe::egui::TextureId>,
    faces: Option<Geometry>,
    edges: Option<Geometry>,
    grid: Option<Geometry>,
}

impl ViewportRenderer {
    pub fn new(rs: &RenderState) -> Self {
        let device = &rs.device;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("viewport"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewport"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = |label: &str,
                        vs: &str,
                        fs: &str,
                        buffers: &[Option<wgpu::VertexBufferLayout>],
                        topology: wgpu::PrimitiveTopology,
                        depth: wgpu::DepthStencilState,
                        blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers,
                },
                primitive: wgpu::PrimitiveState { topology, cull_mode: None, ..Default::default() },
                depth_stencil: Some(depth),
                multisample: wgpu::MultisampleState { count: SAMPLES, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: COLOR_FORMAT,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };

        let depth = |write: bool, compare: wgpu::CompareFunction, bias: wgpu::DepthBiasState| wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(write),
            depth_compare: Some(compare),
            stencil: Default::default(),
            bias,
        };

        let background = pipeline(
            "background",
            "bg_vs",
            "bg_fs",
            &[],
            wgpu::PrimitiveTopology::TriangleList,
            depth(false, wgpu::CompareFunction::Always, Default::default()),
            None,
        );

        let face_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Uint32];
        let faces_pipeline = pipeline(
            "faces",
            "face_vs",
            "face_fs",
            &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<FaceVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &face_attrs,
            })],
            wgpu::PrimitiveTopology::TriangleList,
            // Push faces back slightly so their edges draw cleanly on top.
            depth(true, wgpu::CompareFunction::Less, wgpu::DepthBiasState { constant: 2, slope_scale: 1.5, clamp: 0.0 }),
            None,
        );

        let line_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];
        let lines_pipeline = pipeline(
            "lines",
            "line_vs",
            "line_fs",
            &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<LineVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &line_attrs,
            })],
            wgpu::PrimitiveTopology::LineList,
            depth(false, wgpu::CompareFunction::LessEqual, Default::default()),
            Some(wgpu::BlendState::ALPHA_BLENDING),
        );

        let mut renderer = ViewportRenderer {
            globals,
            bind_group,
            background,
            faces_pipeline,
            lines_pipeline,
            targets: None,
            texture_id: None,
            faces: None,
            edges: None,
            grid: None,
        };
        renderer.grid = line_geometry(device, &grid_lines(&Plane::XY, 10.0, 25));
        renderer
    }

    /// Rebuild the grid on `plane` with `spacing` between lines.
    pub fn set_grid(&mut self, rs: &RenderState, plane: &Plane, spacing: f64, half_cells: i32) {
        self.grid = line_geometry(&rs.device, &grid_lines(plane, spacing, half_cells));
    }

    /// Upload the bodies' meshes. Call when the document regenerates.
    pub fn set_meshes<'a>(&mut self, rs: &RenderState, meshes: impl IntoIterator<Item = (usize, &'a Mesh)>) {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let mut edges = Vec::new();
        let edge_color = [0.12, 0.14, 0.17, 1.0];

        for (body, mesh) in meshes {
            // Unshare vertices per triangle so each one carries its face id.
            for (t, tri) in mesh.indices.chunks_exact(3).enumerate() {
                let id = pack_id(body, mesh.face_ids[t]);
                for &i in tri {
                    indices.push(vertices.len() as u32);
                    vertices.push(FaceVertex {
                        pos: mesh.positions[i as usize],
                        normal: mesh.normals.get(i as usize).copied().unwrap_or([0.0, 0.0, 1.0]),
                        id,
                    });
                }
            }
            for polyline in &mesh.edges {
                for pair in polyline.windows(2) {
                    edges.push(LineVertex { pos: pair[0], color: edge_color });
                    edges.push(LineVertex { pos: pair[1], color: edge_color });
                }
            }
        }

        let device = &rs.device;
        self.faces = (!indices.is_empty()).then(|| Geometry {
            vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("faces"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            indices: Some(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("face indices"),
                contents: bytemuck::cast_slice(&indices),
                usage: wgpu::BufferUsages::INDEX,
            })),
            count: indices.len() as u32,
        });
        self.edges = line_geometry(device, &edges);
    }

    /// Render a frame at `size` physical pixels and return the egui texture showing it.
    pub fn render(&mut self, rs: &RenderState, size: [u32; 2], frame: &Frame) -> eframe::egui::TextureId {
        let size = [size[0].max(1), size[1].max(1)];
        self.ensure_targets(rs, size);
        let targets = self.targets.as_ref().unwrap();

        let globals = Globals {
            view_proj: frame.view_proj.to_cols_array_2d(),
            eye: frame.eye.extend(1.0).to_array(),
            ids: {
                let mut ids = [0u32; 16];
                ids[0] = frame.hovered;
                for (slot, id) in ids[1..].iter_mut().zip(&frame.selected) {
                    *slot = *id;
                }
                ids
            },
            bodies: [frame.body_mask, 0, 0, 0],
            clip: frame.clip.map_or([0.0; 4], |c| c.to_array()),
            bg_top: { let p = crate::ui::theme::palette(); [p.bg_top[0], p.bg_top[1], p.bg_top[2], 1.0] },
            bg_bottom: { let p = crate::ui::theme::palette(); [p.bg_bottom[0], p.bg_bottom[1], p.bg_bottom[2], 1.0] },
            theme: [if crate::ui::theme::is_dark() { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0],
            colors: {
                let mut colors = [[0.0f32; 4]; 32];
                for (slot, c) in colors.iter_mut().zip(&frame.colors) {
                    *slot = *c;
                }
                colors
            },
        };
        rs.queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));

        let mut encoder = rs.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("viewport") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.msaa,
                    depth_slice: None,
                    resolve_target: Some(&targets.resolve),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.bind_group, &[]);

            pass.set_pipeline(&self.background);
            pass.draw(0..3, 0..1);

            if let Some(faces) = &self.faces {
                pass.set_pipeline(&self.faces_pipeline);
                pass.set_vertex_buffer(0, faces.vertices.slice(..));
                pass.set_index_buffer(faces.indices.as_ref().unwrap().slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..faces.count, 0, 0..1);
            }

            pass.set_pipeline(&self.lines_pipeline);
            for lines in [&self.grid, &self.edges].into_iter().flatten() {
                pass.set_vertex_buffer(0, lines.vertices.slice(..));
                pass.draw(0..lines.count, 0..1);
            }
        }
        rs.queue.submit([encoder.finish()]);

        self.texture_id.expect("texture registered in ensure_targets")
    }

    fn ensure_targets(&mut self, rs: &RenderState, size: [u32; 2]) {
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let device = &rs.device;
        let texture = |label, samples, format, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let targets = Targets {
            size,
            msaa: texture("viewport msaa", SAMPLES, COLOR_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT),
            depth: texture("viewport depth", SAMPLES, DEPTH_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT),
            resolve: texture(
                "viewport color",
                1,
                COLOR_FORMAT,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            ),
        };

        let mut egui_renderer = rs.renderer.write();
        match self.texture_id {
            Some(id) => egui_renderer.update_egui_texture_from_wgpu_texture(
                device,
                &targets.resolve,
                wgpu::FilterMode::Linear,
                id,
            ),
            None => {
                self.texture_id =
                    Some(egui_renderer.register_native_texture(device, &targets.resolve, wgpu::FilterMode::Linear));
            }
        }
        self.targets = Some(targets);
    }
}

fn line_geometry(device: &wgpu::Device, lines: &[LineVertex]) -> Option<Geometry> {
    (!lines.is_empty()).then(|| Geometry {
        vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("lines"),
            contents: bytemuck::cast_slice(lines),
            usage: wgpu::BufferUsages::VERTEX,
        }),
        indices: None,
        count: lines.len() as u32,
    })
}

/// Grid on `plane` with heavier lines every 5 cells and its axes colored
/// by the world axis they run along (X red, Y green, Z blue).
fn grid_lines(plane: &Plane, spacing: f64, half_cells: i32) -> Vec<LineVertex> {
    let extent = spacing * half_cells as f64;
    let (u, v) = (plane.x_dir, plane.y_dir());
    // Lift the grid a hair off the plane so it doesn't fight a face lying on it.
    let o: [f64; 3] = std::array::from_fn(|k| plane.origin[k] + plane.normal[k] * 0.01);
    let at = |a: f64, b: f64| -> [f32; 3] { std::array::from_fn(|k| (o[k] + u[k] * a + v[k] * b) as f32) };
    let minor = [0.35, 0.37, 0.42, 0.18];
    let major = [0.30, 0.32, 0.37, 0.35];
    let mut out = Vec::new();
    for i in -half_cells..=half_cells {
        if i == 0 {
            continue;
        }
        let color = if i % 5 == 0 { major } else { minor };
        let d = i as f64 * spacing;
        out.push(LineVertex { pos: at(d, -extent), color });
        out.push(LineVertex { pos: at(d, extent), color });
        out.push(LineVertex { pos: at(-extent, d), color });
        out.push(LineVertex { pos: at(extent, d), color });
    }
    let axis_color = |dir: [f64; 3]| -> [f32; 4] {
        let k = (0..3).max_by(|&a, &b| dir[a].abs().total_cmp(&dir[b].abs())).unwrap();
        [[0.85, 0.20, 0.20, 0.9], [0.20, 0.65, 0.25, 0.9], [0.20, 0.40, 0.85, 0.9]][k]
    };
    out.push(LineVertex { pos: at(-extent, 0.0), color: axis_color(u) });
    out.push(LineVertex { pos: at(extent, 0.0), color: axis_color(u) });
    out.push(LineVertex { pos: at(0.0, -extent), color: axis_color(v) });
    out.push(LineVertex { pos: at(0.0, extent), color: axis_color(v) });
    out
}

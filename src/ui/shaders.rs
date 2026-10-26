//! Custom wgpu rendering for the two visual effects the reference UI gets
//! from CSS `filter: blur()` and a radial gradient:
//!
//! - the header title's per-word `fade-in-blur` reveal (each word starts at
//!   opacity 0, 20px below its final position, `blur(12px)`, and reveals
//!   over 0.3s).
//!
//! (The request card's `Spotlight` paints nothing in the reference: its
//! Tailwind `bg-[radial-gradient(...var(--tw-gradient-stops)...)]` is invalid
//! at computed-value time — `--tw-gradient-position` is `@property`-registered
//! without an initial value, so `--tw-gradient-stops` is guaranteed-invalid
//! and the div's `background-image` computes to `none`. The reference's hover
//! captures are byte-identical to its non-hover captures, so the native app
//! renders no spotlight either.)
//!
//! The blurred text draws with the same math as egui-wgpu's own mesh pipeline
//! (a straight-alpha tint premultiplied in the shader, on a
//! premultiplied-white texture, premultiplied alpha blending), so the blurred
//! text composites exactly like normal egui text — including at partial
//! opacity. The blur is a true separable gaussian (`filter: blur(r)`
//! uses σ = r, as measured from the reference), run as two GPU passes
//! with clamp-to-edge sampling on premultiplied RGBA8 textures.

use std::sync::{Arc, Mutex};

use eframe::egui_wgpu::{CallbackResources, CallbackTrait, RenderState, ScreenDescriptor};
use eframe::wgpu;
use epaint::text::Galley;
use epaint::{ColorImage, PaintCallbackInfo};
use egui::{Color32, Rect, Shape};

// ---------------------------------------------------------------------------
// Shaders
// ---------------------------------------------------------------------------

const BLUR_WGSL: &str = r#"
struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_quad(@builtin(vertex_index) vi: u32) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
    );
    var uvs = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    var out: VsOut;
    out.position = vec4<f32>(corners[vi], 0.0, 1.0);
    out.uv = uvs[vi];
    return out;
}

struct BlurParams {
    sigma: f32,
    radius: f32,
    tex_w: f32,
    tex_h: f32,
    dir_x: f32,
    dir_y: f32,
    pad0: f32,
    pad1: f32,
};

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var<uniform> params: BlurParams;

@fragment
fn fs_blur(in: VsOut) -> @location(0) vec4<f32> {
    let center = vec2<i32>(floor(in.position.xy));
    let max_c = vec2<i32>(i32(params.tex_w) - 1, i32(params.tex_h) - 1);
    let dir = vec2<i32>(i32(params.dir_x), i32(params.dir_y));
    let r = i32(params.radius);
    let sigma2 = params.sigma * params.sigma;
    var acc = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    var wsum = 0.0;
    for (var i = -r; i <= r; i = i + 1) {
        let w = exp(-0.5 * f32(i * i) / sigma2);
        let p = clamp(center + dir * i, vec2<i32>(0, 0), max_c);
        acc = acc + w * textureLoad(src_tex, p, 0);
        wsum = wsum + w;
    }
    return acc / wsum;
}
"#;

const DRAW_WGSL: &str = r#"
struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_quad(@builtin(vertex_index) vi: u32) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
    );
    var uvs = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    var out: VsOut;
    out.position = vec4<f32>(corners[vi], 0.0, 1.0);
    out.uv = uvs[vi];
    return out;
}

struct DrawParams {
    tint: vec4<f32>,
    clip_rect: vec4<f32>,
    clip_radius: f32,
    pad0: f32,
    pad1: f32,
    pad2: f32,
};

@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> params: DrawParams;

fn rounded_rect_sdf(p: vec2<f32>, rect: vec4<f32>, r: f32) -> f32 {
    let center = (rect.xy + rect.zw) * 0.5;
    let half = (rect.zw - rect.xy) * 0.5;
    let q = abs(p - center) - (half - vec2<f32>(r, r));
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - r;
}

@fragment
fn fs_draw(in: VsOut) -> @location(0) vec4<f32> {
    let t = textureSample(tex, samp, in.uv);
    // The font texture is premultiplied white (rgb == a == coverage) and this
    // pipeline blends premultiplied (One, OneMinusSrcAlpha), exactly like
    // egui-wgpu's mesh pipeline — so the straight-alpha tint must itself be
    // premultiplied: out.rgb = tint.rgb * tint.a * coverage, out.a =
    // tint.a * coverage. Without the tint.a factor the text would draw at
    // full color regardless of the reveal opacity.
    let a = params.tint.a * t.a;
    var out = vec4(params.tint.rgb * a, a);
    if params.clip_radius >= 0.0 {
        let d = rounded_rect_sdf(in.position.xy, params.clip_rect, params.clip_radius);
        let coverage = clamp(0.5 - d, 0.0, 1.0);
        out = out * coverage;
    }
    return out;
}
"#;

// ---------------------------------------------------------------------------
// Small wgpu helpers
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct GpuTex {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: [u32; 2],
}

fn create_tex(device: &wgpu::Device, size: [u32; 2], usage: wgpu::TextureUsages) -> GpuTex {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sag-effect"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    GpuTex {
        texture,
        view,
        size,
    }
}

fn upload_image(queue: &wgpu::Queue, tex: &GpuTex, image: &ColorImage) {
    let mut bytes = Vec::with_capacity(image.pixels.len() * 4);
    for px in &image.pixels {
        bytes.extend_from_slice(&[px.r(), px.g(), px.b(), px.a()]);
    }
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex.texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * tex.size[0]),
            rows_per_image: Some(tex.size[1]),
        },
        wgpu::Extent3d {
            width: tex.size[0],
            height: tex.size[1],
            depth_or_array_layers: 1,
        },
    );
}

fn create_uniform_buffer(device: &wgpu::Device, queue: &wgpu::Queue, data: &[f32]) -> wgpu::Buffer {
    let mut bytes = Vec::with_capacity(data.len() * 4);
    for v in data {
        bytes.extend_from_slice(&v.to_ne_bytes());
    }
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("sag-uniform"),
        size: bytes.len() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&buffer, 0, &bytes);
    buffer
}

/// The premultiplied-alpha blend egui-wgpu uses for its mesh pipeline.
fn premultiplied_blend() -> wgpu::BlendState {
    wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
    }
}

// ---------------------------------------------------------------------------
// Blur pipeline (two-pass separable gaussian on RGBA8 scratch textures)
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct BlurPipeline {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

impl BlurPipeline {
    fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sag-blur"),
            source: wgpu::ShaderSource::Wgsl(BLUR_WGSL.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sag-blur-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sag-blur-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sag-blur-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_quad"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_blur"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline, layout }
    }

    /// Blur `src` into `out` (via `tmp`), all the same size. `sigma` is in
    /// device pixels. CSS `blur(r)` uses σ = r (measured from the
    /// reference), so callers pass `r * pixels_per_point`.
    pub fn blur(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        src: &wgpu::TextureView,
        tmp: &wgpu::TextureView,
        out: &wgpu::TextureView,
        size: [u32; 2],
        sigma: f32,
    ) {
        let radius = (3.0 * sigma).ceil().max(1.0);
        self.pass(device, queue, encoder, src, tmp, size, sigma, radius, 1.0, 0.0);
        self.pass(device, queue, encoder, tmp, out, size, sigma, radius, 0.0, 1.0);
    }

    fn pass(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        src: &wgpu::TextureView,
        dst: &wgpu::TextureView,
        size: [u32; 2],
        sigma: f32,
        radius: f32,
        dir_x: f32,
        dir_y: f32,
    ) {
        let params = [
            sigma,
            radius,
            size[0] as f32,
            size[1] as f32,
            dir_x,
            dir_y,
            0.0,
            0.0,
        ];
        let buffer = create_uniform_buffer(device, queue, &params);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sag-blur-bind-group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(src),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &buffer,
                        offset: 0,
                        size: None,
                    }),
                },
            ],
        });
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sag-blur-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: dst,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &bind_group, &[]);
        render_pass.set_viewport(0.0, 0.0, size[0] as f32, size[1] as f32, 0.0, 1.0);
        render_pass.draw(0..6, 0..1);
    }
}

// ---------------------------------------------------------------------------
// Draw pipeline (textured quad with egui's text blending math)
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct DrawPipeline {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl DrawPipeline {
    fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sag-draw"),
            source: wgpu::ShaderSource::Wgsl(DRAW_WGSL.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sag-draw-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sag-draw-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sag-draw-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sag-draw-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_quad"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_draw"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(premultiplied_blend()),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            layout,
            sampler,
        }
    }

    /// Draw `tex` (premultiplied) tinted by `tint` (straight RGBA) into the
    /// current render pass. The viewport must already cover the destination
    /// rect in pixels (egui-wgpu sets it to the callback's rect). `clip` is
    /// an optional rounded-rect SDF clip in absolute device pixels.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        render_pass: &mut wgpu::RenderPass<'static>,
        tex: &wgpu::TextureView,
        tint: [f32; 4],
        clip: Option<([f32; 4], f32)>,
    ) {
        let (clip_rect, clip_radius) = match clip {
            Some((rect, radius)) => (rect, radius),
            None => ([0.0; 4], -1.0),
        };
        let params = [
            tint[0],
            tint[1],
            tint[2],
            tint[3],
            clip_rect[0],
            clip_rect[1],
            clip_rect[2],
            clip_rect[3],
            clip_radius,
            0.0,
            0.0,
            0.0,
        ];
        let buffer = create_uniform_buffer(device, queue, &params);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sag-draw-bind-group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(tex),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &buffer,
                        offset: 0,
                        size: None,
                    }),
                },
            ],
        });
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &bind_group, &[]);
        render_pass.draw(0..6, 0..1);
    }
}

// ---------------------------------------------------------------------------
// Shared GPU state
// ---------------------------------------------------------------------------

/// GPU resources shared by the effect callbacks: the render state (stored
/// once the eframe frame exposes it) and the lazily-created pipelines.
pub struct Effects {
    inner: Mutex<EffectsInner>,
}

struct EffectsInner {
    render_state: Option<RenderState>,
    blur: Option<BlurPipeline>,
    draw: Option<DrawPipeline>,
}

impl Default for Effects {
    fn default() -> Self {
        Self::new()
    }
}

impl Effects {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(EffectsInner {
                render_state: None,
                blur: None,
                draw: None,
            }),
        }
    }

    /// Store the render state (called every frame; only the first sticks).
    pub fn set_render_state(&self, render_state: RenderState) {
        let mut inner = self.inner.lock().unwrap();
        if inner.render_state.is_none() {
            inner.render_state = Some(render_state);
        }
    }

    pub fn render_state(&self) -> Option<RenderState> {
        self.inner.lock().unwrap().render_state.clone()
    }

    fn ensure_pipelines(&self) {
        let mut inner = self.inner.lock().unwrap();
        let Some(rs) = inner.render_state.clone() else {
            return;
        };
        if inner.blur.is_none() {
            inner.blur = Some(BlurPipeline::new(&rs.device));
        }
        if inner.draw.is_none() {
            inner.draw = Some(DrawPipeline::new(&rs.device, rs.target_format));
        }
    }

    pub fn blur(&self) -> Option<BlurPipeline> {
        self.ensure_pipelines();
        self.inner.lock().unwrap().blur.clone()
    }

    pub fn draw(&self) -> Option<DrawPipeline> {
        self.ensure_pipelines();
        self.inner.lock().unwrap().draw.clone()
    }
}

// ---------------------------------------------------------------------------
// Title blur (per-word reveal): blit the segment's glyphs out of the font
// atlas into a padded premultiplied-white texture, blur it per frame, and
// draw it tinted by the title color × the reveal opacity.
// ---------------------------------------------------------------------------

/// The title's maximum blur (`blur(12px)`), in CSS px.
pub const TITLE_BLUR_CSS: f32 = 12.0;

/// Build the padded premultiplied-white texture for one title segment by
/// blitting its glyph bitmaps out of egui's font atlas (the same texels the
/// normal text mesh samples). Returns the image, the ink bounds' min in
/// device pixels (galley-relative), and the padding in device pixels.
/// Returns `None` for segments with no visible glyphs (whitespace).
pub fn build_segment_texture(
    galley: &Galley,
    atlas: &ColorImage,
    ppp: f32,
) -> Option<(ColorImage, [f32; 2], u32)> {
    let pad = (3.0 * TITLE_BLUR_CSS * ppp).ceil() as u32 + 2;
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    let mut any = false;
    for row in &galley.rows {
        for glyph in &row.glyphs {
            let uv = &glyph.uv_rect;
            if uv.is_nothing() {
                continue;
            }
            let lt = row.pos + glyph.pos.to_vec2() + uv.offset;
            let lt_dev = [lt.x * ppp, lt.y * ppp];
            let size_dev = [uv.size.x * ppp, uv.size.y * ppp];
            min[0] = min[0].min(lt_dev[0]);
            min[1] = min[1].min(lt_dev[1]);
            max[0] = max[0].max(lt_dev[0] + size_dev[0]);
            max[1] = max[1].max(lt_dev[1] + size_dev[1]);
            any = true;
        }
    }
    if !any {
        return None;
    }
    // Glyph quads are device-pixel aligned in the galley mesh, so the bounds
    // are whole device pixels (round to kill float dust).
    let min = [min[0].round(), min[1].round()];
    let max = [max[0].ceil(), max[1].ceil()];
    let w = (max[0] - min[0]) as u32 + 2 * pad;
    let h = (max[1] - min[1]) as u32 + 2 * pad;
    let mut pixels = vec![Color32::TRANSPARENT; (w * h) as usize];
    for row in &galley.rows {
        for glyph in &row.glyphs {
            let uv = &glyph.uv_rect;
            if uv.is_nothing() {
                continue;
            }
            let lt = row.pos + glyph.pos.to_vec2() + uv.offset;
            let lt_dev = [(lt.x * ppp).round(), (lt.y * ppp).round()];
            let size_dev = [(uv.size.x * ppp).round(), (uv.size.y * ppp).round()];
            let dst_x = (lt_dev[0] - min[0]) as u32 + pad;
            let dst_y = (lt_dev[1] - min[1]) as u32 + pad;
            let (gw, gh) = (size_dev[0] as u32, size_dev[1] as u32);
            for y in 0..gh {
                for x in 0..gw {
                    let src = atlas.pixels
                        [(uv.min[1] as u32 + y) as usize * atlas.size[0]
                            + (uv.min[0] as u32 + x) as usize];
                    pixels[(dst_y + y) as usize * w as usize + (dst_x + x) as usize] = src;
                }
            }
        }
    }
    Some((ColorImage::new([w as usize, h as usize], pixels), min, pad))
}

/// Shared state for one title segment's blurred rendering (the GPU textures
/// persist across frames; the per-frame params live on the callback).
pub struct TitleBlurShared {
    effects: Arc<Effects>,
    image: ColorImage,
    size: [u32; 2],
    src: Mutex<Option<GpuTex>>,
    tmp: Mutex<Option<GpuTex>>,
    out: Mutex<Option<GpuTex>>,
    /// The texture view + tint to draw this frame (set in `prepare`).
    draw: Mutex<Option<(wgpu::TextureView, [f32; 4])>>,
}

impl TitleBlurShared {
    pub fn new(effects: Arc<Effects>, image: ColorImage) -> Self {
        let size = [image.size[0] as u32, image.size[1] as u32];
        Self {
            effects,
            image,
            size,
            src: Mutex::new(None),
            tmp: Mutex::new(None),
            out: Mutex::new(None),
            draw: Mutex::new(None),
        }
    }
}

/// A paint callback drawing one title segment, blurred by `sigma` device
/// px and tinted by `tint` (the title color × the reveal opacity).
pub struct TitleBlur {
    shared: Arc<TitleBlurShared>,
    /// The padded texture's rect in screen points (absolute).
    pub rect: Rect,
    /// Straight RGBA tint (title color × segment opacity × container opacity).
    pub tint: [f32; 4],
    /// Blur sigma in device px (0 = draw the unblurred texture).
    pub sigma: f32,
}

impl TitleBlur {
    pub fn new(shared: Arc<TitleBlurShared>, rect: Rect, tint: [f32; 4], sigma: f32) -> Self {
        Self {
            shared,
            rect,
            tint,
            sigma,
        }
    }

    pub fn shape(self) -> Shape {
        Shape::Callback(eframe::egui_wgpu::Callback::new_paint_callback(self.rect, self))
    }
}

impl CallbackTrait for TitleBlur {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &ScreenDescriptor,
        egui_encoder: &mut wgpu::CommandEncoder,
        _callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(blur) = self.shared.effects.blur() else {
            return Vec::new();
        };
        let mut src_guard = self.shared.src.lock().unwrap();
        if src_guard.is_none() {
            let tex = create_tex(
                device,
                self.shared.size,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            );
            upload_image(queue, &tex, &self.shared.image);
            *src_guard = Some(tex);
        }
        let src = src_guard.as_ref().unwrap();
        let view = if self.sigma >= 0.05 {
            let mut tmp_guard = self.shared.tmp.lock().unwrap();
            let mut out_guard = self.shared.out.lock().unwrap();
            if tmp_guard.is_none() {
                *tmp_guard = Some(create_tex(
                    device,
                    self.shared.size,
                    wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
                ));
                *out_guard = Some(create_tex(
                    device,
                    self.shared.size,
                    wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
                ));
            }
            let tmp = tmp_guard.as_ref().unwrap();
            let out = out_guard.as_ref().unwrap();
            blur.blur(
                device,
                queue,
                egui_encoder,
                &src.view,
                &tmp.view,
                &out.view,
                self.shared.size,
                self.sigma,
            );
            out.view.clone()
        } else {
            src.view.clone()
        };
        *self.shared.draw.lock().unwrap() = Some((view, self.tint));
        Vec::new()
    }

    fn paint(
        &self,
        _info: PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        _callback_resources: &CallbackResources,
    ) {
        let Some((view, tint)) = self.shared.draw.lock().unwrap().clone() else {
            return;
        };
        let Some(draw) = self.shared.effects.draw() else {
            return;
        };
        let Some(rs) = self.shared.effects.render_state() else {
            return;
        };
        draw.draw(&rs.device, &rs.queue, render_pass, &view, tint, None);
    }
}

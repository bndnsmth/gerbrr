//! Persistent GPU geometry, polarity-correct MSAA layer masks and a cached composite.
use crate::geometry::{Bounds, Geometry, Point};
use bytemuck::{Pod, Zeroable};
use eframe::egui::{self, Pos2, Rect, Vec2};
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor, wgpu};
use std::sync::Arc;
use wgpu::util::DeviceExt;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub center: Point,
    pub scale: f32,
    pub rotation: f32,
    pub mirrored: bool,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            center: [0.0; 2],
            scale: 10.0,
            rotation: 0.0,
            mirrored: false,
        }
    }
}

impl Camera {
    fn basis(self) -> (f32, f32, f32) {
        let (s, c) = self.rotation.to_radians().sin_cos();
        (c, s, if self.mirrored { -1.0 } else { 1.0 })
    }
    pub fn screen(self, p: Point, rect: Rect) -> Pos2 {
        let (c, s, m) = self.basis();
        let x = (p[0] - self.center[0]) * m;
        let y = p[1] - self.center[1];
        rect.center() + Vec2::new(c * x - s * y, -(s * x + c * y)) * self.scale
    }
    pub fn world(self, p: Pos2, rect: Rect) -> Point {
        let (c, s, m) = self.basis();
        let d = (p - rect.center()) / self.scale;
        [
            self.center[0] + (c * d.x - s * d.y) * m,
            self.center[1] - s * d.x - c * d.y,
        ]
    }
    pub fn fit(&mut self, bounds: Bounds, rect: Rect) {
        self.center = bounds.center();
        let (c, s, _) = self.basis();
        let size = bounds.size();
        let w = size[0] * c.abs() + size[1] * s.abs();
        let h = size[0] * s.abs() + size[1] * c.abs();
        self.scale = ((rect.width() - 120.0).max(100.0) / w.max(0.1))
            .min((rect.height() - 120.0).max(100.0) / h.max(0.1))
            .clamp(0.001, 100_000.0);
    }
    pub fn zoom(&mut self, factor: f32, pivot: Pos2, rect: Rect) {
        let before = self.world(pivot, rect);
        self.scale = (self.scale * factor).clamp(0.001, 100_000.0);
        let after = self.world(pivot, rect);
        self.center[0] += before[0] - after[0];
        self.center[1] += before[1] - after[1];
    }
    pub fn pan(&mut self, delta: Vec2, rect: Rect) {
        let a = self.world(rect.center(), rect);
        let b = self.world(rect.center() + delta, rect);
        self.center[0] -= b[0] - a[0];
        self.center[1] -= b[1] - a[1];
    }
    pub fn view_bounds(self, rect: Rect) -> Bounds {
        let mut b = Bounds::default();
        for p in [
            rect.left_top(),
            rect.right_top(),
            rect.left_bottom(),
            rect.right_bottom(),
        ] {
            b.include(self.world(p, rect));
        }
        b
    }
    fn uniform(self, rect: Rect) -> Uniform {
        let (c, s, m) = self.basis();
        let sx = 2.0 * self.scale / rect.width();
        let sy = 2.0 * self.scale / rect.height();
        Uniform {
            x: [
                c * m * sx,
                -s * sx,
                (-c * m * self.center[0] + s * self.center[1]) * sx,
                0.0,
            ],
            y: [
                s * m * sy,
                c * sy,
                (-s * m * self.center[0] - c * self.center[1]) * sy,
                0.0,
            ],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform {
    x: [f32; 4],
    y: [f32; 4],
}

#[derive(Clone)]
pub struct RenderLayer {
    pub geometry: Arc<Geometry>,
    pub color: [u8; 3],
    pub opacity: f32,
    pub visible: bool,
}

pub struct BoardCallback {
    pub camera: Camera,
    pub rect: Rect,
    pub generation: u64,
    pub layers: Vec<RenderLayer>,
}

struct GpuLayer {
    vertex: wgpu::Buffer,
    index: wgpu::Buffer,
    tint: wgpu::Buffer,
    bind: Option<wgpu::BindGroup>,
}

struct Targets {
    size: [u32; 2],
    mask: wgpu::TextureView,
    msaa: wgpu::TextureView,
    result: wgpu::TextureView,
    present: wgpu::BindGroup,
}

#[derive(PartialEq)]
struct CacheKey {
    camera: Camera,
    size: [u32; 2],
    logical_size: [f32; 2],
    layers: Vec<([u8; 3], f32, bool)>,
}

pub struct Renderer {
    dark: wgpu::RenderPipeline,
    clear: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    present: wgpu::RenderPipeline,
    camera_buffer: wgpu::Buffer,
    camera_bind: wgpu::BindGroup,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    dummy_tint: wgpu::Buffer,
    layers: Vec<GpuLayer>,
    generation: u64,
    targets: Option<Targets>,
    cache: Option<CacheKey>,
}

impl Renderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Gerbrr shaders"),
            source: wgpu::ShaderSource::Wgsl(include_str!("renderer.wgsl").into()),
        });
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Camera layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Composite layout"),
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
        let camera_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Geometry layout"),
                bind_group_layouts: &[Some(&camera_layout)],
                ..Default::default()
            });
        let composite_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Composite pipeline layout"),
                bind_group_layouts: &[Some(&texture_layout)],
                ..Default::default()
            });
        let pipeline = |vertex: &str,
                        fragment: &str,
                        layout: &wgpu::PipelineLayout,
                        format,
                        samples,
                        blend,
                        geometry: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fragment),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vertex),
                    compilation_options: Default::default(),
                    buffers: if geometry {
                        &[Some(wgpu::VertexBufferLayout {
                            array_stride: 8,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &[wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x2,
                                offset: 0,
                                shader_location: 0,
                            }],
                        })]
                    } else {
                        &[]
                    },
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState {
                    count: samples,
                    ..Default::default()
                },
                multiview_mask: None,
                cache: None,
            })
        };
        let dark = pipeline(
            "geometry_vertex",
            "dark",
            &camera_pipeline_layout,
            wgpu::TextureFormat::Rgba8Unorm,
            4,
            None,
            true,
        );
        let clear = pipeline(
            "geometry_vertex",
            "clear",
            &camera_pipeline_layout,
            wgpu::TextureFormat::Rgba8Unorm,
            4,
            None,
            true,
        );
        let composite = pipeline(
            "quad",
            "composite",
            &composite_pipeline_layout,
            wgpu::TextureFormat::Rgba8Unorm,
            1,
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            false,
        );
        let present = pipeline(
            "quad",
            "present",
            &composite_pipeline_layout,
            format,
            1,
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            false,
        );
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Camera"),
            contents: bytemuck::bytes_of(&Uniform::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let camera_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Camera binding"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Layer sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let dummy_tint = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Present tint"),
            contents: bytemuck::cast_slice(&[1.0_f32; 4]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        Self {
            dark,
            clear,
            composite,
            present,
            camera_buffer,
            camera_bind,
            texture_layout,
            sampler,
            dummy_tint,
            layers: vec![],
            generation: u64::MAX,
            targets: None,
            cache: None,
        }
    }

    fn texture_bind(
        &self,
        device: &wgpu::Device,
        texture: &wgpu::TextureView,
        tint: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Layer texture"),
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(texture),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: tint.as_entire_binding(),
                },
            ],
        })
    }

    fn prepare(
        &mut self,
        callback: &BoardCallback,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let max = device.limits().max_texture_dimension_2d;
        let size = [
            (callback.rect.width() * screen.pixels_per_point)
                .round()
                .clamp(1.0, max as f32) as u32,
            (callback.rect.height() * screen.pixels_per_point)
                .round()
                .clamp(1.0, max as f32) as u32,
        ];
        if self.generation != callback.generation {
            self.layers = callback
                .layers
                .iter()
                .map(|layer| {
                    let geometry = &layer.geometry;
                    let vertex = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("PCB vertices"),
                        contents: if geometry.vertices.is_empty() {
                            &[0; 8]
                        } else {
                            bytemuck::cast_slice(&geometry.vertices)
                        },
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                    let index = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("PCB indices"),
                        contents: if geometry.indices.is_empty() {
                            &[0; 4]
                        } else {
                            bytemuck::cast_slice(&geometry.indices)
                        },
                        usage: wgpu::BufferUsages::INDEX,
                    });
                    let tint = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Layer tint"),
                        contents: bytemuck::cast_slice(&[0.0_f32; 4]),
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    });
                    GpuLayer {
                        vertex,
                        index,
                        tint,
                        bind: None,
                    }
                })
                .collect();
            self.generation = callback.generation;
            self.targets = None;
            self.cache = None;
        }
        if self.targets.as_ref().is_none_or(|t| t.size != size) {
            let texture = |label, samples| {
                device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some(label),
                        size: wgpu::Extent3d {
                            width: size[0],
                            height: size[1],
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: samples,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | if samples == 1 {
                                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC
                            } else {
                                wgpu::TextureUsages::empty()
                            },
                        view_formats: &[],
                    })
                    .create_view(&Default::default())
            };
            let mask = texture("Polarity mask", 1);
            let msaa = texture("Antialiased mask", 4);
            let result = texture("Cached PCB composite", 1);
            let present = self.texture_bind(device, &result, &self.dummy_tint);
            for i in 0..self.layers.len() {
                self.layers[i].bind = Some(self.texture_bind(device, &mask, &self.layers[i].tint));
            }
            self.targets = Some(Targets {
                size,
                mask,
                msaa,
                result,
                present,
            });
            self.cache = None;
        }
        let key = CacheKey {
            camera: callback.camera,
            size,
            logical_size: [callback.rect.width(), callback.rect.height()],
            layers: callback
                .layers
                .iter()
                .map(|l| (l.color, l.opacity, l.visible))
                .collect(),
        };
        if self.cache.as_ref() == Some(&key) {
            return;
        }
        queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::bytes_of(&callback.camera.uniform(callback.rect)),
        );
        let targets = self.targets.as_ref().unwrap();
        let view = callback.camera.view_bounds(callback.rect);
        // Clear once even when every layer is hidden.
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Clear composite"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.result,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
        for (layer, gpu) in callback.layers.iter().zip(&self.layers) {
            if !layer.visible || layer.opacity <= 0.0 {
                continue;
            }
            let tint = [
                layer.color[0] as f32 / 255.0,
                layer.color[1] as f32 / 255.0,
                layer.color[2] as f32 / 255.0,
                layer.opacity,
            ];
            queue.write_buffer(&gpu.tint, 0, bytemuck::cast_slice(&tint));
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Rasterize layer mask"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &targets.msaa,
                        depth_slice: None,
                        resolve_target: Some(&targets.mask),
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Discard,
                        },
                    })],
                    ..Default::default()
                });
                pass.set_bind_group(0, &self.camera_bind, &[]);
                pass.set_vertex_buffer(0, gpu.vertex.slice(..));
                pass.set_index_buffer(gpu.index.slice(..), wgpu::IndexFormat::Uint32);
                let mut polarity = None;
                for batch in &layer.geometry.batches {
                    if !batch.bounds.intersects(view) {
                        continue;
                    }
                    if polarity != Some(batch.dark) {
                        pass.set_pipeline(if batch.dark { &self.dark } else { &self.clear });
                        polarity = Some(batch.dark);
                    }
                    pass.draw_indexed(batch.indices.clone(), 0, 0..1);
                }
            }
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Composite layer"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &targets.result,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                pass.set_pipeline(&self.composite);
                pass.set_bind_group(0, gpu.bind.as_ref().unwrap(), &[]);
                pass.draw(0..3, 0..1);
            }
        }
        self.cache = Some(key);
    }
}

impl CallbackTrait for BoardCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(renderer) = resources.get_mut::<Renderer>() {
            renderer.prepare(self, device, queue, screen, encoder);
        }
        vec![]
    }
    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        if let Some(renderer) = resources.get::<Renderer>()
            && let Some(targets) = &renderer.targets
        {
            pass.set_pipeline(&renderer.present);
            pass.set_bind_group(0, &targets.present, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn camera_round_trip_and_zoom_pivot() {
        let rect = Rect::from_min_size(Pos2::new(200.0, 50.0), Vec2::new(900.0, 600.0));
        for mirrored in [false, true] {
            for rotation in [0.0, 90.0, 180.0, 270.0] {
                let mut c = Camera {
                    center: [40.0, 20.0],
                    scale: 12.0,
                    rotation,
                    mirrored,
                };
                let p = [32.0, 16.0];
                let s = c.screen(p, rect);
                let w = c.world(s, rect);
                assert!((w[0] - p[0]).abs() < 1e-5 && (w[1] - p[1]).abs() < 1e-5);
                c.zoom(2.0, s, rect);
                assert!(c.screen(p, rect).distance(s) < 0.001);
            }
        }
    }

    #[test]
    fn measurement_snap_radius_is_screen_constant_through_zoom_rotation_and_mirror() {
        let geometry = crate::parser::parse_gerber(
            "%FSLAX36Y36*%%MOMM*%%ADD10C,1*%D10*X3000000Y4000000D03*M02*",
        )
        .unwrap()
        .geometry;
        let rect = Rect::from_min_size(Pos2::new(292.0, 120.0), Vec2::new(1000.0, 650.0));
        for scale in [0.1, 1.0, 10.0, 1000.0] {
            for rotation in [0.0, 37.0, 90.0, 180.0, 270.0] {
                for mirrored in [false, true] {
                    let camera = Camera {
                        center: [3.0, 4.0],
                        scale,
                        rotation,
                        mirrored,
                    };
                    let target = camera.screen([3.0, 4.0], rect);
                    let inside = camera.world(target + Vec2::new(7.0, 2.0), rect);
                    let outside = camera.world(target + Vec2::new(11.0, 0.0), rect);
                    assert_eq!(
                        geometry
                            .snap(inside, 10.0 / scale, crate::snapping::SnapMode::Centers)
                            .unwrap()
                            .point,
                        [3.0, 4.0]
                    );
                    assert!(
                        geometry
                            .snap(outside, 10.0 / scale, crate::snapping::SnapMode::Centers)
                            .is_none()
                    );
                }
            }
        }
    }

    #[test]
    fn gpu_polarity_composite_and_cache() {
        let instance = wgpu::Instance::default();
        let Ok(adapter) =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        else {
            eprintln!("Skipping GPU test: no adapter available");
            return;
        };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let mut renderer = Renderer::new(&device, wgpu::TextureFormat::Rgba8Unorm);
        let parse = |body: &str| {
            Arc::new(
                crate::parser::parse_gerber(&format!(
                    "%FSLAX36Y36*%%MOMM*%%ADD10C,4*%%ADD11C,2*%D10*X0Y0D03*{body}M02*"
                ))
                .unwrap()
                .geometry,
            )
        };
        let mut callback = BoardCallback {
            camera: Camera {
                scale: 16.0,
                ..Default::default()
            },
            rect: Rect::from_min_size(Pos2::ZERO, Vec2::splat(128.0)),
            generation: 1,
            layers: vec![
                RenderLayer {
                    geometry: parse(""),
                    color: [0, 0, 255],
                    opacity: 1.0,
                    visible: true,
                },
                RenderLayer {
                    geometry: parse("%LPC*%D11*X0Y0D03*"),
                    color: [255, 0, 0],
                    opacity: 0.5,
                    visible: true,
                },
            ],
        };
        let screen = ScreenDescriptor {
            size_in_pixels: [128, 128],
            pixels_per_point: 1.0,
        };
        let render = |renderer: &mut Renderer, callback: &BoardCallback| {
            let mut encoder = device.create_command_encoder(&Default::default());
            renderer.prepare(callback, &device, &queue, &screen, &mut encoder);
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 128 * 512,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                renderer
                    .targets
                    .as_ref()
                    .unwrap()
                    .result
                    .texture()
                    .as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(512),
                        rows_per_image: Some(128),
                    },
                },
                wgpu::Extent3d {
                    width: 128,
                    height: 128,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([encoder.finish()]);
            let (sender, receiver) = std::sync::mpsc::channel();
            readback.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                sender.send(r).unwrap();
            });
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            receiver.recv().unwrap().unwrap();
            let bytes = readback.slice(..).get_mapped_range().unwrap().to_vec();
            readback.unmap();
            bytes
        };
        let bytes = render(&mut renderer, &callback);
        let pixel = |x: usize, y: usize| -> [u8; 4] {
            bytes[y * 512 + x * 4..y * 512 + x * 4 + 4]
                .try_into()
                .unwrap()
        };
        assert_eq!(
            pixel(64, 64),
            [0, 0, 255, 255],
            "Clear top-layer aperture must reveal the bottom layer"
        );
        let ring = pixel(88, 64);
        assert!(
            (ring[0] as i32 - 128).abs() <= 1
                && (ring[2] as i32 - 127).abs() <= 1
                && ring[3] == 255,
            "{ring:?}"
        );
        assert_eq!(pixel(4, 4), [0, 0, 0, 0]);
        let vertex = renderer.layers[0].vertex.clone();
        assert_eq!(
            render(&mut renderer, &callback),
            bytes,
            "Unchanged view must reuse the cached composite"
        );
        assert_eq!(
            renderer.layers[0].vertex, vertex,
            "No geometry reupload on cached frames"
        );
        for layer in &mut callback.layers {
            layer.visible = false;
        }
        assert!(
            render(&mut renderer, &callback).iter().all(|b| *b == 0),
            "Hiding every layer must clear the cache"
        );
    }
}

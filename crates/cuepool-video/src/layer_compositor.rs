//! Video layer compositor: draws each layer's picture into its region of the
//! canvas, lowest layer first, with a per-layer opacity (Stop-cue fades).
//!
//! Same shape as rustjay-mixer's per-channel composite — every layer owns a
//! texture, the canvas is rebuilt from them — minus blend modes and keys. Each
//! layer texture is sized to its region, so the converters' fit math runs
//! unchanged against the region instead of the whole canvas.

use cuepool_core::CanvasRect;
use wgpu::{Device, TextureFormat, TextureView};

/// One layer to draw: its texture (linear view), canvas region and opacity.
pub struct LayerDraw<'a> {
    pub view: &'a TextureView,
    pub region: CanvasRect,
    pub opacity: f32,
}

pub struct LayerCompositor {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl LayerCompositor {
    pub fn new(device: &Device, target_format: TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("layer-composite-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("layer-composite-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("layer-composite-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        // Opacity rides the blend constant: out = layer * k + canvas * (1 - k).
        let fade = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Constant,
            dst_factor: wgpu::BlendFactor::OneMinusConstant,
            operation: wgpu::BlendOperation::Add,
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("layer-composite-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState {
                        color: fade,
                        alpha: fade,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("layer-composite-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            layout,
            sampler,
        }
    }

    /// Clear `canvas_view` to black and draw `layers` in order (last on top).
    pub fn encode(
        &self,
        device: &Device,
        encoder: &mut wgpu::CommandEncoder,
        canvas_view: &TextureView,
        layers: &[LayerDraw<'_>],
    ) {
        // ponytail: one bind group per layer per composite (≤4 small allocations);
        // cache per slot if it ever shows in a profile.
        let bind_groups = layers
            .iter()
            .map(|layer| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("layer-composite-bind-group"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(layer.view),
                        },
                    ],
                })
            })
            .collect::<Vec<_>>();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("layer-composite-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: canvas_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        for (layer, bind_group) in layers.iter().zip(&bind_groups) {
            let opacity = f64::from(layer.opacity.clamp(0.0, 1.0));
            if opacity == 0.0 {
                continue;
            }
            let CanvasRect {
                x,
                y,
                width,
                height,
            } = layer.region;
            pass.set_viewport(x as f32, y as f32, width as f32, height as f32, 0.0, 1.0);
            pass.set_blend_constant(wgpu::Color {
                r: opacity,
                g: opacity,
                b: opacity,
                a: opacity,
            });
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

const SHADER: &str = r#"
@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var tex: texture_2d<f32>;

struct VsOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
  // Fullscreen triangle over the viewport; uv (0,0) at top-left.
  let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
  var o: VsOut;
  o.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
  o.uv = vec2<f32>(p.x, 1.0 - p.y);
  return o;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  return vec4<f32>(textureSampleLevel(tex, samp, in.uv, 0.0).rgb, 1.0);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CanvasTexture;

    fn solid(device: &Device, queue: &wgpu::Queue, rgba: [u8; 4]) -> CanvasTexture {
        let layer = CanvasTexture::new(device, 2, 2);
        layer.upload_rgba(queue, &rgba.repeat(4));
        layer
    }

    /// Composite into a 4×2 target and read it back as rows of RGBA.
    fn composite(
        device: &Device,
        queue: &wgpu::Queue,
        layers: &[(&CanvasTexture, CanvasRect, f32)],
    ) -> Vec<[u8; 4]> {
        let (w, h) = (4u32, 2u32);
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("layer-composite-test-target"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let views = layers
            .iter()
            .map(|(texture, _, _)| texture.render_view())
            .collect::<Vec<_>>();
        let draws = layers
            .iter()
            .zip(&views)
            .map(|((_, region, opacity), view)| LayerDraw {
                view,
                region: *region,
                opacity: *opacity,
            })
            .collect::<Vec<_>>();
        let compositor = LayerCompositor::new(device, TextureFormat::Rgba8Unorm);
        let bytes_per_row = (w * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("layer-composite-test-readback"),
            size: u64::from(bytes_per_row * h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        compositor.encode(
            device,
            &mut encoder,
            &target.create_view(&Default::default()),
            &draws,
        );
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = slice.get_mapped_range().expect("mapped range").to_vec();
        (0..h)
            .flat_map(|row| {
                let start = (row * bytes_per_row) as usize;
                data[start..start + (w * 4) as usize]
                    .chunks_exact(4)
                    .map(|px| [px[0], px[1], px[2], px[3]])
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn rect(x: u32, width: u32) -> CanvasRect {
        CanvasRect {
            x,
            y: 0,
            width,
            height: 2,
        }
    }

    #[test]
    fn layers_land_in_their_regions_side_by_side() {
        let _gpu = crate::gpu_test_lock();
        let Some((device, queue)) = crate::test_device_queue(wgpu::Features::empty()) else {
            return;
        };
        let red = solid(&device, &queue, [255, 0, 0, 255]);
        let blue = solid(&device, &queue, [0, 0, 255, 255]);
        let pixels = composite(
            &device,
            &queue,
            &[(&red, rect(0, 2), 1.0), (&blue, rect(2, 2), 1.0)],
        );
        for row in pixels.chunks_exact(4) {
            assert_eq!(row[0], [255, 0, 0, 255]);
            assert_eq!(row[1], [255, 0, 0, 255]);
            assert_eq!(row[2], [0, 0, 255, 255]);
            assert_eq!(row[3], [0, 0, 255, 255]);
        }
    }

    #[test]
    fn opacity_fades_a_layer_over_what_is_beneath() {
        let _gpu = crate::gpu_test_lock();
        let Some((device, queue)) = crate::test_device_queue(wgpu::Features::empty()) else {
            return;
        };
        let white = solid(&device, &queue, [255, 255, 255, 255]);
        let red = solid(&device, &queue, [255, 0, 0, 255]);
        let pixels = composite(
            &device,
            &queue,
            &[(&red, rect(0, 4), 1.0), (&white, rect(0, 2), 0.5)],
        );
        // Half white over red on the left; red alone on the right.
        let faded = pixels[0];
        assert_eq!(faded[0], 255);
        assert!((126..=129).contains(&faded[1]), "got {faded:?}");
        assert_eq!(pixels[3], [255, 0, 0, 255]);
        // A layer at zero opacity draws nothing.
        let pixels = composite(&device, &queue, &[(&white, rect(0, 4), 0.0)]);
        assert!(pixels.iter().all(|px| px[..3] == [0, 0, 0]));
    }
}

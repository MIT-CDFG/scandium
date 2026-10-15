// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §4.2 (device/queue request, error scopes, "any GpuInitError
//! is reported once and the engine continues on the CPU path"), §4.5
//! (the `cs_composite_full` pipeline over the committed
//! [`crate::wgsl::COMPOSITE_WGSL`] contract), §4.3 (BN64 as `r8uint`),
//! §34.3 (this backend is the candidate the parity harness judges
//! against the CPU oracle), W24 (adapter absence is a STATE, never an
//! error — the CPU path is the unconditional floor); plan §2.3 "wgpu
//! pipelines + WGSL". Deps: deps/wgpu.yaml + deps/pollster.yaml
//! (Apache-2.0 elected; default features only per the record).
//!
//! CP-0.5 scope: the kernel-level [`GpuBackend`] — one `cs_composite_full`
//! dispatch per composite over ping-pong tile textures (the
//! [`crate::wgsl`] binding contract), driving the §34.3 parity gate on
//! real adapters. The full DAG-executor integration (batched passes per
//! §4.1 ordering rules, atlas residency per §3.6, pooled transients) is
//! the CP-0.5→0.7 growth path on the same plan/hash machinery
//! ([`crate::dag`]); landing the kernel + parity leg first keeps the
//! gate green from the first real dispatch onward.
//!
//! Determinism note (§4.5 Dissolve / §34.3): GPU multiplication and
//! comparison are correctly rounded, so the hardened-αs coverage
//! pattern is exact cross-path; GPU DIVISION carries up to 2.5 ULP, so
//! composite bytes may differ from the CPU within the §34.3 numeric
//! tolerances, and the WGSL pins the x/x == 1 identity that exact-
//! equality formula guards depend on (Metal-run finding, lane log).

use crate::parity::{CompositeBackend, KernelCase};
use crate::tile_rgba::{TileRgba, TILE_SCALARS};
use crate::wgsl::{bindings, COMPOSITE_ENTRY, COMPOSITE_WGSL};
use appname_core::TILE;

const TILE_U32: u32 = TILE as u32;
/// rgba32float: 16 B/px → 4096 B/row (already 256-aligned for copies).
const ROW_BYTES: u32 = TILE_U32 * 16;

/// Why GPU init did not produce a backend (§4.2 taxonomy). Never fatal:
/// callers report once and stay on the CPU path (W24).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GpuUnavailable {
    /// No suitable adapter (headless CI, W24 `env_nogpu`, remote
    /// desktop ladders §52.3).
    NoAdapter,
    /// Adapter present but the device request failed (limits/features).
    Device(String),
    /// Pipeline/shader creation failed validation (driver/shader combo
    /// — §4.2: degrade to CPU, never abort).
    Pipeline(String),
}

/// The §4.5 composite kernel on a real wgpu device — the parity-gate
/// candidate backend ([`CompositeBackend`]).
pub struct GpuBackend {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bind_layout: wgpu::BindGroupLayout,
    src_tex: wgpu::Texture,
    dst_tex: wgpu::Texture,
    out_tex: wgpu::Texture,
    bn_tex: wgpu::Texture,
    cov_tex: wgpu::Texture,
    uniforms: wgpu::Buffer,
    readback: wgpu::Buffer,
    name: String,
}

impl GpuBackend {
    /// §4.2 shape: adapter → device/queue → pipeline under an error
    /// scope. Every failure is a typed [`GpuUnavailable`]; the caller's
    /// fallback is the CPU path, unconditionally present.
    pub fn new() -> Result<GpuBackend, GpuUnavailable> {
        // §4.2 backend pin: VULKAN | METAL | DX12 (no GL/WebGPU tiers).
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::METAL | wgpu::Backends::DX12,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|_| GpuUnavailable::NoAdapter)?;
        let info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("appname-render"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            ..Default::default()
        }))
        .map_err(|e| GpuUnavailable::Device(e.to_string()))?;

        // §4.2 device-loss + uncaptured-error surfaces: GPU residency is
        // a cache, never truth — a reset flags rebuild, the document is
        // untouched. (stderr until the engine log surface lands; the
        // §4.2 sample uses the log facade, which has no deps record.)
        device.set_device_lost_callback(|reason, msg| {
            eprintln!("appname-render gpu: device lost: {reason:?} {msg}");
        });
        device.on_uncaptured_error(std::sync::Arc::new(|e: wgpu::Error| {
            eprintln!("appname-render gpu: uncaptured error: {e}");
        }));

        // §4.2: ALL device-object creation (shader, pipeline, textures,
        // buffers) under one validation error scope — a bad shader/
        // driver combo or hostile limits degrade to the CPU floor,
        // never abort (slice-D review MINOR-4: creation previously ran
        // partly outside the scope).
        let error_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("composite.wgsl"),
            source: wgpu::ShaderSource::Wgsl(COMPOSITE_WGSL.into()),
        });

        let tex_entry =
            |binding: u32, sample: wgpu::TextureSampleType| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: sample,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            };
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("composite-bgl"),
            entries: &[
                tex_entry(
                    bindings::SRC,
                    wgpu::TextureSampleType::Float { filterable: false },
                ),
                tex_entry(
                    bindings::DST_IN,
                    wgpu::TextureSampleType::Float { filterable: false },
                ),
                wgpu::BindGroupLayoutEntry {
                    binding: bindings::OUT,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba32Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                tex_entry(bindings::BN, wgpu::TextureSampleType::Uint),
                wgpu::BindGroupLayoutEntry {
                    binding: bindings::UNIFORMS,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // §11.2 coverage E (r32float, non-filterable — sampled by
                // integer texel coords, exactly like src/dst).
                tex_entry(
                    bindings::COV,
                    wgpu::TextureSampleType::Float { filterable: false },
                ),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("composite-pl"),
            bind_group_layouts: &[Some(&bind_layout)],
            ..Default::default()
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(COMPOSITE_ENTRY),
            layout: Some(&layout),
            module: &shader,
            entry_point: Some(COMPOSITE_ENTRY),
            compilation_options: Default::default(),
            cache: None,
        });
        let tile_tex = |label: &str, format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: TILE_U32,
                    height: TILE_U32,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        use wgpu::TextureUsages as TU;
        let src_tex = tile_tex(
            "src",
            wgpu::TextureFormat::Rgba32Float,
            TU::TEXTURE_BINDING | TU::COPY_DST,
        );
        let dst_tex = tile_tex(
            "dst",
            wgpu::TextureFormat::Rgba32Float,
            TU::TEXTURE_BINDING | TU::COPY_DST,
        );
        let out_tex = tile_tex(
            "out",
            wgpu::TextureFormat::Rgba32Float,
            TU::STORAGE_BINDING | TU::COPY_SRC,
        );
        // §11.2 coverage E, one r32float scalar per pixel.
        let cov_tex = tile_tex(
            "coverage",
            wgpu::TextureFormat::R32Float,
            TU::TEXTURE_BINDING | TU::COPY_DST,
        );

        // BN64 as r8uint 64×64 — "the same 4096 bytes everywhere"
        // (§4.5); rows padded to the 256-byte copy alignment.
        let bn_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("bn64"),
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Uint,
            usage: TU::TEXTURE_BINDING | TU::COPY_DST,
            view_formats: &[],
        });
        let mut bn_padded = vec![0u8; 256 * 64];
        for (row, chunk) in crate::bn64::BN64.chunks(64).enumerate() {
            bn_padded[row * 256..row * 256 + 64].copy_from_slice(chunk);
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &bn_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bn_padded,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(64),
            },
            wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
        );

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: 32, // 24 B struct (wgsl::bindings contract), rounded up
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (ROW_BYTES * TILE_U32) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        if let Some(e) = pollster::block_on(error_scope.pop()) {
            return Err(GpuUnavailable::Pipeline(e.to_string()));
        }

        Ok(GpuBackend {
            device,
            queue,
            pipeline,
            bind_layout,
            src_tex,
            dst_tex,
            out_tex,
            bn_tex,
            cov_tex,
            uniforms,
            readback,
            name: format!("wgpu-{:?}-{}", info.backend, info.name),
        })
    }

    fn upload_tile(&self, tex: &wgpu::Texture, tile: &TileRgba) {
        let bytes: Vec<u8> = tile
            .as_slice()
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(ROW_BYTES),
                rows_per_image: Some(TILE_U32),
            },
            wgpu::Extent3d {
                width: TILE_U32,
                height: TILE_U32,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Upload the §11.2 coverage plane (one r32float scalar per pixel).
    /// `None` uploads an all-1.0 identity plane, so an unmasked case's
    /// `src *= 1.0` is a bit-exact no-op. `cov` (when present) is exactly
    /// the `TILE·TILE` E plane the CPU multiplies — same bytes, both paths.
    fn upload_coverage(&self, cov: Option<&[f32]>) {
        // r32float: 4 B/px → TILE·4 B/row (256-aligned already at TILE=256).
        let mut bytes = vec![0u8; (TILE * TILE * 4) as usize];
        match cov {
            Some(e) => {
                debug_assert_eq!(e.len(), TILE * TILE, "coverage spans the tile");
                for (dst, v) in bytes.chunks_exact_mut(4).zip(e) {
                    dst.copy_from_slice(&v.to_le_bytes());
                }
            }
            None => {
                for dst in bytes.chunks_exact_mut(4) {
                    dst.copy_from_slice(&1.0f32.to_le_bytes());
                }
            }
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.cov_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(TILE_U32 * 4),
                rows_per_image: Some(TILE_U32),
            },
            wgpu::Extent3d {
                width: TILE_U32,
                height: TILE_U32,
                depth_or_array_layers: 1,
            },
        );
    }
}

impl CompositeBackend for GpuBackend {
    fn name(&self) -> &str {
        &self.name
    }

    /// Gate-scope panic contract (slice-D review MINOR-4, accepted for
    /// CP-0.5): a device lost MID-RUN panics loudly through the expects
    /// below — correct for the parity harness, whose only consumer is
    /// the test tier. `CompositeBackend::composite` has no error
    /// channel; the CP-0.7 engine integration widens the trait to a
    /// fallible form so §4.2's "degrade, never abort" holds on the
    /// interactive path too (lane-log growth note).
    fn composite(&mut self, case: &KernelCase) -> TileRgba {
        self.upload_tile(&self.src_tex, &case.src);
        self.upload_tile(&self.dst_tex, &case.dst);
        // §11.2 coverage: the case's per-pixel E, or an all-1.0 identity
        // plane when unmasked (`src *= 1.0` is bit-exact, so unmasked cases
        // stay identical to the pre-mask kernel).
        self.upload_coverage(case.coverage.as_deref());

        // Uniforms per the wgsl::bindings contract: { mode: u32 @0,
        // opacity: f32 @4, bn_off: vec2<i32> @8, tile_origin: vec2<i32>
        // @16 } — manual LE packing (no POD-cast dependency).
        let mut ub = [0u8; 32];
        ub[0..4].copy_from_slice(&(case.mode as u32).to_le_bytes());
        ub[4..8].copy_from_slice(&case.opacity.to_le_bytes());
        ub[8..12].copy_from_slice(&case.ctx.bn_off.0.to_le_bytes());
        ub[12..16].copy_from_slice(&case.ctx.bn_off.1.to_le_bytes());
        ub[16..20].copy_from_slice(&case.ctx.tile_origin.0.to_le_bytes());
        ub[20..24].copy_from_slice(&case.ctx.tile_origin.1.to_le_bytes());
        self.queue.write_buffer(&self.uniforms, 0, &ub);

        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("composite-bind"),
            layout: &self.bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: bindings::SRC,
                    resource: wgpu::BindingResource::TextureView(&view(&self.src_tex)),
                },
                wgpu::BindGroupEntry {
                    binding: bindings::DST_IN,
                    resource: wgpu::BindingResource::TextureView(&view(&self.dst_tex)),
                },
                wgpu::BindGroupEntry {
                    binding: bindings::OUT,
                    resource: wgpu::BindingResource::TextureView(&view(&self.out_tex)),
                },
                wgpu::BindGroupEntry {
                    binding: bindings::BN,
                    resource: wgpu::BindingResource::TextureView(&view(&self.bn_tex)),
                },
                wgpu::BindGroupEntry {
                    binding: bindings::UNIFORMS,
                    resource: self.uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: bindings::COV,
                    resource: wgpu::BindingResource::TextureView(&view(&self.cov_tex)),
                },
            ],
        });

        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(TILE_U32 / 16, TILE_U32 / 16, 1);
        }
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.out_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(ROW_BYTES),
                    rows_per_image: Some(TILE_U32),
                },
            },
            wgpu::Extent3d {
                width: TILE_U32,
                height: TILE_U32,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([enc.finish()]);

        let slice = self.readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely()).ok();
        rx.recv()
            .expect("map callback delivered")
            .expect("readback map");
        let data = slice.get_mapped_range().expect("mapped range");
        let mut out = TileRgba::transparent();
        debug_assert_eq!(data.len(), TILE_SCALARS * 4);
        for (i, chunk) in data.chunks_exact(4).enumerate() {
            out.as_mut_slice()[i] = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        drop(data);
        self.readback.unmap();
        out
    }
}

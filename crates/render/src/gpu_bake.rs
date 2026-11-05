//! GPU mesh-map baking: [`GpuBaker`] runs the ray casts of `texturecraft_bake` (a
//! [`Tracer`]) as compute shaders (`shaders/bake.wgsl` on the path tracer's BVH traversal in
//! `shaders/rays.wgsl`), on a device of its own created on the first bake. The CPU tracer is the
//! oracle: the GPU gets the same rays and queries and casts the same secondary-ray directions,
//! so both bakes agree up to float rounding. Any GPU failure is a [`BakeError::Tracer`], after
//! which `bake_on` bakes on the CPU.

use std::sync::{Arc, OnceLock};

use bytemuck::{Pod, Zeroable};
use rayon::prelude::*;
use texturecraft_bake::BakeError;
use texturecraft_bake::tracing::{ANY, Hemisphere, Query, Ray, RayKind, RayScene, TraceMesh, Tracer};
use texturecraft_core::{AmbientOcclusionAttenuation, BackfaceMode};
use texturecraft_geom::Hit;
use texturecraft_geom::glam::Vec3;

use crate::gpu_trace::{STACK, buffer, bvh_depth, bvh_rows, entry, scoped, storage};

const WORKGROUP: u32 = 64;
/// Rays per dispatch: short dispatches keep slow GPUs clear of driver watchdogs.
const RAYS_PER_DISPATCH: usize = 1 << 24;
const NONE: u32 = u32::MAX;
const UNIFORM: u32 = 1;
const IGNORE_BACKFACE: u32 = 2;
const SAME_KEY: u32 = 4;

/// `Params` of bake.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
struct Params {
    count: u32,
    offset: u32,
    kind: u32,
    rays: u32,
    dmax: f32,
    eps: f32,
    cos_max: f32,
    sin_max: f32,
    flags: u32,
    root: u32,
    /// Threads per query (hemisphere).
    lanes: u32,
    /// First result row of the pass.
    out: u32,
    min_distance: f32,
    attenuation: u32,
    ground_height: f32,
    backface_mode: u32,
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    name: String,
    layout: wgpu::BindGroupLayout,
    transfer: wgpu::ComputePipeline,
    hemisphere: wgpu::ComputePipeline,
    /// Bytes between the per-dispatch parameter blocks (the dynamic uniform offset alignment).
    stride: usize,
}

fn err(e: impl std::fmt::Display) -> BakeError {
    BakeError::Tracer(format!("GPU baking: {e}"))
}

impl Gpu {
    fn new() -> Result<Gpu, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let options = wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() };
        let adapter = pollster::block_on(instance.request_adapter(&options)).map_err(|e| format!("no GPU adapter: {e}"))?;
        let info = adapter.get_info();
        if info.device_type == wgpu::DeviceType::Cpu {
            return Err(format!("{} is a software renderer", info.name));
        }
        let limits = adapter.limits();
        if limits.max_storage_buffers_per_shader_stage < 3
            || limits.max_compute_invocations_per_workgroup < WORKGROUP
            || limits.max_compute_workgroup_size_x < WORKGROUP
        {
            return Err(format!("{} has no usable compute shaders", info.name));
        }
        let desc = wgpu::DeviceDescriptor { label: Some("tc_bake"), required_limits: limits, ..Default::default() };
        let (device, queue) = pollster::block_on(adapter.request_device(&desc)).map_err(|e| format!("no GPU device: {e}"))?;
        // Errors are caught by the scopes around every call; never let wgpu's default handler panic.
        device.on_uncaptured_error(Arc::new(|_| {}));
        let (layout, transfer, hemisphere) = scoped(&device, "GPU baking: pipelines", || {
            let params = wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: wgpu::BufferSize::new(size_of::<Params>() as u64),
            };
            let entries = [entry(0, params), storage(1, true), storage(2, true), storage(3, true), storage(4, false)];
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("tc_bake"), entries: &entries });
            let source = concat!(include_str!("shaders/rays.wgsl"), include_str!("shaders/bake.wgsl"));
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("tc_bake"), source: wgpu::ShaderSource::Wgsl(source.into()) });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("tc_bake"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
            let pipe = |entry_point: &str| {
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry_point),
                    layout: Some(&pipeline_layout),
                    module: &module,
                    entry_point: Some(entry_point),
                    compilation_options: Default::default(),
                    cache: None,
                })
            };
            (layout, pipe("transfer_main"), pipe("hemisphere_main"))
        })?;
        let stride = size_of::<Params>().next_multiple_of(device.limits().min_uniform_buffer_offset_alignment.max(1) as usize);
        Ok(Gpu { device, queue, name: info.name, layout, transfer, hemisphere, stride })
    }

    /// The largest storage buffer this device binds.
    fn max_buffer(&self) -> u64 {
        let l = self.device.limits();
        l.max_storage_buffer_binding_size.min(l.max_buffer_size)
    }
}

/// The GPU [`Tracer`]. Its device is created on the first bake; without a hardware adapter
/// every bake reports [`BakeError::Tracer`] (and runs on the CPU).
#[derive(Default)]
pub struct GpuBaker {
    gpu: OnceLock<Result<Gpu, String>>,
}

impl GpuBaker {
    pub fn new() -> GpuBaker {
        GpuBaker::default()
    }

    fn gpu(&self) -> Result<&Gpu, BakeError> {
        self.gpu.get_or_init(Gpu::new).as_ref().map_err(|e| BakeError::Tracer(e.clone()))
    }
}

impl Tracer for GpuBaker {
    fn name(&self) -> String {
        match self.gpu.get() {
            Some(Ok(g)) => format!("GPU ({})", g.name),
            _ => "GPU".into(),
        }
    }

    fn scene<'a>(&'a self, meshes: &[TraceMesh<'a>]) -> Result<Box<dyn RayScene + 'a>, BakeError> {
        let gpu = self.gpu()?;
        let (mut nodes, mut tris, mut roots) = (Vec::new(), Vec::new(), Vec::new());
        for m in meshes {
            let tree: Vec<_> = m.bvh.nodes().collect();
            if bvh_depth(&tree).is_none_or(|d| d >= STACK) {
                return Err(err("a mesh's BVH is too deep for GPU traversal"));
            }
            let base = ((nodes.len() / 2) as u32, (tris.len() / 4) as u32);
            roots.push(base.0);
            let (n, t) = bvh_rows(m.bvh, base, |o| {
                ([m.keys.get(o).copied().unwrap_or(ANY), u32::from(m.backface_culling.get(o).copied().unwrap_or(false)), 0], [0, 0, 0, o as u32])
            });
            nodes.extend(n);
            tris.extend(t);
        }
        if (tris.len() as u64).saturating_mul(16) > gpu.max_buffer() || (nodes.len() as u64).saturating_mul(16) > gpu.max_buffer() {
            return Err(err(format!("the meshes ({} MB) exceed this GPU's buffer limit", tris.len() >> 16)));
        }
        let (nodes, tris) = scoped(&gpu.device, "GPU baking: mesh upload", || {
            (buffer(&gpu.device, "tc_bake_nodes", bytemuck::cast_slice(&nodes)), buffer(&gpu.device, "tc_bake_tris", bytemuck::cast_slice(&tris)))
        })
        .map_err(BakeError::Tracer)?;
        Ok(Box::new(GpuScene { gpu, nodes, tris, roots, io: None }))
    }
}

/// The meshes of one bake on the GPU: every BVH in one node and one triangle buffer.
struct GpuScene<'g> {
    gpu: &'g Gpu,
    nodes: wgpu::Buffer,
    tris: wgpu::Buffer,
    /// Root node of each mesh.
    roots: Vec<u32>,
    /// Batch buffers, grown as needed and reused across batches.
    io: Option<Io>,
}

struct Io {
    items: wgpu::Buffer,
    results: wgpu::Buffer,
    staging: wgpu::Buffer,
    params: wgpu::Buffer,
    bind: wgpu::BindGroup,
}

/// One pass over every item: its parameters and the items per dispatch.
struct Pass {
    params: Params,
    per_dispatch: usize,
    /// Items per workgroup.
    per_group: usize,
}

impl GpuScene<'_> {
    fn root(&self, mesh: usize) -> Result<u32, BakeError> {
        self.roots.get(mesh).copied().ok_or_else(|| err(format!("no scene mesh {mesh}")))
    }

    /// Batch buffers of at least these sizes (bytes).
    fn io(&mut self, items: u64, results: u64, params: u64) -> Result<&Io, BakeError> {
        let fits = |io: &Io| io.items.size() >= items && io.results.size() >= results && io.params.size() >= params;
        if !self.io.as_ref().is_some_and(fits) {
            let (g, dev) = (self.gpu, &self.gpu.device);
            let size = |b: u64| b.max(256).next_power_of_two().min(g.max_buffer()).max(b);
            let io = scoped(dev, "GPU baking: batch buffers", || {
                let buf = |label, size, usage| dev.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false });
                let items = buf("tc_bake_items", size(items), wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
                let results = buf("tc_bake_results", size(results), wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC);
                let staging = buf("tc_bake_staging", results.size(), wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST);
                let params = buf("tc_bake_params", size(params), wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
                let block = wgpu::BufferBinding { buffer: &params, offset: 0, size: wgpu::BufferSize::new(size_of::<Params>() as u64) };
                let bind = dev.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("tc_bake"),
                    layout: &g.layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Buffer(block) },
                        wgpu::BindGroupEntry { binding: 1, resource: self.nodes.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 2, resource: self.tris.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 3, resource: items.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 4, resource: results.as_entire_binding() },
                    ],
                });
                Io { items, results, staging, params, bind }
            })
            .map_err(BakeError::Tracer)?;
            self.io = Some(io);
        }
        self.io.as_ref().ok_or_else(|| err("no batch buffers"))
    }

    /// Run `pipeline` over `n` items (`items`: their rows) once per pass in one submission, and
    /// hand `read` one result row per item and pass (pass after pass).
    fn run<R: Default>(
        &mut self,
        pipeline: &wgpu::ComputePipeline,
        items: &[u8],
        n: usize,
        passes: &[Pass],
        read: impl FnOnce(&[[u32; 4]]) -> R,
    ) -> Result<R, BakeError> {
        if n == 0 || passes.is_empty() {
            return Ok(R::default());
        }
        let g = self.gpu;
        let max_groups = g.device.limits().max_compute_workgroups_per_dimension as usize;
        let (mut blocks, mut dispatches) = (Vec::new(), Vec::new());
        for (k, p) in passes.iter().enumerate() {
            let chunk = p.per_dispatch.clamp(p.per_group, max_groups * p.per_group) / p.per_group * p.per_group;
            for offset in (0..n).step_by(chunk) {
                let count = chunk.min(n - offset);
                dispatches.push((blocks.len() as u32, count.div_ceil(p.per_group) as u32));
                let block = Params { offset: offset as u32, count: count as u32, out: (k * n) as u32, ..p.params };
                blocks.extend_from_slice(bytemuck::bytes_of(&block));
                blocks.resize(blocks.len().next_multiple_of(g.stride), 0);
            }
        }
        let (in_bytes, out_bytes) = (items.len() as u64, (n * passes.len()) as u64 * 16);
        if in_bytes.max(out_bytes) > g.max_buffer() {
            return Err(err("a batch exceeds this GPU's buffer limit"));
        }
        let io = self.io(in_bytes, out_bytes, blocks.len() as u64)?;
        let dev = &g.device;
        scoped(dev, "GPU baking: dispatch", || {
            g.queue.write_buffer(&io.items, 0, items);
            g.queue.write_buffer(&io.params, 0, &blocks);
            let mut enc = dev.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("tc_bake") });
            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("tc_bake"), timestamp_writes: None });
                pass.set_pipeline(pipeline);
                for &(offset, groups) in &dispatches {
                    pass.set_bind_group(0, &io.bind, &[offset]);
                    pass.dispatch_workgroups(groups, 1, 1);
                }
            }
            enc.copy_buffer_to_buffer(&io.results, 0, &io.staging, 0, out_bytes);
            g.queue.submit([enc.finish()]);
        })
        .map_err(BakeError::Tracer)?;
        let slice = io.staging.slice(..out_bytes);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        dev.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).map_err(err)?;
        rx.recv().map_err(err)?.map_err(err)?;
        let data = slice.get_mapped_range().map_err(err)?;
        let out = bytemuck::try_cast_slice::<u8, [u32; 4]>(&data).map(read).map_err(err);
        drop(data);
        io.staging.unmap();
        out
    }
}

/// A row of `v` (as bits) and `w`.
fn row(v: Vec3, w: u32) -> [u32; 4] {
    [v.x.to_bits(), v.y.to_bits(), v.z.to_bits(), w]
}

impl RayScene for GpuScene<'_> {
    fn closest(&mut self, mesh: usize, rays: &[Ray], ignore_backface: bool) -> Result<Vec<Option<Hit>>, BakeError> {
        let pass = Pass {
            params: Params { root: self.root(mesh)?, flags: u32::from(ignore_backface) * IGNORE_BACKFACE, ..Params::default() },
            per_dispatch: RAYS_PER_DISPATCH,
            per_group: WORKGROUP as usize,
        };
        let gpu = self.gpu;
        self.run(&gpu.transfer, bytemuck::cast_slice(rays), rays.len(), &[pass], |out| {
            out.par_iter().map(|&[t, u, v, tri]| (tri != NONE).then(|| Hit { t: f32::from_bits(t), tri, u: f32::from_bits(u), v: f32::from_bits(v) })).collect()
        })
    }

    fn hemispheres(&mut self, passes: &[Hemisphere], queries: &[Query]) -> Result<Vec<Vec<[f32; 4]>>, BakeError> {
        let roots = &self.roots;
        let items = queries
            .par_iter()
            .map(|q| {
                let root = roots.get(q.mesh as usize).copied().ok_or_else(|| err(format!("no scene mesh {}", q.mesh)))?;
                Ok([row(q.pos, q.rot[0].to_bits()), row(q.nrm, q.rot[1].to_bits()), row(q.geo, q.key), [root, 0, 0, 0]])
            })
            .collect::<Result<Vec<_>, BakeError>>()?
            .concat();
        let passes: Vec<Pass> = passes
            .iter()
            .map(|h| {
                // A sample's rays spread over `lanes` threads of a workgroup (a power of two).
                let lanes = h.rays.clamp(1, WORKGROUP).next_power_of_two();
                let flags =
                    [(h.uniform, UNIFORM), (h.ground_height.is_some(), 8), (h.same_key, SAME_KEY)].iter().filter(|(on, _)| *on).fold(0, |f, (_, b)| f | b);
                let params = Params {
                    kind: u32::from(h.kind == RayKind::Thickness),
                    rays: h.rays,
                    dmax: h.dmax,
                    min_distance: h.min_distance,
                    attenuation: match h.attenuation {
                        AmbientOcclusionAttenuation::None => 0,
                        AmbientOcclusionAttenuation::Linear => 1,
                        AmbientOcclusionAttenuation::Smooth => 2,
                    },
                    ground_height: h.ground_height.unwrap_or(0.0),
                    backface_mode: match h.ignore_backface {
                        BackfaceMode::Never => 0,
                        BackfaceMode::Always => 1,
                        BackfaceMode::ByMeshName => 2,
                    },
                    eps: h.eps,
                    cos_max: h.cos_max,
                    sin_max: h.sin_max,
                    flags,
                    lanes,
                    ..Params::default()
                };
                Pass { params, per_dispatch: RAYS_PER_DISPATCH / h.rays.max(1) as usize, per_group: (WORKGROUP / lanes) as usize }
            })
            .collect();
        let gpu = self.gpu;
        self.run(&gpu.hemisphere, bytemuck::cast_slice(&items), queries.len(), &passes, |out| {
            out.chunks(queries.len().max(1)).map(|c| c.iter().map(|v| v.map(f32::from_bits)).collect()).collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;
    use texturecraft_core::{
        AmbientOcclusionSettings, BakeAntialiasing, BakeResolution, BentNormalSettings, CageSettings, CommonBakeSettings, CurvatureSettings, HighPolySettings,
        ThicknessSettings,
    };

    use texturecraft_bake::{BakeOutput, BakeRequest, HighPoly, Progress, bake, bake_on};
    use texturecraft_core::{BackfaceMode, BakeSettings, BakedMap, CurvatureMethod, RayDistribution, RaySettings, SelfOcclusion};
    use texturecraft_geom::{Bvh, Mesh, primitives};

    use super::*;

    /// The GPU baker, or None (test skipped) without a hardware adapter.
    fn gpu() -> Option<GpuBaker> {
        let g = GpuBaker::new();
        match g.gpu() {
            Ok(_) => Some(g),
            Err(e) => {
                eprintln!("skipping: {e}");
                None
            }
        }
    }

    fn run(low: &Mesh, high: Option<&Mesh>, s: &BakeSettings, tracer: Option<&GpuBaker>) -> BakeOutput {
        let (lb, hb) = (Bvh::new(low), high.map(Bvh::new));
        let high = high.zip(hb.as_ref()).map(|(mesh, bvh)| HighPoly { file_ids: &[], mesh, bvh, base_color: &[] });
        let req =
            BakeRequest { low_base_color: &[], mesh: low, bvh: &lb, material: 0, settings: s, high, cage: None, skew: None, cage_paint: None, tiles: (1, 1) };
        match tracer {
            Some(t) => bake_on(&req, &Progress::new(), t).unwrap(),
            None => bake(&req, &Progress::new()).unwrap(),
        }
    }

    /// Per map: mean absolute difference, 99th percentile and share of values off by > 0.05.
    fn assert_agree(cpu: &BakeOutput, gpu: &BakeOutput) {
        assert!(gpu.tracer.starts_with("GPU"), "{}", gpu.tracer);
        assert_eq!(cpu.maps.keys().collect::<Vec<_>>(), gpu.maps.keys().collect::<Vec<_>>());
        for (m, a) in &cpu.maps {
            let mut d: Vec<f32> = a.iter().zip(&gpu.maps[m]).map(|(a, b)| (a - b).abs()).collect();
            d.sort_by(f32::total_cmp);
            let mean = d.iter().sum::<f32>() / d.len() as f32;
            let p99 = d[d.len() * 99 / 100];
            let off = d.iter().filter(|v| **v > 0.05).count() as f32 / d.len() as f32;
            eprintln!("{m:?}: mean {mean:.2e}, p99 {p99:.2e}, off {:.3}%", off * 100.0);
            assert!(mean < 2e-3 && p99 < 0.02 && off < 0.005, "{m:?}: mean {mean}, p99 {p99}, off {off}");
        }
    }

    #[test]
    fn gpu_respects_secondary_ray_filters_and_ground_plane() {
        let Some(gpu) = gpu() else { return };

        let mut mesh = primitives::material_ball();
        for part in &mut mesh.submeshes {
            part.name.push_str("_ignorebf");
        }
        let mut settings = BakeSettings::default();
        settings.common.resolution = BakeResolution::R64;
        settings.common.apply_diffusion = false;
        settings.maps.select([BakedMap::AmbientOcclusion, BakedMap::BentNormal, BakedMap::Thickness]);
        settings.high_poly.ignore_backface_suffix = "_ignorebf".into();
        settings.maps.ao.parameters.rays.min_distance = 0.02;
        settings.maps.ao.parameters.ignore_backface = BackfaceMode::ByMeshName;
        settings.maps.ao.parameters.self_occlusion = SelfOcclusion::SameMeshName;
        settings.maps.ao.parameters.ground_plane.enabled = true;
        settings.maps.ao.parameters.ground_plane.offset = 0.1;
        settings.maps.bent_normal.parameters.rays.min_distance = 0.02;
        settings.maps.bent_normal.parameters.ignore_backface = BackfaceMode::ByMeshName;
        settings.maps.thickness.parameters.rays.min_distance = 0.02;
        settings.maps.thickness.parameters.self_occlusion = SelfOcclusion::SameMeshName;
        for attenuation in [AmbientOcclusionAttenuation::None, AmbientOcclusionAttenuation::Smooth] {
            settings.maps.ao.parameters.attenuation = attenuation;
            assert_agree(&run(&mesh, None, &settings, None), &run(&mesh, None, &settings, Some(&gpu)));
        }
    }

    #[test]
    fn gpu_bake_matches_the_cpu() {
        let Some(gpu) = gpu() else { return };

        // Low poly: the material ball (self-occluding); every ray map with its own settings
        let ball = primitives::material_ball();
        let maps = [BakedMap::AmbientOcclusion, BakedMap::BentNormal, BakedMap::Thickness, BakedMap::Position, BakedMap::Curvature];
        let settings = {
            let mut base = BakeSettings::default();
            base.common = CommonBakeSettings { resolution: BakeResolution::R128, antialiasing: BakeAntialiasing::Subsampling2x2, dilation: 0, ..base.common };
            base.maps.select(maps);
            base.maps.ao.parameters = AmbientOcclusionSettings {
                rays: RaySettings { rays: 24, max_distance: 0.3, spread: 150.0, distribution: RayDistribution::Uniform, ..RaySettings::default() },
                ignore_backface: BackfaceMode::Always,
                ..base.maps.ao.parameters
            };
            base.maps.curvature.parameters = CurvatureSettings { method: CurvatureMethod::Mesh, ..base.maps.curvature.parameters };
            base.maps.bent_normal.parameters =
                BentNormalSettings { rays: RaySettings { rays: 16, ..RaySettings::default() }, ..base.maps.bent_normal.parameters };
            base.maps.thickness.parameters =
                ThicknessSettings { rays: RaySettings { rays: 20, spread: 90.0, ..RaySettings::default() }, ..base.maps.thickness.parameters };
            base
        };

        assert_agree(&run(&ball, None, &settings, None), &run(&ball, None, &settings, Some(&gpu)));

        // A bumpy high poly over a sphere, matched by name: the transfer maps and AO on it
        let low = primitives::sphere(1.0, 32, 16);
        let mut high = primitives::sphere(1.0, 128, 64);
        for p in &mut high.positions {
            *p *= 1.0 + 0.02 * (p.x * 9.0).sin() * (p.y * 7.0).cos();
        }
        high.compute_normals();
        high.compute_tangents();
        high.submeshes[0].name = format!("{}_high", low.submeshes[0].name);
        let settings = {
            let mut base = BakeSettings::default();
            base.common = CommonBakeSettings { resolution: BakeResolution::R128, dilation: 0, ..base.common };
            base.maps.select([
                BakedMap::Normal,
                BakedMap::Height,
                BakedMap::Id,
                BakedMap::Opacity,
                BakedMap::AmbientOcclusion,
                BakedMap::WorldNormal,
                BakedMap::Curvature,
            ]);
            base.cage = CageSettings { max_frontal: 0.02, max_rear: 0.02, ..base.cage };
            base.high_poly = HighPolySettings {
                match_mode: texturecraft_core::MeshMatchMode::ByMeshName,
                low_suffix: String::new(),
                use_low_as_high: false,
                ..base.high_poly
            };
            base.maps.ao.parameters = AmbientOcclusionSettings {
                rays: RaySettings { rays: 16, ..RaySettings::default() },
                self_occlusion: SelfOcclusion::SameMeshName,
                ..base.maps.ao.parameters
            };
            base.maps.curvature.parameters = CurvatureSettings { method: CurvatureMethod::Mesh, ..base.maps.curvature.parameters };
            base
        };

        assert_agree(&run(&low, Some(&high), &settings, None), &run(&low, Some(&high), &settings, Some(&gpu)));
    }

    /// `cargo test -p texturecraft-render --release -- --ignored --nocapture gpu_bake_speed`
    #[test]
    #[ignore]
    fn gpu_bake_speed() {
        let Some(g) = gpu() else { return };
        let ball = primitives::material_ball();
        let rays = RaySettings { rays: 64, ..RaySettings::default() };
        let s = {
            let mut base = BakeSettings::default();
            base.common = CommonBakeSettings { resolution: BakeResolution::R2048, ..base.common };
            base.maps.ao.parameters = AmbientOcclusionSettings { rays, ..base.maps.ao.parameters };
            base.maps.thickness.parameters = ThicknessSettings { rays, ..base.maps.thickness.parameters };
            base.maps.bent_normal.parameters = BentNormalSettings { rays, ..base.maps.bent_normal.parameters };
            base
        };
        run(
            &ball,
            None,
            &{
                let mut base = s.clone();
                base.common = CommonBakeSettings { resolution: BakeResolution::R64, ..base.common };
                base
            },
            Some(&g),
        );
        let time = |t: Option<&GpuBaker>| {
            let start = Instant::now();
            let out = run(&ball, None, &s, t);
            (start.elapsed(), out)
        };
        let (cpu_t, cpu) = time(None);
        let (gpu_t, gpu) = time(Some(&g));
        let (gpu_t2, _) = time(Some(&g));
        println!("GPU again (warm clocks): {gpu_t2:.2?}");
        println!(
            "material ball 2048², {} maps, 64 rays: CPU {cpu_t:.2?}, {} {gpu_t:.2?}, {:.1}x",
            s.maps.enabled().count(),
            gpu.tracer,
            cpu_t.as_secs_f64() / gpu_t.as_secs_f64()
        );
        assert_agree(&cpu, &gpu);
        for maps in [vec![BakedMap::Position], vec![BakedMap::AmbientOcclusion], vec![BakedMap::Thickness]] {
            let s = {
                let mut base = s.clone();
                base.maps.select(maps);
                base
            };
            let start = Instant::now();
            run(&ball, None, &s, Some(&g));
            println!("{:?}: {:.2?}", s.maps.enabled().collect::<Vec<_>>(), start.elapsed());
        }
    }
}

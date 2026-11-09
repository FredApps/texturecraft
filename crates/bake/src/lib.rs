//! Deterministic mesh-map baking with bounded batches and CPU fallback

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod bake_error;
mod bake_output;
mod bake_request;
mod cage;
mod high_poly;
mod mesh_extent;
mod progress;

pub use bake_error::BakeError;
pub use bake_output::BakeOutput;
pub use bake_request::BakeRequest;
pub use cage::{CageShell, HighPolyReach, RayStart, VertexReach};
pub use high_poly::HighPoly;
pub(crate) use mesh_extent::MeshExtent;
pub use progress::Progress;

mod map;
mod skew;
#[cfg(test)]
mod tests;
pub mod tracing;

use std::collections::BTreeMap;
use std::sync::atomic::Ordering::Relaxed;

use rayon::prelude::*;
use texturecraft_core::{BakedMap, CurvatureMethod};
use texturecraft_geom::glam::{Vec2, Vec3};
use texturecraft_geom::{Mesh, NO_TRI, Surface, UvGBuffer};
use texturecraft_texel::DilationMap;

mod bake_paint_map;
use crate::tracing::{CpuTracer, RayScene, Tracer};
pub use bake_paint_map::BakePaintMap;

pub const MAX_RESOLUTION: u32 = 8192;
pub const MAX_RAYS: u32 = 4096;
pub const MAX_TEXELS: u64 = 1 << 27;
// Bound each supersampled G-buffer to roughly 200 MB
const STRIP_TEXELS: usize = 1 << 24;
const BATCH: usize = 1 << 18;
// Curvature batches stay small so progress and cancellation remain responsive
const CURVATURE_BATCH: usize = 1 << 15;

fn validate(request: &BakeRequest) -> Result<(), BakeError> {
    let settings = request.settings;
    let invalid = |message: String| Err(BakeError::Invalid(message));
    let (resolution, resolution_height) = settings.common.output_size();

    let (columns, rows) = request.tiles;
    let width = u64::from(resolution) * u64::from(columns);
    let height = u64::from(resolution_height) * u64::from(rows);
    if columns == 0 || rows == 0 || width.saturating_mul(height) > MAX_TEXELS {
        return invalid(format!("{columns} × {rows} UV tiles at {} exceed {MAX_TEXELS} texels", resolution));
    }

    for (map, ray_settings) in [
        (BakedMap::AmbientOcclusion, &settings.maps.ao.parameters.rays),
        (BakedMap::Thickness, &settings.maps.thickness.parameters.rays),
        (BakedMap::BentNormal, &settings.maps.bent_normal.parameters.rays),
    ] {
        if !settings.maps.is_enabled(map) {
            continue;
        }

        if ray_settings.rays == 0 || ray_settings.rays > MAX_RAYS {
            return invalid(format!("{} ray count {} is not in 1..={MAX_RAYS}", map.id(), ray_settings.rays));
        }

        if !(ray_settings.max_distance.is_finite() && ray_settings.max_distance > 0.0) {
            return invalid(format!("{} max distance {} must be positive", map.id(), ray_settings.max_distance));
        }

        if !(ray_settings.min_distance.is_finite() && ray_settings.min_distance >= 0.0 && ray_settings.min_distance <= ray_settings.max_distance) {
            return invalid(format!("{} minimum distance must be finite and between zero and its maximum distance", map.id()));
        }
        if !ray_settings.spread.is_finite() || !(0.0..=180.0).contains(&ray_settings.spread) {
            return invalid(format!("{} spread angle is not finite", map.id()));
        }
    }

    if !(settings.cage.max_frontal.is_finite() && settings.cage.max_frontal >= 0.0 && settings.cage.max_rear.is_finite() && settings.cage.max_rear >= 0.0) {
        return invalid(format!("frontal/rear distances {}/{} must be finite and ≥ 0", settings.cage.max_frontal, settings.cage.max_rear));
    }
    let curvature = settings.maps.curvature.parameters;
    if settings.maps.curvature.is_enabled {
        if curvature.mesh.rays == 0 || curvature.mesh.rays > MAX_RAYS || !curvature.mesh.sampling_radius.is_finite() || curvature.mesh.sampling_radius <= 0.0 {
            return invalid("curvature requires a positive finite radius and a supported ray count".into());
        }
        if !curvature.mesh.value_min.is_finite()
            || !curvature.mesh.value_max.is_finite()
            || (!curvature.mesh.auto_tonemapping && curvature.mesh.value_min >= curvature.mesh.value_max)
        {
            return invalid("curvature bounds must be finite, with minimum below maximum".into());
        }
    }
    for (name, value) in [
        ("edge distance", settings.skew.edge_distance),
        ("edge contrast", settings.skew.edge_contrast),
        ("curvature details", curvature.normal_map.details),
        ("seam intensity", curvature.normal_map.seams_intensity),
    ] {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return invalid(format!("{name} must be finite and in 0..=1"));
        }
    }
    if settings.maps.height.is_enabled
        && (!settings.maps.height.parameters.scaling_divisor.is_finite() || settings.maps.height.parameters.scaling_divisor <= 0.0)
    {
        return invalid("height scaling divisor must be finite and positive".into());
    }
    if !settings.maps.ao.parameters.ground_plane.offset.is_finite() || settings.maps.ao.parameters.ground_plane.offset < 0.0 {
        return invalid("ground plane offset must be finite and nonnegative".into());
    }
    request.mesh.validate().map_err(|e| BakeError::Invalid(e.to_string()))?;
    if request.material >= request.mesh.materials.len().max(1) {
        return invalid(format!("material {} out of range ({} materials)", request.material, request.mesh.materials.len()));
    }

    if let Some(high) = &request.high {
        high.mesh.validate().map_err(|e| BakeError::Invalid(format!("high poly: {e}")))?;
    }

    if let Some(map) = request.cage_paint {
        map.validate()?;
    }
    if let Some(cage) = request.cage {
        cage.validate().map_err(|e| BakeError::Invalid(format!("cage: {e}")))?;
        if cage.indices.len() != request.mesh.indices.len() {
            return invalid(format!("the cage has {} triangles, the low poly {}", cage.triangle_count(), request.mesh.triangle_count()));
        }
    }

    Ok(())
}

fn alloc<T: Clone>(count: usize, value: T) -> Result<Vec<T>, BakeError> {
    let mut values = Vec::new();
    values.try_reserve_exact(count).map_err(|_| BakeError::Invalid(format!("out of memory allocating {count} texels")))?;
    values.resize(count, value);
    Ok(values)
}

pub fn bake(request: &BakeRequest, progress: &Progress) -> Result<BakeOutput, BakeError> {
    bake_in_strips(request, progress, STRIP_TEXELS, &CpuTracer, &[])
}

/// Retry on the CPU if the supplied tracer fails
pub fn bake_on(request: &BakeRequest, progress: &Progress, tracer: &dyn Tracer) -> Result<BakeOutput, BakeError> {
    bake_tiles(request, progress, &[], Some(tracer))
}

/// Excluded atlas blocks produce no samples; their geometry still participates in ray tracing
pub fn bake_tiles(request: &BakeRequest, progress: &Progress, excluded_tiles: &[(u32, u32)], tracer: Option<&dyn Tracer>) -> Result<BakeOutput, BakeError> {
    let tracer = tracer.unwrap_or(&CpuTracer);
    match bake_in_strips(request, progress, STRIP_TEXELS, tracer, excluded_tiles) {
        Err(BakeError::Tracer(error)) => {
            let mut output = bake_in_strips(request, progress, STRIP_TEXELS, &CpuTracer, excluded_tiles)?;
            output.tracer = format!("CPU ({} unavailable: {error})", tracer.name());
            Ok(output)
        }
        result => result,
    }
}

type OutRow<'a> = Vec<&'a mut [f32]>;

fn bake_in_strips(
    request: &BakeRequest,
    progress: &Progress,
    strip_texels: usize,
    tracer: &dyn Tracer,
    excluded_tiles: &[(u32, u32)],
) -> Result<BakeOutput, BakeError> {
    validate(request)?;
    if excluded_tiles.iter().any(|(column, row)| *column >= request.tiles.0 || *row >= request.tiles.1) {
        return Err(BakeError::Invalid("excluded UV tile lies outside the bake atlas".into()));
    }

    let settings = request.settings;
    let (tile_width, tile_height) = settings.common.output_size();
    let tile_size = (tile_width as usize, tile_height as usize);
    let samples_per_axis = settings.common.antialiasing.samples_per_axis() as usize;
    let (width, height) = (tile_size.0 * request.tiles.0 as usize, tile_size.1 * request.tiles.1 as usize);
    let context = map::BakeContext::new(request)?;
    let tracer = if context.casts_rays() { tracer } else { &CpuTracer };
    let meshes = context.trace_meshes();
    let mut scene = tracer.scene(&meshes)?;
    progress.done.store(0, Relaxed);

    // Normal-map curvature needs a world normal bake even when that output is disabled
    let from_normal = settings.maps.curvature.parameters.method == CurvatureMethod::NormalMap && settings.maps.is_enabled(BakedMap::Curvature);
    let mut maps: Vec<BakedMap> = settings.maps.enabled().collect();
    if from_normal && !maps.contains(&BakedMap::WorldNormal) {
        maps.push(BakedMap::WorldNormal);
    }

    let mut buffers = maps.iter().map(|map| alloc(width * height * map.components(), 0.0f32)).collect::<Result<Vec<_>, _>>()?;
    let mut covered = alloc(width * height, false)?;
    let mut padding_map = None;
    let total_rows = height.max(1) as u64;
    let mut rows_done = 0u64;
    let strip_error = || BakeError::Invalid("strip output of range".into());

    let (sample_width, sample_height) = (width * samples_per_axis, height * samples_per_axis);
    let rows_per_strip = (strip_texels / (sample_width * samples_per_axis)).max(1).min(height);
    let batch = if context.traces_curvature() { CURVATURE_BATCH } else { BATCH };
    let rows_per_batch = (batch / (sample_width * samples_per_axis)).max(1);
    let mut strip_mesh = Mesh { indices: request.mesh.indices.clone(), submeshes: request.mesh.submeshes.clone(), ..Mesh::default() };
    for strip_start in (0..height).step_by(rows_per_strip) {
        let rows = rows_per_strip.min(height - strip_start);
        let (sample_start_y, strip_sample_height) = (strip_start * samples_per_axis, rows * samples_per_axis);
        strip_mesh.uvs =
            request.mesh.uvs.iter().map(|uv| Vec2::new(uv.x, (uv.y * sample_height as f32 - sample_start_y as f32) / strip_sample_height as f32)).collect();
        let mut gbuffer = UvGBuffer::new(&strip_mesh, request.material, sample_width as u32, strip_sample_height as u32);
        for &(column, row) in excluded_tiles {
            let start_x = column as usize * tile_size.0 * samples_per_axis;
            let start_y = row as usize * tile_size.1 * samples_per_axis;
            let end_y = (start_y + tile_size.1 * samples_per_axis).min(sample_start_y + strip_sample_height);
            for sample_y in start_y.max(sample_start_y)..end_y {
                let first = (sample_y - sample_start_y) * sample_width + start_x;
                gbuffer.tri.get_mut(first..first + tile_size.0 * samples_per_axis).ok_or_else(strip_error)?.fill(NO_TRI);
            }
        }

        let coverage_rows = covered.get_mut(strip_start * width..(strip_start + rows) * width).ok_or_else(strip_error)?;
        coverage_rows.par_chunks_mut(width).enumerate().for_each(|(row_index, row)| {
            for (x, covered_pixel) in row.iter_mut().enumerate() {
                *covered_pixel = (0..samples_per_axis * samples_per_axis).any(|sample_index| {
                    gbuffer
                        .tri
                        .get(
                            (row_index * samples_per_axis + sample_index / samples_per_axis) * sample_width
                                + x * samples_per_axis
                                + sample_index % samples_per_axis,
                        )
                        .is_some_and(|triangle_index| *triangle_index != NO_TRI)
                });
            }
        });

        let mut output_rows: Vec<OutRow> = (0..rows).map(|_| Vec::with_capacity(maps.len())).collect();
        for (map, data) in maps.iter().zip(buffers.iter_mut()) {
            let component_count = map.components();
            let strip = data.get_mut(strip_start * width * component_count..(strip_start + rows) * width * component_count).ok_or_else(strip_error)?;
            for (row, chunk) in output_rows.iter_mut().zip(strip.chunks_mut(width * component_count)) {
                row.push(chunk);
            }
        }
        let mut bake_strip = || {
            for (batch_index, batch) in output_rows.chunks_mut(rows_per_batch).enumerate() {
                if progress.cancel.load(Relaxed) {
                    return Err(BakeError::Cancelled);
                }
                // Global sample indices keep noise unchanged across strip and batch sizes
                bake_rows(
                    &context,
                    &maps,
                    request.mesh,
                    &gbuffer,
                    scene.as_mut(),
                    batch,
                    batch_index * rows_per_batch,
                    (width, samples_per_axis, sample_start_y * sample_width),
                )?;
                rows_done += batch.len() as u64;
                progress.done.store((rows_done.saturating_mul(10_000) / total_rows).min(10_000) as u32, Relaxed);
            }
            Ok(())
        };
        if strip_start + rows < height || (settings.common.dilation == 0 && !settings.common.apply_diffusion) {
            bake_strip()?;
        } else {
            let padding =
                || DilationMap::in_rectangular_blocks(&covered, width as u32, height as u32, Some(settings.common.dilation), (tile_width, tile_height));
            let (padding, baked) = rayon::join(padding, bake_strip);
            baked?;
            padding_map = Some(padding);
        }
    }

    let mut output: BTreeMap<BakedMap, Vec<f32>> = maps.into_iter().zip(buffers).collect();
    if from_normal {
        let world_normal =
            if settings.maps.is_enabled(BakedMap::WorldNormal) { output.get(&BakedMap::WorldNormal).cloned() } else { output.remove(&BakedMap::WorldNormal) };
        if let (Some(world_normal), Some(curvature_pixels)) = (world_normal, output.get_mut(&BakedMap::Curvature)) {
            let parameters = settings.maps.curvature.parameters.normal_map;
            let grid = map::TexelGrid { covered: &covered, size: (width, height), tile_size };
            map::curvature_from_normal(&world_normal, request.mesh, request.material, &grid, parameters, curvature_pixels);
            if parameters.enable_seams {
                map::fix_seams(request.mesh, request.material, &covered, (width, height), parameters.seams_intensity, curvature_pixels);
            }
        }
    }

    let (bounds_min, bounds_max) = request.mesh.bounds();
    map::normalize(&mut output, &covered, width, tile_size, settings, (bounds_max - bounds_min).max_element().max(1e-6));
    if let Some(padding_map) = padding_map {
        for (map, data) in output.iter_mut() {
            padding_map.apply(data, map.components());
            if settings.common.apply_diffusion {
                map::diffuse(data, map.components(), &padding_map, tile_size);
            }
        }
    }
    if progress.cancel.load(Relaxed) {
        return Err(BakeError::Cancelled);
    }
    progress.done.store(10_000, Relaxed);
    Ok(BakeOutput { resolution: width as u32, height: height as u32, maps: output, tracer: tracer.name() })
}

#[allow(clippy::too_many_arguments)]
fn bake_rows(
    context: &map::BakeContext,
    maps: &[BakedMap],
    mesh: &Mesh,
    gbuffer: &UvGBuffer,
    scene: &mut dyn RayScene,
    rows: &mut [OutRow],
    first_row: usize,
    (width, samples_per_axis, first_sample_seed): (usize, usize, usize),
) -> Result<(), BakeError> {
    let sample_width = width * samples_per_axis;
    let samples_by_row: Vec<Vec<map::Sample>> = (first_row..first_row + rows.len())
        .into_par_iter()
        .map(|row| {
            let subsamples = (0..width).flat_map(|x| {
                (0..samples_per_axis * samples_per_axis).map(move |sample_index| {
                    (x, (row * samples_per_axis + sample_index / samples_per_axis) * sample_width + x * samples_per_axis + sample_index % samples_per_axis)
                })
            });
            subsamples
                .filter_map(|(x, sample_index)| {
                    Some(map::Sample {
                        x,
                        surface: gbuffer.surface(mesh, sample_index)?,
                        bary: *gbuffer.bary.get(sample_index)?,
                        seed: (first_sample_seed + sample_index) as u64,
                    })
                })
                .collect()
        })
        .collect();
    let row_offsets: Vec<usize> = samples_by_row
        .iter()
        .scan(0, |sample_count, row| {
            *sample_count += row.len();
            Some(*sample_count - row.len())
        })
        .collect();
    let samples = || samples_by_row.par_iter().flat_map_iter(|row| row.iter());

    let transfers = match context.has_high() {
        true => {
            let (rays, frontal_distances): (Vec<tracing::Ray>, Vec<f32>) = samples().map(|sample| context.transfer_ray(&sample.surface, sample.bary)).unzip();
            let hits = scene.closest(1, &rays, context.ignore_transfer_backface())?;
            hits.into_par_iter().zip(frontal_distances).map(|(hit, front)| context.transfer(hit, front)).collect()
        }
        false => Vec::new(),
    };
    let transfer = |sample_index: usize| transfers.get(sample_index).and_then(Option::as_ref);
    let curvature = match context.traces_curvature() {
        true => {
            let items: Vec<(&Surface, Option<&map::Transfer>)> = samples_by_row
                .iter()
                .zip(&row_offsets)
                .flat_map(|(row, &first)| row.iter().zip(first..))
                .map(|(sample, sample_index)| (&sample.surface, transfer(sample_index)))
                .collect();
            context.trace_curvature(scene, &items)?
        }
        false => Vec::new(),
    };
    let queries: Vec<tracing::Query> = match context.ao.is_some() || context.bent.is_some() || context.thickness.is_some() {
        true => samples_by_row
            .par_iter()
            .zip(&row_offsets)
            .flat_map_iter(|(row, &first)| row.iter().zip(first..))
            .map(|(sample, sample_index)| context.query(transfer(sample_index), &sample.surface, sample.seed))
            .collect(),
        false => Vec::new(),
    };
    // Share occlusion results when the bent-normal ray settings match
    let separate_bent_pass = context.bent.filter(|_| context.bent != context.ao);
    let passes: Vec<tracing::Hemisphere> = [context.ao, separate_bent_pass, context.thickness].into_iter().flatten().collect();
    let mut results = scene.hemispheres(&passes, &queries)?.into_iter();
    let mut next = |enabled: bool| if enabled { results.next().unwrap_or_default() } else { Vec::new() };
    let (occlusion_results, separate_bent_results, thickness) =
        (next(context.ao.is_some()), next(separate_bent_pass.is_some()), next(context.thickness.is_some()));
    let bent_results = if separate_bent_pass.is_some() { &separate_bent_results } else { &occlusion_results };

    rows.par_iter_mut().zip(&samples_by_row).zip(&row_offsets).for_each(|((row, samples), &first)| {
        let mut sample_values = [[0.0f32; 3]; BakedMap::ALL.len()];
        let mut row_sample_index = 0;
        for x in 0..width {
            let mut accumulated = [[0.0f32; 3]; BakedMap::ALL.len()];
            let mut sample_count = 0u32;
            while let Some(sample) = samples.get(row_sample_index).filter(|sample| sample.x == x) {
                let sample_index = first + row_sample_index;
                let traced = map::Traced {
                    ao: occlusion_results.get(sample_index).map_or(1.0, |value| value[0]),
                    bent: bent_results.get(sample_index).map(|value| Vec3::new(value[1], value[2], value[3])),
                    thickness: thickness.get(sample_index).map_or(1.0, |value| value[0]),
                    curvature: curvature.get(sample_index).copied().unwrap_or(0.5),
                };
                context.evaluate(maps, &sample.surface, sample.bary, transfer(sample_index), &traced, &mut sample_values);
                for (sum, value) in accumulated.iter_mut().zip(&sample_values) {
                    sum.iter_mut().zip(value).for_each(|(sum, value)| *sum += value);
                }
                sample_count += 1;
                row_sample_index += 1;
            }

            for ((map, destination), sum) in maps.iter().zip(row.iter_mut()).zip(&accumulated) {
                let component_count = map.components();
                let mean = sum.map(|sum| sum / sample_count.max(1) as f32);
                let value = if sample_count == 0 || mean.iter().any(|value| !value.is_finite()) { map.neutral() } else { mean };
                if let Some(pixel) = destination.get_mut(x * component_count..(x + 1) * component_count) {
                    pixel.iter_mut().zip(value).for_each(|(component, value)| *component = value);
                }
            }
        }
    });
    Ok(())
}

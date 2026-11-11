use super::curvature::Curvature;
use super::sampling::{pack, splitmix64, tangent_normal};
use super::{position_projection::PositionProjection, source::Source, traced::Traced, transfer::Transfer};
use crate::cage::{RayFrame, RayStart};
use crate::tracing::{ANY, Hemisphere, Query, Ray, RayKind, RayScene, TraceMesh};
use crate::{BakeError, BakeRequest, MeshExtent};
use rayon::prelude::*;
use texturecraft_core::{
    AmbientOcclusionAttenuation, BackfaceMode, BakeSettings, BakedMap, CurvatureMethod, MeshMatchMode, RayDistribution, RaySettings, SelfOcclusion,
};
use texturecraft_geom::{Hit, Surface, surface_at};

const CURVATURE_CHUNK: usize = 1 << 15;

pub(crate) struct BakeContext<'a> {
    low: Source<'a>,
    high: Option<Source<'a>>,
    ray_frame: RayFrame<'a>,
    settings: &'a BakeSettings,
    frontal: f32,
    rear: f32,
    position: PositionProjection,
    curvature_method: CurvatureMethod,
    curvature: Curvature,
    pub ao: Option<Hemisphere>,
    pub bent: Option<Hemisphere>,
    pub thickness: Option<Hemisphere>,
}

impl<'a> BakeContext<'a> {
    pub(crate) fn new(request: &BakeRequest<'a>) -> Result<BakeContext<'a>, BakeError> {
        let settings = request.settings;
        let extent = MeshExtent::of(request.mesh)?;
        let (low, high) = Source::of_request(request);
        let ray_frame = RayFrame::new(request, extent)?;
        let (mesh_scale, ray_epsilon) = (extent.scale, extent.ray_epsilon());
        let pass = |map: BakedMap, ray_settings: &RaySettings, kind: RayKind| {
            let half_angle_radians = (ray_settings.spread.clamp(0.0, 180.0) * 0.5).to_radians();
            let units = if ray_settings.relative_to_bounding_box { mesh_scale } else { 1.0 };
            settings.maps.is_enabled(map).then_some(Hemisphere {
                kind,
                rays: ray_settings.rays,
                dmax: ray_settings.max_distance * units,
                min_distance: ray_settings.min_distance * units,
                attenuation: AmbientOcclusionAttenuation::None,
                ground_height: None,
                eps: ray_epsilon,
                cos_max: half_angle_radians.cos(),
                sin_max: half_angle_radians.sin(),
                uniform: ray_settings.distribution == RayDistribution::Uniform,
                ignore_backface: BackfaceMode::Never,
                same_key: false,
            })
        };

        let mut ao = pass(BakedMap::AmbientOcclusion, &settings.maps.ao.parameters.rays, RayKind::Occlusion);
        if let Some(pass) = &mut ao {
            let parameters = settings.maps.ao.parameters;
            pass.ignore_backface = parameters.ignore_backface;
            pass.same_key = parameters.self_occlusion == SelfOcclusion::SameMeshName;
            pass.attenuation = parameters.attenuation;
            pass.ground_height = parameters.ground_plane.enabled.then_some(extent.bounds_min.y - parameters.ground_plane.offset);
        }

        let mut bent = pass(BakedMap::BentNormal, &settings.maps.bent_normal.parameters.rays, RayKind::Occlusion);
        if let Some(pass) = &mut bent {
            pass.ignore_backface = settings.maps.bent_normal.parameters.ignore_backface;
            pass.same_key = settings.maps.bent_normal.parameters.self_occlusion == SelfOcclusion::SameMeshName;
        }

        let mut thickness = pass(BakedMap::Thickness, &settings.maps.thickness.parameters.rays, RayKind::Thickness);
        if let Some(pass) = &mut thickness {
            pass.same_key = settings.maps.thickness.parameters.self_occlusion == SelfOcclusion::SameMeshName;
        }

        let cage_units = ray_frame.cage_units;
        Ok(BakeContext {
            low,
            high,
            ray_frame,
            settings,
            frontal: settings.cage.max_frontal * cage_units,
            rear: settings.cage.max_rear * cage_units,
            position: PositionProjection::new(request.mesh, request.material, settings.maps.position.parameters),
            curvature_method: settings.maps.curvature.parameters.method,
            curvature: Curvature::new(settings, request.mesh, mesh_scale, request.tiles),
            ao,
            bent,
            thickness,
        })
    }

    pub(crate) fn ignore_transfer_backface(&self) -> bool {
        self.settings.high_poly.ignore_backface
    }

    pub(crate) fn has_high(&self) -> bool {
        self.high.is_some()
    }

    pub(crate) fn casts_rays(&self) -> bool {
        self.has_high() || self.curvature.is_enabled || self.ao.is_some() || self.bent.is_some() || self.thickness.is_some()
    }

    pub(crate) fn trace_meshes(&self) -> Vec<TraceMesh<'_>> {
        std::iter::once(&self.low).chain(&self.high).map(Source::trace).collect()
    }

    pub(crate) fn transfer_ray(&self, surface: &Surface, barycentric: [f32; 2]) -> (Ray, f32) {
        let start = self.ray_frame.start(surface, barycentric);
        let origin = start.origin(surface.pos, self.frontal);
        let (direction, front) = match start {
            RayStart::CageMesh(offset) => (offset.normalize(), offset.length()),
            RayStart::Distance { up, scale } => (-up, self.frontal * scale),
        };
        let key = match self.settings.high_poly.match_mode {
            MeshMatchMode::Always => ANY,
            MeshMatchMode::ByMeshName => self.low.key.get(surface.tri as usize).copied().unwrap_or(ANY),
        };
        (Ray { origin, direction, max_distance: front + self.rear, key }, front)
    }

    pub(crate) fn transfer(&self, hit: Option<Hit>, front: f32) -> Option<Transfer> {
        let (high, hit) = (self.high.as_ref()?, hit?);
        let surface = surface_at(high.mesh, hit.tri, hit.u, hit.v)?;
        Some(Transfer { surface, bary: [hit.u, hit.v], t: hit.t, front })
    }

    pub(crate) fn query(&self, transfer: Option<&Transfer>, low_surface: &Surface, seed: u64) -> Query {
        let (source, surface, mesh) = match (&self.high, transfer) {
            (Some(high), Some(transfer_hit)) => (high, &transfer_hit.surface, 1),
            _ => (&self.low, low_surface, 0),
        };
        let seed_hash = splitmix64(seed);
        let rot = [(seed_hash >> 40) as f32 / (1u64 << 24) as f32, ((seed_hash >> 16) & 0xff_ffff) as f32 / (1u64 << 24) as f32];
        let geo = if surface.geo_nrm.dot(surface.nrm) < 0.0 { -surface.geo_nrm } else { surface.geo_nrm };
        Query { pos: surface.pos, nrm: surface.nrm, geo, rot, key: source.key.get(surface.tri as usize).copied().unwrap_or(ANY), mesh }
    }

    pub(crate) fn traces_curvature(&self) -> bool {
        self.curvature.is_enabled
    }

    pub(crate) fn trace_curvature(&self, scene: &mut dyn RayScene, samples: &[(&Surface, Option<&Transfer>)]) -> Result<Vec<f32>, BakeError> {
        let mut output = vec![0.5; samples.len()];
        let curvature = &self.curvature;
        for mesh in [0usize, 1] {
            let Some(source) = (if mesh == 0 { Some(&self.low) } else { self.high.as_ref() }) else { continue };
            let tuning = if mesh == 0 { &curvature.low } else { &curvature.high };
            let chosen: Vec<(usize, &Surface, f32)> = samples
                .par_iter()
                .enumerate()
                .filter_map(|(sample_index, (low_surface, transfer_hit))| {
                    let surface = match (mesh, transfer_hit) {
                        (1, Some(hit)) => &hit.surface,
                        (0, None) => *low_surface,
                        _ => return None,
                    };
                    Some((sample_index, surface, curvature.texel(low_surface.tri)))
                })
                .collect();
            let rays_per_sample = curvature.rays_per_sample.max(1);
            let key_for = |surface: &Surface| {
                if self.settings.maps.curvature.parameters.mesh.self_intersection == SelfOcclusion::SameMeshName {
                    source.key.get(surface.tri as usize).copied().unwrap_or(ANY)
                } else {
                    ANY
                }
            };
            let chunk_size = CURVATURE_CHUNK.min((1_048_576 / rays_per_sample).max(1));
            for chunk in chosen.chunks(chunk_size) {
                let mut rays: Vec<Ray> = bytemuck::zeroed_vec(chunk.len() * rays_per_sample);
                rays.par_chunks_mut(rays_per_sample)
                    .zip(chunk)
                    .for_each(|(output, (_, surface, texel))| curvature.rays(tuning, surface, *texel, key_for(surface), output));
                let hits = scene.closest(mesh, &rays, false)?;
                let walls =
                    |chunk_index: usize| hits.get(chunk_index * rays_per_sample..chunk_index * rays_per_sample + curvature.wall_count).unwrap_or_default();
                let grounds: Vec<Ray> = chunk
                    .par_iter()
                    .enumerate()
                    .flat_map_iter(|(chunk_index, (_, surface, texel))| {
                        let mut value = Vec::new();
                        curvature.ground_rays(surface, *texel, key_for(surface), walls(chunk_index), &mut value);
                        value
                    })
                    .collect();
                let ground_hits = scene.closest(mesh, &grounds, false)?;
                let ground_offsets: Vec<usize> = (0..chunk.len())
                    .scan(0, |first, chunk_index| {
                        let offset = *first;
                        *first += walls(chunk_index).iter().flatten().count();
                        Some(offset)
                    })
                    .collect();
                let values: Vec<(usize, f32)> = chunk
                    .par_iter()
                    .zip(&ground_offsets)
                    .enumerate()
                    .map(|(chunk_index, ((sample_index, surface, texel), first))| {
                        let ground_count = walls(chunk_index).iter().flatten().count();
                        let sample_ground_hits = ground_hits.get(*first..*first + ground_count).unwrap_or_default();
                        let sample_hits = hits.get(chunk_index * rays_per_sample..(chunk_index + 1) * rays_per_sample).unwrap_or_default();
                        (*sample_index, curvature.value(tuning, source, surface, *texel, sample_hits, sample_ground_hits))
                    })
                    .collect();
                for (sample_index, value) in values {
                    if let Some(destination) = output.get_mut(sample_index) {
                        *destination = value;
                    }
                }
            }
        }
        Ok(output)
    }

    pub(crate) fn evaluate(
        &self,
        maps: &[BakedMap],
        low_surface: &Surface,
        barycentric: [f32; 2],
        transfer: Option<&Transfer>,
        traced: &Traced,
        outputs: &mut [[f32; 3]],
    ) {
        let (source, surface, source_barycentric) = match (&self.high, transfer) {
            (Some(high), Some(hit)) => (high, &hit.surface, hit.bary),
            _ => (&self.low, low_surface, barycentric),
        };

        let missed = self.high.is_some() && transfer.is_none();
        for (&map, output) in maps.iter().zip(outputs.iter_mut()) {
            *output = match map {
                BakedMap::AmbientOcclusion => [traced.ao; 3],
                BakedMap::BentNormal => pack(traced.bent.unwrap_or(surface.nrm)),
                BakedMap::Thickness => [traced.thickness; 3],
                BakedMap::Curvature if self.curvature_method == CurvatureMethod::NormalMap => [0.5; 3],
                BakedMap::Curvature => [traced.curvature; 3],
                BakedMap::WorldNormal => pack(surface.nrm),
                BakedMap::Position => self.position.evaluate(surface.pos),
                BakedMap::Id => source.id_color(surface, source_barycentric, self.settings.maps.id.parameters),
                BakedMap::Normal => transfer.map_or(map.neutral(), |hit| tangent_normal(low_surface, hit.surface.nrm)),
                BakedMap::Height => {
                    let relative_to_rays = self.settings.maps.height.parameters.normalization == texturecraft_core::HeightNormalization::RelativeToRayDistances;
                    [transfer.map_or(if relative_to_rays { 0.5 } else { 0.0 }, |hit| {
                        let height = hit.front - hit.t;
                        if relative_to_rays { (0.5 + height / (hit.front + self.rear).max(1e-20)).clamp(0.0, 1.0) } else { height }
                    }); 3]
                }
                BakedMap::Opacity if missed => [0.0; 3],
                BakedMap::Opacity => [source.color(surface, source_barycentric).w; 3],
            };
        }
    }
}

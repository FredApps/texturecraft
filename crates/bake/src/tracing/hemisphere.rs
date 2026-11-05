use super::{ANY, Query, RayKind, TraceMesh};
use std::f32::consts::TAU;
use texturecraft_core::{AmbientOcclusionAttenuation, BackfaceMode};
use texturecraft_geom::glam::Vec3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hemisphere {
    pub kind: RayKind,
    pub rays: u32,
    /// Maximum ray distance in mesh units
    pub dmax: f32,
    pub min_distance: f32,
    pub attenuation: AmbientOcclusionAttenuation,
    pub ground_height: Option<f32>,
    /// Ray origin offset along the geometric normal
    pub eps: f32,
    pub cos_max: f32,
    pub sin_max: f32,
    pub uniform: bool,
    pub ignore_backface: BackfaceMode,
    pub same_key: bool,
}

impl Hemisphere {
    pub fn direction(&self, (tangent, bitangent, axis): (Vec3, Vec3, Vec3), ray_index: u32, rotation: [f32; 2]) -> Vec3 {
        let azimuth_sample = (ray_index as f32 / self.rays as f32 + rotation[0]).fract();
        let elevation_sample = (ray_index.reverse_bits() as f32 / 4_294_967_296.0 + rotation[1]).fract();
        let (radius, height) = if self.uniform {
            let height = 1.0 - elevation_sample * (1.0 - self.cos_max);
            ((1.0 - height * height).max(0.0).sqrt(), height)
        } else {
            let radius = self.sin_max * elevation_sample.sqrt();
            (radius, (1.0 - radius * radius).max(0.0).sqrt())
        };
        let azimuth = TAU * azimuth_sample;
        tangent * (radius * azimuth.cos()) + bitangent * (radius * azimuth.sin()) + axis * height
    }

    fn frame(&self, query: &Query) -> (Vec3, Vec3, Vec3) {
        let (axis, geometric_normal) = match self.kind {
            RayKind::Occlusion => (query.nrm, query.geo),
            RayKind::Thickness => (-query.nrm, -query.geo),
        };
        (axis, geometric_normal, query.pos + geometric_normal * self.eps)
    }

    pub fn evaluate(&self, mesh: &TraceMesh, query: &Query) -> [f32; 4] {
        let (axis, geometric_normal, origin) = self.frame(query);
        let (tangent, bitangent) = axis.any_orthonormal_pair();
        let key = if self.same_key { query.key } else { ANY };
        let filtered = key != ANY || self.ignore_backface != BackfaceMode::Never;
        let (mut accumulated, mut bent_normal, mut accepted_ray_count) = (0.0f32, Vec3::ZERO, 0u32);
        for ray_index in 0..self.rays {
            let direction = self.direction((tangent, bitangent, axis), ray_index, query.rot);
            if direction.dot(geometric_normal) <= 0.0 {
                continue;
            }
            let ray_origin = origin + direction * self.min_distance;
            let ray_length = (self.dmax - self.min_distance).max(0.0);
            let hit = if filtered {
                mesh.bvh.closest_filtered(ray_origin, direction, ray_length, &|triangle| {
                    let ignore_backface = match self.ignore_backface {
                        BackfaceMode::Never => false,
                        BackfaceMode::Always => true,
                        BackfaceMode::ByMeshName => mesh.backface_culling.get(triangle as usize).copied().unwrap_or(false),
                    };
                    mesh.key_matches(triangle, key) && !(ignore_backface && mesh.back_facing(triangle, direction))
                })
            } else {
                mesh.bvh.closest(ray_origin, direction, ray_length)
            };
            let mut distance = hit.map(|hit| hit.t + self.min_distance);
            if let Some(height) = self.ground_height.filter(|_| direction.y < -1e-8) {
                let ground_distance = (height - origin.y) / direction.y;
                if (self.min_distance..=self.dmax).contains(&ground_distance) {
                    distance = Some(distance.map_or(ground_distance, |distance| distance.min(ground_distance)));
                }
            }
            match self.kind {
                RayKind::Occlusion => {
                    let occlusion = distance.map_or(0.0, |distance| self.occlusion(distance));
                    accumulated += occlusion;
                    bent_normal += direction * (1.0 - occlusion);
                }
                RayKind::Thickness => accumulated += distance.unwrap_or(self.dmax),
            }
            accepted_ray_count += 1;
        }
        match self.kind {
            RayKind::Occlusion => {
                let ambient_occlusion = if accepted_ray_count == 0 { 1.0 } else { (1.0 - accumulated / accepted_ray_count as f32).clamp(0.0, 1.0) };
                let bent_normal = bent_normal.try_normalize().unwrap_or(query.nrm);
                [ambient_occlusion, bent_normal.x, bent_normal.y, bent_normal.z]
            }
            RayKind::Thickness => [if accepted_ray_count == 0 { self.dmax } else { accumulated / accepted_ray_count as f32 }, 0.0, 0.0, 0.0],
        }
    }

    fn occlusion(&self, distance: f32) -> f32 {
        let fraction = ((distance - self.min_distance) / (self.dmax - self.min_distance).max(f32::EPSILON)).clamp(0.0, 1.0);
        match self.attenuation {
            AmbientOcclusionAttenuation::None => 1.0,
            AmbientOcclusionAttenuation::Linear => 1.0 - fraction,
            AmbientOcclusionAttenuation::Smooth => 1.0 - fraction * fraction * (3.0 - 2.0 * fraction),
        }
    }
}

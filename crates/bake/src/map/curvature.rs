use super::{curvature_tuning::CurvatureTuning, sampling::interpolate, source::Source};
use crate::tracing::Ray;
use rayon::prelude::*;
use texturecraft_core::{BakeSettings, BakedMap, CurvatureMethod};
use texturecraft_geom::glam::Vec3;
use texturecraft_geom::{Hit, Mesh, Surface};

const RING_COUNT: usize = 4;
const WALL_RADIUS_TEXELS: f32 = 1.5;
const CONCAVE_WEIGHT: f32 = 1.5;
const LOW_POLY_CONVEX_WEIGHT: f32 = 1.5;
const HIGH_POLY_CONVEX_WEIGHT: f32 = 0.5;
const LOW_POLY_CURVATURE_RADIUS_RATIO: f32 = 0.003;
const LOW_POLY_CURVATURE_WEIGHT: f32 = 1.3;
const HIGH_POLY_CURVATURE_RADIUS_RATIO: f32 = 0.0007;
const HIGH_POLY_CURVATURE_WEIGHT: f32 = 1.6;
const HARD_EDGE_NORMAL_CHANGE_THRESHOLD: f32 = 0.05;
const GENTLE_CURVATURE_THRESHOLD: f32 = 0.1;

pub(super) struct Curvature {
    pub(super) is_enabled: bool,
    walls: f32,
    spread: f32,
    fallback: f32,
    texels: Vec<f32>,
    ring: [Vec<(f32, f32)>; RING_COUNT],
    pub(super) rays_per_sample: usize,
    pub(super) wall_count: usize,
    pub(super) low: CurvatureTuning,
    pub(super) high: CurvatureTuning,
}

impl Curvature {
    pub(super) fn new(settings: &BakeSettings, low: &Mesh, mesh_scale: f32, tiles: (u32, u32)) -> Curvature {
        let is_enabled = settings.maps.is_enabled(BakedMap::Curvature) && settings.maps.curvature.parameters.method == CurvatureMethod::Mesh;
        let (width, height) = settings.common.output_size();
        let resolution = (width as f32 * height as f32 * tiles.0.max(1) as f32 * tiles.1.max(1) as f32).sqrt();
        let texel = |triangle: usize| {
            let [a, b, c] = low.tri(triangle)?;
            let (position_a, position_b, position_c) = (*low.positions.get(a)?, *low.positions.get(b)?, *low.positions.get(c)?);
            let (uv_a, uv_b, uv_c) = (*low.uvs.get(a)?, *low.uvs.get(b)?, *low.uvs.get(c)?);
            let world_area = (position_b - position_a).cross(position_c - position_a).length();
            let uv_area = (uv_b - uv_a).perp_dot(uv_c - uv_a).abs();
            (uv_area > 1e-12 && world_area > 0.0).then(|| (world_area / uv_area).sqrt() / resolution)
        };
        let fallback = mesh_scale * 1e-3;
        let triangle_count = if is_enabled { low.triangle_count() } else { 0 };
        let parameters = settings.maps.curvature.parameters.mesh;
        let radius = parameters.sampling_radius * if parameters.relative_to_bounding_box { mesh_scale } else { 1.0 };
        let radius_scale = radius / (mesh_scale * 0.001);
        let walls = WALL_RADIUS_TEXELS * radius_scale;
        let rays_per_sample = parameters.rays as usize;
        let ring: [Vec<(f32, f32)>; RING_COUNT] = std::array::from_fn(|ring_index| {
            let count = rays_per_sample / RING_COUNT + usize::from(ring_index < rays_per_sample % RING_COUNT);
            (0..count)
                .map(|index| {
                    let angle = std::f32::consts::TAU * index as f32 / count.max(1) as f32;
                    (angle.cos(), angle.sin())
                })
                .collect()
        });
        Curvature {
            is_enabled,
            rays_per_sample,
            wall_count: ring.first().map_or(0, Vec::len),
            walls,
            spread: walls * 4.0 / 3.0,
            fallback,
            texels: (0..triangle_count).into_par_iter().map(|t| texel(t).filter(|v| v.is_finite()).unwrap_or(fallback)).collect(),
            low: CurvatureTuning {
                convex: LOW_POLY_CONVEX_WEIGHT,
                true_radius: mesh_scale * LOW_POLY_CURVATURE_RADIUS_RATIO * radius_scale,
                true_strength: LOW_POLY_CURVATURE_WEIGHT,
            },
            high: CurvatureTuning {
                convex: HIGH_POLY_CONVEX_WEIGHT,
                true_radius: mesh_scale * HIGH_POLY_CURVATURE_RADIUS_RATIO * radius_scale,
                true_strength: HIGH_POLY_CURVATURE_WEIGHT,
            },
            ring,
        }
    }

    pub(super) fn texel(&self, tri: u32) -> f32 {
        self.texels.get(tri as usize).copied().unwrap_or(self.fallback)
    }

    fn radii(&self, tuning: &CurvatureTuning, texel: f32) -> [f32; RING_COUNT] {
        let r = self.walls * texel;
        [r, r, self.spread * texel, tuning.true_radius]
    }

    fn directions(&self, ring: usize, tangent: Vec3, bitangent: Vec3) -> impl Iterator<Item = Vec3> + '_ {
        self.ring.get(ring).into_iter().flatten().map(move |(cosine, sine)| tangent * *cosine + bitangent * *sine)
    }

    pub(super) fn rays(&self, tuning: &CurvatureTuning, surface: &Surface, texel_size: f32, key: u32, output: &mut [Ray]) {
        let (geometric_normal, tangent, bitangent) = surface_frame(surface);
        let mut output = output.iter_mut();
        for (ring, radius) in self.radii(tuning, texel_size).into_iter().enumerate() {
            for direction in self.directions(ring, tangent, bitangent) {
                let Some(ray) = output.next() else {
                    return;
                };
                *ray = match ring {
                    0 => Ray { origin: surface.pos + geometric_normal * (0.15 * radius), direction, max_distance: radius, key },
                    1 => Ray { origin: surface.pos - geometric_normal * (0.15 * radius), direction, max_distance: radius, key },
                    _ => Ray {
                        origin: surface.pos + direction * radius + geometric_normal * (0.25 * radius),
                        direction: -geometric_normal,
                        max_distance: 2.25 * radius,
                        key,
                    },
                };
            }
        }
    }

    pub(super) fn ground_rays(&self, surface: &Surface, texel_size: f32, key: u32, walls: &[Option<Hit>], output: &mut Vec<Ray>) {
        let (geometric_normal, tangent, bitangent) = surface_frame(surface);
        let radius = self.walls * texel_size;
        for (w, direction) in walls.iter().zip(self.directions(0, tangent, bitangent)) {
            if let Some(hit) = w {
                output.push(Ray {
                    origin: surface.pos + geometric_normal * (0.15 * radius) + direction * (hit.t * 0.85),
                    direction: -geometric_normal,
                    max_distance: 0.5 * radius,
                    key,
                });
            }
        }
    }

    fn wall(&self, source: &Source, surface: &Surface, geometric_normal: Vec3, radius: f32, hits: &[Option<Hit>], keep: &mut dyn FnMut() -> bool) -> f32 {
        let mut sum = 0.0;
        for hit in hits.iter().flatten() {
            if !keep() {
                continue;
            }
            let upright = triangle_normal(source.mesh, hit.tri).map_or(1.0, |normal| (1.0 - normal.dot(geometric_normal).powi(2)).max(0.0).sqrt());
            let shaded = interpolate(source.mesh, &source.mesh.normals, hit.tri, [hit.u, hit.v]).and_then(Vec3::try_normalize).unwrap_or(surface.nrm);
            let hard = ((1.0 - shaded.dot(surface.nrm)) / HARD_EDGE_NORMAL_CHANGE_THRESHOLD).clamp(0.0, 1.0);
            sum += (1.0 - hit.t / radius) * upright * hard;
        }
        sum / hits.len().max(1) as f32
    }

    fn spread(
        &self,
        source: &Source,
        surface: &Surface,
        geometric_normal: Vec3,
        directions: impl Iterator<Item = Vec3>,
        hits: &[Option<Hit>],
        turn: f32,
    ) -> (f32, f32) {
        let (mut shading, mut faces) = (0.0, 0.0);
        for (hit, direction) in hits.iter().zip(directions) {
            let Some(hit) = hit else { continue };
            let normal = interpolate(source.mesh, &source.mesh.normals, hit.tri, [hit.u, hit.v]).and_then(Vec3::try_normalize).unwrap_or(surface.nrm);
            if 1.0 - normal.dot(surface.nrm) <= turn {
                shading += (normal - surface.nrm).dot(direction);
                if let Some(f) = triangle_normal(source.mesh, hit.tri) {
                    let f = if f.dot(geometric_normal) < 0.0 { -f } else { f };
                    faces += (f - geometric_normal).dot(direction);
                }
            }
        }
        (shading / hits.len().max(1) as f32, faces / hits.len().max(1) as f32)
    }

    pub(super) fn value(
        &self,
        tuning: &CurvatureTuning,
        source: &Source,
        surface: &Surface,
        texel_size: f32,
        hits: &[Option<Hit>],
        ground_hits: &[Option<Hit>],
    ) -> f32 {
        let (geometric_normal, tangent, bitangent) = surface_frame(surface);
        let ring = |ring_index: usize| {
            let start: usize = self.ring.iter().take(ring_index).map(Vec::len).sum();
            let count = self.ring.get(ring_index).map_or(0, Vec::len);
            hits.get(start..start + count).unwrap_or_default()
        };
        let radius = self.walls * texel_size;
        let mut next = ground_hits.iter();
        let above = self.wall(source, surface, geometric_normal, radius, ring(0), &mut || next.next().copied().flatten().is_some_and(|g| g.t < 0.3 * radius));
        let below = self.wall(source, surface, geometric_normal, radius, ring(1), &mut || true);
        let (shading, _) = self.spread(source, surface, geometric_normal, self.directions(2, tangent, bitangent), ring(2), 2.0);
        let (true_shading, true_faces) = self.spread(source, surface, geometric_normal, self.directions(3, tangent, bitangent), ring(3), 2.0);
        let value =
            -above * CONCAVE_WEIGHT + (below - above).max(0.0) * tuning.convex + shading * 0.5 + tight(agreed(true_shading, true_faces) * tuning.true_strength);
        (value * 2.0).clamp(-1.0, 1.0)
    }
}

fn triangle_normal(mesh: &Mesh, tri: u32) -> Option<Vec3> {
    let [a, b, c] = mesh.tri(tri as usize)?;
    let p = |i: usize| mesh.positions.get(i).copied();
    (p(b)? - p(a)?).cross(p(c)? - p(a)?).try_normalize()
}

fn surface_frame(surface: &Surface) -> (Vec3, Vec3, Vec3) {
    let geometric_normal = if surface.geo_nrm.dot(surface.nrm) < 0.0 { -surface.geo_nrm } else { surface.geo_nrm };
    let (tangent, bitangent) = geometric_normal.any_orthonormal_pair();
    (geometric_normal, tangent, bitangent)
}

fn tight(v: f32) -> f32 {
    v.signum() * (v.abs() - GENTLE_CURVATURE_THRESHOLD).max(0.0)
}

fn agreed(shading: f32, faces: f32) -> f32 {
    if shading * faces <= 0.0 { 0.0 } else { shading.abs().min(faces.abs()).copysign(shading) }
}

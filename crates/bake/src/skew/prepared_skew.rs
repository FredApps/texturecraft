use crate::BakeError;
use crate::BakePaintMap;
use std::collections::HashMap;
use texturecraft_core::{SkewCorrectionMethod, SkewSettings};
use texturecraft_geom::glam::Vec3;
use texturecraft_geom::{Mesh, Surface};

const HARD_EDGE_COS: f32 = 0.866;

pub(crate) struct PreparedSkew<'a> {
    map: &'a BakePaintMap,
    settings: SkewSettings,
    hard: Vec<[bool; 3]>,
    mesh_scale: f32,
}

impl<'a> PreparedSkew<'a> {
    pub(crate) fn new(map: &'a BakePaintMap, mesh: &Mesh, mesh_scale: f32, settings: SkewSettings) -> Result<PreparedSkew<'a>, BakeError> {
        map.validate()?;
        let hard = if settings.edge_protection && settings.edge_distance > 0.0 { hard_edges(mesh) } else { Vec::new() };
        Ok(PreparedSkew { map, settings, hard, mesh_scale })
    }

    pub(crate) fn correct(&self, mesh: &Mesh, surface: &Surface, up: Vec3) -> Vec3 {
        let amount = self.map.sample(surface.uv);
        if amount <= 0.0 {
            return up;
        }
        let protection = match (self.hard.get(surface.tri as usize), mesh.tri(surface.tri as usize)) {
            (Some(edges), Some(corners)) => {
                let nearest_edge_distance = (0..3)
                    .filter(|&edge_index| edges[edge_index])
                    .filter_map(|edge_index| {
                        let edge_start = mesh.positions.get(*corners.get(edge_index)?)?;
                        let edge_end = mesh.positions.get(*corners.get((edge_index + 1) % 3)?)?;
                        Some(segment_distance(surface.pos, *edge_start, *edge_end))
                    })
                    .fold(f32::INFINITY, f32::min);
                (nearest_edge_distance / (self.settings.edge_distance * self.mesh_scale).max(1e-9)).clamp(0.0, 1.0).powf(self.settings.edge_contrast.max(1e-6))
            }
            _ => 1.0,
        };
        let base = match self.settings.method {
            SkewCorrectionMethod::MeshNormalsBased => surface.nrm,
            SkewCorrectionMethod::FaceNormalsBased => {
                if surface.geo_nrm.dot(surface.nrm) < 0.0 {
                    -surface.geo_nrm
                } else {
                    surface.geo_nrm
                }
            }
        };
        let blend = amount * protection;
        (up * (1.0 - blend) + base * blend).try_normalize().unwrap_or(up)
    }
}

pub(super) fn hard_edges(mesh: &Mesh) -> Vec<[bool; 3]> {
    let pt = |i: usize| mesh.positions.get(i).copied().unwrap_or(Vec3::ZERO);
    let key = |i: usize| mesh.positions.get(i).map_or([0; 3], |p| p.to_array().map(|c| (c + 0.0).to_bits()));
    let ordered = |a: [u32; 3], b: [u32; 3]| if a <= b { (a, b) } else { (b, a) };
    let count = mesh.triangle_count();
    let mut face = vec![Vec3::ZERO; count];
    let mut edges: HashMap<([u32; 3], [u32; 3]), Vec<usize>> = HashMap::new();
    for (t, f) in face.iter_mut().enumerate() {
        let Some(c) = mesh.tri(t) else { continue };
        *f = (pt(c[1]) - pt(c[0])).cross(pt(c[2]) - pt(c[0])).try_normalize().unwrap_or(Vec3::ZERO);
        for k in 0..3 {
            edges.entry(ordered(key(c[k]), key(c[(k + 1) % 3]))).or_default().push(t);
        }
    }
    (0..count)
        .map(|t| {
            let mut out = [false; 3];
            if let Some(c) = mesh.tri(t) {
                let own = face.get(t).copied().unwrap_or(Vec3::ZERO);
                for (k, hard) in out.iter_mut().enumerate() {
                    let others = edges.get(&ordered(key(c[k]), key(c[(k + 1) % 3])));
                    *hard = others.is_some_and(|ts| ts.iter().any(|&o| o != t && face.get(o).is_some_and(|f| f.dot(own) < HARD_EDGE_COS)));
                }
            }
            out
        })
        .collect()
}

pub(super) fn segment_distance(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let ab = b - a;
    let len2 = ab.length_squared();
    let t = if len2 > 1e-20 { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
    (p - (a + ab * t)).length()
}

use super::sampling::{generated_hue, interpolate, splitmix64};
use crate::tracing::{ANY, TraceMesh};
use std::collections::HashMap;
use std::sync::Arc;
use texturecraft_core::{HighPolySettings, IdColorGenerator, IdColorSource, IdSettings};
use texturecraft_geom::glam::{Vec2, Vec4};
use texturecraft_geom::{BaseColor, Bvh, Mesh, Surface};
use texturecraft_texel::{linear_to_srgb, srgb_to_linear};

pub(super) struct Source<'a> {
    pub(super) mesh: &'a Mesh,
    bvh: &'a Bvh,
    pub(super) part: Vec<u32>,
    pub(super) key: Vec<u32>,
    backface_culling: Vec<bool>,
    file_ids: &'a [u32],
    mesh_ids: Vec<u32>,
    file_count: u32,
    mesh_count: u32,
    base_color: &'a [Option<Arc<BaseColor>>],
}

impl<'a> Source<'a> {
    pub(super) fn new(
        mesh: &'a Mesh,
        bvh: &'a Bvh,
        base_color: &'a [Option<Arc<BaseColor>>],
        file_ids: &'a [u32],
        suffix: &str,
        settings: &HighPolySettings,
        keys: &mut HashMap<String, u32>,
    ) -> Source<'a> {
        let triangle_count = mesh.triangle_count();
        let mut part = vec![ANY; triangle_count];
        let mut backface_culling = vec![false; triangle_count];
        let mut mesh_keys = HashMap::new();
        let mut mesh_ids = Vec::with_capacity(mesh.submeshes.len());
        let mut part_keys = Vec::with_capacity(mesh.submeshes.len());
        let backface_suffix = settings.ignore_backface_suffix.trim().to_lowercase();
        for (part_index, submesh) in mesh.submeshes.iter().enumerate() {
            let first_triangle = (submesh.start / 3) as usize;
            let last_triangle = (submesh.start.saturating_add(submesh.count) / 3) as usize;
            let range = first_triangle.min(last_triangle)..last_triangle.min(triangle_count);
            part.get_mut(range.clone()).unwrap_or_default().fill(part_index as u32);
            let name = submesh.name.trim().to_lowercase();
            let ignore_backface = !backface_suffix.is_empty() && name.ends_with(&backface_suffix);
            backface_culling.get_mut(range).unwrap_or_default().fill(ignore_backface);
            let name = if ignore_backface { name.strip_suffix(&backface_suffix).unwrap_or(&name) } else { &name };
            part_keys.push(match_key(name, suffix, keys));
            let next = mesh_keys.len() as u32;
            let file_id = file_ids.get(part_index).copied().unwrap_or(0);
            mesh_ids.push(*mesh_keys.entry((file_id, name.to_owned())).or_insert(next));
        }
        let key = part.iter().map(|part_index| part_keys.get(*part_index as usize).copied().unwrap_or(ANY)).collect();
        Source {
            mesh,
            bvh,
            part,
            key,
            backface_culling,
            file_ids,
            file_count: file_ids.iter().copied().max().unwrap_or(0).saturating_add(1),
            mesh_count: mesh_keys.len() as u32,
            mesh_ids,
            base_color,
        }
    }

    pub(super) fn trace(&self) -> TraceMesh<'_> {
        TraceMesh { mesh: self.mesh, bvh: self.bvh, keys: &self.key, backface_culling: &self.backface_culling }
    }

    pub(super) fn id_color(&self, surface: &Surface, barycentric: [f32; 2], settings: IdSettings) -> [f32; 3] {
        let part_index = self.part.get(surface.tri as usize).copied().unwrap_or(0) as usize;
        match settings.source {
            IdColorSource::VertexColor => interpolate(self.mesh, &self.mesh.colors, surface.tri, barycentric).unwrap_or(Vec4::ONE).truncate().to_array(),
            IdColorSource::MaterialColor => {
                let material = self.mesh.submeshes.get(part_index).map(|part| part.material);
                material.and_then(|index| self.base_color.get(index)?.as_ref()).map_or([0.6; 3], |color| color.factor.truncate().to_array())
            }
            source => {
                let (identity, count) = match source {
                    IdColorSource::FileId => (self.file_ids.get(part_index).copied().unwrap_or(0), self.file_count),
                    IdColorSource::MeshId => (self.mesh_ids.get(part_index).copied().unwrap_or(0), self.mesh_count),
                    _ => (part_index as u32, self.mesh.submeshes.len() as u32),
                };
                match settings.generator {
                    IdColorGenerator::HueShift => generated_hue(identity as usize),
                    IdColorGenerator::Grayscale => [(identity + 1) as f32 / count.max(1) as f32; 3],
                    IdColorGenerator::Random => {
                        let seed = splitmix64(u64::from(identity));
                        [0, 16, 32].map(|shift| ((seed >> shift) & 0xffff) as f32 / 65535.0)
                    }
                }
            }
        }
    }

    pub(super) fn color(&self, surface: &Surface, barycentric: [f32; 2]) -> Vec4 {
        let vertex = interpolate(self.mesh, &self.mesh.colors, surface.tri, barycentric).unwrap_or(Vec4::ONE);
        let material = self.part.get(surface.tri as usize).and_then(|part_index| self.mesh.submeshes.get(*part_index as usize)).map(|submesh| submesh.material);
        match material.and_then(|material_index| self.base_color.get(material_index)?.as_deref()) {
            Some(material_color) => vertex * base_color(material_color, surface.uv),
            None => vertex,
        }
    }
}

fn base_color(material: &BaseColor, uv: Vec2) -> Vec4 {
    let sampled_color = if material.texels.is_empty() || !uv.is_finite() {
        Vec4::ONE
    } else {
        let (width, height) = (i64::from(material.width.max(1)), i64::from(material.height.max(1)));
        let texel = |x: i64, y: i64| {
            let rgba = material.texels.get((y.rem_euclid(height) * width + x.rem_euclid(width)) as usize).copied().unwrap_or([255; 4]);
            let color = |component: usize| f32::from(rgba[component]) / 255.0;
            Vec4::new(srgb_to_linear(color(0)), srgb_to_linear(color(1)), srgb_to_linear(color(2)), color(3))
        };
        let (fx, fy) = (uv.x * width as f32 - 0.5, uv.y * height as f32 - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let (x, y) = (x0 as i64, y0 as i64);
        texel(x, y).lerp(texel(x + 1, y), tx).lerp(texel(x, y + 1).lerp(texel(x + 1, y + 1), tx), ty)
    };
    let color = sampled_color * material.factor;
    Vec4::new(linear_to_srgb(color.x), linear_to_srgb(color.y), linear_to_srgb(color.z), color.w)
}

fn match_key(name: &str, suffix: &str, keys: &mut HashMap<String, u32>) -> u32 {
    let name = name.trim().to_lowercase();
    let suffix = suffix.trim().to_lowercase();
    let base = name.strip_suffix(suffix.as_str()).filter(|_| !suffix.is_empty()).unwrap_or(&name).to_string();
    let next = keys.len() as u32;
    *keys.entry(base).or_insert(next)
}

use super::ANY;
use texturecraft_geom::glam::Vec3;
use texturecraft_geom::{Bvh, Mesh};

#[derive(Clone, Copy, Debug)]
pub struct TraceMesh<'a> {
    pub mesh: &'a Mesh,
    pub bvh: &'a Bvh,
    /// Part key per triangle in original mesh order
    pub keys: &'a [u32],
    pub backface_culling: &'a [bool],
}

impl TraceMesh<'_> {
    pub(super) fn key_matches(&self, triangle: u32, key: u32) -> bool {
        key == ANY || self.keys.get(triangle as usize) == Some(&key)
    }

    pub(super) fn back_facing(&self, triangle: u32, direction: Vec3) -> bool {
        let position = |vertex_index: usize| self.mesh.positions.get(vertex_index).copied().unwrap_or(Vec3::ZERO);
        self.mesh.tri(triangle as usize).is_some_and(|[a, b, c]| (position(b) - position(a)).cross(position(c) - position(a)).dot(direction) > 0.0)
    }
}

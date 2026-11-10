use crate::BakeError;
use texturecraft_geom::Mesh;
use texturecraft_geom::glam::Vec3;

/// Smallest size a mesh is treated as having, so a flat or empty mesh still gives finite distances
const MIN_MESH_SIZE: f32 = 1e-6;

/// How far a ray starts past the surface it leaves, as a fraction of the mesh diagonal
const RAY_EPSILON_PER_DIAGONAL: f32 = 1e-6;

/// The low-poly mesh's size, which scales the bake's relative distances
#[derive(Clone, Copy, Debug)]
pub(crate) struct MeshExtent {
    pub(crate) bounds_min: Vec3,
    pub(crate) diagonal: f32,
    /// The bounding box's largest side
    pub(crate) scale: f32,
}

impl MeshExtent {
    pub(crate) fn of(mesh: &Mesh) -> Result<MeshExtent, BakeError> {
        let (bounds_min, bounds_max) = mesh.bounds();
        let diagonal = (bounds_max - bounds_min).length().max(MIN_MESH_SIZE);
        if !diagonal.is_finite() {
            return Err(BakeError::Invalid("mesh bounds are too large".into()));
        }

        Ok(MeshExtent { bounds_min, diagonal, scale: (bounds_max - bounds_min).max_element().max(MIN_MESH_SIZE) })
    }

    /// How far a ray starts past the surface it leaves, in world units
    pub(crate) fn ray_epsilon(&self) -> f32 {
        RAY_EPSILON_PER_DIAGONAL * self.diagonal
    }
}

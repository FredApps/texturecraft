use std::sync::Arc;
use texturecraft_geom::{Bvh, Mesh};

/// Every high poly appended into one mesh, its BVH and per-material base colors.
#[derive(Clone, Debug)]
pub struct MergedHigh {
    pub file_ids: Arc<Vec<u32>>,
    pub mesh: Arc<Mesh>,
    pub bvh: Arc<Bvh>,
    pub base_color: super::MaterialColors,
}

use std::sync::Arc;
use texturecraft_geom::{BaseColor, Mesh};

/// A mesh file the baker reads (high poly or cage): its name, original bytes (saved in the
/// project), parsed mesh and its materials' base colors (high polys).
#[derive(Clone, Debug)]
pub struct BakeMesh {
    pub name: String,
    pub bytes: Arc<Vec<u8>>,
    pub mesh: Arc<Mesh>,
    pub base_color: Vec<Option<Arc<BaseColor>>>,
}

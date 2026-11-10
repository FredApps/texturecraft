use std::collections::BTreeMap;
use std::sync::Arc;

use texturecraft_core::BakedMap;

/// The result of a streamed bake: the finished maps stay shared (`Arc`) with whoever was shown them,
/// so the preview and the document hold one buffer per map, never two
#[derive(Clone, Debug)]
pub struct StreamedBake {
    pub resolution: u32,
    pub height: u32,
    pub maps: BTreeMap<BakedMap, Arc<Vec<f32>>>,
    pub tracer: String,
}

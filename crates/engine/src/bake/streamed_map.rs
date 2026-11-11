use std::sync::Arc;

use texturecraft_core::BakedMap;

/// One finished map of a streamed bake (see [`crate::BakeTask::run_streaming`]).
#[derive(Clone, Debug)]
pub struct StreamedMap {
    pub map: BakedMap,
    /// Position among the maps this bake makes, from 0, and how many there are.
    pub index: usize,
    pub total: usize,
    /// Width and height in texels.
    pub resolution: u32,
    pub height: u32,
    /// Row-major, `map.components()` floats per texel, padded.
    pub data: Arc<Vec<f32>>,
}

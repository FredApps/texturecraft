use texturecraft_compose::BakedMaps;
use texturecraft_core::BakedMap;

/// The maps of one texture set streamed so far.
#[derive(Clone, Debug, Default)]
pub struct PreviewSet {
    pub baked: BakedMaps,
    /// The maps this set's bake makes, in the order they arrive.
    pub order: Vec<BakedMap>,
}

use texturecraft_core::BakedMap;

/// The map a bake is working on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stage {
    pub texture_set: usize,
    pub map: BakedMap,
    /// 1-based position among the set's maps, and how many there are.
    pub position: usize,
    pub total: usize,
}

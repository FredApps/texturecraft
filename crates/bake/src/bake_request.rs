use crate::{BakePaintMap, HighPoly};
use std::sync::Arc;
use texturecraft_core::BakeSettings;
use texturecraft_geom::{BaseColor, Bvh, Mesh};

#[derive(Clone, Copy, Debug)]
pub struct BakeRequest<'a> {
    pub mesh: &'a Mesh,
    pub bvh: &'a Bvh,
    pub material: usize,
    pub low_base_color: &'a [Option<Arc<BaseColor>>],
    pub settings: &'a BakeSettings,
    pub high: Option<HighPoly<'a>>,
    pub cage: Option<&'a Mesh>,
    pub skew: Option<&'a BakePaintMap>,
    pub cage_paint: Option<&'a BakePaintMap>,
    /// UV atlas grid; each tile uses the configured output resolution
    pub tiles: (u32, u32),
}

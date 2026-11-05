use serde::{Deserialize, Serialize};

/// Position map normalization
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PositionMode {
    /// Each axis spans the bounding box (0..1 per axis)
    BoundingBox,
    /// One scale for every axis: the largest extent around the box centre (keeps proportions)
    #[default]
    BoundingSphere,
    /// Keep mesh coordinates without normalization
    None,
}

//! Where a stroke in the 2D view is between pointer samples.

use texturecraft_geom::glam::Vec2;

/// A 2D view stroke's progress: its last point, the distance carried to the next stamp and the stamps so far.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct UvStrokeState {
    /// The last sample's texture coordinates (None before the first sample).
    pub last: Option<Vec2>,
    /// Distance from the last stamp to the last sample, at most one stamp step.
    pub carry: f32,
    /// Stamps so far: the next stamp's seed, so noise alphas don't repeat from sample to sample.
    pub stamps: u32,
}

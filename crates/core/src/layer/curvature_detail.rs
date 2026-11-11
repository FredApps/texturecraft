use serde::{Deserialize, Serialize};

use super::curvature_mode::CurvatureMode;

/// How a curvature generator reads the curvature map: its mode and how much each detail size adds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CurvatureDetail {
    pub mode: CurvatureMode,
    /// Weight (0..1) of each size in [`CurvatureDetail::SIZES`], finest first.
    pub weights: [f32; CurvatureDetail::SIZES.len()],
}

impl CurvatureDetail {
    /// Detail sizes as blur radii in texels at 2K: the map itself, then doubling radii.
    pub const SIZES: [f32; 7] = [0.0, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0];

    /// Weights falling off with size, tuned by eye on hard-surface bakes: sharp edges first, broad shapes lit faintly.
    pub const BROAD_WEIGHTS: [f32; 7] = [1.0, 0.85, 0.7, 0.55, 0.4, 0.25, 0.15];

    /// Mostly the finer sizes, tuned the same way: worn edges are thin lines.
    pub const EDGE_WEIGHTS: [f32; 7] = [1.0, 0.8, 0.5, 0.25, 0.1, 0.0, 0.0];

    /// How many sizes are blurred copies of the map (all but the first).
    pub const BLURRED_SIZES: usize = CurvatureDetail::SIZES.len() - 1;

    /// Which blurred sizes this detail reads: those with a weight above 0.
    pub fn blurred_sizes_in_use(&self) -> [bool; CurvatureDetail::BLURRED_SIZES] {
        std::array::from_fn(|blurred| self.weights.get(blurred + 1).is_some_and(|weight| *weight > 0.0))
    }
}

impl Default for CurvatureDetail {
    fn default() -> Self {
        CurvatureDetail { mode: CurvatureMode::Edges, weights: CurvatureDetail::BROAD_WEIGHTS }
    }
}

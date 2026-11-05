use super::SelfOcclusion;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MeshCurvatureSettings {
    pub rays: u32,
    /// Radius (0.00001..=10 relative to bounds, 0.00001..=1 in mesh units)
    pub sampling_radius: f32,
    pub relative_to_bounding_box: bool,
    pub self_intersection: SelfOcclusion,
    pub auto_tonemapping: bool,
    /// Value bounds (-10..=10)
    pub value_min: f32,
    pub value_max: f32,
}

impl Default for MeshCurvatureSettings {
    fn default() -> Self {
        Self {
            rays: 32,
            sampling_radius: 0.001,
            relative_to_bounding_box: true,
            self_intersection: SelfOcclusion::Always,
            auto_tonemapping: true,
            value_min: -1.0,
            value_max: 1.0,
        }
    }
}

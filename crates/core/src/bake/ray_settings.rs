use super::RayDistribution;
use serde::{Deserialize, Serialize};

/// Secondary-ray parameters of one map (ambient occlusion, thickness, bent normals)
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RaySettings {
    /// Rays per sample (settings range 1..=256)
    pub rays: u32,
    /// Minimum distance in the same units as max_distance (0.00001..=1)
    pub min_distance: f32,
    /// Maximum distance: 0.00001..=1 relative to the bounding box, or 0.00001..=10 in mesh units
    pub max_distance: f32,
    pub relative_to_bounding_box: bool,
    /// Full cone angle around the normal in degrees (180 = the whole hemisphere)
    pub spread: f32,
    pub distribution: RayDistribution,
}

impl Default for RaySettings {
    fn default() -> Self {
        RaySettings { rays: 64, min_distance: 0.00001, max_distance: 1.0, relative_to_bounding_box: true, spread: 180.0, distribution: RayDistribution::Cosine }
    }
}

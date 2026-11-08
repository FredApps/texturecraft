use super::{BakeAntialiasing, BakeResolution};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CommonBakeSettings {
    pub is_enabled: bool,
    pub resolution: BakeResolution,
    /// None locks both axes to the same resolution
    pub resolution_height: Option<BakeResolution>,
    pub antialiasing: BakeAntialiasing,
    /// Padding width in pixels (0..=128)
    pub dilation: u32,
    pub apply_diffusion: bool,
}

impl Default for CommonBakeSettings {
    fn default() -> Self {
        Self {
            is_enabled: true,
            resolution: BakeResolution::R2048,
            resolution_height: None,
            antialiasing: BakeAntialiasing::None,
            dilation: 1,
            apply_diffusion: true,
        }
    }
}

impl CommonBakeSettings {
    pub fn output_size(&self) -> (u32, u32) {
        (self.resolution.pixels(), self.resolution_height.unwrap_or(self.resolution).pixels())
    }
}

use super::HeightNormalization;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HeightSettings {
    pub normalization: HeightNormalization,
    pub scaling_divisor: f32,
}

impl Default for HeightSettings {
    fn default() -> Self {
        Self { normalization: HeightNormalization::RelativeToLowMesh, scaling_divisor: 1.0 }
    }
}

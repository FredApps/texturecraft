use super::SkewCorrectionMethod;
use serde::{Deserialize, Serialize};

/// Skew controls independent of the painted pixel asset
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SkewSettings {
    pub method: SkewCorrectionMethod,
    pub edge_protection: bool,
    /// Edge protection distance (0..=1), retained while protection is disabled
    pub edge_distance: f32,
    /// Edge protection contrast (0..=1)
    pub edge_contrast: f32,
}

impl Default for SkewSettings {
    fn default() -> Self {
        Self { method: SkewCorrectionMethod::MeshNormalsBased, edge_protection: false, edge_distance: 0.15, edge_contrast: 0.5 }
    }
}

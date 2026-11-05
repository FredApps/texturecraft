use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GroundPlaneSettings {
    pub enabled: bool,
    /// Offset in mesh units (0..=100)
    pub offset: f32,
}

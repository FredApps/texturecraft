use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NormalMapCurvatureSettings {
    pub details: f32,
    pub enable_seams: bool,
    pub seams_intensity: f32,
}

impl Default for NormalMapCurvatureSettings {
    fn default() -> Self {
        Self { details: 0.5, enable_seams: true, seams_intensity: 1.0 }
    }
}

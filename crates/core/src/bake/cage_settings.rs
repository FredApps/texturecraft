use super::CageType;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CageSettings {
    pub kind: CageType,
    /// Distance-based cage offsets (slider range 0..=0.05)
    pub max_frontal: f32,
    pub max_rear: f32,
    pub relative_to_bounding_box: bool,
    pub average_normals: bool,
}

impl Default for CageSettings {
    fn default() -> Self {
        Self { kind: CageType::DistanceBased, max_frontal: 0.01, max_rear: 0.01, relative_to_bounding_box: true, average_normals: true }
    }
}

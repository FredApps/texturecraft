use super::{RaySettings, SelfOcclusion, ThicknessNormalization};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThicknessSettings {
    pub rays: RaySettings,
    pub self_occlusion: SelfOcclusion,
    pub normalization: ThicknessNormalization,
}

impl Default for ThicknessSettings {
    fn default() -> Self {
        Self {
            rays: RaySettings { max_distance: 0.1, ..RaySettings::default() },
            self_occlusion: SelfOcclusion::Always,
            normalization: ThicknessNormalization::RelativeToMinMax,
        }
    }
}

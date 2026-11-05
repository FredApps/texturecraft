use super::{BackfaceMode, RaySettings, SelfOcclusion};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BentNormalSettings {
    pub rays: RaySettings,
    pub ignore_backface: BackfaceMode,
    pub self_occlusion: SelfOcclusion,
}

use super::{AmbientOcclusionAttenuation, BackfaceMode, GroundPlaneSettings, RaySettings, SelfOcclusion};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AmbientOcclusionSettings {
    pub rays: RaySettings,
    pub ignore_backface: BackfaceMode,
    pub self_occlusion: SelfOcclusion,
    pub attenuation: AmbientOcclusionAttenuation,
    pub ground_plane: GroundPlaneSettings,
}

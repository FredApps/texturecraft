use super::{PositionAxis, PositionMode, PositionNormalizationScale};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PositionSettings {
    pub axis: PositionAxis,
    pub normalization: PositionMode,
    pub scale: PositionNormalizationScale,
}

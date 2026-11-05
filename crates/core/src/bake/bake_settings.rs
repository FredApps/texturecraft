use super::{CageSettings, CommonBakeSettings, HighPolySettings, MeshMapSettings, SkewSettings};
use serde::{Deserialize, Serialize};

/// Persisted clean-room bake settings, grouped by the controls they belong to
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BakeSettings {
    pub common: CommonBakeSettings,
    pub high_poly: HighPolySettings,
    pub cage: CageSettings,
    pub skew: SkewSettings,
    pub maps: MeshMapSettings,
}

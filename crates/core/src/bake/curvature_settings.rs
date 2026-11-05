use super::{CurvatureMethod, MeshCurvatureSettings, NormalMapCurvatureSettings};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CurvatureSettings {
    pub method: CurvatureMethod,
    pub normal_map: NormalMapCurvatureSettings,
    pub mesh: MeshCurvatureSettings,
}

impl Default for CurvatureSettings {
    fn default() -> Self {
        Self { method: CurvatureMethod::NormalMap, normal_map: NormalMapCurvatureSettings::default(), mesh: MeshCurvatureSettings::default() }
    }
}

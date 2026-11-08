use super::{CurvatureMethod, MeshCurvatureSettings, NormalMapCurvatureSettings};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CurvatureSettings {
    pub method: CurvatureMethod,
    pub normal_map: NormalMapCurvatureSettings,
    pub mesh: MeshCurvatureSettings,
}

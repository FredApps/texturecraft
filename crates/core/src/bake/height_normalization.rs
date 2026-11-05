use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum HeightNormalization {
    RelativeToRayDistances,
    #[default]
    RelativeToLowMesh,
    RelativeToMinMax,
    Manual,
}

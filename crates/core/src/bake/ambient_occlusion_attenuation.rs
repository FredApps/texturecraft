use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AmbientOcclusionAttenuation {
    None,
    Smooth,
    #[default]
    Linear,
}

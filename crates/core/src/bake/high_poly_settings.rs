use super::MeshMatchMode;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HighPolySettings {
    pub use_low_as_high: bool,
    pub ignore_backface: bool,
    pub match_mode: MeshMatchMode,
    pub low_suffix: String,
    pub high_suffix: String,
    pub ignore_backface_suffix: String,
}

impl Default for HighPolySettings {
    fn default() -> Self {
        Self {
            use_low_as_high: false,
            ignore_backface: true,
            match_mode: MeshMatchMode::Always,
            low_suffix: "_low".into(),
            high_suffix: "_high".into(),
            ignore_backface_suffix: "_ignorebf".into(),
        }
    }
}

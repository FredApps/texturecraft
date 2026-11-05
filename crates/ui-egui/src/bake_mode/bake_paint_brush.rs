use serde::{Deserialize, Serialize};
use texturecraft_engine::core::BakePaintTarget;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BakePaintBrush {
    pub on: bool,
    pub target: BakePaintTarget,
    pub radius: f32,
    pub value: f32,
    pub hardness: f32,
}

impl Default for BakePaintBrush {
    fn default() -> Self {
        Self { on: false, target: BakePaintTarget::Skew, radius: 0.03, value: 1.0, hardness: 0.5 }
    }
}

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum IdColorGenerator {
    Random,
    #[default]
    HueShift,
    Grayscale,
}

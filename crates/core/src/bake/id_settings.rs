use super::{IdColorGenerator, IdColorSource};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IdSettings {
    pub source: IdColorSource,
    pub generator: IdColorGenerator,
}

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackfaceMode {
    #[default]
    Never,
    Always,
    ByMeshName,
}

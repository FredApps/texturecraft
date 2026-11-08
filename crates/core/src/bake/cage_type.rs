use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CageType {
    #[default]
    DistanceBased,
    CustomFile,
}

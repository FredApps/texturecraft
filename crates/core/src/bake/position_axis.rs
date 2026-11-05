use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PositionAxis {
    #[default]
    All,
    X,
    Y,
    Z,
}

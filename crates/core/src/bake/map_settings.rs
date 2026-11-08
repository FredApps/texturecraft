use serde::{Deserialize, Serialize};

/// One map's enablement and typed parameters
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MapSettings<T: Default = ()> {
    pub is_enabled: bool,
    pub parameters: T,
}

impl<T: Default> Default for MapSettings<T> {
    fn default() -> Self {
        Self { is_enabled: true, parameters: T::default() }
    }
}

impl<T: Default> MapSettings<T> {
    pub fn disabled() -> Self {
        Self { is_enabled: false, ..Self::default() }
    }
}

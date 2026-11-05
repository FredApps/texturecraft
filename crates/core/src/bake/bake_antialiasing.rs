use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BakeAntialiasing {
    #[default]
    None,
    Subsampling2x2,
    Subsampling4x4,
    Subsampling8x8,
}

impl BakeAntialiasing {
    pub const ALL: [Self; 4] = [Self::None, Self::Subsampling2x2, Self::Subsampling4x4, Self::Subsampling8x8];

    pub fn samples_per_axis(self) -> u32 {
        match self {
            Self::None => 1,
            Self::Subsampling2x2 => 2,
            Self::Subsampling4x4 => 4,
            Self::Subsampling8x8 => 8,
        }
    }

    pub fn from_samples_per_axis(samples: u32) -> Option<Self> {
        Self::ALL.into_iter().find(|antialiasing| antialiasing.samples_per_axis() == samples)
    }
}

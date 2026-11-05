use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BakeResolution {
    R16,
    R32,
    R64,
    R128,
    R256,
    R512,
    R1024,
    #[default]
    R2048,
    R4096,
    R8192,
}

impl BakeResolution {
    pub const ALL: [Self; 10] = [Self::R16, Self::R32, Self::R64, Self::R128, Self::R256, Self::R512, Self::R1024, Self::R2048, Self::R4096, Self::R8192];

    pub fn pixels(self) -> u32 {
        match self {
            Self::R16 => 16,
            Self::R32 => 32,
            Self::R64 => 64,
            Self::R128 => 128,
            Self::R256 => 256,
            Self::R512 => 512,
            Self::R1024 => 1024,
            Self::R2048 => 2048,
            Self::R4096 => 4096,
            Self::R8192 => 8192,
        }
    }

    pub fn from_pixels(pixels: u32) -> Option<Self> {
        Self::ALL.into_iter().find(|resolution| resolution.pixels() == pixels)
    }
}

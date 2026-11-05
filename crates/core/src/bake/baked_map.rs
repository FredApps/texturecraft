use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum BakedMap {
    Normal,
    WorldNormal,
    Id,
    AmbientOcclusion,
    Curvature,
    Position,
    Thickness,
    Height,
    /// World-space direction of least occlusion (packed like the world-space normal)
    BentNormal,
    /// High-poly coverage times vertex and material alpha
    Opacity,
}

impl BakedMap {
    pub const ALL: [BakedMap; 10] = [
        BakedMap::Normal,
        BakedMap::WorldNormal,
        BakedMap::Id,
        BakedMap::AmbientOcclusion,
        BakedMap::Curvature,
        BakedMap::Position,
        BakedMap::Thickness,
        BakedMap::Height,
        BakedMap::BentNormal,
        BakedMap::Opacity,
    ];

    /// Mesh-map identifier shared with MaterialCraft
    pub fn id(self) -> &'static str {
        match self {
            BakedMap::Normal => "normal",
            BakedMap::WorldNormal => "worldNormal",
            BakedMap::Id => "id",
            BakedMap::AmbientOcclusion => "ambientOcclusion",
            BakedMap::Curvature => "curvature",
            BakedMap::Position => "position",
            BakedMap::Thickness => "thickness",
            BakedMap::Height => "height",
            BakedMap::BentNormal => "bentNormal",
            BakedMap::Opacity => "opacity",
        }
    }

    /// Components stored (3 for vector maps, 1 for scalar maps)
    pub fn components(self) -> usize {
        match self {
            BakedMap::Normal | BakedMap::WorldNormal | BakedMap::Id | BakedMap::Position | BakedMap::BentNormal => 3,
            _ => 1,
        }
    }

    /// Value where nothing was baked (uncovered texels, missing maps)
    pub fn neutral(self) -> [f32; 3] {
        match self {
            BakedMap::Normal | BakedMap::WorldNormal | BakedMap::BentNormal => [0.5, 0.5, 1.0],
            BakedMap::AmbientOcclusion | BakedMap::Thickness => [1.0; 3],
            BakedMap::Curvature | BakedMap::Height => [0.5; 3],
            BakedMap::Id | BakedMap::Position | BakedMap::Opacity => [0.0; 3],
        }
    }

    pub fn parse(identifier: &str) -> Option<BakedMap> {
        match identifier {
            "normal" => Some(BakedMap::Normal),
            "worldNormal" => Some(BakedMap::WorldNormal),
            "id" => Some(BakedMap::Id),
            "ambientOcclusion" => Some(BakedMap::AmbientOcclusion),
            "curvature" => Some(BakedMap::Curvature),
            "position" => Some(BakedMap::Position),
            "thickness" => Some(BakedMap::Thickness),
            "height" => Some(BakedMap::Height),
            "bentNormal" => Some(BakedMap::BentNormal),
            "opacity" => Some(BakedMap::Opacity),
            _ => None,
        }
    }
}

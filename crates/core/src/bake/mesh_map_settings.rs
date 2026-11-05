use super::{
    AmbientOcclusionSettings, BakedMap, BentNormalSettings, CurvatureSettings, HeightSettings, IdSettings, MapSettings, PositionSettings, ThicknessSettings,
};
use serde::{Deserialize, Serialize};

/// Per-map configuration; enabled outputs are derived from these flags
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MeshMapSettings {
    pub normal: MapSettings,
    pub world_normal: MapSettings,
    pub id: MapSettings<IdSettings>,
    pub ao: MapSettings<AmbientOcclusionSettings>,
    pub curvature: MapSettings<CurvatureSettings>,
    pub position: MapSettings<PositionSettings>,
    pub thickness: MapSettings<ThicknessSettings>,
    pub height: MapSettings<HeightSettings>,
    pub bent_normal: MapSettings<BentNormalSettings>,
    pub opacity: MapSettings,
}

impl Default for MeshMapSettings {
    fn default() -> Self {
        Self {
            normal: MapSettings::default(),
            world_normal: MapSettings::default(),
            id: MapSettings::default(),
            ao: MapSettings::default(),
            curvature: MapSettings::default(),
            position: MapSettings::default(),
            thickness: MapSettings::default(),
            height: MapSettings::disabled(),
            bent_normal: MapSettings::disabled(),
            opacity: MapSettings::disabled(),
        }
    }
}

impl MeshMapSettings {
    pub fn is_enabled(&self, map: BakedMap) -> bool {
        match map {
            BakedMap::Normal => self.normal.is_enabled,
            BakedMap::WorldNormal => self.world_normal.is_enabled,
            BakedMap::Id => self.id.is_enabled,
            BakedMap::AmbientOcclusion => self.ao.is_enabled,
            BakedMap::Curvature => self.curvature.is_enabled,
            BakedMap::Position => self.position.is_enabled,
            BakedMap::Thickness => self.thickness.is_enabled,
            BakedMap::Height => self.height.is_enabled,
            BakedMap::BentNormal => self.bent_normal.is_enabled,
            BakedMap::Opacity => self.opacity.is_enabled,
        }
    }

    pub fn set_enabled(&mut self, map: BakedMap, is_enabled: bool) {
        let enabled = match map {
            BakedMap::Normal => &mut self.normal.is_enabled,
            BakedMap::WorldNormal => &mut self.world_normal.is_enabled,
            BakedMap::Id => &mut self.id.is_enabled,
            BakedMap::AmbientOcclusion => &mut self.ao.is_enabled,
            BakedMap::Curvature => &mut self.curvature.is_enabled,
            BakedMap::Position => &mut self.position.is_enabled,
            BakedMap::Thickness => &mut self.thickness.is_enabled,
            BakedMap::Height => &mut self.height.is_enabled,
            BakedMap::BentNormal => &mut self.bent_normal.is_enabled,
            BakedMap::Opacity => &mut self.opacity.is_enabled,
        };
        *enabled = is_enabled;
    }

    pub fn enabled(&self) -> impl Iterator<Item = BakedMap> {
        BakedMap::ALL.into_iter().filter(|map| self.is_enabled(*map))
    }

    pub fn select(&mut self, maps: impl IntoIterator<Item = BakedMap>) {
        for map in BakedMap::ALL {
            self.set_enabled(map, false);
        }
        for map in maps {
            self.set_enabled(map, true);
        }
    }
}

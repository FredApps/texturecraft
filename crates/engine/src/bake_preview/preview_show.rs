use texturecraft_core::BakedMap;

/// What the 3D view shows while a bake runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewShow {
    /// The newest finished map (the first one, still neutral, before any is done).
    Follow,
    /// One map, pinned: neutral until it is baked.
    Pin(BakedMap),
    /// The lit material.
    Material,
}

impl PreviewShow {
    pub fn id(self) -> String {
        match self {
            PreviewShow::Follow => "follow".into(),
            PreviewShow::Pin(map) => format!("map:{}", map.id()),
            PreviewShow::Material => "material".into(),
        }
    }

    pub fn parse(text: &str) -> Option<PreviewShow> {
        match text {
            "follow" => Some(PreviewShow::Follow),
            "material" => Some(PreviewShow::Material),
            _ => text.strip_prefix("map:").and_then(BakedMap::parse).map(PreviewShow::Pin),
        }
    }
}

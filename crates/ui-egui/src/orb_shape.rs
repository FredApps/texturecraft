//! The preview orb of the smart mask and smart material previews, baked on first use

use texturecraft_engine::orb_preview::OrbPreview;

/// Whether the preview orb is baked
#[derive(Default)]
pub enum OrbShape {
    #[default]
    NotBaked,
    /// Box the large session to keep the unbaked worker small
    Baked(Box<OrbPreview>),
    /// The bake failed; previews on the orb stay empty
    Failed,
}

impl OrbShape {
    /// Bake on first use; retain failure to avoid retrying every frame
    pub fn get(&mut self) -> Option<&mut OrbPreview> {
        if matches!(self, OrbShape::NotBaked) {
            *self = match OrbPreview::new() {
                Ok(preview) => OrbShape::Baked(Box::new(preview)),
                Err(e) => {
                    log::warn!("preview orb: {e}");
                    OrbShape::Failed
                }
            };
        }
        match self {
            OrbShape::Baked(preview) => Some(preview),
            OrbShape::NotBaked | OrbShape::Failed => None,
        }
    }
}

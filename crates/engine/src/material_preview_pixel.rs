//! One pixel of a smart material preview (see [`crate::orb_preview::OrbPreview::material`])

use texturecraft_geom::glam::Vec3;

use crate::preview::Sample;

/// The material values seen at a preview pixel, with what a display needs to light the shape
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialPreviewPixel {
    /// Linear material values, averaged over the pixel
    pub sample: Sample,
    /// Surface normal in view space (+X right, +Y up, +Z towards the viewer)
    pub normal: Vec3,
    /// Fraction of the pixel the shape covers (0..=1, anti-aliased outline)
    pub coverage: f32,
}

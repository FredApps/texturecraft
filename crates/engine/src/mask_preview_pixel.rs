//! One pixel of a smart mask preview (see [`crate::orb_preview::OrbPreview::mask`])

use texturecraft_geom::glam::Vec3;

/// The smart mask value seen at a preview pixel, with what a display needs to shade the shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaskPreviewPixel {
    /// Mask value (0..=1).
    pub mask: f32,
    /// Surface normal in view space (+X right, +Y up, +Z towards the viewer).
    pub normal: Vec3,
    /// Fraction of the pixel the shape covers (0..=1, anti-aliased outline).
    pub coverage: f32,
}

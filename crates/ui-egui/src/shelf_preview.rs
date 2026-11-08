//! Shelf assets and their preview renderers

use std::sync::Arc;

use egui::ColorImage;
use texturecraft_engine::compose::ImageRes;
use texturecraft_engine::core::ProceduralKind;
use texturecraft_engine::paint::BrushSettings;
use texturecraft_engine::paint::alphas::LibraryAlpha;
use texturecraft_engine::paint::presets::BrushPreset;
use texturecraft_engine::preview;

use crate::preview_image::{alpha_image, image_preview, procedural_image, sphere_image, stroke_preview};

/// The asset a shelf tile previews
pub enum ShelfPreview {
    /// A built-in material or smart material on a sphere
    Material(&'static str),
    /// Smart mask rendering requires the worker's baked shape
    SmartMask(&'static str),
    /// A library filter on a sample material on a sphere
    Filter(&'static str),
    /// A procedural pattern as a grayscale swatch
    Procedural(ProceduralKind),
    /// A library brush alpha, white on black
    Alpha(&'static LibraryAlpha),
    /// A brush preset's stroke along an S curve
    Brush(&'static BrushPreset),
    /// An image resource by name
    Image(String, Arc<ImageRes>),
}

impl ShelfPreview {
    /// The cache key of the preview (one per asset)
    pub fn key(&self) -> String {
        match self {
            ShelfPreview::Material(id) => format!("mat:{id}"),
            ShelfPreview::SmartMask(id) => format!("mask:{id}"),
            ShelfPreview::Filter(id) => format!("filter:{id}"),
            ShelfPreview::Procedural(kind) => format!("proc:{kind:?}"),
            ShelfPreview::Alpha(alpha) => format!("alpha:{}", alpha.id),
            ShelfPreview::Brush(preset) => format!("brush:{}", preset.id),
            ShelfPreview::Image(name, _) => format!("image:{name}"),
        }
    }

    /// Render a `size²` image; unknown assets return None
    /// Smart masks return None here because their bake belongs to the worker
    pub fn image(&self, size: usize) -> Option<ColorImage> {
        match self {
            ShelfPreview::Material(id) => Some(sphere_image(size, &preview::material(id, size)?)),
            ShelfPreview::SmartMask(_) => None,
            ShelfPreview::Filter(id) => Some(sphere_image(size, &preview::filter(id, size)?)),
            ShelfPreview::Procedural(kind) => Some(procedural_image(*kind, size)),
            ShelfPreview::Alpha(alpha) => Some(alpha_image(alpha, size)),
            ShelfPreview::Brush(preset) => Some(stroke_preview(&preset.applied(&BrushSettings::default()), size)),
            ShelfPreview::Image(_, image) => Some(image_preview(image, size)),
        }
    }
}

#[cfg(test)]
mod tests {
    use texturecraft_engine::filter_lib::FILTERS;
    use texturecraft_engine::materials::MATERIALS;
    use texturecraft_engine::paint::alphas;
    use texturecraft_engine::paint::presets::PRESETS;

    use super::*;

    /// The hover preview's size at 100 % scale
    const BIG_PX: usize = 256;

    #[test]
    fn every_tab_renders_a_big_preview() {
        let image = ImageRes { width: 4, height: 2, rgba: vec![[0.8, 0.4, 0.2, 1.0]; 8], srgb: true };
        let previews = [
            ShelfPreview::Material(MATERIALS[0].id),
            ShelfPreview::Filter(FILTERS[0].id),
            ShelfPreview::Procedural(ProceduralKind::ALL[0]),
            ShelfPreview::Alpha(&alphas::library()[0]),
            ShelfPreview::Brush(&PRESETS[0]),
            ShelfPreview::Image("image".into(), Arc::new(image)),
        ];
        for preview in previews {
            let key = preview.key();
            let image = preview.image(BIG_PX).unwrap_or_else(|| panic!("{key}"));
            assert_eq!(image.size, [BIG_PX, BIG_PX], "{key}");
            assert!(image.pixels.iter().any(|c| c.a() > 0 && c.r().max(c.g()).max(c.b()) > 0), "{key}: something shows");
        }

        let finite = |samples: Vec<Option<preview::Sample>>| {
            samples.iter().flatten().all(|s| s.base.iter().chain([&s.metallic, &s.roughness, &s.height]).all(|v| v.is_finite()))
        };
        assert!(finite(preview::material(MATERIALS[0].id, BIG_PX).unwrap_or_default()));
        assert!(finite(preview::filter(FILTERS[0].id, BIG_PX).unwrap_or_default()));
        assert!(ShelfPreview::SmartMask("edges_sharp").image(BIG_PX).is_none(), "smart masks need the baked shape");
    }
}

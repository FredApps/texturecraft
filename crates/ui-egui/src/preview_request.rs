//! Shelf preview work submitted to [`crate::preview_worker::PreviewWorker`]

use egui::ColorImage;

use crate::mask_preview_image::mask_preview_image;
use crate::mask_shape::MaskShape;
use crate::shelf_preview::ShelfPreview;

/// Render `preview` at `size_px²`; the answer comes back under `key`
pub struct PreviewRequest {
    pub key: String,
    pub preview: ShelfPreview,
    pub size_px: usize,
    /// A hover preview: the worker renders the newest of these before anything else
    // The web renders each request at once, so nothing is reordered there
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub hover: bool,
}

impl PreviewRequest {
    /// Render the asset, baking the mask shape on first use
    /// Unknown assets and failed bakes return None
    pub fn render(&self, shape: &mut MaskShape) -> Option<ColorImage> {
        match &self.preview {
            ShelfPreview::SmartMask(id) => Some(mask_preview_image(self.size_px, &shape.get()?.render(id, self.size_px)?)),
            preview => preview.image(self.size_px),
        }
    }
}

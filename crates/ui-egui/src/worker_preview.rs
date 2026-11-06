//! Cached state of a worker-rendered preview

use egui::TextureHandle;

/// A pending, ready or failed preview
pub enum WorkerPreview {
    /// Requested; the worker has not answered yet
    Rendering,
    Ready(TextureHandle),
    /// The asset is unknown or the worker could not render it (the tile keeps its placeholder)
    Failed,
}

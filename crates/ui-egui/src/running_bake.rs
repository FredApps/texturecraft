use std::sync::Arc;

use texturecraft_engine::bake::Progress;

/// The texture set baking on a worker thread: its progress, its result and the maps it finishes
/// on the way (the 3D view shows one per UI poll).
pub struct RunningBake {
    pub set_index: usize,
    pub progress: Arc<Progress>,
    pub receiver: std::sync::mpsc::Receiver<texturecraft_engine::Result<texturecraft_engine::StreamedBake>>,
    pub maps: std::sync::mpsc::Receiver<texturecraft_engine::StreamedMap>,
    /// Maps finished so far, and how many the set makes.
    pub maps_done: Arc<std::sync::atomic::AtomicUsize>,
    pub maps_total: usize,
}

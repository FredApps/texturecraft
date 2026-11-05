use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};

#[derive(Debug, Default)]
pub struct Progress {
    /// Completed basis points, from 0 to 10,000
    pub done: AtomicU32,
    pub cancel: AtomicBool,
}

impl Progress {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn fraction(&self) -> f32 {
        self.done.load(Relaxed).min(10_000) as f32 / 10_000.0
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Relaxed);
    }
}

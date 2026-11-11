use crate::RunningBake;

/// A background bake of one or more texture sets.
pub struct BakeJob {
    pub queue: Vec<usize>,
    pub current: Option<RunningBake>,
    pub total: usize,
}

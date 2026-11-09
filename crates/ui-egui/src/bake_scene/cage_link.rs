use std::sync::Arc;

use texturecraft_engine::bake::CageShell;

use super::cage_shell_key::CageShellKey;

/// A texture set's cage as the GPU has it
pub(super) struct CageLink {
    pub(super) key: CageShellKey,
    /// `None`: the set bakes without a high poly or its cage can't be built
    pub(super) shell: Option<Arc<CageShell>>,
    /// The renderer's name for the shell's corner order and edges
    pub(super) topology: u64,
    /// The max frontal distance the GPU's corners are at
    pub(super) max_frontal: f32,
}

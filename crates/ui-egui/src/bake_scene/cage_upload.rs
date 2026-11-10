use std::sync::Arc;

use texturecraft_engine::bake::CageShell;
use texturecraft_engine::geom::glam::Vec3;

/// A texture set's cage for the GPU: its corners at the current distance and the shell they come from
pub struct CageUpload {
    pub corners: Vec<Vec3>,
    pub shell: Arc<CageShell>,
}

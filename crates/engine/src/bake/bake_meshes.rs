use super::{BakeMesh, MergedHigh};
use std::sync::{Arc, OnceLock};
use texturecraft_geom::{Bvh, Mesh};

/// The high-definition meshes and the cage. Like the low-poly mesh they are not part of the
/// undo history.
#[derive(Clone, Debug, Default)]
pub struct BakeMeshes {
    pub(crate) low_base_color: OnceLock<std::result::Result<super::MaterialColors, String>>,
    pub high: Vec<BakeMesh>,
    pub cage: Option<BakeMesh>,
    /// Every high poly appended into one mesh (rebuilt when the list changes).
    pub(crate) merged: Option<MergedHigh>,
}

impl BakeMeshes {
    /// Every high poly appended into one mesh, as bakes see it
    pub fn merged_high(&self) -> Option<&MergedHigh> {
        self.merged.as_ref()
    }

    pub(crate) fn rebuild(&mut self) {
        self.merged = (!self.high.is_empty()).then(|| {
            let (mut mesh, mut base_color, mut file_ids) = (Mesh::default(), Vec::new(), Vec::new());
            for (file_index, source) in self.high.iter().enumerate() {
                file_ids.extend(std::iter::repeat_n(file_index as u32, source.mesh.submeshes.len()));
                mesh.append(&source.mesh);
                base_color.extend(source.base_color.iter().cloned().chain(std::iter::repeat(None)).take(source.mesh.materials.len()));
            }
            let bvh = Bvh::new(&mesh);
            MergedHigh { file_ids: Arc::new(file_ids), mesh: Arc::new(mesh), bvh: Arc::new(bvh), base_color: Arc::new(base_color) }
        });
    }
}

use super::MergedHigh;
use std::sync::Arc;
use texturecraft_bake::{BakeError, BakePaintMap, BakeRequest, CageShell, HighPoly, HighPolyReach};
use texturecraft_core::BakeSettings;
use texturecraft_geom::{Bvh, Mesh};

/// What a texture set's bake reads to place its transfer rays, owned so a worker thread can use it
#[derive(Clone, Debug)]
pub struct BakeCageInputs {
    pub(super) mesh: Arc<Mesh>,
    pub(super) bvh: Arc<Bvh>,
    pub(super) material: usize,
    pub(super) settings: BakeSettings,
    pub(super) high: MergedHigh,
    pub(super) cage: Option<Arc<Mesh>>,
    pub(super) skew: Option<Arc<BakePaintMap>>,
    pub(super) cage_paint: Option<Arc<BakePaintMap>>,
}

impl BakeCageInputs {
    /// The cage the 3D view draws: the bake's ray origins at each triangle corner
    pub fn cage_shell(&self) -> Result<CageShell, BakeError> {
        CageShell::new(&self.request())
    }

    /// Where each high-poly vertex sits relative to the bake's transfer rays
    pub fn high_poly_reach(&self) -> Result<HighPolyReach, BakeError> {
        HighPolyReach::new(&self.request())
    }

    fn request(&self) -> BakeRequest<'_> {
        BakeRequest {
            low_base_color: &[],
            mesh: &self.mesh,
            bvh: &self.bvh,
            material: self.material,
            settings: &self.settings,
            high: Some(HighPoly { file_ids: &self.high.file_ids, mesh: &self.high.mesh, bvh: &self.high.bvh, base_color: &self.high.base_color }),
            cage: self.cage.as_deref(),
            skew: self.skew.as_deref(),
            cage_paint: self.cage_paint.as_deref(),
            tiles: (1, 1),
        }
    }
}

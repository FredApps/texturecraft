//! Smart mask previews share one baked [`primitives::preview_orb`]
//! Presets use `mask.smart.apply` and the compositor, matching document masks

use serde_json::json;
use texturecraft_core::{BakeResolution, BakedMap, CurvatureMethod, LayerId};
use texturecraft_geom::glam::Vec3;
use texturecraft_geom::primitives;

use crate::mask_preview_camera::MaskPreviewCamera;
pub use crate::mask_preview_pixel::MaskPreviewPixel;
use crate::{EngineError, Result, Session};

/// Bake resolution in texels; 256 loses the thin edge lines
const TEXTURE_PX: u32 = 512;
/// Camera rays per preview pixel along each axis (anti-aliased outline)
const SUPERSAMPLE: usize = 2;
/// Bound camera samples and output allocations for shelf previews
pub const MAX_PREVIEW_SIZE_PX: usize = 512;
/// AO and thickness rays per texel; thumbnail detail needs fewer than the default
const OCCLUSION_RAYS: u32 = 12;
/// Curvature rays per texel; thumbnail detail needs fewer than the default
const CURVATURE_RAYS: u32 = 12;
/// Radius relative to the bounding box; isolates panel edges from the round body
const CURVATURE_RADIUS: f32 = 0.005;
/// The mesh maps the smart mask generators read
const MAPS: [BakedMap; 3] = [BakedMap::Curvature, BakedMap::AmbientOcclusion, BakedMap::Thickness];
/// Keep camera views for the thumbnail and hover preview sizes
const CAMERAS_KEPT: usize = 2;

/// The baked preview shape and, per preview size, what its camera sees
pub struct MaskPreview {
    session: Session,
    layer: LayerId,
    /// Cached camera views, least recently used first
    cameras: Vec<MaskPreviewCamera>,
}

impl MaskPreview {
    /// Bake the preview shape once (previews of any size reuse it)
    pub fn new() -> Result<MaskPreview> {
        let mut session = Session::new_project("Mask preview", primitives::preview_orb(), "builtin:preview_orb", None, TEXTURE_PX)?;
        let set = session.doc.project.texture_set_mut(0)?;
        set.bake.common.resolution = BakeResolution::from_pixels(TEXTURE_PX).ok_or_else(|| EngineError::Other("invalid bake resolution".into()))?;
        set.bake.high_poly.use_low_as_high = true;
        set.bake.maps.curvature.parameters.method = CurvatureMethod::Mesh;
        set.bake.maps.curvature.parameters.mesh.sampling_radius = CURVATURE_RADIUS;
        set.bake.maps.curvature.parameters.mesh.rays = CURVATURE_RAYS;
        set.bake.maps.ao.parameters.rays.rays = OCCLUSION_RAYS;
        set.bake.maps.thickness.parameters.rays.rays = OCCLUSION_RAYS;
        for map in BakedMap::ALL {
            set.bake.maps.set_enabled(map, MAPS.contains(&map));
        }
        let layer = *set.stack.first().ok_or_else(|| EngineError::Other("the preview has no layer".into()))?;
        session.bake_now(0, false)?;

        Ok(MaskPreview { session, layer, cameras: Vec::new() })
    }

    /// Row-major `size²` pixels, with None outside the shape
    /// Returns None for unknown masks or sizes outside 1..=[`MAX_PREVIEW_SIZE_PX`]
    pub fn render(&mut self, id: &str, size: usize) -> Option<Vec<Option<MaskPreviewPixel>>> {
        if !(1..=MAX_PREVIEW_SIZE_PX).contains(&size) {
            return None;
        }

        crate::execute(&mut self.session, "mask.smart.apply", &json!({ "name": id, "layer": self.layer.0 })).ok()?;
        // Previews need no undo history; retaining snapshots grows memory across presets
        self.session.undo.clear();

        let camera = camera_for(&mut self.cameras, &self.session, size)?;
        let values = self.session.mask_at(self.layer, &camera.texels)?;

        let sub_pixels = (SUPERSAMPLE * SUPERSAMPLE) as f32;
        let pixels = (0..size * size)
            .map(|i| {
                let (x, y) = (i % size, i / size);
                let (mut mask, mut normal, mut hits) = (0.0, Vec3::ZERO, 0.0);
                for sy in 0..SUPERSAMPLE {
                    for sx in 0..SUPERSAMPLE {
                        let ray = (y * SUPERSAMPLE + sy) * camera.rays_per_side + x * SUPERSAMPLE + sx;
                        let Some(&Some((texel, n))) = camera.samples.get(ray) else { continue };

                        mask += values.get(texel).copied().unwrap_or(0.0);
                        normal += n;
                        hits += 1.0;
                    }
                }
                (hits > 0.0).then(|| MaskPreviewPixel { mask: mask / hits, normal: normal.normalize_or_zero(), coverage: hits / sub_pixels })
            })
            .collect();
        Some(pixels)
    }
}

/// Cache one camera per preview size, evicting the least recently used view
fn camera_for<'a>(cameras: &'a mut Vec<MaskPreviewCamera>, session: &Session, size: usize) -> Option<&'a MaskPreviewCamera> {
    let rays_per_side = size * SUPERSAMPLE;
    let camera = match cameras.iter().position(|c| c.rays_per_side == rays_per_side) {
        Some(i) => cameras.remove(i),
        None => MaskPreviewCamera::new(session, TEXTURE_PX, rays_per_side),
    };
    if cameras.len() >= CAMERAS_KEPT {
        cameras.remove(0);
    }

    cameras.push(camera);
    cameras.last()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mean absolute difference of the masks of two previews over the pixels both see
    fn difference(a: &[Option<MaskPreviewPixel>], b: &[Option<MaskPreviewPixel>]) -> f32 {
        let pairs: Vec<(f32, f32)> = a.iter().zip(b).filter_map(|(a, b)| Some((a.as_ref()?.mask, b.as_ref()?.mask))).collect();
        pairs.iter().map(|(a, b)| (a - b).abs()).sum::<f32>() / pairs.len().max(1) as f32
    }

    #[test]
    fn invalid_preview_sizes_are_rejected_before_changing_the_shape() {
        let session = Session::new_project("Mask preview", primitives::cube(2.0), "builtin:cube", None, 128).unwrap();
        let layer = session.doc.project.texture_set(0).unwrap().stack[0];
        let mut preview = MaskPreview { session, layer, cameras: Vec::new() };
        for size in [0, MAX_PREVIEW_SIZE_PX + 1, usize::MAX / 2 + 1, usize::MAX] {
            assert!(preview.render("edges_sharp", size).is_none(), "size {size} should be rejected before rendering");
            assert!(preview.cameras.is_empty(), "invalid sizes must not allocate a camera");
            assert!(preview.session.doc.project.layer(layer).unwrap().mask.is_none(), "invalid sizes must not apply a mask");
        }
    }

    #[test]
    fn smart_mask_previews_are_in_range_tell_edges_from_cavities_and_render_at_any_size() {
        let size = 24;
        let mut preview = MaskPreview::new().unwrap();
        for sm in crate::smart_masks::SMART_MASKS {
            let px = preview.render(sm.id, size).unwrap_or_else(|| panic!("{}", sm.id));
            assert_eq!(px.len(), size * size, "{}", sm.id);
            assert!(px[0].is_none(), "{}: the corners are outside the shape", sm.id);
            let seen: Vec<&MaskPreviewPixel> = px.iter().flatten().collect();
            assert!(seen.len() > size * size / 3, "{}: the shape fills the frame", sm.id);
            for p in seen {
                assert!((0.0..=1.0).contains(&p.mask) && (0.0..=1.0).contains(&p.coverage) && p.coverage > 0.0, "{}: {p:?}", sm.id);
                assert!(p.normal.is_finite(), "{}: {p:?}", sm.id);
            }
        }

        let (edges, cavities) = (preview.render("edges_sharp", size).unwrap(), preview.render("dirt_cavities", size).unwrap());
        let shows = |px: &[Option<MaskPreviewPixel>]| px.iter().flatten().any(|p| p.mask > 0.5);
        assert!(shows(&edges) && shows(&cavities), "both masks find something on the shape");
        assert!(difference(&edges, &cavities) > 0.05, "edges and cavities differ: {}", difference(&edges, &cavities));
        assert!(preview.render("nope", size).is_none());

        let big = preview.render("edges_sharp", size * 3).unwrap();
        let mean = |px: &[Option<MaskPreviewPixel>]| px.iter().flatten().map(|p| p.mask).sum::<f32>() / px.iter().flatten().count().max(1) as f32;
        assert_eq!(big.len(), size * size * 9);
        assert!((mean(&big) - mean(&edges)).abs() < 0.05, "{} vs {}", mean(&big), mean(&edges));
        preview.render("edges_sharp", size * 2).unwrap();
        assert_eq!(preview.cameras.len(), CAMERAS_KEPT, "only the most recent sizes keep their camera");
    }
}

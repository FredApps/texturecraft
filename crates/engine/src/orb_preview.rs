//! Smart mask and smart material previews share one baked [`primitives::preview_orb`]
//! Presets use `mask.smart.apply`, materials use [`materials::apply`], both read back from the compositor

use std::collections::{BTreeMap, BTreeSet};

use serde_json::json;
use texturecraft_core::{BakeResolution, BakedMap, Channel, CurvatureMethod, LayerId};
use texturecraft_geom::glam::Vec3;
use texturecraft_geom::primitives;
use texturecraft_texel::TILE;

pub use crate::mask_preview_pixel::MaskPreviewPixel;
pub use crate::material_preview_pixel::MaterialPreviewPixel;
use crate::orb_camera::OrbCamera;
use crate::preview::Sample;
use crate::session::Document;
use crate::{EngineError, Result, Session, materials};

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
/// The mesh maps the smart mask generators read; smart materials build on the same presets
const MAPS: [BakedMap; 3] = [BakedMap::Curvature, BakedMap::AmbientOcclusion, BakedMap::Thickness];
/// Keep camera views for the thumbnail and hover preview sizes
const CAMERAS_KEPT: usize = 2;
/// The channels a material preview shows, in [`Sample`] order
const MATERIAL_CHANNELS: [Channel; 4] = [Channel::BaseColor, Channel::Metallic, Channel::Roughness, Channel::Height];
/// The orb's only texture set
const SET: usize = 0;

/// The baked orb and, per preview size, what its camera sees
pub struct OrbPreview {
    session: Session,
    /// The layer smart masks are applied to
    layer: LayerId,
    /// The baked document and selection, restored before each material
    bare: (Document, BTreeMap<usize, LayerId>),
    /// Cached camera views, least recently used first
    cameras: Vec<OrbCamera>,
}

impl OrbPreview {
    /// Bake the orb once (previews of any size reuse it)
    pub fn new() -> Result<OrbPreview> {
        let mut session = Session::new_project("Orb preview", primitives::preview_orb(), "builtin:preview_orb", None, TEXTURE_PX)?;
        let set = session.doc.project.texture_set_mut(SET)?;
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
        session.bake_now(SET, false)?;

        let bare = (session.doc.clone(), session.selected.clone());
        Ok(OrbPreview { session, layer, bare, cameras: Vec::new() })
    }

    /// Row-major `size²` pixels of smart mask `id`, with None outside the shape
    /// Returns None for unknown masks or sizes outside 1..=[`MAX_PREVIEW_SIZE_PX`]
    pub fn mask(&mut self, id: &str, size: usize) -> Option<Vec<Option<MaskPreviewPixel>>> {
        if !valid_size(size) {
            return None;
        }

        crate::execute(&mut self.session, "mask.smart.apply", &json!({ "name": id, "layer": self.layer.0 })).ok()?;
        // Previews need no undo history; retaining snapshots grows memory across presets
        self.session.undo.clear();

        let camera = camera_for(&mut self.cameras, &self.session, size)?;
        let values = self.session.mask_at(self.layer, &camera.texels)?;

        let sub_pixels = (SUPERSAMPLE * SUPERSAMPLE) as f32;
        let pixels = (0..size * size)
            .map(|pixel| {
                let (mut mask_sum, mut normal_sum, mut hits) = (0.0, Vec3::ZERO, 0.0);
                for (texel, normal) in pixel_hits(camera, size, pixel) {
                    mask_sum += values.get(texel).copied().unwrap_or(0.0);
                    normal_sum += normal;
                    hits += 1.0;
                }

                (hits > 0.0).then(|| MaskPreviewPixel { mask: mask_sum / hits, normal: normal_sum.normalize_or_zero(), coverage: hits / sub_pixels })
            })
            .collect();
        Some(pixels)
    }

    /// Row-major `size²` pixels of built-in material `id`, with None outside the shape
    /// Returns None for unknown materials or sizes outside 1..=[`MAX_PREVIEW_SIZE_PX`]
    pub fn material(&mut self, id: &str, size: usize) -> Option<Vec<Option<MaterialPreviewPixel>>> {
        if !valid_size(size) {
            return None;
        }

        // Each material goes on the bare shape, not over the previous one
        (self.session.doc, self.session.selected) = self.bare.clone();
        let applied = materials::apply(&mut self.session, id);
        // Previews need no undo history; retaining snapshots grows memory across materials
        self.session.undo.clear();
        applied.ok()?;

        let camera = camera_for(&mut self.cameras, &self.session, size)?;
        let values = material_at(&self.session, &camera.texels)?;

        let sub_pixels = (SUPERSAMPLE * SUPERSAMPLE) as f32;
        let pixels = (0..size * size)
            .map(|pixel| {
                let mut sum = Sample { base: [0.0; 3], metallic: 0.0, roughness: 0.0, height: 0.0 };
                let (mut normal_sum, mut hits) = (Vec3::ZERO, 0.0);
                for (texel, normal) in pixel_hits(camera, size, pixel) {
                    let Some(value) = values.get(texel) else { continue };

                    sum.base = [0, 1, 2].map(|component| sum.base[component] + value.base[component]);
                    sum.metallic += value.metallic;
                    sum.roughness += value.roughness;
                    sum.height += value.height;
                    normal_sum += normal;
                    hits += 1.0;
                }

                (hits > 0.0).then(|| {
                    let mean = Sample {
                        base: sum.base.map(|component| component / hits),
                        metallic: sum.metallic / hits,
                        roughness: sum.roughness / hits,
                        height: sum.height / hits,
                    };
                    MaterialPreviewPixel { sample: mean, normal: normal_sum.normalize_or_zero(), coverage: hits / sub_pixels }
                })
            })
            .collect();
        Some(pixels)
    }
}

/// Preview sizes the camera may allocate
fn valid_size(size: usize) -> bool {
    (1..=MAX_PREVIEW_SIZE_PX).contains(&size)
}

/// The rays through `pixel` of a `size²` preview that hit the shape, as (texel index, view-space normal)
fn pixel_hits(camera: &OrbCamera, size: usize, pixel: usize) -> impl Iterator<Item = (usize, Vec3)> + '_ {
    let (x, y) = (pixel % size, pixel / size);
    (0..SUPERSAMPLE * SUPERSAMPLE).filter_map(move |sub_pixel| {
        let (sub_x, sub_y) = (sub_pixel % SUPERSAMPLE, sub_pixel / SUPERSAMPLE);
        let ray = (y * SUPERSAMPLE + sub_y) * camera.rays_per_side + x * SUPERSAMPLE + sub_x;
        camera.samples.get(ray).copied().flatten()
    })
}

/// The material values at `texels`, compositing only the tiles that hold them
fn material_at(session: &Session, texels: &[(u32, u32)]) -> Option<Vec<Sample>> {
    let tiles: BTreeSet<(u32, u32)> = texels.iter().map(|(x, y)| (x / TILE, y / TILE)).collect();
    let tiles: Vec<(u32, u32)> = tiles.into_iter().collect();

    session.with_inputs(SET, |inputs| {
        let mut composite = texturecraft_compose::Composite::new(&MATERIAL_CHANNELS, TEXTURE_PX);
        texturecraft_compose::composite_tiles(inputs, &mut composite, &tiles);

        let samples = texels
            .iter()
            .map(|&(x, y)| {
                let [base, metallic, roughness, height] = MATERIAL_CHANNELS.map(|channel| composite.value(channel, x, y));
                Sample { base, metallic: metallic[0], roughness: roughness[0], height: height[0] }
            })
            .collect();
        Some(samples)
    })
}

/// Cache one camera per preview size, evicting the least recently used view
fn camera_for<'a>(cameras: &'a mut Vec<OrbCamera>, session: &Session, size: usize) -> Option<&'a OrbCamera> {
    let rays_per_side = size * SUPERSAMPLE;
    let camera = match cameras.iter().position(|camera| camera.rays_per_side == rays_per_side) {
        Some(index) => cameras.remove(index),
        None => OrbCamera::new(session, TEXTURE_PX, rays_per_side),
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

    /// Mean mask value over the pixels a preview sees
    fn mean_mask(pixels: &[Option<MaskPreviewPixel>]) -> f32 {
        pixels.iter().flatten().map(|pixel| pixel.mask).sum::<f32>() / pixels.iter().flatten().count().max(1) as f32
    }

    #[test]
    fn invalid_preview_sizes_are_rejected_before_changing_the_shape() {
        let session = Session::new_project("Orb preview", primitives::cube(2.0), "builtin:cube", None, 128).unwrap();
        let layer = session.doc.project.texture_set(SET).unwrap().stack[0];
        let bare = (session.doc.clone(), session.selected.clone());
        let mut preview = OrbPreview { session, layer, bare, cameras: Vec::new() };
        let stack_size = preview.session.doc.project.texture_set(SET).unwrap().stack.len();
        for size in [0, MAX_PREVIEW_SIZE_PX + 1, usize::MAX / 2 + 1, usize::MAX] {
            assert!(preview.mask("edges_sharp", size).is_none(), "size {size} should be rejected before rendering");
            assert!(preview.material("steel_painted_worn", size).is_none(), "size {size} should be rejected before rendering");
            assert!(preview.cameras.is_empty(), "invalid sizes must not allocate a camera");
            assert!(preview.session.doc.project.layer(layer).unwrap().mask.is_none(), "invalid sizes must not apply a mask");
            assert_eq!(preview.session.doc.project.texture_set(SET).unwrap().stack.len(), stack_size, "invalid sizes must not apply a material");
        }
    }

    #[test]
    fn smart_mask_previews_are_in_range_tell_edges_from_cavities_and_render_at_any_size() {
        let size = 24;
        let mut preview = OrbPreview::new().unwrap();
        for smart_mask in crate::smart_masks::SMART_MASKS {
            let id = smart_mask.id;
            let pixels = preview.mask(id, size).unwrap_or_else(|| panic!("{id}"));
            assert_eq!(pixels.len(), size * size, "{id}");
            assert!(pixels[0].is_none(), "{id}: the corners are outside the shape");

            let seen: Vec<&MaskPreviewPixel> = pixels.iter().flatten().collect();
            assert!(seen.len() > size * size / 3, "{id}: the shape fills the frame");
            for pixel in seen {
                assert!((0.0..=1.0).contains(&pixel.mask) && (0.0..=1.0).contains(&pixel.coverage) && pixel.coverage > 0.0, "{id}: {pixel:?}");
                assert!(pixel.normal.is_finite(), "{id}: {pixel:?}");
            }
        }

        let (edges, cavities) = (preview.mask("edges_sharp", size).unwrap(), preview.mask("dirt_cavities", size).unwrap());
        let shows = |pixels: &[Option<MaskPreviewPixel>]| pixels.iter().flatten().any(|pixel| pixel.mask > 0.5);
        assert!(shows(&edges) && shows(&cavities), "both masks find something on the shape");
        assert!(difference(&edges, &cavities) > 0.05, "edges and cavities differ: {}", difference(&edges, &cavities));
        assert!(preview.mask("nope", size).is_none());

        // The same bake renders other sizes, showing the same mask
        let big = preview.mask("edges_sharp", size * 3).unwrap();
        assert_eq!(big.len(), size * size * 9);
        assert!((mean_mask(&big) - mean_mask(&edges)).abs() < 0.05, "{} vs {}", mean_mask(&big), mean_mask(&edges));

        preview.mask("edges_sharp", size * 2).unwrap();
        assert_eq!(preview.cameras.len(), CAMERAS_KEPT, "only the most recent sizes keep their camera");
    }

    #[test]
    fn smart_materials_render_on_the_orb_and_show_their_wear() {
        let size = 24;
        let mut preview = OrbPreview::new().unwrap();
        for material in materials::MATERIALS.iter().filter(|material| material.smart) {
            let id = material.id;
            let pixels = preview.material(id, size).unwrap_or_else(|| panic!("{id}"));
            assert_eq!(pixels.len(), size * size, "{id}");
            assert!(pixels[0].is_none(), "{id}: the corners are outside the shape");

            for pixel in pixels.iter().flatten() {
                let sample = pixel.sample;
                let values = sample.base.into_iter().chain([sample.metallic, sample.roughness, sample.height]);
                assert!(values.into_iter().all(f32::is_finite), "{id}: {pixel:?}");
                assert!(pixel.normal.is_finite() && pixel.coverage > 0.0, "{id}: {pixel:?}");
            }
        }
        assert!(preview.material("nope", size).is_none());

        // Paint chipped at the edges shows the steel under it: the smart material is not its base
        let (worn, steel) = (preview.material("steel_painted_worn", size).unwrap(), preview.material("steel", size).unwrap());
        let metallic_pixels = |pixels: &[Option<MaterialPreviewPixel>]| pixels.iter().flatten().filter(|pixel| pixel.sample.metallic > 0.5).count();
        let (worn_metallic, steel_metallic) = (metallic_pixels(&worn), metallic_pixels(&steel));
        assert!(worn_metallic > 0 && worn_metallic < steel_metallic / 2, "bare steel only at the worn edges: {worn_metallic} of {steel_metallic}");

        let base_colors = |pixels: &[Option<MaterialPreviewPixel>]| pixels.iter().flatten().map(|pixel| pixel.sample.base).collect::<Vec<_>>();
        assert_ne!(base_colors(&worn), base_colors(&steel));

        // Each material starts from the bare orb: rendering another one in between changes nothing
        preview.material("iron_rusted", size).unwrap();
        assert_eq!(preview.material("steel_painted_worn", size).unwrap(), worn);
    }
}

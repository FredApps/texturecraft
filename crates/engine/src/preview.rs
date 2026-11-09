//! Cheap shelf previews on a unit sphere: plain materials and library filters (smart ones use [`crate::orb_preview`])

use rayon::prelude::*;
use texturecraft_compose::pattern::{Coords, procedural};
use texturecraft_core::{Channel, Effect, EffectKind, Fill, Filter, Layer, LayerKind, Procedural, ProceduralKind, Project, Projection, Source, TextureSet};
use texturecraft_geom::glam::{Vec2, Vec3};

use crate::materials::{Builder, build};

/// Linear material values of one preview pixel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub base: [f32; 3],
    pub metallic: f32,
    pub roughness: f32,
    pub height: f32,
}

/// The view-space normal (= position on the unit sphere) of each pixel of a `size²` image,
/// row-major, None outside the sphere.
pub fn sphere_normals(size: usize) -> impl Iterator<Item = Option<Vec3>> {
    let r = (size as f32 * 0.5 - 1.0).max(0.5);
    (0..size * size).map(move |i| {
        let (x, y) = ((i % size) as f32 + 0.5 - size as f32 * 0.5, size as f32 * 0.5 - (i / size) as f32 - 0.5);
        let (dx, dy) = (x / r, y / r);
        let d2 = dx * dx + dy * dy;
        (d2 <= 1.0).then(|| Vec3::new(dx, dy, (1.0 - d2).max(0.0).sqrt()))
    })
}

fn coords(n: Vec3) -> Coords {
    let uv = Vec2::new(n.z.atan2(n.x) / std::f32::consts::TAU + 0.5, n.y.clamp(-1.0, 1.0).acos() / std::f32::consts::PI);
    Coords { uv, pos: n, nrm: n }
}

fn source(s: &Source, proj: &Projection, c: &Coords) -> Option<[f32; 3]> {
    match s {
        Source::Uniform(v) => Some(*v),
        Source::Procedural { pattern, low, high } => {
            let t = procedural(pattern, proj, c);
            Some([0, 1, 2].map(|k| low[k] + (high[k] - low[k]) * t))
        }
        _ => None,
    }
}

const CHANNELS: [Channel; 4] = [Channel::BaseColor, Channel::Metallic, Channel::Roughness, Channel::Height];

/// Preview of plain material `id` (None for an unknown id or a smart material, whose masks need mesh maps)
pub fn material(id: &str, size: usize) -> Option<Vec<Option<Sample>>> {
    let plain = crate::materials::find(id).filter(|found| !found.smart)?;
    let mut project = Project::new("preview", "");
    let layers = build(&mut Builder { p: &mut project, channels: CHANNELS.to_vec() }, plain.id).ok()?;

    // Pixels evaluate independently and big previews take long on one core
    let normals: Vec<_> = sphere_normals(size).collect();
    let samples = normals
        .into_par_iter()
        .map(|normal| {
            let at = coords(normal?);
            let mut values = CHANNELS.map(Channel::default_value);
            for layer in layers.iter().filter(|layer| layer.visible) {
                // Plain materials are unmasked fills
                let LayerKind::Fill(fill) = &layer.kind else { continue };

                for (value, channel) in values.iter_mut().zip(CHANNELS) {
                    let layer_value = fill.sources.get(&channel).and_then(|channel_source| source(channel_source, &fill.projection, &at));
                    if let (Some(blend), Some(layer_value)) = (layer.blend(channel), layer_value) {
                        let weight = blend.opacity.clamp(0.0, 1.0);
                        *value = [0, 1, 2].map(|component| value[component] + (layer_value[component] - value[component]) * weight);
                    }
                }
            }

            let [base, metallic, roughness, height] = values;
            Some(Sample { base, metallic: metallic[0], roughness: roughness[0], height: height[0] })
        })
        .collect();

    Some(samples)
}

/// `f` with its reach in texels multiplied by `k` (previews show at a fraction of 2K).
fn magnified(f: Filter, k: f32) -> Filter {
    match f {
        Filter::Blur { radius } => Filter::Blur { radius: radius * k },
        Filter::DirectionalBlur { radius, angle } => Filter::DirectionalBlur { radius: radius * k, angle },
        Filter::SlopeBlur { radius, scale, seed } => Filter::SlopeBlur { radius: radius * k, scale, seed },
        Filter::Warp { intensity, scale, seed } => Filter::Warp { intensity: intensity * k, scale, seed },
        Filter::HighPass { radius } => Filter::HighPass { radius: radius * k },
        Filter::EdgeDetect { width } => Filter::EdgeDetect { width: width * k },
        Filter::Bevel { width } => Filter::Bevel { width: width * k },
        Filter::Mosaic { size } => Filter::Mosaic { size: size * k },
        Filter::Glow { radius, amount } => Filter::Glow { radius: radius * k, amount },
        Filter::DirectionalWarp { intensity, angle, scale, seed } => Filter::DirectionalWarp { intensity: intensity * k, angle, scale, seed },
        f => f,
    }
}

/// Preview of library filter `id` (see [`crate::filter_lib`]): a sample material (colorful
/// clouds, checker roughness, cell bumps) with the filter on its content, composited by the
/// compositor on a plane and laid over the sphere (None for an unknown id).
pub fn filter(id: &str, size: usize) -> Option<Vec<Option<Sample>>> {
    let res = u32::try_from(size).ok()?.max(1);
    let kind = match crate::filter_lib::build(id, &serde_json::Value::Null).ok()? {
        EffectKind::Filter(f) => EffectKind::Filter(magnified(f, 2048.0 / res as f32)),
        k => k,
    };
    let mesh = texturecraft_geom::primitives::plane(2.0, 2);
    let gbuf = texturecraft_geom::UvGBuffer::new(&mesh, 0, res, res);
    let mut p = Project::new("preview", "");
    p.texture_sets.push(TextureSet::new("preview", 0, res));
    let procedural = |kind, scale, low: [f32; 3], high: [f32; 3]| Source::Procedural { pattern: Procedural { scale, ..Procedural::new(kind) }, low, high };
    let mut fill = Fill::default();
    fill.sources.insert(Channel::BaseColor, procedural(ProceduralKind::Clouds, 2.0, [0.05, 0.2, 0.55], [0.95, 0.55, 0.1]));
    fill.sources.insert(Channel::Roughness, procedural(ProceduralKind::Checker, 4.0, [0.25; 3], [0.7; 3]));
    fill.sources.insert(Channel::Height, procedural(ProceduralKind::Cells, 3.0, [-0.12; 3], [0.12; 3]));
    let mut layer = Layer::new(p.alloc_id(), "sample", LayerKind::Fill(fill), &CHANNELS);
    layer.content_effects.push(Effect::new(p.alloc_id(), kind));
    p.insert_layer(0, layer, None, false).ok()?;
    let (pixels, baked, images) = (Default::default(), Default::default(), Default::default());
    let inp = texturecraft_compose::Inputs {
        project: &p,
        ts: 0,
        mesh: &mesh,
        gbuf: &gbuf,
        pixels: &pixels,
        baked: &baked,
        block_scales: p.texture_set(0).map(|set| set.block_scales()).unwrap_or_default(),
        preview: None,
        images: &images,
        bounds: mesh.bounds(),
    };
    let mut out = texturecraft_compose::Composite::new(&CHANNELS, res);
    texturecraft_compose::composite_tiles(&inp, &mut out, &texturecraft_compose::all_tiles(res, res));
    Some(
        sphere_normals(size)
            .enumerate()
            .map(|(i, n)| {
                n?;
                let (x, y) = ((i % size) as u32, (i / size) as u32);
                let [base, metal, rough, height] = CHANNELS.map(|c| out.value(c, x, y));
                Some(Sample { base, metallic: metal[0], roughness: rough[0], height: height[0] })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether every value of `sample` is finite
    fn finite(sample: &Sample) -> bool {
        sample.base.into_iter().chain([sample.metallic, sample.roughness, sample.height]).all(f32::is_finite)
    }

    #[test]
    fn previews_cover_the_sphere_with_finite_values() {
        let size = 16;
        for built_in in crate::materials::MATERIALS {
            let id = built_in.id;
            if built_in.smart {
                assert!(material(id, size).is_none(), "{id}: smart materials preview on the orb");
                continue;
            }

            let mut project = Project::new("preview", "");
            let layers = build(&mut Builder { p: &mut project, channels: CHANNELS.to_vec() }, id).unwrap();
            assert!(layers.iter().all(|layer| layer.mask.is_none()), "{id}: the sphere preview shows no masks");

            let pixels = material(id, size).unwrap_or_else(|| panic!("{id}"));
            let center = pixels[size / 2 * size + size / 2].unwrap_or_else(|| panic!("{id}"));
            assert!(finite(&center), "{id}");
            assert!(pixels[0].is_none(), "corners are outside the sphere");
        }
        assert!(material("nope", size).is_none() && filter("nope", size).is_none());

        let size = 24;
        for library_filter in crate::filter_lib::FILTERS {
            let id = library_filter.id;
            let pixels = filter(id, size).unwrap_or_else(|| panic!("{id}"));
            let center = pixels[size / 2 * size + size / 2].unwrap_or_else(|| panic!("{id}"));
            assert!(finite(&center), "{id}");
        }
        let (plain, warped) = (filter("levels", size).unwrap(), filter("warp", size).unwrap());
        assert!(plain != warped, "the warp shows in its preview");
    }
}

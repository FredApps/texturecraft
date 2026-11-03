//! Shelf previews: built-in materials and smart masks evaluated on a unit sphere seen from +Z,
//! without a mesh or baked maps (generators that need mesh maps are approximated with world-space
//! noise). Cheap enough for lazily rendered thumbnails.

use texturecraft_compose::pattern::{Coords, procedural, world_noise};
use texturecraft_core::{
    Channel, Effect, EffectKind, Fill, Filter, Generator, GeneratorKind, Layer, LayerKind, Mask, MaskBackground, Procedural, ProceduralKind, Project,
    Projection, Source, TextureSet, blend1,
};
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

/// Approximate generator: mesh-map signals become world noise (a sphere has no edges or cavities).
fn generator(g: &Generator, c: &Coords) -> f32 {
    let dir = Vec3::from(g.direction).try_normalize().unwrap_or(Vec3::Y);
    let wear = (world_noise(c.pos, 3.0, 91) - 0.5) * 2.2;
    let cavity = (world_noise(c.pos, 3.0, 57) - 0.3).clamp(0.0, 1.0) + (1.0 - c.nrm.y) * 0.1;
    let s = match g.kind {
        GeneratorKind::Position => c.pos.dot(dir) * 0.5 + 0.5,
        GeneratorKind::Direction => c.nrm.dot(dir) * 0.5 + 0.5,
        GeneratorKind::EdgeWear => wear,
        GeneratorKind::Curvature => 0.5 + wear * 0.5,
        GeneratorKind::AmbientOcclusion => 1.0 - cavity,
        _ => cavity,
    };
    // Same thresholding as the compositor.
    let s = if g.noise_amount > 0.0 { s + (world_noise(c.pos, g.noise_scale, g.seed) - 0.5) * g.noise_amount } else { s };
    g.shape(s)
}

fn mask(m: &Mask, c: &Coords) -> f32 {
    if !m.enabled {
        return 1.0;
    }
    let mut v = if m.background == MaskBackground::White { 1.0 } else { 0.0 };
    for e in m.effects.iter().filter(|e| e.visible) {
        let val = match &e.kind {
            EffectKind::Fill { source: s, projection } => source(s, projection, c).map(|x| x[0]),
            EffectKind::Generator(g) => Some(generator(g, c)),
            EffectKind::Levels(l) => Some(l.apply(v)),
            EffectKind::Filter(Filter::Invert) => Some(1.0 - v),
            _ => None,
        };
        if let Some(val) = val {
            let blended = if matches!(e.kind, EffectKind::Levels(_) | EffectKind::Filter(_)) { val } else { blend1(e.blend, v, val) };
            v += (blended - v) * e.opacity.clamp(0.0, 1.0);
        }
    }
    v.clamp(0.0, 1.0)
}

const CHANNELS: [Channel; 4] = [Channel::BaseColor, Channel::Metallic, Channel::Roughness, Channel::Height];

/// Preview of built-in material `id` (None for an unknown id).
pub fn material(id: &str, size: usize) -> Option<Vec<Option<Sample>>> {
    let mut p = Project::new("preview", "");
    let layers = build(&mut Builder { p: &mut p, channels: CHANNELS.to_vec() }, id).ok()?;
    Some(
        sphere_normals(size)
            .map(|n| {
                let c = coords(n?);
                let mut acc = CHANNELS.map(Channel::default_value);
                for l in layers.iter().filter(|l| l.visible) {
                    let LayerKind::Fill(f) = &l.kind else { continue };
                    let w = l.mask.as_ref().map_or(1.0, |m| mask(m, &c));
                    for (a, ch) in acc.iter_mut().zip(CHANNELS) {
                        if let (Some(b), Some(v)) = (l.blend(ch), f.sources.get(&ch).and_then(|s| source(s, &f.projection, &c))) {
                            let t = w * b.opacity.clamp(0.0, 1.0);
                            *a = [0, 1, 2].map(|k| a[k] + (v[k] - a[k]) * t);
                        }
                    }
                }
                let [base, metal, rough, height] = acc;
                Some(Sample { base, metallic: metal[0], roughness: rough[0], height: height[0] })
            })
            .collect(),
    )
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

/// Preview of smart mask `id`: the mask value per pixel (None for an unknown id).
pub fn smart_mask(id: &str, size: usize) -> Option<Vec<Option<f32>>> {
    let (bg, fx) = crate::smart_masks::preset(id)?;
    let m = crate::smart_masks::build_mask(&mut Project::new("preview", ""), bg, fx);
    Some(sphere_normals(size).map(|n| n.map(|n| mask(&m, &coords(n)))).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_cover_the_sphere_with_finite_values() {
        for m in crate::materials::MATERIALS {
            let px = material(m.id, 16).unwrap_or_else(|| panic!("{}", m.id));
            let center = px[8 * 16 + 8].unwrap_or_else(|| panic!("{}", m.id));
            assert!(center.base.iter().chain([&center.metallic, &center.roughness, &center.height]).all(|v| v.is_finite()), "{}", m.id);
            assert!(px[0].is_none(), "corners are outside the sphere");
        }
        for sm in crate::smart_masks::SMART_MASKS {
            let px = smart_mask(sm.id, 16).unwrap_or_else(|| panic!("{}", sm.id));
            assert!(px.iter().flatten().all(|v| (0.0..=1.0).contains(v)), "{}", sm.id);
        }
        assert!(material("nope", 8).is_none() && smart_mask("nope", 8).is_none() && filter("nope", 8).is_none());
        for f in crate::filter_lib::FILTERS {
            let px = filter(f.id, 24).unwrap_or_else(|| panic!("{}", f.id));
            let center = px[12 * 24 + 12].unwrap_or_else(|| panic!("{}", f.id));
            assert!(center.base.iter().chain([&center.roughness, &center.height]).all(|v| v.is_finite()), "{}", f.id);
        }
        let (plain, warped) = (filter("levels", 24).unwrap(), filter("warp", 24).unwrap());
        assert!(plain != warped, "the warp shows in its preview");
    }
}

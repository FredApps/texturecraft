//! The hot-layer cache (the stack below and the tape above it, through folders) and blocked
//! compositing give exactly the unoptimized result; instance layers draw their source; masks
//! combine with height.

use std::collections::BTreeMap;

use texturecraft_compose::{BelowCache, Composite, Inputs, PaintTarget, PixelStore, StrokePreview, all_tiles, composite_tiles, composite_tiles_cached};
use texturecraft_core::{
    BlendMode, Channel, Effect, EffectKind, Fill, Filter, Generator, GeneratorKind, HeightMask, Layer, LayerId, LayerKind, Mask, MaskBackground, Procedural,
    ProceduralKind, Project, Projection, Source, TextureSet,
};
use texturecraft_geom::{Mesh, UvGBuffer, primitives};
use texturecraft_texel::{TILE, TileImage, to_u16};

const RES: u32 = 256;

struct Fixture {
    project: Project,
    mesh: Mesh,
    gbuf: UvGBuffer,
    pixels: PixelStore,
}

fn fixture() -> Fixture {
    let mesh = primitives::sphere(1.0, 32, 16);
    let gbuf = UvGBuffer::new(&mesh, 0, RES, RES);
    let mut project = Project::new("t", "builtin:sphere");
    project.texture_sets.push(TextureSet::new("A", 0, RES));
    project.texture_sets.push(TextureSet::new("B", 0, RES));
    Fixture { project, mesh, gbuf, pixels: PixelStore::default() }
}

fn procedural(kind: ProceduralKind, low: f32, high: f32) -> Source {
    Source::Procedural { pattern: Procedural::new(kind), low: [low; 3], high: [high; 3] }
}

fn add(f: &mut Fixture, ts: usize, kind: LayerKind, mask: Option<Mask>) -> LayerId {
    put(f, ts, None, kind, mask)
}

/// A layer on top of `folder`'s children, or of set `ts`'s stack.
fn put(f: &mut Fixture, ts: usize, folder: Option<LayerId>, kind: LayerKind, mask: Option<Mask>) -> LayerId {
    let id = f.project.alloc_id();
    let mut l = Layer::new(id, "l", kind, &Channel::DEFAULT_SET);
    l.mask = mask;
    f.project.insert_layer(ts, l, folder, folder.is_some()).unwrap()
}

fn fill(sources: &[(Channel, Source)]) -> LayerKind {
    let mut f = Fill::default();
    f.sources = sources.iter().cloned().collect();
    LayerKind::Fill(f)
}

fn effect(f: &mut Fixture, kind: EffectKind) -> Effect {
    Effect::new(f.project.alloc_id(), kind)
}

/// Procedural fills, a generator mask with a blur, a folder, painted pixels; returns the top
/// paint layer.
fn heavy_stack(f: &mut Fixture) -> LayerId {
    add(
        f,
        0,
        fill(&[(Channel::BaseColor, procedural(ProceduralKind::Clouds, 0.1, 0.9)), (Channel::Height, procedural(ProceduralKind::Cells, -0.3, 0.3))]),
        None,
    );
    let mut mask = Mask::new(MaskBackground::Black);
    mask.effects.push(effect(f, EffectKind::Generator(Generator::new(GeneratorKind::Position))));
    mask.effects.push(effect(f, EffectKind::Filter(Filter::Blur { radius: 40.0 })));
    add(f, 0, fill(&[(Channel::Roughness, procedural(ProceduralKind::Grunge, 0.0, 1.0))]), Some(mask));
    let group = add(f, 0, LayerKind::Group { children: vec![], collapsed: false }, None);
    for b in f.project.layer_mut(group).unwrap().channels.values_mut() {
        b.mode = BlendMode::Passthrough;
    }
    let inner = f.project.alloc_id();
    let l = Layer::new(inner, "inner", fill(&[(Channel::Metallic, procedural(ProceduralKind::Scratches, 0.0, 1.0))]), &Channel::DEFAULT_SET);
    f.project.insert_layer(0, l, Some(group), true).unwrap();
    let mut blurred = Mask::new(MaskBackground::Black);
    blurred.effects.push(effect(f, EffectKind::Fill { source: procedural(ProceduralKind::Checker, 0.0, 1.0), projection: Projection::default() }));
    blurred.effects.push(effect(f, EffectKind::Filter(Filter::Blur { radius: 24.0 })));
    let top = add(f, 0, LayerKind::Paint, Some(blurred));
    let img = f.pixels.layer_mut(top, Channel::BaseColor, (RES, RES));
    for y in 40..200 {
        for x in 30..220 {
            img.set(x, y, &[to_u16(0.4), to_u16(0.1), 0, to_u16(0.5)]);
        }
    }
    top
}

fn inputs<'a>(f: &'a Fixture, ts: usize, images: &'a texturecraft_compose::ImageStore, baked: &'a texturecraft_compose::BakedMaps) -> Inputs<'a> {
    Inputs {
        project: &f.project,
        ts,
        mesh: &f.mesh,
        gbuf: &f.gbuf,
        pixels: &f.pixels,
        baked,
        block_scales: f.project.texture_sets[ts].block_scales(),
        preview: None,
        images,
        bounds: f.mesh.bounds(),
    }
}

fn run(f: &Fixture, ts: usize, tiles: &[(u32, u32)], below: Option<&mut BelowCache>) -> Composite {
    let (images, baked) = (Default::default(), Default::default());
    let mut out = Composite::new(&f.project.texture_sets[ts].channel_list(), RES);
    composite_tiles_cached(&inputs(f, ts, &images, &baked), &mut out, tiles, below);
    out
}

fn reference(f: &Fixture, ts: usize) -> Composite {
    let (images, baked) = (Default::default(), Default::default());
    let mut out = Composite::new(&f.project.texture_sets[ts].channel_list(), RES);
    composite_tiles(&inputs(f, ts, &images, &baked), &mut out, &all_tiles(RES, RES));
    out
}

#[test]
fn below_cache_and_blocks_match_the_full_composite_exactly() {
    let mut f = fixture();
    let top = heavy_stack(&mut f);
    let want = reference(&f, 0);
    // Tile by tile (each its own block and neighbourhood region) gives the same bits.
    let mut one_by_one = Composite::new(&f.project.texture_sets[0].channel_list(), RES);
    let (images, baked) = (Default::default(), Default::default());
    for t in all_tiles(RES, RES) {
        composite_tiles(&inputs(&f, 0, &images, &baked), &mut one_by_one, &[t]);
    }
    assert!(one_by_one == want, "per-tile composites differ from one pass");

    let mut cache = BelowCache::default();
    cache.set_hot(Some(top));
    let half: Vec<(u32, u32)> = all_tiles(RES, RES).into_iter().filter(|t| (t.0 + t.1) % 2 == 0).collect();
    // Cold, half warm and fully warm caches.
    let mut out = run(&f, 0, &half, Some(&mut cache));
    assert!(!cache.is_empty(), "the stack below the hot layer is cached");
    composite_tiles_cached(&inputs(&f, 0, &images, &baked), &mut out, &all_tiles(RES, RES), Some(&mut cache));
    assert!(out == want, "a partly cached composite differs");
    let warm = run(&f, 0, &all_tiles(RES, RES), Some(&mut cache));
    assert!(warm == want, "a cached composite differs");

    // Painting the hot layer keeps the cache; editing below it empties it.
    f.pixels.layer_mut(top, Channel::BaseColor, (RES, RES)).set(5, 5, &[to_u16(1.0), 0, 0, to_u16(1.0)]);
    let n = cache.len();
    assert!(run(&f, 0, &all_tiles(RES, RES), Some(&mut cache)) == reference(&f, 0));
    assert_eq!(cache.len(), n);
    let low = f.project.texture_sets[0].stack[0];
    if let LayerKind::Fill(fl) = &mut f.project.layer_mut(low).unwrap().kind {
        fl.sources.insert(Channel::Roughness, Source::Uniform([0.9; 3]));
    }
    assert!(run(&f, 0, &all_tiles(RES, RES), Some(&mut cache)) == reference(&f, 0), "a stale cache was used");
    // A hot layer inside a folder caches below the folder.
    let group = f.project.texture_sets[0].stack[2];
    let inner = f.project.layer(group).unwrap().children()[0];
    cache.set_hot(Some(inner));
    run(&f, 0, &all_tiles(RES, RES), Some(&mut cache));
    assert!(!cache.is_empty());
    assert!(run(&f, 0, &all_tiles(RES, RES), Some(&mut cache)) == reference(&f, 0));
}

#[test]
fn instances_draw_their_source_across_texture_sets() {
    let mut f = fixture();
    let top = heavy_stack(&mut f);
    // Set B instances every top-level layer of set A: same composite.
    for id in f.project.texture_sets[0].stack.clone() {
        add(&mut f, 1, LayerKind::Instance { source: id }, None);
    }
    let a = reference(&f, 0);
    assert!(reference(&f, 1).channels == a.channels, "instances draw like their sources");
    // An edit to the source shows in the instance.
    f.pixels.layer_mut(top, Channel::BaseColor, (RES, RES)).set(100, 100, &[to_u16(1.0), 0, 0, to_u16(1.0)]);
    let (a, b) = (reference(&f, 0), reference(&f, 1));
    assert!(b.channels == a.channels);
    // The instance's own mask overrides the source's.
    let inst = *f.project.texture_sets[1].stack.last().unwrap();
    f.project.layer_mut(inst).unwrap().mask = Some(Mask::new(MaskBackground::Black));
    let b = reference(&f, 1);
    let (x, y) = (100, 100);
    assert!((a.value(Channel::BaseColor, x, y)[0] - b.value(Channel::BaseColor, x, y)[0]).abs() > 0.1);
    // A hidden instance or a missing source draws nothing.
    let mut g = fixture();
    let src = add(&mut g, 0, fill(&[(Channel::Roughness, Source::Uniform([0.9; 3]))]), None);
    let inst = add(&mut g, 1, LayerKind::Instance { source: src }, None);
    assert!((reference(&g, 1).value(Channel::Roughness, 128, 128)[0] - 0.9).abs() < 1e-3);
    g.project.layer_mut(inst).unwrap().visible = false;
    assert!((reference(&g, 1).value(Channel::Roughness, 128, 128)[0] - 0.5).abs() < 1e-3);
    g.project.layer_mut(inst).unwrap().visible = true;
    g.project.layers.remove(&src);
    assert!((reference(&g, 1).value(Channel::Roughness, 128, 128)[0] - 0.5).abs() < 1e-3);
}

#[test]
fn mask_with_height_follows_the_layer_height() {
    let mut f = fixture();
    // A white layer over the default gray, half masked, its height falling from 0.25 (left) to
    // -0.25 (right).
    let mut ramp = Procedural::new(ProceduralKind::Gradient);
    ramp.scale = 1.0;
    let height = Source::Procedural { pattern: ramp, low: [0.25; 3], high: [-0.25; 3] };
    let id = add(&mut f, 0, fill(&[(Channel::BaseColor, Source::Uniform([1.0; 3])), (Channel::Height, height)]), None);
    let mut mask = Mask::new(MaskBackground::Black);
    mask.effects.push(effect(&mut f, EffectKind::Fill { source: Source::Uniform([0.5; 3]), projection: Projection::default() }));
    f.project.layer_mut(id).unwrap().mask = Some(mask);
    let plain = reference(&f, 0);
    f.project.layer_mut(id).unwrap().height_mask = HeightMask { enabled: true, contrast: 1.0, offset: 0.0 };
    let hm = reference(&f, 0);
    let row = TILE * 2;
    let (left, right) = (16, RES - 16);
    let v = |c: &Composite, x| c.value(Channel::BaseColor, x, row)[0];
    assert!((v(&plain, left) - 0.75).abs() < 0.01 && (v(&plain, right) - 0.75).abs() < 0.01, "plain mask: 50 % white over gray");
    assert!(v(&hm, left) > 0.84, "high parts show first: {}", v(&hm, left));
    assert!(v(&hm, right) < 0.66, "low parts later: {}", v(&hm, right));
    // Without a mask there is nothing to combine.
    f.project.layer_mut(id).unwrap().mask.as_mut().unwrap().enabled = false;
    assert!((v(&reference(&f, 0), left) - 1.0).abs() < 1e-3);
}

fn group(f: &mut Fixture, folder: Option<LayerId>, mode: BlendMode, opacity: f32, mask: Option<Mask>) -> LayerId {
    let id = put(f, 0, folder, LayerKind::Group { children: vec![], collapsed: false }, mask);
    blend(f, id, mode, opacity);
    id
}

fn blend(f: &mut Fixture, id: LayerId, mode: BlendMode, opacity: f32) {
    for b in f.project.layer_mut(id).unwrap().channels.values_mut() {
        b.mode = mode;
        b.opacity = opacity;
    }
}

/// A black mask revealed by a noisy generator, blurred when `blur` is set.
fn generator_mask(f: &mut Fixture, kind: GeneratorKind, blur: Option<f32>) -> Mask {
    let mut g = Generator::new(kind);
    g.noise_amount = 0.4;
    g.noise_scale = 3.0;
    let mut mask = Mask::new(MaskBackground::Black);
    mask.effects.push(effect(f, EffectKind::Generator(g)));
    if let Some(radius) = blur {
        mask.effects.push(effect(f, EffectKind::Filter(Filter::Blur { radius })));
    }
    mask
}

/// Paint a half-covered rectangle into a paint layer's base color and roughness.
fn paint_rect(f: &mut Fixture, id: LayerId, (x0, y0): (u32, u32), (w, h): (u32, u32), v: f32) {
    for (ch, px) in [(Channel::BaseColor, vec![to_u16(v * 0.5), to_u16(0.1), 0, to_u16(0.5)]), (Channel::Roughness, vec![to_u16(v * 0.5), to_u16(0.5)])] {
        let img = f.pixels.layer_mut(id, ch, (RES, RES));
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                img.set(x, y, &px);
            }
        }
    }
}

struct Nested {
    floor: LayerId,
    base: LayerId,
    g1: LayerId,
    inner: LayerId,
    g2: LayerId,
    deep: LayerId,
    g2_above: LayerId,
    g1_above: LayerId,
    top_fill: LayerId,
    top: LayerId,
}

/// Set 0, bottom to top: a painted layer; a procedural fill; a normal folder (generator mask, 80 %) holding a
/// fill, a pass-through folder (checker mask) holding [a fill with a blurred generator mask,
/// the `deep` paint layer, an overlay fill with a generator mask, an instance of the base fill]
/// and a screen fill with HSL and blur content effects; a multiply fill with a blurred
/// generator mask; a painted layer with an invert effect.
fn nested_stack(f: &mut Fixture) -> Nested {
    let floor = add(f, 0, LayerKind::Paint, None);
    paint_rect(f, floor, (0, 0), (256, 200), 0.6);
    let base = add(
        f,
        0,
        fill(&[(Channel::BaseColor, procedural(ProceduralKind::Clouds, 0.1, 0.9)), (Channel::Height, procedural(ProceduralKind::Cells, -0.3, 0.3))]),
        None,
    );
    let mask = generator_mask(f, GeneratorKind::Position, None);
    let g1 = group(f, None, BlendMode::Normal, 0.8, Some(mask));
    let inner =
        put(f, 0, Some(g1), fill(&[(Channel::Roughness, procedural(ProceduralKind::Grunge, 0.0, 1.0)), (Channel::Metallic, Source::Uniform([0.3; 3]))]), None);
    let mut checker = Mask::new(MaskBackground::Black);
    checker.effects.push(effect(f, EffectKind::Fill { source: procedural(ProceduralKind::Checker, 0.2, 1.0), projection: Projection::default() }));
    let g2 = group(f, Some(g1), BlendMode::Passthrough, 1.0, Some(checker));
    let mask = generator_mask(f, GeneratorKind::Direction, Some(16.0));
    put(f, 0, Some(g2), fill(&[(Channel::BaseColor, procedural(ProceduralKind::Scratches, 0.0, 1.0))]), Some(mask));
    let deep = put(f, 0, Some(g2), LayerKind::Paint, None);
    paint_rect(f, deep, (20, 30), (150, 120), 0.8);
    let mask = generator_mask(f, GeneratorKind::Position, None);
    let g2_above =
        put(f, 0, Some(g2), fill(&[(Channel::BaseColor, Source::Uniform([0.2, 0.6, 0.3])), (Channel::Roughness, Source::Uniform([0.9; 3]))]), Some(mask));
    blend(f, g2_above, BlendMode::Overlay, 0.7);
    put(f, 0, Some(g2), LayerKind::Instance { source: base }, Some(Mask::new(MaskBackground::White)));
    let g1_above = put(f, 0, Some(g1), fill(&[(Channel::BaseColor, procedural(ProceduralKind::Wood, 0.2, 0.8))]), None);
    let hsl = effect(f, EffectKind::Filter(Filter::Hsl { hue: 0.2, saturation: 0.1, lightness: 0.0 }));
    let blur = effect(f, EffectKind::Filter(Filter::Blur { radius: 20.0 }));
    f.project.layer_mut(g1_above).unwrap().content_effects.extend([hsl, blur]);
    blend(f, g1_above, BlendMode::Screen, 0.5);
    let mask = generator_mask(f, GeneratorKind::Position, Some(30.0));
    let top_fill = add(f, 0, fill(&[(Channel::BaseColor, Source::Uniform([0.9, 0.5, 0.1])), (Channel::Height, Source::Uniform([0.2; 3]))]), Some(mask));
    blend(f, top_fill, BlendMode::Multiply, 0.6);
    let top = add(f, 0, LayerKind::Paint, None);
    paint_rect(f, top, (100, 60), (120, 150), 0.3);
    let invert = effect(f, EffectKind::Filter(Filter::Invert));
    f.project.layer_mut(top).unwrap().content_effects.push(invert);
    Nested { floor, base, g1, inner, g2, deep, g2_above, g1_above, top_fill, top }
}

/// A live stroke over the middle of the texture painting `target`.
fn stroke(target: LayerId) -> StrokePreview {
    let mut alpha = TileImage::new(RES, RES, 1);
    for y in 90..170 {
        for x in 10..240 {
            alpha.set(x, y, &[to_u16(((x + y) % 7) as f32 / 6.0)]);
        }
    }
    StrokePreview {
        target: PaintTarget::Layer(target),
        alpha,
        opacity: 0.9,
        erase: false,
        values: [(Channel::BaseColor, [1.0, 0.1, 0.1]), (Channel::Roughness, [0.2; 3])].into_iter().collect(),
        mask_value: 1.0,
        stencil: None,
        carried: BTreeMap::new(),
    }
}

fn run_with(f: &Fixture, ts: usize, tiles: &[(u32, u32)], below: Option<&mut BelowCache>, preview: Option<&StrokePreview>) -> Composite {
    let (images, baked) = (Default::default(), Default::default());
    let mut inp = inputs(f, ts, &images, &baked);
    inp.preview = preview;
    let mut out = Composite::new(&f.project.texture_sets[ts].channel_list(), RES);
    composite_tiles_cached(&inp, &mut out, tiles, below);
    out
}

/// With `hot` cached, cold, partly warm and warm composites (and, for paint layers, a live
/// stroke on it) match the full composite bit for bit. Returns the warm cache.
fn check_cached(f: &Fixture, ts: usize, hot: LayerId) -> BelowCache {
    let all = all_tiles(RES, RES);
    let want = reference(f, ts);
    let mut cache = BelowCache::default();
    cache.set_hot(Some(hot));
    let half: Vec<(u32, u32)> = all.iter().copied().filter(|t| (t.0 + t.1) % 2 == 0).collect();
    let mut out = run(f, ts, &half, Some(&mut cache));
    let (images, baked) = (Default::default(), Default::default());
    composite_tiles_cached(&inputs(f, ts, &images, &baked), &mut out, &all, Some(&mut cache));
    assert!(out == want, "a partly cached composite differs (hot {hot:?})");
    assert!(run(f, ts, &all, Some(&mut cache)) == want, "a warm composite differs (hot {hot:?})");
    if matches!(f.project.layer(hot).unwrap().kind, LayerKind::Paint) {
        let pv = stroke(hot);
        let n = cache.len();
        assert!(run_with(f, ts, &all, Some(&mut cache), Some(&pv)) == run_with(f, ts, &all, None, Some(&pv)), "a live stroke differs (hot {hot:?})");
        assert_eq!(cache.len(), n, "the live stroke on the hot layer keeps the cache");
    }
    cache
}

#[test]
fn hot_layers_anywhere_in_nested_folders_composite_exactly() {
    let mut f = fixture();
    let n = nested_stack(&mut f);
    for hot in [n.deep, n.inner, n.g2, n.g1_above, n.top_fill, n.top, n.floor] {
        let cache = check_cached(&f, 0, hot);
        assert!(!cache.is_empty(), "hot {hot:?} is cached");
    }
    // The base fill is drawn twice (an instance in a folder above): nothing is cached.
    assert!(check_cached(&f, 0, n.base).is_empty());
    // Committing the stroke (painting the hot layer) keeps the cache and stays exact.
    let mut cache = check_cached(&f, 0, n.deep);
    let len = cache.len();
    paint_rect(&mut f, n.deep, (60, 100), (100, 40), 0.1);
    assert!(run(&f, 0, &all_tiles(RES, RES), Some(&mut cache)) == reference(&f, 0));
    assert_eq!(cache.len(), len);
}

#[test]
fn instanced_folders_cache_and_hot_layers_drawn_twice_fall_back() {
    let mut f = fixture();
    let n = nested_stack(&mut f);
    // Set B draws set A's top level through instances: the hot layer sits in an instanced folder.
    for id in f.project.texture_sets[0].stack.clone() {
        add(&mut f, 1, LayerKind::Instance { source: id }, None);
    }
    for hot in [n.deep, n.g1, n.top] {
        let cache = check_cached(&f, 1, hot);
        assert!(!cache.is_empty(), "hot {hot:?} is cached through its instance");
    }
    // An instance of the hot layer above it: drawn twice, nothing is cached.
    put(&mut f, 0, Some(n.g1), LayerKind::Instance { source: n.deep }, None);
    assert!(check_cached(&f, 0, n.deep).is_empty());
}

#[test]
fn anchor_readers_fall_back_to_full_evaluation() {
    let mut f = fixture();
    let n = nested_stack(&mut f);
    let anchor = effect(&mut f, EffectKind::Anchor);
    let source = Source::Anchor { anchor: anchor.id, channel: Channel::Height };
    f.project.layer_mut(n.base).unwrap().content_effects.push(anchor);
    // A mask above the hot layer reads the stack below it.
    let reader = effect(&mut f, EffectKind::Fill { source, projection: Projection::default() });
    f.project.layer_mut(n.g2_above).unwrap().mask.as_mut().unwrap().effects.push(reader);
    for hot in [n.deep, n.top] {
        assert!(check_cached(&f, 0, hot).is_empty(), "anchors read the stack: nothing cached");
    }
}

#[test]
fn edits_off_the_hot_layer_invalidate_and_edits_to_it_do_not() {
    let mut f = fixture();
    let n = nested_stack(&mut f);
    let all = all_tiles(RES, RES);
    let mut cache = check_cached(&f, 0, n.deep);
    // One tile composited after an edit: a cache emptied by the edit holds just that tile.
    let mut emptied = |f: &Fixture| {
        run(f, 0, &[(1, 1)], Some(&mut cache));
        let emptied = cache.len() == 1;
        assert!(run(f, 0, &all, Some(&mut cache)) == reference(f, 0), "a stale cache was used");
        emptied
    };
    paint_rect(&mut f, n.deep, (0, 0), (200, 200), 0.5);
    assert!(!emptied(&f), "painting the hot layer");
    blend(&mut f, n.deep, BlendMode::Multiply, 0.5);
    f.project.layer_mut(n.deep).unwrap().mask = Some(Mask::new(MaskBackground::White));
    assert!(!emptied(&f), "the hot layer's blending and mask");
    if let LayerKind::Fill(fl) = &mut f.project.layer_mut(n.g2_above).unwrap().kind {
        fl.sources.insert(Channel::Roughness, Source::Uniform([0.1; 3]));
    }
    assert!(emptied(&f), "a fill value above");
    if let EffectKind::Generator(g) = &mut f.project.layer_mut(n.g2_above).unwrap().mask.as_mut().unwrap().effects[0].kind {
        g.level = 0.2;
    }
    assert!(emptied(&f), "a mask effect parameter above");
    f.project.layer_mut(n.top_fill).unwrap().visible = false;
    assert!(emptied(&f), "visibility above");
    f.project.move_layer(n.g1_above, 0).unwrap();
    assert!(emptied(&f), "a reorder");
    blend(&mut f, n.g1, BlendMode::Normal, 0.4);
    assert!(emptied(&f), "a folder holding the hot layer");
    paint_rect(&mut f, n.top, (0, 0), (30, 30), 0.9);
    assert!(emptied(&f), "pixels above");
}

/// Paint a half-covered rectangle (base color and roughness `v`) into a paint layer stored at
/// `size` texels.
fn paint_rect_at(f: &mut Fixture, id: LayerId, size: u32, (x0, y0): (u32, u32), (w, h): (u32, u32), v: f32) {
    for (ch, px) in [(Channel::BaseColor, vec![to_u16(v * 0.5), to_u16(0.1), 0, to_u16(0.5)]), (Channel::Roughness, vec![to_u16(v * 0.5), to_u16(0.5)])] {
        let img = f.pixels.layer_mut(id, ch, (size, size));
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                img.set(x, y, &px);
            }
        }
    }
}

#[test]
fn instances_resample_sources_painted_at_another_resolution() {
    let mut f = fixture();
    let half = RES / 2;
    f.project.texture_sets[1].resolution = half;
    // The source lives in set B at half resolution, masked by half-resolution mask pixels.
    let src = add(&mut f, 1, LayerKind::Paint, None);
    paint_rect_at(&mut f, src, half, (20, 30), (60, 50), 0.9);
    let paint = effect(&mut f, EffectKind::Paint);
    let mask_id = paint.id;
    f.project.layer_mut(src).unwrap().mask = Some(Mask { effects: vec![paint], ..Mask::new(MaskBackground::Black) });
    let m = f.pixels.mask_mut(mask_id, (half, half));
    for y in 0..half {
        for x in 0..50 {
            m.set(x, y, &[to_u16(1.0), to_u16(1.0)]);
        }
    }
    add(&mut f, 0, fill(&[(Channel::Metallic, procedural(ProceduralKind::Clouds, 0.0, 1.0))]), None);
    let inst = add(&mut f, 0, LayerKind::Instance { source: src }, None);
    let top = add(&mut f, 0, LayerKind::Paint, None);
    paint_rect(&mut f, top, (0, 0), (40, 40), 0.2);
    let out = reference(&f, 0);
    let rough = |x, y| out.value(Channel::Roughness, x, y)[0];
    // Painted roughness 0.9 at half coverage over the default 0.5, where the mask lets it through.
    assert!((rough(70, 120) - 0.7).abs() < 2e-3, "inside: {}", rough(70, 120));
    assert!((rough(180, 120) - 0.5).abs() < 2e-3, "masked out: {}", rough(180, 120));
    assert!((rough(70, 200) - 0.5).abs() < 2e-3, "not painted: {}", rough(70, 200));
    for hot in [top, inst] {
        check_cached(&f, 0, hot);
    }
    assert!(!check_cached(&f, 0, top).is_empty());
}

#[test]
fn anchors_inside_a_content_stack_record_the_content_up_to_them() {
    let mut f = fixture();
    let low = add(&mut f, 0, fill(&[(Channel::Height, Source::Uniform([0.4; 3]))]), None);
    f.project.layer_mut(low).unwrap().channels.retain(|c, _| *c == Channel::Height);
    blend(&mut f, low, BlendMode::Normal, 0.5);
    let (mid, end) = (effect(&mut f, EffectKind::Anchor), effect(&mut f, EffectKind::Anchor));
    let (mid_id, end_id) = (mid.id, end.id);
    let mut raise = Fill::default();
    raise.sources.insert(Channel::Height, Source::Uniform([0.8; 3]));
    let raise = effect(&mut f, EffectKind::Material(raise));
    f.project.layer_mut(low).unwrap().content_effects.extend([mid, raise, end]);
    let top = add(&mut f, 0, LayerKind::Paint, None);
    paint_rect(&mut f, top, (0, 0), (60, 60), 0.3);
    // Without readers the stack is cached (anchors are still recorded) and exact.
    assert!(!check_cached(&f, 0, top).is_empty());
    // Readers: roughness reads the anchor inside the stack, metallic the one at its end.
    for (c, anchor) in [(Channel::Roughness, mid_id), (Channel::Metallic, end_id)] {
        let reader = add(&mut f, 0, fill(&[(c, Source::Anchor { anchor, channel: Channel::Height })]), None);
        f.project.layer_mut(reader).unwrap().channels.retain(|x, _| *x == c);
    }
    let out = reference(&f, 0);
    let at = |c| out.value(c, 128, 128)[0];
    // Half opacity over the default height 0: 0.4 before the raise, 0.8 after it.
    assert!((at(Channel::Roughness) - 0.2).abs() < 2e-3, "inside the stack: {}", at(Channel::Roughness));
    assert!((at(Channel::Metallic) - 0.4).abs() < 2e-3, "at the end: {}", at(Channel::Metallic));
    assert!((at(Channel::Height) - 0.4).abs() < 2e-3);
    assert!(check_cached(&f, 0, top).is_empty(), "anchor readers evaluate everything");
    // Behind a blur the cut reads the filtered field; before it, the unfiltered content.
    let blur = effect(&mut f, EffectKind::Filter(Filter::Blur { radius: 30.0 }));
    f.project.layer_mut(low).unwrap().content_effects.insert(0, blur);
    let blurred = reference(&f, 0);
    assert!((blurred.value(Channel::Roughness, 128, 128)[0] - 0.2).abs() < 2e-3, "a flat height stays flat");
    let mut one_by_one = Composite::new(&f.project.texture_sets[0].channel_list(), RES);
    let (images, baked) = (Default::default(), Default::default());
    for t in all_tiles(RES, RES) {
        composite_tiles(&inputs(&f, 0, &images, &baked), &mut one_by_one, &[t]);
    }
    assert!(one_by_one == blurred);
}

#[test]
fn gathering_filters_composite_identically_in_any_grouping_and_cached() {
    let mut f = fixture();
    let base = add(
        &mut f,
        0,
        fill(&[(Channel::BaseColor, procedural(ProceduralKind::Clouds, 0.1, 0.9)), (Channel::Roughness, procedural(ProceduralKind::Checker, 0.0, 1.0))]),
        None,
    );
    let mut slope = Mask::new(MaskBackground::Black);
    slope.effects.push(effect(&mut f, EffectKind::Fill { source: procedural(ProceduralKind::Checker, 0.0, 1.0), projection: Projection::default() }));
    slope.effects.push(effect(&mut f, EffectKind::Filter(Filter::SlopeBlur { radius: 24.0, scale: 3.0, seed: 1 })));
    let streaks = add(&mut f, 0, fill(&[(Channel::Metallic, procedural(ProceduralKind::Stripes, 0.0, 1.0))]), Some(slope));
    let top = add(&mut f, 0, LayerKind::Paint, None);
    paint_rect(&mut f, top, (30, 40), (150, 100), 0.4);
    let plain = reference(&f, 0);
    let warp = effect(&mut f, EffectKind::Filter(Filter::Warp { intensity: 30.0, scale: 4.0, seed: 3 }));
    f.project.layer_mut(base).unwrap().content_effects.push(warp);
    let line = effect(&mut f, EffectKind::Filter(Filter::DirectionalBlur { radius: 40.0, angle: 30.0 }));
    f.project.layer_mut(streaks).unwrap().content_effects.push(line);
    let want = reference(&f, 0);
    for c in [Channel::BaseColor, Channel::Roughness, Channel::Metallic] {
        assert!(want.channels[&c] != plain.channels[&c], "{c:?} changed by its filter");
    }
    let mut one_by_one = Composite::new(&f.project.texture_sets[0].channel_list(), RES);
    let (images, baked) = (Default::default(), Default::default());
    for t in all_tiles(RES, RES) {
        composite_tiles(&inputs(&f, 0, &images, &baked), &mut one_by_one, &[t]);
    }
    assert!(one_by_one == want, "per-tile composites differ from one pass");
    for hot in [top, streaks] {
        assert!(!check_cached(&f, 0, hot).is_empty());
    }
}

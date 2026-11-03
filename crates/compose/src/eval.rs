//! The layer-stack evaluator: composites a texture set's stack into one image per channel,
//! tile by tile in parallel. This is the reference implementation (the oracle) for the viewport
//! and for export.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};

use rayon::prelude::*;
use texturecraft_core::{
    BakedMap, BlendMode, BlockScales, Channel, ChannelKind, CompareOp, Effect, EffectKind, Fill, Filter, Generator, GeneratorKind, HeightMask, Layer, LayerId,
    LayerKind, Mask, MaskBackground, NormalFormat, Project, Projection, Source, TextureSet, TileGrid, blend1, blend3, lum, mix3, per_unit,
};
use texturecraft_geom::glam::{Vec2, Vec3};
use texturecraft_geom::{Mesh, NO_TRI, UvGBuffer};
use texturecraft_texel::{DilationMap, TILE, TILE_AREA, TileImage, from_u16, noise, to_u16, to_u16_signed};

use crate::below::{Band, BelowCache, Level};
use crate::pattern::{Coords, decal_uv, image_samples, procedural, world_noise};
use crate::store::{BakedMaps, ImageStore, PixelStore, decode};

const NCH: usize = Channel::ALL.len();

/// Stack values being composited, per channel.
type Acc = [[f32; 3]; NCH];

/// What a live stroke paints into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaintTarget {
    /// A paint layer's channels.
    Layer(LayerId),
    /// A paint effect inside a mask.
    Mask(LayerId),
}

/// A screen-pinned image painted through the camera (projection tool / stencil).
#[derive(Clone, Debug)]
pub struct Stencil {
    pub image: std::sync::Arc<crate::store::ImageRes>,
    /// Camera view-projection captured when the stroke started.
    pub view_proj: texturecraft_geom::glam::Mat4,
    /// Image placement in normalized device coordinates: center, half extents, rotation.
    pub center: [f32; 2],
    pub half: [f32; 2],
    pub rotation: f32,
    /// Repeat the image beyond its bounds (otherwise nothing is painted outside it).
    pub tile: bool,
}

impl Stencil {
    /// Linear color and alpha of the stencil at world point `p` (None outside the image).
    pub fn sample_world(&self, p: Vec3) -> Option<([f32; 3], f32)> {
        let c = self.view_proj * p.extend(1.0);
        if c.w <= 1e-6 {
            return None;
        }
        let (dx, dy) = (c.x / c.w - self.center[0], c.y / c.w - self.center[1]);
        let (sn, cs) = (-self.rotation).sin_cos();
        let lx = (dx * cs - dy * sn) / self.half[0].max(1e-6);
        let ly = (dx * sn + dy * cs) / self.half[1].max(1e-6);
        if !self.tile && (lx.abs() > 1.0 || ly.abs() > 1.0) {
            return None;
        }
        let s = self.image.sample(lx * 0.5 + 0.5, 0.5 - ly * 0.5);
        let rgb = if self.image.srgb { [s[0], s[1], s[2]].map(texturecraft_texel::srgb_to_linear) } else { [s[0], s[1], s[2]] };
        Some((rgb, s[3].clamp(0.0, 1.0)))
    }
}

/// The stroke being drawn, shown before it is committed.
#[derive(Clone, Debug)]
pub struct StrokePreview {
    pub target: PaintTarget,
    /// Stroke alpha per texel (1 component, 0..1).
    pub alpha: TileImage,
    /// Stroke opacity cap.
    pub opacity: f32,
    pub erase: bool,
    /// Values painted per channel (layer targets).
    pub values: BTreeMap<Channel, [f32; 3]>,
    /// Gray value painted into a mask.
    pub mask_value: f32,
    /// Projection tool: base color (and alpha) come from a screen-pinned image.
    pub stencil: Option<Stencil>,
    /// Clone and smudge: straight values per channel carried by the stroke (replace `values`).
    pub carried: BTreeMap<Channel, TileImage>,
}

/// Everything the evaluator reads.
pub struct Inputs<'a> {
    pub project: &'a Project,
    pub ts: usize,
    pub mesh: &'a Mesh,
    pub gbuf: &'a UvGBuffer,
    pub pixels: &'a PixelStore,
    pub baked: &'a BakedMaps,
    pub preview: Option<&'a StrokePreview>,
    /// Imported image resources (fill sources).
    pub images: &'a ImageStore,
    /// Mesh bounds, for scale-independent procedural coordinates.
    pub bounds: (Vec3, Vec3),
}

impl Inputs<'_> {
    /// The UV tile grid of a UV-tiled set (the mesh's UVs for this set are in atlas space).
    fn tile_grid(&self) -> Option<TileGrid> {
        self.project.texture_set(self.ts).ok().and_then(TextureSet::tile_grid)
    }

    /// Texels per atlas block across (the whole width for a classic set): filter radii scale
    /// with it, and with the tile's share of its block.
    fn tile_res(&self) -> u32 {
        self.gbuf.width / self.tile_grid().map_or(1, |g| g.cols.max(1))
    }
}

/// The composited channels of one texture set.
#[derive(Clone, Debug, PartialEq)]
pub struct Composite {
    /// Width in texels (the resolution of a classic, square set).
    pub resolution: u32,
    pub height: u32,
    pub channels: BTreeMap<Channel, TileImage>,
    /// The set's [`TextureSet::block_scales`]: height slopes on the display follow each tile.
    pub block_scales: BlockScales,
}

impl Composite {
    /// A square composite.
    pub fn new(channels: &[Channel], resolution: u32) -> Composite {
        Composite::with_size(channels, resolution, resolution)
    }

    pub fn with_size(channels: &[Channel], width: u32, height: u32) -> Composite {
        Composite {
            resolution: width,
            height,
            channels: channels.iter().map(|c| (*c, TileImage::new(width, height, c.components()))).collect(),
            block_scales: BlockScales::default(),
        }
    }

    /// Channel value at a texel (the channel default when absent).
    pub fn value(&self, c: Channel, x: u32, y: u32) -> [f32; 3] {
        let Some(img) = self.channels.get(&c) else { return c.default_value() };
        let raw = img.get(x, y);
        let k = c.kind();
        if !img.is_allocated(x / TILE, y / TILE) {
            return c.default_value();
        }
        if c.components() == 3 { [decode(k, raw[0]), decode(k, raw[1]), decode(k, raw[2])] } else { [decode(k, raw[0]); 3] }
    }
}

/// Every tile of a `width × height` image.
pub fn all_tiles(width: u32, height: u32) -> Vec<(u32, u32)> {
    let (nx, ny) = (width.div_ceil(TILE), height.div_ceil(TILE));
    (0..ny).flat_map(|y| (0..nx).map(move |x| (x, y))).collect()
}

/// Tiles that hold at least one covered texel.
pub fn covered_tiles(gbuf: &UvGBuffer) -> Vec<(u32, u32)> {
    let (tw, th) = (gbuf.width.div_ceil(TILE), gbuf.height.div_ceil(TILE));
    let mut out = Vec::new();
    for ty in 0..th {
        for tx in 0..tw {
            let any = (ty * TILE..((ty + 1) * TILE).min(gbuf.height))
                .any(|y| (tx * TILE..((tx + 1) * TILE).min(gbuf.width)).any(|x| gbuf.tri.get((y * gbuf.width + x) as usize).is_some_and(|t| *t != NO_TRI)));
            if any {
                out.push((tx, ty));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Per-tile plan: the stack flattened with each layer's tile slices looked up once per tile.

struct ChanPlan<'a> {
    ch: usize,
    kind: ChannelKind,
    comps: usize,
    blend: BlendMode,
    opacity: f32,
    data: Option<Cow<'a, [u16]>>,
    /// Visible content effects, holding this channel's tile for paint effects (empty, and
    /// allocation free, for layers without content effects).
    effects: Vec<EffectPlan<'a>>,
    /// A content neighbourhood filter precomputed over the region: continue the effects after
    /// effect `.0` starting from the field's value.
    start: Option<(usize, &'a Field)>,
}

enum Content<'a> {
    Paint { preview: bool },
    Fill(&'a Fill),
    Group(Vec<Node<'a>>),
}

struct EffectPlan<'a> {
    kind: &'a EffectKind,
    blend: BlendMode,
    opacity: f32,
    data: Option<Cow<'a, [u16]>>,
    preview: bool,
}

struct MaskPlan<'a> {
    background: f32,
    effects: Vec<EffectPlan<'a>>,
    /// A neighbourhood filter (blur, sharpen) precomputed over the region: continue the stack
    /// after effect `.0` starting from the field's value.
    start: Option<(usize, &'a Field)>,
}

/// A layer's mask (one value per texel) or one content channel (straight value and coverage)
/// after its last neighbourhood filter, over a texel region.
pub struct Field {
    x0: u32,
    y0: u32,
    w: u32,
    h: u32,
    comps: usize,
    data: Vec<f32>,
}

impl Field {
    fn get(&self, x: u32, y: u32) -> Option<&[f32]> {
        if x < self.x0 || y < self.y0 || x >= self.x0 + self.w || y >= self.y0 + self.h {
            return None;
        }
        let i = ((y - self.y0) * self.w + (x - self.x0)) as usize * self.comps;
        self.data.get(i..i + self.comps)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FieldKey {
    Mask(LayerId),
    Content(LayerId, Channel),
}

pub type Fields = HashMap<FieldKey, Field>;

/// How a neighbourhood filter reads around a texel.
#[derive(Clone, Copy)]
enum Op {
    /// Box blurs (≈ Gaussian), sharpening by the amount when given.
    Blur(Option<f32>),
    /// The value minus its box blur, around the channel's mid value.
    HighPass,
    /// Values ramped down towards their shape's edge, from the box blur.
    Bevel,
    /// The value plus its box blur times the amount.
    Glow(f32),
    /// The value at the center of its cell (`r` texels across).
    Mosaic,
    /// Average along a direction (unit vector in texels).
    Line([f32; 2]),
    /// Read at an offset given by a noise vector field, or by noise along a direction.
    Warp { scale: f32, seed: u32, dir: Option<[f32; 2]> },
    /// Gradient magnitude from central differences.
    Edge,
    /// Average downhill along a noise height field.
    Slope { scale: f32, seed: u32 },
}

/// Index (among visible effects) of the last neighbourhood filter of an effect stack, its
/// radius in texels (`res` texels per UV tile across) and what it does.
fn neighbourhood(effects: &[Effect], res: u32) -> Option<(usize, f32, Op)> {
    let texels = |v: f32| (v * res as f32 / 2048.0).max(0.5);
    effects
        .iter()
        .filter(|e| e.visible)
        .enumerate()
        .filter_map(|(i, e)| match &e.kind {
            EffectKind::Filter(Filter::Blur { radius }) if *radius > 0.0 => Some((i, texels(*radius), Op::Blur(None))),
            EffectKind::Filter(Filter::Sharpen { amount }) => Some((i, (1.5 * res as f32 / 2048.0).max(0.75), Op::Blur(Some(*amount)))),
            EffectKind::Filter(Filter::DirectionalBlur { radius, angle }) if *radius > 0.0 => Some((i, texels(*radius), Op::Line(direction(*angle)))),
            EffectKind::Filter(Filter::Warp { intensity, scale, seed }) if *intensity > 0.0 => {
                Some((i, texels(*intensity), Op::Warp { scale: *scale, seed: *seed, dir: None }))
            }
            EffectKind::Filter(Filter::DirectionalWarp { intensity, angle, scale, seed }) if *intensity > 0.0 => {
                Some((i, texels(*intensity), Op::Warp { scale: *scale, seed: *seed, dir: Some(direction(*angle)) }))
            }
            EffectKind::Filter(Filter::HighPass { radius }) => Some((i, texels(*radius), Op::HighPass)),
            EffectKind::Filter(Filter::EdgeDetect { width }) => Some((i, texels(*width), Op::Edge)),
            // Three box passes ramp an edge over about twice their radius.
            EffectKind::Filter(Filter::Bevel { width }) => Some((i, texels(*width * 0.5), Op::Bevel)),
            EffectKind::Filter(Filter::Glow { radius, amount }) => Some((i, texels(*radius), Op::Glow(*amount))),
            EffectKind::Filter(Filter::Mosaic { size }) => Some((i, texels(*size), Op::Mosaic)),
            EffectKind::Filter(Filter::SlopeBlur { radius, scale, seed }) if *radius > 0.0 => {
                Some((i, texels(*radius), Op::Slope { scale: *scale, seed: *seed }))
            }
            _ => None,
        })
        .last()
}

/// Unit texel-space vector of `angle` degrees. Texel rows run down: positive angles turn
/// counterclockwise on screen.
fn direction(angle: f32) -> [f32; 2] {
    let (sn, cs) = angle.to_radians().sin_cos();
    [cs, -sn]
}

/// Samples along a directional or slope blur at most (beyond, samples spread out).
const LINE_SAMPLES: i32 = 24;

/// Fractal noise over texel position `(x, y)` (`res` texels per UV unit, `scale` features per
/// UV unit), shifted by `d` in noise space.
fn texel_noise(x: f32, y: f32, d: [f32; 2], res: u32, scale: f32, seed: u32) -> f32 {
    let k = if scale.is_finite() { scale.clamp(0.01, 1000.0) } else { 1.0 } / res.max(1) as f32;
    noise::fbm([x * k + d[0], y * k + d[1], 0.0], 3, seed, noise::perlin)
}

/// The neighbourhood filters that gather samples (directional and slope blur, warp) over a
/// field region `(x0, y0, w, h)` of premultiplied `orig` texels (`comps` each) and their
/// `cover`: bilinear samples weighted by coverage, so uncovered texels don't bleed in.
/// Positions are absolute texel coordinates, so a texel's result does not depend on the region.
#[allow(clippy::too_many_arguments)]
fn gather(op: Op, orig: &[f32], cover: &[f32], comps: usize, (x0, y0, w, h): (u32, u32, u32, u32), r: f32, res: u32, opacity: f32) -> Vec<f32> {
    let mut data = orig.to_vec();
    let (wu, hu) = (w as usize, h as usize);
    // Bilinear sample at absolute texel position (fx, fy) into `acc` (components, then weight).
    let sample = |fx: f32, fy: f32, acc: &mut [f32; 5]| {
        if !(fx.is_finite() && fy.is_finite()) {
            return;
        }
        let (bx, by) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - bx, fy - by);
        for (dx, dy, k) in [(0, 0, (1.0 - tx) * (1.0 - ty)), (1, 0, tx * (1.0 - ty)), (0, 1, (1.0 - tx) * ty), (1, 1, tx * ty)] {
            let (cx, cy) = (bx as i64 + dx - i64::from(x0), by as i64 + dy - i64::from(y0));
            if k <= 0.0 || cx < 0 || cy < 0 || cx >= wu as i64 || cy >= hu as i64 {
                continue;
            }
            let i = cy as usize * wu + cx as usize;
            let (Some(c), Some(px)) = (cover.get(i), orig.get(i * comps..(i + 1) * comps)) else { continue };
            for (a, v) in acc.iter_mut().zip(px) {
                *a += v * k;
            }
            acc[4] += c * k;
        }
    };
    data.par_chunks_mut((comps * wu).max(1)).enumerate().for_each(|(row, line)| {
        for col in 0..wu {
            if cover.get(row * wu + col).is_none_or(|c| *c <= 0.0) {
                continue;
            }
            let (x, y) = ((x0 as usize + col) as f32, (y0 as usize + row) as f32);
            let mut acc = [0.0f32; 5];
            let n = (r.ceil() as i32).clamp(1, LINE_SAMPLES);
            let step = r / n as f32;
            match op {
                Op::Line(d) => {
                    for s in -n..=n {
                        let t = s as f32 * step;
                        sample(x + d[0] * t, y + d[1] * t, &mut acc);
                    }
                }
                Op::Warp { scale, seed, dir } => {
                    let off = |salt: u32| ((texel_noise(x, y, [0.0; 2], res, scale, seed ^ salt) - 0.5) * 3.0).clamp(-1.0, 1.0) * r;
                    match dir {
                        Some(d) => {
                            let t = off(0);
                            sample(x + d[0] * t, y + d[1] * t, &mut acc);
                        }
                        None => sample(x + off(0), y + off(0x5BD1_E995), &mut acc),
                    }
                }
                Op::Edge => {
                    let center = orig.get((row * wu + col) * comps..(row * wu + col + 1) * comps).unwrap_or(&[]);
                    let at = |dx: f32, dy: f32| {
                        let mut a = [0.0f32; 5];
                        sample(x + dx, y + dy, &mut a);
                        a
                    };
                    let (l, rt, u, d) = (at(-r, 0.0), at(r, 0.0), at(0.0, -r), at(0.0, r));
                    // A side outside the UV island reads the texel itself: seams are not edges.
                    let val = |a: &[f32; 5], k: usize| if a[4] > 1e-6 { a[k] / a[4] } else { center.get(k).copied().unwrap_or(0.0) };
                    let mag = |k: usize| (val(&rt, k) - val(&l, k)).hypot(val(&d, k) - val(&u, k)).min(1.0);
                    // Content stays premultiplied by its own coverage; masks are one value.
                    let cov = if comps == 4 { center.get(3).copied().unwrap_or(0.0) } else { 1.0 };
                    for (k, a) in acc.iter_mut().enumerate().take(comps.min(3)) {
                        *a = mag(k) * cov;
                    }
                    if comps == 4 {
                        acc[3] = cov;
                    }
                    acc[4] = 1.0;
                }
                Op::Slope { scale, seed } => {
                    let e = 0.01;
                    let hgt = |dx: f32, dy: f32| texel_noise(x, y, [dx, dy], res, scale, seed);
                    // Downhill; a flat spot keeps its value.
                    let d = Vec2::new(hgt(-e, 0.0) - hgt(e, 0.0), hgt(0.0, -e) - hgt(0.0, e)).try_normalize().unwrap_or(Vec2::ZERO);
                    for s in 0..=n {
                        let t = s as f32 * step;
                        sample(x + d.x * t, y + d.y * t, &mut acc);
                    }
                }
                Op::Mosaic => {
                    let center = |v: f32| ((v / r).floor() + 0.5) * r - 0.5;
                    sample(center(x), center(y), &mut acc);
                }
                Op::Blur(_) | Op::HighPass | Op::Bevel | Op::Glow(_) => {}
            }
            if acc[4] > 1e-6
                && let Some(px) = line.get_mut(col * comps..(col + 1) * comps)
            {
                for (slot, a) in px.iter_mut().zip(acc) {
                    *slot += (a / acc[4] - *slot) * opacity;
                }
            }
        }
    });
    data
}

/// Fixed-point scale of the blur. Integer running sums make a texel's result independent of
/// the region it is computed in, so tiles composited in any grouping agree exactly.
const FIX: f64 = (1u64 << 24) as f64;

/// Three running-sum box blurs (≈ Gaussian) of a `w × h` image along rows then along columns.
/// Rows (and columns, transposed) blur in parallel.
fn blur(data: &mut [f32], w: usize, h: usize, r: usize) {
    let fix = |v: &f32| if v.is_finite() { (f64::from(v.clamp(-1e6, 1e6)) * FIX).round() as i64 } else { 0 };
    let mut v: Vec<i64> = data.par_iter().map(fix).collect();
    blur_rows(&mut v, w, r);
    let mut t = transpose(&v, w, h);
    blur_rows(&mut t, h, r);
    data.par_iter_mut().zip(transpose(&t, h, w)).for_each(|(d, s)| *d = (s as f64 / FIX) as f32);
}

/// Three box blurs of radius `r` along every row (`w` long) of `data`.
fn blur_rows(data: &mut [i64], w: usize, r: usize) {
    if w == 0 {
        return;
    }
    data.par_chunks_mut(w).for_each(|row| {
        let mut tmp = vec![0i64; row.len()];
        for _ in 0..3 {
            box_line(row, &mut tmp, r);
            row.copy_from_slice(&tmp);
        }
    });
}

/// One box blur of radius `r` of a line (texels past the ends count as 0).
fn box_line(src: &[i64], dst: &mut [i64], r: usize) {
    let n = (2 * r + 1) as i64;
    // Rounded division of a window sum.
    let avg = |acc: i64| (2 * acc + n).div_euclid(2 * n);
    let mut acc: i64 = src.iter().take(r + 1).sum();
    for (i, d) in dst.iter_mut().enumerate() {
        *d = avg(acc);
        acc += src.get(i + r + 1).copied().unwrap_or(0);
        if i >= r {
            acc -= src.get(i - r).copied().unwrap_or(0);
        }
    }
}

/// `src` (`w × h`, row-major) transposed to `h × w`.
fn transpose(src: &[i64], w: usize, h: usize) -> Vec<i64> {
    let mut out = vec![0i64; src.len()];
    if h > 0 {
        out.par_chunks_mut(h).enumerate().for_each(|(x, col)| {
            for (y, d) in col.iter_mut().enumerate() {
                *d = src.get(y * w + x).copied().unwrap_or(0);
            }
        });
    }
    out
}

/// Covered texels of tile (tx, ty): (x, y, offset inside the tile).
fn tile_texels(gbuf: &UvGBuffer, tx: u32, ty: u32) -> impl Iterator<Item = (u32, u32, usize)> + '_ {
    let (w, h) = (gbuf.width, gbuf.height);
    (0..TILE).flat_map(|ly| (0..TILE).map(move |lx| (lx, ly))).filter_map(move |(lx, ly)| {
        let (x, y) = (tx * TILE + lx, ty * TILE + ly);
        (x < w && y < h && gbuf.tri.get((y * w + x) as usize).is_some_and(|t| *t != NO_TRI)).then_some((x, y, (ly * TILE + lx) as usize))
    })
}

/// Live stroke alpha at offset `o` of a preview tile.
fn stroke_at(preview: Option<&[u16]>, o: usize) -> f32 {
    preview.and_then(|t| t.get(o)).map(|v| from_u16(*v)).unwrap_or(0.0)
}

/// The full stack of tile (tx, ty) when `layer` reads anchor points (empty otherwise): evaluated
/// up to the layer, it records the anchors below it.
fn anchor_plan<'a>(inp: &Inputs<'a>, id: LayerId, channels: &[Channel], tx: u32, ty: u32, fields: Option<&'a Fields>) -> Vec<Node<'a>> {
    match inp.project.texture_set(inp.ts) {
        Ok(set) if reads_anchor(inp.project, id) => plan(inp, &set.stack, channels, tx, ty, 0, fields),
        _ => Vec::new(),
    }
}

/// Whether layer `id` (its own mask, or the content it draws) reads an anchor point.
pub(crate) fn reads_anchor(p: &Project, id: LayerId) -> bool {
    p.layers.get(&id).is_some_and(Layer::reads_anchor) || p.content_layer(id).is_some_and(Layer::reads_anchor)
}

/// The mask layer `id` draws with: its own, or an instance source's.
fn layer_mask(p: &Project, id: LayerId) -> Option<&Mask> {
    p.layers.get(&p.mask_owner(id))?.mask.as_ref()
}

/// Layers of `list` in drawing order (bottom → top, folder and instanced folder contents after
/// their folder).
fn draw_order(p: &Project, list: &[LayerId], depth: usize, out: &mut Vec<LayerId>) {
    if depth > texturecraft_core::project::MAX_DEPTH {
        return;
    }
    for id in list {
        out.push(*id);
        if let Some(l) = p.content_layer(*id) {
            draw_order(p, l.children(), depth + 1, out);
        }
    }
}

/// What a field holds.
#[derive(Clone, Copy)]
enum Job<'a> {
    Mask(&'a Mask),
    Content(Channel),
}

/// One neighbourhood-filtered field of `layer` over `tiles` plus the filter's reach, built on
/// the fields already computed below it.
fn field(inp: &Inputs, channels: &[Channel], below: &Fields, id: LayerId, job: Job, tiles: &[(u32, u32)]) -> Option<Field> {
    let (gw, gh) = (inp.gbuf.width, inp.gbuf.height);
    let effects = match job {
        Job::Mask(m) => &m.effects,
        Job::Content(_) => &inp.project.content_layer(id)?.content_effects,
    };
    let (k, radius, op) = neighbourhood(effects, inp.tile_res())?;
    let opacity = effects.iter().filter(|e| e.visible).nth(k).map(|e| e.opacity.clamp(0.0, 1.0)).unwrap_or(1.0);
    // Clamped: a hostile radius must not overflow the halo and window arithmetic.
    let r = radius.round().clamp(1.0, gw.max(gh) as f32) as u32;
    // Three box passes reach 3r texels, the other filters r (and a texel for bilinear samples).
    let blurs = matches!(op, Op::Blur(_) | Op::HighPass | Op::Bevel | Op::Glow(_));
    let reach = if blurs { 3 * r } else { r + 1 };
    let halo = reach.div_ceil(TILE);
    let (tx0, ty0) = tiles.iter().fold((u32::MAX, u32::MAX), |(a, b), t| (a.min(t.0), b.min(t.1)));
    let (tx1, ty1) = tiles.iter().fold((0, 0), |(a, b), t| (a.max(t.0), b.max(t.1)));
    let (rx0, ry0) = (tx0.saturating_sub(halo) * TILE, ty0.saturating_sub(halo) * TILE);
    let (rx1, ry1) = (((tx1 + 1 + halo) * TILE).min(gw), ((ty1 + 1 + halo) * TILE).min(gh));
    let (w, h) = (rx1.saturating_sub(rx0), ry1.saturating_sub(ry0));
    if w == 0 || h == 0 {
        return None;
    }
    let comps = match job {
        Job::Mask(_) => 1,
        Job::Content(_) => 4,
    };
    // Evaluate the stack before the filter, tile by tile (content premultiplied by coverage).
    let region_tiles: Vec<(u32, u32)> = (ry0 / TILE..ry1.div_ceil(TILE)).flat_map(|y| (rx0 / TILE..rx1.div_ceil(TILE)).map(move |x| (x, y))).collect();
    let parts: Vec<Vec<(u32, u32, [f32; 4])>> = region_tiles
        .par_iter()
        .map(|&(tx, ty)| {
            let full = anchor_plan(inp, id, channels, tx, ty, Some(below));
            let preview_tile = inp.preview.and_then(|p| p.alpha.tile(tx, ty));
            let mut mask = None;
            let mut nodes = Vec::new();
            match job {
                Job::Mask(m) => {
                    let Some(mut mp) = mask_plan(inp, id, m, tx, ty, None) else { return Vec::new() };
                    mp.effects.truncate(k);
                    mask = Some(mp);
                }
                Job::Content(ch) => {
                    nodes = plan(inp, &[id], &[ch], tx, ty, 0, Some(below));
                    if let Some(c) = nodes.first_mut().and_then(|n| n.chans.first_mut()) {
                        c.effects.truncate(k);
                        c.start = None;
                    }
                }
            }
            tile_texels(inp.gbuf, tx, ty)
                .map(|(x, y, o)| {
                    let mut tex = Texel::new(inp, x, y, o, stroke_at(preview_tile, o));
                    if !full.is_empty() {
                        tex.anchors_below(&full, id);
                    }
                    let v = match (&mask, nodes.first()) {
                        (Some(mp), _) => [tex.mask(mp), 0.0, 0.0, 0.0],
                        (None, Some(n)) => match n.chans.first().and_then(|c| tex.content(&n.content, c)) {
                            Some((v, cov)) => [v[0] * cov, v[1] * cov, v[2] * cov, cov],
                            None => [0.0; 4],
                        },
                        (None, None) => [0.0; 4],
                    };
                    (x, y, v)
                })
                .collect()
        })
        .collect();
    // Scatter into the region, a row of tiles at a time (the region is tile aligned).
    let n = (w * h) as usize;
    let mut orig = vec![0.0f32; n * comps];
    let mut cover = vec![0.0f32; n];
    let row_tiles = (rx1.div_ceil(TILE) - rx0 / TILE).max(1) as usize;
    let band = (TILE * w) as usize;
    orig.par_chunks_mut(band * comps).zip(cover.par_chunks_mut(band)).zip(parts.par_chunks(row_tiles)).for_each(|((orig, cover), row)| {
        for &(x, y, v) in row.iter().flatten() {
            let i = (((y - ry0) % TILE) * w + (x - rx0)) as usize;
            if let (Some(c), Some(px)) = (cover.get_mut(i), orig.get_mut(i * comps..(i + 1) * comps)) {
                *c = 1.0;
                px.copy_from_slice(&v[..comps]);
            }
        }
    });
    // High pass centers on the channel's mid value (0 for signed channels like height).
    let mid = match job {
        Job::Content(ch) if ch.kind() == ChannelKind::Signed => 0.0,
        _ => 0.5,
    };
    // The filter at the radius of a tile filling `scale` of its atlas block (a smaller tile has
    // fewer texels per UV unit, so its filters reach fewer texels).
    let filtered = |scale: f32| -> Vec<f32> {
        let res = ((inp.tile_res() as f32 * scale).round() as u32).max(1);
        let r = neighbourhood(effects, res).map_or(r, |(_, radius, _)| radius.round().clamp(1.0, r as f32) as u32);
        if !blurs {
            return gather(op, &orig, &cover, comps, (rx0, ry0, w, h), r as f32, res, opacity);
        }
        // Weighted so uncovered texels don't bleed in.
        let (wu, hu, ru) = (w as usize, h as usize, r as usize);
        let mut weights = cover.clone();
        blur(&mut weights, wu, hu, ru);
        let mut data = orig.clone();
        for p in 0..comps {
            let mut vals: Vec<f32> = orig.par_chunks(comps).map(|px| px.get(p).copied().unwrap_or(0.0)).collect();
            blur(&mut vals, wu, hu, ru);
            data.par_chunks_mut(comps).zip(vals.par_iter()).zip(weights.par_iter()).for_each(|((px, v), wt)| {
                // Content is premultiplied, so the mid value is too (coverage itself is kept).
                let cov = if comps == 4 { px.get(3).copied().unwrap_or(0.0) } else { 1.0 };
                if *wt > 1e-6
                    && let Some(slot) = px.get_mut(p)
                {
                    let (o, blurred) = (*slot, v / wt);
                    let filtered = match op {
                        Op::Blur(Some(amount)) => o + amount * (o - blurred),
                        Op::HighPass if p < 3 => o - blurred + mid * cov,
                        // Away from 0 by at most the value itself: 0 at the edge, full inside.
                        Op::Bevel if p < 3 => o.signum() * o.abs().min((2.0 * blurred * o.signum() - o.abs()).max(0.0)),
                        Op::Glow(amount) if p < 3 => o + amount * blurred,
                        Op::HighPass | Op::Bevel | Op::Glow(_) => o,
                        _ => blurred,
                    };
                    *slot = o + (filtered - o) * opacity;
                }
            });
        }
        data
    };
    // Each block keeps the result at its tile's scale (blocks are TILE aligned).
    let scales = inp.project.texture_set(inp.ts).map(TextureSet::block_scales).unwrap_or_default();
    let mut levels: Vec<f32> =
        (ry0..ry1).step_by(TILE as usize).flat_map(|y| (rx0..rx1).step_by(TILE as usize).map(move |x| (x, y))).map(|(x, y)| scales.at(x, y)).collect();
    levels.sort_by(f32::total_cmp);
    levels.dedup();
    let (first, rest) = levels.split_first().map_or((1.0, &[][..]), |(f, r)| (*f, r));
    let mut data = filtered(first);
    for &scale in rest {
        let other = filtered(scale);
        let row = w as usize * comps;
        data.par_chunks_mut(row).zip(other.par_chunks(row)).enumerate().for_each(|(y, (dst, src))| {
            for (x, (d, s)) in dst.chunks_mut(comps).zip(src.chunks(comps)).enumerate() {
                if scales.at(rx0 + x as u32, ry0 + y as u32) == scale {
                    d.copy_from_slice(s);
                }
            }
        });
    }
    unpremultiply(&mut data, comps);
    Some(Field { x0: rx0, y0: ry0, w, h, comps, data })
}

/// Clamp filtered masks to 0..1, and turn filtered content back into straight values.
fn unpremultiply(data: &mut [f32], comps: usize) {
    data.par_chunks_mut(comps).for_each(|px| match px {
        [v] => *v = v.clamp(0.0, 1.0),
        [r, g, b, cov] => {
            *cov = cov.clamp(0.0, 1.0);
            let inv = if *cov > 1e-5 { 1.0 / *cov } else { 0.0 };
            *r *= inv;
            *g *= inv;
            *b *= inv;
        }
        _ => {}
    });
}

/// Precompute the neighbourhood-filtered masks and content channels of every layer that needs
/// one, over `tiles` plus the filter's reach. Walks bottom → top so each field sees the fields
/// (and anchors) below it. Cheap when no layer has a blur or sharpen.
pub fn neighbourhood_fields(inp: &Inputs, channels: &[Channel], tiles: &[(u32, u32)]) -> Fields {
    let stack = inp.project.texture_set(inp.ts).map(|s| s.stack.as_slice()).unwrap_or_default();
    fields_of(inp, channels, stack, tiles)
}

/// [`neighbourhood_fields`] of the layers drawn by `list` only.
fn fields_of(inp: &Inputs, channels: &[Channel], list: &[LayerId], tiles: &[(u32, u32)]) -> Fields {
    let mut out = Fields::new();
    if tiles.is_empty() || inp.gbuf.width == 0 {
        return out;
    }
    let res = inp.gbuf.width;
    let mut order = Vec::new();
    draw_order(inp.project, list, 0, &mut order);
    for id in order {
        let Some(layer) = inp.project.content_layer(id) else { continue };
        if let Some(m) = layer_mask(inp.project, id).filter(|m| m.enabled && neighbourhood(&m.effects, res).is_some())
            && let Some(f) = field(inp, channels, &out, id, Job::Mask(m), tiles)
        {
            out.insert(FieldKey::Mask(id), f);
        }
        if layer.is_group() || neighbourhood(&layer.content_effects, res).is_none() {
            continue;
        }
        for ch in channels.iter().filter(|c| layer.channels.contains_key(c)) {
            if let Some(f) = field(inp, channels, &out, id, Job::Content(*ch), tiles) {
                out.insert(FieldKey::Content(id, *ch), f);
            }
        }
    }
    out
}

struct Node<'a> {
    id: LayerId,
    content: Content<'a>,
    chans: Vec<ChanPlan<'a>>,
    mask: Option<MaskPlan<'a>>,
    /// Visible anchor points of the layer: recorded after it is applied, or (`Some(k)`, an
    /// anchor with visible effects above it) with its content cut after `k` visible effects.
    anchors: Vec<(LayerId, Option<usize>)>,
    /// The node or one of its children records an anchor.
    has_anchor: bool,
    height_mask: HeightMask,
    /// Geometry mask: whether the layer shows on each mesh submesh (None: everywhere).
    parts: Option<Vec<bool>>,
}

/// Plans of the visible effects of a stack: mask effects (`channel` None) or the content
/// effects of one channel (paint effects then read that channel's pixels).
fn effect_plans<'a>(inp: &Inputs<'a>, effects: &'a [Effect], tx: u32, ty: u32, channel: Option<Channel>) -> Vec<EffectPlan<'a>> {
    effects
        .iter()
        .filter(|e| e.visible)
        .map(|e| {
            let (img, target) = match channel {
                Some(c) => (inp.pixels.layer(e.id, c), PaintTarget::Layer(e.id)),
                None => (inp.pixels.masks.get(&e.id), PaintTarget::Mask(e.id)),
            };
            EffectPlan {
                kind: &e.kind,
                blend: e.blend,
                opacity: e.opacity,
                data: if e.kind.painted() { tile_of(inp, img, tx, ty, channel) } else { None },
                preview: previews(inp, target),
            }
        })
        .collect()
}

/// Whether the live stroke paints `target` in this composite (a stroke on a layer instanced in a
/// set of another size does not show there).
fn previews(inp: &Inputs, target: PaintTarget) -> bool {
    inp.preview.is_some_and(|p| p.target == target && (p.alpha.width(), p.alpha.height()) == (inp.gbuf.width, inp.gbuf.height))
}

/// Tile (tx, ty) of painted pixels `img` (of `channel`, or a mask when None) at the size of the
/// set being composited: as stored, or resampled (bilinear, premultiplied) when the pixels
/// belong to a layer instanced from a set of another resolution.
fn tile_of<'a>(inp: &Inputs, img: Option<&'a TileImage>, tx: u32, ty: u32, channel: Option<Channel>) -> Option<Cow<'a, [u16]>> {
    let img = img?;
    let (w, h) = (inp.gbuf.width, inp.gbuf.height);
    if (img.width(), img.height()) == (w, h) {
        return img.tile(tx, ty).map(Cow::Borrowed);
    }
    // Empty texels of signed channels hold a signed zero.
    let empty: Vec<u16> = match channel {
        Some(c) if c.kind() == ChannelKind::Signed => [vec![to_u16_signed(0.0); c.components()], vec![0]].concat(),
        _ => Vec::new(),
    };
    img.resampled_tile(w, h, tx, ty, &empty).map(Cow::Owned)
}

fn mask_plan<'a>(inp: &Inputs<'a>, id: LayerId, mask: &'a Mask, tx: u32, ty: u32, fields: Option<&'a Fields>) -> Option<MaskPlan<'a>> {
    if !mask.enabled {
        return None;
    }
    let background = if mask.background == MaskBackground::White { 1.0 } else { 0.0 };
    let effects = effect_plans(inp, &mask.effects, tx, ty, None);
    let start = fields.and_then(|f| f.get(&FieldKey::Mask(id))).and_then(|field| neighbourhood(&mask.effects, inp.gbuf.width).map(|(k, _, _)| (k, field)));
    Some(MaskPlan { background, effects, start })
}

fn plan<'a>(inp: &Inputs<'a>, list: &[LayerId], channels: &[Channel], tx: u32, ty: u32, depth: usize, fields: Option<&'a Fields>) -> Vec<Node<'a>> {
    if depth > texturecraft_core::project::MAX_DEPTH {
        return Vec::new();
    }
    list.iter().filter_map(|id| plan_node(inp, *id, channels, tx, ty, depth, fields, true)).collect()
}

/// The plan of layer `id` at `depth` (None where it draws nothing); a folder's children are
/// planned when `kids`.
#[allow(clippy::too_many_arguments)]
fn plan_node<'a>(
    inp: &Inputs<'a>,
    id: LayerId,
    channels: &[Channel],
    tx: u32,
    ty: u32,
    depth: usize,
    fields: Option<&'a Fields>,
    kids: bool,
) -> Option<Node<'a>> {
    let own: &'a Layer = inp.project.layers.get(&id)?;
    // Instances draw their source's content (a missing source draws nothing).
    let layer: &'a Layer = inp.project.content_layer(id)?;
    if !own.visible {
        return None;
    }
    let paint = matches!(layer.kind, LayerKind::Paint);
    let fx = !layer.is_group() && layer.content_effects.iter().any(|e| e.visible);
    let fx_start = if fx { neighbourhood(&layer.content_effects, inp.gbuf.width).map(|n| n.0) } else { None };
    let chans = channels
        .iter()
        .filter_map(|c| {
            let b = layer.blend(*c)?;
            if b.mode == BlendMode::Disable || b.opacity <= 0.0 {
                return None;
            }
            let data = if paint { tile_of(inp, inp.pixels.layer(layer.id, *c), tx, ty, Some(*c)) } else { None };
            let effects = if fx { effect_plans(inp, &layer.content_effects, tx, ty, Some(*c)) } else { Vec::new() };
            let start = fx_start.zip(fields.and_then(|f| f.get(&FieldKey::Content(id, *c))));
            Some(ChanPlan { ch: c.index(), kind: c.kind(), comps: c.components(), blend: b.mode, opacity: b.opacity.clamp(0.0, 1.0), data, effects, start })
        })
        .collect::<Vec<_>>();
    let content = match &layer.kind {
        LayerKind::Paint => Content::Paint { preview: previews(inp, PaintTarget::Layer(layer.id)) },
        LayerKind::Fill(f) => Content::Fill(f),
        LayerKind::Group { children, .. } => Content::Group(if kids { plan(inp, children, channels, tx, ty, depth + 1, fields) } else { Vec::new() }),
        // `content_layer` already followed instance links.
        LayerKind::Instance { .. } => return None,
    };
    // A paint layer with nothing painted (and not being painted) or added by its content
    // effects contributes nothing.
    if let Content::Paint { preview: false } = content
        && chans.iter().all(|c| c.data.is_none() && c.effects.is_empty())
    {
        return None;
    }
    if chans.is_empty() {
        return None;
    }
    let mask = layer_mask(inp.project, id).and_then(|m| mask_plan(inp, id, m, tx, ty, fields));
    let anchors = if fx { anchor_cuts(&layer.content_effects) } else { Vec::new() };
    let has_anchor = !anchors.is_empty() || matches!(&content, Content::Group(nodes) if nodes.iter().any(|n| n.has_anchor));
    // The geometry mask is the layer's own (an instance in another set has other parts).
    let parts = own.geometry_mask.as_ref().map(|names| inp.mesh.submeshes.iter().map(|s| names.contains(&s.name)).collect());
    Some(Node { id, content, chans, mask, anchors, has_anchor, height_mask: layer.height_mask, parts })
}

/// The visible anchor points of a content stack with where they cut it: after their index among
/// the visible effects, or None for anchors with only anchors above them (they record the
/// layer's full result).
fn anchor_cuts(effects: &[Effect]) -> Vec<(LayerId, Option<usize>)> {
    let visible: Vec<&Effect> = effects.iter().filter(|e| e.visible).collect();
    let last = visible.iter().rposition(|e| !matches!(e.kind, EffectKind::Anchor));
    visible.iter().enumerate().filter(|(_, e)| matches!(e.kind, EffectKind::Anchor)).map(|(i, e)| (e.id, last.filter(|l| *l > i).map(|_| i))).collect()
}

/// One level of the path to the hot layer (see [`Level`]) planned for a tile.
struct LevelPlan<'a> {
    below: Vec<Node<'a>>,
    /// A folder planned without its children, or (last level) the hot layer.
    at: Option<Node<'a>>,
    above: Vec<Node<'a>>,
}

/// Plan the path to the hot layer for tile (tx, ty); the layers before the path are left out
/// on tiles the cache holds (`warm`).
fn plan_levels<'a>(inp: &Inputs<'a>, levels: &[Level], channels: &[Channel], tx: u32, ty: u32, fields: &'a Fields, warm: bool) -> Vec<LevelPlan<'a>> {
    let last = levels.len().saturating_sub(1);
    levels
        .iter()
        .enumerate()
        .map(|(d, l)| LevelPlan {
            below: if warm { Vec::new() } else { plan(inp, &l.below, channels, tx, ty, d, Some(fields)) },
            at: plan_node(inp, l.at, channels, tx, ty, d, Some(fields), d == last),
            above: plan(inp, &l.above, channels, tx, ty, d, Some(fields)),
        })
        .collect()
}

#[inline]
fn blend_chan(c: &ChanPlan, mode: BlendMode, a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    if c.comps == 3 { blend3(mode, a, b) } else { [blend1(mode, a[0], b[0]); 3] }
}

/// Lay value `p` with weight `a` over a straight value and coverage (premultiplied "over").
fn over(c: &ChanPlan, mode: BlendMode, v: [f32; 3], cov: f32, p: [f32; 3], a: f32) -> ([f32; 3], f32) {
    if a <= 0.0 {
        return (v, cov);
    }
    let b = if cov > 0.0 { blend_chan(c, mode, v, p) } else { p };
    let nc = a + cov * (1.0 - a);
    (mix3(v, b, a / nc.max(1e-5)), nc)
}

/// The plain UV projection a decal's frame coordinates are sampled through.
const DECAL_UV: Projection = Projection::Uv { scale: [1.0, 1.0], rotation: 0.0, offset: [0.0, 0.0] };

/// Per-texel context.
struct Texel<'a, 'b> {
    inp: &'b Inputs<'a>,
    x: u32,
    y: u32,
    /// Offset of this texel inside a tile (in texels).
    o: usize,
    res: u32,
    coords: Option<Coords>,
    stroke: f32,
    /// `eval` stops when it reaches this layer.
    stop: Option<LayerId>,
    /// Stack values recorded by the anchor points evaluated so far.
    anchors: Vec<(LayerId, Acc)>,
    /// Where `eval` takes layer masks and content from.
    tape: Tape<'b>,
}

/// What a layer contributes (its mask, its content value and coverage per channel): evaluated
/// (`Off`), evaluated and recorded, or replayed from a recording.
enum Tape<'t> {
    Off,
    Record(Vec<f32>),
    Replay(&'t [f32], usize),
}

impl<'t> Tape<'t> {
    fn next(&mut self) -> f32 {
        self.take(1).first().copied().unwrap_or(0.0)
    }

    /// The next `n` replayed values (empty when not replaying).
    fn take(&mut self, n: usize) -> &'t [f32] {
        match self {
            Tape::Replay(t, i) => {
                let t: &'t [f32] = t;
                let end = i.saturating_add(n);
                let s = t.get(*i..end).unwrap_or_default();
                *i = end;
                s
            }
            _ => &[],
        }
    }
}

impl<'a, 'b> Texel<'a, 'b> {
    fn new(inp: &'b Inputs<'a>, x: u32, y: u32, o: usize, stroke: f32) -> Self {
        Texel { inp, x, y, o, res: inp.gbuf.width, coords: None, stroke, stop: None, anchors: Vec::new(), tape: Tape::Off }
    }

    /// Run `f` with `tape` as this texel's tape.
    fn taped<R>(&mut self, tape: &mut Tape<'b>, f: impl FnOnce(&mut Self) -> R) -> R {
        std::mem::swap(&mut self.tape, tape);
        let r = f(self);
        std::mem::swap(&mut self.tape, tape);
        r
    }

    /// [`Texel::node_mask`] through the tape.
    fn tape_mask(&mut self, n: &Node) -> f32 {
        if let Tape::Replay(..) = self.tape {
            return self.tape.next();
        }
        let m = self.node_mask(n);
        if let Tape::Record(t) = &mut self.tape {
            t.push(m);
        }
        m
    }

    /// [`Texel::content`] through the tape: the coverage (0 where empty), then the value.
    fn tape_content(&mut self, content: &Content, c: &ChanPlan) -> Option<([f32; 3], f32)> {
        if let Tape::Replay(..) = self.tape {
            let cov = self.tape.next();
            // Recorded as 0 where empty.
            if cov <= 0.0 {
                return None;
            }
            let v = if c.comps == 3 { [self.tape.next(), self.tape.next(), self.tape.next()] } else { [self.tape.next(); 3] };
            return Some((v, cov));
        }
        let r = self.content(content, c);
        if let Tape::Record(t) = &mut self.tape {
            match r {
                Some((v, cov)) => {
                    t.push(cov);
                    let n = if c.comps == 3 { 3 } else { 1 };
                    t.extend_from_slice(&v[..n]);
                }
                None => t.push(0.0),
            }
        }
        r
    }

    fn coords(&mut self) -> Coords {
        if let Some(c) = self.coords {
            return c;
        }
        let g = self.inp.gbuf;
        let i = (self.y * g.width + self.x) as usize;
        let (lo, hi) = self.inp.bounds;
        let center = (lo + hi) * 0.5;
        let r = ((hi - lo).length() * 0.5).max(1e-4);
        let mut c = match g.surface(self.inp.mesh, i) {
            Some(s) => Coords { uv: s.uv, pos: (s.pos - center) / r, nrm: s.nrm },
            None => Coords { uv: Vec2::new((self.x as f32 + 0.5) / g.width as f32, (self.y as f32 + 0.5) / g.height as f32), pos: Vec3::ZERO, nrm: Vec3::Y },
        };
        // UV-tiled sets: projections see the mesh's own UVs (patterns repeat per tile).
        if let Ok(set) = self.inp.project.texture_set(self.inp.ts)
            && let Some(grid) = set.tile_grid()
        {
            c.uv = Vec2::from(set.from_atlas(&grid, c.uv.to_array()));
        }
        self.coords = Some(c);
        c
    }

    /// World position of this texel (None when uncovered).
    fn world(&self) -> Option<Vec3> {
        let i = (self.y * self.inp.gbuf.width + self.x) as usize;
        self.inp.gbuf.surface(self.inp.mesh, i).map(|s| s.pos)
    }

    fn baked(&self, m: BakedMap) -> Option<f32> {
        self.inp.baked.scalar(m, self.x, self.y, self.res)
    }

    /// Where projection `proj` samples this texel: the texel's coordinates, or for a decal its
    /// position in the decal's frame (then sampled as UVs) and the decal's coverage.
    fn projected<'p>(&mut self, proj: &'p Projection) -> (Coords, &'p Projection, f32) {
        let c = self.coords();
        match proj {
            Projection::Decal(d) => match self.world().and_then(|p| decal_uv(d, p, c.nrm)) {
                Some((uv, w)) => (Coords { uv, ..c }, &DECAL_UV, w),
                None => (c, &DECAL_UV, 0.0),
            },
            _ => (c, proj, 1.0),
        }
    }

    /// How much a fill projected by `proj` covers this texel: 1, or for a decal its frame
    /// coverage times the alpha of its first image source.
    fn cover<'s>(&mut self, mut sources: impl Iterator<Item = &'s Source>, proj: &Projection) -> f32 {
        if !matches!(proj, Projection::Decal(_)) {
            return 1.0;
        }
        let (c, _, w) = self.projected(proj);
        let image = sources.find_map(|s| match s {
            Source::Image { name, .. } => self.inp.images.get(name),
            _ => None,
        });
        w * image.map_or(1.0, |img| img.sample(c.uv.x, c.uv.y)[3].clamp(0.0, 1.0))
    }

    fn source(&mut self, s: &Source, proj: &Projection, kind: ChannelKind) -> [f32; 3] {
        match s {
            Source::Uniform(v) => *v,
            Source::Procedural { pattern, low, high } => {
                let (c, proj, _) = self.projected(proj);
                let t = procedural(pattern, proj, &c);
                mix3(*low, *high, t)
            }
            Source::Image { name, normal_format } => {
                let Some(img) = self.inp.images.get(name) else { return [0.5; 3] };
                let (c, proj, _) = self.projected(proj);
                let mut acc = [0.0f32; 4];
                for (uv, w) in image_samples(proj, &c).into_iter().filter(|(_, w)| *w > 0.0) {
                    let s = img.sample(uv.x, uv.y);
                    for k in 0..4 {
                        acc[k] += s[k] * w;
                    }
                }
                match kind {
                    ChannelKind::Color if img.srgb => [acc[0], acc[1], acc[2]].map(texturecraft_texel::srgb_to_linear),
                    // Normal channels are OpenGL (+Y up the image): a DirectX image's green flips.
                    ChannelKind::NormalMap if *normal_format == NormalFormat::DirectX => [acc[0], 1.0 - acc[1], acc[2]],
                    ChannelKind::Color | ChannelKind::NormalMap => [acc[0], acc[1], acc[2]],
                    ChannelKind::Scalar => [acc[0]; 3],
                    ChannelKind::Signed => [acc[0] * 2.0 - 1.0; 3],
                }
            }
            Source::Anchor { anchor, channel } => {
                // An anchor not evaluated yet (above this layer, hidden) reads the channel default.
                let v = self.anchors.iter().rev().find(|(id, _)| id == anchor).and_then(|(_, acc)| acc.get(channel.index()).copied());
                let v = v.unwrap_or_else(|| channel.default_value());
                let gray = matches!(channel.kind(), ChannelKind::Color | ChannelKind::NormalMap) && matches!(kind, ChannelKind::Scalar | ChannelKind::Signed);
                if gray { [lum(v); 3] } else { v }
            }
        }
    }

    fn generator(&mut self, g: &Generator) -> f32 {
        let c = self.coords();
        let s = match g.kind {
            GeneratorKind::EdgeWear => (self.baked(BakedMap::Curvature).unwrap_or(0.5) - 0.5) * 2.0,
            GeneratorKind::Curvature => self.baked(BakedMap::Curvature).unwrap_or(0.5),
            GeneratorKind::Dirt => 1.0 - self.baked(BakedMap::AmbientOcclusion).unwrap_or(1.0),
            GeneratorKind::AmbientOcclusion => self.baked(BakedMap::AmbientOcclusion).unwrap_or(1.0),
            GeneratorKind::ThinAreas => 1.0 - self.baked(BakedMap::Thickness).unwrap_or(1.0),
            GeneratorKind::Position => c.pos.dot(Vec3::from(g.direction).try_normalize().unwrap_or(Vec3::Y)) * 0.5 + 0.5,
            GeneratorKind::Direction => c.nrm.dot(Vec3::from(g.direction).try_normalize().unwrap_or(Vec3::Y)) * 0.5 + 0.5,
        };
        let s = if g.noise_amount > 0.0 { s + (world_noise(c.pos, g.noise_scale, g.seed) - 0.5) * g.noise_amount } else { s };
        g.shape(s)
    }

    fn mask(&mut self, m: &MaskPlan) -> f32 {
        let mut v = m.background;
        let mut skip = 0;
        if let Some((k, field)) = m.start
            && let Some(f) = field.get(self.x, self.y).and_then(|d| d.first().copied())
        {
            v = f;
            skip = k + 1;
        }
        for e in m.effects.iter().skip(skip) {
            let o = e.opacity.clamp(0.0, 1.0);
            match e.kind {
                EffectKind::Paint | EffectKind::Path(_) => {
                    let (mut pv, mut cov) = match e.data.as_deref() {
                        Some(d) => (from_u16(d.get(self.o * 2).copied().unwrap_or(0)), from_u16(d.get(self.o * 2 + 1).copied().unwrap_or(0))),
                        None => (0.0, 0.0),
                    };
                    if e.preview
                        && let Some(p) = self.inp.preview
                    {
                        let a = (self.stroke * p.opacity).clamp(0.0, 1.0);
                        if p.erase {
                            pv *= 1.0 - a;
                            cov *= 1.0 - a;
                        } else {
                            pv = p.mask_value * a + pv * (1.0 - a);
                            cov = a + cov * (1.0 - a);
                        }
                    }
                    if cov > 0.0 {
                        let val = pv / cov.max(1e-5);
                        v += (blend1(e.blend, v, val) - v) * cov * o;
                    }
                }
                EffectKind::Fill { source, projection } => {
                    let val = self.source(source, projection, ChannelKind::Scalar)[0];
                    v += (blend1(e.blend, v, val) - v) * o * self.cover(std::iter::once(source), projection);
                }
                EffectKind::Generator(g) => {
                    let val = self.generator(g);
                    v += (blend1(e.blend, v, val) - v) * o;
                }
                EffectKind::Levels(l) => v += (l.apply(v) - v) * o,
                EffectKind::Filter(f) => {
                    // A mask is gray (HSL only changes its lightness); neighbourhood filters come
                    // precomputed through `MaskPlan::start`.
                    let val = f.point(ChannelKind::Scalar, [v; 3]).map_or(v, |x| x[0]);
                    v += (val - v) * o;
                }
                EffectKind::ColorSelection { colors, tolerance } => {
                    let id = self.inp.baked.sample(BakedMap::Id, self.x, self.y, self.res);
                    let hit = id.is_some_and(|c| colors.iter().any(|k| (k[0] - c[0]).abs().max((k[1] - c[1]).abs()).max((k[2] - c[2]).abs()) <= *tolerance));
                    let val = if hit { 1.0 } else { 0.0 };
                    v += (blend1(e.blend, v, val) - v) * o;
                }
                EffectKind::Compare { op, value, tolerance } => {
                    let pass = match op {
                        CompareOp::LesserThan => v < *value,
                        CompareOp::GreaterThan => v > *value,
                        CompareOp::WithinTolerance => (v - value).abs() <= *tolerance,
                    };
                    let val = if pass { 1.0 } else { 0.0 };
                    v += (val - v) * o;
                }
                // Content-only effects.
                EffectKind::Material(_) | EffectKind::Anchor => {}
            }
        }
        v.clamp(0.0, 1.0)
    }

    /// Composite `nodes` over `acc`, recording anchors. Returns true when it reached the `stop`
    /// layer (`acc` then holds the stack below it).
    fn eval(&mut self, nodes: &[Node], acc: &mut Acc) -> bool {
        for n in nodes {
            if self.stop == Some(n.id) {
                return true;
            }
            let m = self.tape_mask(n);
            match &n.content {
                // A masked-out group still runs when it records anchors (they hold its inner
                // stack) or may hold the stop layer.
                Content::Group(children) if m > 0.0 || n.has_anchor || self.stop.is_some() => {
                    let mut inner = *acc;
                    if self.eval(children, &mut inner) {
                        return true;
                    }
                    if m > 0.0 {
                        mix_group(acc, &inner, &n.chans, m);
                    }
                }
                Content::Fill(_) | Content::Paint { .. } if m > 0.0 => {
                    let before = if n.anchors.iter().any(|a| a.1.is_some()) { Some(*acc) } else { None };
                    for c in &n.chans {
                        if let Some((v, cov)) = self.tape_content(&n.content, c) {
                            apply(acc, c, v, cov, m);
                        }
                    }
                    // Anchors inside the content stack: the stack with the content cut there.
                    for (a, cut) in &n.anchors {
                        let rec = match (cut, before) {
                            (Some(k), Some(mut b)) => {
                                for c in &n.chans {
                                    if let Some((v, cov)) = self.content_to(&n.content, c, *k) {
                                        apply(&mut b, c, v, cov, m);
                                    }
                                }
                                b
                            }
                            _ => *acc,
                        };
                        self.anchors.push((*a, rec));
                    }
                    continue;
                }
                _ => {}
            }
            for (a, _) in &n.anchors {
                self.anchors.push((*a, *acc));
            }
        }
        false
    }

    /// [`Texel::eval`] of the whole stack over `acc`, split along the path to the hot layer
    /// (`levels`, top level first). Cold (`warm` false) it evaluates everything, leaving in
    /// `accs` the stack before the path layer of each level and recording on `tape` the masks
    /// of the folders on the path and what the layers after the path contribute. Warm (`accs`
    /// loaded from the cache, `tape` replaying) only the hot layer is evaluated and the blends
    /// after it are replayed. The order of evaluation differs from `eval` only in masks, which
    /// read no stack values (given no anchor readers, see `BelowCache::prepare`), so the result
    /// is identical.
    fn eval_split(&mut self, mut acc: Acc, levels: &[LevelPlan], accs: &mut [Acc], masks: &mut [f32], warm: bool, tape: &mut Tape<'b>) -> Acc {
        let last = levels.len().saturating_sub(1);
        // Down the path: the deepest level the stack reaches (a folder masked out or not drawn
        // here stops it).
        let mut reach = last;
        for (d, lv) in levels.iter().enumerate() {
            if !warm {
                self.eval(&lv.below, &mut acc);
                if let Some(slot) = accs.get_mut(d) {
                    *slot = acc;
                }
            }
            if d == last {
                break;
            }
            let m = match &lv.at {
                Some(g) => self.taped(tape, |t| t.tape_mask(g)),
                None => 0.0,
            };
            if let Some(slot) = masks.get_mut(d) {
                *slot = m;
            }
            if m > 0.0 {
                continue;
            }
            reach = d;
            break;
        }
        let mut acc = accs.get(reach).copied().unwrap_or(acc);
        if reach == last
            && let Some(hot) = levels.get(last).and_then(|l| l.at.as_ref())
        {
            self.eval(std::slice::from_ref(hot), &mut acc);
        }
        // Back up: the layers after the path, then each folder's mix over the stack outside it.
        for d in (0..=reach).rev() {
            if let Some(lv) = levels.get(d) {
                self.taped(tape, |t| t.eval(&lv.above, &mut acc));
            }
            if let Some(o) = d.checked_sub(1)
                && let (Some(outer), Some(g), Some(m)) = (accs.get(o), levels.get(o).and_then(|l| l.at.as_ref()), masks.get(o))
            {
                let inner = acc;
                acc = *outer;
                mix_group(&mut acc, &inner, &g.chans, *m);
            }
        }
        acc
    }

    /// The mask of node `n` at this texel, combined with the layer's height when it masks with
    /// height.
    fn node_mask(&mut self, n: &Node) -> f32 {
        if n.parts.as_ref().is_some_and(|p| !self.in_parts(p)) {
            return 0.0;
        }
        let m = match &n.mask {
            Some(mp) => self.mask(mp),
            None => 1.0,
        };
        if !n.height_mask.enabled || m <= 0.0 || m >= 1.0 {
            return m;
        }
        let height = Channel::Height.index();
        let h = n.chans.iter().find(|c| c.ch == height).and_then(|c| self.content(&n.content, c)).map_or(0.0, |(v, _)| v[0]);
        n.height_mask.apply(m, h)
    }

    /// Whether this texel's triangle lies in a submesh `allowed` marks (uncovered texels and
    /// meshes without submeshes count as inside).
    fn in_parts(&self, allowed: &[bool]) -> bool {
        let g = self.inp.gbuf;
        let tri = g.tri.get((self.y * g.width + self.x) as usize).copied().unwrap_or(NO_TRI);
        if tri == NO_TRI {
            return true;
        }
        let first = tri.saturating_mul(3);
        let subs = &self.inp.mesh.submeshes;
        let holds = |k: usize| subs.get(k).is_some_and(|s| s.start <= first && first < s.start.saturating_add(s.count));
        // Submeshes are in index order; anything else is searched.
        let k = subs.partition_point(|s| s.start <= first).checked_sub(1).filter(|k| holds(*k)).or_else(|| (0..subs.len()).find(|k| holds(*k)));
        k.and_then(|k| allowed.get(k).copied()).unwrap_or(true)
    }

    /// Record the anchors of the stack below layer `id` (`full` is the planned stack), to
    /// evaluate that layer on its own.
    fn anchors_below(&mut self, full: &[Node], id: LayerId) {
        self.anchors.clear();
        self.stop = Some(id);
        let mut acc = Channel::ALL.map(Channel::default_value);
        self.eval(full, &mut acc);
        self.stop = None;
    }

    /// Straight value and coverage of one channel of a paint or fill layer at this texel
    /// (content effects and live stroke preview included). None where the layer is empty.
    fn content(&mut self, content: &Content, c: &ChanPlan) -> Option<([f32; 3], f32)> {
        self.content_to(content, c, usize::MAX)
    }

    /// [`Texel::content`] with only the first `end` visible content effects.
    fn content_to(&mut self, content: &Content, c: &ChanPlan, end: usize) -> Option<([f32; 3], f32)> {
        let start = c.start.filter(|(k, _)| *k < end);
        let (mut v, mut cov, skip) = match start.and_then(|(k, f)| f.get(self.x, self.y).map(|d| (k, d))) {
            Some((k, d)) => {
                let px = |i: usize| d.get(i).copied().unwrap_or(0.0);
                ([px(0), px(1), px(2)], px(3), k + 1)
            }
            None => {
                let (v, cov) = match content {
                    Content::Fill(f) => {
                        let ch = Channel::ALL.get(c.ch).copied()?;
                        // A channel without a source fills with its default (no clone per texel).
                        let v = match f.sources.get(&ch) {
                            Some(s) => self.source(s, &f.projection, ch.kind()),
                            None => ch.default_value(),
                        };
                        (v, self.cover(f.sources.values(), &f.projection))
                    }
                    Content::Paint { preview } => self.painted(c.data.as_deref(), c, *preview),
                    Content::Group(_) => return None,
                };
                (v, cov, 0)
            }
        };
        for e in c.effects.iter().take(end).skip(skip) {
            (v, cov) = self.content_effect(e, c, v, cov);
        }
        (cov > 0.0).then_some((v, cov.min(1.0)))
    }

    /// Straight value and coverage of painted data of one channel (premultiplied values plus
    /// coverage per texel), with the live stroke when `preview`.
    fn painted(&mut self, data: Option<&[u16]>, c: &ChanPlan, preview: bool) -> ([f32; 3], f32) {
        let pv = if preview { self.inp.preview } else { None };
        let mut sa = pv.map(|p| (self.stroke * p.opacity).clamp(0.0, 1.0)).unwrap_or(0.0);
        // Projection: the stencil's alpha shapes the stroke and its color is the base color.
        let stencil = match pv.and_then(|p| p.stencil.as_ref()) {
            Some(st) if sa > 0.0 => {
                let s = self.world().and_then(|w| st.sample_world(w));
                sa *= s.map(|(_, a)| a).unwrap_or(0.0);
                s.map(|(c, _)| c)
            }
            _ => None,
        };
        let mut val = [0.0f32; 3];
        let mut cov = 0.0f32;
        if let Some(d) = data {
            let base = self.o * (c.comps + 1);
            for (k, slot) in val.iter_mut().take(c.comps).enumerate() {
                *slot = decode(c.kind, d.get(base + k).copied().unwrap_or(0));
            }
            cov = from_u16(d.get(base + c.comps).copied().unwrap_or(0));
        }
        if let Some(p) = pv
            && sa > 0.0
        {
            if p.erase {
                val = val.map(|v| v * (1.0 - sa));
                cov *= 1.0 - sa;
            } else if let Some(ch) = Channel::ALL.get(c.ch)
                && let Some(pval) = p.values.get(ch)
            {
                let pval = match stencil {
                    Some(col) if *ch == Channel::BaseColor => col,
                    _ => match p.carried.get(ch) {
                        Some(img) => {
                            let raw = img.get(self.x, self.y).map(|v| decode(c.kind, v));
                            [raw[0], raw[1], raw[2]]
                        }
                        None => *pval,
                    },
                };
                for k in 0..c.comps {
                    val[k] = pval[k] * sa + val[k] * (1.0 - sa);
                }
                cov = sa + cov * (1.0 - sa);
            }
        }
        if cov <= 0.0 {
            return ([0.0; 3], 0.0);
        }
        let inv = 1.0 / cov.max(1e-5);
        let v = if c.comps == 3 { val.map(|x| x * inv) } else { [val[0] * inv; 3] };
        (v, cov.min(1.0))
    }

    /// Apply one content effect to a channel's straight value and coverage.
    fn content_effect(&mut self, e: &EffectPlan, c: &ChanPlan, v: [f32; 3], cov: f32) -> ([f32; 3], f32) {
        let o = e.opacity.clamp(0.0, 1.0);
        match e.kind {
            EffectKind::Paint | EffectKind::Path(_) => {
                let (p, pc) = self.painted(e.data.as_deref(), c, e.preview);
                over(c, e.blend, v, cov, p, pc * o)
            }
            EffectKind::Material(f) => match Channel::ALL.get(c.ch).and_then(|ch| f.sources.get(ch).map(|s| (*ch, s))) {
                Some((ch, s)) => {
                    let p = self.source(s, &f.projection, ch.kind());
                    let w = o * self.cover(f.sources.values(), &f.projection);
                    over(c, e.blend, v, cov, p, w)
                }
                None => (v, cov),
            },
            EffectKind::Levels(l) => (mix3(v, per_unit(c.kind, v, |x| l.apply(x)), o), cov),
            // Baked lighting reads the texel's normal and occlusion, and lights the base color only.
            EffectKind::Filter(f @ Filter::BakedLighting { .. }) => match Channel::ALL.get(c.ch) {
                Some(Channel::BaseColor) => {
                    let nrm = self.coords().nrm.to_array();
                    let k = f.baked_light(nrm, self.baked(BakedMap::AmbientOcclusion).unwrap_or(1.0)).unwrap_or(1.0);
                    (mix3(v, v.map(|x| (x * k).clamp(0.0, 1.0)), o), cov)
                }
                _ => (v, cov),
            },
            // Neighbourhood filters come precomputed through `ChanPlan::start`.
            EffectKind::Filter(f) => (f.point(c.kind, v).map_or(v, |out| mix3(v, out, o)), cov),
            // Anchors are recorded by `eval`; mask-only effects leave content as it is.
            EffectKind::Anchor | EffectKind::Fill { .. } | EffectKind::Generator(_) | EffectKind::ColorSelection { .. } | EffectKind::Compare { .. } => {
                (v, cov)
            }
        }
    }
}

/// Mix a folder's inner stack into the stack outside it by its opacity and mask `m`.
fn mix_group(acc: &mut Acc, inner: &Acc, chans: &[ChanPlan], m: f32) {
    for c in chans {
        if let (Some(a), Some(b)) = (acc.get(c.ch).copied(), inner.get(c.ch).copied())
            && let Some(slot) = acc.get_mut(c.ch)
        {
            *slot = mix3(a, b, c.opacity * m);
        }
    }
}

#[inline]
fn apply(acc: &mut Acc, c: &ChanPlan, v: [f32; 3], cov: f32, mask: f32) {
    let Some(a) = acc.get(c.ch).copied() else { return };
    let w = if c.blend == BlendMode::Replace { c.opacity * mask } else { cov * c.opacity * mask };
    let blended = blend_chan(c, c.blend, a, v);
    if let Some(slot) = acc.get_mut(c.ch) {
        *slot = mix3(a, blended, w);
    }
}

/// Composite `tiles` of texture set `inp.ts` into `out` (other tiles are left as they are).
pub fn composite_tiles(inp: &Inputs, out: &mut Composite, tiles: &[(u32, u32)]) {
    composite_tiles_cached(inp, out, tiles, None);
}

/// Tiles per side of the blocks a composite works through (bounds neighbourhood-filter memory).
const BLOCK: u32 = 16;

/// [`composite_tiles`], serving everything but the hot layer from `below` where it can (and
/// filling it). Results are identical to [`composite_tiles`].
pub fn composite_tiles_cached(inp: &Inputs, out: &mut Composite, tiles: &[(u32, u32)], mut below: Option<&mut BelowCache>) {
    let Ok(set) = inp.project.texture_set(inp.ts) else { return };
    if (inp.gbuf.width, inp.gbuf.height) != (out.resolution, out.height) {
        return;
    }
    let split = below.as_deref_mut().and_then(|b| b.prepare(inp));
    // A pass over more tiles than the cache holds records only what fits: recording the rest
    // would just evict it again. Tiles hold the stack per level plus a tape of varying size.
    let levels = split.as_ref().map_or(1, Vec::len);
    let estimate = TILE_AREA * levels * set.channel_list().iter().map(|c| c.components()).sum::<usize>() * std::mem::size_of::<f32>();
    let tile_bytes = below.as_deref().and_then(BelowCache::tile_bytes).unwrap_or(estimate).max(1);
    let fits = below.as_deref().is_some_and(|b| tiles.len().max(b.pass) <= b.limit() / tile_bytes);
    let mut blocks: BTreeMap<(u32, u32), Vec<(u32, u32)>> = BTreeMap::new();
    for t in tiles {
        blocks.entry((t.1 / BLOCK, t.0 / BLOCK)).or_default().push(*t);
    }
    for block in blocks.values() {
        let quota = match below.as_deref() {
            Some(_) if fits => usize::MAX,
            Some(b) => b.limit().saturating_sub(b.bytes()) / tile_bytes,
            None => 0,
        };
        let cached = below.as_deref().zip(split.as_deref());
        let records = composite_block(inp, &set.stack, out, block, cached, quota);
        if let Some(b) = below.as_deref_mut() {
            for t in block {
                b.touch(*t);
            }
            for (t, bands) in records {
                b.insert(t, bands);
            }
        }
    }
}

/// Rows of a tile composited as one parallel task.
const BAND: u32 = 16;

/// One band of a tile: per-channel buffers, what to cache of it (when recorded) and whether
/// any texel is covered.
type BandResult = (Vec<Vec<u16>>, Option<Band>, bool);

/// Per-channel tiles of one composited tile and whether any texel is covered.
type Joined = (Vec<Option<std::sync::Arc<Vec<u16>>>>, bool);

/// Composite one block of tiles, split along the path to the hot layer when `below` holds it;
/// returns what to cache (for at most `quota` tiles).
fn composite_block(
    inp: &Inputs,
    stack: &[LayerId],
    out: &mut Composite,
    tiles: &[(u32, u32)],
    below: Option<(&BelowCache, &[Level])>,
    quota: usize,
) -> Vec<((u32, u32), Vec<Band>)> {
    let channels: Vec<Channel> = inp.project.texture_set(inp.ts).map(|s| s.channel_list()).unwrap_or_default();
    let (w, h) = (out.resolution, out.height);
    let split = below.map(|(_, l)| l);
    let cached = |t: (u32, u32)| below.and_then(|(b, _)| b.tile(t));
    let fresh: Vec<(u32, u32)> = tiles.iter().copied().filter(|t| cached(*t).is_none()).collect();
    let recorded: std::collections::BTreeSet<(u32, u32)> = if split.is_some() { fresh.iter().copied().take(quota).collect() } else { Default::default() };
    // Off the path, layers are only evaluated on tiles the cache lacks.
    let fields = match split.and_then(<[Level]>::last) {
        Some(hot) if fresh.len() < tiles.len() => {
            let mut fields = fields_of(inp, &channels, stack, &fresh);
            fields.extend(fields_of(inp, &channels, &[hot.at], tiles));
            fields
        }
        _ => fields_of(inp, &channels, stack, tiles),
    };
    let lanes: Vec<Lane> = channels.iter().map(|c| Lane { ch: c.index(), n: c.components(), signed: c.kind() == ChannelKind::Signed }).collect();
    let stride: usize = lanes.iter().map(|l| l.n).sum();
    let nlev = split.map_or(0, <[Level]>::len);
    let defaults: Acc = Channel::ALL.map(Channel::default_value);
    // Tiles split into bands of rows, so a handful of tiles still keeps every thread busy.
    let bands = TILE / BAND;
    let tasks: Vec<((u32, u32), u32)> = tiles.iter().flat_map(|t| (0..bands).map(move |b| (*t, b))).collect();
    let mut results: Vec<BandResult> = tasks
        .par_iter()
        .map(|&((tx, ty), band)| {
            let warm = cached((tx, ty)).and_then(|b| b.get(band as usize));
            let record = warm.is_none() && recorded.contains(&(tx, ty));
            let levels = split.map(|l| plan_levels(inp, l, &channels, tx, ty, &fields, warm.is_some()));
            let nodes = if levels.is_none() { plan(inp, stack, &channels, tx, ty, 0, Some(&fields)) } else { Vec::new() };
            let rows = (BAND * TILE) as usize;
            let mut accs = vec![defaults; nlev];
            let mut masks = vec![0.0f32; nlev];
            let mut tape = match warm {
                Some(b) => Tape::Replay(b, 0),
                None if record => Tape::Record(Vec::new()),
                None => Tape::Off,
            };
            // The levels whose stack the band stores (its header): a level with nothing drawn
            // before the path layer repeats the level above it.
            let stored: Vec<bool> = match (&levels, warm) {
                (Some(_), Some(_)) => tape.take(nlev).iter().map(|v| *v > 0.0).collect(),
                (Some(levels), None) => levels.iter().map(|l| !l.below.is_empty()).collect(),
                (None, _) => Vec::new(),
            };
            if let Tape::Record(t) = &mut tape {
                t.extend(stored.iter().map(|s| if *s { 1.0 } else { 0.0 }));
            }
            let slots = stored.iter().filter(|s| **s).count() * stride;
            let preview_tile = inp.preview.and_then(|p| p.alpha.tile(tx, ty));
            let mut bufs: Vec<Vec<u16>> = channels.iter().map(|c| vec![0u16; rows * c.components()]).collect();
            let mut anchors = Vec::new();
            let mut any = false;
            for ly in band * BAND..(band + 1) * BAND {
                let y = ty * TILE + ly;
                if y >= h {
                    break;
                }
                for lx in 0..TILE {
                    let x = tx * TILE + lx;
                    if x >= w {
                        break;
                    }
                    let gi = (y * w + x) as usize;
                    // Offset in the tile, and in this band.
                    let o = (ly * TILE + lx) as usize;
                    let ob = ((ly - band * BAND) * TILE + lx) as usize;
                    let covered = inp.gbuf.tri.get(gi).is_some_and(|t| *t != NO_TRI);
                    let mut acc = defaults;
                    if covered {
                        any = true;
                        let mut tex = Texel::new(inp, x, y, o, stroke_at(preview_tile, o));
                        // Reuse one anchor buffer per band.
                        tex.anchors = std::mem::take(&mut anchors);
                        match &levels {
                            Some(levels) => {
                                // The texel's stream starts with the stack at each level.
                                let start = match &mut tape {
                                    Tape::Record(t) => {
                                        t.resize(t.len() + slots, 0.0);
                                        t.len() - slots
                                    }
                                    _ => 0,
                                };
                                if warm.is_some() {
                                    load_levels(tape.take(slots), &lanes, stride, &stored, &defaults, &mut accs);
                                }
                                acc = tex.eval_split(defaults, levels, &mut accs, &mut masks, warm.is_some(), &mut tape);
                                if let Tape::Record(t) = &mut tape
                                    && let Some(below) = t.get_mut(start..start + slots)
                                {
                                    store_levels(below, &lanes, stride, &stored, &accs);
                                }
                            }
                            None => {
                                tex.eval(&nodes, &mut acc);
                            }
                        }
                        anchors = tex.anchors;
                        anchors.clear();
                    }
                    for (l, buf) in lanes.iter().zip(&mut bufs) {
                        let v = acc.get(l.ch).copied().unwrap_or([0.0; 3]);
                        for (k, x) in v.iter().take(l.n).enumerate() {
                            let enc = if l.signed { to_u16_signed(*x) } else { to_u16(*x) };
                            if let Some(slot) = buf.get_mut(ob * l.n + k) {
                                *slot = enc;
                            }
                        }
                    }
                }
            }
            let cache = if let Tape::Record(t) = tape { Some(t) } else { None };
            (bufs, cache, any)
        })
        .collect();
    // Join the bands of each tile (in parallel; uncovered tiles are freed).
    let joined: Vec<Joined> = results
        .par_chunks(bands as usize)
        .map(|parts| {
            let any = parts.iter().any(|p| p.2);
            let data = (0..channels.len())
                .map(|k| {
                    any.then(|| {
                        let mut v = Vec::with_capacity(parts.iter().map(|p| p.0.get(k).map_or(0, Vec::len)).sum());
                        for p in parts {
                            v.extend_from_slice(p.0.get(k).map_or(&[][..], Vec::as_slice));
                        }
                        std::sync::Arc::new(v)
                    })
                })
                .collect();
            (data, any)
        })
        .collect();
    let mut records = Vec::new();
    for ((&(tx, ty), (data, any)), parts) in tiles.iter().zip(joined).zip(results.chunks_mut(bands as usize)) {
        for (c, d) in channels.iter().zip(data) {
            if let Some(img) = out.channels.get_mut(c) {
                img.set_tile(tx, ty, d);
            }
        }
        // Tiles without covered texels cost nothing to composite: not worth caching.
        if any && let Some(bands) = parts.iter_mut().map(|p| p.1.take()).collect::<Option<Vec<Band>>>() {
            records.push(((tx, ty), bands));
        }
    }
    records
}

/// Restore the stack at each nesting level of a texel: the `stored` levels from `src`, the
/// others repeat the level above them (the defaults at the top).
fn load_levels(src: &[f32], lanes: &[Lane], stride: usize, stored: &[bool], defaults: &Acc, accs: &mut [Acc]) {
    let (mut k, mut above) = (0, *defaults);
    for (a, s) in accs.iter_mut().zip(stored) {
        if *s {
            load_below(src, lanes, stride, k, a);
            k += 1;
        } else {
            *a = above;
        }
        above = *a;
    }
}

/// Store the `stored` levels of a texel's stack (see [`load_levels`]).
fn store_levels(dst: &mut [f32], lanes: &[Lane], stride: usize, stored: &[bool], accs: &[Acc]) {
    for (k, a) in accs.iter().zip(stored).filter(|(_, s)| **s).map(|(a, _)| a).enumerate() {
        store_below(dst, lanes, stride, k, a);
    }
}

/// Where a texture set channel lives in a stack value, its components and whether it is signed.
#[derive(Clone, Copy)]
struct Lane {
    ch: usize,
    n: usize,
    signed: bool,
}

/// Restore the cached stack value of slot `o` (a texel at one nesting level). Scalars are
/// cached once: only their first component is ever read.
fn load_below(cache: &[f32], lanes: &[Lane], stride: usize, o: usize, acc: &mut Acc) {
    let mut i = o * stride;
    for l in lanes {
        if let (Some(src), Some(slot)) = (cache.get(i..i + l.n), acc.get_mut(l.ch)) {
            *slot = match src {
                [a, b, c] => [*a, *b, *c],
                _ => [src.first().copied().unwrap_or(0.0); 3],
            };
        }
        i += l.n;
    }
}

fn store_below(cache: &mut [f32], lanes: &[Lane], stride: usize, o: usize, acc: &Acc) {
    let mut i = o * stride;
    for l in lanes {
        if let (Some(dst), Some(v)) = (cache.get_mut(i..i + l.n), acc.get(l.ch)) {
            for (d, x) in dst.iter_mut().zip(v) {
                *d = *x;
            }
        }
        i += l.n;
    }
}

/// UV padding for display: fill uncovered texels in `tiles` and their neighbours from their
/// nearest covered texel. Returns the tiles it wrote.
pub fn pad_tiles(out: &mut Composite, dmap: &DilationMap, tiles: &[(u32, u32)]) -> std::collections::BTreeSet<(u32, u32)> {
    let mut written = std::collections::BTreeSet::new();
    let (w, h) = (out.resolution, out.height);
    if (dmap.width, dmap.height) != (w, h) {
        return written;
    }
    let (nx, ny) = (w.div_ceil(TILE) as i64, h.div_ceil(TILE) as i64);
    let mut set = std::collections::BTreeSet::new();
    for &(tx, ty) in tiles {
        for dy in -1..=1i64 {
            for dx in -1..=1i64 {
                let (x, y) = (tx as i64 + dx, ty as i64 + dy);
                if x >= 0 && y >= 0 && x < nx && y < ny {
                    set.insert((x as u32, y as u32));
                }
            }
        }
    }
    let set: Vec<(u32, u32)> = set.into_iter().collect();
    for img in out.channels.values_mut() {
        let comps = img.comps();
        let src: &TileImage = img;
        // Padding writes only uncovered texels and reads only covered ones, so tiles pad
        // independently (in parallel) from the unpadded image.
        let padded: Vec<((u32, u32), Vec<u16>)> = set
            .par_iter()
            .filter_map(|&(tx, ty)| {
                let mut data: Option<Vec<u16>> = None;
                for ly in 0..TILE {
                    for lx in 0..TILE {
                        let (x, y) = (tx * TILE + lx, ty * TILE + ly);
                        if x >= w || y >= h {
                            continue;
                        }
                        let Some((sx, sy)) = dmap.source(x, y) else { continue };
                        if (sx, sy) == (x, y) {
                            continue;
                        }
                        let v = src.get(sx, sy);
                        let d = data.get_or_insert_with(|| src.tile(tx, ty).map_or_else(|| vec![0; TILE_AREA * comps], <[u16]>::to_vec));
                        let o = (ly * TILE + lx) as usize * comps;
                        if let Some(px) = d.get_mut(o..o + comps) {
                            px.copy_from_slice(&v[..comps]);
                        }
                    }
                }
                Some(((tx, ty), data?))
            })
            .collect();
        for ((tx, ty), d) in padded {
            img.set_tile(tx, ty, Some(std::sync::Arc::new(d)));
            written.insert((tx, ty));
        }
    }
    written
}

/// The mask of layer `id` as an 8-bit grayscale image of `size × size` stretched over the
/// texture set (nearest texel; uncovered texels black). None when the layer has no enabled mask.
pub fn mask_image(inp: &Inputs, id: LayerId, size: u32) -> Option<Vec<u8>> {
    let mask = layer_mask(inp.project, id).filter(|m| m.enabled)?;
    let channels = inp.project.texture_set(inp.ts).map(|t| t.channel_list()).unwrap_or_default();
    let (w, h) = (inp.gbuf.width, inp.gbuf.height);
    let all = all_tiles(w, h);
    let fields = neighbourhood_fields(inp, &channels, &all);
    let tiles: Vec<(u32, u32, Vec<u8>)> = all
        .par_iter()
        .filter_map(|&(tx, ty)| {
            let mp = mask_plan(inp, id, mask, tx, ty, Some(&fields))?;
            let full = anchor_plan(inp, id, &channels, tx, ty, Some(&fields));
            let mut buf = vec![0u8; TILE_AREA];
            for (x, y, o) in tile_texels(inp.gbuf, tx, ty) {
                let mut tex = Texel::new(inp, x, y, o, 0.0);
                if !full.is_empty() {
                    tex.anchors_below(&full, id);
                }
                if let Some(b) = buf.get_mut(o) {
                    *b = (tex.mask(&mp) * 255.0 + 0.5) as u8;
                }
            }
            Some((tx, ty, buf))
        })
        .collect();
    let size = size.max(1);
    let mut out = vec![0u8; (size * size) as usize];
    let lookup: std::collections::HashMap<(u32, u32), &Vec<u8>> = tiles.iter().map(|(x, y, b)| ((*x, *y), b)).collect();
    for y in 0..size {
        for x in 0..size {
            let (sx, sy) = ((x as u64 * w as u64 / size as u64) as u32, (y as u64 * h as u64 / size as u64) as u32);
            if let Some(t) = lookup.get(&(sx / TILE, sy / TILE))
                && let Some(v) = t.get(((sy % TILE) * TILE + sx % TILE) as usize)
                && let Some(d) = out.get_mut((y * size + x) as usize)
            {
                *d = *v;
            }
        }
    }
    Some(out)
}

/// Tiles [`effect_histogram`] samples at most (a million texels).
const HISTOGRAM_TILES: usize = 256;

/// The tiles [`effect_histogram`] reads: an even sample of at most [`HISTOGRAM_TILES`], which
/// keeps the cost flat up to 8K. The stride is odd: a power of two would divide the row length
/// and sample the same few columns.
fn histogram_tiles(width: u32, height: u32) -> Vec<(u32, u32)> {
    let all = all_tiles(width, height);
    let stride = all.len().div_ceil(HISTOGRAM_TILES).max(1) | 1;
    all.into_iter().step_by(stride).collect()
}

/// Histogram (`bins` buckets over 0..1) of the values entering effect `effect`: its layer's mask
/// below it, or (a content effect) the layer's `channel` below it where the layer has content,
/// color components pooled and signed values mapped to 0..1. Over the texels the mesh covers
/// (in an even sample of tiles on large sets); None for an unknown effect.
pub fn effect_histogram(inp: &Inputs, effect: LayerId, channel: Channel, bins: usize) -> Option<Vec<u32>> {
    let (owner, _) = inp.project.effect(effect)?;
    let bins = bins.max(1);
    // Evaluate the layer forced visible, its mask enabled, cut just below the effect.
    let mut cut = inp.project.clone();
    let l = cut.layers.get_mut(&owner)?;
    l.visible = true;
    let in_mask = match l.mask.as_mut().and_then(|m| m.effects.iter().position(|e| e.id == effect).map(|k| (m, k))) {
        Some((m, k)) => {
            m.enabled = true;
            m.effects.truncate(k);
            true
        }
        None => {
            let k = l.content_effects.iter().position(|e| e.id == effect)?;
            l.content_effects.truncate(k);
            false
        }
    };
    let cut_inp = Inputs { project: &cut, preview: None, ..*inp };
    let channels = cut.texture_set(inp.ts).map(|t| t.channel_list()).unwrap_or_default();
    let tiles = histogram_tiles(inp.gbuf.width, inp.gbuf.height);
    // A field covers the bounding box of the tiles it is built for, which for a scattered sample
    // is the whole set: with a neighbourhood filter below, each tile gets its own.
    let probe = neighbourhood_fields(&cut_inp, &channels, tiles.get(..1).unwrap_or_default());
    let shared = probe.is_empty().then_some(probe);
    let bin = |v: f32| ((v.clamp(0.0, 1.0) * bins as f32) as usize).min(bins - 1);
    let hist = tiles
        .par_iter()
        .map(|&(tx, ty)| {
            let own;
            let fields = match &shared {
                Some(f) => f,
                None => {
                    own = neighbourhood_fields(&cut_inp, &channels, &[(tx, ty)]);
                    &own
                }
            };
            let mut h = vec![0u32; bins];
            // The mask is planned on its own: a layer with nothing drawn plans no node.
            let mask = if in_mask { layer_mask(&cut, owner).and_then(|m| mask_plan(&cut_inp, owner, m, tx, ty, Some(fields))) } else { None };
            let nodes = if in_mask { Vec::new() } else { plan(&cut_inp, &[owner], &channels, tx, ty, 0, Some(fields)) };
            let content = nodes.first().and_then(|n| n.chans.iter().find(|c| Channel::ALL.get(c.ch) == Some(&channel)).map(|c| (n, c)));
            if mask.is_none() && content.is_none() {
                return h;
            }
            let full = anchor_plan(&cut_inp, owner, &channels, tx, ty, Some(fields));
            for (x, y, o) in tile_texels(inp.gbuf, tx, ty) {
                let mut tex = Texel::new(&cut_inp, x, y, o, 0.0);
                if !full.is_empty() {
                    tex.anchors_below(&full, owner);
                }
                if let Some(m) = &mask {
                    if let Some(b) = h.get_mut(bin(tex.mask(m))) {
                        *b += 1;
                    }
                } else if let Some((n, c)) = content
                    && let Some((v, _)) = tex.content(&n.content, c)
                {
                    // As levels reads them (`per_unit`); normal maps pass through levels unchanged.
                    let (vals, signed) = match c.kind {
                        ChannelKind::Color => (&v[..], false),
                        ChannelKind::Scalar => (&v[..1], false),
                        ChannelKind::Signed => (&v[..1], true),
                        ChannelKind::NormalMap => (&v[..0], false),
                    };
                    for x in vals {
                        if let Some(b) = h.get_mut(bin(if signed { x * 0.5 + 0.5 } else { *x })) {
                            *b += 1;
                        }
                    }
                }
            }
            h
        })
        .reduce(|| vec![0u32; bins], |a, b| a.iter().zip(&b).map(|(x, y)| x.saturating_add(*y)).collect());
    Some(hist)
}

/// One flattened tile of one channel: (channel, tile, premultiplied value + coverage data).
pub type FlatTile = (Channel, (u32, u32), Vec<u16>);

/// Flatten one paint or fill layer: its content (with content effects) times its mask, as
/// premultiplied (value, coverage) tiles per enabled channel, ready to become a plain paint
/// layer. Groups are not supported (None).
pub fn flatten_layer(inp: &Inputs, id: LayerId, tiles: &[(u32, u32)]) -> Option<Vec<FlatTile>> {
    let set = inp.project.texture_set(inp.ts).ok()?;
    if inp.project.content_layer(id)?.is_group() {
        return None;
    }
    let channels: Vec<Channel> = set.channel_list();
    // Plan the layer forced visible so hidden layers flatten too.
    let mut solo = inp.project.clone();
    if let Some(l) = solo.layers.get_mut(&id) {
        l.visible = true;
    }
    let solo_inp = Inputs { project: &solo, preview: None, ..*inp };
    let fields = neighbourhood_fields(&solo_inp, &channels, tiles);
    let out: Vec<Vec<FlatTile>> = tiles
        .par_iter()
        .map(|&(tx, ty)| {
            let nodes = plan(&solo_inp, &[id], &channels, tx, ty, 0, Some(&fields));
            let Some(n) = nodes.first() else { return Vec::new() };
            let full = anchor_plan(&solo_inp, id, &channels, tx, ty, Some(&fields));
            let mut bufs: Vec<(Channel, Vec<u16>, bool)> =
                n.chans.iter().filter_map(|c| Channel::ALL.get(c.ch).map(|ch| (*ch, vec![0u16; TILE_AREA * (c.comps + 1)], false))).collect();
            for (ch, buf, _) in &mut bufs {
                if ch.kind() == ChannelKind::Signed {
                    for px in buf.chunks_exact_mut(2) {
                        px[0] = to_u16_signed(0.0);
                    }
                }
            }
            for (x, y, o) in tile_texels(inp.gbuf, tx, ty) {
                let mut tex = Texel::new(&solo_inp, x, y, o, 0.0);
                if !full.is_empty() {
                    tex.anchors_below(&full, id);
                }
                let m = tex.node_mask(n);
                for (c, (ch, buf, any)) in n.chans.iter().zip(bufs.iter_mut()) {
                    let Some((v, cov)) = tex.content(&n.content, c) else { continue };
                    let a = (cov * m).clamp(0.0, 1.0);
                    if a <= 0.0 {
                        continue;
                    }
                    *any = true;
                    let comps = c.comps;
                    let Some(px) = buf.get_mut(o * (comps + 1)..(o + 1) * (comps + 1)) else { continue };
                    for (slot, pv) in px.iter_mut().zip(v.iter().take(comps)) {
                        *slot = if ch.kind() == ChannelKind::Signed { to_u16_signed(pv * a) } else { to_u16(pv * a) };
                    }
                    if let Some(slot) = px.get_mut(comps) {
                        *slot = to_u16(a);
                    }
                }
            }
            bufs.into_iter().filter(|(_, _, any)| *any).map(|(ch, buf, _)| (ch, (tx, ty), buf)).collect()
        })
        .collect();
    Some(out.into_iter().flatten().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_core::{ChannelBlend, Effect, Layer, Procedural, ProceduralKind, TextureSet};
    use texturecraft_geom::primitives;

    struct Fixture {
        project: Project,
        mesh: Mesh,
        gbuf: UvGBuffer,
        pixels: PixelStore,
        baked: BakedMaps,
    }

    fn fixture() -> Fixture {
        let mesh = primitives::plane(2.0, 2);
        let gbuf = UvGBuffer::new(&mesh, 0, 64, 64);
        let mut project = Project::new("t", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, 64));
        Fixture { project, mesh, gbuf, pixels: PixelStore::default(), baked: BakedMaps::default() }
    }

    fn run(f: &Fixture, preview: Option<&StrokePreview>) -> Composite {
        let images = Default::default();
        let inp = Inputs {
            project: &f.project,
            ts: 0,
            mesh: &f.mesh,
            gbuf: &f.gbuf,
            pixels: &f.pixels,
            baked: &f.baked,
            preview,
            images: &images,
            bounds: f.mesh.bounds(),
        };
        let mut out = Composite::new(&f.project.texture_sets[0].channel_list(), 64);
        composite_tiles(&inp, &mut out, &all_tiles(64, 64));
        out
    }

    fn fill_layer(p: &mut Project, color: [f32; 3], rough: f32) -> LayerId {
        let id = p.alloc_id();
        let mut fill = Fill::default();
        fill.sources.insert(Channel::BaseColor, Source::Uniform(color));
        fill.sources.insert(Channel::Roughness, Source::Uniform([rough; 3]));
        let l = Layer::new(id, "Fill", LayerKind::Fill(fill), &[Channel::BaseColor, Channel::Roughness]);
        p.insert_layer(0, l, None, false).unwrap()
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 2e-4)
    }

    #[test]
    fn histogram_samples_spread_over_every_column() {
        assert_eq!(histogram_tiles(1024, 1024), all_tiles(1024, 1024), "small sets are read whole");
        for size in [2048, 4096, 8192] {
            let tiles = histogram_tiles(size, size);
            let columns: std::collections::BTreeSet<u32> = tiles.iter().map(|t| t.0).collect();
            assert!(tiles.len() <= HISTOGRAM_TILES, "{size}: {} tiles", tiles.len());
            assert!(columns.len() as u32 >= size / TILE / 2, "{size}: only {} columns", columns.len());
        }
    }

    #[test]
    fn empty_stack_gives_channel_defaults() {
        let f = fixture();
        let out = run(&f, None);
        assert!(close(out.value(Channel::BaseColor, 10, 10), [0.5; 3]));
        assert!(close(out.value(Channel::Roughness, 10, 10), [0.5; 3]));
        assert!(close(out.value(Channel::Normal, 10, 10), [0.5, 0.5, 1.0]));
    }

    #[test]
    fn fill_layers_blend_with_opacity_and_modes() {
        let mut f = fixture();
        fill_layer(&mut f.project, [1.0, 0.0, 0.0], 0.2);
        let top = fill_layer(&mut f.project, [0.0, 0.0, 1.0], 0.8);
        f.project.layer_mut(top).unwrap().channels.insert(Channel::BaseColor, ChannelBlend { mode: BlendMode::Normal, opacity: 0.25 });
        let out = run(&f, None);
        assert!(close(out.value(Channel::BaseColor, 5, 5), [0.75, 0.0, 0.25]));
        assert!(close(out.value(Channel::Roughness, 5, 5), [0.8; 3]));
        f.project.layer_mut(top).unwrap().channels.insert(Channel::BaseColor, ChannelBlend { mode: BlendMode::Multiply, opacity: 1.0 });
        let out = run(&f, None);
        assert!(close(out.value(Channel::BaseColor, 5, 5), [0.0, 0.0, 0.0]));
    }

    #[test]
    fn paint_layers_unpremultiply_and_preview_and_masks() {
        let mut f = fixture();
        let id = f.project.alloc_id();
        let l = Layer::new(id, "Paint", LayerKind::Paint, &[Channel::BaseColor]);
        f.project.insert_layer(0, l, None, false).unwrap();
        // Half-covered red texel: premul (0.5, 0, 0), coverage 0.5.
        let img = f.pixels.layer_mut(id, Channel::BaseColor, (64, 64));
        img.set(3, 3, &[to_u16(0.5), 0, 0, to_u16(0.5)]);
        let out = run(&f, None);
        assert!(close(out.value(Channel::BaseColor, 3, 3), [0.75, 0.25, 0.25]));
        assert!(close(out.value(Channel::BaseColor, 4, 3), [0.5; 3]));

        // A live stroke painting blue at full alpha on texel (4, 3).
        let mut alpha = TileImage::new(64, 64, 1);
        alpha.set(4, 3, &[to_u16(1.0)]);
        let pv = StrokePreview {
            target: PaintTarget::Layer(id),
            alpha,
            opacity: 1.0,
            erase: false,
            values: [(Channel::BaseColor, [0.0, 0.0, 1.0])].into_iter().collect(),
            mask_value: 1.0,
            stencil: None,
            carried: BTreeMap::new(),
        };
        let out = run(&f, Some(&pv));
        assert!(close(out.value(Channel::BaseColor, 4, 3), [0.0, 0.0, 1.0]));

        // A black mask hides the layer.
        f.project.layer_mut(id).unwrap().mask = Some(Mask::new(MaskBackground::Black));
        let out = run(&f, None);
        assert!(close(out.value(Channel::BaseColor, 3, 3), [0.5; 3]));
    }

    #[test]
    fn mask_effects_and_procedural_fills_stay_in_range() {
        let mut f = fixture();
        let id = fill_layer(&mut f.project, [1.0, 1.0, 1.0], 1.0);
        let eid = f.project.alloc_id();
        let mut mask = Mask::new(MaskBackground::Black);
        let pattern = Procedural::new(ProceduralKind::Checker);
        mask.effects.push(Effect::new(
            eid,
            EffectKind::Fill { source: Source::Procedural { pattern, low: [0.0; 3], high: [1.0; 3] }, projection: Projection::default() },
        ));
        f.project.layer_mut(id).unwrap().mask = Some(mask);
        let out = run(&f, None);
        let vals: Vec<f32> = (0..64).map(|x| out.value(Channel::BaseColor, x, 20)[0]).collect();
        assert!(vals.iter().any(|v| *v > 0.99) && vals.iter().any(|v| *v < 0.51), "{vals:?}");
    }

    #[test]
    fn blur_filter_softens_mask_edges() {
        let mut f = fixture();
        let id = fill_layer(&mut f.project, [1.0, 1.0, 1.0], 1.0);
        let mut mask = Mask::new(MaskBackground::Black);
        let pattern = Procedural::new(ProceduralKind::Checker);
        let (e1, e2) = (f.project.alloc_id(), f.project.alloc_id());
        mask.effects.push(Effect::new(
            e1,
            EffectKind::Fill { source: Source::Procedural { pattern, low: [0.0; 3], high: [1.0; 3] }, projection: Projection::default() },
        ));
        f.project.layer_mut(id).unwrap().mask = Some(mask.clone());
        let hard: Vec<f32> = (0..64).map(|x| run(&f, None).value(Channel::BaseColor, x, 20)[0]).collect();
        mask.effects.push(Effect::new(e2, EffectKind::Filter(Filter::Blur { radius: 64.0 })));
        f.project.layer_mut(id).unwrap().mask = Some(mask);
        let soft_img = run(&f, None);
        let soft: Vec<f32> = (0..64).map(|x| soft_img.value(Channel::BaseColor, x, 20)[0]).collect();
        let mids = |v: &[f32]| v.iter().filter(|x| **x > 0.55 && **x < 0.95).count();
        assert!(
            mids(&soft) > mids(&hard) + 4,
            "hard {hard:?}
soft {soft:?}"
        );
    }

    #[test]
    fn groups_pass_through_with_opacity() {
        let mut f = fixture();
        let gid = f.project.alloc_id();
        let g = Layer::new(gid, "G", LayerKind::Group { children: vec![], collapsed: false }, &[Channel::BaseColor]);
        f.project.insert_layer(0, g, None, false).unwrap();
        f.project.layer_mut(gid).unwrap().channels.insert(Channel::BaseColor, ChannelBlend { mode: BlendMode::Passthrough, opacity: 0.5 });
        let cid = f.project.alloc_id();
        let mut fill = Fill::default();
        fill.sources.insert(Channel::BaseColor, Source::Uniform([1.0, 1.0, 1.0]));
        let child = Layer::new(cid, "C", LayerKind::Fill(fill), &[Channel::BaseColor]);
        f.project.insert_layer(0, child, Some(gid), true).unwrap();
        let out = run(&f, None);
        assert!(close(out.value(Channel::BaseColor, 1, 1), [0.75; 3]));
    }

    #[test]
    fn padding_copies_nearest_covered_value() {
        let mut out = Composite::new(&[Channel::Roughness], 64);
        out.channels.get_mut(&Channel::Roughness).unwrap().set(10, 10, &[to_u16(0.9)]);
        let mut covered = vec![false; 64 * 64];
        covered[10 * 64 + 10] = true;
        let dmap = DilationMap::new(&covered, 64, 64, Some(4));
        pad_tiles(&mut out, &dmap, &[(0, 0)]);
        assert!((out.value(Channel::Roughness, 12, 12)[0] - 0.9).abs() < 1e-3);
        assert!(out.value(Channel::Roughness, 30, 30)[0] < 0.1 || !out.channels[&Channel::Roughness].is_allocated(0, 0));
    }

    fn content_fx(f: &mut Fixture, id: LayerId, kind: EffectKind) -> LayerId {
        let eid = f.project.alloc_id();
        f.project.layer_mut(id).unwrap().content_effects.push(Effect::new(eid, kind));
        eid
    }

    #[test]
    fn content_filters_hsl_and_opacity_change_layer_values() {
        let mut f = fixture();
        let id = fill_layer(&mut f.project, [1.0, 0.0, 0.0], 0.2);
        content_fx(&mut f, id, EffectKind::Filter(Filter::Hsl { hue: 1.0 / 3.0, saturation: 0.0, lightness: 0.0 }));
        let out = run(&f, None);
        assert!(close(out.value(Channel::BaseColor, 5, 5), [0.0, 1.0, 0.0]), "{:?}", out.value(Channel::BaseColor, 5, 5));
        assert!(close(out.value(Channel::Roughness, 5, 5), [0.2; 3]), "a hue shift leaves scalars alone");
        content_fx(&mut f, id, EffectKind::Filter(Filter::Invert));
        let out = run(&f, None);
        assert!(close(out.value(Channel::BaseColor, 5, 5), [1.0, 0.0, 1.0]));
        assert!(close(out.value(Channel::Roughness, 5, 5), [0.8; 3]));
        // Hidden effects are skipped and opacity fades an effect.
        let fx = &mut f.project.layer_mut(id).unwrap().content_effects;
        fx[0].visible = false;
        fx[1].opacity = 0.5;
        let out = run(&f, None);
        assert!(close(out.value(Channel::BaseColor, 5, 5), [0.5; 3]));
        assert!(close(out.value(Channel::Roughness, 5, 5), [0.5; 3]));
    }

    #[test]
    fn hsl_on_a_mask_changes_its_lightness() {
        let mut f = fixture();
        let id = fill_layer(&mut f.project, [1.0; 3], 1.0);
        let mut mask = Mask::new(MaskBackground::White);
        mask.effects.push(Effect::new(f.project.alloc_id(), EffectKind::Filter(Filter::Hsl { hue: 0.3, saturation: 1.0, lightness: -0.5 })));
        f.project.layer_mut(id).unwrap().mask = Some(mask);
        assert!(close(run(&f, None).value(Channel::BaseColor, 5, 5), [0.75; 3]));
    }

    #[test]
    fn content_blur_softens_fill_edges() {
        let mut f = fixture();
        let id = f.project.alloc_id();
        let mut fill = Fill::default();
        fill.sources.insert(Channel::Roughness, Source::Procedural { pattern: Procedural::new(ProceduralKind::Checker), low: [0.0; 3], high: [1.0; 3] });
        f.project.insert_layer(0, Layer::new(id, "F", LayerKind::Fill(fill), &[Channel::Roughness]), None, false).unwrap();
        let row = |f: &Fixture| {
            let out = run(f, None);
            (0..64).map(|x| out.value(Channel::Roughness, x, 20)[0]).collect::<Vec<f32>>()
        };
        let hard = row(&f);
        content_fx(&mut f, id, EffectKind::Filter(Filter::Blur { radius: 64.0 }));
        let soft = row(&f);
        let mids = |v: &[f32]| v.iter().filter(|x| **x > 0.05 && **x < 0.95).count();
        assert!(mids(&soft) > mids(&hard) + 4, "hard {hard:?}\nsoft {soft:?}");
        assert!(soft.iter().all(|v| (0.0..=1.0).contains(v)));
    }

    #[test]
    fn high_pass_edge_detect_directional_warp_grayscale_and_normal_invert() {
        // Uniform content: high pass is mid gray (0 for height), edge detect black.
        let mut f = fixture();
        let id = fill_layer(&mut f.project, [1.0, 0.0, 0.0], 0.2);
        let hp = content_fx(&mut f, id, EffectKind::Filter(Filter::HighPass { radius: 64.0 }));
        let out = run(&f, None);
        assert!(close(out.value(Channel::BaseColor, 30, 30), [0.5; 3]), "{:?}", out.value(Channel::BaseColor, 30, 30));
        assert!(close(out.value(Channel::Roughness, 30, 30), [0.5; 3]));
        f.project.layer_mut(id).unwrap().content_effects.retain(|e| e.id != hp);
        content_fx(&mut f, id, EffectKind::Filter(Filter::EdgeDetect { width: 64.0 }));
        assert!(close(run(&f, None).value(Channel::Roughness, 30, 30), [0.0; 3]));
        // A checker: its borders light up, the middle of its cells stays black.
        let mut f = fixture();
        let id = f.project.alloc_id();
        let mut fill = Fill::default();
        fill.sources.insert(Channel::Roughness, Source::Procedural { pattern: Procedural::new(ProceduralKind::Checker), low: [0.0; 3], high: [1.0; 3] });
        f.project.insert_layer(0, Layer::new(id, "F", LayerKind::Fill(fill), &[Channel::Roughness]), None, false).unwrap();
        let plain = run(&f, None);
        let edges = content_fx(&mut f, id, EffectKind::Filter(Filter::EdgeDetect { width: 32.0 }));
        let out = run(&f, None);
        let vals: Vec<f32> = (0..64).flat_map(|y| (0..64).map(move |x| (x, y))).map(|(x, y)| out.value(Channel::Roughness, x, y)[0]).collect();
        assert!(vals.iter().all(|v| (0.0..=1.0).contains(v)));
        assert!(vals.iter().filter(|v| **v > 0.5).count() > 64 && vals.iter().filter(|v| **v < 0.01).count() > 64, "{vals:?}");
        f.project.layer_mut(id).unwrap().content_effects.retain(|e| e.id != edges);
        content_fx(&mut f, id, EffectKind::Filter(Filter::DirectionalWarp { intensity: 256.0, angle: 90.0, scale: 4.0, seed: 3 }));
        let warped = run(&f, None);
        let differs = (0..64).any(|y| (0..64).any(|x| (warped.value(Channel::Roughness, x, y)[0] - plain.value(Channel::Roughness, x, y)[0]).abs() > 0.2));
        assert!(differs, "the warp moves the checker");
        // Point filters.
        assert!(close(Filter::Grayscale.point(ChannelKind::Color, [1.0, 0.0, 0.0]).unwrap(), [lum([1.0, 0.0, 0.0]); 3]));
        assert_eq!(Filter::Grayscale.point(ChannelKind::Scalar, [0.3; 3]), Some([0.3; 3]));
        assert_eq!(Filter::NormalInvert.point(ChannelKind::NormalMap, [0.5, 0.25, 1.0]), Some([0.5, 0.75, 1.0]));
        assert_eq!(Filter::NormalInvert.point(ChannelKind::Color, [0.5, 0.25, 1.0]), Some([0.5, 0.25, 1.0]));
        // Replace color: near colors shift (keeping their offset), far ones and scalars stay.
        let rc = Filter::ReplaceColor { from: [0.8, 0.1, 0.1], to: [0.1, 0.1, 0.8], tolerance: 0.1, softness: 0.1 };
        assert!(close(rc.point(ChannelKind::Color, [0.85, 0.1, 0.1]).unwrap(), [0.15, 0.1, 0.8]));
        assert!(close(rc.point(ChannelKind::Color, [0.1, 0.8, 0.1]).unwrap(), [0.1, 0.8, 0.1]));
        let half = rc.point(ChannelKind::Color, [0.8, 0.1 + 0.15 * 3f32.sqrt(), 0.1]).unwrap();
        assert!((half[2] - 0.45).abs() < 0.02, "halfway through the softness: {half:?}");
        assert_eq!(rc.point(ChannelKind::Scalar, [0.8; 3]), Some([0.8; 3]));
    }

    #[test]
    fn bevel_glow_and_mosaic() {
        let checker = |f: &mut Fixture| {
            let id = f.project.alloc_id();
            let mut fill = Fill::default();
            fill.sources.insert(Channel::Roughness, Source::Procedural { pattern: Procedural::new(ProceduralKind::Checker), low: [0.0; 3], high: [1.0; 3] });
            f.project.insert_layer(0, Layer::new(id, "F", LayerKind::Fill(fill), &[Channel::Roughness]), None, false).unwrap();
            id
        };
        let values = |f: &Fixture| {
            let out = run(f, None);
            (0..64 * 64).map(|i| out.value(Channel::Roughness, i % 64, i / 64)[0]).collect::<Vec<f32>>()
        };
        let mut f = fixture();
        let id = checker(&mut f);
        let plain = values(&f);
        // Bevel: never above the shape, and its edges become ramps.
        let fx = content_fx(&mut f, id, EffectKind::Filter(Filter::Bevel { width: 128.0 }));
        let bevel = values(&f);
        let mids = |v: &[f32]| v.iter().filter(|x| **x > 0.05 && **x < 0.95).count();
        assert!(bevel.iter().zip(&plain).all(|(b, p)| *b <= p + 1e-4 && *b >= 0.0), "a bevel only lowers values");
        assert!(mids(&bevel) > mids(&plain) + 64, "{} vs {}", mids(&bevel), mids(&plain));
        // Mosaic: fewer changes from one texel to the next.
        f.project.layer_mut(id).unwrap().content_effects.retain(|e| e.id != fx);
        let fx = content_fx(&mut f, id, EffectKind::Filter(Filter::Mosaic { size: 768.0 }));
        let mosaic = values(&f);
        let steps = |v: &[f32]| v.windows(2).filter(|w| (w[0] - w[1]).abs() > 0.01).count();
        assert!(steps(&mosaic) < steps(&plain), "{} vs {}", steps(&mosaic), steps(&plain));
        assert!(mosaic.iter().all(|v| (0.0..=1.0).contains(v)));
        // Glow on uniform content adds its own blur: 0.2 + 0.2.
        f.project.layer_mut(id).unwrap().content_effects.retain(|e| e.id != fx);
        let mut f = fixture();
        let id = fill_layer(&mut f.project, [0.1; 3], 0.2);
        content_fx(&mut f, id, EffectKind::Filter(Filter::Glow { radius: 32.0, amount: 1.0 }));
        assert!(close(run(&f, None).value(Channel::Roughness, 30, 30), [0.4; 3]));
    }

    #[test]
    fn baked_lighting_lights_the_base_color_by_the_normal() {
        let lit = |elevation: f32| {
            let mut f = fixture();
            let id = fill_layer(&mut f.project, [0.5; 3], 0.3);
            let fx = Filter::BakedLighting { azimuth: 0.0, elevation, light: 0.6, ambient: 0.4, occlusion: 1.0 };
            content_fx(&mut f, id, EffectKind::Filter(fx));
            let out = run(&f, None);
            (out.value(Channel::BaseColor, 30, 30), out.value(Channel::Roughness, 30, 30))
        };
        let n = primitives::plane(2.0, 2).normals[0];
        // Facing the light: ambient + light = 1 (unchanged); facing away: ambient only.
        let (toward, rough) = lit(if n.y > 0.0 { 90.0 } else { -90.0 });
        assert!(close(toward, [0.5; 3]) && close(rough, [0.3; 3]), "{toward:?} {rough:?}");
        let (away, _) = lit(if n.y > 0.0 { -90.0 } else { 90.0 });
        assert!(close(away, [0.2; 3]), "{away:?}");
        let f = Filter::BakedLighting { azimuth: 0.0, elevation: 0.0, light: 0.0, ambient: 1.0, occlusion: 0.5 };
        assert_eq!(f.baked_light([0.0, 1.0, 0.0], 0.2), Some(0.6), "half the occlusion");
        assert_eq!(Filter::Invert.baked_light([0.0; 3], 1.0), None);
    }

    #[test]
    fn geometry_masks_show_layers_on_chosen_parts_only() {
        let mut f = fixture();
        // Two parts of the plane's one material: the first half of its triangles and the rest.
        let n = f.mesh.indices.len() as u32 / 6 * 3;
        f.mesh.submeshes = vec![
            texturecraft_geom::SubMesh { name: "Lid".into(), material: 0, start: 0, count: n },
            texturecraft_geom::SubMesh { name: "Body".into(), material: 0, start: n, count: f.mesh.indices.len() as u32 - n },
        ];
        let id = fill_layer(&mut f.project, [1.0, 0.0, 0.0], 0.2);
        let red = |f: &Fixture| {
            let out = run(f, None);
            (0..64 * 64).filter(|i| close(out.value(Channel::BaseColor, i % 64, i / 64), [1.0, 0.0, 0.0])).count()
        };
        let all = red(&f);
        f.project.layer_mut(id).unwrap().geometry_mask = Some(vec!["Lid".into()]);
        let lid = red(&f);
        f.project.layer_mut(id).unwrap().geometry_mask = Some(vec!["Body".into()]);
        let body = red(&f);
        assert!(lid > 0 && body > 0 && lid + body == all, "{lid} + {body} of {all}");
        f.project.layer_mut(id).unwrap().geometry_mask = Some(Vec::new());
        assert_eq!(red(&f), 0, "no parts: hidden everywhere");
    }

    #[test]
    fn content_paint_and_fill_effects_add_to_layers() {
        let mut f = fixture();
        let id = f.project.alloc_id();
        f.project.insert_layer(0, Layer::new(id, "P", LayerKind::Paint, &[Channel::BaseColor, Channel::Roughness]), None, false).unwrap();
        let mut material = Fill::default();
        material.sources.insert(Channel::Roughness, Source::Uniform([0.2; 3]));
        content_fx(&mut f, id, EffectKind::Material(material));
        let paint = content_fx(&mut f, id, EffectKind::Paint);
        // Roughness 0.9 painted at half coverage (premultiplied).
        f.pixels.layer_mut(paint, Channel::Roughness, (64, 64)).set(3, 3, &[to_u16(0.45), to_u16(0.5)]);
        let out = run(&f, None);
        assert!(close(out.value(Channel::Roughness, 10, 10), [0.2; 3]), "the fill effect covers the empty paint layer");
        assert!(close(out.value(Channel::Roughness, 3, 3), [0.55; 3]), "{:?}", out.value(Channel::Roughness, 3, 3));
        assert!(close(out.value(Channel::BaseColor, 3, 3), [0.5; 3]), "channels without a source stay untouched");
        // Flattening bakes the content effects in.
        let images = Default::default();
        let inp = Inputs {
            project: &f.project,
            ts: 0,
            mesh: &f.mesh,
            gbuf: &f.gbuf,
            pixels: &f.pixels,
            baked: &f.baked,
            preview: None,
            images: &images,
            bounds: f.mesh.bounds(),
        };
        let flat = flatten_layer(&inp, id, &all_tiles(64, 64)).unwrap();
        assert!(flat.iter().all(|(c, _, _)| *c == Channel::Roughness));
        let (_, _, data) = flat.iter().find(|(_, t, _)| *t == (0, 0)).unwrap();
        let o = 3 * TILE as usize + 3;
        assert!((from_u16(data[o * 2]) - 0.55).abs() < 1e-3 && (from_u16(data[o * 2 + 1]) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn anchors_feed_masks_above_in_stack_order() {
        let mut f = fixture();
        let low = f.project.alloc_id();
        let mut fill = Fill::default();
        fill.sources.insert(Channel::Height, Source::Uniform([0.4; 3]));
        f.project.insert_layer(0, Layer::new(low, "Low", LayerKind::Fill(fill), &[Channel::Height]), None, false).unwrap();
        let anchor = content_fx(&mut f, low, EffectKind::Anchor);
        let top = fill_layer(&mut f.project, [1.0; 3], 0.5);
        let mut mask = Mask::new(MaskBackground::Black);
        let source = Source::Anchor { anchor, channel: Channel::Height };
        mask.effects.push(Effect::new(f.project.alloc_id(), EffectKind::Fill { source, projection: Projection::default() }));
        f.project.layer_mut(top).unwrap().mask = Some(mask.clone());
        assert!(close(run(&f, None).value(Channel::BaseColor, 5, 5), [0.7; 3]));
        // Under a blur the anchor is read by the field pre-pass too: a flat height stays flat.
        mask.effects.push(Effect::new(f.project.alloc_id(), EffectKind::Filter(Filter::Blur { radius: 32.0 })));
        f.project.layer_mut(top).unwrap().mask = Some(mask);
        assert!(close(run(&f, None).value(Channel::BaseColor, 5, 5), [0.7; 3]));
        // An anchor above its reader is not evaluated yet: it reads the channel default (0).
        f.project.move_layer(low, 1).unwrap();
        assert!(close(run(&f, None).value(Channel::BaseColor, 5, 5), [0.5; 3]));
    }

    #[test]
    fn decal_fills_cover_only_their_frame_and_image_alpha() {
        let mut f = fixture();
        let s = f.gbuf.surface(&f.mesh, 32 * 64 + 32).unwrap();
        let d = texturecraft_core::Decal {
            center: s.pos.to_array(),
            normal: s.nrm.to_array(),
            up: s.nrm.any_orthonormal_vector().to_array(),
            size: [1.0, 1.0],
            rotation: 0.0,
            offset: [0.0; 2],
            depth: 0.5,
        };
        let id = f.project.alloc_id();
        let mut fill = Fill { projection: Projection::Decal(d), ..Fill::default() };
        fill.sources.insert(Channel::BaseColor, Source::image("decal"));
        f.project.insert_layer(0, Layer::new(id, "Decal", LayerKind::Fill(fill), &[Channel::BaseColor]), None, false).unwrap();
        let composite = |alpha: f32| {
            let img = crate::store::ImageRes { width: 2, height: 2, rgba: vec![[1.0, 0.0, 0.0, alpha]; 4], srgb: false };
            let images: ImageStore = [("decal".to_string(), std::sync::Arc::new(img))].into_iter().collect();
            let inp = Inputs {
                project: &f.project,
                ts: 0,
                mesh: &f.mesh,
                gbuf: &f.gbuf,
                pixels: &f.pixels,
                baked: &f.baked,
                preview: None,
                images: &images,
                bounds: f.mesh.bounds(),
            };
            let mut out = Composite::new(&f.project.texture_sets[0].channel_list(), 64);
            composite_tiles(&inp, &mut out, &all_tiles(64, 64));
            out
        };
        let out = composite(1.0);
        assert!(close(out.value(Channel::BaseColor, 32, 32), [1.0, 0.0, 0.0]), "{:?}", out.value(Channel::BaseColor, 32, 32));
        assert!(close(out.value(Channel::BaseColor, 2, 2), [0.5; 3]), "outside the frame");
        let half = composite(0.5);
        assert!(close(half.value(Channel::BaseColor, 32, 32), [0.75, 0.25, 0.25]), "the image alpha shapes the decal");
    }

    #[test]
    fn anchors_read_colors_as_gray_in_fill_layers() {
        let mut f = fixture();
        let low = fill_layer(&mut f.project, [0.2, 0.4, 0.6], 0.5);
        let anchor = content_fx(&mut f, low, EffectKind::Anchor);
        let id = f.project.alloc_id();
        let mut fill = Fill::default();
        fill.sources.insert(Channel::Roughness, Source::Anchor { anchor, channel: Channel::BaseColor });
        f.project.insert_layer(0, Layer::new(id, "Top", LayerKind::Fill(fill), &[Channel::Roughness]), None, false).unwrap();
        let expect = texturecraft_core::lum([0.2, 0.4, 0.6]);
        assert!(close(run(&f, None).value(Channel::Roughness, 9, 9), [expect; 3]));
    }
}

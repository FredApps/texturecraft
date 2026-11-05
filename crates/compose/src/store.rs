//! Pixel data owned by a document: painted layer channels, painted mask effects and baked mesh
//! maps. Everything is copy-on-write, so cloning the store for undo is cheap.

use std::collections::{BTreeMap, HashMap};

use texturecraft_core::{BakedMap, Channel, ChannelKind, LayerId};
use texturecraft_texel::{TileImage, from_u16, from_u16_signed};

/// Painted data of one paint layer channel: premultiplied value(s) plus coverage, u16 per
/// component (height stored signed).
#[derive(Clone, Debug, Default)]
pub struct PixelStore {
    /// (paint layer or content paint effect, channel) → texels.
    pub layers: HashMap<(LayerId, Channel), TileImage>,
    /// Paint effect of a mask → (premultiplied value, coverage).
    pub masks: HashMap<LayerId, TileImage>,
}

impl PixelStore {
    pub fn layer(&self, id: LayerId, c: Channel) -> Option<&TileImage> {
        self.layers.get(&(id, c))
    }

    /// The image for `(id, c)` at `size` (width, height), created empty if missing or of
    /// another size.
    pub fn layer_mut(&mut self, id: LayerId, c: Channel, size: (u32, u32)) -> &mut TileImage {
        sized(self.layers.entry((id, c)).or_insert_with(|| TileImage::new(size.0, size.1, c.components() + 1)), size)
    }

    pub fn mask_mut(&mut self, id: LayerId, size: (u32, u32)) -> &mut TileImage {
        sized(self.masks.entry(id).or_insert_with(|| TileImage::new(size.0, size.1, 2)), size)
    }

    /// Drop pixels of layers that no longer exist.
    pub fn retain(&mut self, alive: impl Fn(LayerId) -> bool) {
        self.layers.retain(|(id, _), _| alive(*id));
        self.masks.retain(|id, _| alive(*id));
    }

    pub fn bytes(&self) -> usize {
        self.layers.values().chain(self.masks.values()).map(TileImage::bytes).sum()
    }
}

/// `img`, replaced by an empty image of its components when it is not `size`.
fn sized(img: &mut TileImage, size: (u32, u32)) -> &mut TileImage {
    if (img.width(), img.height()) != size {
        *img = TileImage::new(size.0, size.1, img.comps());
    }
    img
}

/// Decode a stored component of channel kind `kind` to its float value.
#[inline]
pub fn decode(kind: ChannelKind, v: u16) -> f32 {
    if kind == ChannelKind::Signed { from_u16_signed(v) } else { from_u16(v) }
}

/// Baked mesh maps of one texture set (dense, row-major, `components()` floats per texel). The
/// width is `resolution`; height follows the output size and UV tile grid
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BakedMaps {
    /// Width in texels.
    pub resolution: u32,
    pub maps: BTreeMap<BakedMap, std::sync::Arc<Vec<f32>>>,
}

impl BakedMaps {
    /// Height in texels of map `m` (from its length).
    pub fn height(&self, m: BakedMap) -> u32 {
        self.maps.get(&m).map_or(0, |d| (d.len() / (self.resolution.max(1) as usize * m.components())) as u32)
    }

    /// Sample the corresponding texel of an image with the supplied output size
    pub fn sample(&self, map: BakedMap, x: u32, y: u32, output_size: (u32, u32)) -> Option<[f32; 3]> {
        let data = self.maps.get(&map)?;
        let width = u64::from(self.resolution.max(1));
        let height = u64::from(self.height(map).max(1));
        let sample_x = (u64::from(x) * width / u64::from(output_size.0.max(1))).min(width - 1);
        let sample_y = (u64::from(y) * height / u64::from(output_size.1.max(1))).min(height - 1);
        let component_count = map.components();
        let offset = ((sample_y * width + sample_x) as usize).checked_mul(component_count)?;
        let texel = data.get(offset..offset.checked_add(component_count)?)?;
        Some(if component_count == 3 { [texel[0], texel[1], texel[2]] } else { [texel[0]; 3] })
    }

    pub fn scalar(&self, m: BakedMap, x: u32, y: u32, size: (u32, u32)) -> Option<f32> {
        self.sample(m, x, y, size).map(|v| v[0])
    }
}

/// A decoded image resource sampled by fills (values in the file's encoding; see `srgb`).
#[derive(Clone, Debug, PartialEq)]
pub struct ImageRes {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<[f32; 4]>,
    /// 8/16-bit images are sRGB-encoded; color channels linearize them.
    pub srgb: bool,
}

/// Image resources by name.
pub type ImageStore = HashMap<String, std::sync::Arc<ImageRes>>;

impl ImageRes {
    fn texel(&self, x: i64, y: i64) -> [f32; 4] {
        let (w, h) = (self.width.max(1) as i64, self.height.max(1) as i64);
        let i = (y.rem_euclid(h) * w + x.rem_euclid(w)) as usize;
        self.rgba.get(i).copied().unwrap_or([0.0; 4])
    }

    /// Bilinear sample with wrap-around (texture coordinates, v down).
    pub fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        if !u.is_finite() || !v.is_finite() {
            return [0.0; 4];
        }
        let fx = u * self.width as f32 - 0.5;
        let fy = v * self.height as f32 - 0.5;
        let (x0, y0) = (fx.floor() as i64, fy.floor() as i64);
        let (tx, ty) = (fx - fx.floor(), fy - fy.floor());
        let (a, b, c, d) = (self.texel(x0, y0), self.texel(x0 + 1, y0), self.texel(x0, y0 + 1), self.texel(x0 + 1, y0 + 1));
        let mut out = [0.0; 4];
        for k in 0..4 {
            let top = a[k] + (b[k] - a[k]) * tx;
            let bot = c[k] + (d[k] - c[k]) * tx;
            out[k] = top + (bot - top) * ty;
        }
        out
    }
}

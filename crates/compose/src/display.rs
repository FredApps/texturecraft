//! Conversion of composited tiles to the texture formats the viewport samples.
//!
//! GPU packing (one texture per [`DisplayKind`], all `TILE`-aligned, same resolution):
//!
//! | texture   | format   | r                 | g              | b                 | a                 |
//! |-----------|----------|-------------------|----------------|-------------------|-------------------|
//! | BaseColor | sRGB8    | base color        |                |                   | opacity           |
//! | Orm       | RGBA8    | AO (× baked)      | roughness      | metallic          | specular level    |
//! | Normal    | RGBA8    | normal x          | normal y       | normal z          | sheen roughness   |
//! | Height    | RGBA16F  | height            | Specular (F0) r| g                 | b                 |
//! | Emissive  | sRGB8    | emissive          |                |                   | scattering weight |
//! | Lobes     | RGBA8    | coat weight       | coat roughness | anisotropy level  | anisotropy angle  |
//! | Sheen     | sRGB8    | sheen color       |                |                   | sheen weight      |
//! | Extra     | RGBA8    | coat normal x     | coat normal y  | transmission      | tile scale        |
//!
//! Extra packs the coat normal's x and y as `128 + 127 × n` (the default normal is exactly 0;
//! z is rebuilt in the shader). Its alpha is the [`Composite::block_scales`] of the texel's UV
//! tile as `32 × log2(1 / scale)` (0 for a tile filling its block): the shader turns height
//! slopes down by it.
//!
//! Alpha of sRGB formats is linear. Channels a set lacks pack their default values, which switch
//! the extra lobes off, so the renderer can keep Lobes and Sheen as 1×1 placeholders until a
//! tile with a non-default value arrives.

use texturecraft_core::{BakedMap, Channel};
use texturecraft_texel::{TILE, TILE_AREA, srgb_lut};

use crate::eval::Composite;
use crate::store::BakedMaps;

/// One of the textures the viewport shader samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DisplayKind {
    /// sRGB8: base color, alpha = opacity.
    BaseColor,
    /// RGBA8 linear: ambient occlusion (channel × baked), roughness (1 − glossiness when the set
    /// has a Glossiness channel), metallic, specular level.
    Orm,
    /// RGBA8: painted tangent-space normal.
    Normal,
    /// RGBA16F: height in r, the linear Specular (F0) color in gba.
    Height,
    /// sRGB8: emissive color, alpha = scattering weight.
    Emissive,
    /// RGBA8 linear: coat weight, coat roughness, anisotropy level, anisotropy angle.
    Lobes,
    /// sRGB8: sheen color, alpha = sheen weight.
    Sheen,
    /// RGBA8 linear: coat normal x and y, transmission, tile scale.
    Extra,
}

impl DisplayKind {
    pub const ALL: [DisplayKind; 8] = [
        DisplayKind::BaseColor,
        DisplayKind::Orm,
        DisplayKind::Normal,
        DisplayKind::Height,
        DisplayKind::Emissive,
        DisplayKind::Lobes,
        DisplayKind::Sheen,
        DisplayKind::Extra,
    ];

    /// Bytes per texel of the upload format.
    pub fn bytes_per_texel(self) -> usize {
        if self == DisplayKind::Height { 8 } else { 4 }
    }

    /// Display textures affected by a channel.
    pub fn of_channel(c: Channel) -> DisplayKind {
        match c {
            Channel::BaseColor | Channel::Opacity => DisplayKind::BaseColor,
            Channel::Normal | Channel::SheenRoughness => DisplayKind::Normal,
            Channel::Height | Channel::Specular => DisplayKind::Height,
            Channel::Emissive | Channel::ScatterWeight => DisplayKind::Emissive,
            Channel::CoatWeight | Channel::CoatRoughness | Channel::AnisotropyLevel | Channel::AnisotropyAngle => DisplayKind::Lobes,
            Channel::SheenColor | Channel::SheenWeight => DisplayKind::Sheen,
            Channel::CoatNormal | Channel::Transmission => DisplayKind::Extra,
            _ => DisplayKind::Orm,
        }
    }

    /// The texel of a set that has none of this texture's channels (the GPU clear value).
    pub fn default_texel(self) -> Vec<u8> {
        let mut px = tile_bytes(&Composite::new(&[], TILE), &BakedMaps::default(), self, 0, 0);
        px.truncate(self.bytes_per_texel());
        px
    }
}

fn u8_of(v: u16) -> u8 {
    (v >> 8) as u8
}

/// Tile bytes (`TILE × TILE`, row-major) for `kind`, using channel defaults for absent source tiles.
pub fn tile_bytes(composite: &Composite, baked: &BakedMaps, kind: DisplayKind, tx: u32, ty: u32) -> Vec<u8> {
    let bytes_per_texel = kind.bytes_per_texel();
    let mut out = vec![0u8; TILE_AREA * bytes_per_texel];
    let srgb = srgb_lut();
    let get = |c: Channel| composite.channels.get(&c).and_then(|img| img.tile(tx, ty));
    let resolution = composite.resolution;
    let default16 = |c: Channel| texturecraft_texel::to_u16(c.default_value()[0]);

    // A scalar channel's texel `i` as 8 bits (its default where the set lacks it)
    let scalar8 = |c: Channel| {
        let (t, d) = (get(c), default16(c));
        move |i: usize| u8_of(t.and_then(|t| t.get(i).copied()).unwrap_or(d))
    };

    match kind {
        DisplayKind::BaseColor | DisplayKind::Emissive | DisplayKind::Sheen => {
            let (c, alpha) = match kind {
                DisplayKind::BaseColor => (Channel::BaseColor, Channel::Opacity),
                DisplayKind::Emissive => (Channel::Emissive, Channel::ScatterWeight),
                _ => (Channel::SheenColor, Channel::SheenWeight),
            };
            let (src, alpha) = (get(c), scalar8(alpha));
            let d = c.default_value().map(|v| (texturecraft_texel::linear_to_srgb(v) * 255.0 + 0.5) as u8);
            for i in 0..TILE_AREA {
                let pixel = &mut out[i * 4..i * 4 + 4];
                match src {
                    Some(s) => {
                        for k in 0..3 {
                            pixel[k] = srgb[(s.get(i * 3 + k).copied().unwrap_or(0) >> 4) as usize];
                        }
                    }
                    None => pixel[..3].copy_from_slice(&d),
                }
                pixel[3] = alpha(i);
            }
        }
        DisplayKind::Lobes => {
            let lanes = [Channel::CoatWeight, Channel::CoatRoughness, Channel::AnisotropyLevel, Channel::AnisotropyAngle].map(scalar8);
            for (i, pixel) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                for (d, lane) in pixel.iter_mut().zip(&lanes) {
                    *d = lane(i);
                }
            }
        }
        DisplayKind::Extra => {
            let (n, transmission) = (get(Channel::CoatNormal), scalar8(Channel::Transmission));

            // Signed 8-bit around 128 so the flat normal packs exactly
            let pack_signed_normal = |v: u16| (128.0 + 127.0 * (v as f32 / 65535.0 * 2.0 - 1.0)).round().clamp(0.0, 255.0) as u8;

            // A TILE never straddles two atlas blocks
            let scale = (-32.0 * composite.block_scales.at(tx * TILE, ty * TILE).log2()).round().clamp(0.0, 255.0) as u8;
            for (i, pixel) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                for (k, d) in pixel.iter_mut().take(2).enumerate() {
                    *d = n.and_then(|t| t.get(i * 3 + k)).map_or(128, |v| pack_signed_normal(*v));
                }
                pixel[2] = transmission(i);
                pixel[3] = scale;
            }
        }
        DisplayKind::Orm => {
            let (ao, m, s) = (get(Channel::AmbientOcclusion), get(Channel::Metallic), get(Channel::SpecularLevel));

            // Specular/glossiness sets: the viewport shades roughness = 1 − glossiness
            let gloss = composite.channels.contains_key(&Channel::Glossiness);
            let (r, r_default) =
                if gloss { (get(Channel::Glossiness), default16(Channel::Glossiness)) } else { (get(Channel::Roughness), default16(Channel::Roughness)) };
            for i in 0..TILE_AREA {
                let (x, y) = (tx * TILE + (i as u32 % TILE), ty * TILE + (i as u32 / TILE));
                let baked_ao = baked
                    .sample_working(BakedMap::AmbientOcclusion, x, y, (resolution, composite.height), &composite.block_scales)
                    .map_or(1.0, |value| value[0]);
                let ch_ao = ao.and_then(|a| a.get(i)).map(|v| *v as f32 / 65535.0).unwrap_or(1.0);
                let pixel = &mut out[i * 4..i * 4 + 4];
                pixel[0] = ((baked_ao * ch_ao).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                let v = r.and_then(|t| t.get(i).copied()).unwrap_or(r_default);
                pixel[1] = u8_of(if gloss { 65535 - v } else { v });
                pixel[2] = u8_of(m.and_then(|t| t.get(i).copied()).unwrap_or(0));
                pixel[3] = u8_of(s.and_then(|t| t.get(i).copied()).unwrap_or(default16(Channel::SpecularLevel)));
            }
        }
        DisplayKind::Normal => {
            let (n, sheen_roughness) = (get(Channel::Normal), scalar8(Channel::SheenRoughness));
            for i in 0..TILE_AREA {
                let pixel = &mut out[i * 4..i * 4 + 4];

                // The baked normal is the base the painted normal details sit on (as in export)
                let (x, y) = (tx * TILE + (i as u32 % TILE), ty * TILE + (i as u32 / TILE));
                let base = baked.sample_working(BakedMap::Normal, x, y, (resolution, composite.height), &composite.block_scales);
                match (n, base) {
                    (Some(t), None) => {
                        for (k, d) in pixel.iter_mut().take(3).enumerate() {
                            *d = u8_of(t.get(i * 3 + k).copied().unwrap_or(0));
                        }
                    }
                    (detail, Some(base)) => {
                        let detail = detail.map_or([0.5, 0.5, 1.0], |t| [0, 1, 2].map(|k| t.get(i * 3 + k).map_or(0.0, |v| *v as f32 / 65535.0)));
                        let v = texturecraft_core::blend::rnm(base, detail);
                        for (d, v) in pixel.iter_mut().zip(v) {
                            *d = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                        }
                    }
                    (None, None) => pixel[..3].copy_from_slice(&[128, 128, 255]),
                }
                pixel[3] = sheen_roughness(i);
            }
        }
        DisplayKind::Height => {
            let (h, spec) = (get(Channel::Height), get(Channel::Specular));
            let spec_default = Channel::Specular.default_value();
            for i in 0..TILE_AREA {
                let v = h.and_then(|t| t.get(i)).map(|v| texturecraft_texel::from_u16_signed(*v)).unwrap_or(0.0);
                let f0 = |k: usize| spec.and_then(|t| t.get(i * 3 + k)).map(|v| *v as f32 / 65535.0).unwrap_or(spec_default[k]);
                for (k, c) in [v, f0(0), f0(1), f0(2)].into_iter().enumerate() {
                    out[i * 8 + k * 2..i * 8 + k * 2 + 2].copy_from_slice(&half::f16::from_f32(c).to_le_bytes());
                }
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_texel::to_u16;

    #[test]
    fn base_color_uses_srgb_and_opacity() {
        let mut comp = Composite::new(&[Channel::BaseColor, Channel::Opacity], 64);
        comp.channels.get_mut(&Channel::BaseColor).unwrap().set(0, 0, &[to_u16(1.0), to_u16(0.5), 0]);
        comp.channels.get_mut(&Channel::Opacity).unwrap().set(0, 0, &[to_u16(0.5)]);
        let b = tile_bytes(&comp, &BakedMaps::default(), DisplayKind::BaseColor, 0, 0);
        assert_eq!(b[0], 255);
        assert!((b[1] as i32 - 188).abs() <= 1, "{}", b[1]);
        assert_eq!(b[2], 0);
        assert!((b[3] as i32 - 127).abs() <= 1);
    }

    #[test]
    fn extra_alpha_holds_the_tile_scale() {
        let mut set = texturecraft_core::TextureSet::new("s", 0, 256);
        set.uv_tiles = vec![1001, 1002];
        set.tile_resolutions.insert(1002, 64);
        let comp = Composite { block_scales: set.block_scales(), ..Composite::new(&[], 512) };
        let alpha = |tx: u32| tile_bytes(&comp, &BakedMaps::default(), DisplayKind::Extra, tx, 0)[3];
        // Tile 1001 fills its block; 1002 (from texel 256) a quarter of it: 32 × log2(4).
        assert_eq!((alpha(0), alpha(3), alpha(4), alpha(7)), (0, 0, 64, 64));
    }

    #[test]
    fn missing_channels_fall_back_to_defaults() {
        let comp = Composite::new(&[], 64);
        let orm = tile_bytes(&comp, &BakedMaps::default(), DisplayKind::Orm, 0, 0);
        assert_eq!(&orm[..4], &[255, 128, 0, 128]);
        let n = tile_bytes(&comp, &BakedMaps::default(), DisplayKind::Normal, 0, 0);
        assert_eq!(&n[..4], &[128, 128, 255, 128], "sheen roughness 0.5 rides in alpha");
        assert_eq!(&tile_bytes(&comp, &BakedMaps::default(), DisplayKind::Emissive, 0, 0)[..4], &[0, 0, 0, 0]);
        assert_eq!(&tile_bytes(&comp, &BakedMaps::default(), DisplayKind::Lobes, 0, 0)[..4], &[0, 25, 0, 0], "lobes off, coat roughness 0.1");
        assert_eq!(&tile_bytes(&comp, &BakedMaps::default(), DisplayKind::Sheen, 0, 0)[..4], &[255, 255, 255, 0]);
        assert_eq!(&tile_bytes(&comp, &BakedMaps::default(), DisplayKind::Extra, 0, 0)[..4], &[128, 128, 0, 0], "flat coat normal, no transmission");
        for k in DisplayKind::ALL {
            assert_eq!(k.default_texel(), tile_bytes(&comp, &BakedMaps::default(), k, 3, 5)[..k.bytes_per_texel()]);
        }
        let h = tile_bytes(&comp, &BakedMaps::default(), DisplayKind::Height, 0, 0);
        assert_eq!(h.len(), TILE_AREA * 8);
        assert_eq!(half::f16::from_le_bytes([h[2], h[3]]).to_f32(), half::f16::from_f32(0.04).to_f32(), "default F0");
    }

    #[test]
    fn the_baked_normal_is_the_base_of_the_shown_normal() {
        // A baked normal tilted to +X (packed), 64² like the set.
        let tilted = [0.8f32, 0.5, 0.9];
        let baked = BakedMaps { resolution: 64, maps: [(BakedMap::Normal, std::sync::Arc::new(tilted.repeat(64 * 64)))].into_iter().collect() };
        let shown = |comp: &Composite| tile_bytes(comp, &baked, DisplayKind::Normal, 0, 0)[..3].to_vec();
        let pack = |v: [f32; 3]| v.map(|x| (x * 255.0 + 0.5) as u8).to_vec();
        // No normal channel: the baked normal as it is.
        assert_eq!(shown(&Composite::new(&[], 64)), pack(tilted));
        // A flat painted normal leaves it; a painted detail is re-oriented onto it.
        let mut comp = Composite::new(&[Channel::Normal], 64);
        comp.channels.get_mut(&Channel::Normal).unwrap().set(0, 0, &[to_u16(0.5), to_u16(0.5), to_u16(1.0)]);
        assert!(shown(&comp).iter().zip(pack(tilted)).all(|(a, b)| (*a as i32 - b as i32).abs() <= 1), "{:?}", shown(&comp));
        comp.channels.get_mut(&Channel::Normal).unwrap().set(0, 0, &[to_u16(0.5), to_u16(0.8), to_u16(0.9)]);
        assert_eq!(shown(&comp), pack(texturecraft_core::blend::rnm(tilted, [0.5, 0.8, 0.9])));
    }

    #[test]
    fn glossiness_shows_as_roughness_and_specular_rides_in_height() {
        let mut comp = Composite::new(&[Channel::Glossiness, Channel::Specular], 64);
        comp.channels.get_mut(&Channel::Glossiness).unwrap().set(0, 0, &[to_u16(0.8)]);
        comp.channels.get_mut(&Channel::Specular).unwrap().set(0, 0, &[to_u16(1.0), to_u16(0.5), 0]);
        let orm = tile_bytes(&comp, &BakedMaps::default(), DisplayKind::Orm, 0, 0);
        assert!((orm[1] as i32 - 51).abs() <= 1, "roughness = 1 - 0.8, got {}", orm[1]);
        let h = tile_bytes(&comp, &BakedMaps::default(), DisplayKind::Height, 0, 0);
        let at = |k: usize| half::f16::from_le_bytes([h[k * 2], h[k * 2 + 1]]).to_f32();
        assert_eq!(at(0), 0.0);
        assert!((at(1) - 1.0).abs() < 1e-3 && (at(2) - 0.5).abs() < 1e-3 && at(3) == 0.0);
        assert_eq!(DisplayKind::of_channel(Channel::Specular), DisplayKind::Height);
        assert_eq!(DisplayKind::of_channel(Channel::Glossiness), DisplayKind::Orm);
    }

    #[test]
    fn lobe_channels_pack_into_lobes_sheen_and_spare_alphas() {
        let mut comp = Composite::new(&Channel::LOBES, 64);
        let mut set = |c: Channel, v: &[u16]| comp.channels.get_mut(&c).unwrap().set(1, 0, v);
        set(Channel::CoatWeight, &[to_u16(1.0)]);
        set(Channel::CoatRoughness, &[to_u16(0.2)]);
        set(Channel::AnisotropyLevel, &[to_u16(0.6)]);
        set(Channel::AnisotropyAngle, &[to_u16(0.25)]);
        set(Channel::SheenColor, &[to_u16(1.0), 0, to_u16(0.5)]);
        set(Channel::SheenWeight, &[to_u16(0.8)]);
        set(Channel::SheenRoughness, &[to_u16(0.3)]);
        set(Channel::ScatterWeight, &[to_u16(0.4)]);
        set(Channel::Transmission, &[to_u16(0.6)]);
        set(Channel::CoatNormal, &[to_u16(1.0), to_u16(0.5), to_u16(1.0)]);
        let px = |k: DisplayKind| tile_bytes(&comp, &BakedMaps::default(), k, 0, 0)[4..8].to_vec();
        let near = |a: &[u8], b: &[u8]| a.iter().zip(b).all(|(x, y)| x.abs_diff(*y) <= 1);
        assert!(near(&px(DisplayKind::Lobes), &[255, 51, 153, 64]), "{:?}", px(DisplayKind::Lobes));
        assert!(near(&px(DisplayKind::Sheen), &[255, 0, 188, 204]), "{:?}", px(DisplayKind::Sheen));
        assert!(near(&px(DisplayKind::Normal)[3..], &[77]) && near(&px(DisplayKind::Emissive)[3..], &[102]));
        assert!(near(&px(DisplayKind::Extra), &[255, 128, 153, 0]), "{:?}", px(DisplayKind::Extra));
        for c in Channel::LOBES {
            assert_ne!(DisplayKind::of_channel(c), DisplayKind::Orm, "{c:?}");
        }
    }
}

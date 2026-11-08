//! Texture export: render the maps of an [`ExportPreset`] from a texture set's composite, baked
//! mesh maps and UV coverage, entirely in memory (so the web build can export too).
//!
//! Each [`MapSpec`] packs sources into the R, G, B, A (or L) channels of one file. Sources are
//! composite channels, virtual maps (final normal in OpenGL or DirectX convention, glossiness,
//! mixed AO, F0), baked mesh maps and constants. The final normal is the baked normal, the
//! composited normal channel and the normal derived from height combined with Reoriented Normal
//! Mapping. Color maps are sRGB-encoded in 8/16-bit formats and linear in float formats; data
//! maps are always linear. Height is stored with 0.5 = zero (`h * 0.5 + 0.5`) in every format.
//!
//! A UV-tiled (UDIM) texture set writes one file per map per tile: `$udim` in the file name
//! becomes the tile number, and names without it get `.$udim` appended. Padding never crosses
//! tiles.

use std::collections::BTreeSet;
use std::io::Cursor;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use texturecraft_compose::{BakedMaps, Composite};
use texturecraft_core::{AoMixing, BakedMap, BlockScales, Channel, ChannelKind, NormalFormat, NormalMixing, Project, TextureSet};
use texturecraft_geom::glam::Vec3;
use texturecraft_texel::{ColorSpace, DilationMap, linear_to_srgb};

use crate::{FormatError, MAX_IMAGE_SIZE, Result};

/// Height-to-normal slope: with strength 1, a height change of 1 across 1/16 of the texture
/// width tilts the normal 45°, at any resolution.
const HEIGHT_SPAN: f32 = 16.0;
const JPEG_QUALITY: u8 = 95;
/// Preset limits (user presets are untrusted JSON).
const MAX_MAPS: usize = 64;
const MAX_TEMPLATE: usize = 256;
const MAX_STEM: usize = 200;
/// Longest preset name.
pub const MAX_PRESET_NAME: usize = 64;

/// File name template variables with what they become (see [`list_outputs`]).
pub const FILE_TOKENS: [(&str, &str); 6] = [
    ("$mesh", "Mesh file name"),
    ("$textureSet", "Texture set name"),
    ("$project", "Project name"),
    ("$channel", "Channel or map id"),
    ("$colorSpace", "Colour maps: sRGB, linear, ACEScg or ACES2065-1; data maps: linear"),
    ("$udim", "UV tile number (1001 for sets without UV tiles)"),
];

/// Why `template` is not a valid file name template: path separators, `..`, control characters
/// and unknown `$` variables are refused.
pub fn template_error(template: &str) -> Option<String> {
    if template.trim().is_empty() || template.len() > MAX_TEMPLATE {
        return Some(format!("file name \"{template}\" is empty or longer than {MAX_TEMPLATE} bytes"));
    }
    if template.contains(['/', '\\']) || template.contains("..") || template.chars().any(char::is_control) {
        return Some(format!("file name \"{template}\" contains a path separator, \"..\" or a control character"));
    }
    let mut rest = template;
    while let Some(i) = rest.find('$') {
        let tail = rest.get(i..).unwrap_or_default();
        match FILE_TOKENS.iter().find(|(t, _)| tail.starts_with(t)) {
            Some((t, _)) => rest = tail.get(t.len()..).unwrap_or_default(),
            None => return Some(format!("file name \"{template}\" has an unknown variable (use {})", FILE_TOKENS.map(|(t, _)| t).join(", "))),
        }
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FileFormat {
    Png,
    Jpeg,
    Tga,
    Tiff,
    Bmp,
    Webp,
    Exr,
    Hdr,
}

impl FileFormat {
    pub const ALL: [FileFormat; 8] =
        [FileFormat::Png, FileFormat::Jpeg, FileFormat::Tga, FileFormat::Tiff, FileFormat::Bmp, FileFormat::Webp, FileFormat::Exr, FileFormat::Hdr];

    pub fn extension(self) -> &'static str {
        match self {
            FileFormat::Png => "png",
            FileFormat::Jpeg => "jpg",
            FileFormat::Tga => "tga",
            FileFormat::Tiff => "tif",
            FileFormat::Bmp => "bmp",
            FileFormat::Webp => "webp",
            FileFormat::Exr => "exr",
            FileFormat::Hdr => "hdr",
        }
    }

    pub fn from_extension(ext: &str) -> Option<FileFormat> {
        let ext = ext.trim_start_matches('.').to_ascii_lowercase();
        match ext.as_str() {
            "jpeg" => Some(FileFormat::Jpeg),
            "tiff" => Some(FileFormat::Tiff),
            _ => FileFormat::ALL.into_iter().find(|f| f.extension() == ext),
        }
    }

    /// Bit depths this format is written with (16 and 32 are half and full float for EXR).
    pub fn bit_depths(self) -> &'static [u8] {
        match self {
            FileFormat::Png | FileFormat::Tiff => &[8, 16],
            FileFormat::Exr => &[16, 32],
            FileFormat::Hdr => &[32],
            _ => &[8],
        }
    }

    /// Floating-point formats hold linear values.
    pub fn is_float(self) -> bool {
        matches!(self, FileFormat::Exr | FileFormat::Hdr)
    }

    /// The supported depth closest to `want` (ties go to the deeper one).
    pub fn depth(self, want: u8) -> u8 {
        self.bit_depths().iter().copied().min_by_key(|d| (d.abs_diff(want), std::cmp::Reverse(*d))).unwrap_or(8)
    }
}

/// Destination channel of an output file. `L` alone writes grayscale; `L` + `A` gray + alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Dest {
    R,
    G,
    B,
    A,
    L,
}

/// Maps computed at export time from several channels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VirtualMap {
    /// Final tangent-space normal, +Y up the image.
    NormalOpenGl,
    /// Final tangent-space normal, green flipped.
    NormalDirectX,
    /// 1 − roughness.
    Glossiness,
    /// Baked AO × AO channel (per the texture set's AO mixing).
    AoMixed,
    /// Dielectric reflectance at normal incidence: 0.08 × specular level.
    F0,
    /// 1 − glossiness (specular/glossiness projects).
    Roughness,
    /// Tangent-space normal from the height channel alone (the export's normal convention).
    HeightNormal,
    /// Sheen color × sheen weight (glTF's sheen color, a color map).
    SheenTint,
}

impl VirtualMap {
    pub const ALL: [VirtualMap; 8] = [
        VirtualMap::NormalOpenGl,
        VirtualMap::NormalDirectX,
        VirtualMap::HeightNormal,
        VirtualMap::Glossiness,
        VirtualMap::Roughness,
        VirtualMap::AoMixed,
        VirtualMap::F0,
        VirtualMap::SheenTint,
    ];

    pub fn id(self) -> &'static str {
        match self {
            VirtualMap::NormalOpenGl | VirtualMap::NormalDirectX | VirtualMap::HeightNormal => "normal",
            VirtualMap::Glossiness => "glossiness",
            VirtualMap::Roughness => "roughness",
            VirtualMap::AoMixed => "ambientOcclusion",
            VirtualMap::F0 => "f0",
            VirtualMap::SheenTint => "sheenTint",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            VirtualMap::NormalOpenGl => "Normal OpenGL",
            VirtualMap::NormalDirectX => "Normal DirectX",
            VirtualMap::HeightNormal => "Normal from height",
            VirtualMap::Glossiness => "Glossiness from roughness",
            VirtualMap::Roughness => "Roughness from glossiness",
            VirtualMap::AoMixed => "Mixed AO",
            VirtualMap::F0 => "F0 from specular level",
            VirtualMap::SheenTint => "Sheen color × weight",
        }
    }

    /// 3 for normals and colors, 1 otherwise.
    pub fn components(self) -> usize {
        match self {
            VirtualMap::NormalOpenGl | VirtualMap::NormalDirectX | VirtualMap::HeightNormal | VirtualMap::SheenTint => 3,
            _ => 1,
        }
    }
}

/// Where one output component comes from. `component` picks r/g/b of 3-component sources.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SourceRef {
    /// A composited channel. `Normal` means the final normal in the export normal format.
    Channel {
        channel: Channel,
        #[serde(default)]
        component: u8,
    },
    Virtual {
        map: VirtualMap,
        #[serde(default)]
        component: u8,
    },
    Baked {
        map: BakedMap,
        #[serde(default)]
        component: u8,
    },
    /// A constant (0 = black, 0.5 = gray, 1 = white).
    Constant(f32),
}

impl SourceRef {
    fn raw_component(&self) -> usize {
        match self {
            SourceRef::Channel { component, .. } | SourceRef::Virtual { component, .. } | SourceRef::Baked { component, .. } => *component as usize,
            SourceRef::Constant(_) => 0,
        }
    }

    fn component(&self) -> usize {
        self.raw_component().min(2)
    }

    /// Components the source has (1 or 3; constants are 1).
    pub fn components(&self) -> usize {
        match self {
            SourceRef::Channel { channel, .. } => channel.components(),
            SourceRef::Virtual { map, .. } => map.components(),
            SourceRef::Baked { map, .. } => map.components(),
            SourceRef::Constant(_) => 1,
        }
    }
}

/// One exported file. `file` is a name template without extension (see [`list_outputs`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MapSpec {
    pub file: String,
    pub format: FileFormat,
    pub bit_depth: u8,
    pub channels: Vec<(Dest, SourceRef)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportPreset {
    pub name: String,
    pub maps: Vec<MapSpec>,
}

impl ExportPreset {
    /// Structural checks for (untrusted) presets.
    pub fn validate(&self) -> Result<()> {
        let bad = |m: String| Err(FormatError::Invalid(format!("export preset \"{}\": {m}", self.name)));
        if self.name.trim().is_empty() || self.name.chars().count() > MAX_PRESET_NAME || self.name.chars().any(char::is_control) {
            return bad(format!("the name must be 1 to {MAX_PRESET_NAME} characters without control characters"));
        }
        if self.maps.len() > MAX_MAPS {
            return bad(format!("more than {MAX_MAPS} maps"));
        }
        for m in &self.maps {
            if let Some(e) = template_error(&m.file) {
                return bad(e);
            }
            if !matches!(m.bit_depth, 8 | 16 | 32) {
                return bad(format!("map \"{}\" has bit depth {} (8, 16 or 32)", m.file, m.bit_depth));
            }
            if m.channels.iter().any(|(_, s)| s.component() != s.raw_component()) {
                return bad(format!("map \"{}\" picks a component other than 0, 1 or 2", m.file));
            }
            let dests: BTreeSet<Dest> = m.channels.iter().map(|(d, _)| *d).collect();
            if dests.len() != m.channels.len() || m.channels.is_empty() {
                return bad(format!("map \"{}\" needs distinct destination channels", m.file));
            }
            if dests.contains(&Dest::L) && dests.iter().any(|d| matches!(d, Dest::R | Dest::G | Dest::B)) {
                return bad(format!("map \"{}\" mixes L with R/G/B", m.file));
            }
            if m.channels.iter().any(|(_, s)| matches!(s, SourceRef::Constant(v) if !v.is_finite())) {
                return bad(format!("map \"{}\" has a non-finite constant", m.file));
            }
        }
        Ok(())
    }
}

/// UV padding of exported maps (texels outside the UV islands).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Padding {
    /// Uncovered texels are zero (transparent where there is alpha).
    None,
    /// Islands are dilated this many texels; beyond, texels are zero.
    Dilation(u32),
    /// Islands are dilated this many texels; beyond, texels hold the channel default.
    DilationDefault(u32),
    /// Every texel takes its nearest island's value.
    Infinite,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportOptions {
    pub preset: ExportPreset,
    /// Write every map in this format instead of the preset's.
    pub format_override: Option<FileFormat>,
    pub bit_depth_override: Option<u8>,
    pub padding: Padding,
    /// Output size (power of two); None keeps the texture set resolution.
    pub size: Option<u32>,
    /// Normal convention for every normal output instead of the project's / preset's.
    pub normal_format: Option<NormalFormat>,
    /// UV-tiled sets: one image of the whole tile atlas per map instead of a file per tile
    /// (for formats without UDIM support, with the mesh's UVs in atlas space).
    pub atlas: bool,
    /// The colour space of the colour maps (data maps are never converted); None writes sRGB
    /// into 8/16-bit files and linear Rec.709 into float files (EXR, HDR).
    #[serde(skip)]
    pub color_space: Option<ColorSpace>,
}

impl ExportOptions {
    /// The colour space colour maps are written in when `format` is used.
    pub fn space_for(&self, format: FileFormat) -> ColorSpace {
        self.color_space.unwrap_or(if format.is_float() { ColorSpace::LinearRec709 } else { ColorSpace::Srgb })
    }
}

impl ExportOptions {
    pub fn new(preset: ExportPreset) -> ExportOptions {
        ExportOptions {
            preset,
            format_override: None,
            bit_depth_override: None,
            padding: Padding::Infinite,
            size: None,
            normal_format: None,
            atlas: false,
            color_space: None,
        }
    }
}

/// What is exported: one texture set's composite with its baked maps and UV coverage (one flag
/// per composite texel, row-major).
#[derive(Clone, Copy)]
pub struct ExportInput<'a> {
    pub project: &'a Project,
    pub ts: usize,
    pub composite: &'a Composite,
    pub baked: &'a BakedMaps,
    pub coverage: &'a [bool],
    pub mesh_name: &'a str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExportedFile {
    pub name: String,
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub warnings: Vec<String>,
}

/// A file a preset produces for a texture set (see [`list_outputs`]).
#[derive(Clone, Debug, PartialEq)]
pub struct PlannedFile {
    pub name: String,
    /// Index into the preset's maps.
    pub map: usize,
    pub format: FileFormat,
    pub bit_depth: u8,
    /// UDIM number of the tile written (None: the whole set).
    pub tile: Option<u32>,
    /// For a tiled file, `name` with the tile number written as `<UDIM>` (what a material that
    /// covers every tile refers to); None when the file isn't tiled.
    pub udim_name: Option<String>,
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------------------------
// Planning: which maps exist for a texture set and what their files are called.

fn normal_present(set: &TextureSet) -> bool {
    set.has_channel(Channel::Normal) || set.has_channel(Channel::Height) || set.baked.contains(&BakedMap::Normal)
}

/// A source with real data in this texture set (constants never count).
fn source_present(set: &TextureSet, s: &SourceRef) -> bool {
    match s {
        SourceRef::Channel { channel: Channel::Normal, .. } => normal_present(set),
        SourceRef::Channel { channel, .. } => set.has_channel(*channel),
        SourceRef::Virtual { map: VirtualMap::NormalOpenGl | VirtualMap::NormalDirectX, .. } => normal_present(set),
        SourceRef::Virtual { map: VirtualMap::Glossiness, .. } => set.has_channel(Channel::Roughness),
        SourceRef::Virtual { map: VirtualMap::AoMixed, .. } => set.has_channel(Channel::AmbientOcclusion) || set.baked.contains(&BakedMap::AmbientOcclusion),
        SourceRef::Virtual { map: VirtualMap::F0, .. } => set.has_channel(Channel::SpecularLevel),
        SourceRef::Virtual { map: VirtualMap::Roughness, .. } => set.has_channel(Channel::Glossiness),
        SourceRef::Virtual { map: VirtualMap::HeightNormal, .. } => set.has_channel(Channel::Height),
        SourceRef::Virtual { map: VirtualMap::SheenTint, .. } => set.has_channel(Channel::SheenWeight),
        SourceRef::Baked { map, .. } => set.baked.contains(map),
        SourceRef::Constant(_) => false,
    }
}

/// The written components in order: L | L A | R G B | R G B A. Alpha from a channel the set
/// does not have is dropped.
fn layout<'m>(set: &TextureSet, map: &'m MapSpec) -> Vec<(Dest, &'m SourceRef)> {
    let get = |d: Dest| map.channels.iter().find(|(x, _)| *x == d).map(|(_, s)| s);
    let alpha = get(Dest::A).filter(|s| !matches!(s, SourceRef::Channel { channel, .. } if !set.has_channel(*channel)));
    let black = &SourceRef::Constant(0.0);
    let mut out = match get(Dest::L) {
        Some(l) => vec![(Dest::L, l)],
        None => [Dest::R, Dest::G, Dest::B].into_iter().map(|d| (d, get(d).unwrap_or(black))).collect(),
    };
    if let Some(a) = alpha {
        out.push((Dest::A, a));
    }
    out
}

fn is_color(map: &MapSpec) -> bool {
    map.channels.iter().any(|(d, s)| {
        *d != Dest::A
            && match s {
                SourceRef::Channel { channel, .. } => channel.kind() == ChannelKind::Color,
                SourceRef::Virtual { map, .. } => *map == VirtualMap::SheenTint,
                _ => false,
            }
    })
}

fn channel_label(map: &MapSpec) -> &'static str {
    map.channels
        .iter()
        .find_map(|(_, s)| match s {
            SourceRef::Channel { channel, .. } => Some(channel.id()),
            SourceRef::Virtual { map, .. } => Some(map.id()),
            SourceRef::Baked { map, .. } => Some(map.id()),
            SourceRef::Constant(_) => None,
        })
        .unwrap_or("constant")
}

/// Keep `[A-Za-z0-9_.-]`, replace everything else with `_`; never empty or a reserved device name.
pub fn sanitize_file_stem(s: &str) -> String {
    let mut out: String = s.chars().take(MAX_STEM).map(|c| if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') { c } else { '_' }).collect();
    if out.trim_matches('.').is_empty() {
        out = "texture".into();
    }
    let device = out.split('.').next().unwrap_or_default().to_ascii_uppercase();
    let reserved = matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (device.len() == 4 && (device.starts_with("COM") || device.starts_with("LPT")) && device.ends_with(|c: char| c.is_ascii_digit()));
    if reserved {
        out.insert(0, '_');
    }
    out
}

/// The files `opts` produces for texture set `ts`, without rendering (for the export dialog).
/// Template variables: [`FILE_TOKENS`].
pub fn list_outputs(project: &Project, ts: usize, mesh_name: &str, opts: &ExportOptions) -> Result<Vec<PlannedFile>> {
    opts.preset.validate()?;
    let set = project.texture_set(ts)?;
    if let Some(s) = opts.size
        && (!s.is_power_of_two() || s > MAX_IMAGE_SIZE)
    {
        return Err(FormatError::Invalid(format!("export size {s} must be a power of two up to {MAX_IMAGE_SIZE}")));
    }
    let mut used = BTreeSet::new();
    let mut out = Vec::new();
    let tiles: Vec<Option<u32>> = if set.uv_tiles.is_empty() || opts.atlas { vec![None] } else { set.uv_tiles.iter().copied().map(Some).collect() };
    for (i, map) in opts.preset.maps.iter().enumerate() {
        if !map.channels.iter().any(|(_, s)| source_present(set, s)) {
            continue;
        }
        let format = opts.format_override.unwrap_or(map.format);
        let bit_depth = format.depth(opts.bit_depth_override.unwrap_or(map.bit_depth));
        let space = opts.space_for(format);
        let color_space = match (is_color(map), space) {
            (false, _) | (true, ColorSpace::LinearRec709) => "linear",
            (true, ColorSpace::Srgb) => "sRGB",
            (true, other) => other.name(),
        };
        // Integer files clamp to 0-1, which wide-gamut linear colour can leave.
        let clamps = is_color(map) && !format.is_float() && matches!(space, ColorSpace::AcesCg | ColorSpace::Aces2065);
        // One file per UV tile: names without `$udim` get it appended.
        let template = if tiles.iter().any(Option::is_some) && !map.file.contains("$udim") { format!("{}.$udim", map.file) } else { map.file.clone() };
        for &tile in &tiles {
            let stem = template
                .replace("$udim", &tile.unwrap_or(1001).to_string())
                .replace("$project", &project.name)
                .replace("$mesh", mesh_name)
                .replace("$textureSet", &set.name)
                .replace("$channel", channel_label(map))
                .replace("$colorSpace", color_space);
            // The same name with the tile number as a placeholder, to write it as `<UDIM>` after.
            let token_stem = tile.map(|_| {
                let t = template
                    .replace("$udim", "UDIMTOKEN")
                    .replace("$project", &project.name)
                    .replace("$mesh", mesh_name)
                    .replace("$textureSet", &set.name)
                    .replace("$channel", channel_label(map))
                    .replace("$colorSpace", color_space);
                sanitize_file_stem(&t)
            });
            let stem = sanitize_file_stem(&stem);
            let ext = format.extension();
            let mut name = format!("{stem}.{ext}");
            let mut warnings = Vec::new();
            if clamps {
                warnings.push(format!(
                    "{} colours outside 0-1 are clamped in {} files; use EXR or HDR to keep them",
                    space.name(),
                    format.extension().to_uppercase()
                ));
            }
            // Case-insensitive: Windows and macOS file systems are.
            let mut k = 2;
            while used.contains(&name.to_ascii_lowercase()) {
                name = format!("{stem}_{k}.{ext}");
                k += 1;
            }
            if k > 2 {
                warnings.push(format!("\"{stem}.{ext}\" is produced by more than one map; this one was renamed to \"{name}\""));
            }
            used.insert(name.to_ascii_lowercase());
            // Only when putting the tile number back gives this file's name: the length limit cuts the
            // longer placeholder elsewhere (or off), and a name may hold the placeholder text itself.
            let udim_name = token_stem.filter(|t| tile.is_some_and(|n| t.contains("UDIMTOKEN") && t.replace("UDIMTOKEN", &n.to_string()) == stem)).map(|t| {
                let renamed = if k > 2 { format!("_{}", k - 1) } else { String::new() };
                format!("{t}{renamed}.{ext}").replace("UDIMTOKEN", "<UDIM>")
            });
            out.push(PlannedFile { name, map: i, format, bit_depth, tile, udim_name, warnings });
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Sampling.

/// Which texel a value is taken from.
#[derive(Clone, Copy, PartialEq)]
enum At {
    Texel(u32, u32),
    /// The channel default (padding beyond the dilation distance).
    Default,
}

struct Sampler<'a> {
    set: &'a TextureSet,
    comp: &'a Composite,
    baked: &'a BakedMaps,
    coverage: &'a [bool],
    /// Composite width.
    res: u32,
    /// Texels per atlas block (`res` for a classic set): the height slope and the Sobel window
    /// follow the block.
    tile_res: u32,
    /// The share of its block each tile fills: a smaller tile's height slope shrinks with it.
    scales: BlockScales,
    /// Normal convention overriding every normal output.
    normal_override: Option<NormalFormat>,
    project_normal: NormalFormat,
}

fn unpack(v: [f32; 3]) -> Vec3 {
    (Vec3::from(v) * 2.0 - Vec3::ONE).try_normalize().unwrap_or(Vec3::Z)
}

/// Reoriented Normal Mapping: `detail` re-expressed around `base` (unit tangent-space vectors).
pub fn rnm(base: Vec3, detail: Vec3) -> Vec3 {
    let t = base + Vec3::Z;
    let u = detail * Vec3::new(-1.0, -1.0, 1.0);
    (t * t.dot(u) / t.z.max(1e-4) - u).try_normalize().unwrap_or(base)
}

impl Sampler<'_> {
    fn channel(&self, c: Channel, at: At) -> [f32; 3] {
        match at {
            At::Texel(x, y) if self.set.has_channel(c) => self.comp.value(c, x, y),
            _ => c.default_value(),
        }
    }

    fn baked(&self, m: BakedMap, at: At) -> [f32; 3] {
        match at {
            At::Texel(x, y) if self.set.baked.contains(&m) => {
                self.baked.sample_working(m, x, y, (self.res, self.comp.height), &self.scales).unwrap_or(m.neutral())
            }
            _ => m.neutral(),
        }
    }

    fn covered(&self, x: u32, y: u32) -> bool {
        self.coverage.get((y as usize) * self.res as usize + x as usize).copied().unwrap_or(false)
    }

    /// Normal from the height channel (3×3 Sobel; uncovered neighbours repeat the centre).
    fn height_normal(&self, x: u32, y: u32) -> Vec3 {
        let center = self.channel(Channel::Height, At::Texel(x, y))[0];
        // Clamped to the texel's UV tile.
        let t = self.tile_res.max(1);
        let (x0, y0, last) = ((x / t * t) as i64, (y / t * t) as i64, t as i64 - 1);
        let h = |dx: i64, dy: i64| {
            let (sx, sy) = ((x as i64 + dx).clamp(x0, x0 + last) as u32, (y as i64 + dy).clamp(y0, y0 + last) as u32);
            if self.covered(sx, sy) { self.channel(Channel::Height, At::Texel(sx, sy))[0] } else { center }
        };
        let gx = (h(1, -1) + 2.0 * h(1, 0) + h(1, 1) - h(-1, -1) - 2.0 * h(-1, 0) - h(-1, 1)) / 8.0;
        let gy = (h(-1, 1) + 2.0 * h(0, 1) + h(1, 1) - h(-1, -1) - 2.0 * h(0, -1) - h(1, -1)) / 8.0;
        let s = self.set.height_strength * self.tile_res as f32 * self.scales.at(x, y) / HEIGHT_SPAN;
        // Image y points down; OpenGL normals have +Y up the image.
        Vec3::new(-gx * s, gy * s, 1.0).try_normalize().unwrap_or(Vec3::Z)
    }

    /// The final tangent-space normal (OpenGL): baked normal ⊕ (height ⊕ normal channel).
    fn final_normal(&self, at: At) -> Vec3 {
        let At::Texel(x, y) = at else { return Vec3::Z };
        let base = if self.set.baked.contains(&BakedMap::Normal) { unpack(self.baked(BakedMap::Normal, at)) } else { Vec3::Z };
        let mut detail = if self.set.has_channel(Channel::Normal) { unpack(self.channel(Channel::Normal, at)) } else { Vec3::Z };
        if self.set.normal_mixing == NormalMixing::Combine && self.set.has_channel(Channel::Height) && self.set.height_strength != 0.0 {
            detail = rnm(self.height_normal(x, y), detail);
        }
        rnm(base, detail)
    }

    fn normal(&self, at: At, format: NormalFormat, cache: &mut Option<Vec3>) -> [f32; 3] {
        let n = *cache.get_or_insert_with(|| self.final_normal(at));
        self.pack_normal(n, format)
    }

    /// A unit OpenGL normal packed into 0..1 in `format` (or the override).
    fn pack_normal(&self, n: Vec3, format: NormalFormat) -> [f32; 3] {
        let y = if self.normal_override.unwrap_or(format) == NormalFormat::DirectX { -n.y } else { n.y };
        [n.x * 0.5 + 0.5, y * 0.5 + 0.5, n.z * 0.5 + 0.5]
    }

    fn sample(&self, src: &SourceRef, at: At, cache: &mut Option<Vec3>) -> f32 {
        let k = src.component();
        let v = match src {
            SourceRef::Constant(v) => return *v,
            SourceRef::Channel { channel: Channel::Normal, .. } => self.normal(at, self.project_normal, cache),
            SourceRef::Channel { channel, .. } => {
                let v = self.channel(*channel, at);
                if channel.kind() == ChannelKind::Signed { v.map(|h| h * 0.5 + 0.5) } else { v }
            }
            SourceRef::Virtual { map, .. } => match map {
                VirtualMap::NormalOpenGl => self.normal(at, NormalFormat::OpenGl, cache),
                VirtualMap::NormalDirectX => self.normal(at, NormalFormat::DirectX, cache),
                VirtualMap::Glossiness => [1.0 - self.channel(Channel::Roughness, at)[0]; 3],
                VirtualMap::F0 => [0.08 * self.channel(Channel::SpecularLevel, at)[0]; 3],
                VirtualMap::Roughness => [1.0 - self.channel(Channel::Glossiness, at)[0]; 3],
                VirtualMap::SheenTint => self.channel(Channel::SheenColor, at).map(|c| c * self.channel(Channel::SheenWeight, at)[0]),
                VirtualMap::HeightNormal => {
                    let n = match at {
                        At::Texel(x, y) => self.height_normal(x, y),
                        At::Default => Vec3::Z,
                    };
                    self.pack_normal(n, self.project_normal)
                }
                VirtualMap::AoMixed => {
                    let ch = self.channel(Channel::AmbientOcclusion, at)[0];
                    let baked = self.baked(BakedMap::AmbientOcclusion, at)[0];
                    [if self.set.ao_mixing == AoMixing::Multiply { ch * baked } else { ch }; 3]
                }
            },
            SourceRef::Baked { map, .. } => self.baked(*map, at),
        };
        v.get(k).copied().unwrap_or(0.0)
    }
}

// ---------------------------------------------------------------------------------------------
// Rendering.

/// Render every file of `opts` for `input`'s texture set.
pub fn render_maps(input: &ExportInput, opts: &ExportOptions) -> Result<Vec<ExportedFile>> {
    let planned = list_outputs(input.project, input.ts, input.mesh_name, opts)?;
    let set = input.project.texture_set(input.ts)?;
    let (res, height) = (input.composite.resolution, input.composite.height);
    let grid = set.tile_grid();
    let tile_res = res / grid.map_or(1, |g| g.cols.max(1));
    if tile_res == 0 || tile_res > MAX_IMAGE_SIZE || height != tile_res * grid.map_or(1, |g| g.rows) {
        return Err(FormatError::Invalid(format!("composite size {res}×{height} does not fit texture set \"{}\"", set.name)));
    }
    if input.coverage.len() != res as usize * height as usize {
        return Err(FormatError::Invalid(format!("coverage has {} texels, expected {res}×{height}", input.coverage.len())));
    }
    // Padding stays inside each UV tile.
    let dmap = match opts.padding {
        Padding::None => None,
        Padding::Dilation(d) | Padding::DilationDefault(d) => Some(DilationMap::in_blocks(input.coverage, res, height, Some(d), tile_res)),
        Padding::Infinite => Some(DilationMap::in_blocks(input.coverage, res, height, None, tile_res)),
    };
    let beyond_default = matches!(opts.padding, Padding::DilationDefault(_) | Padding::Infinite);
    let sampler = Sampler {
        set,
        comp: input.composite,
        baked: input.baked,
        coverage: input.coverage,
        res,
        tile_res,
        scales: set.block_scales(),
        normal_override: opts.normal_format,
        project_normal: input.project.normal_format,
    };
    planned
        .into_iter()
        .map(|p| {
            let map = opts.preset.maps.get(p.map).ok_or_else(|| FormatError::Invalid("planned map out of range".into()))?;
            let lay = layout(set, map);
            let n = lay.len();
            // The texels written: one tile, or the whole set.
            let (x0, y0, w, h) = match (p.tile, grid) {
                (Some(t), Some(g)) => {
                    let (bx, by) = g.block(t).ok_or_else(|| FormatError::Invalid(format!("UV tile {t} is not in the set's grid")))?;
                    // A tile smaller than its block fills the block's top-left corner.
                    let side = ((tile_res as f32 * set.tile_scale(t)).round() as u32).clamp(1, tile_res);
                    (bx * tile_res, by * tile_res, side, side)
                }
                _ => (0, 0, res, height),
            };
            let mut data = vec![0.0f32; w as usize * h as usize * n];
            data.par_chunks_mut(w as usize * n).enumerate().for_each(|(row_y, row)| {
                let y = y0 + row_y as u32;
                for (col, x) in (x0..x0 + w).enumerate() {
                    let at = if sampler.covered(x, y) {
                        Some(At::Texel(x, y))
                    } else {
                        match dmap.as_ref().and_then(|d| d.source(x, y)) {
                            Some((sx, sy)) => Some(At::Texel(sx, sy)),
                            None => beyond_default.then_some(At::Default),
                        }
                    };
                    let Some(at) = at else { continue };
                    let mut cache = None;
                    for (k, (_, src)) in lay.iter().enumerate() {
                        if let Some(v) = row.get_mut(col * n + k) {
                            *v = sampler.sample(src, at, &mut cache);
                        }
                    }
                }
            });
            // `size` is per tile: an atlas scales with it.
            let scale = |v: u32| (u64::from(v) * u64::from(opts.size.unwrap_or(tile_res)) / u64::from(tile_res)) as u32;
            let size = (scale(w), scale(h));
            let data = resize(data, (w, h), n, size);
            // Colour maps leave linear Rec.709 for the chosen space. sRGB in integer files is
            // encoded by `encode`; every other case is converted here (float sRGB included).
            let space = opts.space_for(p.format);
            let rgb = lay.len() >= 3 && lay.iter().take(3).map(|(d, _)| *d).eq([Dest::R, Dest::G, Dest::B]);
            let mut data = data;
            let mut warnings = p.warnings;
            let convert = is_color(map) && (space != ColorSpace::Srgb || p.format.is_float()) && space != ColorSpace::LinearRec709;
            if convert && rgb {
                data.par_chunks_mut(n).for_each(|px| {
                    if let [r, g, b, ..] = px {
                        [*r, *g, *b] = space.from_linear_rec709([*r, *g, *b]);
                    }
                });
            } else if convert {
                warnings.push(format!("\"{}\" packs colour into one channel (L), so it is written in linear Rec.709, not {}", p.name, space.name()));
            }
            let srgb: Vec<bool> = lay.iter().map(|(d, _)| *d != Dest::A && is_color(map) && !p.format.is_float() && space == ColorSpace::Srgb).collect();
            // A colour space the user chose is written into the colour maps (data maps have none).
            let embed = (opts.color_space.is_some() && is_color(map) && rgb).then_some(space);
            let bytes = encode(p.format, p.bit_depth, size, n, &data, &srgb, embed, &mut warnings)?;
            Ok(ExportedFile { name: p.name, bytes, width: size.0, height: size.1, warnings })
        })
        .collect()
}

/// Resample a `from` (width, height) image of `n` components to `to` (same aspect): box filter
/// when shrinking by an integer factor, bilinear otherwise.
fn resize(data: Vec<f32>, from: (u32, u32), n: usize, to: (u32, u32)) -> Vec<f32> {
    if to == from || from.0 == 0 || from.1 == 0 || to.0 == 0 || to.1 == 0 {
        return data;
    }
    let ((r, rh), (t, th)) = ((from.0 as usize, from.1 as usize), (to.0 as usize, to.1 as usize));
    let mut out = vec![0.0f32; t * th * n];
    let px = |x: usize, y: usize, k: usize| data.get((y * r + x) * n + k).copied().unwrap_or(0.0);
    let boxed = r > t && r % t == 0 && rh % th == 0 && r / t == rh / th;
    out.par_chunks_mut(t * n).enumerate().for_each(|(y, row)| {
        for x in 0..t {
            for k in 0..n {
                let v = if boxed {
                    let f = r / t;
                    let sum: f32 = (0..f).flat_map(|dy| (0..f).map(move |dx| (dx, dy))).map(|(dx, dy)| px(x * f + dx, y * f + dy, k)).sum();
                    sum / (f * f) as f32
                } else {
                    let (sx, sy) = (r as f32 / t as f32, rh as f32 / th as f32);
                    let (fx, fy) = (((x as f32 + 0.5) * sx - 0.5).max(0.0), ((y as f32 + 0.5) * sy - 0.5).max(0.0));
                    let (x0, y0) = ((fx as usize).min(r - 1), (fy as usize).min(rh - 1));
                    let (x1, y1) = ((x0 + 1).min(r - 1), (y0 + 1).min(rh - 1));
                    let (ax, ay) = (fx - x0 as f32, fy - y0 as f32);
                    let top = px(x0, y0, k) * (1.0 - ax) + px(x1, y0, k) * ax;
                    let bottom = px(x0, y1, k) * (1.0 - ax) + px(x1, y1, k) * ax;
                    top * (1.0 - ay) + bottom * ay
                };
                if let Some(o) = row.get_mut(x * n + k) {
                    *o = v;
                }
            }
        }
    });
    out
}

/// Encode a `size` (width, height) image of `n` components (1 L, 2 LA, 3 RGB, 4 RGBA; linear
/// values).
#[allow(clippy::too_many_arguments)]
fn encode(
    format: FileFormat,
    depth: u8,
    size: (u32, u32),
    n: usize,
    data: &[f32],
    srgb: &[bool],
    embed: Option<ColorSpace>,
    warnings: &mut Vec<String>,
) -> Result<Vec<u8>> {
    let failed = |e: &dyn std::fmt::Display| FormatError::Invalid(format!("{} encoding failed: {e}", format.extension()));
    // Where the colour space goes: an ICC profile in PNG, JPEG, TIFF and WebP, the chromaticities
    // attribute in EXR. The other formats have no place for it.
    if let Some(space) = embed
        && matches!(format, FileFormat::Tga | FileFormat::Bmp | FileFormat::Hdr)
    {
        warnings.push(format!("{} files can't record a colour space; the colours are in {}", format.extension().to_ascii_uppercase(), space.name()));
    }
    match format {
        FileFormat::Exr => return write_exr(size, n, data, depth == 16, embed).map_err(|e| failed(&e)),
        FileFormat::Hdr => {
            if n == 2 || n == 4 {
                warnings.push("HDR has no alpha channel; alpha was dropped".into());
            }
            let rgb = remap(data, n, if n <= 2 { &[0, 0, 0] } else { &[0, 1, 2] });
            let bytes: Vec<u8> = rgb.iter().flat_map(|v| v.to_ne_bytes()).collect();
            return write_image(format, &bytes, size, image::ExtendedColorType::Rgb32F, None).map_err(|e| failed(&e));
        }
        _ => {}
    }
    // Integer formats: transfer function and quantization first, then fit the channel layout.
    let encoded: Vec<f32> =
        data.iter().enumerate().map(|(i, v)| if srgb.get(i % n).copied().unwrap_or(false) { linear_to_srgb(*v) } else { v.clamp(0.0, 1.0) }).collect();
    let (encoded, n) = match (format, n) {
        (FileFormat::Jpeg, 2 | 4) => {
            warnings.push("JPEG has no alpha channel; alpha was dropped".into());
            (remap(&encoded, n, if n == 2 { &[0] } else { &[0, 1, 2] }), n - 1)
        }
        (FileFormat::Tiff, 2) => (remap(&encoded, 2, &[0, 0, 0, 1]), 4),
        _ => (encoded, n),
    };
    use image::ExtendedColorType as C;
    let (bytes, color) = if depth == 16 {
        let b: Vec<u8> = encoded.iter().flat_map(|v| ((v * 65535.0 + 0.5) as u16).to_ne_bytes()).collect();
        (b, [C::L16, C::La16, C::Rgb16, C::Rgba16].get(n.wrapping_sub(1)).copied().unwrap_or(C::Rgba16))
    } else {
        let b: Vec<u8> = encoded.iter().map(|v| (v * 255.0 + 0.5) as u8).collect();
        (b, [C::L8, C::La8, C::Rgb8, C::Rgba8].get(n.wrapping_sub(1)).copied().unwrap_or(C::Rgba8))
    };
    let icc = embed.and_then(crate::icc::profile);
    write_image(format, &bytes, size, color, icc).map_err(|e| failed(&e))
}

/// Pick components `pick` from every texel of an `n`-component image.
fn remap(data: &[f32], n: usize, pick: &[usize]) -> Vec<f32> {
    data.chunks_exact(n.max(1)).flat_map(|px| pick.iter().map(|k| px.get(*k).copied().unwrap_or(0.0))).collect()
}

/// Encode `bytes` as `format`, with `icc` (an ICC profile) embedded where the format has a place for one.
fn write_image(format: FileFormat, bytes: &[u8], (w, h): (u32, u32), color: image::ExtendedColorType, icc: Option<Vec<u8>>) -> image::ImageResult<Vec<u8>> {
    use image::ImageEncoder;
    let mut out = Cursor::new(Vec::new());
    // `set_icc_profile` says no (an error to ignore) for an encoder without a place for it.
    let fmt = match format {
        FileFormat::Jpeg => {
            let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY);
            if let Some(p) = icc {
                let _ = enc.set_icc_profile(p);
            }
            enc.write_image(bytes, w, h, color)?;
            return Ok(out.into_inner());
        }
        FileFormat::Png if icc.is_some() => {
            let mut enc = image::codecs::png::PngEncoder::new(&mut out);
            if let Some(p) = icc {
                let _ = enc.set_icc_profile(p);
            }
            enc.write_image(bytes, w, h, color)?;
            return Ok(out.into_inner());
        }
        FileFormat::Tiff if icc.is_some() => {
            let mut enc = image::codecs::tiff::TiffEncoder::new(&mut out);
            if let Some(p) = icc {
                let _ = enc.set_icc_profile(p);
            }
            enc.write_image(bytes, w, h, color)?;
            return Ok(out.into_inner());
        }
        FileFormat::Webp if icc.is_some() => {
            let mut enc = image::codecs::webp::WebPEncoder::new_lossless(&mut out);
            if let Some(p) = icc {
                let _ = enc.set_icc_profile(p);
            }
            enc.write_image(bytes, w, h, color)?;
            return Ok(out.into_inner());
        }
        FileFormat::Png => image::ImageFormat::Png,
        FileFormat::Tga => image::ImageFormat::Tga,
        FileFormat::Tiff => image::ImageFormat::Tiff,
        FileFormat::Bmp => image::ImageFormat::Bmp,
        FileFormat::Webp => image::ImageFormat::WebP,
        FileFormat::Hdr => image::ImageFormat::Hdr,
        FileFormat::Exr => image::ImageFormat::OpenExr,
    };
    image::write_buffer_with_format(&mut out, bytes, w, h, color, fmt)?;
    Ok(out.into_inner())
}

/// OpenEXR with Y / YA / RGB / RGBA channels, half or full float, ZIP compression.
fn write_exr((w, h): (u32, u32), n: usize, data: &[f32], half: bool, space: Option<ColorSpace>) -> exr::error::Result<Vec<u8>> {
    use exr::prelude::*;
    let names: &[&str] = match n {
        1 => &["Y"],
        2 => &["Y", "A"],
        3 => &["R", "G", "B"],
        _ => &["R", "G", "B", "A"],
    };
    let channels: SmallVec<[AnyChannel<FlatSamples>; 4]> = names
        .iter()
        .enumerate()
        .map(|(k, name)| {
            let plane = data.iter().skip(k).step_by(n.max(1));
            let samples = if half { FlatSamples::F16(plane.map(|v| f16::from_f32(*v)).collect()) } else { FlatSamples::F32(plane.copied().collect()) };
            AnyChannel::new(*name, samples)
        })
        .collect();
    let layer = Layer::new((w as usize, h as usize), LayerAttributes::default(), Encoding::SMALL_LOSSLESS, AnyChannels::sort(channels));
    let mut out = Cursor::new(Vec::new());
    let mut image = Image::from_layer(layer);
    // The primaries and white point of the values, in the attribute OpenEXR readers look for.
    if let Some(space) = space {
        let [r, g, b, white] = space.chromaticities().map(|xy| Vec2(xy[0] as f32, xy[1] as f32));
        image.attributes.chromaticities = Some(exr::meta::attribute::Chromaticities { red: r, green: g, blue: b, white });
    }
    let writer = image.write();
    // No worker threads on the web.
    #[cfg(target_arch = "wasm32")]
    let writer = writer.non_parallel();
    writer.to_buffered(&mut out)?;
    Ok(out.into_inner())
}

/// Write exported files into `dir` (created if missing), each atomically (`name.tmp`, then
/// rename). Names must be plain file names.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_files(files: &[ExportedFile], dir: &std::path::Path) -> Result<Vec<std::path::PathBuf>> {
    std::fs::create_dir_all(dir)?;
    files
        .iter()
        .map(|f| {
            // The planned name is `<sanitized stem>.<ext>`; the stem alone is capped at MAX_STEM.
            let (stem, ext) = f.name.rsplit_once('.').unwrap_or((&f.name, ""));
            if f.name.is_empty() || stem != sanitize_file_stem(stem) || !ext.chars().all(|c| c.is_ascii_alphanumeric()) || f.name.trim_matches('.').is_empty() {
                return Err(FormatError::Invalid(format!("\"{}\" is not a plain file name", f.name)));
            }
            let path = dir.join(&f.name);
            crate::write_atomic(&path, &f.bytes)?;
            Ok(path)
        })
        .collect()
}

#[cfg(test)]
mod tests;

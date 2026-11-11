//! The project: texture sets, their layer stacks, and project-wide settings

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{BakeSettings, BakedMap, Channel, ChannelFormat, CoreError, Effect, Layer, LayerId, LayerKind, Result};

pub const FORMAT_VERSION: u32 = 2;

/// Maximum group nesting (also bounds recursion over hostile project files)
pub const MAX_DEPTH: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum NormalFormat {
    #[default]
    OpenGl,
    DirectX,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum NormalMixing {
    #[default]
    Combine,
    Replace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AoMixing {
    #[default]
    Multiply,
    None,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextureSet {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Material index in the mesh this set paints.
    pub material: usize,
    /// Power of two, 128..=8192.
    pub resolution: u32,
    pub channels: Vec<(Channel, ChannelFormat)>,
    /// Top-level layers, bottom → top.
    pub stack: Vec<LayerId>,
    #[serde(default)]
    pub normal_mixing: NormalMixing,
    #[serde(default)]
    pub ao_mixing: AoMixing,
    /// Height-to-normal strength.
    #[serde(default = "one")]
    pub height_strength: f32,
    #[serde(default)]
    pub bake: BakeSettings,
    /// Mesh maps that have been baked (pixels live with the engine / project file).
    #[serde(default)]
    pub baked: BTreeSet<BakedMap>,
    #[serde(default = "yes")]
    pub visible: bool,
    /// UDIM numbers of the UV tiles this set paints, ascending (UV tiles mode);
    /// empty for a classic set covering UV 0..1. Each tile is one block of an atlas laid out by
    /// [`TileGrid`], [`TextureSet::block_resolution`] texels square.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uv_tiles: Vec<u32>,
    /// UV tiles excluded from baking; missing entries stay included
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub bake_excluded_tiles: BTreeSet<u32>,
    /// Per-tile resolutions overriding `resolution` (UDIM → texels per side). A tile smaller
    /// than the atlas block fills the block's top-left corner (see [`TextureSet::to_atlas`]).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tile_resolutions: BTreeMap<u32, u32>,
}

/// Smallest and largest texture set (and UV tile) resolutions.
pub const MIN_RESOLUTION: u32 = 128;
pub const MAX_RESOLUTION: u32 = 8192;

/// Lowest and highest UDIM numbers (columns 0..=9, rows 0..=99).
pub const UDIM_RANGE: std::ops::RangeInclusive<u32> = 1001..=2000;

/// The UDIM tile holding mesh UV `uv` (v down: row r covers v in −r..1−r), if in range.
pub fn udim_at(uv: [f32; 2]) -> Option<u32> {
    let (c, r) = (uv[0].floor(), -uv[1].floor());
    ((0.0..=9.0).contains(&c) && (0.0..=99.0).contains(&r)).then(|| 1001 + c as u32 + 10 * r as u32)
}

/// The atlas of a UV-tiled texture set: the bounding grid of its tiles, `cols × rows` blocks of
/// `resolution²` texels. Block (0, 0) is the top-left: the lowest column of the highest row, so
/// the atlas shows the tiles as they sit in UV space. Mesh UVs use v down (glTF), so UDIM row
/// `r` covers v in −r..1−r, and atlas coordinates are an affine map of mesh UVs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileGrid {
    /// UDIM column of the left blocks (0..=9).
    pub col: u32,
    /// UDIM row of the top blocks.
    pub row: u32,
    pub cols: u32,
    pub rows: u32,
}

impl TileGrid {
    /// The bounding grid of UDIM `tiles` (None when empty or a number is outside [`UDIM_RANGE`]).
    pub fn of(tiles: &[u32]) -> Option<TileGrid> {
        let mut cells = tiles.iter().map(|t| UDIM_RANGE.contains(t).then(|| ((t - 1001) % 10, (t - 1001) / 10)));
        let first = cells.next()??;
        let (mut c0, mut c1, mut r0, mut r1) = (first.0, first.0, first.1, first.1);
        for cell in cells {
            let (c, r) = cell?;
            (c0, c1, r0, r1) = (c0.min(c), c1.max(c), r0.min(r), r1.max(r));
        }
        Some(TileGrid { col: c0, row: r1, cols: c1 - c0 + 1, rows: r1 - r0 + 1 })
    }

    /// UDIM number of block (bx, by).
    pub fn udim(&self, bx: u32, by: u32) -> u32 {
        1001 + self.col + bx + 10 * self.row.saturating_sub(by)
    }

    /// Block (bx, by) of UDIM tile `udim`, if the grid holds it.
    pub fn block(&self, udim: u32) -> Option<(u32, u32)> {
        let (c, r) = (udim.checked_sub(1001)? % 10, (udim.checked_sub(1001)?) / 10);
        let (bx, by) = (c.checked_sub(self.col)?, self.row.checked_sub(r)?);
        (bx < self.cols && by < self.rows).then_some((bx, by))
    }

    /// Mesh UV → atlas UV (0..1 over the whole grid).
    pub fn to_atlas(&self, uv: [f32; 2]) -> [f32; 2] {
        [(uv[0] - self.col as f32) / self.cols as f32, (uv[1] + self.row as f32) / self.rows as f32]
    }

    /// Atlas UV → mesh UV.
    pub fn from_atlas(&self, uv: [f32; 2]) -> [f32; 2] {
        [uv[0] * self.cols as f32 + self.col as f32, uv[1] * self.rows as f32 - self.row as f32]
    }
}

pub use crate::block_scales::BlockScales;

fn one() -> f32 {
    1.0
}
fn yes() -> bool {
    true
}

impl TextureSet {
    pub fn new(name: impl Into<String>, material: usize, resolution: u32) -> TextureSet {
        TextureSet {
            name: name.into(),
            description: String::new(),
            material,
            resolution,
            channels: Channel::DEFAULT_SET.iter().map(|c| (*c, ChannelFormat::default_for(*c))).collect(),
            stack: Vec::new(),
            normal_mixing: NormalMixing::Combine,
            ao_mixing: AoMixing::Multiply,
            height_strength: 1.0,
            bake: BakeSettings::default(),
            baked: BTreeSet::new(),
            visible: true,
            uv_tiles: Vec::new(),
            bake_excluded_tiles: BTreeSet::new(),
            tile_resolutions: BTreeMap::new(),
        }
    }

    /// The atlas grid of a UV-tiled set (None for a classic set).
    pub fn tile_grid(&self) -> Option<TileGrid> {
        TileGrid::of(&self.uv_tiles)
    }

    pub fn has_enabled_bake_tiles(&self) -> bool {
        self.uv_tiles.is_empty() || self.uv_tiles.iter().any(|tile| !self.bake_excluded_tiles.contains(tile))
    }

    /// Height strength for slopes measured over the whole texture width (the viewport shader,
    /// the path tracer): a UV tile atlas is one tile per column wide.
    pub fn atlas_height_strength(&self) -> f32 {
        self.height_strength / self.tile_grid().map_or(1.0, |g| g.cols as f32)
    }

    /// Texels across and down: [`TextureSet::block_resolution`] per block of the atlas grid.
    pub fn size(&self) -> (u32, u32) {
        let (cols, rows) = self.tile_grid().map_or((1, 1), |g| (g.cols, g.rows));
        let block = self.block_resolution();
        (block.saturating_mul(cols), block.saturating_mul(rows))
    }

    /// Texels per side of UV tile `udim` (its override, else `resolution`).
    pub fn tile_resolution(&self, udim: u32) -> u32 {
        self.tile_resolutions.get(&udim).copied().unwrap_or(self.resolution)
    }

    /// Texels per side of an atlas block: the largest tile resolution (`resolution` for a
    /// classic set or one without overrides).
    pub fn block_resolution(&self) -> u32 {
        if self.tile_resolutions.is_empty() {
            return self.resolution;
        }
        self.uv_tiles.iter().map(|t| self.tile_resolution(*t)).max().unwrap_or(self.resolution)
    }

    /// The fraction of its atlas block UV tile `udim` fills along each side (1 for uniform sets
    /// and tiles the set does not paint).
    pub fn tile_scale(&self, udim: u32) -> f32 {
        if self.tile_resolutions.is_empty() || !self.uv_tiles.contains(&udim) {
            return 1.0;
        }
        (self.tile_resolution(udim) as f32 / self.block_resolution().max(1) as f32).clamp(1e-3, 1.0)
    }

    /// [`TextureSet::tile_scale`] of every atlas block, to look up by atlas texel.
    pub fn block_scales(&self) -> BlockScales {
        let Some(g) = self.tile_grid().filter(|_| !self.tile_resolutions.is_empty()) else { return BlockScales::default() };
        let scales = (0..g.rows).flat_map(|by| (0..g.cols).map(move |bx| g.udim(bx, by))).map(|u| self.tile_scale(u)).collect();
        BlockScales { block: self.block_resolution(), cols: g.cols, scales }
    }

    /// Scale of the tile holding mesh UV `uv` (v down) and that tile's origin corner.
    fn tile_at(&self, uv: [f32; 2]) -> (f32, [f32; 2]) {
        (udim_at(uv).map_or(1.0, |u| self.tile_scale(u)), [uv[0].floor(), uv[1].floor()])
    }

    /// Mesh UV → atlas UV of a UV-tiled set for a vertex of a triangle whose UV centroid is
    /// `centroid` (it names the tile, so vertices on tile borders map with their own triangle).
    pub fn to_atlas(&self, grid: &TileGrid, centroid: [f32; 2], uv: [f32; 2]) -> [f32; 2] {
        if self.tile_resolutions.is_empty() {
            return grid.to_atlas(uv);
        }
        let (s, [c, r]) = self.tile_at(centroid);
        grid.to_atlas([c + (uv[0] - c) * s, r + (uv[1] - r) * s])
    }

    /// Atlas UV → mesh UV of a UV-tiled set (inverse of [`TextureSet::to_atlas`]).
    pub fn from_atlas(&self, grid: &TileGrid, uv: [f32; 2]) -> [f32; 2] {
        let m = grid.from_atlas(uv);
        if self.tile_resolutions.is_empty() {
            return m;
        }
        let (s, [c, r]) = self.tile_at(m);
        [c + (m[0] - c) / s, r + (m[1] - r) / s]
    }

    pub fn has_channel(&self, c: Channel) -> bool {
        self.channels.iter().any(|(x, _)| *x == c)
    }

    pub fn channel_list(&self) -> Vec<Channel> {
        self.channels.iter().map(|(c, _)| *c).collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ToneMapping {
    #[default]
    Linear,
    AcesFilmic,
}

/// Viewport display settings (saved with the project). Fields missing from
/// older projects take their defaults.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplaySettings {
    /// A built-in environment name, or `resource:<name>` for an imported equirect image.
    pub environment: String,
    /// Background visibility 0..1.
    pub env_opacity: f32,
    pub env_exposure: f32,
    pub env_blur: f32,
    /// Degrees.
    pub env_rotation: f32,
    pub shadows: bool,
    pub tone_mapping: ToneMapping,
    pub fov: f32,
    pub post_effects: bool,
    pub bloom: f32,
    pub vignette: f32,
    pub sharpen: f32,
    pub grain: f32,
    pub ao_intensity: f32,
    /// Accumulate jittered frames while the view is still (temporal anti-aliasing).
    pub taa: bool,
    /// Shader: PBR specular/glossiness (base color = diffuse, Specular = F0 color, Glossiness)
    /// instead of metallic/roughness.
    pub spec_gloss: bool,
    /// Parallax occlusion mapping from the height channel (same depth as height-to-normal).
    pub parallax: bool,
    /// Displacement: the viewport moves the surface by the height channel (a finer copy of the
    /// mesh, same depth as height-to-normal). Painting still follows the base surface.
    pub displacement: bool,
    /// Subsurface scattering (where the Scattering weight channel is painted): linear color of
    /// the light that travels under the surface; each channel scales how far it travels.
    pub scatter_color: [f32; 3],
    /// Subsurface scattering mean free path as a fraction of the mesh's bounding radius.
    pub scatter_radius: f32,
    /// Index of refraction of transmissive surfaces (where the Transmission channel is painted).
    pub ior: f32,
    /// Depth of field of the viewport and the path tracer (thin lens).
    pub dof: DepthOfField,
}

/// A thin-lens camera's depth of field (Display settings: focus distance and aperture).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DepthOfField {
    pub enabled: bool,
    /// Distance of the plane in focus along the view axis (world units); 0 focuses on the
    /// camera's orbit target.
    pub focus_distance: f32,
    /// Lens radius as a fraction of the mesh's bounding radius: how fast the blur grows away
    /// from the focus plane.
    pub aperture: f32,
}

impl Default for DepthOfField {
    fn default() -> Self {
        DepthOfField { enabled: false, focus_distance: 0.0, aperture: 0.05 }
    }
}

/// A saved viewpoint of the camera list (the default camera's orbit parameters, as in
/// `geom::OrbitCamera`): saved views and cameras imported with the mesh.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Camera {
    pub name: String,
    pub target: [f32; 3],
    /// Radians around +Y.
    pub yaw: f32,
    /// Radians above the horizon.
    pub pitch: f32,
    pub distance: f32,
    /// Vertical field of view, degrees (orthographic: the view height at `distance`).
    pub fov: f32,
    pub ortho: bool,
}

impl Default for Camera {
    fn default() -> Self {
        Camera { name: "Camera".into(), target: [0.0; 3], yaw: 0.0, pitch: 0.0, distance: 5.0, fov: 40.0, ortho: false }
    }
}

/// Built-in environments, in menu order.
pub const ENVIRONMENTS: [&str; 6] = ["Studio Soft", "Studio Contrast", "Overcast", "Sunset", "Warehouse", "Night"];

/// Prefix of [`DisplaySettings::environment`] naming an image resource.
pub const ENV_RESOURCE_PREFIX: &str = "resource:";

impl DisplaySettings {
    /// The image resource used as the environment, if any.
    pub fn environment_resource(&self) -> Option<&str> {
        self.environment.strip_prefix(ENV_RESOURCE_PREFIX)
    }
}

impl Default for DisplaySettings {
    fn default() -> Self {
        DisplaySettings {
            environment: "Studio Soft".into(),
            env_opacity: 0.0,
            env_exposure: 0.0,
            env_blur: 0.35,
            env_rotation: 0.0,
            shadows: false,
            tone_mapping: ToneMapping::AcesFilmic,
            fov: 40.0,
            post_effects: false,
            bloom: 0.0,
            vignette: 0.0,
            sharpen: 0.0,
            grain: 0.0,
            ao_intensity: 0.75,
            taa: true,
            spec_gloss: false,
            parallax: false,
            displacement: false,
            scatter_color: [1.0, 0.4, 0.25],
            scatter_radius: 0.1,
            ior: 1.5,
            dof: DepthOfField::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub format_version: u32,
    pub name: String,
    /// Path of the mesh inside the project file (or a built-in primitive id `builtin:<name>`).
    pub mesh: String,
    /// File name the mesh was imported from (`quad.gltf`). The stored mesh is read again under
    /// it, so parts the file leaves unnamed get the same name (from it) as on import.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mesh_file: String,
    /// Where the mesh file was imported from (the project's "3D mesh path"). Reimport mesh reads
    /// it again from there, so edits made to the file since are picked up. Empty when unknown
    /// (built-in meshes, web imports, projects saved before it was kept).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mesh_path: String,
    pub normal_format: NormalFormat,
    pub texture_sets: Vec<TextureSet>,
    pub layers: BTreeMap<LayerId, Layer>,
    pub next_id: u64,
    pub display: DisplaySettings,
    /// Saved and imported cameras (the viewport's default camera is not in the list).
    #[serde(default)]
    pub cameras: Vec<Camera>,
}

impl Project {
    pub fn new(name: impl Into<String>, mesh: impl Into<String>) -> Project {
        Project {
            format_version: FORMAT_VERSION,
            name: name.into(),
            mesh: mesh.into(),
            mesh_file: String::new(),
            mesh_path: String::new(),
            normal_format: NormalFormat::OpenGl,
            texture_sets: Vec::new(),
            layers: BTreeMap::new(),
            next_id: 1,
            display: DisplaySettings::default(),
            cameras: Vec::new(),
        }
    }

    pub fn alloc_id(&mut self) -> LayerId {
        let id = LayerId(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    pub fn texture_set(&self, ts: usize) -> Result<&TextureSet> {
        self.texture_sets.get(ts).ok_or(CoreError::NoTextureSet(ts))
    }
    pub fn texture_set_mut(&mut self, ts: usize) -> Result<&mut TextureSet> {
        self.texture_sets.get_mut(ts).ok_or(CoreError::NoTextureSet(ts))
    }

    pub fn layer(&self, id: LayerId) -> Result<&Layer> {
        self.layers.get(&id).ok_or(CoreError::NoLayer(id))
    }
    pub fn layer_mut(&mut self, id: LayerId) -> Result<&mut Layer> {
        self.layers.get_mut(&id).ok_or(CoreError::NoLayer(id))
    }

    /// The texture set and parent group (None = top level) holding `id`, plus its index.
    pub fn locate(&self, id: LayerId) -> Option<(usize, Option<LayerId>, usize)> {
        for (ts, set) in self.texture_sets.iter().enumerate() {
            if let Some(i) = set.stack.iter().position(|x| *x == id) {
                return Some((ts, None, i));
            }
            if let Some((parent, i)) = self.find_in(&set.stack, id, 0) {
                return Some((ts, Some(parent), i));
            }
        }
        None
    }

    fn find_in(&self, list: &[LayerId], id: LayerId, depth: usize) -> Option<(LayerId, usize)> {
        if depth > MAX_DEPTH {
            return None;
        }
        for g in list {
            let Some(layer) = self.layers.get(g) else { continue };
            let kids = layer.children();
            if let Some(i) = kids.iter().position(|x| *x == id) {
                return Some((*g, i));
            }
            if let Some(found) = self.find_in(kids, id, depth + 1) {
                return Some(found);
            }
        }
        None
    }

    /// The sibling list holding `id` (a texture set stack or a group's children).
    fn siblings_mut(&mut self, ts: usize, parent: Option<LayerId>) -> Result<&mut Vec<LayerId>> {
        match parent {
            None => Ok(&mut self.texture_set_mut(ts)?.stack),
            Some(p) => match &mut self.layer_mut(p)?.kind {
                LayerKind::Group { children, .. } => Ok(children),
                _ => Err(CoreError::Invalid(format!("layer {} is not a group", p.0))),
            },
        }
    }

    /// Insert `layer` into texture set `ts` above `above` (None = top of the stack). When
    /// `above` is a group and `inside` is true, the layer goes to the top of the group.
    pub fn insert_layer(&mut self, ts: usize, layer: Layer, above: Option<LayerId>, inside: bool) -> Result<LayerId> {
        self.texture_set(ts)?;
        let id = layer.id;
        if self.layers.contains_key(&id) {
            return Err(CoreError::Invalid(format!("layer id {} already exists", id.0)));
        }
        self.layers.insert(id, layer);
        let placed = match above {
            Some(target) if inside && self.layers.get(&target).is_some_and(Layer::is_group) => self.siblings_mut(ts, Some(target)).map(|kids| kids.push(id)),
            Some(target) => match self.locate(target) {
                Some((t, parent, i)) if t == ts => self.siblings_mut(ts, parent).map(|list| list.insert(i + 1, id)),
                _ => self.texture_set_mut(ts).map(|s| s.stack.push(id)),
            },
            None => self.texture_set_mut(ts).map(|s| s.stack.push(id)),
        };
        if let Err(e) = placed {
            self.layers.remove(&id);
            return Err(e);
        }
        Ok(id)
    }

    /// Remove a layer (and, for groups, everything inside). Returns the removed ids.
    pub fn remove_layer(&mut self, id: LayerId) -> Result<Vec<LayerId>> {
        let (ts, parent, i) = self.locate(id).ok_or(CoreError::NoLayer(id))?;
        let list = self.siblings_mut(ts, parent)?;
        if i < list.len() {
            list.remove(i);
        }
        let mut removed = Vec::new();
        self.collect_subtree(id, 0, &mut removed);
        for r in &removed {
            self.layers.remove(r);
        }
        Ok(removed)
    }

    fn collect_subtree(&self, id: LayerId, depth: usize, out: &mut Vec<LayerId>) {
        if depth > MAX_DEPTH || out.contains(&id) {
            return;
        }
        out.push(id);
        if let Some(l) = self.layers.get(&id) {
            for c in l.children() {
                self.collect_subtree(*c, depth + 1, out);
            }
        }
    }

    /// Move `id` within its sibling list to `index` (clamped).
    pub fn move_layer(&mut self, id: LayerId, index: usize) -> Result<()> {
        let (ts, parent, i) = self.locate(id).ok_or(CoreError::NoLayer(id))?;
        let list = self.siblings_mut(ts, parent)?;
        if i < list.len() {
            list.remove(i);
            let to = index.min(list.len());
            list.insert(to, id);
        }
        Ok(())
    }

    /// Move `id` above `target` anywhere in the same texture set (drag & drop).
    pub fn move_above(&mut self, id: LayerId, target: Option<LayerId>, inside: bool) -> Result<()> {
        if Some(id) == target {
            return Ok(());
        }
        let (ts, parent, i) = self.locate(id).ok_or(CoreError::NoLayer(id))?;
        if let Some(t) = target {
            let mut sub = Vec::new();
            self.collect_subtree(id, 0, &mut sub);
            if sub.contains(&t) {
                return Err(CoreError::Invalid("cannot move a group into itself".into()));
            }
        }
        let list = self.siblings_mut(ts, parent)?;
        if i < list.len() {
            list.remove(i);
        }
        let layer = self.layers.remove(&id).ok_or(CoreError::NoLayer(id))?;
        self.insert_layer(ts, layer, target, inside).map(|_| ())
    }

    /// Visible-order walk of a stack: every layer id, depth-first, bottom → top.
    pub fn walk(&self, ts: usize) -> Vec<(LayerId, usize)> {
        let mut out = Vec::new();
        if let Some(set) = self.texture_sets.get(ts) {
            self.walk_list(&set.stack, 0, &mut out);
        }
        out
    }

    fn walk_list(&self, list: &[LayerId], depth: usize, out: &mut Vec<(LayerId, usize)>) {
        if depth > MAX_DEPTH {
            return;
        }
        for id in list {
            out.push((*id, depth));
            if let Some(l) = self.layers.get(id) {
                self.walk_list(l.children(), depth + 1, out);
            }
        }
    }

    /// Find an effect anywhere (mask or content stacks) by id: (owning layer, effect).
    pub fn effect(&self, id: LayerId) -> Option<(LayerId, &Effect)> {
        self.layers.values().find_map(|l| l.mask.iter().flat_map(|m| m.effects.iter()).chain(l.content_effects.iter()).find(|e| e.id == id).map(|e| (l.id, e)))
    }

    pub fn effect_mut(&mut self, id: LayerId) -> Option<&mut Effect> {
        self.layers.values_mut().find_map(|l| {
            let in_mask = l.mask.as_mut().and_then(|m| m.effects.iter_mut().find(|e| e.id == id));
            match in_mask {
                Some(e) => Some(e),
                None => l.content_effects.iter_mut().find(|e| e.id == id),
            }
        })
    }

    /// A unique "<base> <n>" name among the layers of texture set `ts`.
    pub fn next_layer_name(&self, ts: usize, base: &str) -> String {
        let names: BTreeSet<&str> = self.walk(ts).iter().filter_map(|(id, _)| self.layers.get(id).map(|l| l.name.as_str())).collect();
        (1..).map(|n| format!("{base} {n}")).find(|n| !names.contains(n.as_str())).unwrap_or_else(|| base.to_string())
    }

    /// Check structural invariants (ids exist, no layer appears twice). Used after loading.
    pub fn validate(&self) -> Result<()> {
        if self.format_version > FORMAT_VERSION {
            return Err(CoreError::Invalid(format!("project format {} is newer than this TextureCraft supports ({FORMAT_VERSION})", self.format_version)));
        }
        if let Some(t) = self.texture_sets.iter().find(|t| !t.uv_tiles.is_empty() && (t.tile_grid().is_none() || !t.uv_tiles.is_sorted_by(|a, b| a < b))) {
            return Err(CoreError::Invalid(format!("texture set \"{}\" has invalid UV tiles {:?}", t.name, t.uv_tiles)));
        }
        let bad_tile_res = |t: &TextureSet| {
            t.tile_resolutions.iter().any(|(u, r)| !t.uv_tiles.contains(u) || !r.is_power_of_two() || !(MIN_RESOLUTION..=MAX_RESOLUTION).contains(r))
        };
        if let Some(t) = self.texture_sets.iter().find(|t| bad_tile_res(t)) {
            return Err(CoreError::Invalid(format!("texture set \"{}\" has invalid UV tile resolutions {:?}", t.name, t.tile_resolutions)));
        }
        let mut seen = BTreeSet::new();
        for ts in 0..self.texture_sets.len() {
            for (id, depth) in self.walk(ts) {
                if depth > MAX_DEPTH {
                    return Err(CoreError::Invalid("layer groups nested too deeply".into()));
                }
                if !self.layers.contains_key(&id) {
                    return Err(CoreError::Invalid(format!("stack references missing layer {}", id.0)));
                }
                if !seen.insert(id) {
                    return Err(CoreError::Invalid(format!("layer {} appears twice", id.0)));
                }
            }
        }
        match self.instance_cycle() {
            Some(id) => Err(CoreError::Invalid(format!("instance layer {} draws itself", id.0))),
            None => Ok(()),
        }
    }

    /// Every layer drawn when `roots` are drawn: the roots, folder children and instance
    /// sources, recursively, each once.
    pub fn render_tree(&self, roots: &[LayerId]) -> BTreeSet<LayerId> {
        let mut out = BTreeSet::new();
        let mut todo: Vec<(LayerId, usize)> = roots.iter().map(|id| (*id, 0)).collect();
        while let Some((id, depth)) = todo.pop() {
            if depth > 2 * MAX_DEPTH || !out.insert(id) {
                continue;
            }
            if let Some(l) = self.layers.get(&id) {
                todo.extend(l.children().iter().chain(l.instance_source().as_ref()).map(|c| (*c, depth + 1)));
            }
        }
        out
    }

    /// An instance layer that draws itself (through folders and other instances), if any.
    pub fn instance_cycle(&self) -> Option<LayerId> {
        self.layers.values().filter_map(|l| Some((l.id, l.instance_source()?))).find(|(id, s)| self.render_tree(&[*s]).contains(id)).map(|(id, _)| id)
    }

    /// The layer whose content `id` draws: itself, or an instance's source.
    pub fn content_layer(&self, id: LayerId) -> Option<&Layer> {
        let mut l = self.layers.get(&id)?;
        for _ in 0..=MAX_DEPTH {
            match l.instance_source() {
                Some(s) => l = self.layers.get(&s)?,
                None => return Some(l),
            }
        }
        None
    }

    /// The layer holding the mask `id` draws with: itself when it has a mask (an instance's
    /// override), else the instance's source.
    pub fn mask_owner(&self, id: LayerId) -> LayerId {
        match self.layers.get(&id) {
            Some(l) if l.mask.is_none() => self.content_layer(id).map_or(id, |c| c.id),
            _ => id,
        }
    }

    /// Texture sets that draw layer `id`: the set holding it and every set with an instance of
    /// it (or of a folder holding it).
    pub fn sets_drawing(&self, id: LayerId) -> Vec<usize> {
        (0..self.texture_sets.len()).filter(|ts| self.texture_sets.get(*ts).is_some_and(|set| self.render_tree(&set.stack).contains(&id))).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> Project {
        let mut p = Project::new("t", "builtin:sphere");
        p.texture_sets.push(TextureSet::new("Body", 0, 512));
        p
    }

    fn paint(p: &mut Project, name: &str) -> Layer {
        Layer::new(p.alloc_id(), name, LayerKind::Paint, &Channel::DEFAULT_SET)
    }

    #[test]
    fn insert_move_remove_keep_tree_consistent() {
        let mut p = project();
        let a = paint(&mut p, "a");
        let a = p.insert_layer(0, a, None, false).unwrap();
        let b = paint(&mut p, "b");
        let b = p.insert_layer(0, b, None, false).unwrap();
        let gid = p.alloc_id();
        let g = Layer::new(gid, "g", LayerKind::Group { children: vec![], collapsed: false }, &Channel::DEFAULT_SET);
        p.insert_layer(0, g, Some(a), false).unwrap();
        assert_eq!(p.texture_sets[0].stack, vec![a, gid, b]);
        p.move_above(b, Some(gid), true).unwrap();
        assert_eq!(p.texture_sets[0].stack, vec![a, gid]);
        assert_eq!(p.layer(gid).unwrap().children(), &[b]);
        assert_eq!(p.locate(b), Some((0, Some(gid), 0)));
        assert!(p.move_above(gid, Some(b), true).is_err(), "a group cannot go inside itself");
        p.validate().unwrap();
        let removed = p.remove_layer(gid).unwrap();
        assert_eq!(removed, vec![gid, b]);
        assert_eq!(p.texture_sets[0].stack, vec![a]);
        assert!(p.layer(b).is_err());
        p.validate().unwrap();
    }

    #[test]
    fn names_are_unique() {
        let mut p = project();
        let l = Layer::new(p.alloc_id(), "Layer 1", LayerKind::Paint, &[]);
        p.insert_layer(0, l, None, false).unwrap();
        assert_eq!(p.next_layer_name(0, "Layer"), "Layer 2");
    }

    #[test]
    fn serde_round_trip_is_stable() {
        let mut p = project();
        let l = paint(&mut p, "x");
        p.insert_layer(0, l, None, false).unwrap();
        let json = serde_json::to_string(&p).unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    #[test]
    fn per_map_bake_settings_round_trip() {
        use crate::{BackfaceMode, PositionMode, RayDistribution, RaySettings, SelfOcclusion};

        let mut project = project();
        let settings = &mut project.texture_sets[0].bake;
        settings.maps.ao.parameters.rays =
            RaySettings { rays: 96, max_distance: 0.4, spread: 120.0, distribution: RayDistribution::Uniform, ..RaySettings::default() };
        settings.maps.ao.parameters.ignore_backface = BackfaceMode::Always;
        settings.maps.ao.parameters.self_occlusion = SelfOcclusion::SameMeshName;
        settings.maps.thickness.parameters.rays.spread = 60.0;
        settings.maps.bent_normal.parameters.rays.rays = 8;
        settings.maps.position.parameters.normalization = PositionMode::BoundingSphere;

        let json = serde_json::to_string(&project).unwrap();
        assert_eq!(serde_json::from_str::<Project>(&json).unwrap(), project);
    }

    #[test]
    fn content_effects_and_anchors_round_trip_and_old_projects_load() {
        use crate::{Effect, EffectKind, Fill, Filter, Mask, MaskBackground, Projection, Source};
        let mut p = project();
        let mut l = paint(&mut p, "x");
        let (anchor, hsl, fill) = (p.alloc_id(), p.alloc_id(), p.alloc_id());
        let mut material = Fill::default();
        material.sources.insert(Channel::Roughness, Source::Uniform([0.2; 3]));
        l.content_effects = vec![
            Effect::new(fill, EffectKind::Material(material)),
            Effect::new(hsl, EffectKind::Filter(Filter::Hsl { hue: 0.25, saturation: -0.5, lightness: 0.1 })),
            Effect::new(anchor, EffectKind::Anchor),
        ];
        let id = p.insert_layer(0, l, None, false).unwrap();
        let mut top = paint(&mut p, "y");
        let source = Source::Anchor { anchor, channel: Channel::Height };
        top.mask = Some(Mask {
            effects: vec![Effect::new(p.alloc_id(), EffectKind::Fill { source, projection: Projection::default() })],
            ..Mask::new(MaskBackground::Black)
        });
        p.insert_layer(0, top, Some(id), false).unwrap();
        let json = serde_json::to_string(&p).unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
        assert_eq!(back.effect(anchor).map(|(owner, _)| owner), Some(id));
        // Projects saved before content effects existed load with empty stacks.
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        for l in v["layers"].as_object_mut().unwrap().values_mut() {
            l.as_object_mut().unwrap().remove("content_effects");
        }
        let old: Project = serde_json::from_value(v).unwrap();
        assert!(old.layers.values().all(|l| l.content_effects.is_empty()));
    }

    #[test]
    fn instances_resolve_draw_in_their_sets_and_cycles_are_found() {
        let mut p = project();
        p.texture_sets.push(TextureSet::new("Head", 1, 512));
        let a = paint(&mut p, "a");
        let a = p.insert_layer(0, a, None, false).unwrap();
        let inst = Layer::new(p.alloc_id(), "a", LayerKind::Instance { source: a }, &[]);
        let inst = p.insert_layer(1, inst, None, false).unwrap();
        assert_eq!(p.content_layer(inst).map(|l| l.id), Some(a));
        assert_eq!(p.sets_drawing(a), vec![0, 1]);
        assert_eq!(p.sets_drawing(inst), vec![1]);
        p.validate().unwrap();
        // A folder holding an instance of itself draws itself.
        let gid = p.alloc_id();
        p.insert_layer(1, Layer::new(gid, "g", LayerKind::Group { children: vec![], collapsed: false }, &[]), None, false).unwrap();
        let loop_id = p.alloc_id();
        p.insert_layer(1, Layer::new(loop_id, "g", LayerKind::Instance { source: gid }, &[]), Some(gid), true).unwrap();
        assert_eq!(p.instance_cycle(), Some(loop_id));
        assert!(p.validate().is_err());
        assert!(p.content_layer(LayerId(999)).is_none());
    }

    #[test]
    fn tile_grids_map_mesh_uvs_to_atlas_blocks() {
        assert_eq!(TileGrid::of(&[]), None);
        assert_eq!(TileGrid::of(&[1001, 3000]), None);
        // 1001 alone is the identity.
        let one = TileGrid::of(&[1001]).unwrap();
        assert_eq!((one.cols, one.rows, one.to_atlas([0.25, 0.75])), (1, 1, [0.25, 0.75]));
        // 1002 and 1013: columns 1..=2, rows 0..=1; the top row is row 1.
        let g = TileGrid::of(&[1002, 1013]).unwrap();
        assert_eq!(g, TileGrid { col: 1, row: 1, cols: 2, rows: 2 });
        assert_eq!([g.udim(0, 0), g.udim(1, 0), g.udim(0, 1), g.udim(1, 1)], [1012, 1013, 1002, 1003]);
        assert_eq!((g.block(1013), g.block(1002), g.block(1001), g.block(5)), (Some((1, 0)), Some((0, 1)), None, None));
        // Mesh v is down: UDIM row 1 is v in -1..0, the top half of the atlas.
        assert_eq!(g.to_atlas([2.5, -0.5]), [0.75, 0.25]);
        assert_eq!(g.from_atlas([0.75, 0.25]), [2.5, -0.5]);
        let mut p = project();
        p.texture_sets[0].uv_tiles = vec![1002, 1001];
        assert!(p.validate().is_err(), "unsorted");
        p.texture_sets[0].uv_tiles = vec![1001, 1002];
        p.validate().unwrap();
        assert_eq!(p.texture_sets[0].size(), (1024, 512));
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<Project>(&json).unwrap(), p);
    }

    #[test]
    fn per_tile_resolutions_scale_tiles_within_their_blocks() {
        let mut p = project();
        let t = &mut p.texture_sets[0];
        t.uv_tiles = vec![1001, 1002];
        let g = t.tile_grid().unwrap();
        // Uniform sets map exactly like the grid.
        assert_eq!(t.to_atlas(&g, [1.5, 0.5], [1.0, 0.25]), g.to_atlas([1.0, 0.25]));
        t.tile_resolutions.insert(1002, 128);
        assert_eq!((t.block_resolution(), t.size(), t.tile_scale(1002), t.tile_scale(1001)), (512, (1024, 512), 0.25, 1.0));
        // Tile 1002 fills the top-left quarter of block 1; a vertex on the 1001/1002 border maps
        // with its own triangle's tile.
        assert_eq!(t.to_atlas(&g, [1.5, 0.5], [2.0, 1.0]), [0.5 + 0.125, 0.25]);
        assert_eq!(t.to_atlas(&g, [1.5, 0.5], [1.0, 0.0]), [0.5, 0.0]);
        assert_eq!(t.to_atlas(&g, [0.5, 0.5], [1.0, 1.0]), [0.5, 1.0]);
        assert_eq!(t.from_atlas(&g, [0.5 + 0.0625, 0.125]), [1.5, 0.5]);
        assert_eq!(udim_at([1.5, -0.5]), Some(1012));
        assert_eq!(udim_at([-0.5, 0.5]), None);
        p.validate().unwrap();
        // A tile resolution must be a valid size of a tile the set paints.
        for bad in [(1002, 100), (1002, 16384), (1005, 256)] {
            let mut q = p.clone();
            q.texture_sets[0].tile_resolutions = [bad].into_iter().collect();
            assert!(q.validate().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn validate_rejects_newer_formats_and_dangling_ids() {
        let mut p = project();
        p.texture_sets[0].stack.push(LayerId(99));
        assert!(p.validate().is_err());
        let mut q = project();
        q.format_version = FORMAT_VERSION + 1;
        assert!(q.validate().is_err());
    }
}

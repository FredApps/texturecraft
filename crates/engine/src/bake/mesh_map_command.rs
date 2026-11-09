//! Mesh-map baking commands (the "Bake Mesh Maps" window): bake settings, the
//! high-definition mesh list, the cage, and running a bake. Bakes cast their rays on the GPU
//! tracer the app installs ([`install_gpu_tracer`]) and on the CPU otherwise.

use super::model_patch;

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};
use texturecraft_core::BakePaintTarget;

use super::{BakeCageInputs, MergedHigh};
use crate::{BakeMesh, BakeTask};
use serde_json::{Value, json};
use texturecraft_bake::tracing::Tracer;
use texturecraft_bake::{BakeOutput, BakePaintMap, MAX_RAYS, Progress};
use texturecraft_core::{
    BackfaceMode, BakeAntialiasing, BakeResolution, BakeSettings, BakedMap, CurvatureMethod, PositionMode, RayDistribution, RaySettings, SelfOcclusion,
};
use texturecraft_geom::Mesh;

use crate::commands::{CommandSpec, always, bad, boolp, f32p, strp, u64p};
use crate::{EngineError, Result, Session, cmd};

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "bake.run",
            "Bake mesh maps",
            ["Mode"],
            None,
            "{ts?: index|[indices]|\"all\"|\"selected\", cpu?: true (skip the GPU), ...any bake.settings parameter} → {baked, tracer: [where each set's rays ran]}",
            always,
            bake_run
        ),
        cmd!(
            "bake.settings",
            "Bake settings",
            [],
            None,
            "{ts?: index|[indices]|\"all\"|\"selected\" (default: the active set), model?: complete nested BakeSettings (named enum values), preserveEnabled?: keep each texture set enabled flag when replacing the model, patch?: nested BakeSettings fields to update after model (omitted fields stay unchanged, null clears optional fields), resolution?, antialiasing?: 1|2|4|8, dilation?, ao?: {rays?, maxDistance?, spread?: degrees (180 = hemisphere), distribution?: cosine|uniform, ignoreBackface?: never|always|byMeshName, selfOcclusion?: always|sameMeshName}, thickness?: {rays?, maxDistance?, spread?, distribution?}, bentNormal?: {rays?, maxDistance?, spread?, distribution?}, rays?, maxDistance? (all three ray maps), curvatureMethod?: normalMap|mesh, positionMode?: boundingBox|boundingSphere|none, maps?: [ids], enabled?, useLowAsHigh?, maxFrontal?, maxRear?, averageNormals?, lowSuffix?, highSuffix?}",
            always,
            settings
        ),
        cmd!(
            "bake.highPoly.add",
            "Add high definition mesh",
            [],
            None,
            "{path} — a mesh file (glb, gltf, fbx, dae, usd, usda, usdc, usdz, abc, obj, ply, stl; base color textures from glTF, embedded FBX media, usdz packages and image files next to a .gltf or USD layer supply opacity)",
            always,
            high_add
        ),
        cmd!("bake.highPoly.remove", "Remove high definition mesh", [], None, "{index? | name? | all?: true}", always, high_remove),
        cmd!("bake.highPoly.list", "List high definition meshes", [], None, "{}", always, |s, _| Ok(meshes_json(s))),
        cmd!("bake.cage.set", "Load cage mesh", [], None, "{path} — a mesh with the low poly's triangles", always, cage_set),
        cmd!("bake.cage.clear", "Remove cage mesh", [], None, "{}", always, |s, _| {
            s.bake_meshes.cage = None;
            s.modified = true;
            Ok(meshes_json(s))
        }),
    ]
}

/// A mesh file as saved in a project: (file name, bytes).
type MeshFile = (String, Vec<u8>);

static GPU_TRACER: OnceLock<Arc<dyn Tracer>> = OnceLock::new();

/// Cast bake rays with `tracer` (the apps install the render crate's GPU baker at start-up);
/// the first call wins.
pub fn install_gpu_tracer(tracer: Arc<dyn Tracer>) {
    let _ = GPU_TRACER.set(tracer);
}

/// A mesh file name made unique among `taken` (and safe as a project file entry).
fn unique_name(file_name: &str, taken: &[&str]) -> String {
    let leaf = std::path::Path::new(file_name).file_name().and_then(|n| n.to_str()).unwrap_or("mesh");
    let base = texturecraft_formats::sanitize_resource_name(leaf);
    let mut name = base.clone();
    let mut n = 2;
    while taken.contains(&name.as_str()) {
        name = format!("{n}_{base}");
        n += 1;
    }
    name
}

fn parse_mesh(file_name: &str, bytes: Vec<u8>) -> Result<BakeMesh> {
    let h = texturecraft_formats::import_high_poly_bytes(file_name, &bytes).map_err(|e| EngineError::Other(format!("{file_name}: {e}")))?;
    let base_color = h.base_color.into_iter().map(|c| c.map(Arc::new)).collect();
    Ok(BakeMesh { name: file_name.to_string(), bytes: Arc::new(bytes), mesh: Arc::new(h.mesh), base_color })
}

impl Session {
    pub fn bake_task(&self, texture_set: usize) -> Result<BakeTask> {
        let set = self.doc.project.texture_set(texture_set)?;
        if !set.has_enabled_bake_tiles() {
            return Err(EngineError::Other(format!("texture set {} has no UV tiles selected for baking", set.name)));
        }

        let needs_low_color = set.bake.maps.opacity.is_enabled
            || (set.bake.maps.id.is_enabled && set.bake.maps.id.parameters.source == texturecraft_core::IdColorSource::MaterialColor);
        let low_base_color = if needs_low_color && (set.bake.high_poly.use_low_as_high || self.bake_meshes.high.is_empty()) {
            self.bake_meshes
                .low_base_color
                .get_or_init(|| match &self.mesh_source {
                    Some((extension, bytes)) => texturecraft_formats::import_high_poly_bytes(&format!("mesh.{extension}"), bytes)
                        .map(|imported| Arc::new(imported.base_color.into_iter().map(|color| color.map(Arc::new)).collect()))
                        .map_err(|error| error.to_string()),
                    None => Ok(Arc::new(Vec::new())),
                })
                .clone()
                .map_err(EngineError::Other)?
        } else {
            Arc::new(Vec::new())
        };

        let high = self.bake_high(texture_set)?;
        let cage = self.bake_cage(texture_set, high.is_some())?;
        let skew = self.bake_paint(texture_set, high.is_some(), BakePaintTarget::Skew)?;
        let cage_paint = self.bake_paint(texture_set, high.is_some(), BakePaintTarget::Cage)?;

        let grid = set.tile_grid();
        let tiles = grid.map_or((1, 1), |grid| (grid.cols, grid.rows));
        let excluded_tiles: Vec<_> = set.uv_tiles.iter().filter(|tile| set.bake_excluded_tiles.contains(tile)).filter_map(|tile| grid?.block(*tile)).collect();
        let previous_maps = if excluded_tiles.is_empty() { Default::default() } else { self.doc.baked.get(&texture_set).cloned().unwrap_or_default() };

        let tracer = GPU_TRACER.get().cloned();
        Ok(BakeTask {
            low_base_color,
            ts: texture_set,
            mesh: self.bake_mesh(texture_set)?,
            bvh: self.bvh.clone(),
            material: set.material,
            settings: set.bake.clone(),
            high,
            cage,
            skew,
            cage_paint,
            tiles,
            excluded_tiles,
            previous_maps,
            tracer,
        })
    }

    /// What texture set `texture_set`'s bake reads to place its transfer rays; `None` when the set bakes without a high poly or misses its cage file
    pub fn bake_cage_inputs(&self, texture_set: usize) -> Result<Option<BakeCageInputs>> {
        let set = self.doc.project.texture_set(texture_set)?;
        let Some(high) = self.bake_high(texture_set)? else { return Ok(None) };
        let Ok(cage) = self.bake_cage(texture_set, true) else { return Ok(None) };

        Ok(Some(BakeCageInputs {
            mesh: self.bake_mesh(texture_set)?,
            bvh: self.bvh.clone(),
            material: set.material,
            settings: set.bake.clone(),
            high,
            cage,
            skew: self.bake_paint(texture_set, true, BakePaintTarget::Skew)?,
            cage_paint: self.bake_paint(texture_set, true, BakePaintTarget::Cage)?,
        }))
    }

    /// The merged high polys texture set `texture_set` bakes from, unless it bakes from the low poly
    fn bake_high(&self, texture_set: usize) -> Result<Option<MergedHigh>> {
        let set = self.doc.project.texture_set(texture_set)?;
        Ok(if set.bake.high_poly.use_low_as_high { None } else { self.bake_meshes.merged.clone() })
    }

    /// Texture set `texture_set`'s cage mesh when it bakes from a high poly with a Custom File cage
    fn bake_cage(&self, texture_set: usize, has_high: bool) -> Result<Option<Arc<Mesh>>> {
        let set = self.doc.project.texture_set(texture_set)?;
        if !has_high || set.bake.cage.kind != texturecraft_core::CageType::CustomFile {
            return Ok(None);
        }

        let cage = self.bake_meshes.cage.as_ref().ok_or_else(|| EngineError::Other("Custom File requires a cage mesh".into()))?;
        Ok(Some(cage.mesh.clone()))
    }

    /// Texture set `texture_set`'s painted skew or cage map, used only when it bakes from a high poly
    fn bake_paint(&self, texture_set: usize, has_high: bool, target: BakePaintTarget) -> Result<Option<Arc<BakePaintMap>>> {
        let set = self.doc.project.texture_set(texture_set)?;
        let paint = self.doc.bake_paint.get(&(texture_set, target)).filter(|_| has_high);
        paint.map(|map| super::layout::paint_for_bake(map, set)).transpose()
    }

    /// The low poly texture set `texture_set` bakes: UV tiles of mixed resolutions move into its atlas
    fn bake_mesh(&self, texture_set: usize) -> Result<Arc<Mesh>> {
        let set = self.doc.project.texture_set(texture_set)?;
        Ok(match set.tile_grid().filter(|_| !set.tile_resolutions.is_empty()) {
            Some(grid) => {
                let mut mesh = (*self.source).clone();
                mesh.map_material_uvs(set.material, |uv| texturecraft_geom::glam::Vec2::from_array(grid.to_atlas(uv.to_array())));
                Arc::new(mesh)
            }
            None => self.mesh.clone(),
        })
    }

    /// Store a finished bake as one undo step
    pub fn apply_bake(&mut self, texture_set: usize, output: BakeOutput) -> Result<()> {
        let tracer = output.tracer.clone();
        self.edit("Bake mesh maps", |session| session.store_bake(texture_set, output))?;
        self.log.push(format!("Baked texture set {texture_set} ({tracer})"));
        Ok(())
    }

    fn store_bake(&mut self, texture_set: usize, output: BakeOutput) -> Result<()> {
        let entry = self.doc.baked.entry(texture_set).or_default();
        if entry.resolution != output.resolution || entry.maps.keys().any(|map| entry.height(*map) != output.height) {
            entry.maps.clear();
        }
        entry.resolution = output.resolution;
        for (map, pixels) in output.maps {
            entry.maps.insert(map, Arc::new(pixels));
        }
        self.doc.project.texture_set_mut(texture_set)?.baked = entry.maps.keys().copied().collect();
        Ok(())
    }

    /// Bake texture set `ts` now (blocking), on the CPU when `cpu`; returns where the rays ran.
    pub fn bake_now(&mut self, ts: usize, cpu: bool) -> Result<String> {
        let mut task = self.bake_task(ts)?;
        if cpu {
            task.tracer = None;
        }
        let out = task.run(&Progress::new())?;
        let tracer = out.tracer.clone();
        self.apply_bake(ts, out)?;
        Ok(tracer)
    }

    /// Add a high-definition mesh from file bytes (the web picker, the desktop dialog and
    /// `bake.highPoly.add`); every texture set then bakes from the high polys. Returns its name.
    pub fn add_high_poly_bytes(&mut self, file_name: &str, bytes: Vec<u8>) -> Result<String> {
        let mut m = parse_mesh(file_name, bytes)?;
        let taken: Vec<&str> = self.bake_meshes.high.iter().map(|h| h.name.as_str()).collect();
        m.name = unique_name(file_name, &taken);
        let textures = m.base_color.iter().flatten().filter(|c| !c.texels.is_empty()).count();
        self.log.push(format!("High poly {}: {} triangles, {textures} base color textures", m.name, m.mesh.triangle_count()));
        let name = m.name.clone();
        self.bake_meshes.high.push(m);
        self.bake_meshes.rebuild();
        for set in &mut self.doc.project.texture_sets {
            set.bake.high_poly.use_low_as_high = false;
        }
        self.modified = true;
        Ok(name)
    }

    /// Remove high-definition mesh `index`.
    pub fn remove_high_poly(&mut self, index: usize) -> Result<BakeMesh> {
        if index >= self.bake_meshes.high.len() {
            return Err(EngineError::Other(format!("no high poly mesh {index}")));
        }
        let removed = self.bake_meshes.high.remove(index);
        self.bake_meshes.rebuild();
        self.modified = true;
        Ok(removed)
    }

    /// Use a cage mesh (same triangles as the low poly) from file bytes.
    pub fn set_cage_bytes(&mut self, file_name: &str, bytes: Vec<u8>) -> Result<String> {
        let mut m = parse_mesh(file_name, bytes)?;
        if m.mesh.triangle_count() != self.mesh.triangle_count() {
            return Err(EngineError::Other(format!(
                "the cage has {} triangles but the low poly has {}; export the cage from the low poly",
                m.mesh.triangle_count(),
                self.mesh.triangle_count()
            )));
        }
        m.name = unique_name(file_name, &[]);
        m.base_color.clear();
        let name = m.name.clone();
        self.bake_meshes.cage = Some(m);
        self.modified = true;
        Ok(name)
    }

    /// Add mesh file `f` as a high-definition mesh, or set it as the cage, logging what it could
    /// not fold in. Replies like `bake.highPoly.add` / `bake.cage.set`: `{name, meshes}`.
    pub fn add_bake_mesh(&mut self, f: texturecraft_formats::MeshFile, cage: bool) -> Result<Value> {
        let name = if cage { self.set_cage_bytes(&f.name, f.bytes)? } else { self.add_high_poly_bytes(&f.name, f.bytes)? };
        self.log.extend(f.warnings);
        Ok(json!({ "name": name, "meshes": meshes_json(self) }))
    }

    /// Restore the bake meshes saved in a project file; unreadable ones are logged and skipped.
    pub(crate) fn load_bake_meshes(&mut self, high: Vec<MeshFile>, cage: Option<MeshFile>) {
        for (name, bytes) in high {
            match parse_mesh(&name, bytes) {
                Ok(m) => self.bake_meshes.high.push(m),
                Err(e) => self.log.push(format!("skipped high poly mesh: {e}")),
            }
        }
        self.bake_meshes.rebuild();
        if let Some((name, bytes)) = cage
            && let Err(e) = self.set_cage_bytes(&name, bytes)
        {
            self.log.push(format!("skipped cage mesh: {e}"));
        }
    }

    pub(crate) fn load_bake_paint(&mut self, maps: BTreeMap<(usize, BakePaintTarget), texturecraft_formats::BakePaintData>) {
        for (key, data) in maps {
            if key.0 >= self.doc.project.texture_sets.len() {
                self.log.push(format!("skipped bake paint map for missing texture set {}", key.0));
                continue;
            }
            let map = BakePaintMap { width: data.width, height: data.height, values: data.values };
            match map.validate() {
                Ok(()) => {
                    self.doc.bake_paint.insert(key, Arc::new(map));
                }
                Err(error) => self.log.push(format!("skipped bake paint map: {error}")),
            }
        }
    }

    pub(crate) fn saved_bake_paint(&self) -> BTreeMap<(usize, BakePaintTarget), texturecraft_formats::BakePaintData> {
        self.doc
            .bake_paint
            .iter()
            .map(|(key, map)| (*key, texturecraft_formats::BakePaintData { width: map.width, height: map.height, values: map.values.clone() }))
            .collect()
    }

    /// The bake meshes for a project file: (high polys, cage).
    pub(crate) fn saved_bake_meshes(&self) -> (Vec<MeshFile>, Option<MeshFile>) {
        let entry = |m: &BakeMesh| (m.name.clone(), m.bytes.as_ref().clone());
        (self.bake_meshes.high.iter().map(entry).collect(), self.bake_meshes.cage.as_ref().map(entry))
    }
}

/// `bake.highPoly.list`: the high polys and the cage (also part of `project.inspect`).
pub fn meshes_json(s: &Session) -> Value {
    let one = |m: &BakeMesh| json!({ "name": m.name, "triangles": m.mesh.triangle_count(), "parts": m.mesh.submeshes.iter().map(|p| p.name.as_str()).collect::<Vec<_>>() });
    let painted = |target| s.doc.bake_paint.get(&(s.ts(), target)).map(|map| json!({ "width": map.width, "height": map.height }));
    json!({ "highPoly": s.bake_meshes.high.iter().map(one).collect::<Vec<_>>(), "cage": s.bake_meshes.cage.as_ref().map(one), "skew": painted(BakePaintTarget::Skew), "paintedCage": painted(BakePaintTarget::Cage), "paintMaps": s.doc.bake_paint.iter().map(|((texture_set, target), map)| {
        let default = match target { BakePaintTarget::Cage => 1.0, BakePaintTarget::Skew => 0.0 };
        json!({"textureSet": texture_set, "target": target, "width": map.width, "height": map.height, "paintedTexels": map.values.iter().filter(|value| **value != default).count()})
    }).collect::<Vec<_>>() })
}

#[cfg(not(target_arch = "wasm32"))]
fn read_mesh_file(cmd: &str, p: &Value) -> Result<texturecraft_formats::MeshFile> {
    let path = strp(p, "path").ok_or_else(|| bad(cmd, "`path` is required"))?;
    crate::file_cmds::read_mesh(cmd, path)
}

#[cfg(target_arch = "wasm32")]
fn read_mesh_file(_: &str, _: &Value) -> Result<texturecraft_formats::MeshFile> {
    Err(EngineError::Other("file paths are not available in the browser; use the bake panel's file picker".into()))
}

fn high_add(s: &mut Session, p: &Value) -> Result<Value> {
    s.add_bake_mesh(read_mesh_file("bake.highPoly.add", p)?, false)
}

fn high_remove(s: &mut Session, p: &Value) -> Result<Value> {
    if boolp(p, "all") == Some(true) {
        s.bake_meshes.high.clear();
        s.bake_meshes.rebuild();
        s.modified = true;
        return Ok(meshes_json(s));
    }
    let index = match (u64p(p, "index"), strp(p, "name")) {
        (Some(i), _) => usize::try_from(i).unwrap_or(usize::MAX),
        (None, Some(n)) => s.bake_meshes.high.iter().position(|h| h.name == n).ok_or_else(|| bad("bake.highPoly.remove", format!("no high poly `{n}`")))?,
        (None, None) => return Err(bad("bake.highPoly.remove", "`index`, `name` or `all` is required")),
    };
    s.remove_high_poly(index)?;
    Ok(meshes_json(s))
}

fn cage_set(s: &mut Session, p: &Value) -> Result<Value> {
    s.add_bake_mesh(read_mesh_file("bake.cage.set", p)?, true)
}

/// Texture sets named by `ts` (default: the active one).
fn sets_param(s: &Session, cmd: &str, p: &Value) -> Result<Vec<usize>> {
    let n = s.set_count();
    let one = |v: &Value| v.as_u64().map(|t| t as usize).filter(|t| *t < n).ok_or_else(|| bad(cmd, format!("invalid texture set {v}")));
    match p.get("ts") {
        Some(Value::String(a)) if a == "all" => Ok((0..n).collect()),
        Some(Value::String(a)) if a == "selected" => {
            Ok((0..n).filter(|t| s.doc.project.texture_sets.get(*t).is_some_and(|set| set.bake.common.is_enabled)).collect())
        }
        Some(Value::Array(a)) => a.iter().map(one).collect(),
        Some(v) => Ok(vec![one(v)?]),
        None => Ok(vec![s.ts()]),
    }
}

/// Parameters shared by the current bake command and UI
pub fn settings_params(settings: &BakeSettings) -> Value {
    let mut ao = rays_params(&settings.maps.ao.parameters.rays);
    ao["ignoreBackface"] = match settings.maps.ao.parameters.ignore_backface {
        BackfaceMode::Never => "never",
        BackfaceMode::Always => "always",
        BackfaceMode::ByMeshName => "byMeshName",
    }
    .into();
    ao["selfOcclusion"] = match settings.maps.ao.parameters.self_occlusion {
        SelfOcclusion::Always => "always",
        SelfOcclusion::SameMeshName => "sameMeshName",
    }
    .into();

    json!({
        "resolution": settings.common.resolution.pixels(),
        "antialiasing": settings.common.antialiasing.samples_per_axis(),
        "dilation": settings.common.dilation,
        "ao": ao,
        "thickness": rays_params(&settings.maps.thickness.parameters.rays),
        "bentNormal": rays_params(&settings.maps.bent_normal.parameters.rays),
        "curvatureMethod": match settings.maps.curvature.parameters.method { CurvatureMethod::NormalMap => "normalMap", CurvatureMethod::Mesh => "mesh" },
        "positionMode": match settings.maps.position.parameters.normalization { PositionMode::BoundingBox => "boundingBox", PositionMode::BoundingSphere => "boundingSphere", PositionMode::None => "none" },
        "maps": settings.maps.enabled().map(|m| m.id()).collect::<Vec<_>>(),
        "enabled": settings.common.is_enabled,
        "useLowAsHigh": settings.high_poly.use_low_as_high,
        "maxFrontal": settings.cage.max_frontal,
        "maxRear": settings.cage.max_rear,
        "averageNormals": settings.cage.average_normals,
        "lowSuffix": settings.high_poly.low_suffix,
        "highSuffix": settings.high_poly.high_suffix,
    })
}

/// Apply only the bake parameters supplied by the caller
fn apply_settings(settings: &mut BakeSettings, params: &Value, command: &str) -> Result<()> {
    // Preserve the legacy flat-parameter limits for existing callers
    const MAX_DILATION_PIXELS: u32 = 256;
    const MAX_SUFFIX_CHARS: usize = 64;

    if let Some(model) = params.get("model") {
        let replacement: BakeSettings = serde_json::from_value(model.clone()).map_err(|error| bad(command, error.to_string()))?;
        validate_model_settings(&replacement, command)?;
        let is_enabled = settings.common.is_enabled;
        *settings = replacement;
        if boolp(params, "preserveEnabled") == Some(true) {
            settings.common.is_enabled = is_enabled;
        }
    }

    if let Some(patch) = params.get("patch") {
        let mut model = json!(settings);
        model_patch::apply_patch(&mut model, patch, command)?;
        let updated: BakeSettings = serde_json::from_value(model).map_err(|error| bad(command, error.to_string()))?;
        validate_model_settings(&updated, command)?;
        *settings = updated;
    }

    if let Some(resolution) = u64p(params, "resolution") {
        settings.common.resolution = u32::try_from(resolution)
            .ok()
            .and_then(BakeResolution::from_pixels)
            .ok_or_else(|| bad(command, "resolution must be a power of two from 16 to 8192"))?;
    }

    if let Some(antialiasing) = u64p(params, "antialiasing") {
        settings.common.antialiasing = u32::try_from(antialiasing)
            .ok()
            .and_then(BakeAntialiasing::from_samples_per_axis)
            .ok_or_else(|| bad(command, "antialiasing must be 1, 2, 4 or 8"))?;
    }

    if let Some(dilation) = u64p(params, "dilation") {
        settings.common.dilation = dilation.min(u64::from(MAX_DILATION_PIXELS)) as u32;
    }

    // Per-map values override the shared ray settings
    for ray_settings in [&mut settings.maps.ao.parameters.rays, &mut settings.maps.thickness.parameters.rays, &mut settings.maps.bent_normal.parameters.rays] {
        apply_rays(ray_settings, params, command)?;
    }

    for (key, ray_settings) in [
        ("ao", &mut settings.maps.ao.parameters.rays),
        ("thickness", &mut settings.maps.thickness.parameters.rays),
        ("bentNormal", &mut settings.maps.bent_normal.parameters.rays),
    ] {
        if let Some(map_params) = params.get(key) {
            apply_rays(ray_settings, map_params, command)?;
        }
    }

    if let Some(ao) = params.get("ao") {
        if let Some(value) = ao.get("ignoreBackface") {
            let mode = value.as_str().ok_or_else(|| bad(command, "ignoreBackface must be never, always or byMeshName"))?;
            settings.maps.ao.parameters.ignore_backface = match mode {
                "never" => BackfaceMode::Never,
                "always" => BackfaceMode::Always,
                "byMeshName" => BackfaceMode::ByMeshName,
                _ => return Err(bad(command, "ignoreBackface must be never, always or byMeshName")),
            };
        }

        if let Some(self_occlusion) = strp(ao, "selfOcclusion") {
            settings.maps.ao.parameters.self_occlusion = match self_occlusion {
                "always" => SelfOcclusion::Always,
                "sameMeshName" => SelfOcclusion::SameMeshName,
                _ => return Err(bad(command, "selfOcclusion must be always or sameMeshName")),
            };
        }
    }

    if let Some(position_mode) = strp(params, "positionMode") {
        settings.maps.position.parameters.normalization = match position_mode {
            "boundingBox" => PositionMode::BoundingBox,
            "boundingSphere" => PositionMode::BoundingSphere,
            "none" => PositionMode::None,
            _ => return Err(bad(command, "positionMode must be boundingBox, boundingSphere or none")),
        };
    }

    if let Some(value) = params.get("curvatureMethod") {
        let method = value.as_str().ok_or_else(|| bad(command, "curvatureMethod must be normalMap or mesh"))?;
        settings.maps.curvature.parameters.method = match method {
            "normalMap" => CurvatureMethod::NormalMap,
            "mesh" => CurvatureMethod::Mesh,
            _ => return Err(bad(command, "curvatureMethod must be normalMap or mesh")),
        };
    }

    for (key, setting, minimum, maximum) in [("maxFrontal", &mut settings.cage.max_frontal, 0.0, 1.0), ("maxRear", &mut settings.cage.max_rear, 0.0, 1.0)] {
        if let Some(value) = f32p(params, key).filter(|value| value.is_finite()) {
            *setting = value.clamp(minimum, maximum);
        }
    }

    for (key, setting) in [
        ("enabled", &mut settings.common.is_enabled),
        ("useLowAsHigh", &mut settings.high_poly.use_low_as_high),
        ("averageNormals", &mut settings.cage.average_normals),
    ] {
        if let Some(value) = boolp(params, key) {
            *setting = value;
        }
    }

    if let Some(maps) = params.get("maps").and_then(Value::as_array) {
        let selected = maps
            .iter()
            .map(|map| map.as_str().and_then(BakedMap::parse).ok_or_else(|| bad(command, format!("unknown mesh map {map}"))))
            .collect::<Result<Vec<_>>>()?;
        settings.maps.select(selected);
    }

    for (key, setting) in [("lowSuffix", &mut settings.high_poly.low_suffix), ("highSuffix", &mut settings.high_poly.high_suffix)] {
        if let Some(suffix) = strp(params, key) {
            *setting = suffix.chars().take(MAX_SUFFIX_CHARS).collect();
        }
    }

    Ok(())
}

fn validate_model_settings(settings: &BakeSettings, command: &str) -> Result<()> {
    let range = |name: &str, value: f32, minimum: f32, maximum: f32| {
        if value.is_finite() && (minimum..=maximum).contains(&value) { Ok(()) } else { Err(bad(command, format!("{name} must be in {minimum}..={maximum}"))) }
    };
    if settings.common.dilation > 128 {
        return Err(bad(command, "dilation must be at most 128"));
    }
    for rays in [&settings.maps.ao.parameters.rays, &settings.maps.thickness.parameters.rays, &settings.maps.bent_normal.parameters.rays] {
        if !(1..=256).contains(&rays.rays) {
            return Err(bad(command, "secondary rays must be in 1..=256"));
        }
        range("minimum occluder distance", rays.min_distance, 0.00001, 1.0)?;
        range("maximum occluder distance", rays.max_distance, 0.00001, if rays.relative_to_bounding_box { 1.0 } else { 10.0 })?;
        range("spread angle", rays.spread, 0.0, 180.0)?;
        if rays.min_distance > rays.max_distance {
            return Err(bad(command, "minimum occluder distance must not exceed maximum distance"));
        }
    }
    let curvature = &settings.maps.curvature.parameters;
    if !(1..=256).contains(&curvature.mesh.rays) {
        return Err(bad(command, "curvature rays must be in 1..=256"));
    }
    if !curvature.mesh.auto_tonemapping && curvature.mesh.value_min >= curvature.mesh.value_max {
        return Err(bad(command, "curvature minimum must be below maximum"));
    }
    range("sampling radius", curvature.mesh.sampling_radius, 0.00001, if curvature.mesh.relative_to_bounding_box { 10.0 } else { 1.0 })?;
    for (name, value, minimum, maximum) in [
        ("frontal distance", settings.cage.max_frontal, 0.0, 0.05),
        ("rear distance", settings.cage.max_rear, 0.0, 0.05),
        ("edge distance", settings.skew.edge_distance, 0.0, 1.0),
        ("edge contrast", settings.skew.edge_contrast, 0.0, 1.0),
        ("ground plane offset", settings.maps.ao.parameters.ground_plane.offset, 0.0, 100.0),
        ("curvature details", curvature.normal_map.details, 0.0, 1.0),
        ("seams intensity", curvature.normal_map.seams_intensity, 0.0, 1.0),
        ("value bounds minimum", curvature.mesh.value_min, -10.0, 10.0),
        ("value bounds maximum", curvature.mesh.value_max, -10.0, 10.0),
        ("height scaling divisor", settings.maps.height.parameters.scaling_divisor, 0.00001, 100.0),
    ] {
        range(name, value, minimum, maximum)?;
    }
    Ok(())
}

/// Ray settings supported by the current bake command
fn rays_params(settings: &RaySettings) -> Value {
    json!({
        "rays": settings.rays,
        "maxDistance": settings.max_distance,
        "spread": settings.spread,
        "distribution": match settings.distribution {
            RayDistribution::Cosine => "cosine",
            RayDistribution::Uniform => "uniform",
        },
    })
}

/// Apply only the ray parameters supplied by the caller
fn apply_rays(settings: &mut RaySettings, params: &Value, command: &str) -> Result<()> {
    if let Some(count) = u64p(params, "rays") {
        settings.rays = count.clamp(1, u64::from(MAX_RAYS)) as u32;
    }

    for (key, setting, minimum, maximum) in [("maxDistance", &mut settings.max_distance, 0.001, 10.0), ("spread", &mut settings.spread, 0.0, 180.0)] {
        if let Some(value) = f32p(params, key).filter(|value| value.is_finite()) {
            *setting = value.clamp(minimum, maximum);
        }
    }

    if let Some(distribution) = strp(params, "distribution") {
        settings.distribution = match distribution {
            "cosine" => RayDistribution::Cosine,
            "uniform" => RayDistribution::Uniform,
            _ => return Err(bad(command, "distribution must be cosine or uniform")),
        };
    }

    Ok(())
}

fn updated_settings(session: &Session, params: &Value, command: &str) -> Result<Vec<(usize, BakeSettings)>> {
    sets_param(session, command, params)?
        .into_iter()
        .map(|index| {
            let mut settings = session.doc.project.texture_set(index)?.bake.clone();
            apply_settings(&mut settings, params, command)?;
            Ok((index, settings))
        })
        .collect()
}

fn set_settings(session: &mut Session, params: &Value, command: &str) -> Result<Vec<usize>> {
    let next = updated_settings(session, params, command)?;
    let sets = next.iter().map(|(index, _)| *index).collect();
    if next.iter().any(|(index, settings)| session.doc.project.texture_sets.get(*index).is_some_and(|set| set.bake != *settings)) {
        session.edit("Bake Settings", |session| {
            for (index, settings) in next {
                session.doc.project.texture_set_mut(index)?.bake = settings;
            }
            Ok(())
        })?;
    }
    Ok(sets)
}

fn settings(session: &mut Session, params: &Value) -> Result<Value> {
    let sets = set_settings(session, params, "bake.settings")?;
    let result: Vec<Value> = sets
        .iter()
        .filter_map(|set_index| {
            session
                .doc
                .project
                .texture_sets
                .get(*set_index)
                .map(|texture_set| json!({ "ts": set_index, "settings": settings_params(&texture_set.bake), "model": &texture_set.bake }))
        })
        .collect();

    Ok(json!(result))
}

fn bake_run(session: &mut Session, params: &Value) -> Result<Value> {
    let next: Vec<_> = updated_settings(session, params, "bake.run")?
        .into_iter()
        .filter(|(index, _)| session.doc.project.texture_sets.get(*index).is_some_and(|texture_set| texture_set.has_enabled_bake_tiles()))
        .collect();
    let sets: Vec<usize> = next.iter().map(|(index, _)| *index).collect();
    if sets.is_empty() {
        return Ok(json!({"baked": [], "tracer": []}));
    }

    let cpu = boolp(params, "cpu") == Some(true);
    let tracers = session.edit("Bake mesh maps", |session| {
        let mut tracers = Vec::with_capacity(next.len());
        for (index, settings) in next {
            session.doc.project.texture_set_mut(index)?.bake = settings;
            let mut task = session.bake_task(index)?;
            if cpu {
                task.tracer = None;
            }
            let output = task.run(&Progress::new())?;
            tracers.push(output.tracer.clone());
            session.store_bake(index, output)?;
        }
        Ok(tracers)
    })?;
    for (index, tracer) in sets.iter().zip(&tracers) {
        session.log.push(format!("Baked texture set {index} ({tracer})"));
    }
    session.update();
    Ok(json!({ "baked": sets, "tracer": tracers }))
}

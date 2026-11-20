//! Files: projects (`.tcraft`), mesh import, texture export. Byte-based functions work on the
//! web; path-based commands are native only.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};
#[cfg(not(target_arch = "wasm32"))]
use texturecraft_core::{NormalFormat, PROJECT_EXT};
use texturecraft_core::{Project, TextureSet};
use texturecraft_formats::{ExportInput, ExportOptions, ExportedFile, FileFormat, MapOverride, Padding, ProjectFile};
use texturecraft_geom::{Mesh, primitives};

use crate::commands::{CommandSpec, always, bad, normal_format_param, strp, u64p};
use crate::session::{Document, Session};
use crate::{EngineError, Result, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "file.new",
            "New project from a mesh file",
            [],
            None,
            "{mesh: path to .glb/.gltf/.fbx/.dae/.usd/.usda/.usdc/.usdz/.abc/.obj/.ply/.stl, resolution?: 2048, name?, uvTiles?: bool (UDIM; default: when the UVs leave tile 1001), template?: project template name, importCameras?: bool (default true: keep the cameras stored in the file)}",
            always,
            new_from_mesh
        ),
        cmd!("file.open", "Open...", ["File"], Some("Ctrl+O"), "{path: .tcraft project or mesh file}", always, open),
        cmd!("file.save", "Save", ["File"], Some("Ctrl+S"), "{path?: .tcraft (default: the project's own file)}", always, save),
        cmd!("file.saveAs", "Save as...", ["File"], Some("Ctrl+Shift+S"), "{path}", always, save_as),
        cmd!("file.saveCopy", "Save as copy...", ["File"], None, "{path}", always, save_copy),
        cmd!("file.close", "Close", ["File"], Some("Ctrl+F4"), "{}", always, close),
        cmd!(
            "file.reimportMesh",
            "Reimport mesh",
            ["Edit"],
            Some("Ctrl+Shift+R"),
            "{path?, preserveStrokes?: bool (painted pixels follow the surface when the UVs changed; default false)} → {preserved?: texels re-projected}",
            always,
            reimport
        ),
        cmd!(
            "file.changeMesh",
            "Change mesh",
            [],
            None,
            "{path: the new mesh file, preserveStrokes?: bool (painted pixels follow the surface where the UVs differ; default true)} → {preserved?: texels re-projected}. Keeps layers, texture sets and settings; texture sets map to the new mesh by material index",
            always,
            change_mesh
        ),
        cmd!(
            "file.exportTextures",
            "Export textures",
            ["File"],
            None,
            "{dir, preset?: built-in or user preset name, or a preset object {name, maps}, ts?: index|[indexes]|\"all\", format?: bmp|jpeg|jng|pgm|png|targa|tiff|hdr|exr|webp|psd (or the extension), bitDepth?: 8|16|32 (32 is float for tiff, hdr and exr; 16 is half float for exr), dithering?: bool (8-bit files), padding?: none|dilation|dilationDefault|diffusion|infinite, dilation?: px, size?: px, normalFormat?: openGl|directX, maps?: [{map: index into the preset's maps, enabled?: bool (false leaves the map out), format?, bitDepth?, dithering?}] (per map file type and bit depth), colorSpace?: sRGB (default for 8/16-bit files)|Linear Rec.709 (default for exr, hdr)|ACEScg|ACES2065-1 (colour maps are converted, data maps never are; 8/16-bit files clamp to 0-1; a chosen space is also recorded in the colour maps: an ICC profile in PNG, JPEG, TIFF and WebP, the chromaticities in EXR; TGA, BMP, HDR, JNG, PGM and PSD exports warn about missing metadata), materialX?: bool (also write a .mtlx OpenPBR material per texture set; needs the \"OpenPBR (separate maps)\" preset)}",
            always,
            export_textures
        ),
        cmd!(
            "file.exportMesh",
            "Export Mesh...",
            ["File"],
            None,
            "{path: .obj, .glb, .usda, .usdc or .usdz file, format?: obj|glb|usda|usdc|usdz (default: from the extension, else obj); obj writes its .mtl and textures, usda/usdc their textures, next to the file} → {path, textures?}",
            always,
            export_mesh
        ),
        cmd!(
            "export.plan",
            "List export outputs",
            [],
            None,
            "{preset?: name or preset object, ts?, format?, bitDepth?, dithering?, size?, colorSpace?, maps?} → [{ts, map, file, format, bitDepth, dithering, size, sourceSize, warnings}] (keeps the first file per case-insensitive name and warns about later duplicates)",
            always,
            plan
        ),
    ]
}

fn fe(e: texturecraft_formats::FormatError) -> EngineError {
    EngineError::Other(e.to_string())
}

fn ext_of(name: &str) -> String {
    std::path::Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

fn stem_of(name: &str) -> String {
    std::path::Path::new(name).file_stem().and_then(|e| e.to_str()).unwrap_or("Untitled").to_string()
}

/// The leaf of an imported mesh file's name (kept in [`Project::mesh_file`]).
fn leaf_of(name: &str) -> String {
    std::path::Path::new(name).file_name().and_then(|e| e.to_str()).unwrap_or_default().to_string()
}

/// Read the project's stored mesh `bytes` (extension `ext`) under the name it was imported
/// with, so its unnamed parts keep their names (and the geometry masks that use them).
fn read_stored_mesh(project: &Project, ext: &str, bytes: &[u8]) -> Result<texturecraft_formats::ImportedMesh> {
    let stem = std::path::Path::new(&project.mesh_file).file_stem().and_then(|s| s.to_str()).filter(|s| !s.is_empty()).unwrap_or("mesh");
    texturecraft_formats::import_mesh_bytes(&format!("{stem}.{ext}"), bytes).map_err(fe)
}

impl Session {
    /// The working mesh as Wavefront OBJ (with its own UVs, UDIM tiles included).
    pub fn mesh_obj_bytes(&self) -> Result<Vec<u8>> {
        texturecraft_formats::obj_bytes(&self.source_mesh(), &self.doc.project.name).map_err(fe)
    }

    /// [`Session::mesh_obj_bytes`] with an MTL material library and each material's maps (like
    /// [`Session::mesh_usd`]'s), to write next to it, named after `stem` (the OBJ file's).
    pub fn mesh_obj_with_mtl(&mut self, stem: &str) -> Result<(Vec<u8>, texturecraft_formats::NamedFiles)> {
        // shortcut: UV-tiled sets get no maps (the OBJ keeps the mesh's UDIM UVs and an MTL map is
        // one image); their tiles export through Export textures with `$udim` names.
        let mut mesh = self.source_mesh();
        // A mesh without materials has one implicit material (and texture set).
        if mesh.materials.is_empty() {
            mesh.materials.push("default".into());
        }
        let files = self.material_files(texturecraft_formats::usd_export_options(), false, mesh.materials.len())?;
        texturecraft_formats::obj_with_mtl(&mesh, &self.doc.project.name, stem, &files).map_err(fe)
    }

    /// The working mesh as binary glTF with each material's textures (from its texture set's
    /// composite, through the texture export pipeline) embedded. glTF has no UV tiles: a
    /// UV-tiled set embeds its tile atlas, with the mesh's UVs in atlas space.
    pub fn mesh_glb_bytes(&mut self) -> Result<Vec<u8>> {
        let files = self.material_files(texturecraft_formats::glb_export_options(), true, self.mesh.materials.len())?;
        let textures: Vec<_> = files.into_iter().map(texturecraft_formats::GlbTextures::from_files).collect();
        texturecraft_formats::glb_bytes(&self.mesh, &self.doc.project.name, &textures).map_err(fe)
    }

    /// The working mesh as USD with a UsdPreviewSurface per material (textures like
    /// [`Session::mesh_glb_bytes`]): usdz packages them, usda / usdc list them to write alongside.
    pub fn mesh_usd(&mut self, format: texturecraft_formats::UsdFormat) -> Result<texturecraft_formats::UsdExport> {
        let files = self.material_files(texturecraft_formats::usd_export_options(), true, self.mesh.materials.len())?;
        texturecraft_formats::usd_bytes(&self.mesh, &self.doc.project.name, &files, format).map_err(fe)
    }

    /// The texture sets of the first `materials` materials rendered with `opts` (as a tile atlas
    /// for UV-tiled sets, or nothing for them without `tiled`).
    fn material_files(&mut self, opts: ExportOptions, tiled: bool, materials: usize) -> Result<Vec<Vec<ExportedFile>>> {
        let opts = ExportOptions { atlas: true, ..opts };

        (0..materials)
            .map(|m| match self.texture_set_of_material(m) {
                Some(ts) if tiled || self.doc.project.texture_sets.get(ts).is_some_and(|t| t.tile_grid().is_none()) => self.export_files(&[ts], &opts),
                _ => Ok(Vec::new()),
            })
            .collect()
    }

    /// A new project from mesh file bytes (one texture set per material).
    pub fn from_mesh_bytes(file_name: &str, bytes: &[u8], resolution: u32, name: &str) -> Result<Session> {
        Session::from_mesh_bytes_tiled(file_name, bytes, resolution, name, None)
    }

    /// [`Session::from_mesh_bytes`] with a UV tile mode (see [`Session::new_project_tiled`]).
    pub fn from_mesh_bytes_tiled(file_name: &str, bytes: &[u8], resolution: u32, name: &str, uv_tiles: Option<bool>) -> Result<Session> {
        Self::from_mesh_source(file_name, Cow::Borrowed(bytes), resolution, name, uv_tiles)
    }

    fn from_mesh_source(file_name: &str, bytes: Cow<'_, [u8]>, resolution: u32, name: &str, uv_tiles: Option<bool>) -> Result<Session> {
        let imported = texturecraft_formats::import_mesh_bytes(file_name, &bytes).map_err(fe)?;
        let extension = ext_of(file_name);
        let mut session = Session::new_project_tiled(name, imported.mesh, &format!("mesh.{extension}"), None, resolution, uv_tiles)?;

        // Packed mesh files can have spare capacity that the project would otherwise retain
        let mut source = bytes.into_owned();
        source.shrink_to_fit();
        session.mesh_source = Some((extension, source));
        session.doc.project.mesh_file = leaf_of(file_name);
        session.doc.project.cameras = imported.cameras;

        session.log.warnings("Importer", imported.report.warnings);
        session.log.info(
            "Importer",
            format!(
                "Imported {} ({} triangles, {} materials{})",
                file_name,
                imported.report.triangles,
                imported.report.materials,
                if imported.report.uvs_generated { ", UVs generated" } else { "" }
            ),
        );

        Ok(session)
    }

    /// Open a `.tcraft` project from bytes.
    pub fn from_project_bytes(bytes: &[u8]) -> Result<Session> {
        let pf = texturecraft_formats::load_project_bytes(bytes).map_err(fe)?;
        Session::from_project_file(pf)
    }

    pub fn from_project_file(pf: ProjectFile) -> Result<Session> {
        let mesh = match (&pf.mesh, pf.project.mesh.strip_prefix("builtin:")) {
            (_, Some(id)) => primitives::builtin(id).ok_or_else(|| EngineError::Other(format!("unknown built-in mesh `{id}`")))?,
            (Some((ext, bytes)), None) => read_stored_mesh(&pf.project, ext, bytes)?.mesh,
            (None, None) => return Err(EngineError::Other("the project has no mesh".into())),
        };

        check_materials(&pf.project, &mesh)?;
        let source = pf.mesh.clone();
        let resources = pf.resources.into_iter().map(|(k, v)| (k, std::sync::Arc::new(v))).collect();
        let mut s =
            Session::new(Document { bake_paint: Default::default(), project: pf.project, pixels: pf.pixels, baked: pf.baked, resources }, mesh, source)?;

        s.sync_images();
        s.load_bake_meshes(pf.high_poly, pf.cage);
        s.load_bake_paint(pf.bake_paint);
        s.log.warnings("Project", pf.warnings);

        Ok(s)
    }

    /// The project with independently owned resource bytes.
    pub fn project_file(&self) -> ProjectFile {
        let resources = self.doc.resources.iter().map(|(name, bytes)| (name.clone(), bytes.as_ref().clone())).collect();
        self.project_with_resources(resources)
    }

    /// A save snapshot that shares immutable resource payloads with the document.
    pub fn project_snapshot(&self) -> ProjectFile<Arc<Vec<u8>>> {
        self.project_with_resources(self.doc.resources.clone())
    }

    fn project_with_resources<Resource>(&self, resources: BTreeMap<String, Resource>) -> ProjectFile<Resource> {
        let mut pixels = self.doc.pixels.clone();

        // Drop pixel images whose size no longer matches their texture set
        let size_of = |id: texturecraft_core::LayerId| {
            self.doc.project.locate(id).and_then(|(ts, _, _)| self.doc.project.texture_sets.get(ts)).map(TextureSet::size).or_else(|| {
                self.doc
                    .project
                    .effect(id)
                    .and_then(|(owner, _)| self.doc.project.locate(owner))
                    .and_then(|(ts, _, _)| self.doc.project.texture_sets.get(ts))
                    .map(TextureSet::size)
            })
        };
        pixels.layers.retain(|(id, _), img| size_of(*id) == Some((img.width(), img.height())));
        pixels.masks.retain(|id, img| size_of(*id) == Some((img.width(), img.height())));

        let (high_poly, cage) = self.saved_bake_meshes();
        ProjectFile {
            project: self.doc.project.clone(),
            pixels,
            baked: self.doc.baked.clone(),
            mesh: self.mesh_source.clone(),
            high_poly,
            cage,
            bake_paint: self.saved_bake_paint(),
            thumbnail: None,
            resources,
            warnings: Vec::new(),
        }
    }

    pub fn to_project_bytes(&self) -> Result<Vec<u8>> {
        let mut reservation = texturecraft_formats::RamReservation::for_save().map_err(fe)?;
        reservation.save_bytes(&self.project_snapshot()).map_err(fe)
    }

    /// Render every export file for texture sets `sets` (in memory; works on the web). A file
    /// named like one an earlier set exports is left out, with a warning (see
    /// [`texturecraft_formats::skip_repeated_names`]).
    pub fn export_files(&mut self, sets: &[usize], opts: &ExportOptions) -> Result<Vec<ExportedFile>> {
        if self.solo.is_none() {
            self.update();
        }

        let mesh_name = stem_of(self.doc.project.mesh.trim_start_matches("builtin:"));
        let mut out = Vec::new();
        let empty = Default::default();
        for &ts in sets {
            let cache = self.caches.get(ts).ok_or_else(|| EngineError::Other(format!("no texture set {ts}")))?;
            let composite = self.document_composite(ts, &cache.covered_tiles).ok_or_else(|| EngineError::Other(format!("no texture set {ts}")))?;
            let coverage = cache.gbuf.coverage_mask();
            let input = ExportInput {
                project: &self.doc.project,
                ts,
                composite: &composite,
                baked: self.doc.baked.get(&ts).unwrap_or(&empty),
                coverage: &coverage,
                mesh_name: &mesh_name,
            };
            out.extend(texturecraft_formats::render_maps(&input, opts).map_err(fe)?);
        }

        // A later set's file would replace an earlier set's of the same name
        texturecraft_formats::skip_repeated_names(&mut out);
        Ok(out)
    }
}

fn check_materials(p: &texturecraft_core::Project, mesh: &Mesh) -> Result<()> {
    for t in &p.texture_sets {
        if t.material >= mesh.materials.len().max(1) {
            return Err(EngineError::Other(format!("texture set \"{}\" refers to a material the mesh doesn't have", t.name)));
        }
    }
    Ok(())
}

/// Export options from command params; `preset` is a built-in or user preset name, or an inline
/// preset object (validated).
pub fn export_options(session: &Session, params: &Value, cmd: &str) -> Result<ExportOptions> {
    let preset = match params.get("preset") {
        None | Some(Value::Null) => session.export_preset(texturecraft_formats::DEFAULT_PRESET),
        Some(Value::String(name)) => session.export_preset(name),
        Some(v @ Value::Object(_)) => Some(crate::preset_cmds::preset_param(v, cmd)?),
        Some(_) => return Err(bad(cmd, "`preset` must be a preset name or a preset object")),
    };

    let preset = preset.ok_or_else(|| bad(cmd, format!("unknown export preset {}", params.get("preset").unwrap_or(&Value::Null))))?;
    let mut options = ExportOptions::new(preset);
    if let Some(f) = strp(params, "format") {
        options.format_override = Some(FileFormat::from_extension(f).ok_or_else(|| bad(cmd, format!("unknown format `{f}`")))?);
    }
    if let Some(depth) = u64p(params, "bitDepth") {
        options.bit_depth_override = Some(depth.min(32) as u8);
    }
    options.dither = params.get("dithering").and_then(Value::as_bool).unwrap_or(false);
    let dilation_pixels = u64p(params, "dilation").map(|v| v.min(256) as u32).unwrap_or(16);
    if let Some(pad) = strp(params, "padding") {
        options.padding = match pad {
            "none" => Padding::None,
            "dilation" => Padding::Dilation(dilation_pixels),
            "dilationDefault" => Padding::DilationDefault(dilation_pixels),
            "diffusion" => Padding::DilationDiffusion(dilation_pixels),
            "infinite" => Padding::Infinite,
            other => return Err(bad(cmd, format!("unknown padding `{other}`"))),
        };
    }
    if let Some(size) = u64p(params, "size") {
        options.size = Some(crate::session::check_resolution(u32::try_from(size).unwrap_or(0))?);
    }
    options.normal_format = normal_format_param(cmd, params)?;
    options.maps = map_overrides(params, options.preset.maps.len(), cmd)?;
    match params.get("colorSpace") {
        None | Some(Value::Null) => {}
        Some(Value::String(name)) => {
            options.color_space = Some(
                texturecraft_texel::ColorSpace::from_name(name)
                    .ok_or_else(|| bad(cmd, format!("unknown colorSpace `{name}` (sRGB, Linear Rec.709, ACEScg or ACES2065-1)")))?,
            );
        }

        // A value of the wrong type is an error, not the format's default space
        Some(other) => return Err(bad(cmd, format!("`colorSpace` is a name (sRGB, Linear Rec.709, ACEScg or ACES2065-1), not {other}"))),
    }
    Ok(options)
}

/// `maps`: per map `{map: index into the preset's maps, enabled?, format?, bitDepth?, dithering?}`.
fn map_overrides(params: &Value, map_count: usize, cmd: &str) -> Result<std::collections::BTreeMap<usize, MapOverride>> {
    let mut overrides = std::collections::BTreeMap::new();
    let Some(list) = params.get("maps").filter(|value| !value.is_null()) else { return Ok(overrides) };

    let list = list.as_array().ok_or_else(|| bad(cmd, "`maps` must be a list of {map, enabled?, format?, bitDepth?, dithering?}"))?;
    for settings in list {
        let index = u64p(settings, "map").and_then(|value| usize::try_from(value).ok()).filter(|index| *index < map_count);
        let index = index.ok_or_else(|| bad(cmd, format!("`maps`: `map` must be the index of one of the preset's {map_count} maps")))?;
        let mut options = MapOverride::default();
        if let Some(enabled) = settings.get("enabled") {
            options.enabled = enabled.as_bool().ok_or_else(|| bad(cmd, "`maps`: `enabled` must be true or false"))?;
        }
        if let Some(format) = settings.get("format").filter(|value| !value.is_null()) {
            let name = format.as_str().ok_or_else(|| bad(cmd, "`maps`: `format` must be a file type name"))?;
            options.format = Some(FileFormat::from_extension(name).ok_or_else(|| bad(cmd, format!("unknown format `{name}`")))?);
        }
        if let Some(depth) = settings.get("bitDepth").filter(|value| !value.is_null()) {
            let depth = depth.as_u64().ok_or_else(|| bad(cmd, "`maps`: `bitDepth` must be a nonnegative integer"))?;
            options.bit_depth = Some(depth.min(32) as u8);
        }
        if let Some(dithering) = settings.get("dithering").filter(|value| !value.is_null()) {
            options.dither = Some(dithering.as_bool().ok_or_else(|| bad(cmd, "`maps`: `dithering` must be true or false"))?);
        }

        overrides.insert(index, options);
    }

    Ok(overrides)
}

fn sets_param(s: &Session, p: &Value, cmd: &str) -> Result<Vec<usize>> {
    match p.get("ts") {
        None => Ok((0..s.set_count()).collect()),
        Some(Value::String(a)) if a == "all" => Ok((0..s.set_count()).collect()),

        // Several sets in one command, so a file name two of them share is found before anything is written
        Some(Value::Array(a)) => {
            let mut sets = a.iter().map(|v| set_index(s, v, cmd)).collect::<Result<Vec<_>>>()?;

            // A set listed twice is written once
            let mut seen = std::collections::HashSet::new();
            sets.retain(|t| seen.insert(*t));
            Ok(sets)
        }
        Some(v) => Ok(vec![set_index(s, v, cmd)?]),
    }
}

fn set_index(s: &Session, v: &Value, cmd: &str) -> Result<usize> {
    v.as_u64().and_then(|index| usize::try_from(index).ok()).filter(|index| *index < s.set_count()).ok_or_else(|| bad(cmd, "invalid `ts`"))
}

fn plan(session: &mut Session, params: &Value) -> Result<Value> {
    use texturecraft_formats::export::{list_set_outputs, scale_export_side};

    let options = export_options(session, params, "export.plan")?;
    let project = &session.doc.project;
    let mesh_name = stem_of(project.mesh.trim_start_matches("builtin:"));
    let sets = sets_param(session, params, "export.plan")?;
    let outputs = list_set_outputs(project, &sets, &mesh_name, &options).map_err(fe)?;

    let mut files = Vec::with_capacity(outputs.len());
    for set_outputs in outputs.chunk_by(|(left, _), (right, _)| left == right) {
        let Some((set_index, _)) = set_outputs.first() else { continue };

        let set = project.texture_set(*set_index).map_err(|error| EngineError::Other(error.to_string()))?;
        let block_size = set.block_resolution();
        for (_, output) in set_outputs {
            let source_size = output.tile.map_or(set.resolution, |tile| set.tile_resolution(tile));
            let size = scale_export_side(source_size, block_size, options.size);

            files.push(json!({
                "ts": set_index,
                "map": output.map,
                "file": output.name,
                "format": output.format.extension(),
                "bitDepth": output.bit_depth,
                "dithering": output.dither,
                "size": size,
                "sourceSize": source_size,
                "warnings": output.warnings,
            }));
        }
    }

    Ok(Value::Array(files))
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use std::path::Path;

    pub fn path_param<'a>(p: &'a Value, key: &str, cmd: &str) -> Result<&'a str> {
        strp(p, key).filter(|s| !s.trim().is_empty()).ok_or_else(|| bad(cmd, format!("`{key}` is required")))
    }

    /// `path` made absolute (as given when that fails), for [`Project::mesh_path`].
    pub fn absolute(path: &str) -> String {
        std::path::absolute(Path::new(path)).map_or_else(|_| path.to_string(), |p| p.display().to_string())
    }

    /// Mesh file `path` with the files it references next to it folded in (a `.gltf`'s buffers
    /// and images, a loose USD layer's textures), ready to import and store in the project.
    pub fn read_mesh(cmd: &str, path: &str) -> Result<texturecraft_formats::MeshFile> {
        texturecraft_formats::read_mesh_file(Path::new(path)).map_err(|e| bad(cmd, format!("cannot read {path}: {e}")))
    }

    /// Open a project or a mesh file.
    pub fn open_path(path: &str) -> Result<Session> {
        let p = Path::new(path);
        if ext_of(path) == PROJECT_EXT {
            let pf = texturecraft_formats::load_project(p).map_err(fe)?;
            let mut s = Session::from_project_file(pf)?;
            s.path = Some(path.to_string());
            s.modified = false;
            Ok(s)
        } else {
            let f = read_mesh("file.open", path)?;
            let mut s = Session::from_mesh_bytes(&f.name, &f.bytes, 2048, &stem_of(path))?;
            s.doc.project.mesh_path = absolute(path);
            s.log.warnings("Importer", f.warnings);
            Ok(s)
        }
    }

    pub fn save_to(s: &mut Session, path: &str, adopt: bool) -> Result<Value> {
        let path = if ext_of(path) == PROJECT_EXT { path.to_string() } else { format!("{path}.{PROJECT_EXT}") };
        let mut reservation = texturecraft_formats::RamReservation::for_save().map_err(fe)?;
        reservation.save(Path::new(&path), &s.project_snapshot()).map_err(fe)?;

        if adopt {
            s.path = Some(path.clone());
            s.modified = false;
        }
        s.log.info("Project", format!("Saved {path}"));
        Ok(json!({ "path": path }))
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::{absolute, open_path, read_mesh};

/// `file.new {mesh, name?, resolution?, uvTiles?, template?, importCameras?}`: a new project from the
/// mesh file `mesh`, remembering its path for Reimport mesh.
#[cfg(not(target_arch = "wasm32"))]
fn new_from_mesh(session: &mut Session, params: &Value) -> Result<Value> {
    const DEFAULT_RESOLUTION_PX: u64 = 2048;

    let path = native::path_param(params, "mesh", "file.new")?;
    let template = crate::template_cmds::template_param(session, params, "file.new")?;
    let resolution = u64p(params, "resolution").or(template.as_ref().map(|template| u64::from(template.resolution))).unwrap_or(DEFAULT_RESOLUTION_PX);
    let name = strp(params, "name").map(str::to_string).unwrap_or_else(|| stem_of(path));
    let uv_tiles = crate::commands::boolp(params, "uvTiles");
    let mesh_file = native::read_mesh("file.new", path)?;

    let mut next = Session::from_mesh_source(&mesh_file.name, Cow::Owned(mesh_file.bytes), u32::try_from(resolution).unwrap_or(0), &name, uv_tiles)?;
    next.doc.project.mesh_path = native::absolute(path);
    next.log.warnings("Importer", mesh_file.warnings);
    if crate::commands::boolp(params, "importCameras") == Some(false) {
        next.doc.project.cameras.clear();
    }
    if let Some(template) = &template {
        next.apply_template(template);
    }

    session.switch_to(next);

    Ok(crate::inspect::session(session))
}

#[cfg(not(target_arch = "wasm32"))]
fn open(s: &mut Session, p: &Value) -> Result<Value> {
    let path = native::path_param(p, "path", "file.open")?;
    let next = native::open_path(path)?;
    s.switch_to(next);
    Ok(crate::inspect::session(s))
}

#[cfg(not(target_arch = "wasm32"))]
fn save(s: &mut Session, p: &Value) -> Result<Value> {
    let path = match strp(p, "path") {
        Some(x) => x.to_string(),
        None => s.path.clone().ok_or_else(|| bad("file.save", "the project has no file yet: give `path` (Save as)"))?,
    };
    native::save_to(s, &path, true)
}

#[cfg(not(target_arch = "wasm32"))]
fn save_as(s: &mut Session, p: &Value) -> Result<Value> {
    let path = native::path_param(p, "path", "file.saveAs")?.to_string();
    native::save_to(s, &path, true)
}

#[cfg(not(target_arch = "wasm32"))]
fn save_copy(s: &mut Session, p: &Value) -> Result<Value> {
    let path = native::path_param(p, "path", "file.saveCopy")?.to_string();
    native::save_to(s, &path, false)
}

/// `file.reimportMesh {path?, preserveStrokes?}`: reimport from `path`, or from the project's own
/// copy of the mesh when there is none. A path given is remembered as the project's mesh path.
/// The remembered path is never read implicitly (a project file could point it anywhere on the
/// host); callers that may read host files pass [`Session::remembered_mesh_path`] as `path`.
#[cfg(not(target_arch = "wasm32"))]
fn reimport(s: &mut Session, p: &Value) -> Result<Value> {
    let path = strp(p, "path");
    let file = path.map(|path| native::read_mesh("file.reimportMesh", path)).transpose()?;
    let reply = s.reimport_mesh(file, crate::commands::boolp(p, "preserveStrokes").unwrap_or(false))?;
    if let Some(path) = path {
        s.doc.project.mesh_path = native::absolute(path);
    }
    Ok(reply)
}

#[cfg(not(target_arch = "wasm32"))]
fn change_mesh(s: &mut Session, p: &Value) -> Result<Value> {
    let path = native::path_param(p, "path", "file.changeMesh")?;
    let file = native::read_mesh("file.changeMesh", path)?;
    let reply = s.reimport_mesh(Some(file), crate::commands::boolp(p, "preserveStrokes").unwrap_or(true))?;
    // Only after the swap succeeded: Reimport mesh and Save keep following the new file.
    s.doc.project.mesh_path = native::absolute(path);
    Ok(reply)
}

#[cfg(not(target_arch = "wasm32"))]
impl Session {
    /// The file the mesh was imported from, when it is still there: what Reimport mesh reads again.
    pub fn remembered_mesh_path(&self) -> Option<&str> {
        Some(self.doc.project.mesh_path.as_str()).filter(|m| !m.is_empty() && std::path::Path::new(m).is_file())
    }

    /// Replace the mesh with `file` (or re-read the project's own copy) and keep the project:
    /// texture sets map by material index. Nothing changes unless the new mesh fits the project.
    /// With `preserve_strokes`, painted pixels follow the surface where the UVs changed. Replies
    /// like `file.reimportMesh`: `{preserved?}`.
    pub fn reimport_mesh(&mut self, file: Option<texturecraft_formats::MeshFile>, preserve_strokes: bool) -> Result<Value> {
        let (mesh, source, warnings, mesh_file) = match file {
            Some(f) => {
                let m = texturecraft_formats::import_mesh_bytes(&f.name, &f.bytes).map_err(fe)?.mesh;
                (m, Some((ext_of(&f.name), f.bytes)), f.warnings, leaf_of(&f.name))
            }
            None => match &self.mesh_source {
                Some((ext, bytes)) => {
                    (read_stored_mesh(&self.doc.project, ext, bytes)?.mesh, self.mesh_source.clone(), Vec::new(), self.doc.project.mesh_file.clone())
                }
                None => return Err(bad("file.reimportMesh", "built-in meshes can't be reimported; give `path`")),
            },
        };
        check_materials(&self.doc.project, &mesh)?;
        let mut doc = self.doc.clone();
        if let Some((ext, _)) = &source {
            doc.project.mesh = format!("mesh.{ext}");
            doc.project.mesh_file = mesh_file;
        }
        let mut next = Session::new(doc, mesh, source)?;
        next.log.warnings("Importer", warnings);
        next.path = self.path.clone();
        next.sync_images();

        // The cage uses triangle indices and barycentrics, so it needs identical geometry
        next.bake_meshes = self.bake_meshes.clone();
        next.bake_meshes.low_base_color = Default::default();
        if next.bake_meshes.cage.is_some() && *next.mesh != *self.mesh {
            next.bake_meshes.cage = None;
            next.log.warn("Project", "the cage mesh was dropped: the low-poly mesh changed");
        }
        if self.doc.bake_paint.keys().any(|(_, target)| *target == texturecraft_core::BakePaintTarget::Skew) && self.source.uvs != next.source.uvs {
            next.log.warn("Project", "the skew map is laid over the old UV layout; check it in Bake mode");
        }
        next.modified = true;
        next.project_id = self.project_id;
        let preserved = preserve_strokes.then(|| crate::preserve::preserve_strokes(self, &mut next));
        self.switch_to(next);
        Ok(preserved.map_or(Value::Null, |n| json!({ "preserved": n })))
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn export_textures(s: &mut Session, p: &Value) -> Result<Value> {
    let dir = native::path_param(p, "dir", "file.exportTextures")?.to_string();
    let opts = export_options(s, p, "file.exportTextures")?;
    let sets = sets_param(s, p, "file.exportTextures")?;
    std::fs::create_dir_all(&dir).map_err(|e| bad("file.exportTextures", format!("cannot create {dir}: {e}")))?;
    let files = s.export_files(&sets, &opts)?;
    let mut warnings: Vec<String> = files.iter().flat_map(|f| f.warnings.iter().cloned()).collect();
    let mut written = texturecraft_formats::write_files(&files, std::path::Path::new(&dir)).map_err(fe)?;
    if p.get("materialX").and_then(Value::as_bool).unwrap_or(false) {
        written.extend(write_materialx(s, &sets, &opts, std::path::Path::new(&dir), &mut warnings)?);
    }
    s.log.info("MapExporter", format!("Exported {} textures to {dir}", written.len()));
    Ok(json!({ "files": written.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(), "warnings": warnings }))
}

/// One `<mesh>_<set>.mtlx` per texture set, next to the textures `opts` just wrote: the
/// OpenPBR inputs those textures feed (see `texturecraft_formats::materialx`). UDIM sets name
/// their files with the `<UDIM>` token. Sets with no recognised map, or whose name gives the
/// same file name as an earlier set's, are skipped with a warning.
#[cfg(not(target_arch = "wasm32"))]
fn write_materialx(s: &Session, sets: &[usize], opts: &ExportOptions, dir: &std::path::Path, warnings: &mut Vec<String>) -> Result<Vec<std::path::PathBuf>> {
    use texturecraft_formats::materialx::{self, Texture};
    let mesh_name = stem_of(s.doc.project.mesh.trim_start_matches("builtin:"));
    let mut out = Vec::new();
    let mut stems = std::collections::HashSet::new();
    let mut written_names = std::collections::HashSet::new();
    let mut normal_map_written = false;
    let aces2065_tag = materialx::colorspace_name(texturecraft_texel::ColorSpace::Aces2065);
    let mut aces2065_written = false;
    for &ts in sets {
        let set = s.doc.project.texture_set(ts).map_err(|e| EngineError::Other(e.to_string()))?;
        let files = texturecraft_formats::list_outputs(&s.doc.project, ts, &mesh_name, opts).map_err(fe)?;

        // If any tile of a map was skipped, its UDIM reference could pick up another set's pixels
        let mut skipped_maps = std::collections::HashSet::new();
        for file in &files {
            if !written_names.insert(file.name.to_ascii_lowercase()) {
                skipped_maps.insert(file.map);
            }
        }

        let mut textures: Vec<Texture> = Vec::new();
        for file in files.into_iter().filter(|file| !skipped_maps.contains(&file.map)) {
            let Some(map) = opts.preset.maps.get(file.map) else { continue };
            let Some(input) = materialx::input_of_template(&map.file) else { continue };

            // The first tile stands for all the tiles with the same naming pattern
            let filename = file.udim_name.unwrap_or(file.name);
            let space = opts.space_for_map(map, file.format, file.bit_depth);
            let colorspace = materialx::colorspace_name(space);
            if !textures.iter().any(|texture| texture.input == input) {
                textures.push(Texture { input: input.to_owned(), file: filename, colorspace });
            }
        }
        normal_map_written |= textures.iter().any(|t| t.input == "geometry_normal");
        if textures.is_empty() {
            warnings.push(format!("MaterialX: texture set \"{}\" has no OpenPBR maps; use the \"OpenPBR (separate maps)\" preset", set.name));
            continue;
        }

        let stem = texturecraft_formats::export::sanitize_file_stem(&format!("{mesh_name}_{}", set.name));

        // Case-insensitive: Windows and macOS file systems are
        if !stems.insert(stem.to_ascii_lowercase()) {
            warnings.push(format!(
                "MaterialX: texture set \"{}\" exports as \"{stem}\" like an earlier set, so its .mtlx was not written; rename one of them",
                set.name
            ));
            continue;
        }

        let path = dir.join(format!("{stem}.mtlx"));
        texturecraft_formats::write_atomic(&path, materialx::write(&format!("{mesh_name}_{}", set.name), &textures, None).as_bytes()).map_err(fe)?;
        aces2065_written |=
            textures.iter().any(|texture| texture.colorspace == aces2065_tag && materialx::INPUTS.contains(&(texture.input.as_str(), materialx::Kind::Color)));
        out.push(path);
    }

    if aces2065_written {
        warnings.push("MaterialX: version 1.39 has no ACES2065-1 colour space; the colour maps are tagged lin_ap0_scene, its name in later versions, which older readers may not know".into());
    }

    // The normalmap node only exists in a material that has a normal map
    if normal_map_written && opts.normal_format == Some(NormalFormat::DirectX) {
        warnings.push("MaterialX: the normalmap node reads OpenGL (+Y) normals; this export wrote DirectX normals, so flip the green channel".into());
    }
    Ok(out)
}

#[cfg(not(target_arch = "wasm32"))]
fn export_mesh(s: &mut Session, p: &Value) -> Result<Value> {
    let path = native::path_param(p, "path", "file.exportMesh")?;
    let out = s.mesh_export(path, strp(p, "format"))?;
    let write = |path: &str, bytes: &[u8]| std::fs::write(path, bytes).map_err(|e| bad("file.exportMesh", format!("cannot write {path}: {e}")));
    let dir = std::path::Path::new(&out.path).parent().unwrap_or(std::path::Path::new(""));
    let mut textures = Vec::new();
    for (name, bytes) in &out.textures {
        let file = dir.join(name).display().to_string();
        write(&file, bytes)?;
        textures.push(file);
    }
    write(&out.path, &out.bytes)?;
    s.log.info("Exporter", format!("Exported mesh to {}", out.path));
    Ok(out.reply(textures))
}

/// The mesh formats `file.exportMesh` writes.
pub const MESH_EXPORT_FORMATS: [&str; 5] = ["obj", "glb", "usda", "usdc", "usdz"];

/// A mesh export: the mesh file's path (with its extension) and bytes, and (obj, usda, usdc) the
/// files it references (textures, an OBJ's MTL), by file name, to write next to it.
pub struct MeshExport {
    pub path: String,
    pub bytes: Vec<u8>,
    pub textures: Vec<(String, Vec<u8>)>,
}

impl MeshExport {
    /// The `file.exportMesh` reply, given where the textures were written.
    pub fn reply(&self, textures: Vec<String>) -> Value {
        if textures.is_empty() { json!({ "path": self.path }) } else { json!({ "path": self.path, "textures": textures }) }
    }
}

impl Session {
    /// The working mesh as a file for `path`, in `format` (obj, glb, usda, usdc, usdz; default:
    /// from the extension, else OBJ). The extension is appended when it doesn't match.
    pub fn mesh_export(&mut self, path: &str, format: Option<&str>) -> Result<MeshExport> {
        let ext = match format {
            Some(f) if MESH_EXPORT_FORMATS.contains(&f) => f.to_string(),
            Some(other) => return Err(bad("file.exportMesh", format!("unknown mesh format `{other}` ({})", MESH_EXPORT_FORMATS.join(", ")))),
            None if MESH_EXPORT_FORMATS.contains(&ext_of(path).as_str()) => ext_of(path),
            None => "obj".into(),
        };
        let path = if ext_of(path) == ext { path.to_string() } else { format!("{path}.{ext}") };
        let (bytes, textures) = match (ext.as_str(), texturecraft_formats::UsdFormat::from_ext(&ext)) {
            ("glb", _) => (self.mesh_glb_bytes()?, Vec::new()),
            (_, Some(format)) => {
                let usd = self.mesh_usd(format)?;
                (usd.bytes, usd.textures)
            }
            _ => self.mesh_obj_with_mtl(std::path::Path::new(&path).file_stem().and_then(|s| s.to_str()).unwrap_or("mesh"))?,
        };
        Ok(MeshExport { path, bytes, textures })
    }
}

#[cfg(target_arch = "wasm32")]
fn unsupported(_: &mut Session, _: &Value) -> Result<Value> {
    Err(EngineError::Other("file paths are not available in the browser; use the web file buttons".into()))
}
#[cfg(target_arch = "wasm32")]
use unsupported as new_from_mesh;
#[cfg(target_arch = "wasm32")]
use unsupported as open;
#[cfg(target_arch = "wasm32")]
use unsupported as save;
#[cfg(target_arch = "wasm32")]
use unsupported as save_as;
#[cfg(target_arch = "wasm32")]
use unsupported as save_copy;
#[cfg(target_arch = "wasm32")]
use unsupported as reimport;
#[cfg(target_arch = "wasm32")]
use unsupported as change_mesh;
#[cfg(target_arch = "wasm32")]
use unsupported as export_textures;
#[cfg(target_arch = "wasm32")]
use unsupported as export_mesh;

fn close(s: &mut Session, _: &Value) -> Result<Value> {
    s.switch_to(Session::closed()?);
    Ok(Value::Null)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    /// `file.new` keeps the cameras stored in the mesh file unless `importCameras` is false.
    #[test]
    fn new_from_mesh_imports_cameras_unless_told_not_to() {
        let dir = std::env::temp_dir().join(format!("tc_new_cameras_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("shot.gltf");
        let gltf = json!({
            "asset": {"version": "2.0"},
            "buffers": [{"byteLength": 36, "uri": "data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],
            "bufferViews": [{"buffer": 0, "byteLength": 36}],
            "accessors": [{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0]}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
            "cameras": [{"name": "Shot", "type": "perspective", "perspective": {"yfov": 0.8, "znear": 0.1}}],
            "nodes": [{"mesh": 0}, {"camera": 0, "translation": [0.5, 0.5, 4.0]}],
            "scenes": [{"nodes": [0, 1]}],
            "scene": 0
        });
        std::fs::write(&file, serde_json::to_vec(&gltf).unwrap()).unwrap();
        let path = file.display().to_string();
        let mut s = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();

        crate::execute(&mut s, "file.new", &json!({"mesh": path, "resolution": 128})).unwrap();
        assert_eq!(s.doc.project.cameras.len(), 1, "the file's camera comes along by default");

        crate::execute(&mut s, "file.new", &json!({"mesh": path, "resolution": 128, "importCameras": false})).unwrap();
        assert!(s.doc.project.cameras.is_empty(), "importCameras false leaves the cameras out");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Reimport mesh reads the mesh file again from where it was imported, so a change saved to
    /// the file since shows up (it used to re-read the project's own copy and change nothing).
    #[test]
    fn reimport_picks_up_changes_saved_to_the_mesh_file() {
        let dir = std::env::temp_dir().join(format!("tc_reimport_disk_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("tri.obj");
        let obj = |z: f32| {
            format!(
                "v -1 -1 {z}
v 1 -1 0
v 1 1 0
vt 0 0
vt 1 0
vt 1 1
f 1/1 2/2 3/3
"
            )
        };
        std::fs::write(&file, obj(0.0)).unwrap();
        let path = file.display().to_string();
        let mut s = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        crate::execute(&mut s, "file.new", &json!({"mesh": path, "resolution": 128})).unwrap();
        let low = |s: &Session| s.mesh.positions.iter().map(|p| p.z.abs()).fold(0.0_f32, f32::max);
        assert_eq!(low(&s), 0.0);
        // Edit the file, save the project, reopen it, then reimport with no path.
        std::fs::write(&file, obj(0.5)).unwrap();
        let mut reopened = Session::from_project_bytes(&s.to_project_bytes().unwrap()).unwrap();
        assert_eq!(low(&reopened), 0.0, "the project still holds the old copy");
        // A command without a path never reads the remembered host path (automation allows it).
        crate::execute(&mut reopened, "file.reimportMesh", &Value::Null).unwrap();
        assert_eq!(low(&reopened), 0.0, "no path: the project's own copy");
        let remembered = reopened.remembered_mesh_path().map(str::to_string).expect("the imported file is remembered");
        crate::execute(&mut reopened, "file.reimportMesh", &json!({ "path": remembered })).unwrap();
        assert!(low(&reopened) > 0.0, "the edited file must be read again");
        // The file gone: nothing is remembered, so the UI asks for one.
        std::fs::remove_file(&file).unwrap();
        assert!(reopened.remembered_mesh_path().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reimporting_a_saved_project_marks_it_modified() {
        let obj = b"v -1 -1 0\nv 1 -1 0\nv 1 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nf 1/1 2/2 3/3\n";
        let original = Session::from_mesh_bytes("mesh.obj", obj, 128, "mesh").unwrap();
        let mut saved = Session::from_project_bytes(&original.to_project_bytes().unwrap()).unwrap();
        assert!(!saved.modified);
        let project = saved.project_id;
        crate::execute(&mut saved, "file.reimportMesh", &Value::Null).unwrap();
        assert!(saved.modified, "reimport must trigger the unsaved-changes prompt");
        assert_eq!(saved.project_id, project, "a reimport stays the same project");
    }

    #[test]
    fn rejected_obj_export_preserves_existing_files_and_publishes_nothing() {
        let mut s = Session::new_project("t", primitives::cube(1.0), "builtin:cube", None, 128).unwrap();
        let dir = std::env::temp_dir().join(format!("tc_rejected_obj_export_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cube.obj");
        // The same command first writes a valid mesh and all its companions.
        crate::execute(&mut s, "file.exportMesh", &json!({ "path": path })).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("\nf "));
        let snapshot = || {
            std::fs::read_dir(&dir)
                .unwrap()
                .map(|entry| {
                    let path = entry.unwrap().path();
                    let bytes = std::fs::read(&path).unwrap();
                    (path, bytes)
                })
                .collect::<BTreeMap<_, _>>()
        };
        let before = snapshot();
        assert!(before.len() > 2, "the valid export includes its MTL and maps");
        // Keep the working/render mesh valid; corrupt only the source exported by both APIs.
        Arc::make_mut(&mut s.source).indices[0] = u32::MAX;
        assert!(s.mesh_obj_bytes().unwrap_err().to_string().contains("corner index is out of range"));
        assert!(s.mesh_obj_with_mtl("cube").unwrap_err().to_string().contains("corner index is out of range"));
        for destination in [path, dir.join("new.obj")] {
            let error = crate::execute(&mut s, "file.exportMesh", &json!({ "path": destination })).unwrap_err();
            assert!(error.to_string().contains("corner index is out of range"), "{error}");
            assert_eq!(snapshot(), before, "a rejected mesh must neither overwrite nor publish any file");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn export_mesh_writes_obj_by_default_and_textured_glb() {
        let mut s = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        let dir = std::env::temp_dir().join(format!("tc_export_mesh_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("ball").display().to_string();
        let obj = crate::execute(&mut s, "file.exportMesh", &json!({ "path": base })).unwrap();
        assert!(obj["path"].as_str().unwrap().ends_with("ball.obj"));
        // The OBJ names its MTL, which names every material and its maps, all written beside it.
        let text = std::fs::read_to_string(obj["path"].as_str().unwrap()).unwrap();
        assert!(text.lines().any(|l| l == "mtllib ball.mtl"), "named after the OBJ: {}", &text[..120]);
        let files: Vec<_> = obj["textures"].as_array().unwrap().iter().map(|f| std::path::PathBuf::from(f.as_str().unwrap())).collect();
        assert!(files.iter().all(|f| f.is_file()));
        let mtl = std::fs::read_to_string(&files[0]).unwrap();
        assert!(files[0].ends_with("ball.mtl"));
        for m in &s.mesh.materials {
            assert!(mtl.contains(&format!("newmtl {m}")) && text.contains(&format!("usemtl {m}")), "{m}");
        }
        for key in ["map_Kd", "map_Pr", "map_Pm", "norm"] {
            assert_eq!(mtl.lines().filter(|l| l.starts_with(&format!("{key} "))).count(), s.mesh.materials.len(), "{key}");
        }
        for f in &files[1..] {
            assert!(mtl.contains(f.file_name().unwrap().to_str().unwrap()), "{f:?} is used");
        }
        let back = texturecraft_formats::import_mesh_bytes("ball.obj", text.as_bytes()).unwrap();
        assert_eq!(back.mesh.triangle_count(), s.mesh.triangle_count());
        let glb = crate::execute(&mut s, "file.exportMesh", &json!({ "path": base, "format": "glb" })).unwrap();
        let bytes = std::fs::read(glb["path"].as_str().unwrap()).unwrap();
        let back = texturecraft_formats::import_mesh_bytes("ball.glb", &bytes).unwrap();
        assert_eq!(back.mesh.triangle_count(), s.mesh.triangle_count());
        assert_eq!(back.mesh.materials, s.mesh.materials);
        // Base color, ORM and normal per texture set.
        assert_eq!(bytes.windows(9).filter(|w| *w == b"image/png").count(), 3 * s.mesh.materials.len());
        assert!(crate::execute(&mut s, "file.exportMesh", &json!({ "path": base, "format": "fbx" })).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A mesh without materials exports its one implicit material, maps included.
    #[test]
    fn obj_export_declares_the_implicit_material() {
        let mut mesh = primitives::cube(1.0);
        mesh.materials.clear();
        mesh.submeshes.clear();
        let mut s = Session::new_project("t", mesh, "builtin:cube", None, 128).unwrap();
        let (obj, files) = s.mesh_obj_with_mtl("cube").unwrap();
        assert!(String::from_utf8(obj).unwrap().contains("usemtl default\n"));
        let mtl = String::from_utf8(files[0].1.clone()).unwrap();
        assert!(mtl.contains("newmtl default\nKd 1 1 1\nmap_Kd cube_default_BaseColor.png"), "{mtl}");
        assert!(files.iter().any(|f| f.0 == "cube_default_BaseColor.png"));
    }

    #[test]
    fn export_mesh_writes_usd_with_textures() {
        let mut s = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        let dir = std::env::temp_dir().join(format!("tc_export_usd_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let usdz = crate::execute(&mut s, "file.exportMesh", &json!({ "path": dir.join("ball.usdz").display().to_string() })).unwrap();
        assert!(usdz.get("textures").is_none(), "usdz packages its textures");
        let bytes = std::fs::read(usdz["path"].as_str().unwrap()).unwrap();
        let back = texturecraft_formats::import_mesh_bytes("ball.usdz", &bytes).unwrap();
        assert_eq!(back.mesh.triangle_count(), s.mesh.triangle_count());
        assert_eq!(back.mesh.materials, s.mesh.materials);
        // Stored (uncompressed) entries: count the PNG signatures.
        let pngs = bytes.windows(4).filter(|w| *w == b"\x89PNG").count();
        assert!(pngs >= s.mesh.materials.len(), "{pngs} textures");
        let usda = crate::execute(&mut s, "file.exportMesh", &json!({ "path": dir.join("ball").display().to_string(), "format": "usda" })).unwrap();
        let textures = usda["textures"].as_array().unwrap();
        assert_eq!(textures.len(), pngs);
        let text = std::fs::read_to_string(usda["path"].as_str().unwrap()).unwrap();
        for t in textures {
            let t = std::path::Path::new(t.as_str().unwrap());
            assert!(t.is_file() && text.contains(&format!("@./{}@", t.file_name().unwrap().to_str().unwrap())));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A project built on one mesh moves to another mesh file with its layers intact, even when
    /// it carries its own copy of the mesh (a plain reimport would only re-read that copy).
    #[test]
    fn change_mesh_swaps_the_mesh_and_keeps_the_layers() {
        let dir = std::env::temp_dir().join(format!("tc_change_mesh_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let quad = dir.join("quad.obj");
        std::fs::write(&quad, "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nf 1/1 2/2 3/3 4/4\n").unwrap();
        let tri = dir.join("tri.obj");
        std::fs::write(&tri, "v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nf 1/1 2/2 3/3\n").unwrap();
        let mut s = Session::from_mesh_bytes("quad.obj", &std::fs::read(&quad).unwrap(), 128, "quad").unwrap();
        crate::execute(&mut s, "layer.add.fill", &json!({"color": [1.0, 0.0, 0.0]})).unwrap();
        let layers = s.doc.project.layers.len();
        assert!(crate::execute(&mut s, "file.changeMesh", &json!({})).is_err(), "a path is required");
        crate::execute(&mut s, "file.changeMesh", &json!({"path": tri.display().to_string()})).unwrap();
        assert_eq!(s.mesh.triangle_count(), 1, "the new mesh is in use");
        assert_eq!(s.doc.project.mesh_file, "tri.obj");
        assert_eq!(s.doc.project.layers.len(), layers, "the layers stay");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Open A, change to B, edit B on disk: Reimport mesh (also after save and reopen) must read B,
    /// not A. A failed change leaves the remembered path alone.
    #[test]
    fn change_mesh_remembers_the_new_file_for_reimport() {
        let dir = std::env::temp_dir().join(format!("tc_change_mesh_path_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let obj = |x: f32| format!("v 0 0 0\nv {x} 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nf 1/1 2/2 3/3\n");
        let (a, b) = (dir.join("a.obj"), dir.join("b.obj"));
        std::fs::write(&a, obj(1.0)).unwrap();
        std::fs::write(&b, obj(2.0)).unwrap();
        let width = |s: &Session| s.mesh.positions.iter().map(|p| p.x).fold(0.0_f32, f32::max);
        let mut s = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        crate::execute(&mut s, "file.new", &json!({"mesh": a.display().to_string(), "resolution": 128})).unwrap();
        let a_abs = s.remembered_mesh_path().map(str::to_string).unwrap();
        // A failed change (unreadable file) keeps A.
        let missing = dir.join("missing.obj");
        assert!(crate::execute(&mut s, "file.changeMesh", &json!({"path": missing.display().to_string()})).is_err());
        assert_eq!(s.remembered_mesh_path(), Some(a_abs.as_str()));
        crate::execute(&mut s, "file.changeMesh", &json!({"path": b.display().to_string()})).unwrap();
        assert_eq!(width(&s), 2.0);
        let b_abs = s.remembered_mesh_path().map(str::to_string).expect("B is remembered");
        assert_ne!(a_abs, b_abs);
        // The remembered path survives save and reopen.
        let mut reopened = Session::from_project_bytes(&s.to_project_bytes().unwrap()).unwrap();
        assert_eq!(reopened.remembered_mesh_path(), Some(b_abs.as_str()));
        // Edit B, then Reimport from the remembered path: B's edits arrive, not A.
        std::fs::write(&b, obj(3.0)).unwrap();
        let remembered = reopened.remembered_mesh_path().map(str::to_string).unwrap();
        crate::execute(&mut reopened, "file.reimportMesh", &json!({"path": remembered})).unwrap();
        assert_eq!(width(&reopened), 3.0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn change_mesh_resets_base_colours_and_drops_a_cage_of_another_mesh() {
        let dir = std::env::temp_dir().join(format!("tc_change_mesh_cage_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.obj");
        std::fs::write(&a, "v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nf 1/1 2/2 3/3\n").unwrap();
        // Same triangle count, other vertices.
        let b = dir.join("b.obj");
        std::fs::write(&b, "v 0 0 0\nv 2 0 0\nv 0 2 0\nvt 0 0\nvt 1 0\nvt 0 1\nf 1/1 2/2 3/3\n").unwrap();
        let mut s = Session::from_mesh_bytes("a.obj", &std::fs::read(&a).unwrap(), 128, "a").unwrap();
        let cage = crate::bake::BakeMesh { name: "cage".into(), bytes: Default::default(), mesh: s.mesh.clone(), base_color: Vec::new() };
        s.bake_meshes.cage = Some(cage);
        let _ = s.bake_meshes.low_base_color.set(Err("old".into()));
        // The same mesh again keeps the cage but not the cached colours.
        crate::execute(&mut s, "file.changeMesh", &json!({"path": a.display().to_string()})).unwrap();
        assert!(s.bake_meshes.cage.is_some());
        assert!(s.bake_meshes.low_base_color.get().is_none(), "the colours are read again");
        crate::execute(&mut s, "file.changeMesh", &json!({"path": b.display().to_string()})).unwrap();
        assert!(s.bake_meshes.cage.is_none(), "the cage was made for another mesh");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unnamed_parts_keep_their_names_and_masks_through_save_and_reopen() {
        // A GLB quad whose mesh and node have no name: its one part is named after the file.
        let mut bin: Vec<u8> = [-1.0f32, -1.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0, 0.0, -1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0]
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .chain([0u16, 1, 2, 0, 2, 3].iter().flat_map(|i| i.to_le_bytes()))
            .collect();
        bin.resize(92, 0);
        let mut gltf = json!({
            "asset": {"version": "2.0"}, "buffers": [{"byteLength": 92}],
            "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": 48}, {"buffer": 0, "byteOffset": 48, "byteLength": 32}, {"buffer": 0, "byteOffset": 80, "byteLength": 12}],
            "accessors": [{"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3", "min": [-1, -1, 0], "max": [1, 1, 0]},
                {"bufferView": 1, "componentType": 5126, "count": 4, "type": "VEC2"}, {"bufferView": 2, "componentType": 5123, "count": 6, "type": "SCALAR"}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "TEXCOORD_0": 1}, "indices": 2}]}],
            "nodes": [{"mesh": 0}], "scenes": [{"nodes": [0]}], "scene": 0
        })
        .to_string()
        .into_bytes();
        gltf.resize(gltf.len().div_ceil(4) * 4, b' ');
        let mut glb = b"glTF".to_vec();
        glb.extend(2u32.to_le_bytes());
        glb.extend(((12 + 8 + gltf.len() + 8 + bin.len()) as u32).to_le_bytes());
        glb.extend((gltf.len() as u32).to_le_bytes());
        glb.extend(b"JSON");
        glb.extend(gltf);
        glb.extend((bin.len() as u32).to_le_bytes());
        glb.extend(b"BIN\0");
        glb.extend(bin);
        let mut s = Session::from_mesh_bytes("dir/quad.glb", &glb, 128, "quad").unwrap();
        let parts = |s: &mut Session| crate::execute(s, "textureSet.parts", &Value::Null).unwrap();
        assert_eq!(parts(&mut s), json!(["quad"]));
        crate::execute(&mut s, "layer.add.fill", &json!({"color": [1.0, 0.0, 0.0]})).unwrap();
        crate::execute(&mut s, "layer.set", &json!({"geometryMask": ["quad"]})).unwrap();
        let mut back = Session::from_project_bytes(&s.to_project_bytes().unwrap()).unwrap();
        assert_eq!(parts(&mut back), json!(["quad"]), "the reopened mesh names its part as the import did");
        crate::execute(&mut back, "file.reimportMesh", &Value::Null).unwrap();
        assert_eq!(parts(&mut back), json!(["quad"]), "and so does a reimport of the stored mesh");
        // Projects saved before the file name was kept read the mesh as before.
        let mut old = back.project_file();
        old.project.mesh_file.clear();
        let mut old = Session::from_project_bytes(&texturecraft_formats::save_project_bytes(&old).unwrap()).unwrap();
        assert_eq!(parts(&mut old), json!(["mesh"]));
    }
}

#[cfg(all(test, target_pointer_width = "32"))]
mod index_tests {
    use super::*;

    #[test]
    fn export_rejects_an_index_that_would_wrap_to_the_first_set() {
        let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        let wrapped_index = u64::from(u32::MAX) + 1;

        assert!(crate::execute(&mut session, "export.plan", &json!({ "ts": wrapped_index })).is_err());
        assert!(crate::execute(&mut session, "export.plan", &json!({ "ts": [wrapped_index] })).is_err());
    }
}

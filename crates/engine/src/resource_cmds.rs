//! Image resources: import (PNG, JPEG, TGA, TIFF, BMP, WEBP, EXR, HDR), list, remove, and use as
//! fill sources.

use std::sync::Arc;

use serde_json::{Value, json};
use texturecraft_compose::ImageRes;
use texturecraft_core::{Channel, Fill, Layer, LayerKind, Source};

use crate::commands::{CommandSpec, always, bad, channel_param, strp};
use crate::{EngineError, Result, Session, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("resource.import", "Import resources", ["File"], None, "{path} — an image file (png, jpg, tga, tif, bmp, webp, exr, hdr)", always, import),
        cmd!("resource.list", "List resources", [], None, "{}", always, |s, _| Ok(list(s))),
        cmd!("resource.remove", "Remove resource", [], None, "{name}", always, remove),
        cmd!("file.removeUnused", "Remove unused resources...", ["File"], None, "{}", always, remove_unused),
        cmd!(
            "layer.add.image",
            "Add fill layer from image",
            [],
            None,
            "{name (exact resource name with extension or unique stem), channel?: baseColor, projection?: uv|triplanar|planar, scale?}",
            always,
            add_image_fill
        ),
    ]
}

/// Largest accepted resource file.
pub(crate) const MAX_BYTES: usize = 512 << 20;

pub(crate) fn decoded(d: texturecraft_formats::DecodedImage) -> Arc<ImageRes> {
    Arc::new(ImageRes { width: d.width, height: d.height, rgba: d.rgba, srgb: d.srgb })
}

impl Session {
    /// Add an image resource from bytes (one undo step); returns its unique, sanitized name
    pub fn import_resource_bytes(&mut self, file_name: &str, bytes: Vec<u8>) -> Result<String> {
        self.edit("Import resource", |s| s.add_image_resource(file_name, bytes, u64::MAX))
    }

    /// Add an image within a decoded texel budget, inside the caller's edit
    pub(crate) fn add_image_resource(&mut self, file_name: &str, bytes: Vec<u8>, max_texels: u64) -> Result<String> {
        if bytes.len() > MAX_BYTES {
            return Err(EngineError::Other(format!("{file_name} is larger than {} MB", MAX_BYTES >> 20)));
        }

        let image = texturecraft_formats::decode_image_within(&bytes, max_texels).map_err(|e| EngineError::Other(format!("{file_name}: {e}")))?;
        let leaf = std::path::Path::new(file_name).file_name().and_then(|n| n.to_str()).unwrap_or("image");
        let base = texturecraft_formats::sanitize_resource_name(leaf);
        let mut name = base.clone();
        let mut n = 2;
        while self.doc.resources.contains_key(&name) {
            name = format!("{n}_{base}");
            n += 1;
        }

        self.doc.resources.insert(name.clone(), Arc::new(bytes));
        self.images.insert(name.clone(), decoded(image));
        Ok(name)
    }

    /// Decode resources that are not decoded yet and forget removed ones (after undo, redo, load).
    pub fn sync_images(&mut self) {
        let keep: Vec<String> = self.doc.resources.keys().cloned().collect();
        self.images.retain(|k, _| keep.contains(k));
        // Linked materials' packages are resources too, not images.
        let missing: Vec<(String, Arc<Vec<u8>>)> = self
            .doc
            .resources
            .iter()
            .filter(|(k, _)| !self.images.contains_key(*k) && !k.ends_with(&format!(".{}", texturecraft_formats::mcmat::MCRAFT_EXT)))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (k, bytes) in missing {
            match texturecraft_formats::decode_image(&bytes) {
                Ok(d) => {
                    self.images.insert(k, decoded(d));
                }
                Err(e) => self.log.push(format!("resource {k}: {e}")),
            }
        }
    }
}

fn list(s: &Session) -> Value {
    json!(
        s.doc
            .resources
            .iter()
            .map(|(k, v)| {
                let (w, h) = s.images.get(k).map(|i| (i.width, i.height)).unwrap_or((0, 0));
                json!({"name": k, "bytes": v.len(), "width": w, "height": h})
            })
            .collect::<Vec<_>>()
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let path = strp(p, "path").ok_or_else(|| bad("resource.import", "`path` is required"))?;
    let bytes = read_image_file("resource.import", path, MAX_BYTES as u64)?;
    let name = s.import_resource_bytes(path, bytes)?;
    Ok(json!({ "name": name }))
}

/// Read a regular image file within both the per-file cap and the caller's remaining byte budget
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn read_image_file(cmd: &str, path: &str, max_bytes: u64) -> Result<Vec<u8>> {
    use std::io::Read;

    let max_bytes = max_bytes.min(MAX_BYTES as u64);
    let read_error = |error| bad(cmd, format!("cannot read {path}: {error}"));
    let too_large = || bad(cmd, format!("{path} is too large (limit {max_bytes} bytes)"));
    let metadata = std::fs::metadata(path).map_err(read_error)?;
    // Pipes can block on open, and devices may never reach EOF
    if !metadata.is_file() {
        return Err(bad(cmd, format!("{path} is not a regular file")));
    }
    if metadata.len() > max_bytes {
        return Err(too_large());
    }

    // Bound the read even if the file grows after the metadata check
    let file = std::fs::File::open(path).map_err(read_error)?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(max_bytes + 1).read_to_end(&mut bytes).map_err(read_error)?;
    if bytes.len() as u64 > max_bytes {
        return Err(too_large());
    }

    Ok(bytes)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn read_image_file(cmd: &str, _: &str, _: u64) -> Result<Vec<u8>> {
    Err(bad(cmd, "file paths are not available in the browser"))
}

#[cfg(target_arch = "wasm32")]
fn import(_: &mut Session, _: &Value) -> Result<Value> {
    Err(EngineError::Other("file paths are not available in the browser; use File > Import resources".into()))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let name = strp(p, "name").ok_or_else(|| bad("resource.remove", "`name` is required"))?.to_string();
    if !s.doc.resources.contains_key(&name) {
        return Err(bad("resource.remove", format!("no resource `{name}`")));
    }
    s.edit("Remove resource", |s| {
        s.doc.resources.remove(&name);
        Ok(())
    })?;
    s.sync_images();
    Ok(Value::Null)
}

/// Drop resources no layer, mask effect, the projection stencil or the environment uses (one undo
/// step).
fn remove_unused(s: &mut Session, _: &Value) -> Result<Value> {
    let mut used = Vec::new();
    for set in &s.doc.project.texture_sets {
        for root in &set.stack {
            crate::material_lib::images_used(&s.doc.project, *root, &mut used, 0);
        }
    }
    used.extend(s.tool.stencil.as_ref().map(|st| st.image.clone()));
    used.extend(s.doc.project.display.environment_resource().map(str::to_string));
    let unused: Vec<String> = s.doc.resources.keys().filter(|k| !used.contains(k)).cloned().collect();
    if !unused.is_empty() {
        s.edit("Remove unused resources", |s| {
            s.doc.resources.retain(|k, _| !unused.contains(k));
            Ok(())
        })?;
        s.sync_images();
    }
    Ok(json!({ "removed": unused }))
}

#[cfg(not(target_arch = "wasm32"))]
const IMPORT_HINT: &str = "resource.import";
#[cfg(target_arch = "wasm32")]
const IMPORT_HINT: &str = "File > Import resources";

/// Resolve an image resource by exact name or unique file stem.
fn resolve_resource_name(s: &Session, name: &str) -> Result<String> {
    if s.images.contains_key(name) {
        return Ok(name.to_string());
    }
    let matches: Vec<&String> = s.images.keys().filter(|k| std::path::Path::new(k).file_stem().and_then(|stem| stem.to_str()) == Some(name)).collect();
    match matches.as_slice() {
        [single] => Ok((*single).clone()),
        [] => {
            let available: Vec<&str> = s.images.keys().map(String::as_str).collect();
            let hint =
                if available.is_empty() { format!(" (no resources imported: use {IMPORT_HINT})") } else { format!(" (available: {})", available.join(", ")) };
            Err(bad("layer.add.image", format!("no resource `{name}`{hint}")))
        }
        multiple => {
            let matches_str = multiple.iter().map(|m| format!("`{m}`")).collect::<Vec<_>>().join(", ");
            Err(bad("layer.add.image", format!("ambiguous resource `{name}` matches {matches_str}: give the full name with extension")))
        }
    }
}

fn add_image_fill(s: &mut Session, p: &Value) -> Result<Value> {
    let raw_name = strp(p, "name").ok_or_else(|| bad("layer.add.image", "`name` is required"))?;
    let name = resolve_resource_name(s, raw_name)?;
    let ch = channel_param("layer.add.image", p, "channel")?.unwrap_or(Channel::BaseColor);
    let projection = crate::layer_cmds::projection_from(p, "layer.add.image", None)?;
    let ts = s.ts();
    let label = std::path::Path::new(&name).file_stem().and_then(|n| n.to_str()).unwrap_or("Image").to_string();
    let layer_name = s.doc.project.next_layer_name(ts, &label);
    let id = s.edit("Add image fill", |s| {
        let id = s.doc.project.alloc_id();
        let mut fill = Fill { projection, ..Fill::default() };
        fill.sources.insert(ch, Source::image(name.clone()));
        crate::layer_cmds::insert_selected(s, Layer::new(id, layer_name, LayerKind::Fill(fill), &[ch]))
    })?;
    Ok(json!({ "layer": id.0 }))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn file_reads_obey_the_remaining_byte_budget() {
        let path = std::env::temp_dir().join(format!("texturecraft-read-image-{}.png", std::process::id()));
        std::fs::write(&path, b"1234").unwrap();
        let name = path.to_str().unwrap();
        assert!(read_image_file("test", name, 3).unwrap_err().to_string().contains("too large"));
        assert_eq!(read_image_file("test", name, 4).unwrap(), b"1234");
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn devices_are_rejected_before_reading() {
        assert!(read_image_file("test", "/dev/null", 16).unwrap_err().to_string().contains("not a regular file"));
    }
}

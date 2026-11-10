//! Mesh export of the working mesh: Wavefront OBJ (positions, UVs, normals, one group per
//! submesh; with an MTL material library and its maps, or alone), binary glTF (one primitive per submesh, a metallic-roughness material per
//! material with its textures embedded) and USD (usda / usdc / usdz, see [`usd`]).

pub(crate) mod usd;
mod usdc;

use std::collections::BTreeSet;
use std::fmt::Write as _;

use serde_json::{Map, Value, json};
use texturecraft_core::{Channel, NormalFormat};
use texturecraft_geom::glam::Vec3;
use texturecraft_geom::{Mesh, SubMesh};

use crate::export::sanitize_file_stem;
use crate::presets::{GLTF_BASE_COLOR, GLTF_CLEARCOAT, GLTF_EMISSIVE, GLTF_NORMAL, GLTF_ORM, GLTF_SHEEN, gltf_pbr_maps};
use crate::{ExportOptions, ExportPreset, ExportedFile, FileFormat, FormatError, Result};

pub use usd::{NamedFiles, UsdExport, UsdFormat, usd_bytes, usd_export_options};

/// The mesh as OBJ text (UV v flipped back to OBJ's bottom-up convention).
pub fn obj_bytes(mesh: &Mesh, name: &str) -> Vec<u8> {
    obj_text(mesh, name, None).into_bytes()
}

/// The mesh as OBJ with a material library: the OBJ (naming `<stem>.mtl`, `stem` being the
/// OBJ file's own, so exports beside each other keep their files apart) and the files to write
/// next to it, the MTL first, then each material's maps (`textures[m]`, rendered with
/// [`usd_export_options`], named like USD's). Roughness, metallic, normal and coat maps use the
/// MTL PBR extension's keys.
pub fn obj_with_mtl(mesh: &Mesh, name: &str, stem: &str, textures: &[Vec<ExportedFile>]) -> (Vec<u8>, NamedFiles) {
    let clean = sanitize_file_stem(stem);
    // OBJ names that sanitize alike (`ball 1`, `ball_1`) must not overwrite each other's files:
    // a changed name gets a hash of the original (FNV-1a).
    let stem = if clean == stem {
        clean
    } else {
        let hash = stem.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193));
        format!("{clean}_{hash:08x}")
    };
    let (maps, mut files) = usd::material_maps(mesh, &stem, textures);
    let mut mtl = format!("# {}, exported by TextureCraft\n", token(name));
    for (material, maps) in unique_ids(&mesh.materials, token).iter().zip(&maps) {
        let _ = writeln!(mtl, "\nnewmtl {material}\nKd 1 1 1");
        for (slot, file) in maps {
            let keys: &[&str] = match usd::SLOTS.get(*slot).map_or("", |s| s.0) {
                "BaseColor" => &["map_Kd"],
                "Roughness" => &["map_Pr"],
                "Metallic" => &["map_Pm"],
                // `norm` is the PBR extension's key; many tools read a normal map from `map_Bump`.
                "Normal" => &["norm", "map_Bump"],
                // The emissive map scales `Ke`.
                "Emissive" => &["Ke 1 1 1\nmap_Ke"],
                "Opacity" => &["map_d"],
                "Clearcoat" => &["map_Pc"],
                "ClearcoatRoughness" => &["map_Pcr"],
                _ => &[],
            };
            for key in keys {
                let _ = writeln!(mtl, "{key} {file}");
            }
        }
    }
    files.insert(0, (format!("{stem}.mtl"), mtl.into_bytes()));
    (obj_text(mesh, name, Some(&format!("{stem}.mtl"))).into_bytes(), files)
}

fn obj_text(mesh: &Mesh, name: &str, mtllib: Option<&str>) -> String {
    let mut text = String::with_capacity(mesh.positions.len() * 96 + mesh.indices.len() * 12);
    let _ = writeln!(text, "# {}, exported by TextureCraft", token(name));
    if let Some(lib) = mtllib {
        let _ = writeln!(text, "mtllib {lib}");
    }
    let _ = writeln!(text, "o {}", token(name));
    for p in &mesh.positions {
        let _ = writeln!(text, "v {} {} {}", p.x, p.y, p.z);
    }

    let uvs = mesh.uvs.len() == mesh.positions.len();
    let normals = mesh.normals.len() == mesh.positions.len();
    if uvs {
        for t in &mesh.uvs {
            let _ = writeln!(text, "vt {} {}", t.x, 1.0 - t.y);
        }
    }
    if normals {
        for n in &mesh.normals {
            let _ = writeln!(text, "vn {} {} {}", n.x, n.y, n.z);
        }
    }

    let materials = unique_ids(&mesh.materials, token);
    for part in &parts(mesh, name) {
        let material = materials.get(part.material).map_or("default", String::as_str);
        let _ = writeln!(text, "g {}\nusemtl {material}", token(&part.name));
        let (a, b) = (part.start as usize, part.start as usize + part.count as usize);
        for triangle in mesh.indices.get(a..b).unwrap_or_default().as_chunks::<3>().0 {
            text.push('f');
            for i in triangle {
                let i = i + 1;
                let _ = match (uvs, normals) {
                    (true, true) => write!(text, " {i}/{i}/{i}"),
                    (true, false) => write!(text, " {i}/{i}"),
                    (false, true) => write!(text, " {i}//{i}"),
                    (false, false) => write!(text, " {i}"),
                };
            }
            text.push('\n');
        }
    }

    text
}

/// `id` of each material name, made unique with `_2`, `_3`… where two would be the same, also
/// ignoring case (the ids name files, and file systems may ignore case).
pub(crate) fn unique_ids(materials: &[String], id: impl Fn(&str) -> String) -> Vec<String> {
    let mut seen = BTreeSet::new();
    materials
        .iter()
        .map(|m| {
            let base = id(m);
            let mut unique = base.clone();
            for n in 2.. {
                if seen.insert(unique.to_lowercase()) {
                    break;
                }
                unique = format!("{base}_{n}");
            }
            unique
        })
        .collect()
}

/// The submeshes, or one covering the whole mesh when it has none.
fn parts(mesh: &Mesh, name: &str) -> Vec<SubMesh> {
    if mesh.submeshes.is_empty() {
        vec![SubMesh { name: name.into(), material: 0, start: 0, count: mesh.indices.len() as u32 }]
    } else {
        mesh.submeshes.clone()
    }
}

/// Textures of one glTF material (PNG bytes), from the files of [`glb_export_options`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlbTextures {
    pub base_color: Option<Vec<u8>>,
    /// Occlusion (R), roughness (G), metallic (B).
    pub orm: Option<Vec<u8>>,
    /// OpenGL convention (+Y up the image), as glTF expects.
    pub normal: Option<Vec<u8>>,
    pub emissive: Option<Vec<u8>>,
    /// KHR_materials_clearcoat: weight (R), roughness (G).
    pub clearcoat: Option<Vec<u8>>,
    /// KHR_materials_sheen: color × weight (RGB), roughness (A, when the set has it).
    pub sheen: Option<Vec<u8>>,
}

impl GlbTextures {
    /// Sort rendered files into slots; an all-black emissive map is dropped.
    pub fn from_files(files: Vec<ExportedFile>) -> GlbTextures {
        let mut t = GlbTextures::default();
        for f in files {
            let slot = match f.name.strip_suffix(".png").unwrap_or_default() {
                GLTF_BASE_COLOR => &mut t.base_color,
                GLTF_ORM => &mut t.orm,
                GLTF_NORMAL => &mut t.normal,
                GLTF_EMISSIVE if !is_black(&f.bytes) => &mut t.emissive,
                GLTF_CLEARCOAT => &mut t.clearcoat,
                GLTF_SHEEN => &mut t.sheen,
                _ => continue,
            };
            *slot = Some(f.bytes);
        }
        t
    }
}

/// Texture export options for [`GlbTextures`]: the glTF maps as 8-bit PNG named by slot.
pub fn glb_export_options() -> ExportOptions {
    png8_options(ExportPreset { name: "glTF binary".into(), maps: gltf_pbr_maps(str::to_string) }, NormalFormat::OpenGl)
}

/// `preset` as 8-bit PNGs with normals in `normal` convention.
fn png8_options(preset: ExportPreset, normal: NormalFormat) -> ExportOptions {
    let mut o = ExportOptions::new(preset);
    o.format_override = Some(FileFormat::Png);
    o.bit_depth_override = Some(8);
    o.normal_format = Some(normal);
    o
}

fn is_black(png: &[u8]) -> bool {
    image::load_from_memory_with_format(png, image::ImageFormat::Png).map_or(true, |img| img.to_rgb8().iter().all(|v| *v == 0))
}

/// PNG color type 4 (gray + alpha) or 6 (RGBA), from the IHDR chunk.
fn png_has_alpha(png: &[u8]) -> bool {
    matches!(png.get(25), Some(4 | 6))
}

/// The binary chunk and the JSON arrays that index into it.
#[derive(Default)]
struct GlbBuilder {
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
    images: Vec<Value>,
    textures: Vec<Value>,
    /// Material extensions used (`extensionsUsed`).
    extensions: BTreeSet<String>,
}

const KHR_CLEARCOAT: &str = "KHR_materials_clearcoat";
const KHR_SHEEN: &str = "KHR_materials_sheen";

impl GlbBuilder {
    fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        let mut v = json!({ "buffer": 0, "byteOffset": self.bin.len(), "byteLength": bytes.len() });
        if let Some(t) = target {
            v["target"] = t.into();
        }
        self.bin.extend_from_slice(bytes);
        self.views.push(v);
        self.views.len() - 1
    }

    fn accessor(&mut self, values: impl Iterator<Item = [u8; 4]>, target: u32, component: u32, kind: &str, count: usize) -> Value {
        let bytes: Vec<u8> = values.flatten().collect();
        let view = self.view(&bytes, Some(target));
        self.accessors.push(json!({ "bufferView": view, "componentType": component, "count": count, "type": kind }));
        Value::from(self.accessors.len() - 1)
    }

    /// A float vertex attribute of `kind` (VEC2 / VEC3) over `n` vertices.
    fn attribute(&mut self, values: impl Iterator<Item = f32>, kind: &str, n: usize) -> Value {
        self.accessor(values.map(f32::to_le_bytes), 34962, 5126, kind, n)
    }

    /// `{"index": n}` of a new texture holding `png`.
    fn texture(&mut self, png: &[u8]) -> Value {
        let view = self.view(png, None);
        self.images.push(json!({ "bufferView": view, "mimeType": "image/png" }));
        self.textures.push(json!({ "source": self.images.len() - 1, "sampler": 0 }));
        json!({ "index": self.textures.len() - 1 })
    }

    fn material(&mut self, name: &str, t: &GlbTextures) -> Value {
        let (mut m, mut pbr) = (Map::new(), Map::new());
        m.insert("name".into(), name.into());
        if let Some(png) = &t.base_color {
            pbr.insert("baseColorTexture".into(), self.texture(png));
            if png_has_alpha(png) {
                m.insert("alphaMode".into(), "BLEND".into());
            }
        }
        match &t.orm {
            Some(png) => {
                let tex = self.texture(png);
                pbr.insert("metallicRoughnessTexture".into(), tex.clone());
                m.insert("occlusionTexture".into(), tex);
            }
            None => {
                pbr.insert("metallicFactor".into(), 0.0.into());
            }
        }
        if let Some(png) = &t.normal {
            m.insert("normalTexture".into(), self.texture(png));
        }
        if let Some(png) = &t.emissive {
            m.insert("emissiveTexture".into(), self.texture(png));
            m.insert("emissiveFactor".into(), json!([1.0, 1.0, 1.0]));
        }
        m.insert("pbrMetallicRoughness".into(), pbr.into());
        let mut ext = Map::new();
        if let Some(png) = &t.clearcoat {
            let tex = self.texture(png);
            let coat = json!({ "clearcoatFactor": 1.0, "clearcoatTexture": tex, "clearcoatRoughnessFactor": 1.0, "clearcoatRoughnessTexture": tex });
            ext.insert(KHR_CLEARCOAT.into(), coat);
        }
        if let Some(png) = &t.sheen {
            let tex = self.texture(png);
            let mut sheen = json!({ "sheenColorFactor": [1.0, 1.0, 1.0], "sheenColorTexture": tex });
            if png_has_alpha(png) {
                sheen["sheenRoughnessFactor"] = 1.0.into();
                sheen["sheenRoughnessTexture"] = tex;
            } else {
                sheen["sheenRoughnessFactor"] = Channel::SheenRoughness.default_value()[0].into();
            }
            ext.insert(KHR_SHEEN.into(), sheen);
        }
        if !ext.is_empty() {
            self.extensions.extend(ext.keys().cloned());
            m.insert("extensions".into(), ext.into());
        }
        m.into()
    }
}

/// The mesh as binary glTF (GLB) named `name`, with `textures[i]` on material `i` (materials
/// without an entry get a plain one). Vertex data is written as is: the mesh already follows
/// glTF's conventions.
pub fn glb_bytes(mesh: &Mesh, name: &str, textures: &[GlbTextures]) -> Result<Vec<u8>> {
    let n = mesh.positions.len();
    if n == 0 || mesh.indices.len() < 3 {
        return Err(FormatError::Invalid("the mesh is empty".into()));
    }
    let mut g = GlbBuilder::default();
    let vec3s = |v: &[Vec3]| v.iter().flat_map(|p| p.to_array()).collect::<Vec<f32>>();
    let mut attributes = Map::new();
    let position = g.attribute(vec3s(&mesh.positions).into_iter(), "VEC3", n);
    let (lo, hi) = mesh.bounds();
    if let Some(a) = g.accessors.last_mut() {
        a["min"] = json!(lo.to_array());
        a["max"] = json!(hi.to_array());
    }
    attributes.insert("POSITION".into(), position);
    if mesh.normals.len() == n {
        attributes.insert("NORMAL".into(), g.attribute(vec3s(&mesh.normals).into_iter(), "VEC3", n));
    }
    if mesh.uvs.len() == n {
        attributes.insert("TEXCOORD_0".into(), g.attribute(mesh.uvs.iter().flat_map(|t| t.to_array()), "VEC2", n));
    }
    let mut primitives = Vec::new();
    for sm in parts(mesh, name) {
        let tris = mesh.indices.get(sm.start as usize..(sm.start as usize).saturating_add(sm.count as usize)).unwrap_or_default();
        let tris = tris.get(..tris.len() / 3 * 3).unwrap_or_default();
        if tris.is_empty() {
            continue;
        }
        let indices = g.accessor(tris.iter().map(|i| i.to_le_bytes()), 34963, 5125, "SCALAR", tris.len());
        let mut p = json!({ "attributes": attributes, "indices": indices });
        if sm.material < mesh.materials.len() {
            p["material"] = sm.material.into();
        }
        primitives.push(p);
    }
    let none = GlbTextures::default();
    let materials: Vec<Value> = mesh.materials.iter().enumerate().map(|(i, m)| g.material(m, textures.get(i).unwrap_or(&none))).collect();
    let mut doc = json!({
        "asset": { "version": "2.0", "generator": "TextureCraft" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "mesh": 0, "name": name }],
        "meshes": [{ "name": name, "primitives": primitives }],
        "buffers": [{ "byteLength": g.bin.len() }],
        "bufferViews": g.views,
        "accessors": g.accessors,
    });
    if !g.textures.is_empty() {
        doc["samplers"] = json!([{ "magFilter": 9729, "minFilter": 9987, "wrapS": 10497, "wrapT": 10497 }]);
    }
    if !g.extensions.is_empty() {
        doc["extensionsUsed"] = json!(g.extensions);
    }
    // glTF forbids empty arrays.
    for (key, list) in [("materials", materials), ("images", g.images), ("textures", g.textures)] {
        if !list.is_empty() {
            doc[key] = list.into();
        }
    }
    let mut js = serde_json::to_vec(&doc).map_err(|e| FormatError::Invalid(format!("glTF JSON: {e}")))?;
    while !js.len().is_multiple_of(4) {
        js.push(b' ');
    }
    let mut bin = g.bin;
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let total = 12 + 8 + js.len() + 8 + bin.len();
    let len32 = |v: usize| u32::try_from(v).map(u32::to_le_bytes).map_err(|_| FormatError::TooLarge(format!("the GLB would be {total} bytes (limit 4 GiB)")));
    let mut out = Vec::with_capacity(total);
    for chunk in [&b"glTF"[..], &2u32.to_le_bytes(), &len32(total)?, &len32(js.len())?, b"JSON", &js, &len32(bin.len())?, b"BIN\0", &bin] {
        out.extend_from_slice(chunk);
    }
    Ok(out)
}

/// OBJ names end at whitespace.
fn token(name: &str) -> String {
    let t: String = name.trim().chars().map(|c| if c.is_whitespace() { '_' } else { c }).collect();
    if t.is_empty() { "unnamed".into() } else { t }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_with_newline_stays_in_the_header_comment() {
        let mesh = texturecraft_geom::primitives::cube(1.0);
        let name = "a\nv 9 9 9\r\nmtllib x";
        let obj = String::from_utf8(obj_bytes(&mesh, name)).unwrap();
        assert!(obj.lines().all(|l| !l.starts_with("v 9 9 9") && !l.starts_with("mtllib x")));
        let (obj, files) = obj_with_mtl(&mesh, name, "m", &[]);
        let obj = String::from_utf8(obj).unwrap();
        assert!(obj.lines().all(|l| !l.starts_with("v 9 9 9") && !l.starts_with("mtllib x")));
        let mtl = String::from_utf8(files[0].1.clone()).unwrap();
        assert!(mtl.lines().all(|l| !l.starts_with("v 9 9 9") && !l.starts_with("mtllib x")));
    }

    #[test]
    fn materials_whose_names_collide_even_by_case_stay_apart() {
        let mut mesh = texturecraft_geom::primitives::cube(1.0);
        mesh.materials = vec!["Mat A".into(), "mat_a".into()];
        mesh.submeshes = vec![
            SubMesh { name: "a".into(), material: 0, start: 0, count: 18 },
            SubMesh { name: "b".into(), material: 1, start: 18, count: mesh.indices.len() as u32 - 18 },
        ];
        let png = |name: &str| ExportedFile { name: name.into(), bytes: vec![1, 2, 3], width: 1, height: 1, warnings: Vec::new() };
        let (obj, files) = obj_with_mtl(&mesh, "project", "m", &[vec![png("BaseColor.png")], vec![png("BaseColor.png")]]);
        let obj = String::from_utf8(obj).unwrap();
        assert!(obj.contains("mtllib m.mtl\n") && obj.contains("usemtl Mat_A\n") && obj.contains("usemtl mat_a_2\n"));
        let names: Vec<_> = files.iter().map(|f| f.0.as_str()).collect();
        assert_eq!(names, ["m.mtl", "m_Mat_A_BaseColor.png", "m_mat_a_2_BaseColor.png"]);
        let mtl = String::from_utf8(files[0].1.clone()).unwrap();
        assert!(
            mtl.contains("newmtl Mat_A\nKd 1 1 1\nmap_Kd m_Mat_A_BaseColor.png") && mtl.contains("newmtl mat_a_2\nKd 1 1 1\nmap_Kd m_mat_a_2_BaseColor.png"),
            "{mtl}"
        );
    }

    #[test]
    fn obj_names_that_sanitize_alike_keep_their_companions_apart() {
        let mesh = texturecraft_geom::primitives::cube(1.0);
        let png = || vec![ExportedFile { name: "BaseColor.png".into(), bytes: vec![1], width: 1, height: 1, warnings: Vec::new() }];
        let names = |stem: &str| obj_with_mtl(&mesh, "p", stem, &[png()]).1.into_iter().map(|f| f.0).collect::<Vec<_>>();
        let (spaced, plain) = (names("ball 1"), names("ball_1"));
        assert_eq!(plain, ["ball_1.mtl", "ball_1_Cube_BaseColor.png"], "a clean name is kept");
        assert!(spaced.iter().all(|n| n.starts_with("ball_1_") && !plain.contains(n)), "{spaced:?}");
        assert_eq!(spaced, names("ball 1"), "the same name always maps alike");
    }

    #[test]
    fn obj_round_trips_through_the_importer() {
        let mesh = texturecraft_geom::primitives::rounded_box(texturecraft_geom::glam::Vec3::splat(1.0), 0.1);
        let bytes = obj_bytes(&mesh, "My box");
        let back = crate::import_mesh_bytes("box.obj", &bytes).unwrap().mesh;
        assert_eq!(back.triangle_count(), mesh.triangle_count());
        let (a, b) = (mesh.bounds(), back.bounds());
        assert!((a.0 - b.0).length() < 1e-4 && (a.1 - b.1).length() < 1e-4);
        let uv_sum = |m: &Mesh| m.indices.iter().map(|&i| m.uvs[i as usize].y as f64).sum::<f64>();
        assert!((uv_sum(&mesh) - uv_sum(&back)).abs() < 1e-3 * mesh.indices.len() as f64);
        assert!(String::from_utf8(bytes).unwrap().contains("o My_box"));
    }

    #[test]
    fn glb_writes_clearcoat_and_sheen_extensions() {
        let png = |color: image::ColorType| {
            let mut out = std::io::Cursor::new(Vec::new());
            image::write_buffer_with_format(&mut out, &vec![200u8; 4 * color.channel_count() as usize], 2, 2, color, image::ImageFormat::Png).unwrap();
            out.into_inner()
        };
        let file = |name: &str, bytes: Vec<u8>| ExportedFile { name: name.into(), bytes, width: 2, height: 2, warnings: vec![] };
        let mesh = texturecraft_geom::primitives::sphere(1.0, 8, 4);
        let json_of = |t: GlbTextures| {
            let glb = glb_bytes(&mesh, "Ball", &[t]).unwrap();
            let len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
            serde_json::from_slice::<Value>(&glb[20..20 + len]).unwrap()
        };
        let t = GlbTextures::from_files(vec![file("Clearcoat.png", png(image::ColorType::Rgb8)), file("Sheen.png", png(image::ColorType::Rgb8))]);
        let doc = json_of(t);
        assert_eq!(doc["extensionsUsed"], json!([KHR_CLEARCOAT, KHR_SHEEN]));
        let ext = &doc["materials"][0]["extensions"];
        let coat = &ext[KHR_CLEARCOAT];
        assert_eq!(coat["clearcoatTexture"], coat["clearcoatRoughnessTexture"], "one map: weight in R, roughness in G");
        let sheen = &ext[KHR_SHEEN];
        assert!(sheen["sheenColorTexture"].is_object() && sheen["sheenRoughnessTexture"].is_null(), "no alpha: no roughness texture");
        assert_eq!(sheen["sheenRoughnessFactor"], json!(0.5));
        let doc = json_of(GlbTextures::from_files(vec![file("Sheen.png", png(image::ColorType::Rgba8))]));
        assert_eq!(doc["extensionsUsed"], json!([KHR_SHEEN]));
        assert_eq!(doc["materials"][0]["extensions"][KHR_SHEEN]["sheenRoughnessTexture"], doc["materials"][0]["extensions"][KHR_SHEEN]["sheenColorTexture"]);
        assert!(json_of(GlbTextures::default()).get("extensionsUsed").is_none());
    }
}

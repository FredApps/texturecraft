//! USD export of the working mesh: `.usda` (text), `.usdc` (Crate binary) or `.usdz` (a
//! package of a `.usdc` layer and its PNG textures). One `Mesh` prim (points, normals and
//! `primvars:st` per vertex, triangles), one `materialBind` GeomSubset per material when there
//! are several, and a UsdPreviewSurface material per material whose maps (base color,
//! roughness, metallic, normal, emissive, opacity, clear coat and its roughness) are
//! UsdUVTexture shaders. Meters, Y up.

use std::fmt::Write as _;
use std::io::Write as _;

use texturecraft_core::{Channel, NormalFormat};
use texturecraft_geom::Mesh;

use super::{is_black, parts};
use crate::export::{ExportOptions, ExportPreset, ExportedFile, FileFormat, MapSpec, VirtualMap, sanitize_file_stem};
use crate::presets::{ch, gray, rgb, virt};
use crate::{FormatError, Result};

/// The three USD encodings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsdFormat {
    Usda,
    Usdc,
    Usdz,
}

impl UsdFormat {
    /// From a file extension (`usda`, `usdc`, `usdz`; `usd` is the binary).
    pub fn from_ext(ext: &str) -> Option<UsdFormat> {
        match ext.to_ascii_lowercase().as_str() {
            "usda" => Some(UsdFormat::Usda),
            "usdc" | "usd" => Some(UsdFormat::Usdc),
            "usdz" => Some(UsdFormat::Usdz),
            _ => None,
        }
    }
}

/// A USD export: the layer or package, and (usda / usdc) the textures it references by
/// relative file name, to be written next to it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UsdExport {
    pub bytes: Vec<u8>,
    pub textures: Vec<(String, Vec<u8>)>,
}

/// A UsdPreviewSurface input fed by a texture: (file stem, input, input type, texture output,
/// sRGB).
pub(crate) const SLOTS: [(&str, &str, &str, &str, bool); 8] = [
    ("BaseColor", "diffuseColor", "color3f", "rgb", true),
    ("Roughness", "roughness", "float", "r", false),
    ("Metallic", "metallic", "float", "r", false),
    ("Normal", "normal", "normal3f", "rgb", false),
    ("Emissive", "emissiveColor", "color3f", "rgb", true),
    ("Opacity", "opacity", "float", "r", false),
    ("Clearcoat", "clearcoat", "float", "r", false),
    ("ClearcoatRoughness", "clearcoatRoughness", "float", "r", false),
];

/// Texture export options for [`usd_bytes`]: one 8-bit PNG per UsdPreviewSurface input, named
/// by slot (maps the texture set has no data for are not produced).
pub fn usd_export_options() -> ExportOptions {
    let maps = SLOTS
        .iter()
        .map(|&(stem, ..)| {
            let channels = match stem {
                "BaseColor" => rgb(|k| ch(Channel::BaseColor, k)),
                "Roughness" => gray(ch(Channel::Roughness, 0)),
                "Metallic" => gray(ch(Channel::Metallic, 0)),
                "Normal" => rgb(|k| virt(VirtualMap::NormalOpenGl, k)),
                "Emissive" => rgb(|k| ch(Channel::Emissive, k)),
                "Clearcoat" => gray(ch(Channel::CoatWeight, 0)),
                "ClearcoatRoughness" => gray(ch(Channel::CoatRoughness, 0)),
                _ => gray(ch(Channel::Opacity, 0)),
            };
            MapSpec { file: stem.into(), format: FileFormat::Png, bit_depth: 8, channels }
        })
        .collect();
    super::png8_options(ExportPreset { name: "USD preview surface".into(), maps }, NormalFormat::OpenGl)
}

/// A value written to a layer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum UsdValue {
    Token(String),
    Str(String),
    Int(i32),
    Double(f64),
    Asset(String),
    Float4([f32; 4]),
    Ints(Vec<i32>),
    Vec2fs(Vec<[f32; 2]>),
    Vec3fs(Vec<[f32; 3]>),
    /// A relationship target or attribute connection (absolute path).
    Target(String),
}

/// An attribute (or, with type `rel`, a relationship).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UsdAttr {
    pub name: String,
    pub type_name: &'static str,
    pub uniform: bool,
    pub value: Option<UsdValue>,
    /// Connection source of an attribute (`name.connect`).
    pub connect: Option<String>,
    pub interpolation: Option<&'static str>,
}

impl UsdAttr {
    fn new(name: &str, type_name: &'static str, value: Option<UsdValue>) -> UsdAttr {
        UsdAttr { name: name.into(), type_name, uniform: false, value, connect: None, interpolation: None }
    }

    fn uniform(mut self) -> UsdAttr {
        self.uniform = true;
        self
    }

    fn rel(name: &str, target: String) -> UsdAttr {
        UsdAttr::new(name, "rel", Some(UsdValue::Target(target)))
    }

    fn connected(name: &str, type_name: &'static str, source: String) -> UsdAttr {
        UsdAttr { connect: Some(source), ..UsdAttr::new(name, type_name, None) }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UsdPrim {
    pub name: String,
    pub type_name: &'static str,
    /// Applies MaterialBindingAPI (the prim has a `material:binding`).
    pub binding_api: bool,
    pub attrs: Vec<UsdAttr>,
    pub children: Vec<UsdPrim>,
}

impl UsdPrim {
    fn new(name: &str, type_name: &'static str) -> UsdPrim {
        UsdPrim { name: name.into(), type_name, binding_api: false, attrs: Vec::new(), children: Vec::new() }
    }
}

/// A layer: its root prim (the default prim), in meters, Y up.
pub(crate) struct UsdLayer {
    pub root: UsdPrim,
}

/// A valid, unique prim name for `s`.
fn identifier(s: &str, taken: &mut Vec<String>) -> String {
    let mut id: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if !id.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
        id.insert(0, '_');
    }
    let base = id.clone();
    let mut k = 1;
    while taken.contains(&id) {
        id = format!("{base}_{k}");
        k += 1;
    }
    taken.push(id.clone());
    id
}

/// The export scene: `/<name>` with the `Mesh` and `Materials`. `files[m]` are the texture
/// file names (by slot) of material `m`.
fn scene(mesh: &Mesh, name: &str, files: &[Vec<(usize, String)>]) -> Result<UsdLayer> {
    let vertex_count = mesh.positions.len();
    if vertex_count == 0 || mesh.indices.len() < 3 {
        return Err(FormatError::Invalid("the mesh is empty".into()));
    }

    let ints =
        |v: &[u32]| v.iter().map(|&i| i32::try_from(i).map_err(|_| FormatError::TooLarge("vertex index beyond 2^31".into()))).collect::<Result<Vec<i32>>>();
    let mut taken = Vec::new();
    let root_name = identifier(name, &mut taken);
    let mut root = UsdPrim::new(&root_name, "Xform");
    let mut geometry = UsdPrim::new(&identifier(&format!("{name}_mesh"), &mut taken), "Mesh");
    let material_names: Vec<String> = mesh.materials.iter().map(|m| identifier(m, &mut taken)).collect();
    let material_path = |m: usize| material_names.get(m).map(|id| format!("/{root_name}/Materials/{id}"));
    let (lo, hi) = mesh.bounds();

    // Faces in submesh order; subsets index them
    let mut indices: Vec<u32> = Vec::with_capacity(mesh.indices.len());
    let mut faces_of: Vec<Vec<i32>> = vec![Vec::new(); mesh.materials.len()];
    for part in parts(mesh, name) {
        let range = mesh.indices.get(part.start as usize..(part.start as usize).saturating_add(part.count as usize)).unwrap_or_default();
        for t in range.as_chunks::<3>().0 {
            if let Some(f) = faces_of.get_mut(part.material) {
                f.push(i32::try_from(indices.len() / 3).map_err(|_| FormatError::TooLarge("triangle count".into()))?);
            }
            indices.extend_from_slice(t);
        }
    }

    let vertex = |mut a: UsdAttr| {
        a.interpolation = Some("vertex");
        a
    };
    geometry.attrs = vec![
        UsdAttr::new("extent", "float3[]", Some(UsdValue::Vec3fs(vec![lo.to_array(), hi.to_array()]))),
        UsdAttr::new("faceVertexCounts", "int[]", Some(UsdValue::Ints(vec![3; indices.len() / 3]))),
        UsdAttr::new("faceVertexIndices", "int[]", Some(UsdValue::Ints(ints(&indices)?))),
        UsdAttr::new("points", "point3f[]", Some(UsdValue::Vec3fs(mesh.positions.iter().map(|p| p.to_array()).collect()))),
        UsdAttr::new("subdivisionScheme", "token", Some(UsdValue::Token("none".into()))).uniform(),
    ];
    if mesh.normals.len() == vertex_count {
        geometry.attrs.push(vertex(UsdAttr::new("normals", "normal3f[]", Some(UsdValue::Vec3fs(mesh.normals.iter().map(|v| v.to_array()).collect())))));
    }
    if mesh.uvs.len() == vertex_count {
        // USD's st origin is bottom-left
        let st = mesh.uvs.iter().map(|t| [t.x, 1.0 - t.y]).collect();
        geometry.attrs.push(vertex(UsdAttr::new("primvars:st", "texCoord2f[]", Some(UsdValue::Vec2fs(st)))));
    }

    let used: Vec<usize> = (0..mesh.materials.len()).filter(|m| faces_of.get(*m).is_some_and(|f| !f.is_empty())).collect();
    match used.as_slice() {
        [] => {}
        [m] => {
            geometry.binding_api = true;
            geometry.attrs.extend(material_path(*m).map(|p| UsdAttr::rel("material:binding", p)));
        }
        _ => {
            for &m in &used {
                let (Some(id), Some(path)) = (material_names.get(m), material_path(m)) else { continue };
                let mut subset = UsdPrim::new(id, "GeomSubset");
                subset.binding_api = true;
                subset.attrs = vec![
                    UsdAttr::new("elementType", "token", Some(UsdValue::Token("face".into()))).uniform(),
                    UsdAttr::new("familyName", "token", Some(UsdValue::Token("materialBind".into()))).uniform(),
                    UsdAttr::new("indices", "int[]", Some(UsdValue::Ints(faces_of.get(m).cloned().unwrap_or_default()))),
                    UsdAttr::rel("material:binding", path),
                ];
                geometry.children.push(subset);
            }
        }
    }

    root.children.push(geometry);
    let mut materials = UsdPrim::new("Materials", "Scope");
    for (m, id) in material_names.iter().enumerate() {
        materials.children.push(material(&format!("/{root_name}/Materials/{id}"), id, files.get(m).map_or(&[][..], Vec::as_slice)));
    }
    if !materials.children.is_empty() {
        root.children.push(materials);
    }

    Ok(UsdLayer { root })
}

/// A UsdPreviewSurface material at `path` whose slot inputs read `files` (slot, file name).
fn material(path: &str, id: &str, files: &[(usize, String)]) -> UsdPrim {
    let mut mat = UsdPrim::new(id, "Material");
    mat.attrs.push(UsdAttr::connected("outputs:surface", "token", format!("{path}/PBR.outputs:surface")));
    let mut pbr = UsdPrim::new("PBR", "Shader");
    pbr.attrs.push(UsdAttr::new("info:id", "token", Some(UsdValue::Token("UsdPreviewSurface".into()))).uniform());
    let mut textures = Vec::new();
    for (slot, file) in files {
        let Some(&(stem, input, input_type, output, srgb)) = SLOTS.get(*slot) else { continue };
        pbr.attrs.push(UsdAttr::connected(&format!("inputs:{input}"), input_type, format!("{path}/{stem}.outputs:{output}")));
        let mut tex = UsdPrim::new(stem, "Shader");
        let token = |s: &str| Some(UsdValue::Token(s.into()));
        tex.attrs = vec![
            UsdAttr::new("info:id", "token", token("UsdUVTexture")).uniform(),
            UsdAttr::new("inputs:file", "asset", Some(UsdValue::Asset(format!("./{file}")))),
            UsdAttr::connected("inputs:st", "float2", format!("{path}/st.outputs:result")),
            UsdAttr::new("inputs:sourceColorSpace", "token", token(if srgb { "sRGB" } else { "raw" })),
            UsdAttr::new("inputs:wrapS", "token", token("repeat")),
            UsdAttr::new("inputs:wrapT", "token", token("repeat")),
        ];
        if stem == "Normal" {
            // Unpack 0..1 texels to -1..1 vectors.
            tex.attrs.push(UsdAttr::new("inputs:scale", "float4", Some(UsdValue::Float4([2.0, 2.0, 2.0, 1.0]))));
            tex.attrs.push(UsdAttr::new("inputs:bias", "float4", Some(UsdValue::Float4([-1.0, -1.0, -1.0, 0.0]))));
        }
        tex.attrs.push(UsdAttr::new(&format!("outputs:{output}"), if output == "rgb" { "float3" } else { "float" }, None));
        textures.push(tex);
    }
    pbr.attrs.push(UsdAttr::new("inputs:useSpecularWorkflow", "int", Some(UsdValue::Int(0))));
    pbr.attrs.push(UsdAttr::new("outputs:surface", "token", None));
    mat.children.push(pbr);
    if !textures.is_empty() {
        let mut st = UsdPrim::new("st", "Shader");
        st.attrs = vec![
            UsdAttr::new("info:id", "token", Some(UsdValue::Token("UsdPrimvarReader_float2".into()))).uniform(),
            // A string, not a token: usdchecker's ARKit profile refuses a token here.
            UsdAttr::new("inputs:varname", "string", Some(UsdValue::Str("st".into()))),
            UsdAttr::new("outputs:result", "float2", None),
        ];
        mat.children.push(st);
        mat.children.extend(textures);
    }
    mat
}

/// The mesh as USD named `name`, with `textures[m]` (rendered with [`usd_export_options`]) on
/// material `m`. usda / usdc reference their textures as `<name>_<material>_<slot>.png` next
/// to the layer; usdz embeds them.
pub fn usd_bytes(mesh: &Mesh, name: &str, textures: &[Vec<ExportedFile>], format: UsdFormat) -> Result<UsdExport> {
    let stem = sanitize_file_stem(name);
    let (names, files) = material_maps(mesh, &stem, textures);
    let layer = scene(mesh, name, &names)?;
    Ok(match format {
        UsdFormat::Usda => UsdExport { bytes: usda(&layer).into_bytes(), textures: files },
        UsdFormat::Usdc => UsdExport { bytes: super::usdc::write(&layer)?, textures: files },
        UsdFormat::Usdz => UsdExport { bytes: usdz(&format!("{stem}.usdc"), &super::usdc::write(&layer)?, &files)?, textures: Vec::new() },
    })
}

/// Files by name, with their bytes.
pub type NamedFiles = Vec<(String, Vec<u8>)>;

/// The maps of each material worth writing (`textures[m]`, rendered with
/// [`usd_export_options`]): per material its [`SLOTS`] indices and file names
/// (`<stem>_<material>_<slot>.png`), and the files with their bytes.
pub(crate) fn material_maps(mesh: &Mesh, stem: &str, textures: &[Vec<ExportedFile>]) -> (Vec<Vec<(usize, String)>>, NamedFiles) {
    let mut files = Vec::new();
    let mut names = Vec::with_capacity(mesh.materials.len());
    for (m, material) in super::unique_ids(&mesh.materials, sanitize_file_stem).iter().enumerate() {
        let mut slots = Vec::new();
        let maps = textures.get(m).map_or(&[][..], Vec::as_slice);
        // Without a coat its roughness means nothing.
        let no_coat = maps.iter().find(|f| f.name == "Clearcoat.png").is_none_or(|f| is_black(&f.bytes));
        for f in maps {
            let slot_stem = f.name.strip_suffix(".png").unwrap_or_default();
            let Some(slot) = SLOTS.iter().position(|s| s.0 == slot_stem) else { continue };
            let skip = match slot_stem {
                "Emissive" => is_black(&f.bytes),
                "Clearcoat" | "ClearcoatRoughness" => no_coat,
                _ => false,
            };
            if skip {
                continue;
            }
            let file = format!("{stem}_{material}_{slot_stem}.png");
            slots.push((slot, file.clone()));
            files.push((file, f.bytes.clone()));
        }
        names.push(slots);
    }
    (names, files)
}

/// A usdz package: the layer first, every entry stored uncompressed at a 64-byte aligned offset.
pub(crate) fn usdz(layer_name: &str, layer: &[u8], files: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored).with_alignment(64);
    let err = |e: zip::result::ZipError| FormatError::Invalid(format!("usdz package: {e}"));
    for (name, bytes) in std::iter::once((layer_name, layer)).chain(files.iter().map(|(n, b)| (n.as_str(), b.as_slice()))) {
        zip.start_file(name, opts).map_err(err)?;
        zip.write_all(bytes)?;
    }
    Ok(zip.finish().map_err(err)?.into_inner())
}

/// Text of a value as usda writes it.
fn usda_value(s: &mut String, v: &UsdValue) {
    let tuple = |s: &mut String, c: &[f32]| {
        s.push('(');
        for (i, x) in c.iter().enumerate() {
            let _ = write!(s, "{}{x}", if i > 0 { ", " } else { "" });
        }
        s.push(')');
    };
    let list = |s: &mut String, n: usize, item: &mut dyn FnMut(&mut String, usize)| {
        s.push('[');
        for i in 0..n {
            if i > 0 {
                s.push_str(", ");
            }
            item(s, i);
        }
        s.push(']');
    };
    match v {
        UsdValue::Token(t) | UsdValue::Str(t) => {
            let _ = write!(s, "\"{t}\"");
        }
        UsdValue::Int(i) => {
            let _ = write!(s, "{i}");
        }
        UsdValue::Double(d) => {
            let _ = write!(s, "{d}");
        }
        UsdValue::Asset(a) => {
            let _ = write!(s, "@{a}@");
        }
        UsdValue::Target(p) => {
            let _ = write!(s, "<{p}>");
        }
        UsdValue::Float4(c) => tuple(s, c),
        UsdValue::Ints(v) => list(s, v.len(), &mut |s, i| {
            let _ = write!(s, "{}", v.get(i).copied().unwrap_or_default());
        }),
        UsdValue::Vec2fs(v) => list(s, v.len(), &mut |s, i| tuple(s, v.get(i).map_or(&[][..], |x| &x[..]))),
        UsdValue::Vec3fs(v) => list(s, v.len(), &mut |s, i| tuple(s, v.get(i).map_or(&[][..], |x| &x[..]))),
    }
}

fn usda_prim(s: &mut String, p: &UsdPrim, depth: usize) {
    let pad = "    ".repeat(depth);
    let _ = write!(s, "\n{pad}def {} \"{}\"", p.type_name, p.name);
    if p.binding_api {
        let _ = write!(s, " (\n{pad}    prepend apiSchemas = [\"MaterialBindingAPI\"]\n{pad})");
    }
    let _ = writeln!(s, "\n{pad}{{");
    for a in &p.attrs {
        s.push_str(&pad);
        s.push_str("    ");
        if a.type_name == "rel" {
            let _ = write!(s, "rel {}", a.name);
        } else {
            let _ = write!(s, "{}{} {}", if a.uniform { "uniform " } else { "" }, a.type_name, a.name);
        }
        if let Some(c) = &a.connect {
            let _ = write!(s, ".connect = <{c}>");
        } else if let Some(v) = &a.value {
            s.push_str(" = ");
            usda_value(s, v);
        }
        if let Some(i) = a.interpolation {
            let _ = write!(s, " (\n{pad}        interpolation = \"{i}\"\n{pad}    )");
        }
        s.push('\n');
    }
    for c in &p.children {
        usda_prim(s, c, depth + 1);
    }
    let _ = writeln!(s, "{pad}}}");
}

/// The layer as `.usda` text.
pub(crate) fn usda(layer: &UsdLayer) -> String {
    let mut s = format!(
        "#usda 1.0\n(\n    defaultPrim = \"{}\"\n    doc = \"Exported by TextureCraft\"\n    metersPerUnit = 1\n    upAxis = \"Y\"\n)\n",
        layer.root.name
    );
    usda_prim(&mut s, &layer.root, 0);
    s
}

//! glTF 2.0 / GLB: the default scene's node tree, flattened to world space.

use gltf::accessor::{Accessor, DataType, Dimensions};
use gltf::mesh::Mode;
use gltf::{Node, Semantic};
use texturecraft_geom::glam::{Mat3, Mat4, Vec2, Vec3, Vec4};

use super::{RawMesh, check_triangles, check_vertices};
use crate::{FormatError, Result, malformed};

const KIND: &str = "glTF";
/// Deepest node hierarchy followed (also stops cyclic node graphs).
const MAX_DEPTH: usize = 64;
/// Node instances visited (bounds exponential instancing in shared node graphs).
const MAX_NODE_VISITS: usize = 1_000_000;

/// A glTF or GLB from its bytes alone: buffers and images in other files must already be folded
/// in as data URIs ([`super::self_contained_mesh`]).
pub(crate) fn read(bytes: &[u8]) -> Result<RawMesh> {
    crate::guard(KIND, || read_inner(bytes))
}

fn read_inner(bytes: &[u8]) -> Result<RawMesh> {
    let gltf::Gltf { document, blob } = gltf::Gltf::from_slice(bytes).map_err(|e| malformed(KIND, e))?;
    check_buffer_uris(&document)?;
    let buffers = gltf::import_buffers(&document, None, blob).map_err(|e| match e {
        gltf::Error::ExternalReferenceInSliceImport => FormatError::Unsupported(
            "this glTF's buffers are in files that were not found next to it; keep its .bin files beside the .gltf, or use a .glb".into(),
        ),
        e => malformed(KIND, e),
    })?;

    let roots: Vec<Node> = match document.default_scene().or_else(|| document.scenes().next()) {
        Some(scene) => scene.nodes().collect(),
        None => {
            // No scene: every node that is nobody's child.
            let mut is_child = vec![false; document.nodes().len()];
            for n in document.nodes() {
                for c in n.children() {
                    if let Some(f) = is_child.get_mut(c.index()) {
                        *f = true;
                    }
                }
            }
            document.nodes().filter(|n| !is_child.get(n.index()).copied().unwrap_or(true)).collect()
        }
    };
    let mut raw = RawMesh { normals: Some(Vec::new()), ..RawMesh::default() };
    let mut attrs = Attrs::default();
    let mut stack: Vec<(Node, Mat4, usize)> = roots.into_iter().rev().map(|n| (n, Mat4::IDENTITY, 0)).collect();
    let mut visits = 0usize;
    while let Some((node, parent, depth)) = stack.pop() {
        visits += 1;
        if visits > MAX_NODE_VISITS {
            return Err(FormatError::TooLarge(format!("the glTF scene has more than {MAX_NODE_VISITS} node instances")));
        }
        let m = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
        if let Some(cam) = node.camera()
            && raw.cameras.len() < super::MAX_CAMERAS
        {
            let (fov, ortho_half_height) = match cam.projection() {
                gltf::camera::Projection::Perspective(p) => (p.yfov().to_degrees(), None),
                gltf::camera::Projection::Orthographic(o) => (40.0, Some(o.ymag())),
            };
            let name = cam.name().or_else(|| node.name()).map_or_else(|| format!("Camera {}", raw.cameras.len() + 1), str::to_string);
            raw.cameras.push(super::RawCamera::placed(name, m, Vec3::NEG_Z, fov, ortho_half_height));
        }
        if let Some(mesh) = node.mesh() {
            let part = node.name().or_else(|| mesh.name()).unwrap_or_default();
            for prim in mesh.primitives() {
                add_primitive(&mut raw, &mut attrs, &prim, m, &buffers, part)?;
            }
        }
        if depth >= MAX_DEPTH {
            return Err(malformed(KIND, "the node hierarchy is nested too deeply (or cyclic)"));
        }
        stack.extend(node.children().collect::<Vec<_>>().into_iter().rev().map(|c| (c, m, depth + 1)));
    }
    match attrs.uv_seen {
        (_, false) => raw.uvs = Some(attrs.uvs),
        (true, true) => raw.warnings.push("some primitives have no UVs; the whole mesh was unwrapped automatically".into()),
        (false, true) => {}
    }
    raw.colors = attrs.color_seen.then_some(attrs.colors);
    Ok(raw)
}

/// Per-vertex attributes gathered across primitives.
#[derive(Default)]
struct Attrs {
    uvs: Vec<Vec2>,
    /// (some primitive had UVs, some primitive had none)
    uv_seen: (bool, bool),
    /// White where a primitive has no COLOR_0.
    colors: Vec<Vec4>,
    color_seen: bool,
}

fn add_primitive(raw: &mut RawMesh, attrs: &mut Attrs, primitive: &gltf::Primitive, transform: Mat4, buffers: &[gltf::buffer::Data], part: &str) -> Result<()> {
    if primitive.mode() != Mode::Triangles {
        raw.warnings.push(format!("skipped a primitive drawn as {:?} (only triangles are supported)", primitive.mode()));
        return Ok(());
    }

    let Some(position_accessor) = primitive.get(&Semantic::Positions) else {
        raw.warnings.push("skipped a primitive without positions".into());
        return Ok(());
    };

    check_accessor(&position_accessor, buffers, Dimensions::Vec3, &[DataType::F32], "position")?;
    let normal_accessor = primitive.get(&Semantic::Normals);
    if let Some(a) = &normal_accessor {
        check_accessor(a, buffers, Dimensions::Vec3, &[DataType::F32], "normal")?;
    }
    let texcoord_accessor = primitive.get(&Semantic::TexCoords(0));
    if let Some(a) = &texcoord_accessor {
        check_accessor(a, buffers, Dimensions::Vec2, &[DataType::F32, DataType::U8, DataType::U16], "texcoord")?;
    }
    let color_accessor = primitive.get(&Semantic::Colors(0));
    if let Some(a) = &color_accessor {
        let dims = if a.dimensions() == Dimensions::Vec3 { Dimensions::Vec3 } else { Dimensions::Vec4 };
        check_accessor(a, buffers, dims, &[DataType::F32, DataType::U8, DataType::U16], "color")?;
    }
    if let Some(a) = primitive.indices() {
        check_accessor(&a, buffers, Dimensions::Scalar, &[DataType::U8, DataType::U16, DataType::U32], "index")?;
    }

    let reader = primitive.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
    let positions: Vec<Vec3> = reader.read_positions().ok_or_else(|| malformed(KIND, "unreadable positions"))?.map(Vec3::from).collect();
    let n = positions.len();
    check_vertices(raw.positions.len().saturating_add(n))?;
    let mut indices: Vec<u32> = match reader.read_indices() {
        Some(i) => i.into_u32().collect(),
        None => (0..n as u32).collect(),
    };
    indices.truncate(indices.len() / 3 * 3);
    if let Some(bad) = indices.iter().find(|i| **i as usize >= n) {
        return Err(malformed(KIND, format!("index {bad} out of range ({n} vertices)")));
    }

    check_triangles(raw.triangle_count(), indices.len() / 3)?;
    let normals: Option<Vec<Vec3>> = normal_accessor.and_then(|_| reader.read_normals()).map(|it| it.map(Vec3::from).collect());
    let texcoords: Option<Vec<Vec2>> = texcoord_accessor.and_then(|_| reader.read_tex_coords(0)).map(|t| t.into_f32().map(Vec2::from).collect());
    let colors: Option<Vec<Vec4>> =
        color_accessor.and_then(|_| reader.read_colors(0)).map(|c| c.into_rgba_f32().map(Vec4::from).collect()).filter(|c: &Vec<Vec4>| c.len() == n);
    attrs.color_seen |= colors.is_some();
    let color_of = |i: usize| colors.as_ref().and_then(|c| c.get(i)).copied().unwrap_or(Vec4::ONE);

    let linear_transform = Mat3::from_mat4(transform);
    let normal_transform = linear_transform.inverse().transpose();
    let flip = linear_transform.determinant() < 0.0;
    let world: Vec<Vec3> = positions.iter().map(|p| transform.transform_point3(*p)).collect();
    let name = match primitive.material().index() {
        None => "Default".to_string(),
        Some(i) => primitive.material().name().map(str::to_string).unwrap_or_else(|| format!("Material_{i}")),
    };
    if !raw.base_colors.iter().any(|c| c.material == name) {
        base_color(raw, &primitive.material(), &name, buffers);
    }
    let base = raw.positions.len() as u32;
    let has_texcoords = texcoords.as_ref().is_some_and(|t| t.len() == n);
    if has_texcoords {
        attrs.uv_seen.0 = true
    } else {
        attrs.uv_seen.1 = true
    }

    let uv_of = |i: usize| texcoords.as_ref().and_then(|t| t.get(i)).copied().unwrap_or_default();
    let raw_normals = raw.normals.get_or_insert_with(Vec::new);
    match normals.filter(|v| v.len() == n) {
        Some(nrm) => {
            raw.positions.extend_from_slice(&world);
            raw_normals.extend(nrm.iter().map(|v| (normal_transform * *v).try_normalize().unwrap_or(Vec3::Y)));
            attrs.uvs.extend((0..n).map(uv_of));
            attrs.colors.extend((0..n).map(color_of));
            let group = raw.group(&name, part);
            for t in indices.as_chunks::<3>().0 {
                let (a, b, c) = (t[0] + base, t[1] + base, t[2] + base);
                group.extend_from_slice(&if flip { [a, c, b] } else { [a, b, c] });
            }
        }
        None => {
            // glTF: flat normals when none are given, so every triangle gets its own corners
            let mut triangles = Vec::with_capacity(indices.len());
            for t in indices.as_chunks::<3>().0 {
                let order = if flip { [t[0], t[2], t[1]] } else { [t[0], t[1], t[2]] };
                let p = order.map(|i| world.get(i as usize).copied().unwrap_or_default());
                let face = (p[1] - p[0]).cross(p[2] - p[0]).try_normalize().unwrap_or(Vec3::Y);
                for (k, i) in order.iter().enumerate() {
                    triangles.push(raw.positions.len() as u32);
                    raw.positions.push(p[k]);
                    raw_normals.push(face);
                    attrs.uvs.push(uv_of(*i as usize));
                    attrs.colors.push(color_of(*i as usize));
                }
            }
            raw.group(&name, part).extend_from_slice(&triangles);
        }
    }

    Ok(())
}

/// Record `material`'s base color factor and embedded texture (GLB buffer view or data URI)
/// unless it is plain white.
fn base_color(raw: &mut RawMesh, material: &gltf::Material, name: &str, buffers: &[gltf::buffer::Data]) {
    let pbr = material.pbr_metallic_roughness();
    let factor = Vec4::from(pbr.base_color_factor());
    let image = pbr.base_color_texture().and_then(|info| {
        if info.tex_coord() != 0 {
            raw.warnings.push(format!("the base color texture of {name} uses UV set {}; only the first is read", info.tex_coord()));
            return None;
        }
        match info.texture().source().source() {
            gltf::image::Source::View { view, .. } => {
                let data = buffers.get(view.buffer().index())?;
                data.0.get(view.offset()..view.offset().checked_add(view.length())?).map(<[u8]>::to_vec)
            }
            // Files next to the .gltf arrive as data URIs; anything else can't be read.
            gltf::image::Source::Uri { uri, .. } if uri.starts_with("data:") => {
                gltf::buffer::Data::from_source(gltf::buffer::Source::Uri(uri), None).ok().map(|d| d.0)
            }
            gltf::image::Source::Uri { .. } => None,
        }
    });
    if image.is_some() || factor != Vec4::ONE {
        raw.base_colors.push(super::RawBaseColor { material: name.to_string(), factor, image });
    }
}

/// Reject accessors the `gltf` reader would panic on or read out of bounds.
fn check_accessor(acc: &Accessor, buffers: &[gltf::buffer::Data], dims: Dimensions, types: &[DataType], what: &str) -> Result<()> {
    let bad = |msg: &str| malformed(KIND, format!("{what} accessor {}: {msg}", acc.index()));
    if acc.sparse().is_some() {
        return Err(FormatError::Unsupported(format!("sparse glTF accessors ({what})")));
    }
    if acc.dimensions() != dims || !types.contains(&acc.data_type()) {
        return Err(bad("unexpected element type"));
    }
    let view = acc.view().ok_or_else(|| bad("has no buffer view"))?;
    let data_len = buffers.get(view.buffer().index()).map(|d| d.0.len()).ok_or_else(|| bad("buffer is missing"))?;
    if view.offset().checked_add(view.length()).is_none_or(|end| end > data_len) {
        return Err(bad("buffer view exceeds its buffer"));
    }
    let size = acc.size();
    let stride = view.stride().unwrap_or(size);
    let end = acc.count().checked_sub(1).and_then(|k| k.checked_mul(stride)).and_then(|x| x.checked_add(acc.offset())).and_then(|x| x.checked_add(size));
    if stride < size || end.is_none_or(|e| e > view.length()) {
        return Err(bad("data exceeds its buffer view"));
    }
    Ok(())
}

/// Check buffer URIs before `gltf` resolves them: percent-decoding must give UTF-8 (`gltf`
/// panics otherwise).
fn check_buffer_uris(doc: &gltf::Document) -> Result<()> {
    for buf in doc.buffers() {
        if let gltf::buffer::Source::Uri(uri) = buf.source()
            && !uri.contains(':')
            && percent_decode(uri).is_none()
        {
            return Err(malformed(KIND, format!("buffer URI \"{uri}\" is not valid UTF-8")));
        }
    }
    Ok(())
}

pub(crate) fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        let hex = |k: usize| b.get(k).and_then(|h| (*h as char).to_digit(16));
        match (c, hex(i + 1), hex(i + 2)) {
            (b'%', Some(h), Some(l)) => {
                out.push((h * 16 + l) as u8);
                i += 3;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

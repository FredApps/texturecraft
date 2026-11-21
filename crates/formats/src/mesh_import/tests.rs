use texturecraft_geom::glam::{Vec2, Vec3, Vec4};

use super::*;

/// A GLB with one mesh of two primitives (materials "Red" and an unnamed one), placed under a
/// mirrored parent node (negative determinant) and a translated child.
fn glb(with_normals: bool, with_uvs: bool) -> Vec<u8> {
    let mut bin = Vec::new();
    for p in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        bin.extend(p.iter().flat_map(|v| v.to_le_bytes()));
    }
    for _ in 0..3 {
        bin.extend([0.0f32, 0.0, 1.0].iter().flat_map(|v| v.to_le_bytes()));
    }
    for uv in [[0.0f32, 1.0], [1.0, 1.0], [0.0, 0.0]] {
        bin.extend(uv.iter().flat_map(|v| v.to_le_bytes()));
    }
    for i in [0u16, 1, 2, 0] {
        bin.extend(i.to_le_bytes());
    }
    let mut attrs = serde_json::json!({ "POSITION": 0 });
    if with_normals {
        attrs["NORMAL"] = 1.into();
    }
    if with_uvs {
        attrs["TEXCOORD_0"] = 2.into();
    }
    let json = serde_json::json!({
        "asset": { "version": "2.0" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "scale": [-1.0, 1.0, 1.0], "children": [1] }, { "translation": [1.0, 0.0, 0.0], "mesh": 0 }],
        "meshes": [{ "primitives": [
            { "attributes": attrs, "indices": 3, "material": 0 },
            { "attributes": attrs, "indices": 3, "material": 1 }
        ] }],
        "materials": [{ "name": "Red" }, {}],
        "buffers": [{ "byteLength": bin.len() }],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0, "byteLength": 36 },
            { "buffer": 0, "byteOffset": 36, "byteLength": 36 },
            { "buffer": 0, "byteOffset": 72, "byteLength": 24 },
            { "buffer": 0, "byteOffset": 96, "byteLength": 6 }
        ],
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0] },
            { "bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3" },
            { "bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC2" },
            { "bufferView": 3, "componentType": 5123, "count": 3, "type": "SCALAR" }
        ]
    });
    let mut js = serde_json::to_vec(&json).unwrap();
    while !js.len().is_multiple_of(4) {
        js.push(b' ');
    }
    let total = 12 + 8 + js.len() + 8 + bin.len();
    let mut out = Vec::new();
    out.extend(b"glTF");
    out.extend(2u32.to_le_bytes());
    out.extend((total as u32).to_le_bytes());
    out.extend((js.len() as u32).to_le_bytes());
    out.extend(b"JSON");
    out.extend(&js);
    out.extend((bin.len() as u32).to_le_bytes());
    out.extend(b"BIN\0");
    out.extend(&bin);
    out
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in data.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for k in 0..4 {
            s.push(if k <= c.len() { T[(n >> (18 - 6 * k) & 63) as usize] as char } else { '=' });
        }
    }
    s
}

/// The same scene as an embedded .gltf (data-URI buffer).
fn embedded_gltf() -> Vec<u8> {
    let g = glb(true, true);
    let jlen = u32::from_le_bytes(g[12..16].try_into().unwrap()) as usize;
    let mut json: serde_json::Value = serde_json::from_slice(&g[20..20 + jlen]).unwrap();
    let bin = &g[20 + jlen + 8..];
    json["buffers"][0]["uri"] = format!("data:application/octet-stream;base64,{}", base64(bin)).into();
    serde_json::to_vec(&json).unwrap()
}

fn assert_outward(m: &Mesh) {
    for t in 0..m.triangle_count() {
        let [a, b, c] = m.tri(t).unwrap();
        let face = (m.positions[b] - m.positions[a]).cross(m.positions[c] - m.positions[a]);
        assert!(face.dot(m.normals[a]) > 0.0, "triangle {t} winding disagrees with its normal");
    }
}

#[test]
fn glb_two_materials_mirrored_node() {
    let im = import_mesh_bytes("scene.glb", &glb(true, true)).unwrap();
    let m = &im.mesh;
    assert_eq!(m.materials, vec!["Red".to_string(), "Material_1".to_string()]);
    assert_eq!(m.submeshes.len(), 2);
    assert_eq!((m.submeshes[1].start, m.submeshes[1].count), (3, 3));
    assert_eq!(im.report.triangles, 2);
    assert!(!im.report.uvs_generated);
    // Translation, then mirror in x.
    assert!(m.positions.contains(&Vec3::new(-1.0, 0.0, 0.0)));
    assert!(m.positions.contains(&Vec3::new(-2.0, 0.0, 0.0)));
    for n in &m.normals {
        assert!((*n - Vec3::Z).length() < 1e-5, "{n}");
    }
    assert_outward(m);
    assert!(m.uvs.contains(&Vec2::new(0.0, 1.0)));
    m.validate().unwrap();
}

/// A scene without a `nodes` array is legal glTF (Blender writes one for a scene with no exported
/// objects); it must not stop the default scene from importing.
#[test]
fn gltf_scene_without_nodes_imports_the_default_scene() {
    let mut json: serde_json::Value = serde_json::from_slice(&embedded_gltf()).unwrap();
    json["scenes"] = serde_json::json!([{ "name": "EmptyReviewScene" }, { "nodes": [0] }]);
    json["scene"] = 1.into();
    let im = import_mesh_bytes("scene.gltf", &serde_json::to_vec(&json).unwrap()).unwrap();
    assert_eq!(im.report.triangles, 2);
    // Only an empty scene: nothing to import, but a clear error rather than a parse failure.
    json["scenes"] = serde_json::json!([{ "name": "EmptyReviewScene" }]);
    json["scene"] = 0.into();
    let err = import_mesh_bytes("scene.gltf", &serde_json::to_vec(&json).unwrap()).unwrap_err();
    assert!(!err.to_string().contains("missing field"), "{err}");
}

#[test]
fn glb_scene_without_nodes_keeps_binary_mesh_in_default_scene() {
    let original = glb(true, true);
    let jlen = u32::from_le_bytes(original[12..16].try_into().unwrap()) as usize;
    let mut json: serde_json::Value = serde_json::from_slice(&original[20..20 + jlen]).unwrap();
    json["scenes"] = serde_json::json!([{ "name": "EmptyReviewScene" }, { "nodes": [0] }]);
    json["scene"] = 1.into();
    let mut js = serde_json::to_vec(&json).unwrap();
    while !js.len().is_multiple_of(4) {
        js.push(b' ');
    }
    let bin_chunk = &original[20 + jlen..];
    let mut glb = Vec::new();
    glb.extend(b"glTF");
    glb.extend(2u32.to_le_bytes());
    glb.extend(((20 + js.len() + bin_chunk.len()) as u32).to_le_bytes());
    glb.extend((js.len() as u32).to_le_bytes());
    glb.extend(b"JSON");
    glb.extend(js);
    glb.extend(bin_chunk);
    let im = import_mesh_bytes("scene.glb", &glb).unwrap();
    assert_eq!(im.report.triangles, 2);
    assert_eq!(im.mesh.materials, ["Red", "Material_1"]);
    assert_outward(&im.mesh);
}

#[test]
fn gltf_embedded_buffers_and_missing_attributes() {
    let im = import_mesh_bytes("scene.gltf", &embedded_gltf()).unwrap();
    assert_eq!(im.mesh.triangle_count(), 2);
    // No normals: flat normals with consistent winding; no UVs: automatic unwrap.
    let im = import_mesh_bytes("x.glb", &glb(false, false)).unwrap();
    assert!(im.report.uvs_generated);
    assert_outward(&im.mesh);
    assert!(im.mesh.uvs.iter().all(|uv| (0.0..=1.0).contains(&uv.x) && (0.0..=1.0).contains(&uv.y)));
    assert_eq!(im.report.uv_out_of_range, 0);
}

#[test]
fn gltf_mixed_primitive_uvs_warn_and_unwrap_the_whole_mesh() {
    let mut json: serde_json::Value = serde_json::from_slice(&embedded_gltf()).unwrap();
    let primitives = json["meshes"][0]["primitives"].as_array_mut().unwrap();
    assert_eq!(primitives.len(), 2);
    assert_eq!(primitives[0]["attributes"]["TEXCOORD_0"], 2);
    let second_attributes = primitives[1]["attributes"].as_object_mut().unwrap();
    assert_eq!(second_attributes.remove("TEXCOORD_0"), Some(serde_json::json!(2)));

    let imported = import_mesh_bytes("mixed-uvs.gltf", &serde_json::to_vec(&json).unwrap()).unwrap();
    assert!(imported.report.uvs_generated);
    let warning = "some primitives have no UVs; the whole mesh was unwrapped automatically";
    assert!(imported.report.warnings.iter().any(|w| w == warning));
    assert_eq!(imported.report.triangles, 2);
    assert_eq!(imported.mesh.materials, ["Red", "Material_1"]);
}

#[test]
fn gltf_external_buffer_needs_a_path() {
    let mut json: serde_json::Value = serde_json::from_slice(&embedded_gltf()).unwrap();
    json["buffers"][0]["uri"] = "scene.bin".into();
    let err = import_mesh_bytes("scene.gltf", &serde_json::to_vec(&json).unwrap()).unwrap_err();
    assert!(matches!(err, FormatError::Unsupported(_)), "{err}");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn gltf_external_buffer_from_disk() {
    let g = glb(true, true);
    let jlen = u32::from_le_bytes(g[12..16].try_into().unwrap()) as usize;
    let dir = std::env::temp_dir().join(format!("tc-formats-gltf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("my scene.bin"), &g[20 + jlen + 8..]).unwrap();
    let mut json: serde_json::Value = serde_json::from_slice(&g[20..20 + jlen]).unwrap();
    json["buffers"][0]["uri"] = "my%20scene.bin".into();
    std::fs::write(dir.join("scene.gltf"), serde_json::to_vec(&json).unwrap()).unwrap();
    let im = import_mesh(&dir.join("scene.gltf")).unwrap();
    assert_eq!(im.mesh.triangle_count(), 2);
    // A buffer that is a directory is refused before reading.
    json["buffers"][0]["uri"] = ".".into();
    std::fs::write(dir.join("bad.gltf"), serde_json::to_vec(&json).unwrap()).unwrap();
    assert!(import_mesh(&dir.join("bad.gltf")).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A .gltf whose buffer and base color image are files next to it becomes one self-contained
/// file that imports with its texture.
#[test]
fn gltf_external_files_are_folded_in() {
    let g = glb(true, true);
    let jlen = u32::from_le_bytes(g[12..16].try_into().unwrap()) as usize;
    let mut json: serde_json::Value = serde_json::from_slice(&g[20..20 + jlen]).unwrap();
    json["buffers"][0]["uri"] = "scene.bin".into();
    json["images"] = serde_json::json!([{ "uri": "tex/base%20color.png" }]);
    json["textures"] = serde_json::json!([{ "source": 0 }]);
    json["materials"][0]["pbrMetallicRoughness"] = serde_json::json!({ "baseColorTexture": { "index": 0 } });
    let gltf = serde_json::to_vec(&json).unwrap();
    let mut png = std::io::Cursor::new(Vec::new());
    image::write_buffer_with_format(&mut png, &[10u8, 200, 30].repeat(4), 2, 2, image::ColorType::Rgb8, image::ImageFormat::Png).unwrap();
    let files = [("scene.bin", g[20 + jlen + 8..].to_vec()), ("tex/base color.png", png.into_inner())];
    let mut read = |p: &str| files.iter().find(|(n, _)| *n == p).map(|(_, b)| b.clone());
    let f = self_contained_mesh("dir/scene.gltf", gltf.clone(), &mut read).unwrap();
    assert_eq!((f.name.as_str(), f.warnings.len()), ("dir/scene.gltf", 0));
    let hp = import_high_poly_bytes(&f.name, &f.bytes).unwrap();
    let red = hp.mesh.materials.iter().position(|m| m == "Red").unwrap();
    assert_eq!(hp.base_color[red].as_ref().map(|c| (c.width, c.texels[0])), Some((2, [10, 200, 30, 255])));
    // A missing image is named; the mesh still imports, untextured.
    let f = self_contained_mesh("scene.gltf", gltf, &mut |p: &str| (p == "scene.bin").then(|| files[0].1.clone())).unwrap();
    assert_eq!(f.warnings, vec!["scene.gltf: tex/base color.png was not found next to the mesh".to_string()]);
    assert!(import_high_poly_bytes(&f.name, &f.bytes).unwrap().base_color[red].is_none());
}

const OBJ: &str = "mtllib missing.mtl
o quad
v 0 0 0
v 1 0 0
v 1 1 0
v 0 1 0
v 2 0 0
vt 0 0
vt 1 0
vt 1 1
vt 0 1
vn 0 0 1
usemtl Wood
f 1/1/1 2/2/1 3/3/1
usemtl Metal
f 1/1/1 3/3/1 4/4/1
usemtl Wood
f 2/2/1 5/2/1 3/3/1 4/4/1
";

#[test]
fn obj_flips_v_and_keeps_usemtl_materials() {
    let im = import_mesh_bytes("a.obj", OBJ.as_bytes()).unwrap();
    let m = &im.mesh;
    assert_eq!(m.materials, vec!["Wood".to_string(), "Metal".to_string()]);
    // Wood: one triangle + a fan-triangulated quad; Metal: one triangle.
    assert_eq!(m.submeshes.iter().map(|s| s.count / 3).collect::<Vec<_>>(), vec![3, 1]);
    for (p, uv) in m.positions.iter().zip(&m.uvs) {
        if *p == Vec3::ZERO {
            assert_eq!(*uv, Vec2::new(0.0, 1.0), "OBJ v must be flipped");
        }
        if *p == Vec3::new(1.0, 1.0, 0.0) {
            assert_eq!(*uv, Vec2::new(1.0, 0.0));
        }
    }
    assert_outward(m);
    m.validate().unwrap();
}

#[test]
fn obj_without_uvs_or_normals_is_completed() {
    let im = import_mesh_bytes("b.OBJ", b"v 0 0 0\nv 1 0 0\nv 0 1 0\nv 0 0 1\nf 1 3 2\nf 1 2 4\nf 1 4 3\nf 2 3 4\n").unwrap();
    assert!(im.report.uvs_generated);
    assert_eq!(im.mesh.materials, vec!["Default".to_string()]);
    im.mesh.validate().unwrap();
    // Smooth normals over welded positions: every copy of a corner shares one normal.
    let n0: Vec<Vec3> = im.mesh.positions.iter().zip(&im.mesh.normals).filter(|(p, _)| **p == Vec3::ZERO).map(|(_, n)| *n).collect();
    assert!(n0.len() > 1 && n0.iter().all(|n| (*n - n0[0]).length() < 1e-5));
}

#[test]
fn obj_with_some_objects_having_uvs_preserves_uvs_without_full_unwrap() {
    let obj = "o Body
v 0 0 0
v 1 0 0
v 1 1 0
vt 0 0
vt 1 0
vt 1 1
f 1/1 2/2 3/3
o Helper
v 2 0 0
v 3 0 0
v 3 1 0
f 4 5 6
";
    let im = import_mesh_bytes("character.obj", obj.as_bytes()).unwrap();
    assert!(!im.report.uvs_generated);
    assert_eq!(im.mesh.uvs[0], Vec2::new(0.0, 1.0));
    assert_eq!(im.mesh.uvs[1], Vec2::new(1.0, 1.0));
    assert_eq!(im.mesh.uvs[2], Vec2::new(1.0, 0.0));
    assert_eq!(im.mesh.uvs[3], Vec2::ZERO);
    assert_eq!(im.mesh.uvs[4], Vec2::ZERO);
    assert_eq!(im.mesh.uvs[5], Vec2::ZERO);
    assert!(im.report.warnings.iter().any(|w| w.contains("some objects have no UVs")));
}

#[test]
fn polygons_keep_their_edges_for_the_wireframe() {
    // An OBJ quad and pentagon (with UVs, so no unwrap splits them), a triangle, and a PLY quad.
    let obj = "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nv 2 0 0\nv 3 0 0\nv 3 1 0\nv 2.5 2 0\nv 2 1 0\nvt 0 0\n\
               f 1/1 2/1 3/1 4/1\nf 5/1 6/1 7/1 8/1 9/1\nf 1/1 2/1 5/1\n";
    let m = import_mesh_bytes("p.obj", obj.as_bytes()).unwrap().mesh;
    assert_eq!(m.triangle_count(), 2 + 3 + 1);
    assert_eq!(m.wire_edges()[0].len(), 4 + 5 + 2, "the triangle adds edges 1-5 and 2-5");
    let ply = import_mesh_bytes("a.ply", ply_ascii().as_bytes()).unwrap().mesh;
    assert_eq!(ply.wire_edges()[0].len(), 4);
    // Triangles alone keep every edge.
    let tris = import_mesh_bytes("t.obj", b"v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nf 1/1 2/1 3/1\nf 1/1 3/1 4/1\n").unwrap().mesh;
    assert!(tris.diagonals.is_empty());
    assert_eq!(tris.wire_edges()[0].len(), 5);
}

#[test]
fn imported_quads_get_quad_tangents() {
    // A non-planar OBJ quad with sheared UVs: the importer keeps the quad for MikkTSpace, so its
    // tangents differ from the same corners written as two triangles (Blender hands quads over).
    let corners = "v 0 0 0
v 1 0 0
v 1 1 0.6
v 0 1 0
vt 0 0.1
vt 1 0
vt 0.9 1
vt 0.3 0.8
";
    let quad = import_mesh_bytes(
        "q.obj",
        format!(
            "{corners}f 1/1 2/2 3/3 4/4
"
        )
        .as_bytes(),
    )
    .unwrap()
    .mesh;
    let tris = import_mesh_bytes(
        "t.obj",
        format!(
            "{corners}f 1/1 2/2 3/3
f 1/1 3/3 4/4
"
        )
        .as_bytes(),
    )
    .unwrap()
    .mesh;
    assert_eq!((quad.indices.clone(), quad.positions.clone()), (tris.indices.clone(), tris.positions.clone()), "the render triangles are the same");
    let widest = quad.tangents.iter().zip(&tris.tangents).map(|(a, b)| a.truncate().dot(b.truncate()).clamp(-1.0, 1.0).acos().to_degrees()).fold(0.0, f32::max);
    assert!(widest > 1.0, "quad tangents differ from triangle tangents by only {widest} degrees");
    assert!(quad.tangents.iter().all(|t| t.is_finite()));
}

fn ply_ascii() -> String {
    "ply\nformat ascii 1.0\ncomment test\nelement vertex 4\nproperty float x\nproperty float y\nproperty float z\nproperty float s\nproperty float t\n\
     element face 1\nproperty list uchar int vertex_indices\nend_header\n\
     0 0 0 0 0\n1 0 0 1 0\n1 1 0 1 1\n0 1 0 0 1\n4 0 1 2 3\n"
        .to_string()
}

fn ply_binary(big_endian: bool) -> Vec<u8> {
    let enc = if big_endian { "binary_big_endian" } else { "binary_little_endian" };
    let mut b = format!(
        "ply\r\nformat {enc} 1.0\r\nelement vertex 4\r\nproperty double x\r\nproperty double y\r\nproperty double z\r\nproperty float nx\r\nproperty float ny\r\nproperty float nz\r\n\
         property uchar red\r\nelement face 1\r\nproperty list uchar uint vertex_indices\r\nend_header\r\n"
    )
    .into_bytes();
    let f64b = |v: f64| if big_endian { v.to_be_bytes().to_vec() } else { v.to_le_bytes().to_vec() };
    let f32b = |v: f32| if big_endian { v.to_be_bytes().to_vec() } else { v.to_le_bytes().to_vec() };
    let u32b = |v: u32| if big_endian { v.to_be_bytes().to_vec() } else { v.to_le_bytes().to_vec() };
    for p in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]] {
        for v in p {
            b.extend(f64b(v));
        }
        for v in [0.0, 0.0, 1.0] {
            b.extend(f32b(v));
        }
        b.push(200);
    }
    b.push(4);
    for i in [0, 1, 2, 3] {
        b.extend(u32b(i));
    }
    b
}

#[test]
fn ply_ascii_and_binary() {
    let a = import_mesh_bytes("a.ply", ply_ascii().as_bytes()).unwrap();
    assert_eq!(a.mesh.triangle_count(), 2);
    assert!(!a.report.uvs_generated);
    // PLY t is bottom-up like OBJ.
    assert_eq!(a.mesh.uvs[0], Vec2::new(0.0, 1.0));
    for be in [false, true] {
        let b = import_mesh_bytes("b.ply", &ply_binary(be)).unwrap();
        assert_eq!(b.mesh.triangle_count(), 2);
        assert!(b.report.uvs_generated);
        assert!(b.mesh.positions.contains(&Vec3::new(1.0, 1.0, 0.0)));
        assert_outward(&b.mesh);
    }
}

fn stl_binary() -> Vec<u8> {
    let mut b = vec![0u8; 80];
    b.extend(2u32.to_le_bytes());
    for tri in [[[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]], [[0.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]]] {
        b.extend([0.0f32; 3].iter().flat_map(|v| v.to_le_bytes()));
        for v in tri {
            b.extend(v.iter().flat_map(|x| x.to_le_bytes()));
        }
        b.extend([0u8, 0]);
    }
    b
}

const STL_ASCII: &str = "solid test
facet normal 0 0 1
 outer loop
  vertex 0 0 0
  vertex 1 0 0
  vertex 1 1 0
 endloop
endfacet
facet normal 0 0 1
 outer loop
  vertex 0 0 0
  vertex 1 1 0
  vertex 0 1 0
 endloop
endfacet
endsolid test
";

#[test]
fn stl_ascii_and_binary() {
    for (name, bytes) in [("a.stl", STL_ASCII.as_bytes().to_vec()), ("b.STL", stl_binary())] {
        let im = import_mesh_bytes(name, &bytes).unwrap();
        assert_eq!(im.mesh.triangle_count(), 2, "{name}");
        assert!(im.report.uvs_generated);
        assert_outward(&im.mesh);
    }
    // A binary file whose header starts with "solid" is still binary.
    let mut b = stl_binary();
    b[..5].copy_from_slice(b"solid");
    assert_eq!(import_mesh_bytes("c.stl", &b).unwrap().mesh.triangle_count(), 2);
}

#[test]
fn stl_shared_reader_keeps_flat_normals_and_surfaces_shared_repairs_and_colors() {
    let mut bytes = stl_binary();
    // VisCAM/SolidView: bit 15 marks a facet color; red occupies bits 10..14.
    bytes[132..134].copy_from_slice(&0xfc00u16.to_le_bytes());
    // A third facet repeats that color after an uncolored facet. The shared reader groups its
    // parts by color, so the adapter must restore the source facet order.
    bytes.extend([0.0f32; 3].iter().flat_map(|v| v.to_le_bytes()));
    for v in [[0.0f32, 0.0, 1.0], [1.0, 0.0, 1.0], [1.0, 1.0, 1.0]] {
        bytes.extend(v.iter().flat_map(|x| x.to_le_bytes()));
    }
    bytes.extend(0xfc00u16.to_le_bytes());
    // Some exporters leave the header count at zero despite having facets.
    bytes[80..84].copy_from_slice(&0u32.to_le_bytes());
    let imported = import_mesh_bytes("colored.stl", &bytes).unwrap();
    assert_eq!(imported.mesh.triangle_count(), 3);
    assert_eq!(imported.mesh.submeshes.len(), 1);
    assert_eq!(imported.mesh.submeshes[0].name, "colored");
    assert_eq!(
        imported.mesh.positions,
        vec![
            Vec3::ZERO,
            Vec3::X,
            Vec3::new(1.0, 1.0, 0.0),
            Vec3::ZERO,
            Vec3::new(1.0, 1.0, 0.0),
            Vec3::Y,
            Vec3::Z,
            Vec3::new(1.0, 0.0, 1.0),
            Vec3::new(1.0, 1.0, 1.0)
        ]
    );
    assert_eq!(imported.mesh.normals, vec![Vec3::Z; 9]);
    assert!(imported.mesh.colors.iter().any(|c| c.x > 0.99 && c.y < 0.01 && c.z < 0.01));
    assert!(imported.report.warnings.iter().any(|w| w.contains("counts no facets")));
}

/// Every prefix and some corruptions of a valid file: errors, never panics (and never an
/// error from the panic guard, which would mean a parser panicked).
pub(crate) fn hammer(name: &str, good: &[u8]) {
    let check = |bytes: &[u8]| {
        if std::env::var("TC_TRACE").is_ok() {
            std::fs::write(std::env::temp_dir().join("tc_last_case.bin"), bytes).unwrap();
            eprintln!("{name} {}", bytes.len());
        }
        if let Err(e) = import_mesh_bytes(name, bytes) {
            assert!(!e.to_string().contains("parser failed"), "{name}: a parser panicked on {} bytes", bytes.len());
        }
    };
    for len in 0..good.len() {
        check(&good[..len]);
    }
    for i in 0..good.len() {
        let mut b = good.to_vec();
        b[i] ^= 0xA5;
        check(&b);
        b[i] = 0xFF;
        check(&b);
    }
    assert!(import_mesh_bytes(name, &[]).is_err());
    assert!(import_mesh_bytes(name, b"\x00\x01garbage\xff\xfe").is_err());
}

#[test]
fn malformed_meshes_are_errors() {
    hammer("x.glb", &glb(true, true));
    hammer("x.gltf", &embedded_gltf());
    hammer("x.obj", OBJ.as_bytes());
    hammer("x.ply", ply_ascii().as_bytes());
    hammer("x.ply", &ply_binary(false));
    hammer("x.ply", &ply_binary(true));
    hammer("x.stl", STL_ASCII.as_bytes());
    hammer("x.stl", &stl_binary());
    assert!(matches!(import_mesh_bytes("x.blend", b"whatever"), Err(FormatError::Unsupported(_))));
    // Hostile counts never allocate their claim.
    let huge = "ply\nformat binary_little_endian 1.0\nelement vertex 4000000000\nproperty float x\nproperty float y\nproperty float z\nend_header\n";
    assert!(import_mesh_bytes("h.ply", huge.as_bytes()).is_err());
    let mut stl = vec![0u8; 80];
    stl.extend(u32::MAX.to_le_bytes());
    assert!(import_mesh_bytes("h.stl", &stl).is_err());
    // Meshes of only degenerate triangles are refused instead of crashing tangent generation.
    assert!(import_mesh_bytes("p.obj", b"v 1 1 1\nv 1 1 1\nv 1 1 1\nf 1 2 3\n").is_err());
    assert!(import_mesh_bytes("q.obj", b"v 0 0 0\nv 0 0 0\nv 0 1 0\nf 1 2 3\nf 2 1 3\n").is_err());
}

#[test]
fn report_counts_uv_problems() {
    let obj = "v 0 0 0\nv 1 0 0\nv 0 1 0\nv 1 1 0\nvt 0 0\nvt 2 0\nvt 0 1\nvt 0 0\nf 1/1 2/2 3/3\nf 2/4 4/4 3/4\n";
    let r = import_mesh_bytes("r.obj", obj.as_bytes()).unwrap().report;
    assert_eq!(r.uv_out_of_range, 1);
    assert_eq!(r.zero_area_uv_triangles, 1);
    assert!(!r.warnings.is_empty());
}

#[test]
fn gltf_vertex_colors_and_node_names() {
    // The embedded scene plus a COLOR_0 (RGBA float) on the first primitive and a named node.
    let mut json: serde_json::Value = serde_json::from_slice(&embedded_gltf()).unwrap();
    let colors: Vec<u8> = [[1.0f32, 0.0, 0.0, 0.5]; 3].iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
    json["buffers"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({ "byteLength": 48, "uri": format!("data:application/octet-stream;base64,{}", base64(&colors)) }));
    json["bufferViews"].as_array_mut().unwrap().push(serde_json::json!({ "buffer": 1, "byteOffset": 0, "byteLength": 48 }));
    json["accessors"].as_array_mut().unwrap().push(serde_json::json!({ "bufferView": 4, "componentType": 5126, "count": 3, "type": "VEC4" }));
    json["meshes"][0]["primitives"][0]["attributes"]["COLOR_0"] = 4.into();
    json["nodes"][1]["name"] = "Body_low".into();
    let m = import_mesh_bytes("scene.gltf", &serde_json::to_vec(&json).unwrap()).unwrap().mesh;
    assert_eq!(m.colors.len(), m.vertex_count());
    let red = Vec4::new(1.0, 0.0, 0.0, 0.5);
    // The colored primitive's vertices are red, the other primitive's white.
    let first = &m.indices[m.submeshes[0].start as usize..][..3];
    assert!(first.iter().all(|i| m.colors[*i as usize] == red), "{:?}", m.colors);
    assert!(m.colors.contains(&Vec4::ONE));
    assert!(m.submeshes.iter().all(|s| s.name == "Body_low"));
    // Unnamed parts take the file stem; a mesh without COLOR_0 has no colors.
    let plain = import_mesh_bytes("scene.glb", &glb(true, true)).unwrap().mesh;
    assert!(plain.submeshes.iter().all(|s| s.name == "scene") && plain.colors.is_empty());
}

#[test]
fn obj_and_ply_vertex_colors_and_objects() {
    let obj = "o Wheel_low\nv 0 0 0 1 0 0\nv 1 0 0 1 0 0\nv 0 1 0 1 0 0\nf 1 2 3\no Body_low\nv 0 0 1 0 1 0\nv 1 0 1 0 1 0\nv 0 1 1 0 1 0\nf 4 5 6\n";
    let m = import_mesh_bytes("car.obj", obj.as_bytes()).unwrap().mesh;
    assert_eq!(m.submeshes.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Wheel_low", "Body_low"]);
    assert_eq!(m.materials.len(), 1, "one material, two parts");
    assert_eq!(m.colors.len(), m.vertex_count());
    for (p, c) in m.positions.iter().zip(&m.colors) {
        assert_eq!(*c, if p.z == 0.0 { Vec4::new(1.0, 0.0, 0.0, 1.0) } else { Vec4::new(0.0, 1.0, 0.0, 1.0) });
    }
    let ply = "ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nproperty uchar red\nproperty uchar green\nproperty uchar blue\n\
               element face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0 255 0 51\n1 0 0 255 0 51\n0 1 0 255 0 51\n3 0 1 2\n";
    let m = import_mesh_bytes("rock_high.ply", ply.as_bytes()).unwrap().mesh;
    assert!(m.colors.iter().all(|c| (*c - Vec4::new(1.0, 0.0, 0.2, 1.0)).length() < 1e-6), "{:?}", m.colors);
    assert_eq!(m.submeshes[0].name, "rock_high");
}

#[test]
fn high_poly_import_skips_unwrap_and_rejects_garbage() {
    let m = import_high_poly_bytes("x.glb", &glb(false, false)).unwrap().mesh;
    m.validate().unwrap();
    assert!(m.uvs.iter().all(|uv| *uv == Vec2::ZERO), "no automatic unwrap");
    assert_eq!(m.tangents.len(), m.vertex_count());
    assert!(import_high_poly_bytes("x.glb", b"glTF\x02\0\0\0garbage").is_err());
    assert!(import_high_poly_bytes("x.obj", b"v 0 0 0\nv nan 1 0\nv 0 1 0\nf 1 2 3\n").is_err());
    assert!(import_high_poly_bytes("x.ply", b"ply\nformat ascii 1.0\nelement vertex 99999999\n").is_err());
    assert!(import_high_poly_bytes("x.txt", b"").is_err());
}

/// The embedded scene plus a perspective camera looking at the triangle along -Z and an unnamed
/// orthographic one looking straight down.
fn gltf_with_cameras() -> Vec<u8> {
    let mut json: serde_json::Value = serde_json::from_slice(&embedded_gltf()).unwrap();
    json["cameras"] = serde_json::json!([
        { "name": "Shot", "type": "perspective", "perspective": { "yfov": 0.8, "znear": 0.1 } },
        { "type": "orthographic", "orthographic": { "xmag": 1.0, "ymag": 0.5, "znear": 0.1, "zfar": 10.0 } }
    ]);
    let nodes = json["nodes"].as_array_mut().unwrap();
    nodes.push(serde_json::json!({ "camera": 0, "translation": [-1.5, 0.5, 4.0] }));
    nodes.push(serde_json::json!({ "camera": 1, "translation": [-1.5, 5.5, 0.0], "rotation": [-std::f32::consts::FRAC_1_SQRT_2, 0.0, 0.0, std::f32::consts::FRAC_1_SQRT_2] }));
    json["scenes"][0]["nodes"] = serde_json::json!([0, 2, 3]);
    serde_json::to_vec(&json).unwrap()
}

#[test]
fn gltf_cameras_become_orbit_cameras() {
    let im = import_mesh_bytes("scene.gltf", &gltf_with_cameras()).unwrap();
    let [shot, top] = im.cameras.as_slice() else { panic!("{:?}", im.cameras) };
    let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
    assert_eq!(shot.name, "Shot");
    assert!(!shot.ortho && near(shot.fov, 0.8f32.to_degrees()) && near(shot.distance, 4.0), "{shot:?}");
    assert!(near(shot.yaw, 0.0) && near(shot.pitch, 0.0) && shot.target.iter().zip([-1.5, 0.5, 0.0]).all(|(a, b)| near(*a, b)), "{shot:?}");
    assert_eq!(top.name, "Camera 2");
    assert!(top.ortho && near(top.distance, 5.0) && near(top.fov, 2.0 * 0.1f32.atan().to_degrees()), "{top:?}");
    assert!(near(top.pitch, 89f32.to_radians()), "looking down, pitch clamped: {top:?}");
    assert!(import_mesh_bytes("scene.gltf", &embedded_gltf()).unwrap().cameras.is_empty());
}

#[test]
fn obj_without_object_name_takes_file_stem() {
    let name = |file: &str, src: &str| import_mesh_bytes(file, src.as_bytes()).unwrap().mesh.submeshes[0].name.clone();
    let body = "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    assert_eq!(name("rock.obj", body), "rock");
    assert_eq!(name("rock.obj", &format!("o Boulder\n{body}")), "Boulder");
    assert_eq!(name("rock.obj", &format!("g Pebble\n{body}")), "Pebble");
}

#[test]
fn ply_face_element_before_vertex_element() {
    let head = "ply\nformat ascii 1.0\nelement face 1\nproperty list uchar int vertex_indices\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nend_header\n";
    let ok = format!("{head}3 0 1 2\n0 0 0\n1 0 0\n0 1 0\n");
    let mesh = import_mesh_bytes("a.ply", ok.as_bytes()).unwrap().mesh;
    assert_eq!(mesh.positions.len(), 3);
    let bad = format!("{head}3 0 1 5\n0 0 0\n1 0 0\n0 1 0\n");
    assert!(import_mesh_bytes("a.ply", bad.as_bytes()).is_err());
}

#[test]
fn group_index_preserves_order_and_reuses_nonadjacent_material_parts() {
    let mut raw = super::RawMesh::default();
    for i in 0..1000 {
        raw.group("shared", &i.to_string()).extend_from_slice(&[0, 1, 2]);
    }
    for i in (0..1000).rev() {
        raw.group("shared", &i.to_string()).extend_from_slice(&[3, 4, 5]);
    }
    raw.group("other", "0").extend_from_slice(&[6, 7, 8]);
    assert_eq!(raw.groups.len(), 1001);
    for (i, (material, part, indices)) in raw.groups.iter().take(1000).enumerate() {
        assert_eq!(material, "shared");
        assert_eq!(part, &i.to_string());
        assert_eq!(indices, &[0, 1, 2, 3, 4, 5]);
    }
    assert_eq!(raw.groups[1000], ("other".into(), "0".into(), vec![6, 7, 8]));
}

#[test]
fn decode_base_color_resizes_oversized_and_rejects_corrupt_bytes() {
    let mut budget = BASE_COLOR_TEXELS;
    assert!(decode_base_color(b"not an image", &mut budget).is_err());

    let side = BASE_COLOR_SIDE + 2;
    let img = craft_image_codecs::Image::from_u8(side, 2, craft_image_codecs::ChannelLayout::Rgba, vec![128; (side * 2 * 4) as usize]).unwrap();
    let png = craft_image_codecs::encode(&img, craft_image_codecs::Format::Png, &Default::default()).unwrap();
    let (w, h, pixels) = decode_base_color(&png, &mut budget).unwrap();
    assert!(w <= BASE_COLOR_SIDE && h <= BASE_COLOR_SIDE);
    assert_eq!(pixels.len(), (w * h) as usize);
}

#[test]
fn base_color_keeps_stored_jpeg_orientation_for_mesh_uvs() {
    let pixels = image::RgbImage::from_fn(4, 2, |x, _| if x < 2 { image::Rgb([255, 0, 0]) } else { image::Rgb([0, 0, 255]) });
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 100).encode_image(&pixels).unwrap();
    // Synthetic TIFF Orientation=6 inside EXIF APP1, not an external image fixture.
    let exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
    let mut tagged = jpeg[..2].to_vec();
    tagged.extend_from_slice(&[0xff, 0xe1]);
    tagged.extend_from_slice(&u16::try_from(exif.len() + 2).unwrap().to_be_bytes());
    tagged.extend_from_slice(exif);
    tagged.extend_from_slice(&jpeg[2..]);
    let expected = image::load_from_memory(&tagged).unwrap().into_rgba8();
    let mut budget = BASE_COLOR_TEXELS;
    let (w, h, actual) = decode_base_color(&tagged, &mut budget).unwrap();
    assert_eq!((w, h), (4, 2));
    assert_eq!(actual, expected.pixels().map(|p| p.0).collect::<Vec<_>>());
}

#[test]
fn base_color_filters_hdr_before_clamping_to_eight_bits() {
    let h = BASE_COLOR_SIDE * 2;
    let samples: Vec<_> = (0..h).map(|y| image::Rgb([if y % 2 == 0 { 4.0 } else { 0.0 }; 3])).collect();
    let mut hdr = Vec::new();
    image::codecs::hdr::HdrEncoder::new(&mut hdr).encode(&samples, 1, h as usize).unwrap();
    let expected = image::load_from_memory(&hdr).unwrap().resize(BASE_COLOR_SIDE, BASE_COLOR_SIDE, image::imageops::FilterType::Triangle).into_rgba8();
    let mut budget = BASE_COLOR_TEXELS;
    let (w, h, actual) = decode_base_color(&hdr, &mut budget).unwrap();
    assert_eq!((w, h), expected.dimensions());
    assert_eq!(actual, expected.pixels().map(|p| p.0).collect::<Vec<_>>());
}

#[test]
fn duplicate_gltf_material_names_keep_first_wins_and_order() {
    let mut raw = RawMesh {
        positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
        base_colors: vec![
            RawBaseColor { material: "Dup".into(), factor: Vec4::new(1.0, 0.0, 0.0, 1.0), image: None },
            RawBaseColor { material: "Dup".into(), factor: Vec4::new(0.0, 0.0, 1.0, 1.0), image: None },
            RawBaseColor { material: "Second".into(), factor: Vec4::new(0.0, 1.0, 0.0, 1.0), image: None },
        ],
        ..RawMesh::default()
    };
    raw.group("Dup", "p1").extend_from_slice(&[0, 1, 2]);
    raw.group("Second", "p2").extend_from_slice(&[0, 1, 2]);
    raw.group("Dup", "p3").extend_from_slice(&[0, 1, 2]);
    let colors = std::mem::take(&mut raw.base_colors);
    let (mesh, _, _) = geometry(raw).unwrap();
    assert_eq!(mesh.materials, vec!["Dup", "Second"]);
    let submesh_materials: Vec<_> = mesh.submeshes.iter().map(|s| (s.name.as_str(), s.material)).collect();
    assert_eq!(submesh_materials, [("p1", 0), ("p3", 0), ("p2", 1)]);
    let mut color_by_material: HashMap<&str, &RawBaseColor> = HashMap::with_capacity(colors.len());
    for color in &colors {
        color_by_material.entry(color.material.as_str()).or_insert(color);
    }
    assert_eq!(color_by_material.get("Dup").unwrap().factor, Vec4::new(1.0, 0.0, 0.0, 1.0));
}

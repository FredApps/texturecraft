//! Bake commands, mesh assets and project persistence

use serde_json::{Value, json};
use texturecraft_engine::core::{BackfaceMode, BakedMap, CurvatureMethod, PositionMode, RayDistribution, RaySettings, SelfOcclusion};
use texturecraft_engine::geom::{Mesh, primitives};
use texturecraft_engine::{Session, execute};

fn plane_session() -> Session {
    let mesh = primitives::builtin("plane").unwrap();
    Session::new_project("t", mesh, "builtin:plane", None, 256).unwrap()
}

/// `mesh` as OBJ text, shifted by `offset_y`, with x clamped to `max_x` and every vertex colored `rgb`.
fn obj(mesh: &Mesh, offset_y: f32, max_x: f32, rgb: [f32; 3]) -> String {
    let mut text = String::from("o Part_high\n");
    for position in &mesh.positions {
        text += &format!("v {} {} {} {} {} {}\n", position.x.min(max_x), position.y + offset_y, position.z, rgb[0], rgb[1], rgb[2]);
    }

    for triangle in mesh.indices.as_chunks::<3>().0 {
        text += &format!("f {} {} {}\n", triangle[0] + 1, triangle[1] + 1, triangle[2] + 1);
    }

    text
}

fn temp_file(name: &str, text: &str) -> String {
    let dir = std::env::temp_dir().join(format!("texturecraft-bake-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path.display().to_string()
}

#[test]
fn high_poly_meshes_drive_the_bake_and_round_trip() {
    let mut session = plane_session();
    execute(&mut session, "bake.settings", &json!({ "ts": "all", "useLowAsHigh": true })).unwrap();

    let low = session.mesh.as_ref().clone();
    let (lo, hi) = low.bounds();
    let midpoint_x = (lo.x + hi.x) * 0.5;

    // A red high poly over the left half of the plane, just above it
    let high = temp_file("hp_high.obj", &obj(&low, 0.002, midpoint_x, [1.0, 0.0, 0.0]));
    let result = execute(&mut session, "bake.highPoly.add", &json!({ "path": high })).unwrap();
    assert_eq!(result["name"], "hp_high.obj");
    assert!(session.doc.project.texture_sets.iter().all(|texture_set| !texture_set.bake.high_poly.use_low_as_high), "adding a high poly bakes from it");
    let list = execute(&mut session, "bake.highPoly.list", &Value::Null).unwrap();
    assert_eq!(list["highPoly"][0]["parts"][0], "Part_high");
    assert!(execute(&mut session, "project.inspect", &Value::Null).unwrap()["bakeMeshes"]["highPoly"].as_array().is_some_and(|a| a.len() == 1));

    execute(&mut session, "bake.run", &json!({ "resolution": 128, "maps": ["opacity"], "dilation": 0 })).unwrap();
    let baked = &session.doc.baked[&0];
    let opacity = &baked.maps[&BakedMap::Opacity];
    let covered = opacity.iter().filter(|v| **v > 0.99).count() as f32 / opacity.len() as f32;
    assert!((covered - 0.5).abs() < 0.08, "the high poly covers half the plane: {covered}");

    // A cage must have the low poly's triangles
    let invalid_cage = temp_file("tri.obj", "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n");
    assert!(execute(&mut session, "bake.cage.set", &json!({ "path": invalid_cage })).is_err());
    let cage = temp_file("cage.obj", &obj(&low, 0.05, f32::MAX, [1.0; 3]));
    execute(&mut session, "bake.cage.set", &json!({ "path": cage })).unwrap();
    execute(&mut session, "bake.settings", &json!({ "maxFrontal": 0.05 })).unwrap();

    // Save and reopen: meshes, cage and settings come back
    let reloaded = Session::from_project_bytes(&session.to_project_bytes().unwrap()).unwrap();
    let names = |session: &Session| session.bake_meshes.high.iter().map(|h| (h.name.clone(), h.mesh.triangle_count())).collect::<Vec<_>>();
    assert_eq!(names(&reloaded), names(&session));
    assert_eq!(reloaded.bake_meshes.cage.as_ref().map(|c| c.name.as_str()), Some("cage.obj"));
    assert_eq!(reloaded.doc.project.texture_sets[0].bake, session.doc.project.texture_sets[0].bake);

    execute(&mut session, "bake.highPoly.remove", &json!({ "name": "hp_high.obj" })).unwrap();
    assert!(session.bake_meshes.high.is_empty());
    execute(&mut session, "bake.cage.clear", &Value::Null).unwrap();
    assert!(session.bake_meshes.cage.is_none());

    // Without high polys the bake falls back to the low poly
    execute(&mut session, "bake.run", &json!({ "resolution": 128, "maps": ["opacity"] })).unwrap();
    assert!(session.doc.baked[&0].maps[&BakedMap::Opacity].iter().all(|v| *v == 1.0));
    let _ = std::fs::remove_dir_all(std::path::Path::new(&high).parent().unwrap());
}

#[test]
fn malformed_high_poly_input_errors_cleanly() {
    let mut session = plane_session();
    execute(&mut session, "bake.settings", &json!({ "ts": "all", "useLowAsHigh": true })).unwrap();
    let before = session.doc.project.clone();

    assert!(session.add_high_poly_bytes("bad.obj", b"this is not a mesh".to_vec()).is_err());
    assert!(session.add_high_poly_bytes("bad.glb", b"glTF\x02\0\0\0\xff\xff\xff\xff".to_vec()).is_err());
    assert!(session.add_high_poly_bytes("bad.xyz", vec![1, 2, 3]).is_err());
    assert!(session.set_cage_bytes("bad.stl", vec![0; 10]).is_err());
    assert!(execute(&mut session, "bake.highPoly.add", &json!({ "path": "/definitely/missing.obj" })).is_err());
    assert!(execute(&mut session, "bake.highPoly.remove", &json!({ "index": 3 })).is_err());
    assert!(execute(&mut session, "bake.highPoly.remove", &Value::Null).is_err());
    assert!(session.bake_meshes.high.is_empty() && session.bake_meshes.cage.is_none());
    assert_eq!(session.doc.project, before, "failed imports preserve project settings");
}

#[test]
fn bake_settings_apply_atomically() {
    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 256).unwrap();
    let result = execute(&mut session, "bake.settings", &json!({ "ts": "all", "antialiasing": 4, "maps": ["bentNormal", "opacity"] })).unwrap();
    assert_eq!(result.as_array().map(Vec::len), Some(2));

    for texture_set in &session.doc.project.texture_sets {
        assert_eq!(texture_set.bake.common.antialiasing, texturecraft_core::BakeAntialiasing::Subsampling4x4);
        assert_eq!(texture_set.bake.maps.enabled().collect::<Vec<_>>(), [BakedMap::BentNormal, BakedMap::Opacity]);
    }

    let before = session.doc.project.clone();

    for bad in [
        json!({ "rays": 4, "antialiasing": 3 }),
        json!({ "maps": ["nope"] }),
        json!({ "ts": "all", "dilation": 8, "maps": ["color_map"] }),
        json!({ "ts": "all", "dilation": 8, "maps": ["world_space_direction"] }),
        json!({ "ts": [0, 9] }),
    ] {
        assert!(execute(&mut session, "bake.settings", &bad).is_err(), "{bad}");
        assert_eq!(session.doc.project, before, "{bad} changed nothing");
    }

    // "selected" = the sets checked for baking
    execute(&mut session, "bake.settings", &json!({ "ts": 1, "enabled": false })).unwrap();
    let result = execute(&mut session, "bake.settings", &json!({ "ts": "selected", "rays": 8 })).unwrap();
    assert_eq!(result[0]["ts"], 0);
    assert_eq!(result.as_array().map(Vec::len), Some(1));

    // The returned parameters set the same settings again (what the UI sends back)
    let mut copy = result[0]["settings"].clone();
    copy["ts"] = 1.into();
    execute(&mut session, "bake.settings", &copy).unwrap();
    let first = &session.doc.project.texture_sets[0].bake;
    let second = &session.doc.project.texture_sets[1].bake;
    assert_eq!(first, second);
}

#[test]
fn current_ui_parameters_preserve_nested_model_settings() {
    let mut session = plane_session();
    let settings = &mut session.doc.project.texture_sets[0].bake;
    settings.common.resolution_height = Some(texturecraft_core::BakeResolution::R1024);
    settings.common.apply_diffusion = false;
    settings.high_poly.ignore_backface_suffix = "_open".into();
    settings.cage.relative_to_bounding_box = false;
    settings.maps.ao.parameters.ground_plane.enabled = true;
    settings.maps.ao.parameters.ground_plane.offset = 3.0;
    settings.maps.ao.parameters.rays.min_distance = 0.02;
    settings.maps.curvature.parameters.mesh.sampling_radius = 0.25;
    settings.maps.curvature.parameters.normal_map.details = 0.75;
    settings.maps.height.parameters.scaling_divisor = 2.0;

    for (name, method) in [("normalMap", CurvatureMethod::NormalMap), ("mesh", CurvatureMethod::Mesh)] {
        let result = execute(&mut session, "bake.settings", &json!({"curvatureMethod": name})).unwrap();
        let expected = session.doc.project.texture_sets[0].bake.clone();
        assert_eq!(expected.maps.curvature.parameters.method, method);
        assert_eq!(result[0]["settings"]["curvatureMethod"], name);

        execute(&mut session, "bake.settings", &result[0]["settings"]).unwrap();
        assert_eq!(session.doc.project.texture_sets[0].bake, expected);

        let reloaded = Session::from_project_bytes(&session.to_project_bytes().unwrap()).unwrap();
        assert_eq!(reloaded.doc.project.texture_sets[0].bake, expected);
    }

    let expected = session.doc.project.clone();
    for method in [json!(true), json!(1), json!("invalid")] {
        assert!(execute(&mut session, "bake.settings", &json!({"dilation": 8, "curvatureMethod": method})).is_err());
        assert_eq!(session.doc.project, expected);
    }
}

#[test]
fn complete_model_edits_round_trip_and_preserve_texture_set_selection() {
    use texturecraft_core::{BakeResolution, HeightNormalization, IdColorGenerator, IdColorSource};

    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 256).unwrap();
    execute(&mut session, "bake.settings", &json!({"ts": 1, "enabled": false})).unwrap();
    let mut model = session.doc.project.texture_sets[0].bake.clone();
    model.common.resolution_height = Some(BakeResolution::R512);
    model.common.apply_diffusion = false;
    model.maps.id.parameters.source = IdColorSource::MeshId;
    model.maps.id.parameters.generator = IdColorGenerator::Grayscale;
    model.maps.ao.parameters.ground_plane.enabled = true;
    model.maps.ao.parameters.ground_plane.offset = 3.25;
    model.maps.height.is_enabled = true;
    model.maps.height.parameters.normalization = HeightNormalization::Manual;
    model.maps.height.parameters.scaling_divisor = 2.125;

    let result = execute(&mut session, "bake.settings", &json!({"ts": "all", "model": model, "preserveEnabled": true})).unwrap();
    assert_eq!(result[0]["model"], json!(model));
    assert!(session.modified);
    assert_eq!(session.doc.project.texture_sets[0].bake, model);
    model.common.is_enabled = false;
    assert_eq!(session.doc.project.texture_sets[1].bake, model);
    let reopened = Session::from_project_bytes(&session.to_project_bytes().unwrap()).unwrap();
    assert_eq!(reopened.doc.project.texture_sets, session.doc.project.texture_sets);
}

#[test]
fn invalid_model_edits_leave_all_texture_sets_unchanged() {
    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 256).unwrap();
    let before = session.doc.project.clone();
    let model = json!(before.texture_sets[0].bake);
    for (path, invalid_value) in [
        ("/common/dilation", json!(129)),
        ("/common/antialiasing", json!(true)),
        ("/cage/max_frontal", json!(0.06)),
        ("/skew/edge_distance", json!(-0.1)),
        ("/maps/ao/parameters/rays/rays", json!(257)),
        ("/maps/ao/parameters/rays/min_distance", json!(0)),
        ("/maps/ao/parameters/rays/max_distance", json!(1.5)),
        ("/maps/curvature/parameters/mesh/sampling_radius", json!(11)),
        ("/maps/curvature/parameters/mesh/value_max", json!(11)),
        ("/maps/height/parameters/normalization", json!(1)),
        ("/maps/height/parameters/scaling_divisor", json!(0)),
    ] {
        let mut invalid = model.clone();
        *invalid.pointer_mut(path).unwrap() = invalid_value;
        assert!(execute(&mut session, "bake.settings", &json!({"ts": "all", "model": invalid})).is_err(), "{path}");
        assert_eq!(session.doc.project, before, "{path}");
    }
}

#[test]
fn per_map_ray_settings_round_trip() {
    let mut session = plane_session();
    let params = json!({
        "ao": { "rays": 48, "maxDistance": 0.5, "spread": 90, "distribution": "uniform", "ignoreBackface": "always", "selfOcclusion": "sameMeshName" },
        "thickness": { "rays": 12, "spread": 45 },
        "bentNormal": { "rays": 6, "maxDistance": 0.1 },
        "positionMode": "boundingSphere",
    });
    let result = execute(&mut session, "bake.settings", &params).unwrap();
    let settings = session.doc.project.texture_sets[0].bake.clone();
    assert_eq!(
        settings.maps.ao.parameters.rays,
        RaySettings { rays: 48, max_distance: 0.5, spread: 90.0, distribution: RayDistribution::Uniform, ..RaySettings::default() }
    );
    assert_eq!(settings.maps.ao.parameters.ignore_backface, BackfaceMode::Always);
    assert_eq!(settings.maps.ao.parameters.self_occlusion, SelfOcclusion::SameMeshName);
    assert_eq!(
        (settings.maps.thickness.parameters.rays.rays, settings.maps.thickness.parameters.rays.spread, settings.maps.thickness.parameters.rays.max_distance),
        (12, 45.0, 0.1)
    );
    assert_eq!(
        (
            settings.maps.bent_normal.parameters.rays.rays,
            settings.maps.bent_normal.parameters.rays.max_distance,
            settings.maps.bent_normal.parameters.rays.spread
        ),
        (6, 0.1, 180.0)
    );
    assert_eq!(settings.maps.position.parameters.normalization, PositionMode::BoundingSphere);
    assert_eq!(result[0]["settings"]["ao"]["selfOcclusion"], "sameMeshName");
    assert_eq!(result[0]["settings"]["thickness"]["distribution"], "cosine");

    // The project file keeps them
    let reloaded = Session::from_project_bytes(&session.to_project_bytes().unwrap()).unwrap();
    assert_eq!(reloaded.doc.project.texture_sets[0].bake, settings);

    // `rays` and `maxDistance` set every ray map; unknown names change nothing
    execute(&mut session, "bake.settings", &json!({ "rays": 16, "maxDistance": 0.2 })).unwrap();
    let settings = session.doc.project.texture_sets[0].bake.clone();
    assert!(
        [settings.maps.ao.parameters.rays, settings.maps.thickness.parameters.rays, settings.maps.bent_normal.parameters.rays]
            .iter()
            .all(|rays| rays.rays == 16 && rays.max_distance == 0.2)
    );
    for bad in [
        json!({ "rays": 8, "ao": { "distribution": "gaussian" } }),
        json!({ "ao": { "selfOcclusion": "never" } }),
        json!({ "thickness": { "distribution": 3, "rays": 2 }, "positionMode": "uv" }),
    ] {
        assert!(execute(&mut session, "bake.settings", &bad).is_err(), "{bad}");
        assert_eq!(session.doc.project.texture_sets[0].bake, settings, "{bad} changed nothing");
    }

    // Forcing the CPU bakes there
    let result = execute(&mut session, "bake.run", &json!({ "resolution": 128, "maps": ["ambientOcclusion"], "cpu": true })).unwrap();
    assert_eq!(result["tracer"], json!(["CPU"]));

    for (mode, expected) in [("never", BackfaceMode::Never), ("always", BackfaceMode::Always), ("byMeshName", BackfaceMode::ByMeshName)] {
        let result = execute(&mut session, "bake.settings", &json!({"ao": {"ignoreBackface": mode}, "positionMode": "none"})).unwrap();
        assert_eq!(session.doc.project.texture_sets[0].bake.maps.ao.parameters.ignore_backface, expected);
        assert_eq!(session.doc.project.texture_sets[0].bake.maps.position.parameters.normalization, PositionMode::None);

        let params = &result[0]["settings"];
        assert_eq!(params["ao"]["ignoreBackface"], mode);
        assert_eq!(params["positionMode"], "none");
        execute(&mut session, "bake.settings", params).unwrap();
        assert_eq!(session.doc.project.texture_sets[0].bake.maps.ao.parameters.ignore_backface, expected);
    }

    let before = session.doc.project.clone();
    for invalid in [json!({"ao": {"ignoreBackface": true}}), json!({"ao": {"ignoreBackface": "invalid"}})] {
        assert!(execute(&mut session, "bake.settings", &invalid).is_err());
        assert_eq!(session.doc.project, before);
    }
}

/// A 4×4 PNG with quadrant alpha values 255, 128, 64 and 32
fn quadrants_png() -> Vec<u8> {
    let quadrants = [[255, 0, 0, 255], [0, 255, 0, 128], [0, 0, 255, 64], [128, 128, 128, 32]];
    let image = image::RgbaImage::from_fn(4, 4, |x, y| image::Rgba(quadrants[(y / 2 * 2 + x / 2) as usize]));
    let mut png = Vec::new();
    image.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    png
}

/// Check the alpha at the center of each baked quadrant
fn assert_opacity_quadrants(session: &Session) {
    let opacity = &session.doc.baked[&0].maps[&BakedMap::Opacity];
    for ((x, y), alpha) in [((32, 32), 255.0), ((96, 32), 128.0), ((32, 96), 64.0), ((96, 96), 32.0)] {
        let actual = opacity[y * 128 + x];
        assert!((actual - alpha / 255.0).abs() < 2e-3, "texel {x},{y}: {actual} vs {alpha}/255");
    }
}

/// A GLB of `mesh` lifted by `dy`: one part `Part_high` whose material's base color is `png`.
fn textured_glb(mesh: &Mesh, dy: f32, png: &[u8]) -> Vec<u8> {
    let mut bin = Vec::new();
    let mut view = |bytes: &[u8]| {
        let offset = bin.len();
        bin.extend_from_slice(bytes);
        bin.resize(bin.len().next_multiple_of(4), 0);
        json!({ "buffer": 0, "byteOffset": offset, "byteLength": bytes.len() })
    };
    let floats = |v: Vec<f32>| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let views = [
        view(&floats(mesh.positions.iter().flat_map(|p| [p.x, p.y + dy, p.z]).collect())),
        view(&floats(mesh.uvs.iter().flat_map(|uv| [uv.x, uv.y]).collect())),
        view(&mesh.indices.iter().flat_map(|i| i.to_le_bytes()).collect::<Vec<u8>>()),
        view(png),
    ];
    let (lo, hi) = mesh.bounds();
    let doc = json!({
        "asset": { "version": "2.0" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "name": "Part_high", "mesh": 0 }],
        "meshes": [{ "primitives": [{ "attributes": { "POSITION": 0, "TEXCOORD_0": 1 }, "indices": 2, "material": 0 }] }],
        "materials": [{ "name": "Painted", "pbrMetallicRoughness": { "baseColorTexture": { "index": 0 } } }],
        "textures": [{ "source": 0 }],
        "images": [{ "bufferView": 3, "mimeType": "image/png" }],
        "buffers": [{ "byteLength": bin.len() }],
        "bufferViews": views,
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": mesh.positions.len(), "type": "VEC3", "min": [lo.x, lo.y + dy, lo.z], "max": [hi.x, hi.y + dy, hi.z] },
            { "bufferView": 1, "componentType": 5126, "count": mesh.positions.len(), "type": "VEC2" },
            { "bufferView": 2, "componentType": 5125, "count": mesh.indices.len(), "type": "SCALAR" }
        ]
    });
    let mut text = serde_json::to_vec(&doc).unwrap();
    text.resize(text.len().next_multiple_of(4), b' ');
    let mut glb = Vec::new();
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&((12 + 8 + text.len() + 8 + bin.len()) as u32).to_le_bytes());
    for (chunk, kind) in [(&text, b"JSON"), (&bin, b"BIN\0")] {
        glb.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
        glb.extend_from_slice(kind);
        glb.extend_from_slice(chunk);
    }
    glb
}

#[test]
fn opacity_samples_a_textured_gltf_high_poly() {
    let mut session = plane_session();
    let low = session.mesh.as_ref().clone();
    let png = quadrants_png();
    session.add_high_poly_bytes("painted.glb", textured_glb(&low, 0.002, &png)).unwrap();
    assert!(session.log.iter().any(|l| l.contains("1 base color textures")), "{:?}", session.log);
    execute(&mut session, "bake.run", &json!({ "resolution": 128, "maps": ["opacity"], "dilation": 0 })).unwrap();
    assert_opacity_quadrants(&session);
    assert!(session.doc.baked[&0].maps[&BakedMap::Opacity].iter().all(|o| *o > 0.0), "the high poly covers the plane");

    // The textures come back with the project
    let reloaded = Session::from_project_bytes(&session.to_project_bytes().unwrap()).unwrap();
    assert_eq!(reloaded.bake_meshes.high[0].base_color, session.bake_meshes.high[0].base_color);
}

/// USDZ opacity uses the UsdPreviewSurface texture alpha
#[test]
fn opacity_samples_a_textured_usdz_high_poly() {
    use texturecraft_engine::formats::{ExportedFile, UsdFormat, usd_bytes};
    let mut session = plane_session();
    let mut high = session.mesh.as_ref().clone();
    for p in &mut high.positions {
        p.y += 0.002;
    }

    let png = quadrants_png();
    let texture = ExportedFile { name: "BaseColor.png".into(), bytes: png, width: 4, height: 4, warnings: vec![] };
    let usdz = usd_bytes(&high, "painted", &[vec![texture]], UsdFormat::Usdz).unwrap().bytes;
    session.add_high_poly_bytes("painted.usdz", usdz).unwrap();
    execute(&mut session, "bake.run", &json!({ "resolution": 128, "maps": ["opacity"], "dilation": 0 })).unwrap();
    assert_opacity_quadrants(&session);
}

/// External glTF and USD textures remain available after saving the project
#[test]
fn opacity_samples_textures_next_to_a_high_poly() {
    use texturecraft_engine::formats::{ExportedFile, UsdFormat, usd_bytes};
    let dir = std::env::temp_dir().join(format!("texturecraft-bake-external-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("tex")).unwrap();
    let low = plane_session().mesh.as_ref().clone();

    // The textured GLB taken apart: JSON, its .bin and the image as files
    let glb = textured_glb(&low, 0.002, &quadrants_png());
    let jlen = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let mut doc: Value = serde_json::from_slice(&glb[20..20 + jlen]).unwrap();
    doc["buffers"][0]["uri"] = "painted.bin".into();
    doc["images"] = json!([{ "uri": "tex/painted%20color.png" }]);
    std::fs::write(dir.join("painted.bin"), &glb[20 + jlen + 8..]).unwrap();
    std::fs::write(dir.join("tex/painted color.png"), quadrants_png()).unwrap();
    std::fs::write(dir.join("painted.gltf"), serde_json::to_vec(&doc).unwrap()).unwrap();

    // The same high poly as a usda with its texture beside it
    let mut high = low.clone();
    for p in &mut high.positions {
        p.y += 0.002;
    }

    let texture = ExportedFile { name: "BaseColor.png".into(), bytes: quadrants_png(), width: 4, height: 4, warnings: vec![] };
    let usda = usd_bytes(&high, "painted", &[vec![texture]], UsdFormat::Usda).unwrap();
    for (name, bytes) in &usda.textures {
        std::fs::write(dir.join(name), bytes).unwrap();
    }
    std::fs::write(dir.join("painted.usda"), &usda.bytes).unwrap();
    for (file, stored) in [("painted.gltf", "painted.gltf"), ("painted.usda", "painted.usdz")] {
        let mut session = plane_session();
        let result = execute(&mut session, "bake.highPoly.add", &json!({ "path": dir.join(file).display().to_string() })).unwrap();
        assert_eq!(result["name"], stored);
        assert!(session.log.iter().any(|l| l.contains("1 base color textures")), "{file}: {:?}", session.log);
        execute(&mut session, "bake.run", &json!({ "resolution": 128, "maps": ["opacity"], "dilation": 0 })).unwrap();
        assert_opacity_quadrants(&session);

        // The project holds the folded file: the textures come back without the folder
        let reloaded = Session::from_project_bytes(&session.to_project_bytes().unwrap()).unwrap();
        assert_eq!(reloaded.bake_meshes.high[0].base_color, session.bake_meshes.high[0].base_color, "{file}");
    }

    // A missing image is named in the log; the high poly still loads, untextured
    std::fs::remove_file(dir.join("tex/painted color.png")).unwrap();
    let mut session = plane_session();
    execute(&mut session, "bake.highPoly.add", &json!({ "path": dir.join("painted.gltf").display().to_string() })).unwrap();
    assert!(session.log.iter().any(|l| l.contains("tex/painted color.png was not found")), "{:?}", session.log);
    execute(&mut session, "bake.run", &json!({ "resolution": 128, "maps": ["opacity"], "dilation": 0 })).unwrap();
    assert!(session.doc.baked[&0].maps[&BakedMap::Opacity].iter().all(|opacity| *opacity == 1.0));

    std::fs::remove_dir_all(&dir).unwrap();
}

/// A plane session whose vertex normals lean, with a high poly plane `dy` above it.
fn leaning_session(lean: bool) -> Session {
    let mut mesh = primitives::builtin("plane").unwrap();
    if lean {
        mesh.normals = vec![texturecraft_engine::geom::glam::Vec3::new(0.6, 0.8, 0.0); mesh.positions.len()];
        mesh.compute_tangents();
    }
    let high_text = obj(&mesh, 0.1, f32::MAX, [1.0; 3]);
    let mut s = Session::new_project("t", mesh, "builtin:plane", None, 256).unwrap();
    // A folder of its own per call: tests run in parallel, and another one removes the shared folder.
    static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let call = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("texturecraft-lean-test-{}-{call}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let high = dir.join("high.obj");
    std::fs::write(&high, high_text).unwrap();
    execute(&mut s, "bake.highPoly.add", &json!({ "path": high.display().to_string() })).unwrap();
    // The mesh is parsed and kept on add, so the file can go.
    let _ = std::fs::remove_dir_all(&dir);
    execute(
        &mut s,
        "bake.settings",
        &json!({ "resolution": 128, "dilation": 0, "maps": ["height"], "maxFrontal": 0.5, "maxRear": 0.2, "averageNormals": true }),
    )
    .unwrap();
    s
}

#[test]
fn the_skew_map_is_saved_with_the_project() {
    let mut s = leaning_session(false);
    execute(&mut s, "bake.skew.set", &json!({ "value": 0.25 })).unwrap();
    let back = Session::from_project_bytes(&s.to_project_bytes().unwrap()).unwrap();
    assert_eq!(back.doc.bake_paint, s.doc.bake_paint);
    // No skew: none comes back.
    execute(&mut s, "bake.skew.clear", &Value::Null).unwrap();
    assert!(Session::from_project_bytes(&s.to_project_bytes().unwrap()).unwrap().doc.bake_paint.is_empty());
    // A map with a non-finite value is skipped with a log line, and the project still opens.
    let mut pf = s.project_file();
    pf.bake_paint.insert((0, texturecraft_core::BakePaintTarget::Skew), texturecraft_formats::BakePaintData { width: 1, height: 1, values: vec![f32::NAN] });
    let opened = Session::from_project_file(pf).unwrap();
    assert!(
        !opened.doc.bake_paint.contains_key(&(0, texturecraft_core::BakePaintTarget::Skew))
            && opened.log.iter().any(|line| line.contains("skipped bake paint map")),
        "{:?}",
        opened.log
    );
}

#[test]
fn painting_skew_widens_a_map_and_bad_strokes_change_nothing() {
    let mut s = leaning_session(true);
    // Painting with no map makes a black one at `size` and puts the dab in it.
    let r = execute(&mut s, "bake.skew.paint", &json!({ "points": [[0.5, 0.5]], "radius": 0.1, "size": 64 })).unwrap();
    assert_eq!(r["skew"]["width"], 64);
    let map = s.doc.bake_paint.get(&(0, texturecraft_core::BakePaintTarget::Skew)).cloned().unwrap();
    assert!(map.values.iter().any(|v| *v > 0.99) && map.values.contains(&0.0));
    let rev = s.revision;
    // Erasing with value 0 brings it back down; the size stays.
    execute(&mut s, "bake.skew.paint", &json!({ "points": [[0.5, 0.5]], "radius": 0.2, "value": 0.0, "hardness": 1.0 })).unwrap();
    let after = s.doc.bake_paint.get(&(0, texturecraft_core::BakePaintTarget::Skew)).cloned().unwrap();
    assert_eq!(after.width, 64);
    assert!(after.values.iter().all(|v| *v < 0.01) && s.revision > rev);
    // A constant map is widened, keeping its value where nothing is painted.
    execute(&mut s, "bake.skew.set", &json!({ "value": 0.5 })).unwrap();
    execute(&mut s, "bake.skew.paint", &json!({ "points": [[0.1, 0.1]], "radius": 0.05, "size": 32 })).unwrap();
    let wide = s.doc.bake_paint.get(&(0, texturecraft_core::BakePaintTarget::Skew)).cloned().unwrap();
    assert_eq!(wide.width, 32);
    assert!((wide.values[wide.values.len() / 2] - 0.5).abs() < 1e-5, "{}", wide.values[wide.values.len() / 2]);
    // Hostile input is an error and leaves the map alone.
    let before = s.doc.bake_paint.get(&(0, texturecraft_core::BakePaintTarget::Skew)).cloned();
    for bad in [
        json!({}),
        json!({ "points": [] }),
        json!({ "points": [[0.5]] }),
        json!({ "points": [["a", "b"]] }),
        json!({ "points": [[0.5, 0.5]], "radius": 0.0 }),
        json!({ "points": [[0.5, 0.5]], "radius": 5.0 }),
        json!({ "points": [[0.5, 0.5]], "radius": -1.0 }),
        json!({ "points": [[0.5, 0.5]], "size": 2 }),
        json!({ "points": [[0.5, 0.5]], "size": 100000 }),
        json!({ "points": [[0.0, 0.0], [1e9, 1e9]], "radius": 1e-4 }),
    ] {
        assert!(execute(&mut s, "bake.skew.paint", &bad).is_err(), "{bad}");
    }
    assert_eq!(s.doc.bake_paint.get(&(0, texturecraft_core::BakePaintTarget::Skew)).cloned(), before);
    // And the painted map is saved with the project.
    let back = Session::from_project_bytes(&s.to_project_bytes().unwrap()).unwrap();
    assert_eq!(back.doc.bake_paint, s.doc.bake_paint);
}

#[test]
fn bake_brush_strokes_are_undoable_and_continuations_merge() {
    use texturecraft_core::BakePaintTarget;
    let mut session = plane_session();
    let key = (0, BakePaintTarget::Skew);
    execute(&mut session, "bake.skew.paint", &json!({"points": [[0.2, 0.2]], "radius": 0.1, "size": 64})).unwrap();
    execute(&mut session, "bake.skew.paint", &json!({"points": [[0.2, 0.2], [0.8, 0.8]], "radius": 0.1, "continue": true})).unwrap();
    let painted = session.doc.bake_paint.get(&key).cloned().unwrap();
    session.undo().unwrap();
    assert!(!session.doc.bake_paint.contains_key(&key));
    session.redo().unwrap();
    assert_eq!(session.doc.bake_paint.get(&key), Some(&painted));
}

#[test]
fn cage_paint_is_scoped_saved_and_undoable() {
    use texturecraft_core::BakePaintTarget;
    let mut session = plane_session();
    let mut second_set = session.doc.project.texture_sets[0].clone();
    second_set.stack.clear();
    session.doc.project.texture_sets.push(second_set);
    execute(&mut session, "bake.cage.paint", &json!({"ts": 1, "points": [[0.25, 0.25]], "radius": 0.1, "value": 0.2, "size": 32})).unwrap();
    let key = (1, BakePaintTarget::Cage);
    let painted = session.doc.bake_paint[&key].clone();
    assert!(painted.values.iter().any(|value| *value < 0.5));
    assert!(painted.values.contains(&1.0));
    assert!(!session.doc.bake_paint.contains_key(&(0, BakePaintTarget::Cage)));
    let reloaded = Session::from_project_bytes(&session.to_project_bytes().unwrap()).unwrap();
    assert_eq!(reloaded.doc.bake_paint[&key], painted);
    execute(&mut session, "edit.undo", &Value::Null).unwrap();
    assert!(!session.doc.bake_paint.contains_key(&key));
    execute(&mut session, "edit.redo", &Value::Null).unwrap();
    assert_eq!(session.doc.bake_paint[&key], painted);
    execute(&mut session, "bake.cage.reset", &json!({"ts": 1})).unwrap();
    assert!(!session.doc.bake_paint.contains_key(&key));
}

#[test]
fn changing_bake_aspect_replaces_incompatible_maps() {
    let mut session = plane_session();
    execute(&mut session, "bake.run", &json!({"resolution": 64, "maps": ["normal", "id"]})).unwrap();
    execute(&mut session, "bake.settings", &json!({"model": {"common": {"resolution": "R64", "resolution_height": "R128"}}})).unwrap();
    execute(&mut session, "bake.run", &json!({"maps": ["id"]})).unwrap();
    let baked = &session.doc.baked[&0];
    assert_eq!((baked.resolution, baked.height(BakedMap::Id)), (64, 128));
    assert_eq!(baked.maps.len(), 1);
    assert_eq!(session.doc.project.texture_sets[0].baked.len(), 1);
    let reloaded = Session::from_project_bytes(&session.to_project_bytes().unwrap()).unwrap();
    assert_eq!(reloaded.doc.baked[&0], *baked);
}

#[test]
fn a_failed_multi_set_bake_restores_settings_pixels_and_history() {
    let mut session = leaning_session(false);
    let mut second = session.doc.project.texture_sets[0].clone();
    second.stack.clear();
    second.bake.cage.kind = texturecraft_core::CageType::CustomFile;
    session.doc.project.texture_sets.push(second);
    let original = session.doc.clone();
    let history = session.history_labels();
    let error = execute(&mut session, "bake.run", &json!({"ts": "all", "resolution": 64, "maps": ["id"]})).unwrap_err();
    assert!(error.to_string().contains("requires a cage"), "{error}");
    assert_eq!(session.doc.project, original.project);
    assert_eq!(session.doc.baked, original.baked);
    assert_eq!(session.history_labels(), history);
}

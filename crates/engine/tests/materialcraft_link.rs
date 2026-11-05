//! MaterialCraft materials (`.mcmat`) as linked fill layers: import, parameters, re-cooks,
//! undo, mesh-map inputs and saving. Cooking through a real `materialcraft-cli` runs only when
//! `MATERIALCRAFT_CLI` points at one.

use std::io::Write;
use std::sync::Arc;

use serde_json::{Value, json};
use texturecraft_engine::compose::BakedMaps;
use texturecraft_engine::core::{BakedMap, Channel, LayerId, LayerKind, NormalFormat, Source};
use texturecraft_engine::formats::mcmat::mesh_map_png;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::materials::Place;
use texturecraft_engine::{Session, execute};

fn session() -> Session {
    let mut s = Session::new_project("t", primitives::sphere(1.0, 32, 16), "builtin:sphere", None, 128).unwrap();
    s.update();
    s
}

/// A `.mcmat` as MaterialCraft writes it: uniform maps, one float input `wear` cooked at `wear`.
fn mcmat(roughness: f32, wear: f32) -> Vec<u8> {
    let gray = |v: f32| mesh_map_png(4, 1, &[v; 16]).unwrap();
    let manifest = json!({
        "format": "craft-material", "version": 1, "name": "Brick wall", "graph": "Brick_wall", "size": [4, 4],
        "maps": [
            {"usage": "baseColor", "file": "maps/baseColor.png", "colorSpace": "srgb"},
            {"usage": "roughness", "file": "maps/roughness.png", "colorSpace": "linear"},
            {"usage": "ambientOcclusion", "file": "maps/ambientOcclusion.png", "colorSpace": "linear"}
        ],
        "parameters": [
            {"id": "wear", "label": "Wear", "default": {"t": "float", "v": 0.5}, "min": 0.0, "max": 1.0, "value": {"t": "float", "v": wear}},
            {"id": "tint", "label": "Tint", "default": {"t": "color", "v": [1.0, 1.0, 1.0, 1.0]}, "value": {"t": "color", "v": [1.0, 1.0, 1.0, 1.0]}},
            {"id": "note", "label": "Note", "default": {"t": "text", "v": ""}, "value": {"t": "text", "v": ""}}
        ],
        "inputs": [
            {"identifier": "curvature", "kind": "gray"},
            {"identifier": "normal", "kind": "color"},
            {"identifier": "worldNormal", "kind": "color"},
            {"identifier": "ambientOcclusion", "kind": "gray"},
            {"identifier": "bentNormal", "kind": "color"}
        ],
        "presets": [{"name": "Worn", "values": {"wear": {"t": "float", "v": 1.0}, "note": {"t": "text", "v": "x"}}}],
        "source": "source.mcraft"
    });
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default();
    for (name, bytes) in [
        ("material.json", manifest.to_string().into_bytes()),
        ("maps/baseColor.png", mesh_map_png(4, 3, &[0.5; 48]).unwrap()),
        ("maps/roughness.png", gray(roughness)),
        ("maps/ambientOcclusion.png", gray(1.0)),
        ("source.mcraft", b"package bytes".to_vec()),
    ] {
        z.start_file(name, o).unwrap();
        z.write_all(&bytes).unwrap();
    }
    z.finish().unwrap().into_inner()
}

fn roughness(s: &mut Session) -> f32 {
    s.update();
    s.caches[0].composite.value(Channel::Roughness, 64, 64)[0]
}

fn link(s: &Session, id: LayerId) -> texturecraft_engine::core::MaterialLink {
    match &s.doc.project.layer(id).unwrap().kind {
        LayerKind::Fill(f) => f.link.as_deref().unwrap().clone(),
        _ => panic!("not a fill"),
    }
}

fn map_name(s: &Session, id: LayerId, c: Channel) -> String {
    match &s.doc.project.layer(id).unwrap().kind {
        LayerKind::Fill(f) => match f.sources.get(&c) {
            Some(Source::Image { name, .. }) => name.clone(),
            other => panic!("{other:?}"),
        },
        _ => panic!("not a fill"),
    }
}

#[test]
fn imports_as_a_linked_fill_with_its_maps() {
    let mut s = session();
    let id = s.add_linked_material_bytes_at(&mcmat(0.25, 0.5), Place::Selection).unwrap();
    assert_eq!(s.selected_layer(), Some(id));
    assert!((roughness(&mut s) - 0.25).abs() < 2e-3);
    // The material's channels join the texture set; text inputs are not editable here.
    assert!(s.doc.project.texture_sets[0].has_channel(Channel::AmbientOcclusion));
    let l = link(&s, id);
    assert_eq!(l.graph, "Brick_wall");
    assert_eq!(l.params.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["wear", "tint"]);
    assert_eq!(l.inputs, ["curvature", "normal", "worldNormal", "ambientOcclusion", "bentNormal"]);
    assert_eq!(s.doc.resources.get(&l.package).map(|b| b.as_slice()), Some(&b"package bytes"[..]));
    assert!(!s.images.contains_key(&l.package), "the package is not decoded as an image");
    // Inspect shows the link; one undo removes everything.
    let inspect = execute(&mut s, "project.inspect", &Value::Null).unwrap().to_string();
    assert!(inspect.contains("Brick_wall"), "{inspect}");
    let n = s.doc.resources.len();
    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert!(s.doc.project.layer(id).is_err() && s.doc.resources.len() < n);
    // Garbage is an error, not a layer.
    assert!(s.add_linked_material_bytes_at(b"PK not really", Place::Selection).is_err());
}

/// An image marked DirectX keeps the mark when a re-cook replaces it.
#[test]
fn recooks_keep_the_normal_format_of_a_map() {
    let mut s = session();
    let id = s.add_linked_material_bytes_at(&mcmat(0.25, 0.5), Place::Selection).unwrap();
    execute(&mut s, "layer.fill.set", &json!({"layer": id.0, "channel": "roughness", "normalFormat": "directX"})).unwrap();
    s.apply_link_cook(id, &mcmat(0.75, 0.5), false).unwrap();
    let LayerKind::Fill(fill) = &s.doc.project.layer(id).unwrap().kind else { panic!("a fill") };
    assert!(matches!(fill.sources.get(&Channel::Roughness), Some(Source::Image { normal_format: NormalFormat::DirectX, .. })));
}

#[test]
fn parameters_are_checked_stored_and_recooked_in_one_undo_step() {
    let mut s = session();
    let id = s.add_linked_material_bytes_at(&mcmat(0.25, 0.5), Place::Selection).unwrap();
    let set = |s: &mut Session, p: Value| execute(s, "material.link.set", &p);
    assert!(set(&mut s, json!({"id": "nope", "value": 1, "cook": false})).is_err());
    assert!(set(&mut s, json!({"id": "wear", "value": "soft", "cook": false})).is_err());
    assert!(set(&mut s, json!({"id": "wear", "value": f64::NAN, "cook": false})).is_err());
    set(&mut s, json!({"id": "tint", "value": "#ff0000", "cook": false})).unwrap();
    assert_eq!(link(&s, id).params[1].value, json!({"t": "color", "v": [1.0, 0.0, 0.0, 1.0]}));
    set(&mut s, json!({"id": "wear", "value": 0.9, "cook": false})).unwrap();
    assert_eq!(link(&s, id).params[0].value, json!({"t": "float", "v": 0.9}));

    // The job carries every value and the texture set's resolution.
    let job = s.link_job(id, &serde_json::Map::new()).unwrap();
    assert_eq!(job.size, 128);
    assert_eq!(job.values["wear"], json!({"t": "float", "v": 0.9}));

    // What the job cooked (here: built by hand) replaces the maps, merged into the edit.
    let old = map_name(&s, id, Channel::Roughness);
    s.apply_link_cook(id, &mcmat(0.75, 0.9), true).unwrap();
    assert!((roughness(&mut s) - 0.75).abs() < 2e-3);
    assert!(!s.doc.resources.contains_key(&old), "old maps are dropped");
    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert!((roughness(&mut s) - 0.25).abs() < 2e-3, "undo restores the earlier maps");
    assert_eq!(link(&s, id).params[0].value, json!({"t": "float", "v": 0.5}));
    execute(&mut s, "edit.redo", &Value::Null).unwrap();
    assert!((roughness(&mut s) - 0.75).abs() < 2e-3);

    // Presets skip inputs the painter can't set; unknown presets are errors.
    assert!(execute(&mut s, "material.link.preset", &json!({"name": "Nope", "cook": false})).is_err());
    execute(&mut s, "material.link.preset", &json!({"name": "Worn", "cook": false})).unwrap();
    assert_eq!(link(&s, id).params[0].value, json!({"t": "float", "v": 1.0}));
}

#[test]
fn duplicates_keep_their_maps_and_mesh_maps_feed_inputs() {
    let mut session = session();
    let id = session.add_linked_material_bytes_at(&mcmat(0.25, 0.5), Place::Selection).unwrap();
    let duplicate = LayerId(execute(&mut session, "layer.duplicate", &json!({"layer": id.0})).unwrap()["layer"].as_u64().unwrap());
    let shared = map_name(&session, duplicate, Channel::Roughness);
    session.apply_link_cook(id, &mcmat(0.75, 0.5), false).unwrap();
    assert!(session.doc.resources.contains_key(&shared), "the duplicate still shows the first cook");

    let maps = [BakedMap::Curvature, BakedMap::Normal, BakedMap::WorldNormal, BakedMap::AmbientOcclusion, BakedMap::BentNormal];
    session.doc.baked.insert(0, BakedMaps { resolution: 4, maps: maps.into_iter().map(|map| (map, Arc::new(vec![0.5f32; 16 * map.components()]))).collect() });
    let job = session.link_job(id, &serde_json::Map::new()).unwrap();
    assert_eq!(job.inputs.iter().map(|(identifier, _)| identifier.as_str()).collect::<Vec<_>>(), maps.map(BakedMap::id));
    assert_eq!(maps.map(BakedMap::id), ["curvature", "normal", "worldNormal", "ambientOcclusion", "bentNormal"]);
}

#[test]
fn projects_save_the_link_and_its_package() {
    let mut s = session();
    let id = s.add_linked_material_bytes_at(&mcmat(0.25, 0.5), Place::Selection).unwrap();
    let back = Session::from_project_bytes(&s.to_project_bytes().unwrap()).unwrap();
    assert_eq!(link(&back, id), link(&s, id));
    assert!(back.doc.resources.contains_key(&link(&s, id).package));
    // Smart materials made from the layer carry the package too.
    let tcmat = s.smart_material_bytes(id, "Bricks").unwrap();
    let mut other = session();
    let copy = other.add_smart_material_bytes(&tcmat).unwrap();
    assert!(other.doc.resources.contains_key(&link(&other, copy).package));
}

/// The whole path through a real MaterialCraft: a package cooks on import, then re-cooks.
#[test]
fn cooks_through_materialcraft_cli_when_available() {
    let Some(cli) = std::env::var_os("MATERIALCRAFT_CLI") else {
        eprintln!("MATERIALCRAFT_CLI is not set: skipping the materialcraft-cli round trip");
        return;
    };
    let dir = std::env::temp_dir().join(format!("tc-mc-link-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pkg = dir.join("bricks.mcraft");
    let ok = std::process::Command::new(&cli).args(["new", "bricks"]).arg(&pkg).args(["--size", "64"]).status().unwrap();
    assert!(ok.success());
    let exposed = dir.join("exposed.mcraft");
    let ok = std::process::Command::new(&cli)
        .arg("run")
        .arg(&pkg)
        .arg("graph.exposed.add")
        .arg(r#"{"id": "wear", "default": 0.5}"#)
        .arg("-o")
        .arg(&exposed)
        .status()
        .unwrap();
    assert!(ok.success());
    let mut s = session();
    let id = s.add_linked_material_bytes_at(&std::fs::read(&exposed).unwrap(), Place::Selection).unwrap();
    assert_eq!(link(&s, id).size, 128, "cooked at the texture set's resolution");
    assert!(link(&s, id).params.iter().any(|p| p.id == "wear"));
    execute(&mut s, "material.link.set", &json!({"layer": id.0, "id": "wear", "value": 0.8})).unwrap();
    assert_eq!(link(&s, id).params.iter().find(|p| p.id == "wear").unwrap().value, json!({"t": "float", "v": 0.8}));
    let out = dir.join("back.mcraft");
    execute(&mut s, "material.link.saveSource", &json!({"layer": id.0, "path": out.to_str().unwrap()})).unwrap();
    assert!(std::fs::metadata(&out).unwrap().len() > 0);
    let _ = std::fs::remove_dir_all(&dir);
}

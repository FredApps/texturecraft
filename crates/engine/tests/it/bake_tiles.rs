use std::sync::Arc;

use serde_json::{Value, json};
use texturecraft_engine::bake::Progress;
use texturecraft_engine::compose::BakedMaps;
use texturecraft_engine::core::BakedMap;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

fn session() -> Session {
    let mut session = Session::new_project("Tile baking", primitives::udim_plane(), "builtin:udim_plane", None, 128).unwrap();
    execute(&mut session, "bake.settings", &json!({"resolution": 32, "maps": ["ambientOcclusion"], "rays": 2})).unwrap();
    session
}

#[test]
fn tile_selection_is_inspectable_saved_and_undoable() {
    let mut session = session();
    let before = session.doc.project.clone();
    let result = execute(&mut session, "bake.tile.set", &json!({"tile": 1002, "enabled": false})).unwrap();
    assert_eq!(result, json!({"ts": 0, "tile": 1002, "enabled": false}));
    let selected = session.doc.project.clone();
    let inspected = execute(&mut session, "textureSet.inspect", &Value::Null).unwrap();
    assert_eq!(inspected["tiles"][0]["bakeEnabled"], true);
    assert_eq!(inspected["tiles"][1]["bakeEnabled"], false);
    let reopened = Session::from_project_bytes(&session.to_project_bytes().unwrap()).unwrap();
    assert_eq!(reopened.doc.project, selected);

    let history = session.history_labels();
    execute(&mut session, "bake.tile.set", &json!({"tile": 1002, "enabled": false})).unwrap();
    assert_eq!(session.history_labels(), history);
    session.undo().unwrap();
    assert_eq!(session.doc.project, before);
    session.redo().unwrap();
    assert_eq!(session.doc.project, selected);

    for params in [json!({}), json!({"tile": 1003, "enabled": false}), json!({"tile": u64::MAX, "enabled": false}), json!({"tile": 1001, "enabled": 1})] {
        assert!(execute(&mut session, "bake.tile.set", &params).is_err());
        assert_eq!(session.doc.project, selected);
        assert_eq!(session.history_labels(), history);
    }
}

#[test]
fn partial_bake_preserves_unchecked_tiles_and_uses_the_workers_selection() {
    let mut session = session();
    session.doc.baked.insert(0, BakedMaps { resolution: 64, maps: [(BakedMap::AmbientOcclusion, Arc::new(vec![0.25; 64 * 32]))].into() });
    execute(&mut session, "bake.tile.set", &json!({"tile": 1002, "enabled": false})).unwrap();
    let mut task = session.bake_task(0).unwrap();
    task.tracer = None;

    // Editing the checkboxes while a bake runs affects the next bake
    execute(&mut session, "bake.tile.set", &json!({"tile": 1002, "enabled": true})).unwrap();
    execute(&mut session, "bake.tile.set", &json!({"tile": 1001, "enabled": false})).unwrap();
    let before = session.doc.baked.clone();
    let output = task.run(&Progress::new()).unwrap();
    session.apply_bake(0, output).unwrap();

    let pixels = &session.doc.baked[&0].maps[&BakedMap::AmbientOcclusion];
    for row in pixels.chunks_exact(64) {
        assert!(row[..32].iter().all(|value| *value > 0.99));
        assert!(row[32..].iter().all(|value| *value == 0.25));
    }
    session.undo().unwrap();
    assert_eq!(session.doc.baked, before);
}

#[test]
fn partial_bake_resizes_retained_tiles_and_unchecked_map_outputs() {
    let mut session = session();
    session.doc.baked.insert(
        0,
        BakedMaps {
            resolution: 64,
            maps: [(BakedMap::AmbientOcclusion, Arc::new(vec![0.25; 64 * 32])), (BakedMap::Normal, Arc::new(vec![0.4; 64 * 32 * 3]))].into(),
        },
    );
    execute(&mut session, "bake.tile.set", &json!({"tile": 1002, "enabled": false})).unwrap();
    let result = execute(&mut session, "bake.run", &json!({"resolution": 64, "cpu": true})).unwrap();
    assert_eq!(result["baked"], json!([0]));

    let baked = &session.doc.baked[&0];
    assert_eq!((baked.resolution, baked.height(BakedMap::AmbientOcclusion)), (128, 64));
    for row in baked.maps[&BakedMap::AmbientOcclusion].chunks_exact(128) {
        assert!(row[..64].iter().all(|value| *value > 0.99));
        assert!(row[64..].iter().all(|value| *value == 0.25));
    }
    assert_eq!(baked.maps[&BakedMap::Normal].len(), 128 * 64 * 3);
    assert!(baked.maps[&BakedMap::Normal].iter().all(|value| *value == 0.4));
    let reopened = Session::from_project_bytes(&session.to_project_bytes().unwrap()).unwrap();
    assert_eq!(reopened.doc.baked, session.doc.baked);
}

#[test]
fn tile_selection_preserves_the_correct_udim_row() {
    let mut mesh = primitives::udim_plane();
    let second_plane = mesh.uvs.len() / 2;
    for uv in mesh.uvs.iter_mut().skip(second_plane) {
        uv.x -= 1.0;
        uv.y -= 1.0;
    }
    let mut session = Session::new_project("Vertical UV tiles", mesh, "builtin:udim_plane", None, 128).unwrap();
    assert_eq!(session.doc.project.texture_sets[0].uv_tiles, [1001, 1011]);
    session.doc.baked.insert(0, BakedMaps { resolution: 32, maps: [(BakedMap::AmbientOcclusion, Arc::new(vec![0.25; 32 * 64]))].into() });
    execute(&mut session, "bake.tile.set", &json!({"tile": 1011, "enabled": false})).unwrap();
    execute(&mut session, "bake.run", &json!({"resolution": 32, "maps": ["ambientOcclusion"], "rays": 2, "cpu": true})).unwrap();

    let pixels = &session.doc.baked[&0].maps[&BakedMap::AmbientOcclusion];
    assert!(pixels[..32 * 32].iter().all(|value| *value == 0.25));
    assert!(pixels[32 * 32..].iter().all(|value| *value > 0.99));
}

#[test]
fn excluded_tiles_without_previous_pixels_use_neutral_values() {
    let mut session = session();
    execute(&mut session, "bake.tile.set", &json!({"tile": 1001, "enabled": false})).unwrap();
    execute(&mut session, "bake.run", &json!({"maps": ["normal", "ambientOcclusion"], "cpu": true})).unwrap();
    let baked = &session.doc.baked[&0];
    for map in [BakedMap::Normal, BakedMap::AmbientOcclusion] {
        for row in baked.maps[&map].chunks_exact(64 * map.components()) {
            for pixel in row[..32 * map.components()].chunks_exact(map.components()) {
                assert_eq!(pixel, &map.neutral()[..map.components()]);
            }
        }
    }
}

#[test]
fn no_selected_tiles_is_a_noop_and_old_projects_include_every_tile() {
    let mut session = session();
    let serialized = serde_json::to_value(&session.doc.project).unwrap();
    assert!(serialized["texture_sets"][0].get("bake_excluded_tiles").is_none());
    let legacy: texturecraft_engine::core::Project = serde_json::from_value(serialized).unwrap();
    assert!(legacy.texture_sets[0].has_enabled_bake_tiles());

    for tile in [1001, 1002] {
        execute(&mut session, "bake.tile.set", &json!({"tile": tile, "enabled": false})).unwrap();
    }
    let before = session.doc.project.clone();
    let history = session.history_labels();
    let result = execute(&mut session, "bake.run", &json!({"ts": "selected", "cpu": true})).unwrap();
    assert_eq!(result["baked"], json!([]));
    assert_eq!(session.doc.project, before);
    assert!(session.doc.baked.is_empty());
    assert_eq!(session.history_labels(), history);
    assert!(session.bake_task(0).is_err());

    execute(&mut session, "textureSet.uvTiles", &json!({"enabled": false})).unwrap();
    assert!(session.doc.project.texture_sets[0].bake_excluded_tiles.is_empty());
    session.undo().unwrap();
    assert_eq!(session.doc.project, before);
}

#[test]
#[ignore]
fn partial_bake_timing() {
    let mut session = session();
    execute(&mut session, "bake.run", &json!({"resolution": 256, "rays": 64, "cpu": true})).unwrap();
    for enabled in [true, false] {
        execute(&mut session, "bake.tile.set", &json!({"tile": 1002, "enabled": enabled})).unwrap();
        let mut task = session.bake_task(0).unwrap();
        task.tracer = None;
        let mut times = Vec::new();
        for _ in 0..5 {
            let started = std::time::Instant::now();
            task.run(&Progress::new()).unwrap();
            times.push(started.elapsed());
        }
        times.sort();
        eprintln!("{} of 2 UV tiles, 256 square, AO 64 rays, CPU: median {:?}, runs {:?}", if enabled { 2 } else { 1 }, times[2], times);
    }
}

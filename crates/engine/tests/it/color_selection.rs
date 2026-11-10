//! Color selection effects: editing their ID colors and picking them from the ID mesh map

use serde_json::{Value, json};
use texturecraft_engine::core::{EffectKind, LayerId};
use texturecraft_engine::geom::glam::Vec2;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

const TOP_LEFT_ID: [u8; 3] = [200, 30, 10];
const TOP_RIGHT_ID: [u8; 3] = [10, 200, 30];
const BOTTOM_ID: [u8; 3] = [30, 10, 200];
const NO_PART: [f32; 3] = [0.0, 0.0, 0.0];

fn session() -> Session {
    let mut session = Session::new_project("t", primitives::sphere(1.0, 48, 24), "builtin:sphere", None, 256).unwrap();
    session.viewport = Vec2::new(400.0, 400.0);
    session.camera.yaw = 0.0;
    session.camera.pitch = 0.0;
    session.update();
    session
}

/// An 8×8 ID map: two parts on the top half, one on the bottom half
fn id_map_png() -> Vec<u8> {
    let image = image::RgbaImage::from_fn(8, 8, |x, y| {
        let [red, green, blue] = match (x < 4, y < 4) {
            (true, true) => TOP_LEFT_ID,
            (false, true) => TOP_RIGHT_ID,
            _ => BOTTOM_ID,
        };
        image::Rgba([red, green, blue, 255])
    });
    let mut bytes = Vec::new();
    image.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    bytes
}

fn stored(id: [u8; 3]) -> [f32; 3] {
    id.map(|channel| channel as f32 / 255.0)
}

fn assert_close(actual: [f32; 3], expected: [f32; 3]) {
    assert!(actual.iter().zip(expected).all(|(channel, wanted)| (channel - wanted).abs() < 1e-3), "{actual:?} vs {expected:?}");
}

/// A fill layer masked by a color selection that selects no part yet: (layer, effect)
fn masked_layer(session: &mut Session, with_id_map: bool) -> (LayerId, LayerId) {
    if with_id_map {
        let image = session.import_resource_bytes("id.png", id_map_png()).unwrap();
        execute(session, "textureSet.meshMap.set", &json!({"map": "id", "image": image})).unwrap();
    }

    let layer = LayerId(execute(session, "layer.add.fill", &json!({"color": [0.9, 0.1, 0.1]})).unwrap()["layer"].as_u64().unwrap());
    execute(session, "mask.add.colorSelection", &json!({"color": NO_PART, "tolerance": 0.05})).unwrap();
    let effect = session.doc.project.layer(layer).unwrap().mask.as_ref().unwrap().effects[0].id;
    (layer, effect)
}

fn colors(session: &Session, effect: LayerId) -> Vec<[f32; 3]> {
    match &session.doc.project.effect(effect).unwrap().1.kind {
        EffectKind::ColorSelection { colors, .. } => colors.clone(),
        other => panic!("not a color selection: {other:?}"),
    }
}

#[test]
fn a_color_picked_on_the_mesh_is_selected_where_it_was_picked() {
    let mut session = session();
    let (layer, effect) = masked_layer(&mut session, true);
    let center = Vec2::new(200.0, 200.0);
    let hit = session.pick(center, session.viewport).unwrap();
    let (width, height) = session.caches[0].size;
    let texel = ((hit.uv.x * width as f32) as u32, (hit.uv.y * height as f32) as u32);
    assert_eq!(session.mask_at(layer, &[texel]).unwrap(), vec![0.0], "nothing selected before the pick");

    let picked = execute(&mut session, "effect.colorSelection.pick", &json!({"effect": effect.0, "x": center.x, "y": center.y})).unwrap();

    assert_eq!(picked["index"], json!(1));
    assert_eq!(colors(&session, effect).len(), 2);
    assert_eq!(session.mask_at(layer, &[texel]).unwrap(), vec![1.0]);
    execute(&mut session, "edit.undo", &Value::Null).unwrap();
    assert_eq!(colors(&session, effect), vec![NO_PART], "one undo step");
}

#[test]
fn a_uv_pick_adds_the_color_there_or_replaces_one() {
    let mut session = session();
    let (_, effect) = masked_layer(&mut session, true);

    execute(&mut session, "effect.colorSelection.pick", &json!({"effect": effect.0, "uv": [0.25, 0.25]})).unwrap();
    assert_close(colors(&session, effect)[1], stored(TOP_LEFT_ID));

    let replaced = execute(&mut session, "effect.colorSelection.pick", &json!({"effect": effect.0, "uv": [0.75, 0.75], "index": 0})).unwrap();
    assert_eq!(replaced["index"], json!(0));
    let picked = colors(&session, effect);
    assert_eq!(picked.len(), 2);
    assert_close(picked[0], stored(BOTTOM_ID));
    execute(&mut session, "edit.undo", &Value::Null).unwrap();
    assert_eq!(colors(&session, effect)[0], NO_PART, "one undo step");
}

#[test]
fn the_colors_param_replaces_the_list_in_one_undo_step() {
    let mut session = session();
    let (_, effect) = masked_layer(&mut session, false);
    let edited = [stored(TOP_LEFT_ID), stored(TOP_RIGHT_ID)];

    execute(&mut session, "effect.set", &json!({"effect": effect.0, "colors": edited, "tolerance": 0.2})).unwrap();
    assert_eq!(colors(&session, effect), edited.to_vec());
    let Some((_, found)) = session.doc.project.effect(effect) else { panic!("effect gone") };
    assert!(matches!(found.kind, EffectKind::ColorSelection { tolerance, .. } if tolerance == 0.2));

    execute(&mut session, "edit.undo", &Value::Null).unwrap();
    assert_eq!(colors(&session, effect), vec![NO_PART]);
}

#[test]
fn bad_colors_are_refused_and_change_nothing() {
    let mut session = session();
    let (_, effect) = masked_layer(&mut session, false);
    let refused = [json!([]), json!([[0.5, 0.5]]), json!([[0.5, "x", 0.5]]), json!([[0.5, null, 0.5]]), json!("red")];

    for colors_param in &refused {
        assert!(execute(&mut session, "effect.set", &json!({"effect": effect.0, "colors": colors_param})).is_err(), "{colors_param}");
    }
    assert_eq!(colors(&session, effect), vec![NO_PART]);
    execute(&mut session, "effect.set", &json!({"effect": effect.0, "colors": [[1e300, 0.5, 0.5]]})).unwrap();
    assert_eq!(colors(&session, effect), vec![[1.0, 0.5, 0.5]], "out-of-range values clamp to 0..1");
}

#[test]
fn picking_explains_what_is_missing() {
    let mut session = session();
    let (layer, effect) = masked_layer(&mut session, false);
    let pick = |session: &mut Session, params: Value| execute(session, "effect.colorSelection.pick", &params).unwrap_err().to_string();

    assert!(pick(&mut session, json!({"effect": effect.0, "uv": [0.5, 0.5]})).contains("no ID map"));
    let image = session.import_resource_bytes("id.png", id_map_png()).unwrap();
    execute(&mut session, "textureSet.meshMap.set", &json!({"map": "id", "image": image})).unwrap();
    assert!(pick(&mut session, json!({"effect": effect.0, "x": 1, "y": 1})).contains("nothing under that point"));
    assert!(pick(&mut session, json!({"effect": effect.0, "uv": [1.5, 0.5]})).contains("outside"));
    assert!(pick(&mut session, json!({"effect": effect.0, "uv": [0.5, 0.5], "index": 7})).contains("no color 7"));
    assert!(pick(&mut session, json!({"effect": effect.0})).contains("`x` and `y`"));

    let fill = execute(&mut session, "effect.add.fill", &json!({"layer": layer.0})).unwrap()["effect"].as_u64().unwrap();
    assert!(pick(&mut session, json!({"effect": fill, "uv": [0.5, 0.5]})).contains("not a color selection"));
    assert_eq!(colors(&session, effect), vec![NO_PART]);
}

#[test]
fn hostile_params_never_crash_the_color_selection_commands() {
    let mut session = session();
    let (_, effect) = masked_layer(&mut session, true);
    let hostile = [
        json!({"effect": effect.0, "x": f32::MAX, "y": -1e30, "viewport": [0, 0]}),
        json!({"effect": effect.0, "uv": [f64::MAX, -1e300], "index": u64::MAX}),
        json!({"effect": effect.0, "uv": [0.5], "colors": [[f64::MAX, -f64::MAX, 0.0]]}),
        json!({"effect": effect.0, "uv": "x", "colors": {"r": 1}}),
        json!({"effect": u64::MAX, "uv": [0.5, 0.5]}),
    ];

    for params in &hostile {
        let _ = execute(&mut session, "effect.colorSelection.pick", params);
        let _ = execute(&mut session, "effect.set", params);
        session.update();
    }
}

#[test]
fn a_hidden_texture_set_does_not_block_a_pick_behind_it() {
    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 256).unwrap();
    session.viewport = Vec2::new(400.0, 400.0);
    session.update();
    let set_named = |name: &str| session.doc.project.texture_sets.iter().position(|set| set.name == name).unwrap();
    let (base, ball) = (set_named("Base"), set_named("Ball"));
    execute(&mut session, "textureSet.select", &json!({"ts": base})).unwrap();
    let (_, effect) = masked_layer(&mut session, true);
    let pick_at = |session: &mut Session, y: f32| execute(session, "effect.colorSelection.pick", &json!({"effect": effect.0, "x": 200, "y": y}));
    let ball_in_front = |session: &Session, y: f32| {
        session.pick(Vec2::new(200.0, y), session.viewport).is_some_and(|hit| session.texture_set_of_material(hit.material) == Some(ball))
    };

    // Down the middle until a pixel where the hidden ball covers the base: the pick reads the base
    execute(&mut session, "textureSet.visible", &json!({"ts": ball, "visible": false})).unwrap();
    let mut covered = None;
    for y in (200..400).map(|y| y as f32) {
        if ball_in_front(&session, y) && pick_at(&mut session, y).is_ok() {
            covered = Some(y);
            break;
        }
    }
    let y = covered.expect("a pixel where the ball covers the base");
    assert_eq!(colors(&session, effect).len(), 2);

    execute(&mut session, "textureSet.visible", &json!({"ts": ball, "visible": true})).unwrap();
    assert!(pick_at(&mut session, y).unwrap_err().to_string().contains("another texture set"), "a shown ball still blocks it");
}

#[test]
fn malformed_replacement_indices_are_rejected_without_editing() {
    let mut session = session();
    let (_, effect) = masked_layer(&mut session, true);
    let history = session.history_labels();

    for index in [json!(-1), json!(0.4), json!("0"), json!(true), json!([])] {
        let result = execute(&mut session, "effect.colorSelection.pick", &json!({"effect": effect.0, "uv": [0.25, 0.25], "index": index}));
        assert!(result.is_err(), "invalid index {index} was accepted: {result:?}");
        assert_eq!(colors(&session, effect), vec![NO_PART]);
        assert_eq!(session.history_labels(), history);
    }
}

#[test]
fn the_color_limit_rejects_additions_but_allows_replacement() {
    let mut session = session();
    let (_, effect) = masked_layer(&mut session, true);
    let full = vec![NO_PART; 256];
    execute(&mut session, "effect.set", &json!({"effect": effect.0, "colors": full})).unwrap();
    let history = session.history_labels();
    let project = session.doc.project.clone();

    for params in [json!({"effect": effect.0, "uv": [0.25, 0.25]}), json!({"effect": effect.0, "uv": [0.25, 0.25], "index": 256})] {
        assert!(execute(&mut session, "effect.colorSelection.pick", &params).is_err());
        assert_eq!(colors(&session, effect), full);
        assert_eq!(session.history_labels(), history);
    }
    assert!(execute(&mut session, "effect.set", &json!({"effect": effect.0, "colors": vec![NO_PART; 257], "name": "Invalid edit"})).is_err());
    assert_eq!(session.history_labels(), history);
    assert_eq!(session.doc.project, project, "invalid colors also roll back edits to other fields");

    execute(&mut session, "effect.colorSelection.pick", &json!({"effect": effect.0, "uv": [0.25, 0.25], "index": 255})).unwrap();
    assert_eq!(colors(&session, effect).len(), 256);
    assert_close(colors(&session, effect)[255], stored(TOP_LEFT_ID));
}

#[test]
fn inspect_exposes_the_selected_colors_and_tolerance() {
    let mut session = session();
    let (layer, effect) = masked_layer(&mut session, true);
    execute(&mut session, "effect.colorSelection.pick", &json!({"effect": effect.0, "uv": [0.25, 0.25]})).unwrap();
    execute(&mut session, "effect.set", &json!({"effect": effect.0, "tolerance": 0.25})).unwrap();

    let inspected = execute(&mut session, "project.inspect", &Value::Null).unwrap();
    let layer = inspected["layers"].as_array().unwrap().iter().find(|found| found["id"] == layer.0).unwrap();
    let found = &layer["mask"]["effects"][0];
    assert_eq!(found["colors"], json!(colors(&session, effect)));
    assert_eq!(found["tolerance"], json!(0.25));
}

#[test]
fn picks_use_the_working_texel_on_a_smaller_uv_tile() {
    let mut session = Session::new_project("UV tiles", primitives::udim_plane(), "builtin:udim_plane", None, 256).unwrap();
    for tile in [1001, 1002] {
        session.import_resource_bytes(&format!("id.{tile}.png"), id_map_png()).unwrap();
    }
    execute(&mut session, "textureSet.meshMap.set", &json!({"map": "id", "image": "id.1001.png"})).unwrap();
    let (layer, effect) = masked_layer(&mut session, false);
    execute(&mut session, "textureSet.tile.resolution", &json!({"tile": 1002, "resolution": 128})).unwrap();
    execute(&mut session, "effect.set", &json!({"effect": effect.0, "tolerance": 0.0})).unwrap();
    let size = session.doc.project.texture_sets[0].size();
    let uv = [0.625, 0.25];
    let texel = ((uv[0] * size.0 as f32) as u32, (uv[1] * size.1 as f32) as u32);
    assert_eq!(session.mask_at(layer, &[texel]).unwrap(), vec![0.0]);

    execute(&mut session, "effect.colorSelection.pick", &json!({"effect": effect.0, "uv": uv})).unwrap();
    assert_eq!(session.mask_at(layer, &[texel]).unwrap(), vec![1.0]);
    let before = colors(&session, effect);
    let result = execute(&mut session, "effect.colorSelection.pick", &json!({"effect": effect.0, "uv": [0.9, 0.8]}));
    assert!(result.unwrap_err().to_string().contains("no ID color"), "unused tile space has no sample");
    assert_eq!(colors(&session, effect), before);
}

#[test]
#[ignore = "release benchmark; run with --ignored --nocapture --test-threads=1"]
fn benchmark_color_selection_edits() {
    use std::time::{Duration, Instant};

    const SAMPLES: u32 = 10;
    for resolution in [256, 1024] {
        let started = Instant::now();
        let mut session = Session::new_project("benchmark", primitives::sphere(1.0, 48, 24), "builtin:sphere", None, resolution).unwrap();
        let (_, effect) = masked_layer(&mut session, true);
        session.update();
        let construction = started.elapsed();

        for count in [1, 256] {
            execute(&mut session, "effect.set", &json!({"effect": effect.0, "colors": vec![NO_PART; count]})).unwrap();
            session.update();
            let params = json!({"effect": effect.0, "uv": [0.25, 0.25], "index": count - 1});
            let (mut command_time, mut composite_time) = (Duration::ZERO, Duration::ZERO);
            for _ in 0..SAMPLES {
                let started = Instant::now();
                execute(&mut session, "effect.colorSelection.pick", &params).unwrap();
                command_time += started.elapsed();
                let started = Instant::now();
                session.update();
                composite_time += started.elapsed();
                execute(&mut session, "edit.undo", &Value::Null).unwrap();
                session.update();
            }
            println!(
                "{resolution}x{resolution}, {count} colors, {SAMPLES} samples: construction {construction:.3?}; pick {:.3?}; composite {:.3?}; session memory {} bytes",
                command_time / SAMPLES,
                composite_time / SAMPLES,
                session.memory().total()
            );
        }
    }
}

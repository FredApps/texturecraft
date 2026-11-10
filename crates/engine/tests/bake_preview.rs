//! Live bake preview: maps stream into the view while a bake runs, the view mode comes back,
//! painting is paused, a cancelled bake leaves the set it was baking alone (sets finished before it
//! stay applied), the streamed pixels are the ones a single bake makes, and the preview and the
//! document share each map's buffer instead of holding two copies.

use serde_json::{Value, json};
use texturecraft_engine::bake::Progress;
use texturecraft_engine::core::{BakeResolution, BakedMap, Channel};
use texturecraft_engine::geom::glam::Vec2;
use texturecraft_engine::{BakeTask, PreviewShow, Session, ViewMode, execute};

const MAPS: [BakedMap; 4] = [BakedMap::Id, BakedMap::AmbientOcclusion, BakedMap::Curvature, BakedMap::Height];

fn session() -> Session {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    for texture_set in &mut session.doc.project.texture_sets {
        texture_set.bake = Default::default();
        texture_set.bake.common.resolution = BakeResolution::R64;
        texture_set.bake.maps.ao.parameters.rays.rays = 8;
        texture_set.bake.maps.select(MAPS);
    }

    session
}

fn task(session: &Session) -> BakeTask {
    let mut task = session.bake_task(0).unwrap();
    task.tracer = None;
    task
}

fn order(session: &Session) -> Vec<BakedMap> {
    task(session).maps()
}

#[test]
fn streamed_maps_are_the_pixels_of_one_bake() {
    let session = session();
    let task = task(&session);
    let whole = task.run(&Progress::new()).unwrap();
    let mut streamed = Vec::new();

    let combined = task.run_streaming(&Progress::new(), &mut |map| streamed.push(map)).unwrap();

    assert_eq!(streamed.iter().map(|map| map.map).collect::<Vec<_>>(), order(&session));
    assert_eq!(streamed.iter().map(|map| (map.index, map.total)).collect::<Vec<_>>(), (0..MAPS.len()).map(|index| (index, MAPS.len())).collect::<Vec<_>>());
    assert_eq!((combined.resolution, combined.height), (whole.resolution, whole.height));
    assert_eq!(combined.maps.keys().collect::<Vec<_>>(), whole.maps.keys().collect::<Vec<_>>());
    for (map, pixels) in &whole.maps {
        assert!(*combined.maps[map] == *pixels, "{map:?} differs between a streamed and a single bake");
    }
    for map in &streamed {
        assert!(*map.data == whole.maps[&map.map], "{:?} handed over differs from the bake", map.map);
    }
}

#[test]
fn the_view_follows_each_map_and_the_previous_mode_comes_back() {
    let mut session = session();
    session.view = ViewMode::Channel(Channel::Roughness);
    let before = session.view;
    let baked_before = session.doc.baked.get(&0).cloned();
    let maps = order(&session);

    session.bake_preview_begin(vec![(0, maps.clone())]);
    session.bake_preview_set_current(0);

    // Nothing is baked yet: the first map shows, neutral
    assert_eq!(session.view, ViewMode::MeshMap(maps[0]));
    assert!(session.bake_preview_baked(0).is_some_and(|baked| baked.maps.is_empty()));

    let task = task(&session);
    let mut shown = Vec::new();
    let mut stages = Vec::new();
    let output = task
        .run_streaming(&Progress::new(), &mut |map| {
            session.bake_preview_map(0, map);
            shown.push(session.view);
            stages.push(session.bake_preview.as_ref().and_then(|preview| preview.stage()).map(|stage| (stage.position, stage.total)));
        })
        .unwrap();

    assert_eq!(shown, maps.iter().map(|map| ViewMode::MeshMap(*map)).collect::<Vec<_>>(), "the view follows the newest map");
    assert_eq!(stages, [Some((2, 4)), Some((3, 4)), Some((4, 4)), None], "the stage names the next map");
    assert_eq!(session.doc.baked.get(&0).cloned(), baked_before, "the document is untouched until the bake is applied");
    let undo_before = session.undo_label().map(str::to_owned);

    session.apply_streamed_bake(0, output).unwrap();
    assert_ne!(session.undo_label().map(str::to_owned), undo_before, "applying the bake is one undo step");
    session.bake_preview_end();

    assert_eq!(session.view, before, "the previous view mode is restored");
    assert!(!session.bake_preview_active());
    assert!(maps.iter().all(|map| session.doc.baked[&0].maps.contains_key(map)));
}

#[test]
fn pinning_material_and_follow() {
    let mut session = session();
    let maps = order(&session);

    session.bake_preview_begin(vec![(0, maps.clone())]);
    session.bake_preview_set_current(0);
    let task = task(&session);
    let mut count = 0;

    task.run_streaming(&Progress::new(), &mut |map| {
        session.bake_preview_map(0, map);
        count += 1;
        match count {
            // Pin the finished map: later maps do not take the view
            1 => session.bake_preview_show(PreviewShow::Pin(maps[0])).unwrap(),
            2 => {
                assert_eq!(session.view, ViewMode::MeshMap(maps[0]));
                session.bake_preview_show(PreviewShow::Material).unwrap();
            }
            3 => {
                assert_eq!(session.view, ViewMode::Material);
                session.bake_preview_show(PreviewShow::Follow).unwrap();
                assert_eq!(session.view, ViewMode::MeshMap(maps[2]));
            }
            _ => {}
        }
    })
    .unwrap();

    // A mesh map picked in the viewport pins it; another mode is what comes back at the end
    execute(&mut session, "viewport.view", &json!({ "mode": "map:height" })).unwrap();
    assert_eq!(session.bake_preview.as_ref().map(|preview| preview.show), Some(PreviewShow::Pin(BakedMap::Height)));
    execute(&mut session, "viewport.view", &json!({ "mode": "uvChecker" })).unwrap();
    assert_eq!(session.view, ViewMode::UvChecker);
    session.bake_preview_end();

    assert_eq!(session.view, ViewMode::UvChecker);
}

#[test]
fn unbaked_sets_show_neutral_and_the_control_command_reports() {
    let mut session = session();
    assert_eq!(execute(&mut session, "bake.preview", &Value::Null).unwrap()["active"], false);
    assert!(execute(&mut session, "bake.preview", &json!({ "show": "material" })).is_err(), "nothing to show when no bake runs");

    session.bake_preview_begin(vec![(0, order(&session)), (1, order(&session))]);
    session.bake_preview_set_current(0);

    assert!(session.bake_preview_baked(1).is_some_and(|baked| baked.maps.is_empty()), "a queued set has nothing yet");
    assert!(session.bake_preview_baked(7).is_none(), "a set outside the bake shows its document maps");
    let reply = execute(&mut session, "bake.preview", &json!({ "show": "map:ambientOcclusion" })).unwrap();

    assert_eq!(
        (reply["active"].clone(), reply["show"].clone(), reply["shown"].clone()),
        (json!(true), json!("map:ambientOcclusion"), json!("ambientOcclusion"))
    );
    assert!(execute(&mut session, "bake.preview", &json!({ "show": "nonsense" })).is_err());
    assert_eq!(texturecraft_engine::inspect::session(&session)["bakePreview"]["active"], true);
}

#[test]
fn painting_is_refused_while_a_bake_shows() {
    let mut session = session();
    session.bake_preview_begin(vec![(0, order(&session))]);

    let error = session.begin_stroke(Vec2::new(10.0, 10.0), Vec2::new(100.0, 100.0), 1.0).unwrap_err().to_string();

    assert_eq!(error, texturecraft_engine::bake_preview::PAINTING_PAUSED);
    assert!(session.begin_stroke_uv(Vec2::new(0.5, 0.5), 1.0).is_err());
    assert!(session.polygon_fill(Vec2::new(10.0, 10.0), Vec2::new(100.0, 100.0)).is_err());
    assert!(!session.stroke_active());
    session.bake_preview_end();
    assert!(session.painting_blocked().is_none());
}

#[test]
fn cancelling_mid_bake_leaves_the_document_and_the_shown_maps_whole() {
    let mut session = session();
    let before = session.doc.baked.get(&0).cloned();
    let maps = order(&session);
    let full = task(&session).run(&Progress::new()).unwrap();
    session.bake_preview_begin(vec![(0, maps.clone())]);
    session.bake_preview_set_current(0);
    let progress = Progress::new();
    let task = task(&session);

    let result = task.run_streaming(&progress, &mut |map| {
        if map.index == 1 {
            progress.cancel();
        }
        session.bake_preview_map(0, map);
    });

    assert!(result.is_err(), "a cancelled bake fails");

    // The two maps that landed are whole and equal to a full bake's; the others are still neutral
    let shown = session.bake_preview_baked(0).unwrap();
    assert_eq!(shown.maps.len(), 2);
    for (map, data) in &shown.maps {
        assert!(**data == full.maps[map], "{map:?}");
    }

    // What the app does on cancel: end the preview; the document keeps what it had
    session.bake_preview_end();

    assert_eq!(session.doc.baked.get(&0).cloned(), before);
    assert!(session.bake_preview_baked(0).is_none());
}

#[test]
fn the_preview_and_the_applied_document_share_each_map_buffer() {
    let mut session = session();
    let maps = order(&session);

    session.bake_preview_begin(vec![(0, maps)]);
    session.bake_preview_set_current(0);
    let mut handed_over = Vec::new();

    let output = task(&session)
        .run_streaming(&Progress::new(), &mut |map| {
            handed_over.push((map.map, map.data.clone()));
            session.bake_preview_map(0, map);
        })
        .unwrap();

    // Sharing the streamed buffers keeps preview memory independent of document ownership
    for (map, data) in &handed_over {
        assert!(std::sync::Arc::ptr_eq(data, &output.maps[map]), "{map:?} was copied on the way out of the bake");
        let shown = &session.bake_preview_baked(0).unwrap().maps[map];
        assert!(std::sync::Arc::ptr_eq(data, shown), "{map:?} is held twice by the preview");
    }

    // Applying the result must preserve sharing with the preview
    session.apply_streamed_bake(0, output).unwrap();
    for (map, data) in &handed_over {
        assert!(std::sync::Arc::ptr_eq(data, &session.doc.baked[&0].maps[map]), "{map:?} was copied into the document");
    }
    session.bake_preview_end();
}

#[test]
fn cancelling_keeps_a_set_that_was_already_applied_as_one_undo_step() {
    let mut session = session();
    let before = session.doc.baked.get(&0).cloned();
    let maps = order(&session);

    // A multi-set bake applies each set when it finishes; a later cancel does not undo it
    session.bake_preview_begin(vec![(0, maps)]);
    session.bake_preview_set_current(0);
    let output = task(&session)
        .run_streaming(&Progress::new(), &mut |map| {
            session.bake_preview_map(0, map);
        })
        .unwrap();
    session.apply_streamed_bake(0, output).unwrap();
    session.bake_preview_end();

    assert!(MAPS.iter().all(|map| session.doc.baked[&0].maps.contains_key(map)), "the finished set stays baked after the cancel");
    session.undo().unwrap();
    assert_eq!(session.doc.baked.get(&0).cloned(), before, "applying the set was one undo step");
}

#[test]
fn a_resized_partial_streamed_bake_keeps_unchecked_maps() {
    use std::sync::Arc;
    use texturecraft_engine::compose::BakedMaps;
    use texturecraft_engine::geom::primitives;

    let mut session = Session::new_project("Partial bake", primitives::udim_plane(), "builtin:udim_plane", None, 128).unwrap();
    session.doc.baked.insert(
        0,
        BakedMaps {
            resolution: 64,
            maps: [
                (BakedMap::AmbientOcclusion, Arc::new(vec![0.25; 64 * 32])),
                (BakedMap::Normal, Arc::new(vec![0.4; 64 * 32 * 3])),
                (BakedMap::Height, Arc::new(vec![0.75; 64 * 32])),
            ]
            .into(),
        },
    );
    execute(&mut session, "bake.settings", &json!({"resolution": 64, "maps": ["ambientOcclusion", "height"], "rays": 2})).unwrap();
    execute(&mut session, "bake.tile.set", &json!({"tile": 1002, "enabled": false})).unwrap();
    let before = session.doc.baked.clone();
    let task = task(&session);
    let whole = task.run(&Progress::new()).unwrap();
    let mut streamed = Vec::new();

    let output = task.run_streaming(&Progress::new(), &mut |map| streamed.push(map)).unwrap();

    assert_eq!((output.resolution, output.height), (128, 64));
    assert_eq!(output.maps.keys().collect::<Vec<_>>(), whole.maps.keys().collect::<Vec<_>>());
    for (map, pixels) in &whole.maps {
        assert_eq!(*output.maps[map], *pixels, "{map:?} must match a whole bake, including retained pixels");
    }
    assert_eq!(streamed.iter().map(|map| map.map).collect::<Vec<_>>(), [BakedMap::AmbientOcclusion, BakedMap::Height]);
    for map in &streamed {
        assert!(Arc::ptr_eq(&map.data, &output.maps[&map.map]));
    }

    session.apply_streamed_bake(0, output).unwrap();

    for (map, pixels) in &whole.maps {
        assert_eq!(*session.doc.baked[&0].maps[map], *pixels);
    }
    session.undo().unwrap();
    assert_eq!(session.doc.baked, before);
}

#[test]
fn box_fills_are_refused_while_a_bake_shows() {
    let mut session = session();
    execute(&mut session, "layer.add.paint", &Value::Null).unwrap();
    execute(&mut session, "tool.polygonFill", &Value::Null).unwrap();
    session.bake_preview_begin(vec![(0, order(&session))]);
    let before = texturecraft_engine::inspect::session(&session);
    let viewport = Vec2::new(100.0, 100.0);

    for result in [
        session.drag_polygon_fill_box_uv(Vec2::ZERO, Vec2::ONE),
        session.drag_polygon_fill_box(Vec2::ZERO, viewport, viewport),
        session.polygon_fill_box_uv(Vec2::ZERO, Vec2::ONE),
        session.polygon_fill_box(Vec2::ZERO, viewport, viewport),
        execute(&mut session, "tool.click", &json!({"uv0": [0, 0], "uv1": [1, 1]})).map(|_| 0),
        execute(&mut session, "tool.click", &json!({"x0": 0, "y0": 0, "x1": 100, "y1": 100})).map(|_| 0),
    ] {
        assert_eq!(result.unwrap_err().to_string(), texturecraft_engine::bake_preview::PAINTING_PAUSED);
    }

    assert_eq!(session.end_polygon_fill_box(), 0);
    assert_eq!(texturecraft_engine::inspect::session(&session), before);
    session.bake_preview_end();
    assert!(session.polygon_fill_box_uv(Vec2::ZERO, Vec2::ONE).unwrap() > 0);
}

#[test]
fn preview_order_matches_each_sets_bake_task() {
    let mut session = session();
    session.doc.project.texture_sets[1].bake.maps.select([BakedMap::WorldNormal, BakedMap::Height]);
    let expected: Vec<_> = [0, 1].into_iter().map(|texture_set| session.bake_task(texture_set).unwrap().maps()).collect();

    session.bake_preview_begin_for(&[0, 1]).unwrap();

    let preview = session.bake_preview.as_ref().unwrap();
    for (texture_set, maps) in expected.iter().enumerate() {
        assert_eq!(&preview.sets[&texture_set].order, maps);
        assert!(preview.sets[&texture_set].baked.maps.is_empty());
    }
    assert_eq!(session.view, ViewMode::MeshMap(expected[0][0]));
}

#[test]
fn invalid_preview_sets_preserve_the_existing_preview_and_view() {
    let mut session = session();
    session.bake_preview_begin_for(&[0]).unwrap();
    session.bake_preview_set_current(0);
    session.bake_preview_show(PreviewShow::Material).unwrap();
    let before = texturecraft_engine::bake_preview::preview_json(&session);
    let view = session.view;
    let revision = session.revision;
    let invalid_set = session.set_count();
    let expected_error = session.doc.project.texture_set(invalid_set).unwrap_err().to_string();

    let error = session.bake_preview_begin_for(&[1, invalid_set]).unwrap_err();

    assert!(error.to_string().contains(&expected_error));
    assert_eq!(texturecraft_engine::bake_preview::preview_json(&session), before);
    assert_eq!(session.view, view);
    assert_eq!(session.revision, revision);
}

#[test]
fn starting_a_bake_preview_discards_an_active_stroke() {
    for (from_settings, paint_mask) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut session = session();
        execute(&mut session, "layer.add.paint", &Value::Null).unwrap();
        if paint_mask {
            session.target = texturecraft_engine::StackTarget::Mask;
        }

        let before = session.doc.clone();
        let history = session.history_labels();
        let view = session.view;
        session.begin_stroke_uv(Vec2::splat(0.5), 1.0).unwrap();
        assert!(session.stroke_tiles() > 0, "the stroke must contain paint before the preview starts");

        if from_settings {
            session.bake_preview_begin_for(&[0]).unwrap();
        } else {
            session.bake_preview_begin(vec![(0, order(&session))]);
        }
        let stroke_survived = session.stroke_active();
        session.continue_stroke_uv(Vec2::splat(0.6), 1.0);
        session.end_stroke().unwrap();

        assert_eq!(session.history_labels(), history, "releasing the pointer during a preview must not commit paint");
        assert!(!stroke_survived, "activation must cancel the stroke immediately");
        assert_eq!(session.doc.project, before.project);
        assert_eq!(session.doc.pixels.layers, before.pixels.layers);
        assert_eq!(session.doc.pixels.masks, before.pixels.masks);
        assert!(session.bake_preview_active());
        session.bake_preview_end();

        assert_eq!(session.view, view);
        session.begin_stroke_uv(Vec2::splat(0.5), 1.0).unwrap();
        session.end_stroke().unwrap();
        assert_eq!(session.history_labels().len(), history.len() + 1, "painting resumes after the preview ends");
    }
}

#[test]
fn a_rejected_bake_preview_preserves_the_active_stroke() {
    let mut session = session();
    execute(&mut session, "layer.add.paint", &Value::Null).unwrap();
    let history_steps = session.history_labels().len();
    let view = session.view;
    session.begin_stroke_uv(Vec2::splat(0.5), 1.0).unwrap();
    let painted_tiles = session.stroke_tiles();
    assert!(painted_tiles > 0);

    let result = session.bake_preview_begin_for(&[0, session.set_count()]);

    assert!(result.is_err());
    assert!(!session.bake_preview_active());
    assert!(session.stroke_active());
    assert_eq!(session.stroke_tiles(), painted_tiles);
    assert_eq!(session.view, view);
    session.end_stroke().unwrap();

    assert_eq!(session.history_labels().len(), history_steps + 1, "a rejected preview must not discard painting");
}

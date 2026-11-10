use serde_json::json;
use texturecraft_engine::core::{BakeResolution, BakeSettings};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

fn session() -> Session {
    let mut session = Session::new_project("Bake settings", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    let second = &mut session.doc.project.texture_sets[1].bake;
    second.common.is_enabled = false;
    second.common.resolution = BakeResolution::R128;
    second.common.resolution_height = Some(BakeResolution::R256);
    second.maps.ao.is_enabled = false;
    second.maps.ao.parameters.rays.min_distance = 0.25;
    second.maps.ao.parameters.rays.rays = 16;
    second.maps.height.parameters.scaling_divisor = 2.0;
    session
}

#[test]
fn nested_settings_patch_preserves_other_fields_and_is_one_undo_step() {
    let mut session = session();
    let before = session.doc.project.clone();
    let history_steps = session.history_labels().len();
    let patch = json!({"maps": {"ao": {"parameters": {"rays": {"rays": 128}}}}});

    let result = execute(&mut session, "bake.settings", &json!({"ts": "all", "patch": patch})).unwrap();

    let mut expected = before.clone();
    for texture_set in &mut expected.texture_sets {
        texture_set.bake.maps.ao.parameters.rays.rays = 128;
    }
    assert_eq!(session.doc.project, expected);
    assert_eq!(result.as_array().unwrap().len(), 2);
    assert_eq!(result[1]["model"], json!(expected.texture_sets[1].bake));
    assert_eq!(session.history_labels().len(), history_steps + 1);
    let reopened = Session::from_project_bytes(&session.to_project_bytes().unwrap()).unwrap();
    assert_eq!(reopened.doc.project.texture_sets, expected.texture_sets);

    session.undo().unwrap();
    assert_eq!(session.doc.project, before);
    session.redo().unwrap();
    assert_eq!(session.doc.project, expected);
}

#[test]
fn settings_patch_validates_every_target_before_changing_any() {
    let mut session = session();
    let before = session.doc.project.clone();
    let history = session.history_labels();

    // This maximum is valid for the active set but below the second set's minimum
    let patch = json!({"maps": {"ao": {"parameters": {"rays": {"max_distance": 0.1}}}}});
    let error = execute(&mut session, "bake.settings", &json!({"ts": "all", "patch": patch})).unwrap_err();
    assert!(error.to_string().contains("minimum occluder distance"), "{error}");
    assert_eq!(session.doc.project, before);
    assert_eq!(session.history_labels(), history);
}

#[test]
fn settings_patch_rejects_unknown_fields_and_invalid_shapes() {
    let mut session = session();
    let before = session.doc.project.clone();
    let history = session.history_labels();

    for patch in [
        json!(null),
        json!([]),
        json!({"unknown": 1}),
        json!({"common": null}),
        json!({"common": {"resolution": "invalid"}}),
        json!({"common": {"antialiasing": true}}),
        json!({"maps": {"ao": {"parameters": {"rays": {"rays": 257}}}}}),
        json!({"maps": {"ao": {"parameters": {"rays": {"rayz": 128}}}}}),
    ] {
        assert!(execute(&mut session, "bake.settings", &json!({"ts": "all", "patch": patch})).is_err(), "{patch}");
        assert_eq!(session.doc.project, before);
        assert_eq!(session.history_labels(), history);
    }
}

#[test]
fn settings_patch_can_clear_optional_height_and_keeps_target_selection() {
    let mut session = session();
    let first = session.doc.project.texture_sets[0].bake.clone();
    let mut second = session.doc.project.texture_sets[1].bake.clone();
    let patch = json!({"common": {"resolution_height": null}});

    execute(&mut session, "bake.settings", &json!({"ts": [1], "patch": patch})).unwrap();

    second.common.resolution_height = None;
    assert_eq!(session.doc.project.texture_sets[0].bake, first);
    assert_eq!(session.doc.project.texture_sets[1].bake, second);

    let history_steps = session.history_labels().len();
    execute(&mut session, "bake.settings", &json!({"ts": "all", "patch": {}})).unwrap();
    execute(&mut session, "bake.settings", &json!({"ts": 1, "patch": patch})).unwrap();
    assert_eq!(session.history_labels().len(), history_steps);
}

#[test]
fn settings_patch_can_reset_one_section_without_replacing_other_settings() {
    let mut session = session();
    let mut expected = session.doc.project.clone();
    let defaults = BakeSettings::default().maps.ao.parameters;
    for texture_set in &mut expected.texture_sets {
        texture_set.bake.maps.ao.parameters = defaults;
    }

    execute(&mut session, "bake.settings", &json!({"ts": "all", "patch": {"maps": {"ao": {"parameters": defaults}}}})).unwrap();

    assert_eq!(session.doc.project, expected);
}

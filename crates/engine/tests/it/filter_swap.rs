//! Swapping a filter effect's type in place through `effect.set {filter}`

use serde_json::{Value, json};
use texturecraft_engine::core::{BlendMode, Effect, EffectKind, Filter, LayerId};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, StackTarget, execute};

/// A sphere project with its default layer selected
fn session() -> Session {
    Session::new_project("t", primitives::sphere(1.0, 16, 8), "builtin:sphere", None, 256).unwrap()
}

/// Runs effect-adding `command` and returns the new effect, which it selects
fn add(s: &mut Session, command: &str, params: Value) -> LayerId {
    execute(s, command, &params).unwrap();
    let StackTarget::Effect(added) = s.target else { panic!("adding an effect selects it: {:?}", s.target) };

    added
}

/// A copy of effect `id`
fn effect(s: &Session, id: LayerId) -> Effect {
    s.doc.project.effect(id).unwrap().1.clone()
}

/// Ids of the selected layer's content effects, bottom first
fn content_order(s: &Session) -> Vec<LayerId> {
    let layer = s.selected_layer().unwrap();
    s.doc.project.layer(layer).unwrap().content_effects.iter().map(|effect| effect.id).collect()
}

#[test]
fn swapping_a_filter_keeps_its_place_blend_opacity_and_visibility() {
    let mut s = session();
    let below = add(&mut s, "effect.add.filter", json!({"filter": "invert", "target": "content"}));
    let swapped = add(&mut s, "effect.add.filter", json!({"filter": "invert", "target": "content"}));
    let above = add(&mut s, "effect.add.filter", json!({"filter": "grayscale", "target": "content"}));
    execute(&mut s, "effect.set", &json!({"effect": swapped.0, "blend": "Multiply", "opacity": 0.4, "visible": false})).unwrap();

    execute(&mut s, "effect.set", &json!({"effect": swapped.0, "filter": "blur", "radius": 5.0})).unwrap();

    let blur = effect(&s, swapped);
    assert!(matches!(blur.kind, EffectKind::Filter(Filter::Blur { radius }) if radius == 5.0), "{:?}", blur.kind);
    assert_eq!(blur.name, "Blur", "a default name follows the new filter");
    assert_eq!((blur.blend, blur.opacity, blur.visible), (BlendMode::Multiply, 0.4, false));
    assert_eq!(content_order(&s), vec![below, swapped, above]);
}

#[test]
fn swapping_a_filter_keeps_a_name_the_user_chose() {
    let mut s = session();
    let id = add(&mut s, "effect.add.filter", json!({"filter": "invert", "target": "content", "name": "Flip"}));

    execute(&mut s, "effect.set", &json!({"effect": id.0, "filter": "threshold"})).unwrap();

    assert_eq!(effect(&s, id).name, "Flip");
}

#[test]
fn one_undo_restores_the_old_filter() {
    let mut s = session();
    let id = add(&mut s, "effect.add.filter", json!({"filter": "threshold", "target": "content", "level": 0.3}));

    execute(&mut s, "effect.set", &json!({"effect": id.0, "filter": "sharpen"})).unwrap();
    execute(&mut s, "edit.undo", &Value::Null).unwrap();

    let restored = effect(&s, id);
    assert!(matches!(restored.kind, EffectKind::Filter(Filter::Threshold { level }) if level == 0.3), "{:?}", restored.kind);
    assert_eq!(restored.name, "Threshold");
}

#[test]
fn a_mask_refuses_a_content_only_filter() {
    let mut s = session();
    let id = add(&mut s, "effect.add.filter", json!({"filter": "invert", "target": "mask"}));

    let err = execute(&mut s, "effect.set", &json!({"effect": id.0, "filter": "grayscale"})).unwrap_err();

    assert!(err.to_string().contains("not on masks"), "{err}");
    assert!(matches!(effect(&s, id).kind, EffectKind::Filter(Filter::Invert)));
}

#[test]
fn a_mask_filter_swaps_in_place_on_the_mask() {
    let mut s = session();
    let below = add(&mut s, "effect.add.filter", json!({"filter": "invert", "target": "mask"}));
    let swapped = add(&mut s, "effect.add.filter", json!({"filter": "invert", "target": "mask"}));
    let above = add(&mut s, "effect.add.filter", json!({"filter": "invert", "target": "mask"}));
    execute(&mut s, "effect.set", &json!({"effect": swapped.0, "blend": "Multiply", "opacity": 0.4})).unwrap();

    execute(&mut s, "effect.set", &json!({"effect": swapped.0, "filter": "blur"})).unwrap();

    let blur = effect(&s, swapped);
    assert!(matches!(blur.kind, EffectKind::Filter(Filter::Blur { .. })), "{:?}", blur.kind);
    assert_eq!((blur.name.as_str(), blur.blend, blur.opacity), ("Blur", BlendMode::Multiply, 0.4));
    let layer = s.selected_layer().unwrap();
    let mask_order: Vec<LayerId> = s.doc.project.layer(layer).unwrap().mask.as_ref().unwrap().effects.iter().map(|effect| effect.id).collect();
    assert_eq!(mask_order, vec![below, swapped, above]);
}

#[test]
fn naming_the_current_filter_edits_it_in_place() {
    let mut s = session();
    let id = add(&mut s, "effect.add.filter", json!({"filter": "blur", "target": "content", "radius": 5.0}));

    execute(&mut s, "effect.set", &json!({"effect": id.0, "filter": "blur", "radius": 9.0})).unwrap();
    execute(&mut s, "effect.set", &json!({"effect": id.0, "filter": "blur"})).unwrap();

    let blur = effect(&s, id);
    assert!(matches!(blur.kind, EffectKind::Filter(Filter::Blur { radius }) if radius == 9.0), "the radius is kept, not reset: {:?}", blur.kind);
}

#[test]
fn a_name_given_with_the_swap_wins() {
    let mut s = session();
    let id = add(&mut s, "effect.add.filter", json!({"filter": "invert", "target": "content"}));

    execute(&mut s, "effect.set", &json!({"effect": id.0, "filter": "threshold", "name": "Edges"})).unwrap();

    let swapped = effect(&s, id);
    assert!(matches!(swapped.kind, EffectKind::Filter(Filter::Threshold { .. })), "{:?}", swapped.kind);
    assert_eq!(swapped.name, "Edges");
}

#[test]
fn a_refused_swap_changes_nothing_and_adds_no_undo_step() {
    let mut s = session();
    let id = add(&mut s, "effect.add.filter", json!({"filter": "invert", "target": "mask"}));
    execute(&mut s, "effect.set", &json!({"effect": id.0, "blend": "Multiply", "opacity": 0.4, "name": "Flip"})).unwrap();
    let before = effect(&s, id);

    for refused in [
        json!({"effect": id.0, "filter": "grayscale"}),
        json!({"effect": id.0, "filter": "nope"}),
        json!({"effect": id.0, "filter": "levels"}),
        json!({"effect": id.0, "filter": "colorBalance", "shadows": "not a color"}),
    ] {
        assert!(execute(&mut s, "effect.set", &refused).is_err(), "{refused}");
    }

    assert_eq!(effect(&s, id), before);
    // The last step is still the blend, opacity and name edit
    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert_eq!(effect(&s, id).name, "Invert");
}

#[test]
fn only_filter_effects_swap_and_only_to_filters() {
    let mut s = session();
    let levels = add(&mut s, "effect.add.levels", json!({"target": "content"}));
    let filter = add(&mut s, "effect.add.filter", json!({"filter": "invert", "target": "content"}));

    assert!(execute(&mut s, "effect.set", &json!({"effect": levels.0, "filter": "blur"})).is_err(), "levels is not a filter effect");
    assert!(execute(&mut s, "effect.set", &json!({"effect": levels.0, "filter": "nope"})).is_err(), "an unknown filter on levels");
    assert!(execute(&mut s, "effect.set", &json!({"effect": filter.0, "filter": "matFinishGrind"})).is_err(), "a material finish is a fill");
    assert!(execute(&mut s, "effect.set", &json!({"effect": filter.0, "filter": "nope"})).is_err());
    assert!(matches!(effect(&s, filter).kind, EffectKind::Filter(Filter::Invert)));
}

#[test]
fn invalid_swap_parameters_report_effect_set_and_preserve_history() {
    let mut s = session();
    let id = add(&mut s, "effect.add.filter", json!({"filter": "invert", "target": "content"}));
    let before = effect(&s, id);
    let history = s.history_labels();

    let error = execute(&mut s, "effect.set", &json!({"effect": id.0, "filter": "colorBalance", "shadows": "invalid", "name": "Changed"})).unwrap_err();

    assert!(matches!(error, texturecraft_engine::EngineError::BadParams { ref cmd, .. } if cmd == "effect.set"), "{error}");
    assert_eq!(effect(&s, id), before);
    assert_eq!(s.history_labels(), history);
}

//! Painting while a filter is selected (it is selected right after the shelf adds it): the
//! stroke paints what lies beneath the filter, so the new paint goes through it.

use serde_json::{Value, json};
use texturecraft_engine::core::{Channel, EffectKind, LayerId};
use texturecraft_engine::geom::glam::Vec2;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, StackTarget, execute};

const RES: f32 = 256.0;

fn session() -> Session {
    let mut s = Session::new_project("t", primitives::sphere(1.0, 48, 24), "builtin:sphere", None, 256).unwrap();
    s.viewport = Vec2::new(400.0, 400.0);
    s.camera.yaw = 0.0;
    s.camera.pitch = 0.0;
    s.update();
    s
}

/// Base color of the composite under a viewport point.
fn color_at(s: &mut Session, x: f32, y: f32) -> [f32; 3] {
    s.update();
    let hit = s.pick(Vec2::new(x, y), s.viewport).unwrap();
    s.caches[0].composite.value(Channel::BaseColor, (hit.uv.x * RES) as u32, (hit.uv.y * RES) as u32)
}

fn brush(s: &mut Session, color: [f32; 3]) {
    execute(s, "tool.paint", &Value::Null).unwrap();
    execute(s, "tool.material", &json!({"channel": "baseColor", "color": color})).unwrap();
    execute(s, "tool.brush", &json!({"size": 20, "hardness": 90})).unwrap();
}

fn dab(s: &mut Session) -> Result<Value, String> {
    execute(s, "tool.stroke", &json!({"points": [[200, 200]]})).map_err(|e| e.to_string())
}

fn add_filter(s: &mut Session, filter: &str, target: &str) -> LayerId {
    execute(s, "effect.add.filter", &json!({"filter": filter, "target": target})).unwrap();
    let StackTarget::Effect(e) = s.target else { panic!("adding a filter selects it: {:?}", s.target) };
    e
}

fn kinds(effects: &[texturecraft_engine::core::Effect]) -> Vec<&'static str> {
    effects.iter().map(|e| if matches!(e.kind, EffectKind::Paint) { "paint" } else { "other" }).collect()
}

#[test]
fn a_selected_content_filter_paints_its_paint_layer_through_it() {
    let mut s = session();
    let layer = s.selected_layer().unwrap();
    add_filter(&mut s, "invert", "content");
    brush(&mut s, [1.0, 0.0, 0.0]);
    dab(&mut s).expect("painting with the filter selected works");
    let v = color_at(&mut s, 200.0, 200.0);
    assert!(v[0] < 0.15 && v[1] > 0.85 && v[2] > 0.85, "red paint shows inverted (cyan): {v:?}");
    assert!(s.doc.pixels.layers.keys().any(|(id, _)| *id == layer), "the paint landed in the layer's own pixels");
    // One undo step removes the stroke and leaves the filter.
    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert!(!s.doc.pixels.layers.iter().any(|((id, _), img)| *id == layer && img.allocated_count() > 0));
    assert_eq!(s.doc.project.layer(layer).unwrap().content_effects.len(), 1);
}

#[test]
fn a_selected_content_filter_paints_the_paint_effect_below_it() {
    let mut s = session();
    let layer = s.selected_layer().unwrap();
    execute(&mut s, "effect.add.paint", &json!({"target": "content"})).unwrap();
    let StackTarget::Effect(paint) = s.target else { panic!("{:?}", s.target) };
    add_filter(&mut s, "invert", "content");
    brush(&mut s, [1.0, 0.0, 0.0]);
    dab(&mut s).unwrap();
    assert!(s.doc.pixels.layers.iter().any(|((id, _), img)| *id == paint && img.allocated_count() > 0), "painted into the paint effect");
    assert!(!s.doc.pixels.layers.iter().any(|((id, _), img)| *id == layer && img.allocated_count() > 0), "not into the layer below it");
    let v = color_at(&mut s, 200.0, 200.0);
    assert!(v[0] < 0.15 && v[1] > 0.85, "through the filter: {v:?}");
}

#[test]
fn a_selected_filter_on_a_fill_layer_explains_why_it_cannot_paint() {
    let mut s = session();
    execute(&mut s, "layer.add.fill", &json!({"color": [0.2, 0.4, 0.8]})).unwrap();
    add_filter(&mut s, "invert", "content");
    brush(&mut s, [1.0, 0.0, 0.0]);
    let e = dab(&mut s).unwrap_err();
    assert!(e.contains("fill layer"), "{e}");
}

#[test]
fn a_selected_mask_filter_paints_a_mask_paint_effect_below_it() {
    let mut s = session();
    execute(&mut s, "layer.add.fill", &json!({"color": [1.0, 0.0, 0.0]})).unwrap();
    let layer = s.selected_layer().unwrap();
    execute(&mut s, "mask.add.black", &Value::Null).unwrap();
    add_filter(&mut s, "invert", "mask");
    let mask = |s: &Session| s.doc.project.layer(layer).unwrap().mask.clone().unwrap().effects;
    assert_eq!(kinds(&mask(&s)), ["other"]);
    // Black mask, inverted: the red fill shows everywhere.
    assert!(color_at(&mut s, 200.0, 200.0)[0] > 0.85);
    brush(&mut s, [1.0, 1.0, 1.0]);
    s.tool.mask_value = 1.0;
    dab(&mut s).expect("painting the mask with its filter selected works");
    assert_eq!(kinds(&mask(&s)), ["paint", "other"], "a paint effect was inserted below the filter");
    // White paint goes through the invert: the fill disappears where it was painted only.
    let painted = color_at(&mut s, 200.0, 200.0);
    let elsewhere = color_at(&mut s, 120.0, 200.0);
    assert!(painted[0] < 0.6 && elsewhere[0] > 0.85, "painted {painted:?}, elsewhere {elsewhere:?}");
    // The next stroke reuses that paint effect.
    dab(&mut s).unwrap();
    assert_eq!(kinds(&mask(&s)), ["paint", "other"]);
}

//! Instance layers across texture sets, mask with height, change-driven invalidation and the
//! frame-budgeted compositor, through engine commands.

use serde_json::{Value, json};
use texturecraft_engine::core::{Channel, LayerId, LayerKind};
use texturecraft_engine::geom::glam::Vec2;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

/// Two texture sets (ball and base) at 256².
fn session() -> Session {
    let mut s = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 256).unwrap();
    s.viewport = Vec2::new(400.0, 400.0);
    s.update();
    s
}

fn id(v: Value, key: &str) -> LayerId {
    LayerId(v[key].as_u64().unwrap())
}

/// A texel covered by texture set `ts`.
fn covered(s: &Session, ts: usize) -> (u32, u32) {
    let g = &s.caches[ts].gbuf;
    let i = g.tri.iter().position(|t| *t != texturecraft_engine::geom::NO_TRI).unwrap();
    (i as u32 % g.width, i as u32 / g.width)
}

fn rough(s: &mut Session, ts: usize) -> f32 {
    s.update();
    let (x, y) = covered(s, ts);
    s.caches[ts].composite.value(Channel::Roughness, x, y)[0]
}

/// Texels of set `ts` whose base color is red.
fn reds(s: &Session, ts: usize) -> usize {
    let c = &s.caches[ts].composite;
    (0..256)
        .flat_map(|y| (0..256).map(move |x| (x, y)))
        .filter(|&(x, y)| s.caches[ts].gbuf.tri[(y * 256 + x) as usize] != u32::MAX)
        .filter(|&(x, y)| {
            let v = c.value(Channel::BaseColor, x, y);
            v[0] > 0.8 && v[1] < 0.2
        })
        .count()
}

#[test]
fn instances_follow_their_source_across_texture_sets() {
    let mut s = session();
    let src = id(execute(&mut s, "layer.add.fill", &json!({"roughness": 0.8})).unwrap(), "layer");
    let inst = id(execute(&mut s, "layer.instance", &json!({"layer": src.0, "ts": 1})).unwrap(), "layer");
    assert!(matches!(s.doc.project.layer(inst).unwrap().kind, LayerKind::Instance { source } if source == src));
    assert_eq!(s.doc.project.locate(inst).map(|l| l.0), Some(1));
    assert!((rough(&mut s, 1) - 0.8).abs() < 1e-3);
    // Editing the source (directly or through the instance) updates the instance's set.
    execute(&mut s, "layer.fill.set", &json!({"layer": src.0, "channel": "roughness", "value": 0.3})).unwrap();
    assert!(!s.caches[1].dirty.is_empty(), "the instance's set is invalidated");
    assert!((rough(&mut s, 1) - 0.3).abs() < 1e-3);
    execute(&mut s, "layer.fill.set", &json!({"layer": inst.0, "channel": "roughness", "value": 0.6})).unwrap();
    assert!((rough(&mut s, 0) - 0.6).abs() < 1e-3 && (rough(&mut s, 1) - 0.6).abs() < 1e-3);
    execute(&mut s, "layer.blend", &json!({"layer": inst.0, "channel": "roughness", "opacity": 0.5})).unwrap();
    assert_eq!(s.doc.project.layer(src).unwrap().blend(Channel::Roughness).unwrap().opacity, 0.5, "blending is linked");
    // Inspect shows the link; an instance of an instance links to the source.
    let stack = execute(&mut s, "layer.list", &json!({"ts": 1})).unwrap();
    assert_eq!(stack[0]["instanceOf"]["layer"], src.0, "{stack}");
    let again = id(execute(&mut s, "layer.instance", &json!({"layer": inst.0, "ts": 1})).unwrap(), "layer");
    assert_eq!(s.doc.project.layer(again).unwrap().instance_source(), Some(src));
    // Save/load keeps instances; undo removes them.
    let back = Session::from_project_bytes(&s.to_project_bytes().unwrap()).unwrap();
    assert_eq!(back.doc.project, s.doc.project);
    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert!(s.doc.project.layer(again).is_err());
}

#[test]
fn painting_a_source_paints_its_instances() {
    let mut s = session();
    let paint = s.selected_layer().unwrap();
    execute(&mut s, "layer.instance", &json!({"layer": paint.0, "ts": 1})).unwrap();
    execute(&mut s, "textureSet.select", &json!({"ts": 0})).unwrap();
    execute(&mut s, "layer.select", &json!({"layer": paint.0})).unwrap();
    execute(&mut s, "tool.material", &json!({"channel": "baseColor", "color": [1.0, 0.0, 0.0]})).unwrap();
    execute(&mut s, "tool.brush", &json!({"size": 30, "hardness": 90})).unwrap();
    assert_eq!(reds(&s, 1), 0);
    // The live stroke shows in the instance before it is committed.
    s.begin_stroke(Vec2::new(200.0, 170.0), s.viewport, 1.0).unwrap();
    s.continue_stroke(Vec2::new(230.0, 170.0), 1.0);
    s.update();
    let live = reds(&s, 1);
    assert!(live > 0, "the instance shows the live stroke");
    s.end_stroke().unwrap();
    s.update();
    assert!(reds(&s, 1) >= live);
    // Painting through the instance paints the source.
    let inst = s.doc.project.texture_sets[1].stack.last().copied().unwrap();
    execute(&mut s, "layer.select", &json!({"layer": inst.0})).unwrap();
    let before = s.doc.pixels.layer(paint, Channel::BaseColor).map(|i| i.allocated_count()).unwrap_or(0);
    execute(&mut s, "tool.stroke", &json!({"points": [[150, 330], [250, 330]]})).unwrap();
    assert!(s.doc.pixels.layer(paint, Channel::BaseColor).unwrap().allocated_count() >= before);
    assert!(s.doc.pixels.layers.keys().all(|(l, _)| *l != inst), "instances hold no pixels");
}

#[test]
fn cycles_are_refused_and_removed_sources_detach() {
    let mut s = session();
    let g = id(execute(&mut s, "layer.add.group", &Value::Null).unwrap(), "layer");
    let fill = id(execute(&mut s, "layer.add.fill", &json!({"roughness": 0.9})).unwrap(), "layer");
    assert_eq!(s.doc.project.locate(fill).unwrap().1, Some(g));
    // The folder selected: the instance would land inside it and draw itself.
    execute(&mut s, "layer.select", &json!({"layer": g.0})).unwrap();
    let doc = s.doc.project.clone();
    assert!(execute(&mut s, "layer.instance", &json!({"layer": g.0})).is_err());
    assert_eq!(s.doc.project, doc, "a refused edit leaves the document as it was");
    // Moving an instance of the folder into it is refused too.
    execute(&mut s, "layer.select", &json!({"layer": fill.0})).unwrap();
    execute(&mut s, "layer.move", &json!({"layer": fill.0, "above": Value::Null})).unwrap();
    execute(&mut s, "layer.select", &json!({"layer": fill.0})).unwrap();
    let inst = id(execute(&mut s, "layer.instance", &json!({"layer": g.0})).unwrap(), "layer");
    assert!(execute(&mut s, "layer.move", &json!({"layer": inst.0, "above": Value::Null, "inside": false})).is_ok());
    assert!(execute(&mut s, "layer.move", &json!({"layer": inst.0, "above": g.0, "inside": true})).is_err());

    // Removing a source turns its instances into copies that look the same.
    let src = id(execute(&mut s, "layer.add.fill", &json!({"roughness": 0.15})).unwrap(), "layer");
    execute(&mut s, "layer.instantiateAll", &json!({"layer": src.0})).unwrap();
    let inst = s.doc.project.texture_sets[1].stack.last().copied().unwrap();
    assert!(s.doc.project.layer(inst).unwrap().instance_source() == Some(src));
    let look = rough(&mut s, 1);
    assert!((look - 0.15).abs() < 1e-3);
    execute(&mut s, "layer.remove", &json!({"layer": src.0})).unwrap();
    assert!(s.doc.project.layer(inst).is_err());
    let copy = s.doc.project.texture_sets[1].stack.last().copied().unwrap();
    assert!(matches!(s.doc.project.layer(copy).unwrap().kind, LayerKind::Fill(_)));
    assert!((rough(&mut s, 1) - look).abs() < 1e-4);
    s.doc.project.validate().unwrap();
    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert_eq!(s.doc.project.layer(inst).unwrap().instance_source(), Some(src));
}

#[test]
fn paste_as_instance_and_instantiate_or_copy_across_sets() {
    let mut s = session();
    let src = id(execute(&mut s, "layer.add.fill", &json!({"roughness": 0.7})).unwrap(), "layer");
    assert!(execute(&mut s, "layer.pasteInstance", &Value::Null).is_err(), "empty clipboard");
    execute(&mut s, "layer.copy", &json!({"layer": src.0})).unwrap();
    execute(&mut s, "textureSet.select", &json!({"ts": 1})).unwrap();
    let pasted = id(execute(&mut s, "layer.pasteInstance", &Value::Null).unwrap(), "layer");
    assert_eq!(s.doc.project.layer(pasted).unwrap().instance_source(), Some(src));
    assert_eq!(s.selected_layer(), Some(pasted));
    // A cut layer is gone: paste it, not an instance of it.
    execute(&mut s, "layer.cut", &json!({"layer": pasted.0})).unwrap();
    assert!(execute(&mut s, "layer.pasteInstance", &Value::Null).is_err());

    let made = execute(&mut s, "layer.instantiateAll", &json!({"layer": src.0})).unwrap();
    let inst = LayerId(made["layers"][0].as_u64().unwrap());
    assert_eq!(s.doc.project.layer(inst).unwrap().instance_source(), Some(src));
    let made = execute(&mut s, "layer.instantiateAll", &json!({"layer": src.0, "copy": true})).unwrap();
    let copy = LayerId(made["layers"][0].as_u64().unwrap());
    assert!(matches!(s.doc.project.layer(copy).unwrap().kind, LayerKind::Fill(_)));
    // Flattening an instance makes it an independent paint layer with the same look.
    let look = rough(&mut s, 1);
    execute(&mut s, "layer.remove", &json!({"layer": copy.0})).unwrap();
    execute(&mut s, "layer.flatten", &json!({"layer": inst.0})).unwrap();
    let l = s.doc.project.layer(inst).unwrap();
    assert!(matches!(l.kind, LayerKind::Paint) && l.blend(Channel::Roughness).is_some());
    assert!((rough(&mut s, 1) - look).abs() < 2e-3);
}

#[test]
fn mask_with_height_through_layer_set() {
    let mut s = session();
    let l = id(execute(&mut s, "layer.add.fill", &json!({"color": [1.0, 1.0, 1.0], "height": 0.5})).unwrap(), "layer");
    execute(&mut s, "mask.add.black", &Value::Null).unwrap();
    execute(&mut s, "effect.add.fill", &json!({"value": 0.5})).unwrap();
    let (x, y) = covered(&s, 0);
    s.update();
    let plain = s.caches[0].composite.value(Channel::BaseColor, x, y)[0];
    let r = execute(&mut s, "layer.set", &json!({"layer": l.0, "heightMask": {"enabled": true, "contrast": 1.0, "offset": 0.0}})).unwrap();
    assert_eq!(r["heightMask"]["enabled"], true);
    s.update();
    let with = s.caches[0].composite.value(Channel::BaseColor, x, y)[0];
    assert!((plain - 0.75).abs() < 0.01 && (with - 1.0).abs() < 0.01, "{plain} {with}");
    let inspect = execute(&mut s, "layer.list", &Value::Null).unwrap();
    assert_eq!(inspect[0]["heightMask"]["contrast"], 1.0);
    execute(&mut s, "layer.set", &json!({"layer": l.0, "heightMask": false})).unwrap();
    assert!(!s.doc.project.layer(l).unwrap().height_mask.enabled);
    assert!(execute(&mut s, "layer.set", &json!({"layer": l.0})).is_err());
    assert!(execute(&mut s, "layer.set", &json!({"layer": l.0, "heightMask": 3})).is_err());
    let hostile = execute(&mut s, "layer.set", &json!({"layer": l.0, "heightMask": {"contrast": 1e30, "offset": -1e30}})).unwrap();
    assert_eq!(hostile["heightMask"]["contrast"], 64.0);
    assert_eq!(hostile["heightMask"]["offset"], -1.0);
}

#[test]
fn edits_invalidate_only_the_sets_they_change() {
    let mut s = session();
    let l = id(execute(&mut s, "layer.add.fill", &json!({"roughness": 0.4})).unwrap(), "layer");
    s.update();
    execute(&mut s, "layer.rename", &json!({"layer": l.0, "name": "Renamed"})).unwrap();
    assert_eq!(s.pending_tiles(), 0, "a rename changes no pixels");
    execute(&mut s, "layer.fill.set", &json!({"layer": l.0, "channel": "roughness", "value": 0.2})).unwrap();
    assert!(!s.caches[0].dirty.is_empty() && s.caches[1].dirty.is_empty(), "only the edited set");
    s.update();
    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert!(!s.caches[0].dirty.is_empty() && s.caches[1].dirty.is_empty());
    assert!((rough(&mut s, 0) - 0.4).abs() < 1e-3);
}

#[test]
fn budgeted_updates_spread_work_and_end_identical() {
    let mut s = session();
    execute(&mut s, "textureSet.resolution", &json!({"ts": 0, "resolution": 1024})).unwrap();
    execute(&mut s, "layer.add.fill", &Value::Null).unwrap();
    execute(&mut s, "layer.fill.set", &json!({"channel": "baseColor", "procedural": "grunge"})).unwrap();
    execute(&mut s, "mask.add.black", &Value::Null).unwrap();
    execute(&mut s, "effect.add.generator", &json!({"generator": "position"})).unwrap();
    execute(&mut s, "effect.add.filter", &json!({"filter": "blur", "radius": 30})).unwrap();
    s.update();
    let full = s.caches[0].composite.clone();
    s.caches[0].invalidate_all();
    // A clock that is always over budget: one chunk per call.
    let mut calls = 0;
    // Progress: a tile is two steps from done while dirty (coarse look, then exact), one when refining.
    let steps = |s: &texturecraft_engine::Session| 2 * s.caches[0].dirty.len() + s.caches[0].refine.len() + s.caches[0].settle.len();
    while s.pending_tiles() > 0 {
        let before = steps(&s);
        s.update_for(0.0, || 1.0);
        assert!(steps(&s) < before);
        calls += 1;
    }
    assert!(calls > 1, "the work was spread over calls");
    assert!(s.caches[0].composite == full, "chunked compositing gives the same pixels");
    assert!(s.memory().composites > 0);
}

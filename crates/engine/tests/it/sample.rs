//! The startup sample project must be ready to paint: an empty paint layer on top, selected.

use serde_json::json;
use texturecraft_engine::core::LayerKind;
use texturecraft_engine::geom::glam::Vec2;
use texturecraft_engine::{execute, project_cmds::sample_session};

#[test]
fn the_sample_opens_on_an_empty_top_paint_layer_that_shows_strokes() {
    let mut s = sample_session(256, 64).unwrap();
    s.viewport = Vec2::new(800.0, 600.0);
    for ts in 0..s.set_count() {
        let set = s.doc.project.texture_set(ts).unwrap();
        let top = *set.stack.last().unwrap();
        assert!(matches!(s.doc.project.layer(top).unwrap().kind, LayerKind::Paint), "set {ts}: the top layer is a paint layer");
        assert_eq!(s.selected.get(&ts), Some(&top), "set {ts}: the top paint layer is selected");
    }
    s.update();
    let before = s.caches[0].composite.clone();
    execute(&mut s, "tool.brush", &json!({"size": 10})).unwrap();
    execute(&mut s, "tool.stroke", &json!({"points": [[300, 260], [500, 300]]})).unwrap();
    s.update();
    assert_ne!(s.caches[0].composite, before, "a stroke on the sample changes what is shown");
}

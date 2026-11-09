//! Effect rows in the layer stack: dragging one reorders its stack.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};
use texturecraft_engine::core::LayerId;
use texturecraft_engine::execute;
use texturecraft_ui_egui::TextureCraftApp;

fn id(v: Value, key: &str) -> u64 {
    v[key].as_u64().unwrap()
}

/// Drag the row labelled `from` onto the upper (`below` false) or lower half of the row `to`.
fn drag(h: &mut Harness<'_, TextureCraftApp>, from: &str, to: &str, below: bool) {
    let start = h.get_by_label(from).rect().center();
    let r = h.get_by_label(to).rect();
    let end = egui::pos2(r.center().x, if below { r.bottom() - 4.0 } else { r.top() + 4.0 });
    h.drag_at(start);
    h.run_steps(2);
    h.hover_at(end);
    h.run_steps(2);
    h.drop_at(end);
    h.run_steps(3);
}

#[test]
fn dragging_an_effect_row_reorders_its_stack() {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = LayerId(id(execute(&mut session, "layer.add.fill", &Value::Null).unwrap(), "layer"));
    let mut add =
        |name: &str| id(execute(&mut session, "effect.add.filter", &json!({"target": "content", "filter": "invert", "name": name})).unwrap(), "effect");
    let (a, b, c) = (add("Effect A"), add("Effect B"), add("Effect C"));
    execute(&mut session, "layer.select", &json!({"layer": layer.0})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1400.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.run_steps(4);
    let order = |h: &Harness<'_, TextureCraftApp>| h.state().session.doc.project.layers[&layer].content_effects.iter().map(|e| e.id.0).collect::<Vec<_>>();
    assert_eq!(order(&h), [a, b, c]);
    // The rows list the top first: C, B, A. A dropped on C's upper half goes to the top.
    drag(&mut h, "Effect A", "Effect C", false);
    assert_eq!(order(&h), [b, c, a]);
    // A dropped on B's lower half goes back to the bottom.
    drag(&mut h, "Effect A", "Effect B", true);
    assert_eq!(order(&h), [a, b, c]);
    // C dropped on A's upper half lands between A and B.
    drag(&mut h, "Effect C", "Effect A", false);
    assert_eq!(order(&h), [a, c, b]);
}

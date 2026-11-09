//! The path list in a path sublayer's properties: pick a path to edit it, remove one.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};
use texturecraft_engine::core::LayerId;
use texturecraft_engine::execute;
use texturecraft_ui_egui::TextureCraftApp;

#[test]
fn path_rows_edit_and_remove_paths() {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    execute(&mut session, "layer.add.paint", &Value::Null).unwrap();
    let r = execute(&mut session, "tool.path", &json!({"uvPoints": [[0.2, 0.5], [0.5, 0.4], [0.8, 0.5]], "commit": true})).unwrap();
    let eid = LayerId(r["committed"]["effect"].as_u64().unwrap());
    execute(&mut session, "tool.path", &json!({"uvPoints": [[0.2, 0.8], [0.8, 0.8]], "commit": true})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1400.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.run_steps(4);
    let editing = |h: &Harness<'_, TextureCraftApp>| h.state().session.tool.path.editing;
    let count = |h: &Harness<'_, TextureCraftApp>| h.state().session.paths(eid).map_or(0, <[_]>::len);

    // Picking a row edits that path; picking it again finishes the edit.
    h.get_by_label("Path 2").click();
    h.run_steps(3);
    assert_eq!(editing(&h), Some((eid, 1)));
    assert_eq!(h.state().session.tool.path.points.len(), 2);
    h.get_by_label("Path 2").click();
    h.run_steps(3);
    assert_eq!(editing(&h), None);

    // The remove button on the first row removes the first path.
    let row = h.get_by_label("Path 1").rect();
    let remove = h.query_all_by_label("Remove").find(|n| (n.rect().center().y - row.center().y).abs() < row.height()).expect("a remove button on the row");
    remove.click();
    h.run_steps(3);
    assert_eq!(count(&h), 1);
    assert!(h.query_by_label("Path 2").is_none());
}

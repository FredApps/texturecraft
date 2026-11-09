//! Polygon fill drag transitions preserve already painted targets

use serde_json::json;
use texturecraft_engine::core::Channel;
use texturecraft_engine::geom::{glam::Vec2, primitives};
use texturecraft_engine::{Session, execute};

#[test]
fn changing_layers_during_a_box_drag_preserves_the_first_fill() {
    let mut session = Session::new_project("t", primitives::plane(2.0, 4), "test", None, 128).unwrap();
    let first = session.selected_layer().unwrap();
    execute(&mut session, "layer.add.paint", &json!({})).unwrap();
    let second = session.selected_layer().unwrap();
    execute(&mut session, "layer.select", &json!({"layer": first.0})).unwrap();
    let history = session.history_labels().len();

    session.drag_polygon_fill_box_uv(Vec2::ZERO, Vec2::ONE).unwrap();
    execute(&mut session, "layer.select", &json!({"layer": second.0})).unwrap();
    session.drag_polygon_fill_box_uv(Vec2::ZERO, Vec2::new(0.4, 1.0)).unwrap();
    session.end_polygon_fill_box();

    assert!(session.doc.pixels.layer(first, Channel::BaseColor).is_some(), "the first layer keeps its fill");
    assert!(session.doc.pixels.layer(second, Channel::BaseColor).is_some(), "the new target receives the next fill");
    assert_eq!(session.history_labels().len(), history + 2, "each target has its own undo step");
    session.undo().unwrap();
    assert!(session.doc.pixels.layer(first, Channel::BaseColor).is_some());
    assert!(session.doc.pixels.layer(second, Channel::BaseColor).is_none());
}

#[test]
fn changing_from_content_to_mask_during_a_box_drag_preserves_content() {
    let mut session = Session::new_project("t", primitives::plane(2.0, 4), "test", None, 128).unwrap();
    let layer = session.selected_layer().unwrap();
    execute(&mut session, "mask.add.black", &json!({})).unwrap();
    execute(&mut session, "layer.select", &json!({"layer": layer.0, "target": "content"})).unwrap();
    let history = session.history_labels().len();

    session.drag_polygon_fill_box_uv(Vec2::ZERO, Vec2::ONE).unwrap();
    execute(&mut session, "layer.select", &json!({"layer": layer.0, "target": "mask"})).unwrap();
    session.drag_polygon_fill_box_uv(Vec2::ZERO, Vec2::new(0.4, 1.0)).unwrap();
    session.end_polygon_fill_box();

    assert!(session.doc.pixels.layer(layer, Channel::BaseColor).is_some(), "switching to the mask keeps the content fill");
    assert_eq!(session.history_labels().len(), history + 2);
    session.undo().unwrap();
    assert!(session.doc.pixels.layer(layer, Channel::BaseColor).is_some());
    assert!(session.doc.project.layer(layer).unwrap().mask.as_ref().unwrap().effects.is_empty());
}

#[test]
#[ignore = "release performance measurement"]
fn benchmark_live_box_fill() {
    use std::hint::black_box;
    use std::time::Instant;

    for (name, mesh) in [("ball", primitives::material_ball()), ("sphere", primitives::sphere(1.0, 512, 256))] {
        let triangles = mesh.triangle_count();
        let mut session = Session::new_project("benchmark", mesh, "test", None, 1024).unwrap();
        let viewport = Vec2::new(1920.0, 1080.0);
        let start = Instant::now();
        session.drag_polygon_fill_box(viewport * 0.4, viewport * 0.6, viewport).unwrap();
        let first_ms = start.elapsed().as_secs_f64() * 1000.0;
        let mut times = Vec::new();
        for step in 1..=12 {
            let radius = 0.1 + step as f32 * 0.02;
            let start = Instant::now();
            black_box(session.drag_polygon_fill_box(viewport * (0.5 - radius), viewport * (0.5 + radius), viewport).unwrap());
            times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        let (corner, opposite) = (viewport * 0.16, viewport * 0.84);
        session.drag_polygon_fill_box(corner, opposite, viewport).unwrap();
        let start = Instant::now();
        for _ in 0..1000 {
            black_box(session.drag_polygon_fill_box(corner, opposite, viewport).unwrap());
        }
        let unchanged_us = start.elapsed().as_secs_f64() * 1000.0;
        session.end_polygon_fill_box();
        let start = Instant::now();
        session.update();
        let composite_ms = start.elapsed().as_secs_f64() * 1000.0;
        times.sort_by(f64::total_cmp);
        let memory = session.memory();
        eprintln!(
            "{name}: {triangles} triangles, first={first_ms:.3} ms, growing median={:.3} ms max={:.3} ms, unchanged mean={unchanged_us:.3} us, composite={composite_ms:.3} ms, pixels={} bytes composites={} bytes cache={} bytes",
            times[6], times[11], memory.pixels, memory.composites, memory.cache
        );
    }
}

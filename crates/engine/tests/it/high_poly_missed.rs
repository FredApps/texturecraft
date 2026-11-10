use texturecraft_engine::bake::{HighPolyReach, RayStart, VertexReach, high_poly_missed};
use texturecraft_engine::geom::glam::Vec3;

fn vertex(height: f32) -> Option<VertexReach> {
    Some(VertexReach {
        low_position: Vec3::ZERO,
        start: RayStart::Distance { up: Vec3::Y, scale: 1.0 },
        high_position: Vec3::new(0.0, height, 0.0),
        high_facing: Vec3::Y,
        name_matches: true,
    })
}

fn reach(vertices: Vec<Option<VertexReach>>) -> Option<HighPolyReach> {
    Some(HighPolyReach { vertices, cage_units: 1.0, ignore_backface: false })
}

fn sets() -> Vec<Option<HighPolyReach>> {
    vec![None, reach(vec![vertex(0.5), vertex(-0.25), None, None, None]), reach(vec![None, None, vertex(0.5), vertex(-0.25), None])]
}

const DISTANCES: [(f32, f32); 3] = [(9.0, 9.0), (0.25, 0.25), (0.5, 0.125)];

#[test]
fn each_owning_set_uses_its_own_frontal_and_rear_distances() {
    let reaches = sets();
    assert_eq!(high_poly_missed(&reaches, &DISTANCES, None, 6), vec![1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    let expanded = [(9.0, 9.0), (0.5, 0.25), (0.5, 0.25)];
    assert_eq!(high_poly_missed(&reaches, &expanded, None, 6), vec![0.0; 6]);
}

#[test]
fn selected_set_filter_leaves_other_sets_unmarked() {
    let reaches = sets();
    assert_eq!(high_poly_missed(&reaches, &DISTANCES, Some(1), 6), vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    assert_eq!(high_poly_missed(&reaches, &DISTANCES, Some(2), 6), vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    for only_set in [0, reaches.len(), usize::MAX] {
        assert_eq!(high_poly_missed(&reaches, &DISTANCES, Some(only_set), 6), vec![0.0; 6]);
    }
}

#[test]
fn missing_reach_or_distances_leave_vertices_unmarked() {
    assert_eq!(high_poly_missed(&[], &DISTANCES, None, 6), vec![0.0; 6]);
    assert_eq!(high_poly_missed(&sets(), &[], None, 6), vec![0.0; 6]);
    assert_eq!(high_poly_missed(&sets(), &DISTANCES[..2], None, 6), vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    assert_eq!(high_poly_missed(&sets(), &DISTANCES, None, 1), vec![1.0]);
    assert!(high_poly_missed(&sets(), &DISTANCES, None, 0).is_empty());
}

#[test]
fn a_reached_vertex_is_not_overridden_by_a_later_set() {
    let reaches = [reach(vec![vertex(0.5)]), reach(vec![vertex(0.5)])];
    let distances = [(0.5, 0.0), (0.25, 0.0)];
    assert_eq!(high_poly_missed(&reaches, &distances, None, 1), vec![0.0]);
    assert_eq!(high_poly_missed(&reaches, &distances, Some(1), 1), vec![1.0]);
}

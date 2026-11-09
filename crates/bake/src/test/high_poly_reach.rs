use super::settings;
use crate::map::{BakeContext, transfer_keys};
use crate::tracing::ANY;
use crate::{BakeRequest, HighPoly, HighPolyReach, RayStart};
use texturecraft_core::{BakedMap, MeshMatchMode};
use texturecraft_geom::glam::Vec3;
use texturecraft_geom::{Bvh, Mesh, primitives, surface_at};

/// Ray distances closer than this to the end of the ray's range are left out: rounding decides them
const BOUNDARY: f32 = 1e-4;

/// How close the ray's hit must be to the vertex to count as reaching it
const HIT_TOLERANCE: f32 = 1e-4;

/// A bumpy plane over the flat low poly, with the triangles right of `FLIPPED_FROM_X` wound the other way
fn bumpy_high_poly() -> (Mesh, Vec<bool>) {
    const FLIPPED_FROM_X: f32 = 0.2;

    let mut mesh = primitives::plane(1.0, 40);
    for position in &mut mesh.positions {
        position.y = 0.04 * (position.x * 9.0).sin() * (position.z * 7.0).cos() + 0.005;
    }

    let mut flipped = vec![false; mesh.positions.len()];
    let mut unflipped = vec![false; mesh.positions.len()];
    for corners in mesh.indices.chunks_exact_mut(3) {
        let center_x = corners.iter().map(|&corner| mesh.positions[corner as usize].x).sum::<f32>() / 3.0;
        if center_x > FLIPPED_FROM_X {
            corners.swap(1, 2);
        }
        for &corner in corners.iter() {
            if center_x > FLIPPED_FROM_X { flipped[corner as usize] = true } else { unflipped[corner as usize] = true }
        }
    }
    // Vertices on the border face both ways: neither answer is right for them
    let mixed = flipped.iter().zip(&unflipped).map(|(flipped, unflipped)| *flipped && *unflipped).collect();
    (mesh, mixed)
}

#[test]
fn a_vertex_is_missed_exactly_when_the_bake_ray_misses_it() {
    let low = primitives::plane(1.0, 8);
    let (high_mesh, mixed) = bumpy_high_poly();
    let (low_bvh, high_bvh) = (Bvh::new(&low), Bvh::new(&high_mesh));
    let high = HighPoly { file_ids: &[], mesh: &high_mesh, bvh: &high_bvh, base_color: &[] };
    let back_facing = |triangle: u32, direction: Vec3| {
        let [a, b, c] = high_mesh.tri(triangle as usize).unwrap().map(|corner| high_mesh.positions[corner]);
        (b - a).cross(c - a).dot(direction) > 0.0
    };
    let mut checked = [0usize; 2];

    for ignore_backface in [false, true] {
        for (max_frontal, max_rear) in [(0.0, 0.01), (0.02, 0.0), (0.02, 0.01), (0.05, 0.05)] {
            let mut bake_settings = settings(16, &[BakedMap::Normal]);
            bake_settings.cage.relative_to_bounding_box = false;
            bake_settings.high_poly.ignore_backface = ignore_backface;
            (bake_settings.cage.max_frontal, bake_settings.cage.max_rear) = (max_frontal, max_rear);
            let request = BakeRequest {
                low_base_color: &[],
                mesh: &low,
                bvh: &low_bvh,
                material: 0,
                settings: &bake_settings,
                high: Some(high),
                cage: None,
                skew: None,
                cage_paint: None,
                tiles: (1, 1),
            };
            let reach = HighPolyReach::new(&request).unwrap();
            let context = BakeContext::new(&request).unwrap();

            for (vertex, vertex_reach) in reach.vertices.iter().enumerate() {
                let (vertex_reach, missed) = (vertex_reach.unwrap(), reach.missed(vertex, max_frontal, max_rear).unwrap());
                let RayStart::Distance { scale, .. } = vertex_reach.start else { panic!("a distance-based cage") };
                let height = vertex_reach.high_position.y - vertex_reach.low_position.y;
                let near_the_range_end = [max_frontal * scale, -max_rear].iter().any(|end| (height - end).abs() < BOUNDARY);
                if near_the_range_end || mixed[vertex] {
                    continue;
                }

                let nearest = low_bvh.nearest(vertex_reach.high_position, f32::INFINITY, &|_| true).unwrap();
                let surface = surface_at(&low, nearest.tri, nearest.u, nearest.v).unwrap();
                let (ray, _) = context.transfer_ray(&surface, [nearest.u, nearest.v]);
                let hit = high_bvh
                    .closest_filtered(ray.origin, ray.direction, ray.max_distance, &|triangle| !(ignore_backface && back_facing(triangle, ray.direction)));
                let reaches_the_vertex = hit.is_some_and(|hit| (ray.origin + ray.direction * hit.t - vertex_reach.high_position).length() < HIT_TOLERANCE);
                assert_eq!(
                    missed, !reaches_the_vertex,
                    "vertex {vertex}, ignore backface {ignore_backface}, frontal {max_frontal}, rear {max_rear}: height {height}"
                );
                checked[usize::from(missed)] += 1;
            }
        }
    }
    assert!(checked[0] > 100 && checked[1] > 100, "both reached and missed vertices were compared: {checked:?}");
}

#[test]
fn a_vertex_belongs_to_the_texture_set_whose_low_poly_is_nearest() {
    let mut low = primitives::plane(1.0, 4);
    let mut right = primitives::plane(1.0, 4);
    for position in &mut right.positions {
        position.x += 2.0;
    }
    low.append(&right);
    assert_eq!(low.materials.len(), 2, "each plane is its own texture set");
    let mut high_mesh = primitives::plane(3.0, 30);
    for position in &mut high_mesh.positions {
        position.x += 1.0;
        position.y = 0.3;
    }
    let (low_bvh, high_bvh) = (Bvh::new(&low), Bvh::new(&high_mesh));
    let bake_settings = settings(16, &[BakedMap::Normal]);
    let request = |material| BakeRequest {
        low_base_color: &[],
        mesh: &low,
        bvh: &low_bvh,
        material,
        settings: &bake_settings,
        high: Some(HighPoly { file_ids: &[], mesh: &high_mesh, bvh: &high_bvh, base_color: &[] }),
        cage: None,
        skew: None,
        cage_paint: None,
        tiles: (1, 1),
    };
    let (left, right) = (HighPolyReach::new(&request(0)).unwrap(), HighPolyReach::new(&request(1)).unwrap());

    for (vertex, position) in high_mesh.positions.iter().enumerate() {
        let owners = (left.vertices[vertex].is_some(), right.vertices[vertex].is_some());
        assert!(owners != (true, true), "vertex {vertex} belongs to one set at most");
        if position.x < 0.9 {
            assert_eq!(owners, (true, false), "vertex {vertex} at {position} is over the left set");
        } else if position.x > 1.1 {
            assert_eq!(owners, (false, true), "vertex {vertex} at {position} is over the right set");
        }
    }
}

#[test]
fn matching_by_mesh_name_misses_the_parts_with_another_name() {
    const HEIGHT_INSIDE_THE_CAGE: f32 = 0.01;

    let mut low = primitives::plane(1.0, 8);
    low.submeshes[0].name = "part_low".into();
    let mut high_mesh = primitives::plane(0.5, 10);
    let mut other_part = primitives::plane(0.5, 10);
    for (mesh, center_x) in [(&mut high_mesh, -0.25), (&mut other_part, 0.25)] {
        for position in &mut mesh.positions {
            position.x += center_x;
            position.y = HEIGHT_INSIDE_THE_CAGE;
        }
    }
    high_mesh.append(&other_part);
    high_mesh.submeshes[0].name = "part_high".into();
    high_mesh.submeshes[1].name = "other_high".into();
    let (low_bvh, high_bvh) = (Bvh::new(&low), Bvh::new(&high_mesh));
    let mut bake_settings = settings(16, &[BakedMap::Normal]);
    bake_settings.cage.relative_to_bounding_box = false;
    bake_settings.cage.max_frontal = 0.02;
    bake_settings.high_poly.match_mode = MeshMatchMode::ByMeshName;
    let request = BakeRequest {
        low_base_color: &[],
        mesh: &low,
        bvh: &low_bvh,
        material: 0,
        settings: &bake_settings,
        high: Some(HighPoly { file_ids: &[], mesh: &high_mesh, bvh: &high_bvh, base_color: &[] }),
        cage: None,
        skew: None,
        cage_paint: None,
        tiles: (1, 1),
    };
    let reach = HighPolyReach::new(&request).unwrap();
    let context = BakeContext::new(&request).unwrap();
    let (_, high_keys) = transfer_keys(&request);

    for (vertex, vertex_reach) in reach.vertices.iter().enumerate() {
        let (vertex_reach, missed) = (vertex_reach.unwrap(), reach.missed(vertex, 0.02, 0.01).unwrap());
        // On the line where the two parts meet, a ray reaches either part's triangles
        if vertex_reach.high_position.x.abs() < BOUNDARY {
            continue;
        }

        let nearest = low_bvh.nearest(vertex_reach.high_position, f32::INFINITY, &|_| true).unwrap();
        let surface = surface_at(&low, nearest.tri, nearest.u, nearest.v).unwrap();
        let (ray, _) = context.transfer_ray(&surface, [nearest.u, nearest.v]);
        let same_name = |triangle: u32| ray.key == ANY || high_keys.get(triangle as usize) == Some(&ray.key);
        let hit = high_bvh.closest_filtered(ray.origin, ray.direction, ray.max_distance, &same_name);
        let reaches_the_vertex = hit.is_some_and(|hit| (ray.origin + ray.direction * hit.t - vertex_reach.high_position).length() < HIT_TOLERANCE);
        assert_eq!(missed, !reaches_the_vertex, "vertex {vertex} at {}", vertex_reach.high_position);
        assert_eq!(
            missed,
            vertex >= high_mesh.positions.len() - other_part.positions.len(),
            "only the part named like the low poly is reached: vertex {vertex}"
        );
    }
}

#[test]
fn without_a_high_poly_no_vertex_has_a_reach() {
    let low = primitives::plane(1.0, 4);
    let low_bvh = Bvh::new(&low);
    let bake_settings = settings(16, &[BakedMap::Normal]);
    let request = BakeRequest {
        low_base_color: &[],
        mesh: &low,
        bvh: &low_bvh,
        material: 0,
        settings: &bake_settings,
        high: None,
        cage: None,
        skew: None,
        cage_paint: None,
        tiles: (1, 1),
    };

    let reach = HighPolyReach::new(&request).unwrap();
    assert!(reach.vertices.is_empty());
    // Any cage distances: there is no vertex to miss
    assert_eq!(reach.missed(0, 0.02, 0.01), None);
}

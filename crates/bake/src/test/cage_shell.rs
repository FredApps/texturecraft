use super::settings;
use crate::cage::CORNERS;
use crate::map::BakeContext;
use crate::{BakePaintMap, BakeRequest, CageShell, HighPoly};
use texturecraft_core::{BakedMap, CageType};
use texturecraft_geom::glam::{Vec2, Vec3};
use texturecraft_geom::{Bvh, Mesh, primitives, surface_at};

/// A painted map whose values vary over the UV square
fn painted_map() -> BakePaintMap {
    let mut map = BakePaintMap::constant(0.25).resized(32, 32).unwrap();
    map.paint(&[Vec2::new(0.2, 0.3), Vec2::new(0.7, 0.6)], 0.3, 1.0, 0.5, 1.0).unwrap();
    map
}

/// Every corner of `request`'s cage equals the origin of the bake's transfer ray at that corner, bit for bit
fn assert_cage_matches_the_ray_origins(request: &BakeRequest) {
    let shell = CageShell::new(request).unwrap();
    let corners = shell.corners(request.settings.cage.max_frontal);
    let context = BakeContext::new(request).unwrap();
    let triangles: Vec<usize> = request.mesh.triangles_of(request.material).collect();
    assert_eq!(corners.len(), triangles.len() * 3);

    for (corner_index, corner) in corners.iter().enumerate() {
        let barycentric = CORNERS[corner_index % 3];
        let surface = surface_at(request.mesh, triangles[corner_index / 3] as u32, barycentric[0], barycentric[1]).unwrap();
        let (ray, _) = context.transfer_ray(&surface, barycentric);
        assert_eq!(*corner, ray.origin, "corner {corner_index}");
    }
}

#[test]
fn the_cage_corners_are_the_bake_ray_origins() {
    let low = primitives::cube(1.0);
    let high_mesh = primitives::sphere(0.8, 16, 8);
    let (low_bvh, high_bvh) = (Bvh::new(&low), Bvh::new(&high_mesh));
    let high = HighPoly { file_ids: &[], mesh: &high_mesh, bvh: &high_bvh, base_color: &[] };
    let (skew, cage_paint) = (painted_map(), painted_map());
    let mut bake_settings = settings(16, &[BakedMap::Normal]);
    bake_settings.cage.max_frontal = 0.03;
    bake_settings.skew.edge_protection = true;
    let request = |settings| BakeRequest {
        low_base_color: &[],
        mesh: &low,
        bvh: &low_bvh,
        material: 0,
        settings,
        high: Some(high),
        cage: None,
        skew: Some(&skew),
        cage_paint: Some(&cage_paint),
        tiles: (1, 1),
    };

    assert_cage_matches_the_ray_origins(&request(&bake_settings));
    let face_normals = {
        let mut face_normals = bake_settings.clone();
        face_normals.cage.average_normals = false;
        face_normals.cage.relative_to_bounding_box = false;
        face_normals
    };
    assert_cage_matches_the_ray_origins(&request(&face_normals));
}

#[test]
fn the_cage_follows_the_frontal_distance_and_a_cage_mesh() {
    let low = primitives::cube(1.0);
    let low_bvh = Bvh::new(&low);
    let high = HighPoly { file_ids: &[], mesh: &low, bvh: &low_bvh, base_color: &[] };
    let mut bake_settings = settings(16, &[BakedMap::Normal]);
    bake_settings.cage.relative_to_bounding_box = false;
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
    let shell = CageShell::new(&request).unwrap();
    let distance_from_the_cube = |max_frontal: f32| shell.corners(max_frontal).iter().map(|corner| corner.abs().max_element() - 0.5).fold(0.0, f32::max);

    assert!(distance_from_the_cube(0.0).abs() < 1e-6, "at frontal 0 the cage is the low poly");
    // Averaged normals point diagonally at the corners, so a corner moves out by at least half the distance along each axis
    assert!(distance_from_the_cube(0.2) > 0.1 && distance_from_the_cube(0.2) <= 0.2 + 1e-6, "the cage grows with the frontal distance");
    assert_eq!(shell.edges.len(), low.wire_edges()[0].len(), "one wireframe line per polygon edge");

    let cage_mesh = Mesh { positions: low.positions.iter().map(|position| *position * 1.5).collect(), ..low.clone() };
    let custom_settings = {
        let mut custom_settings = bake_settings.clone();
        custom_settings.cage.kind = CageType::CustomFile;
        custom_settings
    };
    let custom = BakeRequest { settings: &custom_settings, cage: Some(&cage_mesh), ..request };
    assert_cage_matches_the_ray_origins(&custom);
    let corners = CageShell::new(&custom).unwrap().corners(0.0);
    assert!(corners.iter().all(|corner| (corner.abs().max_element() - 0.75).abs() < 1e-6), "the cage mesh is drawn as it is");
    assert!(corners.iter().all(|corner| corner.is_finite() && *corner != Vec3::ZERO));
}

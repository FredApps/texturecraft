//! Visibility agrees with the surface under each pixel

use texturecraft_geom::glam::{Vec2, Vec3};
use texturecraft_geom::{Mesh, OrbitCamera};
use texturecraft_paint::EyeDepth;

#[test]
fn a_large_front_triangle_occludes_a_small_rear_triangle() {
    let positions = vec![
        Vec3::new(-4.0, -4.0, 0.0),
        Vec3::new(4.0, -4.0, 0.0),
        Vec3::new(0.0, 4.0, 0.0),
        Vec3::new(-0.5, -0.5, -1.0),
        Vec3::new(0.5, -0.5, -1.0),
        Vec3::new(0.0, 0.5, -1.0),
    ];
    let mesh = Mesh::from_parts(positions, None, vec![Vec2::ZERO; 6], (0..6).collect(), "m").unwrap();

    for ortho in [false, true] {
        let camera = OrbitCamera { pitch: 0.0, ortho, ..Default::default() };
        let depth = EyeDepth::render_with_triangles(&mesh, &camera, 128, 128);
        assert_eq!(depth.triangle[64 * 128 + 64], 0, "orthographic: {ortho}");
        assert!(!depth.visible(Vec3::new(0.0, 0.0, -1.0)), "the rear face is hidden");
        assert!(depth.visible(Vec3::ZERO), "the front face is visible");
    }
}

#[test]
#[ignore = "release performance measurement"]
fn benchmark_eye_depth() {
    use std::hint::black_box;
    use std::time::Instant;
    use texturecraft_geom::primitives;

    for (name, mesh) in [("ball", primitives::material_ball()), ("sphere", primitives::sphere(1.0, 512, 256))] {
        let mut camera = OrbitCamera::default();
        let (low, high) = mesh.bounds();
        camera.frame(low, high);
        for triangles in [false, true] {
            let mut times = Vec::new();
            for iteration in 0..12 {
                let start = Instant::now();
                black_box(if triangles { EyeDepth::render_with_triangles(&mesh, &camera, 960, 540) } else { EyeDepth::render(&mesh, &camera, 960, 540) });
                if iteration >= 2 {
                    times.push(start.elapsed().as_secs_f64() * 1000.0);
                }
            }
            times.sort_by(f64::total_cmp);
            eprintln!("{name}: {} triangles, record={triangles}, median={:.3} ms, range={:.3}..{:.3} ms", mesh.triangle_count(), times[5], times[0], times[9]);
        }
    }
}

#[test]
fn perspective_depth_matches_ray_intersections() {
    use texturecraft_geom::Bvh;

    let positions = vec![Vec3::new(-3.0, -2.0, 2.0), Vec3::new(3.0, -2.0, -2.0), Vec3::new(0.0, 3.0, 0.0)];
    let mesh = Mesh::from_parts(positions, None, vec![Vec2::ZERO; 3], vec![0, 1, 2], "m").unwrap();
    let camera = OrbitCamera { pitch: 0.0, ..Default::default() };
    let bvh = Bvh::new(&mesh);
    let depth = EyeDepth::render_with_triangles(&mesh, &camera, 128, 128);

    for (x, y) in [(40, 60), (64, 64), (88, 60)] {
        let ndc = Vec2::new((x as f32 + 0.5) / 64.0 - 1.0, 1.0 - (y as f32 + 0.5) / 64.0);
        let ray = camera.ray(ndc, 1.0);
        let hit = bvh.closest(ray.origin, ray.dir, f32::MAX).unwrap();
        let expected = (ray.origin + ray.dir * hit.t).distance(camera.eye());
        assert!((depth.dist[y * 128 + x] - expected).abs() < 0.001, "pixel {x},{y}: {} vs {expected}", depth.dist[y * 128 + x]);
    }
}

#[test]
fn capped_buffers_keep_the_requested_viewport_aspect() {
    let positions = vec![Vec3::new(4.0, -1.0, 0.0), Vec3::new(7.0, -1.0, 0.0), Vec3::new(5.5, 1.0, 0.0)];
    let mesh = Mesh::from_parts(positions, None, vec![Vec2::ZERO; 3], vec![0, 1, 2], "m").unwrap();
    let camera = OrbitCamera { pitch: 0.0, ..Default::default() };
    let depth = EyeDepth::render_with_triangles(&mesh, &camera, 8192, 1024);
    let point = camera.project(Vec3::new(5.5, 0.0, 0.0), 8.0).unwrap();
    let x = ((point.x * 0.5 + 0.5) * depth.width as f32) as usize;
    let y = ((0.5 - point.y * 0.5) * depth.height as f32) as usize;

    assert_eq!(depth.triangle[y * depth.width as usize + x], 0, "the triangle stays under its viewport position after the buffer is capped");
}

#[test]
fn triangle_ids_do_not_expand_hidden_cube_faces_past_the_silhouette() {
    use std::collections::HashSet;
    use texturecraft_geom::{NO_TRI, primitives};

    let mesh = primitives::cube(1.0);
    let mut camera = OrbitCamera { yaw: 0.6, pitch: 0.4, ..Default::default() };
    let (low, high) = mesh.bounds();
    camera.frame(low, high);
    let depth = EyeDepth::render_with_triangles(&mesh, &camera, 400, 400);
    let visible: HashSet<_> = depth.triangle.iter().copied().filter(|triangle| *triangle != NO_TRI).collect();

    assert_eq!(visible.len(), mesh.triangle_count() / 2, "only the three front faces own screen pixels");
}

//! Symmetry around a moved plane or axis, driven through engine commands the way agents paint

use serde_json::json;
use texturecraft_engine::core::Channel;
use texturecraft_engine::geom::glam::{Vec2, Vec3, Vec4};
use texturecraft_engine::geom::{Mesh, primitives};
use texturecraft_engine::{Session, execute};

const TEXTURE_SIZE: f32 = 256.0;
const VIEWPORT: f32 = 400.0;

fn session(mesh: Mesh) -> Session {
    let mut session = Session::new_project("t", mesh, "builtin:test", None, TEXTURE_SIZE as u32).unwrap();
    session.viewport = Vec2::splat(VIEWPORT);
    session.camera.yaw = 0.0;
    session.camera.pitch = 0.0;
    session.update();
    execute(&mut session, "tool.material", &json!({"channel": "baseColor", "color": [1.0, 0.0, 0.0]})).unwrap();
    execute(&mut session, "tool.brush", &json!({"size": 6, "hardness": 90})).unwrap();
    session
}

/// A 2×2 wall in the XY plane facing the camera (+Z)
fn wall() -> Mesh {
    let mut mesh = primitives::plane(2.0, 8);
    for point in &mut mesh.positions {
        *point = Vec3::new(point.x, -point.z, 0.0);
    }
    mesh.normals.iter_mut().for_each(|normal| *normal = Vec3::Z);
    mesh.tangents.iter_mut().for_each(|tangent| *tangent = Vec4::new(1.0, 0.0, 0.0, 1.0));
    mesh
}

/// Viewport pixel of world `point`
fn screen(session: &Session, point: Vec3) -> Vec2 {
    let ndc = session.camera.project(point, 1.0).unwrap();
    Vec2::new((ndc.x * 0.5 + 0.5) * VIEWPORT, (0.5 - ndc.y * 0.5) * VIEWPORT)
}

/// Is the surface at world `point` painted red?
fn red_at(session: &Session, point: Vec3) -> bool {
    let hit = session.pick(screen(session, point), session.viewport).unwrap();
    let color = session.caches[0].composite.value(Channel::BaseColor, (hit.uv.x * TEXTURE_SIZE) as u32, (hit.uv.y * TEXTURE_SIZE) as u32);
    color[0] > 0.85 && color[1] < 0.15
}

/// Where a viewport pixel lands on the mesh
fn surface(session: &Session, x: f32, y: f32) -> Vec3 {
    session.pick(Vec2::new(x, y), session.viewport).unwrap().pos
}

#[test]
fn a_moved_mirror_plane_paints_the_copy_at_the_mirrored_point() {
    let mut session = session(wall());
    let reply = execute(&mut session, "tool.brush", &json!({"symmetryX": true, "symmetryOffset": [0.25, 0.0, 0.0]})).unwrap();

    // Right of the view's center, on the wall
    let stroke = surface(&session, 300.0, 200.0);
    execute(&mut session, "tool.stroke", &json!({"points": [[300, 200]]})).unwrap();
    session.update();

    let mirrored = Vec3::new(0.5 - stroke.x, stroke.y, stroke.z);
    assert!(red_at(&session, stroke), "the stroke");
    assert!(red_at(&session, mirrored), "its copy across x = 0.25");
    assert!(!red_at(&session, Vec3::new(-stroke.x, stroke.y, stroke.z)), "nothing across the mesh center");
    assert_eq!(reply["symmetryOffset"], json!([0.25, 0.0, 0.0]), "agents read the offset back");
}

#[test]
fn radial_copies_turn_around_the_moved_axis() {
    let mut session = session(wall());
    let axis = Vec3::new(0.2, -0.1, 0.0);
    execute(&mut session, "tool.brush", &json!({"symmetryRadial": 2, "symmetryAxis": "z", "symmetryOffset": axis.to_array()})).unwrap();

    // Up and right of the view's center, on the wall
    let stroke = surface(&session, 290.0, 160.0);
    execute(&mut session, "tool.stroke", &json!({"points": [[290, 160]]})).unwrap();
    session.update();

    // Half a turn around Z through the moved axis
    assert!(red_at(&session, stroke), "the stroke");
    assert!(red_at(&session, axis * 2.0 - stroke), "its copy half a turn around the moved axis");
    assert!(!red_at(&session, Vec3::new(-stroke.x, -stroke.y, 0.0)), "nothing around the mesh center");
}

#[test]
fn a_closed_path_fill_mirrors_around_the_moved_plane() {
    let mut session = session(wall());
    execute(&mut session, "tool.brush", &json!({"symmetryX": true, "symmetryOffset": [-0.2, 0.0, 0.0]})).unwrap();

    // A small square right of the view's center, on the wall
    let square = json!([[250, 180], [280, 180], [280, 220], [250, 220]]);
    execute(&mut session, "tool.path", &json!({"points": square, "closed": true, "fill": true, "commit": true})).unwrap();

    let inside = surface(&session, 265.0, 200.0);
    assert!(red_at(&session, inside), "the filled lasso");
    assert!(red_at(&session, Vec3::new(-0.4 - inside.x, inside.y, 0.0)), "its copy across x = -0.2");
    assert!(!red_at(&session, Vec3::new(-inside.x, inside.y, 0.0)), "nothing across the mesh center");
}

/// How much redder than green a texel must be to count as painted (faded edges included)
const PAINTED_TINT: f32 = 0.05;

/// Texels tinted red in the top and bottom half of the texture
fn painted_halves(session: &Session) -> (usize, usize) {
    let painted = |x: u32, y: u32| {
        let color = session.caches[0].composite.value(Channel::BaseColor, x, y);
        color[0] - color[1] > PAINTED_TINT
    };
    let half = TEXTURE_SIZE as u32 / 2;
    let count = |rows: std::ops::Range<u32>| rows.flat_map(|y| (0..TEXTURE_SIZE as u32).map(move |x| (x, y))).filter(|&(x, y)| painted(x, y)).count();
    (count(0..half), count(half..TEXTURE_SIZE as u32))
}

#[test]
fn a_mirror_copy_paints_exactly_where_its_stroke_does() {
    let mut session = session(primitives::sphere(1.0, 48, 24));
    execute(&mut session, "tool.brush", &json!({"size": 4, "symmetryY": true})).unwrap();

    // From the middle of the view to past the ball's right edge, above center, where it fades out; the copy below must fade the same way
    let points: Vec<serde_json::Value> = (0..40).map(|step| json!([200.0 + step as f32 * 6.0, 165.0])).collect();
    execute(&mut session, "tool.stroke", &json!({"points": points})).unwrap();
    session.update();

    let (stroke, copy) = painted_halves(&session);
    assert!(stroke > 0);
    // Within 4%: the ball's texture layout is not exactly symmetric texel for texel
    assert!(stroke.abs_diff(copy) * 25 <= stroke, "stroke {stroke} texels, copy {copy}");
}

#[test]
fn a_copy_paints_on_a_curved_mesh_across_a_moved_plane() {
    let mut session = session(primitives::sphere(1.0, 48, 24));
    execute(&mut session, "tool.brush", &json!({"symmetryX": true, "symmetryOffset": [0.1, 0.0, 0.0]})).unwrap();

    // Left of the view's center; across x = 0.1 its copy lands where the ball curves away a little more
    let stroke = surface(&session, 185.0, 200.0);
    execute(&mut session, "tool.stroke", &json!({"points": [[185, 200]]})).unwrap();
    session.update();

    let mirrored_x = 0.2 - stroke.x;
    let on_ball = Vec3::new(mirrored_x, stroke.y, (1.0 - mirrored_x * mirrored_x - stroke.y * stroke.y).sqrt());
    assert!(red_at(&session, stroke), "the stroke");
    assert!(red_at(&session, on_ball), "its copy on the ball across x = 0.1");
}

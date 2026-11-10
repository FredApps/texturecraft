use crate::{RayStart, VertexReach};
use texturecraft_geom::glam::Vec3;

/// Cage distances in world units; every case's ray runs down the Y axis from 0.5 to -0.25
const FRONTAL: f32 = 0.5;
const REAR: f32 = 0.25;

/// A frontal distance a cage-mesh start must ignore: its ray starts on the cage
const IGNORED_FRONTAL: f32 = 9.0;

/// How far past an end of the ray's range a vertex counts as outside it
const PAST_THE_END: f32 = 1e-3;

/// A vertex at `height` above the low-poly point at the origin, its triangles facing `facing`
fn vertex(start: RayStart, height: f32, facing: Vec3, name_matches: bool) -> VertexReach {
    VertexReach { low_position: Vec3::ZERO, start, high_position: Vec3::new(0.0, height, 0.0), high_facing: facing, name_matches }
}

#[test]
fn a_vertex_is_missed_outside_the_ray_range_on_another_part_or_facing_away() {
    // A painted cage amount of 2 doubles the frontal distance
    let starts = [
        (RayStart::Distance { up: Vec3::Y, scale: 1.0 }, FRONTAL),
        (RayStart::Distance { up: Vec3::Y, scale: 2.0 }, FRONTAL / 2.0),
        (RayStart::CageMesh(Vec3::new(0.0, -FRONTAL, 0.0)), IGNORED_FRONTAL),
    ];
    let (top, bottom) = (FRONTAL, -REAR);
    // Height, facing, name matches, ignore backface: missed
    let cases = [
        (top, Vec3::Y, true, false, false),
        (bottom, Vec3::Y, true, false, false),
        (0.0, Vec3::Y, true, false, false),
        (top + PAST_THE_END, Vec3::Y, true, false, true),
        (bottom - PAST_THE_END, Vec3::Y, true, false, true),
        (0.0, Vec3::Y, false, false, true),
        (0.0, Vec3::NEG_Y, true, false, false),
        (0.0, Vec3::NEG_Y, true, true, true),
        (0.0, Vec3::Y, true, true, false),
    ];

    for (start, frontal) in starts {
        for (height, facing, name_matches, ignore_backface, missed) in cases {
            let reach = vertex(start, height, facing, name_matches);
            assert_eq!(
                reach.missed(frontal, REAR, ignore_backface),
                missed,
                "{start:?} at height {height}, facing {facing}, name matches {name_matches}, ignore backface {ignore_backface}"
            );
        }
    }
}

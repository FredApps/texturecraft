use rayon::prelude::*;
use texturecraft_core::NormalMapCurvatureSettings;
use texturecraft_geom::glam::{Vec2, Vec3};
use texturecraft_geom::{Mesh, NO_TRI, UvGBuffer};

use super::TexelGrid;

/// Output slope per unit of normal turn per texel at a 2048-texel tile and Details 0.5, set by eye on a hard-surface bake
const SLOPE_AT_2K: f32 = 3.0;

/// How fast the baked world normal turns along the low poly's surface; positive curvature is convex
pub(crate) fn curvature_from_normal(
    world_normal: &[f32],
    mesh: &Mesh,
    material: usize,
    grid: &TexelGrid,
    settings: NormalMapCurvatureSettings,
    output: &mut [f32],
) {
    let TexelGrid { covered, size, tile_size } = *grid;
    let triangle_of_texel = triangle_of_each_texel(mesh, material, grid);
    let surface_directions = surface_direction_of_each_triangle(mesh, size);
    let normal_at = |index: usize| {
        let encoded = world_normal.get(index * 3..index * 3 + 3).and_then(|components| <[f32; 3]>::try_from(components).ok()).unwrap_or([0.5, 0.5, 1.0]);
        Vec3::from_array(encoded) * 2.0 - 1.0
    };

    // Turn of the normal per texel, measured along the surface direction each texel axis points to
    let turn_along = |axis: usize| -> Vec<f32> {
        (0..covered.len())
            .into_par_iter()
            .map(|index| {
                let Some(direction) =
                    triangle_of_texel.get(index).and_then(|triangle| surface_directions.get(*triangle as usize)).and_then(|pair| pair.get(axis).copied())
                else {
                    return 0.0;
                };

                let step = if axis == 0 { (1, 0) } else { (0, 1) };
                let forward = grid.neighbour(index, step.0, step.1);
                let backward = grid.neighbour(index, -step.0, -step.1);
                let change = match (forward, backward) {
                    (Some(ahead), Some(behind)) => (normal_at(ahead) - normal_at(behind)) * 0.5,
                    (Some(ahead), None) => normal_at(ahead) - normal_at(index),
                    (None, Some(behind)) => normal_at(index) - normal_at(behind),
                    (None, None) => Vec3::ZERO,
                };
                change.dot(direction)
            })
            .collect()
    };
    let (turn_along_x, turn_along_y) = (turn_along(0), turn_along(1));

    let slope = SLOPE_AT_2K * tile_size.0.min(tile_size.1) as f32 / 2048.0 * 2.0f32.powf((settings.details - 0.5) * 4.0);
    output.par_chunks_mut(size.0).enumerate().for_each(|(y, row)| {
        for (x, value) in row.iter_mut().enumerate() {
            let index = y * size.0 + x;
            if covered.get(index) != Some(&true) {
                continue;
            }

            // A 1-2-1 average across each derivative keeps lines equally wide in every direction
            let curvature = grid.average_along(&turn_along_x, index, (0, 1)) + grid.average_along(&turn_along_y, index, (1, 0));

            // Rises at `slope` near flat and rolls off softly, so strong edges never clip into flat white
            *value = 0.5 + 0.5 * (2.0 * slope * curvature).tanh();
        }
    });
}

/// Unit surface directions of a texel step in x and in y on each triangle of the low poly
fn surface_direction_of_each_triangle(mesh: &Mesh, size: (usize, usize)) -> Vec<[Vec3; 2]> {
    let texel_scale = Vec2::new(size.0 as f32, size.1 as f32);
    (0..mesh.indices.len() / 3)
        .map(|triangle| {
            let corner = |slot: usize| -> Option<(Vec3, Vec2)> {
                let vertex = *mesh.tri(triangle)?.get(slot)?;
                Some((*mesh.positions.get(vertex)?, *mesh.uvs.get(vertex)? * texel_scale))
            };
            let (Some((first_position, first_texel)), Some((second_position, second_texel)), Some((third_position, third_texel))) =
                (corner(0), corner(1), corner(2))
            else {
                return [Vec3::ZERO; 2];
            };

            let (position_edge_1, position_edge_2) = (second_position - first_position, third_position - first_position);
            let (texel_edge_1, texel_edge_2) = (second_texel - first_texel, third_texel - first_texel);
            let determinant = texel_edge_1.perp_dot(texel_edge_2);
            // A triangle with no UV area has no texel directions
            if determinant.abs() < 1e-12 {
                return [Vec3::ZERO; 2];
            }

            let along_x = (position_edge_1 * texel_edge_2.y - position_edge_2 * texel_edge_1.y) / determinant;
            let along_y = (position_edge_2 * texel_edge_1.x - position_edge_1 * texel_edge_2.x) / determinant;
            [along_x.normalize_or_zero(), along_y.normalize_or_zero()]
        })
        .collect()
}

/// Low-poly triangle under each covered texel; edge texels covered only by antialiasing borrow a neighbour's
fn triangle_of_each_texel(mesh: &Mesh, material: usize, grid: &TexelGrid) -> Vec<u32> {
    let centres = UvGBuffer::new(mesh, material, grid.size.0 as u32, grid.size.1 as u32).tri;
    (0..centres.len())
        .into_par_iter()
        .map(|index| {
            let own = centres.get(index).copied().unwrap_or(NO_TRI);
            if own != NO_TRI || grid.covered.get(index) != Some(&true) {
                return own;
            }

            [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, -1), (1, -1), (-1, 1)]
                .into_iter()
                .filter_map(|(step_x, step_y)| grid.neighbour(index, step_x, step_y))
                .map(|neighbour| centres.get(neighbour).copied().unwrap_or(NO_TRI))
                .find(|triangle| *triangle != NO_TRI)
                .unwrap_or(NO_TRI)
        })
        .collect()
}

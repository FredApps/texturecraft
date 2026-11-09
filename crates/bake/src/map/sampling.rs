use std::collections::HashMap;
use std::ops::{Add, Mul};
use texturecraft_geom::glam::Vec3;
use texturecraft_geom::{Mesh, Surface};

/// `values` (one per vertex) at `barycentric` in triangle `triangle`
pub(crate) fn interpolate<T: Copy + Mul<f32, Output = T> + Add<Output = T>>(mesh: &Mesh, values: &[T], triangle: u32, barycentric: [f32; 2]) -> Option<T> {
    let [first, second, third] = mesh.tri(triangle as usize)?;
    let [second_weight, third_weight] = barycentric;

    Some(*values.get(first)? * (1.0 - second_weight - third_weight) + *values.get(second)? * second_weight + *values.get(third)? * third_weight)
}

/// Vertices closer than this fraction of the mesh diagonal count as one when welding
const WELD_TOLERANCE_PER_DIAGONAL: f32 = 1e-5;

/// Vertices that share a position merged: each vertex's welded index, the welded positions and their averaged normals
pub(crate) fn welded(mesh: &Mesh, diagonal: f32) -> (Vec<u32>, Vec<Vec3>, Vec<Vec3>) {
    let tolerance = diagonal * WELD_TOLERANCE_PER_DIAGONAL;
    let mut welded_index_of_cell: HashMap<[i64; 3], u32> = HashMap::new();
    let (mut positions, mut normal_sums) = (Vec::new(), Vec::<Vec3>::new());

    let welded_indices: Vec<u32> = mesh
        .positions
        .iter()
        .zip(&mesh.normals)
        .map(|(position, normal)| {
            let cell = [position.x, position.y, position.z].map(|coordinate| (coordinate / tolerance).round() as i64);
            let welded_index = *welded_index_of_cell.entry(cell).or_insert_with(|| {
                positions.push(*position);
                normal_sums.push(Vec3::ZERO);
                (positions.len() - 1) as u32
            });
            if let Some(normal_sum) = normal_sums.get_mut(welded_index as usize) {
                *normal_sum += *normal;
            }
            welded_index
        })
        .collect();

    let normals = normal_sums.into_iter().map(|normal_sum| normal_sum.try_normalize().unwrap_or(Vec3::ZERO)).collect();
    (welded_indices, positions, normals)
}

pub(super) fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

pub(super) fn pack(v: Vec3) -> [f32; 3] {
    (v * 0.5 + 0.5).clamp(Vec3::ZERO, Vec3::ONE).to_array()
}

pub(super) fn generated_hue(part: usize) -> [f32; 3] {
    let h = ((part as f64 * 0.618_033_988_75 + 0.05).fract() * 6.0) as f32;
    let (s, v) = (0.7f32, 0.95f32);
    let f = h.fract();
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    match h as u32 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

pub(super) fn tangent_normal(s: &Surface, n: Vec3) -> [f32; 3] {
    let z = s.nrm;
    let t = (s.tangent.truncate() - z * z.dot(s.tangent.truncate())).try_normalize().unwrap_or_else(|| z.any_orthonormal_vector());
    let b = z.cross(t) * s.tangent.w;
    pack(Vec3::new(t.dot(n), b.dot(n), z.dot(n)))
}

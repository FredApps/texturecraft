use rayon::prelude::*;
use texturecraft_bake::HighPolyReach;

/// Per high-poly vertex: 1.0 if its owning set's bake misses it, otherwise 0.0
/// Distances are each set's (max frontal, max rear); filtered or unavailable sets leave their vertices unmarked
/// `vertex_count` is the validated high-poly mesh's vertex count
pub fn high_poly_missed(reach: &[Option<HighPolyReach>], distances: &[(f32, f32)], only_set: Option<usize>, vertex_count: usize) -> Vec<f32> {
    // Ownership is already assigned by HighPolyReach; other sets must not color the vertex
    let sets: Vec<(&HighPolyReach, f32, f32)> = reach
        .iter()
        .zip(distances)
        .enumerate()
        .filter(|(texture_set, _)| only_set.is_none_or(|only| only == *texture_set))
        .filter_map(|(_, (reach, &(max_frontal, max_rear)))| Some((reach.as_ref()?, max_frontal, max_rear)))
        .collect();
    (0..vertex_count)
        .into_par_iter()
        .map(|vertex| {
            let missed = sets.iter().find_map(|(reach, max_frontal, max_rear)| reach.missed(vertex, *max_frontal, *max_rear));
            f32::from(u8::from(missed.unwrap_or(false)))
        })
        .collect()
}

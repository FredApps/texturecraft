use std::collections::HashMap;
use texturecraft_geom::{Mesh, glam::Vec2};

pub(crate) fn fix_seams(mesh: &Mesh, material: usize, covered: &[bool], size: (usize, usize), intensity: f32, pixels: &mut [f32]) {
    if intensity <= 0.0 {
        return;
    }
    let (minimum, maximum) = mesh.bounds();
    let tolerance = (maximum - minimum).length().max(1e-6) * 1e-6;
    let position_key = |index: usize| mesh.positions.get(index).map(|position| position.to_array().map(|value| (value / tolerance).round() as i64));
    let mut edges = HashMap::new();
    let source = pixels.to_vec();
    let texel_scale = Vec2::new(size.0 as f32, size.1 as f32);
    let texel = |point: Vec2| {
        if point.x < 0.0 || point.y < 0.0 || point.x >= size.0 as f32 || point.y >= size.1 as f32 {
            return None;
        }
        let index = point.y.floor() as usize * size.0 + point.x.floor() as usize;
        (covered.get(index) == Some(&true)).then_some(index)
    };
    for triangle in mesh.triangles_of(material) {
        let Some(corners) = mesh.tri(triangle) else {
            continue;
        };
        for edge in 0..3 {
            let (Some(&first), Some(&second), Some(&third)) = (corners.get(edge), corners.get((edge + 1) % 3), corners.get((edge + 2) % 3)) else {
                continue;
            };
            let (Some(first_key), Some(second_key)) = (position_key(first), position_key(second)) else {
                continue;
            };
            let (first, second, key) =
                if first_key <= second_key { (first, second, (first_key, second_key)) } else { (second, first, (second_key, first_key)) };
            let (Some(start), Some(end), Some(interior)) = (mesh.uvs.get(first), mesh.uvs.get(second), mesh.uvs.get(third)) else {
                continue;
            };
            let (start, end, interior) = (*start * texel_scale, *end * texel_scale, *interior * texel_scale);
            let inward = (interior - (start + end) * 0.5).try_normalize().unwrap_or(Vec2::ZERO) * 0.75;
            let Some((other_start, other_end, other_inward)) = edges.insert(key, (start, end, inward)) else {
                continue;
            };
            if start.distance_squared(other_start) + end.distance_squared(other_end) < 0.25 {
                continue;
            }
            let steps = start.distance(end).max(other_start.distance(other_end)).ceil().clamp(1.0, size.0.max(size.1) as f32 * 2.0) as usize;
            for step in 0..=steps {
                let fraction = step as f32 / steps as f32;
                let (Some(first), Some(second)) = (texel(start.lerp(end, fraction) + inward), texel(other_start.lerp(other_end, fraction) + other_inward))
                else {
                    continue;
                };
                let (Some(&first_value), Some(&second_value)) = (source.get(first), source.get(second)) else {
                    continue;
                };
                let average = (first_value + second_value) * 0.5;
                for (index, value) in [(first, first_value), (second, second_value)] {
                    if let Some(destination) = pixels.get_mut(index) {
                        *destination = value + (average - value) * intensity;
                    }
                }
            }
        }
    }
}

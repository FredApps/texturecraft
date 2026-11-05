use std::collections::HashMap;
use std::ops::{Add, Mul};
use texturecraft_geom::glam::Vec3;
use texturecraft_geom::{Mesh, Surface};

pub(super) fn interpolate<T: Copy + Mul<f32, Output = T> + Add<Output = T>>(mesh: &Mesh, v: &[T], tri: u32, [b1, b2]: [f32; 2]) -> Option<T> {
    let [a, b, c] = mesh.tri(tri as usize)?;
    Some(*v.get(a)? * (1.0 - b1 - b2) + *v.get(b)? * b1 + *v.get(c)? * b2)
}

pub(super) fn welded(mesh: &Mesh, diag: f32) -> (Vec<u32>, Vec<Vec3>, Vec<Vec3>) {
    let q = diag * 1e-5;
    let mut ids: HashMap<[i64; 3], u32> = HashMap::new();
    let (mut pos, mut nrm) = (Vec::new(), Vec::<Vec3>::new());
    let weld: Vec<u32> = mesh
        .positions
        .iter()
        .zip(&mesh.normals)
        .map(|(p, n)| {
            let key = [p.x, p.y, p.z].map(|c| (c / q).round() as i64);
            let id = *ids.entry(key).or_insert_with(|| {
                pos.push(*p);
                nrm.push(Vec3::ZERO);
                (pos.len() - 1) as u32
            });
            if let Some(acc) = nrm.get_mut(id as usize) {
                *acc += *n;
            }
            id
        })
        .collect();
    let nrm = nrm.into_iter().map(|n| n.try_normalize().unwrap_or(Vec3::ZERO)).collect();
    (weld, pos, nrm)
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

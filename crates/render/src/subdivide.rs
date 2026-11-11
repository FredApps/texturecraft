//! The dense copy of the mesh that viewport displacement moves in the vertex shader (wgpu has no
//! tessellation stage): every triangle split into a regular grid of `4^level` triangles, the
//! attributes interpolated linearly (flat subdivision: the surface stays where it was until the
//! height channel moves it).

use bytemuck::{Pod, Zeroable};

/// The mesh vertex as the viewport draws it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub(crate) struct Vertex {
    pub pos: [f32; 3],
    pub nrm: [f32; 3],
    pub tan: [f32; 4],
    pub uv: [f32; 2],
}

/// An index range to draw: first index, count, material.
pub(crate) type Draw = (u32, u32, usize);

/// Most triangles of a dense mesh: the level is the highest that stays within it.
pub(crate) const DENSE_TRIANGLES: usize = 1 << 19;
/// Finest split: 64 × 64 segments per edge (for very coarse meshes such as a cube).
const MAX_LEVEL: u32 = 6;

/// The vertices of `mesh` as the viewport draws them (missing attributes get defaults).
pub(crate) fn vertices(mesh: &texturecraft_geom::Mesh) -> Vec<Vertex> {
    mesh.positions
        .iter()
        .enumerate()
        .map(|(i, p)| Vertex {
            pos: p.to_array(),
            nrm: mesh.normals.get(i).map(|n| n.to_array()).unwrap_or([0.0, 1.0, 0.0]),
            tan: mesh.tangents.get(i).map(|t| t.to_array()).unwrap_or([1.0, 0.0, 0.0, 1.0]),
            uv: mesh.uvs.get(i).map(|u| u.to_array()).unwrap_or([0.0, 0.0]),
        })
        .collect()
}

/// Subdivision level for a mesh of `triangles` (0: dense enough as it is).
pub(crate) fn level_for(triangles: usize) -> u32 {
    (0..=MAX_LEVEL).rev().find(|l| triangles.saturating_mul(1 << (2 * l)) <= DENSE_TRIANGLES).unwrap_or(0)
}

/// `w` of `a`, `b`, `c` per component.
fn mix<const N: usize>(a: [f32; N], b: [f32; N], c: [f32; N], w: [f32; 3]) -> [f32; N] {
    std::array::from_fn(|k| a[k] * w[0] + b[k] * w[1] + c[k] * w[2])
}

/// The point `k / n` of the way from vertex `p` to vertex `q`, with the same bits whichever way
/// round the edge is given (by position: vertices split at UV seams agree too).
fn on_edge(verts: &[Vertex], (p, q): (u32, u32), k: u32, n: u32) -> Option<Vertex> {
    let (p, q) = (*verts.get(p as usize)?, *verts.get(q as usize)?);
    let key = |v: &Vertex| v.pos.map(f32::to_bits);
    let (a, b, k) = if key(&p) <= key(&q) { (p, q, k) } else { (q, p, n - k) };
    if k == 0 || k == n {
        return Some(if k == 0 { a } else { b });
    }
    let t = k as f32 / n as f32;
    fn lerp<const N: usize>(x: [f32; N], y: [f32; N], t: f32) -> [f32; N] {
        std::array::from_fn(|i| x[i] + (y[i] - x[i]) * t)
    }
    Some(Vertex { pos: lerp(a.pos, b.pos, t), nrm: lerp(a.nrm, b.nrm, t), tan: lerp(a.tan, b.tan, t), uv: lerp(a.uv, b.uv, t) })
}

/// Split the triangles of `indices` (index ranges `draws`: first, count, material) `level`
/// times. Returns the vertices, indices and the draws over them.
pub(crate) fn subdivide(vertices: &[Vertex], indices: &[u32], draws: &[Draw], level: u32) -> (Vec<Vertex>, Vec<u32>, Vec<Draw>) {
    let segments = 1u32 << level.min(MAX_LEVEL);
    let (mut output_vertices, mut output_indices, mut output_draws) = (Vec::new(), Vec::new(), Vec::new());

    for &(start, count, material) in draws {
        let first = output_indices.len() as u32;
        let triangles = indices.get(start as usize..(start as usize).saturating_add(count as usize)).unwrap_or_default();
        for t in triangles.as_chunks::<3>().0 {
            let (Some(a), Some(b), Some(c)) = (vertices.get(t[0] as usize), vertices.get(t[1] as usize), vertices.get(t[2] as usize)) else { continue };
            let base = output_vertices.len() as u32;

            // Row j holds segments + 1 - j points inside the triangle
            // Shared edges use identical interpolation in both triangles to prevent cracks
            for j in 0..=segments {
                for i in 0..=segments - j {
                    let edge = match (i, j) {
                        (_, 0) => Some(((t[0], t[1]), i)),
                        (0, _) => Some(((t[0], t[2]), j)),
                        _ if i + j == segments => Some(((t[1], t[2]), j)),
                        _ => None,
                    };
                    let p = match edge.and_then(|(ends, k)| on_edge(vertices, ends, k, segments)) {
                        Some(p) => p,
                        None => {
                            let w = [1.0 - (i + j) as f32 / segments as f32, i as f32 / segments as f32, j as f32 / segments as f32];
                            Vertex {
                                pos: mix(a.pos, b.pos, c.pos, w),
                                nrm: mix(a.nrm, b.nrm, c.nrm, w),
                                tan: mix(a.tan, b.tan, c.tan, w),
                                uv: mix(a.uv, b.uv, c.uv, w),
                            }
                        }
                    };
                    output_vertices.push(Vertex { tan: [p.tan[0], p.tan[1], p.tan[2], a.tan[3]], ..p });
                }
            }

            let at = |i: u32, j: u32| base + j * (segments + 1) - j * j.saturating_sub(1) / 2 + i;
            for j in 0..segments {
                for i in 0..segments - j {
                    output_indices.extend_from_slice(&[at(i, j), at(i + 1, j), at(i, j + 1)]);
                    if i + j + 1 < segments {
                        output_indices.extend_from_slice(&[at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)]);
                    }
                }
            }
        }
        output_draws.push((first, output_indices.len() as u32 - first, material));
    }

    (output_vertices, output_indices, output_draws)
}

/// World length per unit of UV over the whole mesh (the square root of their area ratio): how
/// far a height change moves the surface for the slope height-to-normal shows.
pub(crate) fn world_per_uv(vertices: &[Vertex], indices: &[u32]) -> Option<f32> {
    let (mut world_area, mut uv_area) = (0.0f64, 0.0f64);
    for t in indices.as_chunks::<3>().0 {
        let (Some(a), Some(b), Some(c)) = (vertices.get(t[0] as usize), vertices.get(t[1] as usize), vertices.get(t[2] as usize)) else { continue };
        let d = |p: [f32; 3], q: [f32; 3]| [f64::from(q[0] - p[0]), f64::from(q[1] - p[1]), f64::from(q[2] - p[2])];
        let (e1, e2) = (d(a.pos, b.pos), d(a.pos, c.pos));
        let cross = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
        world_area += (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt() * 0.5;
        let (u1, u2) = ([b.uv[0] - a.uv[0], b.uv[1] - a.uv[1]], [c.uv[0] - a.uv[0], c.uv[1] - a.uv[1]]);
        uv_area += f64::from(u1[0] * u2[1] - u1[1] * u2[0]).abs() * 0.5;
    }

    let scale = (world_area / uv_area).sqrt() as f32;
    (uv_area > 1e-12 && scale.is_finite() && scale > 0.0).then_some(scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(pos: [f32; 3], uv: [f32; 2]) -> Vertex {
        Vertex { pos, nrm: [0.0, 0.0, 1.0], tan: [1.0, 0.0, 0.0, -1.0], uv }
    }

    #[test]
    fn grids_cover_each_triangle_with_four_per_level() {
        let verts = [v([0.0, 0.0, 0.0], [0.0, 0.0]), v([2.0, 0.0, 0.0], [1.0, 0.0]), v([0.0, 2.0, 0.0], [0.0, 1.0]), v([2.0, 2.0, 0.0], [1.0, 1.0])];
        let indices = [0, 1, 2, 1, 3, 2];
        for level in 0..=3 {
            let (vs, is, draws) = subdivide(&verts, &indices, &[(0, 3, 0), (3, 3, 1)], level);
            let per = 1usize << (2 * level);
            assert_eq!(draws, vec![(0, 3 * per as u32, 0), (3 * per as u32, 3 * per as u32, 1)]);
            assert_eq!(is.len(), 6 * per);
            assert!(is.iter().all(|i| (*i as usize) < vs.len()));
            // Same area, same winding (all +z), attributes interpolated.
            let area: f32 = is
                .chunks(3)
                .map(|t| {
                    let (a, b, c) = (vs[t[0] as usize].pos, vs[t[1] as usize].pos, vs[t[2] as usize].pos);
                    ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])) * 0.5
                })
                .inspect(|a| assert!(*a > 0.0))
                .sum();
            assert!((area - 4.0).abs() < 1e-4, "level {level}: area {area}");
            assert!(vs.iter().all(|x| (x.uv[0] * 2.0 - x.pos[0]).abs() < 1e-5 && x.tan[3] == -1.0));
            // The shared edge (vertices 1-2, given the other way round by the second triangle)
            // has the same points, to the bit, on both sides.
            let n = 1usize << level;
            let bits = |v: &Vertex| v.pos.map(f32::to_bits);
            let (first, second) = vs.split_at(vs.len() / 2);
            let shared = first.iter().filter(|p| second.iter().any(|q| bits(q) == bits(p))).count();
            assert_eq!(shared, n + 1, "level {level}");
        }
        let flipped = [0, 1, 2, 2, 1, 3];
        let (vs, _, _) = subdivide(&verts, &flipped, &[(0, 3, 0), (3, 3, 1)], 3);
        let bits = |v: &Vertex| v.pos.map(f32::to_bits);
        let (first, second) = vs.split_at(vs.len() / 2);
        assert_eq!(first.iter().filter(|p| second.iter().any(|q| bits(q) == bits(p))).count(), 9);
        assert_eq!(level_for(12), 6);
        assert_eq!(level_for(DENSE_TRIANGLES), 0);
        assert_eq!(level_for(DENSE_TRIANGLES / 4), 1);
        assert_eq!(world_per_uv(&verts, &indices), Some(2.0));
        assert_eq!(world_per_uv(&verts, &[]), None);
    }
}

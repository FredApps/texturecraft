use super::{RayFrame, RayStart};
use crate::{BakeError, BakeRequest, MeshExtent};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use texturecraft_geom::glam::Vec3;
use texturecraft_geom::surface_at;

/// Barycentric coordinates of a triangle's three corners
pub(crate) const CORNERS: [[f32; 2]; 3] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];

/// The cage of one texture set as the bake sees it: where its transfer rays start at each corner of the set's triangles
#[derive(Clone, Debug, Default)]
pub struct CageShell {
    /// Low-poly position of each triangle corner, three per triangle
    pub positions: Vec<Vec3>,
    pub starts: Vec<RayStart>,
    /// Corner pairs along the low poly's polygon edges, for the wireframe
    pub edges: Vec<[u32; 2]>,
    /// World units per unit of the max frontal distance setting
    pub cage_units: f32,
}

impl CageShell {
    /// The cage of `request`'s texture set, from the same ray starts the bake uses
    pub fn new(request: &BakeRequest) -> Result<CageShell, BakeError> {
        let ray_frame = RayFrame::new(request, MeshExtent::of(request.mesh)?)?;
        let (frame, mesh) = (&ray_frame, request.mesh);
        let triangles: Vec<usize> = mesh.triangles_of(request.material).collect();
        let (positions, starts): (Vec<Vec3>, Vec<RayStart>) = triangles
            .par_iter()
            .flat_map_iter(|&triangle| {
                CORNERS.into_iter().map(move |barycentric| match surface_at(mesh, triangle as u32, barycentric[0], barycentric[1]) {
                    Some(surface) => (surface.pos, frame.start(&surface, barycentric)),
                    None => (Vec3::ZERO, RayStart::Distance { up: Vec3::ZERO, scale: 0.0 }),
                })
            })
            .unzip();

        let wire_edges: HashSet<(u32, u32)> = mesh.wire_edges().into_iter().nth(request.material).unwrap_or_default().into_iter().collect();
        let mut edge_corners = HashMap::new();
        for (triangle_index, &triangle) in triangles.iter().enumerate() {
            let Some(vertices) = mesh.tri(triangle) else { continue };

            for corner in 0..3 {
                let next = (corner + 1) % 3;
                let (from, to) = (vertices[corner] as u32, vertices[next] as u32);
                let key = (from.min(to), from.max(to));
                if wire_edges.contains(&key) {
                    let first_corner = (triangle_index * 3) as u32;
                    edge_corners.entry(key).or_insert([first_corner + corner as u32, first_corner + next as u32]);
                }
            }
        }
        let mut edges: Vec<[u32; 2]> = edge_corners.into_values().collect();
        edges.sort_unstable();

        Ok(CageShell { positions, starts, edges, cage_units: ray_frame.cage_units })
    }

    /// Each corner's cage position at `max_frontal`, the setting before relative-to-bounding-box scaling
    pub fn corners(&self, max_frontal: f32) -> Vec<Vec3> {
        let frontal = max_frontal * self.cage_units;
        self.positions.par_iter().zip(&self.starts).map(|(position, start)| start.origin(*position, frontal)).collect()
    }
}

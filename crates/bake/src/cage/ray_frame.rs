use super::RayStart;
use crate::MeshExtent;
use crate::map::{interpolate, welded};
use crate::skew::PreparedSkew;
use crate::{BakeError, BakePaintMap, BakeRequest};
use texturecraft_core::CageType;
use texturecraft_geom::glam::Vec3;
use texturecraft_geom::{Mesh, Surface};

/// What decides where transfer rays start: the cage mesh or the averaged normals, skew and painted cage amount
pub(crate) struct RayFrame<'a> {
    mesh: &'a Mesh,
    ray_normals: Vec<Vec3>,
    cage: Option<&'a Mesh>,
    skew: Option<PreparedSkew<'a>>,
    cage_paint: Option<&'a BakePaintMap>,
    /// Cage-file offsets this short count as none: the ray starts from the distance-based cage instead
    ray_epsilon: f32,
    /// World units per unit of the cage distance settings
    pub(crate) cage_units: f32,
}

impl<'a> RayFrame<'a> {
    pub(crate) fn new(request: &BakeRequest<'a>, extent: MeshExtent) -> Result<RayFrame<'a>, BakeError> {
        let settings = request.settings;
        let ray_normals = match settings.cage.average_normals && request.high.is_some() {
            true => {
                let (welded_indices, _, normals) = welded(request.mesh, extent.diagonal);
                welded_indices.iter().map(|index| normals.get(*index as usize).copied().unwrap_or(Vec3::Y)).collect()
            }
            false => Vec::new(),
        };

        Ok(RayFrame {
            mesh: request.mesh,
            ray_normals,
            cage: if settings.cage.kind == CageType::CustomFile { request.cage } else { None },
            skew: request.skew.map(|skew_map| PreparedSkew::new(skew_map, request.mesh, extent.scale, settings.skew)).transpose()?,
            cage_paint: request.cage_paint,
            ray_epsilon: extent.ray_epsilon(),
            cage_units: if settings.cage.relative_to_bounding_box { extent.scale } else { 1.0 },
        })
    }

    /// Where the transfer ray of the low-poly point `surface` at `barycentric` starts
    pub(crate) fn start(&self, surface: &Surface, barycentric: [f32; 2]) -> RayStart {
        let cage_offset = self
            .cage
            .and_then(|cage_mesh| interpolate(cage_mesh, &cage_mesh.positions, surface.tri, barycentric))
            .map(|cage_position| surface.pos - cage_position)
            .filter(|offset| offset.length() > self.ray_epsilon);
        if let Some(offset) = cage_offset {
            return RayStart::CageMesh(offset);
        }

        let averaged_normal = interpolate(self.mesh, &self.ray_normals, surface.tri, barycentric).and_then(Vec3::try_normalize).unwrap_or(surface.nrm);
        let up = match &self.skew {
            Some(skew) => skew.correct(self.mesh, surface, averaged_normal),
            None => averaged_normal,
        };
        RayStart::Distance { up, scale: self.cage_paint.map_or(1.0, |map| map.sample(surface.uv)) }
    }
}

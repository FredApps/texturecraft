use super::{ANY, Hemisphere, Query, Ray, RayScene, TraceMesh};
use crate::BakeError;
use rayon::prelude::*;
use texturecraft_geom::Hit;

pub(super) struct CpuScene<'a>(pub(super) Vec<TraceMesh<'a>>);

impl CpuScene<'_> {
    fn mesh(&self, index: usize) -> Result<&TraceMesh<'_>, BakeError> {
        self.0.get(index).ok_or_else(|| BakeError::Invalid(format!("no scene mesh {index}")))
    }
}

impl RayScene for CpuScene<'_> {
    fn closest(&mut self, mesh: usize, rays: &[Ray], ignore_backface: bool) -> Result<Vec<Option<Hit>>, BakeError> {
        let trace_mesh = self.mesh(mesh)?;
        Ok(rays
            .par_iter()
            .map(|ray| {
                if ray.key == ANY && !ignore_backface {
                    trace_mesh.bvh.closest(ray.origin, ray.direction, ray.max_distance)
                } else {
                    trace_mesh.bvh.closest_filtered(ray.origin, ray.direction, ray.max_distance, &|triangle| {
                        trace_mesh.key_matches(triangle, ray.key) && !(ignore_backface && trace_mesh.back_facing(triangle, ray.direction))
                    })
                }
            })
            .collect())
    }

    fn hemispheres(&mut self, passes: &[Hemisphere], queries: &[Query]) -> Result<Vec<Vec<[f32; 4]>>, BakeError> {
        passes.iter().map(|hemisphere| queries.par_iter().map(|query| Ok(hemisphere.evaluate(self.mesh(query.mesh as usize)?, query))).collect()).collect()
    }
}

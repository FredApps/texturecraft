use super::{Hemisphere, Query, Ray};
use crate::BakeError;
use texturecraft_geom::Hit;

pub trait RayScene: Send {
    fn closest(&mut self, mesh: usize, rays: &[Ray], ignore_backface: bool) -> Result<Vec<Option<Hit>>, BakeError>;
    fn hemispheres(&mut self, passes: &[Hemisphere], queries: &[Query]) -> Result<Vec<Vec<[f32; 4]>>, BakeError>;
}

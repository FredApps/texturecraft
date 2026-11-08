use super::{RayScene, TraceMesh};
use crate::BakeError;

pub trait Tracer: Send + Sync {
    fn name(&self) -> String;
    fn scene<'a>(&'a self, meshes: &[TraceMesh<'a>]) -> Result<Box<dyn RayScene + 'a>, BakeError>;
}

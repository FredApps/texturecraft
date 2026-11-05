use super::cpu_scene::CpuScene;
use super::{RayScene, TraceMesh, Tracer};
use crate::BakeError;

#[derive(Clone, Copy, Debug, Default)]
pub struct CpuTracer;

impl Tracer for CpuTracer {
    fn name(&self) -> String {
        "CPU".into()
    }

    fn scene<'a>(&'a self, meshes: &[TraceMesh<'a>]) -> Result<Box<dyn RayScene + 'a>, BakeError> {
        Ok(Box::new(CpuScene(meshes.to_vec())))
    }
}

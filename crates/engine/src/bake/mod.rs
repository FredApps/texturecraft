mod bake_mesh;
mod bake_meshes;
mod bake_task;
mod merged_high;

pub use bake_mesh::BakeMesh;
pub use bake_meshes::BakeMeshes;
pub use bake_task::BakeTask;
pub use merged_high::MergedHigh;

type MaterialColors = std::sync::Arc<Vec<Option<std::sync::Arc<texturecraft_geom::BaseColor>>>>;

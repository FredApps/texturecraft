//! Baking tasks, commands, settings helpers, and the mesh-map baking API

mod bake_cage_inputs;
mod bake_mesh;
mod bake_meshes;
mod bake_task;
mod high_poly_missed;
mod layout;
mod merged_high;
mod mesh_map_command;
mod model_patch;
mod paint_command;
mod preserve_tiles;
mod streamed_bake;
mod streamed_map;
mod tile_command;

pub use bake_cage_inputs::BakeCageInputs;
pub use bake_mesh::BakeMesh;
pub use bake_meshes::BakeMeshes;
pub use bake_task::BakeTask;
pub use high_poly_missed::high_poly_missed;
pub use merged_high::MergedHigh;
pub use mesh_map_command::{install_gpu_tracer, meshes_json, settings_params};
pub use model_patch::settings_patch;
pub use streamed_bake::StreamedBake;
pub use streamed_map::StreamedMap;
pub use texturecraft_bake::*;

use crate::CommandSpec;

type MaterialColors = std::sync::Arc<Vec<Option<std::sync::Arc<texturecraft_geom::BaseColor>>>>;

pub(crate) fn specs() -> Vec<CommandSpec> {
    let mut commands = mesh_map_command::specs();
    commands.extend(paint_command::specs());
    commands.extend(tile_command::specs());
    commands
}

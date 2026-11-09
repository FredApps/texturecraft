//! Keeps the GPU's copy of bake mode's visualization (high poly, cages, UV seams) in step with the session

mod bake_scene_link;
mod cage_link;
mod cage_shell_key;
mod cage_upload;
mod reach_link;
mod shared_handle;

pub use bake_scene_link::BakeSceneLink;
pub use cage_upload::CageUpload;

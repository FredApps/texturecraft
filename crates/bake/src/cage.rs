//! Where the bake's transfer rays start, shared by the bake and the cage the 3D view draws

mod cage_shell;
mod high_poly_reach;
mod ray_frame;
mod ray_start;
mod vertex_reach;

#[cfg(test)]
pub(crate) use cage_shell::CORNERS;
pub use cage_shell::CageShell;
pub use high_poly_reach::HighPolyReach;
pub(crate) use ray_frame::RayFrame;
pub use ray_start::RayStart;
pub use vertex_reach::VertexReach;

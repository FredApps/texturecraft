use texturecraft_geom::glam::Vec3;

/// GPU buffer layout; batches upload without repacking
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Ray {
    pub origin: Vec3,
    pub max_distance: f32,
    pub direction: Vec3,
    pub key: u32,
}

use texturecraft_geom::glam::Vec3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Query {
    pub pos: Vec3,
    pub nrm: Vec3,
    pub geo: Vec3,
    pub rot: [f32; 2],
    pub key: u32,
    pub mesh: u32,
}

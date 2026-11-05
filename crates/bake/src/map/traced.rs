use texturecraft_geom::glam::Vec3;

pub(crate) struct Traced {
    pub ao: f32,
    pub bent: Option<Vec3>,
    pub thickness: f32,
    pub curvature: f32,
}

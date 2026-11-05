use texturecraft_geom::Surface;

pub(crate) struct Transfer {
    pub(super) surface: Surface,
    pub(super) bary: [f32; 2],
    pub(super) t: f32,
    pub(super) front: f32,
}

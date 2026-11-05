use texturecraft_geom::Surface;

pub(crate) struct Sample {
    pub x: usize,
    pub surface: Surface,
    pub bary: [f32; 2],
    pub seed: u64,
}

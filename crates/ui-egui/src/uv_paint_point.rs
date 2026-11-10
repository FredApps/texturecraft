//! A finished stroke's endpoint in one mesh's UV layout

use texturecraft_engine::{Session, geom::glam::Vec2};

/// A line anchor scoped to its project, UV layout and texture set
#[derive(Clone, Copy, Debug)]
pub struct UvPaintPoint {
    mesh_id: u64,
    texture_set: usize,
    uv: Vec2,
}

impl UvPaintPoint {
    pub(crate) fn new(session: &Session, uv: Vec2) -> Self {
        Self { mesh_id: session.mesh_id, texture_set: session.ts(), uv }
    }

    pub(crate) fn uv_for(self, session: &Session) -> Option<Vec2> {
        (self.mesh_id == session.mesh_id && self.texture_set == session.ts()).then_some(self.uv)
    }
}

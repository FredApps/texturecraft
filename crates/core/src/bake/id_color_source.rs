use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum IdColorSource {
    VertexColor,
    #[default]
    MaterialColor,
    FileId,
    MeshId,
    Submesh,
}

use serde::{Deserialize, Serialize};

/// Mesh filtering for occlusion, thickness and curvature rays
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SelfOcclusion {
    /// Every part occludes every other
    #[default]
    Always,
    /// Only parts with the same mesh name (suffixes removed) occlude each other
    SameMeshName,
}

use serde::{Deserialize, Serialize};

/// How secondary rays spread over their cone
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RayDistribution {
    /// Cosine-weighted: more rays near the normal
    #[default]
    Cosine,
    /// Uniform over the cone's solid angle
    Uniform,
}

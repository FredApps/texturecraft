#[derive(Debug, thiserror::Error)]
pub enum BakeError {
    #[error("bake cancelled")]
    Cancelled,
    #[error("invalid bake request: {0}")]
    Invalid(String),
    #[error("ray tracer: {0}")]
    Tracer(String),
}

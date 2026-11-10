use super::source::Source;
use crate::BakeRequest;

/// The bake's mesh-name key of each low-poly and each high-poly triangle: with By Mesh Name a transfer ray from a low-poly triangle only hits high-poly triangles with its key
pub(crate) fn transfer_keys(request: &BakeRequest) -> (Vec<u32>, Vec<u32>) {
    let (low, high) = Source::of_request(request);
    (low.key, high.map(|high| high.key).unwrap_or_default())
}

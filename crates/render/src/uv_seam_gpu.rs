/// The low poly's UV seams as a line list over its vertex buffer
pub(crate) struct UvSeamGpu {
    /// The low-poly mesh the lines belong to
    pub(crate) mesh_id: u64,
    pub(crate) lines: wgpu::Buffer,
    /// (first index, count, material) per material
    pub(crate) line_draws: Vec<(u32, u32, usize)>,
}

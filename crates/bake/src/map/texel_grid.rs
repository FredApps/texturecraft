/// Covered texels of a baked map, with UV tiles that neighbours never cross
#[derive(Clone, Copy)]
pub(crate) struct TexelGrid<'a> {
    pub covered: &'a [bool],
    pub size: (usize, usize),
    pub tile_size: (usize, usize),
}

impl TexelGrid<'_> {
    /// The covered texel one step away in the same tile, if any
    pub fn neighbour(&self, index: usize, step_x: isize, step_y: isize) -> Option<usize> {
        let (width, height) = self.size;
        let (x, y) = (index % width.max(1), index / width.max(1));
        let (next_x, next_y) = (x.checked_add_signed(step_x)?, y.checked_add_signed(step_y)?);
        if next_x >= width || next_y >= height {
            return None;
        }

        let same_tile = next_x / self.tile_size.0.max(1) == x / self.tile_size.0.max(1) && next_y / self.tile_size.1.max(1) == y / self.tile_size.1.max(1);
        let next = next_y * width + next_x;
        (same_tile && self.covered.get(next) == Some(&true)).then_some(next)
    }

    /// A 1-2-1 weighted average of `values` along `step`, over covered texels only
    pub fn average_along(&self, values: &[f32], index: usize, step: (isize, isize)) -> f32 {
        let centre = values.get(index).copied().unwrap_or(0.0);
        let sides = [self.neighbour(index, step.0, step.1), self.neighbour(index, -step.0, -step.1)];
        let (sum, weight) =
            sides.into_iter().flatten().fold((2.0 * centre, 2.0), |(sum, weight), side| (sum + values.get(side).copied().unwrap_or(0.0), weight + 1.0));
        sum / weight
    }
}

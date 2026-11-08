//! UV padding: fill uncovered texels from their nearest covered texel.

/// No source texel.
pub const NONE: u32 = u32::MAX;

/// For every texel, the index of the covered texel whose value it takes (itself when covered,
/// [`NONE`] when beyond the padding distance). Computed once per UV layout and reused for every
/// dilation of that texture set.
#[derive(Clone, Debug, PartialEq)]
pub struct DilationMap {
    pub width: u32,
    pub height: u32,
    pub src: Vec<u32>,
}

impl DilationMap {
    /// `covered.len()` must be `width × height` (shorter input is treated as uncovered).
    /// `max_dist` limits padding in texels (Chebyshev steps); `None` fills everything reachable.
    pub fn new(covered: &[bool], width: u32, height: u32, max_dist: Option<u32>) -> DilationMap {
        DilationMap::in_blocks(covered, width, height, max_dist, 0)
    }

    /// [`DilationMap::new`] that never pads across the borders of `block × block` squares (the
    /// UV tiles of an atlas); 0 means one block.
    pub fn in_blocks(covered: &[bool], width: u32, height: u32, max_dist: Option<u32>, block: u32) -> DilationMap {
        Self::in_rectangular_blocks(covered, width, height, max_dist, (block, block))
    }

    pub fn in_rectangular_blocks(covered: &[bool], width: u32, height: u32, max_dist: Option<u32>, block: (u32, u32)) -> DilationMap {
        let n = (width as usize).saturating_mul(height as usize);
        let mut src = vec![NONE; n];
        let bsz = (if block.0 == 0 { u32::MAX } else { block.0 }, if block.1 == 0 { u32::MAX } else { block.1 });
        let is_covered = |i: usize| covered.get(i).copied().unwrap_or(false);
        // Breadth first, one distance at a time; only island borders seed it (interior texels
        // have nothing to spread to, so the result is the same at a fraction of the work).
        let mut frontier = Vec::new();
        for i in 0..n {
            if is_covered(i) {
                if let Some(s) = src.get_mut(i) {
                    *s = i as u32;
                }
                if neighbours(i, width, height, bsz).any(|j| !is_covered(j)) {
                    frontier.push(i);
                }
            }
        }
        let limit = max_dist.unwrap_or(u32::MAX);
        let mut d = 0;
        while d < limit && !frontier.is_empty() {
            let mut next = Vec::new();
            for &i in &frontier {
                let from = src.get(i).copied().unwrap_or(NONE);
                for j in neighbours(i, width, height, bsz) {
                    if let Some(s) = src.get_mut(j).filter(|s| **s == NONE) {
                        *s = from;
                        next.push(j);
                    }
                }
            }
            frontier = next;
            d += 1;
        }
        DilationMap { width, height, src }
    }

    /// Copy padded values in place: `data` holds `comps` components per texel. Texels with no
    /// source keep their value.
    pub fn apply<T: Copy>(&self, data: &mut [T], comps: usize) {
        for (i, &s) in self.src.iter().enumerate() {
            let s = s as usize;
            if s == i || s == NONE as usize {
                continue;
            }
            for c in 0..comps {
                if let Some(v) = data.get(s * comps + c).copied()
                    && let Some(d) = data.get_mut(i * comps + c)
                {
                    *d = v;
                }
            }
        }
    }

    /// The source texel of (x, y), if any.
    pub fn source(&self, x: u32, y: u32) -> Option<(u32, u32)> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let s = *self.src.get((y * self.width + x) as usize)?;
        (s != NONE).then(|| (s % self.width, s / self.width))
    }
}

/// The 8-neighbours of texel `i` of a `w × h` image inside the image and inside its `bsz`
/// square block.
fn neighbours(i: usize, w: u32, h: u32, bsz: (u32, u32)) -> impl Iterator<Item = usize> {
    let (wu, x, y) = (w as usize, (i % w.max(1) as usize) as u32, (i / w.max(1) as usize) as u32);
    let left = x > 0 && x % bsz.0 != 0;
    let right = x + 1 < w && (x + 1) % bsz.0 != 0;
    let up = y > 0 && y % bsz.1 != 0;
    let down = y + 1 < h && (y + 1) % bsz.1 != 0;
    [
        (left, i.wrapping_sub(1)),
        (right, i + 1),
        (up, i.wrapping_sub(wu)),
        (down, i + wu),
        (left && up, i.wrapping_sub(wu + 1)),
        (right && up, i.wrapping_sub(wu).wrapping_add(1)),
        (left && down, (i + wu).wrapping_sub(1)),
        (right && down, i + wu + 1),
    ]
    .into_iter()
    .filter_map(|(ok, j)| ok.then_some(j))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uncovered_texels_take_the_nearest_covered_value() {
        // 5×1 strip, only texel 1 covered.
        let covered = [false, true, false, false, false];
        let m = DilationMap::new(&covered, 5, 1, Some(2));
        assert_eq!(m.src, vec![1, 1, 1, 1, NONE]);
        let mut data = [0u8, 9, 0, 0, 0];
        m.apply(&mut data, 1);
        assert_eq!(data, [9, 9, 9, 9, 0]);
    }

    #[test]
    fn infinite_padding_fills_everything_and_keeps_covered_values() {
        let covered = [true, false, false, false, false, false, false, false, true];
        let m = DilationMap::new(&covered, 3, 3, None);
        assert!(m.src.iter().all(|s| *s != NONE));
        assert_eq!(m.src[0], 0);
        assert_eq!(m.src[8], 8);
        assert_eq!(m.source(2, 2), Some((2, 2)));
        assert_eq!(m.source(9, 9), None);
    }

    #[test]
    fn blocks_never_pad_across_their_borders() {
        // 4×1 strip of two 2-texel blocks, only texel 0 covered.
        let covered = [true, false, false, false];
        assert_eq!(DilationMap::in_blocks(&covered, 4, 1, None, 2).src, vec![0, 0, NONE, NONE]);
        assert_eq!(DilationMap::new(&covered, 4, 1, None).src, vec![0; 4]);
    }

    /// The plain multi-source BFS (every covered texel seeds, one queue).
    fn reference(covered: &[bool], width: u32, height: u32, max_dist: Option<u32>, block: u32) -> Vec<u32> {
        let n = (width * height) as usize;
        let (mut src, mut dist) = (vec![NONE; n], vec![u32::MAX; n]);
        let mut queue = std::collections::VecDeque::new();
        for i in (0..n).filter(|i| covered[*i]) {
            src[i] = i as u32;
            dist[i] = 0;
            queue.push_back(i);
        }
        let (w, h, b) = (width as i64, height as i64, if block == 0 { i64::MAX } else { block as i64 });
        while let Some(i) = queue.pop_front() {
            if dist[i] >= max_dist.unwrap_or(u32::MAX) {
                continue;
            }
            let (x, y) = (i as i64 % w, i as i64 / w);
            for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)] {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx >= w || ny >= h || nx / b != x / b || ny / b != y / b {
                    continue;
                }
                let j = (ny * w + nx) as usize;
                if dist[j] == u32::MAX {
                    dist[j] = dist[i] + 1;
                    src[j] = src[i];
                    queue.push_back(j);
                }
            }
        }
        src
    }

    #[test]
    fn matches_the_plain_breadth_first_search() {
        let (w, h) = (40, 24);
        let mut seed = 7u32;
        let covered: Vec<bool> = (0..w * h)
            .map(|i| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                // Blobs: a coarse random field.
                let (x, y) = (i % w, i / w);
                (x / 5 + y / 3 + (seed >> 29)).is_multiple_of(4)
            })
            .collect();
        for (max_dist, block) in [(None, 0), (Some(3), 0), (Some(2), 8), (None, 16), (Some(1), 5)] {
            assert_eq!(DilationMap::in_blocks(&covered, w, h, max_dist, block).src, reference(&covered, w, h, max_dist, block), "{max_dist:?} {block}");
        }
    }

    #[test]
    fn nothing_covered_is_a_no_op() {
        let m = DilationMap::new(&[], 4, 4, None);
        let mut data = [3u16; 16];
        m.apply(&mut data, 1);
        assert_eq!(data, [3u16; 16]);
    }
}

#[cfg(test)]
mod rectangular_test {
    use super::*;
    #[test]
    fn rectangular_tiles_keep_padding_within_both_axes() {
        let mut covered = vec![false; 8 * 6];
        covered[2 * 8 + 3] = true;
        let padding = DilationMap::in_rectangular_blocks(&covered, 8, 6, None, (4, 3));
        for y in 0..6 {
            for x in 0..8 {
                assert_eq!(padding.source(x, y).is_some(), x < 4 && y < 3);
            }
        }
    }
}

use std::sync::Arc;

use rayon::prelude::*;
use texturecraft_bake::BakePaintMap;
use texturecraft_core::TextureSet;
use texturecraft_geom::glam::Vec2;

use crate::{EngineError, Result};

/// Correction paint stays in working coordinates; the worker reads a uniform bake atlas
pub(super) fn paint_for_bake(painted: &Arc<BakePaintMap>, set: &TextureSet) -> Result<Arc<BakePaintMap>> {
    let Some(grid) = set.tile_grid().filter(|_| !set.tile_resolutions.is_empty()) else {
        return Ok(painted.clone());
    };

    painted.validate().map_err(|error| EngineError::Other(error.to_string()))?;
    let (width, height) = (painted.width, painted.height);
    let mut values = vec![0.0; painted.values.len()];
    values.par_chunks_mut(width as usize).enumerate().for_each(|(y, row)| {
        for (x, value) in row.iter_mut().enumerate() {
            let uv = grid.from_atlas([(x as f32 + 0.5) / width as f32, (y as f32 + 0.5) / height as f32]);
            *value = painted.sample(Vec2::from_array(set.to_atlas(&grid, uv, uv)));
        }
    });
    Ok(Arc::new(BakePaintMap { width, height, values }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correction_paint_follows_the_same_surface_in_bake_coordinates() {
        let mut set = TextureSet::new("Tiles", 0, 256);
        set.uv_tiles = vec![1001, 1002];
        set.tile_resolutions.insert(1002, 128);
        let mut paint = BakePaintMap::constant(0.0).resized(128, 64).unwrap();
        paint.paint(&[Vec2::new(0.625, 0.25)], 0.05, 1.0, 1.0, 1.0).unwrap();
        let paint = Arc::new(paint);
        let baked = paint_for_bake(&paint, &set).unwrap();
        assert!(paint.sample(Vec2::new(0.625, 0.25)) > 0.99);
        assert!(baked.sample(Vec2::new(0.75, 0.5)) > 0.99);
        assert_eq!(baked.sample(Vec2::new(0.25, 0.5)), 0.0);
        assert_eq!(paint.sample(Vec2::new(0.75, 0.5)), 0.0);
    }
}

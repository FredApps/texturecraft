use texturecraft_texel::DilationMap;

pub(crate) fn diffuse(pixels: &mut [f32], components: usize, padding: &DilationMap, tile_size: (usize, usize)) {
    let width = padding.width as usize;
    let height = padding.height as usize;
    for start_y in (0..height).step_by(tile_size.1) {
        for start_x in (0..width).step_by(tile_size.0) {
            let tile_width = tile_size.0.min(width - start_x);
            let tile_height = tile_size.1.min(height - start_y);
            let mut values = vec![0.0; tile_width * tile_height * components];
            let mut valid = vec![false; tile_width * tile_height];
            for (index, covered) in valid.iter_mut().enumerate() {
                let source = (start_y + index / tile_width) * width + start_x + index % tile_width;
                *covered = padding.src.get(source).is_some_and(|source| *source != u32::MAX);
                if let (Some(destination), Some(pixel)) =
                    (values.get_mut(index * components..(index + 1) * components), pixels.get(source * components..(source + 1) * components))
                {
                    destination.copy_from_slice(pixel);
                }
            }
            if !valid.iter().any(|covered| *covered) {
                continue;
            }
            fill(&mut values, &valid, tile_width, tile_height, components);
            for (index, covered) in valid.iter().enumerate() {
                if *covered {
                    continue;
                }
                let destination = (start_y + index / tile_width) * width + start_x + index % tile_width;
                if let (Some(pixel), Some(value)) =
                    (pixels.get_mut(destination * components..(destination + 1) * components), values.get(index * components..(index + 1) * components))
                {
                    pixel.copy_from_slice(value);
                }
            }
        }
    }
}

fn fill(values: &mut [f32], valid: &[bool], width: usize, height: usize, components: usize) {
    if width == 1 && height == 1 {
        return;
    }
    let (small_width, small_height) = (width.div_ceil(2), height.div_ceil(2));
    let mut small = vec![0.0; small_width * small_height * components];
    let mut small_valid = vec![false; small_width * small_height];
    for (index, covered) in small_valid.iter_mut().enumerate() {
        let (x, y) = (index % small_width * 2, index / small_width * 2);
        let mut count = 0;
        for child_y in y..(y + 2).min(height) {
            for child_x in x..(x + 2).min(width) {
                let child = child_y * width + child_x;
                if valid.get(child) != Some(&true) {
                    continue;
                }
                count += 1;
                for component in 0..components {
                    if let Some(value) = small.get_mut(index * components + component) {
                        *value += values.get(child * components + component).copied().unwrap_or(0.0);
                    }
                }
            }
        }
        *covered = count > 0;
        if let Some(pixel) = small.get_mut(index * components..(index + 1) * components) {
            pixel.iter_mut().for_each(|value| *value /= count.max(1) as f32);
        }
    }
    fill(&mut small, &small_valid, small_width, small_height, components);
    for (index, covered) in valid.iter().enumerate() {
        if *covered {
            continue;
        }
        let (x, y) = ((index % width) as f32 * 0.5 - 0.25, (index / width) as f32 * 0.5 - 0.25);
        let (fraction_x, fraction_y) = (x - x.floor(), y - y.floor());
        let sample = |offset_x: i32, offset_y: i32, component: usize| {
            let sample_x = (x.floor() as i32 + offset_x).clamp(0, small_width as i32 - 1) as usize;
            let sample_y = (y.floor() as i32 + offset_y).clamp(0, small_height as i32 - 1) as usize;
            small.get((sample_y * small_width + sample_x) * components + component).copied().unwrap_or(0.0)
        };
        for component in 0..components {
            let top = sample(0, 0, component) * (1.0 - fraction_x) + sample(1, 0, component) * fraction_x;
            let bottom = sample(0, 1, component) * (1.0 - fraction_x) + sample(1, 1, component) * fraction_x;
            if let Some(value) = values.get_mut(index * components + component) {
                *value = top * (1.0 - fraction_y) + bottom * fraction_y;
            }
        }
    }
}

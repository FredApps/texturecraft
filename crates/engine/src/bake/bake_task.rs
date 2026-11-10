use super::{MergedHigh, StreamedBake, StreamedMap};
use crate::{EngineError, Result};
use std::sync::Arc;
use texturecraft_bake::{BakeOutput, BakePaintMap, BakeRequest, HighPoly, Progress, bake_tiles, tracing::Tracer};
use texturecraft_compose::BakedMaps;
use texturecraft_core::{BakeSettings, BakedMap};
use texturecraft_geom::{Bvh, Mesh};

/// A self-contained bake task that can run on another thread.
pub struct BakeTask {
    pub low_base_color: super::MaterialColors,
    pub ts: usize,
    pub mesh: Arc<Mesh>,
    pub bvh: Arc<Bvh>,
    pub material: usize,
    pub settings: BakeSettings,
    /// The merged high polys, unless the set bakes from the low poly.
    pub high: Option<MergedHigh>,
    pub cage: Option<Arc<Mesh>>,
    pub skew: Option<Arc<BakePaintMap>>,
    pub cage_paint: Option<Arc<BakePaintMap>>,
    /// UV tile grid (columns, rows) of the set; (1, 1) without UV tiles.
    pub tiles: (u32, u32),
    /// Snapshot of tile selection and pixels to retain while the worker runs
    pub excluded_tiles: Vec<(u32, u32)>,
    pub previous_maps: BakedMaps,
    /// The GPU ray tracer, if installed and not turned off (`bake.run {cpu: true}`).
    pub tracer: Option<Arc<dyn Tracer>>,
}

impl BakeTask {
    /// The maps this task makes, in the order [`BakeTask::run_streaming`] finishes them
    pub fn maps(&self) -> Vec<BakedMap> {
        self.settings.maps.enabled().collect()
    }

    /// Bake the maps one after another, handing each one to `on_map` as soon as it is padded,
    /// and return all of them together. Each map is a bake of its own, so with a high poly
    /// the transfer rays are cast once per map: slower than [`BakeTask::run`] with several
    /// maps, in exchange for showing them as they land. The pixels are the same as `run`'s.
    /// The handed-over buffers are the returned ones too (no copy is made), so a preview that
    /// keeps them costs no second set of maps. A cancel or an error leaves the maps already
    /// handed over whole.
    pub fn run_streaming(&self, progress: &Progress, on_map: &mut dyn FnMut(StreamedMap)) -> Result<StreamedBake> {
        let maps = self.maps();
        let total = maps.len();
        let mut combined: Option<StreamedBake> = None;

        for (index, map) in maps.iter().copied().enumerate() {
            let mut settings = self.settings.clone();
            settings.maps.select([map]);
            let mut output = self.run_with(&settings, progress)?;
            let Some(data) = output.maps.remove(&map) else { continue };
            if combined.is_none() {
                // Retain unchecked maps once, using the full selection rather than this one-map pass.
                super::preserve_tiles::restore_unchecked(&mut output, &self.previous_maps, &self.excluded_tiles, &maps)?;
            }

            let data = Arc::new(data);
            on_map(StreamedMap { map, index, total, resolution: output.resolution, height: output.height, data: data.clone() });
            match &mut combined {
                Some(combined) => {
                    combined.maps.insert(map, data);
                }
                None => {
                    // A resized partial bake also returns unchecked maps that must survive application
                    let mut retained: std::collections::BTreeMap<_, _> = output.maps.into_iter().map(|(map, pixels)| (map, Arc::new(pixels))).collect();
                    retained.insert(map, data);
                    combined = Some(StreamedBake { resolution: output.resolution, height: output.height, maps: retained, tracer: output.tracer });
                }
            }
        }

        match combined {
            Some(output) => Ok(output),
            None => {
                let output = self.run(progress)?;
                Ok(StreamedBake {
                    resolution: output.resolution,
                    height: output.height,
                    maps: output.maps.into_iter().map(|(map, pixels)| (map, Arc::new(pixels))).collect(),
                    tracer: output.tracer,
                })
            }
        }
    }

    pub fn run(&self, progress: &Progress) -> Result<BakeOutput> {
        let mut output = self.run_with(&self.settings, progress)?;
        super::preserve_tiles::restore_unchecked(&mut output, &self.previous_maps, &self.excluded_tiles, &[])?;
        Ok(output)
    }

    fn run_with(&self, settings: &BakeSettings, progress: &Progress) -> Result<BakeOutput> {
        let high =
            self.high.as_ref().map(|source| HighPoly { file_ids: &source.file_ids, mesh: &source.mesh, bvh: &source.bvh, base_color: &source.base_color });
        let request = BakeRequest {
            low_base_color: &self.low_base_color,
            mesh: &self.mesh,
            bvh: &self.bvh,
            material: self.material,
            settings,
            high,
            cage: self.cage.as_deref(),
            skew: self.skew.as_deref(),
            cage_paint: self.cage_paint.as_deref(),
            tiles: self.tiles,
        };

        let mut output = bake_tiles(&request, progress, &self.excluded_tiles, self.tracer.as_deref())
            .map_err(|error| EngineError::Other(format!("bake failed: {error}")))?;
        super::preserve_tiles::restore(&mut output, &self.previous_maps, &self.excluded_tiles, self.tiles)?;
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Session, execute};
    use serde_json::json;
    use texturecraft_geom::primitives;

    const PREVIOUS_MAPS: [BakedMap; 3] = [BakedMap::Normal, BakedMap::AmbientOcclusion, BakedMap::Height];
    const SELECTED_MAPS: [BakedMap; 2] = [BakedMap::AmbientOcclusion, BakedMap::Height];

    fn partial_task(selected: &[BakedMap], resized: bool, excluded: bool) -> BakeTask {
        let mut session = Session::new_project("Partial bake", primitives::udim_plane(), "builtin:udim_plane", None, 128).unwrap();
        execute(&mut session, "bake.settings", &json!({"resolution": 64, "rays": 2})).unwrap();
        if excluded {
            execute(&mut session, "bake.tile.set", &json!({"tile": 1002, "enabled": false})).unwrap();
        }
        let mut task = session.bake_task(0).unwrap();
        task.tracer = None;
        task.settings.maps.select(selected.iter().copied());
        let (width, height) = if resized { (64, 32) } else { (128, 64) };
        task.previous_maps = BakedMaps {
            resolution: width,
            maps: PREVIOUS_MAPS.into_iter().map(|map| (map, Arc::new(vec![0.25; width as usize * height * map.components()]))).collect(),
        };
        task
    }

    #[test]
    fn streaming_allocates_only_truly_unchecked_resized_maps_once() {
        for (selected, resized, excluded) in [
            (SELECTED_MAPS.as_slice(), true, true),
            (PREVIOUS_MAPS.as_slice(), true, true),
            (SELECTED_MAPS.as_slice(), false, true),
            (SELECTED_MAPS.as_slice(), true, false),
        ] {
            let task = partial_task(selected, resized, excluded);
            let whole = task.run(&Progress::new()).unwrap();
            super::super::preserve_tiles::take_retained_allocations();
            let mut allocations = Vec::new();
            let mut handed_over = Vec::new();
            let output = task
                .run_streaming(&Progress::new(), &mut |map| {
                    allocations.extend(super::super::preserve_tiles::take_retained_allocations());
                    assert!(allocations.iter().all(|map| !selected.contains(map)), "maps scheduled to bake must never allocate retained buffers");
                    handed_over.push(map);
                })
                .unwrap();
            allocations.extend(super::super::preserve_tiles::take_retained_allocations());
            let expected: Vec<_> = PREVIOUS_MAPS.into_iter().filter(|map| resized && excluded && !selected.contains(map)).collect();
            assert_eq!(allocations, expected, "each truly unchecked resized map must allocate exactly once");
            assert_eq!(output.maps.keys().collect::<Vec<_>>(), whole.maps.keys().collect::<Vec<_>>());
            for (map, pixels) in &whole.maps {
                assert_eq!(*output.maps[map], *pixels, "{map:?} must preserve the ordinary bake's pixels");
            }
            for map in handed_over {
                assert!(Arc::ptr_eq(&map.data, &output.maps[&map.map]), "streaming must keep sharing the baked buffer");
            }
        }
    }

    #[test]
    fn cancelling_a_partial_stream_does_not_resample_future_maps() {
        let task = partial_task(&SELECTED_MAPS, true, true);
        let whole = task.run(&Progress::new()).unwrap();
        super::super::preserve_tiles::take_retained_allocations();
        let progress = Progress::new();
        let mut handed_over = Vec::new();
        let result = task.run_streaming(&progress, &mut |map| {
            handed_over.push(map);
            progress.cancel();
        });
        assert!(result.is_err());
        assert_eq!(handed_over.len(), 1);
        for map in handed_over {
            assert_eq!(*map.data, whole.maps[&map.map], "cancellation leaves the handed-over map whole");
        }
        assert_eq!(super::super::preserve_tiles::take_retained_allocations(), [BakedMap::Normal]);
    }

    #[test]
    fn an_empty_streaming_selection_keeps_the_ordinary_bake_behavior() {
        let task = partial_task(&[], true, true);
        super::super::preserve_tiles::take_retained_allocations();
        let whole = task.run(&Progress::new());
        let expected_allocations = super::super::preserve_tiles::take_retained_allocations();
        let output = task.run_streaming(&Progress::new(), &mut |_| panic!("an empty selection must not stream a map"));
        assert_eq!(super::super::preserve_tiles::take_retained_allocations(), expected_allocations);
        match (whole, output) {
            (Ok(whole), Ok(output)) => {
                assert_eq!((output.resolution, output.height), (whole.resolution, whole.height));
                assert_eq!(output.maps.keys().collect::<Vec<_>>(), whole.maps.keys().collect::<Vec<_>>());
                for (map, pixels) in whole.maps {
                    assert_eq!(*output.maps[&map], pixels);
                }
            }
            (Err(whole), Err(output)) => assert_eq!(whole.to_string(), output.to_string()),
            _ => panic!("empty streaming and ordinary bakes must agree"),
        }
    }
}

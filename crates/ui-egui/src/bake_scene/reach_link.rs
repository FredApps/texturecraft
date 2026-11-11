use std::sync::mpsc::{Receiver, TryRecvError};

use texturecraft_engine::Session;
use texturecraft_engine::bake::{HighPolyReach, high_poly_missed};
use texturecraft_engine::core::HighPolySettings;

use super::cage_shell_key::CageShellKey;
use crate::gpu::SceneSync;

/// Each set's (max frontal, max rear) and the selected-set filter
type ShownDistances = (Vec<(f32, f32)>, Option<usize>);

/// The red on the high poly: each texture set's ray reach, worked out on a worker thread, and the distances the GPU's colors show
#[derive(Default)]
pub(super) struct ReachLink {
    /// Each set's ray inputs and high-poly settings (backfaces, name matching), and the id of the high poly, the reach was started for
    keys: Vec<Option<(CageShellKey, HighPolySettings)>>,
    high_id: u64,
    job: Option<Receiver<Vec<Option<HighPolyReach>>>>,
    reach: Vec<Option<HighPolyReach>>,
    /// Each set's (max frontal, max rear) and the selected-set filter the GPU's colors are for
    shown: Option<ShownDistances>,
}

impl ReachLink {
    /// Whether a reach is being worked out (the view must keep drawing to pick it up)
    pub(super) fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Send the colors again (the GPU lost what it was sent); the reach itself stays
    pub(super) fn resend(&mut self) {
        self.shown = None;
    }

    /// Pick up the reach once the worker finishes; runs every frame, even when nothing is drawn, so `busy` clears
    pub(super) fn poll(&mut self) {
        let Some(job) = &self.job else { return };

        match job.try_recv() {
            Ok(reach) => {
                self.reach = reach;
                self.shown = None;
                self.job = None;
            }
            Err(TryRecvError::Disconnected) => self.job = None,
            Err(TryRecvError::Empty) => {}
        }
    }

    /// Start a new reach when the high poly (sent under `high_id`) or a set's inputs changed, and color the high poly when the reach or the distances changed
    pub(super) fn sync(&mut self, session: &Session, high_id: u64, only_set: Option<usize>, out: &mut SceneSync) {
        // Coalesce paint edits while a worker runs; after polling it, the next job captures the latest inputs
        if self.job.is_some() {
            return;
        }

        let set_count = session.doc.project.texture_sets.len();
        let keys: Vec<Option<(CageShellKey, HighPolySettings)>> = session
            .doc
            .project
            .texture_sets
            .iter()
            .enumerate()
            .map(|(texture_set, set)| Some((CageShellKey::of(session, texture_set)?, set.bake.high_poly.clone())))
            .collect();
        if keys != self.keys || high_id != self.high_id {
            let inputs: Vec<_> = (0..set_count).map(|texture_set| session.bake_cage_inputs(texture_set).ok().flatten()).collect();
            self.job = Some(crate::spawn_worker("texturecraft-bake-reach", move || {
                inputs.iter().map(|inputs| inputs.as_ref().and_then(|inputs| inputs.high_poly_reach().ok())).collect()
            }));
            (self.keys, self.high_id) = (keys, high_id);
        }

        // Colors wait for the reach being worked out: the old ones stay until it arrives
        if self.job.is_some() {
            return;
        }

        let distances: Vec<(f32, f32)> = session.doc.project.texture_sets.iter().map(|set| (set.bake.cage.max_frontal, set.bake.cage.max_rear)).collect();
        let shown = (distances, only_set);
        if self.shown.as_ref() == Some(&shown) {
            return;
        }

        let Some(vertex_count) = session.bake_meshes.merged_high().map(|high| high.mesh.positions.len()) else { return };

        out.bake_high_missed = Some((self.high_id, high_poly_missed(&self.reach, &shown.0, only_set, vertex_count)));
        self.shown = Some(shown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, mpsc};
    use std::time::Duration;
    use texturecraft_engine::bake::BakePaintMap;
    use texturecraft_engine::core::BakePaintTarget;
    use texturecraft_engine::geom::primitives;

    #[test]
    fn input_changes_wait_for_the_running_reach_then_use_the_latest_inputs() {
        let mut session = Session::new_project("Reach", primitives::plane(1.0, 1), "plane", None, 128).unwrap();
        session.add_high_poly_bytes("high.obj", b"v 0 0.1 0\nv 0.1 0.1 0\nv 0 0.1 0.1\nf 1 2 3\n".to_vec()).unwrap();
        let (sender, receiver) = mpsc::channel();
        let mut link = ReachLink { job: Some(receiver), ..Default::default() };

        // Hold the original job open while paint and mesh inputs change
        for amount in [0.25, 0.5, 0.75] {
            session.doc.bake_paint.insert((0, BakePaintTarget::Cage), Arc::new(BakePaintMap::constant(amount)));
            let mut out = SceneSync::default();
            link.sync(&session, 2, None, &mut out);
            assert!(out.bake_high_missed.is_none());
        }
        assert!(sender.send(Vec::new()).is_ok(), "input changes must keep the running job's receiver");
        link.poll();

        // The finished job is stale: a new job must capture the latest settings before sending colors
        session.doc.project.texture_sets[0].bake.high_poly.use_low_as_high = true;
        let mut out = SceneSync::default();
        link.sync(&session, 2, None, &mut out);
        assert!(link.busy());
        assert!(out.bake_high_missed.is_none());

        for _ in 0..2000 {
            link.poll();
            if !link.busy() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!link.busy(), "the latest reach must finish");
        link.sync(&session, 2, None, &mut out);
        let (high_id, missed) = out.bake_high_missed.unwrap();
        assert_eq!(high_id, 2);
        assert_eq!(missed, vec![0.0; session.bake_meshes.merged_high().unwrap().mesh.positions.len()]);
        assert!(link.reach[0].is_none(), "the latest setting bakes without a high poly");
    }
}

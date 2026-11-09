use std::sync::Arc;

use texturecraft_engine::Session;
use texturecraft_engine::geom::Mesh;
use texturecraft_render::BakeVisualization;

use super::CageUpload;
use super::cage_link::CageLink;
use super::cage_shell_key::CageShellKey;
use super::reach_link::ReachLink;
use super::shared_handle::SharedHandle;
use crate::gpu::SceneSync;

/// What the GPU has of bake mode's visualization: the high poly, each set's cage and the UV seams
#[derive(Default)]
pub struct BakeSceneLink {
    /// The high poly sent to the GPU and the id it was sent under (each new high poly gets the next one)
    high: SharedHandle<Mesh>,
    high_id: u64,
    /// Whether the GPU has the high poly (a lost frame sends it again under the same id)
    high_sent: bool,
    cages: Vec<Option<CageLink>>,
    /// How many texture sets' cages the GPU was last told to keep
    sent_cage_count: Option<usize>,
    /// The low-poly mesh whose seams are uploaded
    seams_mesh_id: Option<u64>,
    next_topology: u64,
    reach: ReachLink,
}

impl BakeSceneLink {
    /// Send everything again (the GPU lost what it was sent); the cage shells and the reach worked out on the CPU stay
    pub fn resend(&mut self) {
        self.high_sent = false;
        self.sent_cage_count = None;
        self.seams_mesh_id = None;
        for link in self.cages.iter_mut().flatten() {
            link.max_frontal = f32::NAN;
        }
        self.reach.resend();
    }

    /// Whether something is still being worked out off the UI thread
    pub fn busy(&self) -> bool {
        self.reach.busy()
    }

    /// Add to `out` what the GPU needs to draw `visualization` this frame
    pub fn sync(&mut self, session: &Session, visualization: Option<BakeVisualization>, out: &mut SceneSync) {
        self.reach.poll();

        let Some(visualization) = visualization else { return };

        if visualization.high_poly {
            let high = session.bake_meshes.merged_high().map(|high| high.mesh.clone());
            let handle = SharedHandle::of(high.as_ref());
            if handle != self.high {
                self.high = handle;
                self.high_id += 1;
                self.high_sent = false;
            }
            if !self.high_sent {
                out.bake_high = Some((high, self.high_id));
                self.high_sent = true;
            }
            self.reach.sync(session, self.high_id, visualization.only_set, out);
        }

        if visualization.uv_seams && self.seams_mesh_id != Some(session.mesh_id) {
            self.seams_mesh_id = Some(session.mesh_id);
            out.uv_seams = Some((session.mesh_id, session.mesh.uv_seam_edges()));
        }

        if visualization.cage {
            self.sync_cages(session, out);
        }
    }

    /// Rebuild the cages whose inputs changed and move the corners of those whose distance changed
    fn sync_cages(&mut self, session: &Session, out: &mut SceneSync) {
        let set_count = session.doc.project.texture_sets.len();
        if self.sent_cage_count != Some(set_count) {
            out.bake_cage_count = Some(set_count);
            self.sent_cage_count = Some(set_count);
        }
        self.cages.truncate(set_count);
        self.cages.resize_with(set_count, || None);

        for (texture_set, link) in self.cages.iter_mut().enumerate() {
            let (Some(key), Some(set)) = (CageShellKey::of(session, texture_set), session.doc.project.texture_sets.get(texture_set)) else { continue };

            let max_frontal = set.bake.cage.max_frontal;
            match link {
                Some(link) if link.key == key => {
                    if link.max_frontal == max_frontal {
                        continue;
                    }
                    link.max_frontal = max_frontal;
                }
                _ => {
                    self.next_topology += 1;
                    let shell = session.bake_cage_inputs(texture_set).ok().flatten().and_then(|inputs| inputs.cage_shell().ok()).map(Arc::new);
                    *link = Some(CageLink { key, shell, topology: self.next_topology, max_frontal });
                }
            }

            let Some(link) = link.as_ref() else { continue };

            let upload = link.shell.as_ref().map(|shell| CageUpload { corners: shell.corners(max_frontal), shell: shell.clone() });
            out.bake_cages.push((texture_set, upload, link.topology));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const EVERYTHING: BakeVisualization = BakeVisualization { high_poly: true, cage: true, uv_seams: true, low_poly: true, only_set: None };

    /// How long a test waits for the reach worker: frames, and the pause between them
    const WAIT_FRAMES: usize = 2000;
    const WAIT_FRAME_PAUSE: std::time::Duration = std::time::Duration::from_millis(5);

    /// The sample project with a high poly a little outside its low poly
    fn session() -> Session {
        const HIGH_POLY_OFFSET: f32 = 0.02;

        let mut session = texturecraft_engine::sample_session(128, 128).unwrap();
        let low = session.mesh.clone();
        let mut obj = String::new();
        for (position, normal) in low.positions.iter().zip(&low.normals) {
            let raised = *position + *normal * HIGH_POLY_OFFSET;
            obj += &format!("v {} {} {}\n", raised.x, raised.y, raised.z);
        }
        for corners in low.indices.chunks(3) {
            obj += &format!("f {} {} {}\n", corners[0] + 1, corners[1] + 1, corners[2] + 1);
        }
        session.add_high_poly_bytes("high.obj", obj.into_bytes()).unwrap();
        session
    }

    fn sync(link: &mut BakeSceneLink, session: &Session, visualization: BakeVisualization) -> SceneSync {
        let mut out = SceneSync::default();
        link.sync(session, Some(visualization), &mut out);
        out
    }

    /// Sync until the reach worked out on the worker thread colors the high poly; the share of vertices drawn red
    fn red_share(link: &mut BakeSceneLink, session: &Session, visualization: BakeVisualization) -> f32 {
        for _ in 0..WAIT_FRAMES {
            if let Some((_, missed)) = sync(link, session, visualization).bake_high_missed {
                return missed.iter().sum::<f32>() / missed.len().max(1) as f32;
            }
            std::thread::sleep(WAIT_FRAME_PAUSE);
        }
        panic!("the reach never arrived");
    }

    fn set_distances(session: &mut Session, max_frontal: f32) {
        texturecraft_engine::execute(session, "bake.settings", &json!({"ts": "all", "maxFrontal": max_frontal})).unwrap();
    }

    #[test]
    fn a_new_distance_moves_the_corners_and_new_settings_rebuild_the_cage() {
        let (mut session, mut link) = (session(), BakeSceneLink::default());
        let first = sync(&mut link, &session, EVERYTHING);
        assert!(first.bake_high.is_some() && first.uv_seams.is_some() && first.bake_cage_count == Some(session.doc.project.texture_sets.len()));
        let (_, Some(cage), topology) = first.bake_cages.first().unwrap() else { panic!("set 0 has a cage") };

        let unchanged = sync(&mut link, &session, EVERYTHING);
        assert!(unchanged.bake_high.is_none() && unchanged.bake_cages.is_empty() && unchanged.uv_seams.is_none(), "nothing changed, nothing is sent");

        set_distances(&mut session, 0.03);
        let moved = sync(&mut link, &session, EVERYTHING);
        let (_, Some(moved_cage), moved_topology) = moved.bake_cages.first().unwrap() else { panic!("set 0 has a cage") };
        assert_eq!(moved_topology, topology, "a distance only moves the corners");
        assert_ne!(moved_cage.corners, cage.corners);

        texturecraft_engine::execute(&mut session, "bake.settings", &json!({"ts": "all", "averageNormals": false})).unwrap();
        let rebuilt = sync(&mut link, &session, EVERYTHING);
        assert_ne!(rebuilt.bake_cages.first().unwrap().2, *topology, "new settings rebuild the cage");
        // The rebuilt cage starts a new reach: let it finish so the last check sees only what the resend starts
        red_share(&mut link, &session, EVERYTHING);

        link.resend();
        let resent = sync(&mut link, &session, EVERYTHING);
        assert!(resent.bake_high.is_some() && resent.uv_seams.is_some() && resent.bake_cage_count.is_some(), "a lost frame sends everything again");
        assert_eq!(resent.bake_cages.len(), rebuilt.bake_cages.len());
        assert_eq!(resent.bake_cages.first().unwrap().2, rebuilt.bake_cages.first().unwrap().2, "the cages are not rebuilt for a lost frame");
        assert_eq!(resent.bake_high.unwrap().1, first.bake_high.unwrap().1, "the high poly goes again under the same id");
        assert!(!link.busy(), "a lost frame doesn't work out the reach again");
    }

    #[test]
    fn a_reach_finishes_while_nothing_is_drawn() {
        let session = session();
        let no_high_poly = BakeVisualization { high_poly: false, ..EVERYTHING };
        for hidden in [None, Some(no_high_poly)] {
            let mut link = BakeSceneLink::default();
            sync(&mut link, &session, EVERYTHING);
            assert!(link.busy(), "the first frame starts working out the reach");

            let finished = (0..WAIT_FRAMES).any(|_| {
                std::thread::sleep(WAIT_FRAME_PAUSE);
                link.sync(&session, hidden, &mut SceneSync::default());
                !link.busy()
            });
            assert!(finished, "the view stops redrawing for a reach once it is done: {hidden:?}");
        }
    }

    #[test]
    fn the_red_follows_the_distances_and_the_selected_set() {
        let (mut session, mut link) = (session(), BakeSceneLink::default());
        set_distances(&mut session, 0.0);
        let at_zero = red_share(&mut link, &session, EVERYTHING);
        assert!(at_zero > 0.5, "at frontal 0 the high poly outside the low poly is red: {at_zero}");

        set_distances(&mut session, 0.05);
        let covered = red_share(&mut link, &session, EVERYTHING);
        assert!(covered < at_zero / 10.0, "a cage past the high poly reaches it: {covered}");

        set_distances(&mut session, 0.0);
        let by_name = |session: &mut Session, low_suffix: &str| {
            for set in &mut session.doc.project.texture_sets {
                set.bake.high_poly.match_mode = texturecraft_engine::core::MeshMatchMode::ByMeshName;
                set.bake.high_poly.low_suffix = low_suffix.into();
            }
        };
        by_name(&mut session, "_no_such_suffix");
        let by_name_red = red_share(&mut link, &session, EVERYTHING);
        assert!(by_name_red > 0.5, "matching by name and other suffixes works out the reach again: {by_name_red}");

        let no_set = BakeVisualization { only_set: Some(session.doc.project.texture_sets.len()), ..EVERYTHING };
        assert_eq!(red_share(&mut link, &session, no_set), 0.0, "sets left out by the filter color nothing");
    }
}

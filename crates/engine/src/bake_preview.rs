//! Live bake preview: while the app bakes, the 3D view shows each mesh map on the model as soon
//! as it is finished ([`crate::BakeTask::run_streaming`]). The streamed maps live here, not in
//! the document: nothing enters the undo history until a texture set's bake is applied
//! ([`Session::apply_streamed_bake`]). Cancellation leaves the current set unchanged; sets already
//! applied keep their results and undo steps. The preview shows one map at a time, raw on the
//! model; maps and texture sets not baked yet show their neutral value.

mod preview_set;
mod preview_show;
mod stage;

pub use preview_set::PreviewSet;
pub use preview_show::PreviewShow;
pub use stage::Stage;

use std::collections::BTreeMap;

use serde_json::{Value, json};
use texturecraft_compose::BakedMaps;
use texturecraft_core::BakedMap;

use crate::bake::StreamedMap;
use crate::commands::{CommandSpec, always, bad, strp};
use crate::session::ViewMode;
use crate::{Result, Session, cmd};

/// What painting says while a bake preview shows (the app translates it).
pub const PAINTING_PAUSED: &str = "Painting is paused while a bake is shown. Cancel the bake or wait for it to finish.";

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "bake.preview",
        "Bake preview",
        [],
        None,
        "{show?: \"follow\" (the newest finished map) | \"material\" | \"map:<id>\" (pin a map)} → {active, show, shown, newest, stage, sets}. While the app bakes, the 3D view shows the maps as they finish; `show` picks what it shows. With no bake running {active: false} (setting `show` is then an error)",
        always,
        preview
    )]
}

#[derive(Clone, Debug)]
pub struct BakePreview {
    /// The view mode to put back when the preview ends.
    pub restore: ViewMode,
    pub show: PreviewShow,
    pub sets: BTreeMap<usize, PreviewSet>,
    /// The texture set being baked.
    pub current: Option<usize>,
    /// The last map that finished.
    pub newest: Option<BakedMap>,
}

impl BakePreview {
    /// The map the view shows, None for the material.
    pub fn shown(&self) -> Option<BakedMap> {
        match self.show {
            PreviewShow::Material => None,
            PreviewShow::Pin(map) => Some(map),
            PreviewShow::Follow => self
                .newest
                .or_else(|| self.current_set().and_then(|set| set.order.first().copied()))
                .or_else(|| self.sets.values().find_map(|set| set.order.first().copied())),
        }
    }

    pub fn current_set(&self) -> Option<&PreviewSet> {
        self.sets.get(&self.current?)
    }

    /// The map being baked now (None between sets and once the set's maps are all in).
    pub fn stage(&self) -> Option<Stage> {
        let texture_set = self.current?;
        let set = self.sets.get(&texture_set)?;
        let done = set.baked.maps.len();
        Some(Stage { texture_set, map: *set.order.get(done)?, position: done + 1, total: set.order.len() })
    }
}

impl Session {
    /// Start a preview from each selected texture set's enabled maps, in the same order as
    /// [`crate::BakeTask::maps`]. Invalid indices return the project's texture-set error before
    /// changing the preview or view mode. This reads settings without preparing bake geometry.
    pub fn bake_preview_begin_for(&mut self, sets: &[usize]) -> Result<()> {
        let orders = sets
            .iter()
            .map(|&texture_set| {
                let set = self.doc.project.texture_set(texture_set)?;
                Ok((texture_set, set.bake.maps.enabled().collect()))
            })
            .collect::<Result<Vec<_>>>()?;

        self.bake_preview_begin(orders);
        Ok(())
    }

    /// Start showing a bake of `sets` (each with the maps it makes, in order): the 3D view
    /// follows the newest map and every set shows neutral values until its maps arrive. The
    /// view mode in use comes back with [`Session::bake_preview_end`]. An active stroke is
    /// discarded so a later pointer release cannot commit paint during the preview.
    pub fn bake_preview_begin(&mut self, sets: Vec<(usize, Vec<BakedMap>)>) {
        self.cancel_stroke();

        let restore = self.bake_preview.as_ref().map_or(self.view, |preview| preview.restore);
        let sets = sets.into_iter().map(|(texture_set, order)| (texture_set, PreviewSet { baked: BakedMaps::default(), order })).collect();

        self.bake_preview = Some(BakePreview { restore, show: PreviewShow::Follow, sets, current: None, newest: None });
        self.bake_preview_refresh();
    }

    /// The texture set whose maps stream in now.
    pub fn bake_preview_set_current(&mut self, texture_set: usize) {
        if let Some(preview) = &mut self.bake_preview {
            preview.current = Some(texture_set);
        }

        self.bake_preview_refresh();
    }

    /// Install one finished map of set `texture_set`. Returns false when no preview shows.
    pub fn bake_preview_map(&mut self, texture_set: usize, streamed: StreamedMap) -> bool {
        let Some(preview) = &mut self.bake_preview else { return false };

        let set = preview.sets.entry(texture_set).or_default();

        // A new bake of the set starts from nothing (the baker never repeats a map)
        if streamed.index == 0 {
            set.baked = BakedMaps::default();
        }

        set.baked.resolution = streamed.resolution;
        set.baked.maps.insert(streamed.map, streamed.data);
        preview.newest = Some(streamed.map);
        self.bake_preview_refresh();
        true
    }

    /// Choose what the view shows (Follow, a pinned map or the material).
    pub fn bake_preview_show(&mut self, show: PreviewShow) -> Result<()> {
        let Some(preview) = &mut self.bake_preview else { return Err(bad("bake.preview", "no bake is running")) };

        preview.show = show;
        self.bake_preview_refresh();
        Ok(())
    }

    /// The user picked a view mode during a bake: a mesh map pins it, any other mode shows
    /// the material and is what comes back when the bake ends.
    pub(crate) fn bake_preview_user_view(&mut self, view: ViewMode) {
        let Some(preview) = &mut self.bake_preview else { return };

        match view {
            ViewMode::MeshMap(map) => preview.show = PreviewShow::Pin(map),
            other => {
                preview.show = PreviewShow::Material;
                preview.restore = other;
            }
        }

        self.bake_preview_refresh();
        if !matches!(view, ViewMode::MeshMap(_)) {
            self.view = view;
        }
    }

    /// Drop the displayed maps and restore the previous view mode. This does not stop a
    /// worker or undo any applied results; the UI also uses it when leaving Bake mode.
    pub fn bake_preview_end(&mut self) {
        if let Some(preview) = self.bake_preview.take() {
            self.view = preview.restore;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    pub fn bake_preview_active(&self) -> bool {
        self.bake_preview.is_some()
    }

    /// The streamed maps of set `texture_set` while a preview shows (empty for a set not baked
    /// yet: neutral values), None when the document's baked maps are what shows.
    pub fn bake_preview_baked(&self, texture_set: usize) -> Option<&BakedMaps> {
        self.bake_preview.as_ref()?.sets.get(&texture_set).map(|set| &set.baked)
    }

    /// Why painting is not possible now, when a bake preview shows.
    pub fn painting_blocked(&self) -> Option<&'static str> {
        self.bake_preview.as_ref().map(|_| PAINTING_PAUSED)
    }

    fn bake_preview_refresh(&mut self) {
        let Some(preview) = &self.bake_preview else { return };

        self.view = match preview.shown() {
            Some(map) => ViewMode::MeshMap(map),
            None => ViewMode::Material,
        };
        self.revision = self.revision.wrapping_add(1);
    }
}

/// The preview as JSON, for `bake.preview`, `session.inspect` and `ui.inspect`.
pub fn preview_json(session: &Session) -> Value {
    let Some(preview) = &session.bake_preview else { return json!({ "active": false }) };

    let stage = preview.stage().map(|stage| json!({ "set": stage.texture_set, "map": stage.map.id(), "n": stage.position, "total": stage.total }));
    let sets: Vec<Value> = preview
        .sets
        .iter()
        .map(|(texture_set, set)| {
            json!({
                "set": texture_set,
                "order": set.order.iter().map(|map| map.id()).collect::<Vec<_>>(),
                "done": set.baked.maps.keys().map(|map| map.id()).collect::<Vec<_>>(),
            })
        })
        .collect();

    json!({
        "active": true,
        "show": preview.show.id(),
        "shown": preview.shown().map(|map| map.id()),
        "newest": preview.newest.map(|map| map.id()),
        "restore": preview.restore.label(),
        "stage": stage,
        "sets": sets,
    })
}

fn preview(session: &mut Session, params: &Value) -> Result<Value> {
    if let Some(show) = strp(params, "show") {
        let show = PreviewShow::parse(show).ok_or_else(|| bad("bake.preview", format!("unknown `show` {show:?}: follow, material or map:<id>")))?;
        session.bake_preview_show(show)?;
    }

    Ok(preview_json(session))
}

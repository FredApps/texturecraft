//! Bridge between the session and the wgpu renderer: per-frame scene sync (mesh, texture set
//! layout, dirty tiles, environment) and the egui paint callback that renders and blits a view.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, ScreenDescriptor};
use rayon::prelude::*;
use texturecraft_engine::Session;
use texturecraft_engine::compose::{DisplayKind, ImageRes, tile_bytes};
use texturecraft_engine::core::BakedMap;
use texturecraft_engine::geom::Mesh;
use texturecraft_render::{Frame, Renderer, wgpu};

/// Content shown through the aux texture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AuxView {
    MeshMap(BakedMap),
    Mask(texturecraft_engine::core::LayerId),
    /// The quick mask being edited, over the base color.
    QuickMask,
}

/// A texture set's texels across and down, and its mesh material.
pub type SetLayout = ((u32, u32), usize);

/// What the renderer must catch up with before drawing.
#[derive(Default)]
pub struct SceneSync {
    pub mesh: Option<(Arc<Mesh>, u64)>,
    /// (texels across and down, material) per texture set, the mesh material count and the
    /// viewport texture resolution (display shift).
    pub sets: Option<(Vec<SetLayout>, usize, u32)>,
    /// The environment name and, for `resource:<name>`, its decoded image.
    pub env: Option<(String, Option<Arc<ImageRes>>)>,
    pub tiles: Vec<(usize, DisplayKind, u32, u32, Vec<u8>)>,
    pub aux: Vec<(usize, u32, Vec<u8>)>,
}

/// Lives in egui's callback resources.
pub struct GpuState {
    pub renderer: Renderer,
    /// Raised when texture-set textures were recreated: the UI must upload every tile again.
    pub reset: Arc<AtomicBool>,
    pub reports: Arc<Mutex<GpuReports>>,
    /// Wakes the UI when the GPU side has something to report.
    pub ctx: egui::Context,
}

/// Problems the GPU side hands to the UI, which shows them as toasts.
#[derive(Default)]
pub struct GpuReports {
    /// Texture sets whose display textures could not be allocated (out of GPU memory).
    pub memory: Vec<usize>,
    /// wgpu errors no error scope caught (logged instead of wgpu's default panic).
    pub errors: Vec<String>,
}

/// Most distinct uncaptured GPU errors kept until the UI shows them.
const MAX_GPU_ERRORS: usize = 8;

/// UI-side bookkeeping of what the GPU already has.
pub struct GpuLink {
    pub reset: Arc<AtomicBool>,
    pub reports: Arc<Mutex<GpuReports>>,
    /// Raised when a view callback was dropped before the GPU applied its scene sync (the
    /// frame was never painted, e.g. a WebGPU surface not ready yet): send everything again.
    pub lost: Arc<AtomicBool>,
    mesh_id: u64,
    sets: Vec<SetLayout>,
    /// The display shift the GPU has (`Prefs::viewport_texture`).
    shift: u32,
    /// The environment the GPU has: name and image identity (a re-imported resource differs).
    env: (String, usize),
    /// What each set's aux texture holds: (mesh map or mask key, revision).
    aux: BTreeMap<usize, (String, u64)>,
    pub adapter: String,
    /// Most tiles waiting to be composited since compositing last caught up (progress).
    composite_total: usize,
}

/// Compositing time per frame: a full recomposite of a large texture set spreads over frames.
const COMPOSITE_BUDGET_MS: f64 = 30.0;
/// The most a frame spends on the scene (with the work already done this frame, such as a
/// stroke's commit): compositing and display conversion leave the rest for the next frames, so
/// a large (8K) texture set never stalls one frame.
const FRAME_BUDGET_MS: f64 = 35.0;
/// Each frame still makes this much progress.
const MIN_SLICE_MS: f64 = 4.0;
/// Display tiles converted between budget checks.
const UPLOAD_CHUNK: usize = 64;
/// Estimated GPU upload cost of one display tile (write_texture of one 64×64 tile of one
/// display kind, measured at ~5.5 µs on a desktop GPU), counted against the frame budget.
const UPLOAD_MS_PER_TILE: f64 = 0.006;

pub fn init(rs: &egui_wgpu::RenderState, ctx: &egui::Context) -> GpuLink {
    let reset = Arc::new(AtomicBool::new(false));
    let reports = Arc::new(Mutex::new(GpuReports::default()));
    // wgpu's default handler panics on any error no scope caught (an out-of-memory texture, a
    // driver hiccup): report it instead, so the user keeps the session.
    let (sink, wake) = (reports.clone(), ctx.clone());
    rs.device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
        log::error!("GPU error: {e}");
        let mut r = sink.lock().unwrap_or_else(PoisonError::into_inner);
        let text = e.to_string();
        if r.errors.len() < MAX_GPU_ERRORS && !r.errors.contains(&text) {
            r.errors.push(text);
        }
        // The UI may be idle: draw a frame to show it.
        wake.request_repaint();
    }));
    let renderer = Renderer::new(&rs.device, &rs.queue, rs.target_format);
    rs.renderer.write().callback_resources.insert(GpuState { renderer, reset: reset.clone(), reports: reports.clone(), ctx: ctx.clone() });
    let info = rs.adapter.get_info();
    GpuLink {
        reset,
        reports,
        lost: Arc::new(AtomicBool::new(false)),
        mesh_id: 0,
        sets: Vec::new(),
        shift: 0,
        env: Default::default(),
        aux: BTreeMap::new(),
        adapter: format!("{} ({:?})", info.name, info.backend),
        composite_total: 0,
    }
}

impl GpuLink {
    /// What the GPU side reported since the last call (see [`GpuReports`]).
    pub fn take_reports(&self) -> GpuReports {
        std::mem::take(&mut *self.reports.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Collect everything the GPU needs this frame (runs the compositor first).
    /// Compositing progress (0..1) while tiles wait to be composited.
    pub fn composite_progress(&self, s: &Session) -> Option<f32> {
        let pending = s.pending_tiles();
        (pending > 0 && self.composite_total > 0).then(|| 1.0 - pending as f32 / self.composite_total as f32)
    }

    /// `spent_ms`: time this frame already took before the sync.
    pub fn sync(&mut self, s: &mut Session, aux_map: Option<AuxView>, shift: u32, spent_ms: f64) -> SceneSync {
        let start = crate::now_ms();
        let mut queued = 0usize;
        let left = |queued: usize| FRAME_BUDGET_MS - spent_ms - (crate::now_ms() - start) - queued as f64 * UPLOAD_MS_PER_TILE;
        let pending = s.pending_tiles();
        self.composite_total = if pending == 0 { 0 } else { self.composite_total.max(pending) };
        s.update_for(COMPOSITE_BUDGET_MS.min(left(0)).max(MIN_SLICE_MS), crate::now_ms);
        let mut out = SceneSync::default();
        if self.lost.swap(false, Ordering::AcqRel) {
            self.mesh_id = 0;
            self.sets.clear();
            self.env = Default::default();
            self.reset.store(true, Ordering::Release);
        }
        if self.mesh_id != s.mesh_id {
            out.mesh = Some((s.mesh.clone(), s.mesh_id));
            self.mesh_id = s.mesh_id;
            self.aux.clear();
        }
        let sets: Vec<SetLayout> = s.doc.project.texture_sets.iter().map(|t| (t.size(), t.material)).collect();
        if sets != self.sets || shift != self.shift || out.mesh.is_some() {
            out.sets = Some((sets.clone(), s.mesh.materials.len().max(1), shift));
            self.sets = sets;
            self.shift = shift;
        }
        let d = &s.doc.project.display;
        let image = d.environment_resource().and_then(|n| s.images.get(n));
        let ptr = image.map_or(0, |i| Arc::as_ptr(i) as usize);
        if self.env.0 != d.environment || self.env.1 != ptr {
            self.env = (d.environment.clone(), ptr);
            out.env = Some((d.environment.clone(), image.cloned()));
        }
        if self.reset.swap(false, Ordering::AcqRel) {
            for c in &mut s.caches {
                c.upload.extend(texturecraft_engine::compose::all_tiles(c.size.0, c.size.1));
            }
            self.aux.clear();
        }
        for ts in 0..s.caches.len() {
            let tiles = s.take_uploads(ts);
            if tiles.is_empty() {
                continue;
            }
            let mut done = 0;
            if let Some(cache) = s.caches.get(ts) {
                let empty = Default::default();
                let baked = s.doc.baked.get(&ts).unwrap_or(&empty);
                let comp = &cache.composite;
                // In chunks while the frame has time (at least one chunk); the rest waits.
                for chunk in tiles.chunks(UPLOAD_CHUNK) {
                    let bytes: Vec<_> = chunk
                        .par_iter()
                        .flat_map_iter(|&(tx, ty)| DisplayKind::ALL.into_iter().map(move |k| (k, tx, ty)))
                        .map(|(k, tx, ty)| (ts, k, tx, ty, tile_bytes(comp, baked, k, tx, ty)))
                        .collect();
                    queued += bytes.len();
                    out.tiles.extend(bytes);
                    done += chunk.len();
                    if left(queued) < MIN_SLICE_MS {
                        break;
                    }
                }
            }
            if let (Some(rest), Some(cache)) = (tiles.get(done..), s.caches.get_mut(ts)) {
                cache.upload.extend(rest.iter().copied());
            }
        }
        match aux_map {
            Some(AuxView::MeshMap(m)) => {
                for ts in 0..s.caches.len() {
                    let rev = s.doc.baked.get(&ts).and_then(|b| b.maps.get(&m)).map(|d| Arc::as_ptr(d) as u64).unwrap_or(0);
                    let key = (format!("{m:?}"), rev);
                    if self.aux.get(&ts) == Some(&key) {
                        continue;
                    }
                    let Some(cache) = s.caches.get(ts) else { continue };

                    let res = texturecraft_render::aux_size(cache.size);
                    out.aux.push((
                        ts,
                        res,
                        texturecraft_render::mesh_maps::mesh_map_rgba(s.doc.baked.get(&ts), m, res, cache.size, &cache.composite.block_scales),
                    ));
                    self.aux.insert(ts, key);
                }
            }
            Some(AuxView::Mask(id)) => {
                if let Some((ts, _, _)) = s.doc.project.locate(id) {
                    let key = (format!("mask{}", id.0), s.revision);
                    if self.aux.get(&ts) != Some(&key) && !s.stroke_active() {
                        let res = s.caches.get(ts).map_or(1024, |c| texturecraft_render::aux_size(c.size));
                        if let Some(rgba) = s.mask_view_rgba(id, res) {
                            out.aux.push((ts, res, rgba));
                        }
                        self.aux.insert(ts, key);
                    }
                }
            }
            Some(AuxView::QuickMask) => {
                for ts in 0..s.caches.len() {
                    let Some(cache) = s.caches.get(ts) else { continue };
                    let key = (format!("quickmask{}", s.quick_mask.revision), cache.revision);
                    if self.aux.get(&ts) == Some(&key) {
                        continue;
                    }
                    let res = texturecraft_render::aux_size(cache.size);
                    if let Some(rgba) = s.quick_mask_rgba(ts, res) {
                        out.aux.push((ts, res, rgba));
                    }
                    self.aux.insert(ts, key);
                }
            }
            None => {}
        }
        out
    }
}

/// Paint callback for one view.
pub struct ViewCallback {
    pub frame: Frame,
    /// Scene changes, applied by the first view drawn this frame.
    pub sync: Mutex<Option<SceneSync>>,
    pub rect: egui::Rect,
    /// [`GpuLink::lost`].
    pub lost: Arc<AtomicBool>,
}

impl Drop for ViewCallback {
    fn drop(&mut self) {
        if self.sync.get_mut().unwrap_or_else(PoisonError::into_inner).is_some() {
            self.lost.store(true, Ordering::Release);
        }
    }
}

impl CallbackTrait for ViewCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(gpu) = resources.get_mut::<GpuState>() else { return Vec::new() };
        let sync = self.sync.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(sync) = sync {
            if let Some((env, image)) = &sync.env {
                gpu.renderer.set_environment(device, queue, env, image.as_deref());
            }
            if let Some((mesh, id)) = &sync.mesh {
                gpu.renderer.set_mesh(device, queue, mesh, *id);
            }
            if let Some((sets, mats, shift)) = &sync.sets {
                gpu.renderer.set_display_shift(*shift);
                let reset = gpu.renderer.set_texture_sets(device, queue, sets, *mats);
                if !reset.is_empty() {
                    gpu.reset.store(true, Ordering::Release);
                }
            }
            for (ts, k, tx, ty, bytes) in &sync.tiles {
                gpu.renderer.upload_tile(device, queue, *ts, *k, *tx, *ty, bytes);
            }
            for (ts, size, bytes) in &sync.aux {
                gpu.renderer.upload_aux(device, queue, *ts, *size, bytes);
            }
        }
        // Also installs (or rejects) full-size textures WebGPU answered for since last frame.
        gpu.renderer.prepare(device, queue, encoder, &self.frame);
        let failed = gpu.renderer.take_memory_failures();
        if !failed.is_empty() {
            let mut r = gpu.reports.lock().unwrap_or_else(PoisonError::into_inner);
            for ts in failed {
                if !r.memory.contains(&ts) {
                    r.memory.push(ts);
                }
            }
            // This frame's report drain already ran: the next frame shows the toast.
            gpu.ctx.request_repaint();
        }
        if gpu.renderer.awaiting_gpu() {
            gpu.ctx.request_repaint();
        }
        let ppp = screen.pixels_per_point;
        let r = self.rect;
        gpu.renderer.set_blit_rect(queue, self.frame.view_id, [r.min.x * ppp, r.min.y * ppp, r.width() * ppp, r.height() * ppp], screen.size_in_pixels);
        Vec::new()
    }

    fn paint(&self, info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, resources: &CallbackResources) {
        let Some(gpu) = resources.get::<GpuState>() else { return };
        let [sw, sh] = info.screen_size_px;
        let clip = info.clip_rect_in_pixels();
        if clip.width_px <= 0 || clip.height_px <= 0 || sw == 0 || sh == 0 {
            return;
        }
        pass.set_viewport(0.0, 0.0, sw as f32, sh as f32, 0.0, 1.0);
        pass.set_scissor_rect(clip.left_px.max(0) as u32, clip.top_px.max(0) as u32, clip.width_px as u32, clip.height_px as u32);
        gpu.renderer.blit_prepared(pass, self.frame.view_id);
    }
}

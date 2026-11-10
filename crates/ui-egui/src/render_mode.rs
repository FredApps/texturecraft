//! Rendering mode (F10): a progressive path-traced view of the current camera, restarted whenever
//! the scene, camera, view size or settings change.
//!
//! Where the device has compute shaders (desktop, WebGPU) it traces on the GPU: the tracer lives in
//! egui's callback resources next to the viewport renderer (whose texture sets it copies), a paint
//! callback adds samples each frame and resolves into a texture egui shows as it is, so nothing is
//! read back until the render is saved. Otherwise (WebGL2, the CPU setting, or a GPU error) the CPU
//! tracer runs on a worker thread (one slice per frame on the single-threaded web).

use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, ScreenDescriptor};
use egui::{Color32, ColorImage, Rect, RichText, TextureHandle, TextureOptions, Ui, vec2};
use texturecraft_engine::geom::{Bvh, Mesh, OrbitCamera};
use texturecraft_render::displace::trace_geometry;
use texturecraft_render::gpu_trace::GpuTracer;
use texturecraft_render::ibl::EnvImage;
use texturecraft_render::readback::Readback;
use texturecraft_render::trace::{TraceDisplay, TraceScene, TraceSet, TraceSettings, Tracer};
use texturecraft_render::{Renderer, wgpu};

use crate::TextureCraftApp;
use crate::i18n::{n_, tr, trf};
use crate::theme::Tokens;

/// A finished pass, ready to show.
struct Frame {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    samples: u32,
}

/// The tracer and everything it needs.
struct Work {
    scene: TraceScene,
    camera: OrbitCamera,
    settings: TraceSettings,
    display: TraceDisplay,
    max_samples: u32,
    tracer: Tracer,
}

impl Work {
    fn frame(&self) -> Frame {
        Frame { width: self.tracer.width, height: self.tracer.height, rgba: self.tracer.rgba8(&self.display), samples: self.tracer.samples }
    }

    /// One more sample per pixel, or None when done.
    #[cfg(not(target_arch = "wasm32"))]
    fn step(&mut self) -> Option<Frame> {
        if self.tracer.samples >= self.max_samples {
            return None;
        }
        self.tracer.pass(&self.scene, &self.camera, &self.settings);
        Some(self.frame())
    }

    /// Trace for about `budget_ms` of this frame (the web has one thread: the UI must keep
    /// running); a frame to show when a pass finished.
    #[cfg(target_arch = "wasm32")]
    fn step_for(&mut self, budget_ms: f64) -> Option<Frame> {
        let start = js_sys::Date::now();
        let mut rows = 4;
        while self.tracer.samples < self.max_samples {
            let t = js_sys::Date::now();
            if self.tracer.pass_rows(&self.scene, &self.camera, &self.settings, rows) {
                return Some(self.frame());
            }
            let now = js_sys::Date::now();
            if now - start >= budget_ms {
                break;
            }
            // Grow the slice while rows are cheap, staying inside the budget.
            if now - t < budget_ms / 8.0 {
                rows *= 2;
            }
        }
        None
    }
}

/// The running CPU render of one scene state.
struct Job {
    key: String,
    latest: Arc<Mutex<Option<Frame>>>,
    started: f64,
    #[cfg(not(target_arch = "wasm32"))]
    cancel: Arc<AtomicBool>,
    #[cfg(target_arch = "wasm32")]
    work: Work,
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

/// What the GPU callback reports back to the UI.
#[derive(Default)]
struct GpuStatus {
    samples: u32,
    error: Option<String>,
}

/// "Save render..." on the GPU: the copy of the shown render on its way to the CPU.
#[derive(Default)]
enum Read {
    #[default]
    Idle,
    /// The next callback copies the render.
    Wanted,
    /// Copied in a submitted frame: map it.
    Copied(Readback),
    Mapping(Readback),
}

/// The GPU render shown in the view.
struct GpuView {
    size: (u32, u32),
    /// What [`GpuView::scene`] was built from (everything but the camera, size and settings).
    scene_key: String,
    scene: Arc<TraceScene>,
    /// The restart key of the samples counted in `status`.
    key: String,
    started: f64,
    out: wgpu::Texture,
    out_view: wgpu::TextureView,
    texture: egui::TextureId,
    status: Arc<Mutex<GpuStatus>>,
    /// Samples per pixel per frame: grows while frames are quick, halves when they are slow.
    spp: u32,
    /// What steers `spp`.
    spp_control: SppControl,
    read: Arc<Mutex<Read>>,
    /// An image of this render was resolved into `texture` at least once, so a restart keeps
    /// showing it (the previous camera's frame) until the new samples replace it.
    has_image: bool,
    /// The egui texture of the last image traced at the smaller moving size, shown instead of
    /// `texture` while the full-size render is too grainy (see [`holds_previous`]).
    held: Option<egui::TextureId>,
    /// When `held` started being shown (egui time, seconds): a restart while it is up (a display
    /// slider dragged) resets `started`, so the hold is timed from this instead.
    held_since: f64,
}

/// While the camera moves, the GPU render is traced at this fraction of the view size per axis
/// (the image is stretched to the view). A quarter of the pixels means about four times the samples
/// per pixel in the same frame time, so an orbit shows a smooth image instead of one or two noisy
/// samples; a faster GPU affords more samples (see `GpuView::spp`) at the same size.
const MOVING_SCALE: f32 = 0.5;

/// Seconds without a camera change after which the render returns to the full view size.
const MOTION_SETTLE_S: f64 = 0.2;

/// While textures load, new tiles are taken into the render at most this often (seconds). Each
/// time restarts the render, so taking them in every frame leaves it forever one or two samples
/// deep (raw noise) until loading ends; this lets about a third of a second of samples build up
/// between restarts, at the price of a little delay before a new tile shows.
const LOADING_RESTART_S: f64 = 0.35;

/// Most samples per frame while textures load (see [`LOADING_RESTART_S`]). The frame time is
/// then the compositor's, not the tracer's, so the count is not steered by it: it stays where it
/// was, at most this (cheap at the smaller loading size).
const LOADING_MAX_SPP: u32 = 8;

/// Whether textures are still loading: tiles wait to be composited, or composited ones to be
/// uploaded (the GPU render copies the uploaded display textures).
fn textures_loading(s: &texturecraft_engine::Session) -> bool {
    s.pending_tiles() > 0 || s.caches.iter().any(|c| !c.upload.is_empty())
}

/// Whether changed textures are taken into the render now: always, except while loading, when at
/// most every [`LOADING_RESTART_S`].
fn take_in_textures(loading: bool, now: f64, last_sync: f64) -> bool {
    !loading || now - last_sync >= LOADING_RESTART_S
}

/// When the render returns to full size after the camera stops, the soft image traced while it
/// moved stays on screen until the full-size render has this many samples (it starts from none,
/// and its first few samples are grainy)...
const SETTLE_HOLD_SAMPLES: u32 = 8;

/// ...or for this many seconds at most, so a slow GPU never leaves the soft image up for long.
const SETTLE_HOLD_MAX_S: f64 = 0.5;

/// Whether the image from before the view returned to full size is still shown: the full-size
/// render has `samples` samples after `elapsed` seconds.
fn holds_previous(samples: u32, elapsed: f64) -> bool {
    samples < SETTLE_HOLD_SAMPLES && elapsed < SETTLE_HOLD_MAX_S
}

/// Most samples per pixel one frame adds.
const MAX_SPP: u32 = 16;

/// Frames over which [`FrameFloor`] keeps the quickest frame time.
const FLOOR_FRAMES: usize = 30;

/// The range the quickest frame time is held to (seconds): from 75 frames a second to 30, so a
/// run of slow frames never makes slow look normal. The quick end matters as much: frames that
/// are not paced by the display (or tiny ones while little is traced) can take 3 ms, and taking
/// that as the baseline cuts the samples per frame to one at the first frame that does real work,
/// which left the render grainy for seconds. Below this, frames count as quick, not as the target.
const FLOOR_RANGE_S: (f64, f64) = (1.0 / 75.0, 1.0 / 30.0);

/// A frame quicker than this many times the quickest recent one adds one more sample next time.
/// With vsync, frames come at the display's refresh rate however light they are, so the limits
/// are relative to it (a fixed 1/90 s never let a 60 Hz display grow at all).
const SPP_GROW_BELOW: f64 = 1.25;

/// A frame at least this many times the quickest recent one (a missed refresh with vsync is two;
/// wide enough that ordinary variation in a busy frame does not cut) cuts the samples per frame (by a
/// quarter, by the same ratio the frame overshoots when that is larger), so a slow frame is
/// followed by a short one, not a long one. With vsync a frame that misses the refresh takes two.
const SPP_CUT_ABOVE: f64 = 1.75;

/// Frames after a cut during which the samples per frame do not grow again, so they settle
/// below the limit instead of climbing straight back to the frame that missed it (a hitch every
/// few frames).
const SPP_GROW_PAUSE_FRAMES: u32 = 6;

/// The quickest of the last [`FLOOR_FRAMES`] frame times: about the display's refresh interval
/// while the GPU keeps up.
#[derive(Default)]
struct FrameFloor {
    recent: std::collections::VecDeque<f64>,
}

impl FrameFloor {
    /// Record frame time `dt` (seconds); the quickest recent one, within [`FLOOR_RANGE_S`].
    fn push(&mut self, dt: f64) -> f64 {
        if dt.is_finite() && dt > 0.0 {
            if self.recent.len() >= FLOOR_FRAMES {
                self.recent.pop_front();
            }
            self.recent.push_back(dt);
        }
        self.recent.iter().copied().fold(FLOOR_RANGE_S.1, f64::min).clamp(FLOOR_RANGE_S.0, FLOOR_RANGE_S.1)
    }
}

/// Samples per pixel for the next frame, from the last frame's time `dt` and the quickest recent
/// one `floor` (seconds; see [`FrameFloor`]): one more while frames are quick, fewer when slow,
/// steady in between and for [`SPP_GROW_PAUSE_FRAMES`] after a cut (`since_cut` frames ago).
/// Steady between the two limits is what stops the count from climbing until the GPU backs up,
/// jumping down, and climbing again.
fn next_spp(spp: u32, dt: f64, floor: f64, since_cut: u32) -> u32 {
    let cut_above = floor * SPP_CUT_ABOVE;
    if dt >= cut_above {
        let by_ratio = (f64::from(spp) * cut_above / dt) as u32;
        by_ratio.min(spp * 3 / 4).max(1)
    } else if dt < floor * SPP_GROW_BELOW && since_cut >= SPP_GROW_PAUSE_FRAMES {
        (spp + 1).min(MAX_SPP)
    } else {
        spp
    }
}

/// What steers the GPU render's samples per pixel per frame (see [`next_spp`]).
struct SppControl {
    floor: FrameFloor,
    /// Frames since the samples per frame were last cut.
    since_cut: u32,
}

impl Default for SppControl {
    /// A new render may grow from its first frame.
    fn default() -> SppControl {
        SppControl { floor: FrameFloor::default(), since_cut: SPP_GROW_PAUSE_FRAMES }
    }
}

impl SppControl {
    /// The samples per frame after a frame of `dt` seconds traced with `spp`.
    fn next(&mut self, spp: u32, dt: f64) -> u32 {
        let floor = self.floor.push(dt);
        let next = next_spp(spp, dt, floor, self.since_cut);
        self.since_cut = if next < spp { 0 } else { self.since_cut.saturating_add(1) };
        next
    }
}

/// Samples per frame after the traced size changed from `old` to `new` pixels (frame time is
/// about proportional to samples times pixels).
fn rescale_spp(spp: u32, old: (u32, u32), new: (u32, u32)) -> u32 {
    let (o, n) = (u64::from(old.0) * u64::from(old.1), (u64::from(new.0) * u64::from(new.1)).max(1));
    ((u64::from(spp) * o / n).max(1) as u32).min(MAX_SPP)
}

/// Whether the camera is moving, for [`MOVING_SCALE`].
#[derive(Default)]
struct Motion {
    /// The camera as last seen (compared, never parsed).
    camera: String,
    /// When it last changed. The first frame counts as a change: a render that has just started
    /// begins at the smaller size too (four times the samples at once, so it opens smooth
    /// instead of as raw one-sample noise) and grows to full size like after any move.
    last_change: Option<f64>,
}

impl Motion {
    /// Record `camera` at time `now` (seconds); true while it changed within [`MOTION_SETTLE_S`].
    /// `loading`: textures are still being composited. Each finished tile restarts the render, so
    /// like a moving camera it keeps being redone from no samples, and is traced small for the
    /// same reason (four times the samples per restart).
    fn moving(&mut self, camera: &OrbitCamera, now: f64, loading: bool) -> bool {
        if loading {
            self.last_change = Some(now);
        }
        let c = format!("{camera:?}");
        if self.camera != c {
            self.last_change = Some(now);
            self.camera = c;
        }
        self.last_change.is_some_and(|t| now - t < MOTION_SETTLE_S)
    }
}

/// The size to trace at for a view of `full` pixels: smaller while the camera `moving`.
fn traced_size(full: (u32, u32), moving: bool) -> (u32, u32) {
    if !moving {
        return full;
    }
    let scale = |n: u32| ((n as f32 * MOVING_SCALE) as u32).clamp(16.min(n), n);
    (scale(full.0), scale(full.1))
}

/// The GPU tracer in egui's callback resources.
struct TraceState {
    tracer: GpuTracer,
    scene: Option<Arc<TraceScene>>,
    key: String,
    /// Samples in the resolved image (None: resolve again).
    resolved: Option<u32>,
    /// When the tracer last took in changed textures (egui time, seconds).
    last_sync: f64,
}

/// Traced geometry and the key of what it was built from.
type Geometry = (String, Arc<Mesh>, Arc<Bvh>);

/// Rendering mode state kept by the app.
#[derive(Default)]
pub struct RenderView {
    job: Option<Job>,
    gpu: Option<GpuView>,
    /// Why the GPU tracer failed on the scene of this key (the CPU renders it instead).
    gpu_error: Option<(String, String)>,
    texture: Option<TextureHandle>,
    motion: Motion,
    shown: Option<(u32, u32, Vec<u8>)>,
    samples: u32,
    elapsed: f64,
    /// The traced geometry (displaced or not) and what it was built from: changes to the display
    /// alone don't rebuild it.
    geometry: Mutex<Option<Geometry>>,
    /// The traced environment and what it was built from (see [`EnvKey`]): building one takes a
    /// while, and the scene is rebuilt on every display change.
    env: Mutex<Option<(EnvKey, Arc<EnvImage>)>>,
}

/// What a traced environment is built from: the display's environment name and the identity of
/// its imported image (0: none).
type EnvKey = (String, usize);

/// Viewport resolution of a CPU render (the web renders at half size: it has one thread).
fn render_size(rect: Rect) -> (u32, u32) {
    let scale = if cfg!(target_arch = "wasm32") { 0.5 } else { 1.0 };
    ((rect.width() * scale).max(16.0) as u32, (rect.height() * scale).max(16.0) as u32)
}

fn trace_scene(app: &TextureCraftApp) -> TraceScene {
    let s = &app.session;
    let d = &s.doc.project.display;
    let sets: Vec<TraceSet> = s
        .doc
        .project
        .texture_sets
        .iter()
        .zip(&s.caches)
        .map(|(ts, c)| TraceSet { material: ts.material, composite: c.composite.clone(), height_strength: ts.atlas_height_strength() })
        .collect();
    let env_image = d.environment_resource().and_then(|n| s.images.get(n));
    // Only displaced geometry depends on the composites (their heights).
    let revisions: Vec<u64> = if d.displacement { s.caches.iter().map(|c| c.revision).collect() } else { Vec::new() };
    let strengths: Vec<f32> = sets.iter().map(|t| t.height_strength).collect();
    let key = format!("{}|{revisions:?}|{strengths:?}|{}", s.mesh_id, d.displacement);
    let mut cached = app.render.geometry.lock().unwrap_or_else(PoisonError::into_inner);
    let (mesh, bvh) = match cached.as_ref() {
        Some((k, m, b)) if *k == key => (m.clone(), b.clone()),
        _ => {
            let (m, b) = trace_geometry(&s.mesh, &s.bvh, &sets, d);
            *cached = Some((key, m.clone(), b.clone()));
            (m, b)
        }
    };
    drop(cached);
    let env_key: EnvKey = (d.environment.clone(), env_image.map_or(0, |i| Arc::as_ptr(i) as usize));
    let mut env_cached = app.render.env.lock().unwrap_or_else(PoisonError::into_inner);
    let env = match env_cached.as_ref() {
        Some((k, e)) if *k == env_key => e.clone(),
        _ => {
            let e = Arc::new(TraceScene::display_env(d, env_image.map(|i| i.as_ref())));
            *env_cached = Some((env_key, e.clone()));
            e
        }
    };
    TraceScene::for_display_env(mesh, bvh, sets, d, env)
}

fn settings(app: &TextureCraftApp) -> TraceSettings {
    TraceSettings { bounces: app.ui.render.bounces }
}

fn display(app: &TextureCraftApp) -> TraceDisplay {
    TraceDisplay { denoise: app.ui.render.denoise, ..TraceDisplay::from_display(&app.session.doc.project.display) }
}

fn work(app: &TextureCraftApp, size: (u32, u32)) -> Work {
    Work {
        scene: trace_scene(app),
        camera: app.session.camera,
        settings: settings(app),
        display: display(app),
        max_samples: app.ui.render.max_samples.max(1),
        tracer: Tracer::new(size.0, size.1),
    }
}

/// What the traced scene depends on. `composites`: the texture-set composites too (their
/// revisions bump with every composited chunk, so while textures load this changes every frame).
fn scene_key(s: &texturecraft_engine::Session, composites: bool) -> String {
    let revisions: Vec<u64> = if composites { s.caches.iter().map(|c| c.revision).collect() } else { Vec::new() };
    format!("{}|{revisions:?}|{:?}", s.mesh_id, s.doc.project.display)
}

/// [`scene_key`] of the GPU render. It takes texture changes in by copying the display textures
/// when their content changes (throttled while textures load, see [`take_in_textures`]), so the
/// composites only count when they displace the geometry. Restarting on every composited chunk
/// would rebuild the scene and reset the render every frame of a load.
fn gpu_scene_key(s: &texturecraft_engine::Session) -> String {
    scene_key(s, s.doc.project.display.displacement)
}

/// What the render of scene `scene_key` depends on; a change restarts it.
fn key(app: &TextureCraftApp, scene_key: &str, size: (u32, u32)) -> String {
    format!("{scene_key}|{:?}|{size:?}|{:?}", app.session.camera, app.ui.render)
}

fn start(app: &TextureCraftApp, ctx: &egui::Context, key: String, size: (u32, u32)) -> Job {
    let latest = Arc::new(Mutex::new(None));
    let started = ctx.input(|i| i.time);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut w = work(app, size);
        let (out, stop, ctx) = (latest.clone(), cancel.clone(), ctx.clone());
        let spawned = std::thread::Builder::new().name("texturecraft-render".into()).spawn(move || {
            while !stop.load(Ordering::Acquire) {
                let Some(frame) = w.step() else { break };
                if stop.load(Ordering::Acquire) {
                    break;
                }
                *out.lock().unwrap_or_else(PoisonError::into_inner) = Some(frame);
                ctx.request_repaint();
            }
        });
        if let Err(e) = spawned {
            log::error!("could not start the render thread: {e}");
        }
        Job { key, latest, started, cancel }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = ctx;
        Job { key, latest, started, work: work(app, size) }
    }
}

/// Draw Rendering mode over the 3D view in `rect` (navigation is handled by the 3D view).
pub fn show(app: &mut TextureCraftApp, ui: &mut Ui, rect: Rect) {
    let on_gpu = !app.ui.render.cpu && show_gpu(app, ui, rect);
    if !on_gpu {
        drop_gpu(app);
        show_cpu(app, ui, rect);
    }
    overlay(app, ui, rect, on_gpu);
}

fn show_cpu(app: &mut TextureCraftApp, ui: &mut Ui, rect: Rect) {
    let ctx = ui.ctx().clone();
    let size = render_size(rect);
    // The CPU tracer reads the composites themselves: every change restarts it.
    let k = key(app, &scene_key(&app.session, true), size);
    if app.render.job.as_ref().is_none_or(|j| j.key != k) {
        app.render.job = Some(start(app, &ctx, k, size));
        app.render.samples = 0;
    }
    let now = ctx.input(|i| i.time);
    if let Some(job) = app.render.job.as_mut() {
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(f) = job.work.step_for(30.0) {
                *job.latest.lock().unwrap_or_else(PoisonError::into_inner) = Some(f);
            }
            if job.work.tracer.samples < job.work.max_samples {
                ctx.request_repaint();
            }
        }
        if let Some(f) = job.latest.lock().unwrap_or_else(PoisonError::into_inner).take() {
            let img = ColorImage::from_rgba_unmultiplied([f.width as usize, f.height as usize], &f.rgba);
            match app.render.texture.as_mut() {
                Some(t) => t.set(img, TextureOptions::LINEAR),
                None => app.render.texture = Some(ctx.load_texture("render-view", img, TextureOptions::LINEAR)),
            }
            app.render.samples = f.samples;
            app.render.elapsed = now - job.started;
            app.render.shown = Some((f.width, f.height, f.rgba));
        }
    }
    let shown = app.render.texture.as_ref().filter(|_| show_image(true, app.render.samples)).map(TextureHandle::id);
    paint(ui, rect, shown);
}

/// Whether the view shows the render image rather than the "Starting render..." placeholder.
/// `has_image`: an earlier frame of this render is on screen. A restart (the camera moved) resets
/// the sample count to 0 but keeps that frame up, so orbiting doesn't flash the placeholder.
/// Only the very first frame, before anything was traced, has nothing to show.
fn show_image(has_image: bool, samples: u32) -> bool {
    has_image || samples > 0
}

/// The render, or a placeholder until its first samples.
fn paint(ui: &Ui, rect: Rect, texture: Option<egui::TextureId>) {
    let tk = Tokens::get(ui.ctx());
    let p = ui.painter_at(rect);
    match texture {
        Some(t) => {
            p.image(t, rect, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            p.rect_filled(rect, 0.0, tk.viewport);
            p.text(rect.center(), egui::Align2::CENTER_CENTER, tr("Starting render..."), egui::FontId::proportional(14.0), tk.viewport_text);
        }
    }
}

/// Trace on the GPU; false when it can't (no compute shaders, or it failed on this scene).
fn show_gpu(app: &mut TextureCraftApp, ui: &Ui, rect: Rect) -> bool {
    let Some(rs) = app.render_state.clone() else { return false };
    let scene_key = gpu_scene_key(&app.session);
    if !GpuTracer::supported(&rs.device) || app.render.gpu_error.as_ref().is_some_and(|(k, _)| *k == scene_key) {
        return false;
    }
    app.render.gpu_error = None;
    if app.render.job.take().is_some() {
        app.render.texture = None;
        app.render.shown = None;
    }
    let ctx = ui.ctx().clone();
    let now = ctx.input(|i| i.time);
    let full = (rect.width().max(16.0) as u32, rect.height().max(16.0) as u32);
    let loading = textures_loading(&app.session);
    let moving = app.render.motion.moving(&app.session.camera, now, loading);
    if moving {
        // Come back and restore the full size once the camera has been still, even with no input.
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(MOTION_SETTLE_S));
    }
    let size = traced_size(full, moving);
    let key = key(app, &scene_key, size);
    let mut g = match app.render.gpu.take() {
        Some(g) if g.size == size => g,
        old => {
            let out = GpuTracer::output_texture(&rs.device, size.0, size.1);
            let out_view = out.create_view(&Default::default());
            // A resized view keeps its scene and egui texture id.
            let old_has_image = old.as_ref().is_some_and(|o| o.has_image);
            let old_spp = old.as_ref().map_or(1, |o| rescale_spp(o.spp, o.size, size));
            let mut held_since = old.as_ref().map_or(now, |o| o.held_since);
            let growing = old.as_ref().is_some_and(|o| u64::from(o.size.0) * u64::from(o.size.1) < u64::from(size.0) * u64::from(size.1));
            let mut held = None;
            let (texture, scene_key, scene, spp_control) = match old {
                // Returning to full size: the old image stays up under its own texture id.
                Some(old) if growing && old.has_image => {
                    let mut r = rs.renderer.write();
                    if let Some(h) = old.held {
                        r.free_texture(&h);
                    }
                    held = Some(old.texture);
                    held_since = now;
                    (r.register_native_texture(&rs.device, &out_view, wgpu::FilterMode::Linear), old.scene_key, old.scene, old.spp_control)
                }
                Some(old) => {
                    let mut r = rs.renderer.write();
                    if let Some(h) = old.held {
                        r.free_texture(&h);
                    }
                    r.update_egui_texture_from_wgpu_texture(&rs.device, &out_view, wgpu::FilterMode::Linear, old.texture);
                    (old.texture, old.scene_key, old.scene, old.spp_control)
                }
                None => (
                    rs.renderer.write().register_native_texture(&rs.device, &out_view, wgpu::FilterMode::Linear),
                    scene_key.clone(),
                    Arc::new(trace_scene(app)),
                    SppControl::default(),
                ),
            };
            GpuView {
                size,
                scene_key,
                scene,
                key: String::new(),
                started: now,
                out,
                out_view,
                texture,
                status: Arc::default(),
                spp: old_spp,
                spp_control,
                read: Arc::default(),
                has_image: old_has_image,
                held,
                held_since,
            }
        }
    };
    let (samples, error) = {
        let mut st = g.status.lock().unwrap_or_else(PoisonError::into_inner);
        g.has_image |= st.samples > 0;
        if g.key != key {
            st.samples = 0;
        }
        (st.samples, st.error.take())
    };
    if let Some(e) = error {
        log::warn!("GPU path tracing failed, rendering on the CPU: {e}");
        app.render.gpu_error = Some((scene_key, e));
        app.render.gpu = Some(g);
        return false;
    }
    if g.key != key {
        g.key = key.clone();
        g.started = now;
    }
    if g.scene_key != scene_key {
        g.scene = Arc::new(trace_scene(app));
        g.scene_key = scene_key;
    }
    let max_samples = app.ui.render.max_samples.max(1);
    if samples < max_samples {
        // While textures load the frame time is the compositor's, not the tracer's: reading it
        // would cut the samples to one. Keep them as they are (see `LOADING_MAX_SPP`).
        g.spp = if loading { g.spp.min(LOADING_MAX_SPP) } else { g.spp_control.next(g.spp, f64::from(ctx.input(|i| i.stable_dt))) };
        app.render.elapsed = now - g.started;
        ctx.request_repaint();
    }
    app.render.samples = samples;
    if poll_read(app, &rs.device, &g) {
        ctx.request_repaint();
    }
    let cb = TraceCallback {
        key,
        scene: g.scene.clone(),
        camera: app.session.camera,
        settings: settings(app),
        display: display(app),
        max_samples,
        spp: g.spp,
        loading,
        now,
        size,
        out: g.out.clone(),
        out_view: g.out_view.clone(),
        status: g.status.clone(),
        read: g.read.clone(),
    };
    ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, cb));
    if g.held.is_some()
        && !holds_previous(samples, now - g.held_since)
        && let Some(h) = g.held.take()
    {
        rs.renderer.write().free_texture(&h);
    }
    paint(ui, rect, show_image(g.has_image, samples).then_some(g.held.unwrap_or(g.texture)));
    app.render.gpu = Some(g);
    true
}

/// Advance "Save render..." on the GPU: map the copy once its frame was submitted, then save.
/// True while it is pending.
fn poll_read(app: &mut TextureCraftApp, device: &wgpu::Device, g: &GpuView) -> bool {
    let mut read = g.read.lock().unwrap_or_else(PoisonError::into_inner);
    let pixels = match std::mem::take(&mut *read) {
        Read::Copied(r) => {
            r.map();
            *read = Read::Mapping(r);
            None
        }
        Read::Mapping(r) => {
            // Runs map callbacks on native backends (the browser runs them on its own).
            let _ = device.poll(wgpu::PollType::Poll);
            match r.take() {
                Some(px) => Some(px),
                None => {
                    *read = Read::Mapping(r);
                    None
                }
            }
        }
        other => {
            *read = other;
            None
        }
    };
    let pending = !matches!(*read, Read::Idle);
    drop(read);
    match pixels {
        Some(Some(px)) => {
            app.render.shown = Some(px);
            save(app);
        }
        Some(None) => app.toast(tr("Couldn't read the render back from the GPU"), true),
        None => {}
    }
    pending
}

/// Adds samples on the GPU and resolves them into the shown texture.
struct TraceCallback {
    key: String,
    scene: Arc<TraceScene>,
    camera: OrbitCamera,
    settings: TraceSettings,
    display: TraceDisplay,
    max_samples: u32,
    spp: u32,
    /// Textures are still loading (see [`LOADING_RESTART_S`]) and the egui time of this frame.
    loading: bool,
    now: f64,
    size: (u32, u32),
    out: wgpu::Texture,
    out_view: wgpu::TextureView,
    status: Arc<Mutex<GpuStatus>>,
    read: Arc<Mutex<Read>>,
}

impl TraceCallback {
    fn run(
        &self,
        st: &mut TraceState,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        renderer: &Renderer,
    ) -> Result<(), String> {
        let t = &mut st.tracer;
        if !st.scene.as_ref().is_some_and(|s| Arc::ptr_eq(s, &self.scene)) {
            t.set_scene(device, queue, &self.scene)?;
            st.scene = Some(self.scene.clone());
        }
        let take_in = take_in_textures(self.loading, self.now, st.last_sync);
        if take_in {
            st.last_sync = self.now;
        }
        if (take_in && t.sync_materials(device, queue, encoder, renderer)?) || st.key != self.key {
            t.reset(device, encoder, self.size.0, self.size.1)?;
            st.key.clone_from(&self.key);
            st.resolved = None;
        }
        let done = t.samples();
        if done < self.max_samples {
            t.pass(device, queue, encoder, &self.camera, &self.settings, self.spp.min(self.max_samples - done));
        }
        if st.resolved != Some(t.samples()) {
            t.resolve(device, queue, encoder, &self.display, &self.out_view);
            st.resolved = Some(t.samples());
        }
        let mut read = self.read.lock().unwrap_or_else(PoisonError::into_inner);
        if matches!(*read, Read::Wanted) {
            *read = Read::Copied(Readback::new(device, encoder, &self.out));
        }
        Ok(())
    }
}

impl CallbackTrait for TraceCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let state = match resources.remove::<TraceState>() {
            Some(st) => Ok(st),
            None => GpuTracer::new(device).map(|tracer| TraceState { tracer, scene: None, key: String::new(), resolved: None, last_sync: f64::MIN }),
        };
        let result = state.and_then(|mut st| {
            let r = match resources.get::<crate::gpu::GpuState>() {
                Some(gpu) => self.run(&mut st, device, queue, encoder, &gpu.renderer),
                None => Err("the viewport renderer is missing".into()),
            };
            let samples = st.tracer.samples();
            resources.insert(st);
            r.map(|()| samples)
        });
        let mut status = self.status.lock().unwrap_or_else(PoisonError::into_inner);
        match result {
            Ok(samples) => status.samples = samples,
            Err(e) => status.error = Some(e),
        }
        Vec::new()
    }

    fn paint(&self, _info: egui::PaintCallbackInfo, _pass: &mut wgpu::RenderPass<'static>, _resources: &CallbackResources) {}
}

/// Free the GPU render (its texture and the tracer's scene memory; the pipelines stay).
fn drop_gpu(app: &mut TextureCraftApp) {
    let Some(g) = app.render.gpu.take() else { return };
    if let Some(rs) = &app.render_state {
        let mut r = rs.renderer.write();
        r.free_texture(&g.texture);
        if let Some(h) = g.held {
            r.free_texture(&h);
        }
        if let Some(st) = r.callback_resources.get_mut::<TraceState>() {
            st.tracer.release();
            (st.scene, st.resolved) = (None, None);
            st.key.clear();
            // The materials went with the release: the next render takes textures in at once.
            st.last_sync = f64::MIN;
        }
    }
}

/// The render status and settings card in the top-left corner.
fn overlay(app: &mut TextureCraftApp, ui: &mut Ui, rect: Rect, on_gpu: bool) {
    let tk = Tokens::get(ui.ctx());
    let card = Rect::from_min_size(rect.min + vec2(12.0, 12.0), vec2(250.0, 200.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(card).layout(egui::Layout::top_down(egui::Align::Min)));
    egui::Frame::popup(child.style()).show(&mut child, |ui| {
        ui.set_width(226.0);
        let max = app.ui.render.max_samples.max(1);
        let running = app.render.samples < max;
        ui.label(RichText::new(tr("RENDERING")).size(11.0).strong());
        let template = match (running, on_gpu) {
            (true, true) => n_("Rendering on GPU: {n} / {max} samples, {s} s"),
            (true, false) => n_("Rendering on CPU: {n} / {max} samples, {s} s"),
            (false, true) => n_("Done on GPU: {n} / {max} samples, {s} s"),
            (false, false) => n_("Done on CPU: {n} / {max} samples, {s} s"),
        };
        let line = ui.label(
            RichText::new(trf(template, &[("n", &app.render.samples), ("max", &max), ("s", &format!("{:.1}", app.render.elapsed))]))
                .size(11.0)
                .color(tk.text_dim),
        );
        if let Some((_, e)) = app.render.gpu_error.as_ref().filter(|_| !on_gpu && !app.ui.render.cpu) {
            line.on_hover_text(trf("The GPU couldn't render this scene: {error}", &[("error", e)]));
        }
        ui.add(egui::ProgressBar::new(app.render.samples as f32 / max as f32).desired_height(4.0));
        ui.horizontal(|ui| {
            ui.label(tr("Samples"));
            ui.add(egui::DragValue::new(&mut app.ui.render.max_samples).range(1..=16384));
            ui.label(tr("Bounces"));
            ui.add(egui::DragValue::new(&mut app.ui.render.bounces).range(1..=32));
        });
        ui.checkbox(&mut app.ui.render.denoise, tr("Denoise")).on_hover_text(tr("Filter the remaining noise while keeping edges and texture detail"));
        if app.render_state.as_ref().is_some_and(|rs| GpuTracer::supported(&rs.device)) {
            ui.checkbox(&mut app.ui.render.cpu, tr("Render on the CPU")).on_hover_text(tr("Path trace on the processor instead of the graphics card (slower)"));
        }
        ui.horizontal(|ui| {
            let saving = app.render.gpu.as_ref().is_some_and(|g| !matches!(*g.read.lock().unwrap_or_else(PoisonError::into_inner), Read::Idle));
            let ready = if on_gpu { app.render.samples > 0 && !saving } else { app.render.shown.is_some() };
            if ui.add_enabled(ready, egui::Button::new(tr("Save render..."))).clicked() {
                match app.render.gpu.as_ref().filter(|_| on_gpu) {
                    Some(g) => *g.read.lock().unwrap_or_else(PoisonError::into_inner) = Read::Wanted,
                    None => save(app),
                }
            }
            if ui.button(tr("Back to painting")).clicked() {
                app.ui.mode = crate::state::Mode::Painting;
            }
        });
    });
}

/// Save the shown render as PNG.
fn save(app: &mut TextureCraftApp) {
    let Some((w, h, rgba)) = app.render.shown.clone() else { return };
    let Some(png) = crate::control::encode_png(w, h, rgba).map(|(_, _, png)| png) else {
        app.toast(tr("Couldn't encode the render"), true);
        return;
    };
    let name = format!("{}_render.png", texturecraft_engine::formats::export::sanitize_file_stem(&app.session.doc.project.name));
    #[cfg(not(target_arch = "wasm32"))]
    {
        use crate::file_dialogs::{Bound, FileDialog};
        app.ask_file(FileDialog::save().filter(tr("PNG image"), &["png"]).file_name(name), Bound::App, move |app, paths| {
            let Some(path) = paths.first() else { return };
            match std::fs::write(path, &png) {
                Ok(()) => app.toast(trf("Saved {path}", &[("path", &path.display())]), false),
                Err(e) => app.toast(trf("Couldn't save the render: {error}", &[("error", &e)]), true),
            }
        });
    }
    #[cfg(target_arch = "wasm32")]
    match crate::web_files::download(&name, &png) {
        Ok(()) => app.toast(trf("Downloaded {name}", &[("name", &name)]), false),
        Err(e) => app.toast(trf("Couldn't save the render: {error}", &[("error", &e)]), true),
    }
}

impl RenderView {
    pub fn is_active(&self) -> bool {
        self.job.is_some() || self.gpu.is_some() || self.texture.is_some()
    }
}

/// Stop rendering (leaving Rendering mode frees the worker, the GPU memory and the image).
pub fn stop(app: &mut TextureCraftApp) {
    drop_gpu(app);
    app.render = RenderView::default();
}

#[cfg(test)]
mod tests {
    use super::{
        FLOOR_FRAMES, FLOOR_RANGE_S, FrameFloor, LOADING_RESTART_S, MAX_SPP, MOTION_SETTLE_S, Motion, OrbitCamera, SETTLE_HOLD_MAX_S, SETTLE_HOLD_SAMPLES,
        SPP_GROW_PAUSE_FRAMES, SppControl, gpu_scene_key, holds_previous, next_spp, rescale_spp, scene_key, show_image, take_in_textures, textures_loading,
        traced_size,
    };
    use texturecraft_engine::Session;
    use texturecraft_engine::geom::primitives;

    fn session() -> Session {
        let mut s = Session::new_project("t", primitives::sphere(1.0, 16, 8), "builtin:sphere", None, 128).unwrap();
        s.update();
        s
    }

    /// Every composited chunk bumps a revision. The CPU render restarts on it (it reads the
    /// composites); the GPU render does not (it copies the display textures when they change,
    /// throttled while loading) unless the heights displace its geometry.
    #[test]
    fn composited_chunks_restart_the_gpu_render_only_when_displaced() {
        let mut s = session();
        let (cpu, gpu) = (scene_key(&s, true), gpu_scene_key(&s));
        s.caches[0].revision += 1;
        assert_ne!(scene_key(&s, true), cpu);
        assert_eq!(gpu_scene_key(&s), gpu, "the GPU scene is kept while textures load");
        s.doc.project.display.displacement = true;
        let displaced = gpu_scene_key(&s);
        s.caches[0].revision += 1;
        assert_ne!(gpu_scene_key(&s), displaced, "displaced geometry follows the heights");
    }

    /// Loading lasts until the composited tiles are uploaded too (the GPU render reads them).
    #[test]
    fn textures_load_until_their_tiles_are_uploaded() {
        let mut s = session();
        s.caches[0].upload.clear();
        assert!(!textures_loading(&s));
        s.caches[0].upload.insert((0, 0));
        assert!(textures_loading(&s), "a tile waits to be uploaded");
        s.caches[0].upload.clear();
        s.caches[0].invalidate_all();
        assert!(textures_loading(&s), "tiles wait to be composited");
    }

    /// Orbiting restarts the render every frame (samples back to 0): the last image stays up.
    #[test]
    fn a_restart_keeps_the_previous_image_on_screen() {
        assert!(show_image(true, 0), "restart with an earlier frame must not show the placeholder");
        assert!(show_image(true, 5));
        assert!(show_image(false, 1));
    }

    /// Before anything was traced there is nothing to show but the placeholder.
    #[test]
    fn a_moving_camera_traces_smaller_and_a_still_one_at_full_size() {
        assert_eq!(traced_size((1600, 900), false), (1600, 900));
        assert_eq!(traced_size((1600, 900), true), (800, 450));
        // Tiny views are never made smaller than they are or than 16 pixels.
        assert_eq!(traced_size((10, 10), true), (10, 10));
        assert_eq!(traced_size((20, 400), true), (16, 200));
    }

    #[test]
    fn motion_starts_on_a_camera_change_and_settles_after_a_moment() {
        let mut m = Motion::default();
        let mut cam = OrbitCamera::default();
        assert!(m.moving(&cam, 0.0, false), "a render that just started opens at the smaller size");
        assert!(!m.moving(&cam, 1.0, false));
        cam.yaw += 0.1;
        assert!(m.moving(&cam, 2.0, false));
        assert!(m.moving(&cam, 2.0 + MOTION_SETTLE_S * 0.5, false));
        assert!(!m.moving(&cam, 2.0 + MOTION_SETTLE_S * 1.5, false));
        assert!(m.moving(&cam, 5.0, true), "textures still loading count as motion");
        assert!(m.moving(&cam, 5.0 + MOTION_SETTLE_S * 0.5, false), "and settle a moment after they finish");
        assert!(!m.moving(&cam, 5.0 + MOTION_SETTLE_S * 1.5, false));
    }

    #[test]
    fn the_soft_image_stays_until_the_full_size_render_has_samples_or_time_runs_out() {
        assert!(holds_previous(0, 0.0));
        assert!(holds_previous(SETTLE_HOLD_SAMPLES - 1, SETTLE_HOLD_MAX_S * 0.5));
        assert!(!holds_previous(SETTLE_HOLD_SAMPLES, 0.0), "enough samples: show the new render");
        assert!(!holds_previous(0, SETTLE_HOLD_MAX_S), "a slow GPU: don't hold it forever");
    }

    #[test]
    fn samples_per_frame_grow_slowly_hold_in_between_and_drop_fast() {
        let (floor, free) = (1.0 / 144.0, SPP_GROW_PAUSE_FRAMES);
        assert_eq!(next_spp(4, floor, floor, free), 5, "frames at the refresh rate: one more");
        assert_eq!(next_spp(MAX_SPP, floor, floor, free), MAX_SPP);
        assert_eq!(next_spp(4, floor, floor, 0), 4, "just after a cut: no growth yet");
        assert_eq!(next_spp(6, floor * 1.4, floor, free), 6, "between the limits: steady");
        assert_eq!(next_spp(8, floor * 2.5, floor, free), 5, "slow frame: cut by the overshoot (8 * 1.75/2.5), more than a quarter");
        assert_eq!(next_spp(8, floor * 1.7, floor, free), 8, "a little slow: steady, not a cut (cuts are for real overload)");
        assert_eq!(next_spp(8, floor * 1.8, floor, free), 6, "slightly over the limit: a quarter fewer");
        assert_eq!(next_spp(8, floor * 20.0, floor, free), 1, "very slow frame: by the overshoot, at least one");
        assert_eq!(next_spp(1, floor * 20.0, floor, free), 1);
    }

    /// With vsync on a 60 Hz display every frame takes ~16.7 ms however light: that is the
    /// refresh, not a slow GPU, so the samples grow; a missed refresh (two intervals) cuts them.
    #[test]
    fn samples_per_frame_grow_at_60_hz_and_drop_on_a_missed_refresh() {
        let hz60 = 1.0 / 60.0;
        let mut c = SppControl::default();
        let mut spp = 2;
        for _ in 0..10 {
            spp = c.next(spp, hz60);
        }
        assert_eq!(spp, 12, "one more per frame at the refresh rate");
        let cut = c.next(spp, 2.0 * hz60);
        assert!(cut < spp && cut >= spp / 2, "a missed refresh cuts: {spp} -> {cut}");
        // No climb straight back to the frame that missed: steady for a while, then growth again.
        let mut after = cut;
        for _ in 0..SPP_GROW_PAUSE_FRAMES - 1 {
            after = c.next(after, hz60);
        }
        assert_eq!(after, cut, "steady after the cut");
        for _ in 0..3 {
            after = c.next(after, hz60);
        }
        assert!(after > cut, "then it grows again");
    }

    /// Frames that are not paced by the display (or take almost no work) can be a few ms; a frame
    /// that then does real work must not read as slow and cut the samples to one (the render was
    /// left grainy for seconds).
    #[test]
    fn a_few_very_quick_frames_do_not_collapse_the_samples_per_frame() {
        let mut c = SppControl::default();
        for _ in 0..5 {
            c.next(4, 0.003);
        }
        let working = 0.012;
        let mut spp = 4;
        for _ in 0..10 {
            spp = c.next(spp, working);
        }
        assert!(spp >= 4, "12 ms frames after 3 ms ones are fine: {spp}");
    }

    #[test]
    fn the_frame_floor_is_the_quickest_recent_frame_within_range() {
        let mut f = FrameFloor::default();
        assert_eq!(f.push(1.0 / 60.0), 1.0 / 60.0);
        assert_eq!(f.push(0.05), 1.0 / 60.0, "a slow frame does not raise it");
        assert_eq!(f.push(f64::NAN), 1.0 / 60.0, "a broken time is ignored");
        assert_eq!(f.push(0.0001), FLOOR_RANGE_S.0, "never below the quick end of the range");
        for _ in 0..FLOOR_FRAMES {
            f.push(0.2);
        }
        assert_eq!(f.push(0.2), FLOOR_RANGE_S.1, "only recent frames count, and at most 1/30 s");
    }

    #[test]
    fn samples_per_frame_follow_the_pixel_count_when_the_size_changes() {
        assert_eq!(rescale_spp(12, (500, 400), (1000, 800)), 3, "four times the pixels: a quarter of the samples");
        assert_eq!(rescale_spp(3, (1000, 800), (500, 400)), 12);
        assert_eq!(rescale_spp(1, (500, 400), (1000, 800)), 1, "never below one");
        assert_eq!(rescale_spp(16, (100, 100), (10, 10)), MAX_SPP, "never above the cap");
    }

    #[test]
    fn loading_textures_are_taken_in_at_a_limited_rate() {
        assert!(take_in_textures(false, 1.0, 1.0), "not loading: every frame");
        assert!(!take_in_textures(true, 1.0 + LOADING_RESTART_S * 0.5, 1.0));
        assert!(take_in_textures(true, 1.0 + LOADING_RESTART_S, 1.0));
        assert!(take_in_textures(true, 0.0, f64::MIN), "the first frame always");
    }

    #[test]
    fn nothing_traced_yet_shows_the_placeholder() {
        assert!(!show_image(false, 0));
    }
}

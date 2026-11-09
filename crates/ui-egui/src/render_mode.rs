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
    read: Arc<Mutex<Read>>,
}

/// The GPU tracer in egui's callback resources.
struct TraceState {
    tracer: GpuTracer,
    scene: Option<Arc<TraceScene>>,
    key: String,
    /// Samples in the resolved image (None: resolve again).
    resolved: Option<u32>,
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
    shown: Option<(u32, u32, Vec<u8>)>,
    samples: u32,
    elapsed: f64,
    /// The traced geometry (displaced or not) and what it was built from: changes to the display
    /// alone don't rebuild it.
    geometry: Mutex<Option<Geometry>>,
}

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
    let revisions: Vec<u64> = s.caches.iter().map(|c| c.revision).collect();
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
    TraceScene::for_display(mesh, bvh, sets, d, env_image.map(|i| i.as_ref()))
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

/// What the traced scene depends on.
fn scene_key(app: &TextureCraftApp) -> String {
    let s = &app.session;
    let revisions: Vec<u64> = s.caches.iter().map(|c| c.revision).collect();
    format!("{}|{revisions:?}|{:?}", s.mesh_id, s.doc.project.display)
}

/// What the render depends on; a change restarts it.
fn key(app: &TextureCraftApp, size: (u32, u32)) -> String {
    format!("{}|{:?}|{size:?}|{:?}", scene_key(app), app.session.camera, app.ui.render)
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
    let k = key(app, size);
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
    let shown = app.render.texture.as_ref().filter(|_| app.render.samples > 0).map(TextureHandle::id);
    paint(ui, rect, shown);
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
    let scene_key = scene_key(app);
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
    let size = (rect.width().max(16.0) as u32, rect.height().max(16.0) as u32);
    let key = key(app, size);
    let mut g = match app.render.gpu.take() {
        Some(g) if g.size == size => g,
        old => {
            let out = GpuTracer::output_texture(&rs.device, size.0, size.1);
            let out_view = out.create_view(&Default::default());
            // A resized view keeps its scene and egui texture id.
            let (texture, scene_key, scene) = match old {
                Some(old) => {
                    rs.renderer.write().update_egui_texture_from_wgpu_texture(&rs.device, &out_view, wgpu::FilterMode::Linear, old.texture);
                    (old.texture, old.scene_key, old.scene)
                }
                None => (
                    rs.renderer.write().register_native_texture(&rs.device, &out_view, wgpu::FilterMode::Linear),
                    scene_key.clone(),
                    Arc::new(trace_scene(app)),
                ),
            };
            GpuView { size, scene_key, scene, key: String::new(), started: now, out, out_view, texture, status: Arc::default(), spp: 1, read: Arc::default() }
        }
    };
    let (samples, error) = {
        let mut st = g.status.lock().unwrap_or_else(PoisonError::into_inner);
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
        // Keep the UI near 60 fps: more samples while frames are quick, half as many when slow.
        let dt = ctx.input(|i| i.stable_dt);
        g.spp = if dt > 1.0 / 40.0 {
            (g.spp / 2).max(1)
        } else if dt < 1.0 / 60.0 {
            (g.spp + 1).min(16)
        } else {
            g.spp
        };
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
        size,
        out: g.out.clone(),
        out_view: g.out_view.clone(),
        status: g.status.clone(),
        read: g.read.clone(),
    };
    ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, cb));
    paint(ui, rect, (samples > 0).then_some(g.texture));
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
        if t.sync_materials(device, queue, encoder, renderer)? || st.key != self.key {
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
            None => GpuTracer::new(device).map(|tracer| TraceState { tracer, scene: None, key: String::new(), resolved: None }),
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
        if let Some(st) = r.callback_resources.get_mut::<TraceState>() {
            st.tracer.release();
            (st.scene, st.resolved) = (None, None);
            st.key.clear();
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

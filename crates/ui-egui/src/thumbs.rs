//! Shelf and brush preview caches
//! Small synchronous renders are limited per frame
//! Smart masks and hover previews use [`PreviewWorker`]

use std::collections::{HashMap, VecDeque};

use egui::{Color32, ColorImage, Painter, Pos2, Rect, TextureHandle, TextureOptions};
use texturecraft_engine::core::ProceduralKind;
use texturecraft_engine::paint::presets::BrushPreset;
use texturecraft_engine::paint::{BrushAlpha, alphas};
use texturecraft_engine::texel::linear_to_srgb;

use crate::preview_image::{aces, image_preview, shade};
use crate::preview_request::PreviewRequest;
use crate::preview_worker::PreviewWorker;
use crate::shelf_preview::ShelfPreview;
use crate::worker_preview::WorkerPreview;

/// Shelf thumbnails rendered per frame at most (the rest follow on the next frames)
const PER_FRAME: usize = 2;
/// Shelf sphere and smart mask thumbnail size in pixels
const SPHERE_PX: usize = 72;
/// Shelf procedural swatch size in pixels
const PROCEDURAL_PX: usize = 56;
/// Shelf alpha and brush stroke swatch size in pixels
const SWATCH_PX: usize = 64;
/// Image resource preview size in pixels
const IMAGE_PX: usize = 64;
/// Big hover previews kept at most (the least recently shown go first)
const BIG_PREVIEWS_KEPT: usize = 32;

/// The shelf's and the brush panel's preview textures
#[derive(Default)]
pub struct Thumbs {
    /// Shelf thumbnails by [`ShelfPreview::key`]
    shelf: HashMap<String, TextureHandle>,
    images: HashMap<String, TextureHandle>,
    /// (frame, thumbnails rendered in it)
    budget: (u64, usize),
    /// Renders the smart mask thumbnails and the big previews
    worker: PreviewWorker,
    /// Worker-rendered previews by [`worker_key`]
    rendered: HashMap<String, WorkerPreview>,
    /// The keys of the big previews in `rendered`, least recently shown first
    big_shown: VecDeque<String>,
    /// The frame that last took in the worker's results
    received_in_pass: u64,
}

impl Thumbs {
    /// A larger cached texture of an image resource for the stencil overlay (up to 1024 px)
    pub fn stencil(&mut self, ctx: &egui::Context, name: &str, img: &texturecraft_engine::compose::ImageRes) -> TextureHandle {
        let key = format!("stencil:{name}");
        if let Some(t) = self.images.get(&key) {
            return t.clone();
        }
        let scale = (1024.0 / img.width.max(img.height).max(1) as f32).min(1.0);
        let (w, h) = (((img.width as f32 * scale) as usize).max(1), ((img.height as f32 * scale) as usize).max(1));
        let mut ci = ColorImage::filled([w, h], Color32::TRANSPARENT);
        for y in 0..h {
            for x in 0..w {
                let s = img.sample((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
                let enc = |v: f32| if img.srgb { (v.clamp(0.0, 1.0) * 255.0) as u8 } else { (linear_to_srgb(v) * 255.0) as u8 };
                ci.pixels[y * w + x] = Color32::from_rgba_unmultiplied(enc(s[0]), enc(s[1]), enc(s[2]), (s[3].clamp(0.0, 1.0) * 255.0) as u8);
            }
        }
        let t = ctx.load_texture(key.clone(), ci, TextureOptions::LINEAR);
        self.images.insert(key, t.clone());
        t
    }

    /// Cached preview of an image resource (64 px, letterboxed)
    pub fn image(&mut self, ctx: &egui::Context, name: &str, img: &texturecraft_engine::compose::ImageRes) -> TextureHandle {
        if let Some(t) = self.images.get(name) {
            return t.clone();
        }
        let t = ctx.load_texture(format!("res_{name}"), image_preview(img, IMAGE_PX), TextureOptions::LINEAR);
        self.images.insert(name.to_string(), t.clone());
        t
    }

    /// Render within the frame budget; deferred thumbnails request a repaint
    fn shelf(&mut self, ctx: &egui::Context, preview: ShelfPreview, size_px: usize) -> Option<TextureHandle> {
        let key = preview.key();
        if let Some(t) = self.shelf.get(&key) {
            return Some(t.clone());
        }

        let frame = ctx.cumulative_pass_nr();
        if self.budget.0 != frame {
            self.budget = (frame, 0);
        }
        if self.budget.1 >= PER_FRAME {
            ctx.request_repaint();
            return None;
        }
        self.budget.1 += 1;

        // An unknown asset shows as an empty thumbnail
        let image = preview.image(size_px).unwrap_or_else(|| ColorImage::filled([size_px, size_px], Color32::TRANSPARENT));
        let t = ctx.load_texture(format!("thumb_{key}"), image, TextureOptions::LINEAR);
        self.shelf.insert(key, t.clone());
        Some(t)
    }

    /// Request once per asset and size; return None while pending or after failure
    /// Hover requests take priority in the native worker
    fn worker_rendered(&mut self, ctx: &egui::Context, preview: ShelfPreview, size_px: usize, hover: bool) -> Option<TextureHandle> {
        self.receive(ctx);

        let key = worker_key(&preview, size_px);
        if !self.rendered.contains_key(&key) {
            let queued = self.worker.request(ctx, PreviewRequest { key: key.clone(), preview, size_px, hover });
            self.rendered.insert(key.clone(), if queued { WorkerPreview::Rendering } else { WorkerPreview::Failed });
        }
        self.ready(&key)
    }

    /// The worker-rendered preview under `key` if it arrived
    fn ready(&self, key: &str) -> Option<TextureHandle> {
        match self.rendered.get(key) {
            Some(WorkerPreview::Ready(t)) => Some(t.clone()),
            Some(WorkerPreview::Rendering | WorkerPreview::Failed) | None => None,
        }
    }

    /// Poll once per pass and discard results evicted from the cache
    fn receive(&mut self, ctx: &egui::Context) {
        let pass = ctx.cumulative_pass_nr();
        if self.received_in_pass == pass {
            return;
        }
        self.received_in_pass = pass;

        for (key, image) in self.worker.poll() {
            let Some(entry) = self.rendered.get_mut(&key) else { continue };

            *entry = match image {
                Some(image) => WorkerPreview::Ready(ctx.load_texture(format!("thumb_{key}"), image, TextureOptions::LINEAR)),
                None => WorkerPreview::Failed,
            };
        }

        // A stopped worker never answers the rest
        if self.worker.is_stopped() {
            for entry in self.rendered.values_mut().filter(|e| matches!(e, WorkerPreview::Rendering)) {
                *entry = WorkerPreview::Failed;
            }
        }
    }

    /// Request a hover preview and evict the least recently shown beyond the cache limit
    pub fn big(&mut self, ctx: &egui::Context, preview: ShelfPreview, size_px: usize) -> Option<TextureHandle> {
        let key = worker_key(&preview, size_px);
        self.big_shown.retain(|k| *k != key);
        self.big_shown.push_back(key);
        while self.big_shown.len() > BIG_PREVIEWS_KEPT {
            if let Some(oldest) = self.big_shown.pop_front() {
                self.rendered.remove(&oldest);
            }
        }

        self.worker_rendered(ctx, preview, size_px, true)
    }

    /// The big preview of `preview` if the worker already rendered it (requests nothing)
    pub fn rendered_big(&mut self, ctx: &egui::Context, preview: &ShelfPreview, size_px: usize) -> Option<TextureHandle> {
        self.receive(ctx);
        self.ready(&worker_key(preview, size_px))
    }

    /// Sphere thumbnail of built-in material `id`
    pub fn material(&mut self, ctx: &egui::Context, id: &'static str) -> Option<TextureHandle> {
        self.shelf(ctx, ShelfPreview::Material(id), SPHERE_PX)
    }

    /// Smart mask thumbnail; None until the worker returns it
    pub fn smart_mask(&mut self, ctx: &egui::Context, id: &'static str) -> Option<TextureHandle> {
        self.worker_rendered(ctx, ShelfPreview::SmartMask(id), SPHERE_PX, false)
    }

    /// Sphere thumbnail of library filter `id` applied to a sample material
    pub fn filter(&mut self, ctx: &egui::Context, id: &'static str) -> Option<TextureHandle> {
        self.shelf(ctx, ShelfPreview::Filter(id), SPHERE_PX)
    }

    /// Grayscale swatch of procedural `kind`
    pub fn procedural(&mut self, ctx: &egui::Context, kind: ProceduralKind) -> Option<TextureHandle> {
        self.shelf(ctx, ShelfPreview::Procedural(kind), PROCEDURAL_PX)
    }

    /// White-on-black swatch of library alpha `alpha`
    pub fn alpha(&mut self, ctx: &egui::Context, alpha: &'static alphas::LibraryAlpha) -> Option<TextureHandle> {
        self.shelf(ctx, ShelfPreview::Alpha(alpha), SWATCH_PX)
    }

    /// A brush preset's stroke along an S curve
    pub fn brush_preset(&mut self, ctx: &egui::Context, preset: &'static BrushPreset) -> Option<TextureHandle> {
        self.shelf(ctx, ShelfPreview::Brush(preset), SWATCH_PX)
    }
}

/// The key of a worker-rendered preview: one per asset and size
fn worker_key(preview: &ShelfPreview, size_px: usize) -> String {
    format!("{}@{size_px}", preview.key())
}

/// A small shaded sphere drawn with a vertex-colored mesh (layer thumbnails, previews).
pub fn paint_sphere(p: &Painter, c: Pos2, r: f32, base: [f32; 3]) {
    let mut mesh = egui::Mesh::default();
    let rings = 10;
    let segs = 28;
    let color = |dx: f32, dy: f32| {
        let d2 = (dx * dx + dy * dy).min(1.0);
        let n = [dx, dy, (1.0 - d2).sqrt()];
        let s = shade(n, base, 0.0, 0.5).map(|v| (linear_to_srgb(aces(v * 1.4)) * 255.0) as u8);
        Color32::from_rgb(s[0], s[1], s[2])
    };
    mesh.colored_vertex(c, color(0.0, 0.0));
    for i in 1..=rings {
        let rr = i as f32 / rings as f32;
        for j in 0..segs {
            let a = j as f32 / segs as f32 * std::f32::consts::TAU;
            let (dx, dy) = (a.cos() * rr, a.sin() * rr);
            mesh.colored_vertex(Pos2::new(c.x + dx * r, c.y - dy * r), color(dx, dy));
        }
    }
    for j in 0..segs {
        let (a, b) = (1 + j, 1 + (j + 1) % segs);
        mesh.add_triangle(0, a as u32, b as u32);
    }
    for i in 1..rings {
        let r0 = 1 + (i - 1) * segs;
        let r1 = 1 + i * segs;
        for j in 0..segs {
            let (a, b) = (r0 + j, r0 + (j + 1) % segs);
            let (c2, d) = (r1 + j, r1 + (j + 1) % segs);
            mesh.add_triangle(a as u32, c2 as u32, d as u32);
            mesh.add_triangle(a as u32, d as u32, b as u32);
        }
    }
    p.add(egui::Shape::mesh(mesh));
}

/// Brush alpha preview drawn as small cells (cheap, no texture).
pub fn paint_alpha(p: &Painter, rect: Rect, a: BrushAlpha, hardness: f32) {
    let n = 20;
    let cell = rect.width() / n as f32;
    for y in 0..n {
        for x in 0..n {
            let lx = (x as f32 + 0.5) / n as f32 * 2.0 - 1.0;
            let ly = (y as f32 + 0.5) / n as f32 * 2.0 - 1.0;
            let v = a.sample(lx, ly, hardness, 7);
            if v > 0.02 {
                let r = Rect::from_min_size(rect.min + egui::vec2(x as f32 * cell, y as f32 * cell), egui::vec2(cell + 0.5, cell + 0.5));
                p.rect_filled(r, 0.0, Color32::from_white_alpha((v * 255.0) as u8));
            }
        }
    }
}

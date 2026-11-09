//! Interactive painting on the session: strokes (begin / continue / end / cancel), polygon fill
//! and the color picker. The UI, CLI and agents all go through these.

use std::collections::{BTreeMap, BTreeSet};

use texturecraft_compose::PaintTarget;
use texturecraft_core::{Channel, Effect, EffectKind, LayerId, LayerKind, Mask, MaskBackground};
use texturecraft_geom::glam::Vec2;
use texturecraft_paint::{
    CommitSource, EyeDepth, PickCtx, Pickup, PickupMode, RasterCtx, Shape, Staging, Stroke, StrokeCanvas, SurfaceHit, commit_layer, commit_mask, pick,
    rasterize,
};

use crate::session::{ActiveStroke, CloneFrom, Session, StackTarget, Tool};
use crate::uv_stroke_state::UvStrokeState;
use crate::{EngineError, Result};

/// Shortest distance between 2D view stamps, so a stamp with no radius (a pen at zero pressure) can't stall a stroke.
const MIN_UV_STAMP_STEP: f32 = 1e-5;

/// 2D view stamps rasterized together, bounding the memory of one batch.
const UV_STAMP_BATCH: usize = 4096;

/// Covers a UV diagonal at the minimum step (about 142k stamps), including light pen pressure, while bounding hostile coordinates
const MAX_UV_STAMPS_PER_SAMPLE: usize = 150_000;

impl Session {
    /// Camera visibility for a stroke at `viewport` (cached until the camera, size or mesh change).
    fn eye_depth(&mut self, viewport: Vec2) -> std::sync::Arc<EyeDepth> {
        let size = ((viewport.x / 2.0).max(64.0) as u32, (viewport.y / 2.0).max(64.0) as u32);
        if let Some((cam, sz, mesh, depth)) = &self.depth_cache
            && *cam == self.camera
            && *sz == size
            && *mesh == self.mesh_id
        {
            return depth.clone();
        }
        let depth = std::sync::Arc::new(EyeDepth::render(&self.mesh, &self.camera, size.0, size.1));
        self.depth_cache = Some((self.camera, size, self.mesh_id, depth.clone()));
        depth
    }

    pub fn pick(&self, screen: Vec2, viewport: Vec2) -> Option<SurfaceHit> {
        let ctx = PickCtx { mesh: &self.mesh, bvh: &self.bvh, camera: &self.camera, viewport };
        pick(&ctx, screen)
    }

    /// The texture set that paints mesh material `material` (a [`SurfaceHit::material`])
    pub fn texture_set_of_material(&self, material: usize) -> Option<usize> {
        self.doc.project.texture_sets.iter().position(|t| t.material == material)
    }

    /// The texture set of the surface under viewport pixel `screen`
    pub fn texture_set_at(&self, screen: Vec2, viewport: Vec2) -> Option<usize> {
        self.pick(screen, viewport).and_then(|hit| self.texture_set_of_material(hit.material))
    }

    /// Resolve what a stroke on the selected layer paints into, creating the mask paint effect
    /// when painting a mask. Must run inside the stroke's history snapshot.
    fn paint_target(&mut self) -> Result<PaintTarget> {
        let id = self.selected_layer().ok_or_else(|| EngineError::Other("select a layer to paint on".into()))?;
        // Painting an instance paints its source (its own mask when it overrides the mask).
        let p = &self.doc.project;
        let id = if self.target == StackTarget::Mask { p.mask_owner(id) } else { p.content_layer(id).map_or(id, |l| l.id) };
        match self.target {
            StackTarget::Content => {
                let l = self.doc.project.layer(id)?;
                if matches!(l.kind, LayerKind::Paint) {
                    Ok(PaintTarget::Layer(id))
                } else {
                    Err(EngineError::Other(format!("\"{}\" is a {} layer: paint on a paint layer or on its mask", l.name, l.kind.name())))
                }
            }
            StackTarget::Effect(e) => match self.doc.project.effect(e) {
                // Content paint effects hold channel pixels like a paint layer.
                Some((owner, eff)) if matches!(eff.kind, EffectKind::Paint) => {
                    let content = self.doc.project.layer(owner)?.content_effects.iter().any(|x| x.id == e);
                    Ok(if content { PaintTarget::Layer(e) } else { PaintTarget::Mask(e) })
                }
                // Path sublayers are painted only from their stored paths.
                Some((owner, eff)) if matches!(eff.kind, EffectKind::Path(_)) && self.painting_paths => {
                    let content = self.doc.project.layer(owner)?.content_effects.iter().any(|x| x.id == e);
                    Ok(if content { PaintTarget::Layer(e) } else { PaintTarget::Mask(e) })
                }
                Some((_, eff)) if matches!(eff.kind, EffectKind::Path(_)) => {
                    Err(EngineError::Other("a path sublayer holds paths only: edit them with the Path tool, or paint on a paint sublayer".into()))
                }
                // A filter (or another effect) is selected, e.g. just added from the shelf: paint
                // what lies beneath it, so the new paint goes through it.
                Some((owner, _)) => self.paint_target_below(owner, e),
                None => Err(EngineError::Other("the selected effect is not a paint effect".into())),
            },
            StackTarget::Mask => {
                let eid = self.doc.project.alloc_id();
                let layer = self.doc.project.layer_mut(id)?;
                let mask = layer.mask.get_or_insert_with(|| Mask::new(MaskBackground::Black));
                if let Some(top) = mask.effects.last().filter(|e| matches!(e.kind, EffectKind::Paint)) {
                    return Ok(PaintTarget::Mask(top.id));
                }
                mask.effects.push(Effect::new(eid, EffectKind::Paint));
                Ok(PaintTarget::Mask(eid))
            }
        }
    }

    /// What a stroke paints while non-paint effect `effect` of layer `owner` is selected: the
    /// nearest paint effect below it in the same stack, else the paint layer's own pixels (for
    /// content) or a new paint effect inserted just below it (for a mask).
    fn paint_target_below(&mut self, owner: LayerId, effect: LayerId) -> Result<PaintTarget> {
        let eid = self.doc.project.alloc_id();
        let layer = self.doc.project.layer_mut(owner)?;
        let below = |effects: &[Effect], at: usize| effects.get(..at).and_then(|b| b.iter().rev().find(|x| matches!(x.kind, EffectKind::Paint))).map(|x| x.id);
        if let Some(at) = layer.content_effects.iter().position(|x| x.id == effect) {
            if let Some(id) = below(&layer.content_effects, at) {
                return Ok(PaintTarget::Layer(id));
            }
            return if matches!(layer.kind, LayerKind::Paint) {
                Ok(PaintTarget::Layer(owner))
            } else {
                Err(EngineError::Other(format!(
                    "\"{}\" is a {} layer: paint on a paint layer or on its mask (a filter changes what lies below it)",
                    layer.name,
                    layer.kind.name()
                )))
            };
        }
        let mask = layer.mask.as_mut().ok_or_else(|| EngineError::Other("the selected effect is not a paint effect".into()))?;
        let at = mask.effects.iter().position(|x| x.id == effect).ok_or_else(|| EngineError::Other("the selected effect is not a paint effect".into()))?;
        if let Some(id) = below(&mask.effects, at) {
            return Ok(PaintTarget::Mask(id));
        }
        mask.effects.insert(at, Effect::new(eid, EffectKind::Paint));
        Ok(PaintTarget::Mask(eid))
    }

    fn stroke_values(&self, target: PaintTarget) -> BTreeMap<Channel, [f32; 3]> {
        let ts = self.ts();
        let Ok(set) = self.doc.project.texture_set(ts) else { return BTreeMap::new() };
        let layer_channels = match target {
            // A content paint effect paints its owner layer's channels.
            PaintTarget::Layer(id) => {
                let owner = self.doc.project.effect(id).map(|(owner, _)| owner).unwrap_or(id);
                self.doc.project.layer(owner).map(|l| l.channels.keys().copied().collect::<BTreeSet<_>>()).unwrap_or_default()
            }
            PaintTarget::Mask(_) => BTreeSet::new(),
        };
        self.tool
            .values
            .iter()
            .filter(|(c, _)| self.tool.enabled.contains(c) && set.has_channel(**c) && layer_channels.contains(c))
            .map(|(c, v)| (*c, *v))
            .collect()
    }

    /// Common setup of 3D and 2D strokes. `first_uv` is where the stroke starts on the mesh
    /// (the Clone tool's destination anchor).
    fn start_stroke(&mut self, viewport: Vec2, uv_mode: bool, first_uv: Option<Vec2>) -> Result<()> {
        let tool = self.tool.tool;
        let erase = tool == Tool::Eraser;
        let quick_mask = self.quick_mask.editing;
        let ts = self.ts();
        let size = self.caches.get(ts).map(|c| c.size).unwrap_or((1024, 1024));
        let pickup = if quick_mask { None } else { self.pickup_mode(first_uv)? };
        if pickup.is_some() {
            // Clone and smudge read the composite: bring it up to date first.
            self.update();
        }
        // Cloning from the selected layer reads that layer alone.
        let layer_source = match self.selected_layer() {
            Some(id) if pickup.is_some() && tool == Tool::Clone && self.tool.clone_from == CloneFrom::Layer => {
                Some(self.flatten(ts, id).ok_or_else(|| EngineError::Other("clone from a paint or fill layer (folders have no content of their own)".into()))?)
            }
            _ => None,
        };
        let before = self.doc.clone();
        let target = if quick_mask {
            None
        } else {
            match self.paint_target() {
                Ok(t) => Some(t),
                Err(e) => {
                    self.doc = before;
                    return Err(e);
                }
            }
        };
        let values = target.map(|t| self.stroke_values(t)).unwrap_or_default();
        let stencil = if tool == Tool::Projection && !quick_mask { self.capture_stencil(viewport) } else { None };
        let problem = if !erase && values.is_empty() && matches!(target, Some(PaintTarget::Layer(_))) {
            Some("no channel enabled to paint: enable channels in the tool's material")
        } else if pickup.is_some() && matches!(target, Some(PaintTarget::Mask(_))) {
            Some("the Clone and Smudge tools paint layer content: select the layer instead of its mask")
        } else if tool == Tool::Projection && !quick_mask && stencil.is_none() && !uv_mode {
            Some("the Projection tool needs a stencil: pick an image in the tool properties (tool.stencil)")
        } else {
            None
        };
        if let Some(msg) = problem {
            self.doc = before;
            return Err(EngineError::Other(msg.into()));
        }
        let depth = (!uv_mode).then(|| self.eye_depth(viewport));
        let mut brush = self.tool.brush.clone();
        if tool == Tool::Smudge {
            // shortcut: smudge drags one stamp trail; mirrored trails would need one pickup each.
            brush.symmetry = Default::default();
        }
        let stroke = Stroke::new(&brush, &self.mesh);
        let mut canvas = StrokeCanvas::new(size.0, size.1);
        if !quick_mask {
            canvas.limit = self.stroke_limit(ts, size);
        }
        if let Some((mode, offset)) = pickup
            && let Some(cache) = self.caches.get(ts)
        {
            let mode = match mode {
                PickupMode::Clone { .. } => {
                    PickupMode::Clone { offset: [(offset[0] * size.0 as f32).round() as i64, (offset[1] * size.1 as f32).round() as i64] }
                }
                m => m,
            };
            canvas.pickup = Some(match &layer_source {
                Some(flat) => Pickup::from_layer(mode, flat, size, cache.gbuf.clone(), values.keys().copied()),
                None => Pickup::new(mode, cache.composite.clone(), cache.gbuf.clone(), values.keys().copied()),
            });
            if tool == Tool::Clone
                && let Some(src) = self.tool.clone_source.as_mut()
            {
                src.offset.get_or_insert(offset);
            }
        }
        let label = if quick_mask {
            "Quick mask stroke"
        } else if erase {
            "Eraser stroke"
        } else if matches!(target, Some(PaintTarget::Mask(_))) {
            "Mask stroke"
        } else {
            "Paint stroke"
        };
        self.stroke = Some(ActiveStroke {
            ts,
            target,
            stroke,
            canvas,
            depth,
            uv: uv_mode.then(UvStrokeState::default),
            camera: self.camera,
            viewport,
            values,
            erase,
            stencil,
            before,
            label: label.to_string(),
            staging: Staging::default(),
        });
        Ok(())
    }

    /// Clone / smudge pickup for the current tool, with the clone offset (source minus
    /// destination) in texture coordinates.
    fn pickup_mode(&self, first_uv: Option<Vec2>) -> Result<Option<(PickupMode, [f32; 2])>> {
        match self.tool.tool {
            Tool::Smudge => Ok(Some((PickupMode::Smudge { strength: self.tool.smudge_strength }, [0.0; 2]))),
            Tool::Clone => {
                let src =
                    self.tool.clone_source.ok_or_else(|| EngineError::Other("set a clone source first: V + click on the mesh (tool.cloneSource)".into()))?;
                let offset = match (src.offset, first_uv) {
                    (Some(o), _) => o,
                    (None, Some(p)) => [src.uv[0] - p.x, src.uv[1] - p.y],
                    (None, None) => return Err(EngineError::Other("start the clone stroke on the mesh".into())),
                };
                Ok(Some((PickupMode::Clone { offset: [0; 2] }, offset)))
            }
            _ => Ok(None),
        }
    }

    /// The brush shape: the alpha image when one is set and loaded, else the library alpha when
    /// one is set, else the procedural alpha.
    fn shape<'a>(images: &'a texturecraft_compose::ImageStore, b: &texturecraft_paint::BrushSettings) -> Shape<'a> {
        Shape {
            alpha: b.alpha,
            image: b.alpha_image.as_ref().and_then(|n| images.get(n)).map(|i| &**i),
            library: b.alpha_library.as_deref().and_then(texturecraft_paint::alphas::find).map(|a| a.image()),
            level: 0,
        }
    }

    /// The stencil placed for the current camera and viewport.
    fn capture_stencil(&self, viewport: Vec2) -> Option<texturecraft_compose::Stencil> {
        let st = self.tool.stencil.as_ref()?;
        let image = self.images.get(&st.image)?.clone();
        let aspect = viewport.x.max(1.0) / viewport.y.max(1.0);
        let (center, half, rotation) = st.placement(image.width as f32 / image.height.max(1) as f32, aspect);
        Some(texturecraft_compose::Stencil { image, view_proj: self.camera.view_proj(aspect), center, half, rotation, tile: st.tile })
    }

    /// Start a stroke at `screen` (viewport pixels). Returns false when the cursor misses the
    /// mesh (the stroke starts anyway and resumes when the cursor enters the mesh).
    pub fn begin_stroke(&mut self, screen: Vec2, viewport: Vec2, pressure: f32) -> Result<bool> {
        self.cancel_stroke();
        match self.tool.tool {
            Tool::PolygonFill => return self.polygon_fill(screen, viewport).map(|_| true),
            Tool::ColorPicker => return self.color_pick(screen, viewport).map(|_| true),
            Tool::GeometryMask => return crate::instance_cmds::toggle_part_at(self, screen, viewport).map(|_| true),
            _ => {}
        }
        let first_uv = if self.tool.tool == Tool::Clone { self.pick(screen, viewport).map(|h| h.uv) } else { None };
        self.start_stroke(viewport, false, first_uv)?;
        Ok(self.continue_stroke(screen, pressure))
    }

    /// Start a stroke in the 2D view at texture coordinate `uv` (polygon fill fills there).
    pub fn begin_stroke_uv(&mut self, uv: Vec2, pressure: f32) -> Result<()> {
        self.cancel_stroke();
        match self.tool.tool {
            Tool::PolygonFill => return self.polygon_fill_uv(uv).map(|_| ()),
            Tool::ColorPicker | Tool::GeometryMask => return Err(EngineError::Other("use this tool in the 3D view".into())),
            _ => {}
        }

        self.start_stroke(self.viewport, true, Some(uv))?;
        self.continue_stroke_uv(uv, pressure);
        Ok(())
    }

    /// Add a 2D view sample (texture coordinates). Brush size is relative to the UV square.
    pub fn continue_stroke_uv(&mut self, uv: Vec2, pressure: f32) {
        let Some(active) = self.stroke.as_mut() else { return };
        let Some(progress) = active.uv.as_mut() else { return };
        let Some(cache) = self.caches.get_mut(active.ts) else { return };

        let settings = &active.stroke.settings;
        let pressure = pressure.clamp(0.0, 1.0);
        let size_scale = if pressure < 1.0 { settings.min_size + (1.0 - settings.min_size) * pressure } else { 1.0 };
        let radius = settings.size * 2.0 * size_scale;
        let step = (settings.spacing * 2.0 * radius).max(MIN_UV_STAMP_STEP);

        // Follow path: turn the stamp with the stroke direction (y up on screen, v grows down)
        let angle = match progress.last {
            Some(previous) if settings.follow_path && uv != previous => settings.angle + (previous.y - uv.y).atan2(uv.x - previous.x).to_degrees() - 90.0,
            _ => settings.angle,
        };

        // The first sample is one stamp; a later one stamps every `step` along the segment from the previous
        let (start, segment, mut distance) = match progress.last.replace(uv) {
            None => (uv, Vec2::ZERO, 0.0),
            // Pressure can shrink the step below the carry from the preceding sample
            Some(previous) => (previous, uv - previous, (step - progress.carry).max(0.0)),
        };
        let length = segment.length();
        let shape = Self::shape(&self.images, settings);
        let mut batch = Vec::new();
        let mut stamped = 0;
        while distance <= length && stamped < MAX_UV_STAMPS_PER_SAMPLE {
            let batch_size = UV_STAMP_BATCH.min(MAX_UV_STAMPS_PER_SAMPLE - stamped);
            batch.clear();
            while distance <= length && batch.len() < batch_size {
                let center = start + segment * (distance / length.max(f32::MIN_POSITIVE));
                let seed = progress.stamps.wrapping_add((stamped + batch.len()) as u32);
                batch.push(texturecraft_paint::UvStamp { center, radius, angle, flow: settings.flow, hardness: settings.hardness, seed });
                distance += step;
            }

            stamped += batch.len();
            texturecraft_paint::rasterize_uv(&mut active.canvas, &cache.gbuf, shape, &batch);
        }

        // At most one step, so a segment cut off at the stamp limit can't push the next one's stamps backwards
        progress.carry = (length - (distance - step)).min(step);
        progress.stamps = progress.stamps.wrapping_add(stamped as u32);
        self.stroke_painted();
    }

    /// Add a pointer sample to the active stroke. Returns whether it hit the mesh.
    pub fn continue_stroke(&mut self, screen: Vec2, pressure: f32) -> bool {
        let Some(st) = self.stroke.as_mut() else { return false };
        let Some(cache) = self.caches.get_mut(st.ts) else { return false };
        let material = self.doc.project.texture_sets.get(st.ts).map(|t| t.material).unwrap_or(0);
        let ctx = PickCtx { mesh: &self.mesh, bvh: &self.bvh, camera: &st.camera, viewport: st.viewport };
        let stamps = st.stroke.add(&ctx, screen, pressure);
        let hit = st.stroke.last_hit().is_some();
        if stamps.is_empty() {
            return hit;
        }
        let rctx = RasterCtx {
            mesh: &self.mesh,
            bvh: &self.bvh,
            gbuf: &cache.gbuf,
            material,
            depth: st.depth.as_deref(),
            eye: st.camera.eye(),
            backface_culling: st.stroke.settings.backface_culling,
            angle_fade: st.stroke.settings.angle_fade,
            shape: Self::shape(&self.images, &st.stroke.settings),
        };
        rasterize(&mut st.canvas, &rctx, &stamps);
        self.stroke_painted();
        hit
    }

    /// After stamps were rasterized: prepare the tiles the commit will write (so it only swaps
    /// them in) and mark the touched tiles for the live preview.
    fn stroke_painted(&mut self) {
        let Some(st) = self.stroke.as_mut() else { return };
        let recent = std::mem::take(&mut st.canvas.recent);
        let (ts, target) = (st.ts, st.target);
        let Some(t) = target else {
            self.quick_mask.revision = self.quick_mask.revision.wrapping_add(1);
            return;
        };
        let gbuf = self.caches.get(ts).map(|c| c.gbuf.clone());
        let (pixels, mask_value) = (&self.doc.pixels, self.tool.mask_value);
        let mut staging = std::mem::take(&mut st.staging);
        with_commit_source(st, gbuf.as_deref(), &self.mesh, &recent, |src| match t {
            PaintTarget::Layer(id) => staging.stage_layer(pixels, id, &st.values, &src),
            PaintTarget::Mask(eid) => staging.stage_mask(pixels, eid, mask_value, &src),
        });
        st.staging = staging;
        self.mark_painted(ts, target, recent);
    }

    pub fn stroke_active(&self) -> bool {
        self.stroke.is_some()
    }

    /// Commit the active stroke (one undo step).
    pub fn end_stroke(&mut self) -> Result<()> {
        let Some(mut st) = self.stroke.take() else { return Ok(()) };
        let size = (st.canvas.alpha.width(), st.canvas.alpha.height());
        if st.canvas.dirty.is_empty() {
            // Nothing painted: drop any mask effect the stroke created.
            self.doc = st.before;
            return Ok(());
        }
        let gbuf = self.caches.get(st.ts).map(|c| c.gbuf.clone());
        let mesh = self.mesh.clone();
        let mut staging = std::mem::take(&mut st.staging);
        let mask_value = self.tool.mask_value;
        match st.target {
            Some(PaintTarget::Layer(id)) => {
                let pixels = &mut self.doc.pixels;
                with_commit_source(&st, gbuf.as_deref(), &mesh, &st.canvas.dirty, |src| commit_layer(pixels, id, size, &st.values, src, &mut staging));
            }
            Some(PaintTarget::Mask(eid)) => {
                let pixels = &mut self.doc.pixels;
                with_commit_source(&st, gbuf.as_deref(), &mesh, &st.canvas.dirty, |src| commit_mask(pixels, eid, size, mask_value, src, &mut staging));
            }
            None => {
                with_commit_source(&st, gbuf.as_deref(), &mesh, &st.canvas.dirty, |src| self.commit_quick_mask(st.ts, size, src));
                return Ok(());
            }
        }
        self.undo.push((st.label.clone(), st.before));
        if self.undo.len() > crate::session::MAX_HISTORY {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.modified = true;
        self.revision = self.revision.wrapping_add(1);
        // The live preview already shows the committed result: the stroke's tiles recomposite
        // from the committed pixels at low priority, without holding up this frame.
        self.mark_settle(st.ts, st.target, st.canvas.dirty.iter().copied());
        Ok(())
    }

    /// Abandon the active stroke (Esc).
    pub fn cancel_stroke(&mut self) {
        if let Some(st) = self.stroke.take() {
            self.mark_painted(st.ts, st.target, st.canvas.dirty.iter().copied());
            if st.target.is_none() {
                self.quick_mask.revision = self.quick_mask.revision.wrapping_add(1);
            }
            self.doc = st.before;
        }
    }

    /// Paint a whole stroke through viewport points (scripted strokes from CLI/MCP).
    pub fn stroke_points(&mut self, points: &[Vec2], viewport: Vec2) -> Result<usize> {
        let Some(first) = points.first() else { return Err(EngineError::Other("a stroke needs at least one point".into())) };
        self.begin_stroke(*first, viewport, 1.0)?;
        let mut hits = 0;
        for p in points.iter().skip(1) {
            if self.continue_stroke(*p, 1.0) {
                hits += 1;
            }
        }
        let stamps = self.stroke.as_ref().map(|s| s.stroke.stamps.len()).unwrap_or(0);
        self.end_stroke()?;
        let _ = hits;
        Ok(stamps)
    }

    /// Paint a whole 2D view stroke through texture coordinates. Returns the touched tiles.
    pub fn stroke_uv_points(&mut self, points: &[Vec2]) -> Result<usize> {
        let Some(first) = points.first() else { return Err(EngineError::Other("a stroke needs at least one point".into())) };
        self.begin_stroke_uv(*first, 1.0)?;
        for q in points.iter().skip(1) {
            self.continue_stroke_uv(*q, 1.0);
        }
        let tiles = self.stroke_tiles();
        self.end_stroke()?;
        Ok(tiles)
    }

    /// The G-buffer of texture set `ts` and an empty canvas for a one-shot fill, limited by the
    /// quick mask unless the fill edits it.
    pub(crate) fn fill_canvas(&self, ts: usize) -> Result<(std::sync::Arc<texturecraft_geom::UvGBuffer>, StrokeCanvas)> {
        let gbuf = self.caches.get(ts).map(|c| c.gbuf.clone()).ok_or_else(|| EngineError::Other("texture set not ready".into()))?;
        let mut canvas = StrokeCanvas::new(gbuf.width, gbuf.height);
        if !self.quick_mask.editing {
            canvas.limit = self.stroke_limit(ts, (gbuf.width, gbuf.height));
        }
        Ok((gbuf, canvas))
    }

    /// Commit a one-shot fill with the tool's material (one undo step), or into the quick mask.
    pub(crate) fn commit_fill(&mut self, label: &str, ts: usize, canvas: &StrokeCanvas) -> Result<()> {
        if self.quick_mask.editing {
            return self.paint_fill(ts, canvas);
        }

        // Each fill is its own step, never merged into a box drag's step of the same label
        self.edit_merging(label, false, |session| session.paint_fill(ts, canvas))
    }

    /// Paint a one-shot fill with the tool's material into the paint target, or into the quick mask while it is edited (no undo step)
    pub(crate) fn paint_fill(&mut self, ts: usize, canvas: &StrokeCanvas) -> Result<()> {
        let size = (canvas.alpha.width(), canvas.alpha.height());
        let opacity = self.tool.brush.opacity;
        let source = CommitSource { alpha: &canvas.alpha, tiles: &canvas.dirty, opacity, erase: false, per_texel: None, carried: None };
        if self.quick_mask.editing {
            self.commit_quick_mask(ts, size, source);
            return Ok(());
        }

        let target = self.paint_target()?;
        let values = self.stroke_values(target);
        match target {
            PaintTarget::Layer(id) => commit_layer(&mut self.doc.pixels, id, size, &values, source, &mut Staging::default()),
            PaintTarget::Mask(effect) => commit_mask(&mut self.doc.pixels, effect, size, self.tool.mask_value, source, &mut Staging::default()),
        }
        Ok(())
    }

    /// Color picker: copy the material under the cursor into the tool's paint values.
    pub fn color_pick(&mut self, screen: Vec2, viewport: Vec2) -> Result<BTreeMap<Channel, [f32; 3]>> {
        let hit = self.pick(screen, viewport).ok_or_else(|| EngineError::Other("nothing under the cursor".into()))?;
        let ts = self.texture_set_of_material(hit.material).unwrap_or(self.ts());
        let cache = self.caches.get(ts).ok_or_else(|| EngineError::Other("texture set not ready".into()))?;
        let (w, h) = (cache.size.0 as f32, cache.size.1 as f32);
        let (x, y) = ((hit.uv.x * w).clamp(0.0, w - 1.0) as u32, (hit.uv.y * h).clamp(0.0, h - 1.0) as u32);
        let mut out = BTreeMap::new();
        for c in cache.composite.channels.keys() {
            let v = cache.composite.value(*c, x, y);
            self.tool.values.insert(*c, v);
            out.insert(*c, v);
        }
        Ok(out)
    }

    /// The stroke canvas alpha of the active stroke (for overlays and tests).
    pub fn stroke_tiles(&self) -> usize {
        self.stroke.as_ref().map(|s| s.canvas.dirty.len()).unwrap_or(0)
    }

    /// Where the Clone tool reads for destination `at` (texture coordinates) as a surface
    /// point: the source itself until the first stroke fixes the offset, or when `at` is None.
    pub fn clone_source_point(&self, at: Option<Vec2>) -> Option<texturecraft_geom::glam::Vec3> {
        let src = self.tool.clone_source?;
        let uv = match (src.offset, at) {
            (Some(o), Some(p)) => Vec2::new(p.x + o[0], p.y + o[1]),
            _ => Vec2::from(src.uv),
        };
        if !((0.0..1.0).contains(&uv.x) && (0.0..1.0).contains(&uv.y)) {
            return None;
        }
        let g = &self.caches.get(self.ts())?.gbuf;
        let (x, y) = ((uv.x * g.width as f32) as usize, (uv.y * g.height as f32) as usize);
        Some(g.surface(&self.mesh, y * g.width as usize + x)?.pos)
    }

    pub fn selected_layer_id(&self) -> Option<LayerId> {
        self.selected_layer()
    }
}

/// Run `f` with stroke `st` as a commit source over `tiles` (the Projection tool's stencil
/// colors and alpha come from the surface under each texel).
fn with_commit_source<R>(
    st: &ActiveStroke,
    gbuf: Option<&texturecraft_geom::UvGBuffer>,
    mesh: &texturecraft_geom::Mesh,
    tiles: &BTreeSet<(u32, u32)>,
    f: impl FnOnce(CommitSource) -> R,
) -> R {
    let stencil_fn = |x: u32, y: u32| -> Option<([f32; 3], f32)> {
        let g = gbuf?;
        let p = g.surface(mesh, (y * g.width + x) as usize)?.pos;
        st.stencil.as_ref()?.sample_world(p)
    };
    let per_texel: Option<texturecraft_paint::TexelFn> = if st.stencil.is_some() { Some(&stencil_fn) } else { None };
    let carried = st.canvas.pickup.as_ref().map(|p| &p.values);
    f(CommitSource { alpha: &st.canvas.alpha, tiles, opacity: st.stroke.settings.opacity, erase: st.erase, per_texel, carried })
}

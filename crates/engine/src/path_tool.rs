//! The Path tool: control points placed on the surface (3D view) or in texture space (2D view)
//! and a smooth curve through them, painted with the brush, or, when closed and filled, as an
//! antialiased lasso fill of the visible surface (3D) or of the UV region (2D).
//!
//! Paths are non-destructive: a committed path is stored in a path sublayer
//! ([`EffectKind::Path`]) that is repainted from its paths whenever one is added, edited
//! (`path.edit`, live while the points, brush or material change) or removed (`path.remove`).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use texturecraft_core::{Channel, Effect, EffectKind, LayerId, Mask, MaskBackground, PathShape};
use texturecraft_geom::glam::{Vec2, Vec3};
use texturecraft_geom::{OrbitCamera, surface_at};
use texturecraft_paint::{BrushSettings, Lasso, fill_coverage, spline};

use crate::commands::{CommandSpec, always, bad, boolp, has_layer, u64p};
use crate::session::{StackTarget, Tool};
use crate::tool_cmds::{points_param, set_tool, viewport_of};
use crate::{EngineError, Result, Session, cmd};

/// Control points kept at most.
pub const MAX_PATH_POINTS: usize = 4096;
/// Curve sampling step in viewport pixels (3D view) and texture units (2D view).
const STEP_PX: f32 = 2.0;
const STEP_UV: f32 = 0.001;
const MIN_PROJECTIVE_W: f32 = 1e-6;
// Allow ray hits to differ slightly from transformed texel positions
const RAY_HIT_RELATIVE_TOLERANCE: f32 = 0.001;

/// The curve being edited with the Path tool.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PathState {
    /// Control points: surface positions (3D view) or texture coordinates with z = 0 (2D view).
    pub points: Vec<Vec3>,
    /// The points are texture coordinates (placed in the 2D view).
    pub uv: bool,
    /// The curve loops back to its first point.
    pub closed: bool,
    /// A closed path fills its interior instead of stroking the curve.
    pub fill: bool,
    /// The stored path being edited: (path sublayer, index). Changes repaint it live.
    #[serde(default)]
    pub editing: Option<(LayerId, usize)>,
    /// This edit has already repainted: later changes join its undo step, not an earlier edit's.
    #[serde(default)]
    pub edited: bool,
}

impl PathState {
    /// Append a control point; a point placed in the other view starts a new path.
    pub fn push(&mut self, p: Vec3, uv: bool) {
        if uv != self.uv {
            self.points.clear();
            self.uv = uv;
        }
        if self.points.len() < MAX_PATH_POINTS {
            self.points.push(p);
        }
    }
}

/// A path sublayer's pixels after painting its first paths (`below`), with the mesh and
/// texture size they were painted for.
#[derive(Clone)]
// shortcut: the key doesn't cover image resources a path's brush alpha reads; replacing one
// leaves cached paths with the old alpha until a path below the edited one changes.
pub(crate) struct PathCache {
    eid: LayerId,
    key: (u64, Option<(u32, u32)>),
    below: Vec<PathShape>,
    mask: Option<texturecraft_texel::TileImage>,
    layers: Vec<((LayerId, Channel), texturecraft_texel::TileImage)>,
}

/// What a stored path is painted with ([`PathShape::paint`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct PathPaint {
    brush: BrushSettings,
    values: BTreeMap<Channel, [f32; 3]>,
    enabled: BTreeSet<Channel>,
    mask_value: f32,
    camera: OrbitCamera,
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "tool.path",
            "Path",
            ["Tools"],
            Some("7"),
            "{points?: [[x,y],...] viewport pixels on the mesh | uvPoints?: [[u,v],...], viewport?: [w,h], closed?, fill?, commit?: bool, clear?: bool} — selects the Path tool and edits its curve; commit adds the path to the selected path sublayer (or a new one), painted with the current brush or filled when closed and filled. While a stored path is edited (path.edit), changes repaint it at once",
            always,
            path
        ),
        cmd!(
            "path.edit",
            "Edit path",
            ["Tools"],
            None,
            "{effect: path sublayer id, index: path index} — loads a stored path into the Path tool (with its brush and material); changes to it repaint the sublayer live until commit or clear",
            has_layer,
            edit_path
        ),
        cmd!(
            "path.remove",
            "Remove path",
            ["Tools"],
            None,
            "{effect: path sublayer id, index: path index} — removes a stored path and repaints its sublayer",
            has_layer,
            remove_path
        ),
    ]
}

fn path(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "tool.path";
    if s.tool.tool != Tool::Path {
        set_tool(s, Tool::Path)?;
    }
    let vp = viewport_of(s, p);
    if boolp(p, "clear") == Some(true) {
        s.tool.path.points.clear();
        s.tool.path.editing = None;
    }
    let placed = match (points_param(CMD, p, "points")?, points_param(CMD, p, "uvPoints")?) {
        (Some(pts), _) => Some((
            pts.iter()
                .enumerate()
                .map(|(i, q)| s.pick(*q, vp).map(|h| h.pos).ok_or_else(|| bad(CMD, format!("point {i} misses the mesh"))))
                .collect::<Result<Vec<_>>>()?,
            false,
        )),
        (None, Some(pts)) => Some((pts.iter().map(|q| q.clamp(Vec2::ZERO, Vec2::ONE).extend(0.0)).collect(), true)),
        (None, None) => None,
    };
    if let Some((points, uv)) = placed {
        if points.len() > MAX_PATH_POINTS {
            return Err(bad(CMD, format!("at most {MAX_PATH_POINTS} control points")));
        }
        s.tool.path.points = points;
        s.tool.path.uv = uv;
    }
    if let Some(v) = boolp(p, "closed") {
        s.tool.path.closed = v;
    }
    if let Some(v) = boolp(p, "fill") {
        s.tool.path.fill = v;
    }
    s.revision = s.revision.wrapping_add(1);
    let committed = if boolp(p, "commit") == Some(true) {
        let r = s.commit_path(vp)?;
        s.update();
        r
    } else {
        if s.path_live()? {
            s.update();
        }
        Value::Null
    };
    let st = &s.tool.path;
    let editing = st.editing.map(|(e, i)| json!({ "effect": e.0, "index": i }));
    Ok(json!({ "points": st.points.len(), "uv": st.uv, "closed": st.closed, "fill": st.fill, "editing": editing, "committed": committed }))
}

/// The `effect` and `index` params naming a stored path, with the path.
fn path_ref(s: &Session, cmd: &str, p: &Value) -> Result<(LayerId, usize, PathShape)> {
    let eid = LayerId(u64p(p, "effect").ok_or_else(|| bad(cmd, "`effect` (a path sublayer id) is required"))?);
    let index = u64p(p, "index").ok_or_else(|| bad(cmd, "`index` is required"))?;
    let shapes = s.paths(eid).ok_or_else(|| bad(cmd, format!("{} is not a path sublayer", eid.0)))?;
    let i =
        usize::try_from(index).ok().filter(|i| *i < shapes.len()).ok_or_else(|| bad(cmd, format!("no path {index} (the sublayer has {})", shapes.len())))?;
    let shape = shapes.get(i).cloned().ok_or_else(|| bad(cmd, format!("no path {index}")))?;
    Ok((eid, i, shape))
}

fn edit_path(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "path.edit";
    let (eid, index, shape) = path_ref(s, CMD, p)?;
    let paint = read_paint(&shape)?;
    set_tool(s, Tool::Path)?;
    s.tool.brush = paint.brush;
    s.tool.values = paint.values;
    s.tool.enabled = paint.enabled;
    s.tool.mask_value = paint.mask_value;
    s.tool.path = PathState { editing: Some((eid, index)), ..state_of(&shape) };
    s.target = StackTarget::Effect(eid);
    Ok(json!({ "effect": eid.0, "index": index, "points": s.tool.path.points.len() }))
}

fn remove_path(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "path.remove";
    let (eid, index, _) = path_ref(s, CMD, p)?;
    s.edit("Remove path", |s| {
        if let Some(Effect { kind: EffectKind::Path(shapes), .. }) = s.doc.project.effect_mut(eid)
            && index < shapes.len()
        {
            shapes.remove(index);
        }
        s.render_paths(eid, index)
    })?;
    if s.tool.path.editing.is_some_and(|(e, _)| e == eid) {
        s.stop_path_edit();
    }
    s.update();
    Ok(json!({ "effect": eid.0, "paths": s.paths(eid).map_or(0, <[_]>::len) }))
}

/// A stored path's settings, with the brush clamped like the tool's own (they come from the file).
fn read_paint(shape: &PathShape) -> Result<PathPaint> {
    let mut paint: PathPaint = serde_json::from_value(shape.paint.clone()).map_err(|e| EngineError::Other(format!("unreadable path settings: {e}")))?;
    paint.brush = paint.brush.sanitized();
    Ok(paint)
}

/// The Path tool state showing a stored path (finite points only, at most [`MAX_PATH_POINTS`]).
fn state_of(shape: &PathShape) -> PathState {
    let points = shape.points.iter().filter(|p| p.iter().all(|v| v.is_finite())).take(MAX_PATH_POINTS).map(|p| Vec3::from_array(*p)).collect();
    PathState { points, uv: shape.uv, closed: shape.closed, fill: shape.fill, editing: None, edited: false }
}

/// NDC (y up) to viewport pixels (origin top-left).
fn to_pixels(ndc: Vec2, viewport: Vec2) -> Vec2 {
    Vec2::new((ndc.x * 0.5 + 0.5) * viewport.x, (0.5 - ndc.y * 0.5) * viewport.y)
}

impl Session {
    /// The path's control points in view space: viewport pixels for a 3D path (None behind the
    /// camera), texture coordinates for a 2D one.
    pub fn path_view_points(&self, viewport: Vec2) -> Vec<Option<Vec2>> {
        let path = &self.tool.path;
        if path.uv {
            return path.points.iter().map(|p| Some(p.truncate())).collect();
        }
        let aspect = viewport.x.max(1.0) / viewport.y.max(1.0);
        path.points.iter().map(|p| self.camera.project(*p, aspect).map(|n| to_pixels(n.truncate(), viewport))).collect()
    }

    /// The stored paths of path sublayer `eid` (None if it isn't one).
    pub fn paths(&self, eid: LayerId) -> Option<&[PathShape]> {
        match self.doc.project.effect(eid) {
            Some((_, Effect { kind: EffectKind::Path(shapes), .. })) => Some(shapes),
            _ => None,
        }
    }

    /// The Path tool's curve with the current brush and material, as painted from `camera`.
    fn path_shape(&self, camera: OrbitCamera, viewport: Vec2) -> PathShape {
        let (t, path) = (&self.tool, &self.tool.path);
        let paint = PathPaint { brush: t.brush.clone(), values: t.values.clone(), enabled: t.enabled.clone(), mask_value: t.mask_value, camera };
        PathShape {
            points: path.points.iter().map(|p| p.to_array()).collect(),
            uv: path.uv,
            closed: path.closed,
            fill: path.fill,
            viewport: viewport.to_array(),
            paint: serde_json::to_value(paint).unwrap_or_default(),
        }
    }

    /// Store the Path tool's curve (a new path in the selected path sublayer, or in a new one;
    /// or the path being edited) and repaint the sublayer, as one undo step. Clears the tool.
    pub fn commit_path(&mut self, viewport: Vec2) -> Result<Value> {
        let shape = self.path_shape(self.camera, viewport);
        let editing = self.tool.path.editing;
        let label = editing.map_or("Path".to_string(), |(_, i)| format!("Edit path {}", i + 1));
        let out = self.edit_merging(&label, editing.is_some() && self.tool.path.edited, |s| {
            let (eid, index) = match editing {
                Some(at) => at,
                None => (s.path_effect()?, usize::MAX),
            };
            s.put_path(eid, index, shape)
        })?;
        if let Some(eid) = out.get("effect").and_then(Value::as_u64) {
            self.target = StackTarget::Effect(LayerId(eid));
        }
        self.tool.path.points.clear();
        self.tool.path.editing = None;
        Ok(out)
    }

    /// While a stored path is edited, repaint its sublayer when the tool's points, brush or
    /// material changed (orbiting or resizing the view alone doesn't), merged into one undo
    /// step. Cheap when nothing changed; the UI calls it every frame. Returns whether it
    /// repainted.
    pub fn path_live(&mut self) -> Result<bool> {
        let Some((eid, index)) = self.tool.path.editing else { return Ok(false) };
        let Some(stored) = self.paths(eid).and_then(|v| v.get(index)).cloned() else {
            self.stop_path_edit();
            return Ok(false);
        };
        let path = &self.tool.path;
        if self.tool.tool != Tool::Path || self.stroke.is_some() || path.points.is_empty() || (path.closed && path.fill && path.points.len() < 3) {
            return Ok(false);
        }
        // Typed comparison: the stored settings went through JSON (f64), maybe a save and load.
        let unchanged = read_paint(&stored).is_ok_and(|p| {
            let now = self.path_shape(p.camera, Vec2::from_array(stored.viewport));
            (&now.points, now.uv, now.closed, now.fill) == (&stored.points, stored.uv, stored.closed, stored.fill) && read_paint(&now).is_ok_and(|n| n == p)
        });
        if unchanged {
            return Ok(false);
        }
        let shape = self.path_shape(self.camera, self.viewport);
        self.edit_merging(&format!("Edit path {}", index + 1), self.tool.path.edited, |s| s.put_path(eid, index, shape))?;
        self.tool.path.edited = true;
        Ok(true)
    }

    /// Stop editing a stored path (its changes are already applied) and clear the tool.
    pub fn stop_path_edit(&mut self) {
        if self.tool.path.editing.take().is_some() {
            self.tool.path.points.clear();
        }
    }

    /// The selected path sublayer, or a new one on the selected layer's content or mask
    /// (whichever is selected).
    fn path_effect(&mut self) -> Result<LayerId> {
        let id = self.selected_layer().ok_or_else(|| EngineError::Other("select a layer to add the path to".into()))?;
        let p = &self.doc.project;
        let mask = match self.target {
            StackTarget::Effect(e) => match p.effect(e) {
                Some((_, x)) if matches!(x.kind, EffectKind::Path(_)) => return Ok(e),
                Some((owner, _)) => !p.layer(owner)?.content_effects.iter().any(|x| x.id == e),
                None => false,
            },
            StackTarget::Mask => true,
            StackTarget::Content => false,
        };
        // An instance's content and (unless it has its own) mask belong to its source.
        let owner = if mask { p.mask_owner(id) } else { p.content_layer(id).map_or(id, |l| l.id) };
        let eid = self.doc.project.alloc_id();
        let l = self.doc.project.layer_mut(owner)?;
        let e = Effect::new(eid, EffectKind::Path(Vec::new()));
        if mask {
            l.mask.get_or_insert_with(|| Mask::new(MaskBackground::Black)).effects.push(e);
        } else if l.is_group() {
            return Err(EngineError::Other(format!("\"{}\" is a folder: add the path to a layer or a mask", l.name)));
        } else {
            l.content_effects.push(e);
        }
        Ok(eid)
    }

    /// Replace path `index` of sublayer `eid` (append when out of range) and repaint the
    /// sublayer. Returns what painting that path reported, with the sublayer and index.
    fn put_path(&mut self, eid: LayerId, index: usize, shape: PathShape) -> Result<Value> {
        let index = match self.doc.project.effect_mut(eid) {
            Some(Effect { kind: EffectKind::Path(shapes), .. }) => match shapes.get_mut(index) {
                Some(slot) => {
                    *slot = shape;
                    index
                }
                None => {
                    shapes.push(shape);
                    shapes.len() - 1
                }
            },
            _ => return Err(EngineError::Other(format!("{} is not a path sublayer", eid.0))),
        };
        let mut out = self.render_paths(eid, index)?.get(index).cloned().unwrap_or_else(|| json!({}));
        out["effect"] = json!(eid.0);
        out["index"] = json!(index);
        Ok(out)
    }

    /// Repaint path sublayer `eid` from its stored paths, bottom to top, where paths before
    /// `from` are unchanged: their pixels come from [`PathCache`] when it holds them, and are
    /// cached for the next repaint otherwise, so editing a path repaints it and the paths above
    /// only. Runs inside the caller's undo step. Returns each painted path's report (null for
    /// the cached ones).
    fn render_paths(&mut self, eid: LayerId, from: usize) -> Result<Vec<Value>> {
        let shapes = self.paths(eid).ok_or_else(|| EngineError::Other(format!("{} is not a path sublayer", eid.0)))?.to_vec();
        let key = (self.mesh_id, self.caches.get(self.ts()).map(|c| c.size));
        self.doc.pixels.masks.remove(&eid);
        self.doc.pixels.layers.retain(|(id, _), _| *id != eid);
        let start = match &self.path_cache {
            Some(c) if c.eid == eid && c.key == key && c.below.len() <= from && shapes.get(..c.below.len()) == Some(&c.below[..]) => {
                if let Some(m) = &c.mask {
                    self.doc.pixels.masks.insert(eid, m.clone());
                }
                self.doc.pixels.layers.extend(c.layers.iter().cloned());
                c.below.len()
            }
            _ => 0,
        };
        // Each path goes through the regular stroke and fill code with its own settings; their
        // own history entries are dropped (the caller records one step).
        let saved = (self.tool.clone(), self.camera, self.target, self.quick_mask.editing);
        let history = (std::mem::take(&mut self.undo), std::mem::take(&mut self.redo));
        self.target = StackTarget::Effect(eid);
        self.quick_mask.editing = false;
        self.tool.tool = Tool::Path;
        self.painting_paths = true;
        let mut out = vec![Value::Null; start];
        let mut result = Ok(());
        for i in start..=shapes.len() {
            if i == from.min(shapes.len()) && i > start {
                let pixels = &self.doc.pixels;
                let layers = pixels.layers.iter().filter(|((id, _), _)| *id == eid).map(|(k, v)| (*k, v.clone())).collect();
                self.path_cache = Some(PathCache { eid, key, below: shapes[..i].to_vec(), mask: pixels.masks.get(&eid).cloned(), layers });
            }
            let Some(sh) = shapes.get(i) else { break };
            match self.paint_shape(sh) {
                Ok(v) => out.push(v),
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        self.painting_paths = false;
        (self.tool, self.camera, self.target, self.quick_mask.editing) = saved;
        (self.undo, self.redo) = history;
        result.map(|_| out)
    }

    fn paint_shape(&mut self, shape: &PathShape) -> Result<Value> {
        let paint = read_paint(shape)?;
        self.tool.brush = paint.brush;
        self.tool.values = paint.values;
        self.tool.enabled = paint.enabled;
        self.tool.mask_value = paint.mask_value;
        self.camera = paint.camera;
        self.tool.path = state_of(shape);
        self.paint_path(Vec2::from_array(shape.viewport).max(Vec2::splat(16.0)).min(Vec2::splat(16384.0)))
    }

    /// Paint the Path tool's curve with the current brush (or fill it when closed and filled)
    /// into the paint target.
    fn paint_path(&mut self, viewport: Vec2) -> Result<Value> {
        let (uv, closed) = (self.tool.path.uv, self.tool.path.closed);
        let fill = closed && self.tool.path.fill;
        let pts: Vec<Vec2> = self.path_view_points(viewport).into_iter().flatten().collect();
        if pts.is_empty() {
            return Err(EngineError::Other("the path has no points: click the surface to add some".into()));
        }
        if fill && pts.len() < 3 {
            return Err(EngineError::Other("a filled path needs at least three points".into()));
        }
        let curve = spline(&pts, closed, if uv { STEP_UV } else { STEP_PX });
        Ok(if fill {
            json!({ "filledTiles": self.lasso_fill(curve, uv, viewport)? })
        } else if uv {
            json!({ "tiles": self.stroke_uv_points(&curve)? })
        } else {
            json!({ "stamps": self.stroke_points(&curve, viewport)? })
        })
    }

    /// Fill the inside of `poly` (viewport pixels over the visible surface, or texture coordinates); returns the touched tiles
    fn lasso_fill(&mut self, poly: Vec<Vec2>, uv: bool, viewport: Vec2) -> Result<usize> {
        let ts = self.ts();
        let (gbuf, mut canvas) = self.fill_canvas(ts)?;
        let w = gbuf.width.max(1);

        if uv {
            let lasso = Lasso::new(poly.iter().map(|p| *p * Vec2::new(w as f32, gbuf.height as f32)).collect());
            fill_coverage(&mut canvas, &gbuf, |i, _| lasso.coverage(Vec2::new((i as u32 % w) as f32 + 0.5, (i as u32 / w) as f32 + 0.5)));
        } else {
            let lasso = Lasso::new(poly);
            let view_proj = self.camera.view_proj(viewport.x.max(1.0) / viewport.y.max(1.0));
            let inv = view_proj.inverse();
            let (cull, mesh, bvh) = (self.tool.brush.backface_culling, &self.mesh, &self.bvh);

            // A visible source inside the lasso can paint each symmetry copy
            let (lo, hi) = mesh.bounds();
            let symmetry = self.tool.brush.sanitized().symmetry;
            let symmetry_origin = symmetry.origin((lo + hi) * 0.5);
            let inverses: Vec<_> = symmetry.transforms().iter().map(|m| m.inverse()).collect();

            fill_coverage(&mut canvas, &gbuf, |i, _| {
                let Some(s) = gbuf.surface(mesh, i) else { return 0.0 };
                let mut best = 0.0f32;

                for m in &inverses {
                    let pos = symmetry_origin + *m * (s.pos - symmetry_origin);
                    let c = view_proj * pos.extend(1.0);
                    if c.w <= MIN_PROJECTIVE_W {
                        continue;
                    }
                    let ndc = Vec2::new(c.x, c.y) / c.w;
                    let a = lasso.coverage(to_pixels(ndc, viewport));
                    if a <= best {
                        continue;
                    }

                    // Use the source surface normal; a copy may face away from the camera
                    let o = inv * ndc.extend(0.0).extend(1.0);
                    let origin = o.truncate() / o.w;
                    let to = pos - origin;
                    let d = to.length();
                    let Some(dir) = to.try_normalize() else { continue };
                    let Some(hit) =
                        bvh.closest(origin, dir, d * (1.0 + RAY_HIT_RELATIVE_TOLERANCE)).filter(|hit| hit.t >= d * (1.0 - RAY_HIT_RELATIVE_TOLERANCE))
                    else {
                        continue;
                    };

                    let Some(source) = surface_at(mesh, hit.tri, hit.u, hit.v) else { continue };

                    if cull && source.nrm.dot(dir) >= 0.0 {
                        continue;
                    }

                    best = a;
                }

                best
            });
        }
        let tiles = canvas.dirty.len();
        self.commit_fill("Path fill", ts, &canvas)?;
        Ok(tiles)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{Session, execute};

    /// The path sublayer's pixels, comparable.
    fn pixels(s: &Session, eid: texturecraft_core::LayerId) -> Vec<String> {
        let mut v: Vec<String> = s.doc.pixels.layers.iter().filter(|((id, _), _)| *id == eid).map(|(k, img)| format!("{k:?}{img:?}")).collect();
        v.sort();
        v
    }

    #[test]
    fn cached_repaint_matches_a_full_repaint() {
        let mut s = Session::new_project("t", texturecraft_geom::primitives::sphere(1.0, 24, 12), "builtin:test", None, 128).unwrap();
        s.update();
        execute(&mut s, "tool.brush", &json!({"size": 6})).unwrap();
        let mut eid = 0;
        for v in [0.2, 0.5, 0.8] {
            let r = execute(&mut s, "tool.path", &json!({"uvPoints": [[0.1, v], [0.9, v + 0.05]], "commit": true})).unwrap();
            eid = r["committed"]["effect"].as_u64().unwrap();
        }
        let eid = texturecraft_core::LayerId(eid);
        // Live edits of the middle path reuse the bottom path's pixels.
        execute(&mut s, "path.edit", &json!({"effect": eid.0, "index": 1})).unwrap();
        for v in [0.4, 0.45] {
            execute(&mut s, "tool.path", &json!({"uvPoints": [[0.1, v], [0.5, v], [0.9, v]]})).unwrap();
        }
        assert_eq!(s.path_cache.as_ref().map(|c| c.below.len()), Some(1));
        let r = s.render_paths(eid, 1).unwrap();
        assert!(r[0].is_null() && !r[1].is_null(), "the bottom path came from the cache: {r:?}");
        let cached = pixels(&s, eid);
        s.path_cache = None;
        s.render_paths(eid, 0).unwrap();
        assert_eq!(cached, pixels(&s, eid), "same pixels as painting every path");
        assert!(!cached.is_empty());
    }
}

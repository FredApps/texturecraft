//! Polygon fill: fill the triangles, polygons, UV islands or connected meshes under a click or inside a dragged box, in either view.

use std::borrow::Cow;
use std::collections::{BTreeSet, HashSet};

use texturecraft_geom::glam::Vec2;
use texturecraft_geom::{Mesh, NO_TRI};
use texturecraft_paint::{EyeDepth, fill_triangles, screen_point};

use crate::fill_mode::FillMode;
use crate::polygon_fill_drag::PolygonFillDrag;
use crate::session::Session;
use crate::triangle_partition::TrianglePartition;
use crate::{EngineError, Result};

/// Corners closer than one step of this grid (per unit of position or UV) count as the same corner
const CORNER_GRID_STEPS: f32 = 1e5;

impl Session {
    /// Fill the triangle / polygon / UV island / connected mesh under viewport pixel `screen`
    pub fn polygon_fill(&mut self, screen: Vec2, viewport: Vec2) -> Result<usize> {
        self.refuse_while_baking()?;
        let hit = self.pick(screen, viewport).ok_or_else(|| EngineError::Other("nothing under the cursor".into()))?;
        if hit.material != self.fill_material()? {
            return Err(EngineError::Other("that surface belongs to another texture set".into()));
        }

        self.fill_group_of(hit.tri)
    }

    /// Fill like [`Session::polygon_fill`] at texture coordinate `uv` (the 2D view): the triangle covering that texel's center, so slivers thinner than a texel can't be clicked
    pub fn polygon_fill_uv(&mut self, uv: Vec2) -> Result<usize> {
        self.refuse_while_baking()?;
        let gbuf = self.caches.get(self.ts()).map(|cache| cache.gbuf.clone()).ok_or_else(|| EngineError::Other("texture set not ready".into()))?;
        let size = Vec2::new(gbuf.width as f32, gbuf.height as f32);
        let texel = (uv * size).floor();
        let triangle = (texel.cmpge(Vec2::ZERO).all() && texel.cmplt(size).all())
            .then(|| gbuf.tri.get(texel.y as usize * gbuf.width as usize + texel.x as usize).copied())
            .flatten()
            .filter(|triangle| *triangle != NO_TRI)
            .ok_or_else(|| EngineError::Other("nothing under the cursor".into()))?;

        self.fill_group_of(triangle)
    }

    /// Fill what the box between viewport pixels `corner` and `opposite` selects (one undo step)
    pub fn polygon_fill_box(&mut self, corner: Vec2, opposite: Vec2, viewport: Vec2) -> Result<usize> {
        self.refuse_while_baking()?;

        // A scripted box stands alone: it finishes any drag under way, and ends even when it fails
        self.end_polygon_fill_box();
        let shown = self.drag_polygon_fill_box(corner, opposite, viewport);
        let filled = self.end_polygon_fill_box();
        shown.map(|_| filled)
    }

    /// Fill what the box between texture coordinates `corner` and `opposite` selects (the 2D view, one undo step)
    pub fn polygon_fill_box_uv(&mut self, corner: Vec2, opposite: Vec2) -> Result<usize> {
        self.refuse_while_baking()?;

        self.end_polygon_fill_box();
        let shown = self.drag_polygon_fill_box_uv(corner, opposite);
        let filled = self.end_polygon_fill_box();
        shown.map(|_| filled)
    }

    /// Show the fill of a box being dragged between viewport pixels `corner` and `opposite`
    pub fn drag_polygon_fill_box(&mut self, corner: Vec2, opposite: Vec2, viewport: Vec2) -> Result<usize> {
        // Whole pixels, as the visibility buffer is rendered at
        self.drag_polygon_fill(corner, opposite, Some(viewport.round().max(Vec2::ONE)))
    }

    /// Show the fill of a box being dragged between texture coordinates `corner` and `opposite` (the 2D view)
    pub fn drag_polygon_fill_box_uv(&mut self, corner: Vec2, opposite: Vec2) -> Result<usize> {
        self.drag_polygon_fill(corner, opposite, None)
    }

    /// Finish the dragged box: what it filled becomes one undo step. Returns the filled triangle count
    pub fn end_polygon_fill_box(&mut self) -> usize {
        let Some(drag) = self.polygon_fill_drag.take() else { return 0 };

        let filled = drag.filled.len();
        // Quick mask edits keep no history, as with its strokes
        if filled > 0 && drag.quick_mask_before.is_none() {
            self.push_history("Polygon fill", drag.before);
        }
        filled
    }

    /// Show the fill of a box from `corner` to `opposite` in the 3D view of size `viewport`, or in the 2D view when None
    fn drag_polygon_fill(&mut self, corner: Vec2, opposite: Vec2, viewport: Option<Vec2>) -> Result<usize> {
        self.refuse_while_baking()?;
        if !corner.is_finite() || !opposite.is_finite() {
            return Err(EngineError::Other("the box corners must be finite".into()));
        }

        let (low, high) = (corner.min(opposite), corner.max(opposite));
        self.start_polygon_fill_drag(viewport)?;
        let Some(drag) = self.polygon_fill_drag.as_mut() else { return Ok(0) };

        if drag.failed || drag.last_box == Some((low, high)) {
            return Ok(drag.filled.len());
        }

        drag.last_box = Some((low, high));
        let selected = self.box_selection(low, high);
        self.show_box_fill(selected)
    }

    /// Continue a box drag only while its view, paint target and quick mask state still match
    fn start_polygon_fill_drag(&mut self, viewport: Option<Vec2>) -> Result<()> {
        let ts = self.ts();
        let target = (self.selected_layer(), self.target);
        let quick_mask = self.quick_mask.editing;
        // Finish the old target before another preview can restore its earlier document snapshot
        if let Some(drag) = &self.polygon_fill_drag {
            if drag.ts == ts && drag.target == target && drag.viewport == viewport && drag.quick_mask_before.is_some() == quick_mask {
                // A camera move (a wheel zoom mid-drag) keeps the drag, which then sees the mesh from the new camera
                if drag.camera != self.camera {
                    let (points, depth) = self.box_space(viewport);
                    if let Some(drag) = self.polygon_fill_drag.as_mut() {
                        (drag.points, drag.depth, drag.camera, drag.last_box) = (points, depth, self.camera, None);
                    }
                }

                return Ok(());
            }

            self.end_polygon_fill_box();
        }

        let material = self.fill_material()?;
        let (points, depth) = self.box_space(viewport);
        let polygons = match self.tool.fill_mode {
            FillMode::Triangle | FillMode::Polygon => None,
            FillMode::UvIsland | FillMode::Mesh => Some(TrianglePartition::new(&self.mesh, material, polygon_edges)),
        };
        self.polygon_fill_drag = Some(PolygonFillDrag {
            ts,
            target,
            viewport,
            camera: self.camera,
            before: self.doc.clone(),
            quick_mask_before: quick_mask.then(|| self.quick_mask.masks.clone()),
            points,
            depth,
            fill_groups: self.fill_partition(material),
            polygons,
            filled: BTreeSet::new(),
            last_box: None,
            failed: false,
        });
        Ok(())
    }

    /// Each vertex where a box sees it (viewport pixels for a 3D view of size `viewport`, else texture coordinates) and, for visible polygons only in 3D, what the camera sees
    fn box_space(&self, viewport: Option<Vec2>) -> (Vec<Option<Vec2>>, Option<EyeDepth>) {
        let Some(size) = viewport else { return (self.mesh.uvs.iter().copied().map(Some).collect(), None) };

        let view_proj = self.camera.view_proj(size.x / size.y);
        let points = self.mesh.positions.iter().map(|position| screen_point(view_proj, *position, size)).collect();
        let depth = self.tool.fill_visible_only.then(|| EyeDepth::render_with_triangles(&self.shown_mesh(), &self.camera, size.x as u32, size.y as u32));
        (points, depth)
    }

    /// The mesh material of the texture set being painted
    fn fill_material(&self) -> Result<usize> {
        Ok(self.doc.project.texture_set(self.ts())?.material)
    }

    /// The triangles the current fill mode fills together, for `material`
    fn fill_partition(&self, material: usize) -> TrianglePartition {
        match self.tool.fill_mode {
            FillMode::Triangle => TrianglePartition::new(&self.mesh, material, |_, _| Vec::<u32>::new()),
            FillMode::Polygon => TrianglePartition::new(&self.mesh, material, polygon_edges),
            FillMode::UvIsland => TrianglePartition::new(&self.mesh, material, uv_edges),
            FillMode::Mesh => TrianglePartition::new(&self.mesh, material, position_edges),
        }
    }

    /// Fill the fill-mode group holding `triangle` (one undo step)
    fn fill_group_of(&mut self, triangle: u32) -> Result<usize> {
        let ts = self.ts();
        let groups = self.fill_partition(self.fill_material()?);
        let triangles: BTreeSet<u32> = groups.group(triangle).unwrap_or_default().iter().copied().collect();

        let (gbuf, mut canvas) = self.fill_canvas(ts)?;
        fill_triangles(&mut canvas, &gbuf, &triangles);
        self.commit_fill("Polygon fill", ts, &canvas)?;
        Ok(triangles.len())
    }

    /// The mesh as the 3D view shows it: hidden texture sets' triangles collapse to a point, so nothing behind them counts as hidden
    fn shown_mesh(&self) -> Cow<'_, Mesh> {
        let sets = &self.doc.project.texture_sets;
        if sets.iter().all(|set| set.visible) {
            return Cow::Borrowed(&self.mesh);
        }

        let mut mesh = (*self.mesh).clone();
        for set in sets.iter().filter(|set| !set.visible) {
            for triangle in self.mesh.triangles_of(set.material) {
                if let Some(corners) = mesh.indices.get_mut(triangle * 3..triangle * 3 + 3) {
                    let first = corners[0];
                    corners.fill(first);
                }
            }
        }
        Cow::Owned(mesh)
    }

    /// Triangles the dragged box from `low` to `high` fills: each polygon the hit-test selects, grown by the fill mode
    fn box_selection(&self, low: Vec2, high: Vec2) -> BTreeSet<u32> {
        let Some(drag) = &self.polygon_fill_drag else { return BTreeSet::new() };

        let visible = drag.depth.as_ref().zip(drag.viewport).map(|(depth, viewport)| visible_triangles(depth, viewport, low, high));
        let point = |vertex: usize| drag.points.get(vertex).copied().flatten();
        let corners = |triangle: u32| {
            let [first, second, third] = self.mesh.tri(triangle as usize)?;
            Some([point(first)?, point(second)?, point(third)?])
        };
        let hit_test = self.tool.fill_hit_test;
        let mut polygon_corners = Vec::new();
        let mut groups = BTreeSet::new();
        for polygon in &drag.polygons.as_ref().unwrap_or(&drag.fill_groups).groups {
            let seen = visible.as_ref().is_none_or(|visible| polygon.iter().any(|triangle| visible.contains(triangle)));
            if !seen {
                continue;
            }

            polygon_corners.clear();
            polygon_corners.extend(polygon.iter().map_while(|triangle| corners(*triangle)));
            if polygon_corners.len() == polygon.len() && hit_test.selects(&polygon_corners, low, high) {
                groups.extend(polygon.iter().filter_map(|triangle| drag.fill_groups.index_of(*triangle)));
            }
        }

        groups.iter().filter_map(|group| drag.fill_groups.groups.get(*group)).flatten().copied().collect()
    }

    /// Show the box drag's fill of `selected`: the document before the drag, refilled when the selection changed
    fn show_box_fill(&mut self, selected: BTreeSet<u32>) -> Result<usize> {
        let Some(drag) = self.polygon_fill_drag.as_mut() else { return Ok(0) };
        if drag.filled == selected {
            return Ok(selected.len());
        }

        let ts = drag.ts;
        let previous = std::mem::replace(&mut self.doc, drag.before.clone());
        if let Some(masks) = &drag.quick_mask_before {
            self.quick_mask.masks = masks.clone();
            self.quick_mask.revision = self.quick_mask.revision.wrapping_add(1);
        }

        let painted = if selected.is_empty() {
            Ok(())
        } else {
            self.fill_canvas(ts).and_then(|(gbuf, mut canvas)| {
                fill_triangles(&mut canvas, &gbuf, &selected);
                self.paint_fill(ts, &canvas)
            })
        };
        self.invalidate_changed(&previous);

        // A failed fill is reported once and leaves the document as before the drag; the rest of the drag fills nothing
        let selected_count = selected.len();
        if let Some(drag) = self.polygon_fill_drag.as_mut() {
            drag.failed = painted.is_err();
            drag.filled = if drag.failed { BTreeSet::new() } else { selected };
        }
        painted.map(|_| selected_count)
    }
}

/// Triangles `depth` (rendered for a `viewport` pixel view) shows in the viewport pixels from `low` to `high`, both edges included
fn visible_triangles(depth: &EyeDepth, viewport: Vec2, low: Vec2, high: Vec2) -> HashSet<u32> {
    let depth_pixels_per_viewport_pixel = Vec2::new(depth.width as f32, depth.height as f32) / viewport;
    let (low, high) = (low * depth_pixels_per_viewport_pixel, high * depth_pixels_per_viewport_pixel);
    let columns = pixel_span(low.x, high.x, depth.width);
    let rows = pixel_span(low.y, high.y, depth.height);

    rows.flat_map(|row| depth.triangle.get(row * depth.width as usize..).into_iter().flat_map(|pixels| pixels.iter().take(columns.end).skip(columns.start)))
        .copied()
        .filter(|triangle| *triangle != NO_TRI)
        .collect()
}

/// The pixels `0..count` a span from `low` to `high` covers, both edges included (empty when it lies outside)
fn pixel_span(low: f32, high: f32, count: u32) -> std::ops::Range<usize> {
    let first = low.floor().clamp(0.0, count as f32) as usize;
    let end = (high.floor() + 1.0).clamp(0.0, count as f32) as usize;
    first..end.max(first)
}

/// Texture coordinates of `triangle`'s corners
fn uv_corners(mesh: &Mesh, triangle: usize) -> Option<[Vec2; 3]> {
    let [first, second, third] = mesh.tri(triangle)?;
    Some([*mesh.uvs.get(first)?, *mesh.uvs.get(second)?, *mesh.uvs.get(third)?])
}

/// The polygon diagonals of `triangle`: edges the importer added to split a polygon
fn polygon_edges(mesh: &Mesh, triangle: usize) -> Vec<(u32, u32)> {
    let hidden = mesh.diagonals.get(triangle).copied().unwrap_or(0);
    let Some(vertices) = mesh.indices.get(triangle * 3..triangle * 3 + 3) else { return Vec::new() };

    (0..3)
        .filter(|edge| hidden >> edge & 1 == 1)
        .map(|edge| {
            let (from, to) = (vertices[edge], vertices[(edge + 1) % 3]);
            (from.min(to), from.max(to))
        })
        .collect()
}

/// `triangle`'s edges in UV space: imports often split vertices along hard edges where the UVs run on, so vertex indices alone break islands apart
fn uv_edges(mesh: &Mesh, triangle: usize) -> Vec<([i64; 2], [i64; 2])> {
    let corner = |uv: Vec2| (uv * CORNER_GRID_STEPS).round().to_array().map(|step| step as i64);
    let Some([first, second, third]) = uv_corners(mesh, triangle).map(|uvs| uvs.map(corner)) else { return Vec::new() };

    [(first, second), (second, third), (third, first)].map(|(from, to)| (from.min(to), from.max(to))).to_vec()
}

/// `triangle`'s edges by position: UV seams and hard edges split vertices but leave the geometry connected
fn position_edges(mesh: &Mesh, triangle: usize) -> Vec<([i64; 3], [i64; 3])> {
    let corner = |vertex: usize| mesh.positions.get(vertex).map(|position| (*position * CORNER_GRID_STEPS).round().to_array().map(|step| step as i64));
    let Some([first, second, third]) = mesh.tri(triangle) else { return Vec::new() };
    let (Some(first), Some(second), Some(third)) = (corner(first), corner(second), corner(third)) else { return Vec::new() };

    [(first, second), (second, third), (third, first)].map(|(from, to)| (from.min(to), from.max(to))).to_vec()
}

#[cfg(test)]
mod tests {
    use texturecraft_core::Channel;
    use texturecraft_geom::glam::{Vec2, Vec3};
    use texturecraft_geom::mesh::Mesh;
    use texturecraft_geom::primitives;

    use crate::fill_hit_test::FillHitTest;
    use crate::fill_mode::FillMode;
    use crate::session::{Session, Tool};

    /// The roughness the test fills paint, to tell filled texels apart
    const FILL_ROUGHNESS: f32 = 0.9;

    /// A polygon fill session on `mesh` painting roughness [`FILL_ROUGHNESS`]
    fn fill_session(mesh: Mesh) -> Session {
        let mut session = Session::new_project("t", mesh, "test", None, 128).unwrap();
        session.viewport = Vec2::new(400.0, 400.0);
        session.tool.tool = Tool::PolygonFill;
        session.tool.values.insert(Channel::Roughness, [FILL_ROUGHNESS; 3]);
        session.update();
        session
    }

    /// Whether the texel at texture coordinate `uv` shows the fill's roughness
    fn filled_at(session: &Session, uv: Vec2) -> bool {
        let (width, height) = session.caches[0].size;
        let roughness = session.caches[0].composite.value(Channel::Roughness, (uv.x * width as f32) as u32, (uv.y * height as f32) as u32);
        (roughness[0] - FILL_ROUGHNESS).abs() < 0.01
    }

    /// Two copies of a plane apart in space and in UV space, each triangle with its own three vertices (as hard edges or per-face imports leave a mesh)
    fn split_planes() -> (Mesh, usize) {
        let plane = primitives::plane(2.0, 4);
        let (mut positions, mut uvs) = (Vec::<Vec3>::new(), Vec::<Vec2>::new());
        for shift in [0.0, 0.5] {
            for vertex in plane.indices.iter().map(|index| *index as usize) {
                positions.push(plane.positions[vertex] + Vec3::Y * shift);
                uvs.push(plane.uvs[vertex] * 0.5 + shift);
            }
        }
        let indices = (0..positions.len() as u32).collect();
        let mesh = Mesh::from_parts(positions, None, uvs, indices, "m").unwrap();
        assert_eq!(mesh.vertex_count(), 6 * plane.triangle_count(), "no vertex is shared");
        (mesh, plane.triangle_count())
    }

    #[test]
    fn uv_island_and_mesh_fills_follow_seams_split_vertices_leave_apart() {
        let (mesh, per_plane) = split_planes();
        let mut session = Session::new_project("t", mesh, "test", None, 128).unwrap();

        for fill_mode in [FillMode::UvIsland, FillMode::Mesh] {
            session.tool.fill_mode = fill_mode;
            let groups = session.fill_partition(0);
            assert_eq!(groups.group(0).map(<[u32]>::len), Some(per_plane), "{fill_mode:?}: the whole first plane");
            assert_eq!(groups.group(per_plane as u32).map(<[u32]>::len), Some(per_plane), "{fill_mode:?}: only the second plane");
        }
    }

    #[test]
    fn polygon_mode_groups_the_two_triangles_of_each_quad() {
        let mut session = fill_session(primitives::plane(2.0, 2));

        session.tool.fill_mode = FillMode::Triangle;
        assert_eq!(session.fill_partition(0).group(0), Some(&[0][..]));

        session.tool.fill_mode = FillMode::Polygon;
        let mut quad = session.fill_partition(0).group(0).unwrap().to_vec();
        quad.sort_unstable();
        assert_eq!(quad, [0, 1], "the quad's two triangles and no other quad");
    }

    #[test]
    fn a_uv_box_over_half_a_plane_fills_that_half() {
        let mut session = fill_session(primitives::plane(2.0, 4));
        session.tool.fill_hit_test = FillHitTest::ContainedCenter;

        let filled = session.polygon_fill_box_uv(Vec2::new(-0.01, -0.01), Vec2::new(0.5, 1.01)).unwrap();
        session.update();

        assert_eq!(filled, 16, "the left two columns of quads");
        assert!(filled_at(&session, Vec2::new(0.1, 0.5)) && filled_at(&session, Vec2::new(0.45, 0.9)));
        assert!(!filled_at(&session, Vec2::new(0.55, 0.5)) && !filled_at(&session, Vec2::new(0.9, 0.1)));
    }

    #[test]
    fn intersection_and_contained_center_differ_on_a_box_missing_a_quads_center() {
        let mut session = fill_session(primitives::plane(2.0, 2));
        session.tool.fill_mode = FillMode::Polygon;
        let (corner, opposite) = (Vec2::new(0.0, 0.0), Vec2::new(0.2, 0.2));

        session.tool.fill_hit_test = FillHitTest::ContainedCenter;
        assert_eq!(session.polygon_fill_box_uv(corner, opposite).unwrap(), 0, "the quad's center (0.25, 0.25) is outside");
        assert_ne!(session.undo_label(), Some("Polygon fill"), "an empty box records no undo step");

        session.tool.fill_hit_test = FillHitTest::Intersection;
        assert_eq!(session.polygon_fill_box_uv(corner, opposite).unwrap(), 2, "the quad the box cuts");
    }

    #[test]
    fn visible_only_skips_the_back_of_a_cube() {
        let mut session = fill_session(primitives::cube(1.0));
        session.camera.yaw = 0.6;
        session.camera.pitch = 0.4;
        let (low, high) = (Vec2::ZERO, session.viewport);
        let every_triangle = session.mesh.triangle_count();

        session.tool.fill_visible_only = true;
        assert_eq!(session.polygon_fill_box(low, high, session.viewport).unwrap(), every_triangle / 2, "the three sides the camera sees");

        session.tool.fill_visible_only = false;
        assert_eq!(session.polygon_fill_box(low, high, session.viewport).unwrap(), every_triangle);
    }

    #[test]
    fn visible_only_sees_through_a_hidden_texture_set() {
        let mut session = fill_session(primitives::material_ball());
        let base = session.doc.project.texture_sets.iter().position(|set| set.name == "Base").unwrap();
        session.active_set = base;
        let (low, high) = (Vec2::ZERO, session.viewport);

        let behind_the_ball = session.polygon_fill_box(low, high, session.viewport).unwrap();
        session.doc.project.texture_sets[0].visible = false;
        let ball_hidden = session.polygon_fill_box(low, high, session.viewport).unwrap();

        assert!(ball_hidden > behind_the_ball, "hiding the ball shows more of the base: {ball_hidden} vs {behind_the_ball}");
    }

    #[test]
    fn a_dragged_box_fills_live_and_records_one_step_when_it_ends() {
        let mut session = fill_session(primitives::plane(2.0, 4));
        let history = session.history_labels().len();

        assert_eq!(session.drag_polygon_fill_box_uv(Vec2::ZERO, Vec2::ONE).unwrap(), 32);
        assert_eq!(session.drag_polygon_fill_box_uv(Vec2::ZERO, Vec2::new(0.45, 1.0)).unwrap(), 16, "shrinking the box unfills");
        session.update();
        assert!(filled_at(&session, Vec2::new(0.2, 0.5)) && !filled_at(&session, Vec2::new(0.8, 0.5)), "the fill shows while dragging");
        assert_eq!(session.history_labels().len(), history, "no undo step yet");

        assert_eq!(session.end_polygon_fill_box(), 16);
        assert_eq!(session.history_labels().len(), history + 1);
        session.undo().unwrap();
        session.update();
        assert!(!filled_at(&session, Vec2::new(0.2, 0.5)), "one undo removes the whole drag");
    }

    #[test]
    fn a_failed_box_fill_leaves_no_drag_behind() {
        let mut session = fill_session(primitives::plane(2.0, 4));
        let layer = session.selected.remove(&0).unwrap();
        assert!(session.polygon_fill_box_uv(Vec2::ZERO, Vec2::ONE).is_err(), "no layer to paint on");

        session.selected.insert(0, layer);
        assert_eq!(session.polygon_fill_box_uv(Vec2::ZERO, Vec2::ONE).unwrap(), 32, "the next box fills again");
    }

    #[test]
    fn undo_and_tool_changes_finish_a_box_drag_first() {
        let mut session = fill_session(primitives::plane(2.0, 4));
        let history = session.history_labels().len();

        session.drag_polygon_fill_box_uv(Vec2::ZERO, Vec2::ONE).unwrap();
        assert_eq!(session.undo().unwrap(), "Polygon fill", "undo takes back the drag it finished");
        assert!(session.polygon_fill_drag.is_none());
        assert_eq!(session.history_labels().len(), history);

        session.drag_polygon_fill_box_uv(Vec2::ZERO, Vec2::ONE).unwrap();
        crate::tool_cmds::set_tool(&mut session, Tool::Paint).unwrap();
        assert!(session.polygon_fill_drag.is_none());
        assert_eq!(session.history_labels().len(), history + 1, "a tool change keeps the fill as one step");
    }

    #[test]
    fn an_unmoved_box_keeps_its_fill() {
        let mut session = fill_session(primitives::plane(2.0, 4));
        let narrow_box = Vec2::new(0.3, 1.0);
        assert_eq!(session.drag_polygon_fill_box_uv(Vec2::ZERO, narrow_box).unwrap(), 16, "intersection: the two columns the box touches");

        // Contained center would select one column, so a second selection would show 8
        session.tool.fill_hit_test = FillHitTest::ContainedCenter;
        assert_eq!(session.drag_polygon_fill_box_uv(Vec2::ZERO, narrow_box).unwrap(), 16, "the same box selects nothing again");
        assert_eq!(session.drag_polygon_fill_box_uv(Vec2::ZERO, narrow_box + Vec2::new(0.01, 0.0)).unwrap(), 8, "a moved box selects again");
    }

    #[test]
    fn a_uv_box_over_the_second_uv_tile_fills_that_tile_only() {
        let mut session = fill_session(primitives::udim_plane());
        assert_eq!(session.doc.project.texture_sets[0].uv_tiles, [1001, 1002]);
        session.tool.fill_hit_test = FillHitTest::ContainedCenter;

        let every_triangle = session.mesh.triangle_count();
        assert_eq!(session.polygon_fill_box_uv(Vec2::new(0.5, 0.0), Vec2::ONE).unwrap(), every_triangle / 2, "the atlas's right half is tile 1002");
    }

    #[test]
    fn a_2d_click_skips_a_triangle_with_collapsed_uvs() {
        // A face left unmapped (all corners at UV 0,0) comes first, as imports often leave them
        let plane = primitives::plane(2.0, 1);
        let mut positions = vec![Vec3::ZERO, Vec3::X, Vec3::Z];
        let mut uvs = vec![Vec2::ZERO; 3];
        positions.extend(&plane.positions);
        uvs.extend(&plane.uvs);
        let indices = [0, 1, 2].into_iter().chain(plane.indices.iter().map(|index| index + 3)).collect();
        let mut session = fill_session(Mesh::from_parts(positions, None, uvs, indices, "m").unwrap());

        assert_eq!(session.polygon_fill_uv(Vec2::new(0.2, 0.7)).unwrap(), 1);
        session.update();
        assert!(filled_at(&session, Vec2::new(0.2, 0.7)), "the triangle under the cursor, not the collapsed one");
    }

    #[test]
    fn a_failing_drag_reports_once_and_records_nothing() {
        let mut session = fill_session(primitives::plane(2.0, 4));
        let history = session.history_labels().len();
        session.selected.clear();

        assert!(session.drag_polygon_fill_box_uv(Vec2::ZERO, Vec2::new(0.5, 1.0)).is_err());
        assert_eq!(session.drag_polygon_fill_box_uv(Vec2::ZERO, Vec2::ONE).unwrap(), 0, "the rest of the drag fills nothing");
        assert_eq!(session.end_polygon_fill_box(), 0);
        assert_eq!(session.history_labels().len(), history);
    }

    #[test]
    fn an_edit_during_a_drag_records_the_fill_first() {
        let mut session = fill_session(primitives::plane(2.0, 4));
        session.drag_polygon_fill_box_uv(Vec2::ZERO, Vec2::ONE).unwrap();

        let layers = session.doc.project.layers.len();
        crate::execute(&mut session, "layer.add.paint", &serde_json::Value::Null).unwrap();
        assert!(session.polygon_fill_drag.is_none());
        let labels = session.history_labels();
        assert_eq!(labels.get(labels.len().wrapping_sub(2)).map(String::as_str), Some("Polygon fill"), "{labels:?}");

        session.drag_polygon_fill_box_uv(Vec2::ZERO, Vec2::new(0.45, 1.0)).unwrap();
        assert_eq!(session.doc.project.layers.len(), layers + 1, "a later drag move keeps the added layer");
    }

    #[test]
    fn a_zero_size_box_on_whole_pixels_selects_what_is_under_it() {
        let mut session = fill_session(primitives::cube(1.0));
        let center = (session.viewport * 0.5).floor();

        assert!(session.polygon_fill_box(center, center, session.viewport).unwrap() > 0);
    }

    #[test]
    fn a_camera_move_mid_drag_keeps_one_drag() {
        let mut session = fill_session(primitives::cube(1.0));
        let viewport = session.viewport;
        let history = session.history_labels().len();

        let (corner, opposite) = (viewport * 0.4, viewport * 0.6);

        let before_zoom = session.drag_polygon_fill_box(corner, opposite, viewport).unwrap();
        session.camera.zoom(0.5);
        let after_zoom = session.drag_polygon_fill_box(corner, opposite, viewport).unwrap();
        session.end_polygon_fill_box();

        assert_ne!(before_zoom, after_zoom, "the same box selects again from the new camera");
        assert_eq!(session.history_labels().len(), history + 1, "a wheel zoom during the drag doesn't split it");
    }

    #[test]
    fn pixel_spans_include_both_edges_and_nothing_outside() {
        assert_eq!(super::pixel_span(2.0, 2.0, 100), 2..3, "a zero-size span on a whole pixel");
        assert_eq!(super::pixel_span(-50.0, -10.0, 100), 0..0, "before the first pixel");
        assert_eq!(super::pixel_span(120.0, 150.0, 100), 100..100, "past the last pixel");
        assert_eq!(super::pixel_span(-5.0, 500.0, 100), 0..100);
    }

    #[test]
    fn a_2d_click_fills_the_triangle_under_it() {
        let mut session = fill_session(primitives::plane(2.0, 1));

        assert_eq!(session.polygon_fill_uv(Vec2::new(0.2, 0.7)).unwrap(), 1);
        session.update();

        assert!(filled_at(&session, Vec2::new(0.2, 0.7)));
        assert!(!filled_at(&session, Vec2::new(0.7, 0.2)), "the quad's other triangle stays");
        assert!(session.polygon_fill_uv(Vec2::new(1.5, 0.5)).is_err(), "nothing outside the UVs");
    }
}

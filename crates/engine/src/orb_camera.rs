//! Cached camera rays for one preview orb size

use std::collections::HashMap;

use texturecraft_geom::glam::{Quat, Vec2, Vec3};
use texturecraft_geom::surface_at;

use crate::Session;

/// Camera angles expose the front, left and top panels
const CAMERA_PITCH_DEG: f32 = 35.0;
const CAMERA_YAW_DEG: f32 = -34.4;

/// Index into [`OrbCamera::texels`] and view-space normal; None on a miss
pub(crate) type CameraSample = Option<(usize, Vec3)>;

/// What each ray of a square orthographic view of the orb sees
pub(crate) struct OrbCamera {
    /// Rays along each side of the view
    pub rays_per_side: usize,
    /// The texels (x, y) the camera sees, each once
    pub texels: Vec<(u32, u32)>,
    /// Per ray, row-major `rays_per_side²`
    pub samples: Vec<CameraSample>,
}

impl OrbCamera {
    /// Trace a square orthographic view onto a `texture_px²` texture
    pub fn new(session: &Session, texture_px: u32, rays_per_side: usize) -> OrbCamera {
        let (min, max) = session.mesh.bounds();
        let center = (min + max) * 0.5;
        let radius = session.mesh.positions.iter().map(|position| position.distance(center)).fold(0.0f32, f32::max);
        let pitch = CAMERA_PITCH_DEG.to_radians();
        let turn = Quat::from_rotation_y(CAMERA_YAW_DEG.to_radians());
        let forward = turn * Vec3::new(0.0, -pitch.sin(), -pitch.cos());
        let right = turn * Vec3::X;
        let up = right.cross(forward);
        let max_texel = texture_px.saturating_sub(1);

        let mut texels = Vec::new();
        let mut index_of = HashMap::new();
        let samples = (0..rays_per_side * rays_per_side)
            .map(|ray| {
                let side = rays_per_side as f32;
                let ndc = Vec2::new(((ray % rays_per_side) as f32 + 0.5) / side * 2.0 - 1.0, 1.0 - ((ray / rays_per_side) as f32 + 0.5) / side * 2.0);
                // Start well outside the shape so every ray enters it from the front
                let origin = center + (right * ndc.x + up * ndc.y) * radius - forward * radius * 4.0;
                let hit = session.bvh.closest(origin, forward, radius * 8.0)?;
                let surface = surface_at(&session.mesh, hit.tri, hit.u, hit.v)?;
                let texel = (((surface.uv.x * texture_px as f32) as u32).min(max_texel), ((surface.uv.y * texture_px as f32) as u32).min(max_texel));
                let index = *index_of.entry(texel).or_insert_with(|| {
                    texels.push(texel);
                    texels.len() - 1
                });

                let normal = surface.nrm;
                Some((index, Vec3::new(normal.dot(right), normal.dot(up), -normal.dot(forward))))
            })
            .collect();

        OrbCamera { rays_per_side, texels, samples }
    }
}

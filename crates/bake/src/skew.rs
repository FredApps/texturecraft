//! Skew correction adapted from SurfaceCraft's baker (MIT OR Apache-2.0)

mod prepared_skew;

#[cfg(test)]
use crate::BakePaintMap;
pub(crate) use prepared_skew::PreparedSkew;

#[cfg(test)]
mod tests {
    use super::prepared_skew::{hard_edges, segment_distance};
    use super::*;
    use texturecraft_geom::glam::{Vec2, Vec3};
    use texturecraft_geom::{Mesh, Surface};

    fn fold() -> Mesh {
        Mesh { positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z, Vec3::new(1.0, 1.0, 0.0)], indices: vec![0, 1, 2, 0, 3, 1, 1, 4, 2], ..Mesh::default() }
    }

    #[test]
    fn hard_edges_are_where_faces_meet_steeply() {
        let hard = hard_edges(&fold());
        assert!(hard[0][0] && hard[1][2], "the fold is hard from both sides: {hard:?}");
        assert!(!hard[0][1] && !hard[2][2], "the flat join is soft: {hard:?}");
        assert!((segment_distance(Vec3::new(0.5, 1.0, 0.0), Vec3::ZERO, Vec3::X) - 1.0).abs() < 1e-6);
        assert!(hard_edges(&Mesh::default()).is_empty());
    }

    #[test]
    fn maps_sample_bilinearly_and_bad_ones_are_refused() {
        let map = BakePaintMap { width: 2, height: 1, values: vec![0.0, 1.0] };
        assert!(map.validate().is_ok());
        assert!(
            map.sample(Vec2::new(0.0, 0.5)).abs() < 1e-6
                && (map.sample(Vec2::new(1.0, 0.5)) - 1.0).abs() < 1e-6
                && (map.sample(Vec2::new(0.5, 0.5)) - 0.5).abs() < 1e-6
        );
        assert!(map.sample(Vec2::new(-3.0, 9.0)).is_finite() && map.sample(Vec2::new(f32::NAN, 0.0)) == 0.0);
        for bad in [
            BakePaintMap { values: vec![0.0], ..map.clone() },
            BakePaintMap { width: 0, values: vec![], ..map.clone() },
            BakePaintMap { values: vec![0.0, f32::NAN], ..map.clone() },
            BakePaintMap { width: u32::MAX, height: u32::MAX, ..map.clone() },
        ] {
            assert!(bad.validate().is_err());
        }
    }

    #[test]
    fn correction_blends_towards_the_base_and_edges_hold_it_back() {
        let mesh = fold();
        let up = Vec3::new(0.6, 0.8, 0.0);
        let base = Vec3::Y;
        let go = |map: &BakePaintMap, pos: Vec3, edge_distance: f32| {
            let s = Surface { pos, nrm: base, geo_nrm: base, tangent: Default::default(), uv: Vec2::splat(0.5), tri: 0 };
            PreparedSkew::new(
                map,
                &mesh,
                1.0,
                texturecraft_core::SkewSettings { edge_protection: edge_distance > 0.0, edge_distance, edge_contrast: 1.0, ..Default::default() },
            )
            .unwrap()
            .correct(&mesh, &s, up)
        };
        assert!((go(&BakePaintMap::constant(0.0), Vec3::ZERO, 0.0) - up).length() < 1e-6, "black leaves the ray alone");
        assert!((go(&BakePaintMap::constant(1.0), Vec3::ZERO, 0.0) - base).length() < 1e-6, "white turns it onto the base normal");
        let half = go(&BakePaintMap::constant(0.5), Vec3::ZERO, 0.0);
        assert!(half.x > 0.0 && half.x < up.x && (half.length() - 1.0).abs() < 1e-5, "grey is in between and still unit: {half}");
        let guarded = BakePaintMap::constant(1.0);
        assert!((go(&guarded, Vec3::new(0.5, 0.0, 0.0), 0.5) - up).length() < 1e-6, "on the hard edge the ray stays on the cage direction");
        assert!((go(&guarded, Vec3::new(0.3, 0.9, 0.0), 0.5) - base).length() < 1e-6, "away from every hard edge the correction is full");
    }

    #[test]
    fn painting_blends_dabs_in_and_resizing_keeps_the_picture() {
        let mut map = BakePaintMap::constant(0.0).resized(64, 64).unwrap();
        assert!(map.values.iter().all(|v| *v == 0.0));
        map.paint(&[Vec2::new(0.5, 0.5)], 0.2, 1.0, 1.0, 1.0).unwrap();
        let at = |m: &BakePaintMap, u: f32, v: f32| m.sample(Vec2::new(u, v));
        assert!(at(&map, 0.5, 0.5) > 0.99 && at(&map, 0.95, 0.95) == 0.0, "{} {}", at(&map, 0.5, 0.5), at(&map, 0.95, 0.95));
        let mut dab = BakePaintMap::constant(0.0).resized(64, 64).unwrap();
        dab.paint(&[Vec2::new(0.5, 0.5)], 0.1, 0.8, 1.0, 0.5).unwrap();
        assert!((at(&dab, 0.5, 0.5) - 0.4).abs() < 0.02, "{}", at(&dab, 0.5, 0.5));
        let mut line = BakePaintMap::constant(0.0).resized(64, 64).unwrap();
        line.paint(&[Vec2::new(0.1, 0.9), Vec2::new(0.9, 0.9)], 0.05, 0.5, 1.0, 0.5).unwrap();
        assert!(at(&line, 0.5, 0.9) > 0.45 && at(&line, 0.5, 0.9) <= 0.5 + 1e-4, "{}", at(&line, 0.5, 0.9));
        assert!(at(&line, 0.5, 0.1) == 0.0, "the other end of v is untouched");
        map.paint(&[Vec2::new(0.5, 0.5)], 0.2, 0.0, 1.0, 1.0).unwrap();
        assert!(at(&map, 0.5, 0.5) < 0.01);
        let mut small = BakePaintMap::constant(0.0).resized(8, 8).unwrap();
        small.paint(&[Vec2::new(0.25, 0.75)], 0.2, 1.0, 1.0, 1.0).unwrap();
        let big = small.resized(32, 32).unwrap();
        assert!((at(&big, 0.25, 0.75) - at(&small, 0.25, 0.75)).abs() < 0.1 && at(&big, 0.9, 0.1) == 0.0);
    }

    #[test]
    fn hostile_brushes_are_refused_and_change_nothing() {
        let mut map = BakePaintMap::constant(0.3).resized(16, 16).unwrap();
        let before = map.clone();
        let c = [Vec2::new(0.5, 0.5)];
        for (pts, r, v) in
            [(&c[..], 0.0, 1.0), (&c[..], -1.0, 1.0), (&c[..], f32::NAN, 1.0), (&c[..], 0.1, f32::INFINITY), (&[Vec2::new(f32::NAN, 0.0)][..], 0.1, 1.0)]
        {
            assert!(map.paint(pts, r, v, 0.5, 1.0).is_err(), "{pts:?} {r} {v}");
        }
        map.paint(&[Vec2::new(1e6, -1e6)], 0.1, 1.0, 0.5, 1.0).unwrap();
        assert_eq!(map, before);
        assert!(map.paint(&[Vec2::ZERO, Vec2::new(1e6, 1e6)], 1e-4, 1.0, 0.5, 1.0).is_err());
        assert_eq!(map, before);
        let mut big = BakePaintMap::constant(0.0).resized(4096, 4096).unwrap();
        let untouched = big.clone();
        assert!(big.paint(&[Vec2::splat(0.5); 8], 1.0, 1.0, 0.5, 1.0).is_err());
        assert_eq!(big, untouched, "nothing was painted before the refusal");
        assert!(big.paint(&[Vec2::splat(0.5)], 1.0, 1.0, 0.5, 1.0).is_ok(), "one such dab is within the budget");
        assert!(BakePaintMap::constant(0.0).resized(0, 4).is_err() && BakePaintMap::constant(0.0).resized(100_000, 100_000).is_err());
    }
}

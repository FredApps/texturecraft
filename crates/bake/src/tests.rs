use std::sync::Arc;
use texturecraft_geom::{BaseColor, Bvh};

use texturecraft_core::{
    AmbientOcclusionSettings, BakeAntialiasing, BakeResolution, BakeSettings, BentNormalSettings, CageSettings, CommonBakeSettings, CurvatureSettings,
    PositionSettings,
};

use std::sync::atomic::Ordering::Relaxed;

use texturecraft_core::{BackfaceMode, PositionMode, RayDistribution, RaySettings, SelfOcclusion};
use texturecraft_geom::glam::{Vec2, Vec3, Vec4};
use texturecraft_geom::primitives;

use super::*;

#[path = "test/setting.rs"]
mod setting;

#[path = "test/normal_map_curvature.rs"]
mod normal_map_curvature;

#[path = "test/tile_selection.rs"]
mod tile_selection;

fn settings(resolution: u32, maps: &[BakedMap]) -> BakeSettings {
    with_rays(
        {
            let mut base = BakeSettings::default();
            base.common =
                CommonBakeSettings { resolution: BakeResolution::from_pixels(resolution).unwrap(), dilation: 0, apply_diffusion: false, ..base.common };
            base.maps.curvature.parameters = CurvatureSettings { method: CurvatureMethod::Mesh, ..base.maps.curvature.parameters };
            base.maps.position.parameters = PositionSettings { normalization: PositionMode::BoundingBox, ..base.maps.position.parameters };
            base.maps.curvature.parameters.mesh.auto_tonemapping = false;
            base.maps.height.parameters.normalization = texturecraft_core::HeightNormalization::RelativeToRayDistances;
            base.maps.thickness.parameters.normalization = texturecraft_core::ThicknessNormalization::RelativeToRayDistances;
            base.maps.select(maps.iter().copied());
            base
        },
        64,
        0.25,
    )
}

fn with_rays(mut s: BakeSettings, rays: u32, max_distance: f32) -> BakeSettings {
    for r in [&mut s.maps.ao.parameters.rays, &mut s.maps.thickness.parameters.rays, &mut s.maps.bent_normal.parameters.rays] {
        *r = RaySettings { rays, max_distance, ..*r };
    }
    s
}

fn run(mesh: &Mesh, s: &BakeSettings) -> Result<BakeOutput, BakeError> {
    let bvh = Bvh::new(mesh);
    bake(
        &BakeRequest { low_base_color: &[], mesh, bvh: &bvh, material: 0, settings: s, high: None, cage: None, skew: None, cage_paint: None, tiles: (1, 1) },
        &Progress::new(),
    )
}

fn covered_values(mesh: &Mesh, out: &BakeOutput, map: BakedMap) -> Vec<(Vec3, f32)> {
    let res = out.resolution;
    let g = UvGBuffer::new(mesh, 0, res, res);
    let data = &out.maps[&map];
    (0..(res * res) as usize).filter_map(|i| g.surface(mesh, i).map(|s| (s.pos, data[i * map.components()]))).collect()
}

fn mean(v: impl Iterator<Item = f32>) -> f32 {
    let (s, n) = v.fold((0.0, 0), |(s, n), x| (s + x, n + 1));
    assert!(n > 0, "no samples");
    s / n as f32
}

fn quad() -> Mesh {
    Mesh::from_parts(
        vec![Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 0.0, -1.0), Vec3::new(1.0, 0.0, 1.0), Vec3::new(-1.0, 0.0, 1.0)],
        Some(vec![Vec3::Y; 4]),
        vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0), Vec2::new(0.0, 1.0)],
        vec![0, 2, 1, 0, 3, 2],
        "Quad",
    )
    .unwrap()
}

#[test]
fn open_quad_is_unoccluded() {
    let m = quad();
    let out = run(&m, &settings(32, &[BakedMap::AmbientOcclusion])).unwrap();
    let ao = covered_values(&m, &out, BakedMap::AmbientOcclusion);
    assert!(ao.iter().all(|(_, v)| *v > 0.999), "{:?}", ao.iter().map(|v| v.1).fold(1.0f32, f32::min));
}

#[test]
fn inside_of_a_closed_box_is_occluded() {
    let mut m = primitives::cube(2.0);
    for t in m.indices.chunks_exact_mut(3) {
        t.swap(1, 2);
    }
    for n in &mut m.normals {
        *n = -*n;
    }
    m.compute_tangents();
    let s = with_rays(settings(64, &[BakedMap::AmbientOcclusion]), 64, 4.0);
    let out = run(&m, &s).unwrap();
    let ao = mean(covered_values(&m, &out, BakedMap::AmbientOcclusion).into_iter().map(|v| v.1));
    assert!(ao < 0.3, "{ao}");
}

#[test]
fn thin_slab_is_thinner_than_a_block() {
    let thick = |m: &Mesh| mean(covered_values(m, &run(m, &settings(64, &[BakedMap::Thickness])).unwrap(), BakedMap::Thickness).into_iter().map(|v| v.1));
    let slab = thick(&primitives::rounded_box(Vec3::new(1.0, 0.05, 1.0), 0.0));
    let block = thick(&primitives::cube(2.0));
    assert!(slab < 0.4 && block > 0.6, "slab {slab}, block {block}");
}

#[test]
fn sphere_curvature_stays_calm() {
    for (rings, res) in [(64, 64), (12, 128)] {
        let m = primitives::sphere(1.0, rings * 2, rings);
        let s = settings(res, &[BakedMap::Curvature]);
        let out = run(&m, &s).unwrap();
        let c: Vec<f32> = covered_values(&m, &out, BakedMap::Curvature).into_iter().map(|v| v.1).collect();
        let (lo, hi) = c.iter().fold((1.0f32, 0.0f32), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
        assert!(lo >= 0.5 && hi < 0.6 && hi - lo < 0.08, "{rings} rings: {lo}..{hi}");
    }
}

#[test]
fn flat_surface_is_exactly_mid_grey() {
    let m = quad();
    let out = run(&m, &settings(64, &[BakedMap::Curvature])).unwrap();
    let c = covered_values(&m, &out, BakedMap::Curvature);
    assert!(c.iter().filter(|(p, _)| p.x.abs() < 0.9 && p.z.abs() < 0.9).all(|(_, v)| *v == 0.5));
}

#[test]
fn contact_with_a_wall_is_a_dark_line_and_the_rest_stays_flat() {
    let m = floor_and_wall(false);
    let s = settings(128, &[BakedMap::Curvature]);
    let line = floor_mean(&m, &s, BakedMap::Curvature, 0.96..1.0);
    assert!(line < 0.47, "beside the wall {line}");
    assert_eq!(floor_mean(&m, &s, BakedMap::Curvature, -0.9..0.5), 0.5);
}

fn smooth_shaded_box() -> Mesh {
    let (mut pos, mut nrm, mut uvs, mut idx) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let faces: [(Vec3, Vec3, Vec3); 6] = [
        (Vec3::X, Vec3::Y, Vec3::Z),
        (Vec3::NEG_X, Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::Z, Vec3::X),
        (Vec3::NEG_Y, Vec3::X, Vec3::Z),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (Vec3::NEG_Z, Vec3::Y, Vec3::X),
    ];
    for (f, (n, a, b)) in faces.iter().enumerate() {
        let base = pos.len() as u32;
        let cell = Vec2::new((f % 3) as f32 / 3.0, (f / 3) as f32 / 2.0);
        for (u, v) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let p = *n + *a * u + *b * v;
            pos.push(p);
            nrm.push(p.normalize());
            uvs.push(cell + Vec2::new((u * 0.5 + 0.5) * 0.3 + 0.015, (v * 0.5 + 0.5) * 0.45 + 0.025));
        }
        idx.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    Mesh::from_parts(pos, Some(nrm), uvs, idx, "Smooth box").unwrap()
}

fn edge_peak(m: &Mesh, res: u32, band: f32) -> f32 {
    let c = covered_values(m, &run(m, &settings(res, &[BakedMap::Curvature])).unwrap(), BakedMap::Curvature);
    c.iter().filter(|(p, _)| p.abs().to_array().iter().filter(|v| **v > 1.0 - band).count() >= 2).map(|(_, v)| (v - 0.5).abs()).fold(0.0, f32::max)
}

#[test]
fn smooth_shaded_corners_draw_no_hard_line() {
    let (hard, smooth) = (edge_peak(&primitives::cube(2.0), 256, 0.03), edge_peak(&smooth_shaded_box(), 256, 0.03));
    assert!(hard > 0.05, "hard {hard}");
    assert!(smooth < hard * 0.5, "smooth {smooth} hard {hard}");
}

#[test]
fn lines_are_as_wide_in_texels_at_any_scale_of_the_part() {
    let width = |res: u32| {
        let m = primitives::cube(2.0);
        let c = covered_values(&m, &run(&m, &settings(res, &[BakedMap::Curvature])).unwrap(), BakedMap::Curvature);
        c.iter()
            .filter(|(p, v)| (v - 0.5).abs() > 0.02 && p.abs().to_array().iter().filter(|a| **a > 0.999).count() == 1)
            .map(|(p, _)| {
                let mut a = p.abs().to_array();
                a.sort_by(f32::total_cmp);
                1.0 - a[1]
            })
            .fold(0.0, f32::max)
    };
    let (coarse, fine) = (width(128), width(256));
    assert!(fine > 0.0 && (fine / coarse - 0.5).abs() < 0.2, "world widths {coarse} and {fine}");
}

fn barrel(radius: f32) -> Mesh {
    const N: u32 = 128;
    let (mut pos, mut nrm, mut uvs, mut idx) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for i in 0..=N {
        let u = i as f32 / N as f32;
        let (s, c) = (u * std::f32::consts::TAU).sin_cos();
        for (y, v) in [(-1.0f32, 0.0f32), (1.0, 1.0)] {
            pos.push(Vec3::new(c * radius, y, s * radius));
            nrm.push(Vec3::new(c, 0.0, s));
            uvs.push(Vec2::new(u, v));
        }
    }
    for i in 0..N {
        let b = i * 2;
        idx.extend([b, b + 1, b + 2, b + 1, b + 3, b + 2]);
    }
    Mesh::from_parts(pos, Some(nrm), uvs, idx, "Barrel").unwrap()
}

#[test]
fn a_gently_round_barrel_stays_below_the_wear_masks() {
    let m = barrel(0.1);
    let c = covered_values(&m, &run(&m, &settings(512, &[BakedMap::Curvature])).unwrap(), BakedMap::Curvature);
    let (lo, hi) = c.iter().filter(|(p, _)| p.y.abs() < 0.9).fold((1.0f32, 0.0f32), |(lo, hi), (_, v)| (lo.min(*v), hi.max(*v)));
    assert!(hi < 0.53 && hi - lo < 0.02, "{lo}..{hi}");
}

#[test]
fn a_rounded_bevel_reads_the_same_at_any_map_size() {
    let m = primitives::rounded_box(Vec3::ONE, 0.05);
    let peak = |res: u32| {
        let c = covered_values(&m, &run(&m, &settings(res, &[BakedMap::Curvature])).unwrap(), BakedMap::Curvature);
        let mut v: Vec<f32> = c.iter().filter(|(p, _)| p.abs().to_array().iter().filter(|a| **a > 0.97).count() >= 2).map(|v| v.1).collect();
        v.sort_by(f32::total_cmp);
        v.get(v.len() / 2).copied().unwrap_or(0.5)
    };
    let (coarse, fine) = (peak(512), peak(1024));
    assert!(coarse > 0.58, "bevel at 512: {coarse}");
    assert!((coarse - fine).abs() < 0.1, "512: {coarse}, 1024: {fine}");
}

fn check_box_curvature(m: &Mesh, edge: f32) {
    let s = {
        let mut base = settings(128, &[BakedMap::Curvature]);
        base.common = CommonBakeSettings { dilation: 4, ..base.common };
        base
    };
    let c = covered_values(m, &run(m, &s).unwrap(), BakedMap::Curvature);
    let near = |p: Vec3, t: f32| p.abs().to_array().iter().filter(|c| **c > t).count();
    let edges: Vec<f32> = c.iter().filter(|(p, _)| near(*p, edge) >= 2).map(|v| v.1).collect();
    let faces: Vec<f32> = c.iter().filter(|(p, _)| near(*p, 0.5) == 1).map(|v| v.1).collect();
    assert!(!edges.is_empty() && !faces.is_empty());
    let min_edge = edges.iter().copied().fold(1.0, f32::min);
    assert!(min_edge > 0.55, "edge min {min_edge}");
    assert!(faces.iter().all(|v| (0.45..=0.55).contains(v)), "face {:?}", faces.iter().copied().fold((1.0f32, 0.0f32), |(a, b), v| (a.min(v), b.max(v))));
}

#[test]
fn rounded_box_bevels_are_convex() {
    check_box_curvature(&primitives::rounded_box(Vec3::ONE, 0.15), 0.9);
}

#[test]
fn cube_edges_are_convex() {
    check_box_curvature(&primitives::cube(2.0), 0.97);
}

#[test]
fn two_runs_are_identical_and_sized() {
    let m = primitives::material_ball();
    let s = with_rays(
        {
            let mut base = settings(32, &[]);
            base.common = CommonBakeSettings { antialiasing: BakeAntialiasing::Subsampling2x2, dilation: 2, ..base.common };
            base.maps.select(BakedMap::ALL);
            base
        },
        8,
        0.25,
    );
    let a = run(&m, &s).unwrap();
    let b = run(&m, &s).unwrap();
    assert_eq!(a, b);
    assert_eq!(a.maps.len(), BakedMap::ALL.len());
    for (map, data) in &a.maps {
        assert_eq!(data.len(), 32 * 32 * map.components(), "{map:?}");
        assert!(data.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)), "{map:?}");
    }
}

#[test]
fn strips_match_a_single_pass() {
    let m = primitives::material_ball();
    let bvh = Bvh::new(&m);
    let s = with_rays(
        {
            let mut base = settings(64, &[]);
            base.common = CommonBakeSettings { antialiasing: BakeAntialiasing::Subsampling2x2, ..base.common };
            base.maps.select(BakedMap::ALL);
            base
        },
        4,
        0.25,
    );
    let req = BakeRequest {
        low_base_color: &[],
        mesh: &m,
        bvh: &bvh,
        material: 0,
        settings: &s,
        high: None,
        cage: None,
        skew: None,
        cage_paint: None,
        tiles: (1, 1),
    };
    let whole = bake(&req, &Progress::new()).unwrap();
    let strips = bake_in_strips(&req, &Progress::new(), 128 * 2, &CpuTracer, &[]).unwrap();
    for (map, a) in &whole.maps {
        let b = &strips.maps[map];
        let off = a.iter().zip(b).filter(|(a, b)| (*a - *b).abs() > 1e-3).count();
        assert!(off * 200 < a.len(), "{map:?}: {off} of {} differ", a.len());
    }
}

#[test]
fn cancel_and_progress() {
    let m = primitives::sphere(1.0, 16, 8);
    let bvh = Bvh::new(&m);
    let s = settings(16, &[BakedMap::AmbientOcclusion, BakedMap::Position]);
    let req = BakeRequest {
        low_base_color: &[],
        mesh: &m,
        bvh: &bvh,
        material: 0,
        settings: &s,
        high: None,
        cage: None,
        skew: None,
        cage_paint: None,
        tiles: (1, 1),
    };
    let p = Progress::new();
    p.cancel();
    assert!(matches!(bake(&req, &p), Err(BakeError::Cancelled)));
    let p = Progress::new();
    bake(&req, &p).unwrap();
    assert_eq!(p.done.load(Relaxed), 10_000);
    assert_eq!(p.fraction(), 1.0);
}

#[test]
fn a_curvature_bake_can_be_cancelled_midway_and_reports_progress() {
    let m = primitives::rounded_box(Vec3::ONE, 0.15);
    let bvh = Bvh::new(&m);
    let s = settings(2048, &[BakedMap::Curvature]);
    let req = BakeRequest {
        low_base_color: &[],
        mesh: &m,
        bvh: &bvh,
        material: 0,
        settings: &s,
        high: None,
        cage: None,
        skew: None,
        cage_paint: None,
        tiles: (1, 1),
    };
    let p = Progress::new();
    std::thread::scope(|t| {
        let baking = t.spawn(|| bake(&req, &p));
        let start = std::time::Instant::now();
        while p.done.load(Relaxed) == 0 {
            assert!(!baking.is_finished() && start.elapsed().as_secs() < 120, "the bake ended or stalled before reporting progress");
            std::thread::yield_now();
        }
        let partway = p.fraction();
        p.cancel();
        assert!(partway < 1.0, "{partway}");
        assert!(matches!(baking.join().unwrap(), Err(BakeError::Cancelled)));
    });
}

#[test]
fn hostile_tile_dimensions_are_rejected() {
    let mesh = quad();
    let bvh = Bvh::new(&mesh);
    let settings = settings(16, &[BakedMap::Position]);

    for tiles in [(1 << 31, 1 << 31), (u32::MAX, u32::MAX), (MAX_TEXELS as u32, 1), (0, 1), (1, 0)] {
        let request = BakeRequest {
            low_base_color: &[],
            mesh: &mesh,
            bvh: &bvh,
            material: 0,
            settings: &settings,
            high: None,
            cage: None,
            skew: None,
            cage_paint: None,
            tiles,
        };

        assert!(matches!(validate(&request), Err(BakeError::Invalid(_))), "{tiles:?}");
        assert!(matches!(bake(&request, &Progress::new()), Err(BakeError::Invalid(_))), "{tiles:?}");
    }
}

#[test]
fn invalid_requests_are_errors() {
    let m = quad();
    let ok = settings(16, &[BakedMap::AmbientOcclusion]);
    let bad = [
        {
            let mut base = ok.clone();
            base.maps.ao.parameters = AmbientOcclusionSettings { rays: RaySettings { rays: 0, ..ok.maps.ao.parameters.rays }, ..base.maps.ao.parameters };
            base
        },
        {
            let mut base = ok.clone();
            base.maps.ao.parameters =
                AmbientOcclusionSettings { rays: RaySettings { rays: MAX_RAYS + 1, ..ok.maps.ao.parameters.rays }, ..base.maps.ao.parameters };
            base
        },
        {
            let mut base = ok.clone();
            base.maps.ao.parameters =
                AmbientOcclusionSettings { rays: RaySettings { max_distance: f32::NAN, ..ok.maps.ao.parameters.rays }, ..base.maps.ao.parameters };
            base
        },
        {
            let mut base = ok.clone();
            base.maps.ao.parameters =
                AmbientOcclusionSettings { rays: RaySettings { max_distance: 0.0, ..ok.maps.ao.parameters.rays }, ..base.maps.ao.parameters };
            base
        },
        {
            let mut base = ok.clone();
            base.maps.ao.parameters =
                AmbientOcclusionSettings { rays: RaySettings { spread: f32::INFINITY, ..ok.maps.ao.parameters.rays }, ..base.maps.ao.parameters };
            base
        },
        {
            let mut base = ok.clone();
            base.cage = CageSettings { max_rear: -1.0, ..base.cage };
            base
        },
    ];
    for s in &bad {
        assert!(matches!(run(&m, s), Err(BakeError::Invalid(_))), "{s:?}");
    }
    let bvh = Bvh::new(&m);
    let req = |mesh: &Mesh, material| {
        bake(
            &BakeRequest { low_base_color: &[], mesh, bvh: &bvh, material, settings: &ok, high: None, cage: None, skew: None, cage_paint: None, tiles: (1, 1) },
            &Progress::new(),
        )
    };
    assert!(matches!(req(&m, 1), Err(BakeError::Invalid(_))));
    let mut broken = m.clone();
    broken.indices[0] = 99;
    assert!(matches!(req(&broken, 0), Err(BakeError::Invalid(_))));
    let mut nan = m.clone();
    nan.positions[0].x = f32::NAN;
    assert!(matches!(req(&nan, 0), Err(BakeError::Invalid(_))));
    let empty = Mesh { materials: vec!["m".into()], ..Mesh::default() };
    let out = req(&empty, 0).unwrap();
    assert!(out.maps[&BakedMap::AmbientOcclusion].iter().all(|v| *v == 1.0));
}

#[test]
fn neutral_values_and_padding() {
    let mut m = quad();
    for uv in &mut m.uvs {
        uv.x *= 0.5;
    }
    let maps = [BakedMap::AmbientOcclusion, BakedMap::Curvature, BakedMap::Thickness, BakedMap::Normal, BakedMap::Position];
    let out = run(&m, &{
        let mut base = settings(16, &maps);
        base.common = CommonBakeSettings { dilation: 2, ..base.common };
        base
    })
    .unwrap();
    let pos = &out.maps[&BakedMap::Position];
    let at = |x: usize, y: usize| &pos[(y * 16 + x) * 3..(y * 16 + x) * 3 + 3];
    assert_eq!(at(9, 5)[0], at(7, 5)[0], "padded texel copies a texel of the last covered column");
    assert_eq!(at(12, 5), [0.0, 0.0, 0.0], "beyond the padding");
    assert_eq!(out.maps[&BakedMap::AmbientOcclusion][5 * 16 + 12], 1.0);
    assert_eq!(out.maps[&BakedMap::Curvature][5 * 16 + 12], 0.5);
    assert_eq!(out.maps[&BakedMap::Thickness][5 * 16 + 12], 1.0);
    assert_eq!(&out.maps[&BakedMap::Normal][(5 * 16 + 12) * 3..(5 * 16 + 12) * 3 + 3], [0.5, 0.5, 1.0]);
}

#[test]
fn normal_and_height_transfer_from_a_high_poly() {
    let low = quad();
    let mut high = quad();
    for p in &mut high.positions {
        p.y += 0.005;
    }
    let tilt = Vec3::new(0.3, 1.0, 0.0).normalize();
    high.normals = vec![tilt; 4];
    high.compute_tangents();
    let (lb, hb) = (Bvh::new(&low), Bvh::new(&high));
    let s = settings(16, &[BakedMap::Normal, BakedMap::Height]);
    let out = bake(
        &BakeRequest {
            low_base_color: &[],
            mesh: &low,
            bvh: &lb,
            material: 0,
            settings: &s,
            high: Some(HighPoly { file_ids: &[], mesh: &high, bvh: &hb, base_color: &[] }),
            cage: None,
            skew: None,
            cage_paint: None,
            tiles: (1, 1),
        },
        &Progress::new(),
    )
    .unwrap();
    let i = 8 * 16 + 8;
    let n = &out.maps[&BakedMap::Normal][i * 3..i * 3 + 3];
    assert!((n[0] - (0.5 + 0.5 * tilt.x)).abs() < 1e-3 && (n[1] - 0.5).abs() < 1e-3 && (n[2] - (0.5 + 0.5 * tilt.y)).abs() < 1e-3, "{n:?}");
    let diag = 2.0;
    let expected = 0.5 + 0.005 / (0.02 * diag);
    assert!((out.maps[&BakedMap::Height][i] - expected).abs() < 1e-3, "{}", out.maps[&BakedMap::Height][i]);
    let flat = run(&low, &s).unwrap();
    assert_eq!(&flat.maps[&BakedMap::Normal][i * 3..i * 3 + 3], [0.5, 0.5, 1.0]);
    assert_eq!(flat.maps[&BakedMap::Height][i], 0.5);
}

fn transfer(low: &Mesh, high: &Mesh, cage: Option<&Mesh>, s: &BakeSettings) -> Result<BakeOutput, BakeError> {
    let (lb, hb) = (Bvh::new(low), Bvh::new(high));
    let mut settings = s.clone();
    if cage.is_some() {
        settings.cage.kind = texturecraft_core::CageType::CustomFile;
    }
    bake(
        &BakeRequest {
            low_base_color: &[],
            mesh: low,
            bvh: &lb,
            material: 0,
            settings: &settings,
            high: Some(HighPoly { file_ids: &[], mesh: high, bvh: &hb, base_color: &[] }),
            cage,
            skew: None,
            cage_paint: None,
            tiles: (1, 1),
        },
        &Progress::new(),
    )
}

#[test]
fn a_high_poly_box_has_bright_edges_and_flat_faces() {
    let s = {
        let mut base = settings(128, &[BakedMap::Curvature]);
        base.cage = CageSettings { max_frontal: 0.06, max_rear: 0.02, ..base.cage };
        base.common = CommonBakeSettings { dilation: 4, ..base.common };
        base
    };
    for (m, edge) in [(primitives::cube(2.0), 0.97), (primitives::rounded_box(Vec3::ONE, 0.15), 0.9)] {
        let c = covered_values(&m, &transfer(&m, &m, None, &s).unwrap(), BakedMap::Curvature);
        let near = |p: Vec3, t: f32| p.abs().to_array().iter().filter(|c| **c > t).count();
        let brightest = c.iter().filter(|(p, _)| near(*p, edge) >= 2).map(|v| v.1).fold(0.0, f32::max);
        assert!(brightest > 0.55, "edge {brightest}");
        assert!(c.iter().filter(|(p, _)| near(*p, 0.5) == 1).all(|v| (0.45..=0.55).contains(&v.1)), "faces");
    }
}

#[test]
fn default_curvature_brightens_box_edges_and_never_darkens_them() {
    let mut bake_settings = settings(256, &[BakedMap::Curvature]);
    bake_settings.cage = CageSettings { max_frontal: 0.06, max_rear: 0.02, ..bake_settings.cage };
    bake_settings.maps.curvature.parameters.method = CurvatureMethod::default();

    // A box has no concave texel, whether it is baked from itself or from a beveled high poly
    let low = primitives::cube(2.0);
    let high = primitives::rounded_box(Vec3::ONE, 0.15);
    let bakes = [("itself", run(&low, &bake_settings).unwrap()), ("a beveled high poly", transfer(&low, &high, None, &bake_settings).unwrap())];
    for (source, bake_output) in bakes {
        let curvature = covered_values(&low, &bake_output, BakedMap::Curvature);
        let darkest = curvature.iter().map(|(_, value)| *value).fold(1.0, f32::min);
        let brightest = curvature.iter().map(|(_, value)| *value).fold(0.0, f32::max);
        assert!(darkest > 0.45, "baked from {source}, darkest {darkest}");
        assert!(brightest > 0.6, "baked from {source}, brightest {brightest}");
    }
}

#[test]
fn a_narrow_high_poly_bevel_stays_bright() {
    let s = {
        let mut base = settings(1024, &[BakedMap::Curvature]);
        base.cage = CageSettings { max_frontal: 0.06, max_rear: 0.02, ..base.cage };
        base.common = CommonBakeSettings { dilation: 4, ..base.common };
        base
    };
    let brightest = |bevel: f32| {
        let m = primitives::rounded_box(Vec3::ONE, bevel);
        let c = covered_values(&m, &transfer(&m, &m, None, &s).unwrap(), BakedMap::Curvature);
        let edge = 1.0 - bevel * 0.7;
        c.iter().filter(|(p, _)| p.abs().to_array().iter().filter(|c| **c > edge).count() >= 2).map(|v| v.1).fold(0.0, f32::max)
    };
    let (narrow, wide) = (brightest(0.01), brightest(0.05));
    assert!(narrow > 0.8, "narrow: {narrow}");
    assert!(wide > 0.55, "wide: {wide}");
}

#[test]
fn a_gently_round_high_poly_stays_below_the_wear_masks() {
    let m = primitives::sphere(1.0, 96, 48);
    let s = {
        let mut base = settings(256, &[BakedMap::Curvature]);
        base.cage = CageSettings { max_frontal: 0.06, max_rear: 0.02, ..base.cage };
        base.common = CommonBakeSettings { dilation: 4, ..base.common };
        base
    };
    let c = covered_values(&m, &transfer(&m, &m, None, &s).unwrap(), BakedMap::Curvature);
    let over = c.iter().filter(|v| v.1 > 0.65).count() as f32 / c.len() as f32;
    assert!(over < 0.01, "{over}");
}

fn quad_texel(res: u32, x: f32, z: f32) -> usize {
    let t = |c: f32| (((c + 1.0) * 0.5 * res as f32) as usize).min(res as usize - 1);
    t(z) * res as usize + t(x)
}

fn bumped(h: f32, sigma: f32) -> Mesh {
    let mut m = primitives::plane(2.0, 128);
    for p in &mut m.positions {
        p.y = h * (-(p.x * p.x + p.z * p.z) / (sigma * sigma)).exp();
    }
    m.compute_normals();
    m.compute_tangents();
    m
}

#[test]
fn bump_tilts_the_baked_normal() {
    let (h, sigma) = (0.1f32, 0.4f32);
    let s = {
        let mut base = settings(64, &[BakedMap::Normal, BakedMap::Height]);
        base.cage = CageSettings { max_frontal: 0.06, max_rear: 0.02, ..base.cage };
        base
    };
    let out = transfer(&quad(), &bumped(h, sigma), None, &s).unwrap();
    let normal = &out.maps[&BakedMap::Normal];
    for (x, z) in [(0.3f32, 0.0f32), (-0.3, 0.0), (0.0, 0.3), (0.0, -0.3), (0.25, 0.25)] {
        let i = quad_texel(64, x, z);
        let (cx, cz) = (((i % 64) as f32 + 0.5) / 32.0 - 1.0, ((i / 64) as f32 + 0.5) / 32.0 - 1.0);
        let f = h * (-(cx * cx + cz * cz) / (sigma * sigma)).exp();
        let (dx, dz) = (-2.0 * cx / (sigma * sigma) * f, -2.0 * cz / (sigma * sigma) * f);
        let n = Vec3::new(-dx, 1.0, -dz).normalize();
        let expected = [0.5 + 0.5 * n.x, 0.5 - 0.5 * n.z, 0.5 + 0.5 * n.y];
        let got = &normal[i * 3..i * 3 + 3];
        assert!(got.iter().zip(expected).all(|(g, e)| (g - e).abs() < 0.02), "at {x},{z}: {got:?} vs {expected:?}");
        assert!((got[0] - 0.5) * x >= 0.0 && (got[1] - 0.5) * -z >= 0.0, "tilts away from the bump centre at {x},{z}: {got:?}");
    }
    let height = &out.maps[&BakedMap::Height];
    assert!(height[quad_texel(64, 0.0, 0.0)] > height[quad_texel(64, 0.6, 0.6)] + 0.2, "the bump top is higher");
}

#[test]
fn bent_normals_point_away_from_occluders() {
    let mut m = quad();
    let wall = Mesh::from_parts(
        vec![Vec3::new(1.0, 0.0, -1.0), Vec3::new(1.0, 0.0, 1.0), Vec3::new(1.0, 2.0, 1.0), Vec3::new(1.0, 2.0, -1.0)],
        Some(vec![-Vec3::X; 4]),
        vec![Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y],
        vec![0, 2, 1, 0, 3, 2],
        "Wall",
    )
    .unwrap();
    m.append(&wall);
    let s = with_rays(settings(32, &[BakedMap::BentNormal, BakedMap::AmbientOcclusion]), 64, 2.0);
    let out = run(&m, &s).unwrap();
    let bent = &out.maps[&BakedMap::BentNormal];
    let ao = covered_values(&m, &out, BakedMap::AmbientOcclusion);
    let g = UvGBuffer::new(&m, 0, 32, 32);
    let near: Vec<&[f32]> = (0..32 * 32).filter(|i| g.surface(&m, *i).is_some_and(|s| s.pos.x > 0.6)).map(|i| &bent[i * 3..i * 3 + 3]).collect();
    let far: Vec<&[f32]> = (0..32 * 32).filter(|i| g.surface(&m, *i).is_some_and(|s| s.pos.x < -0.6)).map(|i| &bent[i * 3..i * 3 + 3]).collect();
    let mean_x = |v: &[&[f32]]| mean(v.iter().map(|b| b[0] * 2.0 - 1.0));
    assert!(mean_x(&near) < -0.15, "near the wall the bent normal leans away from it: {}", mean_x(&near));
    assert!(mean_x(&near) < mean_x(&far), "{} vs {}", mean_x(&near), mean_x(&far));
    assert!(near.iter().all(|b| b[1] > 0.6), "still points up");
    let ao_near = mean(ao.iter().filter(|(p, _)| p.x > 0.6).map(|v| v.1));
    assert!(ao_near < mean(ao.iter().filter(|(p, _)| p.x < -0.6).map(|v| v.1)));
}

#[test]
fn opacity_transfers_where_the_high_poly_is() {
    let mut high = quad();
    high.positions.iter_mut().for_each(|p| p.x = p.x.min(0.0));
    high.colors = vec![Vec4::new(0.0, 1.0, 0.0, 0.5); 4];
    let out = transfer(&quad(), &high, None, &settings(32, &[BakedMap::Opacity])).unwrap();
    let opacity = &out.maps[&BakedMap::Opacity];
    let (left, right) = (quad_texel(32, -0.5, 0.0), quad_texel(32, 0.5, 0.0));
    assert_eq!((opacity[left], opacity[right]), (0.5, 0.0), "vertex alpha where covered, 0 where missed");

    let low_poly_bake = run(&quad(), &settings(32, &[BakedMap::Opacity])).unwrap();
    assert_eq!(low_poly_bake.maps[&BakedMap::Opacity][right], 1.0);
}

#[test]
fn ray_distances_and_the_cage_limit_hits() {
    let low = quad();
    let mut high = quad();
    high.positions.iter_mut().for_each(|p| p.y += 0.2);
    let diag = 2.0;
    let opacity = |s: &BakeSettings, cage: Option<&Mesh>| transfer(&low, &high, cage, s).map(|o| o.maps[&BakedMap::Opacity][quad_texel(16, 0.0, 0.0)]);
    let near = settings(16, &[BakedMap::Opacity]);
    assert_eq!(opacity(&near, None).unwrap(), 0.0, "0.2 above is beyond the default frontal distance");
    assert_eq!(
        opacity(
            &{
                let mut base = near.clone();
                base.cage = CageSettings { max_frontal: 0.25 / diag, ..base.cage };
                base
            },
            None
        )
        .unwrap(),
        1.0
    );
    let mut below = high.clone();
    below.positions.iter_mut().for_each(|p| p.y -= 0.4);
    let rear = |r: f32| {
        transfer(&low, &below, None, &{
            let mut base = near.clone();
            base.cage = CageSettings { max_rear: r, ..base.cage };
            base
        })
        .unwrap()
        .maps[&BakedMap::Opacity][quad_texel(16, 0.0, 0.0)]
    };
    assert_eq!((rear(0.1 / diag), rear(0.25 / diag)), (0.0, 1.0));
    let mut cage = low.clone();
    cage.positions.iter_mut().for_each(|p| p.y += 0.3);
    assert_eq!(opacity(&near, Some(&cage)).unwrap(), 1.0);
    cage.positions.iter_mut().for_each(|p| p.y -= 0.2);
    assert_eq!(opacity(&near, Some(&cage)).unwrap(), 0.0);
    assert!(matches!(opacity(&near, Some(&primitives::cube(1.0))), Err(BakeError::Invalid(_))));
}

#[test]
#[ignore]
fn bench_material_ball_2k() {
    let m = primitives::material_ball();
    let bvh = Bvh::new(&m);
    let all = with_rays(
        {
            let mut base = BakeSettings::default();
            base.common = CommonBakeSettings { resolution: BakeResolution::R2048, ..base.common };
            base
        },
        64,
        0.25,
    );
    for maps in [vec![BakedMap::Position], vec![BakedMap::AmbientOcclusion], vec![BakedMap::Thickness], vec![BakedMap::Curvature], all.maps.enabled().collect()]
    {
        let s = {
            let mut base = all.clone();
            base.maps.select(maps);
            base
        };
        let t = std::time::Instant::now();
        let out = bake(
            &BakeRequest {
                low_base_color: &[],
                mesh: &m,
                bvh: &bvh,
                material: 0,
                settings: &s,
                high: None,
                cage: None,
                skew: None,
                cage_paint: None,
                tiles: (1, 1),
            },
            &Progress::new(),
        )
        .unwrap();
        println!("material ball, material 0, {}² × {:?}, {} rays: {:.2?}", out.resolution, out.maps.keys(), s.maps.ao.parameters.rays.rays, t.elapsed());
    }
}

#[test]
fn uv_tiles_bake_side_by_side_in_one_atlas() {
    let mut m = primitives::udim_plane();
    let grid = texturecraft_core::TileGrid::of(&[1001, 1002]).unwrap();
    m.map_material_uvs(0, |uv| Vec2::from(grid.to_atlas(uv.to_array())));
    let s = {
        let mut base = settings(32, &[BakedMap::Position]);
        base.common = CommonBakeSettings { dilation: 4, ..base.common };
        base
    };
    let bvh = Bvh::new(&m);
    let out = bake(
        &BakeRequest {
            low_base_color: &[],
            mesh: &m,
            bvh: &bvh,
            material: 0,
            settings: &s,
            high: None,
            cage: None,
            skew: None,
            cage_paint: None,
            tiles: (2, 1),
        },
        &Progress::new(),
    )
    .unwrap();
    assert_eq!((out.resolution, out.height), (64, 32));
    let pos = &out.maps[&BakedMap::Position];
    assert_eq!(pos.len(), 64 * 32 * 3);
    let x_at = |x: usize, y: usize| pos[(y * 64 + x) * 3];
    for y in [1, 16, 30] {
        assert!(x_at(2, y) < 0.3 && x_at(29, y) < 0.5, "tile 1001 holds the left plane");
        assert!(x_at(34, y) > 0.5 && x_at(62, y) > 0.7, "tile 1002 holds the right plane");
    }
    let too_many = BakeRequest {
        low_base_color: &[],
        mesh: &m,
        bvh: &bvh,
        material: 0,
        settings: &s,
        high: None,
        cage: None,
        skew: None,
        cage_paint: None,
        tiles: (0, 1),
    };
    assert!(bake(&too_many, &Progress::new()).is_err());
}

#[test]
fn curvature_on_uv_tiles_matches_a_single_tile() {
    let one = primitives::rounded_box(Vec3::ONE, 0.15);
    let mut two = one.clone();
    two.uvs.iter_mut().for_each(|uv| uv.x *= 0.5);
    let s = {
        let mut base = settings(64, &[BakedMap::Curvature]);
        base.common = CommonBakeSettings { dilation: 0, ..base.common };
        base
    };
    let bake_tiles = |m: &Mesh, tiles| {
        let bvh = Bvh::new(m);
        bake(
            &BakeRequest { low_base_color: &[], mesh: m, bvh: &bvh, material: 0, settings: &s, high: None, cage: None, skew: None, cage_paint: None, tiles },
            &Progress::new(),
        )
        .unwrap()
    };
    let (a, b) = (bake_tiles(&one, (1, 1)), bake_tiles(&two, (2, 1)));
    let (a, b) = (&a.maps[&BakedMap::Curvature], &b.maps[&BakedMap::Curvature]);
    let worst = (0..64 * 64).map(|i| (a[i] - b[(i / 64) * 128 + i % 64]).abs()).fold(0.0, f32::max);
    assert!(worst < 1e-3, "{worst}");
}

fn floor_and_wall(away: bool) -> Mesh {
    let mut m = quad();
    let (n, indices) = if away { (Vec3::X, vec![0, 2, 1, 0, 3, 2]) } else { (-Vec3::X, vec![0, 1, 2, 0, 2, 3]) };
    let wall = Mesh::from_parts(
        vec![Vec3::new(1.0, 0.0, -1.0), Vec3::new(1.0, 0.0, 1.0), Vec3::new(1.0, 2.0, 1.0), Vec3::new(1.0, 2.0, -1.0)],
        Some(vec![n; 4]),
        vec![Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y],
        indices,
        "Wall",
    )
    .unwrap();
    m.append(&wall);
    m
}

fn floor_mean(m: &Mesh, s: &BakeSettings, map: BakedMap, range: std::ops::Range<f32>) -> f32 {
    let out = run(m, s).unwrap();
    mean(covered_values(m, &out, map).into_iter().filter(|(p, _)| range.contains(&p.x)).map(|v| v.1))
}

fn near_wall(m: &Mesh, s: &BakeSettings, map: BakedMap) -> f32 {
    floor_mean(m, s, map, 0.6..1.0)
}

#[test]
fn ambient_occlusion_settings_shape_the_occlusion() {
    let mesh = floor_and_wall(false);
    let mut base = with_rays(settings(32, &[BakedMap::AmbientOcclusion]), 64, 2.0);
    base.maps.ao.parameters.rays.relative_to_bounding_box = false;
    base.maps.ao.parameters.rays.max_distance = (mesh.bounds().1 - mesh.bounds().0).length() * 2.0;
    let ao = |configure: &dyn Fn(&mut BakeSettings)| {
        let mut bake_settings = base.clone();
        configure(&mut bake_settings);
        near_wall(&mesh, &bake_settings, BakedMap::AmbientOcclusion)
    };
    let wide = ao(&|_| {});
    assert!(wide < 0.85, "the wall occludes the floor beside it: {wide}");

    let far = |spread: f32| {
        floor_mean(
            &mesh,
            &{
                let mut base = base.clone();
                base.maps.ao.parameters = AmbientOcclusionSettings { rays: RaySettings { spread, ..base.maps.ao.parameters.rays }, ..base.maps.ao.parameters };
                base
            },
            BakedMap::AmbientOcclusion,
            -1.0..-0.2,
        )
    };
    let (narrow, wide_far) = (far(60.0), far(180.0));
    assert!(narrow > 0.99 && wide_far < narrow - 0.05, "far from the wall: spread 60° {narrow}, 180° {wide_far}");

    let uniform = {
        let mut base = base.clone();
        base.maps.ao.parameters = AmbientOcclusionSettings {
            rays: RaySettings { distribution: RayDistribution::Uniform, ..base.maps.ao.parameters.rays },
            ..base.maps.ao.parameters
        };
        base
    };
    let uniform_far = floor_mean(&mesh, &uniform, BakedMap::AmbientOcclusion, -1.0..-0.2);
    assert!(uniform_far < wide_far - 0.02, "far from the wall: uniform {uniform_far}, cosine {wide_far}");

    let own = ao(&|bake_settings| bake_settings.maps.ao.parameters.self_occlusion = SelfOcclusion::SameMeshName);
    assert!(own > 0.999, "{own}");

    let back = |away: bool, ignore: BackfaceMode| {
        near_wall(
            &floor_and_wall(away),
            &{
                let mut base = base.clone();
                base.maps.ao.parameters = AmbientOcclusionSettings { ignore_backface: ignore, ..base.maps.ao.parameters };
                base
            },
            BakedMap::AmbientOcclusion,
        )
    };
    assert!((back(true, BackfaceMode::Never) - wide).abs() < 1e-6, "without the option a wall occludes from either side");
    assert!(back(true, BackfaceMode::Always) > 0.999 && (back(false, BackfaceMode::Always) - wide).abs() < 1e-6);
}

#[test]
fn occlusion_falls_off_linearly_to_the_max_distance() {
    let ceiling_height = 0.5;
    let corners = |height: f32| [Vec3::new(-1.0, height, -1.0), Vec3::new(1.0, height, -1.0), Vec3::new(1.0, height, 1.0), Vec3::new(-1.0, height, 1.0)];
    let mut uvs = vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0), Vec2::new(0.0, 1.0)];
    // The ceiling occludes the floor without contributing baked texels
    uvs.extend([Vec2::ZERO; 4]);
    let normals = [[Vec3::Y; 4], [Vec3::NEG_Y; 4]].concat();
    let mesh = Mesh::from_parts(
        corners(0.0).into_iter().chain(corners(ceiling_height)).collect(),
        Some(normals),
        uvs,
        vec![0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7],
        "Room",
    )
    .unwrap();

    // Nearly vertical rays meet the ceiling at half their maximum distance
    let mut parameters = settings(16, &[BakedMap::AmbientOcclusion]);
    parameters.maps.ao.parameters.rays =
        RaySettings { max_distance: 2.0 * ceiling_height, relative_to_bounding_box: false, spread: 2.0, ..parameters.maps.ao.parameters.rays };
    let output = run(&mesh, &parameters).unwrap();
    let ambient_occlusion = mean(covered_values(&mesh, &output, BakedMap::AmbientOcclusion).into_iter().map(|(_, value)| value));

    assert!((ambient_occlusion - 0.5).abs() < 0.01, "{ambient_occlusion}");
}

#[test]
fn secondary_ray_defaults_preserve_per_map_distances() {
    let parameters = BakeSettings::default();
    let expected_rays = RaySettings {
        rays: 64,
        min_distance: 0.00001,
        max_distance: 1.0,
        relative_to_bounding_box: true,
        spread: 180.0,
        distribution: RayDistribution::Cosine,
    };

    assert_eq!(parameters.maps.ao.parameters.rays, expected_rays);
    assert_eq!(parameters.maps.ao.parameters.attenuation, texturecraft_core::AmbientOcclusionAttenuation::Linear);
    assert_eq!(parameters.maps.bent_normal.parameters.rays, expected_rays);
    assert_eq!(parameters.maps.thickness.parameters.rays, RaySettings { max_distance: 0.1, ..expected_rays });
}

#[test]
fn bent_normals_and_thickness_have_their_own_rays() {
    let m = floor_and_wall(false);
    let s = with_rays(settings(32, &[BakedMap::BentNormal, BakedMap::AmbientOcclusion]), 64, 2.0);
    let lean = |s: &BakeSettings| near_wall(&m, s, BakedMap::BentNormal) * 2.0 - 1.0;
    assert!(lean(&s) < -0.15, "{}", lean(&s));
    let short = {
        let mut base = s.clone();
        base.maps.bent_normal.parameters =
            BentNormalSettings { rays: RaySettings { max_distance: 0.05, rays: 16, ..s.maps.bent_normal.parameters.rays }, ..base.maps.bent_normal.parameters };
        base
    };
    assert!(lean(&short) > 0.5 * lean(&s), "{} vs {}", lean(&short), lean(&s));
    assert_eq!(near_wall(&m, &short, BakedMap::AmbientOcclusion), near_wall(&m, &s, BakedMap::AmbientOcclusion));
    let slab = primitives::rounded_box(Vec3::new(1.0, 0.05, 1.0), 0.0);
    let thick = |spread: f32| {
        let mut s = settings(64, &[BakedMap::Thickness]);
        s.maps.thickness.parameters.rays.spread = spread;
        mean(
            covered_values(&slab, &run(&slab, &s).unwrap(), BakedMap::Thickness)
                .into_iter()
                .filter(|(p, _)| p.y.abs() > 0.049 && p.x.abs().max(p.z.abs()) < 0.8)
                .map(|v| v.1),
        )
    };
    let (narrow, wide) = (thick(20.0), thick(180.0));
    assert!(wide > narrow * 1.2, "spread 180°: {wide} vs 20°: {narrow}");
}

#[test]
fn position_modes() {
    let mesh = quad();
    let positions = |mode: PositionMode| {
        let out = run(&mesh, &{
            let mut base = settings(16, &[BakedMap::Position]);
            base.maps.position.parameters = PositionSettings { normalization: mode, ..base.maps.position.parameters };
            base
        })
        .unwrap();
        let gbuffer = UvGBuffer::new(&mesh, 0, 16, 16);
        (0..256).filter(|i| gbuffer.surface(&mesh, *i).is_some()).map(|i| [0, 1, 2].map(|k| out.maps[&BakedMap::Position][i * 3 + k])).collect::<Vec<_>>()
    };

    let (box_positions, sphere_positions) = (positions(PositionMode::BoundingBox), positions(PositionMode::BoundingSphere));
    assert!(box_positions.iter().all(|p| p[1] == 0.0) && box_positions.iter().any(|p| p[0] < 0.05) && box_positions.iter().any(|p| p[0] > 0.95));
    let unit_scale = 0.25;
    assert!(sphere_positions.iter().all(|p| p[1] == 0.5 && (p[0] - 0.5).abs() <= 2.0 * unit_scale + 1e-4 && (p[2] - 0.5).abs() <= 2.0 * unit_scale + 1e-4));

    let raw_settings = {
        let mut base = settings(16, &[BakedMap::Position]);
        base.maps.position.parameters = PositionSettings { normalization: PositionMode::None, ..base.maps.position.parameters };
        base
    };
    let raw = run(&mesh, &raw_settings).unwrap();
    let gbuffer = UvGBuffer::new(&mesh, 0, 16, 16);
    for texel in 0..16 * 16 {
        let Some(surface) = gbuffer.surface(&mesh, texel) else { continue };

        let actual = &raw.maps[&BakedMap::Position][texel * 3..texel * 3 + 3];
        assert!(actual.iter().zip(surface.pos.to_array()).all(|(actual, expected)| (actual - expected).abs() < 1e-6));
    }
}

#[test]
fn opacity_samples_the_high_poly_base_color_texture() {
    let low = quad();
    let mut high = quad();
    high.positions.iter_mut().for_each(|p| p.y += 0.005);

    let red = [255, 0, 0, 255];
    let blue = [0, 0, 255, 128];
    let texture = BaseColor { factor: Vec4::ONE, width: 4, height: 1, texels: vec![red, red, blue, blue] };
    let high_bvh = Bvh::new(&high);
    let low_bvh = Bvh::new(&low);
    let bake_settings = settings(32, &[BakedMap::Opacity]);
    let bake_with = |base_color: BaseColor| {
        let base_color = [Some(Arc::new(base_color))];
        let request = BakeRequest {
            low_base_color: &[],
            mesh: &low,
            bvh: &low_bvh,
            material: 0,
            settings: &bake_settings,
            high: Some(HighPoly { file_ids: &[], mesh: &high, bvh: &high_bvh, base_color: &base_color }),
            cage: None,
            skew: None,
            cage_paint: None,
            tiles: (1, 1),
        };
        bake(&request, &Progress::new()).unwrap()
    };

    let out = bake_with(texture);

    let (left, right) = (16 * 32 + 8, 16 * 32 + 24);
    let opacity = &out.maps[&BakedMap::Opacity];
    assert!((opacity[left] - 1.0).abs() < 1e-6 && (opacity[right] - 128.0 / 255.0).abs() < 1e-3);

    let half = bake_with(BaseColor { factor: Vec4::new(1.0, 1.0, 1.0, 0.5), width: 0, height: 0, texels: Vec::new() });
    assert_eq!(half.maps[&BakedMap::Opacity][left], 0.5);
}

struct Broken;

impl Tracer for Broken {
    fn name(&self) -> String {
        "Broken".into()
    }

    fn scene<'a>(&'a self, _: &[tracing::TraceMesh<'a>]) -> Result<Box<dyn RayScene + 'a>, BakeError> {
        Err(BakeError::Tracer("no device".into()))
    }
}

#[test]
fn a_failing_tracer_falls_back_to_the_cpu() {
    let m = floor_and_wall(false);
    let bvh = Bvh::new(&m);
    let s = with_rays(settings(16, &[BakedMap::AmbientOcclusion, BakedMap::Thickness]), 8, 1.0);
    let req = BakeRequest {
        low_base_color: &[],
        mesh: &m,
        bvh: &bvh,
        material: 0,
        settings: &s,
        high: None,
        cage: None,
        skew: None,
        cage_paint: None,
        tiles: (1, 1),
    };
    let out = bake_on(&req, &Progress::new(), &Broken).unwrap();
    assert_eq!(out.tracer, "CPU (Broken unavailable: no device)");
    assert_eq!(out.maps, bake(&req, &Progress::new()).unwrap().maps);
    let flat = {
        let mut base = s.clone();
        base.maps.select([BakedMap::Position]);
        base
    };
    assert_eq!(bake_on(&BakeRequest { low_base_color: &[], settings: &flat, ..req }, &Progress::new(), &Broken).unwrap().tracer, "CPU");
}

fn plane(y: f32, half: f32, n: Vec3) -> Mesh {
    Mesh::from_parts(
        vec![Vec3::new(-half, y, -half), Vec3::new(half, y, -half), Vec3::new(half, y, half), Vec3::new(-half, y, half)],
        Some(vec![n; 4]),
        vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0), Vec2::new(0.0, 1.0)],
        vec![0, 2, 1, 0, 3, 2],
        "Plane",
    )
    .unwrap()
}

#[test]
fn skew_correction_preserves_uniform_mesh_normals_and_rejects_invalid_values() {
    let lean = Vec3::new(0.6, 0.8, 0.0);
    let (low, high) = (plane(0.0, 1.0, lean), plane(0.3, 4.0, Vec3::Y));
    let (lb, hb) = (Bvh::new(&low), Bvh::new(&high));
    let s = {
        let mut base = settings(16, &[BakedMap::Height]);
        base.cage = CageSettings { max_frontal: 0.5, max_rear: 0.1, average_normals: true, ..base.cage };
        base
    };
    let diag = (low.bounds().1 - low.bounds().0).max_element();
    let (front, rear) = (0.5 * diag, 0.1 * diag);
    let height = |skew: Option<&BakePaintMap>| {
        let req = BakeRequest {
            low_base_color: &[],
            mesh: &low,
            bvh: &lb,
            material: 0,
            settings: &s,
            high: Some(HighPoly { file_ids: &[], mesh: &high, bvh: &hb, base_color: &[] }),
            cage: None,
            skew,
            cage_paint: None,
            tiles: (1, 1),
        };
        bake(&req, &Progress::new()).unwrap().maps[&BakedMap::Height][8 * 16 + 8]
    };
    let expect = |c: f32| 0.5 + (0.3 / c) / (front + rear);
    let plain = height(None);
    assert!((plain - expect(0.8)).abs() < 1e-3, "leaning rays: {plain} vs {}", expect(0.8));
    assert!((height(Some(&BakePaintMap::constant(0.0))) - plain).abs() < 1e-6, "black leaves the rays alone");
    assert!((height(Some(&BakePaintMap::constant(1.0))) - plain).abs() < 1e-4);
    let bad = BakePaintMap { values: vec![f32::NAN], ..BakePaintMap::constant(0.0) };
    let req = BakeRequest {
        low_base_color: &[],
        mesh: &low,
        bvh: &lb,
        material: 0,
        settings: &s,
        high: None,
        cage: None,
        skew: Some(&bad),
        cage_paint: None,
        tiles: (1, 1),
    };
    assert!(matches!(bake(&req, &Progress::new()), Err(BakeError::Invalid(_))));
}

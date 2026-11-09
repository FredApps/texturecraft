use super::{Broken, settings};
use crate::tracing::CpuTracer;
use crate::{BakeRequest, Progress, bake, bake_in_strips, bake_tiles};
use texturecraft_core::{BakeAntialiasing, BakedMap, TileGrid};
use texturecraft_geom::glam::Vec2;
use texturecraft_geom::{Bvh, primitives};

#[test]
fn excluded_tiles_match_across_strips_antialiasing_and_cpu_fallback() {
    let mut mesh = primitives::udim_plane();
    let grid = TileGrid::of(&[1001, 1002]).unwrap();
    mesh.map_material_uvs(0, |uv| Vec2::from(grid.to_atlas(uv.to_array())));
    let bvh = Bvh::new(&mesh);
    let mut settings = settings(32, &[BakedMap::Position, BakedMap::AmbientOcclusion]);
    settings.maps.ao.parameters.rays.rays = 4;
    settings.common.antialiasing = BakeAntialiasing::Subsampling2x2;
    settings.common.dilation = 4;
    settings.common.apply_diffusion = true;
    let request = BakeRequest {
        mesh: &mesh,
        bvh: &bvh,
        material: 0,
        low_base_color: &[],
        settings: &settings,
        high: None,
        cage: None,
        skew: None,
        cage_paint: None,
        tiles: (2, 1),
    };
    let full = bake(&request, &Progress::new()).unwrap();
    for excluded in [[(0, 0)], [(1, 0)]] {
        let partial = bake_tiles(&request, &Progress::new(), &excluded, None).unwrap();
        let strips = bake_in_strips(&request, &Progress::new(), 512, &CpuTracer, &excluded).unwrap();
        let fallback = bake_tiles(&request, &Progress::new(), &excluded, Some(&Broken)).unwrap();
        assert_eq!(strips.maps, partial.maps);
        assert_eq!(fallback.maps, partial.maps);
        assert!(fallback.tracer.contains("Broken unavailable"));

        for map in [BakedMap::Position, BakedMap::AmbientOcclusion] {
            let components = map.components();
            for (index, pixel) in partial.maps[&map].chunks_exact(components).enumerate() {
                if index % 64 / 32 == excluded[0].0 as usize {
                    assert_eq!(pixel, &map.neutral()[..components]);
                } else {
                    assert_eq!(pixel, &full.maps[&map][index * components..(index + 1) * components]);
                }
            }
        }
    }

    assert!(bake_tiles(&request, &Progress::new(), &[(2, 0)], None).is_err());
    assert!(bake_tiles(&request, &Progress::new(), &[(0, u32::MAX)], None).is_err());
}

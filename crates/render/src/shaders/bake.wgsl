// GPU mesh-map baking: the ray casts of `texturecraft_bake::tracing` (the CPU oracle) as compute
// shaders, concatenated after rays.wgsl. `transfer_main` finds the closest high-poly hit of each
// transfer ray; `hemisphere_main` casts a query's secondary rays (ambient occlusion and bent
// normal, or thickness) in the same directions as `Hemisphere::direction`, spread over `lanes`
// threads of a workgroup and summed in workgroup memory. Every buffer holds u32 words (floats as
// bits) so integer keys never pass through float registers.

const TAU: f32 = 6.28318530717959;
const UNIFORM: u32 = 1u;
const IGNORE_BACKFACE: u32 = 2u;
const SAME_KEY: u32 = 4u;
const WORKGROUP: u32 = 64u;

struct Params {
    count: u32,   // items in this dispatch
    offset: u32,  // first item
    kind: u32,    // hemisphere: 0 occlusion, 1 thickness
    rays: u32,
    dmax: f32,
    eps: f32,
    cos_max: f32,
    sin_max: f32,
    flags: u32,   // UNIFORM | IGNORE_BACKFACE | SAME_KEY
    root: u32,    // transfer: BVH root node of the high poly
    lanes: u32,   // hemisphere: threads per query, a power of two dividing the workgroup
    out: u32,     // first result row of this pass
    min_distance: f32,
    attenuation: u32,
    ground_height: f32,
    backface_mode: u32,
};

@group(0) @binding(0) var<uniform> P: Params;
// Triangle rows: w words (part key, 0, 0), last row (0, 0, 0, original index).
// Transfer: per ray (origin, tmax), (direction, part key or NONE).
// Hemisphere: per query (pos, rotation u), (normal, rotation v), (geometric normal, part key), (BVH root, 0, 0, 0).
@group(0) @binding(3) var<storage, read> items: array<vec4<u32>>;
// Transfer: (t, u, v, original triangle or NONE); hemisphere: (value, bent normal).
@group(0) @binding(4) var<storage, read_write> results: array<vec4<u32>>;

var<private> want_key: u32;
var<private> skip_back: bool;
var<private> backface_by_name: bool;

fn accept(i: u32, d: vec3<f32>) -> bool {
    if (want_key != NONE && tris[i * 4u].w != want_key) {
        return false;
    }
    if (skip_back || (backface_by_name && tris[i * 4u + 1u].w != 0u)) {
        let p0 = tri_p(i, 0u);
        return dot(cross(tri_p(i, 1u) - p0, tri_p(i, 2u) - p0), d) <= 0.0;
    }
    return true;
}

@compute @workgroup_size(64)
fn transfer_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= P.count) {
        return;
    }
    let k = P.offset + gid.x;
    let a = items[k * 2u];
    let b = items[k * 2u + 1u];
    want_key = b.w;
    skip_back = (P.flags & IGNORE_BACKFACE) != 0u;
    backface_by_name = false;
    let h = traverse_from(P.root, bitcast<vec3<f32>>(a.xyz), bitcast<vec3<f32>>(b.xyz), bitcast<f32>(a.w), false);
    var out = vec4<u32>(0u, 0u, 0u, NONE);
    if (h.idx != NONE) {
        out = vec4<u32>(bitcast<u32>(h.t), bitcast<u32>(h.u), bitcast<u32>(h.v), tris[h.idx * 4u + 3u].w);
    }
    results[P.out + k] = out;
}

// Per thread: (sum, bent normal) and ray count, reduced per query.
var<workgroup> red: array<vec4<f32>, WORKGROUP>;
var<workgroup> red_n: array<u32, WORKGROUP>;

@compute @workgroup_size(64)
fn hemisphere_main(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let lane = li % P.lanes;
    let item = wg.x * (WORKGROUP / P.lanes) + li / P.lanes;
    let valid = item < P.count;
    let k = P.offset + min(item, max(P.count, 1u) - 1u);
    let r0 = items[k * 4u];
    let r1 = items[k * 4u + 1u];
    let r2 = items[k * 4u + 2u];
    let root = items[k * 4u + 3u].x;
    let nrm = bitcast<vec3<f32>>(r1.xyz);
    let rot = vec2<f32>(bitcast<f32>(r0.w), bitcast<f32>(r1.w));
    let occlusion = P.kind == 0u;
    let axis = select(-nrm, nrm, occlusion);
    let geo = select(-1.0, 1.0, occlusion) * bitcast<vec3<f32>>(r2.xyz);
    let o = bitcast<vec3<f32>>(r0.xyz) + geo * P.eps;
    let t = onb_t(axis);
    let b = onb_b(axis);
    want_key = select(NONE, r2.w, (P.flags & SAME_KEY) != 0u);
    skip_back = P.backface_mode == 1u;
    backface_by_name = P.backface_mode == 2u;
    var sum = 0.0;
    var bent = vec3<f32>(0.0);
    var n = 0u;
    for (var i = lane; valid && i < P.rays; i += P.lanes) {
        let u1 = fract(f32(i) / f32(P.rays) + rot.x);
        let u2 = fract(f32(reverseBits(i)) / 4294967296.0 + rot.y);
        var r: f32;
        var z: f32;
        if ((P.flags & UNIFORM) != 0u) {
            z = 1.0 - u2 * (1.0 - P.cos_max);
            r = sqrt(max(1.0 - z * z, 0.0));
        } else {
            r = P.sin_max * sqrt(u2);
            z = sqrt(max(1.0 - r * r, 0.0));
        }
        let phi = TAU * u1;
        let d = t * (r * cos(phi)) + b * (r * sin(phi)) + axis * z;
        if (dot(d, geo) <= 0.0) {
            continue;
        }
        let h = traverse_from(root, o + d * P.min_distance, d, max(0.0, P.dmax - P.min_distance), false);
        var hit = h.idx != NONE;
        var distance = select(P.dmax, h.t + P.min_distance, hit);
        if ((P.flags & 8u) != 0u && d.y < -1e-8) {
            let ground_distance = (P.ground_height - o.y) / d.y;
            if (ground_distance >= P.min_distance && ground_distance <= P.dmax) {
                distance = min(distance, ground_distance);
                hit = true;
            }
        }
        if (occlusion) {
            let fraction = clamp((distance - P.min_distance) / max(P.dmax - P.min_distance, 1.1920929e-7), 0.0, 1.0);
            var attenuation = 1.0;
            if (P.attenuation == 1u) { attenuation = 1.0 - fraction; }
            if (P.attenuation == 2u) { attenuation = 1.0 - fraction * fraction * (3.0 - 2.0 * fraction); }
            let v = select(0.0, attenuation, hit);
            sum += v;
            bent += d * (1.0 - v);
        } else {
            sum += distance;
        }
        n += 1u;
    }
    red[li] = vec4<f32>(sum, bent);
    red_n[li] = n;
    for (var s = P.lanes / 2u; s > 0u; s /= 2u) {
        workgroupBarrier();
        if (lane < s) {
            red[li] += red[li + s];
            red_n[li] += red_n[li + s];
        }
    }
    if (!valid || lane != 0u) {
        return;
    }
    sum = red[li].x;
    bent = red[li].yzw;
    n = red_n[li];
    var out = vec4<f32>(1.0, 0.0, 0.0, 0.0);
    if (occlusion) {
        let l2 = dot(bent, bent);
        let bn = select(nrm, bent * inverseSqrt(l2), l2 > 1e-30);
        out = vec4<f32>(select(1.0, clamp(1.0 - sum / f32(n), 0.0, 1.0), n > 0u), bn);
    } else {
        out.x = select(P.dmax, sum / f32(n), n > 0u);
    }
    results[P.out + k] = bitcast<vec4<u32>>(out);
}

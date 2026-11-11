// Bake mode's 3D view: the high poly, the cage around the low poly and the UV seams

struct BakeViewOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    // 1 where the bake's rays miss the high poly, 0 where they reach it
    @location(2) missed: f32,
};

const HIGH_POLY_COLOR: vec3<f32> = vec3<f32>(0.10, 0.36, 0.56);
const MISSED_COLOR: vec3<f32> = vec3<f32>(0.80, 0.01, 0.01);
const CAGE_COLOR: vec3<f32> = vec3<f32>(0.55, 0.42, 0.26);
const CAGE_OPACITY: f32 = 0.16;
const CAGE_WIRE_COLOR: vec4<f32> = vec4<f32>(0.75, 0.62, 0.42, 0.7);
const UV_SEAM_COLOR: vec4<f32> = vec4<f32>(0.20, 0.85, 0.30, 1.0);

// Brightness of surfaces seen edge-on; facing the camera adds the rest
const HEADLIGHT_AMBIENT: f32 = 0.3;

// Fractions of the way to the camera each vertex moves, so near-coincident surfaces and lines draw in a fixed order
const HIGH_POLY_PULL_TOWARD_EYE: f32 = 0.001;
const CAGE_WIRE_PULL_TOWARD_EYE: f32 = 0.002;

// Lit from the camera: surfaces facing it are brightest
fn headlight(color: vec3<f32>, normal: vec3<f32>, world_position: vec3<f32>) -> vec3<f32> {
    let facing = abs(dot(normal, normalize(g.eye.xyz - world_position)));
    return color * (HEADLIGHT_AMBIENT + (1.0 - HEADLIGHT_AMBIENT) * facing);
}

fn pulled_toward_eye(world_position: vec3<f32>, fraction: f32) -> vec4<f32> {
    return g.view_proj * vec4<f32>(world_position + (g.eye.xyz - world_position) * fraction, 1.0);
}

// Pulled slightly toward the camera: where the high poly nearly touches the low poly it shows instead of flickering with it
@vertex
fn vs_bake_high(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(4) missed: f32) -> BakeViewOut {
    var out: BakeViewOut;
    out.clip = pulled_toward_eye(position, HIGH_POLY_PULL_TOWARD_EYE);
    out.world_position = position;
    out.normal = normal;
    out.missed = missed;
    return out;
}

@fragment
fn fs_bake_high(surface: BakeViewOut) -> @location(0) vec4<f32> {
    let color = mix(HIGH_POLY_COLOR, MISSED_COLOR, surface.missed);
    return vec4<f32>(headlight(color, normalize(surface.normal), surface.world_position), 1.0);
}

@vertex
fn vs_bake_cage(@location(0) position: vec3<f32>) -> BakeViewOut {
    var out: BakeViewOut;
    out.clip = g.view_proj * vec4<f32>(position, 1.0);
    out.world_position = position;
    out.normal = vec3<f32>(0.0, 1.0, 0.0);
    out.missed = 0.0;
    return out;
}

// The cage's own faces: their normal comes from the screen-space change of the position
@fragment
fn fs_bake_cage(surface: BakeViewOut) -> @location(0) vec4<f32> {
    let face_normal = normalize(cross(dpdx(surface.world_position), dpdy(surface.world_position)));
    return vec4<f32>(headlight(CAGE_COLOR, face_normal, surface.world_position), CAGE_OPACITY);
}

// Pulled toward the camera like the 3D wireframe, so the lines win against the cage's faces
@vertex
fn vs_bake_cage_wire(@location(0) position: vec3<f32>) -> BakeViewOut {
    var out: BakeViewOut;
    out.clip = pulled_toward_eye(position, CAGE_WIRE_PULL_TOWARD_EYE);
    out.world_position = position;
    out.normal = vec3<f32>(0.0, 1.0, 0.0);
    out.missed = 0.0;
    return out;
}

@fragment
fn fs_wire_bake_cage(surface: BakeViewOut) -> @location(0) vec4<f32> {
    return CAGE_WIRE_COLOR;
}

@fragment
fn fs_wire_uv_seam(surface: VOut) -> @location(0) vec4<f32> {
    return UV_SEAM_COLOR;
}

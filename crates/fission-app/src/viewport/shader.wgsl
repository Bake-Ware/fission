struct Globals {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    // ids[0].x: hovered face id; the other 15 slots: selected face ids (0 = none)
    ids: array<vec4<u32>, 4>,
    // x: bitmask of bodies (by display index, < 32) to tint as selected
    bodies: vec4<u32>,
    // Section plane: keep where dot(xyz, p) <= w; off when xyz is zero.
    clip: vec4<f32>,
    // Background gradient; theme.x is 1 in dark mode.
    bg_top: vec4<f32>,
    bg_bottom: vec4<f32>,
    theme: vec4<f32>,
    // Per display body (< 32): appearance color, used when alpha > 0.
    colors: array<vec4<f32>, 32>,
};

fn is_clipped(p: vec3<f32>) -> bool {
    return dot(globals.clip.xyz, globals.clip.xyz) > 0.0 && dot(globals.clip.xyz, p) > globals.clip.w;
}

fn is_selected(id: u32) -> bool {
    if (id == 0u) {
        return false;
    }
    for (var i = 1u; i < 16u; i = i + 1u) {
        if (globals.ids[i / 4u][i % 4u] == id) {
            return true;
        }
    }
    return false;
}

@group(0) @binding(0) var<uniform> globals: Globals;

// ---- background: vertical gradient, drawn as one oversized triangle ----

struct BgOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) t: f32,
};

@vertex
fn bg_vs(@builtin(vertex_index) i: u32) -> BgOut {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: BgOut;
    out.pos = vec4<f32>(uv * 2.0 - 1.0, 1.0, 1.0);
    out.t = uv.y;
    return out;
}

@fragment
fn bg_fs(in: BgOut) -> @location(0) vec4<f32> {
    return vec4<f32>(mix(globals.bg_bottom.rgb, globals.bg_top.rgb, clamp(in.t, 0.0, 1.0)), 1.0);
}

// ---- shaded faces ----

struct FaceIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) id: u32,
};

struct FaceOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) @interpolate(flat) id: u32,
};

@vertex
fn face_vs(in: FaceIn) -> FaceOut {
    var out: FaceOut;
    out.clip = globals.view_proj * vec4<f32>(in.pos, 1.0);
    out.world = in.pos;
    out.normal = in.normal;
    out.id = in.id;
    return out;
}

@fragment
fn face_fs(in: FaceOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    if (is_clipped(in.world)) {
        discard;
    }
    let body = (in.id >> 16u) - 1u;
    var base = vec3<f32>(0.70, 0.73, 0.77);
    if (body < 32u && globals.colors[body].a > 0.0) {
        base = globals.colors[body].rgb;
    }
    // Section caps: through the cut we see solids from inside (back faces).
    // Fill them flat with diagonal hatching, like Fusion's section view.
    let sectioned = dot(globals.clip.xyz, globals.clip.xyz) > 0.0;
    if (sectioned && !front) {
        let stripe = fract((in.clip.x + in.clip.y) / 9.0) < 0.22;
        let cap = mix(base, vec3<f32>(0.85, 0.45, 0.20), 0.35) * 0.85;
        return vec4<f32>(select(cap, cap * 0.6, stripe), 1.0);
    }
    let view_dir = normalize(globals.eye.xyz - in.world);
    var n = normalize(in.normal);
    // Two-sided: tessellated normals can point either way per face.
    if (dot(n, view_dir) < 0.0) {
        n = -n;
    }

    // Headlight plus a fixed key light from above, like a CAD studio setup.
    let key = normalize(vec3<f32>(0.35, -0.45, 0.82));
    let diffuse = 0.55 * max(dot(n, view_dir), 0.0) + 0.25 * max(dot(n, key), 0.0);
    let half_vec = normalize(view_dir + key);
    let spec = 0.18 * pow(max(dot(n, half_vec), 0.0), 48.0);

    let body_picked = body < 32u && ((globals.bodies.x >> body) & 1u) == 1u;
    if (body_picked) {
        base = mix(base, vec3<f32>(0.30, 0.62, 0.95), 0.55);
    }
    if (is_selected(in.id)) {
        base = vec3<f32>(0.30, 0.62, 0.95);
    } else if (in.id == globals.ids[0].x && in.id != 0u) {
        base = mix(base, vec3<f32>(0.45, 0.72, 1.0), 0.45);
    }

    let color = base * (0.28 + diffuse) + vec3<f32>(spec);
    return vec4<f32>(color, 1.0);
}

// ---- lines: model edges, grid, axes ----

struct LineIn {
    @location(0) pos: vec3<f32>,
    @location(1) color: vec4<f32>,
};

struct LineOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) world: vec3<f32>,
};

@vertex
fn line_vs(in: LineIn) -> LineOut {
    var out: LineOut;
    out.clip = globals.view_proj * vec4<f32>(in.pos, 1.0);
    out.color = in.color;
    out.world = in.pos;
    return out;
}

@fragment
fn line_fs(in: LineOut) -> @location(0) vec4<f32> {
    // Model edges are the opaque lines; the grid and axes stay uncut.
    if (in.color.a > 0.99 && is_clipped(in.world)) {
        discard;
    }
    // Dark mode: the grid (faint, uncolored lines) turns light.
    if (globals.theme.x > 0.5 && in.color.a < 0.5) {
        return vec4<f32>(vec3<f32>(0.75, 0.78, 0.85), in.color.a * 0.8);
    }
    return in.color;
}

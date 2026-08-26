/// Render configuration parsed from the JSON dict the Typst wrapper passes in.
///
/// Uses `serde_json::Value` for parsing — we already pull `serde_json` in
/// transitively via `gltf-rs`, so this costs nothing extra and skips writing
/// a hand-rolled JSON parser like maquette's.

use serde_json::Value;

#[derive(Clone, Copy)]
pub struct SsaoCfg {
    pub samples: usize,
    pub radius: f64,
    pub bias: f64,
    pub strength: f64,
}

impl Default for SsaoCfg {
    fn default() -> Self {
        Self { samples: 16, radius: 0.5, bias: 0.025, strength: 1.0 }
    }
}

/// Analytical hemispheric image-based lighting. Replaces the constant `ambient`
/// with sky/ground gradient irradiance for diffuse and reflection-direction
/// sampling for specular. Karis's polynomial split-sum approximation combines
/// F0/roughness/N·V into (scale, bias) without needing a precomputed BRDF LUT.
/// Image-based lighting. Two modes:
///   * **Procedural** (default): sky/ground colour + intensity → analytical
///     hemispheric env with sun disc. Cheap, no external asset.
///   * **HDR photograph**: an .hdr file (Radiance RGBE) supplied as bytes.
///     Parsed at render start, projected equirectangular → octahedral.
///
/// The Typst caller picks between them via `ibl: (sky/ground/…)` or
/// `ibl: (hdr: read("env.hdr", encoding: none))`.
#[derive(Clone)]
pub struct IblCfg {
    /// Linear RGB colour above the horizon (procedural mode).
    pub sky: [f32; 3],
    /// Linear RGB colour below the horizon (procedural mode).
    pub ground: [f32; 3],
    /// Overall brightness multiplier — applies to both modes.
    pub intensity: f32,
    /// Optional Radiance RGBE bytes. When present, supersedes sky/ground and
    /// samples a real captured environment. `intensity` still applies.
    pub hdr_bytes: Option<Vec<u8>>,
    /// Rotation around the up axis, in radians. Rotates the HDR (or procedural
    /// sun) around the model — lets the user aim highlights without repositioning
    /// the light. 0 = HDR's canonical orientation.
    pub rotation: f32,
}

impl Default for IblCfg {
    fn default() -> Self {
        Self {
            sky:    [0.65, 0.75, 0.95],  // pale sky blue
            ground: [0.20, 0.15, 0.10],  // warm earth
            intensity: 1.0,
            hdr_bytes: None,
            rotation: 0.0,
        }
    }
}

/// Shadow-mapping knobs. Per-light depth map, PCF filtered at shade time.
#[derive(Clone, Copy)]
pub struct ShadowCfg {
    /// Depth map side length (square). 256 = fast/blocky, 1024 = crisp/slow.
    pub resolution: usize,
    /// PCF kernel radius in texels. 0 = hard shadow (1 tap), 1 = 3×3 (9 taps),
    /// 2 = 5×5 (25 taps). Higher = softer, more expensive per pixel.
    pub softness: usize,
    /// Depth-comparison bias. Small positive value avoids self-shadowing acne.
    pub bias: f32,
    /// Normal-offset multiplier in texel-world units. Pushes the receiver
    /// sample point along its normal toward the light before comparison.
    pub normal_bias: f32,
    /// Slope-scaled bias: extra bias proportional to `tan(θ)` where θ is the
    /// angle between light dir and normal. Handles grazing-angle acne.
    pub slope_bias: f32,
    /// Contact-hardening PCSS. `0.0` = disabled (plain PCF). Positive =
    /// emitter's world-space size for the penumbra estimate. Common values:
    /// `0.05` for small light source, `0.3` for a soft area light.
    pub pcss_light_size: f32,
}

/// Shadow-receiving ground plane. Uses a matte medium-roughness dielectric
/// material so it visibly picks up cast shadows without competing with the
/// subject for attention.
#[derive(Clone, Copy)]
pub struct GroundCfg {
    /// Linear RGB tint. Default a neutral cool gray.
    pub color: [f32; 3],
    /// Half-side of the square, as a multiplier of the model's bbox radius.
    /// Larger = shadow projects across more ground; too large means the
    /// directional shadow-map ortho frustum can't cover it. Default 3.0.
    pub size_scale: f32,
    /// Y position in world space (up axis assumed). `None` = auto (uses
    /// `bbox_min.y`, i.e. just below the model).
    pub y: Option<f32>,
    /// Roughness of the ground material. 1.0 = fully matte, 0.3 = shiny
    /// (mirror-like reflections of the environment). Default 0.85.
    pub roughness: f32,
}

impl Default for GroundCfg {
    fn default() -> Self {
        Self {
            color: [0.28, 0.28, 0.30],
            size_scale: 3.0,
            y: None,
            roughness: 0.85,
        }
    }
}

impl Default for ShadowCfg {
    fn default() -> Self {
        Self {
            resolution: 512,
            softness: 1,
            bias: 0.001,
            normal_bias: 1.0,
            slope_bias: 2.0,
            pcss_light_size: 0.0,
        }
    }
}

#[derive(Clone)]
pub struct RenderConfig {
    // Viewport
    pub width: usize,
    pub height: usize,
    pub background: String,

    // Camera
    pub camera: Option<[f64; 3]>,
    pub center: [f64; 3],
    pub up: [f64; 3],
    pub azimuth: f64,
    pub elevation: f64,
    pub distance: Option<f64>,
    pub fov: f64,
    pub auto_center: bool,
    pub auto_fit: bool,
    /// Pick a glTF-authored camera by its `name` (glTF `camera.name` field).
    /// When resolved, overrides `camera` / `azimuth` / `elevation` / `fov`.
    pub camera_name: Option<String>,
    /// Pick a glTF-authored camera by its index in `document.cameras()`.
    /// Falls back to `camera_name` matching if that's also set (name wins).
    pub camera_index: Option<usize>,
    /// When neither `camera_name` nor `camera_index` is set (and the caller
    /// hasn't overridden framing via `camera`/`azimuth`/`elevation`/`distance`),
    /// auto-pick the first camera declared in the asset. Default true — matches
    /// mainstream viewer convention. Set false to force the orbit fallback.
    pub camera_auto_use: bool,
    /// glTF scene selector — index into `document.scenes()`. `None` picks the
    /// document's authored default scene (or scene 0 as a fallback). Only
    /// useful for assets that declare more than one scene as switchable roots.
    pub scene_index: Option<usize>,
    /// Animation clip selector — index into `document.animations()`. `None`
    /// plays every clip stacked (last-write-wins per node channel). Assets
    /// typically ship separate clips (idle/walk/run/...); set this to pick one.
    pub animation_index: Option<usize>,

    // Shading
    pub light_dir: [f64; 3],
    pub ambient: f64,
    pub cull_backface: bool,

    /// Image-based lighting. `None` = falls back to the constant `ambient`
    /// scalar; `Some` = hemispheric sky/ground with Karis BRDF approximation.
    /// Biggest visual lift for metallic materials.
    pub ibl: Option<IblCfg>,

    /// Shadow maps. `None` = no shadow pass; `Some` = build a depth map
    /// per casting light and PCF-sample at shade time. Adds a full extra
    /// rasterization pass per light per render.
    pub shadows: Option<ShadowCfg>,

    /// Ground plane below the model that receives shadows. Anchors the
    /// subject in space — without it, renders float against the background.
    /// `None` = no ground; `Some` = two triangles at model bottom, sized to
    /// `bbox_radius * scale`.
    pub ground: Option<GroundCfg>,

    // Post-process
    /// SSAO: `false` to disable, `true` for defaults, or object with
    /// `{ samples, radius, bias, strength }` overrides.
    pub ssao: Option<SsaoCfg>,
    /// Supersample anti-aliasing factor. 1 = off (default), 2 = render at
    /// 2× each dim and downsample (4 subpixels), 4 = 4× (16 subpixels).
    /// Combines well with FXAA — SSAA cleans the edges, FXAA smooths what
    /// remains. Cost scales with factor².
    pub antialias: usize,
    /// FXAA edge anti-aliasing on the final RGB buffer.
    pub fxaa: bool,
    /// Tone-mapping operator: `""` = none, `"reinhard"`, `"aces"`.
    pub tone_mapping: String,
    /// Exposure multiplier applied inside the tone-mapping operator (no
    /// effect when `tone_mapping = ""`).
    pub exposure: f64,

    // Texture-load knobs (see docs on decode cost: JPEG in wasm is ~1 MB/s,
    // so a 3 MB glTF can cost 3 s cold-decode).
    /// Skip all texture decoding — draft/preview mode. All textures fall back
    /// to their material factors. Sub-100 ms renders regardless of asset.
    pub no_textures: bool,
    /// Cap max side of each decoded texture; downsample after decode when
    /// larger. Bounds memory + per-pixel sampling cost. Doesn't speed decode
    /// (zune-jpeg has no scaled decode), but pairs with mipmap LOD selection
    /// to keep sampling cheap. `None` = keep source resolution.
    pub texture_max_size: Option<u32>,

    /// Animation playback time, in seconds. `0.0` = t=0 pose (typically the
    /// "rest" state). Every distinct value invalidates the scene cache, so
    /// rendering N frames means N cache misses.
    pub time: f64,

    /// KHR_materials_variants: which variant index to use for primitives that
    /// declare mappings. Defaults to 0 (first variant, or the primitive's
    /// baked material when no mapping matches).
    pub material_variant: u32,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            width: 500,
            height: 500,
            background: "#f0f0f0".to_string(),
            camera: None,
            center: [0.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0], // glTF is Y-up by default (unlike STL/OBJ/CAD → Z-up in maquette)
            azimuth: 30.0,
            elevation: 20.0,
            distance: None,
            fov: 45.0,
            auto_center: true,
            auto_fit: true,
            camera_name: None,
            camera_index: None,
            camera_auto_use: true,
            scene_index: None,
            animation_index: None,
            light_dir: [1.0, 2.0, 3.0],
            ambient: 0.2,
            cull_backface: true,
            ibl: None,
            shadows: None,
            ground: None,
            ssao: None,
            antialias: 1,
            fxaa: false,
            tone_mapping: String::new(),
            exposure: 1.0,
            no_textures: false,
            texture_max_size: None,
            time: 0.0,
            material_variant: 0,
        }
    }
}

pub fn parse(json_bytes: &[u8]) -> Result<RenderConfig, String> {
    if json_bytes.is_empty() {
        return Ok(RenderConfig::default());
    }
    let value: Value = serde_json::from_slice(json_bytes)
        .map_err(|e| format!("config JSON: {}", e))?;
    let Value::Object(map) = value else {
        return Err("config must be a JSON object".into());
    };

    let mut cfg = RenderConfig::default();
    for (key, v) in map.iter() {
        match key.as_str() {
            "width"         => if let Some(n) = as_usize(v) { cfg.width = n.max(1); }
            "height"        => if let Some(n) = as_usize(v) { cfg.height = n.max(1); }
            "background"    => if let Some(s) = v.as_str()  { cfg.background = s.to_string(); }
            "camera"        => cfg.camera        = as_vec3(v),
            "center"        => if let Some(a) = as_vec3(v) { cfg.center = a; }
            "up"            => if let Some(a) = as_vec3(v) { cfg.up = a; }
            "azimuth"       => if let Some(f) = v.as_f64() { cfg.azimuth = f; }
            "elevation"     => if let Some(f) = v.as_f64() { cfg.elevation = f; }
            "distance"      => cfg.distance      = v.as_f64().filter(|&d| d > 0.0),
            "fov"           => if let Some(f) = v.as_f64() { cfg.fov = f.clamp(1.0, 179.0); }
            "auto_center"   => if let Some(b) = v.as_bool() { cfg.auto_center = b; }
            "auto_fit"      => if let Some(b) = v.as_bool() { cfg.auto_fit = b; }
            "camera_name"   => if let Some(s) = v.as_str()  { cfg.camera_name = Some(s.to_string()); }
            "camera_index"  => if let Some(n) = as_usize(v) { cfg.camera_index = Some(n); }
            "camera_auto_use" => if let Some(b) = v.as_bool() { cfg.camera_auto_use = b; }
            "scene_index"   => if let Some(n) = as_usize(v) { cfg.scene_index = Some(n); }
            "animation_index" => if let Some(n) = as_usize(v) { cfg.animation_index = Some(n); }
            "light_dir"     => if let Some(a) = as_vec3(v) { cfg.light_dir = a; }
            "ambient"       => if let Some(f) = v.as_f64() { cfg.ambient = f.clamp(0.0, 1.0); }
            "cull_backface" => if let Some(b) = v.as_bool() { cfg.cull_backface = b; }
            "ibl"           => cfg.ibl = parse_ibl(v),
            "shadows"       => cfg.shadows = parse_shadows(v),
            "ground"        => cfg.ground = parse_ground(v),
            "ssao"          => cfg.ssao = parse_ssao(v),
            "antialias"     => if let Some(n) = as_usize(v) { cfg.antialias = n.clamp(1, 4); }
            "fxaa"          => if let Some(b) = v.as_bool() { cfg.fxaa = b; }
            "tone_mapping"  => if let Some(s) = v.as_str()  { cfg.tone_mapping = s.to_string(); }
            "exposure"      => if let Some(f) = v.as_f64() { cfg.exposure = f.max(0.0); }
            "no_textures"   => if let Some(b) = v.as_bool() { cfg.no_textures = b; }
            "texture_max_size" => cfg.texture_max_size = as_usize(v).map(|n| n as u32).filter(|&n| n >= 4),
            "time"          => if let Some(f) = v.as_f64() { cfg.time = f.max(0.0); }
            "material_variant" => if let Some(n) = as_usize(v) { cfg.material_variant = n as u32; }
            _ => {} // Unknown keys silently ignored, forward-compatible with v2.
        }
    }
    Ok(cfg)
}

fn as_usize(v: &Value) -> Option<usize> {
    v.as_u64().map(|n| n as usize)
        .or_else(|| v.as_f64().map(|n| n.max(0.0) as usize))
}

fn parse_ibl(v: &Value) -> Option<IblCfg> {
    match v {
        Value::Bool(false) => None,
        Value::Bool(true) => Some(IblCfg::default()),
        Value::Object(o) => {
            let mut c = IblCfg::default();
            if let Some(s) = o.get("sky").and_then(|x| x.as_str()) {
                let (r, g, b) = maquette_core::color::parse_hex_color(s);
                c.sky = [maquette_core::color::srgb_to_linear(r), maquette_core::color::srgb_to_linear(g), maquette_core::color::srgb_to_linear(b)];
            }
            if let Some(s) = o.get("ground").and_then(|x| x.as_str()) {
                let (r, g, b) = maquette_core::color::parse_hex_color(s);
                c.ground = [maquette_core::color::srgb_to_linear(r), maquette_core::color::srgb_to_linear(g), maquette_core::color::srgb_to_linear(b)];
            }
            if let Some(f) = o.get("intensity").and_then(|x| x.as_f64()) {
                c.intensity = (f as f32).max(0.0);
            }
            if let Some(f) = o.get("rotation").and_then(|x| x.as_f64()) {
                c.rotation = f as f32;
            }
            // HDR bytes come from Typst's `read(..., encoding: none)`, encoded
            // as an array of integers in JSON. Copy them out to owned Vec.
            if let Some(arr) = o.get("hdr").and_then(|x| x.as_array()) {
                let mut bytes = Vec::with_capacity(arr.len());
                for v in arr {
                    if let Some(n) = v.as_u64() {
                        bytes.push((n & 0xff) as u8);
                    }
                }
                if !bytes.is_empty() {
                    c.hdr_bytes = Some(bytes);
                }
            }
            Some(c)
        }
        _ => None,
    }
}

fn parse_ground(v: &Value) -> Option<GroundCfg> {
    match v {
        Value::Bool(false) => None,
        Value::Bool(true) => Some(GroundCfg::default()),
        Value::Object(o) => {
            let mut c = GroundCfg::default();
            if let Some(s) = o.get("color").and_then(|x| x.as_str()) {
                let (r, g, b) = maquette_core::color::parse_hex_color(s);
                c.color = [
                    maquette_core::color::srgb_to_linear(r),
                    maquette_core::color::srgb_to_linear(g),
                    maquette_core::color::srgb_to_linear(b),
                ];
            }
            if let Some(f) = o.get("size_scale").and_then(|x| x.as_f64()) {
                c.size_scale = (f as f32).clamp(0.5, 20.0);
            }
            if let Some(f) = o.get("y").and_then(|x| x.as_f64()) {
                c.y = Some(f as f32);
            }
            if let Some(f) = o.get("roughness").and_then(|x| x.as_f64()) {
                c.roughness = (f as f32).clamp(0.05, 1.0);
            }
            Some(c)
        }
        _ => None,
    }
}

fn parse_shadows(v: &Value) -> Option<ShadowCfg> {
    match v {
        Value::Bool(false) => None,
        Value::Bool(true) => Some(ShadowCfg::default()),
        Value::Object(o) => {
            let mut c = ShadowCfg::default();
            if let Some(n) = o.get("resolution").and_then(|x| x.as_u64()) {
                c.resolution = (n as usize).clamp(64, 4096);
            }
            if let Some(n) = o.get("softness").and_then(|x| x.as_u64()) {
                c.softness = (n as usize).clamp(0, 4);
            }
            if let Some(f) = o.get("bias").and_then(|x| x.as_f64()) { c.bias = f as f32; }
            if let Some(f) = o.get("normal_bias").and_then(|x| x.as_f64()) { c.normal_bias = f as f32; }
            if let Some(f) = o.get("slope_bias").and_then(|x| x.as_f64()) { c.slope_bias = f as f32; }
            if let Some(f) = o.get("pcss_light_size").and_then(|x| x.as_f64()) { c.pcss_light_size = f as f32; }
            Some(c)
        }
        _ => None,
    }
}

fn parse_ssao(v: &Value) -> Option<SsaoCfg> {
    match v {
        Value::Bool(false) => None,
        Value::Bool(true) => Some(SsaoCfg::default()),
        Value::Object(o) => {
            let mut c = SsaoCfg::default();
            if let Some(n) = o.get("samples").and_then(|x| x.as_u64()) { c.samples = (n as usize).clamp(4, 64); }
            if let Some(f) = o.get("radius").and_then(|x| x.as_f64())  { c.radius = f.clamp(0.01, 2.0); }
            if let Some(f) = o.get("bias").and_then(|x| x.as_f64())    { c.bias = f.max(0.0); }
            if let Some(f) = o.get("strength").and_then(|x| x.as_f64()) { c.strength = f.clamp(0.0, 2.0); }
            Some(c)
        }
        _ => None,
    }
}

fn as_vec3(v: &Value) -> Option<[f64; 3]> {
    let arr = v.as_array()?;
    if arr.len() < 3 { return None; }
    Some([
        arr[0].as_f64()?,
        arr[1].as_f64()?,
        arr[2].as_f64()?,
    ])
}

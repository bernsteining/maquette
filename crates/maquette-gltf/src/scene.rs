/// Flatten a glTF scene graph into a flat list of world-space triangles with
/// per-vertex attributes (position, normal, UV) + a material handle.
///
/// Traversal is **iterative** with an explicit stack, not recursive: wasmi's
/// wasm-stack budget in Typst is small and deep node hierarchies (skinned
/// characters, scientific viz with thousands of parts) would blow it.
///
/// UVs are stored but ignored until texture sampling lands in phase 2; the
/// slot is here now so materials, rasterizer, and shader all agree on the
/// vertex layout before we start wiring the sampler.

use crate::gltf_loader::LoadedGltf;
use maquette_core::math::{Mat3, Mat4, Vec3};
use maquette_core::texture::{build_mips, Filter, MipLevel, Texture, Wrap};
use maquette_core::texture_decode;

#[derive(Clone, Copy)]
pub struct Vertex {
    /// World-space position.
    pub position: Vec3,
    /// World-space unit normal.
    pub normal: Vec3,
    /// TEXCOORD_0. Zero-filled when the primitive has no UV set.
    pub uv: [f32; 2],
    /// TEXCOORD_1. Zero-filled when the primitive has no secondary UV set.
    /// Materials pick per-slot which coord set to sample via `texcoord_*` fields.
    pub uv1: [f32; 2],
    /// COLOR_0 (linear-space RGBA). glTF spec: multiplied component-wise with
    /// baseColor. `[1, 1, 1, 1]` when the primitive has no color set — the
    /// identity for the shader's multiply.
    pub color: [f32; 4],
    /// World-space tangent (xyz) + bitangent handedness (w = ±1). Per glTF
    /// spec, TANGENT is `[T.x, T.y, T.z, w]`. When the primitive ships no
    /// TANGENT, we compute per-triangle from UV derivatives (all three
    /// vertices of a triangle share the same tangent — visually equivalent
    /// to flat tangent shading and good enough for most normal maps).
    pub tangent: [f32; 4],
}

#[derive(Clone, Copy)]
pub struct Triangle {
    pub vertices: [Vertex; 3],
    /// Index into `Scene::materials`.
    pub material_id: u32,
}

/// Line primitive (from `LINES`/`LINE_STRIP`/`LINE_LOOP` mode). Two world-space
/// endpoints + material. Rendered as an unlit, single-pixel-wide screen-space
/// line — points and lines don't have BRDFs in glTF's spec so we just use the
/// material's base color × vertex color if present.
#[derive(Clone, Copy)]
pub struct LinePrim {
    pub a: Vertex,
    pub b: Vertex,
    pub material_id: u32,
}

/// Point primitive (from `POINTS` mode). Rendered as a 1×1 pixel splat at the
/// projected vertex position, z-buffered.
#[derive(Clone, Copy)]
pub struct PointPrim {
    pub p: Vertex,
    pub material_id: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AlphaMode {
    Opaque,
    /// Discard fragments whose alpha is below the cutoff.
    Mask,
    /// Straight alpha blending. Rasterizer doesn't sort — for MVP we treat as
    /// opaque and rely on z-buffer, which is wrong for translucency but keeps
    /// output well-defined until we add a sort/OIT pass.
    Blend,
}

#[derive(Clone)]
pub struct Material {
    /// glTF `pbrMetallicRoughness.baseColorFactor` (linear RGBA). Multiplied
    /// with `base_color_texture` sample when the latter is present.
    pub base_color: [f32; 4],
    /// glTF `metallicFactor` [0, 1]. Multiplied with texture B channel.
    pub metallic: f32,
    /// glTF `roughnessFactor` [0, 1]. Multiplied with texture G channel.
    pub roughness: f32,
    /// glTF `emissiveFactor` (linear RGB). Multiplied with texture sample.
    pub emissive: [f32; 3],
    /// glTF `doubleSided`. When true the rasterizer skips backface culling for
    /// this material and lights the back face with the flipped normal.
    pub double_sided: bool,
    pub alpha_mode: AlphaMode,
    /// `alphaCutoff` — meaningful only for `AlphaMode::Mask`.
    pub alpha_cutoff: f32,
    /// KHR_materials_unlit: skip all lighting, output base color directly.
    pub unlit: bool,

    // Texture indices into `Scene::textures`. Populated when present in the
    // glTF material; `None` means "use the factor only".
    pub base_color_texture: Option<u32>,
    pub metallic_roughness_texture: Option<u32>,
    pub normal_texture: Option<u32>,
    pub occlusion_texture: Option<u32>,
    pub emissive_texture: Option<u32>,
    // TEXCOORD_N selector per texture slot (0 or 1). Defaults to 0. glTF spec
    // allows either UV set per texture_info via `texCoord: 1`.
    pub texcoord_base:              u8,
    pub texcoord_mr:                u8,
    pub texcoord_normal:            u8,
    pub texcoord_occlusion:         u8,
    pub texcoord_emissive:          u8,
    pub texcoord_clearcoat_normal:  u8,
    pub texcoord_transmission:      u8,
    /// Normal-map strength: sampled normal is `(sample · 2 − 1) · scale`
    /// before renormalisation. glTF default 1.0.
    pub normal_scale: f32,
    /// Occlusion strength: `ao = mix(1, sampled_ao, strength)`. Default 1.0.
    pub occlusion_strength: f32,

    // KHR_materials_clearcoat — thin dielectric layer over the base BRDF.
    // `factor = 0` disables the lobe (no perf cost). Clearcoat normal
    // texture is optional; when absent, clearcoat uses the geometric normal
    // (spec-correct — clearcoat does *not* inherit the base normal-map
    // perturbation).
    pub clearcoat_factor: f32,
    pub clearcoat_roughness: f32,
    pub clearcoat_normal_texture: Option<u32>,
    pub clearcoat_normal_scale: f32,

    // KHR_materials_sheen — fabric-style backscatter (Charlie NDF + Neubelt V).
    // `sheen_color = [0,0,0]` disables the lobe.
    pub sheen_color: [f32; 3],
    pub sheen_roughness: f32,

    // KHR_materials_ior — dielectric refractive index. Default 1.5 gives the
    // conventional F0 ≈ 0.04.
    pub ior: f32,
    // KHR_materials_specular — tint + scale for dielectric F0. Metals ignore
    // both (their F0 is base color).
    pub specular_factor: f32,
    pub specular_color: [f32; 3],

    // KHR_texture_transform — per-texture-info UV transform. Identity when
    // the extension isn't declared on that texture_info. One slot per texture.
    pub xform_base:             TextureTransform,
    pub xform_mr:               TextureTransform,
    pub xform_normal:           TextureTransform,
    pub xform_occlusion:        TextureTransform,
    pub xform_emissive:         TextureTransform,
    pub xform_clearcoat_normal: TextureTransform,
    pub xform_transmission:     TextureTransform,

    // KHR_materials_transmission — thin-walled dielectric transmission. When
    // > 0, part of the light passes through the surface, sampled from the IBL
    // env (or ambient fallback) at the refraction direction. Factor 0 = opaque
    // (no perf cost). Tinted by base_color, attenuated per KHR_materials_volume.
    pub transmission_factor:  f32,
    pub transmission_texture: Option<u32>,

    // KHR_materials_volume — attenuation of light traversing the medium.
    // `thickness_factor` gives the world-space path length (approximate; we
    // use it directly rather than a thickness texture). Beer-Lambert:
    // `T = exp(-thickness / attenuation_distance · -ln(attenuation_color))`.
    pub thickness_factor:      f32,
    pub attenuation_color:     [f32; 3],
    pub attenuation_distance:  f32,

    // KHR_materials_iridescence — thin-film interference on top of the
    // dielectric surface. Modulates F0 with a wavelength-dependent Fresnel
    // computed from film IOR + thickness. Zero factor = disabled (no perf
    // cost).
    pub iridescence_factor:            f32,
    pub iridescence_ior:               f32,
    pub iridescence_thickness_min:     f32,
    pub iridescence_thickness_max:     f32,
    pub iridescence_texture:           Option<u32>,
    pub iridescence_thickness_texture: Option<u32>,
    pub texcoord_iridescence:          u8,
    pub texcoord_iridescence_thickness: u8,

    // KHR_materials_anisotropy — directional roughness (brushed metal, hair).
    // `strength` in [0, 1] controls how "stretched" the specular highlight
    // becomes. `rotation` (radians) rotates the tangent basis around N.
    // Texture RG stores 2D tangent direction (offset by 0.5, so unit vector
    // = 2*sample - 1), B stores per-pixel strength.
    pub anisotropy_strength: f32,
    pub anisotropy_rotation: f32,
    pub anisotropy_texture:  Option<u32>,
    pub texcoord_anisotropy: u8,

    // KHR_materials_dispersion — chromatic dispersion in transmissive materials.
    // Only meaningful when `transmission_factor > 0`. Splits the refractive
    // index across RGB: higher wavelength (red) refracts less, shorter
    // wavelength (blue) refracts more. Zero factor = no dispersion.
    pub dispersion: f32,

    // KHR_materials_diffuse_transmission — light diffusely transmitted through
    // the surface (thin cloth, backlit leaves, paper). Distinct from
    // `KHR_materials_transmission` which is refractive (glass). At the shader,
    // adds a `max(0, -N·L)` back-lit lambertian term tinted by
    // `diffuse_transmission_color`. Zero factor = disabled (no perf cost).
    pub diffuse_transmission_factor:  f32,
    pub diffuse_transmission_color:   [f32; 3],
    pub diffuse_transmission_texture: Option<u32>,
    pub diffuse_transmission_color_texture: Option<u32>,
    pub texcoord_diffuse_transmission:       u8,
    pub texcoord_diffuse_transmission_color: u8,
    pub xform_diffuse_transmission:       TextureTransform,
    pub xform_diffuse_transmission_color: TextureTransform,

    /// Values derived from the fields above that don't change per-triangle.
    /// Populated once at scene flatten and read by `MaterialShader::new` so
    /// per-triangle setup skips transcendentals (`.cos()`, `.sin()`, `.powf()`)
    /// and small arithmetic chains.
    pub precomp: MaterialPrecomp,
}

/// Per-material constants that would otherwise be recomputed on every
/// triangle inside `MaterialShader::new`. Cached alongside the material so
/// hot-loop setup is a handful of splats + loads instead of the full compute.
///
/// Only fields that need real work belong here — trivial `f32x4_splat(f)` of a
/// raw material field isn't worth caching (splat is one wasm op).
#[derive(Clone, Copy, Default)]
pub struct MaterialPrecomp {
    /// `[((n-1)/(n+1))² · specular_color[i] · specular_factor].min(1.0)`, 3 chans.
    pub dielectric_f0: [f32; 3],
    /// `1 / max(1e-4, sheen_roughness²)`.
    pub sheen_inv_alpha: f32,
    /// Beer-Lambert attenuation per channel: `color[i].powf(thickness / dist)`.
    pub volume_attenuation: [f32; 3],
    /// Reciprocals used by refraction/transmission and dispersion — bake the
    /// `1 / n` divide once per material.
    pub ior_ratio: f32,
    pub ior_ratio_r: f32,
    pub ior_ratio_b: f32,
    /// Anisotropy tangent rotation — `.cos()` / `.sin()` are expensive per-tri.
    pub anisotropy_cos_rot: f32,
    pub anisotropy_sin_rot: f32,
}

impl MaterialPrecomp {
    /// Compute the derived constants from a fully-populated `Material`. Called
    /// once at scene flatten, never on the hot path.
    pub fn from_material(m: &Material) -> Self {
        let n = m.ior;
        let x = (n - 1.0) / (n + 1.0);
        let x2 = x * x;
        let vol = |ch: usize| {
            if m.thickness_factor <= 0.0 || !m.attenuation_distance.is_finite() {
                1.0
            } else {
                let c = m.attenuation_color[ch].clamp(1e-6, 1.0);
                let t = m.thickness_factor / m.attenuation_distance.max(1e-6);
                c.powf(t)
            }
        };
        Self {
            dielectric_f0: [
                (x2 * m.specular_color[0] * m.specular_factor).min(1.0),
                (x2 * m.specular_color[1] * m.specular_factor).min(1.0),
                (x2 * m.specular_color[2] * m.specular_factor).min(1.0),
            ],
            sheen_inv_alpha: 1.0 / (m.sheen_roughness * m.sheen_roughness).max(1e-4),
            volume_attenuation: [vol(0), vol(1), vol(2)],
            ior_ratio:   1.0 / m.ior.max(1.0001),
            ior_ratio_r: 1.0 / (m.ior - 0.02 * m.dispersion).max(1.0001),
            ior_ratio_b: 1.0 / (m.ior + 0.02 * m.dispersion).max(1.0001),
            anisotropy_cos_rot: m.anisotropy_rotation.cos(),
            anisotropy_sin_rot: m.anisotropy_rotation.sin(),
        }
    }
}

/// KHR_texture_transform: `uv_new = R · (uv · scale) + offset` where `R`
/// is a 2D rotation matrix. Applied at sample time — no per-vertex UV
/// modification, so different textures on the same primitive can use
/// different transforms.
#[derive(Clone, Copy)]
pub struct TextureTransform {
    pub scale: [f32; 2],
    pub offset: [f32; 2],
    /// Precomputed `cos(rotation)` and `sin(rotation)` — rotation defined
    /// counter-clockwise per the extension spec.
    pub rot_cos: f32,
    pub rot_sin: f32,
}

impl TextureTransform {
    pub const IDENTITY: Self = Self {
        scale: [1.0, 1.0],
        offset: [0.0, 0.0],
        rot_cos: 1.0,
        rot_sin: 0.0,
    };

    /// Apply to a UV coordinate. Cheap: 4 muls + 4 adds per sample.
    #[inline]
    pub fn apply(&self, uv: [f32; 2]) -> [f32; 2] {
        let sx = uv[0] * self.scale[0];
        let sy = uv[1] * self.scale[1];
        [
             self.rot_cos * sx + self.rot_sin * sy + self.offset[0],
            -self.rot_sin * sx + self.rot_cos * sy + self.offset[1],
        ]
    }

    /// Whether this transform is the identity (allows the shader to skip
    /// the multiply-add chain when it doesn't matter).
    #[inline]
    pub fn is_identity(&self) -> bool {
        self.scale == [1.0, 1.0]
            && self.offset == [0.0, 0.0]
            && self.rot_cos == 1.0
            && self.rot_sin == 0.0
    }
}

impl Material {
    /// Refresh `self.precomp` from the current field values. Call after any
    /// caller-side field mutation (e.g. render.rs builds the ground material
    /// via `default_gltf()` + a handful of overrides — the resulting precomp
    /// needs to reflect the overrides, not the defaults).
    pub fn recompute_precomp(&mut self) {
        self.precomp = MaterialPrecomp::from_material(self);
    }

    /// glTF's default when a primitive has no material.
    pub fn default_gltf() -> Self {
        let mut m = Self {
            base_color: [1.0, 1.0, 1.0, 1.0],
            metallic: 1.0,
            roughness: 1.0,
            emissive: [0.0, 0.0, 0.0],
            double_sided: false,
            alpha_mode: AlphaMode::Opaque,
            alpha_cutoff: 0.5,
            unlit: false,
            base_color_texture: None,
            metallic_roughness_texture: None,
            normal_texture: None,
            occlusion_texture: None,
            emissive_texture: None,
            normal_scale: 1.0,
            occlusion_strength: 1.0,
            clearcoat_factor: 0.0,
            clearcoat_roughness: 0.0,
            clearcoat_normal_texture: None,
            clearcoat_normal_scale: 1.0,
            sheen_color: [0.0, 0.0, 0.0],
            sheen_roughness: 0.0,
            ior: 1.5,
            specular_factor: 1.0,
            specular_color: [1.0, 1.0, 1.0],
            xform_base:             TextureTransform::IDENTITY,
            xform_mr:               TextureTransform::IDENTITY,
            xform_normal:           TextureTransform::IDENTITY,
            xform_occlusion:        TextureTransform::IDENTITY,
            xform_emissive:         TextureTransform::IDENTITY,
            xform_clearcoat_normal: TextureTransform::IDENTITY,
            xform_transmission:     TextureTransform::IDENTITY,
            texcoord_base:              0,
            texcoord_mr:                0,
            texcoord_normal:            0,
            texcoord_occlusion:         0,
            texcoord_emissive:          0,
            texcoord_clearcoat_normal:  0,
            texcoord_transmission:      0,
            transmission_factor:  0.0,
            transmission_texture: None,
            thickness_factor:      0.0,
            attenuation_color:     [1.0, 1.0, 1.0],
            attenuation_distance:  f32::INFINITY,
            iridescence_factor:            0.0,
            iridescence_ior:               1.3,
            iridescence_thickness_min:     100.0,
            iridescence_thickness_max:     400.0,
            iridescence_texture:           None,
            iridescence_thickness_texture: None,
            texcoord_iridescence:          0,
            texcoord_iridescence_thickness: 0,
            anisotropy_strength: 0.0,
            anisotropy_rotation: 0.0,
            anisotropy_texture:  None,
            texcoord_anisotropy: 0,
            dispersion: 0.0,
            diffuse_transmission_factor:  0.0,
            diffuse_transmission_color:   [1.0, 1.0, 1.0],
            diffuse_transmission_texture: None,
            diffuse_transmission_color_texture: None,
            texcoord_diffuse_transmission:       0,
            texcoord_diffuse_transmission_color: 0,
            xform_diffuse_transmission:       TextureTransform::IDENTITY,
            xform_diffuse_transmission_color: TextureTransform::IDENTITY,
            precomp: MaterialPrecomp::default(),
        };
        m.recompute_precomp();
        m
    }
}

pub struct Scene {
    pub triangles: Vec<Triangle>,
    /// Line-mode primitives (POINTS excluded — see `points`).
    pub lines: Vec<LinePrim>,
    /// Point-mode primitives.
    pub points: Vec<PointPrim>,
    pub materials: Vec<Material>,
    /// Decoded textures, indexed from `Material::*_texture`. Owned by the
    /// leaky texture cache (`crate::cache`) — we hold a `'static` ref so
    /// re-flattening across animation frames doesn't re-decode.
    pub textures: &'static [Texture],
    /// KHR_lights_punctual — resolved to world space at scene flatten time.
    /// Empty when the glTF declares no lights; renderer falls back to the
    /// config's `light_dir` in that case.
    pub lights: Vec<PunctualLight>,
    /// glTF-authored cameras, resolved to world space. Empty when the file
    /// declares none; user config `camera_name` / `camera_index` picks from
    /// this list.
    pub cameras: Vec<SceneCamera>,
    pub bbox_min: Vec3,
    pub bbox_max: Vec3,
}

// `LightKind` and `PunctualLight` are re-exported from maquette-core so that
// shadow-map builder (which lives in core) can consume them without pulling
// in this crate. Struct layout / field docs live in `maquette_core::light`.
pub use maquette_core::light::{LightKind, PunctualLight};

/// A glTF-authored camera resolved to world space at scene flatten time.
/// Users can pick one by name or index via `camera_name` / `camera_index`
/// config keys — useful for showcasing model-author camera framings.
#[derive(Clone)]
pub struct SceneCamera {
    pub name: Option<String>,
    /// Camera position in world space.
    pub position: Vec3,
    /// Where the camera looks. Derived from `position + world.transform(-Z)`.
    pub target: Vec3,
    /// Camera up direction in world space (node's +Y axis).
    pub up: Vec3,
    /// Perspective vertical field of view in degrees. `None` → orthographic.
    pub fov_y_deg: Option<f64>,
    /// Orthographic half-height when non-perspective. Ignored for perspective.
    pub ortho_half_height: f64,
    /// Orthographic half-width when non-perspective. Ignored for perspective.
    pub ortho_half_width: f64,
    /// glTF near clip plane. `None` = the render's default (−1e−4).
    pub znear: Option<f64>,
    /// glTF far clip plane. `None` = unbounded (spec allows omission for
    /// perspective cameras — treated as infinite).
    pub zfar: Option<f64>,
}

impl Scene {
    fn empty() -> Self {
        Scene {
            triangles: Vec::new(),
            lines: Vec::new(),
            points: Vec::new(),
            materials: Vec::new(),
            textures: &[],
            lights: Vec::new(),
            cameras: Vec::new(),
            bbox_min: Vec3::new( f64::INFINITY,  f64::INFINITY,  f64::INFINITY),
            bbox_max: Vec3::new(-f64::INFINITY, -f64::INFINITY, -f64::INFINITY),
        }
    }

    #[inline]
    fn extend_bbox(&mut self, p: Vec3) {
        if p.x < self.bbox_min.x { self.bbox_min.x = p.x; }
        if p.y < self.bbox_min.y { self.bbox_min.y = p.y; }
        if p.z < self.bbox_min.z { self.bbox_min.z = p.z; }
        if p.x > self.bbox_max.x { self.bbox_max.x = p.x; }
        if p.y > self.bbox_max.y { self.bbox_max.y = p.y; }
        if p.z > self.bbox_max.z { self.bbox_max.z = p.z; }
    }

    pub fn bounds(&self) -> (Vec3, f64) {
        if self.triangles.is_empty() {
            return (Vec3::new(0.0, 0.0, 0.0), 0.0);
        }
        let center = Vec3::new(
            0.5 * (self.bbox_min.x + self.bbox_max.x),
            0.5 * (self.bbox_min.y + self.bbox_max.y),
            0.5 * (self.bbox_min.z + self.bbox_max.z),
        );
        let radius = 0.5 * (self.bbox_max - self.bbox_min).length();
        (center, radius.max(1e-6))
    }
}

/// Texture-load options driven by `RenderConfig`. Kept as a plain struct so
/// the cache key can hash it cheaply.
#[derive(Clone, Copy)]
pub struct TextureLoadOpts {
    pub disabled: bool,
    pub max_size: Option<u32>,
}

impl TextureLoadOpts {
    pub fn from_config(c: &crate::config::RenderConfig) -> Self {
        Self { disabled: c.no_textures, max_size: c.texture_max_size }
    }
}

/// Scene-wide options that affect the flattened output — includes texture
/// load knobs plus the animation time. Combined so the cache key can hash
/// everything in one shot.
#[derive(Clone, Copy)]
pub struct SceneOpts {
    pub textures: TextureLoadOpts,
    pub time: f32,
    /// KHR_materials_variants: which variant index to render. When a primitive
    /// has a `mappings` list, we look up the material assigned to this
    /// variant; if the variant isn't mapped, we fall back to the primitive's
    /// default material.
    pub variant: u32,
    /// Which glTF scene to render — an index into `document.scenes()`. When
    /// `None`, uses `document.default_scene()` (or scene 0 as a fallback).
    /// Assets that ship multiple scenes as switchable roots (rare but legal)
    /// need this to reach the non-default ones.
    pub scene_index: Option<usize>,
    /// Which animation clip to play — an index into `document.animations()`.
    /// glTF assets typically ship multiple clips (idle, walk, run, ...) as
    /// independent playable animations. `None` stacks every clip (last-write-
    /// wins per node channel) which is rarely what you want; `Some(i)` picks
    /// the i-th clip. Out-of-range indices fall back to the stacked path.
    pub animation_index: Option<usize>,
}

impl SceneOpts {
    pub fn from_config(c: &crate::config::RenderConfig) -> Self {
        Self {
            textures: TextureLoadOpts::from_config(c),
            time: c.time as f32,
            variant: c.material_variant,
            scene_index: c.scene_index,
            animation_index: c.animation_index,
        }
    }
}

/// Pulls textures from the leaky texture cache (see `cache::textures_for`).
/// Animation frames of the same asset share texture data via `&'static`
/// reference — zero clone, zero re-decode.
pub fn flatten_with_cached_textures(loaded: &LoadedGltf, opts: SceneOpts, bytes: &[u8]) -> Scene {
    let mut scene = Scene::empty();
    scene.textures = crate::cache::textures_for(bytes, loaded, opts.textures);
    scene.materials = collect_materials(loaded);
    let anim = sample_animations(loaded, opts.time, opts.animation_index);
    fill_scene(&mut scene, loaded, &anim, opts.variant, opts.scene_index);
    scene
}

/// Public entry to `collect_textures` for the leaky texture cache to call.
pub fn collect_textures_pub(loaded: &LoadedGltf, opts: TextureLoadOpts) -> Vec<Texture> {
    collect_textures(loaded, opts)
}

/// Resolve KHR_lights_punctual attachments after node transforms are known.
/// Point/Spot positions come from the node's world-space origin; Spot / Dir
/// use the node's world-space `-Z` as the light direction (glTF convention).
/// Walk nodes looking for `camera` attachments. glTF convention: camera
/// looks along its node's local -Z axis; up is local +Y.
fn collect_cameras(loaded: &LoadedGltf, world_transforms: &[Mat4]) -> Vec<SceneCamera> {
    let mut cameras = Vec::new();
    for node in loaded.document.nodes() {
        let Some(cam) = node.camera() else { continue; };
        let m = world_transforms[node.index()];
        let position = m.transform_point(Vec3::new(0.0, 0.0, 0.0));
        let forward  = m.transform_vector(Vec3::new(0.0, 0.0, -1.0)).normalized();
        let up       = m.transform_vector(Vec3::new(0.0, 1.0,  0.0)).normalized();
        let (fov_y_deg, ortho_half_height, ortho_half_width, znear, zfar) = match cam.projection() {
            gltf::camera::Projection::Perspective(p) => (
                Some((p.yfov() as f64).to_degrees()),
                0.0,
                0.0,
                Some(p.znear() as f64),
                p.zfar().map(|f| f as f64),
            ),
            gltf::camera::Projection::Orthographic(o) => (
                None,
                o.ymag() as f64,
                o.xmag() as f64,
                Some(o.znear() as f64),
                Some(o.zfar() as f64),
            ),
        };
        cameras.push(SceneCamera {
            name: cam.name().map(str::to_string),
            position,
            target: position.add(forward),
            up,
            fov_y_deg,
            ortho_half_height,
            ortho_half_width,
            znear,
            zfar,
        });
    }
    cameras
}

fn collect_lights(loaded: &LoadedGltf, world_transforms: &[Mat4]) -> Vec<PunctualLight> {
    let mut lights = Vec::new();
    for node in loaded.document.nodes() {
        let Some(light) = node.light() else { continue; };
        let m = world_transforms[node.index()];
        // World-space light origin: apply full transform to (0,0,0).
        let position = m.transform_point(Vec3::new(0.0, 0.0, 0.0));
        // Direction: node's local -Z axis in world space, normalised. The
        // linear part of the world matrix suffices (no translation).
        let direction = m.transform_vector(Vec3::new(0.0, 0.0, -1.0)).normalized();

        let color = light.color();
        let intensity = light.intensity();
        let scaled = [color[0] * intensity, color[1] * intensity, color[2] * intensity];

        let (kind, inner_cos, outer_cos) = match light.kind() {
            gltf::khr_lights_punctual::Kind::Directional => (LightKind::Directional, 0.0, 0.0),
            gltf::khr_lights_punctual::Kind::Point       => (LightKind::Point, 0.0, 0.0),
            gltf::khr_lights_punctual::Kind::Spot { inner_cone_angle, outer_cone_angle } => (
                LightKind::Spot,
                inner_cone_angle.cos(),
                outer_cone_angle.cos(),
            ),
        };

        lights.push(PunctualLight {
            kind,
            position,
            direction,
            color: scaled,
            range: light.range().unwrap_or(0.0),
            inner_cone_cos: inner_cos,
            outer_cone_cos: outer_cos,
            cast_shadow: true,
        });
    }
    lights
}

/// Same as `flatten` but skips texture decoding. Used by `get_gltf_info`
/// where nothing needs texel data — decoding ~4 MB of JPEG for a triangle
/// count is wasteful. Uses t=0 rest pose (bboxes at rest are the useful
/// case for camera framing anyway).
pub fn flatten_geometry_only(loaded: &LoadedGltf) -> Scene {
    let mut scene = Scene::empty();
    scene.materials = collect_materials(loaded);
    // `get_gltf_info` doesn't sample textures, but material paths may still
    // index into them — leak placeholder Vec once and reuse.
    scene.textures = placeholder_textures_for(loaded.document.textures().len());
    let anim = sample_animations(loaded, 0.0, None);
    fill_scene(&mut scene, loaded, &anim, 0, None);
    scene
}

fn fill_scene(scene: &mut Scene, loaded: &LoadedGltf, anim: &[AnimSample], variant: u32, scene_index: Option<usize>) {
    // Phase 1: resolve every node's world-space transform (needed up front
    // for skinning, which references joint node transforms possibly outside
    // the current mesh's parent chain).
    let world_transforms = compute_world_transforms(loaded, anim);

    // Phase 1.5: resolve KHR_lights_punctual attachments + glTF cameras.
    scene.lights = collect_lights(loaded, &world_transforms);
    scene.cameras = collect_cameras(loaded, &world_transforms);

    // Phase 2: emit meshes. Skinned meshes ignore their owning node's
    // transform per the glTF spec — only the joint palette matters.
    // Scene selection: explicit `scene_index` wins; otherwise the document's
    // authored default; otherwise the first scene. glTF assets can declare
    // multiple scenes as alternative composition roots — animations, cameras
    // and node hierarchies attach to nodes, but only nodes reachable from
    // the chosen root are rendered.
    let root_scene = scene_index
        .and_then(|i| loaded.document.scenes().nth(i))
        .or_else(|| loaded.document.default_scene())
        .or_else(|| loaded.document.scenes().next());
    let Some(root_scene) = root_scene else { return; };

    let mut stack: Vec<usize> = root_scene.nodes().map(|n| n.index()).collect();
    let mut seen = vec![false; loaded.document.nodes().count()];

    while let Some(node_index) = stack.pop() {
        if seen[node_index] { continue; }
        seen[node_index] = true;
        let node = loaded.document.nodes().nth(node_index).unwrap();

        if let Some(mesh) = node.mesh() {
            let palette = node.skin().map(|s| compute_joint_palette(loaded, &s, &world_transforms));
            let world = world_transforms[node_index];
            // Morph-target weights: animation-sampled if animated, else the
            // node's default weights, else the mesh's default weights.
            let node_weights = anim.get(node_index).and_then(|a| a.weights.clone())
                .or_else(|| node.weights().map(|w| w.to_vec()))
                .or_else(|| mesh.weights().map(|w| w.to_vec()));
            emit_mesh(scene, loaded, mesh, world, palette.as_deref(), node_weights.as_deref(), variant);
        }

        for child in node.children() {
            stack.push(child.index());
        }
    }
}

/// Resolve every node's world-space transform in one pass, applying animation
/// sample overrides at each node before composing with the parent chain.
fn compute_world_transforms(loaded: &LoadedGltf, anim: &[AnimSample]) -> Vec<Mat4> {
    let n = loaded.document.nodes().count();
    let mut world = vec![Mat4::identity(); n];
    let mut visited = vec![false; n];

    let scenes: Vec<_> = loaded.document.scenes().collect();
    for scene in &scenes {
        let mut stack: Vec<(usize, Mat4)> = scene.nodes().map(|n| (n.index(), Mat4::identity())).collect();
        while let Some((idx, parent)) = stack.pop() {
            if visited[idx] { continue; }
            visited[idx] = true;
            let node = loaded.document.nodes().nth(idx).unwrap();
            let local = node_transform_animated(&node, anim.get(idx));
            let w = parent.mul(local);
            world[idx] = w;
            for c in node.children() {
                stack.push((c.index(), w));
            }
        }
    }
    world
}

/// Build the joint matrix palette for a skin at the current animation state.
/// `palette[k]` = joint-space → world-space transform for joint k in
/// `skin.joints()` order, which is what JOINTS_0 vertex indices reference.
fn compute_joint_palette(loaded: &LoadedGltf, skin: &gltf::Skin, world_transforms: &[Mat4]) -> Vec<Mat4> {
    let reader = skin.reader(|b| loaded.buffers.get(b.index()).map(|v| v.as_slice()));
    let ibms: Vec<Mat4> = match reader.read_inverse_bind_matrices() {
        Some(iter) => iter.map(Mat4::from_gltf_column_major).collect(),
        None => Vec::new(),
    };
    let identity = Mat4::identity();
    skin.joints().enumerate().map(|(k, joint_node)| {
        let ibm = ibms.get(k).copied().unwrap_or(identity);
        world_transforms[joint_node.index()].mul(ibm)
    }).collect()
}

fn collect_materials(loaded: &LoadedGltf) -> Vec<Material> {
    // Reserve one extra slot at index 0 for glTF's "no material" default.
    // We'll refer to indexed materials by (index + 1) in emit_mesh.
    let mut materials = Vec::with_capacity(loaded.document.materials().len() + 1);
    materials.push(Material::default_gltf());
    for m in loaded.document.materials() {
        let pbr = m.pbr_metallic_roughness();
        let alpha_mode = match m.alpha_mode() {
            gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
            gltf::material::AlphaMode::Mask => AlphaMode::Mask,
            gltf::material::AlphaMode::Blend => AlphaMode::Blend,
        };
        let transmission = m.transmission();
        let iridescence = parse_iridescence(&m);
        let anisotropy = parse_anisotropy(&m);
        let dispersion = parse_dispersion(&m);
        let diffuse_transmission = parse_diffuse_transmission(&m);
        // KHR_materials_pbrSpecularGlossiness — legacy alternative to the
        // metallic-roughness workflow. Convert to MR at load time so the shader
        // only needs one code path. When present, overrides pbr_metallic_roughness.
        let sg = m.pbr_specular_glossiness();
        let volume = m.volume();
        // KHR_materials_emissive_strength scales the emissive factor. Spec:
        // "final emissive = emissiveFactor · emissiveTexture · emissiveStrength."
        // Baking it into the factor at load time keeps the shader hot path
        // untouched (one splat, not a runtime branch).
        let em_strength = m.emissive_strength().unwrap_or(1.0);
        let emissive_scaled = {
            let e = m.emissive_factor();
            [e[0] * em_strength, e[1] * em_strength, e[2] * em_strength]
        };
        // If pbrSpecularGlossiness is present, compute an equivalent MR
        // baseColor/metallic/roughness triple (Khronos-recommended fit).
        let (base_from_sg, metallic_from_sg, roughness_from_sg, base_tex_from_sg, texcoord_base_from_sg, xform_base_from_sg) =
            if let Some(sg) = sg.as_ref() {
                let (b, mtl, rgh) = spec_gloss_to_mr(
                    sg.diffuse_factor(),
                    sg.specular_factor(),
                    sg.glossiness_factor(),
                );
                let btex = sg.diffuse_texture().map(|t| t.texture().index() as u32);
                let btc  = sg.diffuse_texture().map(|t| clamp_texcoord(t.tex_coord())).unwrap_or(0);
                let bxf  = sg.diffuse_texture().map(|t| load_texture_transform(&t)).unwrap_or(TextureTransform::IDENTITY);
                (Some(b), Some(mtl), Some(rgh), Some(btex), Some(btc), Some(bxf))
            } else {
                (None, None, None, None, None, None)
            };
        materials.push(Material {
            base_color: base_from_sg.unwrap_or_else(|| pbr.base_color_factor()),
            metallic: metallic_from_sg.unwrap_or_else(|| pbr.metallic_factor()),
            roughness: roughness_from_sg.unwrap_or_else(|| pbr.roughness_factor()),
            emissive: emissive_scaled,
            double_sided: m.double_sided(),
            alpha_mode,
            alpha_cutoff: m.alpha_cutoff().unwrap_or(0.5),
            unlit: m.unlit(),
            base_color_texture:         base_tex_from_sg.unwrap_or_else(|| pbr.base_color_texture().map(|t| t.texture().index() as u32)),
            metallic_roughness_texture: pbr.metallic_roughness_texture().map(|t| t.texture().index() as u32),
            normal_texture:             m.normal_texture().map(|t| t.texture().index() as u32),
            occlusion_texture:          m.occlusion_texture().map(|t| t.texture().index() as u32),
            emissive_texture:           m.emissive_texture().map(|t| t.texture().index() as u32),
            texcoord_base:              texcoord_base_from_sg.unwrap_or_else(|| pbr.base_color_texture().map(|t| clamp_texcoord(t.tex_coord())).unwrap_or(0)),
            texcoord_mr:                pbr.metallic_roughness_texture().map(|t| clamp_texcoord(t.tex_coord())).unwrap_or(0),
            texcoord_normal:            m.normal_texture().map(|t| clamp_texcoord(t.tex_coord())).unwrap_or(0),
            texcoord_occlusion:         m.occlusion_texture().map(|t| clamp_texcoord(t.tex_coord())).unwrap_or(0),
            texcoord_emissive:          m.emissive_texture().map(|t| clamp_texcoord(t.tex_coord())).unwrap_or(0),
            texcoord_clearcoat_normal:  m.clearcoat().and_then(|c| c.clearcoat_normal_texture().map(|t| clamp_texcoord(t.tex_coord()))).unwrap_or(0),
            texcoord_transmission:      transmission.as_ref().and_then(|t| t.transmission_texture().map(|ti| clamp_texcoord(ti.tex_coord()))).unwrap_or(0),
            normal_scale:               m.normal_texture().map(|t| t.scale()).unwrap_or(1.0),
            occlusion_strength:         m.occlusion_texture().map(|t| t.strength()).unwrap_or(1.0),
            clearcoat_factor:    m.clearcoat().map(|c| c.clearcoat_factor()).unwrap_or(0.0),
            clearcoat_roughness: m.clearcoat().map(|c| c.clearcoat_roughness_factor()).unwrap_or(0.0),
            clearcoat_normal_texture: m.clearcoat().and_then(|c| {
                c.clearcoat_normal_texture().map(|t| t.texture().index() as u32)
            }),
            clearcoat_normal_scale:   m.clearcoat().and_then(|c| {
                c.clearcoat_normal_texture().map(|t| t.scale())
            }).unwrap_or(1.0),
            sheen_color:         m.sheen().map(|s| s.sheen_color_factor()).unwrap_or([0.0, 0.0, 0.0]),
            sheen_roughness:     m.sheen().map(|s| s.sheen_roughness_factor()).unwrap_or(0.0),
            ior:             m.ior().unwrap_or(1.5),
            specular_factor: m.specular().map(|s| s.specular_factor()).unwrap_or(1.0),
            specular_color:  m.specular().map(|s| s.specular_color_factor()).unwrap_or([1.0, 1.0, 1.0]),
            xform_base:             xform_base_from_sg.unwrap_or_else(|| pbr.base_color_texture().map(|t| load_texture_transform(&t)).unwrap_or(TextureTransform::IDENTITY)),
            xform_mr:               pbr.metallic_roughness_texture().map(|t| load_texture_transform(&t)).unwrap_or(TextureTransform::IDENTITY),
            xform_normal:           m.normal_texture().map(|t| load_normal_transform(&t)).unwrap_or(TextureTransform::IDENTITY),
            xform_occlusion:        m.occlusion_texture().map(|t| load_occlusion_transform(&t)).unwrap_or(TextureTransform::IDENTITY),
            xform_emissive:         m.emissive_texture().map(|t| load_texture_transform(&t)).unwrap_or(TextureTransform::IDENTITY),
            xform_clearcoat_normal: m.clearcoat().and_then(|c| c.clearcoat_normal_texture().map(|t| load_normal_transform(&t)))
                .unwrap_or(TextureTransform::IDENTITY),
            xform_transmission:     transmission.as_ref().and_then(|t| t.transmission_texture().map(|ti| load_texture_transform(&ti))).unwrap_or(TextureTransform::IDENTITY),
            transmission_factor:  transmission.as_ref().map(|t| t.transmission_factor()).unwrap_or(0.0),
            transmission_texture: transmission.as_ref().and_then(|t| t.transmission_texture().map(|ti| ti.texture().index() as u32)),
            thickness_factor:     volume.as_ref().map(|v| v.thickness_factor()).unwrap_or(0.0),
            attenuation_color:    volume.as_ref().map(|v| v.attenuation_color()).unwrap_or([1.0, 1.0, 1.0]),
            attenuation_distance: volume.as_ref().map(|v| v.attenuation_distance()).unwrap_or(f32::INFINITY),

            iridescence_factor:            iridescence.factor,
            iridescence_ior:               iridescence.ior,
            iridescence_thickness_min:     iridescence.thickness_min,
            iridescence_thickness_max:     iridescence.thickness_max,
            iridescence_texture:           iridescence.texture,
            iridescence_thickness_texture: iridescence.thickness_texture,
            texcoord_iridescence:          iridescence.texcoord,
            texcoord_iridescence_thickness: iridescence.texcoord_thickness,

            anisotropy_strength: anisotropy.strength,
            anisotropy_rotation: anisotropy.rotation,
            anisotropy_texture:  anisotropy.texture,
            texcoord_anisotropy: anisotropy.texcoord,

            dispersion,

            diffuse_transmission_factor:  diffuse_transmission.factor,
            diffuse_transmission_color:   diffuse_transmission.color,
            diffuse_transmission_texture: diffuse_transmission.texture,
            diffuse_transmission_color_texture: diffuse_transmission.color_texture,
            texcoord_diffuse_transmission:       diffuse_transmission.texcoord,
            texcoord_diffuse_transmission_color: diffuse_transmission.texcoord_color,
            xform_diffuse_transmission:       diffuse_transmission.xform,
            xform_diffuse_transmission_color: diffuse_transmission.xform_color,

            precomp: MaterialPrecomp::default(),
        });
    }
    // Fill in per-material precomputes now that every input field is set —
    // MaterialShader::new reads these on every triangle, so the transcendentals
    // and small arithmetic chains only pay once per material this way.
    for m in &mut materials { m.precomp = MaterialPrecomp::from_material(m); }
    materials
}

/// KHR_materials_pbrSpecularGlossiness → metallic-roughness conversion,
/// following the Khronos-recommended shader-space fit. Rough but visually
/// close for typical assets. `(diffuse, spec, glossiness) → (base, metallic,
/// roughness)`.
fn spec_gloss_to_mr(diffuse: [f32; 4], spec: [f32; 3], gloss: f32) -> ([f32; 4], f32, f32) {
    const DIELECTRIC_SPEC: f32 = 0.04;
    const EPS: f32 = 1e-6;
    let roughness = (1.0 - gloss).clamp(0.0, 1.0);
    let spec_max = spec[0].max(spec[1]).max(spec[2]);
    let one_minus_spec = 1.0 - spec_max;
    let diff_lum = 0.299 * diffuse[0] + 0.587 * diffuse[1] + 0.114 * diffuse[2];
    let spec_lum = 0.299 * spec[0] + 0.587 * spec[1] + 0.114 * spec[2];
    let (metallic, base) = if spec_lum < DIELECTRIC_SPEC {
        (0.0, [diffuse[0], diffuse[1], diffuse[2]])
    } else {
        let a = DIELECTRIC_SPEC;
        let b = diff_lum * one_minus_spec / (1.0 - DIELECTRIC_SPEC + EPS) + spec_lum - 2.0 * DIELECTRIC_SPEC;
        let c = DIELECTRIC_SPEC - spec_lum;
        let disc = (b * b - 4.0 * a * c).max(0.0);
        let m = ((-b + disc.sqrt()) / (2.0 * a)).clamp(0.0, 1.0);
        let inv_1m = 1.0 / (1.0 - m + EPS);
        let base_dielectric = [
            diffuse[0] * one_minus_spec / (1.0 - DIELECTRIC_SPEC + EPS) * inv_1m,
            diffuse[1] * one_minus_spec / (1.0 - DIELECTRIC_SPEC + EPS) * inv_1m,
            diffuse[2] * one_minus_spec / (1.0 - DIELECTRIC_SPEC + EPS) * inv_1m,
        ];
        let m2 = m * m;
        let base_metallic = spec;
        let base = [
            base_dielectric[0] * (1.0 - m2) + base_metallic[0] * m2,
            base_dielectric[1] * (1.0 - m2) + base_metallic[1] * m2,
            base_dielectric[2] * (1.0 - m2) + base_metallic[2] * m2,
        ];
        (m, base)
    };
    ([base[0], base[1], base[2], diffuse[3]], metallic, roughness)
}

/// Extract KHR_materials_dispersion — a single scalar factor (0 = no
/// dispersion, higher = more chromatic separation).
fn parse_dispersion(m: &gltf::Material) -> f32 {
    m.extension_value("KHR_materials_dispersion")
        .and_then(|ext| ext.get("dispersion"))
        .and_then(|v| v.as_f64())
        .map(|v| v as f32)
        .unwrap_or(0.0)
}

/// Parsed KHR_materials_iridescence bundle (defaults per spec).
#[derive(Default)]
struct IridescenceCfg {
    factor: f32,
    ior: f32,
    thickness_min: f32,
    thickness_max: f32,
    texture: Option<u32>,
    thickness_texture: Option<u32>,
    texcoord: u8,
    texcoord_thickness: u8,
}

/// Parsed KHR_materials_anisotropy bundle.
#[derive(Default)]
struct AnisotropyCfg {
    strength: f32,
    rotation: f32,
    texture: Option<u32>,
    texcoord: u8,
}

/// Parsed KHR_materials_diffuse_transmission bundle (defaults per spec).
struct DiffuseTransmissionCfg {
    factor: f32,
    color: [f32; 3],
    texture: Option<u32>,
    color_texture: Option<u32>,
    texcoord: u8,
    texcoord_color: u8,
    xform: TextureTransform,
    xform_color: TextureTransform,
}

impl Default for DiffuseTransmissionCfg {
    fn default() -> Self {
        Self {
            factor: 0.0,
            color: [1.0, 1.0, 1.0],
            texture: None,
            color_texture: None,
            texcoord: 0,
            texcoord_color: 0,
            xform: TextureTransform::IDENTITY,
            xform_color: TextureTransform::IDENTITY,
        }
    }
}

/// Extract KHR_materials_diffuse_transmission from a material. Not in gltf-rs's
/// feature list — parsed from the raw JSON extension map. Spec:
///   https://github.com/KhronosGroup/glTF/tree/main/extensions/2.0/Khronos/KHR_materials_diffuse_transmission
///
/// The extension adds a diffuse lobe for light passing THROUGH the surface
/// (thin cloth, backlit leaves, paper). `diffuseTransmissionTexture` sampled
/// from the .a channel; `diffuseTransmissionColorTexture` from RGB.
fn parse_diffuse_transmission(m: &gltf::Material) -> DiffuseTransmissionCfg {
    let mut cfg = DiffuseTransmissionCfg::default();
    let Some(ext) = m.extension_value("KHR_materials_diffuse_transmission") else { return cfg; };
    if let Some(v) = ext.get("diffuseTransmissionFactor").and_then(|x| x.as_f64()) {
        cfg.factor = (v as f32).clamp(0.0, 1.0);
    }
    if let Some(arr) = ext.get("diffuseTransmissionColorFactor").and_then(|x| x.as_array()) {
        if arr.len() >= 3 {
            cfg.color = [
                arr[0].as_f64().unwrap_or(1.0) as f32,
                arr[1].as_f64().unwrap_or(1.0) as f32,
                arr[2].as_f64().unwrap_or(1.0) as f32,
            ];
        }
    }
    if let Some(t) = ext.get("diffuseTransmissionTexture") {
        if let Some(idx) = t.get("index").and_then(|x| x.as_u64()) { cfg.texture = Some(idx as u32); }
        if let Some(tc)  = t.get("texCoord").and_then(|x| x.as_u64()) { cfg.texcoord = clamp_texcoord(tc as u32); }
        cfg.xform = parse_texture_transform_json(t);
    }
    if let Some(t) = ext.get("diffuseTransmissionColorTexture") {
        if let Some(idx) = t.get("index").and_then(|x| x.as_u64()) { cfg.color_texture = Some(idx as u32); }
        if let Some(tc)  = t.get("texCoord").and_then(|x| x.as_u64()) { cfg.texcoord_color = clamp_texcoord(tc as u32); }
        cfg.xform_color = parse_texture_transform_json(t);
    }
    cfg
}

/// Extract `KHR_texture_transform` from a raw JSON `textureInfo` (used by
/// the extension parsers that go through `extension_value` rather than
/// gltf-rs's typed API). Returns identity when the transform is missing.
fn parse_texture_transform_json(t: &serde_json::Value) -> TextureTransform {
    let Some(tt) = t.get("extensions").and_then(|e| e.get("KHR_texture_transform")) else {
        return TextureTransform::IDENTITY;
    };
    let scale = tt.get("scale").and_then(|s| s.as_array()).map(|a| [
        a.get(0).and_then(|v| v.as_f64()).unwrap_or(1.0) as f32,
        a.get(1).and_then(|v| v.as_f64()).unwrap_or(1.0) as f32,
    ]).unwrap_or([1.0, 1.0]);
    let offset = tt.get("offset").and_then(|s| s.as_array()).map(|a| [
        a.get(0).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
        a.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
    ]).unwrap_or([0.0, 0.0]);
    let rot = tt.get("rotation").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
    TextureTransform {
        scale, offset,
        rot_cos: rot.cos(),
        rot_sin: rot.sin(),
    }
}

/// Extract KHR_materials_iridescence from a material's raw extension map.
/// Extension isn't in the gltf-rs crate's feature list, so we parse the JSON
/// directly. Spec-default values kick in when a field is missing.
fn parse_iridescence(m: &gltf::Material) -> IridescenceCfg {
    let mut cfg = IridescenceCfg { ior: 1.3, thickness_min: 100.0, thickness_max: 400.0, ..Default::default() };
    let Some(ext) = m.extension_value("KHR_materials_iridescence") else { return cfg; };
    if let Some(v) = ext.get("iridescenceFactor").and_then(|x| x.as_f64()) {
        cfg.factor = v as f32;
    }
    if let Some(v) = ext.get("iridescenceIor").and_then(|x| x.as_f64()) {
        cfg.ior = v as f32;
    }
    if let Some(v) = ext.get("iridescenceThicknessMinimum").and_then(|x| x.as_f64()) {
        cfg.thickness_min = v as f32;
    }
    if let Some(v) = ext.get("iridescenceThicknessMaximum").and_then(|x| x.as_f64()) {
        cfg.thickness_max = v as f32;
    }
    if let Some(t) = ext.get("iridescenceTexture") {
        if let Some(idx) = t.get("index").and_then(|x| x.as_u64()) {
            cfg.texture = Some(idx as u32);
        }
        if let Some(tc) = t.get("texCoord").and_then(|x| x.as_u64()) {
            cfg.texcoord = clamp_texcoord(tc as u32);
        }
    }
    if let Some(t) = ext.get("iridescenceThicknessTexture") {
        if let Some(idx) = t.get("index").and_then(|x| x.as_u64()) {
            cfg.thickness_texture = Some(idx as u32);
        }
        if let Some(tc) = t.get("texCoord").and_then(|x| x.as_u64()) {
            cfg.texcoord_thickness = clamp_texcoord(tc as u32);
        }
    }
    cfg
}

fn parse_anisotropy(m: &gltf::Material) -> AnisotropyCfg {
    let mut cfg = AnisotropyCfg::default();
    let Some(ext) = m.extension_value("KHR_materials_anisotropy") else { return cfg; };
    if let Some(v) = ext.get("anisotropyStrength").and_then(|x| x.as_f64()) {
        cfg.strength = (v as f32).clamp(0.0, 1.0);
    }
    if let Some(v) = ext.get("anisotropyRotation").and_then(|x| x.as_f64()) {
        cfg.rotation = v as f32;
    }
    if let Some(t) = ext.get("anisotropyTexture") {
        if let Some(idx) = t.get("index").and_then(|x| x.as_u64()) {
            cfg.texture = Some(idx as u32);
        }
        if let Some(tc) = t.get("texCoord").and_then(|x| x.as_u64()) {
            cfg.texcoord = clamp_texcoord(tc as u32);
        }
    }
    cfg
}

#[inline]
fn clamp_texcoord(n: u32) -> u8 {
    // glTF allows arbitrary TEXCOORD_N — the spec puts no upper bound on N.
    // We only wire slots 0 and 1 through the `Vertex` struct; anything with
    // `texCoord: 2` or higher silently falls back to slot 1 (which is the
    // closer approximation than falling back to slot 0 — a material that
    // authored TEXCOORD_2 will have TEXCOORD_1 too more often than not).
    //
    // Widening the vertex to carry more slots is a real change (~200 LOC
    // through `Vertex` + shader + rasterizer barycentric interp). Deferred
    // — very few real-world assets use TEXCOORD_N for N ≥ 2.
    if n >= 1 { 1 } else { 0 }
}

fn load_texture_transform(info: &gltf::texture::Info) -> TextureTransform {
    match info.texture_transform() {
        Some(tt) => {
            let (scale, offset, rot) = (tt.scale(), tt.offset(), tt.rotation());
            TextureTransform {
                scale,
                offset,
                rot_cos: rot.cos(),
                rot_sin: rot.sin(),
            }
        }
        None => TextureTransform::IDENTITY,
    }
}

fn load_normal_transform(info: &gltf::material::NormalTexture) -> TextureTransform {
    match info.texture_transform() {
        Some(tt) => {
            let (scale, offset, rot) = (tt.scale(), tt.offset(), tt.rotation());
            TextureTransform { scale, offset, rot_cos: rot.cos(), rot_sin: rot.sin() }
        }
        None => TextureTransform::IDENTITY,
    }
}

fn load_occlusion_transform(info: &gltf::material::OcclusionTexture) -> TextureTransform {
    match info.texture_transform() {
        Some(tt) => {
            let (scale, offset, rot) = (tt.scale(), tt.offset(), tt.rotation());
            TextureTransform { scale, offset, rot_cos: rot.cos(), rot_sin: rot.sin() }
        }
        None => TextureTransform::IDENTITY,
    }
}

fn collect_textures(loaded: &LoadedGltf, opts: TextureLoadOpts) -> Vec<Texture> {
    let n = loaded.document.textures().len();
    if opts.disabled {
        // Fast-path draft mode: every texture is a solid-white 1×1 placeholder.
        // Materials fall back to their factors alone.
        return (0..n).map(|_| placeholder_texture()).collect();
    }
    let mut textures = Vec::with_capacity(n);
    for t in loaded.document.textures() {
        // Best-effort: skip textures we can't decode (log-friendly failure
        // instead of aborting the whole render). Downstream shader treats
        // a missing texture as "factor only", which is the graceful path.
        match load_texture(loaded, &t, opts) {
            Ok(tex) => textures.push(tex),
            Err(_e) => textures.push(placeholder_texture()),
        }
    }
    textures
}

fn load_texture(loaded: &LoadedGltf, t: &gltf::Texture, opts: TextureLoadOpts) -> Result<Texture, String> {
    // `allow_empty_texture` returns `Option<Image>` — None means the texture
    // references an image via an extension we didn't build gltf-rs to
    // understand (EXT_texture_avif, KHR_texture_basisu). Fall through to the
    // placeholder texture rather than panicking.
    let image = t.source().ok_or("texture has no primary source (unsupported ext?)")?;
    // Bind a Vec outside the match so the data-URI branch can own the decoded
    // bytes while the View/sidecar branches borrow. Rust's "definitely assigned"
    // analysis lets us leave it uninitialised until the arm needs it.
    let data_uri_owned: Vec<u8>;
    let (bytes, mime): (&[u8], Option<&str>) = match image.source() {
        gltf::image::Source::View { view, mime_type } => {
            let buf = loaded.buffers.get(view.buffer().index())
                .ok_or("texture: buffer index out of range")?;
            let start = view.offset();
            let end = start + view.length();
            (buf.get(start..end).ok_or("texture: buffer view out of range")?, Some(mime_type))
        }
        gltf::image::Source::Uri { uri, mime_type } => {
            if let Some(decoded) = crate::gltf_loader::decode_data_uri(uri) {
                data_uri_owned = decoded;
                (data_uri_owned.as_slice(), mime_type)
            } else {
                let bytes = loaded.sidecars.get(uri)
                    .ok_or_else(|| format!(
                        "texture references external URI '{}' but no matching \
                         sidecar was provided", uri))?;
                (bytes.as_slice(), mime_type)
            }
        }
    };
    let decoded = texture_decode::decode(bytes, mime)?;
    let mut base = MipLevel { width: decoded.width, height: decoded.height, rgba: decoded.rgba };
    // Downsample to `max_size` before mip-chain generation so subsequent mips
    // start from the capped resolution. Halving in a loop (not one-shot to
    // arbitrary size) keeps the resample cheap and gives identical output to
    // "one extra mip level" — the difference is only where LOD 0 sits.
    if let Some(cap) = opts.max_size {
        while base.width > cap || base.height > cap {
            if base.width <= 4 || base.height <= 4 { break; }
            base = maquette_core::texture::downsample_2x_pub(&base);
        }
    }
    let sampler = t.sampler();
    let mips = build_mips(base);
    let (bw, bh) = (mips[0].width, mips[0].height);
    Ok(Texture {
        mips,
        wrap_s: map_wrap(sampler.wrap_s()),
        wrap_t: map_wrap(sampler.wrap_t()),
        mag_filter: sampler.mag_filter().map(map_mag).unwrap_or(Filter::Linear),
        min_filter: sampler.min_filter().map(map_min).unwrap_or(Filter::Linear),
        lod_bias: 0.5 * ((bw * bh) as f32).log2(),
    })
}

/// Leak a Vec of placeholder textures so `get_gltf_info` can return a
/// `&'static [Texture]` for its cheap scene.
fn placeholder_textures_for(n: usize) -> &'static [Texture] {
    static mut PLACEHOLDERS: Vec<Vec<Texture>> = Vec::new();
    unsafe {
        // Reuse if a prior call already leaked the same size (uncommon).
        for v in PLACEHOLDERS.iter() {
            if v.len() == n {
                return std::mem::transmute::<&[Texture], &'static [Texture]>(v.as_slice());
            }
        }
        let v: Vec<Texture> = (0..n).map(|_| placeholder_texture()).collect();
        PLACEHOLDERS.push(v);
        let last = PLACEHOLDERS.last().unwrap();
        std::mem::transmute::<&[Texture], &'static [Texture]>(last.as_slice())
    }
}

fn placeholder_texture() -> Texture {
    // Solid white 1×1 so missing textures collapse to "factor only" behaviour.
    Texture {
        mips: vec![MipLevel { width: 1, height: 1, rgba: vec![255, 255, 255, 255] }],
        wrap_s: Wrap::Repeat, wrap_t: Wrap::Repeat,
        mag_filter: Filter::Nearest, min_filter: Filter::Nearest,
        lod_bias: 0.0,   // 0.5 · log₂(1) = 0
    }
}

fn map_wrap(w: gltf::texture::WrappingMode) -> Wrap {
    match w {
        gltf::texture::WrappingMode::ClampToEdge   => Wrap::ClampToEdge,
        gltf::texture::WrappingMode::MirroredRepeat => Wrap::MirroredRepeat,
        gltf::texture::WrappingMode::Repeat        => Wrap::Repeat,
    }
}

fn map_mag(f: gltf::texture::MagFilter) -> Filter {
    match f {
        gltf::texture::MagFilter::Nearest => Filter::Nearest,
        gltf::texture::MagFilter::Linear  => Filter::Linear,
    }
}

fn map_min(f: gltf::texture::MinFilter) -> Filter {
    // Mipmap modes collapse to their base filter until we generate mips.
    use gltf::texture::MinFilter as M;
    match f {
        M::Nearest | M::NearestMipmapNearest | M::NearestMipmapLinear => Filter::Nearest,
        M::Linear  | M::LinearMipmapNearest  | M::LinearMipmapLinear  => Filter::Linear,
    }
}

fn node_transform_animated(node: &gltf::Node, anim: Option<&AnimSample>) -> Mat4 {
    // If any TRS component is animated for this node, we build a fresh TRS
    // matrix using the node's base T/R/S as the "unanimated" fallback for
    // components that this animation doesn't touch. If nothing's animated
    // we fall back to the node's own transform (matrix or decomposed).
    let anim = match anim {
        Some(a) if a.any() => a,
        _ => return match node.transform() {
            gltf::scene::Transform::Matrix { matrix } => Mat4::from_gltf_column_major(matrix),
            gltf::scene::Transform::Decomposed { translation, rotation, scale } => Mat4::from_trs(translation, rotation, scale),
        },
    };
    // Base TRS: from decomposed if present, else fall back to identity when
    // the node uses a matrix (spec: animated nodes shouldn't use matrix, so
    // this path is theoretical).
    let (bt, br, bs) = match node.transform() {
        gltf::scene::Transform::Decomposed { translation, rotation, scale } => (translation, rotation, scale),
        gltf::scene::Transform::Matrix { .. } => ([0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
    };
    let t = anim.translation.unwrap_or(bt);
    let r = anim.rotation.unwrap_or(br);
    let s = anim.scale.unwrap_or(bs);
    Mat4::from_trs(t, r, s)
}

// ---------------------------------------------------------------------------
// Animation sampling
// ---------------------------------------------------------------------------

/// Per-node TRS override sampled from animation channels at a specific time.
/// Any `None` field means that node component isn't animated — fall back to
/// the node's base transform.
#[derive(Clone, Default)]
pub struct AnimSample {
    pub translation: Option<[f32; 3]>,
    pub rotation: Option<[f32; 4]>,
    pub scale: Option<[f32; 3]>,
    /// Morph-target weights. `None` = not animated; falls back to
    /// node.weights() / mesh.weights() defined in the glTF.
    pub weights: Option<Vec<f32>>,
}

impl AnimSample {
    fn any(&self) -> bool {
        self.translation.is_some() || self.rotation.is_some() || self.scale.is_some()
    }
}

/// Sample animation channels at `time` (seconds) and produce a per-node TRS
/// override table.
///
/// `animation_index` picks a specific clip; when `None` we play every
/// animation stacked (last-write-wins on shared channels — the legacy
/// behaviour). glTF assets ship animations as independent playable clips
/// (idle, walk, run, ...) so the picker is what users normally want.
/// Missing / out-of-range indices fall back to the stacked path.
fn sample_animations(loaded: &LoadedGltf, time: f32, animation_index: Option<usize>) -> Vec<AnimSample> {
    let n_nodes = loaded.document.nodes().count();
    let mut samples = vec![AnimSample::default(); n_nodes];

    let animations: Box<dyn Iterator<Item = gltf::Animation>> = match animation_index {
        Some(i) if i < loaded.document.animations().len() => {
            Box::new(loaded.document.animations().nth(i).into_iter())
        }
        _ => Box::new(loaded.document.animations()),
    };
    for anim in animations {
        for channel in anim.channels() {
            let node_idx = channel.target().node().index();
            if node_idx >= n_nodes { continue; }
            let sampler = channel.sampler();
            let interp = sampler.interpolation();
            let reader = channel.reader(|buffer| {
                loaded.buffers.get(buffer.index()).map(|v| v.as_slice())
            });
            let times: Vec<f32> = match reader.read_inputs() {
                Some(iter) => iter.collect(),
                None => continue,
            };
            if times.is_empty() { continue; }

            match reader.read_outputs() {
                Some(gltf::animation::util::ReadOutputs::Translations(iter)) => {
                    let values: Vec<[f32; 3]> = iter.collect();
                    samples[node_idx].translation = Some(sample_vec3(&times, &values, time, interp));
                }
                Some(gltf::animation::util::ReadOutputs::Rotations(rots)) => {
                    let values: Vec<[f32; 4]> = rots.into_f32().collect();
                    samples[node_idx].rotation = Some(sample_quat(&times, &values, time, interp));
                }
                Some(gltf::animation::util::ReadOutputs::Scales(iter)) => {
                    let values: Vec<[f32; 3]> = iter.collect();
                    samples[node_idx].scale = Some(sample_vec3(&times, &values, time, interp));
                }
                None => {}
                Some(gltf::animation::util::ReadOutputs::MorphTargetWeights(w)) => {
                    let flat: Vec<f32> = w.into_f32().collect();
                    // Weights are packed as `num_targets` per keyframe, in the
                    // same input-time order as the sampler. Determine
                    // num_targets from the mesh on the target node.
                    let num_targets = loaded.document.nodes().nth(node_idx)
                        .and_then(|n| n.mesh())
                        .and_then(|m| m.primitives().next())
                        .map(|p| p.morph_targets().count())
                        .unwrap_or(0);
                    if num_targets > 0 && !flat.is_empty() && flat.len() % num_targets == 0 {
                        samples[node_idx].weights = Some(sample_weights(&times, &flat, num_targets, time, interp));
                    }
                }
            }
        }
    }
    samples
}

fn sample_weights(times: &[f32], flat: &[f32], num_targets: usize, t: f32, interp: gltf::animation::Interpolation) -> Vec<f32> {
    use gltf::animation::Interpolation as I;
    let (i, alpha) = find_bracket(times, t);
    let (values_per_kf, val_offset) = match interp {
        I::CubicSpline => (num_targets * 3, num_targets),
        _ => (num_targets, 0),
    };
    let read = |kf: usize| -> Vec<f32> {
        let base = kf * values_per_kf + val_offset;
        (0..num_targets).map(|k| flat.get(base + k).copied().unwrap_or(0.0)).collect()
    };
    if alpha == 0.0 || (i + 1) * values_per_kf > flat.len() {
        return read(i);
    }
    match interp {
        I::Step => read(i),
        I::Linear => {
            let a = read(i);
            let b = read(i + 1);
            (0..num_targets).map(|k| a[k] + alpha * (b[k] - a[k])).collect()
        }
        I::CubicSpline => {
            let td = times[i + 1] - times[i];
            let (h00, h10, h01, h11) = hermite_basis(alpha);
            let read_at = |kf: usize, sub: usize| -> Vec<f32> {
                let base = kf * values_per_kf + sub * num_targets;
                (0..num_targets).map(|k| flat.get(base + k).copied().unwrap_or(0.0)).collect()
            };
            let vk  = read_at(i, 1);
            let bk  = read_at(i, 2);
            let ak1 = read_at(i + 1, 0);
            let vk1 = read_at(i + 1, 1);
            (0..num_targets).map(|k| {
                h00*vk[k] + td*h10*bk[k] + h01*vk1[k] + td*h11*ak1[k]
            }).collect()
        }
    }
}

/// Locate the two keyframes bracketing `t`. Returns `(lo, alpha)` where
/// `lo` is the index and `alpha ∈ [0,1]` interpolates lo → lo+1.
fn find_bracket(times: &[f32], t: f32) -> (usize, f32) {
    if t <= times[0] { return (0, 0.0); }
    if t >= *times.last().unwrap() { return (times.len() - 1, 0.0); }
    // Linear search — fine for typical animations (<100 keyframes/channel);
    // upgrade to binary search if we ever hit assets with dense sampling.
    for i in 0..times.len() - 1 {
        if t < times[i + 1] {
            let span = times[i + 1] - times[i];
            let alpha = if span > 1e-9 { (t - times[i]) / span } else { 0.0 };
            return (i, alpha);
        }
    }
    (times.len() - 1, 0.0)
}

fn sample_vec3(times: &[f32], values: &[[f32; 3]], t: f32, interp: gltf::animation::Interpolation) -> [f32; 3] {
    use gltf::animation::Interpolation as I;
    let (i, alpha) = find_bracket(times, t);
    if alpha == 0.0 || i + 1 >= values.len() {
        // CubicSpline uses 3 output values per keyframe (in-tangent, value,
        // out-tangent). We fall back to picking `value` — index * 3 + 1.
        return match interp {
            I::CubicSpline => values.get(i * 3 + 1).copied().unwrap_or([0.0, 0.0, 0.0]),
            _ => values[i.min(values.len() - 1)],
        };
    }
    match interp {
        I::Step => values[i],
        I::Linear => [
            values[i][0] + alpha * (values[i + 1][0] - values[i][0]),
            values[i][1] + alpha * (values[i + 1][1] - values[i][1]),
            values[i][2] + alpha * (values[i + 1][2] - values[i][2]),
        ],
        // Cubic Hermite spline. glTF packs each keyframe as
        //   [in_tangent, value, out_tangent]
        // consecutively, so keyframe k lives at indices k*3..k*3+3.
        I::CubicSpline => {
            let td = times[i + 1] - times[i];
            let (h00, h10, h01, h11) = hermite_basis(alpha);
            let vk  = values.get(i * 3 + 1).copied().unwrap_or([0.0; 3]);
            let bk  = values.get(i * 3 + 2).copied().unwrap_or([0.0; 3]);
            let ak1 = values.get((i + 1) * 3).copied().unwrap_or([0.0; 3]);
            let vk1 = values.get((i + 1) * 3 + 1).copied().unwrap_or(vk);
            [
                h00*vk[0] + td*h10*bk[0] + h01*vk1[0] + td*h11*ak1[0],
                h00*vk[1] + td*h10*bk[1] + h01*vk1[1] + td*h11*ak1[1],
                h00*vk[2] + td*h10*bk[2] + h01*vk1[2] + td*h11*ak1[2],
            ]
        }
    }
}

#[inline]
fn hermite_basis(s: f32) -> (f32, f32, f32, f32) {
    let s2 = s * s;
    let s3 = s2 * s;
    (
        2.0 * s3 - 3.0 * s2 + 1.0,   // h00 for value at k
        s3 - 2.0 * s2 + s,           // h10 for out-tangent at k
        -2.0 * s3 + 3.0 * s2,        // h01 for value at k+1
        s3 - s2,                     // h11 for in-tangent at k+1
    )
}

fn sample_quat(times: &[f32], values: &[[f32; 4]], t: f32, interp: gltf::animation::Interpolation) -> [f32; 4] {
    use gltf::animation::Interpolation as I;
    let (i, alpha) = find_bracket(times, t);
    if alpha == 0.0 || i + 1 >= values.len() {
        return match interp {
            I::CubicSpline => values.get(i * 3 + 1).copied().unwrap_or([0.0, 0.0, 0.0, 1.0]),
            _ => values[i.min(values.len() - 1)],
        };
    }
    match interp {
        I::Step => values[i],
        I::Linear => nlerp_quat(values[i], values[i + 1], alpha),
        I::CubicSpline => {
            let td = times[i + 1] - times[i];
            let (h00, h10, h01, h11) = hermite_basis(alpha);
            let vk  = values.get(i * 3 + 1).copied().unwrap_or([0.0, 0.0, 0.0, 1.0]);
            let bk  = values.get(i * 3 + 2).copied().unwrap_or([0.0; 4]);
            let ak1 = values.get((i + 1) * 3).copied().unwrap_or([0.0; 4]);
            let vk1 = values.get((i + 1) * 3 + 1).copied().unwrap_or(vk);
            let mut q = [
                h00*vk[0] + td*h10*bk[0] + h01*vk1[0] + td*h11*ak1[0],
                h00*vk[1] + td*h10*bk[1] + h01*vk1[1] + td*h11*ak1[1],
                h00*vk[2] + td*h10*bk[2] + h01*vk1[2] + td*h11*ak1[2],
                h00*vk[3] + td*h10*bk[3] + h01*vk1[3] + td*h11*ak1[3],
            ];
            let len2 = q[0]*q[0] + q[1]*q[1] + q[2]*q[2] + q[3]*q[3];
            if len2 > 1e-14 {
                let inv = len2.sqrt().recip();
                q[0] *= inv; q[1] *= inv; q[2] *= inv; q[3] *= inv;
            }
            q
        }
    }
}

fn nlerp_quat(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let dot = a[0]*b[0] + a[1]*b[1] + a[2]*b[2] + a[3]*b[3];
    let sign = if dot < 0.0 { -1.0 } else { 1.0 };
    let it = 1.0 - t;
    let mut q = [
        it * a[0] + t * sign * b[0],
        it * a[1] + t * sign * b[1],
        it * a[2] + t * sign * b[2],
        it * a[3] + t * sign * b[3],
    ];
    let len2 = q[0]*q[0] + q[1]*q[1] + q[2]*q[2] + q[3]*q[3];
    if len2 > 1e-14 {
        let inv = len2.sqrt().recip();
        q[0] *= inv; q[1] *= inv; q[2] *= inv; q[3] *= inv;
    }
    q
}

#[allow(clippy::too_many_arguments)]
fn emit_mesh(
    scene: &mut Scene,
    loaded: &LoadedGltf,
    mesh: gltf::Mesh,
    world: Mat4,
    skin_palette: Option<&[Mat4]>,
    morph_weights: Option<&[f32]>,
    variant: u32,
) {
    for primitive in mesh.primitives() {
        // All modes handled — Points/Lines emit into scene.points/lines from
        // `emit_indexed` and return early; triangle modes continue below.

        // KHR_materials_variants: if the primitive has variant mappings and
        // the requested variant is listed, use the mapped material; otherwise
        // fall back to the primitive's default material.
        let variant_material = primitive.mappings()
            .find(|m| m.variants().iter().any(|&v| v == variant))
            .map(|m| m.material());
        let base_material = variant_material.unwrap_or_else(|| primitive.material());
        let material_id = base_material.index()
            .map(|i| (i + 1) as u32)
            .unwrap_or(0);

        let reader = primitive.reader(|buffer| {
            loaded.buffers.get(buffer.index()).map(|v| v.as_slice())
        });

        // gltf-rs's typed readers (`read_positions/normals/tangents`) work
        // by reinterpreting raw accessor bytes as `f32` — which is wrong
        // for KHR_mesh_quantization assets where POSITION/NORMAL/TANGENT
        // are stored as normalized i8/u8/i16/u16 integers. We route
        // through a helper that dispatches on `data_type()` and covers
        // every spec-legal componentType.
        let Some(pos_acc) = primitive.get(&gltf::Semantic::Positions) else { continue; };
        let positions_raw = match read_vec3_f32(&pos_acc, loaded) {
            Ok(v) => v, Err(_) => continue,
        };
        let mut positions_bind: Vec<Vec3> = positions_raw.iter()
            .map(|p| Vec3::new(p[0] as f64, p[1] as f64, p[2] as f64)).collect();
        let mut normals_bind: Option<Vec<Vec3>> = primitive
            .get(&gltf::Semantic::Normals)
            .and_then(|acc| read_vec3_f32(&acc, loaded).ok())
            .map(|v| v.iter()
                .map(|n| Vec3::new(n[0] as f64, n[1] as f64, n[2] as f64)).collect());
        let mut tangents_bind: Option<Vec<[f32; 4]>> = primitive
            .get(&gltf::Semantic::Tangents)
            .and_then(|acc| read_vec4_f32(&acc, loaded).ok());

        // Apply morph-target deltas in bind space (per glTF spec: morph
        // targets add position/normal/tangent deltas weighted by
        // node/mesh/animation weights, THEN skinning transforms the result).
        // Morph deltas may be quantized under KHR_mesh_quantization too,
        // so we go through the same f32-dispatch helpers as the base attrs.
        if let Some(weights) = morph_weights.filter(|w| !w.is_empty()) {
            for (i, target) in primitive.morph_targets().enumerate() {
                let w = weights.get(i).copied().unwrap_or(0.0);
                if w == 0.0 { continue; }
                if let Some(deltas) = target.positions().and_then(|acc| read_vec3_f32(&acc, loaded).ok()) {
                    for (v, d) in positions_bind.iter_mut().zip(deltas.iter()) {
                        v.x += w as f64 * d[0] as f64;
                        v.y += w as f64 * d[1] as f64;
                        v.z += w as f64 * d[2] as f64;
                    }
                }
                if let Some(ns) = normals_bind.as_mut() {
                    if let Some(deltas) = target.normals().and_then(|acc| read_vec3_f32(&acc, loaded).ok()) {
                        for (v, d) in ns.iter_mut().zip(deltas.iter()) {
                            v.x += w as f64 * d[0] as f64;
                            v.y += w as f64 * d[1] as f64;
                            v.z += w as f64 * d[2] as f64;
                        }
                    }
                }
                if let Some(ts) = tangents_bind.as_mut() {
                    if let Some(deltas) = target.tangents().and_then(|acc| read_vec3_f32(&acc, loaded).ok()) {
                        // Morph target tangent deltas are vec3 per spec
                        // (the .w handedness is inherited from the base
                        // tangent, not morphed).
                        for (t, d) in ts.iter_mut().zip(deltas.iter()) {
                            t[0] += w * d[0];
                            t[1] += w * d[1];
                            t[2] += w * d[2];
                        }
                    }
                }
            }
        }

        // Transform to world space — either via the mesh's node transform
        // (unskinned) or via per-vertex joint weighting (skinned). glTF spec:
        // for skinned meshes, the node transform is ignored.
        let (positions, normals, tangents) = if let Some(palette) = skin_palette {
            let joints_iter = reader.read_joints(0);
            let weights_iter = reader.read_weights(0);
            let (Some(j), Some(w)) = (joints_iter, weights_iter) else { continue; };
            let joints: Vec<[u16; 4]> = j.into_u16().collect();
            let weights: Vec<[f32; 4]> = w.into_f32().collect();
            // JOINTS_1 / WEIGHTS_1 extend the influence set to 8 per vertex.
            // glTF spec allows arbitrary N-multiples but rec. supporting ≥ 2.
            // Empty vecs when the primitive has no second set — skinning code
            // treats them as all-zero weights (no-op).
            let joints1: Vec<[u16; 4]> = reader.read_joints(1).map(|j| j.into_u16().collect()).unwrap_or_default();
            let weights1: Vec<[f32; 4]> = reader.read_weights(1).map(|w| w.into_f32().collect()).unwrap_or_default();
            apply_skinning(&positions_bind, normals_bind.as_deref(), tangents_bind.as_deref(), &joints, &weights, &joints1, &weights1, palette)
        } else {
            let normal_mat = world.normal_matrix_3x3();
            let pos: Vec<Vec3> = positions_bind.iter().map(|p| world.transform_point(*p)).collect();
            let nrm: Option<Vec<Vec3>> = normals_bind.as_ref().map(|ns| {
                ns.iter().map(|n| normal_mat.transform_vector(*n).normalized()).collect()
            });
            let tan: Option<Vec<[f32; 4]>> = tangents_bind.as_ref().map(|ts| {
                ts.iter().map(|t| {
                    let tw = normal_mat.transform_vector(Vec3::new(t[0] as f64, t[1] as f64, t[2] as f64)).normalized();
                    [tw.x as f32, tw.y as f32, tw.z as f32, t[3]]
                }).collect()
            });
            (pos, nrm, tan)
        };

        // Texcoord accessors go through the same quantization-aware helper
        // as positions/normals — gltf-rs's `.into_f32()` divides u16 by
        // 65535 even when the accessor is `normalized: false`, which
        // corrupts KHR_mesh_quantization texcoords (they're paired with a
        // KHR_texture_transform that expects raw ints).
        let uvs: Vec<[f32; 2]> = primitive.get(&gltf::Semantic::TexCoords(0))
            .and_then(|acc| read_vec2_f32(&acc, loaded).ok())
            .unwrap_or_default();
        let uvs1: Vec<[f32; 2]> = primitive.get(&gltf::Semantic::TexCoords(1))
            .and_then(|acc| read_vec2_f32(&acc, loaded).ok())
            .unwrap_or_default();
        // COLOR_0 may be vec3 or vec4 per glTF spec — the utils reader
        // hands us `[f32; 4]` in either case (alpha=1 padded for vec3).
        let colors: Vec<[f32; 4]> = reader
            .read_colors(0)
            .map(|c| c.into_rgba_f32().collect())
            .unwrap_or_default();

        emit_indexed(scene, &positions, normals.as_deref(), &uvs, &uvs1, &colors, tangents.as_deref(), primitive.mode(), reader, material_id);
    }
}

/// Per-vertex linear-blend skinning: `pos_out = Σ_k w_k · (joint_matrix[j_k] · pos_bind)`.
/// Normals + tangents (when present) transform the same way but through the
/// blended matrix's rotational component (no re-orthonormalisation across
/// weighted joints — good enough for rigid + roughly-uniform-scale joints,
/// which cover ~all real skinned characters).
fn apply_skinning(
    positions_bind: &[Vec3],
    normals_bind: Option<&[Vec3]>,
    tangents_bind: Option<&[[f32; 4]]>,
    joints: &[[u16; 4]],
    weights: &[[f32; 4]],
    joints1: &[[u16; 4]],
    weights1: &[[f32; 4]],
    palette: &[Mat4],
) -> (Vec<Vec3>, Option<Vec<Vec3>>, Option<Vec<[f32; 4]>>) {
    let n = positions_bind.len();
    let mut positions = Vec::with_capacity(n);
    let mut normals = normals_bind.map(|_| Vec::with_capacity(n));
    let mut tangents = tangents_bind.map(|_| Vec::with_capacity(n));

    for i in 0..n {
        // Sum the (weight · joint_matrix) into one matrix, then apply. Loops
        // over both influence sets — up to 8 joints/weights per vertex per
        // glTF spec (JOINTS_0/WEIGHTS_0 and JOINTS_1/WEIGHTS_1). Renormalise
        // the raw weights to sum to 1 first — the spec requires it, but
        // exporters (Blender, gltfpack) sometimes emit slightly-off sums
        // (float drift, or quantized u8/u16 weights that don't quite hit
        // 255/65535). Un-normalised weights leave the blended matrix
        // shorter/longer than a pure rotation, which visibly shrinks or
        // stretches deformed verts.
        let raw_sum: f64 =
            weights.get(i).copied().unwrap_or([0.0; 4]).iter().map(|w| *w as f64).sum::<f64>()
          + weights1.get(i).copied().unwrap_or([0.0; 4]).iter().map(|w| *w as f64).sum::<f64>();
        let norm = if raw_sum > 1e-9 { 1.0 / raw_sum } else { 1.0 };
        let mut m = [[0.0f64; 4]; 4];
        for (js_arr, ws_arr) in [(joints, weights), (joints1, weights1)] {
            let js = js_arr.get(i).copied().unwrap_or([0; 4]);
            let ws = ws_arr.get(i).copied().unwrap_or([0.0; 4]);
            for k in 0..4 {
                let w = ws[k] as f64 * norm;
                if w == 0.0 { continue; }
                let idx = js[k] as usize;
                let Some(jm) = palette.get(idx) else { continue; };
                for r in 0..4 {
                    for c in 0..4 {
                        m[r][c] += w * jm.0[r][c];
                    }
                }
            }
        }
        let blended = Mat4(m);
        positions.push(blended.transform_point(positions_bind[i]));
        if let (Some(nb), Some(out)) = (normals_bind, normals.as_mut()) {
            out.push(blended.transform_vector(nb[i]).normalized());
        }
        if let (Some(tb), Some(out)) = (tangents_bind, tangents.as_mut()) {
            let t = Vec3::new(tb[i][0] as f64, tb[i][1] as f64, tb[i][2] as f64);
            let tw = blended.transform_vector(t).normalized();
            out.push([tw.x as f32, tw.y as f32, tw.z as f32, tb[i][3]]);
        }
    }
    (positions, normals, tangents)
}

fn emit_indexed<'a, F: Clone + Fn(gltf::Buffer<'a>) -> Option<&'a [u8]>>(
    scene: &mut Scene,
    positions: &[Vec3],
    normals: Option<&[Vec3]>,
    uvs: &[[f32; 2]],
    uvs1: &[[f32; 2]],
    colors: &[[f32; 4]],
    tangents: Option<&[[f32; 4]]>,
    mode: gltf::mesh::Mode,
    reader: gltf::mesh::Reader<'a, 'a, F>,
    material_id: u32,
) {
    let indices: Vec<u32> = match reader.read_indices() {
        Some(iter) => iter.into_u32().collect(),
        None => (0..positions.len() as u32).collect(),
    };

    let get_uv    = |i: usize| -> [f32; 2] { uvs.get(i).copied().unwrap_or([0.0, 0.0]) };
    let get_uv1   = |i: usize| -> [f32; 2] { uvs1.get(i).copied().unwrap_or([0.0, 0.0]) };
    let get_color = |i: usize| -> [f32; 4] { colors.get(i).copied().unwrap_or([1.0, 1.0, 1.0, 1.0]) };
    let get_normal = |i: usize| -> Vec3 {
        normals.and_then(|n| n.get(i).copied()).unwrap_or(Vec3::new(0.0, 1.0, 0.0))
    };
    let get_tangent = |i: usize| -> [f32; 4] {
        tangents.and_then(|t| t.get(i).copied()).unwrap_or([1.0, 0.0, 0.0, 1.0])
    };
    let mk_vertex = |i: usize| -> Vertex {
        Vertex {
            position: positions[i],
            normal:   get_normal(i),
            uv:       get_uv(i),
            uv1:      get_uv1(i),
            color:    get_color(i),
            tangent:  get_tangent(i),
        }
    };

    // Non-triangle primitive modes emit into scene.lines / scene.points and
    // return early. Points/lines don't participate in shadow maps or PBR
    // shading; they render with the material's base color × vertex color.
    match mode {
        gltf::mesh::Mode::Points => {
            for &i in &indices {
                let idx = i as usize;
                if idx >= positions.len() { continue; }
                let p = positions[idx];
                scene.extend_bbox(p);
                scene.points.push(PointPrim { p: mk_vertex(idx), material_id });
            }
            return;
        }
        gltf::mesh::Mode::Lines => {
            for pair in indices.chunks_exact(2) {
                let (i0, i1) = (pair[0] as usize, pair[1] as usize);
                if i0 >= positions.len() || i1 >= positions.len() { continue; }
                scene.extend_bbox(positions[i0]); scene.extend_bbox(positions[i1]);
                scene.lines.push(LinePrim { a: mk_vertex(i0), b: mk_vertex(i1), material_id });
            }
            return;
        }
        gltf::mesh::Mode::LineStrip => {
            for w in indices.windows(2) {
                let (i0, i1) = (w[0] as usize, w[1] as usize);
                if i0 >= positions.len() || i1 >= positions.len() { continue; }
                scene.extend_bbox(positions[i0]); scene.extend_bbox(positions[i1]);
                scene.lines.push(LinePrim { a: mk_vertex(i0), b: mk_vertex(i1), material_id });
            }
            return;
        }
        gltf::mesh::Mode::LineLoop => {
            if indices.len() >= 2 {
                for w in indices.windows(2) {
                    let (i0, i1) = (w[0] as usize, w[1] as usize);
                    if i0 >= positions.len() || i1 >= positions.len() { continue; }
                    scene.extend_bbox(positions[i0]); scene.extend_bbox(positions[i1]);
                    scene.lines.push(LinePrim { a: mk_vertex(i0), b: mk_vertex(i1), material_id });
                }
                // Closing segment last → first.
                let (i0, i1) = (*indices.last().unwrap() as usize, indices[0] as usize);
                if i0 < positions.len() && i1 < positions.len() {
                    scene.lines.push(LinePrim { a: mk_vertex(i0), b: mk_vertex(i1), material_id });
                }
            }
            return;
        }
        gltf::mesh::Mode::Triangles | gltf::mesh::Mode::TriangleStrip | gltf::mesh::Mode::TriangleFan => {}
    }

    let tri_indices: Vec<[u32; 3]> = match mode {
        gltf::mesh::Mode::Triangles => indices
            .chunks_exact(3)
            .map(|c| [c[0], c[1], c[2]])
            .collect(),
        gltf::mesh::Mode::TriangleStrip => (0..indices.len().saturating_sub(2))
            .map(|i| if i & 1 == 0 {
                [indices[i], indices[i + 1], indices[i + 2]]
            } else {
                [indices[i + 1], indices[i], indices[i + 2]]
            })
            .collect(),
        gltf::mesh::Mode::TriangleFan => (1..indices.len().saturating_sub(1))
            .map(|i| [indices[0], indices[i], indices[i + 1]])
            .collect(),
        _ => unreachable!("non-triangle modes handled above"),
    };

    // Resolve per-face-vertex normals up front so both MikkTSpace and the emit
    // loop see the same values (flat face normal for prims without NORMAL).
    let face_vert_normal = |face: usize, vert: usize| -> Vec3 {
        let [ia, ib, ic] = tri_indices[face];
        if let Some(n) = normals {
            let i = [ia, ib, ic][vert] as usize;
            n[i]
        } else {
            let (a, b, c) = (positions[ia as usize], positions[ib as usize], positions[ic as usize]);
            Vec3::face_normal(a, b, c).unwrap_or(Vec3::new(0.0, 0.0, 1.0))
        }
    };

    // Tangent source strategy:
    //   * TANGENT attribute → per-vertex, index by (vertex index).
    //   * MikkTSpace → per-face-vertex, index by (face * 3 + vert). Only
    //     computed if the primitive has both UVs and normals AND at least
    //     one non-degenerate triangle; otherwise fall back to flat.
    //   * Flat (per-triangle UV derivative) — last resort when MikkT can't
    //     run (missing UVs).
    let mikkt_tangents: Option<Vec<[f32; 4]>> = if tangents.is_none() && !uvs.is_empty() {
        let mut geom = MikkTGeom {
            positions,
            uvs,
            tri_indices: &tri_indices,
            face_vert_normal: &face_vert_normal,
            tangents: vec![[0.0, 0.0, 1.0, 1.0]; tri_indices.len() * 3],
        };
        if mikktspace::generate_tangents(&mut geom) {
            Some(geom.tangents)
        } else {
            None
        }
    } else {
        None
    };

    for (face, [ia, ib, ic]) in tri_indices.iter().enumerate() {
        let (a, b, c) = (positions[*ia as usize], positions[*ib as usize], positions[*ic as usize]);
        let (uva, uvb, uvc) = (get_uv(*ia as usize), get_uv(*ib as usize), get_uv(*ic as usize));
        let (uv1a, uv1b, uv1c) = (get_uv1(*ia as usize), get_uv1(*ib as usize), get_uv1(*ic as usize));
        let (ca, cb, cc_col) = (get_color(*ia as usize), get_color(*ib as usize), get_color(*ic as usize));

        let (na, nb, nc) = (face_vert_normal(face, 0), face_vert_normal(face, 1), face_vert_normal(face, 2));

        if Vec3::face_normal(a, b, c).is_none() { continue; }

        let (ta, tb, tc) = if let Some(t) = tangents {
            (t[*ia as usize], t[*ib as usize], t[*ic as usize])
        } else if let Some(mt) = mikkt_tangents.as_deref() {
            (mt[face * 3], mt[face * 3 + 1], mt[face * 3 + 2])
        } else {
            let flat = compute_flat_tangent(a, b, c, uva, uvb, uvc, na);
            (flat, flat, flat)
        };

        scene.extend_bbox(a); scene.extend_bbox(b); scene.extend_bbox(c);
        scene.triangles.push(Triangle {
            vertices: [
                Vertex { position: a, normal: na, uv: uva, uv1: uv1a, color: ca,     tangent: ta },
                Vertex { position: b, normal: nb, uv: uvb, uv1: uv1b, color: cb,     tangent: tb },
                Vertex { position: c, normal: nc, uv: uvc, uv1: uv1c, color: cc_col, tangent: tc },
            ],
            material_id,
        });
    }
}

// MikkTSpace `Geometry` adapter over a primitive's per-face-vertex data.
// Tangents are written to `tangents` indexed by `face * 3 + vert`.
struct MikkTGeom<'a, N> {
    positions: &'a [Vec3],
    uvs: &'a [[f32; 2]],
    tri_indices: &'a [[u32; 3]],
    face_vert_normal: &'a N,
    tangents: Vec<[f32; 4]>,
}

impl<'a, N: Fn(usize, usize) -> Vec3> mikktspace::Geometry for MikkTGeom<'a, N> {
    fn num_faces(&self) -> usize { self.tri_indices.len() }
    fn num_vertices_of_face(&self, _face: usize) -> usize { 3 }
    fn position(&self, face: usize, vert: usize) -> [f32; 3] {
        let i = self.tri_indices[face][vert] as usize;
        let p = self.positions[i];
        [p.x as f32, p.y as f32, p.z as f32]
    }
    fn normal(&self, face: usize, vert: usize) -> [f32; 3] {
        let n = (self.face_vert_normal)(face, vert);
        [n.x as f32, n.y as f32, n.z as f32]
    }
    fn tex_coord(&self, face: usize, vert: usize) -> [f32; 2] {
        let i = self.tri_indices[face][vert] as usize;
        self.uvs.get(i).copied().unwrap_or([0.0, 0.0])
    }
    fn set_tangent_encoded(&mut self, tangent: [f32; 4], face: usize, vert: usize) {
        self.tangents[face * 3 + vert] = tangent;
    }
}

/// Per-triangle tangent from UV derivatives, orthonormalised against the
/// (already-unit) face normal. Returns `[t.x, t.y, t.z, w]` where `w = ±1`
/// is the bitangent handedness (matches glTF's TANGENT convention so the
/// shader path is the same as pre-shipped tangents).
/// Read a vec2 vertex-attribute accessor (TEXCOORD_N). Same dispatch as
/// `read_vec_f32` — needed because gltf-rs's typed texcoord reader
/// UNCONDITIONALLY divides u8/u16 by their max, which is wrong for
/// KHR_mesh_quantization assets where texcoords are stored as raw
/// non-normalized integers and dequantized by a paired KHR_texture_transform.
fn read_vec2_f32(accessor: &gltf::Accessor, loaded: &LoadedGltf) -> Result<Vec<[f32; 2]>, String> {
    read_vec_f32::<2>(accessor, loaded)
}

/// Read a vec3 vertex-attribute accessor (POSITION, NORMAL, TANGENT delta,
/// morph-target delta) as tightly-packed `[f32; 3]` per point, dispatching
/// on `KHR_mesh_quantization` component types. See `read_vec_f32` for the
/// full dequantization rules and the sparse-accessor caveat.
fn read_vec3_f32(accessor: &gltf::Accessor, loaded: &LoadedGltf) -> Result<Vec<[f32; 3]>, String> {
    read_vec_f32::<3>(accessor, loaded)
}

/// Read a vec4 vertex-attribute accessor (TANGENT). Same story as
/// `read_vec3_f32` — only shape and stride differ.
fn read_vec4_f32(accessor: &gltf::Accessor, loaded: &LoadedGltf) -> Result<Vec<[f32; 4]>, String> {
    read_vec_f32::<4>(accessor, loaded)
}

/// Read a vec-N vertex attribute as `[f32; N]` per element, dequantizing
/// per glTF 2.0 §3.6.2.2 for any of the componentTypes KHR_mesh_quantization
/// legalises for POSITION / NORMAL / TANGENT (BYTE, UNSIGNED_BYTE, SHORT,
/// UNSIGNED_SHORT — normalized) as well as the base `FLOAT` case.
///
///   normalized `u8`  →  `f = i / 255`
///   normalized `u16` →  `f = i / 65535`
///   normalized `i8`  →  `f = max(i / 127,  −1.0)`  (clamps −128 to −1)
///   normalized `i16` →  `f = max(i / 32767, −1.0)`
///   non-normalized   →  `f = i as f32`
///
/// gltf-rs's typed readers reinterpret bytes as `f32` and produce garbage
/// on quantized attributes; this helper is the single point of correctness.
///
/// Sparse accessors: not overlaid here. `KHR_mesh_quantization` + sparse
/// combined is spec-legal but essentially unheard-of in the wild (sparse
/// is mostly used for morph targets, which typically stay `F32`). Callers
/// hit gltf-rs's typed reader when the accessor is `F32` non-sparse and
/// this helper only for the quantized case — sparse-quantized will render
/// wrong; open an issue if you see it.
fn read_vec_f32<const N: usize>(accessor: &gltf::Accessor, loaded: &LoadedGltf) -> Result<Vec<[f32; N]>, String> {
    use gltf::accessor::DataType;
    let view = accessor.view().ok_or("attribute accessor missing bufferView")?;
    let buf = loaded.buffers.get(view.buffer().index())
        .ok_or("attribute buffer index out of range")?;
    let count = accessor.count();
    let dt = accessor.data_type();
    let comp_size = dt.size();
    let elem_size = comp_size * N;
    let stride = view.stride().unwrap_or(elem_size);
    let base = view.offset() + accessor.offset();
    let normalized = accessor.normalized();

    // Scale factor per glTF §3.6.2.2. Non-normalized integer paths use 1.0
    // and produce plain integer-to-float casts (rarely used for these
    // attributes but spec-legal).
    let scale = match (dt, normalized) {
        (DataType::I8,  true) => 1.0 / 127.0,
        (DataType::U8,  true) => 1.0 / 255.0,
        (DataType::I16, true) => 1.0 / 32767.0,
        (DataType::U16, true) => 1.0 / 65535.0,
        _ => 1.0,
    };
    let signed_clamp = normalized && matches!(dt, DataType::I8 | DataType::I16);

    let mut out: Vec<[f32; N]> = Vec::with_capacity(count);
    for i in 0..count {
        let o = base + i * stride;
        if o + elem_size > buf.len() {
            return Err("attribute read overruns buffer".into());
        }
        let mut elem = [0.0f32; N];
        for k in 0..N {
            let ko = o + k * comp_size;
            let f: f32 = match dt {
                DataType::F32 => f32::from_le_bytes(buf[ko..ko+4].try_into().unwrap()),
                DataType::I8  => (buf[ko] as i8 as f32) * scale,
                DataType::U8  => (buf[ko] as f32) * scale,
                DataType::I16 => i16::from_le_bytes(buf[ko..ko+2].try_into().unwrap()) as f32 * scale,
                DataType::U16 => u16::from_le_bytes(buf[ko..ko+2].try_into().unwrap()) as f32 * scale,
                DataType::U32 => u32::from_le_bytes(buf[ko..ko+4].try_into().unwrap()) as f32,
            };
            elem[k] = if signed_clamp { f.max(-1.0) } else { f };
        }
        out.push(elem);
    }
    Ok(out)
}

fn compute_flat_tangent(
    p0: Vec3, p1: Vec3, p2: Vec3,
    uv0: [f32; 2], uv1: [f32; 2], uv2: [f32; 2],
    n: Vec3,
) -> [f32; 4] {
    let e1 = p1 - p0;
    let e2 = p2 - p0;
    let du1x = (uv1[0] - uv0[0]) as f64;
    let du1y = (uv1[1] - uv0[1]) as f64;
    let du2x = (uv2[0] - uv0[0]) as f64;
    let du2y = (uv2[1] - uv0[1]) as f64;
    let det = du1x * du2y - du2x * du1y;
    if det.abs() < 1e-12 {
        // Degenerate UVs — pick an arbitrary tangent orthogonal to N so the
        // shader still has a valid basis (normal map contributions cancel
        // out visually in that case anyway).
        let axis = if n.x.abs() < 0.9 { Vec3::new(1.0, 0.0, 0.0) } else { Vec3::new(0.0, 1.0, 0.0) };
        let t = n.cross(axis).normalized();
        return [t.x as f32, t.y as f32, t.z as f32, 1.0];
    }
    let inv = 1.0 / det;
    let t_raw = Vec3::new(
        (e1.x * du2y - e2.x * du1y) * inv,
        (e1.y * du2y - e2.y * du1y) * inv,
        (e1.z * du2y - e2.z * du1y) * inv,
    );
    let b_raw = Vec3::new(
        (e2.x * du1x - e1.x * du2x) * inv,
        (e2.y * du1x - e1.y * du2x) * inv,
        (e2.z * du1x - e1.z * du2x) * inv,
    );
    // Gram-Schmidt: strip the N component from T.
    let dot_nt = n.dot(t_raw);
    let t = Vec3::new(t_raw.x - n.x * dot_nt, t_raw.y - n.y * dot_nt, t_raw.z - n.z * dot_nt)
        .normalized();
    // Handedness: sign of (N × T) · B — determines whether the bitangent
    // should be flipped for the (T, B, N) frame to match the UV winding.
    let sign = if n.cross(t).dot(b_raw) < 0.0 { -1.0 } else { 1.0 };
    [t.x as f32, t.y as f32, t.z as f32, sign as f32]
}

// Mesh-side shadow builder.
//
// The shade-time half — the `ShadowMap` depth map with its PCF/PCSS sampling,
// the `LightShadow` dispatch, and `BiasParams` — is shared from maquette-core
// and re-exported here so the rasterizer's `crate::shadow::…` paths are
// unchanged. Only the frustum *builder* stays local: the mesh frames each light
// view at the scene centre (a simple, robust approximation), whereas gltf aims
// along the light's own direction with its real spot cone. The mesh also
// filters occluders (transparent triangles don't cast) via `is_occluder`.

use crate::config::{LightKind, ShadowMapConfig};
use crate::math::{Mat4, Vec3};
use crate::parser::Triangle;
use crate::shading::ResolvedLight;

pub use maquette_core::shadow::{build_cube, BiasParams, LightShadow, ShadowMap};

/// Build one shadow (single frustum or cube) per light — None for lights that
/// don't cast. `bc`/`br` frame each view; `is_occluder` filters caster tris.
pub fn build_shadow_maps(
    triangles: &[Triangle],
    lights: &[ResolvedLight],
    bc: Vec3,
    br: f64,
    up: Vec3,
    cfg: &ShadowMapConfig,
    is_occluder: &dyn Fn(&Triangle) -> bool,
) -> Vec<Option<LightShadow>> {
    let res = cfg.resolution.clamp(64, 4096);
    lights
        .iter()
        .map(|light| {
            if !light.cast_shadow {
                return None;
            }
            let mut ls = if cfg.omni && light.kind != LightKind::Directional {
                LightShadow::Cube(build_cube(light.vector, br, res))
            } else {
                LightShadow::Single(build_one(light, bc, br, up, res))
            };
            // Depth pass with the occluder filter (transparent tris don't cast).
            match &mut ls {
                LightShadow::Single(m) => {
                    for tri in triangles {
                        if is_occluder(tri) {
                            m.splat_tri(tri.vertices);
                        }
                    }
                }
                LightShadow::Cube(faces) => faces.iter_mut().for_each(|m| {
                    for tri in triangles {
                        if is_occluder(tri) {
                            m.splat_tri(tri.vertices);
                        }
                    }
                }),
            }
            Some(ls)
        })
        .collect()
}

fn build_one(light: &ResolvedLight, bc: Vec3, br: f64, up: Vec3, res: usize) -> ShadowMap {
    // Pick an up vector for the light view that isn't parallel to its forward axis.
    let forward = if light.kind == LightKind::Directional {
        // `vector` points toward the light; the light shines along -vector.
        light.vector.normalized().scale(-1.0)
    } else {
        (bc - light.vector).normalized()
    };
    let up_aux = if forward.cross(up).length() > 1e-3 {
        up
    } else {
        Vec3::new(1.0, 0.0, 0.0)
    };

    if light.kind == LightKind::Directional {
        let eye = bc - forward.scale(br * 2.0);
        let view = Mat4::look_at(eye, bc, up_aux);
        let eye_dist = br * 2.0;
        ShadowMap::new(
            view,
            true,
            br * 1.05,
            0.0,
            (eye_dist - br * 1.2).max(1e-4),
            eye_dist + br * 1.2,
            res,
            forward,
            eye,
        )
    } else {
        let eye = light.vector;
        let dist = (bc - eye).length().max(br * 0.1);
        let view = Mat4::look_at(eye, bc, up_aux);
        ShadowMap::new(
            view,
            false,
            0.0,
            (br * 1.1 / dist).clamp(0.05, 10.0),
            (dist - br * 1.2).max(dist * 0.01),
            dist + br * 1.2,
            res,
            forward,
            eye,
        )
    }
}

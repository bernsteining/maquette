
use crate::config::{LightKind, ShadowMapConfig};
use crate::math::Vec3;
use crate::parser::Triangle;
use crate::shading::ResolvedLight;

pub use maquette_core::shadow::{build_cube, BiasParams, LightShadow, ShadowMap};

fn caster_tris(tri: &Triangle) -> impl Iterator<Item = [Vec3; 3]> {
    let [c, a, b] = tri.vertices;
    let quad = if tri.splat {
        const HALF: f64 = 0.886_226_925_452_758;
        let (ra, rb) = (a.sub(c).scale(HALF), b.sub(c).scale(HALF));
        let q = [c.add(ra).add(rb), c.sub(ra).add(rb), c.sub(ra).sub(rb), c.add(ra).sub(rb)];
        Some([[q[0], q[1], q[2]], [q[0], q[2], q[3]]])
    } else {
        None
    };
    let single = if tri.splat { None } else { Some(tri.vertices) };
    quad.into_iter().flatten().chain(single)
}

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
    let mut casters: Vec<[Vec3; 3]> = Vec::new();
    for tri in triangles.iter().filter(|t| is_occluder(t)) {
        casters.extend(caster_tris(tri));
    }
    lights
        .iter()
        .map(|light| {
            if !light.cast_shadow {
                return None;
            }
            let mut ls = if cfg.omni && light.kind != LightKind::Directional {
                LightShadow::Cube(build_cube(light.vector, br, res))
            } else if light.kind == LightKind::Directional {
                LightShadow::Single(ShadowMap::directional(light.vector.normalized().scale(-1.0), bc, br, up, res))
            } else {
                LightShadow::Single(ShadowMap::perspective(light.vector, bc, (bc - light.vector).normalized(), None, bc, br, up, res))
            };
            ls.add_casters(&casters);
            Some(ls)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Camera setup, projection modes, and view transforms
// ---------------------------------------------------------------------------

use crate::config::RenderConfig;
use crate::math::Vec3;

pub(crate) struct ViewParams {
    pub(crate) camera: Vec3,
    pub(crate) center: Vec3,
    pub(crate) up: Vec3,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Projection {
    Ortho, Cabinet, Cavalier, Fisheye, Stereographic, Curvilinear,
    Cylindrical, Pannini, TinyPlanet, Perspective,
}

// ---------------------------------------------------------------------------
// Camera resolution
// ---------------------------------------------------------------------------

pub(crate) fn resolve_config_view(config: &RenderConfig, bc: Vec3, br: f64) -> ViewParams {
    let center = if config.auto_center
        && config.center[0] == 0.0
        && config.center[1] == 0.0
        && config.center[2] == 0.0
    {
        bc
    } else {
        Vec3::new(config.center[0], config.center[1], config.center[2])
    };

    let up = Vec3::new(config.up[0], config.up[1], config.up[2]);

    // Cartesian camera overrides spherical when explicitly set
    let camera = if let Some(cam) = config.camera {
        let raw_camera = Vec3::new(cam[0], cam[1], cam[2]);
        let dist = raw_camera.sub(center).length();
        let dist = if dist < 1e-6 { br * 3.0 } else { dist };
        axonometric_camera(center, dist, &config.projection).unwrap_or(raw_camera)
    } else {
        // Spherical coordinates (default): build orthonormal basis from `up`
        let az = config.azimuth.to_radians();
        let el = config.elevation.to_radians();
        let dist = config.distance.filter(|&d| d > 0.0).unwrap_or(br * 3.0);
        axonometric_camera(center, dist, &config.projection).unwrap_or_else(|| {
            let arbitrary = if up.x.abs() < 0.9 { Vec3::new(1.0, 0.0, 0.0) } else { Vec3::new(0.0, 1.0, 0.0) };
            let right = up.cross(arbitrary).normalized();
            let forward = right.cross(up).normalized();
            let offset = right.scale(el.cos() * az.cos())
                .add(forward.scale(el.cos() * az.sin()))
                .add(up.scale(el.sin()));
            center.add(offset.scale(dist))
        })
    };

    ViewParams { camera, center, up }
}

pub(crate) fn named_view(name: &str, bc: Vec3, br: f64) -> ViewParams {
    let dist = br * 3.0;
    let z_up = Vec3::new(0.0, 0.0, 1.0);
    let front = Vec3::new(bc.x, bc.y - dist, bc.z);

    if let Some(cam) = axonometric_camera(bc, dist, name) {
        return ViewParams { camera: cam, center: bc, up: z_up };
    }

    let (camera, up) = match name {
        "back"         => (Vec3::new(bc.x, bc.y + dist, bc.z), z_up),
        "right" | "side" => (Vec3::new(bc.x + dist, bc.y, bc.z), z_up),
        "left"         => (Vec3::new(bc.x - dist, bc.y, bc.z), z_up),
        "top"          => (Vec3::new(bc.x, bc.y, bc.z + dist), Vec3::new(0.0, 1.0, 0.0)),
        "bottom"       => (Vec3::new(bc.x, bc.y, bc.z - dist), Vec3::new(0.0, -1.0, 0.0)),
        _              => (front, z_up), // front, cabinet, cavalier, unknown
    };
    ViewParams { camera, center: bc, up }
}

pub(crate) fn spherical_camera(center: Vec3, dist: f64, elev: f64, azim: f64) -> Vec3 {
    Vec3::new(
        center.x + dist * elev.cos() * azim.cos(),
        center.y + dist * elev.cos() * azim.sin(),
        center.z + dist * elev.sin(),
    )
}

fn axonometric_camera(center: Vec3, dist: f64, projection: &str) -> Option<Vec3> {
    match projection {
        "isometric" => Some(spherical_camera(center, dist,
            (1.0_f64 / 2.0_f64.sqrt()).atan(), std::f64::consts::FRAC_PI_4)),
        "dimetric" => Some(spherical_camera(center, dist,
            (1.0_f64 / 8.0_f64.sqrt()).asin(), std::f64::consts::FRAC_PI_4)),
        "trimetric" => Some(spherical_camera(center, dist,
            25.0_f64.to_radians(), 30.0_f64.to_radians())),
        "military" => Some(spherical_camera(center, dist,
            2.0_f64.sqrt().atan(), std::f64::consts::FRAC_PI_4)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Projection setup and application
// ---------------------------------------------------------------------------

#[inline]
fn ortho_scale(config: &RenderConfig, view: &ViewParams, vw: f64, vh: f64, br: f64) -> f64 {
    if config.auto_fit {
        vh.min(vw) * 0.45 / br
    } else {
        let dist = (view.camera - view.center).length();
        let half_extent = dist * (config.fov.to_radians() / 2.0).tan();
        vh.min(vw) / (2.0 * half_extent)
    }
}

#[inline]
pub(crate) fn resolve_projection(s: &str) -> Projection {
    match s {
        "orthographic" | "isometric" | "dimetric" | "trimetric" | "military" => Projection::Ortho,
        "cabinet" => Projection::Cabinet,
        "cavalier" => Projection::Cavalier,
        "fisheye" => Projection::Fisheye,
        "stereographic" => Projection::Stereographic,
        "curvilinear" => Projection::Curvilinear,
        "cylindrical" => Projection::Cylindrical,
        "pannini" => Projection::Pannini,
        "tiny-planet" => Projection::TinyPlanet,
        _ => Projection::Perspective,
    }
}

// ---------------------------------------------------------------------------
// Precomputed projection setup — avoids recomputing scale/d/aspect per triangle
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub(crate) enum ProjectionSetup {
    Ortho { s: f64, hw: f64, hh: f64 },
    Perspective { sx: f64, sy: f64, hw: f64, hh: f64 },
    Cabinet { s: f64, hw: f64, hh: f64, fc: f64, fs: f64, cd: f64 },
    Fisheye { f: f64, hw: f64, hh: f64 },
    Stereographic { f: f64, hw: f64, hh: f64 },
    Curvilinear { d: f64, k: f64, aspect: f64, hw: f64, hh: f64 },
    Cylindrical { f: f64, hw: f64, hh: f64 },
    Pannini { f: f64, hw: f64, hh: f64 },
    TinyPlanet { f: f64, hw: f64, hh: f64 },
}

/// Focal length for equidistant projections (fisheye, cylindrical).
#[inline]
fn focal_equidistant(config: &RenderConfig, view: &ViewParams, hw: f64, hh: f64, br: f64) -> f64 {
    let half_size = hw.min(hh);
    if config.auto_fit {
        let dist = (view.camera - view.center).length();
        half_size * 0.45 / (br / dist).atan()
    } else {
        half_size / (config.fov.to_radians().max(0.01) / 2.0)
    }
}

/// Focal length for stereographic projections (stereographic, pannini).
#[inline]
fn focal_stereographic(config: &RenderConfig, view: &ViewParams, hw: f64, hh: f64, br: f64) -> f64 {
    let half_size = hw.min(hh);
    if config.auto_fit {
        let dist = (view.camera - view.center).length();
        half_size * 0.45 / (2.0 * ((br / dist).atan() / 2.0).tan())
    } else {
        half_size / (2.0 * (config.fov.to_radians().max(0.01) / 4.0).tan())
    }
}

pub(crate) fn setup_projection(proj: Projection, config: &RenderConfig, view: &ViewParams, vw: f64, vh: f64, br: f64) -> ProjectionSetup {
    let hw = vw / 2.0;
    let hh = vh / 2.0;
    match proj {
        Projection::Ortho => {
            ProjectionSetup::Ortho { s: ortho_scale(config, view, vw, vh, br), hw, hh }
        }
        Projection::Cabinet | Projection::Cavalier => {
            let s = ortho_scale(config, view, vw, vh, br);
            let factor = if proj == Projection::Cabinet { 0.5 } else { 1.0 };
            let correction = 1.0 / (1.0 + factor * std::f64::consts::FRAC_1_SQRT_2);
            let s = if config.auto_fit { s * correction } else { s };
            let angle = std::f64::consts::FRAC_PI_4;
            let fc = factor * angle.cos();
            let fs = factor * angle.sin();
            let cd = (view.camera - view.center).length();
            ProjectionSetup::Cabinet { s, hw, hh, fc, fs, cd }
        }
        Projection::Fisheye => {
            ProjectionSetup::Fisheye { f: focal_equidistant(config, view, hw, hh, br), hw, hh }
        }
        Projection::Stereographic => {
            ProjectionSetup::Stereographic { f: focal_stereographic(config, view, hw, hh, br), hw, hh }
        }
        Projection::Curvilinear => {
            let fov_rad = config.fov.to_radians();
            let k = 0.3_f64;
            let d = if config.auto_fit {
                let dist = (view.camera - view.center).length();
                let required = (br / dist).atan();
                let current = fov_rad / 2.0;
                1.0 / required.max(current).tan() / (1.0 + k)
            } else {
                1.0 / (fov_rad / 2.0).tan()
            };
            ProjectionSetup::Curvilinear { d, k, aspect: vw / vh, hw, hh }
        }
        Projection::Cylindrical => {
            ProjectionSetup::Cylindrical { f: focal_equidistant(config, view, hw, hh, br), hw, hh }
        }
        Projection::Pannini => {
            ProjectionSetup::Pannini { f: focal_stereographic(config, view, hw, hh, br), hw, hh }
        }
        Projection::TinyPlanet => {
            let half_size = hw.min(hh);
            let f = if config.auto_fit {
                half_size * 0.45 / std::f64::consts::PI
            } else {
                half_size / config.fov.to_radians().max(0.01)
            };
            ProjectionSetup::TinyPlanet { f, hw, hh }
        }
        Projection::Perspective => {
            let fov_rad = config.fov.to_radians();
            let d = if config.auto_fit {
                let dist = (view.camera - view.center).length();
                let required = (br / dist).atan();
                let current = fov_rad / 2.0;
                1.0 / required.max(current).tan()
            } else {
                1.0 / (fov_rad / 2.0).tan()
            };
            let aspect = vw / vh;
            ProjectionSetup::Perspective { sx: d * hw / aspect, sy: d * hh, hw, hh }
        }
    }
}

#[inline(always)]
pub(crate) fn apply_projection(setup: &ProjectionSetup, cam: &[Vec3; 3]) -> [(f64, f64); 3] {
    match *setup {
        ProjectionSetup::Ortho { s, hw, hh } => [
            (hw + cam[0].x * s, hh - cam[0].y * s),
            (hw + cam[1].x * s, hh - cam[1].y * s),
            (hw + cam[2].x * s, hh - cam[2].y * s),
        ],
        ProjectionSetup::Perspective { sx, sy, hw, hh } => {
            let mut pts = [(0.0, 0.0); 3];
            for (i, v) in cam.iter().enumerate() {
                let z = (-v.z).max(0.001);
                pts[i] = (hw + v.x * sx / z, hh - v.y * sy / z);
            }
            pts
        }
        ProjectionSetup::Cabinet { s, hw, hh, fc, fs, cd } => {
            let mut pts = [(0.0, 0.0); 3];
            for (i, v) in cam.iter().enumerate() {
                let rd = -v.z - cd;
                pts[i] = (hw + (v.x + rd * fc) * s, hh - (v.y + rd * fs) * s);
            }
            pts
        }
        ProjectionSetup::Fisheye { f, hw, hh } => {
            let mut pts = [(0.0, 0.0); 3];
            for (i, v) in cam.iter().enumerate() {
                let xy = (v.x * v.x + v.y * v.y).sqrt();
                if xy < 1e-10 { pts[i] = (hw, hh); continue; }
                let r = xy.atan2((-v.z).max(1e-6)) * f;
                pts[i] = (hw + r * (v.x / xy), hh - r * (v.y / xy));
            }
            pts
        }
        ProjectionSetup::Stereographic { f, hw, hh } => {
            let mut pts = [(0.0, 0.0); 3];
            for (i, v) in cam.iter().enumerate() {
                let xy = (v.x * v.x + v.y * v.y).sqrt();
                if xy < 1e-10 { pts[i] = (hw, hh); continue; }
                let r = 2.0 * f * (xy.atan2((-v.z).max(1e-6)) / 2.0).tan();
                pts[i] = (hw + r * (v.x / xy), hh - r * (v.y / xy));
            }
            pts
        }
        ProjectionSetup::Curvilinear { d, k, aspect, hw, hh } => {
            let mut pts = [(0.0, 0.0); 3];
            for (i, v) in cam.iter().enumerate() {
                let z = (-v.z).max(0.001);
                let px = (v.x * d) / (z * aspect);
                let py = (v.y * d) / z;
                let f = 1.0 + k * (px * px + py * py);
                pts[i] = (hw + px * f * hw, hh - py * f * hh);
            }
            pts
        }
        ProjectionSetup::Cylindrical { f, hw, hh } => {
            let mut pts = [(0.0, 0.0); 3];
            for (i, v) in cam.iter().enumerate() {
                let horiz = (v.x * v.x + v.z * v.z).sqrt();
                if horiz < 1e-10 { pts[i] = (hw, hh); continue; }
                pts[i] = (hw + v.x.atan2((-v.z).max(1e-6)) * f, hh - (v.y / horiz) * f);
            }
            pts
        }
        ProjectionSetup::Pannini { f, hw, hh } => {
            let mut pts = [(0.0, 0.0); 3];
            for (i, v) in cam.iter().enumerate() {
                let z = (-v.z).max(1e-6);
                let horiz = (v.x * v.x + z * z).sqrt();
                if horiz < 1e-10 { pts[i] = (hw, hh); continue; }
                let theta = v.x.atan2(z);
                let scale = 2.0 / (1.0 + theta.cos());
                pts[i] = (hw + theta.sin() * scale * f, hh - (v.y / horiz) * scale * f);
            }
            pts
        }
        ProjectionSetup::TinyPlanet { f, hw, hh } => {
            let pi = std::f64::consts::PI;
            let mut pts = [(0.0, 0.0); 3];
            for (i, v) in cam.iter().enumerate() {
                let xy = (v.x * v.x + v.y * v.y).sqrt();
                if xy < 1e-10 {
                    let r = if -v.z > 0.0 { pi * f } else { 0.0 };
                    pts[i] = (hw + r, hh);
                    continue;
                }
                let r = (pi - xy.atan2(-v.z)) * f;
                pts[i] = (hw + r * (v.x / xy), hh - r * (v.y / xy));
            }
            pts
        }
    }
}

//! Punctual lights — the shape KHR_lights_punctual assumes and the shadow
//! map builder in [`crate::shadow`] consumes. Lives here (not in a plugin
//! crate) so shadow-map construction can stay format-agnostic: any plugin
//! that provides a `Vec<PunctualLight>` can drive it.

use crate::math::Vec3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LightKind {
    Directional,
    Point,
    Spot,
}

#[derive(Clone)]
pub struct PunctualLight {
    pub kind: LightKind,
    /// World-space position (Point / Spot only).
    pub position: Vec3,
    /// World-space direction (Directional / Spot). Unit vector.
    pub direction: Vec3,
    /// Linear RGB tint × intensity — pre-multiplied at scene-build time.
    pub color: [f32; 3],
    /// Attenuation range for Point / Spot (`0.0` = unbounded).
    pub range: f32,
    /// Spot cone precomputed cosines. Zero for Directional / Point.
    pub inner_cone_cos: f32,
    pub outer_cone_cos: f32,
    /// Whether this light should cast a shadow. Consumed by
    /// `build_shadow_maps` — `false` returns `None` for the slot.
    pub cast_shadow: bool,
}

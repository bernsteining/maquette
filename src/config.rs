use crate::color::{parse_hex_color, srgb_to_linear};
use crate::math::parse_f64_fast;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// JSON parser — minimal recursive-descent, no dependencies
// ---------------------------------------------------------------------------

pub(crate) struct JsonParser<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    pub fn new(s: &'a str) -> Self {
        Self { b: s.as_bytes(), pos: 0 }
    }

    #[inline]
    fn skip_ws(&mut self) {
        while self.pos < self.b.len() {
            match self.b[self.pos] {
                b' ' | b'\t' | b'\n' | b'\r' => self.pos += 1,
                _ => break,
            }
        }
    }

    #[inline]
    fn peek(&self) -> u8 {
        if self.pos < self.b.len() { self.b[self.pos] } else { 0 }
    }

    #[inline]
    fn expect(&mut self, ch: u8) -> Result<(), String> {
        self.skip_ws();
        if self.peek() == ch {
            self.pos += 1;
            Ok(())
        } else {
            Err("unexpected token".into())
        }
    }

    /// After parsing a value inside an object or array, eat an optional comma.
    /// Returns true if the closing character `close` was found (but does NOT consume it).
    #[inline]
    fn eat_comma_or(&mut self, close: u8) -> bool {
        self.skip_ws();
        if self.peek() == b',' {
            self.pos += 1;
            false
        } else {
            self.peek() == close
        }
    }

    /// Parse a JSON string, returning a borrowed slice (zero-copy).
    /// Only works for strings without escape sequences.
    fn parse_str(&mut self) -> Result<&'a str, String> {
        self.skip_ws();
        if self.peek() != b'"' { return Err("expected string".into()); }
        self.pos += 1;
        let start = self.pos;
        while self.pos < self.b.len() && self.b[self.pos] != b'"' {
            if self.b[self.pos] == b'\\' {
                self.pos += 2;
                continue;
            }
            self.pos += 1;
        }
        // Safety: input is &str so always valid UTF-8
        let s = unsafe { std::str::from_utf8_unchecked(&self.b[start..self.pos]) };
        self.pos += 1; // closing "
        Ok(s)
    }

    /// Parse a JSON string into an owned String.
    fn parse_string(&mut self) -> Result<String, String> {
        Ok(self.parse_str()?.to_string())
    }

    fn parse_f64(&mut self) -> Result<f64, String> {
        self.skip_ws();
        let start = self.pos;
        while self.pos < self.b.len() {
            match self.b[self.pos] {
                b'0'..=b'9' | b'.' | b'-' | b'+' | b'e' | b'E' => self.pos += 1,
                _ => break,
            }
        }
        // Safety: input is &str, digits/signs/dots are ASCII
        let s = unsafe { std::str::from_utf8_unchecked(&self.b[start..self.pos]) };
        parse_f64_fast(s).ok_or_else(|| "invalid number".into())
    }

    fn parse_bool(&mut self) -> Result<bool, String> {
        self.skip_ws();
        if self.b[self.pos..].starts_with(b"true") {
            self.pos += 4;
            Ok(true)
        } else if self.b[self.pos..].starts_with(b"false") {
            self.pos += 5;
            Ok(false)
        } else {
            Err("expected bool".into())
        }
    }

    fn parse_usize(&mut self) -> Result<usize, String> {
        self.parse_f64().map(|v| v as usize)
    }

    fn is_null(&mut self) -> bool {
        self.skip_ws();
        if self.b[self.pos..].starts_with(b"null") {
            self.pos += 4;
            true
        } else {
            false
        }
    }

    fn parse_f64_3(&mut self) -> Result<[f64; 3], String> {
        self.expect(b'[')?;
        let a = self.parse_f64()?; self.expect(b',')?;
        let b = self.parse_f64()?; self.expect(b',')?;
        let c = self.parse_f64()?;
        self.expect(b']')?;
        Ok([a, b, c])
    }

    fn parse_f64_4(&mut self) -> Result<[f64; 4], String> {
        self.expect(b'[')?;
        let a = self.parse_f64()?; self.expect(b',')?;
        let b = self.parse_f64()?; self.expect(b',')?;
        let c = self.parse_f64()?; self.expect(b',')?;
        let d = self.parse_f64()?;
        self.expect(b']')?;
        Ok([a, b, c, d])
    }

    fn parse_string_vec(&mut self) -> Result<Vec<String>, String> {
        self.expect(b'[')?;
        let mut v = Vec::new();
        self.skip_ws();
        if self.peek() != b']' {
            loop {
                v.push(self.parse_string()?);
                if self.eat_comma_or(b']') { break; }
            }
        }
        self.expect(b']')?;
        Ok(v)
    }

    fn parse_string_map(&mut self) -> Result<HashMap<String, String>, String> {
        self.expect(b'{')?;
        let mut m = HashMap::new();
        self.skip_ws();
        if self.peek() != b'}' {
            loop {
                let k = self.parse_string()?;
                self.expect(b':')?;
                let v = self.parse_string()?;
                m.insert(k, v);
                if self.eat_comma_or(b'}') { break; }
            }
        }
        self.expect(b'}')?;
        Ok(m)
    }

    /// Skip any JSON value (string, number, bool, null, array, object).
    fn skip_value(&mut self) -> Result<(), String> {
        self.skip_ws();
        match self.peek() {
            b'"' => { self.parse_str()?; }
            b'{' => {
                self.pos += 1;
                self.skip_ws();
                if self.peek() != b'}' {
                    loop {
                        self.parse_str()?; // key
                        self.expect(b':')?;
                        self.skip_value()?;
                        if self.eat_comma_or(b'}') { break; }
                    }
                }
                self.expect(b'}')?;
            }
            b'[' => {
                self.pos += 1;
                self.skip_ws();
                if self.peek() != b']' {
                    loop {
                        self.skip_value()?;
                        if self.eat_comma_or(b']') { break; }
                    }
                }
                self.expect(b']')?;
            }
            b't' => { self.parse_bool()?; }
            b'f' => { self.parse_bool()?; }
            b'n' => { if !self.is_null() { return Err("expected null".into()); } }
            _ => { self.parse_f64()?; }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

/// Ambient lighting configuration: flat scalar or hemisphere (sky/ground gradient).
#[derive(Clone)]
pub struct AmbientConfig {
    pub intensity: f64,
    pub sky: String,
    pub ground: String,
}

impl Default for AmbientConfig {
    fn default() -> Self {
        Self {
            intensity: 0.3,
            sky: "#ccd4e0".into(),
            ground: "#d4ccc4".into(),
        }
    }
}

/// Bloom post-processing configuration.
#[derive(Clone)]
pub struct BloomConfig {
    pub threshold: f64,
    pub intensity: f64,
    pub radius: usize,
}

impl Default for BloomConfig {
    fn default() -> Self {
        Self { threshold: 0.8, intensity: 0.3, radius: 10 }
    }
}

/// SSAO (screen-space ambient occlusion) configuration.
#[derive(Clone)]
pub struct SsaoConfig {
    pub samples: usize,
    pub radius: f64,
    pub bias: f64,
    pub strength: f64,
}

impl Default for SsaoConfig {
    fn default() -> Self {
        Self { samples: 16, radius: 0.5, bias: 0.025, strength: 1.0 }
    }
}

/// Glow post-processing configuration.
#[derive(Clone)]
pub struct GlowConfig {
    pub color: String,
    pub intensity: f64,
    pub radius: usize,
}

impl Default for GlowConfig {
    fn default() -> Self {
        Self { color: "#ffffff".into(), intensity: 0.5, radius: 15 }
    }
}

/// Sharpening post-processing configuration.
#[derive(Clone)]
pub struct SharpenConfig {
    pub strength: f64,
}

impl Default for SharpenConfig {
    fn default() -> Self {
        Self { strength: 0.5 }
    }
}

/// Fake subsurface scattering configuration.
#[derive(Clone)]
pub struct SssConfig {
    pub intensity: f64,
    pub power: f64,
    pub distortion: f64,
}

impl Default for SssConfig {
    fn default() -> Self {
        Self { intensity: 0.5, power: 3.0, distortion: 0.2 }
    }
}

/// Ground shadow configuration.
#[derive(Clone)]
pub struct ShadowConfig {
    pub opacity: f64,
    pub color: String,
}

impl Default for ShadowConfig {
    fn default() -> Self {
        Self { opacity: 0.3, color: "#000000".into() }
    }
}

/// Annotation configuration.
#[derive(Clone)]
pub struct AnnotationConfig {
    pub groups: Vec<String>,
    pub color: String,
    pub font_size: f64,
    pub offset: f64,
}

impl Default for AnnotationConfig {
    fn default() -> Self {
        Self { groups: Vec::new(), color: "#333333".into(), font_size: 12.0, offset: 40.0 }
    }
}

/// Outline (edge detection) configuration.
#[derive(Clone)]
pub struct OutlineConfig {
    pub color: String,
    pub width: f64,
}

impl Default for OutlineConfig {
    fn default() -> Self {
        Self { color: "#000000".into(), width: 2.0 }
    }
}

/// Triangle edge stroke configuration.
#[derive(Clone)]
pub struct StrokeConfig {
    pub color: String,
    pub width: f64,
}

impl Default for StrokeConfig {
    fn default() -> Self {
        Self { color: "none".into(), width: 0.0 }
    }
}

/// Wireframe overlay configuration.
#[derive(Clone)]
pub struct WireframeConfig {
    pub color: String,
    pub width: f64,
}

impl Default for WireframeConfig {
    fn default() -> Self {
        Self { color: String::new(), width: 1.0 }
    }
}

/// Fresnel rim lighting configuration.
#[derive(Clone)]
pub struct FresnelConfig {
    pub intensity: f64,
    pub power: f64,
}

impl Default for FresnelConfig {
    fn default() -> Self {
        Self { intensity: 0.3, power: 5.0 }
    }
}

/// Turntable multi-view configuration.
#[derive(Clone)]
pub struct TurntableConfig {
    pub iterations: usize,
    pub elevation: f64,
}

impl Default for TurntableConfig {
    fn default() -> Self {
        Self { iterations: 0, elevation: 40.0 }
    }
}

/// Tone mapping configuration.
#[derive(Clone)]
pub struct ToneMappingConfig {
    pub method: String,
    pub exposure: f64,
}

impl Default for ToneMappingConfig {
    fn default() -> Self {
        Self { method: String::new(), exposure: 1.0 }
    }
}

/// Per-group appearance overrides for OBJ group highlighting.
#[derive(Clone, Default)]
pub struct GroupAppearance {
    pub color: Option<String>,
    pub specular: Option<f64>,
    pub shininess: Option<f64>,
    pub ambient: Option<f64>,
    pub stroke: Option<String>,
    pub stroke_width: Option<f64>,
    pub opacity: Option<f64>,
    pub name: Option<String>,
}

/// A highlight value: either a plain hex color string or a full appearance override.
#[derive(Clone)]
pub enum GroupStyle {
    Color(String),
    Full(GroupAppearance),
}

impl GroupStyle {
    pub fn color_hex(&self) -> Option<&str> {
        match self {
            GroupStyle::Color(c) => Some(c.as_str()),
            GroupStyle::Full(a) => a.color.as_deref(),
        }
    }

    pub fn appearance(&self) -> Option<&GroupAppearance> {
        match self {
            GroupStyle::Color(_) => None,
            GroupStyle::Full(a) => Some(a),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum LightKind {
    Directional,
    Positional,
}

#[derive(Clone)]
pub struct LightDef {
    pub kind: LightKind,
    pub vector: [f64; 3],
    pub color: (f32, f32, f32),
    pub intensity: f64,
}

impl Default for LightDef {
    fn default() -> Self {
        Self {
            kind: LightKind::Directional,
            vector: [1.0, 2.0, 3.0],
            color: (1.0f32, 1.0f32, 1.0f32),
            intensity: 1.0,
        }
    }
}

pub struct RenderConfig {
    pub camera: Option<[f64; 3]>,
    pub center: [f64; 3],
    pub up: [f64; 3],
    pub width: f64,
    pub height: f64,
    pub projection: String,
    pub fov: f64,
    pub color: String,
    pub stroke: StrokeConfig,
    pub light_dir: [f64; 3],
    pub ambient: AmbientConfig,
    pub background: String,
    pub mode: String,
    pub cull_backface: bool,
    pub wireframe: WireframeConfig,
    pub auto_center: bool,
    pub auto_fit: bool,
    pub materials: HashMap<String, String>,
    pub highlight: HashMap<String, GroupStyle>,
    pub shadow: Option<ShadowConfig>,
    pub views: Option<Vec<String>>,
    pub grid_labels: bool,
    pub turntable: TurntableConfig,
    pub color_map: String,
    pub color_map_palette: Vec<String>,
    pub overhang_angle: f64,
    pub smooth: bool,
    pub specular: f64,
    pub shininess: f64,
    pub outline: Option<OutlineConfig>,
    pub clip_plane: Option<[f64; 4]>,
    pub explode: f64,
    pub debug: bool,
    pub debug_color: String,
    pub antialias: usize,
    pub fxaa: bool,
    pub azimuth: f64,
    pub elevation: f64,
    pub distance: Option<f64>,
    pub ssao: Option<SsaoConfig>,
    pub bloom: Option<BloomConfig>,
    pub glow: Option<GlowConfig>,
    pub sharpen: Option<SharpenConfig>,
    pub scalar_function: String,
    pub vertex_smoothing: usize,
    pub opacity: f64,
    pub xray_opacity: f64,
    pub gamma_correction: bool,
    pub fresnel: FresnelConfig,
    pub lights: Vec<LightDef>,
    pub tone_mapping: ToneMappingConfig,
    pub shading: String,
    pub gooch_warm: String,
    pub gooch_cool: String,
    pub cel_bands: usize,
    pub sss: Option<SssConfig>,
    pub annotations: Option<AnnotationConfig>,
    pub point_size: f64,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            camera: None,
            center: [0.0, 0.0, 0.0],
            up: [0.0, 0.0, 1.0],
            width: 500.0,
            height: 500.0,
            projection: "perspective".into(),
            fov: 45.0,
            color: "#4488cc".into(),
            stroke: StrokeConfig::default(),
            light_dir: [1.0, 2.0, 3.0],
            ambient: AmbientConfig { intensity: 0.15, ..AmbientConfig::default() },
            background: "#f0f0f0".into(),
            mode: "solid".into(),
            cull_backface: true,
            wireframe: WireframeConfig::default(),
            auto_center: true,
            auto_fit: true,
            materials: HashMap::new(),
            highlight: HashMap::new(),
            shadow: None,
            views: None,
            grid_labels: true,
            turntable: TurntableConfig::default(),
            color_map: String::new(),
            color_map_palette: Vec::new(),
            overhang_angle: 45.0,
            smooth: true,
            specular: 0.2,
            shininess: 32.0,
            outline: None,
            clip_plane: None,
            explode: 0.0,
            debug: false,
            debug_color: "#cc2222".into(),
            antialias: 1,
            fxaa: true,
            azimuth: 0.0,
            elevation: 0.0,
            distance: None,
            ssao: None,
            bloom: None,
            glow: None,
            sharpen: None,
            scalar_function: String::new(),
            vertex_smoothing: 4,
            opacity: 1.0,
            xray_opacity: 0.1,
            gamma_correction: true,
            fresnel: FresnelConfig::default(),
            lights: Vec::new(),
            tone_mapping: ToneMappingConfig::default(),
            shading: String::new(),
            gooch_warm: "#ffcc44".into(),
            gooch_cool: "#4466cc".into(),
            cel_bands: 4,
            sss: None,
            annotations: None,
            point_size: 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Parsing functions
// ---------------------------------------------------------------------------

fn parse_group_appearance(p: &mut JsonParser) -> Result<GroupAppearance, String> {
    let mut ga = GroupAppearance::default();
    p.expect(b'{')?;
    p.skip_ws();
    if p.peek() != b'}' {
        loop {
            let key = p.parse_str()?;
            p.expect(b':')?;
            match key {
                "color" => ga.color = Some(p.parse_string()?),
                "specular" => ga.specular = Some(p.parse_f64()?),
                "shininess" => ga.shininess = Some(p.parse_f64()?),
                "ambient" => ga.ambient = Some(p.parse_f64()?),
                "stroke" => ga.stroke = Some(p.parse_string()?),
                "stroke_width" => ga.stroke_width = Some(p.parse_f64()?),
                "opacity" => ga.opacity = Some(p.parse_f64()?),
                "name" => ga.name = Some(p.parse_string()?),
                _ => { p.skip_value()?; }
            }
            if p.eat_comma_or(b'}') { break; }
        }
    }
    p.expect(b'}')?;
    Ok(ga)
}

fn parse_group_style(p: &mut JsonParser) -> Result<GroupStyle, String> {
    p.skip_ws();
    if p.peek() == b'"' {
        Ok(GroupStyle::Color(p.parse_string()?))
    } else {
        Ok(GroupStyle::Full(parse_group_appearance(p)?))
    }
}

fn parse_highlight_map(p: &mut JsonParser) -> Result<HashMap<String, GroupStyle>, String> {
    p.expect(b'{')?;
    let mut m = HashMap::new();
    p.skip_ws();
    if p.peek() != b'}' {
        loop {
            let k = p.parse_string()?;
            p.expect(b':')?;
            let v = parse_group_style(p)?;
            m.insert(k, v);
            if p.eat_comma_or(b'}') { break; }
        }
    }
    p.expect(b'}')?;
    Ok(m)
}

fn parse_light_def(p: &mut JsonParser) -> Result<LightDef, String> {
    let mut light = LightDef::default();
    p.expect(b'{')?;
    p.skip_ws();
    if p.peek() != b'}' {
        loop {
            let key = p.parse_str()?;
            p.expect(b':')?;
            match key {
                "type" => {
                    let s = p.parse_str()?;
                    light.kind = if s == "positional" || s == "point" {
                        LightKind::Positional
                    } else {
                        LightKind::Directional
                    };
                }
                "vector" => light.vector = p.parse_f64_3()?,
                "color" => {
                    let hex = p.parse_str()?;
                    let (r, g, b) = parse_hex_color(hex);
                    light.color = (srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b));
                }
                "intensity" => light.intensity = p.parse_f64()?,
                _ => { p.skip_value()?; }
            }
            if p.eat_comma_or(b'}') { break; }
        }
    }
    p.expect(b'}')?;
    Ok(light)
}

fn parse_light_vec(p: &mut JsonParser) -> Result<Vec<LightDef>, String> {
    p.skip_ws();
    if p.peek() == b'{' {
        // Single light object — wrap in a vec.
        return Ok(vec![parse_light_def(p)?]);
    }
    p.expect(b'[')?;
    let mut v = Vec::new();
    p.skip_ws();
    if p.peek() != b']' {
        loop {
            v.push(parse_light_def(p)?);
            if p.eat_comma_or(b']') { break; }
        }
    }
    p.expect(b']')?;
    Ok(v)
}

/// Parse an optional config object: `{...}` → `Some(parsed)`, `true` → `Some(default)`, `false` → `None`.
macro_rules! parse_optional_object {
    ($p:expr, $T:ty, { $($key:literal => $field:ident = $method:ident),* $(,)? }) => {{
        $p.skip_ws();
        if $p.peek() == b'{' {
            let mut v = <$T>::default();
            $p.expect(b'{')?; $p.skip_ws();
            while $p.peek() != b'}' {
                let k = $p.parse_str()?; $p.expect(b':')?;
                match k {
                    $( $key => v.$field = $p.$method()?, )*
                    _ => { $p.skip_value()?; }
                }
                $p.eat_comma_or(b'}');
            }
            $p.expect(b'}')?;
            Some(v)
        } else {
            if $p.parse_bool()? { Some(<$T>::default()) } else { None }
        }
    }};
}

/// Parse a compound config: `{...}` → parsed object, or a shorthand value → applied to a specific field.
macro_rules! parse_object_or {
    ($p:expr, $T:ty, { $($key:literal => $field:ident = $method:ident),* $(,)? }, |$v:ident, $pp:ident| $shorthand:expr) => {{
        $p.skip_ws();
        if $p.peek() == b'{' {
            let mut $v = <$T>::default();
            $p.expect(b'{')?; $p.skip_ws();
            while $p.peek() != b'}' {
                let k = $p.parse_str()?; $p.expect(b':')?;
                match k {
                    $( $key => $v.$field = $p.$method()?, )*
                    _ => { $p.skip_value()?; }
                }
                $p.eat_comma_or(b'}');
            }
            $p.expect(b'}')?;
            $v
        } else {
            let mut $v = <$T>::default();
            let $pp = &mut *$p;
            $shorthand;
            $v
        }
    }};
}

fn parse_render_config(p: &mut JsonParser) -> Result<RenderConfig, String> {
    let mut cfg = RenderConfig::default();
    let mut smooth_explicit = false;
    p.expect(b'{')?;
    p.skip_ws();
    if p.peek() != b'}' {
        loop {
            let key = p.parse_str()?;
            p.expect(b':')?;
            match key {
                "camera" => cfg.camera = if p.is_null() { None } else { Some(p.parse_f64_3()?) },
                "center" => cfg.center = p.parse_f64_3()?,
                "up" => cfg.up = p.parse_f64_3()?,
                "width" => cfg.width = p.parse_f64()?,
                "height" => cfg.height = p.parse_f64()?,
                "projection" => cfg.projection = p.parse_string()?,
                "fov" => cfg.fov = p.parse_f64()?,
                "color" => cfg.color = p.parse_string()?,
                "stroke" => cfg.stroke = parse_object_or!(p, StrokeConfig, {
                    "color" => color = parse_string,
                    "width" => width = parse_f64,
                }, |v, p| v.color = p.parse_string()?),
                "light_dir" => cfg.light_dir = p.parse_f64_3()?,
                "ambient" => cfg.ambient = parse_object_or!(p, AmbientConfig, {
                    "intensity" => intensity = parse_f64,
                    "sky" => sky = parse_string,
                    "ground" => ground = parse_string,
                }, |v, p| v.intensity = p.parse_f64()?),
                "background" => cfg.background = p.parse_string()?,
                "mode" => cfg.mode = p.parse_string()?,
                "cull_backface" => cfg.cull_backface = p.parse_bool()?,
                "wireframe" => cfg.wireframe = parse_object_or!(p, WireframeConfig, {
                    "color" => color = parse_string,
                    "width" => width = parse_f64,
                }, |v, p| v.color = p.parse_string()?),
                "auto_center" => cfg.auto_center = p.parse_bool()?,
                "auto_fit" => cfg.auto_fit = p.parse_bool()?,
                "materials" => cfg.materials = p.parse_string_map()?,
                "highlight" => cfg.highlight = parse_highlight_map(p)?,
                "ground_shadow" => cfg.shadow = parse_optional_object!(p, ShadowConfig, {
                    "opacity" => opacity = parse_f64,
                    "color" => color = parse_string,
                }),
                "views" => cfg.views = if p.is_null() { None } else { Some(p.parse_string_vec()?) },
                "grid_labels" => cfg.grid_labels = p.parse_bool()?,
                "turntable" => cfg.turntable = parse_object_or!(p, TurntableConfig, {
                    "iterations" => iterations = parse_usize,
                    "elevation" => elevation = parse_f64,
                }, |v, p| v.iterations = p.parse_usize()?),
                "color_map" => cfg.color_map = p.parse_string()?,
                "color_map_palette" => cfg.color_map_palette = p.parse_string_vec()?,
                "overhang_angle" => cfg.overhang_angle = p.parse_f64()?,
                "smooth" => { cfg.smooth = p.parse_bool()?; smooth_explicit = true; },
                "specular" => cfg.specular = p.parse_f64()?,
                "shininess" => cfg.shininess = p.parse_f64()?,
                "outline" => cfg.outline = parse_optional_object!(p, OutlineConfig, {
                    "color" => color = parse_string,
                    "width" => width = parse_f64,
                }),
                "clip_plane" => cfg.clip_plane = if p.is_null() { None } else { Some(p.parse_f64_4()?) },
                "explode" => cfg.explode = p.parse_f64()?,
                "debug" => cfg.debug = p.parse_bool()?,
                "debug_color" => cfg.debug_color = p.parse_string()?,
                "antialias" => cfg.antialias = p.parse_usize()?,
                "fxaa" => cfg.fxaa = p.parse_bool()?,
                "azimuth" => cfg.azimuth = p.parse_f64()?,
                "elevation" => cfg.elevation = p.parse_f64()?,
                "distance" => cfg.distance = if p.is_null() { None } else { Some(p.parse_f64()?) },
                "ssao" => cfg.ssao = parse_optional_object!(p, SsaoConfig, {
                    "samples" => samples = parse_usize,
                    "radius" => radius = parse_f64,
                    "bias" => bias = parse_f64,
                    "strength" => strength = parse_f64,
                }),
                "bloom" => cfg.bloom = parse_optional_object!(p, BloomConfig, {
                    "threshold" => threshold = parse_f64,
                    "intensity" => intensity = parse_f64,
                    "radius" => radius = parse_usize,
                }),
                "glow" => cfg.glow = parse_optional_object!(p, GlowConfig, {
                    "color" => color = parse_string,
                    "intensity" => intensity = parse_f64,
                    "radius" => radius = parse_usize,
                }),
                "sharpen" => cfg.sharpen = parse_optional_object!(p, SharpenConfig, {
                    "strength" => strength = parse_f64,
                }),
                "scalar_function" => cfg.scalar_function = p.parse_string()?,
                "vertex_smoothing" => cfg.vertex_smoothing = p.parse_usize()?,
                "opacity" => cfg.opacity = p.parse_f64()?,
                "xray_opacity" => cfg.xray_opacity = p.parse_f64()?,
                "gamma_correction" => cfg.gamma_correction = p.parse_bool()?,
                "fresnel" => cfg.fresnel = parse_object_or!(p, FresnelConfig, {
                    "intensity" => intensity = parse_f64,
                    "power" => power = parse_f64,
                }, |v, p| v.intensity = p.parse_f64()?),
                "lights" => cfg.lights = parse_light_vec(p)?,
                "tone_mapping" => cfg.tone_mapping = parse_object_or!(p, ToneMappingConfig, {
                    "method" => method = parse_string,
                    "type" => method = parse_string,
                    "exposure" => exposure = parse_f64,
                }, |v, p| v.method = p.parse_string()?),
                "shading" => cfg.shading = p.parse_string()?,
                "gooch_warm" => cfg.gooch_warm = p.parse_string()?,
                "gooch_cool" => cfg.gooch_cool = p.parse_string()?,
                "cel_bands" => cfg.cel_bands = p.parse_usize()?,
                "sss" => cfg.sss = parse_optional_object!(p, SssConfig, {
                    "intensity" => intensity = parse_f64,
                    "power" => power = parse_f64,
                    "distortion" => distortion = parse_f64,
                }),
                "annotations" => cfg.annotations = parse_optional_object!(p, AnnotationConfig, {
                    "groups" => groups = parse_string_vec,
                    "color" => color = parse_string,
                    "font_size" => font_size = parse_f64,
                    "offset" => offset = parse_f64,
                }),
                "point_size" => cfg.point_size = p.parse_f64()?,
                _ => { p.skip_value()?; }
            }
            if p.eat_comma_or(b'}') { break; }
        }
    }
    p.expect(b'}')?;
    // Flat shading implies no smooth normals unless the user explicitly set smooth.
    if cfg.shading == "flat" && !smooth_explicit {
        cfg.smooth = false;
    }
    Ok(cfg)
}

/// Entry point: parse a JSON config string into a RenderConfig.
pub fn parse_config_json(s: &str) -> Result<RenderConfig, String> {
    let mut p = JsonParser::new(s);
    parse_render_config(&mut p)
}

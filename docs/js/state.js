const GLTF_EXTS = new Set(["glb", "gltf", "blg"]);
const isGltf = (name) => GLTF_EXTS.has(ext(name || ""));
const MOL_FMTS = { pdb: "pdb", cif: "cif", mmcif: "mmcif", bcif: "bcif", xyz: "xyz" };
const isMolExt = (name) => ext(name || "") in MOL_FMTS;

const PROJ = ["perspective","orthographic","isometric","dimetric","trimetric","military",
  "cabinet","cavalier","fisheye","stereographic","curvilinear","cylindrical","pannini","tiny-planet"];

const SCHEMA = [
  { s: "Point cloud (PLY)", open: true, when: () => model._ext === "ply" && !model._mol && !model.scad, fields: [
    { k: "point_size", label: "Point size / radius (0 = auto)", t: "num", def: 0, omitIf: v => v === 0 },
    { k: "point_neighbors", label: "Neighbors k (higher = fewer holes)", t: "num", def: 12, omitIf: v => v === 12 },
    { k: "point_boundary", label: "Boundary cut angle ° (0 = off)", t: "num", def: 60, omitIf: v => v === 60 },
  ]},
  { s: "Molecule (molfig)", open: true, when: () => model._mol, fields: [
    { k: "mol_representation", label: "Representation", t: "sel", def: "cartoon",
      opts: [
        ["default", "default"],
        ["ball-and-stick", "ball-and-stick"],
        ["spacefill", "spacefill"],
        ["cartoon", "cartoon"],
        ["ribbon", "ribbon"],
        ["backbone", "backbone"],
        ["surface", "molecular-surface"],
      ], recompile: "mol",
      onSet: v => { if (v === "surface" && state.mol_quality !== "highest") { state.mol_quality = "highest"; _rebuildForm(); } } },
    { k: "mol_color_theme", label: "Color theme", t: "sel", def: "element-symbol",
      opts: [
        ["element-symbol", "element-symbol"],
        ["chain-id", "chain-id"],
        ["entity-id", "entity-id"],
        ["plddt-confidence", "pLDDT confidence"],
        ["partial-charges", "partial charges"],
      ], recompile: "mol" },
    { k: "mol_carbon_color", label: "Carbon color", t: "sel", def: "element-symbol",
      opts: [
        ["chain-id", "chain-id (per-chain)"],
        ["element-symbol", "element-symbol (grey)"],
        ["operator-name", "operator-name"],
      ], recompile: "mol" },
    { k: "mol_quality", label: "Mesh quality", t: "sel", def: "medium",
      opts: [["lowest","lowest"],["low","low"],["medium","medium"],["high","high"],["higher","higher"],["highest","highest"]], recompile: "mol" },
    { k: "mol_infer_bonds", label: "Infer bonds (small molecules)", t: "bool", def: true, recompile: "mol" },
    { k: "mol_radius_scale", label: "Radius scale", t: "rng", def: 1.0, min: 0.1, max: 3, step: 0.05, recompile: "mol" },
    { k: "mol_atom_radius", label: "Atom radius (Å)", t: "num", def: 0.28, recompile: "mol" },
    { k: "mol_bond_radius", label: "Bond radius (Å)", t: "num", def: 0.12, recompile: "mol" },
    { k: "mol_assembly", label: "Assembly id (biological unit)", t: "txt", def: "1", allowBlank: true, recompile: "mol" },
    { k: "mol_center", label: "Center on load", t: "bool", def: true, recompile: "mol" },
  ]},
  { s: "Molecule advanced (molfig)", open: false, when: () => model._mol, fields: [
    { k: "mol_alt_loc", label: "Alt-loc filter", t: "txt", def: "", allowBlank: true, recompile: "mol" },
    { k: "mol_block_header", label: "CIF block header filter", t: "txt", def: "", allowBlank: true, recompile: "mol" },
    { k: "mol_block_index", label: "CIF block index (-1 = default)", t: "num", def: -1, omitIf: v => v === -1, recompile: "mol" },
    { k: "mol_decimate", label: "Decimate (0 = off)", t: "rng", def: 0, min: 0, max: 1, step: 0.05, recompile: "mol" },
    { k: "mol_sphere_detail", label: "Sphere detail (icosphere subdiv)", t: "num", def: 2, recompile: "mol" },
    { k: "mol_ribbon_radius", label: "Ribbon radius", t: "num", def: 0.2, recompile: "mol" },
    { k: "mol_ribbon_width", label: "Ribbon width", t: "num", def: 0.55, recompile: "mol" },
    { k: "mol_helix_profile", label: "Helix profile", t: "sel", def: "elliptical",
      opts: [["elliptical","elliptical"],["rounded","rounded"],["square","square"]], recompile: "mol" },
    { k: "mol_round_cap", label: "Round cap", t: "bool", def: false, recompile: "mol" },
    { k: "mol_sheet_arrow_factor", label: "Sheet arrow factor", t: "num", def: 1.5, recompile: "mol" },
    { k: "mol_tubular_helices", label: "Tubular helices", t: "bool", def: false, recompile: "mol" },
    { k: "mol_linear_segments", label: "Linear segments", t: "num", def: 8, recompile: "mol" },
    { k: "mol_radial_segments", label: "Radial segments", t: "num", def: 16, recompile: "mol" },
  ]},
  { s: "Camera & viewport", fields: [
    { k: "_cam", label: "Camera mode", t: "sel", def: "cartesian", init: "spherical", opts: [["cartesian","Cartesian (x,y,z)"],["spherical","Spherical"]] },
    { k: "camera", label: "Position", t: "vec", def: [3,3,3], when: s => s._cam === "cartesian" },
    { k: "azimuth", label: "Azimuth °", t: "num", def: 0, init: 180, when: s => s._cam === "spherical" },
    { k: "elevation", label: "Elevation °", t: "num", def: 0, when: s => s._cam === "spherical" },
    { k: "distance", label: "Distance (0 = auto)", t: "num", def: 0, omitIf: v => v === 0, when: s => s._cam === "spherical" },
    { k: "center", label: "Look-at center", t: "vec", def: [0,0,0] },
    { k: "up", label: "Up vector", t: "vec", def: [0,0,1], init: [0,1,0] },
    { k: "projection", label: "Projection", t: "sel", def: "perspective", opts: PROJ.map(p => [p, p]) },
    { k: "fov", label: "Field of view °", t: "num", def: 45 },
    { k: "zoom", label: "Zoom", t: "rng", def: 1, init: 1.4, min: 0.3, max: 4, step: 0.05 },
    { k: "pan", label: "Pan [right, up]", t: "vec", def: [0,0] },
    { k: "auto_center", label: "Auto-center", t: "bool", def: true },
    { k: "auto_fit", label: "Auto-fit to viewport", t: "bool", def: true },
    { k: "background", label: "Background", t: "col", def: "#f0f0f0" },
    { k: "_bgNone", label: "Transparent background", t: "bool", def: false },
    { k: "width", label: "Render width px", t: "num", def: 0, noExport: true },
    { k: "height", label: "Render height px", t: "num", def: 0, noExport: true },
  ]},

  { s: "Material", fields: [
    { k: "color", label: "Model color", t: "col", def: "#4488cc" },
    { k: "opacity", label: "Opacity", t: "rng", def: 1, min: 0, max: 1, step: 0.01 },
    { k: "specular", label: "Specular", t: "rng", def: 0.2, min: 0, max: 1, step: 0.01 },
    { k: "shininess", label: "Shininess", t: "num", def: 32 },
    { k: "smooth", label: "Smooth shading", t: "bool", def: true },
    { k: "scad_smooth_normals", label: "SCAD smooth normals", t: "bool", def: false, recompile: "scad" },
    { k: "gamma_correction", label: "Gamma correction", t: "bool", def: true },
    { k: "cull_backface", label: "Back-face culling", t: "bool", def: true },
  ]},

  { s: "Shading model", fields: [
    { k: "shading", label: "Model", t: "sel", def: "", opts: [["","Blinn–Phong"],["gooch","Gooch"],["cel","Cel"],["flat","Flat"],["normal","Normal map"]] },
    { k: "gooch_warm", label: "Gooch warm", t: "col", def: "#ffcc44", when: s => s.shading === "gooch" },
    { k: "gooch_cool", label: "Gooch cool", t: "col", def: "#4466cc", when: s => s.shading === "gooch" },
    { k: "cel_bands", label: "Cel bands", t: "num", def: 4, when: s => s.shading === "cel" },
  ]},

  { s: "Render mode", fields: [
    { k: "mode", label: "Mode", t: "sel", def: "solid", opts: [["solid","Solid"],["wireframe","Wireframe"],["solid+wireframe","Solid + wireframe"],["x-ray","X-ray"]] },
    { k: "xray_opacity", label: "X-ray opacity", t: "rng", def: 0.1, min: 0, max: 1, step: 0.01, when: s => s.mode === "x-ray" },
    { k: "stroke", label: "Edge stroke", t: "grp", toggle: true, def: { color: "#000000", width: 1 }, fields: [
      { k: "color", label: "Color", t: "col", def: "#000000" },
      { k: "width", label: "Width", t: "num", def: 1 },
    ]},
    { k: "wireframe", label: "Wireframe style", t: "grp", toggle: true, def: { color: "#000000", width: 1 }, fields: [
      { k: "color", label: "Color", t: "col", def: "#000000" },
      { k: "width", label: "Width", t: "num", def: 1 },
    ]},
  ]},

  { s: "Lighting", fields: [
    { k: "light_dir", label: "Light direction", t: "vec", def: [1,2,3] },
    { k: "ambient", label: "Ambient", t: "rng", def: 0.15, min: 0, max: 1, step: 0.01, when: s => !s._hemi.__on },
    { k: "_hemi", label: "Hemisphere ambient", t: "grp", toggle: true, def: { intensity: 0.3, sky: "#ccd4e0", ground: "#d4ccc4" }, fields: [
      { k: "intensity", label: "Intensity", t: "rng", def: 0.3, min: 0, max: 1, step: 0.01 },
      { k: "sky", label: "Sky", t: "col", def: "#ccd4e0" },
      { k: "ground", label: "Ground", t: "col", def: "#d4ccc4" },
    ]},
    { k: "fresnel", label: "Fresnel rim", t: "grp", toggle: false, def: { intensity: 0.3, power: 5 }, fields: [
      { k: "intensity", label: "Intensity", t: "rng", def: 0.3, min: 0, max: 1, step: 0.01 },
      { k: "power", label: "Power", t: "num", def: 5 },
    ]},
    { k: "tone_mapping", label: "Tone mapping", t: "grp", toggle: false, def: { method: "", exposure: 1 }, fields: [
      { k: "method", label: "Method", t: "sel", def: "", opts: [["","None"],["aces","ACES"],["reinhard","Reinhard"]] },
      { k: "exposure", label: "Exposure", t: "rng", def: 1, min: 0, max: 4, step: 0.05 },
    ]},
    { k: "sss", label: "Subsurface scattering", t: "grp", toggle: true, bool: true, def: { intensity: 0.5, power: 3, distortion: 0.2 }, fields: [
      { k: "intensity", label: "Intensity", t: "num", def: 0.5 },
      { k: "power", label: "Power", t: "num", def: 3 },
      { k: "distortion", label: "Distortion", t: "num", def: 0.2 },
    ]},
    { k: "lights", label: "Extra lights", t: "lights", def: [] },
  ]},

  { s: "Color mapping", fields: [
    { k: "color_map", label: "Map", t: "sel", def: "", opts: [["","Off"],["overhang","Overhang"],["curvature","Curvature"],["scalar","Scalar"],["ply_scalar","PLY scalar"]] },
    { k: "overhang_angle", label: "Overhang angle °", t: "num", def: 45, when: s => s.color_map === "overhang" },
    { k: "scalar_function", label: "Scalar function", t: "txt", def: "", when: s => s.color_map === "scalar" },
    { k: "vertex_smoothing", label: "Vertex smoothing 0–4", t: "num", def: 4, when: s => s.color_map !== "" },
    { k: "color_map_palette", label: "Palette", t: "palette", def: [], when: s => s.color_map === "curvature" || s.color_map === "scalar" || s.color_map === "ply_scalar" },
  ]},

  { s: "Outlines", fields: [
    { k: "outline", label: "Silhouette outline", t: "grp", toggle: true, bool: true, def: { color: "#000000", width: 2 }, fields: [
      { k: "color", label: "Color", t: "col", def: "#000000" },
      { k: "width", label: "Width", t: "num", def: 2 },
    ]},
  ]},

  { s: "Shadows", fields: [
    { k: "ground_shadow", label: "Ground shadow", t: "grp", toggle: true, bool: true, def: { opacity: 0.3, color: "#000000" }, fields: [
      { k: "opacity", label: "Opacity", t: "rng", def: 0.3, min: 0, max: 1, step: 0.01 },
      { k: "color", label: "Color", t: "col", def: "#000000" },
    ]},
    { k: "shadows", label: "Cast shadows (self-shadowing)", t: "grp", toggle: true, bool: true, def: {
        per_pixel: false, light_size: 0, strength: 1, softness: 1, color: "", resolution: 512, omni: false, bias: 0.0008, normal_bias: 2, slope_bias: 1 },
      fields: [
        { k: "per_pixel", label: "Per-pixel", t: "bool", def: false },
        { k: "light_size", label: "Light size (soft)", t: "num", def: 0 },
        { k: "strength", label: "Strength", t: "rng", def: 1, min: 0, max: 1, step: 0.01 },
        { k: "softness", label: "Softness", t: "num", def: 1 },
        { k: "color", label: "Tint (blank = none)", t: "col", def: "", allowBlank: true },
        { k: "resolution", label: "Resolution", t: "num", def: 512 },
        { k: "omni", label: "Omnidirectional", t: "bool", def: false },
        { k: "bias", label: "Bias", t: "num", def: 0.0008 },
        { k: "normal_bias", label: "Normal bias", t: "num", def: 2 },
        { k: "slope_bias", label: "Slope bias", t: "num", def: 1 },
      ]},
  ]},

  { s: "Post-processing", fields: [
    { k: "antialias", label: "Antialiasing", t: "sel", def: 1, num: true, opts: [[0,"Off"],[1,"FXAA"],[2,"SSAA ×2"],[4,"SSAA ×4"]] },
    { k: "ssao", label: "Ambient occlusion", t: "grp", toggle: true, bool: true, def: { samples: 16, radius: 0.5, bias: 0.025, strength: 1 }, fields: [
      { k: "samples", label: "Samples", t: "num", def: 16 },
      { k: "radius", label: "Radius", t: "num", def: 0.5 },
      { k: "bias", label: "Bias", t: "num", def: 0.025 },
      { k: "strength", label: "Strength", t: "rng", def: 1, min: 0, max: 2, step: 0.05 },
    ]},
    { k: "bloom", label: "Bloom", t: "grp", toggle: true, bool: true, def: { threshold: 0.8, intensity: 0.3, radius: 10 }, fields: [
      { k: "threshold", label: "Threshold", t: "rng", def: 0.8, min: 0, max: 1, step: 0.01 },
      { k: "intensity", label: "Intensity", t: "rng", def: 0.3, min: 0, max: 2, step: 0.05 },
      { k: "radius", label: "Radius", t: "num", def: 10 },
    ]},
    { k: "glow", label: "Glow", t: "grp", toggle: true, bool: true, def: { color: "#ffffff", intensity: 0.5, radius: 15 }, fields: [
      { k: "color", label: "Color", t: "col", def: "#ffffff" },
      { k: "intensity", label: "Intensity", t: "rng", def: 0.5, min: 0, max: 2, step: 0.05 },
      { k: "radius", label: "Radius", t: "num", def: 15 },
    ]},
    { k: "sharpen", label: "Sharpen", t: "grp", toggle: true, bool: true, def: { strength: 0.5 }, fields: [
      { k: "strength", label: "Strength", t: "rng", def: 0.5, min: 0, max: 2, step: 0.05 },
    ]},
  ]},

  { s: "Geometry & clipping", fields: [
    { k: "clip", label: "Clip plane", t: "grp", toggle: true, def: {
        source: "camera", plane: [0, 0, 1, 0], depth: 0.5, keep: "far", cap: true,
        hatch: false, hstyle: "lines", hangle: 45, hspacing: 6, hwidth: 0.6, hcolor: "#333333" },
      build: "clip", fields: [
        { k: "source", label: "From", t: "sel", def: "camera", opts: [["camera","Camera"],["x","X axis"],["y","Y axis"],["z","Z axis"],["plane","Plane (a,b,c,d)"]] },
        { k: "plane", label: "Plane a, b, c, d", t: "vec", def: [0,0,1,0], when: (s,l) => l.source === "plane" },
        { k: "depth", label: "Depth", t: "rng", def: 0.5, min: 0, max: 1, step: 0.01, when: (s,l) => l.source !== "plane" },
        { k: "keep", label: "Keep", t: "sel", def: "far", opts: [["far","Far half"],["near","Near half"]] },
        { k: "cap", label: "Cap cross-section", t: "bool", def: true },
        { k: "hatch", label: "Hatch cap", t: "bool", def: false },
        { k: "hstyle", label: "Hatch style", t: "sel", def: "lines", opts: [["lines","Lines"],["cross","Cross"],["crosses","Crosses"]], when: (s,l) => l.hatch },
        { k: "hangle", label: "Hatch angle °", t: "num", def: 45, when: (s,l) => l.hatch },
        { k: "hspacing", label: "Hatch spacing", t: "num", def: 6, when: (s,l) => l.hatch },
        { k: "hwidth", label: "Hatch width", t: "num", def: 0.6, when: (s,l) => l.hatch },
        { k: "hcolor", label: "Hatch color", t: "col", def: "#333333", when: (s,l) => l.hatch },
      ]},
    { k: "explode", label: "Explode", t: "rng", def: 0, min: 0, max: 1, step: 0.02 },
    { k: "decimate", label: "Decimate", t: "rng", def: 0, min: 0, max: 1, step: 0.02 },
  ]},

  { s: "Multi-view", fields: [
    { k: "views", label: "Grid views", t: "views", def: [], opts: ["front","back","left","right","top","bottom","isometric"] },
    { k: "grid_labels", label: "Grid labels", t: "bool", def: true, when: s => s.views.length > 0 },
    { k: "turntable", label: "Turntable", t: "grp", toggle: true, def: { iterations: 6, elevation: 40 }, build: "turntable", fields: [
      { k: "iterations", label: "Frames", t: "num", def: 6 },
      { k: "elevation", label: "Elevation °", t: "num", def: 40 },
    ]},
  ]},

  { s: "OBJ groups", fields: [
    { k: "materials", label: "Materials (name → color)", t: "map", def: [] },
    { k: "highlight", label: "Highlight (group → appearance)", t: "map", rich: true, def: [] },
    { k: "annotations", label: "Annotations", t: "grp", toggle: true, bool: true, def: { color: "#333333", font_size: 12, offset: 40 }, fields: [
      { k: "color", label: "Color", t: "col", def: "#333333" },
      { k: "font_size", label: "Font size", t: "num", def: 12 },
      { k: "offset", label: "Offset", t: "num", def: 40 },
    ]},
  ]},

  { s: "Diagnostics", fields: [
    { k: "debug", label: "Debug overlay", t: "bool", def: false },
    { k: "debug_color", label: "Debug color", t: "col", def: "#cc2222", when: s => s.debug },
  ]},
];

const GLTF_SCHEMA = [
  { s: "Camera & viewport", fields: [
    { k: "_cam", label: "Camera mode", t: "sel", def: "cartesian",
      opts: [["cartesian", "Cartesian (x,y,z)"], ["spherical", "Spherical (az/el/dist)"]] },
    { k: "camera",    label: "Position [x,y,z]", t: "vec", def: [2.5, 1.5, 2.5],
      when: s => s._cam === "cartesian" },
    { k: "azimuth",   label: "Azimuth °",   t: "num", def: 30, when: s => s._cam === "spherical" },
    { k: "elevation", label: "Elevation °", t: "num", def: 20, when: s => s._cam === "spherical" },
    { k: "distance",  label: "Distance (0 = auto)", t: "num", def: 0,
      omitIf: v => v === 0, when: s => s._cam === "spherical" },
    { k: "center",     label: "Look-at",          t: "vec", def: [0, 0, 0] },
    { k: "up",         label: "Up",               t: "vec", def: [0, 1, 0] },
    { k: "fov",        label: "Field of view °",  t: "num", def: 40 },
    { k: "auto_center", label: "Auto-center on bbox", t: "bool", def: true },
    { k: "auto_fit",    label: "Auto-fit distance",   t: "bool", def: true },
    { k: "camera_auto_use", label: "Prefer glTF-authored camera when present", t: "bool", def: true },
    { k: "camera_name",  label: "Named camera (from glTF, blank = ignore)", t: "txt", def: "", allowBlank: true },
    { k: "camera_index", label: "Camera index (-1 = ignore)", t: "num", def: -1, omitIf: v => v === -1 },
    { k: "scene_index",  label: "Scene index (-1 = default)", t: "num", def: -1, omitIf: v => v === -1 },
    { k: "cull_backface", label: "Back-face culling", t: "bool", def: true },
    { k: "background", label: "Background", t: "col", def: "#181820" },
    { k: "width",      label: "Render width px",  t: "num", def: 0, noExport: true },
    { k: "height",     label: "Render height px", t: "num", def: 0, noExport: true },
  ]},

  { s: "Direct lighting", fields: [
    { k: "light_dir", label: "Sun direction [x,y,z]", t: "vec", def: [0.4, 1.0, 0.5] },
    { k: "ambient",   label: "Fallback ambient (used only without IBL)", t: "rng", def: 0.05, min: 0, max: 1, step: 0.01 },
  ]},

  { s: "Image-based lighting", fields: [
    { k: "ibl", label: "IBL env", t: "grp", toggle: true, def: { __on: true, sky: "#20273c", ground: "#403020", intensity: 1.4, rotation: 0 }, fields: [
      { k: "sky",       label: "Sky colour",    t: "col", def: "#20273c" },
      { k: "ground",    label: "Ground colour", t: "col", def: "#403020" },
      { k: "intensity", label: "Intensity",     t: "rng", def: 1.4, min: 0, max: 4, step: 0.05 },
      { k: "rotation",  label: "Rotation rad",  t: "num", def: 0 },
    ]},
  ]},

  { s: "Shadows", fields: [
    { k: "shadows", label: "Cast shadows", t: "grp", toggle: true, bool: true, def: {
        __on: true, resolution: 1024, softness: 2, bias: 0.001, normal_bias: 1.5, slope_bias: 2.0, pcss_light_size: 0
      }, fields: [
      { k: "resolution",       label: "Resolution",       t: "num", def: 1024 },
      { k: "softness",         label: "PCF softness (texels)", t: "num", def: 2 },
      { k: "bias",             label: "Bias",             t: "num", def: 0.001 },
      { k: "normal_bias",      label: "Normal bias",      t: "num", def: 1.5 },
      { k: "slope_bias",       label: "Slope bias",       t: "num", def: 2.0 },
      { k: "pcss_light_size",  label: "PCSS light size (0 = plain PCF)", t: "num", def: 0 },
    ]},
  ]},

  { s: "Ground plane", fields: [
    { k: "ground", label: "Ground plane", t: "grp", toggle: true, bool: true, def: {
        color: "#282838", size_scale: 3.0, roughness: 0.9, y: 0
      }, fields: [
      { k: "color",      label: "Colour",     t: "col", def: "#282838" },
      { k: "size_scale", label: "Size scale × bbox radius", t: "num", def: 3.0 },
      { k: "roughness",  label: "Roughness",  t: "rng", def: 0.9, min: 0, max: 1, step: 0.01 },
      { k: "y",          label: "Y position (0 = auto)", t: "num", def: 0, omitIf: v => v === 0 },
    ]},
  ]},

  { s: "Post-processing", fields: [
    { k: "antialias",    label: "SSAA",       t: "sel", def: 1, num: true, opts: [[1, "Off"], [2, "×2"], [4, "×4"]] },
    { k: "fxaa",         label: "FXAA",       t: "bool", def: true },
    { k: "tone_mapping", label: "Tone mapping", t: "sel", def: "aces", opts: [["none", "None"], ["reinhard", "Reinhard"], ["aces", "ACES"]] },
    { k: "exposure",     label: "Exposure",   t: "rng", def: 1.2, min: 0, max: 4, step: 0.05 },
    { k: "ssao", label: "SSAO", t: "grp", toggle: true, bool: false, def: {
        samples: 16, radius: 0.4, bias: 0.02, strength: 1.0
      }, fields: [
      { k: "samples",  label: "Samples",  t: "num", def: 16 },
      { k: "radius",   label: "Radius",   t: "num", def: 0.4 },
      { k: "bias",     label: "Bias",     t: "num", def: 0.02 },
      { k: "strength", label: "Strength", t: "rng", def: 1.0, min: 0, max: 3, step: 0.05 },
    ]},
  ]},

  { s: "Animation & variants", fields: [
    { k: "time",             label: "Animation time (s)", t: "num", def: 0 },
    { k: "animation_index",  label: "Animation clip (-1 = stack all)", t: "num", def: -1, omitIf: v => v === -1 },
    { k: "material_variant", label: "Material variant (KHR_materials_variants)", t: "num", def: 0, omitIf: v => v === 0 },
    { k: "no_textures",      label: "Skip textures (fast preview)", t: "bool", def: false },
    { k: "texture_max_size", label: "Texture max size (0 = full)", t: "num", def: 0, omitIf: v => v === 0 },
  ]},
];

function getSchema() { return model._gltf ? GLTF_SCHEMA : SCHEMA; }


const HELP = {
  _cam: "Cartesian gives an explicit (x,y,z) camera; Spherical orbits by azimuth/elevation/distance.",
  camera: "Camera position in world space.", azimuth: "Horizontal orbit angle, in degrees.",
  elevation: "Vertical orbit angle, in degrees.", distance: "Camera distance from the center; 0 = auto-fit.",
  center: "Look-at target point.", up: "Up direction. Bunny/most OBJ models are Y-up (0,1,0).",
  projection: "Camera projection — perspective, orthographic, or one of 12 others.",
  fov: "Vertical field of view in degrees (perspective only).",
  zoom: "Magnify the auto-fit framing (>1 zooms in). Scroll the render to change.",
  pan: "Shift the framing in screen space, [right, up] as a fraction of the viewport.",
  auto_center: "Center the camera on the model's bounding box.",
  auto_fit: "Scale the model to fill the viewport.", background: "Background color.",
  camera_auto_use: "If the glTF ships an authored camera, use it instead of your framing arguments.",
  scene_index: "Pick a scene by index. -1 uses the asset's authored default scene (typically scene 0).",
  animation_index: "Pick a single animation clip by index (0-based). -1 plays every clip stacked.",
  _bgNone: "Render on a transparent background instead of a color.",
  width: "Render width in pixels (0 = fit to view).", height: "Render height in pixels (0 = fit to view).",
  color: "Base model fill color.", opacity: "Whole-model opacity (0 = invisible, 1 = opaque).",
  specular: "Specular highlight intensity.", shininess: "Specular exponent — higher = tighter highlight.",
  smooth: "Gouraud smooth shading (best with PNG).", gamma_correction: "Light in linear sRGB for accurate midtones.",
  scad_smooth_normals: "OpenSCAD only: attach per-vertex normals via Manifold's calculate_normals(30°) so curved surfaces render smooth-shaded while crease edges stay crisp. Off = faceted (OpenSCAD-native look).",
  cull_backface: "Skip triangles facing away from the camera.",
  shading: "Shading model — Blinn-Phong, Gooch, Cel, Flat, or Normal-map.",
  gooch_warm: "Gooch warm-tone color.", gooch_cool: "Gooch cool-tone color.", cel_bands: "Number of cel-shading bands.",
  mode: "Render as solid, wireframe, both, or x-ray.", xray_opacity: "Front-face opacity in x-ray mode.",
  stroke: "Draw an outline stroke on every triangle edge.", wireframe: "Wireframe edge color/width (wireframe modes).",
  light_dir: "Direction the key directional light comes from.",
  ambient: "Uniform fill light reaching all surfaces (0–1).",
  _hemi: "Sky/ground gradient ambient instead of a flat value.",
  fresnel: "Rim highlight on grazing-angle edges.", tone_mapping: "HDR tone mapping (ACES/Reinhard) + exposure.",
  sss: "Fake subsurface scattering — glow through thin geometry.", lights: "Add extra directional/positional/area lights.",
  color_map: "Color the surface by overhang, curvature, or a scalar function.",
  overhang_angle: "Overhang threshold in degrees.", scalar_function: "Expression over x,y,z, e.g. sqrt(x*x+y*y+z*z).",
  vertex_smoothing: "Smooth color-map values across vertices (0–4).", color_map_palette: "Custom gradient stops.",
  outline: "Bold silhouette contour around the model.",
  ground_shadow: "Project a silhouette shadow onto a floor plane.",
  shadows: "True self-shadowing via depth maps (PNG only).",
  antialias: "0 off · 1 FXAA · 2/4 supersampling (PNG only).",
  ssao: "Screen-space ambient occlusion — contact shadows (PNG only).",
  bloom: "Bleed light from bright areas (PNG only).", glow: "Aura around the silhouette (PNG only).",
  sharpen: "Unsharp-mask edge sharpening (PNG only).",
  clip: "Cut the model with a plane; optionally cap and hatch the section.",
  explode: "Push components outward from the center (multi-part models).",
  decimate: "Simplify the mesh (higher = fewer triangles).", point_size: "Neighbor radius for PLY point clouds.", point_neighbors: "PLY clouds: neighbors per point (higher = fewer holes, slower).", point_boundary: "PLY clouds: cut connections across a normal jump > this angle\u00b0 (0 = keep all).",
  views: "Render a grid of named orthographic views.", grid_labels: "Show labels on the multi-view grid.",
  turntable: "Render a spun grid of frames around the model.",
  materials: "Map OBJ material names to colors.", highlight: "Recolor named OBJ groups.",
  annotations: "Label OBJ groups on the render.", debug: "Overlay model metadata and light gizmos.",
  debug_color: "Debug overlay text color.",
};


const ext = (name) => name.split(".").pop().toLowerCase();
let PLUGINS = [], MODELS = [], MODELS_BY_PLUGIN = {}, MOLECULES = {};
let PLUGIN_OF_MODEL = {}, MODEL_DEFAULTS = {}, DEFAULTS_KEYS = [];
const makeModel = (name, bytes, extra = null) => {
  const mol = MOLECULES[name];
  if (mol) {
    return { name, bytes, _ext: "obj", _gltf: false, _mol: true,
             _molSrc: null, _molSrcPath: mol.src, _molFmt: mol.fmt,
             _molMaterials: null, ...(extra || {}) };
  }
  const e = ext(name);
  return extra ? { name, bytes, _ext: e, _gltf: GLTF_EXTS.has(e), _mol: false, ...extra }
               : { name, bytes, _ext: e, _gltf: GLTF_EXTS.has(e), _mol: false };
};
let model = makeModel("bunny.obj", null);
function initState() {
  const st = {};
  for (const sec of getSchema()) for (const f of sec.fields) {
    if (f.t === "grp") st[f.k] = { __on: !!f.def.__on, ...structuredClone(f.def) };
    else { const d = f.init !== undefined ? f.init : f.def; st[f.k] = Array.isArray(d) ? d.slice() : d; }
  }
  return st;
}
const state = initState();

const eq = (a, b) => {
  if (a === b) return true;
  if (Array.isArray(a) && Array.isArray(b)) {
    if (a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
    return true;
  }
  return JSON.stringify(a) === JSON.stringify(b);
};
const num = (v) => Number.isInteger(v) ? String(v) : String(+v.toFixed(4));
const round3 = (x) => Math.round(x * 1000) / 1000;
function fmtT(v) {
  if (typeof v === "string") return `"${v}"`;
  if (typeof v === "boolean") return v ? "true" : "false";
  if (typeof v === "number") return num(v);
  if (Array.isArray(v)) return `(${v.map(fmtT).join(", ")})`;
  return `(${Object.entries(v).map(([k,x]) => `${k}: ${fmtT(x)}`).join(", ")})`;
}

function group(f, mode) {
  const s = state[f.k];
  if (f.build === "clip") {
    const o = {};
    if (s.source === "plane") o.plane = s.plane.slice();
    else { o.depth = s.depth; if (s.source === "camera") o.from = "camera"; else o.axis = s.source; }
    if (s.keep === "near") o.keep = "near";
    if (mode === "cfg" || s.cap !== true) o.cap = s.cap;
    if (s.hatch) o.hatch = { style: s.hstyle, angle: s.hangle, spacing: s.hspacing, width: s.hwidth, color: s.hcolor };
    return o;
  }
  if (f.build === "turntable") return { iterations: s.iterations, elevation: s.elevation };
  const o = {};
  for (const sub of f.fields) {
    const v = s[sub.k];
    if (v === undefined) continue;
    if (sub.allowBlank && v === "") continue;
    if (mode === "diff" && eq(v, sub.def)) continue;
    o[sub.k] = v;
  }
  return o;
}

let renderOverride = null;

const hlNormalize = (cv) => {
  const a = { color: "#88ccff", stroke: "", stroke_width: 0, opacity: 1 };
  if (typeof cv === "string") a.color = cv;
  else if (cv && typeof cv === "object") {
    if (cv.color) a.color = cv.color;
    if (cv.stroke) a.stroke = cv.stroke;
    if (cv.stroke_width != null) a.stroke_width = cv.stroke_width;
    if (cv.opacity != null) a.opacity = cv.opacity;
  }
  return a;
};
const hlCollapse = (v) => {
  if (typeof v === "string") return v;
  const o = { color: v.color };
  if (v.stroke) o.stroke = v.stroke;
  if (v.stroke_width) o.stroke_width = v.stroke_width;
  if (v.opacity != null && v.opacity !== 1) o.opacity = v.opacity;
  return Object.keys(o).length === 1 ? o.color : o;
};

const ambientCfg = () => state._hemi.__on
  ? { intensity: state._hemi.intensity, sky: state._hemi.sky, ground: state._hemi.ground }
  : state.ambient;
const bgCfg = () => state._bgNone ? "none" : state.background;

const _topFieldsCache = new WeakMap();
const topFields = () => {
  const s = getSchema();
  let tf = _topFieldsCache.get(s);
  if (!tf) _topFieldsCache.set(s, tf = Object.fromEntries(s.flatMap(sec => sec.fields.map(f => [f.k, f]))));
  return tf;
};

function resetState() {
  for (const k in state) delete state[k];
  Object.assign(state, initState());
}


let outputFormat = "png";
const setModel = (v) => { model = v; };
const setRenderOverride = (v) => { renderOverride = v; };
const setOutputFormat = (v) => { outputFormat = v; };
let _rebuildForm = () => {};
const setRebuildForm = (fn) => { _rebuildForm = fn; };
export { ext, GLTF_EXTS, isGltf, MOL_FMTS, isMolExt, PROJ, SCHEMA, GLTF_SCHEMA, getSchema, HELP, PLUGINS, MODELS, MODELS_BY_PLUGIN, MOLECULES, PLUGIN_OF_MODEL, MODEL_DEFAULTS, DEFAULTS_KEYS, makeModel, model, setModel, initState, state, resetState, topFields, eq, num, round3, fmtT, group, renderOverride, setRenderOverride, hlNormalize, hlCollapse, ambientCfg, bgCfg, outputFormat, setOutputFormat, setRebuildForm };

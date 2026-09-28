const PROJ = ["perspective","orthographic","isometric","dimetric","trimetric","military",
  "cabinet","cavalier","fisheye","stereographic","curvilinear","cylindrical","pannini","tiny-planet"];

const SCHEMA = [
  { s: "Point cloud (PLY)", open: true, when: (s, l, m) => m._ext === "ply" && !m._mol && !m.scad, fields: [
    { k: "point_splat", label: "Splat as points (no surface)", help: "PLY clouds: draw points as round splats instead of reconstructing a surface.", t: "bool", def: false, omitIf: v => v === false,
      onSet: (v, s) => { if (v) { s._psPrev = s.point_size; s.point_size = 0.012; } else if (s._psPrev != null) { s.point_size = s._psPrev; } return true; } },
    { k: "point_size", label: "Point size / radius (0 = auto)", help: "PLY clouds: kNN radius, or splat radius when splatting (0 = auto).", t: "num", def: 0, step: 0.001, min: 0, omitIf: v => v === 0 },
    { k: "point_neighbors", label: "Neighbors k (higher = fewer holes)", help: "PLY clouds: neighbors per point (higher = fewer holes, slower).", t: "num", def: 12, omitIf: v => v === 12, when: s => !s.point_splat },
    { k: "point_boundary", label: "Boundary cut angle ° (0 = off)", help: "PLY clouds: don't connect neighbouring points whose normals differ by more than this angle, so separate surfaces (a cube's faces, an object on a table) stay apart. 0 keeps every connection.", t: "rng", def: 60, min: 0, max: 180, step: 1, omitIf: v => v === 60, when: s => !s.point_splat },
    { k: "point_denoise", label: "Denoise colors (crisper regions)", help: "PLY clouds (reconstruction): clean up scan colors with an edge-preserving filter so regions read crisply instead of bleeding.", t: "bool", def: false, omitIf: v => v === false, when: s => !s.point_splat },
  ]},
  { s: "Molecule (molfig)", open: true, when: (s, l, m) => m._mol, fields: [
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
      onSet: (v, s) => { if (v === "surface" && s.mol_quality !== "highest") { s.mol_quality = "highest"; return true; } return false; } },
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
  { s: "Molecule advanced (molfig)", open: false, when: (s, l, m) => m._mol, fields: [
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
    { k: "_cam", label: "Camera mode", help: "Cartesian gives an explicit (x,y,z) camera; Spherical orbits by azimuth/elevation/distance.", t: "sel", def: "cartesian", init: "spherical", opts: [["cartesian","Cartesian (x,y,z)"],["spherical","Spherical"]] },
    { k: "camera", label: "Position", help: "Camera position in world space.", t: "vec", def: [3,3,3], when: s => s._cam === "cartesian" },
    { k: "azimuth", label: "Azimuth °", help: "Horizontal orbit angle, in degrees.", t: "num", def: 0, init: 180, when: s => s._cam === "spherical" },
    { k: "elevation", label: "Elevation °", help: "Vertical orbit angle, in degrees.", t: "num", def: 0, when: s => s._cam === "spherical" },
    { k: "distance", label: "Distance (0 = auto)", help: "Camera distance from the center; 0 = auto-fit.", t: "num", def: 0, omitIf: v => v === 0, when: s => s._cam === "spherical" },
    { k: "center", label: "Look-at center", help: "Look-at target point.", t: "vec", def: [0,0,0] },
    { k: "up", label: "Up vector", help: "Up direction. Bunny/most OBJ models are Y-up (0,1,0).", t: "vec", def: [0,0,1], init: [0,1,0] },
    { k: "projection", label: "Projection", help: "Camera projection — perspective, orthographic, or one of 12 others.", t: "sel", def: "perspective", opts: PROJ.map(p => [p, p]) },
    { k: "fov", label: "Field of view °", help: "Vertical field of view in degrees (perspective only).", t: "num", def: 45 },
    { k: "zoom", label: "Zoom", help: "Magnify the auto-fit framing (>1 zooms in). Scroll the render to change.", t: "rng", def: 1, init: 1.4, min: 0.3, max: 4, step: 0.05 },
    { k: "pan", label: "Pan [right, up]", help: "Shift the framing in screen space, [right, up] as a fraction of the viewport.", t: "vec", def: [0,0] },
    { k: "auto_center", label: "Auto-center", help: "Center the camera on the model's bounding box.", t: "bool", def: true },
    { k: "auto_fit", label: "Auto-fit to viewport", help: "Scale the model to fill the viewport.", t: "bool", def: true },
    { k: "background", label: "Background", help: "Background color.", t: "col", def: "#f0f0f0" },
    { k: "_bgNone", label: "Transparent background", help: "Render on a transparent background instead of a color.", t: "bool", def: false },
    { k: "width", label: "Render width px", help: "Render width in pixels (0 = fit to view).", t: "num", def: 0, noExport: true },
    { k: "height", label: "Render height px", help: "Render height in pixels (0 = fit to view).", t: "num", def: 0, noExport: true },
  ]},

  { s: "Material", fields: [
    { k: "color", label: "Model color", help: "Base model fill color.", t: "col", def: "#4488cc" },
    { k: "opacity", label: "Opacity", help: "Whole-model opacity (0 = invisible, 1 = opaque).", t: "rng", def: 1, min: 0, max: 1, step: 0.01 },
    { k: "specular", label: "Specular", help: "Specular highlight intensity.", t: "rng", def: 0.2, min: 0, max: 1, step: 0.01 },
    { k: "shininess", label: "Shininess", help: "Specular exponent — higher = tighter highlight.", t: "num", def: 32 },
    { k: "smooth", label: "Smooth shading", help: "Gouraud smooth shading (best with PNG).", t: "bool", def: true },
    { k: "scad_smooth_normals", label: "SCAD smooth normals", help: "OpenSCAD only: attach per-vertex normals via Manifold's calculate_normals(30°) so curved surfaces render smooth-shaded while crease edges stay crisp. Off = faceted (OpenSCAD-native look).", t: "bool", def: false, recompile: "scad" },
    { k: "gamma_correction", label: "Gamma correction", help: "Light in linear sRGB for accurate midtones.", t: "bool", def: true },
    { k: "cull_backface", label: "Back-face culling", help: "Skip triangles facing away from the camera.", t: "bool", def: true },
  ]},

  { s: "Shading model", fields: [
    { k: "shading", label: "Model", help: "Shading model — Blinn-Phong, Gooch, Cel, Flat, Normal-map, or Unlit (flat base colour, no lights or shadows).", t: "sel", def: "", opts: [["","Blinn–Phong"],["gooch","Gooch"],["cel","Cel"],["flat","Flat"],["normal","Normal map"],["unlit","Unlit"]] },
    { k: "gooch_warm", label: "Gooch warm", help: "Gooch warm-tone color.", t: "col", def: "#ffcc44", when: s => s.shading === "gooch" },
    { k: "gooch_cool", label: "Gooch cool", help: "Gooch cool-tone color.", t: "col", def: "#4466cc", when: s => s.shading === "gooch" },
    { k: "cel_bands", label: "Cel bands", help: "Number of cel-shading bands.", t: "num", def: 4, when: s => s.shading === "cel" },
  ]},

  { s: "Render mode", fields: [
    { k: "mode", label: "Mode", help: "Render as solid, wireframe, both, or x-ray.", t: "sel", def: "solid", opts: [["solid","Solid"],["wireframe","Wireframe"],["solid+wireframe","Solid + wireframe"],["x-ray","X-ray"]] },
    { k: "xray_opacity", label: "X-ray opacity", help: "Front-face opacity in x-ray mode.", t: "rng", def: 0.1, min: 0, max: 1, step: 0.01, when: s => s.mode === "x-ray" },
    { k: "stroke", label: "Edge stroke", help: "Draw an outline stroke on every triangle edge.", t: "grp", toggle: true, def: { color: "#000000", width: 1 }, fields: [
      { k: "color", label: "Color", t: "col", def: "#000000" },
      { k: "width", label: "Width", t: "num", def: 1 },
    ]},
    { k: "wireframe", label: "Wireframe style", help: "Wireframe edge color/width (wireframe modes).", t: "grp", toggle: true, def: { color: "#000000", width: 1 }, fields: [
      { k: "color", label: "Color", t: "col", def: "#000000" },
      { k: "width", label: "Width", t: "num", def: 1 },
    ]},
  ]},

  { s: "Lighting", fields: [
    { k: "light_dir", label: "Light direction", help: "Direction the key directional light comes from.", t: "vec", def: [1,2,3] },
    { k: "ambient", label: "Ambient", help: "Uniform fill light reaching all surfaces (0–1).", t: "rng", def: 0.15, min: 0, max: 1, step: 0.01, when: s => !s._hemi.__on },
    { k: "_hemi", label: "Hemisphere ambient", help: "Sky/ground gradient ambient instead of a flat value.", t: "grp", toggle: true, def: { intensity: 0.3, sky: "#ccd4e0", ground: "#d4ccc4" }, fields: [
      { k: "intensity", label: "Intensity", t: "rng", def: 0.3, min: 0, max: 1, step: 0.01 },
      { k: "sky", label: "Sky", t: "col", def: "#ccd4e0" },
      { k: "ground", label: "Ground", t: "col", def: "#d4ccc4" },
    ]},
    { k: "fresnel", label: "Fresnel rim", help: "Rim highlight on grazing-angle edges.", t: "grp", toggle: false, def: { intensity: 0.3, power: 5 }, fields: [
      { k: "intensity", label: "Intensity", t: "rng", def: 0.3, min: 0, max: 1, step: 0.01 },
      { k: "power", label: "Power", t: "num", def: 5 },
    ]},
    { k: "tone_mapping", label: "Tone mapping", help: "HDR tone mapping (ACES/Reinhard) + exposure.", t: "grp", toggle: false, def: { method: "", exposure: 1 }, fields: [
      { k: "method", label: "Method", t: "sel", def: "", opts: [["","None"],["aces","ACES"],["reinhard","Reinhard"]] },
      { k: "exposure", label: "Exposure", t: "rng", def: 1, min: 0, max: 4, step: 0.05 },
    ]},
    { k: "sss", label: "Subsurface scattering", help: "Fake subsurface scattering — glow through thin geometry.", t: "grp", toggle: true, bool: true, def: { intensity: 0.5, power: 3, distortion: 0.2 }, fields: [
      { k: "intensity", label: "Intensity", t: "num", def: 0.5 },
      { k: "power", label: "Power", t: "num", def: 3 },
      { k: "distortion", label: "Distortion", t: "num", def: 0.2 },
    ]},
    { k: "lights", label: "Extra lights", help: "Add extra directional/positional/area lights.", t: "lights", def: [] },
  ]},

  { s: "Color mapping", fields: [
    { k: "color_map", label: "Map", help: "Color the surface by overhang, curvature, or a scalar function.", t: "sel", def: "", opts: [["","Off"],["overhang","Overhang"],["curvature","Curvature"],["scalar","Scalar"],["ply_scalar","PLY scalar"]] },
    { k: "overhang_angle", label: "Overhang angle °", help: "Overhang threshold in degrees.", t: "num", def: 45, when: s => s.color_map === "overhang" },
    { k: "scalar_function", label: "Scalar function", help: "Expression over x,y,z, e.g. sqrt(x*x+y*y+z*z).", t: "txt", def: "", when: s => s.color_map === "scalar" },
    { k: "vertex_smoothing", label: "Vertex smoothing 0–4", help: "Smooth color-map values across vertices (0–4).", t: "num", def: 4, when: s => s.color_map !== "" },
    { k: "color_map_palette", label: "Palette", help: "Custom gradient stops.", t: "palette", def: [], when: s => s.color_map === "curvature" || s.color_map === "scalar" || s.color_map === "ply_scalar" },
  ]},

  { s: "Outlines", fields: [
    { k: "outline", label: "Silhouette outline", help: "Bold silhouette contour around the model.", t: "grp", toggle: true, bool: true, def: { color: "#000000", width: 2, threshold: 5 }, fields: [
      { k: "color", label: "Color", t: "col", def: "#000000" },
      { k: "width", label: "Width", t: "num", def: 2 },
      { k: "threshold", label: "Depth threshold (pixels)", help: "Edge when the depth jump exceeds this many pixel footprints; stays consistent across zoom and image size.", t: "num", def: 5 },
    ]},
  ]},

  { s: "Shadows", fields: [
    { k: "ground_shadow", label: "Ground shadow", help: "Project a silhouette shadow onto a floor plane.", t: "grp", toggle: true, bool: true, def: { opacity: 0.3, color: "#000000" }, fields: [
      { k: "opacity", label: "Opacity", t: "rng", def: 0.3, min: 0, max: 1, step: 0.01 },
      { k: "color", label: "Color", t: "col", def: "#000000" },
    ]},
    { k: "shadows", label: "Cast shadows (self-shadowing)", help: "True self-shadowing via depth maps (PNG only).", t: "grp", toggle: true, bool: true, def: {
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
    { k: "antialias", label: "Antialiasing", help: "0 off · 1 FXAA · 2/4 supersampling · 5/6 FXAA on top of supersampling ×2/×4 (PNG only).", t: "sel", def: 2, typstDef: 1, num: true, opts: [[0,"Off"],[1,"FXAA"],[2,"SSAA ×2"],[4,"SSAA ×4"],[5,"FXAA + SSAA ×2"],[6,"FXAA + SSAA ×4"]] },
    { k: "ssao", label: "Ambient occlusion", help: "Ambient occlusion from the depth buffer — contact shadows (PNG only).", t: "grp", toggle: true, bool: true, def: { __on: true, samples: 16, radius: 0, bias: 0.025, strength: 1, space: "scene" }, fields: [
      { k: "samples", label: "Samples", t: "num", def: 16 },
      { k: "radius", label: "Radius (0 = auto)", help: "Scene units (auto: 10% of the model's size), or a fraction of the image in screen space.", t: "num", def: 0, omitIf: v => !v },
      { k: "bias", label: "Bias", t: "num", def: 0.025 },
      { k: "strength", label: "Strength", t: "rng", def: 1, min: 0, max: 2, step: 0.05 },
      { k: "space", label: "Space", help: "Screen: radius is a fraction of the image. Scene: radius and bias are in model units, sampled around each pixel's 3D position.", t: "sel", def: "scene", opts: [["scene", "Scene units"], ["screen", "Screen (legacy)"]] },
    ]},
    { k: "bloom", label: "Bloom", help: "Bleed light from bright areas (PNG only).", t: "grp", toggle: true, bool: true, def: { threshold: 0.8, intensity: 0.3, radius: 10 }, fields: [
      { k: "threshold", label: "Threshold", t: "rng", def: 0.8, min: 0, max: 1, step: 0.01 },
      { k: "intensity", label: "Intensity", t: "rng", def: 0.3, min: 0, max: 2, step: 0.05 },
      { k: "radius", label: "Radius", t: "num", def: 10 },
    ]},
    { k: "glow", label: "Glow", help: "Aura around the silhouette (PNG only).", t: "grp", toggle: true, bool: true, def: { color: "#ffffff", intensity: 0.5, radius: 15 }, fields: [
      { k: "color", label: "Color", t: "col", def: "#ffffff" },
      { k: "intensity", label: "Intensity", t: "rng", def: 0.5, min: 0, max: 2, step: 0.05 },
      { k: "radius", label: "Radius", t: "num", def: 15 },
    ]},
    { k: "fog", label: "Depth fog", help: "Fade distant surfaces toward the background, as in Mol* (PNG only). Intensity 0–100 moves where the fade starts, from the back to the front of the model.", t: "grp", toggle: true, bool: true, def: { intensity: 50, color: "" }, fields: [
      { k: "intensity", label: "Intensity", t: "rng", def: 50, min: 0, max: 100, step: 1 },
      { k: "color", label: "Color (blank = background)", t: "col", def: "", allowBlank: true },
    ]},
    { k: "sharpen", label: "Sharpen", help: "Unsharp-mask edge sharpening (PNG only).", t: "grp", toggle: true, bool: true, def: { strength: 0.5 }, fields: [
      { k: "strength", label: "Strength", t: "rng", def: 0.5, min: 0, max: 2, step: 0.05 },
    ]},
  ]},

  { s: "Geometry & clipping", fields: [
    { k: "clip", label: "Clip plane", help: "Cut the model with a plane; optionally cap and hatch the section.", t: "grp", toggle: true, def: {
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
    { k: "explode", label: "Explode", help: "Push components outward from the center (multi-part models).", t: "rng", def: 0, min: 0, max: 1, step: 0.02 },
    { k: "decimate", label: "Decimate", help: "Simplify the mesh (higher = fewer triangles).", t: "rng", def: 0, min: 0, max: 1, step: 0.02 },
  ]},

  { s: "Multi-view", fields: [
    { k: "views", label: "Grid views", help: "Render a grid of named orthographic views.", t: "views", def: [], opts: ["front","back","left","right","top","bottom","isometric"] },
    { k: "grid_labels", label: "Grid labels", help: "Show labels on the multi-view grid.", t: "bool", def: true, when: s => s.views.length > 0 },
    { k: "turntable", label: "Turntable", help: "Render a spun grid of frames around the model.", t: "grp", toggle: true, def: { iterations: 6, elevation: 40 }, build: "turntable", fields: [
      { k: "iterations", label: "Frames", t: "num", def: 6 },
      { k: "elevation", label: "Elevation °", t: "num", def: 40 },
    ]},
  ]},

  { s: "OBJ groups", fields: [
    { k: "materials", label: "Materials (name → color)", help: "Map OBJ material names to colors.", t: "map", def: [] },
    { k: "highlight", label: "Highlight (group → appearance)", help: "Recolor named OBJ groups.", t: "map", rich: true, def: [] },
    { k: "annotations", label: "Annotations", help: "Label OBJ groups on the render.", t: "grp", toggle: true, bool: true, def: { color: "#333333", font_size: 12, offset: 40 }, fields: [
      { k: "color", label: "Color", t: "col", def: "#333333" },
      { k: "font_size", label: "Font size", t: "num", def: 12 },
      { k: "offset", label: "Offset", t: "num", def: 40 },
    ]},
  ]},

  { s: "Diagnostics", fields: [
    { k: "debug", label: "Debug overlay", help: "Overlay model metadata and light gizmos.", t: "bool", def: false },
    { k: "debug_color", label: "Debug color", help: "Debug overlay text color.", t: "col", def: "#cc2222", when: s => s.debug },
  ]},
];

const GLTF_SCHEMA = [
  { s: "Camera & viewport", fields: [
    { k: "_cam", label: "Camera mode", help: "Cartesian gives an explicit (x,y,z) camera; Spherical orbits by azimuth/elevation/distance.", t: "sel", def: "cartesian",
      opts: [["cartesian", "Cartesian (x,y,z)"], ["spherical", "Spherical (az/el/dist)"]] },
    { k: "camera",    label: "Position [x,y,z]", help: "Camera position in world space.", t: "vec", def: [2.5, 1.5, 2.5],
      when: s => s._cam === "cartesian" },
    { k: "azimuth",   label: "Azimuth °", help: "Horizontal orbit angle, in degrees.",   t: "num", def: 30, when: s => s._cam === "spherical" },
    { k: "elevation", label: "Elevation °", help: "Vertical orbit angle, in degrees.", t: "num", def: 20, when: s => s._cam === "spherical" },
    { k: "distance",  label: "Distance (0 = auto)", help: "Camera distance from the center; 0 = auto-fit.", t: "num", def: 0,
      omitIf: v => v === 0, when: s => s._cam === "spherical" },
    { k: "center",     label: "Look-at", help: "Look-at target point.",          t: "vec", def: [0, 0, 0] },
    { k: "up",         label: "Up", help: "Up direction. Bunny/most OBJ models are Y-up (0,1,0).",               t: "vec", def: [0, 1, 0] },
    { k: "fov",        label: "Field of view °", help: "Vertical field of view in degrees (perspective only).",  t: "num", def: 40 },
    { k: "auto_center", label: "Auto-center on bbox", help: "Center the camera on the model's bounding box.", t: "bool", def: true },
    { k: "auto_fit",    label: "Auto-fit distance", help: "Scale the model to fill the viewport.",   t: "bool", def: true },
    { k: "camera_auto_use", label: "Prefer glTF-authored camera when present", help: "If the glTF ships an authored camera, use it instead of your framing arguments.", t: "bool", def: true },
    { k: "camera_name",  label: "Named camera (from glTF, blank = ignore)", t: "txt", def: "", allowBlank: true },
    { k: "camera_index", label: "Camera index (-1 = ignore)", t: "num", def: -1, omitIf: v => v === -1 },
    { k: "scene_index",  label: "Scene index (-1 = default)", help: "Pick a scene by index. -1 uses the asset's authored default scene (typically scene 0).", t: "num", def: -1, omitIf: v => v === -1 },
    { k: "cull_backface", label: "Back-face culling", help: "Skip triangles facing away from the camera.", t: "bool", def: true },
    { k: "background", label: "Background", help: "Background color.", t: "col", def: "#181820" },
    { k: "width",      label: "Render width px", help: "Render width in pixels (0 = fit to view).",  t: "num", def: 0, noExport: true },
    { k: "height",     label: "Render height px", help: "Render height in pixels (0 = fit to view).", t: "num", def: 0, noExport: true },
  ]},

  { s: "Direct lighting", fields: [
    { k: "light_dir", label: "Sun direction [x,y,z]", help: "Direction the key directional light comes from.", t: "vec", def: [0.4, 1.0, 0.5] },
    { k: "ambient",   label: "Fallback ambient (used only without IBL)", help: "Uniform fill light reaching all surfaces (0–1).", t: "rng", def: 0.05, min: 0, max: 1, step: 0.01 },
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
    { k: "shadows", label: "Cast shadows", help: "True self-shadowing via depth maps (PNG only).", t: "grp", toggle: true, bool: true, def: {
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
    { k: "antialias",    label: "SSAA", help: "0 off · 1 FXAA · 2/4 supersampling (PNG only).",       t: "sel", def: 2, typstDef: 1, num: true, opts: [[1, "Off"], [2, "×2"], [4, "×4"]] },
    { k: "fxaa",         label: "FXAA",       t: "bool", def: false },
    { k: "tone_mapping", label: "Tone mapping", help: "HDR tone mapping (ACES/Reinhard) + exposure.", t: "sel", def: "aces", opts: [["none", "None"], ["reinhard", "Reinhard"], ["aces", "ACES"]] },
    { k: "exposure",     label: "Exposure",   t: "rng", def: 1.2, min: 0, max: 4, step: 0.05 },
    { k: "ssao", label: "SSAO", help: "Ambient occlusion from the depth buffer — contact shadows (PNG only).", t: "grp", toggle: true, bool: false, def: {
        __on: true, samples: 16, radius: 0, bias: 0.025, strength: 1.0, space: "scene"
      }, fields: [
      { k: "samples",  label: "Samples",  t: "num", def: 16 },
      { k: "radius",   label: "Radius (0 = auto)", help: "Scene units (auto: 10% of the scene's size), or a fraction of the image in screen space.", t: "num", def: 0, omitIf: v => !v },
      { k: "bias",     label: "Bias",     t: "num", def: 0.025 },
      { k: "strength", label: "Strength", t: "rng", def: 1.0, min: 0, max: 3, step: 0.05 },
      { k: "space", label: "Space", help: "Screen: radius is a fraction of the image. Scene: radius and bias are in model units, sampled around each pixel's 3D position.", t: "sel", def: "scene", opts: [["scene", "Scene units"], ["screen", "Screen (legacy)"]] },
    ]},
  ]},

  { s: "Animation & variants", fields: [
    { k: "time",             label: "Animation time (s)", t: "num", def: 0 },
    { k: "animation_index",  label: "Animation clip (-1 = stack all)", help: "Pick a single animation clip by index (0-based). -1 plays every clip stacked.", t: "num", def: -1, omitIf: v => v === -1 },
    { k: "material_variant", label: "Material variant (KHR_materials_variants)", t: "num", def: 0, omitIf: v => v === 0 },
    { k: "no_textures",      label: "Skip textures (fast preview)", t: "bool", def: false },
    { k: "texture_max_size", label: "Texture max size (0 = full)", t: "num", def: 0, omitIf: v => v === 0 },
  ]},
];

export { SCHEMA, GLTF_SCHEMA };

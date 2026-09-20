import { $, elCode } from "./dom.js";
import { state, model, getSchema, num, fmtT, eq, group, ambientCfg, bgCfg, hlCollapse, outputFormat, topFields } from "./state.js";

function buildConfig() {
  const c = {};
  // For glTF the ambient/background fields are plain scalars — they come out of
  // the SCHEMA walk directly, no polymorphic hemispheric-ambient / transparent-
  // background handling needed. For maquette they're polymorphic and set below.
  const gltf = model._gltf;
  for (const sec of getSchema()) {
    if (sec.when && !sec.when(state, state)) continue;   // Molfig / point-cloud sections gate here
    for (const f of sec.fields) {
    if (f.when && !f.when(state, state)) continue;
    if (f.k[0] === "_") continue;                     // UI-only fields
    if (f.k.startsWith("mol_") || f.k.startsWith("scad_")) continue;  // source-plugin state — not maquette config
    if (!gltf && (f.k === "ambient" || f.k === "background")) continue; // polymorphic — set below
    if (f.omitIf && f.omitIf(state[f.k])) continue;
    switch (f.t) {
      case "grp":
        if (f.toggle && !state[f.k].__on) break;
        c[f.k] = group(f, "cfg");
        break;
      case "views": if (state.views.length) c.views = state.views.slice(); break;
      case "palette": if (state[f.k].length) c[f.k] = state[f.k].slice(); break;
      case "lights": if (state.lights.length) c.lights = state.lights.map(l => ({ ...l })); break;
      case "map": if (state[f.k].length) c[f.k] = Object.fromEntries(state[f.k].filter(r => r[0]).map(([n, v]) => [n, f.rich ? hlCollapse(v) : v])); break;
      default: c[f.k] = state[f.k];
    }
  }
  }
  if (!gltf) {
    c.ambient = ambientCfg();          // number, or hemisphere {intensity,sky,ground}
    c.background = bgCfg();             // color, or "none" (transparent)
  }
  return c;
}

function buildTypst() {
  // Different Typst function name + import path for glTF (a different plugin).
  const gltf = model._gltf;
  const fn = gltf ? "render-gltf"
    : ({ obj: "render-obj", stl: "render-stl", ply: "render-ply" }[model._ext] || "render-obj");
  const P = [];
  const push = (k, v) => P.push(`${k}: ${v}`);
  for (const sec of getSchema()) {
    if (sec.when && !sec.when(state, state)) continue;
    for (const f of sec.fields) {
    if (f.when && !f.when(state, state)) continue;
    if (f.noExport || f.k === "_cam" || f.k === "width" || f.k === "height") continue;
    if (f.k.startsWith("mol_") || f.k.startsWith("scad_")) continue;   // handled by the source-plugin's own snippet block
    if (f.omitIf && f.omitIf(state[f.k])) continue;
    // The polymorphic ambient / background handling is maquette-only. For
    // glTF, ambient/background are plain scalars and go through the default
    // branch below.
    if (!gltf) {
      if (f.k === "background") { const b = bgCfg(); if (b !== f.def) push("background", b === "none" ? "none" : fmtT(b)); continue; }
      if (f.k === "_bgNone") continue;
      if (f.k === "ambient") { if (!state._hemi.__on && state.ambient !== f.def) push("ambient", num(state.ambient)); continue; }
      if (f.k === "_hemi") { if (state._hemi.__on) push("ambient", fmtT(ambientCfg())); continue; }
    }
    if (f.k[0] === "_") continue;
    switch (f.t) {
      case "grp": {
        if (f.toggle && !state[f.k].__on) break;
        if (f.build === "clip") { push(f.k, fmtT(group(f, "cfg"))); break; } // clip always needs its dict
        if (f.build === "turntable") { const s = state[f.k]; push("turntable", s.elevation === f.def.elevation ? num(s.iterations) : fmtT({ iterations: s.iterations, elevation: s.elevation })); break; }
        const d = group(f, "diff");
        if (Object.keys(d).length === 0) { if (f.toggle && f.bool) push(f.k, "true"); break; } // enabled-at-defaults → `key: true`; always-on group unchanged → omit
        push(f.k, fmtT(d));
        break;
      }
      case "views": if (state.views.length) push("views", fmtT(state.views)); break;
      case "palette": if (state[f.k].length) push(f.k, fmtT(state[f.k])); break;
      case "lights": if (state.lights.length) push("lights", fmtT(state.lights)); break;
      case "map": {
        const rows = state[f.k].filter(r => r[0]);
        if (!rows.length) break;
        const entries = rows.map(([n, v]) => `"${n}": ${fmtT(f.rich ? hlCollapse(v) : v)}`);
        // Wrap the dict across multiple lines once the joined single-line
        // form would push the containing snippet line past a screen-width
        // budget (`k: (...),` prefix ~15 chars + 4-space body indent).
        const oneLine = `(${entries.join(", ")})`;
        push(f.k, oneLine.length + f.k.length + 4 <= 80
          ? oneLine
          : `(\n    ${entries.join(",\n    ")},\n  )`);
        break;
      }
      default: if (!eq(state[f.k], f.def)) push(f.k, fmtT(state[f.k]));
    }
  }
  }
  if (outputFormat === "svg") P.push('format: "svg"');
  const body = P.length ? `#${fn}(model,\n  ${P.join(",\n  ")},\n)` : `#${fn}(model)`;
  if (model.scad) {
    const skip = new Set(["specular: 0", "cull_backface: false", `color: "#f9d72c"`]);
    const args = P.filter(s => !skip.has(s));
    if (!state.scad_smooth_normals) args.unshift("smooth-normals: none");
    const inner = args.length
      ? `\n  read("model.scad"),\n  ${args.join(",\n  ")},\n`
      : `read("model.scad")`;
    return `#import "@preview/maquette-scad:0.1.0": render-scad\n\n#render-scad(${inner})`;
  }
  if (gltf) {
    return `#import "@preview/maquette-gltf:0.1.0": ${fn}\n\n#let model = read("${model.name}", encoding: none)\n\n${body}`;
  }
  if (model._mol) {
    // Same option surface as buildMolOpts — drop mesh-format (molfig.render
    // owns it) and format the rest as Typst named args.
    const opts = buildMolOpts(state, model._molFmt);
    delete opts["mesh-format"];
    if (opts.format === "auto") delete opts.format;
    const molArgs = Object.entries(opts).map(([k, v]) => `  ${k}: ${fmtT(v)}`);
    const cfgLines = P.filter(l => !l.startsWith("materials:"));
    const cfgBlock = cfgLines.length ? `  config: (\n    ${cfgLines.join(",\n    ")},\n  )` : "";
    const allArgs = [...molArgs, ...(cfgBlock ? [cfgBlock] : [])];
    return `#import "@preview/molfig:0.1.5"\n\n`
      + `#let data = read("${model._molSrcPath}", encoding: none)\n\n`
      + `#molfig.render(\n  data,\n${allArgs.join(",\n")},\n)`;
  }
  return `#import "@preview/maquette:0.1.3": ${fn}\n\n#let model = read("${model.name}", encoding: none)\n\n${body}`;
}

// Tiny Typst highlighter — the generated snippet has a small, known grammar, so a
// hand-rolled tokenizer beats pulling in a library and keeps the demo self-contained.
const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
const HL_RULES = [
  ["ws", /^\s+/], ["comment", /^\/\/[^\n]*/],
  ["string", /^"(?:[^"\\]|\\.)*"/], ["number", /^-?\d+(?:\.\d+)?/],
  ["directive", /^#[A-Za-z][\w-]*/], ["kw", /^(?:none|true|false|auto)\b/],
  ["ident", /^[A-Za-z_][\w-]*/], ["punct", /^[(){}\[\],:*+=]/],
];
function highlightLine(line) {
  let s = line, out = "";
  outer: while (s) {
    for (let [type, re] of HL_RULES) {
      const m = re.exec(s);
      if (!m) continue;
      const txt = m[0];
      if (type === "ident" && /^\s*:/.test(s.slice(txt.length))) type = "key"; // `key:` → property
      out += (type === "ws") ? esc(txt) : `<span class="t-${type}">${esc(txt)}</span>`;
      s = s.slice(txt.length);
      continue outer;
    }
    out += esc(s[0]); s = s.slice(1); // fallback: emit one char
  }
  return out || "&nbsp;";
}
function renderCode() {
  elCode.innerHTML = buildTypst().split("\n").map((l, i) =>
    `<div class="cline"><span class="gutter">${i + 1}</span><span class="src">${highlightLine(l)}</span></div>`
  ).join("");
}

// OpenSCAD highlighter for the editable source panel. Hand-rolled (same rationale
// as the Typst one) and block-comment aware (`/* … */` can span lines), reusing
// the shared .t-* token classes. Keywords vs built-in modules/functions get
// distinct colors; `$fn`/`$fa`/… render like directives.
const SCAD_KW = new Set(["module", "function", "if", "else", "for", "let", "each",
  "true", "false", "undef", "echo", "assert", "include", "use", "intersection_for", "return"]);
const SCAD_BUILTIN = new Set(["cube", "sphere", "cylinder", "polyhedron", "square", "circle",
  "polygon", "text", "translate", "rotate", "scale", "mirror", "resize", "multmatrix", "hull",
  "minkowski", "union", "difference", "intersection", "linear_extrude", "rotate_extrude", "offset",
  "projection", "color", "render", "children", "child", "import", "surface", "group",
  "sin", "cos", "tan", "asin", "acos", "atan", "atan2", "abs", "sign", "floor", "ceil", "round",
  "ln", "log", "pow", "sqrt", "exp", "min", "max", "norm", "cross", "concat", "len", "str",
  "chr", "ord", "search", "lookup", "rands", "is_undef", "is_num", "is_list", "is_string", "is_bool"]);
function highlightScad(text) {
  let inBlock = false;
  return text.split("\n").map((line) => {
    let s = line, out = "", m;
    while (s.length) {
      if (inBlock) {                                   // inside /* … */
        const end = s.indexOf("*/");
        const seg = end === -1 ? s : s.slice(0, end + 2);
        out += `<span class="t-comment">${esc(seg)}</span>`;
        if (end === -1) { s = ""; } else { s = s.slice(end + 2); inBlock = false; }
        continue;
      }
      if (m = /^\s+/.exec(s)) { out += esc(m[0]); s = s.slice(m[0].length); continue; }
      if (s.startsWith("//")) { out += `<span class="t-comment">${esc(s)}</span>`; s = ""; continue; }
      if (s.startsWith("/*")) {
        const end = s.indexOf("*/", 2);
        const seg = end === -1 ? s : s.slice(0, end + 2);
        out += `<span class="t-comment">${esc(seg)}</span>`;
        if (end === -1) { s = ""; inBlock = true; } else { s = s.slice(end + 2); }
        continue;
      }
      if (m = /^"(?:[^"\\]|\\.)*"/.exec(s)) { out += `<span class="t-string">${esc(m[0])}</span>`; s = s.slice(m[0].length); continue; }
      if (m = /^-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?/.exec(s)) { out += `<span class="t-number">${esc(m[0])}</span>`; s = s.slice(m[0].length); continue; }
      if (m = /^\$[A-Za-z_]\w*/.exec(s)) { out += `<span class="t-directive">${esc(m[0])}</span>`; s = s.slice(m[0].length); continue; }
      if (m = /^[A-Za-z_]\w*/.exec(s)) {
        const w = m[0];
        const cls = SCAD_KW.has(w) ? "t-kw" : SCAD_BUILTIN.has(w) ? "t-key" : null;
        out += cls ? `<span class="${cls}">${esc(w)}</span>` : esc(w);
        s = s.slice(w.length); continue;
      }
      if (m = /^[(){}\[\],:;*+\/=<>!&|%.?-]/.exec(s)) { out += `<span class="t-punct">${esc(m[0])}</span>`; s = s.slice(m[0].length); continue; }
      out += esc(s[0]); s = s.slice(1);
    }
    return out;
  }).join("\n");
}
function updateScadHighlight() {
  // trailing newline so the final line and the caret past it stay visible
  $("scad-hl").innerHTML = highlightScad($("scad-src").value) + "\n";
}

const MOL_ALWAYS_SEND = new Set(["mol_representation", "mol_color_theme", "mol_carbon_color"]);
function buildMolOpts(s, format) {
  const o = { format: format || "auto", "mesh-format": "obj" };
  const TF = topFields();
  for (const k in s) {
    if (!k.startsWith("mol_")) continue;
    const v = s[k];
    if (v === "" || v == null) continue;
    const f = TF[k];
    if (!MOL_ALWAYS_SEND.has(k) && f && eq(v, f.def)) continue;
    if (k === "mol_carbon_color") { (o.theme ??= {}).carbonColor = v; continue; }
    o[k.slice(4).replace(/_/g, "-")] = v;
  }
  return o;
}
// Bundle framing: "%08d%08d" (materials_len, info_len) || materials-json || info-json || mesh.


export { buildConfig, buildTypst, renderCode, updateScadHighlight, buildMolOpts, esc, highlightScad, highlightLine };

#import "@preview/zebraw:0.6.1": zebraw

#let parse-hl(text) = {
  let hl = ()
  let cols = (1, 1)
  let code = ()
  for line in text.split("\n") {
    let t = line.trim()
    if t.starts-with("// hl:") {
      for part in t.slice(6).trim().split(",") {
        let p = part.trim()
        if p.contains("-") {
          let b = p.split("-")
          for n in range(int(b.at(0).trim()), int(b.at(1).trim()) + 1) { hl.push(n) }
        } else if p != "" { hl.push(int(p)) }
      }
    } else if t.starts-with("// cols:") {
      let s = t.slice(8).trim().split(" ").filter(x => x != "")
      cols = (float(s.at(0)), float(s.at(1)))
    } else { code.push(line) }
  }
  (code.join("\n"), hl, cols)
}

#let filter-eval(text) = text.split("\n").filter(l =>
  not l.starts-with("#import") and not (l.starts-with("#let ") and l.contains("read("))
).join("\n")

#let png-only = box(fill: luma(225), inset: (x: 4pt, y: 1.5pt), radius: 3pt, baseline: 0.15em,
  text(size: 7pt, fill: luma(85), weight: "bold", tracking: 0.4pt)[PNG ONLY])

#let doc(name: "", scope: (:), stacked: false, body) = {
  set page(margin: 1.5em, footer: context grid(
    columns: (1fr, 1fr),
    align(left, text(size: 7.5pt, fill: luma(120))[#name documentation]),
    align(right, text(size: 7.5pt, fill: luma(120), counter(page).display())),
  ))
  set par(justify: true)
  show heading: set block(sticky: true)
  show: zebraw.with(lang: false, numbering: false)
  show raw.where(lang: "example"): it => {
    let (code, hl, cols) = parse-hl(it.text)
    let listing = zebraw(lang: false, numbering: false, highlight-lines: hl, raw(block: true, lang: "typst", code))
    let result = eval(filter-eval(code), mode: "markup", scope: scope)
    if stacked {
      block(breakable: false, width: 100%, stack(spacing: 0.5em, { set image(width: 100%); align(center, result) }, listing))
    } else {
      grid(columns: (cols.at(0) * 1fr, cols.at(1) * 1fr), gutter: 1em, listing, align(center + horizon, result))
    }
  }
  show raw.where(lang: "examplev"): it => {
    let (code, hl, ..) = parse-hl(it.text)
    let listing = zebraw(lang: false, numbering: false, highlight-lines: hl, raw(block: true, lang: "typst", code))
    let result = align(center, eval(filter-eval(code), mode: "markup", scope: scope))
    if stacked { block(breakable: false, width: 100%, stack(spacing: 0.5em, result, listing)) } else { listing; result }
  }
  body
}

#let cover(title, subtitle, hero, links, version) = {
  v(1fr)
  align(center)[
    #hero
    #v(1em)
    #text(font: "Libertinus Serif", size: 40pt, weight: "bold", title)
    #v(0.35em)
    #text(size: 15pt, fill: gray, subtitle)
    #v(1.6em)
    #{
      set text(size: 12pt)
      grid(
        columns: 2, column-gutter: 0.8em, row-gutter: 0.6em, align: (right, left),
        ..links.map(((label, href)) => (text(fill: luma(140), label), link(href, text(fill: blue, href.replace(regex("^https://"), "").trim("/", at: end))))).flatten(),
      )
    }
    #v(0.4em)
    #text(size: 10pt, fill: luma(150))[Version #version #h(0.4em)·#h(0.4em) #datetime.today().display("[month repr:long] [day], [year]")]
  ]
  v(1fr)
  pagebreak(weak: true)
}

#let contents() = {
  align(center, text(size: 20pt, weight: "bold", tracking: 2pt)[CONTENTS])
  v(1em)
  set text(size: 9pt)
  show link: it => if type(it.dest) == str { it.body } else { it }
  set outline.entry(fill: repeat(text(fill: luma(180))[.#h(2pt)]))
  show outline.entry.where(level: 1): it => { v(0.4em); strong(it) }
  columns(2, gutter: 2em, outline(title: none, indent: 1.2em))
  pagebreak(weak: true)
}

#let ref-table(header, rows, widths: (auto, auto, 1fr)) = table(
  columns: widths,
  align: left + horizon,
  inset: (x: 7pt, y: 5pt),
  stroke: none,
  fill: (_, y) => if y == 0 { luma(230) } else if calc.even(y) { luma(249) },
  table.header(..header.map(h => strong(h))),
  ..rows.flatten(),
)

#let labelled(label, body) = block(breakable: false, align(center)[
  #body
  #v(-0.4em)
  #text(size: 8.5pt, fill: luma(90), label)
])

#let demo-callout(url, what) = block(
  width: 100%, inset: (x: 12pt, y: 10pt), radius: 6pt,
  fill: rgb("#239DAD").lighten(88%), stroke: (left: 3pt + rgb("#239DAD")),
)[
  #text(weight: "bold", fill: rgb("#1b7c88"))[Try it live] #h(0.4em)
  #link(url, text(fill: blue, url))
  #v(-0.3em)
  #text(size: 9.5pt, what)
]

#let demo-base = "https://bernsteining.github.io/maquette/"
#let hexd = "0123456789ABCDEF"
#let safe-bytes = array(bytes("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~,:!$*;@/?[]{}\"()"))
#let pct(s) = {
  let out = ""
  for b in array(bytes(s)) {
    if safe-bytes.contains(b) { out += str(bytes((b,))) }
    else { out += "%" + hexd.slice(calc.quo(b, 16), calc.quo(b, 16) + 1) + hexd.slice(calc.rem(b, 16), calc.rem(b, 16) + 1) }
  }
  out
}
#let field-codes = ("model", "camera", "azimuth", "elevation", "distance", "center", "up", "projection",
  "fov", "zoom", "pan", "auto_center", "auto_fit", "background", "width", "height", "color", "opacity",
  "specular", "shininess", "smooth", "gamma_correction", "cull_backface", "shading", "gooch_warm",
  "gooch_cool", "cel_bands", "mode", "xray_opacity", "stroke", "wireframe", "light_dir", "ambient",
  "fresnel", "tone_mapping", "sss", "lights", "color_map", "overhang_angle", "scalar_function",
  "vertex_smoothing", "color_map_palette", "outline", "ground_shadow", "shadows", "antialias", "ssao",
  "bloom", "glow", "sharpen", "clip", "explode", "decimate", "views", "grid_labels", "turntable",
  "materials", "highlight", "annotations", "debug", "debug_color", "point_size", "point_neighbors",
  "point_boundary", "point_denoise", "point_splat", "_cam", "_hemi", "_bgNone")
#let code-alphabet = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"
#let code-for(i) = if i < 52 { code-alphabet.slice(i, i + 1) } else {
  code-alphabet.slice(calc.quo(i - 52, 52), calc.quo(i - 52, 52) + 1) + code-alphabet.slice(calc.rem(i - 52, 52), calc.rem(i - 52, 52) + 1)
}
#let key-alias = {
  let m = (:)
  for (i, k) in field-codes.enumerate() { m.insert(k, code-for(i)) }
  m
}
#let alias-key(k) = key-alias.at(k, default: k)
#let demo-url(name, cfg) = {
  let parts = (alias-key("model") + "=" + pct(name),)
  for (k, v) in cfg {
    if k in ("width", "height", "format", "read") { continue }
    let vs = if v == none { "none" } else if type(v) == str { v } else { str(json.encode(v, pretty: false)) }
    parts.push(alias-key(k) + "=" + pct(vs))
  }
  demo-base + "?" + parts.join("&")
}
#let cfg-of(args) = {
  let c = (:)
  if args.pos().len() >= 1 and type(args.pos().at(0)) == dictionary { c = args.pos().at(0) }
  c + args.named()
}
#let dm(name, data) = (maq-model: true, name: name, data: data)
#let unwrap(m) = if type(m) == dictionary and m.at("maq-model", default: false) { m.data } else { m }
#let wrap-render(fn, paths: (:)) = (m, ..args) => if type(m) == dictionary and m.at("maq-model", default: false) {
  link(demo-url(m.name, cfg-of(args)), fn(m.data, ..args))
} else if type(m) == str and m in paths {
  link(demo-url(paths.at(m), cfg-of(args)), fn(m, ..args))
} else { fn(m, ..args) }
#let wrap-info(fn) = (m, ..args) => fn(unwrap(m), ..args)

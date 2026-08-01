// maquette — render 3D models (STL, OBJ, PLY) as SVG or PNG images in Typst

#let maquette-plugin = plugin("maquette.wasm")

#let _parse-args(args) = {
  // Extract display args (not part of render config)
  let named = args.named()
  let width = named.at("width", default: auto)
  let height = named.at("height", default: auto)
  let format = named.at("format", default: "png")

  // Build config: named params (minus display args) merged with positional dict if any
  let config = (:)
  if args.pos().len() > 0 {
    let first = args.pos().at(0)
    if type(first) == dictionary {
      config = first
    }
  }
  for (k, v) in named {
    if k not in ("width", "height", "format") {
      config.insert(k, v)
    }
  }
  (
    cfg: bytes(json.encode(config)),
    width: width,
    height: height,
    format: format,
  )
}

#let _render(data, png-fn, svg-fn, args) = {
  let a = _parse-args(args)
  if a.format == "png" {
    let result = png-fn(data, a.cfg)
    if result.at(0) == 0x3C {
      // SVG-wrapped raster (annotations, debug overlay, or a labelled grid).
      image(result, format: "svg", width: a.width, height: a.height)
    } else {
      // Raw RGBA blob: [0x00][width u32 LE][height u32 LE][rgba8…]. Embedding
      // the pixels directly skips PNG encode (in the plugin) and decode (in
      // Typst) — and avoids re-compressing for the PDF.
      let w = result.at(1) + result.at(2) * 256 + result.at(3) * 65536 + result.at(4) * 16777216
      let h = result.at(5) + result.at(6) * 256 + result.at(7) * 65536 + result.at(8) * 16777216
      image(
        result.slice(9),
        format: (encoding: "rgba8", width: w, height: h),
        width: a.width,
        height: a.height,
      )
    }
  } else {
    image(svg-fn(data, a.cfg), format: "svg", width: a.width, height: a.height)
  }
}

#let render-stl(stl-data, ..args) = {
  _render(stl-data, maquette-plugin.render_stl_png, maquette-plugin.render_stl, args)
}

#let render-obj(obj-data, ..args) = {
  let data = bytes(obj-data)
  _render(data, maquette-plugin.render_obj_png, maquette-plugin.render_obj, args)
}

#let render-ply(ply-data, ..args) = {
  _render(ply-data, maquette-plugin.render_ply_png, maquette-plugin.render_ply, args)
}

#let get-stl-info(stl-data, ..args) = {
  let a = _parse-args(args)
  json(maquette-plugin.get_stl_info(stl-data, a.cfg))
}

#let get-obj-info(obj-data, ..args) = {
  let a = _parse-args(args)
  json(maquette-plugin.get_obj_info(bytes(obj-data), a.cfg))
}

#let get-ply-info(ply-data, ..args) = {
  let a = _parse-args(args)
  json(maquette-plugin.get_ply_info(ply-data, a.cfg))
}

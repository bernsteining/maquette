#import "@preview/zebraw:0.5.5": zebraw
#set page(width: 420pt, height: auto, margin: 10pt)
#zebraw(lang: [Typst DSL · typc], numbering: false, raw("difference(
  cube(10, center: true),
  sphere(6, fn: 48),
)", lang: "typc", block: true))

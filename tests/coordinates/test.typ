#import "../../maquette/maquette.typ":*
#let teapot = read("../../examples/data/teapot.obj")

#set page(width: auto, height: auto, margin: 0pt)
#render-obj(
  teapot, 
  width: 5cm,
  (
    up: (0, 1, 0),
    wasd:"aaaaaaaawwwwwaaaadddddssaaaaaaaaaaaaaaaaaa",
    background:"ffffff",
    distance: 6,
  ),
)
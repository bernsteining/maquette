#import "surface-plot.typ": surface-plot
#set page(width: 520pt, height: 520pt, margin: 0pt, fill: white)
#surface-plot(
  (x, y) => calc.sin(x) * calc.cos(y),
  x-range: (-4, 4), y-range: (-4, 4), samples: 60, z-scale: 1.4,
)

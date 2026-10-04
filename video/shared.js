export const FPS = 30, DUR = 27, W = 1920, H = 1080;
export const BPM = 120, BEAT = 60 / BPM;
const b = (n) => n * BEAT;
export const T = { s1: 0, s2: b(6), s3: b(16), cut: b(3), s4a: b(34), s4b: b(39), s5: b(44), s6: b(48), end: DUR };
export const clamp = (x, a = 0, b = 1) => Math.min(b, Math.max(a, x));
export const prog = (t, a, b) => clamp((t - a) / (b - a));
export const lerp = (a, b, p) => a + (b - a) * p;
export const easeOutCubic = (p) => 1 - Math.pow(1 - p, 3);
export const easeOutExpo = (p) => (p >= 1 ? 1 : 1 - Math.pow(2, -10 * p));
export const easeInOut = (p) => (p < 0.5 ? 4 * p * p * p : 1 - Math.pow(-2 * p + 2, 3) / 2);
export const easeInCubic = (p) => p * p * p;
export const easeOutBack = (p) => { const c1 = 1.70158, c3 = c1 + 1; return 1 + c3 * Math.pow(p - 1, 3) + c1 * Math.pow(p - 1, 2); };

export const S2 = { type0: 3.3, type1: 3.9, compile: b(8), scrub0: b(9), scrubMid: 5.25, scrub1: 5.95, sss: b(12), sssFade: 0.3, chips: 6.75 };
export function azS2(t) {
  if (t < S2.scrub0) return 180;
  if (t < S2.scrubMid) return lerp(180, 228, easeInOut(prog(t, S2.scrub0, S2.scrubMid)));
  if (t < S2.scrub1) return lerp(228, 150, easeInOut(prog(t, S2.scrubMid, S2.scrub1)));
  return lerp(150, 185, easeOutCubic(prog(t, S2.scrub1, T.s3)));
}

export const CUTS = [
  { id: "lights", label: "Multiple\nlights", code: "lights: (red, cyan, gold)", bg: "#0e0f13", fg: "#f6f7f9" },
  { id: "glow", label: "Glow", code: 'glow: (color: "#22d3ee")', bg: "#4338ca", fg: "#ffffff" },
  { id: "xray", label: "X-ray", code: 'mode: "x-ray"', bg: "#0e0f13", fg: "#f6f7f9" },
  { id: "ann", label: "Annotations", code: "annotations: true", bg: "#f6f7f9", fg: "#0e0f13" },
  { id: "clip", label: "Section\ncuts", code: "clip: (plane: (2, -1, 0, 1))", bg: "#239DAD", fg: "#0b1418" },
  { id: "explode", label: "Exploded\nviews", code: "explode: 0.8", bg: "#f9d72c", fg: "#14150f" },
];
export function cutAt(t) {
  const i = clamp(Math.floor((t - T.s3) / T.cut + 1e-9), 0, CUTS.length - 1);
  return { i, u: t - T.s3 - i * T.cut };
}
export const azS3 = (t) => 185 + 40 * (t - T.s3);

export function scadHole(t) {
  const a = T.s4a;
  if (t < a + 0.6) return 25;
  if (t < a + 1.15) return lerp(25, 13, easeInOut(prog(t, a + 0.6, a + 1.15)));
  if (t < a + 1.8) return lerp(13, 31, easeInOut(prog(t, a + 1.15, a + 1.8)));
  return lerp(31, 25, easeInOut(prog(t, a + 1.8, a + 2.2)));
}
export const scadAz = (t) => 205 + 16 * (t - T.s4a);
export const scadSrc = (hole) => `$fn = 100;
module rod(d, h) cylinder(h, d = d, center = true);
hole = ${hole};
len = 62.5;
difference() {
  sphere(d = 50);
  rod(hole, len);
  rotate([90, 0, 0]) rod(hole, len);
}
color([0.5, 0.3, 0.1, 0.6])
rotate([0, 90, 0]) rod(hole, len);
`;

export const tokyoParams = (t) => ({ time: 5.4 + (t - T.s4b), angle: lerp(-13, 9, prog(t, T.s4b, T.s5)) });

export const pad = (f) => String(f).padStart(4, "0");
export const frameFile = (seq, f) => `seq/${seq}/${pad(f)}.webp`;

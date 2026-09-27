const sub = (a, b) => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const add = (a, b) => [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
const scale = (a, s) => [a[0] * s, a[1] * s, a[2] * s];
const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const length = (a) => Math.hypot(a[0], a[1], a[2]);
const normalize = (a) => { const l = length(a) || 1; return [a[0] / l, a[1] / l, a[2] / l]; };

function rotate(v, k, angle) {
  const c = Math.cos(angle), s = Math.sin(angle), d = dot(k, v), x = cross(k, v);
  return [0, 1, 2].map((i) => v[i] * c + x[i] * s + k[i] * d * (1 - c));
}

function basis(up) {
  const arbitrary = Math.abs(up[0]) < 0.9 ? [1, 0, 0] : [0, 1, 0];
  const right = normalize(cross(up, arbitrary));
  return { right, forward: normalize(cross(right, up)) };
}

const DEG = Math.PI / 180;

function fromSpherical(up, azimuthDeg, elevationDeg) {
  const { right, forward } = basis(up);
  const az = azimuthDeg * DEG, el = elevationDeg * DEG;
  return normalize(add(add(scale(right, Math.cos(el) * Math.cos(az)), scale(forward, Math.cos(el) * Math.sin(az))), scale(up, Math.sin(el))));
}

function toSpherical(dir, up) {
  const { right, forward } = basis(up);
  return {
    azimuth: Math.atan2(dot(dir, forward), dot(dir, right)) / DEG,
    elevation: Math.asin(Math.max(-1, Math.min(1, dot(dir, up)))) / DEG,
  };
}

export { sub, add, scale, dot, cross, length, normalize, rotate, basis, fromSpherical, toSpherical, DEG };

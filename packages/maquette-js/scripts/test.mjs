// Smoke test: render an OpenSCAD source and an OBJ file, check the raster.
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { createMaquette } from "../src/index.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");
const mq = createMaquette();

const scad = await mq.renderScad(
  "difference(){ cube(20, center=true); sphere(12); }",
  { width: 200, height: 200, azimuth: 30, shading: "gooch" },
  { facets: 48 }
);
console.log("renderScad:", scad.width + "x" + scad.height, scad.pixels.length, "bytes", scad.width === 200 ? "OK" : "FAIL");

const svg = await mq.renderScad("cube(10);", { width: 150, height: 150 }, { format: "svg" });
console.log("renderScad svg:", svg.startsWith("<svg") ? "OK" : "FAIL");

const obj = await readFile(join(root, "examples/data/teapot.obj"));
const r = await mq.renderObj(new Uint8Array(obj), { width: 240, height: 180, azimuth: 40 });
console.log("renderObj(teapot):", r.width + "x" + r.height, r.width === 240 ? "OK" : "FAIL");

const info = await mq.infoObj(new Uint8Array(obj));
console.log("infoObj keys:", Object.keys(info).slice(0, 4).join(","), "OK");

try {
  await mq.renderScad("@@@ not scad @@@");
  console.log("error path: FAIL (no throw)");
} catch (e) {
  console.log("invalid scad → threw:", String(e.message).slice(0, 40), "OK");
}

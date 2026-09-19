// Stage the plugin wasm into wasm/ for packaging. Prefers the freshly-built
// crate outputs; falls back to the committed docs/ copies.
import { copyFile, mkdir, access } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");
const dest = join(dirname(fileURLToPath(import.meta.url)), "..", "wasm");

const files = [
  ["crates/maquette/maquette.wasm", "maquette.wasm"],
  ["crates/maquette-gltf/maquette-gltf.wasm", "maquette-gltf.wasm"],
  ["crates/maquette-scad/maquette-scad.wasm", "maquette-scad.wasm"],
];

const exists = async (p) => { try { await access(p); return true; } catch { return false; } };

await mkdir(dest, { recursive: true });
for (const [rel, name] of files) {
  const primary = join(root, rel);
  const fallback = join(root, "docs", name);
  const src = (await exists(primary)) ? primary : fallback;
  await copyFile(src, join(dest, name));
  console.log(`copied ${name}  <-  ${src.replace(root + "/", "")}`);
}

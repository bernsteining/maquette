export interface Raster {
  width: number;
  height: number;
  /** Row-major RGBA8 pixels (width * height * 4 bytes). */
  pixels: Uint8Array;
}

export type Config = Record<string, unknown> | string | null;
export type Data = Uint8Array | ArrayBufferLike | string;

export interface RenderOptions {
  /** "png" → a decoded {@link Raster} (default); "svg" → an SVG string. */
  format?: "png" | "svg";
}
export interface ScadOptions extends RenderOptions {
  /** Default facet count ($fn) for round shapes. */
  facets?: number;
}

export interface Maquette {
  renderStl(data: Data, config?: Config, opts?: RenderOptions): Promise<Raster | string>;
  renderObj(data: Data, config?: Config, opts?: RenderOptions): Promise<Raster | string>;
  renderPly(data: Data, config?: Config, opts?: RenderOptions): Promise<Raster | string>;
  /** glTF (.glb/.gltf), PBR — PNG raster only. */
  renderGltf(data: Data, config?: Config): Promise<Raster>;
  renderScad(src: string, config?: Config, opts?: ScadOptions): Promise<Raster | string>;
  /** OpenSCAD → PLY bytes. */
  compileScad(src: string, opts?: { facets?: number }): Promise<Uint8Array>;
  infoStl(data: Data): Promise<any>;
  infoObj(data: Data): Promise<any>;
  infoPly(data: Data): Promise<any>;
  infoGltf(data: Data): Promise<any>;
  decodeRaster(bytes: Uint8Array): Raster;
  toImageData(raster: Raster): ImageData;
}

export interface CreateOptions {
  /** Override how plugin wasm files are fetched (e.g. a custom URL/path). */
  load?: (file: string) => Promise<Uint8Array>;
}

export function createMaquette(opts?: CreateOptions): Maquette;
export function decodeRaster(bytes: Uint8Array): Raster;
export function toImageData(raster: Raster): ImageData;

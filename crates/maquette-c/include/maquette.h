/* maquette — C ABI for a headless CPU 3D renderer (STL/OBJ/PLY, glTF, OpenSCAD).
 *
 * Every function returns 0 on success, 1 on error. On return, *out points to a
 * heap buffer of *out_len bytes owned by YOU: the rendered bytes on success
 * (PNG, or SVG/PLY/JSON text — not NUL-terminated), or a UTF-8 error message on
 * error. Free it with maquette_free(*out, *out_len). `config` is a JSON string
 * of the render-config keys, or NULL for defaults.
 */
#ifndef MAQUETTE_H
#define MAQUETTE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* mesh renderers: data = file bytes, config = JSON or NULL */
int32_t maquette_render_stl_png(const uint8_t *data, size_t len, const char *config, uint8_t **out, size_t *out_len);
int32_t maquette_render_stl_svg(const uint8_t *data, size_t len, const char *config, uint8_t **out, size_t *out_len);
int32_t maquette_render_obj_png(const uint8_t *data, size_t len, const char *config, uint8_t **out, size_t *out_len);
int32_t maquette_render_obj_svg(const uint8_t *data, size_t len, const char *config, uint8_t **out, size_t *out_len);
int32_t maquette_render_ply_png(const uint8_t *data, size_t len, const char *config, uint8_t **out, size_t *out_len);
int32_t maquette_render_ply_svg(const uint8_t *data, size_t len, const char *config, uint8_t **out, size_t *out_len);

/* glTF (.glb/.gltf) — PBR, PNG only */
int32_t maquette_render_gltf_png(const uint8_t *data, size_t len, const char *config, uint8_t **out, size_t *out_len);

/* OpenSCAD: src = NUL-terminated source, facets = default $fn */
int32_t maquette_compile_scad(const char *src, size_t facets, uint8_t **out, size_t *out_len);            /* -> PLY bytes */
int32_t maquette_render_scad_png(const char *src, const char *config, size_t facets, uint8_t **out, size_t *out_len);
int32_t maquette_render_scad_svg(const char *src, const char *config, size_t facets, uint8_t **out, size_t *out_len);

/* metadata -> JSON bytes (triangle/vertex count, bbox, …) */
int32_t maquette_info_stl(const uint8_t *data, size_t len, const char *config, uint8_t **out, size_t *out_len);
int32_t maquette_info_obj(const uint8_t *data, size_t len, const char *config, uint8_t **out, size_t *out_len);
int32_t maquette_info_ply(const uint8_t *data, size_t len, const char *config, uint8_t **out, size_t *out_len);
int32_t maquette_info_gltf(const uint8_t *data, size_t len, const char *config, uint8_t **out, size_t *out_len);

/* free a buffer returned via *out */
void maquette_free(uint8_t *ptr, size_t len);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* MAQUETTE_H */

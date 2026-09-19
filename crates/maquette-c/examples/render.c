#include "maquette.h"
#include <stdio.h>
#include <string.h>

int main(void) {
    const char *src = "difference(){ cube(20, center=true); sphere(12); }";
    const char *cfg = "{\"width\":256,\"height\":256,\"azimuth\":30,\"shading\":\"gooch\"}";
    uint8_t *out = NULL; size_t n = 0;

    int rc = maquette_render_scad_png(src, cfg, 48, &out, &n);
    if (rc != 0) { fprintf(stderr, "error: %.*s\n", (int)n, out); maquette_free(out, n); return 1; }
    if (n < 8 || memcmp(out, "\x89PNG\r\n\x1a\n", 8) != 0) { fprintf(stderr, "not a PNG\n"); return 1; }
    FILE *f = fopen("/tmp/mqc.png", "wb"); fwrite(out, 1, n, f); fclose(f);
    printf("render_scad_png -> /tmp/mqc.png (%zu bytes) OK\n", n);
    maquette_free(out, n);

    /* error path: invalid scad -> rc=1, message in out */
    uint8_t *e = NULL; size_t en = 0;
    rc = maquette_render_scad_png("@@@ not scad @@@", NULL, 32, &e, &en);
    printf("invalid scad -> rc=%d, msg=\"%.*s\" %s\n", rc, (int)(en>40?40:en), e, rc==1 ? "OK" : "FAIL");
    maquette_free(e, en);
    return 0;
}

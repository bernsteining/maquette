import asyncio, io, json, os, subprocess, sys, time
import numpy as np
from PIL import Image
from playwright.async_api import async_playwright

FPS, DUR = 30, 40
SUB, SHUTTER = 4, 0.5
W, H = 1920, 1080
out = sys.argv[1] if len(sys.argv) > 1 else "frames"
only = [int(x) for x in sys.argv[2].split(",")] if len(sys.argv) > 2 else list(range(FPS * DUR))
N = int(os.environ.get("WORKERS", "4"))
os.makedirs(out, exist_ok=True)

beats = json.load(open("beats.json"))
DROPS = [(h["t"], h.get("amp", 1)) for h in beats["hits"] if h["kind"] in ("drop", "final")]
HITS = [(h["t"], h.get("amp", 1)) for h in beats["hits"]]
yy, xx = np.mgrid[0:H, 0:W].astype(np.float32)
radius = np.sqrt(((xx - W / 2) / (W / 2)) ** 2 + ((yy - H / 2) / (H / 2)) ** 2)
VIGNETTE = (1 - 0.2 * np.clip(radius - 0.6, 0, None) ** 1.5)[..., None]
LEAK_COLOR = np.array([1.0, 0.58, 0.28], np.float32)


def envelope(t, times, decay):
    e = 0.0
    for tk, amp in times:
        d = t - tk
        if 0 <= d < 1.5:
            e = max(e, amp * float(np.exp(-d * decay)))
    return e


def finish(x, f):
    t = f / FPS
    d, c = envelope(t, DROPS, 4.5), envelope(t, HITS, 10)
    if d > 0.02:
        cx, cy = W * (0.15 + 0.7 * (1 - d)), H * 0.28
        blob = np.exp(-(((xx - cx) / (W * 0.33)) ** 2 + ((yy - cy) / (H * 0.5)) ** 2) / 2)[..., None]
        x = 1 - (1 - x) * (1 - 0.36 * d * blob * LEAK_COLOR)
    shift = int(round(5 * d + 2 * c))
    if shift:
        x[..., 0] = np.roll(x[..., 0], shift, axis=1)
        x[..., 2] = np.roll(x[..., 2], -shift, axis=1)
    x = x * VIGNETTE
    if d > 0.02:
        x = x + np.random.default_rng(f).normal(0, 0.006 * d, (H // 2, W // 2, 1)).astype(np.float32).repeat(2, 0).repeat(2, 1)
    return Image.fromarray((np.clip(x, 0, 1) * 255 + 0.5).astype(np.uint8))


srv = subprocess.Popen([sys.executable, "-m", "http.server", "8765", "--bind", "127.0.0.1"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
time.sleep(1)


async def worker(browser, k):
    page = await browser.new_page(viewport={"width": W, "height": H}, device_scale_factor=1)
    page.on("pageerror", lambda e: print("PAGEERROR", e))
    page.on("console", lambda m: print("CONSOLE", m.text) if m.type == "error" else None)
    await page.goto("http://127.0.0.1:8765/index.html")
    await page.wait_for_function("window.ready === true", timeout=30000)
    for f in only[k::N]:
        acc = np.zeros((H, W, 3), np.float32)
        for s in range(SUB):
            errs = await page.evaluate("f => window.seek(f)", f + s * SHUTTER / SUB)
            if errs:
                print("missing images at frame", f, await page.evaluate("window.__errors.splice(0)"))
            shot = await page.screenshot(type="png")
            acc += np.asarray(Image.open(io.BytesIO(shot)).convert("RGB"), np.float32)
        finish(acc / (SUB * 255), f).save(f"{out}/{f:04d}.png")
    await page.close()


async def main():
    async with async_playwright() as p:
        b = await p.chromium.launch(args=["--font-render-hinting=none", "--disable-lcd-text"])
        await asyncio.gather(*[worker(b, k) for k in range(N)])
        await b.close()


try:
    asyncio.run(main())
finally:
    srv.terminate()

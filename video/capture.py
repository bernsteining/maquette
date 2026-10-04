import asyncio, sys, os, subprocess, time
from playwright.async_api import async_playwright
FPS, DUR = 30, 27
out = sys.argv[1] if len(sys.argv) > 1 else "frames"
only = [int(x) for x in sys.argv[2].split(",")] if len(sys.argv) > 2 else list(range(FPS * DUR))
N = int(os.environ.get("WORKERS", "4"))
os.makedirs(out, exist_ok=True)
srv = subprocess.Popen([sys.executable, "-m", "http.server", "8765", "--bind", "127.0.0.1"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
time.sleep(1)
async def worker(browser, k):
    page = await browser.new_page(viewport={"width": 1920, "height": 1080}, device_scale_factor=1)
    page.on("pageerror", lambda e: print("PAGEERROR", e))
    page.on("console", lambda m: print("CONSOLE", m.text) if m.type == "error" else None)
    await page.goto("http://127.0.0.1:8765/index.html")
    await page.wait_for_function("window.ready === true", timeout=30000)
    for f in only[k::N]:
        errs = await page.evaluate("f => window.seek(f)", f)
        if errs: print("missing images at frame", f, await page.evaluate("window.__errors.splice(0)"))
        await page.screenshot(path=f"{out}/{f:04d}.png", type="png")
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

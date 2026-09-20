

const _elCache = {};
const $ = (id) => id in _elCache ? _elCache[id] : (_elCache[id] = document.getElementById(id));

const ENC = new TextEncoder(), DEC = new TextDecoder();
const elCode = $("code"), elErr = $("err"), elOut = $("out"), elOutc = $("outc"), elRtime = $("rtime"), elMeasure = $("measure");

(function () {
  const el = $("build"); if (!el) return;
  el.style.cursor = "pointer";
  const stamped = el.dataset.commit;
  let v = stamped && !stamped.startsWith("__") ? stamped : "";   // CI stamps the SHA into index.html
  const wire = () => {
    el.title = v ? "build · " + v : "dev build";
    el.onclick = () => { const o = el.textContent; el.textContent = v || "dev"; setTimeout(() => (el.textContent = o), 1600); };
  };
  wire();
  // Local (`make demo`): no HTML stamp, so read the short SHA from build.txt.
  if (!v) fetch("build.txt").then(r => r.ok ? r.text() : "").then(t => {
    t = (t || "").trim();
    if (/^[0-9a-f]{7,40}$/.test(t)) { v = t.slice(0, 12); wire(); }
  }).catch(() => {});
})();


// Announce a status message to screen readers via the polite live region.
// Clears first so the same text re-announces (e.g. reloading the same model).
function announce(msg) {
  const el = $("sr-live"); if (!el) return;
  el.textContent = "";
  requestAnimationFrame(() => { el.textContent = msg; });
}

export { $, ENC, DEC, elCode, elErr, elOut, elOutc, elRtime, elMeasure, announce };



const _elCache = {};
const $ = (id) => id in _elCache ? _elCache[id] : (_elCache[id] = document.getElementById(id));

const ENC = new TextEncoder(), DEC = new TextDecoder();
const elCode = $("code"), elErr = $("err"), elOut = $("out"), elOutc = $("outc"), elRtime = $("rtime"), elMeasure = $("measure");

(function () {
  const el = $("build"); if (!el) return;
  const c = el.dataset.commit;
  const v = c && !c.startsWith("__") ? c : "dev";
  el.title = "deployed build · " + v;
  el.style.cursor = "pointer";
  el.onclick = () => { const o = el.textContent; el.textContent = v; setTimeout(() => (el.textContent = o), 1600); };
})();


export { $, ENC, DEC, elCode, elErr, elOut, elOutc, elRtime, elMeasure };

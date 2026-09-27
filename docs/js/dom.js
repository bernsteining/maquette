const elCache = {};
const $ = (id) => {
  const hit = elCache[id];
  return hit && hit.isConnected !== false ? hit : (elCache[id] = document.getElementById(id));
};

const ENC = new TextEncoder(), DEC = new TextDecoder();
const elCode = $("code"), elErr = $("err"), elOut = $("out"), elOverlay = $("outo"), elRtime = $("rtime"), elMeasure = $("measure");

function announce(msg) {
  const el = $("sr-live"); if (!el) return;
  el.textContent = "";
  requestAnimationFrame(() => { el.textContent = msg; });
}

function flashLabel(el, text, ms) {
  const o = el.textContent;
  el.textContent = text;
  setTimeout(() => (el.textContent = o), ms);
}

export { $, ENC, DEC, elCode, elErr, elOut, elOverlay, elRtime, elMeasure, announce, flashLabel };

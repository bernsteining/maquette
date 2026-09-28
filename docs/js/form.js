import { $ } from "./dom.js";
import { state, model, getSchema } from "./state.js";
import { hooks } from "./hooks.js";

const controlRefs = {};
const conds = [];
const searchItems = [];
const searchSections = [];
const VEC_AXES = ["X", "Y", "Z", "W"];

function el(tag, props = {}, ...children) {
  const e = document.createElement(tag);
  for (const k in props) {
    if (k === "attrs") for (const a in props.attrs) e.setAttribute(a, props.attrs[a]);
    else e[k] = props[k];
  }
  e.append(...children);
  return e;
}
const changed = () => hooks.change();

function numStep(f, v) {
  const s = String(v);
  const dot = s.indexOf(".");
  const step = 10 ** -(dot < 0 ? 0 : s.length - dot - 1);
  return f.step != null ? Math.min(step, f.step) : step;
}

function ctl(f, slot, local) {
  const wrap = el("div", { className: "ctl" });
  const set = (v) => {
    slot[f.k] = v;
    if (f.onSet && f.onSet(v, state)) rebuildForm();
    changed();
    if (f.recompile) hooks.recompile(f.recompile);
  };
  const cur = slot[f.k];
  let labelEl, sync = null;

  if (f.t === "bool") {
    const cb = el("input", { type: "checkbox", checked: !!cur, onchange: () => set(cb.checked) });
    labelEl = el("label", { className: "chk" }, cb, f.label);
    wrap.append(labelEl);
    sync = (v) => { cb.checked = !!v; };
  } else {
    labelEl = el("label", {}, el("span", { textContent: f.label }));
    let input;
    if (f.t === "sel") {
      input = el("select", { attrs: { "aria-label": f.label } }, ...f.opts.map(([v, t]) => el("option", { value: v, textContent: t })));
      input.value = cur;
      input.onchange = () => set(f.num ? +input.value : input.value);
      sync = (v) => { input.value = v; };
    } else if (f.t === "rng") {
      const valEl = el("span", { className: "val" });
      labelEl.append(valEl);
      const range = el("input", { type: "range", min: f.min, max: f.max, step: f.step, attrs: { "aria-label": f.label } });
      input = range;
      sync = (v) => {
        const t = (+v).toFixed(2);
        range.value = v; valEl.textContent = t;
        range.setAttribute("aria-valuetext", t);
      };
      sync(cur);
      const scrub = (v) => {
        slot[f.k] = v;
        if (f.onSet && f.onSet(v, state)) rebuildForm();
        hooks.scrub();
        if (f.recompile) hooks.recompile(f.recompile);
      };
      range.oninput = () => { sync(range.value); scrub(+range.value); };
      if (f.play) {
        let playing = false;
        const btn = el("button", { type: "button", className: "play", textContent: "▶", attrs: { "aria-label": "Play animation" } });
        const stop = () => { playing = false; btn.textContent = "▶"; btn.setAttribute("aria-label", "Play animation"); };
        btn.onclick = async () => {
          if (playing) return stop();
          playing = true; btn.textContent = "⏸"; btn.setAttribute("aria-label", "Pause animation");
          const lo = +f.min, span = +f.max - lo, step = +f.step || 0.01;
          const start = performance.now() - (+range.value - lo) * 1000;
          while (playing && btn.isConnected) {
            const t = ((performance.now() - start) / 1000) % span;
            const v = +(lo + Math.round(t / step) * step).toFixed(6);
            sync(v); slot[f.k] = v;
            await hooks.frame();
            await new Promise((r) => requestAnimationFrame(r));
          }
          stop();
        };
        input = el("div", { className: "play-row" }, range, btn);
      }
    } else if (f.t === "num") {
      const ni = el("input", { type: "number", value: cur, attrs: { "aria-label": f.label } });
      if (f.min != null) ni.min = f.min;
      if (f.max != null) ni.max = f.max;
      const current = () => ni.value === "" ? (f.def || 0) : +ni.value;
      const applyStep = () => { ni.step = String(numStep(f, current())); };
      applyStep();
      ni.oninput = () => { applyStep(); set(ni.value === "" ? f.def : +ni.value); };
      const stepBy = (dir) => {
        const base = current();
        let nv = +(base + dir * numStep(f, base)).toFixed(12);
        if (f.min != null) nv = Math.max(f.min, nv);
        if (f.max != null) nv = Math.min(f.max, nv);
        ni.value = nv; applyStep(); set(nv);
      };
      const spinBtn = (text, dir) => el("button", { type: "button", tabIndex: -1, textContent: text, onclick: () => stepBy(dir), attrs: { "aria-hidden": "true" } });
      input = el("div", { className: "num-wrap" }, ni, el("div", { className: "spin" }, spinBtn("▲", 1), spinBtn("▼", -1)));
      sync = (v) => { ni.value = v; applyStep(); };
    } else if (f.t === "col") {
      input = el("input", { type: "color", value: cur || "#000000", attrs: { "aria-label": f.label } });
      input.oninput = () => set(input.value);
      sync = (v) => { input.value = v || "#000000"; };
    } else if (f.t === "txt") {
      input = el("input", { type: "text", value: cur, attrs: { "aria-label": f.label } });
      input.oninput = () => set(input.value);
      sync = (v) => { input.value = v; };
    } else if (f.t === "vec") {
      input = el("div", { className: "row", attrs: { role: "group", "aria-label": f.label } });
      cur.forEach((n, i) => {
        const ni = el("input", { type: "number", value: n, step: "any", attrs: { "aria-label": `${f.label} ${VEC_AXES[i] || i}` } });
        ni.oninput = () => { const a = slot[f.k].slice(); a[i] = +ni.value; set(a); };
        input.append(ni);
      });
      sync = (v) => v.forEach((n, i) => { if (input.children[i]) input.children[i].value = n; });
    }
    wrap.append(labelEl, input);
  }
  attachTip(labelEl, f.help);
  if (slot === state && sync) controlRefs[f.k] = sync;
  if (f.when) conds.push({ node: wrap, when: f.when, local });
  return wrap;
}

function groupNode(f) {
  const box = el("div", { className: "group" + (f.toggle && !state[f.k].__on ? " off" : "") });
  const head = el("label", { className: "head" });
  if (f.toggle) {
    const cb = el("input", { type: "checkbox", checked: state[f.k].__on });
    cb.onchange = () => { state[f.k].__on = cb.checked; box.classList.toggle("off", !cb.checked); changed(); };
    head.append(cb);
  }
  head.append(f.label);
  attachTip(head, f.help);
  box.append(head, el("div", { className: "sub" }, ...f.fields.map((s) => ctl(s, state[f.k], state[f.k]))));
  return box;
}

const removeButton = (onclick) => el("button", { className: "rm", textContent: "✕", title: "remove", onclick });
const labeled = (label, child) => el("div", { className: "ctl" }, el("label", {}, el("span", { textContent: label })), child);

function dynList(arr, addLabel, newItem, renderItem, wrapRow = false) {
  const box = el("div");
  const rerender = () => {
    box.replaceChildren();
    const host = wrapRow ? el("div", { className: "row wrap" }) : box;
    if (wrapRow) box.append(host);
    arr.forEach((item, i) => host.append(renderItem(item, i, rerender)));
    box.append(el("button", { className: "mini-btn", textContent: addLabel, onclick: () => { arr.push(newItem()); rerender(); changed(); } }));
  };
  rerender();
  return box;
}

function lightsNode() {
  return dynList(state.lights, "+ add light",
    () => ({ type: "directional", vector: [1, 2, 3], color: "#ffffff", intensity: 1, cast_shadow: true, size: 0 }),
    (L, i, rerender) => {
      const type = el("select", {}, ...[["directional", "Directional"], ["positional", "Positional"], ["area", "Area"]].map(([v, t]) => el("option", { value: v, textContent: t })));
      type.value = L.type;
      type.onchange = () => { L.type = type.value; changed(); };
      const vec = el("div", {}, ...["X", "Y", "Z"].map((axis, j) => {
        const rng = el("input", { type: "range", min: "-3", max: "3", step: "0.05", value: L.vector[j] });
        const ni = el("input", { type: "number", step: "any", value: L.vector[j] });
        rng.oninput = () => { L.vector[j] = +rng.value; ni.value = rng.value; changed(); };
        ni.oninput = () => { L.vector[j] = +ni.value; if (Math.abs(+ni.value) <= 3) rng.value = ni.value; changed(); };
        return el("div", { className: "light-axis" }, el("span", { textContent: axis }), rng, ni);
      }));
      const field = (type, value, apply) => { const e = el("input", { type, value }); if (type === "number") e.step = "any"; e.oninput = () => { apply(e.value); changed(); }; return e; };
      const cast = el("input", { type: "checkbox", checked: L.cast_shadow, onchange: () => { L.cast_shadow = cast.checked; changed(); } });
      return el("div", { className: "item" },
        el("div", { className: "item-head" }, type, el("span", { className: "spacer" }), removeButton(() => { state.lights.splice(i, 1); rerender(); changed(); })),
        labeled("Vector", vec),
        labeled("Color", field("color", L.color, (v) => { L.color = v; })),
        labeled("Intensity", field("number", L.intensity, (v) => { L.intensity = +v; })),
        labeled("Size (area)", field("number", L.size, (v) => { L.size = +v; })),
        el("label", { className: "chk" }, cast, "casts shadow"));
    });
}

function paletteNode(f) {
  return dynList(state[f.k], "+ color", () => "#888888",
    (c, i, rerender) => {
      const col = el("input", { type: "color", value: c, oninput: () => { state[f.k][i] = col.value; changed(); } });
      return el("div", { className: "swatch" }, col, el("button", { className: "rm", textContent: "✕", onclick: () => { state[f.k].splice(i, 1); rerender(); changed(); } }));
    }, true);
}

function mapNode(f) {
  const remove = (i, rerender) => removeButton(() => { state[f.k].splice(i, 1); rerender(); changed(); });
  const input = (type, value, apply, attrs) => el("input", { type, value, attrs: attrs || {}, oninput: (e) => { apply(e.target.value); changed(); } });
  if (f.rich) return dynList(state[f.k], "+ entry",
    () => ["", { color: "#88ccff", stroke: "", stroke_width: 0, opacity: 1 }],
    (row, i, rerender) => {
      const v = row[1];
      return el("div", { className: "item" },
        el("div", { className: "row map-row" },
          input("text", row[0], (x) => { row[0] = x; }, { placeholder: "group name" }),
          input("color", v.color, (x) => { v.color = x; }),
          remove(i, rerender)),
        labeled("Stroke (blank = none)", input("text", v.stroke, (x) => { v.stroke = x; }, { placeholder: "#ffffff" })),
        labeled("Stroke width", input("number", v.stroke_width, (x) => { v.stroke_width = x === "" ? 0 : +x; }, { step: "any" })),
        labeled("Opacity", input("number", v.opacity, (x) => { v.opacity = x === "" ? 1 : +x; }, { step: "0.05", min: "0", max: "1" })));
    });
  return dynList(state[f.k], "+ entry", () => ["", "#88ccff"],
    (row, i, rerender) => el("div", { className: "row map-row" },
      input("text", row[0], (x) => { row[0] = x; }, { placeholder: "name" }),
      input("color", row[1], (x) => { row[1] = x; }),
      remove(i, rerender)));
}

function viewsNode(f) {
  return el("div", { className: "row wrap" }, ...f.opts.map((view) => {
    const cb = el("input", { type: "checkbox", checked: state.views.includes(view) });
    cb.onchange = () => {
      state.views = cb.checked ? [...state.views, view] : state.views.filter((x) => x !== view);
      changed();
    };
    return el("label", { className: "chk fixed" }, cb, view);
  }));
}

function labelWrap(f, node) {
  const w = el("div", { className: "ctl" });
  if (f.label) { const l = el("label", {}, el("span", { textContent: f.label })); attachTip(l, f.help); w.append(l); }
  w.append(node);
  if (f.when) conds.push({ node: w, when: f.when, local: state });
  return w;
}

const WIDGETS = { grp: groupNode, lights: (f) => labelWrap(f, lightsNode()), palette: (f) => labelWrap(f, paletteNode(f)), map: (f) => labelWrap(f, mapNode(f)), views: (f) => labelWrap(f, viewsNode(f)) };

function fieldText(f) {
  let t = (f.label || "") + " " + f.k;
  if (f.fields) for (const s of f.fields) t += " " + (s.label || "") + " " + s.k;
  return t.toLowerCase();
}

function buildForm() {
  const root = $("form");
  root.replaceChildren();
  conds.length = 0; searchItems.length = 0; searchSections.length = 0;
  for (const k in controlRefs) delete controlRefs[k];
  for (const sec of getSchema()) {
    const body = el("div", { className: "body" });
    const d = el("details", { open: !!sec.open }, el("summary", { textContent: sec.s }), body);
    const items = sec.fields.map((f) => {
      const node = WIDGETS[f.t] ? WIDGETS[f.t](f) : ctl(f, state, state);
      body.append(node);
      return { node, text: fieldText(f) };
    });
    searchItems.push(...items);
    root.append(d);
    if (sec.when) conds.push({ node: d, when: sec.when, local: state });
    searchSections.push({ el: d, open: !!sec.open, items });
  }
}

function filterForm(query) {
  const q = (query || "").trim().toLowerCase();
  for (const it of searchItems) it.node.classList.toggle("search-hidden", !!q && !it.text.includes(q));
  for (const sec of searchSections) {
    const hit = sec.items.some((it) => !it.node.classList.contains("search-hidden"));
    sec.el.classList.toggle("search-hidden", !!q && !hit);
    sec.el.open = q ? hit : sec.open;
  }
}

function refreshVisibility() {
  for (const c of conds) c.node.style.display = c.when(state, c.local, model) ? "" : "none";
}

function rebuildForm() {
  buildForm(); refreshVisibility(); filterForm($("search").value);
}

function attachTip(target, tip) {
  if (!tip) return;
  target.dataset.tip = tip;
  target.classList.add("has-tip");
}

function initTooltips() {
  const tipEl = $("tip");
  const position = (target) => {
    const r = target.getBoundingClientRect(), m = 8;
    const tw = tipEl.offsetWidth, th = tipEl.offsetHeight;
    let y = r.bottom + 6;
    if (y + th > innerHeight - m) y = r.top - th - 6;
    tipEl.style.left = Math.max(m, Math.min(r.left, innerWidth - tw - m)) + "px";
    tipEl.style.top = Math.max(m, y) + "px";
  };
  document.addEventListener("mouseover", (e) => {
    const t = e.target.closest?.("[data-tip]");
    if (!t) return;
    tipEl.textContent = t.dataset.tip; tipEl.classList.add("show"); position(t);
  });
  document.addEventListener("mouseout", (e) => {
    const t = e.target.closest?.("[data-tip]");
    if (t && !t.contains(e.relatedTarget)) tipEl.classList.remove("show");
  });
}

export { controlRefs, buildForm, rebuildForm, refreshVisibility, filterForm, initTooltips };

import { $ } from "./dom.js";
import { state, getSchema, HELP } from "./state.js";
import { buildTypst } from "./config.js";
import { onChange, copyText } from "./render.js";
import { triggerRecompile } from "./models.js";

const controlRefs = {};
const searchItems = [];   // {node, section, text}
const searchSections = []; // {el, open}

const conds = []; // {node, when, local} for visibility refresh

const VEC_AXES = ["X", "Y", "Z", "W"];

function ctl(f, slot, local) {
  const wrap = document.createElement("div");
  wrap.className = "ctl";
  // Fields flagged `recompile: "scad" | "mol"` re-run the source-plugin
  // (maquette-scad or molfig) to regenerate the mesh before the maquette
  // render call — otherwise the change wouldn't be visible in the output.
  const set = (v) => { slot[f.k] = v; if (f.onSet) f.onSet(v); onChange(); if (f.recompile) triggerRecompile(f.recompile); };
  const cur = slot[f.k];
  let labelEl, sync = null;

  if (f.t === "bool") {
    labelEl = document.createElement("label"); labelEl.className = "chk";
    const cb = document.createElement("input"); cb.type = "checkbox"; cb.checked = !!cur;
    cb.onchange = () => { set(cb.checked); };
    // The <label> wraps text + input, so the checkbox is properly labelled
    // for screen readers already — no extra aria-label needed.
    labelEl.append(cb, document.createTextNode(f.label)); wrap.append(labelEl);
    sync = (v) => { cb.checked = !!v; };
  } else {
    labelEl = document.createElement("label");
    const span = document.createElement("span"); span.textContent = f.label; labelEl.append(span);
    let input, valEl;
    if (f.t === "sel") {
      input = document.createElement("select");
      for (const [v, t] of f.opts) { const o = document.createElement("option"); o.value = v; o.textContent = t; input.append(o); }
      input.value = cur;
      input.setAttribute("aria-label", f.label);
      input.onchange = () => set(f.num ? +input.value : input.value);
      sync = (v) => { input.value = v; };
    } else if (f.t === "rng") {
      valEl = document.createElement("span"); valEl.className = "val"; valEl.textContent = (+cur).toFixed(2); labelEl.append(valEl);
      input = document.createElement("input"); input.type = "range"; input.min = f.min; input.max = f.max; input.step = f.step; input.value = cur;
      // A11y: name + spoken value. `aria-valuetext` beats aria-valuenow for
      // fractional values because SRs read the raw number otherwise ("point
      // eight five" instead of the rounded 0.85 shown next to the slider).
      input.setAttribute("aria-label", f.label);
      input.setAttribute("aria-valuetext", (+cur).toFixed(2));
      input.oninput = () => {
        const t = (+input.value).toFixed(2);
        valEl.textContent = t;
        input.setAttribute("aria-valuetext", t);
        set(+input.value);
      };
      sync = (v) => {
        const t = (+v).toFixed(2);
        input.value = v;
        valEl.textContent = t;
        input.setAttribute("aria-valuetext", t);
      };
    } else if (f.t === "num") {
      input = document.createElement("input"); input.type = "number"; input.value = cur; input.step = "any";
      input.setAttribute("aria-label", f.label);
      input.oninput = () => set(input.value === "" ? f.def : +input.value);
      sync = (v) => { input.value = v; };
    } else if (f.t === "col") {
      input = document.createElement("input"); input.type = "color"; input.value = cur || "#000000";
      input.setAttribute("aria-label", f.label);
      input.oninput = () => set(input.value);
      sync = (v) => { input.value = v || "#000000"; };
    } else if (f.t === "txt") {
      input = document.createElement("input"); input.type = "text"; input.value = cur;
      input.setAttribute("aria-label", f.label);
      input.oninput = () => set(input.value);
      sync = (v) => { input.value = v; };
    } else if (f.t === "vec") {
      input = document.createElement("div"); input.className = "row";
      input.setAttribute("role", "group"); input.setAttribute("aria-label", f.label);
      cur.forEach((n, i) => {
        const ni = document.createElement("input"); ni.type = "number"; ni.value = n; ni.step = "any";
        ni.setAttribute("aria-label", `${f.label} ${VEC_AXES[i] || i}`);
        ni.oninput = () => { const a = slot[f.k].slice(); a[i] = +ni.value; set(a); };
        input.append(ni);
      });
      sync = (v) => v.forEach((n, i) => { if (input.children[i]) input.children[i].value = n; });
    }
    wrap.append(labelEl, input);
  }
  attachTip(labelEl, f.help || HELP[f.k]);
  if (slot === state && sync) controlRefs[f.k] = sync;
  if (f.when) conds.push({ node: wrap, when: f.when, local });
  return wrap;
}

function groupNode(f) {
  const box = document.createElement("div"); box.className = "group" + (f.toggle && !state[f.k].__on ? " off" : "");
  const head = document.createElement("label"); head.className = "head";
  if (f.toggle) {
    const cb = document.createElement("input"); cb.type = "checkbox"; cb.checked = state[f.k].__on;
    cb.onchange = () => { state[f.k].__on = cb.checked; box.classList.toggle("off", !cb.checked); onChange(); };
    head.append(cb);
  }
  head.append(document.createTextNode(f.label));
  attachTip(head, f.help || HELP[f.k]);
  const sub = document.createElement("div"); sub.className = "sub";
  for (const s of f.fields) sub.append(ctl(s, state[f.k], state[f.k]));
  box.append(head, sub);
  return box;
}

// Dynamic array editor shared by extra-lights / palette / materials-map. `arr` is
// mutated in place (push/splice); `renderItem(item, i, rerender)` builds one row.
function dynList(arr, addLabel, newItem, renderItem, wrapRow = false) {
  const box = document.createElement("div");
  const rerender = () => {
    box.innerHTML = "";
    const host = wrapRow ? Object.assign(document.createElement("div"), { className: "row" }) : box;
    if (wrapRow) { host.style.flexWrap = "wrap"; box.append(host); }
    arr.forEach((item, i) => host.append(renderItem(item, i, rerender)));
    const add = document.createElement("button"); add.className = "mini-btn"; add.textContent = addLabel;
    add.onclick = () => { arr.push(newItem()); rerender(); onChange(); };
    box.append(add);
  };
  rerender();
  return box;
}

function listNode(f) {   // extra lights
  const mk = (label, el) => { const d = document.createElement("div"); d.className = "ctl"; const lb = document.createElement("label"); lb.innerHTML = `<span>${label}</span>`; d.append(lb, el); return d; };
  return dynList(state.lights, "+ add light",
    () => ({ type: "directional", vector: [1,2,3], color: "#ffffff", intensity: 1, cast_shadow: true, size: 0 }),
    (L, i, rerender) => {
      const it = document.createElement("div"); it.className = "item";
      const sel = document.createElement("select");
      [["directional","Directional"],["positional","Positional"],["area","Area"]].forEach(([v,t]) => { const o = document.createElement("option"); o.value=v; o.textContent=t; sel.append(o); });
      sel.value = L.type; sel.onchange = () => { L.type = sel.value; onChange(); };
      const vec = document.createElement("div"); // per-axis slider + precise number box
      ["X","Y","Z"].forEach((axis, j) => {
        const row = document.createElement("div"); row.style.cssText = "display:flex;gap:6px;align-items:center;margin:2px 0";
        const tag = document.createElement("span"); tag.textContent = axis; tag.style.cssText = "width:1em;color:var(--muted);font-size:12px";
        const rng = document.createElement("input"); rng.type="range"; rng.min="-3"; rng.max="3"; rng.step="0.05"; rng.value=L.vector[j]; rng.style.flex="1";
        const ni = document.createElement("input"); ni.type="number"; ni.step="any"; ni.value=L.vector[j]; ni.style.width="4.5em";
        rng.oninput=()=>{ L.vector[j]=+rng.value; ni.value=rng.value; onChange(); };
        ni.oninput =()=>{ L.vector[j]=+ni.value; if (Math.abs(+ni.value)<=3) rng.value=ni.value; onChange(); };
        row.append(tag, rng, ni); vec.append(row);
      });
      const col = document.createElement("input"); col.type="color"; col.value=L.color; col.oninput=()=>{L.color=col.value;onChange();};
      const inten = document.createElement("input"); inten.type="number"; inten.step="any"; inten.value=L.intensity; inten.oninput=()=>{L.intensity=+inten.value;onChange();};
      const size = document.createElement("input"); size.type="number"; size.step="any"; size.value=L.size; size.oninput=()=>{L.size=+size.value;onChange();};
      const castL = document.createElement("label"); castL.className="chk"; const cast=document.createElement("input"); cast.type="checkbox"; cast.checked=L.cast_shadow; cast.onchange=()=>{L.cast_shadow=cast.checked;onChange();}; castL.append(cast, document.createTextNode("casts shadow"));
      const rm = document.createElement("button"); rm.className="rm"; rm.textContent="✕"; rm.title="remove"; rm.onclick=()=>{ state.lights.splice(i,1); rerender(); onChange(); };
      const top = document.createElement("div"); top.style.cssText="display:flex;gap:6px;align-items:center;margin-bottom:6px"; const sp=document.createElement("span"); sp.style.flex="1"; top.append(sel, sp, rm);
      it.append(top, mk("Vector", vec), mk("Color", col), mk("Intensity", inten), mk("Size (area)", size), castL);
      return it;
    });
}

function paletteNode(f) {
  return dynList(state[f.k], "+ color", () => "#888888",
    (c, i, rerender) => {
      const cell = document.createElement("div"); cell.style.cssText="display:flex;align-items:center;gap:2px;flex:0 0 auto";
      const col = document.createElement("input"); col.type="color"; col.value=c; col.style.width="34px"; col.oninput=()=>{ state[f.k][i]=col.value; onChange(); };
      const rm = document.createElement("button"); rm.className="rm"; rm.textContent="✕"; rm.onclick=()=>{ state[f.k].splice(i,1); rerender(); onChange(); };
      cell.append(col, rm); return cell;
    }, true);
}

function mapNode(f) {
  const rm = (i, rerender) => { const b = document.createElement("button"); b.className="rm"; b.textContent="✕"; b.title="remove"; b.onclick=()=>{ state[f.k].splice(i,1); rerender(); onChange(); }; return b; };
  if (f.rich) return dynList(state[f.k], "+ entry",       // group → {color, stroke, stroke_width, opacity}
    () => ["", { color: "#88ccff", stroke: "", stroke_width: 0, opacity: 1 }],
    (row, i, rerender) => {
      const v = row[1];
      const mk = (label, el) => { const d = document.createElement("div"); d.className="ctl"; const l = document.createElement("label"); l.innerHTML = `<span>${label}</span>`; d.append(l, el); return d; };
      const inp = (type, val, on, extra) => { const e = document.createElement("input"); e.type = type; e.value = val; if (extra) for (const a in extra) e.setAttribute(a, extra[a]); e.oninput = () => on(e.value); return e; };
      const name = inp("text", row[0], x => { row[0] = x; onChange(); }, { placeholder: "group name" });
      const color = inp("color", v.color, x => { v.color = x; onChange(); }); color.style.flex = "0 0 40px";
      const top = document.createElement("div"); top.className = "row"; top.style.marginBottom = "5px"; top.append(name, color, rm(i, rerender));
      const it = document.createElement("div"); it.className = "item";
      it.append(top,
        mk("Stroke (blank = none)", inp("text", v.stroke, x => { v.stroke = x; onChange(); }, { placeholder: "#ffffff" })),
        mk("Stroke width", inp("number", v.stroke_width, x => { v.stroke_width = x === "" ? 0 : +x; onChange(); }, { step: "any" })),
        mk("Opacity", inp("number", v.opacity, x => { v.opacity = x === "" ? 1 : +x; onChange(); }, { step: "0.05", min: "0", max: "1" })));
      return it;
    });
  return dynList(state[f.k], "+ entry", () => ["", "#88ccff"],
    (row, i, rerender) => {
      const r = document.createElement("div"); r.className="row"; r.style.marginBottom="5px";
      const name = document.createElement("input"); name.type="text"; name.placeholder="name"; name.value=row[0]; name.oninput=()=>{ row[0]=name.value; onChange(); };
      const col = document.createElement("input"); col.type="color"; col.value=row[1]; col.style.flex="0 0 40px"; col.oninput=()=>{ row[1]=col.value; onChange(); };
      r.append(name, col, rm(i, rerender)); return r;
    });
}

function viewsNode(f) {
  const box = document.createElement("div"); box.className="row"; box.style.flexWrap="wrap";
  f.opts.forEach(v => {
    const lab = document.createElement("label"); lab.className="chk"; lab.style.flex="0 0 auto";
    const cb = document.createElement("input"); cb.type="checkbox"; cb.checked=state.views.includes(v);
    cb.onchange = () => { if (cb.checked) state.views.push(v); else state.views = state.views.filter(x=>x!==v); onChange(); };
    lab.append(cb, document.createTextNode(v)); box.append(lab);
  });
  return box;
}

function fieldText(f) {
  let t = (f.label || "") + " " + f.k;
  if (f.fields) for (const s of f.fields) t += " " + (s.label || "") + " " + s.k;
  return t.toLowerCase();
}
function buildForm() {
  const root = $("form");
  root.innerHTML = "";
  conds.length = 0; searchItems.length = 0; searchSections.length = 0;
  for (const sec of getSchema()) {
    const d = document.createElement("details"); if (sec.open) d.open = true;
    const sum = document.createElement("summary"); sum.textContent = sec.s; d.append(sum);
    const body = document.createElement("div"); body.className = "body";
    // Per-section list wired straight into the section entry so filterForm
    // can check "does this section have any match?" in O(items-in-section)
    // instead of scanning the full searchItems array per section.
    const secItems = [];
    for (const f of sec.fields) {
      let node;
      if (f.t === "grp") node = groupNode(f);
      else if (f.t === "lights") node = labelWrap(f, listNode(f));
      else if (f.t === "palette") node = labelWrap(f, paletteNode(f));
      else if (f.t === "map") node = labelWrap(f, mapNode(f));
      else if (f.t === "views") node = labelWrap(f, viewsNode(f));
      else node = ctl(f, state, state);
      body.append(node);
      const it = { node, text: fieldText(f) };
      searchItems.push(it); secItems.push(it);
    }
    d.append(body); root.append(d);
    if (sec.when) conds.push({ node: d, when: sec.when, local: state });
    searchSections.push({ el: d, open: !!sec.open, items: secItems });
  }
}

// Filter the form by a query — hides non-matching fields/sections, expands
// sections that contain a match. Composes with when-visibility (CSS !important).
function filterForm(query) {
  const q = (query || "").trim().toLowerCase();
  for (const it of searchItems) it.node.classList.toggle("search-hidden", !!q && !it.text.includes(q));
  for (const sec of searchSections) {
    const hit = sec.items.some(it => !it.node.classList.contains("search-hidden"));
    sec.el.classList.toggle("search-hidden", !!q && !hit);
    sec.el.open = q ? hit : sec.open;
  }
}

// Rebuild the whole form from current state (used by reset & shared-link restore).
function rebuildForm() {
  buildForm(); refreshVisibility(); filterForm($("search").value);
}
function labelWrap(f, node) {
  const w = document.createElement("div"); w.className = "ctl";
  if (f.label) { const l = document.createElement("label"); l.innerHTML = `<span>${f.label}</span>`; attachTip(l, f.help || HELP[f.k]); w.append(l); }
  w.append(node);
  if (f.when) conds.push({ node: w, when: f.when, local: state });
  return w;
}

function refreshVisibility() {
  for (const c of conds) c.node.style.display = c.when(state, c.local) ? "" : "none";
}

// ─────────────────────── hover descriptions (tooltips) ─────────────────────
// Tag an element with its help text; a single delegated listener shows an
// immediate, styled tooltip on hover (native `title` is too slow/subtle).
function attachTip(el, tip) { if (!tip) return; el.dataset.tip = tip; el.classList.add("has-tip"); }
const tipEl = $("tip");
function positionTip(target) {
  const r = target.getBoundingClientRect(), m = 8;
  const tw = tipEl.offsetWidth, th = tipEl.offsetHeight;
  let x = Math.min(r.left, innerWidth - tw - m);
  let y = r.bottom + 6;
  if (y + th > innerHeight - m) y = r.top - th - 6;   // flip above when no room below
  tipEl.style.left = Math.max(m, x) + "px";
  tipEl.style.top = Math.max(m, y) + "px";
}
document.addEventListener("mouseover", (e) => {
  const el = e.target.closest && e.target.closest("[data-tip]");
  if (!el) return;
  tipEl.textContent = el.dataset.tip; tipEl.classList.add("show"); positionTip(el);
});
document.addEventListener("mouseout", (e) => {
  const el = e.target.closest && e.target.closest("[data-tip]");
  if (el && !el.contains(e.relatedTarget)) tipEl.classList.remove("show");
});
$("copy").onclick = async () => { await copyText(buildTypst()); const o = $("copy").textContent; $("copy").textContent = "Copied!"; setTimeout(() => ($("copy").textContent = o), 1200); };


export { controlRefs, conds, buildForm, rebuildForm, refreshVisibility, filterForm, attachTip, ctl, labelWrap };

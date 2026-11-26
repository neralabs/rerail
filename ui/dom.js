/**
 * Element construction and in-place reconciliation. A render builds a node once (`create`)
 * and afterwards only patches what differs (`update`), so a poll never replaces nodes that
 * hold scroll position, focus, or an open disclosure.
 */

const SVG_NS = "http://www.w3.org/2000/svg";

/** `h("button.chip", { "aria-pressed": "true", onclick }, "All")` */
export function h(spec, attrs, ...children) {
  const [tag, ...classes] = spec.split(".");
  const el = document.createElement(tag || "div");
  if (classes.length) el.className = classes.join(" ");
  if (attrs) for (const [k, v] of Object.entries(attrs)) setAttr(el, k, v);
  append(el, children);
  return el;
}

function append(el, children) {
  for (const child of children) {
    if (child === null || child === undefined || child === false) continue;
    if (Array.isArray(child)) append(el, child);
    else el.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
}

/** Set, or remove when `null`/`undefined`/`false`, without touching an unchanged attribute. */
export function setAttr(el, name, value) {
  if (name === "hidden") {
    if (el.hidden !== Boolean(value)) el.hidden = Boolean(value);
    return;
  }
  if (name.startsWith("on") && typeof value === "function") { el[name] = value; return; }
  if (value === null || value === undefined || value === false) {
    if (el.hasAttribute(name)) el.removeAttribute(name);
    return;
  }
  const text = value === true ? "" : String(value);
  if (el.getAttribute(name) !== text) el.setAttribute(name, text);
}

export function setText(el, text) {
  const next = text === null || text === undefined ? "" : String(text);
  if (el.textContent !== next) el.textContent = next;
}

export function setClass(el, name, on) {
  if (el.classList.contains(name) !== Boolean(on)) el.classList.toggle(name, Boolean(on));
}

/** Replace children only when the rendered signature changes; for small static fragments. */
export function setContent(el, signature, build) {
  if (el.dataset.sig === signature) return;
  el.dataset.sig = signature;
  el.replaceChildren(...[build()].flat(Infinity).filter(x => x !== null && x !== undefined && x !== false).map(x => (x instanceof Node ? x : document.createTextNode(String(x)))));
}

/** Small chevron used by carets, folds, and event headers. */
export function chevron(size = 10) {
  const svg = document.createElementNS(SVG_NS, "svg");
  svg.setAttribute("width", size); svg.setAttribute("height", size); svg.setAttribute("viewBox", "0 0 10 10");
  svg.setAttribute("fill", "none"); svg.setAttribute("stroke", "currentColor"); svg.setAttribute("stroke-width", "1.6");
  svg.setAttribute("aria-hidden", "true");
  const path = document.createElementNS(SVG_NS, "path");
  path.setAttribute("d", "M3 1l4 4-4 4");
  svg.append(path);
  return svg;
}

export function searchIcon(size = 14) {
  const svg = document.createElementNS(SVG_NS, "svg");
  svg.setAttribute("width", size); svg.setAttribute("height", size); svg.setAttribute("viewBox", "0 0 16 16");
  svg.setAttribute("fill", "none"); svg.setAttribute("stroke", "currentColor"); svg.setAttribute("stroke-width", "1.6");
  svg.setAttribute("aria-hidden", "true");
  const c = document.createElementNS(SVG_NS, "circle");
  c.setAttribute("cx", "7"); c.setAttribute("cy", "7"); c.setAttribute("r", "4.5");
  const p = document.createElementNS(SVG_NS, "path");
  p.setAttribute("d", "M10.5 10.5 14 14");
  svg.append(c, p);
  return svg;
}

/** Phosphor bold glyphs on a 256 grid: actions, the added check, the Log / Steer / Block outcome marks, the frame's, a repository, the arrow a row opens with, and the Overview banners' moments where nothing is at stake (day one, a new version, stopped), and the question mark of why Rerail suggested a pack. */
const ICONS = {
  plus: "M228,128a12,12,0,0,1-12,12H140v76a12,12,0,0,1-24,0V140H40a12,12,0,0,1,0-24h76V40a12,12,0,0,1,24,0v76h76A12,12,0,0,1,228,128Z",
  check: "M232.49,80.49l-128,128a12,12,0,0,1-17,0l-56-56a12,12,0,1,1,17-17L96,183,215.51,63.51a12,12,0,0,1,17,17Z",
  copy: "M216,28H88A12,12,0,0,0,76,40V76H40A12,12,0,0,0,28,88V216a12,12,0,0,0,12,12H168a12,12,0,0,0,12-12V180h36a12,12,0,0,0,12-12V40A12,12,0,0,0,216,28ZM156,204H52V100H156Zm48-48H180V88a12,12,0,0,0-12-12H100V52H204Z",
  back: "M228,128a12,12,0,0,1-12,12H69l51.52,51.51a12,12,0,0,1-17,17l-72-72a12,12,0,0,1,0-17l72-72a12,12,0,0,1,17,17L69,116H216A12,12,0,0,1,228,128Z",
  done: "M176.49,95.51a12,12,0,0,1,0,17l-56,56a12,12,0,0,1-17,0l-24-24a12,12,0,1,1,17-17L112,143l47.51-47.52A12,12,0,0,1,176.49,95.51ZM236,128A108,108,0,1,1,128,20,108.12,108.12,0,0,1,236,128Zm-24,0a84,84,0,1,0-84,84A84.09,84.09,0,0,0,212,128Z",
  logged: "M128,20A108,108,0,1,0,236,128,108.12,108.12,0,0,0,128,20Zm0,192a84,84,0,1,1,84-84A84.09,84.09,0,0,1,128,212Zm-12-80V80a12,12,0,0,1,24,0v52a12,12,0,0,1-24,0Zm28,40a16,16,0,1,1-16-16A16,16,0,0,1,144,172Z",
  steered: "M240.26,186.1,152.81,34.23h0a28.74,28.74,0,0,0-49.62,0L15.74,186.1a27.45,27.45,0,0,0,0,27.71A28.31,28.31,0,0,0,40.55,228h174.9a28.31,28.31,0,0,0,24.79-14.19A27.45,27.45,0,0,0,240.26,186.1Zm-20.8,15.7a4.46,4.46,0,0,1-4,2.2H40.55a4.46,4.46,0,0,1-4-2.2,3.56,3.56,0,0,1,0-3.73L124,46.2a4.77,4.77,0,0,1,8,0l87.44,151.87A3.56,3.56,0,0,1,219.46,201.8ZM116,136V104a12,12,0,0,1,24,0v32a12,12,0,0,1-24,0Zm28,40a16,16,0,1,1-16-16A16,16,0,0,1,144,176Z",
  blocked: "M116,132V80a12,12,0,0,1,24,0v52a12,12,0,0,1-24,0ZM236,91.55v72.9a19.86,19.86,0,0,1-5.86,14.14l-51.55,51.55A19.85,19.85,0,0,1,164.45,236H91.55a19.85,19.85,0,0,1-14.14-5.86L25.86,178.59A19.86,19.86,0,0,1,20,164.45V91.55a19.86,19.86,0,0,1,5.86-14.14L77.41,25.86A19.85,19.85,0,0,1,91.55,20h72.9a19.85,19.85,0,0,1,14.14,5.86l51.55,51.55A19.86,19.86,0,0,1,236,91.55Zm-24,1.66L162.79,44H93.21L44,93.21v69.58L93.21,212h69.58L212,162.79ZM128,156a16,16,0,1,0,16,16A16,16,0,0,0,128,156Z",
  repo: "M236,64a36,36,0,1,0-48,33.94V112a4,4,0,0,1-4,4H96a27.8,27.8,0,0,0-4,.29V97.94a36,36,0,1,0-24,0v60.12a36,36,0,1,0,24,0V144a4,4,0,0,1,4-4h88a28,28,0,0,0,28-28V97.94A36.07,36.07,0,0,0,236,64ZM80,52A12,12,0,1,1,68,64,12,12,0,0,1,80,52Zm0,152a12,12,0,1,1,12-12A12,12,0,0,1,80,204ZM200,76a12,12,0,1,1,12-12A12,12,0,0,1,200,76Z",
  forward: "M224.49,136.49l-72,72a12,12,0,0,1-17-17L187,140H40a12,12,0,0,1,0-24H187L135.51,64.48a12,12,0,0,1,17-17l72,72A12,12,0,0,1,224.49,136.49Z",
  caret: "M216.49,104.49l-80,80a12,12,0,0,1-17,0l-80-80a12,12,0,0,1,17-17L128,159l71.51-71.52a12,12,0,0,1,17,17Z",
  overview: "M218.29,182.17a12,12,0,0,1-16.47,4.12L140,149.19V216a12,12,0,0,1-24,0V149.19l-61.82,37.1a12,12,0,1,1-12.35-20.58L104.68,128,41.83,90.29A12,12,0,1,1,54.18,69.71L116,106.81V40a12,12,0,0,1,24,0v66.81l61.82-37.1a12,12,0,1,1,12.35,20.58L151.32,128l62.85,37.71A12,12,0,0,1,218.29,182.17Z",
  rule: "M228,128a12,12,0,0,1-12,12H128a12,12,0,0,1,0-24h88A12,12,0,0,1,228,128ZM128,76h88a12,12,0,0,0,0-24H128a12,12,0,0,0,0,24Zm88,104H128a12,12,0,0,0,0,24h88a12,12,0,0,0,0-24ZM79.51,39.51,56,63l-7.51-7.52a12,12,0,0,0-17,17l16,16a12,12,0,0,0,17,0l32-32a12,12,0,0,0-17-17Zm0,64L56,127l-7.51-7.52a12,12,0,1,0-17,17l16,16a12,12,0,0,0,17,0l32-32a12,12,0,0,0-17-17Zm0,64L56,191l-7.51-7.52a12,12,0,1,0-17,17l16,16a12,12,0,0,0,17,0l32-32a12,12,0,0,0-17-17Z",
  session: "M236.34,187.09A84,84,0,0,0,172.29,68.9,84,84,0,0,0,19.66,139.09l-6.84,23.26a20,20,0,0,0,24.83,24.83l23.26-6.84a83.94,83.94,0,0,0,22.76,6.74,84.06,84.06,0,0,0,111.42,41.26l23.26,6.84a20,20,0,0,0,24.83-24.83ZM62,155.5a11.88,11.88,0,0,0-3.39.49l-20.72,6.09L44,141.35a12,12,0,0,0-.93-9A60,60,0,1,1,67.7,156.92,12,12,0,0,0,62,155.5Zm150.89,24.8a12,12,0,0,0-.93,9l6.09,20.73L197.36,204a12,12,0,0,0-9.06.93A60,60,0,0,1,111,186.63a83.93,83.93,0,0,0,68.55-91.37,60,60,0,0,1,33.38,85Z",
  pencil: "M230.15,70.54,185.46,25.86a20,20,0,0,0-28.28,0L33.86,149.17A19.86,19.86,0,0,0,28,163.31V208a20,20,0,0,0,20,20H216a12,12,0,0,0,0-24H125L230.15,98.83A20,20,0,0,0,230.15,70.54ZM91,204H52V165l84-84,39,39ZM192,103,153,64l18.34-18.34,39,39Z",
  gear: "M128,76a52,52,0,1,0,52,52A52.06,52.06,0,0,0,128,76Zm0,80a28,28,0,1,1,28-28A28,28,0,0,1,128,156Zm113.86-49.57A12,12,0,0,0,236,98.34L208.21,82.49l-.11-31.31a12,12,0,0,0-4.25-9.12,116,116,0,0,0-38-21.41,12,12,0,0,0-9.68.89L128,37.27,99.83,21.53a12,12,0,0,0-9.7-.9,116.06,116.06,0,0,0-38,21.47,12,12,0,0,0-4.24,9.1l-.14,31.34L20,98.35a12,12,0,0,0-5.85,8.11,110.7,110.7,0,0,0,0,43.11A12,12,0,0,0,20,157.66l27.82,15.85.11,31.31a12,12,0,0,0,4.25,9.12,116,116,0,0,0,38,21.41,12,12,0,0,0,9.68-.89L128,218.73l28.14,15.74a12,12,0,0,0,9.7.9,116.06,116.06,0,0,0,38-21.47,12,12,0,0,0,4.24-9.1l.14-31.34,27.81-15.84a12,12,0,0,0,5.85-8.11A110.7,110.7,0,0,0,241.86,106.43Zm-22.63,33.18-26.88,15.28a11.94,11.94,0,0,0-4.55,4.59c-.54,1-1.11,1.93-1.7,2.88a12,12,0,0,0-1.83,6.31L184.13,199a91.83,91.83,0,0,1-21.07,11.87l-27.15-15.19a12,12,0,0,0-5.86-1.53h-.29c-1.14,0-2.3,0-3.44,0a12.08,12.08,0,0,0-6.14,1.51L93,210.82A92.27,92.27,0,0,1,71.88,199l-.11-30.24a12,12,0,0,0-1.83-6.32c-.58-.94-1.16-1.91-1.7-2.88A11.92,11.92,0,0,0,63.7,155L36.8,139.63a86.53,86.53,0,0,1,0-23.24l26.88-15.28a12,12,0,0,0,4.55-4.58c.54-1,1.11-1.94,1.7-2.89a12,12,0,0,0,1.83-6.31L71.87,57A91.83,91.83,0,0,1,92.94,45.17l27.15,15.19a11.92,11.92,0,0,0,6.15,1.52c1.14,0,2.3,0,3.44,0a12.08,12.08,0,0,0,6.14-1.51L163,45.18A92.27,92.27,0,0,1,184.12,57l.11,30.24a12,12,0,0,0,1.83,6.32c.58.94,1.16,1.91,1.7,2.88A11.92,11.92,0,0,0,192.3,101l26.9,15.33A86.53,86.53,0,0,1,219.23,139.61Z",
  wave: "M184.63,23.14a12,12,0,0,1,15-8A72.34,72.34,0,0,1,228.4,32a12,12,0,0,1-16.58,17.35,48.31,48.31,0,0,0-19.2-11.2A12,12,0,0,1,184.63,23.14ZM232.85,167.8A92,92,0,0,1,64.31,190l-40-69.32A32,32,0,0,1,36,77a31.72,31.72,0,0,1,7.54-3.14A32,32,0,0,1,88.65,32.45a32,32,0,0,1,53,3.55l14.53,25.17A32,32,0,0,1,206.35,68l17.31,30A91.38,91.38,0,0,1,232.85,167.8Zm-30-57.81L185.57,80a8,8,0,0,0-13.86,8h0l6.68,11.57a12,12,0,0,1-3.18,15.59,28,28,0,0,0-6.41,38.1,12,12,0,1,1-19.86,13.46,52.05,52.05,0,0,1,3.9-63.4L120.9,48A8,8,0,0,0,107,56l24.25,42a12,12,0,1,1-20.78,12L79.33,56a8,8,0,0,0-13.85,8L99,122a12,12,0,0,1-20.78,12L58.93,100.67a8,8,0,1,0-13.85,8L85.1,178a68,68,0,0,0,117.78-68ZM50.46,198a12,12,0,1,0-20.78,12,132.79,132.79,0,0,0,20,26.38,12,12,0,1,0,17.17-16.76A109.2,109.2,0,0,1,50.46,198Z",
  rocket: "M227.85,46.89a20,20,0,0,0-18.74-18.74c-13.13-.77-46.65.42-74.48,28.24L131,60H74.36a19.83,19.83,0,0,0-14.14,5.86L25.87,100.19a20,20,0,0,0,11.35,33.95l37.14,5.18,42.32,42.32,5.19,37.18A19.88,19.88,0,0,0,135.34,235a20.13,20.13,0,0,0,6.37,1,19.9,19.9,0,0,0,14.1-5.87l34.34-34.35A19.85,19.85,0,0,0,196,181.64V125l3.6-3.59C227.43,93.54,228.62,60,227.85,46.89ZM76,84h31L75.75,115.28l-27.23-3.8ZM151.6,73.37A72.27,72.27,0,0,1,204,52a72.17,72.17,0,0,1-21.38,52.41L128,159,97,128ZM172,180l-27.49,27.49-3.8-27.23L172,149Zm-72,22c-8.71,11.85-26.19,26-60,26a12,12,0,0,1-12-12c0-33.84,14.12-51.32,26-60A12,12,0,1,1,68.18,175.3C62.3,179.63,55.51,187.8,53,203c15.21-2.51,23.37-9.3,27.7-15.18A12,12,0,1,1,100,202Z",
  melting: "M180,136a16,16,0,1,1-16-16A16,16,0,0,1,180,136ZM136,92a16,16,0,1,0-16,16A16,16,0,0,0,136,92Zm67.83-40.9A108,108,0,0,0,47.5,200a12,12,0,0,0,17.89-16,84,84,0,1,1,125.22,0,12,12,0,0,0,17.89,16,108,108,0,0,0-4.67-148.9ZM140,164h-4c-19.81,0-44-16.61-44-36a36.47,36.47,0,0,1,.5-6,12,12,0,0,0-23.67-4A60.67,60.67,0,0,0,68,128c0,16.1,8,31.46,22.49,43.25C103.58,181.9,120.17,188,136,188h4a8,8,0,0,1,0,16H100a32,32,0,0,0-32,32,12,12,0,0,0,24,0,8,8,0,0,1,8-8h40a32,32,0,0,0,0-64Z",
  question: "M144,180a16,16,0,1,1-16-16A16,16,0,0,1,144,180Zm92-52A108,108,0,1,1,128,20,108.12,108.12,0,0,1,236,128Zm-24,0a84,84,0,1,0-84,84A84.09,84.09,0,0,0,212,128ZM128,64c-24.26,0-44,17.94-44,40v4a12,12,0,0,0,24,0v-4c0-8.82,9-16,20-16s20,7.18,20,16-9,16-20,16a12,12,0,0,0-12,12v8a12,12,0,0,0,23.73,2.56C158.31,137.88,172,122.37,172,104,172,81.94,152.26,64,128,64Z",
};

/**
 * The mark of something that waits for a person's approval: a check in a dashed circle, "not yet approved". A stroke
 * drawing, unlike the filled glyphs above, at their 256 grid and weight; twelve short dashes, their round ends inside
 * the gaps (12 × 50.27 is the circle).
 */
export function pendingIcon(size = 22) {
  const svg = document.createElementNS(SVG_NS, "svg");
  for (const [k, v] of [["class", "ico pending"], ["width", size], ["height", size], ["viewBox", "0 0 256 256"], ["fill", "none"], ["stroke", "currentColor"],
    ["stroke-width", "24"], ["stroke-linecap", "round"], ["stroke-linejoin", "round"], ["aria-hidden", "true"]]) svg.setAttribute(k, v);
  const circle = document.createElementNS(SVG_NS, "circle");
  for (const [k, v] of [["cx", 128], ["cy", 128], ["r", 96], ["stroke-dasharray", "6 44.27"], ["transform", "rotate(-90 128 128)"]]) circle.setAttribute(k, v);
  const check = document.createElementNS(SVG_NS, "path");
  check.setAttribute("d", "M88 132l28 28 56-56");
  svg.append(circle, check);
  return svg;
}

export function icon(name, size = 12) {
  const svg = document.createElementNS(SVG_NS, "svg");
  svg.setAttribute("class", "ico");
  svg.setAttribute("width", size); svg.setAttribute("height", size); svg.setAttribute("viewBox", "0 0 256 256");
  svg.setAttribute("fill", "currentColor"); svg.setAttribute("aria-hidden", "true");
  const path = document.createElementNS(SVG_NS, "path");
  path.setAttribute("d", ICONS[name]);
  svg.append(path);
  return svg;
}

const KEYED = new WeakMap();

/**
 * Reconcile `parent`'s children against `items` in order. `key(item)` identifies a node across
 * renders; `create(item)` builds a fresh node; `update(node, item, index)` patches an existing
 * one (also called right after create). Nodes whose key disappeared are removed before any
 * comparison, so a removal never dislodges the survivors after it; surviving nodes are moved
 * only when their relative order changed.
 */
export function syncChildren(parent, items, key, create, update) {
  let map = KEYED.get(parent);
  if (map === undefined) { map = new Map(); KEYED.set(parent, map); }
  const keys = items.map(key);
  const want = new Set(keys);
  let cursor = parent.firstChild;
  items.forEach((item, index) => {
    const k = keys[index];
    let node = map.get(k);
    if (node === undefined || node.parentNode !== parent) {
      node = create(item);
      node.dataset.key = k;
      map.set(k, node);
    }
    while (cursor !== null && !want.has(cursor.dataset?.key)) {
      const next = cursor.nextSibling;
      cursor.remove();
      cursor = next;
    }
    if (node !== cursor) {
      if (cursor === null) parent.append(node); else cursor.before(node);
    }
    update(node, item, index);
    cursor = node.nextSibling;
  });
  while (cursor !== null) {
    const next = cursor.nextSibling;
    cursor.remove();
    cursor = next;
  }
  for (const k of map.keys()) if (!want.has(k)) map.delete(k);
}

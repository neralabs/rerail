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

export function setStyle(el, prop, value) {
  if (el.style.getPropertyValue(prop) !== value) el.style.setProperty(prop, value);
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

/** Phosphor bold glyphs on a 256 grid: actions, the added check, and the Log / Steer / Block outcome marks. */
const ICONS = {
  plus: "M228,128a12,12,0,0,1-12,12H140v76a12,12,0,0,1-24,0V140H40a12,12,0,0,1,0-24h76V40a12,12,0,0,1,24,0v76h76A12,12,0,0,1,228,128Z",
  check: "M232.49,80.49l-128,128a12,12,0,0,1-17,0l-56-56a12,12,0,1,1,17-17L96,183,215.51,63.51a12,12,0,0,1,17,17Z",
  copy: "M216,28H88A12,12,0,0,0,76,40V76H40A12,12,0,0,0,28,88V216a12,12,0,0,0,12,12H168a12,12,0,0,0,12-12V180h36a12,12,0,0,0,12-12V40A12,12,0,0,0,216,28ZM156,204H52V100H156Zm48-48H180V88a12,12,0,0,0-12-12H100V52H204Z",
  back: "M228,128a12,12,0,0,1-12,12H69l51.52,51.51a12,12,0,0,1-17,17l-72-72a12,12,0,0,1,0-17l72-72a12,12,0,0,1,17,17L69,116H216A12,12,0,0,1,228,128Z",
  done: "M176.49,95.51a12,12,0,0,1,0,17l-56,56a12,12,0,0,1-17,0l-24-24a12,12,0,1,1,17-17L112,143l47.51-47.52A12,12,0,0,1,176.49,95.51ZM236,128A108,108,0,1,1,128,20,108.12,108.12,0,0,1,236,128Zm-24,0a84,84,0,1,0-84,84A84.09,84.09,0,0,0,212,128Z",
  logged: "M128,20A108,108,0,1,0,236,128,108.12,108.12,0,0,0,128,20Zm0,192a84,84,0,1,1,84-84A84.09,84.09,0,0,1,128,212Zm-12-80V80a12,12,0,0,1,24,0v52a12,12,0,0,1-24,0Zm28,40a16,16,0,1,1-16-16A16,16,0,0,1,144,172Z",
  steered: "M240.26,186.1,152.81,34.23h0a28.74,28.74,0,0,0-49.62,0L15.74,186.1a27.45,27.45,0,0,0,0,27.71A28.31,28.31,0,0,0,40.55,228h174.9a28.31,28.31,0,0,0,24.79-14.19A27.45,27.45,0,0,0,240.26,186.1Zm-20.8,15.7a4.46,4.46,0,0,1-4,2.2H40.55a4.46,4.46,0,0,1-4-2.2,3.56,3.56,0,0,1,0-3.73L124,46.2a4.77,4.77,0,0,1,8,0l87.44,151.87A3.56,3.56,0,0,1,219.46,201.8ZM116,136V104a12,12,0,0,1,24,0v32a12,12,0,0,1-24,0Zm28,40a16,16,0,1,1-16-16A16,16,0,0,1,144,176Z",
  blocked: "M116,132V80a12,12,0,0,1,24,0v52a12,12,0,0,1-24,0ZM236,91.55v72.9a19.86,19.86,0,0,1-5.86,14.14l-51.55,51.55A19.85,19.85,0,0,1,164.45,236H91.55a19.85,19.85,0,0,1-14.14-5.86L25.86,178.59A19.86,19.86,0,0,1,20,164.45V91.55a19.86,19.86,0,0,1,5.86-14.14L77.41,25.86A19.85,19.85,0,0,1,91.55,20h72.9a19.85,19.85,0,0,1,14.14,5.86l51.55,51.55A19.86,19.86,0,0,1,236,91.55Zm-24,1.66L162.79,44H93.21L44,93.21v69.58L93.21,212h69.58L212,162.79ZM128,156a16,16,0,1,0,16,16A16,16,0,0,0,128,156Z",
};

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

/** Drop the keyed map so the next sync rebuilds from scratch (used when a container is emptied). */
export function clearKeyed(parent) {
  KEYED.delete(parent);
  parent.replaceChildren();
}

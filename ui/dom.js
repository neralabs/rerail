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
  el.replaceChildren(...[build()].flat().filter(Boolean));
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

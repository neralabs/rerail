/**
 * The little of the DOM the dashboard's `dom.js`, `shell.js` toast and a view's render use, for rendering a view in a
 * unit test without a browser: elements and text nodes, attributes, `dataset`, `classList`, `hidden`, child moves,
 * `textContent`, and `querySelectorAll` / `closest` / `matches` for simple selectors (a tag, classes and
 * `[attr]` / `[attr="value"]`, joined by spaces for descendants). Nothing parses markup: there is no `innerHTML`,
 * so a test can tell text that stayed text from markup a view built.
 */

export class FakeNode {
  parentNode: FakeElement | null = null;

  get nextSibling(): FakeNode | null {
    const siblings = this.parentNode?.childNodes;
    if (siblings === undefined) return null;
    return siblings[siblings.indexOf(this) + 1] ?? null;
  }

  get textContent(): string { return ""; }

  remove(): void {
    const parent = this.parentNode;
    if (parent === null) return;
    parent.childNodes.splice(parent.childNodes.indexOf(this), 1);
    this.parentNode = null;
  }

  before(...nodes: Array<FakeNode | string>): void {
    const parent = this.parentNode;
    if (parent === null) return;
    for (const node of nodes.map(toNode)) {
      node.remove();
      parent.childNodes.splice(parent.childNodes.indexOf(this), 0, node);
      node.parentNode = parent;
    }
  }
}

export class FakeText extends FakeNode {
  constructor(public data: string) { super(); }
  override get textContent(): string { return this.data; }
  override set textContent(value: string) { this.data = value; }
}

const toNode = (node: FakeNode | string): FakeNode => (node instanceof FakeNode ? node : new FakeText(String(node)));
const dataName = (prop: string) => `data-${prop.replace(/[A-Z]/gu, c => `-${c.toLowerCase()}`)}`;

interface Compound { tag: string | null; classes: string[]; attrs: Array<[string, string | null]> }

function parseCompound(text: string): Compound {
  const compound: Compound = { tag: null, classes: [], attrs: [] };
  const pattern = /^([a-z0-9-]+)|\.([\w-]+)|\[([\w-]+)(?:="([^"]*)")?\]/giu;
  for (const [, tag, cls, attr, value] of text.matchAll(pattern)) {
    if (tag !== undefined) compound.tag = tag.toLowerCase();
    else if (cls !== undefined) compound.classes.push(cls);
    else if (attr !== undefined) compound.attrs.push([attr, value ?? null]);
  }
  return compound;
}

function matchesCompound(el: FakeElement, compound: Compound): boolean {
  if (compound.tag !== null && el.tagName.toLowerCase() !== compound.tag) return false;
  if (!compound.classes.every(cls => el.classList.contains(cls))) return false;
  return compound.attrs.every(([name, value]) => (value === null ? el.hasAttribute(name) : el.getAttribute(name) === value));
}

export class FakeElement extends FakeNode {
  readonly tagName: string;
  childNodes: FakeNode[] = [];
  readonly attributes = new Map<string, string>();
  readonly dataset: Record<string, string | undefined>;
  readonly classList = {
    contains: (name: string) => this.classNames().includes(name),
    add: (...names: string[]) => { this.className = [...new Set([...this.classNames(), ...names])].join(" "); },
    remove: (...names: string[]) => { this.className = this.classNames().filter(n => !names.includes(n)).join(" "); },
    toggle: (name: string, force?: boolean) => {
      const on = force ?? !this.classList.contains(name);
      if (on) this.classList.add(name); else this.classList.remove(name);
      return on;
    },
  };

  constructor(tag: string) {
    super();
    this.tagName = tag.toUpperCase();
    this.dataset = new Proxy({}, {
      get: (_target, prop) => (typeof prop === "string" ? this.getAttribute(dataName(prop)) ?? undefined : undefined),
      set: (_target, prop, value) => { if (typeof prop === "string") this.setAttribute(dataName(prop), String(value)); return true; },
      deleteProperty: (_target, prop) => { if (typeof prop === "string") this.removeAttribute(dataName(prop)); return true; },
    });
  }

  private classNames(): string[] { return (this.getAttribute("class") ?? "").split(/\s+/u).filter(Boolean); }
  get className(): string { return this.getAttribute("class") ?? ""; }
  set className(value: string) { this.setAttribute("class", value); }
  get hidden(): boolean { return this.hasAttribute("hidden"); }
  set hidden(value: boolean) { if (value) this.setAttribute("hidden", ""); else this.removeAttribute("hidden"); }
  get disabled(): boolean { return this.hasAttribute("disabled"); }
  get id(): string { return this.getAttribute("id") ?? ""; }

  getAttribute(name: string): string | null { return this.attributes.get(name) ?? null; }
  setAttribute(name: string, value: unknown): void { this.attributes.set(name, String(value)); }
  hasAttribute(name: string): boolean { return this.attributes.has(name); }
  removeAttribute(name: string): void { this.attributes.delete(name); }

  get firstChild(): FakeNode | null { return this.childNodes[0] ?? null; }
  get children(): FakeElement[] { return this.childNodes.filter((n): n is FakeElement => n instanceof FakeElement); }

  append(...nodes: Array<FakeNode | string>): void {
    for (const node of nodes.map(toNode)) {
      node.remove();
      this.childNodes.push(node);
      node.parentNode = this;
    }
  }

  replaceChildren(...nodes: Array<FakeNode | string>): void {
    for (const child of [...this.childNodes]) child.remove();
    this.append(...nodes);
  }

  override get textContent(): string { return this.childNodes.map(n => n.textContent).join(""); }
  override set textContent(value: string) { this.replaceChildren(new FakeText(value)); }

  /** Every element under this one, depth first, in document order. */
  descendants(): FakeElement[] {
    return this.children.flatMap(child => [child, ...child.descendants()]);
  }

  matches(selector: string): boolean {
    const parts = selector.trim().split(/\s+/u).map(parseCompound);
    if (!matchesCompound(this, parts.at(-1)!)) return false;
    let at: FakeElement | null = this.parentNode;
    for (let i = parts.length - 2; i >= 0; i -= 1) {
      while (at !== null && !matchesCompound(at, parts[i]!)) at = at.parentNode;
      if (at === null) return false;
      at = at.parentNode;
    }
    return true;
  }

  querySelectorAll(selector: string): FakeElement[] {
    return this.descendants().filter(el => el.matches(selector));
  }

  querySelector(selector: string): FakeElement | null {
    return this.querySelectorAll(selector)[0] ?? null;
  }

  closest(selector: string): FakeElement | null {
    let at: FakeElement | null = this;
    while (at !== null && !at.matches(selector)) at = at.parentNode;
    return at;
  }

  focus(): void {}
  select(): void {}
}

/** The document `fakeDocument` builds: the calls the dashboard makes on `document`. */
export interface FakeDocument {
  readonly body: FakeElement;
  readonly hidden: boolean;
  createElement(tag: string): FakeElement;
  createElementNS(ns: string, tag: string): FakeElement;
  createTextNode(text: string): FakeText;
  getElementById(id: string): FakeElement | null;
  execCommand(command: string): boolean;
  addEventListener(): void;
}

/** A document whose body holds `#toast`, as `index.html` does; stub it as `document` with `Node` as `FakeNode`. */
export function fakeDocument(): FakeDocument {
  const body = new FakeElement("body");
  const toast = new FakeElement("output");
  toast.setAttribute("id", "toast");
  body.append(toast);
  return {
    body,
    hidden: false,
    createElement: tag => new FakeElement(tag),
    createElementNS: (_ns, tag) => new FakeElement(tag),
    createTextNode: text => new FakeText(text),
    getElementById: id => body.descendants().find(el => el.id === id) ?? null,
    execCommand: () => false,
    addEventListener: () => {},
  };
}

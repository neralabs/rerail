import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderStatus } from "../../../ui/frame/topbar.js";
import { LIVE_MS } from "../../../ui/poll.js";
import { data, setScope } from "../../../ui/state.js";

/**
 * A page element as the status pill's drawing uses it: attributes, text, the classes a ".name" selector matches, the
 * children (the version's slot is prepended and redrawn with `replaceChildren`) and `dataset`, where the slot keeps
 * the key of what it last drew. It stands in for the page's `Node` too, so the drawing's `instanceof Node` holds.
 */
class FakeNode {
  hidden = false;
  className = "";
  readonly dataset: Record<string, string> = {};
  readonly children: FakeNode[] = [];
  #parent: FakeNode | null = null;
  #text = "";
  readonly #attributes = new Map<string, string>();

  get textContent(): string {
    return this.children.length > 0 ? this.children.map(child => child.textContent).join("") : this.#text;
  }

  set textContent(text: string) {
    this.replaceChildren();
    this.#text = text;
  }

  hasAttribute(name: string): boolean { return this.#attributes.has(name); }
  getAttribute(name: string): string | null { return this.#attributes.get(name) ?? null; }
  setAttribute(name: string, value: string): void { this.#attributes.set(name, value); }
  removeAttribute(name: string): void { this.#attributes.delete(name); }

  /** The first element under this one with the class a ".name" selector names; the pill's drawing uses no other kind. */
  querySelector(selector: string): FakeNode | null {
    if (!/^\.[\w-]+$/.test(selector)) throw new Error(`the stand-in page answers only ".name" selectors, not ${selector}`);
    for (const child of this.children) {
      if (child.className.split(" ").includes(selector.slice(1))) return child;
      const found = child.querySelector(selector);
      if (found !== null) return found;
    }
    return null;
  }

  prepend(...nodes: FakeNode[]): void { this.children.unshift(...this.#adopt(nodes)); }
  append(...nodes: FakeNode[]): void { this.children.push(...this.#adopt(nodes)); }

  replaceChildren(...nodes: FakeNode[]): void {
    for (const child of [...this.children]) child.remove();
    this.append(...nodes);
  }

  remove(): void {
    const siblings = this.#parent?.children;
    siblings?.splice(siblings.indexOf(this), 1);
    this.#parent = null;
  }

  #adopt(nodes: FakeNode[]): FakeNode[] {
    for (const node of nodes) {
      node.remove();
      node.#parent = this;
    }
    return nodes;
  }
}

/** An element with the classes of `spec` ("span.dot") and the given children. */
function element(spec: string, ...children: FakeNode[]): FakeNode {
  const node = new FakeNode();
  node.className = spec.split(".").slice(1).join(" ");
  node.append(...children);
  return node;
}

function textNode(text: string): FakeNode {
  const node = new FakeNode();
  node.textContent = text;
  return node;
}

let pill: FakeNode;
let words: FakeNode;
const NOW = Date.UTC(2026, 10, 18, 9, 0, 0);
const MINUTE = 60_000;

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date"], now: NOW });
  // As the page holds it: the pill (#status) with its dot and the words beside the dot (#statusText).
  words = element("span");
  pill = element("div.status", element("span.dot"), words);
  const byId: Record<string, FakeNode> = { status: pill, statusText: words };
  vi.stubGlobal("Node", FakeNode);
  vi.stubGlobal("document", {
    getElementById: (id: string) => byId[id] ?? null,
    createElement: () => new FakeNode(),
    createTextNode: textNode,
  });
  vi.stubGlobal("location", { origin: "http://127.0.0.1:4000" });
  setScope({ repository: "A" });
  Object.assign(data, {
    loaded: true, disconnected: false, accessRequired: false, missing: false, health: { version: "1" },
    repository: { setUp: true, state: "running" }, updatedAt: NOW, heardAt: NOW, behindSince: 0, readingSince: 0,
  });
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("the status pill", () => {
  it("says Live while the change stream's messages come, though the last read is older than two missed polls", () => {
    data.updatedAt = NOW - 5 * MINUTE;

    renderStatus();

    expect(words.textContent).toBe("Live");
  });

  it("turns from Live to how old the data is two missed beats after the last message, with nothing else drawing it", async () => {
    renderStatus();
    const version = pill.querySelector(".ver");
    expect([words.textContent, version?.textContent]).toEqual(["Live", "v1"]);

    await vi.advanceTimersByTimeAsync(LIVE_MS + 1);

    expect(words.textContent).toBe("Updated just now");
    // The timer's redraw keeps the version's button, not a new one, so a keyboard focus on it is not lost.
    expect(pill.querySelector(".ver")).toBe(version);
  });

  it("stops saying Live once a read has hung for two beats, though the daemon still beats", () => {
    data.behindSince = NOW - LIVE_MS - 1_000;

    renderStatus();

    expect(words.textContent).toBe("Updated just now");
  });

  it("stops saying Live once any read the page started, a view or range the person asked for among them, hangs for two beats", () => {
    data.readingSince = NOW - LIVE_MS - 1_000;

    renderStatus();

    expect(words.textContent).toBe("Updated just now");
  });

  it("goes by the earlier of a change it has not read and a read still out", () => {
    Object.assign(data, { behindSince: NOW - 2_000, readingSince: NOW - LIVE_MS - 1_000 });

    renderStatus();

    expect(words.textContent).toBe("Updated just now");
  });

  it("says how old the data shown is while reads fail and the daemon still beats, and that it retries", () => {
    Object.assign(data, { disconnected: true, updatedAt: NOW - 3 * MINUTE });

    renderStatus();

    expect([words.textContent, pill.getAttribute("title")]).toEqual(["Reconnecting…", "The last refresh failed. Showing data from 3 min ago; retrying."]);
  });
});

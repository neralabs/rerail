import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderStatus } from "../../../ui/frame/topbar.js";
import { LIVE_MS } from "../../../ui/poll.js";
import { data, setScope } from "../../../ui/state.js";

/** The status pill's elements as the page holds them: its class and title, its dot, and the words beside the dot. */
interface FakeElement {
  textContent: string;
  hidden: boolean;
  hasAttribute(name: string): boolean;
  getAttribute(name: string): string | null;
  setAttribute(name: string, value: string): void;
  removeAttribute(name: string): void;
  querySelector(selector: string): FakeElement;
}

function fakeElement(dot?: FakeElement): FakeElement {
  const attributes = new Map<string, string>();
  return {
    textContent: "",
    hidden: false,
    hasAttribute: name => attributes.has(name),
    getAttribute: name => attributes.get(name) ?? null,
    setAttribute: (name, value) => { attributes.set(name, value); },
    removeAttribute: name => { attributes.delete(name); },
    querySelector: () => dot!,
  };
}

let pill: FakeElement;
let words: FakeElement;
const NOW = Date.UTC(2026, 10, 18, 9, 0, 0);
const MINUTE = 60_000;

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date"], now: NOW });
  pill = fakeElement(fakeElement());
  words = fakeElement();
  vi.stubGlobal("document", { getElementById: (id: string) => (id === "status" ? pill : words) });
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
    expect(words.textContent).toBe("Live");

    await vi.advanceTimersByTimeAsync(LIVE_MS + 1);

    expect(words.textContent).toBe("Updated just now");
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

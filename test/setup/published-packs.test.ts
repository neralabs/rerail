import { mkdtempSync, realpathSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { RulePack } from "../../src/engine/index.js";
import {
  PUBLISHED_PACKS_MAX_AGE_MS,
  PUBLISHED_RETRY_MS,
  publishedPackReader,
  publishedPacksOf,
  type PublishedPackReaderOptions,
  type PublishedPacksMessage,
} from "../../src/setup/published-packs.js";
import { discoverPacks, type PackStore } from "../../src/setup/rule-pack-store.js";
import { installPackDirectory } from "../support/installed-packs.js";
import { repositoryPath } from "../support/oracles.js";
import { removeFolder } from "../support/platform/remove-folder.js";

const listed = (id: string, version = "0.1.0") => ({ pack: { id } as RulePack, packageName: `@neralabs/rerail-pack-${id}`, version, ruleCount: 3 });

/** npm as the reader asks it: each read takes the next answer, given when the test settles it. */
function npm() {
  const pending: Array<{ settle: (answer: PublishedPacksMessage | Error) => void }> = [];
  return {
    pending,
    discover: async (): Promise<PublishedPacksMessage> => await new Promise((resolve, reject) => {
      pending.push({ settle: answer => (answer instanceof Error ? reject(answer) : resolve(answer)) });
    }),
    /** Settles the oldest read still out. */
    answer(answer: PublishedPacksMessage | Error) {
      pending.shift()!.settle(answer);
    },
  };
}

/** npm's answer listing `packs`, none left out. */
const found = (...packs: ReturnType<typeof listed>[]): PublishedPacksMessage => ({ packs, problems: [] });

/** A reader over `registry` at the clock `now` gives, its log kept in `logged`. */
function readerOver(registry: ReturnType<typeof npm>, now: () => number, logged: string[] = [], options: Partial<PublishedPackReaderOptions> = {}) {
  return publishedPackReader({ home: "/unused", discover: registry.discover, now, log: line => logged.push(line), ...options });
}

/** Once every promise the last answer settles has run: a read in the background has kept its list or failed. */
const settled = async () => await new Promise(resolve => setImmediate(resolve));

describe("publishedPackReader", () => {
  it("asks npm at a person's first read and waits for it, then shows that list for a day without asking again", async () => {
    const registry = npm();
    let now = 1_000;
    const reader = readerOver(registry, () => now);

    const first = reader.read(true);
    expect(registry.pending).toHaveLength(1);
    registry.answer(found(listed("react")));
    expect(await first).toEqual({ packs: [listed("react")], readAt: 1_000 });

    now += PUBLISHED_PACKS_MAX_AGE_MS - 1;
    expect(await reader.read(true)).toEqual({ packs: [listed("react")], readAt: 1_000 });
    expect(registry.pending).toHaveLength(0);
  });

  it("shows a day-old list at once while it reads the next one", async () => {
    const registry = npm();
    let now = 0;
    const reader = readerOver(registry, () => now);
    const first = reader.read(true);
    registry.answer(found(listed("react", "0.1.0")));
    await first;

    now = PUBLISHED_PACKS_MAX_AGE_MS;
    expect(await reader.read(true)).toEqual({ packs: [listed("react", "0.1.0")], readAt: 0 });
    expect(registry.pending).toHaveLength(1);
    // A read while that one is out joins it rather than asking npm again.
    expect(await reader.read(true)).toEqual({ packs: [listed("react", "0.1.0")], readAt: 0 });
    expect(registry.pending).toHaveLength(1);
    registry.answer(found(listed("react", "0.2.0")));
    await settled();
    expect(await reader.read(true)).toEqual({ packs: [listed("react", "0.2.0")], readAt: PUBLISHED_PACKS_MAX_AGE_MS });
  });

  it("reads npm once for reads that come together", async () => {
    const registry = npm();
    const reader = readerOver(registry, () => 5);
    const reads = [reader.read(true), reader.read(true)];
    expect(registry.pending).toHaveLength(1);
    registry.answer(found(listed("vue")));
    expect(await Promise.all(reads)).toEqual([{ packs: [listed("vue")], readAt: 5 }, { packs: [listed("vue")], readAt: 5 }]);
  });

  it("never asks npm for a read that does not refresh, a coding agent's, and says why there is no list", async () => {
    const registry = npm();
    const reader = readerOver(registry, Date.now);
    expect(await reader.read(false)).toEqual({ unread: expect.stringMatching(/when a person opens Add rule packs/u) as string });
    expect(registry.pending).toHaveLength(0);
  });

  it("after a failed read, says why and waits before asking npm again, then holds the list it reads", async () => {
    const registry = npm();
    let now = 7;
    const logged: string[] = [];
    const reader = readerOver(registry, () => now, logged);

    const offline = reader.read(true);
    registry.answer({ unread: "could not reach the npm registry" });
    expect(await offline).toEqual({ unread: "could not reach the npm registry" });
    expect(logged).toEqual([expect.stringMatching(/reading the published rule packs from npm failed, .* after 5 min: could not reach the npm registry\n$/u)]);

    // A person who comes back at once, and a coding agent, get why; npm is not asked.
    now += PUBLISHED_RETRY_MS - 1;
    expect(await reader.read(true)).toEqual({ unread: "could not reach the npm registry" });
    expect(await reader.read(false)).toEqual({ unread: "could not reach the npm registry" });
    expect(registry.pending).toHaveLength(0);

    now += 1;
    const failed = reader.read(true);
    registry.answer(new Error("reading the published rule packs ended with code 1 and no answer\nstack"));
    expect(await failed).toEqual({ unread: "reading the published rule packs ended with code 1 and no answer" });

    now += PUBLISHED_RETRY_MS;
    const read = reader.read(true);
    registry.answer(found(listed("sql")));
    expect(await read).toEqual({ packs: [listed("sql")], readAt: now });
    expect(logged).toHaveLength(2);
  });

  it("logs each pack a read leaves out, and still holds the others", async () => {
    const registry = npm();
    const logged: string[] = [];
    const reader = readerOver(registry, () => 3, logged);
    const read = reader.read(true);
    registry.answer({ packs: [listed("vue")], problems: ["@neralabs/rerail-pack-sql: npm install stopped after 60 s"] });
    expect(await read).toEqual({ packs: [listed("vue")], readAt: 3 });
    expect(logged).toEqual(["rerail daemon: Add rule packs leaves out a published rule pack: @neralabs/rerail-pack-sql: npm install stopped after 60 s\n"]);
  });

  it("keeps showing the list it holds when reading the next one fails", async () => {
    const registry = npm();
    let now = 0;
    const reader = readerOver(registry, () => now);
    const first = reader.read(true);
    registry.answer(found(listed("blade")));
    await first;

    now = PUBLISHED_PACKS_MAX_AGE_MS;
    await reader.read(true);
    registry.answer({ unread: "could not reach the npm registry" });
    await settled();
    expect(await reader.read(false)).toEqual({ packs: [listed("blade")], readAt: 0 });
    // Nor does a person's read start another until the wait after the failure has passed.
    expect(await reader.read(true)).toEqual({ packs: [listed("blade")], readAt: 0 });
    expect(registry.pending).toHaveLength(0);
  });
});

describe("publishedPacksOf", () => {
  const homes: string[] = [];
  afterEach(async () => { for (const home of homes.splice(0)) await removeFolder(home); });

  it("offers what discovery found, each pack at its version with the rules it starts on", () => {
    const discovered = { ...listed("react", "0.1.12"), match: { reason: "unused", folder: "" }, installed: null };
    expect(publishedPacksOf({ available: [discovered], problems: ["@neralabs/rerail-pack-sql: npm install stopped after 60 s"] }))
      .toEqual({ packs: [listed("react", "0.1.12")], problems: ["@neralabs/rerail-pack-sql: npm install stopped after 60 s"] });
  });

  it("says why when npm gave no pack, rather than listing the installed packs standing in for them", async () => {
    const home = realpathSync(mkdtempSync(join(tmpdir(), "rerail-published-offline-")));
    homes.push(home);
    installPackDirectory(repositoryPath("packs", "typescript"), "typescript", home);
    const offline: PackStore = {
      home,
      fetch: async ids => ({ fetched: [], problems: ids.map(id => ({ packageName: `@neralabs/rerail-pack-${id}`, reason: "could not reach the npm registry (ENOTFOUND)" })), release: async () => {} }),
      install: async () => {},
      remove: async () => {},
    };

    const discovery = await discoverPacks(undefined, offline);
    expect(discovery.available.map(found => found.pack.id)).toEqual(["typescript"]);
    expect(publishedPacksOf(discovery)).toEqual({ unread: "could not reach the npm registry (ENOTFOUND)" });
  });
});

describe("publishedPackReader's worker", () => {
  afterEach(() => { vi.unstubAllEnvs(); });

  it("runs discovery on a thread of its own, which ends with why when npm cannot reach its registry", { timeout: 120_000 }, async () => {
    const home = realpathSync(mkdtempSync(join(tmpdir(), "rerail-published-worker-")));
    try {
      // A registry nothing listens on, asked once, with a cache of its own: every pack fails at once.
      vi.stubEnv("npm_config_registry", "http://127.0.0.1:9/");
      vi.stubEnv("npm_config_fetch_retries", "0");
      vi.stubEnv("npm_config_cache", join(home, "npm-cache"));
      const logged: string[] = [];
      const reader = publishedPackReader({ home, log: line => logged.push(line) });

      const read = await reader.read(true);
      expect(read).toEqual({ unread: expect.stringMatching(/could not reach the npm registry \(http:\/\/127\.0\.0\.1:9, ECONNREFUSED\)/u) as string });
      expect(logged).toHaveLength(1);
    } finally {
      await removeFolder(home);
    }
  });
});

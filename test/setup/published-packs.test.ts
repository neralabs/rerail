import { describe, expect, it } from "vitest";
import type { RulePack } from "../../src/engine/index.js";
import { PUBLISHED_PACKS_MAX_AGE_MS, publishedPackReader, type PublishedPacksMessage } from "../../src/setup/published-packs.js";

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

/** Once every promise the last answer settles has run: a read in the background has kept its list or failed. */
const settled = async () => await new Promise(resolve => setImmediate(resolve));

describe("publishedPackReader", () => {
  it("asks npm at a person's first read and waits for it, then shows that list for a day without asking again", async () => {
    const registry = npm();
    let now = 1_000;
    const reader = publishedPackReader({ home: "/unused", discover: registry.discover, now: () => now });

    const first = reader.read(true);
    expect(registry.pending).toHaveLength(1);
    registry.answer({ packs: [listed("react")] });
    expect(await first).toEqual({ packs: [listed("react")], readAt: 1_000 });

    now += PUBLISHED_PACKS_MAX_AGE_MS - 1;
    expect(await reader.read(true)).toEqual({ packs: [listed("react")], readAt: 1_000 });
    expect(registry.pending).toHaveLength(0);
  });

  it("shows a day-old list at once while it reads the next one", async () => {
    const registry = npm();
    let now = 0;
    const reader = publishedPackReader({ home: "/unused", discover: registry.discover, now: () => now });
    const first = reader.read(true);
    registry.answer({ packs: [listed("react", "0.1.0")] });
    await first;

    now = PUBLISHED_PACKS_MAX_AGE_MS;
    expect(await reader.read(true)).toEqual({ packs: [listed("react", "0.1.0")], readAt: 0 });
    expect(registry.pending).toHaveLength(1);
    // A read while that one is out joins it rather than asking npm again.
    expect(await reader.read(true)).toEqual({ packs: [listed("react", "0.1.0")], readAt: 0 });
    expect(registry.pending).toHaveLength(1);
    registry.answer({ packs: [listed("react", "0.2.0")] });
    await settled();
    expect(await reader.read(true)).toEqual({ packs: [listed("react", "0.2.0")], readAt: PUBLISHED_PACKS_MAX_AGE_MS });
  });

  it("reads npm once for reads that come together", async () => {
    const registry = npm();
    const reader = publishedPackReader({ home: "/unused", discover: registry.discover, now: () => 5 });
    const reads = [reader.read(true), reader.read(true)];
    expect(registry.pending).toHaveLength(1);
    registry.answer({ packs: [listed("vue")] });
    expect(await Promise.all(reads)).toEqual([{ packs: [listed("vue")], readAt: 5 }, { packs: [listed("vue")], readAt: 5 }]);
  });

  it("never asks npm for a read that does not refresh, a coding agent's, and says why there is no list", async () => {
    const registry = npm();
    const reader = publishedPackReader({ home: "/unused", discover: registry.discover });
    expect(await reader.read(false)).toEqual({ unread: expect.stringMatching(/when a person opens Add rule packs/u) as string });
    expect(registry.pending).toHaveLength(0);
  });

  it("holds no failed read: the next person's read asks npm again, and an agent's still finds nothing", async () => {
    const registry = npm();
    const reader = publishedPackReader({ home: "/unused", discover: registry.discover, now: () => 7 });

    const offline = reader.read(true);
    registry.answer({ unread: "could not reach the npm registry" });
    expect(await offline).toEqual({ unread: "could not reach the npm registry" });

    const failed = reader.read(true);
    registry.answer(new Error("reading the published rule packs ended with code 1 and no answer\nstack"));
    expect(await failed).toEqual({ unread: "reading the published rule packs ended with code 1 and no answer" });
    expect(await reader.read(false)).toEqual({ unread: expect.any(String) as string });

    const read = reader.read(true);
    registry.answer({ packs: [listed("sql")] });
    expect(await read).toEqual({ packs: [listed("sql")], readAt: 7 });
  });

  it("keeps showing the list it holds when reading the next one fails", async () => {
    const registry = npm();
    let now = 0;
    const reader = publishedPackReader({ home: "/unused", discover: registry.discover, now: () => now });
    const first = reader.read(true);
    registry.answer({ packs: [listed("blade")] });
    await first;

    now = PUBLISHED_PACKS_MAX_AGE_MS;
    await reader.read(true);
    registry.answer({ unread: "could not reach the npm registry" });
    await settled();
    expect(await reader.read(false)).toEqual({ packs: [listed("blade")], readAt: 0 });
  });
});

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, parse, sep } from "node:path";
import { and, eq, ne } from "drizzle-orm";
import { afterEach, describe, expect, it } from "vitest";
import { openRerailDatabase, type RerailDatabase } from "../../../src/engine/storage/database.js";
import { checkouts, repositories } from "../../../src/engine/storage/schema.js";
import { withinCheckout } from "../../../src/engine/catalog/checkout-control.js";
import type { CheckoutRecord, RepositoryStatus } from "../../../src/engine/catalog/repository-model.js";
import {
  adoptRemote,
  checkoutStatus,
  linkAlias,
  listedCheckoutStates,
  listRepositories,
  lookupCheckout,
  parentCheckouts,
  pruneCheckouts,
  recordCheckout,
} from "../../../src/engine/catalog/repository-registry.js";
import { repositoryIdentityLocation } from "../../../src/engine/catalog/repository-identity.js";
import { setUpRepository } from "../../../src/engine/catalog/rule-configuration.js";

const temporaryDirectories: string[] = [];
const openDatabases: RerailDatabase[] = [];
afterEach(async () => {
  for (const database of openDatabases.splice(0)) database.$client.close();
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function temporary(prefix: string): Promise<string> {
  const path = realpathSync(await mkdtemp(join(tmpdir(), prefix)));
  temporaryDirectories.push(path);
  return path;
}

function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", ["-c", "protocol.file.allow=always", ...args], { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
}

function gitRepository(path: string, origin?: string): string {
  mkdirSync(path, { recursive: true });
  git(path, "init", "-q", "-b", "main");
  git(path, "-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", "commit", "-q", "--allow-empty", "-m", "base");
  if (origin !== undefined) git(path, "remote", "add", "origin", origin);
  return path;
}

/** Whether this filesystem matches names case-insensitively, as macOS does by default. */
function caseInsensitiveFilesystem(): boolean {
  const probe = realpathSync(mkdtempSync(join(tmpdir(), "rerail-case-")));
  try {
    return existsSync(probe.toUpperCase()) && existsSync(probe.toLowerCase());
  } finally {
    rmSync(probe, { recursive: true, force: true });
  }
}

function open(path: string): RerailDatabase {
  const database = openRerailDatabase(path);
  openDatabases.push(database);
  return database;
}

function checkoutSetting(database: RerailDatabase, checkoutPath: string): void {
  database.$client.prepare(
    "INSERT INTO rule_settings (owner_kind, owner_id, folder, target_kind, target_id, \"on\", mode, updated_at) VALUES ('checkout', ?, '', 'rule', 'r', 0, NULL, 1)",
  ).run(checkoutPath);
}

function settingOwners(database: RerailDatabase): unknown[] {
  return database.$client.prepare("SELECT owner_kind, owner_id FROM rule_settings ORDER BY owner_kind, owner_id").all();
}

/**
 * A stopped, set-up repository with `rows` (path, own on/off) recorded straight into the table in that order, in
 * spellings `recordCheckout` would not write but older builds and other tools left. The clock reads `times`, by
 * default one millisecond more for each row.
 */
async function recordedRows(rows: ReadonlyArray<readonly [string, boolean | null]>, times = rows.map((_, index) => index + 1)) {
  const database = open(join(await temporary("rerail-registry-listed-"), "rerail.sqlite"));
  const repositoryKey = "remote:github.com/acme/web";
  database.insert(repositories).values({ repositoryKey, displayName: "web", configuredAt: 1, started: false, createdAt: 1, updatedAt: 1 }).run();
  database.insert(checkouts).values(rows.map(([checkoutPath, started], index) => ({
    checkoutPath, repositoryKey, cloneId: "clone", started, createdAt: times[index]!, lastSeenAt: 1,
  }))).run();
  return { database, repositoryKey };
}

/** Each recorded checkout's on/off, what decided it and its nearest parent: as the dashboard's listing gives them, and as the runtime reads them. */
function bothReads(database: RerailDatabase, repositoryKey: string) {
  const listing = listRepositories(database).find(repository => repository.repositoryKey === repositoryKey)!;
  const stateAt = listedCheckoutStates(listing);
  const read = (status: RepositoryStatus, parent: string | null) => [status.started, status.startedBy, parent];
  return {
    listed: new Map(listing.checkouts.map(row => {
      const { status, parentCheckoutPath } = stateAt(row.checkoutPath);
      return [row.checkoutPath, read(status, parentCheckoutPath)];
    })),
    runtime: new Map(listing.checkouts.map(row => [
      row.checkoutPath,
      read(checkoutStatus(database, { ...row, git: true, main: true }), parentCheckouts(database, row)[0]?.checkoutPath ?? null),
    ])),
  };
}

/**
 * `main`'s readers of a checkout's containing checkouts and of its state, as they were before the dashboard's listing
 * reader, copied here to hold that reader to `main`'s answers. Containing checkouts of one path length come in the
 * order SQLite returns the rows: the order they were recorded in.
 */
function mainParentCheckouts(database: RerailDatabase, checkout: Pick<CheckoutRecord, "checkoutPath" | "repositoryKey">): CheckoutRecord[] {
  return database.select().from(checkouts)
    .where(and(eq(checkouts.repositoryKey, checkout.repositoryKey), ne(checkouts.checkoutPath, checkout.checkoutPath))).all()
    .filter(row => withinCheckout(row.checkoutPath, checkout.checkoutPath))
    .sort((left, right) => right.checkoutPath.length - left.checkoutPath.length);
}

/** `main`'s `checkoutStatus` of a recorded checkout (see {@link mainParentCheckouts}). */
function mainCheckoutStatus(database: RerailDatabase, checkout: Pick<CheckoutRecord, "checkoutPath" | "repositoryKey">): RepositoryStatus {
  const repository = database.select({ started: repositories.started, configuredAt: repositories.configuredAt })
    .from(repositories).where(eq(repositories.repositoryKey, checkout.repositoryKey)).get();
  const setUp = repository?.configuredAt != null;
  const own = database.select({ started: checkouts.started }).from(checkouts).where(eq(checkouts.checkoutPath, checkout.checkoutPath)).get()?.started;
  if (own != null) return { started: own, setUp, startedBy: "checkout" };
  const parent = mainParentCheckouts(database, checkout).find(row => row.started !== null);
  if (parent?.started != null) return { started: parent.started, setUp, startedBy: "parent" };
  return { started: repository?.started === true, setUp, startedBy: "repository" };
}

/** A checkout's on/off, whether its repository is set up, what decided the on/off, and its nearest parent. */
function answerOf(status: RepositoryStatus, parentCheckoutPath: string | null): string {
  return JSON.stringify([status.started, status.setUp, status.startedBy, parentCheckoutPath]);
}

/** Each recorded checkout's answer, by path, as the dashboard's listing reader gives it. */
function listedAnswers(database: RerailDatabase): Map<string, string> {
  const answers = new Map<string, string>();
  for (const listing of listRepositories(database)) {
    const stateAt = listedCheckoutStates(listing);
    for (const { checkoutPath } of listing.checkouts) {
      const { status, parentCheckoutPath } = stateAt(checkoutPath);
      answers.set(checkoutPath, answerOf(status, parentCheckoutPath));
    }
  }
  return answers;
}

/** Each recorded checkout's answer, by path, as `main`'s readers give it. */
function mainAnswers(database: RerailDatabase): Map<string, string> {
  return new Map(database.select().from(checkouts).all().map(row => [
    row.checkoutPath,
    answerOf(mainCheckoutStatus(database, row), mainParentCheckouts(database, row)[0]?.checkoutPath ?? null),
  ]));
}

/** Each recorded checkout's answer, by path, as the runtime's readers give it. */
function runtimeAnswers(database: RerailDatabase): Map<string, string> {
  return new Map(database.select().from(checkouts).all().map(row => [
    row.checkoutPath,
    answerOf(checkoutStatus(database, { ...row, git: true, main: true }), parentCheckouts(database, row)[0]?.checkoutPath ?? null),
  ]));
}

/** mulberry32: the same numbers from the same seed on every run. */
function seeded(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let mixed = Math.imul(state ^ (state >>> 15), state | 1);
    mixed ^= mixed + Math.imul(mixed ^ (mixed >>> 7), mixed | 61);
    return ((mixed ^ (mixed >>> 14)) >>> 0) / 4294967296;
  };
}

/** `items` in a random order. */
function shuffled<T>(items: readonly T[], next: () => number): T[] {
  const order = [...items];
  for (let index = order.length - 1; index > 0; index -= 1) {
    const other = Math.floor(next() * (index + 1));
    [order[index], order[other]] = [order[other]!, order[index]!];
  }
  return order;
}

/** Folder names of random trees: two in two letter cases each, two one letter apart, one that ends in a dot. */
const TREE_NAMES = ["a", "ab", "wt", "WT", "é", "É", "a."];
const ON_OFF = [true, false, null, null] as const;

/**
 * A folder as older builds and other tools recorded it: as is, with a `.` segment, with a trailing separator, or with a
 * trailing `.` segment. All four name one folder, and the second and the fourth are as long as each other.
 */
const SPELLINGS: ReadonlyArray<(segments: readonly string[], next: () => number) => string> = [
  segments => segments.join(sep),
  (segments, next) => {
    const at = 1 + Math.floor(next() * (segments.length - 1));
    return [...segments.slice(0, at), ".", ...segments.slice(at)].join(sep);
  },
  segments => `${segments.join(sep)}${sep}`,
  segments => `${segments.join(sep)}${sep}.`,
];

interface RandomTree {
  readonly repositories: ReadonlyArray<typeof repositories.$inferInsert>;
  /** In the order they were recorded. */
  readonly checkouts: ReadonlyArray<typeof checkouts.$inferInsert>;
}

/** Each of `folders` in one to three random spellings, each under one of `roots`. */
function spelledPaths(next: () => number, roots: readonly string[], folders: ReadonlyArray<readonly string[]>): string[] {
  const pick = <T>(items: readonly T[]): T => items[Math.floor(next() * items.length)]!;
  const paths = new Set<string>();
  for (const names of folders) {
    for (let count = 1 + Math.floor(next() * 3); count > 0; count -= 1) paths.add(pick(SPELLINGS)([pick(roots), ...names], next));
  }
  return [...paths];
}

/**
 * How the clock moves between two recordings: on a millisecond with chance `tick`, and back by up to a second with
 * chance `back`, as a clock set back by hand or by a time sync does.
 */
interface Clock {
  readonly tick: number;
  readonly back: number;
}

/**
 * The clocks of random trees, by seed: every row in one millisecond, about half of them in the millisecond of the row
 * before, each in a millisecond of its own, and one that goes back now and then.
 */
const CLOCKS: readonly Clock[] = [{ tick: 0, back: 0 }, { tick: 0.5, back: 0 }, { tick: 1, back: 0 }, { tick: 1, back: 0.5 }];

/** How far `clock` moves after a recording. */
function clockStep(clock: Clock, next: () => number): number {
  if (next() < clock.back) return -1 - Math.floor(next() * 1_000);
  return next() < clock.tick ? 1 : 0;
}

/** `count` times read from `clock`, one per recording. */
function recordingTimes(count: number, clock: Clock, next: () => number): number[] {
  const times: number[] = [];
  for (let now = 1_000_000; times.length < count; now += clockStep(clock, next)) times.push(now);
  return times;
}

/**
 * Two repositories and their checkouts in random folders and spellings under `roots`, now and then one at the
 * filesystem's root, recorded in a random order by one of {@link CLOCKS}.
 */
function randomTree(seed: number, roots: readonly string[]): RandomTree {
  const next = seeded(seed);
  const pick = <T>(items: readonly T[]): T => items[Math.floor(next() * items.length)]!;
  const folders: string[][] = [[]];
  for (let index = 0; index < 7; index += 1) folders.push([...pick(folders), pick(TREE_NAMES)]);
  const paths = shuffled([...spelledPaths(next, roots, folders), ...pick([[], [], [parse(roots[0]!).root]])], next);
  const times = recordingTimes(paths.length, CLOCKS[seed % CLOCKS.length]!, next);
  return {
    repositories: ["k1", "k2"].map(repositoryKey => ({
      repositoryKey, displayName: repositoryKey, started: pick(ON_OFF), configuredAt: pick([1, null]), createdAt: 1, updatedAt: 1,
    })),
    checkouts: paths.map((checkoutPath, index) => ({
      checkoutPath, repositoryKey: pick(["k1", "k1", "k1", "k2"]), cloneId: "clone", started: pick(ON_OFF), createdAt: times[index]!, lastSeenAt: 1,
    })),
  };
}

/** Puts `tree`'s rows in place of every repository and checkout row, the checkouts recorded in their order. */
function recordTree(database: RerailDatabase, tree: RandomTree): void {
  database.transaction(transaction => {
    transaction.delete(checkouts).run();
    transaction.delete(repositories).run();
    transaction.insert(repositories).values([...tree.repositories]).run();
    transaction.insert(checkouts).values([...tree.checkouts]).run();
  });
}

/** Each checkout `reader` answers otherwise than `expected` has it, with the seed of its tree. */
function differences(seed: number, reader: string, answers: Map<string, string>, expected: Map<string, string>): string[] {
  const differing = answers.size === expected.size ? [] : [`seed ${seed}: ${reader} answers ${answers.size} checkouts of ${expected.size}`];
  for (const [path, answer] of answers) {
    if (answer !== expected.get(path)) differing.push(`seed ${seed}, ${reader}, ${path}: ${answer}, where main gives ${expected.get(path)}`);
  }
  return differing;
}

/** Each checkout of `tree` that the dashboard's listing reader or the runtime's readers answer otherwise than main's. */
function comparedWithMain(database: RerailDatabase, seed: number, tree: RandomTree): string[] {
  recordTree(database, tree);
  const expected = mainAnswers(database);
  return [
    ...differences(seed, "listing", listedAnswers(database), expected),
    ...differences(seed, "runtime", runtimeAnswers(database), expected),
  ];
}

describe("the repository registry", () => {
  it("puts clones that share a remote and worktrees that share a clone in one repository", async () => {
    const root = await temporary("rerail-registry-join-");
    const database = open(join(root, "rerail.sqlite"));
    const first = gitRepository(join(root, "first"), "ssh://git@GitHub.com:22/acme/web.git/");
    const second = gitRepository(join(root, "second"), "git@github.com:acme/web.git");
    const worktree = join(root, "first-feature");
    git(first, "worktree", "add", "-q", "-b", "feature", worktree);

    const recorded = [first, second, worktree].map(path => recordCheckout(database, path));
    expect(recorded.map(checkout => checkout.repositoryKey)).toEqual(Array(3).fill("remote:github.com/acme/web"));
    expect(recorded[2]).toMatchObject({ checkoutPath: worktree, cloneId: recorded[0]!.cloneId, main: false });
    // A remote-keyed repository is named after its remote, not the folder it was first recorded in.
    expect(listRepositories(database)).toMatchObject([{ repositoryKey: "remote:github.com/acme/web", displayName: "web", checkouts: [{}, {}, {}] }]);

    // A recorded checkout stays where it is when its `origin` changes later.
    git(second, "remote", "set-url", "origin", "https://github.com/acme/other.git");
    expect(recordCheckout(database, second).repositoryKey).toBe("remote:github.com/acme/web");
  });

  it("keys a clone without an origin and with several remotes by its clone id", async () => {
    const root = await temporary("rerail-registry-remotes-");
    const database = open(join(root, "rerail.sqlite"));
    const checkout = gitRepository(join(root, "checkout"));
    git(checkout, "remote", "add", "upstream", "https://github.com/acme/web.git");
    git(checkout, "remote", "add", "fork", "https://github.com/someone/web.git");
    const recorded = recordCheckout(database, checkout);
    expect(recorded.repositoryKey).toBe(`clone:${recorded.cloneId}`);
  });

  it("records a re-clone at a recorded path again, without the old checkout's settings", async () => {
    const root = await temporary("rerail-registry-reclone-");
    const database = open(join(root, "rerail.sqlite"));
    const path = gitRepository(join(root, "checkout"));
    const before = recordCheckout(database, path);
    checkoutSetting(database, path);
    rmSync(path, { recursive: true, force: true });
    gitRepository(path, "https://github.com/acme/web.git");
    const after = recordCheckout(database, path);
    expect(after.cloneId).not.toBe(before.cloneId);
    expect(after.repositoryKey).toBe("remote:github.com/acme/web");
    expect(settingOwners(database)).toEqual([]);
  });

  it("finds the nearest containing checkout of the same repository first", async () => {
    const root = await temporary("rerail-registry-parent-");
    const database = open(join(root, "rerail.sqlite"));
    const main = gitRepository(join(root, "main"));
    const outer = join(main, "worktrees", "outer");
    const inner = join(outer, "worktrees", "inner");
    const beside = join(root, "beside");
    git(main, "worktree", "add", "-q", "-b", "outer", outer);
    git(main, "worktree", "add", "-q", "-b", "inner", inner);
    git(main, "worktree", "add", "-q", "-b", "beside", beside);
    const [, , innerCheckout] = [main, outer, inner, beside].map(path => recordCheckout(database, path));
    expect(parentCheckouts(database, innerCheckout!).map(row => row.checkoutPath)).toEqual([outer, main]);
  });

  it("gives each checkout in the dashboard's listing the state and parent the runtime gives it, in whatever spelling a folder was recorded", async () => {
    const base = join(parse(tmpdir()).root, "rerail-listed");
    const { database, repositoryKey } = await recordedRows([
      // The filesystem's root holds every other row.
      [parse(base).root, null],
      // A trailing separator and a `.` segment: the folders a and b, as path.relative reads them.
      [`${join(base, "a")}${sep}`, true],
      [join(base, "a", "inner"), null],
      [`${join(base, "b")}${sep}.`, true],
      [join(base, "b", "inner"), null],
      [join(base, "beside"), null],
      // One folder on Windows, where path.relative ignores letter case; two elsewhere.
      [join(base, "CASE"), true],
      [join(base, "case", "inner"), null],
      // A share root recorded without the separator path.resolve gives it.
      ...process.platform === "win32" ? [["\\\\server\\share", true], ["\\\\server\\share\\inner", null]] as const : [],
    ]);
    const { listed, runtime } = bothReads(database, repositoryKey);
    expect(listed).toEqual(runtime);
    // The inner checkouts follow their parents in the runtime's reads, so the two reads agree on more than the repository.
    expect(runtime.get(join(base, "a", "inner"))).toEqual([true, "parent", `${join(base, "a")}${sep}`]);
    expect(runtime.get(join(base, "b", "inner"))).toEqual([true, "parent", `${join(base, "b")}${sep}.`]);
    if (process.platform === "win32") expect(runtime.get("\\\\server\\share\\inner")).toEqual([true, "parent", "\\\\server\\share"]);
  });

  it("lets the spelling recorded first decide for the checkouts inside a folder recorded under two spellings of one length", async () => {
    const base = join(parse(tmpdir()).root, "rerail-listed");
    const inner = join(base, "wt", "inner");
    // One folder as path.relative reads both: a `.` segment before or after its name, and on Windows its letter case.
    const spellings: Array<readonly [string, string]> = [
      [`${base}${sep}.${sep}wt`, `${base}${sep}wt${sep}.`],
      ...process.platform === "win32" ? [[join(base, "wt"), join(base, "WT")] as const] : [],
    ];
    // The clock moving on between the recordings, standing still, and set back.
    const clocks = [[1, 2, 3], [1, 1, 1], [2_000, 1_000, 3_000]];
    for (const [one, other] of spellings) {
      for (const [first, second] of [[one, other], [other, one]] as const) {
        for (const times of clocks) {
          // Stopped as the spelling recorded first, started as the other one.
          const { database, repositoryKey } = await recordedRows([[first, false], [second, true], [inner, null]], times);
          const { listed, runtime } = bothReads(database, repositoryKey);
          expect(runtime.get(inner), `${first} then ${second} at ${times.join(", ")}`).toEqual([false, "parent", first]);
          expect(listed.get(inner), `${first} then ${second} at ${times.join(", ")}`).toEqual([false, "parent", first]);
        }
      }
    }
  });

  it("gives each checkout, in the dashboard's listing and in the runtime, the state and parent main gave it, whatever the clock did", async () => {
    const database = open(join(await temporary("rerail-registry-main-"), "rerail.sqlite"));
    // These rows need not survive a crash, so no transaction waits for the disk.
    database.$client.pragma("synchronous = OFF");
    const base = join(parse(tmpdir()).root, "rerail-listed");
    const roots = process.platform === "win32" ? [base, base, "\\\\server\\share"] : [base];
    const differing: string[] = [];
    for (let seed = 1; seed <= 200; seed += 1) differing.push(...comparedWithMain(database, seed, randomTree(seed, roots)));
    expect(differing).toEqual([]);
  }, 30_000);

  it("puts a submodule checked out under two worktrees of its parent in one repository", async () => {
    const root = await temporary("rerail-registry-submodule-");
    const database = open(join(root, "rerail.sqlite"));
    const library = gitRepository(join(root, "library"));
    const parent = gitRepository(join(root, "parent"));
    git(parent, "submodule", "add", "-q", library, "vendor/library");
    git(parent, "-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", "commit", "-q", "-m", "submodule");
    const worktree = join(root, "parent-feature");
    git(parent, "worktree", "add", "-q", "-b", "feature", worktree);
    git(worktree, "submodule", "update", "-q", "--init");
    const submodules = [join(parent, "vendor", "library"), join(worktree, "vendor", "library")];
    for (const submodule of submodules) git(submodule, "remote", "set-url", "origin", "https://github.com/acme/library.git");

    const recorded = submodules.map(path => recordCheckout(database, path));
    expect(recorded[0]!.cloneId).not.toBe(recorded[1]!.cloneId);
    expect(recorded.map(checkout => checkout.repositoryKey)).toEqual(["remote:github.com/acme/library", "remote:github.com/acme/library"]);
    expect(recordCheckout(database, parent).repositoryKey).not.toBe("remote:github.com/acme/library");
  });

  it("links an alias so a checkout with the old address joins the repository", async () => {
    const root = await temporary("rerail-registry-alias-");
    const database = open(join(root, "rerail.sqlite"));
    const current = gitRepository(join(root, "current"), "https://github.com/acme/web.git");
    const renamed = gitRepository(join(root, "renamed"), "git@github.com:acme/old-web.git");
    const key = recordCheckout(database, current).repositoryKey;
    expect(linkAlias(database, key, "https://github.com/acme/old-web")).toBe("remote:github.com/acme/old-web");
    expect(recordCheckout(database, renamed).repositoryKey).toBe(key);

    // An address a set-up repository already owns is refused.
    const other = gitRepository(join(root, "other"), "https://github.com/acme/api.git");
    const otherKey = recordCheckout(database, other).repositoryKey;
    setUpRepository(database, otherKey, { packs: [] });
    expect(() => linkAlias(database, key, "https://github.com/acme/api")).toThrow(/set-up repository/u);
  });

  it("keeps the merged repository's own key and aliases when an alias brings it in", async () => {
    const root = await temporary("rerail-registry-merge-");
    const database = open(join(root, "rerail.sqlite"));
    const web = gitRepository(join(root, "web"), "https://github.com/acme/web.git");
    const mirror = gitRepository(join(root, "mirror"), "https://github.com/acme/web-mirror.git");
    const key = recordCheckout(database, web).repositoryKey;
    const mirrorKey = recordCheckout(database, mirror).repositoryKey;
    linkAlias(database, mirrorKey, "https://github.com/acme/web-old");

    // Linking the mirror's alias to web brings the mirror's repository, with its own key and aliases, into web.
    linkAlias(database, key, "https://github.com/acme/web-old");
    const merged = listRepositories(database).find(repository => repository.repositoryKey === key)!;
    expect([...merged.aliases].sort()).toEqual([mirrorKey, "remote:github.com/acme/web-old"].sort());
    // A fresh clone of the mirror joins web instead of starting a repository of its own.
    const fresh = gitRepository(join(root, "fresh-mirror"), "https://github.com/acme/web-mirror.git");
    expect(recordCheckout(database, fresh).repositoryKey).toBe(key);
  });

  it("adopts a remote added to a clone after it was first recorded", async () => {
    const root = await temporary("rerail-registry-late-remote-");
    const database = open(join(root, "rerail.sqlite"));
    const clone = gitRepository(join(root, "clone"));
    expect(recordCheckout(database, clone).repositoryKey).toMatch(/^clone:/u);
    git(clone, "remote", "add", "origin", "https://github.com/acme/late.git");
    expect(recordCheckout(database, clone).repositoryKey).toBe("remote:github.com/acme/late");
  });

  it("gives an unconfigured target the clone repository's configuration, and moves only checkouts to a configured one", async () => {
    const root = await temporary("rerail-registry-adopt-");
    const database = open(join(root, "rerail.sqlite"));
    const clone = gitRepository(join(root, "clone"));
    const cloneCheckout = recordCheckout(database, clone);
    const cloneKey = cloneCheckout.repositoryKey;
    // A clone-keyed repository keeps its folder's name until it adopts a remote.
    expect(listRepositories(database)).toMatchObject([{ repositoryKey: cloneKey, displayName: "clone" }]);
    setUpRepository(database, cloneKey, {
      packs: [{ packId: "python", folder: "" }],
      settings: [{ owner: { kind: "repository", repositoryKey: cloneKey }, folder: "", targetKind: "rule", targetId: "r", on: false, mode: null }],
    });
    database.$client.prepare("UPDATE repositories SET started = 1 WHERE repository_key = ?").run(cloneKey);

    adoptRemote(database, cloneKey, "remote:github.com/acme/web");
    // Another process adopting the same repository afterwards finds it gone and leaves it as it is.
    adoptRemote(database, cloneKey, "remote:github.com/acme/web");
    expect(listRepositories(database)).toMatchObject([{
      repositoryKey: "remote:github.com/acme/web", displayName: "web", aliases: [cloneKey], started: true, configuredAt: expect.any(Number),
      checkouts: [{ checkoutPath: clone }],
    }]);
    expect(database.$client.prepare("SELECT repository_key, source_id FROM rule_attachments").all())
      .toEqual([{ repository_key: "remote:github.com/acme/web", source_id: "python" }]);
    expect(settingOwners(database)).toEqual([{ owner_kind: "repository", owner_id: "remote:github.com/acme/web" }]);

    // A configured target wins: the other repository keeps its key and loses only its checkouts.
    const stray = gitRepository(join(root, "stray"));
    const strayKey = recordCheckout(database, stray).repositoryKey;
    setUpRepository(database, strayKey, { packs: [{ packId: "web", folder: "" }] });
    adoptRemote(database, strayKey, "remote:github.com/acme/web");
    const repositories = listRepositories(database);
    expect(repositories.find(row => row.repositoryKey === "remote:github.com/acme/web")?.checkouts.map(row => row.checkoutPath).sort())
      .toEqual([clone, stray].sort());
    expect(repositories.find(row => row.repositoryKey === strayKey)?.checkouts).toEqual([]);
  });

  it("prunes a removed worktree and its settings, and keeps live checkouts", async () => {
    const root = await temporary("rerail-registry-prune-");
    const database = open(join(root, "rerail.sqlite"));
    const main = gitRepository(join(root, "main"));
    const worktree = join(root, "feature");
    git(main, "worktree", "add", "-q", "-b", "feature", worktree);
    recordCheckout(database, main);
    recordCheckout(database, worktree);
    checkoutSetting(database, worktree);
    checkoutSetting(database, main);
    git(main, "worktree", "remove", worktree);

    expect(pruneCheckouts(database)).toEqual([worktree]);
    expect(listRepositories(database)[0]?.checkouts.map(row => row.checkoutPath)).toEqual([main]);
    expect(settingOwners(database)).toEqual([{ owner_kind: "checkout", owner_id: main }]);
  });

  it.skipIf(!caseInsensitiveFilesystem())("keeps a worktree Git registered under a path typed in another case than the folder has", async () => {
    const root = await temporary("rerail-registry-prune-case-");
    const database = open(join(root, "rerail.sqlite"));
    const main = gitRepository(join(root, "main"));
    mkdirSync(join(root, "elsewhere"));
    git(main, "worktree", "add", "-q", "-b", "feature", join(root, "Elsewhere", "feature"));
    recordCheckout(database, main);
    const worktree = recordCheckout(database, join(root, "elsewhere", "feature")).checkoutPath;
    expect(worktree).toBe(join(root, "elsewhere", "feature"));

    expect(pruneCheckouts(database)).toEqual([]);
  });

  it("looks a checkout up without writing anything or creating a clone id", async () => {
    const root = await temporary("rerail-registry-lookup-");
    const database = open(join(root, "rerail.sqlite"));
    const checkout = gitRepository(join(root, "checkout"));
    expect(lookupCheckout(database, checkout)).toBeUndefined();
    expect(existsSync(repositoryIdentityLocation(checkout).identityPath)).toBe(false);
    expect(listRepositories(database)).toEqual([]);
    const recorded = recordCheckout(database, checkout);
    mkdirSync(join(checkout, "src"));
    expect(lookupCheckout(database, join(checkout, "src"))).toEqual(recorded);
  });

  it("finds a fresh clone without an id file by its remote, with no clone id, and records it under that repository", async () => {
    const root = await temporary("rerail-registry-fresh-");
    const database = open(join(root, "rerail.sqlite"));
    const first = gitRepository(join(root, "first"), "https://github.com/acme/web.git");
    const fresh = gitRepository(join(root, "fresh"), "git@github.com:acme/web.git");
    const known = recordCheckout(database, first);

    expect(lookupCheckout(database, fresh)).toEqual({ checkoutPath: fresh, repositoryKey: known.repositoryKey, cloneId: null, git: true, main: true });
    expect(existsSync(repositoryIdentityLocation(fresh).identityPath)).toBe(false);
    expect(listRepositories(database)[0]?.checkouts.map(row => row.checkoutPath)).toEqual([first]);

    const recorded = recordCheckout(database, fresh);
    expect(recorded).toMatchObject({ checkoutPath: fresh, repositoryKey: known.repositoryKey, cloneId: expect.any(String) });
    expect(recorded.cloneId).not.toBe(known.cloneId);
    expect(lookupCheckout(database, fresh)).toEqual(recorded);
  });
});



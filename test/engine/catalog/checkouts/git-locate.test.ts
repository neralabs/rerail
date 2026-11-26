/**
 * The three reads that ask Git where a folder's repository is (`repositoryIdentityLocation`, `placeWithoutBlocking`,
 * `repositoryIdentityLocationAsync`) when the Git asked gives no answer: a Git that never ran (Windows' 0xC0000142,
 * as CI saw it on a loaded runner; on Linux the loader's exit 127), one ended at its time limit, and one that failed
 * for another reason (exit 1). Each such ask reads the folder as outside Git for that ask alone, as before, with the
 * ending named on the answer; recording the checkout and the map of which checkout owns a file refuse to keep it, and
 * the next ask, whose Git answers, places it. Git's own answer that a folder is outside a repository still reads as
 * one, in a person's language too. The read of a clone's legacy id from Git's config fails on every ending but Git's
 * answers: a clone whose config holds one gets no new id when that Git gives no answer.
 *
 * The stand-in is a real process, this Node running a script, started in place of the one Git that is asked where the
 * folder is, or for the config's legacy id ({@link STAND_IN}); every other Git is the machine's, and each one a case
 * starts for its set-up goes through `runProgram`, whose failure says how it ended.
 */
import { existsSync, mkdirSync, mkdtempSync, realpathSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, afterEach, describe, expect, it, vi } from "vitest";
import { physicalPath } from "../../../../src/engine/catalog/checkout-control.js";
import { CheckoutOwners } from "../../../../src/engine/catalog/checkout-owner.js";
import { placeWithoutBlocking, repositoryIdentityLocation, repositoryIdentityLocationAsync } from "../../../../src/engine/catalog/repository-identity.js";
import { recordCheckout } from "../../../../src/engine/catalog/repository-registry.js";
import { openRerailDatabase } from "../../../../src/engine/storage/database.js";
import { removeFolderSync } from "../../../support/platform/remove-folder.js";
import { runProgram } from "../../../support/processes/run-program.js";

/** The ending the next asks take, how many of them, the argument that names that ask, and the script that plays it. */
const next = vi.hoisted(() => ({ ending: undefined as string | undefined, asks: 1, ask: "--show-toplevel", script: "" }));

vi.mock("../../../../src/engine/child-process.js", async importOriginal => {
  const actual = await importOriginal<typeof import("../../../../src/engine/child-process.js")>();
  /** The program and arguments to start: the stand-in, for the next `next.asks` asks `next.ask` names. */
  const started = (file: string, args: readonly string[]): [string, string[]] => {
    const ending = next.ending;
    if (ending === undefined || !args.includes(next.ask)) return [file, [...args]];
    next.asks -= 1;
    if (next.asks === 0) next.ending = undefined;
    return [process.execPath, [next.script, ending]];
  };
  const spawn = ((file: string, args: readonly string[], options: object) => actual.spawn(...started(file, args), options)) as typeof actual.spawn;
  const spawnSync = ((file: string, args: readonly string[], options: object) => actual.spawnSync(...started(file, args), options)) as typeof actual.spawnSync;
  return { ...actual, spawn, spawnSync };
});

/**
 * Each ending: Windows' status for a process it could not start, and Linux's loader failing; past the time limit; exit 1;
 * killed (Windows has no signals: its kill ends a process with exit status 1); a wrapper that prints Git's own line for a
 * folder outside a repository but exits 1. And Git's own answer, dying (128), here for a config it cannot read.
 */
const STAND_IN = String.raw`
const ending = process.argv[2];
if (ending === "never-ran" && process.platform === "win32") process.exit(3221225794);
if (ending === "never-ran") {
  process.stderr.write("git: error while loading shared libraries: libpcre2-8.so.0: cannot open shared object file: No such file or directory\n");
  process.exit(127);
}
if (ending === "past-limit") setTimeout(() => process.exit(0), 60000);
if (ending === "failed") {
  process.stderr.write("error: could not lock config file .git/config: File exists\n");
  process.exit(1);
}
if (ending === "killed") process.kill(process.pid, "SIGKILL");
if (ending === "wrapped") {
  process.stderr.write("fatal: not a git repository (or any of the parent directories): .git\n");
  process.exit(1);
}
if (ending === "git-refuses") {
  process.stderr.write("fatal: bad config line 1 in file .git/config\n");
  process.exit(128);
}
`;

const root = realpathSync.native(mkdtempSync(join(tmpdir(), "rerail-git-could-not-answer-")));
next.script = join(root, "stand-in.cjs");
writeFileSync(next.script, STAND_IN);
let made = 0;

afterEach(() => {
  next.ending = undefined;
  next.asks = 1;
  next.ask = "--show-toplevel";
  vi.unstubAllEnvs();
  vi.restoreAllMocks();
});
afterAll(() => {
  removeFolderSync(root);
});

function git(cwd: string, ...args: string[]): void {
  runProgram("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", ...args], { cwd });
}

/** A new checkout with one commit, which no read in this process has placed yet. */
function checkout(): string {
  made += 1;
  const top = join(root, `checkout-${made}`);
  git(root, "init", "-q", top);
  writeFileSync(join(top, "a.txt"), "a\n");
  git(top, "commit", "-q", "--allow-empty", "-m", "seed");
  return top;
}

/** What the failure of an ask that ended so says, after the folder: each names the status in decimal and hex, or none. */
const SAID: Readonly<Record<string, RegExp>> = {
  "never-ran": process.platform === "win32"
    ? /: exit status 3221225794 \(0xC0000142\), signal none, stderr: nothing$/u
    : /: exit status 127 \(0x0000007F\), signal none, stderr: git: error while loading shared libraries: /u,
  "past-limit": /: no exit status \(.*(?:ETIMEDOUT|time limit).*\), signal \w+, stderr: nothing$/u,
  failed: /: exit status 1 \(0x00000001\), signal none, stderr: error: could not lock config file \.git\/config: File exists$/u,
  killed: process.platform === "win32" ? /: exit status 1 \(0x00000001\), signal none, stderr: nothing$/u : /: no exit status \(.*\), signal SIGKILL, stderr: nothing$/u,
  wrapped: /: exit status 1 \(0x00000001\), signal none, stderr: fatal: not a git repository /u,
};

const READS = {
  "on the caller's thread": async (folder: string) => repositoryIdentityLocation(folder),
  "without blocking, for the placements after it": async (folder: string) => await placeWithoutBlocking(folder),
  "without blocking, for Merge Assist": async (folder: string) => await repositoryIdentityLocationAsync(folder),
};

describe("a read of where a folder's repository is, whose Git gives no answer", () => {
  for (const [name, read] of Object.entries(READS)) {
    for (const ending of Object.keys(SAID)) {
      it(`answers ${name} outside Git for that ask alone when that Git ends so (${ending}), naming it, and the next ask places the checkout`, async () => {
        const top = checkout();
        const write = vi.spyOn(process.stderr, "write");
        next.ending = ending;
        const first = await read(top);
        expect(first).toMatchObject({ git: false, workspaceRoot: top });
        expect(first?.unanswered?.message).toMatch(SAID[ending]!);
        expect(first?.unanswered?.message).toContain(`Git could not say where the repository of ${top} is: `);
        expect(next.ending).toBeUndefined();
        // Only the daemon writes the line.
        expect(write).not.toHaveBeenCalled();
        await expect(read(top)).resolves.toMatchObject({ git: true, main: true, workspaceRoot: top });
      }, 30_000);
    }
  }

  it("records nothing for a checkout whose asks failed: no repository, checkout row or identity in its folder", () => {
    const top = checkout();
    const database = openRerailDatabase(join(root, `record-${made}.sqlite`));
    try {
      // Recording asks twice: whether the folder is one Rerail may record, then where its clone id lives.
      next.ending = "never-ran";
      next.asks = 2;
      expect(() => recordCheckout(database, top)).toThrow(SAID["never-ran"]!);
      expect(existsSync(join(top, ".rerail"))).toBe(false);
      expect(database.$client.prepare("select count(*) as n from checkouts").get()).toEqual({ n: 0 });
      expect(database.$client.prepare("select count(*) as n from repositories").get()).toEqual({ n: 0 });
      expect(recordCheckout(database, top).git).toBe(true);
    } finally {
      database.$client.close();
    }
  });

  it("keeps no owner for a checkout whose ask failed: the next lookup asks Git again and finds it", () => {
    const top = checkout();
    const owners = new CheckoutOwners();
    next.ending = "never-ran";
    expect(owners.ownerOf(join(top, "a.txt"))).toBeUndefined();
    expect(next.ending).toBeUndefined();
    expect(owners.ownerOf(join(top, "a.txt"))?.checkoutRoot).toBe(physicalPath(top));
  });

  it("reads a stopped read as outside Git, as before, and says nothing", async () => {
    const top = checkout();
    const write = vi.spyOn(process.stderr, "write");
    const stop = new AbortController();
    next.ending = "past-limit";
    const asked = repositoryIdentityLocationAsync(top, stop.signal);
    stop.abort();
    const location = await asked;
    expect(location).toMatchObject({ git: false });
    expect(location.unanswered).toBeUndefined();
    expect(write).not.toHaveBeenCalled();
  }, 30_000);
});

describe("a read of a clone's legacy id from Git's config, whose Git gives no answer", () => {
  /** The endings of a Git that gave no answer, and Git's own death (128), which is no answer for a config value either. */
  const ENDINGS: Readonly<Record<string, RegExp>> = {
    "never-ran": SAID["never-ran"]!,
    "past-limit": SAID["past-limit"]!,
    "git-refuses": /: exit status 128 \(0x00000080\), signal none, stderr: fatal: bad config line 1 in file \.git\/config$/u,
  };

  for (const [ending, said] of Object.entries(ENDINGS)) {
    it(`records nothing for a clone whose config holds its id when that Git ends so (${ending}), and the next record takes that id`, () => {
      const top = checkout();
      git(top, "config", "rerail.repository-id", "legacy-id");
      const database = openRerailDatabase(join(root, `legacy-${made}.sqlite`));
      try {
        next.ask = "rerail.repository-id";
        next.ending = ending;
        expect(() => recordCheckout(database, top)).toThrow(said);
        expect(next.ending).toBeUndefined();
        expect(existsSync(join(top, ".git", "rerail-repository-id"))).toBe(false);
        expect(recordCheckout(database, top).cloneId).toBe("legacy-id");
      } finally {
        database.$client.close();
      }
    }, 30_000);
  }

  it("names the ask in its line, and records no checkout or repository", () => {
    const top = checkout();
    git(top, "config", "rerail.repository-id", "legacy-id");
    const database = openRerailDatabase(join(root, `legacy-${made}.sqlite`));
    try {
      next.ask = "rerail.repository-id";
      next.ending = "never-ran";
      expect(() => recordCheckout(database, top)).toThrow(`Git could not say which clone id the config of the repository of ${top} holds: `);
      expect(database.$client.prepare("select count(*) as n from checkouts").get()).toEqual({ n: 0 });
      expect(database.$client.prepare("select count(*) as n from repositories").get()).toEqual({ n: 0 });
    } finally {
      database.$client.close();
    }
  });

  it("gives a clone whose config holds no id a new one, as Git's exit 1 says the key is not set", () => {
    const top = checkout();
    const database = openRerailDatabase(join(root, `legacy-${made}.sqlite`));
    try {
      expect(recordCheckout(database, top).cloneId).toMatch(/^git:[0-9a-f-]{36}$/u);
    } finally {
      database.$client.close();
    }
  });
});

describe("Git's own answer that a folder is outside any repository", () => {
  it("reads as outside Git, in English and in another language", async () => {
    // A `.git` folder Git does not take: the disk cannot tell, so Git is asked, and dies with 'not a git repository'.
    const folder = join(root, `empty-git-${made}`);
    mkdirSync(join(folder, ".git"), { recursive: true });
    for (const language of [undefined, "fr", "de"]) {
      vi.stubEnv("LANG", language === undefined ? process.env.LANG ?? "" : "C.UTF-8");
      vi.stubEnv("LANGUAGE", language ?? "");
      expect(repositoryIdentityLocation(folder)).toMatchObject({ git: false, workspaceRoot: folder });
      await expect(repositoryIdentityLocationAsync(folder)).resolves.toMatchObject({ git: false });
    }
  });

  it("reads a checkout another account owns as outside Git, as Git refuses it", async () => {
    const top = checkout();
    vi.stubEnv("GIT_TEST_ASSUME_DIFFERENT_OWNER", "1");
    expect(repositoryIdentityLocation(top)).toMatchObject({ git: false });
    await expect(repositoryIdentityLocationAsync(top)).resolves.toMatchObject({ git: false });
  });

  it("reads a folder whose Git dies for any reason of its own (128) as outside Git, whatever line it prints", async () => {
    const top = checkout();
    next.ending = "git-refuses";
    const placed = repositoryIdentityLocation(top);
    expect(placed).toMatchObject({ git: false, workspaceRoot: top });
    // Git's own answer: nothing refuses to record it.
    expect(placed.unanswered).toBeUndefined();
    next.ending = "git-refuses";
    expect((await repositoryIdentityLocationAsync(top)).unanswered).toBeUndefined();
  });

  it("reads a folder whose .git file Git cannot read as one as outside Git", () => {
    made += 1;
    const folder = join(root, `garbled-${made}`);
    mkdirSync(folder, { recursive: true });
    writeFileSync(join(folder, ".git"), "not a gitdir line\n");
    expect(repositoryIdentityLocation(folder)).toMatchObject({ git: false, workspaceRoot: folder });
  });

  it("reads a Git folder, which has no top level, as outside Git", () => {
    const top = checkout();
    expect(repositoryIdentityLocation(join(top, ".git"))).toMatchObject({ git: false });
  });
});

describe("the daemon's line for an ask Git gave no answer", () => {
  it("is written once for each folder and ending, and again after an ask there is answered", async () => {
    vi.resetModules();
    const git = await import("../../../../src/engine/git.js");
    const identity = await import("../../../../src/engine/catalog/repository-identity.js");
    git.gitLivesOn();
    const top = checkout();
    const write = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    const fail = async (ending: string): Promise<void> => {
      next.ending = ending;
      expect((await identity.repositoryIdentityLocationAsync(top)).unanswered?.message).toMatch(SAID[ending]!);
    };
    await fail("failed");
    await fail("failed");
    await fail("never-ran");
    await expect(identity.repositoryIdentityLocationAsync(top)).resolves.toMatchObject({ git: true });
    await fail("never-ran");
    const lines = write.mock.calls.map(([text]) => String(text));
    expect(lines).toHaveLength(3);
    expect(lines.every(line => line.startsWith(`rerail: Git could not say where the repository of ${top} is: `))).toBe(true);
    expect(lines.map(line => line.match(/exit status (\d+)/u)?.[1])).toEqual(["1", ...Array<string>(2).fill(process.platform === "win32" ? "3221225794" : "127")]);
  });
});

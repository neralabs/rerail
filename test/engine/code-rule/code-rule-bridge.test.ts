import { execFileSync } from "node:child_process";
import type * as ChildProcess from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import { appendFile, lstat, mkdir, mkdtemp, readFile, realpath, rename, rm, utimes, writeFile } from "node:fs/promises";
import type * as FileSystem from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, relative, resolve, sep } from "node:path";
import { setFlagsFromString } from "node:v8";
import { runInNewContext } from "node:vm";
import { afterEach, describe, expect, it, onTestFinished, vi } from "vitest";
import {
  CodeRuleBridge,
  filesystemClock,
  generatedEditPaths,
  isCommandTool,
  ledgerPreimage,
  type Preimage,
  type SourceObservationCache,
} from "../../../src/engine/code-rule/code-rule-bridge.js";
import { MAX_FINISHED_CALLS, ToolCallWindows } from "../../../src/engine/code-rule/tool-call-windows.js";
import { claudeCodeReportedPreimage, type ReportedPreimage } from "../../../src/engine/code-rule/tool-result/reported-preimage.js";
import type { CodeRuleEditBatch, CodeRuleAuditor, CodeRuleResponse } from "../../../src/engine/code-rule/code-rule.js";
import type { IgnoreRulesChange } from "../../../src/engine/code-rule/ignore-rule-files.js";
import type * as Git from "../../../src/engine/git.js";
import { codeRuleDialectForPath } from "../../../src/engine/code-rule/code-rule-languages.js";
import type * as WalkReads from "../../../src/engine/code-rule/walk-read-buffers.js";
import { holdRemovals } from "../../support/platform/held-removal.js";
import { linkFolder } from "../../support/platform/links.js";
import { removeFolder, runningIn } from "../../support/platform/remove-folder.js";

const discovery = vi.hoisted(() => ({
  reads: new Map<string, Array<() => Promise<void>>>(),
  directories: new Map<string, Array<() => Promise<void>>>(),
  /** A file handle about to open there, before any byte is read. */
  opens: new Map<string, Array<() => Promise<void>>>(),
}));
/**
 * The arguments of every Git process Rerail started without blocking, for the tests that pin what a pass costs, a Git
 * command (`failing`) that is made to fail, by an option Git does not know, while it is set, and one (`hanging`, by
 * every argument it holds) that runs until it is ended instead: `cat-file --batch` waits for revisions on an input
 * nothing writes to. The next command holding `throwsNext` throws as it starts, as a spawn Node cannot make (ENOMEM) does.
 */
const asyncGitCalls = vi.hoisted(() => ({
  args: [] as string[][], failing: undefined as string | undefined, hanging: undefined as readonly string[] | undefined,
  throwsNext: undefined as string | undefined,
}));
/**
 * The listings of a checkout's changed files a test makes late: the next walk listing (`gitModifiedOrUntrackedPaths`) of
 * each checkout in `late`, and the next listing asked again (`gitModifiedOrUntrackedListing`) of each in `lateAgain`,
 * once per entry, answer as a Git still at work at its time limit does, and start no Git. While `held` is set, each
 * listing asked again waits for it first; each of the next ones asked again takes the first hold left in `next`, names
 * its checkout to it and waits for its release; each then waits `delayMs` before it asks Git. `calls` names each
 * listing asked for, in order: `walk <checkout>` or `again <checkout>`; `asked` holds each listing asked again, in
 * order, with when the bridge asked for it and when it answered (`performance.now()`).
 */
const listings = vi.hoisted(() => ({
  late: [] as string[], lateAgain: [] as string[], held: undefined as Promise<void> | undefined, calls: [] as string[],
  next: [] as Array<{ asked(checkout: string): void; released: Promise<void> }>, delayMs: 0,
  asked: [] as Array<{ checkout: string; asked: number; answered: number | undefined }>,
}));
/** For each path in it, the next file handles opened there call these, in turn, once closed (`readEnded`). */
const closedReads = vi.hoisted(() => new Map<string, Array<() => void>>());
/** How many of the next Git reads run, in Git's place, a program that stays until it is ended, as a Git still at work does. */
const heldGit = vi.hoisted(() => ({ reads: 0 }));
/** How many of the files a walk read were made into text (`LentBytes.text`), for the tests that pin what a walk keeps. */
const decodedReads = vi.hoisted(() => ({ count: 0 }));
/** Each buffer a file read filled while `watching`, by `readFile` or a file handle's `read`, for what a walk allocates. */
const readBuffers = vi.hoisted(() => ({ watching: false, seen: new Set<ArrayBufferLike>() }));
vi.mock("node:child_process", async importOriginal => {
  const actual = await importOriginal<typeof ChildProcess>();
  // `git.ts` starts the Git of each read that does not block through `spawn`, with `-C <checkout>`; any other process
  // goes through unchanged.
  const spawn = (...call: Parameters<typeof actual.spawn>) => {
    if (Array.isArray(call[1]) && call[1].includes("-C")) {
      const args = call[1] as readonly string[];
      if (asyncGitCalls.throwsNext !== undefined && args.includes(asyncGitCalls.throwsNext)) {
        asyncGitCalls.throwsNext = undefined;
        throw new Error("spawn ENOMEM");
      }
      const fails = asyncGitCalls.failing !== undefined && args.includes(asyncGitCalls.failing);
      const hangs = asyncGitCalls.hanging?.every(part => args.includes(part)) === true;
      const forwarded = hangs ? [...args.slice(0, args.indexOf("-C") + 2), "cat-file", "--batch"] : fails ? [...args, "--no-such-option"] : [...args];
      call[1] = forwarded;
      asyncGitCalls.args.push(forwarded);
      if (heldGit.reads > 0) {
        heldGit.reads -= 1;
        call[0] = process.execPath;
        call[1] = ["-e", "setTimeout(() => {}, 30_000)", "--", ...forwarded];
      }
    }
    return (actual.spawn as (...forwarded: unknown[]) => ReturnType<typeof actual.spawn>)(...call);
  };
  return { ...actual, spawn };
});
vi.mock("node:fs/promises", async importOriginal => {
  const actual = await importOriginal<typeof FileSystem>();
  return {
    ...actual,
    readFile: async (...args: Parameters<typeof actual.readFile>) => {
      const contents = await actual.readFile(...args);
      if (readBuffers.watching && Buffer.isBuffer(contents)) readBuffers.seen.add(contents.buffer);
      return contents;
    },
    open: async (...args: Parameters<typeof actual.open>) => {
      await discovery.opens.get(String(args[0]))?.shift()?.();
      const handle = await actual.open(...args);
      const closed = closedReads.get(String(args[0]))?.shift();
      if (closed !== undefined) {
        const close = handle.close.bind(handle);
        handle.close = async () => {
          await close();
          closed();
        };
      }
      if (!readBuffers.watching) return handle;
      // Every form of `read` passes through unchanged; only the buffer it fills is noted.
      const read = handle.read.bind(handle) as (...call: unknown[]) => ReturnType<typeof handle.read>;
      handle.read = ((...call: unknown[]) => {
        if (Buffer.isBuffer(call[0])) readBuffers.seen.add(call[0].buffer);
        return read(...call);
      }) as typeof handle.read;
      return handle;
    },
    opendir: async (...args: Parameters<typeof actual.opendir>) => {
      await discovery.directories.get(String(args[0]))?.shift()?.();
      return await actual.opendir(...args);
    },
    // A test can make Windows hold a folder for a moment (holdRemovals).
    rm: vi.fn(actual.rm),
  };
});
vi.mock("../../../src/engine/code-rule/walk-read-buffers.js", async importOriginal => {
  const actual = await importOriginal<typeof WalkReads>();
  /** A walk's read held once its bytes are in and the file closed (`discovery.reads`), each copy made into text counted. */
  class HeldReadBuffers extends actual.WalkReadBuffers {
    override async lend<T>(absolute: string, size: number, use: (read: WalkReads.LentBytes) => Promise<T>): Promise<T> {
      return await super.lend(absolute, size, async read => {
        await discovery.reads.get(absolute)?.shift()?.();
        return await use({ bytes: read.bytes, text: () => countedText(read) });
      });
    }
  }
  function countedText(read: WalkReads.LentBytes): string {
    decodedReads.count += 1;
    return read.text();
  }
  return { ...actual, WalkReadBuffers: HeldReadBuffers };
});
vi.mock("../../../src/engine/git.js", async importOriginal => {
  const actual = await importOriginal<typeof Git>();
  /** Takes `checkout` out of `list` once, and says whether it was there. */
  const taken = (list: string[], checkout: string): boolean => {
    const at = list.indexOf(checkout);
    if (at >= 0) list.splice(at, 1);
    return at >= 0;
  };
  return {
    ...actual,
    gitModifiedOrUntrackedPaths: async (checkout: string, options?: Git.GitReadOptions) => {
      listings.calls.push(`walk ${checkout}`);
      return taken(listings.late, checkout) ? undefined : await actual.gitModifiedOrUntrackedPaths(checkout, options);
    },
    gitModifiedOrUntrackedListing: async (checkout: string, options?: Git.GitReadOptions): Promise<Git.ChangedPathsListing> => {
      listings.calls.push(`again ${checkout}`);
      const asked = { checkout, asked: performance.now(), answered: undefined as number | undefined };
      listings.asked.push(asked);
      try {
        const next = listings.next.shift();
        next?.asked(checkout);
        await next?.released;
        await listings.held;
        if (listings.delayMs > 0) await new Promise(resolve => setTimeout(resolve, listings.delayMs));
        return taken(listings.lateAgain, checkout) ? { ended: "timed-out" } : await actual.gitModifiedOrUntrackedListing(checkout, options);
      } finally {
        asked.answered = performance.now();
      }
    },
  };
});

interface DiscoveryGate {
  entered: Promise<void>;
  release(): void;
  started: boolean;
}
const bridges: CodeRuleBridge[] = [];
const releases: Array<() => void> = [];
function holdDiscovery(kind: "reads" | "directories" | "opens", path: string): DiscoveryGate {
  const entered = Promise.withResolvers<void>();
  const released = Promise.withResolvers<void>();
  const gate = { entered: entered.promise, release: released.resolve, started: false };
  const queue = discovery[kind].get(path) ?? [];
  queue.push(async () => {
    gate.started = true;
    entered.resolve();
    await released.promise;
  });
  discovery[kind].set(path, queue);
  releases.push(gate.release);
  return gate;
}

/** Holds every listing asked again before it asks Git (`listings.held`) until `release`, which the clean-up calls too. */
function holdListingsAgain(): { release(): void } {
  const held = Promise.withResolvers<void>();
  listings.held = held.promise;
  releases.push(held.resolve);
  return { release: held.resolve };
}

/**
 * Holds the next listing asked again that no earlier hold took before it asks Git (`listings.next`) until `release`,
 * which the clean-up calls too; `asked` resolves with its checkout.
 */
function holdNextListingAgain(): { asked: Promise<string>; release(): void } {
  const asked = Promise.withResolvers<string>();
  const released = Promise.withResolvers<void>();
  listings.next.push({ asked: asked.resolve, released: released.promise });
  releases.push(released.resolve);
  return { asked: asked.promise, release: released.resolve };
}

/**
 * Runs `run` with every `setTimeout` held back, and drops those it started once it ends: a slot a listing asked again
 * took meanwhile is handed on only once its Git answers, never at the wait (`LISTING_AGAIN_WAIT_MS`), however slow the
 * machine. Nothing `run` waits for may need a timer.
 */
async function withoutSlotWaits<T>(run: () => Promise<T>): Promise<T> {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  try {
    return await run();
  } finally {
    vi.useRealTimers();
  }
}

/** As in `code-rule-bridge.ts`: how many listings asked again hold a slot to ask Git at once, and for how long at most. */
const LISTINGS_AGAIN_AT_ONCE = 4;
const LISTING_AGAIN_WAIT_MS = 1_000;

/**
 * The most listings asked again that held a slot as another was asked for: asked before it, not answered yet, and asked
 * less than the wait before it (less 50 ms, the timers' leeway). A slot its listing handed on at the wait is not held.
 */
function mostHoldingSlots(asked: typeof listings.asked): number {
  const holding = asked.map(({ asked: at }, index) => asked.slice(0, index)
    .filter(before => (before.answered === undefined || before.answered > at) && at - before.asked < LISTING_AGAIN_WAIT_MS - 50).length);
  return Math.max(0, ...holding);
}

/** Resolves once the next file handle opened at `path` is closed: a read of the file has ended. */
function readEnded(path: string): Promise<void> {
  const ended = Promise.withResolvers<void>();
  closedReads.set(path, [...(closedReads.get(path) ?? []), ended.resolve]);
  return ended.promise;
}

/** Resolves once `promise` has, or after `ms` whatever it does. */
async function atMost(promise: Promise<unknown>, ms: number): Promise<void> {
  let timer: NodeJS.Timeout | undefined;
  await Promise.race([promise, new Promise(resolve => { timer = setTimeout(resolve, ms); })]);
  clearTimeout(timer);
}

function createBridge(...args: ConstructorParameters<typeof CodeRuleBridge>): CodeRuleBridge {
  const bridge = new CodeRuleBridge(...args);
  bridges.push(bridge);
  return bridge;
}

/** Starts a checkpoint that blocks on `gate`; `pass` resolves once the gate is released and the walk is idle. */
const passes: Array<Promise<unknown>> = [];
async function requestDiscovery(
  bridge: CodeRuleBridge,
  gate: DiscoveryGate,
  checkpoint: "turn" | "final" = "turn",
): Promise<{ pass: Promise<CodeRuleResponse> }> {
  const pass = bridge.checkpoint(checkpoint);
  passes.push(pass.catch(() => undefined));
  await gate.entered;
  expect(gate.started).toBe(true);
  return { pass };
}
async function settled<T>(promise: Promise<T>): Promise<boolean> {
  return await Promise.race([promise.then(() => true, () => true), new Promise<boolean>(resolve => setImmediate(() => resolve(false)))]);
}

const directories: string[] = [];
afterEach(async () => {
  // A test that timed out never reached its own `finally`: no Git hangs in the tests after it, and no timer stays held.
  vi.useRealTimers();
  asyncGitCalls.hanging = undefined;
  asyncGitCalls.throwsNext = undefined;
  listings.late.length = 0;
  listings.lateAgain.length = 0;
  listings.held = undefined;
  listings.next.length = 0;
  listings.delayMs = 0;
  listings.asked.length = 0;
  listings.calls.length = 0;
  closedReads.clear();
  releases.splice(0).forEach(release => release());
  discovery.reads.clear();
  discovery.directories.clear();
  discovery.opens.clear();
  heldGit.reads = 0;
  readBuffers.watching = false;
  readBuffers.seen.clear();
  await Promise.all(passes.splice(0));
  await Promise.all(bridges.splice(0).map(bridge => bridge.close()));
  await Promise.all(directories.splice(0).map(async path => await removeFolder(path)));
});

async function workspace(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-policy-bridge-"));
  directories.push(root);
  await mkdir(join(root, "src"));
  return await realpath(root);
}
/**
 * Linux stamps files from a per-tick clock, so files written just before a session open can share its tick.
 * Waits until the filesystem clock has passed every stamp under `paths` so later writes are unambiguous.
 */
async function awaitClockAfter(...paths: string[]): Promise<void> {
  const stamps = await Promise.all(paths.map(async path => { const info = await lstat(path); return Math.max(info.mtimeMs, info.ctimeMs); }));
  const since = Math.max(...stamps);
  const probe = await mkdtemp(join(tmpdir(), "rerail-policy-clock-"));
  directories.push(probe);
  for (;;) {
    await writeFile(join(probe, "tick"), "");
    const info = await lstat(join(probe, "tick"));
    if (Math.max(info.mtimeMs, info.ctimeMs) > since) return;
  }
}
setFlagsFromString("--expose-gc");
const gc = runInNewContext("gc") as () => void;
/** Heap bytes in use once full collections have run and their sweeping has finished. */
async function heapInUse(): Promise<number> {
  for (let pass = 0; pass < 3; pass += 1) {
    gc();
    await new Promise(resolve => setTimeout(resolve, 10));
  }
  return process.memoryUsage().heapUsed;
}
function auditor(auditGeneratedEdits: CodeRuleAuditor["auditGeneratedEdits"]): CodeRuleAuditor {
  return { openWorkspace: vi.fn(async () => undefined), acceptsPath: () => true, auditGeneratedEdits, close: vi.fn(async () => undefined) };
}
function response(batch: CodeRuleEditBatch) {
  return { revision: batch.revision, disposition: "continue" as const, violations: [], failures: [] };
}
/**
 * The 8.3 short name Windows gave the file or folder `name` in `folder`, or undefined when the volume makes none:
 * `cmd`'s `%~snxI` then gives the long name back.
 */
function windowsShortName(folder: string, name: string): string | undefined {
  const short = execFileSync("cmd", ["/d", "/c", `for %I in (${name}) do @echo %~snxI`], { cwd: folder, encoding: "utf8" }).trim();
  return short === "" || short.toLowerCase() === name.toLowerCase() ? undefined : short;
}
/** Each captured preimage as the session ledger records it (`ledgerPreimage`). */
function ledgerForms(captured: Readonly<Record<string, Preimage>>): Record<string, Preimage> {
  return Object.fromEntries(Object.entries(captured).map(([path, preimage]) => [path, ledgerPreimage(preimage)]));
}

describe("CodeRuleBridge", () => {
  it("extracts the paths an edit tool names, any file, leaving out multi-line strings", () => {
    expect(generatedEditPaths("write", { path: "src/a.py", content: "pass" })).toEqual(["src/a.py"]);
    expect(generatedEditPaths("edit", { patch: "[src/a.ts#A1B2]\nPUT 1.=1:\n+pass\n" })).toEqual(["src/a.ts"]);
    expect(generatedEditPaths("ast_edit", { paths: ["src/a.rs", "docs/policy.md"] })).toEqual(["src/a.rs", "docs/policy.md"]);
    expect(generatedEditPaths("write", { path: "Justfile", content: "build:\n" })).toEqual(["Justfile"]);
    expect(generatedEditPaths("apply_patch", { input: "patch body\nwith no file header\n" })).toEqual([]);
    expect(generatedEditPaths("read", { path: "src/a.py" })).toEqual([]);
  });

  it("recognises command tools across hosts", () => {
    for (const tool of ["Bash", "shell", "local_shell", "exec_command", "run_terminal_cmd", "container.exec"]) expect(isCommandTool(tool)).toBe(true);
    for (const tool of ["read", "Read", "write", "edit", "apply_patch", "Grep", "Glob", "WebFetch"]) expect(isCommandTool(tool)).toBe(false);
  });

  it("captures and completes an edit while a repository audit remains pending", async () => {
    const root = await workspace();
    const path = join(root, "src", "a.py");
    await writeFile(path, "before\n");
    const entered = Promise.withResolvers<void>();
    const release = Promise.withResolvers<readonly string[]>();
    releases.push(() => release.resolve(["audit-run"]));
    const bridge = createBridge(root, {
      ...auditor(async (_sessionId, batch) => response(batch)),
      audit: async () => {
        entered.resolve();
        return await release.promise;
      },
    });
    await bridge.openWorkspace("session-1");
    let auditCompleted = false;
    const audit = bridge.audit({ scope: "repository", mode: "log" }).then(result => {
      auditCompleted = true;
      return result;
    });
    await entered.promise;
    const capture = bridge.begin("edit-1", "write", { path: "src/a.py" });
    try {
      await expect(capture).resolves.toEqual({ "src/a.py": { text: "before\n" } });
      await writeFile(path, "after\n");
      await expect(bridge.finish("edit-1")).resolves.toEqual([
        expect.objectContaining({ revision: 1, disposition: "continue" }),
      ]);
      expect(auditCompleted).toBe(false);
    } finally {
      release.resolve(["audit-run"]);
      await capture;
      await audit;
    }
  });

  it("captures only the digest and size of a secret-like file as its preimage, never its text", async () => {
    const root = await workspace();
    const text = "TOKEN = 'sk-live-0123456789'\n";
    await writeFile(join(root, "src", "secrets.py"), text);
    await writeFile(join(root, "src", "a.py"), "before\n");
    const bridge = createBridge(root, auditor(async (_sessionId, batch) => response(batch)));
    await bridge.openWorkspace("session-1");
    await expect(bridge.begin("edit-1", "write", { path: ["src/secrets.py", "src/a.py"] })).resolves.toEqual({
      "src/secrets.py": { withheld: { sha256: createHash("sha256").update(text).digest("hex"), bytes: Buffer.byteLength(text) } },
      "src/a.py": { text: "before\n" },
    });
  });

  // A link to a folder named like one holding secrets leads to the same files: `conf/config.py`, with `conf` a link (a
  // junction on Windows) to `.aws`, is `.aws/config.py`, and so is `src/../conf/config.py`. The session ledger in the
  // checkout holds only the file's digest, as for a direct edit, and never its text; the session keeps the text, so
  // `rewind` can still put the file back. No secret pattern matches the text, so only where the file is withholds it.
  it("records only the digest of a file a link to a secret-like folder leads to, while the session keeps its text", async () => {
    const root = await workspace();
    const text = "REGION = 'eu-west-1'\n";
    expect(ledgerPreimage({ text })).toEqual({ text });
    await mkdir(join(root, ".aws"));
    await writeFile(join(root, ".aws", "config.py"), text);
    linkFolder(".aws", join(root, "conf"));
    linkFolder(join("..", ".aws"), join(root, "src", "keys"));
    const bridge = createBridge(root, auditor(async (_sessionId, batch) => response(batch)));
    await bridge.openWorkspace("session-1");
    const captured = await bridge.begin("edit-1", "write", { path: ["conf/config.py", "src/keys/config.py", "src/../conf/config.py"] });
    const withheld = { withheld: { sha256: createHash("sha256").update(text).digest("hex"), bytes: Buffer.byteLength(text) } };
    expect(ledgerForms(captured)).toEqual({ "conf/config.py": withheld, "src/keys/config.py": withheld });
    expect(Object.values(captured).map(preimage => preimage.text)).toEqual([text, text]);
  });

  // A spelling named like one holding secrets keeps only the digest wherever it leads: `.aws/config.py`, with `.aws` a
  // link (a junction on Windows) to an ordinary folder, is still judged by its spelling.
  it("captures only the digest of a file spelled through a folder named like one holding secrets that links to an ordinary one", async () => {
    const root = await workspace();
    const text = "REGION = 'eu-west-1'\n";
    await mkdir(join(root, "plain"));
    await writeFile(join(root, "plain", "config.py"), text);
    linkFolder("plain", join(root, ".aws"));
    const bridge = createBridge(root, auditor(async (_sessionId, batch) => response(batch)));
    await bridge.openWorkspace("session-1");
    await expect(bridge.begin("edit-1", "write", { path: ".aws/config.py" })).resolves.toEqual({
      ".aws/config.py": { withheld: { sha256: createHash("sha256").update(text).digest("hex"), bytes: Buffer.byteLength(text) } },
    });
  });

  // Where a file really is is judged from the checkout that owns it, as its spelling is: a checkout that itself sits in a
  // folder named like one holding secrets (`.docker/app`) keeps the text of its ordinary files.
  it("captures the text of an ordinary file in a checkout that sits in a folder named like one holding secrets", async () => {
    const root = join(await workspace(), ".docker", "app");
    await mkdir(join(root, "src"), { recursive: true });
    await writeFile(join(root, "src", "a.py"), "x = 1\n");
    const bridge = createBridge(root, auditor(async (_sessionId, batch) => response(batch)));
    await bridge.openWorkspace("session-1");
    await expect(bridge.begin("edit-1", "write", { path: "src/a.py" })).resolves.toEqual({ "src/a.py": { text: "x = 1\n" } });
  });

  // Windows opens a folder by its 8.3 short name too (`AWS~1` for `.aws`), which no name check knows.
  it("records only the digest of a file in a secret-like folder named by its Windows short name", async ({ skip }) => {
    skip(process.platform !== "win32", "only Windows gives a folder a short name");
    const root = await workspace();
    const text = "REGION = 'eu-west-1'\n";
    expect(ledgerPreimage({ text })).toEqual({ text });
    await mkdir(join(root, ".aws"));
    await writeFile(join(root, ".aws", "config.py"), text);
    const short = windowsShortName(root, ".aws");
    if (short === undefined) return skip("this volume makes no short names");
    const bridge = createBridge(root, auditor(async (_sessionId, batch) => response(batch)));
    await bridge.openWorkspace("session-1");
    const captured = await bridge.begin("edit-1", "write", { path: `${short}/config.py` });
    expect(ledgerForms(captured)).toEqual({
      [`${short}/config.py`]: { withheld: { sha256: createHash("sha256").update(text).digest("hex"), bytes: Buffer.byteLength(text) } },
    });
  });

  // A file named like one holding secrets has a short name of its own (`WP-CON~1.PHP` for `wp-config.php`), which no
  // name check knows either, with no link and no secret-like folder on the way.
  it("records only the digest of a file named like one holding secrets, opened by its Windows short name", async ({ skip }) => {
    skip(process.platform !== "win32", "only Windows gives a file a short name");
    const root = await workspace();
    const text = "<?php $region = 'eu-west-1';\n";
    expect(ledgerPreimage({ text })).toEqual({ text });
    await writeFile(join(root, "src", "wp-config.php"), text);
    const short = windowsShortName(join(root, "src"), "wp-config.php");
    if (short === undefined) return skip("this volume makes no short names");
    const bridge = createBridge(root, auditor(async (_sessionId, batch) => response(batch)));
    await bridge.openWorkspace("session-1");
    const captured = await bridge.begin("edit-1", "write", { path: `src/${short}` });
    expect(ledgerForms(captured)).toEqual({
      [`src/${short}`]: { withheld: { sha256: createHash("sha256").update(text).digest("hex"), bytes: Buffer.byteLength(text) } },
    });
  });

  it("opens and checks tool edits while a stale baseline read is stalled", async () => {
    const root = await workspace();
    const path = join(root, "src", "a.py");
    await writeFile(path, "before\n");
    await writeFile(join(root, "src", "untouched.py"), "existing\n");
    await awaitClockAfter(path, join(root, "src", "untouched.py"));
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits));
    // Hold bytes already read from disk, not the tool's later preimage/postimage reads.
    const baseline = holdDiscovery("reads", path);
    let opened = false;
    const opening = bridge.openWorkspace("session-1").then(() => { opened = true; });
    await baseline.entered;
    await vi.waitFor(() => expect(opened).toBe(true));
    await opening;

    await expect(bridge.begin("edit-1", "write", { path: "src/a.py" })).resolves.toEqual({
      "src/a.py": { text: "before\n" },
    });
    await writeFile(path, "tool edit\n");
    await expect(bridge.finish("edit-1")).resolves.toEqual([response({ revision: 1, checkpoints: [], edits: [] })]);
    expect(auditGeneratedEdits.mock.calls).toEqual([["session-1", {
      revision: 1,
      checkpoints: ["edit", "function"],
      edits: [{ path: "src/a.py", language: "python", previousText: "before\n", generatedText: "tool edit\n" }],
    }]]);

    // A turn checkpoint waits for the walk instead of reporting partial coverage.
    const turn = bridge.checkpoint("turn");
    passes.push(turn.catch(() => undefined));
    expect(await settled(turn)).toBe(false);
    baseline.release();
    await expect(turn).resolves.toEqual({ revision: 1, disposition: "continue", violations: [], failures: [] });
    // A complete reconciliation must neither replay stale baseline bytes nor emit untouched code.
    expect(auditGeneratedEdits).toHaveBeenCalledTimes(1);

    await writeFile(path, "external edit\n");
    await bridge.checkpoint("final");
    expect(auditGeneratedEdits.mock.calls).toEqual([
      ["session-1", {
        revision: 1,
        checkpoints: ["edit", "function"],
        edits: [{ path: "src/a.py", language: "python", previousText: "before\n", generatedText: "tool edit\n" }],
      }],
      // Made while no call of the session ran, the external edit is another writer's: the evaluator, holding the file as
      // the tool edit left it, is told in Log, with no generated lines.
      ["session-1", {
        revision: 2,
        checkpoints: ["final"],
        mode: "log",
        edits: [{ path: "src/a.py", language: "python", previousText: "external edit\n", generatedText: "external edit\n" }],
      }],
    ]);
  });

  it("reconciles shell writes at a command tool boundary before its result returns", async () => {
    const root = await workspace();
    const path = join(root, "src", "a.py");
    await writeFile(path, "transaction.fetch_value('SELECT 1')\n");
    await awaitClockAfter(path);
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits));
    await bridge.openWorkspace("session-1");
    await bridge.checkpoint("turn");
    expect(auditGeneratedEdits).not.toHaveBeenCalled();

    await expect(bridge.begin("shell-1", "shell", { command: "python3 -c \"...write_text(...)\"" })).resolves.toEqual({});
    await writeFile(path, "cursor.execute('SELECT 1')\n");
    await expect(bridge.finish("shell-1")).resolves.toEqual([expect.objectContaining({ revision: 1, disposition: "continue" })]);
    expect(auditGeneratedEdits.mock.calls).toEqual([["session-1", {
      revision: 1,
      checkpoints: ["edit", "function"],
      edits: [{
        path: "src/a.py", language: "python",
        previousReadError: "Source changed outside a tool with a captured preimage", generatedText: "cursor.execute('SELECT 1')\n",
      }],
    }]]);

    // A read-only command and a read tool find nothing new.
    await bridge.begin("shell-2", "shell", { command: "sed -n '1,200p' src/a.py" });
    await expect(bridge.finish("shell-2")).resolves.toEqual([]);
    await bridge.begin("read-1", "read", { path: "src/a.py" });
    await expect(bridge.finish("read-1")).resolves.toEqual([]);
    expect(auditGeneratedEdits).toHaveBeenCalledTimes(1);

    // A failing command may still have written before it failed.
    await bridge.begin("shell-3", "Bash", { command: "python3 write_then_crash.py" });
    await writeFile(path, "cursor.execute('SELECT 2')\n");
    await expect(bridge.finish("shell-3", true)).resolves.toEqual([expect.objectContaining({ revision: 2 })]);
    expect(auditGeneratedEdits).toHaveBeenLastCalledWith("session-1", expect.objectContaining({
      revision: 2,
      edits: [expect.objectContaining({ path: "src/a.py", generatedText: "cursor.execute('SELECT 2')\n" })],
    }));
  });

  it("gives a shell write the preimage the session or Git holds, including its first sight of a file Git does not hold", async () => {
    const root = await workspace();
    const clean = join(root, "src", "clean.py");
    const dirty = join(root, "src", "dirty.py");
    await writeFile(clean, "one\n");
    await writeFile(dirty, "committed\n");
    commitAll(root);
    // Changed before the session and never staged: neither the index nor HEAD holds what the session first saw.
    await writeFile(dirty, "uncommitted\n");
    await awaitClockAfter(clean, dirty);
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits));
    await bridge.openWorkspace("session-1");
    await bridge.checkpoint("turn");

    // The session's first sight of a file Git does not hold is kept, so a shell write to it counts only its own lines.
    await bridge.begin("shell-1", "shell", { command: "./rewrite.sh" });
    await writeFile(clean, "one\ntwo\n");
    await writeFile(dirty, "uncommitted\nmore\n");
    await bridge.finish("shell-1");
    expect(auditGeneratedEdits.mock.calls.at(-1)?.[1].edits).toEqual([
      { path: "src/clean.py", language: "python", previousText: "one\n", generatedText: "one\ntwo\n" },
      { path: "src/dirty.py", language: "python", previousText: "uncommitted\n", generatedText: "uncommitted\nmore\n" },
    ]);

    // The session saw what the first write made of the file, so the next write knows its preimage.
    await bridge.begin("shell-2", "shell", { command: "./rewrite.sh" });
    await writeFile(dirty, "uncommitted\nmore\nagain\n");
    await bridge.finish("shell-2");
    expect(auditGeneratedEdits.mock.calls.at(-1)?.[1].edits).toEqual([
      { path: "src/dirty.py", language: "python", previousText: "uncommitted\nmore\n", generatedText: "uncommitted\nmore\nagain\n" },
    ]);
  });

  it("gives a shell write to a file untracked before the session the text the session first saw", async () => {
    const root = await workspace();
    const untracked = join(root, "src", "untracked.py");
    await writeFile(join(root, "src", "kept.py"), "kept\n");
    commitAll(root);
    await writeFile(untracked, "draft\n");
    await awaitClockAfter(untracked);
    const { bridge, auditGeneratedEdits } = await opened(root);
    await shell(bridge, () => writeFile(untracked, "draft\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/untracked.py", language: "python", previousText: "draft\n", generatedText: "draft\nmine\n" }],
    ]);
  });

  it("keeps first-sight text only within its budget, and measures a shell edit past it as before", async () => {
    const root = await workspace();
    const [first, second] = [join(root, "src", "a.py"), join(root, "src", "b.py")];
    await writeFile(first, "old\n");
    await writeFile(second, "old\n");
    commitAll(root);
    await writeFile(first, "aaaa\n");
    await writeFile(second, "bbbb\n");
    await awaitClockAfter(first, second);
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits), { firstSightTextBytes: 5 });
    await bridge.openWorkspace("session-1");
    await bridge.checkpoint("turn");
    await shell(bridge, async () => {
      await writeFile(first, "aaaa\nmine\n");
      await writeFile(second, "bbbb\nmine\n");
    });
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([[
      { path: "src/a.py", language: "python", previousText: "aaaa\n", generatedText: "aaaa\nmine\n" },
      {
        path: "src/b.py", language: "python",
        previousReadError: "Source changed outside a tool with a captured preimage", generatedText: "bbbb\nmine\n",
      },
    ]]);
  });

  it("measures a shell edit from the text a later walk first saw in a folder the baseline could not read", async () => {
    const root = await realpath(await workspace());
    const draft = join(root, "deep", "draft.py");
    await writeFile(join(root, "src", "kept.py"), "kept\n");
    commitAll(root);
    await mkdir(join(root, "deep"));
    await writeFile(draft, "draft\n");
    await awaitClockAfter(draft);
    discovery.directories.set(join(root, "deep"), [async () => { throw new Error("EMFILE: too many open files"); }]);
    const { bridge, auditGeneratedEdits } = await opened(root);
    // The next walk reads the folder: its first sight of the file.
    await bridge.checkpoint("turn");
    await shell(bridge, () => writeFile(draft, "draft\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "deep/draft.py", language: "python", previousText: "draft\n", generatedText: "draft\nmine\n" }],
    ]);
  });

  it("makes no text of the files a later walk first sees, as its baseline, in a folder the baseline could not read", async () => {
    const root = await realpath(await workspace());
    await mkdir(join(root, "deep"));
    const files = Array.from({ length: 20 }, (_, index) => join(root, "deep", `f${index}.py`));
    await Promise.all(files.map((file, index) => writeFile(file, `x = ${index}\n`)));
    commitAll(root);
    await awaitClockAfter(...files);
    discovery.directories.set(join(root, "deep"), [async () => { throw new Error("EMFILE: too many open files"); }]);
    decodedReads.count = 0;
    const { bridge, auditGeneratedEdits } = await opened(root);
    // A later walk reads the folder: its first sight of every file there, none of which can be an edit.
    await bridge.checkpoint("turn");
    expect(decodedReads.count).toBe(0);
    await shell(bridge, () => writeFile(files[0]!, "x = 0\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "deep/f0.py", language: "python", previousText: "x = 0\n", generatedText: "x = 0\nmine\n" }],
    ]);
  });

  it("audits a shell command's writes that its own walk is the first to see, in a folder the baseline could not read", async () => {
    const root = await realpath(await workspace());
    await writeFile(join(root, "src", "kept.py"), "kept\n");
    await mkdir(join(root, "deep"));
    await writeFile(join(root, "deep", "old.py"), "old\n");
    commitAll(root);
    await awaitClockAfter(join(root, "deep", "old.py"));
    // Every walk the session's opening makes fails to list the folder; the command's walk is the first to read it.
    const failing = async () => { throw new Error("EMFILE: too many open files"); };
    discovery.directories.set(join(root, "deep"), Array.from({ length: 20 }, () => failing));
    const { bridge, auditGeneratedEdits } = await opened(root);
    discovery.directories.delete(join(root, "deep"));
    await shell(bridge, async () => {
      await writeFile(join(root, "deep", "old.py"), "old\nviolation\n");
      await writeFile(join(root, "deep", "new.py"), "violation\n");
    });
    const uncaptured = "Source changed during initial discovery without a captured preimage";
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => [batch.edits, batch.mode])).toEqual([[[
      { path: "deep/new.py", language: "python", previousReadError: uncaptured, generatedText: "violation\n" },
      { path: "deep/old.py", language: "python", previousReadError: uncaptured, generatedText: "old\nviolation\n" },
    ], undefined]]);
  });

  it("measures a shell edit from the text of a file a command moved into the session folder, stamped before the session", async () => {
    const root = await workspace();
    await writeFile(join(root, "src", "a.py"), "a\n");
    commitAll(root);
    const outside = await workspace();
    await mkdir(join(outside, "pkg"));
    const moved = join(outside, "pkg", "m.py");
    await writeFile(moved, "moved\n");
    await awaitClockAfter(join(root, "src", "a.py"), moved);
    const { bridge, auditGeneratedEdits } = await opened(root);
    // A move keeps the stamps of the files in the folder, so the walk first sees m.py with a stamp from before the session.
    await shell(bridge, () => rename(join(outside, "pkg"), join(root, "src", "pkg")));
    await shell(bridge, () => writeFile(join(root, "src", "pkg", "m.py"), "moved\nmine\n"));
    // Whether the move itself is audited depends on the stamps the platform gives a moved folder: only the edit is pinned.
    expect(auditGeneratedEdits.mock.calls.at(-1)?.[1].edits).toEqual([
      { path: "src/pkg/m.py", language: "python", previousText: "moved\n", generatedText: "moved\nmine\n" },
    ]);
  });

  it("puts in checkpoint's answer once a checkout whose Git did not list the files changed before the session in time, and lists them again off the walks' path", async () => {
    const root = await workspace();
    const dirty = join(root, "src", "dirty.py");
    await writeFile(dirty, "committed\n");
    commitAll(root);
    await writeFile(dirty, "uncommitted\n");
    await awaitClockAfter(dirty);
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits));
    // The baseline walk's listing runs past its time limit; the listing asked again answers.
    listings.late.push(root);
    await bridge.openWorkspace("session-1");
    await expect(bridge.checkpoint("turn")).resolves.toEqual(expect.objectContaining({
      failures: [expect.objectContaining({ code: "source-incomplete", message: ".: Git did not list the checkout's changed files in time" })],
    }));
    await bridge.settleListingsAgain();
    expect((await bridge.checkpoint("turn")).failures).toEqual([]);
    // The listing asked again kept the session's first sight of the file.
    await shell(bridge, () => writeFile(dirty, "uncommitted\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/dirty.py", language: "python", previousText: "uncommitted\n", generatedText: "uncommitted\nmine\n" }],
    ]);
  });

  it("checks a shell write at the boundary it reaches while Git does not list the files changed before the session in time", async () => {
    const root = await workspace();
    const dirty = join(root, "src", "dirty.py");
    const clean = join(root, "src", "clean.py");
    await writeFile(dirty, "committed\n");
    await writeFile(clean, "committed\n");
    commitAll(root);
    await writeFile(dirty, "uncommitted\n");
    await awaitClockAfter(dirty);
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits));
    // Both listings run past their time limit: the baseline walk's, then the one asked again once that walk is applied,
    // which waits until the shell write has been checked.
    const asked = holdListingsAgain();
    listings.late.push(root);
    listings.lateAgain.push(root);
    await bridge.openWorkspace("session-1");
    expect((await bridge.checkpoint("turn")).failures).toEqual([
      expect.objectContaining({ code: "source-incomplete", message: ".: Git did not list the checkout's changed files in time" }),
    ]);
    // While the listing asked again waits, the walk this shell write asks for checks it within the boundary's wait.
    await shell(bridge, () => writeFile(clean, "committed\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/clean.py", language: "python", previousText: "committed\n", generatedText: "committed\nmine\n" }],
    ]);
    asked.release();
    await bridge.settleListingsAgain();
    // The second late listing enters checkpoint's answer once, and is the last.
    expect((await bridge.checkpoint("turn")).failures).toEqual([expect.objectContaining({
      code: "source-incomplete",
      message: ".: Git did not list the checkout's changed files in time again: a shell edit to a file changed there before the session is checked against Git's text of it, or whole",
    })]);
    expect((await bridge.checkpoint("turn")).failures).toEqual([]);
    expect(listings.calls).toEqual([`walk ${root}`, `again ${root}`]);
  });

  it("puts a checkout whose listing asked again failed in checkpoint's answer once, and its files changed before the session keep no first sight", async () => {
    const root = await workspace();
    const dirty = join(root, "src", "dirty.py");
    await writeFile(dirty, "committed\n");
    commitAll(root);
    await writeFile(dirty, "uncommitted\n");
    await awaitClockAfter(dirty);
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits));
    // The baseline walk's listing runs past its time limit; the one asked again fails, as a Git that exits with an error does.
    listings.late.push(root);
    asyncGitCalls.failing = "--modified";
    const answered: string[] = [];
    try {
      await bridge.openWorkspace("session-1");
      answered.push(...(await bridge.checkpoint("turn")).failures.map(failure => failure.message));
      await bridge.settleListingsAgain();
    } finally {
      asyncGitCalls.failing = undefined;
    }
    answered.push(...(await bridge.checkpoint("turn")).failures.map(failure => failure.message));
    // In one answer: the first boundary's after it, or that one beside the late listing when it failed first.
    expect(answered.flatMap(message => message.split("; ")).filter(message => message.includes("asked again"))).toEqual([
      ".: Git could not list the checkout's changed files when asked again: a shell edit to a file changed there before the session is checked against Git's text of it, or whole",
    ]);
    expect((await bridge.checkpoint("turn")).failures).toEqual([]);
  });

  it("asks Git again at the next late listing of a checkout whose listing asked again threw, and puts the throw in checkpoint's answer once", async () => {
    const root = await workspace();
    const dirty = join(root, "src", "dirty.py");
    const draft = join(root, "deep", "draft.py");
    await writeFile(dirty, "committed\n");
    commitAll(root);
    await writeFile(dirty, "uncommitted\n");
    await mkdir(join(root, "deep"));
    await writeFile(draft, "draft\n");
    await awaitClockAfter(dirty, draft);
    // The baseline walk cannot read `deep`, so the walk after it is the first to see draft.py there and lists it.
    discovery.directories.set(join(root, "deep"), [async () => { throw new Error("EMFILE: too many open files"); }]);
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits), { discoveryWaitMs: 100 });
    // Both walks' listings run past their time limit. The one asked again after the first walk throws as it starts; the
    // one asked again after the second answers.
    listings.late.push(root, root);
    asyncGitCalls.throwsNext = "--modified";
    const secondWalk = holdDiscovery("reads", draft);
    await bridge.openWorkspace("session-1");
    await secondWalk.entered;
    // Taken while the second walk waits at draft.py.
    const afterThrow = await bridge.checkpoint("turn");
    secondWalk.release();
    await vi.waitFor(async () => expect((await bridge.checkpoint("turn")).failures).toEqual([
      expect.objectContaining({ code: "source-incomplete", message: ".: Git did not list the checkout's changed files in time" }),
    ]), { timeout: 10_000, interval: 20 });
    await bridge.settleListingsAgain();
    await shell(bridge, () => writeFile(draft, "draft\nmine\n"));
    // The shell call's boundary waits 100 ms for its walk; a slower walk is audited once it ends.
    await vi.waitFor(() => expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "deep/draft.py", language: "python", previousText: "draft\n", generatedText: "draft\nmine\n" }],
    ]), { timeout: 10_000, interval: 20 });
    expect(afterThrow.failures).toEqual([expect.objectContaining({
      code: "source-incomplete", message: expect.stringContaining(".: Git could not list the checkout's changed files when asked again: "),
    })]);
  });

  it("puts both in checkpoint's answer: a checkout's listing asked again failed, and a later walk's listing ran late, with no boundary between", async () => {
    const root = await workspace();
    const dirty = join(root, "src", "dirty.py");
    const draft = join(root, "deep", "draft.py");
    await writeFile(dirty, "committed\n");
    commitAll(root);
    await writeFile(dirty, "uncommitted\n");
    await mkdir(join(root, "deep"));
    await writeFile(draft, "draft\n");
    await awaitClockAfter(dirty, draft);
    // The baseline walk cannot read `deep`, so the walk after it is the first to see draft.py there and lists it.
    discovery.directories.set(join(root, "deep"), [async () => { throw new Error("EMFILE: too many open files"); }]);
    const bridge = createBridge(root, auditor(vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch))));
    // Both walks' listings run past their time limit. The one asked again after the first fails, as a Git that exits with
    // an error does, while the second walk waits at draft.py.
    listings.late.push(root, root);
    asyncGitCalls.failing = "--modified";
    const secondWalk = holdDiscovery("reads", draft);
    try {
      await bridge.openWorkspace("session-1");
      await secondWalk.entered;
      await bridge.settleListingsAgain();
    } finally {
      asyncGitCalls.failing = undefined;
    }
    secondWalk.release();
    // The first boundary's answer holds what the failure cost the files changed before the session, besides the late listings.
    expect((await bridge.checkpoint("turn")).failures.map(failure => failure.message)).toEqual([
      ".: Git did not list the checkout's changed files in time; .: Git could not list the checkout's changed files when asked again: a shell edit to a file changed there before the session is checked against Git's text of it, or whole",
    ]);
    await bridge.settleListingsAgain();
    expect((await bridge.checkpoint("turn")).failures).toEqual([]);
  });

  it(`asks Git again for the checkouts a walk found late ${LISTINGS_AGAIN_AT_ONCE} at a time, each until it answers, and keeps what it first saw in each`, async () => {
    const { root, checkouts, changed, sourceRoot } = await changedCheckouts(LISTINGS_AGAIN_AT_ONCE + 2);
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, { ...auditor(auditGeneratedEdits), sourceRoot });
    // The baseline walk's listing of each checkout runs past its time limit. Each listing asked again answers 50 ms or
    // more after it was asked: a fast Git, slower than a timer of 0 ms.
    listings.late.push(...checkouts);
    listings.delayMs = 50;
    await bridge.openWorkspace("session-1");
    await bridge.checkpoint("turn");
    await bridge.settleListingsAgain();
    expect(listings.asked).toHaveLength(checkouts.length);
    // The first ones asked filled every slot at once, and no listing was asked while every slot was held: by a listing
    // asked and not answered yet, and asked less than the wait before. This holds however slow Git is.
    expect(mostHoldingSlots(listings.asked)).toBe(LISTINGS_AGAIN_AT_ONCE - 1);
    // Each asked Git in the order the walk found its checkout late: a slot goes to the listing that has waited longest.
    const walked = listings.calls.filter(call => call.startsWith("walk ")).map(call => call.slice("walk ".length));
    expect(listings.asked.map(listing => listing.checkout)).toEqual(walked);
    await shell(bridge, async () => { await Promise.all(changed.map(path => writeFile(path, "uncommitted\nmine\n"))); });
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([firstSightEdits(root, changed)]);
  });

  it("asks Git again for the checkouts behind slow listings asked again once one has run a second in its slot, keeps what it first saw in them, and hands each slot on once", async () => {
    const { root, checkouts, changed, sourceRoot } = await changedCheckouts(2 * LISTINGS_AGAIN_AT_ONCE + 2);
    // Every walk until the last one below fails to list each checkout's `deep`, so that walk is the first to see a file
    // there and lists each checkout again.
    const deep = checkouts.map(checkout => join(checkout, "deep"));
    await Promise.all(deep.map(async folder => {
      await mkdir(folder);
      await writeFile(join(folder, "d.py"), "d\n");
    }));
    await awaitClockAfter(...deep.map(folder => join(folder, "d.py")));
    const failing = async () => { throw new Error("EMFILE: too many open files"); };
    for (const folder of deep) discovery.directories.set(folder, Array.from({ length: 20 }, () => failing));
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, { ...auditor(auditGeneratedEdits), sourceRoot });
    // The baseline walk's listing of each checkout runs past its time limit. The first listings asked again, one in each
    // slot, wait until the end of the test, as Gits that take long do; the six behind them answer 50 ms or more after
    // they ask.
    listings.late.push(...checkouts);
    const slow = Array.from({ length: LISTINGS_AGAIN_AT_ONCE }, () => holdNextListingAgain());
    listings.delayMs = 50;
    await bridge.openWorkspace("session-1");
    await bridge.checkpoint("turn");
    const held = await Promise.all(slow.map(hold => hold.asked));
    const behind = changed.filter(path => !held.includes(sourceRoot(path)));
    // Each listing behind them reads the file the walk saw first there once a slow one has run a second: without that
    // wait, not before a slow one answered.
    await atMost(Promise.all(behind.map(path => readEnded(path))), 5_000);
    await shell(bridge, async () => { await Promise.all(behind.map(path => writeFile(path, "uncommitted\nmine\n"))); });
    const asked = listings.calls.filter(call => call.startsWith("again"));
    // Released before any check, so a failing run leaves no listing for the tests after it.
    for (const hold of slow) hold.release();
    await bridge.settleListingsAgain();
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([firstSightEdits(root, behind)]);
    expect(asked).toHaveLength(checkouts.length);
    // The four that took the slots the slow ones handed on asked Git once the first slot's wait had run out, and well
    // before a wait twice as long would have.
    const heldAt = listings.asked.slice(0, LISTINGS_AGAIN_AT_ONCE).map(listing => listing.asked);
    for (const { asked: at } of listings.asked.slice(LISTINGS_AGAIN_AT_ONCE, 2 * LISTINGS_AGAIN_AT_ONCE)) {
      expect(at - Math.min(...heldAt)).toBeGreaterThanOrEqual(LISTING_AGAIN_WAIT_MS - 50);
      expect(at - Math.max(...heldAt)).toBeLessThan(2 * LISTING_AGAIN_WAIT_MS - 100);
    }
    // Each of those four held its slot from then until its Git answered, its own wait counted from when it took the
    // slot, not from when it was asked for a second before: the last two asked only once one of the four had answered.
    expect(mostHoldingSlots(listings.asked.slice(LISTINGS_AGAIN_AT_ONCE))).toBe(LISTINGS_AGAIN_AT_ONCE - 1);
    // Each slow listing handed its slot on once, at the wait, and each behind once, at its answer: a later late walk
    // finds the four slots free again, and no more.
    const before = listings.asked.length;
    listings.late.push(...checkouts);
    for (const folder of deep) discovery.directories.delete(folder);
    await bridge.checkpoint("turn");
    await bridge.settleListingsAgain();
    expect(listings.asked).toHaveLength(before + checkouts.length);
    expect(mostHoldingSlots(listings.asked.slice(before))).toBe(LISTINGS_AGAIN_AT_ONCE - 1);
  }, 30_000);

  it("hands the slot of a slow listing asked again on once it has held it a second, and not before", async () => {
    const { root, checkouts, sourceRoot } = await changedCheckouts(LISTINGS_AGAIN_AT_ONCE + 1);
    const bridge = createBridge(root, { ...auditor(vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch))), sourceRoot });
    // The baseline walk's listing of each checkout runs past its time limit. The first listings asked again, one in each
    // slot, wait until the end of the test, as Gits that take long do; the last waits for a slot. Only the test moves the
    // faked timers the slots' waits run on, so a slow machine cannot move them past the second.
    listings.late.push(...checkouts);
    const slow = Array.from({ length: LISTINGS_AGAIN_AT_ONCE }, () => holdNextListingAgain());
    const askedGit = (): number => listings.calls.filter(call => call.startsWith("again ")).length;
    // Resolves at the next turn of the event loop, once what the timers just settled has run its callbacks.
    const nextTurn = async (): Promise<void> => { await new Promise(resolve => setImmediate(resolve)); };
    const asked: number[] = [];
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      await bridge.openWorkspace("session-1");
      await bridge.checkpoint("turn");
      await Promise.all(slow.map(hold => hold.asked));
      vi.advanceTimersByTime(LISTING_AGAIN_WAIT_MS - 1);
      await nextTurn();
      asked.push(askedGit());
      vi.advanceTimersByTime(1);
      await nextTurn();
      asked.push(askedGit());
    } finally {
      vi.useRealTimers();
    }
    for (const hold of slow) hold.release();
    await bridge.settleListingsAgain();
    // A millisecond before the second, the last still waited for a slot; at the second it took one and asked Git.
    expect(asked).toEqual([LISTINGS_AGAIN_AT_ONCE, LISTINGS_AGAIN_AT_ONCE + 1]);
  });

  it("asks Git for the next checkout once a listing asked again has answered, while an audit holds the bridge's queue", async () => {
    const { root, checkouts, sourceRoot } = await changedCheckouts(LISTINGS_AGAIN_AT_ONCE + 1);
    const audited = Promise.withResolvers<void>();
    const audit = Promise.withResolvers<void>();
    releases.push(audit.resolve);
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => {
      audited.resolve();
      await audit.promise;
      return response(batch);
    });
    const bridge = createBridge(root, { ...auditor(auditGeneratedEdits), sourceRoot });
    // The baseline walk's listing of each checkout runs past its time limit. The listings asked again, one in each slot
    // and one waiting for a slot, wait until an edit's audit holds the bridge's queue. Until then no slot's wait runs
    // out, so only a Git's answer can hand a slot on, however long the edit and its audit take.
    listings.late.push(...checkouts);
    const asking = holdListingsAgain();
    const { edited } = await withoutSlotWaits(async () => {
      await bridge.openWorkspace("session-1");
      await bridge.checkpoint("turn");
      await bridge.begin("edit-1", "write", { path: "src/held.py" });
      await writeFile(join(root, "src", "held.py"), "held\n");
      const finishing = bridge.finish("edit-1");
      await audited.promise;
      expect(listings.asked).toHaveLength(LISTINGS_AGAIN_AT_ONCE);
      asking.release();
      return { edited: finishing };
    });
    // Those in a slot answer, read what they list and wait for the queue to keep it; the last asks Git once one answered.
    await vi.waitFor(() => expect(listings.asked.length).toBeGreaterThan(LISTINGS_AGAIN_AT_ONCE), { timeout: 5_000, interval: 5 }).catch(() => undefined);
    const releasedAt = performance.now();
    audit.resolve();
    await edited;
    await bridge.settleListingsAgain();
    const answered = Math.min(...listings.asked.slice(0, LISTINGS_AGAIN_AT_ONCE).map(listing => listing.answered ?? Infinity));
    expect(listings.asked).toHaveLength(checkouts.length);
    // Asked while the audit still held the queue, as soon as a slot's Git answered, not once the reads and the keep of
    // the listing in that slot were done.
    expect(listings.asked[LISTINGS_AGAIN_AT_ONCE]?.asked).toBeLessThan(releasedAt);
    expect(listings.asked[LISTINGS_AGAIN_AT_ONCE]?.asked).toBeLessThanOrEqual(answered + 100);
  }, 30_000);

  it("asks Git nothing for the listings asked again still waiting for a slot once the session closes, while an audit holds the bridge's queue", async () => {
    // Four listings in the slots and six behind them. No slot's wait runs out before the session closes, so the six
    // still wait for a slot as it closes, however long the edit and its audit take.
    const { root, checkouts, sourceRoot } = await changedCheckouts(2 * LISTINGS_AGAIN_AT_ONCE + 2);
    const audited = Promise.withResolvers<void>();
    const audit = Promise.withResolvers<void>();
    releases.push(audit.resolve);
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => {
      audited.resolve();
      await audit.promise;
      return response(batch);
    });
    const bridge = createBridge(root, { ...auditor(auditGeneratedEdits), sourceRoot });
    // The baseline walk's listing of each checkout runs past its time limit. The four listings in the slots wait until the
    // session is closing; each listing asked again answers as a Git its time limit ended, starting none.
    listings.late.push(...checkouts);
    listings.lateAgain.push(...checkouts);
    const slow = Array.from({ length: LISTINGS_AGAIN_AT_ONCE }, () => holdNextListingAgain());
    const { edited, closed, askedAfter } = await withoutSlotWaits(async () => {
      await bridge.openWorkspace("session-1");
      await bridge.checkpoint("turn");
      await Promise.all(slow.map(hold => hold.asked));
      await bridge.begin("edit-1", "write", { path: "src/held.py" });
      await writeFile(join(root, "src", "held.py"), "held\n");
      const finishing = bridge.finish("edit-1");
      await audited.promise;
      // Only the four in the slots have asked Git: the six behind them still wait for a slot.
      expect(listings.calls.filter(call => call.startsWith("again "))).toHaveLength(LISTINGS_AGAIN_AT_ONCE);
      // The close cancels the session at once, then waits for the audit to leave the bridge's queue before it stops the
      // session's Git reads. Meanwhile the listings in the slots answer and hand their slots on.
      const askedBefore = listings.calls.length;
      const closing = bridge.close();
      for (const hold of slow) hold.release();
      await bridge.settleListingsAgain();
      return { edited: finishing, closed: closing, askedAfter: listings.calls.slice(askedBefore) };
    });
    audit.resolve();
    await edited;
    await closed;
    expect(askedAfter).toEqual([]);
  });

  it("keeps the first sight of a file a later walk saw first, whose listing ran late while the one asked again waited", async () => {
    const root = await workspace();
    const dirty = join(root, "src", "dirty.py");
    const draft = join(root, "deep", "draft.py");
    await writeFile(dirty, "committed\n");
    commitAll(root);
    await writeFile(dirty, "uncommitted\n");
    await mkdir(join(root, "deep"));
    await writeFile(draft, "draft\n");
    await awaitClockAfter(dirty, draft);
    // The baseline walk cannot read `deep`, so the walk after it is the first to see draft.py there.
    discovery.directories.set(join(root, "deep"), [async () => { throw new Error("EMFILE: too many open files"); }]);
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits));
    // Both walks' listings run past their time limit, and the listing asked again after the first waits until both walks
    // are applied: the second walk hands it draft.py rather than asking Git again.
    const asked = holdListingsAgain();
    listings.late.push(root, root);
    await bridge.openWorkspace("session-1");
    await bridge.checkpoint("turn");
    asked.release();
    await bridge.settleListingsAgain();
    expect(listings.calls).toEqual([`walk ${root}`, `again ${root}`, `walk ${root}`]);
    await shell(bridge, () => writeFile(draft, "draft\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "deep/draft.py", language: "python", previousText: "draft\n", generatedText: "draft\nmine\n" }],
    ]);
  });

  it("closes with no Git a listing asked again started still running in the folder", async () => {
    const root = await workspace();
    const dirty = join(root, "src", "dirty.py");
    await writeFile(dirty, "committed\n");
    commitAll(root);
    await writeFile(dirty, "uncommitted\n");
    await awaitClockAfter(dirty);
    const bridge = new CodeRuleBridge(root, auditor(vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch))));
    // The baseline walk's listing runs past its time limit; the one asked again starts a Git that never finishes.
    listings.late.push(root);
    asyncGitCalls.hanging = ["--modified"];
    await bridge.openWorkspace("session-1");
    await bridge.checkpoint("turn");
    await vi.waitFor(() => expect(runningIn(root).filter(entry => entry.includes("--batch"))).toHaveLength(1), { timeout: 10_000 });
    await bridge.close();

    expect(runningIn(root)).toEqual([]);
    // One try: on Windows a process still running in the folder keeps it from being removed.
    await removeFolder(root, { retry: false });
  });

  it("audits nothing Git wrote over a file changed before the session, and measures the next shell edit from Git's text", async () => {
    const root = await workspace();
    const dirty = join(root, "src", "dirty.py");
    await writeFile(dirty, "committed\n");
    commitAll(root);
    await writeFile(dirty, "uncommitted\n");
    await awaitClockAfter(dirty);
    const { bridge, auditGeneratedEdits } = await opened(root);
    // The session's first sight of the file is no change of its own, and no longer the file's text once Git wrote over it.
    await shell(bridge, () => git(root, "stash", "--quiet"));
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
    await shell(bridge, () => writeFile(dirty, "committed\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/dirty.py", language: "python", previousText: "committed\n", generatedText: "committed\nmine\n" }],
    ]);
  });

  it("closes and reopens without waiting for an abandoned baseline read", async () => {
    const root = await workspace();
    const path = join(root, "src", "a.py");
    await writeFile(path, "old session baseline\n");
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const engine = auditor(auditGeneratedEdits);
    const bridge = createBridge(root, engine);
    const abandoned = holdDiscovery("reads", path);
    let opened = false;
    const opening = bridge.openWorkspace("old-session").then(() => { opened = true; });
    await abandoned.entered;
    await vi.waitFor(() => expect(opened).toBe(true));
    await opening;
    await bridge.close();
    await writeFile(path, "replacement baseline\n");
    const replacement = holdDiscovery("reads", path);
    await bridge.openWorkspace("replacement-session");
    await replacement.entered;
    await bridge.begin("replacement-edit", "write", { path: "src/a.py" });
    await writeFile(path, "replacement edit\n");
    await bridge.finish("replacement-edit");

    abandoned.release();
    replacement.release();
    await bridge.checkpoint("turn");
    expect(auditGeneratedEdits.mock.calls).toEqual([["replacement-session", {
      revision: 1,
      checkpoints: ["edit", "function"],
      edits: [{
        path: "src/a.py", language: "python",
        previousText: "replacement baseline\n", generatedText: "replacement edit\n",
      }],
    }]]);
    expect(engine.close).toHaveBeenCalledTimes(1);
  });

  it("reports a command's uncaptured shell writes made before baseline discovery reaches their files", async () => {
    const root = await workspace();
    await writeFile(join(root, "src", "a.py"), "existing before startup\n");
    await writeFile(join(root, "src", "untouched.py"), "existing unchanged\n");
    await awaitClockAfter(join(root, "src", "a.py"), join(root, "src", "untouched.py"));
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits));
    const baseline = holdDiscovery("directories", root);
    await bridge.openWorkspace("session-1");
    await baseline.entered;
    await bridge.begin("shell-1", "Bash", { command: "./replace.sh" });
    await writeFile(join(root, "src", "a.py"), "external replacement\n");
    await writeFile(join(root, "src", "new.py"), "external creation\n");
    const final = bridge.checkpoint("final");
    passes.push(final.catch(() => undefined));
    baseline.release();

    await expect(final).resolves.toEqual({ revision: 1, disposition: "continue", violations: [], failures: [] });
    expect(auditGeneratedEdits.mock.calls).toEqual([["session-1", {
      revision: 1,
      checkpoints: ["final"],
      edits: [
        {
          path: "src/a.py", language: "python",
          previousReadError: "Source changed during initial discovery without a captured preimage", generatedText: "external replacement\n",
        },
        {
          path: "src/new.py", language: "python",
          previousReadError: "Source changed during initial discovery without a captured preimage", generatedText: "external creation\n",
        },
      ],
    }]]);
    await bridge.checkpoint("turn");
    expect(auditGeneratedEdits).toHaveBeenCalledTimes(1);
    await expect(bridge.finish("shell-1")).resolves.toEqual([]);
  });

  it("reconciles a shell write made before the baseline reaches its file at the command boundary", async () => {
    const root = await workspace();
    const path = join(root, "src", "a.py");
    await writeFile(path, "transaction.fetch_value('SELECT 1')\n");
    await awaitClockAfter(path);
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits));
    const baseline = holdDiscovery("directories", root);
    await bridge.openWorkspace("session-1");
    await baseline.entered;

    await bridge.begin("shell-1", "Bash", { command: "python3 -c \"...write_text(...)\"" });
    await writeFile(path, "cursor.execute('SELECT 1')\n");
    const result = bridge.finish("shell-1");
    passes.push(result.catch(() => undefined));
    expect(await settled(result)).toBe(false);
    baseline.release();
    await expect(result).resolves.toEqual([expect.objectContaining({ revision: 1 })]);
    expect(auditGeneratedEdits.mock.calls).toEqual([["session-1", {
      revision: 1,
      checkpoints: ["edit", "function"],
      edits: [{
        path: "src/a.py", language: "python",
        previousReadError: "Source changed during initial discovery without a captured preimage", generatedText: "cursor.execute('SELECT 1')\n",
      }],
    }]]);
  });

  it("answers a turn checkpoint once its wait runs out on a baseline walk, and audits what the walk finds behind it", async () => {
    const root = await workspace();
    const path = join(root, "src", "a.py");
    await writeFile(path, "before\n");
    await awaitClockAfter(path);
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits), { discoveryWaitMs: 50 });
    const baseline = holdDiscovery("directories", root);
    await bridge.openWorkspace("session-1");
    await baseline.entered;
    await bridge.begin("shell-1", "Bash", { command: "./write.sh &" });
    await writeFile(path, "shell write\n");

    await expect(bridge.checkpoint("turn")).resolves.toEqual(expect.objectContaining({
      failures: [expect.objectContaining({ code: "source-incomplete", message: "Initial source discovery is still pending" })],
    }));
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
    baseline.release();
    await vi.waitFor(() => expect(auditGeneratedEdits).toHaveBeenCalledTimes(1));
    expect(auditGeneratedEdits.mock.calls).toEqual([["session-1", expect.objectContaining({
      edits: [expect.objectContaining({ path: "src/a.py", generatedText: "shell write\n" })],
    })]]);
  });

  it("answers a shell result once its wait runs out on a baseline walk, marks the command boundary, and audits the write behind it", async () => {
    const root = await workspace();
    const path = join(root, "src", "a.py");
    await writeFile(path, "before\n");
    await awaitClockAfter(path);
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const pending: string[] = [];
    const bridge = createBridge(root, auditor(auditGeneratedEdits), { discoveryWaitMs: 50, onDiscoveryPending: boundary => { pending.push(boundary); } });
    const baseline = holdDiscovery("directories", root);
    await bridge.openWorkspace("session-1");
    await baseline.entered;

    await bridge.begin("shell-1", "Bash", { command: "python3 -c \"...write_text(...)\"" });
    await writeFile(path, "shell write\n");
    await expect(bridge.finish("shell-1")).resolves.toEqual([]);
    expect(pending).toEqual(["command"]);
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
    baseline.release();
    await vi.waitFor(() => expect(auditGeneratedEdits).toHaveBeenCalledTimes(1));
    expect(auditGeneratedEdits.mock.calls).toEqual([["session-1", expect.objectContaining({
      edits: [expect.objectContaining({ path: "src/a.py", generatedText: "shell write\n" })],
    })]]);
  });

  it("answers a boundary and still closes the evaluator when recording that the walk is still pending fails", async () => {
    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
    const root = await workspace();
    const evaluator = auditor(vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch)));
    const onDiscoveryPending = vi.fn(() => { throw new Error("database is locked"); });
    const bridge = createBridge(root, evaluator, { discoveryWaitMs: 50, onDiscoveryPending });
    const baseline = holdDiscovery("directories", root);
    try {
      await bridge.openWorkspace("session-1");
      await baseline.entered;

      await expect(bridge.checkpoint("turn")).resolves.toEqual(expect.objectContaining({
        failures: [expect.objectContaining({ code: "source-incomplete" })],
      }));
      await expect(bridge.close()).resolves.toBeUndefined();
      expect(onDiscoveryPending.mock.calls).toEqual([["turn"], ["close"]]);
      expect(evaluator.close).toHaveBeenCalledTimes(1);
      expect(stderr).toHaveBeenCalledWith(expect.stringContaining("recording failed: database is locked"));
    } finally {
      stderr.mockRestore();
    }
  });

  it("discovers a command's shell write that races the startup snapshot read", async () => {
    const root = await workspace();
    const path = join(root, "src", "a.py");
    await writeFile(path, "before\n");
    await awaitClockAfter(path);
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits));
    const reading = holdDiscovery("reads", path);
    await bridge.openWorkspace("session-1");
    await reading.entered;
    await bridge.begin("shell-race", "Bash", { command: "./write.sh" });
    await writeFile(path, "after\n");
    reading.release();
    await bridge.finish("shell-race");
    expect(auditGeneratedEdits).toHaveBeenCalledWith("session-1", expect.objectContaining({
      edits: [expect.objectContaining({ path: "src/a.py", generatedText: "after\n" })],
    }));
    await bridge.checkpoint("turn");
    // Each audit is the session's own discovery, never a Log audit of another writer's change.
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.mode)).not.toContain("log");
    expect(auditGeneratedEdits).toHaveBeenLastCalledWith("session-1", expect.objectContaining({
      edits: [expect.objectContaining({ path: "src/a.py", generatedText: "after\n" })],
    }));
  });

  // Claude Code runs a turn's commands side by side. The walk at one's result found the file the other was still
  // writing, emptied and not yet written again, audited that as the session's edit, and measured the other's change
  // from the empty file, so every line of it read as the agent's. So too for a command answered while the evaluator
  // opened, as a reopened session's first one in OMP, whose writes the turn's end reports.
  it.each([
    ["begun", false, (bridge: CodeRuleBridge) => bridge.begin("shell-1", "Bash", { command: "python3 generate.py" }), (bridge: CodeRuleBridge) => bridge.finish("shell-1")],
    ["answered while the evaluator opened", true, (bridge: CodeRuleBridge) => bridge.unwatchedCall("shell-1", "Bash"), async (bridge: CodeRuleBridge) => {
      await bridge.unwatchedResult("shell-1");
      await bridge.checkpoint("turn");
    }],
  ] as const)("leaves a file a command %s is still writing to the walk after it ends, though another command's result walks first", async (_kind, whileOpening, start, end) => {
    const root = await workspace();
    const path = join(root, "src", "a.py");
    await writeFile(path, "x = 1\n");
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const opening = Promise.withResolvers<void>();
    const entered = Promise.withResolvers<void>();
    const bridge = createBridge(root, {
      ...auditor(auditGeneratedEdits),
      openWorkspace: async () => {
        entered.resolve();
        await opening.promise;
      },
    });
    const open = bridge.openWorkspace("session-1");
    passes.push(open);
    await entered.promise;
    if (whileOpening) await start(bridge);
    opening.resolve();
    await open;
    await bridge.begin("write-1", "write", { path: ["src/a.py"] });
    await writeFile(path, "x = 2\n");
    await bridge.finish("write-1");
    await awaitClockAfter(path);

    if (!whileOpening) await start(bridge);
    await bridge.begin("shell-2", "Bash", { command: "ls" });
    await writeFile(path, "");
    await bridge.finish("shell-2");
    await writeFile(path, "x = 2\ny = 3\n");
    await end(bridge);
    expect(auditGeneratedEdits.mock.calls.flatMap(([, batch]) => batch.edits).map(({ generatedText, previousText, previousReadError }) => ({ generatedText, previousText, previousReadError }))).toEqual([
      { generatedText: "x = 2\n", previousText: "x = 1\n", previousReadError: undefined },
      { generatedText: "x = 2\ny = 3\n", previousText: "x = 2\n", previousReadError: undefined },
    ]);
  });

  // The walk at one command's result read the file the other was writing, and that command ended before the walk was
  // audited: the walk began while it ran, so the file is still left to the walk after.
  it("leaves a file to the walk after a command ends, though the command ends after a walk read the file and before the walk is audited", async () => {
    const root = await workspace();
    const path = join(root, "src", "a.py");
    await writeFile(path, "x = 1\n");
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits));
    await bridge.openWorkspace("session-1");
    await bridge.begin("write-1", "write", { path: ["src/a.py"] });
    await writeFile(path, "x = 2\n");
    await bridge.finish("write-1");
    // Every walk so far has run, so the read held below is the one of the walk at shell-2's result.
    await bridge.checkpoint("turn");
    await awaitClockAfter(path);

    const finished = vi.spyOn(ToolCallWindows.prototype, "finish");
    try {
      await bridge.begin("shell-1", "Bash", { command: "python3 generate.py" });
      await bridge.begin("shell-2", "Bash", { command: "ls" });
      await writeFile(path, "x = 2\ny");
      const read = holdDiscovery("reads", path);
      const walked = bridge.finish("shell-2");
      await read.entered;
      await writeFile(path, "x = 2\ny = 3\n");
      const ended = bridge.finish("shell-1");
      await vi.waitFor(() => expect(finished).toHaveBeenCalledWith("shell-1", expect.any(Number)));
      read.release();
      await Promise.all([walked, ended]);
    } finally {
      finished.mockRestore();
    }
    expect(auditGeneratedEdits.mock.calls.flatMap(([, batch]) => batch.edits).map(({ generatedText, previousText }) => ({ generatedText, previousText }))).toEqual([
      { generatedText: "x = 2\n", previousText: "x = 1\n" },
      { generatedText: "x = 2\ny = 3\n", previousText: "x = 2\n" },
    ]);
  });

  // A walk looked at a file one command had written, and read it only after a second command, begun meanwhile, emptied
  // it: its text was the emptied file, while the stamp it looked at said the change came before the second command. So
  // too when the command then removed the file before the walk could stamp it again.
  it.each([
    ["empties", false],
    ["empties and then removes", true],
  ] as const)("leaves a file a command %s while a walk reads it to the walk after that command ends", async (_kind, removed) => {
    const root = await workspace();
    const path = join(root, "src", "a.py");
    await writeFile(path, "x = 1\n");
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits));
    await bridge.openWorkspace("session-1");
    await bridge.begin("write-1", "write", { path: ["src/a.py"] });
    await writeFile(path, "x = 2\n");
    await bridge.finish("write-1");
    // No walk runs from here until the first command's result.
    await bridge.checkpoint("turn");

    await bridge.begin("shell-1", "Bash", { command: "./first.sh" });
    await writeFile(path, "x = 3\n");
    await awaitClockAfter(path);
    // The walk at the first command's result has looked at the file, and waits to open it.
    const opening = holdDiscovery("opens", path);
    const reading = holdDiscovery("reads", path);
    const firstResult = bridge.finish("shell-1");
    await opening.entered;
    await bridge.begin("shell-2", "Bash", { command: "python3 generate.py" });
    await writeFile(path, "");
    opening.release();
    await reading.entered;
    if (removed) await rm(path);
    reading.release();
    await firstResult;
    await writeFile(path, "x = 3\ny = 4\n");
    await bridge.finish("shell-2");
    expect(auditGeneratedEdits.mock.calls.flatMap(([, batch]) => batch.edits).map(({ generatedText, previousText }) => ({ generatedText, previousText }))).toEqual([
      { generatedText: "x = 2\n", previousText: "x = 1\n" },
      { generatedText: "x = 3\ny = 4\n", previousText: "x = 2\n" },
    ]);
  });

  // A walk that began while a command ran left the file the command wrote to the walk after it, but read the clock as it
  // began only after the command had ended. Taken as answered by that walk, the command's run was forgotten, so the walk
  // after took its write for another writer's: audited in Log, as if the agent wrote none of it.
  it("credits a command with the file a walk left to the walk after it, though that walk read the clock after the command ended", async () => {
    const root = await workspace();
    const path = join(root, "src", "a.py");
    await writeFile(path, "x = 1\n");
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits));
    await bridge.openWorkspace("session-1");
    await bridge.begin("write-1", "write", { path: ["src/a.py"] });
    await writeFile(path, "x = 2\n");
    await bridge.finish("write-1");
    // No walk runs from here until the second command's result.
    await bridge.checkpoint("turn");

    await bridge.begin("shell-1", "Bash", { command: "python3 generate.py" });
    await writeFile(path, "x = 2\ny = 3\n");
    await awaitClockAfter(path);
    const { rm: actualRm } = await vi.importActual<typeof FileSystem>("node:fs/promises");
    // The first command's end is sampled from the clock, and counted once its probe is removed. The clean-up lets the
    // removal go on before the bridges close, and puts `rm` and `finish` back, though this test times out.
    const sampled = Promise.withResolvers<void>();
    const counted = Promise.withResolvers<void>();
    releases.push(counted.resolve);
    let held = false;
    vi.mocked(rm).mockImplementation(async (target, options) => {
      if (!held && /rerail-clock-[^\\/]+$/u.test(String(target))) {
        held = true;
        sampled.resolve();
        await counted.promise;
      }
      return await actualRm(target, options);
    });
    const finished = vi.spyOn(ToolCallWindows.prototype, "finish");
    onTestFinished(() => {
      finished.mockRestore();
      vi.mocked(rm).mockReset();
    });
    const firstResult = bridge.finish("shell-1");
    await sampled.promise;
    const now = await mkdtemp(join(tmpdir(), "rerail-policy-now-"));
    directories.push(now);
    await awaitClockAfter(now);
    // The walk at the second command's result begins while the first still runs, and reads the clock after its end.
    const reading = holdDiscovery("reads", path);
    await bridge.begin("shell-2", "Bash", { command: "ls" });
    const secondResult = bridge.finish("shell-2");
    await reading.entered;
    counted.resolve();
    await vi.waitFor(() => expect(finished).toHaveBeenCalledWith("shell-1", expect.any(Number)));
    reading.release();
    await Promise.all([firstResult, secondResult]);
    expect(auditGeneratedEdits.mock.calls.flatMap(([, batch]) => batch.edits).map(({ generatedText, previousText }) => ({ generatedText, previousText }))).toEqual([
      { generatedText: "x = 2\n", previousText: "x = 1\n" },
      { generatedText: "x = 2\ny = 3\n", previousText: "x = 2\n" },
    ]);
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.mode)).not.toContain("log");
  });

  it("delivers committed full-file snapshots in deterministic batches", async () => {
    const root = await workspace();
    await writeFile(join(root, "src", "a.py"), "before\n");
    await writeFile(join(root, "src", "b.rs"), "fn before() {}\n");
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits));

    await bridge.openWorkspace("session-1");
    await bridge.begin("call-1", "write", { path: ["src/b.rs", "src/a.py"] });
    await writeFile(join(root, "src", "a.py"), "after\n");
    await writeFile(join(root, "src", "b.rs"), "fn after() {}\n");
    await bridge.finish("call-1");
    await bridge.close();

    expect(auditGeneratedEdits).toHaveBeenCalledTimes(1);
    expect(auditGeneratedEdits).toHaveBeenCalledWith("session-1", {
      revision: 1,
      checkpoints: ["edit", "function"],
      edits: [
        { path: "src/a.py", language: "python", previousText: "before\n", generatedText: "after\n" },
        { path: "src/b.rs", language: "rust", previousText: "fn before() {}\n", generatedText: "fn after() {}\n" },
      ],
    });
  });

  it("labels React files with the tsx language so the engine selects the JSX grammar", async () => {
    const root = await workspace();
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = new CodeRuleBridge(root, auditor(auditGeneratedEdits));

    await bridge.openWorkspace("session-1");
    await bridge.begin("call-1", "write", { path: ["src/view.tsx", "src/plain.ts"] });
    await writeFile(join(root, "src", "view.tsx"), "export const View = () => <div />;\n");
    await writeFile(join(root, "src", "plain.ts"), "export const plain = 1;\n");
    await bridge.finish("call-1");
    await bridge.close();

    expect(auditGeneratedEdits).toHaveBeenCalledWith("session-1", expect.objectContaining({
      edits: [
        { path: "src/plain.ts", language: "typescript", generatedText: "export const plain = 1;\n" },
        { path: "src/view.tsx", language: "tsx", generatedText: "export const View = () => <div />;\n" },
      ],
    }));
  });

  it("labels Blade templates with the blade language, not php, suffix-only basenames by their extname, and an unlisted name as text", async () => {
    const root = await workspace();
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = new CodeRuleBridge(root, auditor(auditGeneratedEdits));

    await bridge.openWorkspace("session-1");
    await bridge.begin("call-1", "write", { path: ["src/page.blade.php", "src/Kernel.php", "src/.blade.php", "src/.php"] });
    await writeFile(join(root, "src", "page.blade.php"), "{{ $title }}\n");
    await writeFile(join(root, "src", "Kernel.php"), "<?php\necho 1;\n");
    await writeFile(join(root, "src", ".blade.php"), "<?php\necho 2;\n");
    await writeFile(join(root, "src", ".php"), "<?php\necho 3;\n");
    await bridge.finish("call-1");
    await bridge.close();

    expect(auditGeneratedEdits).toHaveBeenCalledWith("session-1", expect.objectContaining({
      edits: [
        { path: "src/.blade.php", language: "php", generatedText: "<?php\necho 2;\n" },
        { path: "src/.php", language: "text", generatedText: "<?php\necho 3;\n" },
        { path: "src/Kernel.php", language: "php", generatedText: "<?php\necho 1;\n" },
        { path: "src/page.blade.php", language: "blade", generatedText: "{{ $title }}\n" },
      ],
    }));
  });

  it("classifies benchmark dialects by the same basename suffix rule as the bridge", () => {
    expect(codeRuleDialectForPath("src/page.blade.php")).toBe("blade");
    expect(codeRuleDialectForPath("src/Page.Blade.PHP")).toBe("blade");
    expect(codeRuleDialectForPath("src/.blade.php")).toBe("php");
    expect(codeRuleDialectForPath("src/.php")).toBeUndefined();
  });

  it("does not consume revisions for failed, unsupported, or no-op transactions", async () => {
    const root = await workspace();
    await writeFile(join(root, "src", "a.py"), "before\n");
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits));

    await bridge.openWorkspace("session-1");
    // The baseline walk, and the pass that follows it, end before the first call: one still running once the failed call
    // has answered finds the file that call wrote, stamped while it ran, and audits it as the session's change. What this
    // test pins is that no call's own result is audited but the committed one's.
    await bridge.checkpoint("turn");
    await bridge.begin("failed", "write", { path: "src/a.py" });
    await writeFile(join(root, "src", "a.py"), "failed\n");
    await bridge.finish("failed", true);
    await bridge.begin("unsupported", "read", { path: "src/a.py" });
    await bridge.finish("unsupported");
    await bridge.begin("noop", "write", { path: "src/a.py" });
    await writeFile(join(root, "src", "a.py"), "failed\n");
    await bridge.finish("noop");
    await bridge.begin("committed", "write", { path: "src/a.py" });
    await writeFile(join(root, "src", "a.py"), "after\n");
    await bridge.finish("committed");

    expect(auditGeneratedEdits).toHaveBeenCalledTimes(1);
    expect(auditGeneratedEdits).toHaveBeenCalledWith("session-1", expect.objectContaining({ revision: 1, edits: [expect.objectContaining({ path: "src/a.py", generatedText: "after\n" })] }));
  });

  it("discovers shell changes without treating scope changes or identical rewrites as edits", async () => {
    const root = await workspace();
    await writeFile(join(root, "src", "a.py"), "before\n");
    await awaitClockAfter(join(root, "src", "a.py"));
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const onRevision = vi.fn();
    let enabled = false;
    const bridge = createBridge(root, {
      ...auditor(auditGeneratedEdits), acceptsPath: () => enabled,
    }, { onRevision });

    await bridge.openWorkspace("session-1");
    await bridge.checkpoint("turn");
    enabled = true;
    await bridge.checkpoint("final");
    expect(auditGeneratedEdits).not.toHaveBeenCalled();

    await bridge.begin("shell-1", "Bash", { command: "./write.sh &" });
    await writeFile(join(root, "src", "a.py"), "after\n");
    await bridge.checkpoint("final");
    expect(auditGeneratedEdits).toHaveBeenCalledTimes(1);
    // Rewriting identical bytes changes the stamp but not the source.
    const rewrite = holdDiscovery("directories", root);
    const { pass } = await requestDiscovery(bridge, rewrite);
    await writeFile(join(root, "src", "a.py"), "after\n");
    rewrite.release();
    await pass;
    await bridge.checkpoint("turn");

    expect(auditGeneratedEdits).toHaveBeenCalledTimes(1);
    expect(auditGeneratedEdits).toHaveBeenCalledWith("session-1", expect.objectContaining({
      revision: 1,
      checkpoints: ["final"],
      edits: [expect.objectContaining({ path: "src/a.py", previousReadError: expect.any(String), generatedText: "after\n" })],
    }));
    expect(onRevision).toHaveBeenCalledWith("session-1", 1, ["src/a.py"]);
    await expect(bridge.finish("shell-1")).resolves.toEqual([]);
  });

  it("opens the workspace before the baseline walk and publishes the walk once settled", async () => {
    const root = await workspace();
    await writeFile(join(root, "src", "a.py"), "before\n");
    const observations: SourceObservationCache = new Map();
    const bridge = createBridge(root, auditor(vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch))), { observations });

    const baseline = holdDiscovery("reads", join(root, "src", "a.py"));
    await bridge.openWorkspace("session-1");
    await baseline.entered;
    expect(observations.size).toBe(0);
    baseline.release();

    await vi.waitFor(() => expect([...observations.get(root)?.keys() ?? []]).toEqual(["src/a.py"]));
    expect(observations.get(root)?.get("src/a.py")).toMatchObject({ language: "python", digest: expect.any(String), stamp: expect.any(String) });
  });

  it("still discovers sources changed between sessions that share baseline observations", async () => {
    const root = await workspace();
    await writeFile(join(root, "src", "a.py"), "before\n");
    await writeFile(join(root, "src", "b.py"), "stable\n");
    // A stamp from the same tick as the first walk is racy and would be re-read, so let the clock pass it.
    await awaitClockAfter(join(root, "src", "a.py"), join(root, "src", "b.py"));
    const observations: SourceObservationCache = new Map();
    const first = createBridge(root, auditor(vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch))), { observations });
    await first.openWorkspace("session-1");
    await vi.waitFor(() => expect(observations.get(root)?.get("src/b.py")?.digest).toBeDefined());
    const shared = new Map(observations.get(root));
    await first.close();

    await writeFile(join(root, "src", "a.py"), "after\n");
    await awaitClockAfter(join(root, "src", "a.py"));
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const second = createBridge(root, auditor(auditGeneratedEdits), { observations });
    const unchangedRead = holdDiscovery("reads", join(root, "src", "b.py"));
    await second.openWorkspace("session-2");
    // The second baseline reuses b.py's digest by stamp and re-reads only the changed a.py.
    await vi.waitFor(() => expect(observations.get(root)?.get("src/a.py")?.digest).not.toBe(shared.get("src/a.py")?.digest));
    expect(observations.get(root)?.get("src/b.py")).toEqual(shared.get("src/b.py"));
    expect(unchangedRead.started).toBe(false);
    expect(auditGeneratedEdits).not.toHaveBeenCalled();

    await shell(second, () => writeFile(join(root, "src", "a.py"), "after again\n"));
    expect(auditGeneratedEdits).toHaveBeenCalledTimes(1);
    expect(auditGeneratedEdits).toHaveBeenCalledWith("session-2", expect.objectContaining({
      edits: [expect.objectContaining({ path: "src/a.py", previousReadError: expect.any(String), generatedText: "after again\n" })],
    }));
  });

  it("keeps one observation of an unchanged source for all the sessions that share baseline observations", async () => {
    const root = await workspace();
    await writeFile(join(root, "src", "read.py"), "stable\n");
    await writeFile(join(root, "src", "skipped.py"), "stable\n");
    await awaitClockAfter(join(root, "src", "read.py"), join(root, "src", "skipped.py"));
    const observations: SourceObservationCache = new Map();
    // A source no rule reads is observed by its stamp alone.
    const open = async (sessionId: string): Promise<void> => {
      const bridge = createBridge(root, {
        ...auditor(vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch))),
        acceptsPath: path => path !== "src/skipped.py",
      }, { observations });
      await bridge.openWorkspace(sessionId);
    };
    await open("session-1");
    await vi.waitFor(() => expect(observations.get(root)?.get("src/read.py")?.digest).toBeDefined());
    const first = observations.get(root)!;
    await open("session-2");
    await vi.waitFor(() => expect(observations.get(root)).not.toBe(first));
    // Each live session keeps a baseline of every source of its checkout, so a copy each would cost a whole checkout's
    // observations a session.
    expect(observations.get(root)?.get("src/read.py")).toBe(first.get("src/read.py"));
    expect(observations.get(root)?.get("src/skipped.py")).toBe(first.get("src/skipped.py"));
  });

  it("costs each session a table of the shared observations, not copies of them or of their paths", { timeout: 60_000 }, async () => {
    const root = await workspace();
    const files = 2_000;
    // Names of about 165 characters: a path string of a session's own then costs more than the session's table, so a
    // session that keys its table by the walk's own path strings fails the bound by a wide margin.
    const name = (index: number): string => `module_${index}_${"x".repeat(150)}.py`;
    for (let index = 0; index < files; index += 1) await writeFile(join(root, "src", name(index)), "stable\n");
    await awaitClockAfter(join(root, "src", name(files - 1)));
    const observations: SourceObservationCache = new Map();
    const open = async (sessionId: string): Promise<void> => {
      const published = observations.get(root);
      const bridge = createBridge(root, auditor(vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch))), { observations });
      await bridge.openWorkspace(sessionId);
      await vi.waitFor(() => expect(observations.get(root)).not.toBe(published), { timeout: 20_000 });
      // The walk that follows a baseline has finished too: nothing a walk holds while it runs is counted.
      await bridge.checkpoint("turn");
    };
    // The first sessions read every source and pay for the compiled code.
    await open("session-1");
    await open("session-2");
    const before = await heapInUse();
    const sessions = 5;
    for (let index = 0; index < sessions; index += 1) await open(`session-${index + 3}`);
    // Each session's tables hold 60 to 69 bytes a source. A path string of its own brings that to about 250, and an
    // observation of its own, which "keeps one observation of an unchanged source" also catches, to about 195.
    expect((await heapInUse() - before) / sessions / files).toBeLessThan(105);
  });

  it("abandons the baseline walk of an interrupted session", async () => {
    const root = await workspace();
    await writeFile(join(root, "src", "a.py"), "before\n");
    const observations: SourceObservationCache = new Map();
    const bridge = new CodeRuleBridge(root, auditor(vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch))), { observations });

    await bridge.openWorkspace("session-1");
    bridge.interrupt();
    await bridge.close();

    expect(observations.size).toBe(0);
  });

  it("closes mid-walk with no Git it started still running in the folder, which can then be removed at once", async () => {
    const root = await workspace();
    git(root, "init", "--quiet");
    const bridge = new CodeRuleBridge(root, auditor(vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch))), {
      onIgnoreRulesChanged: () => undefined,
    });

    await bridge.openWorkspace("session-1");
    // The baseline walk reads HEAD and the ignore rules ask Git for their exclude files, neither waited for by the open.
    expect(runningIn(root)).not.toEqual([]);
    await bridge.close();

    expect(runningIn(root)).toEqual([]);
    // One try: on Windows a process still running in the folder keeps it from being removed.
    await removeFolder(root, { retry: false });
  });

  it("reopens with no Git the earlier session started still running in the folder", async () => {
    const root = await workspace();
    git(root, "init", "--quiet");
    const bridge = createBridge(root, auditor(vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch))));
    // The first session's walk reads HEAD first, through a Git that runs until it is ended.
    heldGit.reads = 1;
    await bridge.openWorkspace("session-1");
    const first = runningIn(root);
    expect(first).not.toEqual([]);

    await bridge.openWorkspace("session-2");
    // The second session's own Git may still run; none of the first session's does.
    expect(runningIn(root).filter(entry => first.includes(entry))).toEqual([]);
  });

  it("sends a deletion with its preimage instead of an unreadable workspace scan", async () => {
    const root = await workspace();
    await writeFile(join(root, "src", "a.py"), "before\n");
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits));

    await bridge.openWorkspace("session-1");
    await bridge.begin("delete", "edit", { path: "src/a.py" });
    await rm(join(root, "src", "a.py"));
    await bridge.finish("delete");

    expect(auditGeneratedEdits).toHaveBeenCalledWith("session-1", {
      revision: 1,
      checkpoints: ["edit", "function"],
      edits: [{ path: "src/a.py", language: "python", previousText: "before\n", deleted: true }],
    });
  });

  it("reports a failed preimage capture instead of treating it as a new file", async () => {
    const root = await workspace();
    await mkdir(join(root, "src", "a.py"));
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits));

    await bridge.openWorkspace("session-1");
    await bridge.begin("replace", "write", { path: "src/a.py" });
    await removeFolder(join(root, "src", "a.py"));
    await writeFile(join(root, "src", "a.py"), "after\n");
    await bridge.finish("replace");
    expect(auditGeneratedEdits).toHaveBeenCalledWith("session-1", expect.objectContaining({ edits: [expect.objectContaining({ path: "src/a.py", previousReadError: expect.any(String), generatedText: "after\n" })] }));
  });

  it("audits a write outside the session folder and never reports it deleted by a later walk of the folder", async () => {
    const base = await workspace();
    const session = join(base, "session");
    const sibling = join(base, "sibling");
    await mkdir(join(session, "src"), { recursive: true });
    await mkdir(sibling);
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(session, {
      ...auditor(auditGeneratedEdits),
      sourceRoot: path => resolve(session, path).startsWith(`${sibling}${sep}`) ? sibling : session,
    });

    await bridge.openWorkspace("session-1");
    await bridge.checkpoint("turn");
    await bridge.begin("call-1", "write", { path: join(sibling, "x.py") });
    await writeFile(join(sibling, "x.py"), "after\n");
    await bridge.finish("call-1");
    // The walk covers the session folder only; the sibling file it never sees is not a deletion.
    await bridge.checkpoint("turn");
    await bridge.checkpoint("final");

    expect(auditGeneratedEdits.mock.calls.flatMap(([, batch]) => batch.edits)).toEqual([
      { path: "../sibling/x.py", language: "python", generatedText: "after\n" },
    ]);
  });

  it("does not read a file the auditor finds no owner for", async () => {
    const root = await workspace();
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    // The owner can change between `acceptsPath` and the read; no owner must never mean "the session folder".
    const bridge = createBridge(root, { ...auditor(auditGeneratedEdits), sourceRoot: () => undefined });

    await bridge.openWorkspace("session-1");
    await bridge.begin("call-1", "write", { path: "src/a.py" });
    await writeFile(join(root, "src", "a.py"), "after\n");
    await bridge.finish("call-1");

    expect(auditGeneratedEdits.mock.calls.flatMap(([, batch]) => batch.edits)).toEqual([]);
  });

  // A session suspended while a tool call waited is opened again by that call's result, and the call is begun again
  // once the evaluator is open. The reopened session's first walk could read the call's edit before that, as another
  // writer's change (Log mode), so the edit was read twice and the call's steer could miss the next answer.
  it("leaves the files of a call the ledger held open out of the reopened session's first walk, for the call's own check", async () => {
    const root = await workspace();
    const path = join(root, "src", "a.py");
    await writeFile(path, "before\n");
    const first = createBridge(root, auditor(async (_sessionId, batch) => response(batch)));
    await first.openWorkspace("session-1");
    await first.checkpoint("turn");
    // The session's own edit, so the suspension holds the file as the session's, then a call that waits on a person.
    await first.begin("edit-0", "write", { path: "src/a.py" });
    await writeFile(path, "mine\n");
    await first.finish("edit-0");
    const preimages = await first.begin("edit-1", "write", { path: "src/a.py" });
    const resume = await first.walkResume();
    await first.close();
    // The call's write lands while the session is closed.
    await writeFile(path, "mine\nlater\n");
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const second = createBridge(root, auditor(auditGeneratedEdits), {
      ...(resume === undefined ? {} : { resume }),
      heldCalls: [{ toolCallId: "edit-1", paths: ["src/a.py"], preimages }],
    });
    await second.openWorkspace("session-1");
    await second.checkpoint("turn");
    await second.begin("edit-1", "write", { paths: ["src/a.py"] }, preimages);
    await second.finish("edit-1");

    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => ({ mode: batch.mode ?? "session", edits: batch.edits }))).toEqual([
      { mode: "session", edits: [{ path: "src/a.py", language: "python", previousText: "mine\n", generatedText: "mine\nlater\n" }] },
    ]);
  });

  // The open session holds a source as its own until the text budget drops its text. A source held before a suspension
  // stayed held after the budget dropped its text: another writer's change to it was checked as the open session would
  // not have, and every later suspension carried it on.
  it("lets go of a source held before a suspension once the session's text budget drops its text, as the open session does", async () => {
    const root = await workspace();
    const [held, other] = [join(root, "src", "a.py"), join(root, "src", "b.py")];
    await writeFile(held, "before\n");
    const first = createBridge(root, auditor(async (_sessionId, batch) => response(batch)));
    await first.openWorkspace("session-1");
    await first.checkpoint("turn");
    await first.begin("edit-0", "write", { path: "src/a.py" });
    await writeFile(held, "mine\n");
    await first.finish("edit-0");
    const resume = await first.walkResume();
    await first.close();
    expect([...resume?.held.keys() ?? []]).toEqual(["src/a.py"]);
    // Room for the five bytes of a.py's text, read back as the session opens again, and not for b.py's nine as well.
    const second = createBridge(root, auditor(async (_sessionId, batch) => response(batch)), {
      ...(resume === undefined ? {} : { resume }),
      sessionTextBytes: 8,
    });
    await second.openWorkspace("session-1");
    await second.checkpoint("turn");
    await second.begin("edit-1", "write", { path: "src/b.py" });
    await writeFile(other, "mine too\n");
    await second.finish("edit-1");

    expect([...(await second.walkResume())?.held.keys() ?? []]).toEqual([]);
  });

  // Held on after the budget dropped its text, a source was compared at the next open with what was last observed of it,
  // so that open's first walk read no text of it: the agent's next shell write there was measured from nothing, where
  // a session opened fresh measures it from the text the agent left.
  it("measures the agent's shell write to a source the budget let go of from the text it left there, at a later open too", async () => {
    const root = await workspace();
    const [held, other] = [join(root, "src", "a.py"), join(root, "src", "b.py")];
    await writeFile(held, "before\n");
    git(root, "init", "--quiet");
    git(root, "config", "core.autocrlf", "false");
    git(root, "add", ".");
    git(root, "commit", "--quiet", "-m", "seed");
    const first = createBridge(root, auditor(async (_sessionId, batch) => response(batch)));
    await first.openWorkspace("session-1");
    await first.checkpoint("turn");
    await first.begin("edit-0", "write", { path: "src/a.py" });
    await writeFile(held, "mine\n");
    await first.finish("edit-0");
    const quiet = await first.walkResume();
    await first.close();
    // Room for the five bytes of a.py's text or the nine of b.py's, not both: b.py's edit drops a.py's text.
    const second = createBridge(root, auditor(async (_sessionId, batch) => response(batch)), { ...(quiet === undefined ? {} : { resume: quiet }), sessionTextBytes: 12 });
    await second.openWorkspace("session-1");
    await second.checkpoint("turn");
    await second.begin("edit-1", "write", { path: "src/b.py" });
    await writeFile(other, "mine too\n");
    await second.finish("edit-1");
    const quietAgain = await second.walkResume();
    await second.close();
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const third = createBridge(root, auditor(auditGeneratedEdits), { ...(quietAgain === undefined ? {} : { resume: quietAgain }), sessionTextBytes: 12 });
    await third.openWorkspace("session-1");
    await third.checkpoint("turn");
    await shell(third, async () => await writeFile(held, "mine\nmore\n"));

    expect(auditGeneratedEdits.mock.calls.flatMap(([, batch]) => batch.edits.map(edit => ({ mode: batch.mode ?? "session", ...edit })))).toEqual([
      { mode: "session", path: "src/a.py", language: "python", previousText: "mine\n", generatedText: "mine\nmore\n" },
    ]);
  });
});

describe("CodeRuleBridge and a call it takes up only after its tool wrote the file", () => {
  /**
   * `src/<file>` holding `before` when the session opens, in a repository's one commit with `committed`, then `after`
   * written by a Claude Code Edit whose call the bridge begins only once the tool ran, as a daemon too busy to answer the
   * hook in time does; the call finishes with the tool's result, `output`, once something else has written `then`, when
   * given (a formatter, the next edit). The edits the auditor was given.
   */
  async function lateEdit(
    before: string,
    after: string,
    output: (path: string) => unknown,
    { file = "a.py", committed = false, then }: { file?: string; committed?: boolean; then?: string } = {},
  ) {
    const root = await workspace();
    const path = join(root, "src", file);
    await writeFile(path, before);
    if (committed) commitAll(root);
    await awaitClockAfter(path);
    const { bridge, auditGeneratedEdits } = await opened(root);
    await writeFile(path, after);
    await bridge.begin("edit-1", "Edit", { file_path: path });
    if (then !== undefined) {
      await awaitClockAfter(path);
      await writeFile(path, then);
    }
    await bridge.finish("edit-1", false, { reported: claudeCodeReportedPreimage(output(path)) });
    return auditGeneratedEdits.mock.calls.flatMap(([, batch]) => batch.edits);
  }
  /** Claude Code's Edit result with the line diff it made (`structuredPatch`) and no copy of the text it replaced. */
  const editResult = (lines: readonly string[]) => (path: string) => ({
    filePath: path, oldString: "two", newString: "TWO", originalFile: null, userModified: false, replaceAll: false,
    structuredPatch: [{ oldStart: 1, oldLines: 2, newStart: 1, newLines: 2, lines }],
  });

  it("reads the edit against the text the result's line diff rebuilds when the result leaves out the text it replaced", async () => {
    const before = "one\ntwo\nthree\n";
    const after = "one\nTWO\nthree\nfour\n";
    const output = (path: string) => ({
      ...editResult([])(path),
      structuredPatch: [{ oldStart: 1, oldLines: 3, newStart: 1, newLines: 4, lines: [" one", "-two", "+TWO", " three", "+four"] }],
    });
    expect(await lateEdit(before, after, output)).toEqual([{ path: "src/a.py", language: "python", previousText: before, generatedText: after }]);
  });

  it.each([
    { carriageReturns: "without", lines: [" one", "-two", "+TWO"] },
    { carriageReturns: "with", lines: [" one\r", "-two\r", "+TWO\r"] },
  ])("rebuilds a file with CRLF line endings from a line diff $carriageReturns carriage returns", async ({ lines }) => {
    expect(await lateEdit("one\r\ntwo\r\n", "one\r\nTWO\r\n", editResult(lines))).toEqual([
      { path: "src/a.py", language: "python", previousText: "one\r\ntwo\r\n", generatedText: "one\r\nTWO\r\n" },
    ]);
  });

  it("rebuilds a last line with no line feed", async () => {
    const lines = [" one", "-two", "\\ No newline at end of file", "+TWO", "\\ No newline at end of file"];
    expect(await lateEdit("one\ntwo", "one\nTWO", editResult(lines))).toEqual([
      { path: "src/a.py", language: "python", previousText: "one\ntwo", generatedText: "one\nTWO" },
    ]);
  });

  // Claude Code's line diff shows each tab of the run a line starts with as two spaces: the file's own lines are matched
  // through that, and a line only the diff holds gets its tabs back where the hunk's lines in the file have them.
  it.each([
    {
      indents: "one tab",
      before: "if a:\n\tone\n\ttwo\n", after: "if a:\n\tone\n\tTWO\n",
      lines: [" if a:", "   one", "-  two", "+  TWO"], newStart: 1,
    },
    {
      indents: "two tabs, under a line with one",
      before: "class A:\n\tdef f(self):\n\t\treturn 1\n\t# note\n", after: "class A:\n\tdef f(self):\n\t\treturn 2\n\t# note\n",
      lines: ["   def f(self):", "-    return 1", "+    return 2", "   # note"], newStart: 2,
    },
    {
      indents: "one tab, beside a line indented tab, space, tab, which the diff shows as three spaces and a tab",
      before: "if a:\n\t \t# note\n\tone\n\ttwo\n", after: "if a:\n\t \t# note\n\tone\n\tTWO\n",
      lines: [" if a:", "    \t# note", "   one", "-  two", "+  TWO"], newStart: 1,
    },
  ])("rebuilds a file indented with tabs ($indents) from a line diff that shows them as spaces", async ({ before, after, lines, newStart }) => {
    const output = (path: string) => ({
      ...editResult([])(path),
      structuredPatch: [{ oldStart: newStart, oldLines: 3, newStart, newLines: 3, lines }],
    });
    expect(await lateEdit(before, after, output)).toEqual([{ path: "src/a.py", language: "python", previousText: before, generatedText: after }]);
  });

  // A file that starts with a byte order mark, whose line diff may show its first line with or without the mark: the
  // file's first line is matched either way, and the text rebuilt holds the mark once, as the file does.
  it.each([
    { edit: "beside line 1", after: "﻿one\nTWO\nthree\n", lines: [" one", "-two", "+TWO", " three"] },
    { edit: "beside line 1, which the diff shows with the mark", after: "﻿one\nTWO\nthree\n", lines: [" ﻿one", "-two", "+TWO", " three"] },
    { edit: "of line 1", after: "﻿ONE\ntwo\nthree\n", lines: ["-one", "+ONE", " two", " three"] },
    { edit: "of line 1, which the diff shows with the mark", after: "﻿ONE\ntwo\nthree\n", lines: ["-﻿one", "+﻿ONE", " two", " three"] },
  ])("rebuilds a file with a byte order mark from a line diff $edit", async ({ after, lines }) => {
    const before = "﻿one\ntwo\nthree\n";
    const output = (path: string) => ({ ...editResult([])(path), structuredPatch: [{ oldStart: 1, oldLines: 3, newStart: 1, newLines: 3, lines }] });
    expect(await lateEdit(before, after, output)).toEqual([{ path: "src/a.py", language: "python", previousText: before, generatedText: after }]);
    const then = `${after}four\n`;
    expect(await lateEdit(before, after, output, { then })).toEqual([{ path: "src/a.py", language: "python", previousText: before, generatedText: then }]);
  });

  it("checks the edit over its whole file, saying why, when the file no longer holds the diff's lines and the result gives no text it replaced", async () => {
    // Changed again after the tool wrote it, and `originalFile` is null: nothing says what the file held, so the edit is
    // read as one whose earlier text is unknown, as a shell write to a file never seen is, not as no change.
    expect(await lateEdit("one\ntwo\n", "one\nTWO, then changed\n", editResult([" one", "-two", "+TWO"]))).toEqual([{
      path: "src/a.py", language: "python", generatedText: "one\nTWO, then changed\n",
      previousReadError: "The tool's result no longer fits the file, so what it held before the tool ran is unknown",
    }]);
  });

  // Changed again after the tool wrote it (a formatter, another edit): the diff no longer fits, so the edit is read
  // against the text the result gives, as an edit whose call came on time is read against its capture whatever wrote
  // the file since. That text takes the file's CRLF line endings where it has none of its own.
  it.each([
    { endings: "LF", before: "one\ntwo\n", after: "one\nTWO, then changed\n" },
    { endings: "CRLF", before: "one\r\ntwo\r\n", after: "one\r\nTWO, then changed\r\n" },
  ])("reads the edit against the text the result gives where the $endings file no longer holds the diff's lines", async ({ before, after }) => {
    const output = (path: string) => ({ ...editResult([" one", "-two", "+TWO"])(path), originalFile: "one\ntwo\n" });
    expect(await lateEdit(before, after, output)).toEqual([{ path: "src/a.py", language: "python", previousText: before, generatedText: after }]);
  });

  // Changed again after the call's capture took the tool's write (a formatter, the next edit): the line diff fits that
  // capture, as no capture taken before the tool ran does, so the edit is read against the text it rebuilds from it,
  // the tool's own lines with whatever changed since, as an edit whose call came on time is read.
  it.each([
    { changed: "by a formatter", then: "ONE\nTWO\nthree\n", originalFile: null },
    { changed: "by a formatter, its result giving the text it replaced", then: "ONE\nTWO\nthree\n", originalFile: "one\ntwo\nthree\n" },
    { changed: "by the next edit", then: "one\nTWO\nTHREE\n", originalFile: null },
  ])("reads the edit against the text the diff rebuilds from its capture when the file changed $changed after it", async ({ then, originalFile }) => {
    const output = (path: string) => ({ ...editResult([" one", "-two", "+TWO", " three"])(path), originalFile });
    expect(await lateEdit("one\ntwo\nthree\n", "one\nTWO\nthree\n", output, { then })).toEqual([
      { path: "src/a.py", language: "python", previousText: "one\ntwo\nthree\n", generatedText: then },
    ]);
  });

  // An Edit that added a line at the file's end: its line diff holds only the lines before the one it added, which a
  // capture taken after the tool wrote holds too, so where the file changed again only what the capture holds of the
  // tool's line tells when it was taken.
  const threeLines = "a = 0\nb = 1\nc = 2\n";
  const appendResult = (path: string) => ({
    ...editResult([])(path),
    structuredPatch: [{ oldStart: 1, oldLines: 3, newStart: 1, newLines: 4, lines: [" a = 0", " b = 1", " c = 2", "+mine = 3"] }],
  });

  it("reads the edit against the text the diff rebuilds from its capture when the next edit wrote the tool's line again", async () => {
    const then = `${threeLines}mine = 3\nmine = 3\n`;
    expect(await lateEdit(threeLines, `${threeLines}mine = 3\n`, appendResult, { then })).toEqual([
      { path: "src/a.py", language: "python", previousText: threeLines, generatedText: then },
    ]);
  });

  it("reads the edit against its capture when a formatter rewrote the tool's line before the call was read and the file changed again", async () => {
    // Nothing in the result rebuilds what the tool replaced, and its line diff does not place the capture before the
    // tool: it shows nothing about the capture, which another writer between the call and the tool would leave the
    // same way. Read against the capture, as an edit whose result says nothing is. A call that came while the rules
    // opened is read the other way (`deferredEdit` below).
    const then = `${threeLines}mine = 3  # fmt\nx = 4\n`;
    expect(await lateEdit(threeLines, `${threeLines}mine = 3  # fmt\n`, appendResult, { then })).toEqual([
      { path: "src/a.py", language: "python", previousText: `${threeLines}mine = 3  # fmt\n`, generatedText: then },
    ]);
  });

  it("finds no edit where an on-time Edit's file was put back before its result, which gives no text it replaced", async () => {
    const root = await workspace();
    const path = join(root, "src", "a.py");
    await writeFile(path, threeLines);
    await awaitClockAfter(path);
    const { bridge, auditGeneratedEdits } = await opened(root);
    await bridge.begin("edit-1", "Edit", { file_path: path });
    await writeFile(path, `${threeLines}mine = 3\n`);
    await writeFile(path, threeLines);
    await bridge.finish("edit-1", false, { reported: claudeCodeReportedPreimage(appendResult(path)) });
    expect(auditGeneratedEdits.mock.calls.flatMap(([, batch]) => batch.edits)).toEqual([]);
  });

  it("reads an on-time edit against its capture where the next edit changed the file and the capture holds a line like the tool's", async () => {
    const root = await workspace();
    const path = join(root, "src", "a.py");
    await writeFile(path, "mine\na\nb\n");
    await awaitClockAfter(path);
    const { bridge, auditGeneratedEdits } = await opened(root);
    await bridge.begin("edit-1", "Edit", { file_path: path });
    await writeFile(path, "mine\na\nmine\nb\n");
    await writeFile(path, "MINE\na\nmine\nb\n");
    const output = { ...editResult([])(path), structuredPatch: [{ oldStart: 1, oldLines: 3, newStart: 1, newLines: 4, lines: [" mine", " a", "+mine", " b"] }] };
    await bridge.finish("edit-1", false, { reported: claudeCodeReportedPreimage(output) });
    expect(auditGeneratedEdits.mock.calls.flatMap(([, batch]) => batch.edits)).toEqual([
      { path: "src/a.py", language: "python", previousText: "mine\na\nb\n", generatedText: "MINE\na\nmine\nb\n" },
    ]);
  });

  it("leaves the edit unchecked when the result names another file", async () => {
    const output = (path: string) => ({ ...editResult([" one", "+TWO"])(path), filePath: join(path, "..", "b.py") });
    expect(await lateEdit("one\n", "one\nTWO\n", output)).toEqual([]);
  });

  // A file named like one holding secrets keeps only the digest of what the result says it held, as a capture does: the
  // edit is read against that text only where the session or Git holds text with that digest.
  it.each([
    { holds: "Git holds it", committed: true, previous: { previousText: "one\n" } },
    { holds: "nothing holds it", committed: false, previous: { previousReadError: "Rerail keeps no copy of a file holding secrets" } },
  ])("keeps only the digest of what a secret-like file held, and reads the edit against it where $holds", async ({ committed, previous }) => {
    expect(await lateEdit("one\n", "one\nTWO\n", editResult([" one", "+TWO"]), { file: "secrets.py", committed })).toEqual([
      { path: "src/secrets.py", language: "python", ...previous, generatedText: "one\nTWO\n" },
    ]);
  });

  // `conf/config.py`, with `conf` a link (a junction on Windows) to `.aws`, is `.aws/config.py`: what the result says
  // that file held keeps only its digest however the call spells the file, so the edit is read as one of `.aws/config.py`.
  // A spelling named like one holding secrets keeps only the digest wherever it leads: `.aws/config.py` with `.aws` a
  // link to an ordinary folder.
  it.each([
    { spelled: "conf/config.py", folder: ".aws", link: "conf" },
    { spelled: ".aws/config.py", folder: ".aws", link: "conf" },
    { spelled: ".aws/config.py", folder: "plain", link: ".aws" },
  ])("keeps only the digest of what the result says $spelled held, with $link a link to $folder", async ({ spelled, folder, link }) => {
    const root = await workspace();
    const real = join(root, folder, "config.py");
    await mkdir(join(root, folder));
    await writeFile(real, "one\n");
    linkFolder(folder, join(root, link));
    await awaitClockAfter(real);
    const { bridge, auditGeneratedEdits } = await opened(root);
    await writeFile(real, "one\nTWO\n");
    const path = join(root, spelled);
    await bridge.begin("edit-1", "Edit", { file_path: path });
    await bridge.finish("edit-1", false, { reported: claudeCodeReportedPreimage(editResult([" one", "+TWO"])(path)) });
    expect(auditGeneratedEdits.mock.calls.flatMap(([, batch]) => batch.edits)).toEqual([
      { path: spelled, language: "python", previousReadError: "Rerail keeps no copy of a file holding secrets", generatedText: "one\nTWO\n" },
    ]);
  });

  // A result with no line diff cannot show that the call's capture is the file before the tool: a capture taken after the
  // tool wrote holds what it wrote, and an edit read against it would show none of the tool's lines. What the result says
  // the file held is read instead: nothing, for a file the tool made where the result does not say what it wrote, or the
  // text the tool replaced, whatever wrote the file between the call and the tool. A Write that made a file the call read
  // another text of than it wrote came after that capture: the file was removed between the call and the tool, and the
  // edit is read against the capture, as an edit whose result says nothing is.
  it.each([
    {
      says: "the tool made the file", then: "one\nTWO\n", reported: (path: string): ReportedPreimage => ({ kind: "created", path }),
      previous: {},
    },
    {
      says: "the tool made again the file it removed after the call", then: "one\nTWO\n",
      reported: (path: string) => claudeCodeReportedPreimage({ type: "create", filePath: path, content: "one\nTWO\n", structuredPatch: [], originalFile: null }),
      previous: { previousText: "one\n" },
    },
    {
      says: "the text the tool replaced", then: "other\nTWO\n", reported: (path: string): ReportedPreimage => ({ kind: "text", path, text: "other\n" }),
      previous: { previousText: "other\n" },
    },
  ])("reads an edit whose result has no line diff against what the result says the file held, where it says $says", async ({ then, reported, previous }) => {
    const root = await workspace();
    const path = join(root, "src", "a.py");
    await writeFile(path, "one\n");
    await awaitClockAfter(path);
    const { bridge, auditGeneratedEdits } = await opened(root);
    await bridge.begin("edit-1", "Edit", { file_path: path });
    await writeFile(path, then);
    await bridge.finish("edit-1", false, { reported: reported(path) });
    expect(auditGeneratedEdits.mock.calls.flatMap(([, batch]) => batch.edits)).toEqual([
      { path: "src/a.py", language: "python", ...previous, generatedText: then },
    ]);
  });

  // A call taken up only after its tool wrote, its file written again before the result (a formatter, the next edit),
  // whose result has no line diff: its capture holds what the tool wrote, in whatever form the file had when the call
  // read it, so a Write that made the file is read against an empty file where that capture is its `content`.
  const madeFile = (content: string) => (path: string) => ({ type: "create", filePath: path, content, structuredPatch: [], originalFile: null });
  it.each([
    { says: "a Write made the file", before: undefined, atCall: "mine = 1\n", previous: {}, output: madeFile("mine = 1\n") },
    { says: "a Write made the file, held with CRLF line endings", before: undefined, atCall: "mine = 1\r\n", previous: {}, output: madeFile("mine = 1\n") },
    { says: "a Write made the file, held with a byte order mark", before: undefined, atCall: "﻿mine = 1\n", previous: {}, output: madeFile("mine = 1\n") },
    { says: "a Write made the file, held with a last line feed", before: undefined, atCall: "mine = 1\n", previous: {}, output: madeFile("mine = 1") },
    {
      says: "a Write replaced the text it gives", before: "one\n", atCall: "one\nmine = 1\n", previous: { previousText: "one\n" },
      output: (path: string) => ({ type: "update", filePath: path, content: "one\nmine = 1\n", originalFile: "one\n" }),
    },
  ])("reads a late call's edit against what its result says the file held where $says and the file changed again", async ({ before, atCall, previous, output }) => {
    const root = await workspace();
    const path = join(root, "src", "a.py");
    if (before !== undefined) {
      await writeFile(path, before);
      await awaitClockAfter(path);
    }
    const { bridge, auditGeneratedEdits } = await opened(root);
    await writeFile(path, atCall);
    await bridge.begin("write-1", "Write", { file_path: path, content: atCall });
    await awaitClockAfter(path);
    await writeFile(path, `${atCall}x = 2\n`);
    await bridge.finish("write-1", false, { reported: claudeCodeReportedPreimage(output(path)) });
    expect(auditGeneratedEdits.mock.calls.flatMap(([, batch]) => batch.edits)).toEqual([
      { path: "src/a.py", language: "python", ...previous, generatedText: `${atCall}x = 2\n` },
    ]);
  });

  // A Write whose result says it made the file, where the call's capture holds other text than its `content`: the call
  // read the file before the tool ran and another process removed it between the call and the tool, or the call was
  // taken up only after a formatter changed the made file. No result tells the two apart, so once the file changed again
  // the edit is read against that capture, as an edit whose result says nothing is, and the lines the file held before
  // the Write are not read as the Write's.
  it.each([
    { capture: "the file before another process removed it after the call", before: "one\n", atTool: "one\nTWO\n", early: true },
    { capture: "the made file once a formatter changed it", before: undefined, atTool: "mine = 1  # fmt\n", early: false },
  ])("reads a Write that made its file against its capture where that capture is $capture and the file changed again", async ({ before, atTool, early }) => {
    const root = await workspace();
    const path = join(root, "src", "a.py");
    if (before !== undefined) {
      await writeFile(path, before);
      await awaitClockAfter(path);
    }
    const { bridge, auditGeneratedEdits } = await opened(root);
    if (early) {
      await bridge.begin("write-1", "Write", { file_path: path, content: atTool });
      await rm(path);
    }
    await writeFile(path, atTool);
    if (!early) await bridge.begin("write-1", "Write", { file_path: path, content: atTool });
    await awaitClockAfter(path);
    await writeFile(path, `${atTool}x = 2\n`);
    await bridge.finish("write-1", false, { reported: claudeCodeReportedPreimage(madeFile(early ? atTool : "mine = 1\n")(path)) });
    expect(auditGeneratedEdits.mock.calls.flatMap(([, batch]) => batch.edits)).toEqual([
      { path: "src/a.py", language: "python", previousText: before ?? atTool, generatedText: `${atTool}x = 2\n` },
    ]);
  });

  it("reads an on-time edit against its capture where nothing in its result rebuilds what the tool replaced", async () => {
    // Another writer put a line at the top between the call and the tool, so the diff's lines are not where its
    // `oldStart` places them in the capture; the tool added a line the file already holds, and a formatter rewrote the
    // file before the result, which gives no `originalFile`. Read against the capture, as on a call no result describes.
    const root = await workspace();
    const path = join(root, "src", "a.py");
    await writeFile(path, "a\nb\nc\n");
    await awaitClockAfter(path);
    const { bridge, auditGeneratedEdits } = await opened(root);
    await bridge.begin("edit-1", "Edit", { file_path: path });
    await writeFile(path, "top\na\nb\nc\n");
    await writeFile(path, "top\na\nb\na\nc\n");
    await writeFile(path, "top\na\nb\na\nC\n");
    const output = { ...editResult([])(path), structuredPatch: [{ oldStart: 1, oldLines: 4, newStart: 1, newLines: 5, lines: [" top", " a", " b", "+a", " c"] }] };
    await bridge.finish("edit-1", false, { reported: claudeCodeReportedPreimage(output) });
    expect(auditGeneratedEdits.mock.calls.flatMap(([, batch]) => batch.edits)).toEqual([
      { path: "src/a.py", language: "python", previousText: "a\nb\nc\n", generatedText: "top\na\nb\na\nC\n" },
    ]);
  });

  it("reads an edit whose call came before its tool against the call's own capture, though the result's line diff also fits that capture", async () => {
    // Removing one of three like lines: the diff's lines fit the file before the tool as well as after it.
    const root = await workspace();
    const path = join(root, "src", "a.py");
    await writeFile(path, "x\nx\nx\n");
    await awaitClockAfter(path);
    const { bridge, auditGeneratedEdits } = await opened(root);
    await bridge.begin("edit-1", "Edit", { file_path: path });
    await writeFile(path, "x\nx\n");
    const output = { filePath: path, originalFile: null, structuredPatch: [{ oldStart: 1, oldLines: 3, newStart: 1, newLines: 2, lines: [" x", "-x", " x"] }] };
    await bridge.finish("edit-1", false, { reported: claudeCodeReportedPreimage(output) });
    expect(auditGeneratedEdits.mock.calls.flatMap(([, batch]) => batch.edits)).toEqual([
      { path: "src/a.py", language: "python", previousText: "x\nx\nx\n", generatedText: "x\nx\n" },
    ]);
  });

  it.each([
    { then: undefined },
    { then: "﻿ONE\ntwo\nthree  # fmt\n" },
  ])("reads an on-time Edit of line 1 of a file with a byte order mark against its capture (then $then)", async ({ then }) => {
    const before = "﻿one\ntwo\nthree\n";
    const root = await workspace();
    const path = join(root, "src", "a.py");
    await writeFile(path, before);
    await awaitClockAfter(path);
    const { bridge, auditGeneratedEdits } = await opened(root);
    await bridge.begin("edit-1", "Edit", { file_path: path });
    await writeFile(path, "﻿ONE\ntwo\nthree\n");
    if (then !== undefined) await writeFile(path, then);
    const output = { ...editResult([])(path), structuredPatch: [{ oldStart: 1, oldLines: 3, newStart: 1, newLines: 3, lines: ["-one", "+ONE", " two", " three"] }] };
    await bridge.finish("edit-1", false, { reported: claudeCodeReportedPreimage(output) });
    expect(auditGeneratedEdits.mock.calls.flatMap(([, batch]) => batch.edits)).toEqual([
      { path: "src/a.py", language: "python", previousText: before, generatedText: then ?? "﻿ONE\ntwo\nthree\n" },
    ]);
  });

  // The result of a Write that left the file as it was may give the text it replaced without the file's byte order
  // mark, or with LF where the file has CRLF: that difference is no edit.
  it.each([
    { differs: "CRLF line endings", file: "one\r\ntwo\r\n" },
    { differs: "byte order mark", file: "\uFEFFone\ntwo\n" },
  ])("finds no edit where a Write left the file as it was and its result gives the text without the file's $differs", async ({ file }) => {
    const root = await workspace();
    const path = join(root, "src", "a.py");
    await writeFile(path, file);
    await awaitClockAfter(path);
    const { bridge, auditGeneratedEdits } = await opened(root);
    await bridge.begin("write-1", "Write", { file_path: path, content: file });
    await writeFile(path, file);
    const output = { type: "update", filePath: path, content: file, structuredPatch: [], originalFile: "one\ntwo\n" };
    await bridge.finish("write-1", false, { reported: claudeCodeReportedPreimage(output) });
    expect(auditGeneratedEdits.mock.calls.flatMap(([, batch]) => batch.edits)).toEqual([]);
  });
});

// Several Claude Code Edits of one file whose tools all ran before a busy daemon took any of their calls up, the
// common case under load.
describe("CodeRuleBridge and a burst of late calls on one file", () => {
  const numbered = (index: number) => `line${String(index).padStart(2, "0")}`;
  /** Twenty numbered lines with `ending`, those in `change` replaced. */
  const text = (ending: string, change: Record<number, string> = {}) =>
    Array.from({ length: 20 }, (_, index) => `${change[index + 1] ?? numbered(index + 1)}${ending}`).join("");
  /** Claude Code's Edit result: its line diff, one hunk from `newStart`, and `originalFile`. */
  const editResult = (newStart: number, lines: readonly string[], originalFile: string | null) => (path: string) => ({
    filePath: path, oldString: "x", newString: "y", originalFile, userModified: false, replaceAll: false,
    structuredPatch: [{ oldStart: newStart, oldLines: 0, newStart, newLines: 0, lines }],
  });
  /** Each tool writes its text in turn; only then are the calls begun and finished in order. The edits of each call. */
  async function burst(before: string, edits: ReadonlyArray<{ after: string; output: (path: string) => unknown }>) {
    const root = await workspace();
    const path = join(root, "src", "a.py");
    await writeFile(path, before);
    await awaitClockAfter(path);
    const { bridge, auditGeneratedEdits } = await opened(root);
    for (const edit of edits) await writeFile(path, edit.after);
    const perCall: unknown[][] = [];
    for (const [index, edit] of edits.entries()) {
      const seen = auditGeneratedEdits.mock.calls.length;
      await bridge.begin(`edit-${index}`, "Edit", { file_path: path });
      await bridge.finish(`edit-${index}`, false, { reported: claudeCodeReportedPreimage(edit.output(path)) });
      perCall.push(auditGeneratedEdits.mock.calls.slice(seen).flatMap(([, batch]) => batch.edits));
    }
    return perCall;
  }
  const edit = (previousText: string, generatedText: string) => ({ path: "src/a.py", language: "python", previousText, generatedText });

  it.each([{ ending: "\n" }, { ending: "\r\n" }])("reads each of two late edits far apart against its own lines ($ending)", async ({ ending }) => {
    const before = text(ending);
    const afterFirst = text(ending, { 3: "LINE03" });
    const after = text(ending, { 3: "LINE03", 17: "LINE17" });
    const first = editResult(1, [" line01", " line02", "-line03", "+LINE03", " line04", " line05", " line06"], null);
    const second = editResult(14, [" line14", " line15", " line16", "-line17", "+LINE17", " line18", " line19", " line20"], null);
    expect(await burst(before, [{ after: afterFirst, output: first }, { after, output: second }])).toEqual([
      [edit(text(ending, { 17: "LINE17" }), after)],
      [edit(afterFirst, after)],
    ]);
  });

  it("reads the first of two late edits of one line against originalFile, holding the second's line too", async () => {
    const before = text("\n");
    const afterFirst = text("\n", { 3: "LINE03" });
    const after = text("\n", { 3: "LINE03B" });
    const first = editResult(1, [" line01", " line02", "-line03", "+LINE03", " line04", " line05", " line06"], before);
    const second = editResult(1, [" line01", " line02", "-LINE03", "+LINE03B", " line04", " line05", " line06"], afterFirst);
    expect(await burst(before, [{ after: afterFirst, output: first }, { after, output: second }])).toEqual([
      [edit(before, after)],
      [edit(afterFirst, after)],
    ]);
  });

  it("checks the first of two late edits of one line over its whole file where its result gives no originalFile", async () => {
    const before = text("\n");
    const afterFirst = text("\n", { 3: "LINE03" });
    const after = text("\n", { 3: "LINE03B" });
    const first = editResult(1, [" line01", " line02", "-line03", "+LINE03", " line04", " line05", " line06"], null);
    const second = editResult(1, [" line01", " line02", "-LINE03", "+LINE03B", " line04", " line05", " line06"], null);
    expect(await burst(before, [{ after: afterFirst, output: first }, { after, output: second }])).toEqual([
      [{
        path: "src/a.py", language: "python", generatedText: after,
        previousReadError: "The tool's result no longer fits the file, so what it held before the tool ran is unknown",
      }],
      [edit(afterFirst, after)],
    ]);
  });
});

describe("ledgerPreimage", () => {
  it("keeps the text of a source that only names a token, and withholds one that assigns one", () => {
    const source = "import { getAuthTokenFromHeader } from './auth';\n\nconst token = getAuthTokenFromHeader;\nexport default token;\n";
    expect(ledgerPreimage({ text: source })).toEqual({ text: source });
    const leaky = "const token = 'Zk9xQ2mP7vLw8s';\n";
    expect(ledgerPreimage({ text: leaky })).toEqual({
      withheld: { sha256: createHash("sha256").update(leaky).digest("hex"), bytes: Buffer.byteLength(leaky) },
    });
  });
});

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}
/** `path` relative to `root` as the bridge names a file, with `/` between its parts on every platform. */
function repositoryPath(root: string, path: string): string {
  return relative(root, path).split(sep).join("/");
}
/** Makes `root` a repository whose one commit holds every file in it. */
function commitAll(root: string): void {
  git(root, "init", "--quiet");
  git(root, "add", ".");
  git(root, "commit", "--quiet", "-m", "seed");
}
/**
 * A session folder that is a repository holding `count - 1` nested ones, each with `src/x.py` changed since its commit;
 * `sourceRoot` names the checkout of a path, as a bridge's option.
 */
async function changedCheckouts(count: number) {
  const root = await workspace();
  const checkouts = [root, ...Array.from({ length: count - 1 }, (_, at) => join(root, `n${at}`))];
  for (const checkout of checkouts) {
    await mkdir(join(checkout, "src"), { recursive: true });
    await writeFile(join(checkout, "src", "x.py"), "committed\n");
    commitAll(checkout);
    await writeFile(join(checkout, "src", "x.py"), "uncommitted\n");
  }
  const changed = checkouts.map(checkout => join(checkout, "src", "x.py"));
  await awaitClockAfter(...changed);
  const nested = checkouts.slice(1);
  const sourceRoot = (path: string) => nested.find(checkout => resolve(root, path).startsWith(`${checkout}${sep}`)) ?? root;
  return { root, checkouts, changed, sourceRoot };
}
/** The shell edits of `changed` (from `uncommitted` to `uncommitted` and `mine`) as audited from what the walk first saw. */
function firstSightEdits(root: string, changed: readonly string[]) {
  return changed.map(path => ({ path: repositoryPath(root, path), language: "python", previousText: "uncommitted\n", generatedText: "uncommitted\nmine\n" }))
    .sort((left, right) => left.path.localeCompare(right.path));
}
/** A repository on `main`, with a branch `other` ahead of it: it changes `src/a.py`, adds `src/new.py` and removes `src/gone.py`. */
async function twoBranches(): Promise<string> {
  const root = await workspace();
  await writeFile(join(root, "src", "a.py"), "old\n");
  await writeFile(join(root, "src", "gone.py"), "gone\n");
  git(root, "init", "--quiet", "--initial-branch=main");
  // Git writes the files as committed, LF, whatever the machine sets: Git for Windows sets core.autocrlf=true for every
  // repository. The describe block on core.autocrlf sets it for its own repositories.
  git(root, "config", "core.autocrlf", "false");
  git(root, "add", ".");
  git(root, "commit", "--quiet", "-m", "seed");
  git(root, "switch", "--quiet", "-c", "other");
  await writeFile(join(root, "src", "a.py"), "old\nviolation\n");
  await writeFile(join(root, "src", "new.py"), "violation\n");
  await rm(join(root, "src", "gone.py"));
  git(root, "add", ".");
  git(root, "commit", "--quiet", "-m", "other");
  git(root, "switch", "--quiet", "main");
  await awaitClockAfter(join(root, "src", "a.py"), join(root, "src", "gone.py"));
  return root;
}
/** A bridge opened on `root` with its baseline taken, and its auditor's calls. */
async function opened(root: string, extra: Partial<CodeRuleAuditor> = {}, worktreeWalkGapMs = 0) {
  const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
  const bridge = createBridge(root, { ...auditor(auditGeneratedEdits), ...extra }, { worktreeWalkGapMs });
  await bridge.openWorkspace("session-1");
  await bridge.checkpoint("turn");
  await bridge.settleWorktreeWalk();
  return { bridge, auditGeneratedEdits };
}
/** Runs `run` as a shell tool call of `bridge`, and what the call's finish answered. */
async function shell(bridge: CodeRuleBridge, run: () => void | Promise<void>): Promise<readonly CodeRuleResponse[]> {
  await bridge.begin("shell-1", "shell", { command: "./run.sh" });
  await run();
  return await bridge.finish("shell-1");
}
/** How many Git processes `run` started to read a checkout's `HEAD`, and to list its unmerged entries. */
async function gitReads(run: () => Promise<unknown>): Promise<{ heads: number; entries: number }> {
  asyncGitCalls.args.length = 0;
  await run();
  const named = (name: string) => asyncGitCalls.args.filter(args => args.includes(name)).length;
  return { heads: named("--verify"), entries: named("--unmerged") };
}
/**
 * Makes `repository` one on `ours`, which changed `src/gone.py`; `other` removed it, so merging `other` stops on a
 * modify/delete conflict that changes no file. With `both`, each branch also changed `src/both.py` in its own place, and
 * that merge writes it.
 */
async function modifyDeleteBranches(repository: string, both: boolean): Promise<void> {
  await mkdir(join(repository, "src"), { recursive: true });
  await writeFile(join(repository, "src", "gone.py"), "gone\n");
  await writeFile(join(repository, "src", "both.py"), "1\n2\n3\n4\n5\n6\n7\n");
  git(repository, "init", "--quiet", "--initial-branch=main");
  git(repository, "add", ".");
  git(repository, "commit", "--quiet", "-m", "seed");
  git(repository, "switch", "--quiet", "-c", "other");
  await rm(join(repository, "src", "gone.py"));
  if (both) await writeFile(join(repository, "src", "both.py"), "theirs\n2\n3\n4\n5\n6\n7\n");
  git(repository, "commit", "--quiet", "-am", "other");
  git(repository, "switch", "--quiet", "-c", "ours", "main");
  await writeFile(join(repository, "src", "gone.py"), "gone\nours\n");
  if (both) await writeFile(join(repository, "src", "both.py"), "1\n2\n3\n4\n5\n6\nours\n");
  git(repository, "commit", "--quiet", "-am", "ours");
}
/**
 * A repository with a checkout in `lib` that owns its files: a repository of its own (`modifyDeleteBranches`), or with
 * `linked` a worktree of one elsewhere, whose `.git` is a file.
 */
async function nestedCheckout({ both = false, linked = false } = {}) {
  const root = await workspace();
  await writeFile(join(root, "src", "a.py"), "old\n");
  commitAll(root);
  const nested = join(root, "lib");
  const repository = linked ? await workspace() : nested;
  await modifyDeleteBranches(repository, both);
  if (linked) {
    git(repository, "switch", "--quiet", "main");
    git(repository, "worktree", "add", "--quiet", nested, "ours");
  }
  await awaitClockAfter(join(nested, "src", "gone.py"), join(nested, "src", "both.py"));
  const extra: Partial<CodeRuleAuditor> = { sourceRoot: path => resolve(root, path).startsWith(`${nested}${sep}`) ? nested : root };
  return { root, nested, extra };
}

/** `root` (from `twoBranches`) with a worktree of `branch`, and an auditor that owns its files and walks it. */
async function withWorktree(root: string, place: string, branch = "other") {
  git(root, "worktree", "add", "--quiet", place, branch);
  const worktree = await realpath(place);
  const roots = [worktree];
  const extra: Partial<CodeRuleAuditor> = {
    sourceRoot: path => roots.find(known => resolve(root, path).startsWith(`${known}${sep}`)) ?? root,
    worktreeRoots: () => roots,
  };
  return { worktree, roots, extra };
}

describe("CodeRuleBridge and every worktree of the repository", () => {
  /** Runs `run` as a shell call, then lets the walk of the other worktrees, which the call does not wait for, find it. */
  async function shellInWorktree(bridge: CodeRuleBridge, run: () => void | Promise<void>): Promise<void> {
    await shell(bridge, run);
    await bridge.settleWorktreeWalk();
    await bridge.checkpoint("turn");
    await bridge.settleWorktreeWalk();
  }

  it("audits a shell write in a worktree outside the session's folder", async () => {
    const root = await twoBranches();
    const { worktree, extra } = await withWorktree(root, join(await workspace(), "w"));
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    await shellInWorktree(bridge, () => writeFile(join(worktree, "src", "a.py"), "old\nviolation\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: repositoryPath(root, join(worktree, "src", "a.py")), language: "python", previousText: "old\nviolation\n", generatedText: "old\nviolation\nmine\n" }],
    ]);
  });

  // Once a shared walk's snapshot agrees with what the session saw of a file in another worktree, the session keeps no
  // observation of its own (`#observe`). A suspension read only the session's own, so it recorded each such held file as
  // gone: the reopened session read back no text of it, and took another writer's change made meanwhile as its baseline.
  it("holds the files it edited in another worktree across a suspension, once a shared snapshot agrees with them", async () => {
    const root = await twoBranches();
    const { worktree, extra } = await withWorktree(root, join(await workspace(), "w"));
    const [changed, kept] = [join(worktree, "src", "a.py"), join(worktree, "src", "b.py")];
    const { bridge } = await opened(root, extra);
    await shellInWorktree(bridge, async () => {
      await writeFile(changed, "old\nviolation\nmine\n");
      await writeFile(kept, "b = 1\n");
    });
    const resume = await bridge.walkResume();
    await bridge.close();
    const digest = (text: string) => createHash("sha256").update(text).digest("hex");
    expect([changed, kept].map(file => resume?.held.get(repositoryPath(root, file))?.digest)).toEqual([digest("old\nviolation\nmine\n"), digest("b = 1\n")]);
    // Another writer, while the session is suspended: its change is recorded, though the suspension kept no text of it.
    await writeFile(changed, "old\nviolation\nmine\ntheirs\n");
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const second = createBridge(root, { ...auditor(auditGeneratedEdits), ...extra }, {
      worktreeWalkGapMs: 0,
      ...(resume === undefined ? {} : { resume }),
    });
    await second.openWorkspace("session-1");
    await second.checkpoint("turn");
    await second.settleWorktreeWalk();
    // The unchanged file's text was read back as the session opened, so a later change to it is measured from it.
    await writeFile(kept, "b = 1\ntheirs\n");
    await second.checkpoint("turn");
    await second.settleWorktreeWalk();
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => [batch.edits, batch.mode])).toEqual([
      [[{
        path: repositoryPath(root, changed), language: "python",
        previousReadError: "Source changed outside a tool with a captured preimage", generatedText: "old\nviolation\nmine\ntheirs\n",
      }], "log"],
      [[{ path: repositoryPath(root, kept), language: "python", previousText: "b = 1\n", generatedText: "b = 1\ntheirs\n" }], "log"],
    ]);
  });

  it("records a shell write in another worktree in Log, so it steers no session, and steers on one in its own folder", async () => {
    const root = await twoBranches();
    const { worktree, extra } = await withWorktree(root, join(await workspace(), "w"));
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    await shellInWorktree(bridge, () => writeFile(join(worktree, "src", "a.py"), "old\nviolation\nmine\n"));
    await shell(bridge, () => writeFile(join(root, "src", "a.py"), "old\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => [batch.edits.map(edit => edit.path), batch.mode])).toEqual([
      [[repositoryPath(root, join(worktree, "src", "a.py"))], "log"],
      [["src/a.py"], undefined],
    ]);
  });

  it("measures a shell edit to a file changed before the session, in a worktree inside its folder, from the text first seen", async () => {
    const root = await twoBranches();
    const { worktree, extra } = await withWorktree(root, join(root, "wt"));
    const path = join(worktree, "src", "a.py");
    await writeFile(path, "old\nviolation\nlocal\n");
    await awaitClockAfter(path);
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    await shellInWorktree(bridge, () => writeFile(path, "old\nviolation\nlocal\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "wt/src/a.py", language: "python", previousText: "old\nviolation\nlocal\n", generatedText: "old\nviolation\nlocal\nmine\n" }],
    ]);
  });

  it("puts in checkpoint's answer a worktree whose Git did not list the files changed before the session in time, and lists them again off the walks' path", async () => {
    const root = await twoBranches();
    const { worktree, extra } = await withWorktree(root, join(root, "wt"));
    const path = join(worktree, "src", "a.py");
    await writeFile(path, "old\nviolation\nlocal\n");
    await awaitClockAfter(path);
    // The worktree's first walk's listing runs past its time limit; the one asked again answers.
    listings.late.push(worktree);
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    expect((await bridge.checkpoint("turn")).failures).toEqual([
      expect.objectContaining({ code: "source-incomplete", message: "wt: Git did not list the checkout's changed files in time" }),
    ]);
    await bridge.settleWorktreeWalk();
    await bridge.settleListingsAgain();
    await shellInWorktree(bridge, () => writeFile(path, "old\nviolation\nlocal\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "wt/src/a.py", language: "python", previousText: "old\nviolation\nlocal\n", generatedText: "old\nviolation\nlocal\nmine\n" }],
    ]);
  });

  it("records a shell edit of a file a modify/delete conflict in another worktree left as it was, where that worktree's walk found no change", async () => {
    const root = await workspace();
    await writeFile(join(root, "src", "gone.py"), "gone\n");
    git(root, "init", "--quiet", "--initial-branch=main");
    git(root, "add", ".");
    git(root, "commit", "--quiet", "-m", "seed");
    git(root, "switch", "--quiet", "-c", "other");
    await rm(join(root, "src", "gone.py"));
    git(root, "commit", "--quiet", "-am", "other");
    git(root, "switch", "--quiet", "-c", "kept", "main");
    await writeFile(join(root, "src", "gone.py"), "gone\nours\n");
    git(root, "commit", "--quiet", "-am", "ours");
    git(root, "switch", "--quiet", "main");
    const { worktree, extra } = await withWorktree(root, join(await workspace(), "w"), "kept");
    const path = join(worktree, "src", "gone.py");
    await awaitClockAfter(path);
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    await shellInWorktree(bridge, () => { try { git(worktree, "merge", "--quiet", "other"); } catch { /* The conflict is the point. */ } });
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
    await shellInWorktree(bridge, () => writeFile(path, "gone\nours\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => [batch.edits, batch.mode])).toEqual([
      [[{ path: repositoryPath(root, path), language: "python", previousText: "gone\nours\n", generatedText: "gone\nours\nmine\n" }], "log"],
    ]);
  });

  /** A worktree outside the session's folder with a repository of its own in its `lib` (`modifyDeleteBranches`), which owns its files. */
  async function checkoutInWorktree({ both = false } = {}) {
    const root = await twoBranches();
    const { worktree, extra } = await withWorktree(root, join(await workspace(), "w"));
    const nested = join(worktree, "lib");
    await modifyDeleteBranches(nested, both);
    await awaitClockAfter(join(nested, "src", "gone.py"), join(nested, "src", "both.py"));
    const sourceRoot = (path: string) => resolve(root, path).startsWith(`${nested}${sep}`) ? nested : extra.sourceRoot!(path);
    return { root, nested, extra: { ...extra, sourceRoot } };
  }
  /** Merges `other` in `nested`, which stops on the modify/delete conflict and leaves its unmerged entries. */
  const stopMerge = (nested: string) => () => {
    expect(() => git(nested, "merge", "--quiet", "other")).toThrow();
    expect(execFileSync("git", ["ls-files", "--unmerged"], { cwd: nested, encoding: "utf8" })).not.toBe("");
  };

  const eitherWay = [["", false], [", once a walk has found a change there", true]] as const;
  /** With `earlier`, a shell write the worktrees' walk finds in `nested` first, which no later assertion counts. */
  async function openedOn(root: string, nested: string, extra: Partial<CodeRuleAuditor>, earlier: boolean) {
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    if (earlier) await shellInWorktree(bridge, () => writeFile(join(nested, "src", "note.py"), "mine\n"));
    auditGeneratedEdits.mockClear();
    return { bridge, auditGeneratedEdits };
  }

  it.each(eitherWay)("records a shell edit of a file a modify/delete conflict left as it was, in a checkout inside another worktree%s", async (_name, earlier) => {
    const { root, nested, extra } = await checkoutInWorktree();
    const path = join(nested, "src", "gone.py");
    const { bridge, auditGeneratedEdits } = await openedOn(root, nested, extra, earlier);
    await shellInWorktree(bridge, stopMerge(nested));
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
    await shellInWorktree(bridge, () => writeFile(path, "gone\nours\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => [batch.edits, batch.mode])).toEqual([
      [[{ path: repositoryPath(root, path), language: "python", previousText: "gone\nours\n", generatedText: "gone\nours\nmine\n" }], "log"],
    ]);
  });

  it.each(eitherWay)("records nothing a merge wrote beside a modify/delete conflict, in a checkout inside another worktree%s", async (_name, earlier) => {
    const { root, nested, extra } = await checkoutInWorktree({ both: true });
    const { bridge, auditGeneratedEdits } = await openedOn(root, nested, extra, earlier);
    await shellInWorktree(bridge, stopMerge(nested));
    expect(await readFile(join(nested, "src", "both.py"), "utf8")).toBe("theirs\n2\n3\n4\n5\n6\nours\n");
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
  });

  it.each(eitherWay)("records nothing a branch switch wrote or removed in a checkout inside another worktree%s, and a file written and committed after it from the branch's text", async (_name, earlier) => {
    const { root, nested, extra } = await checkoutInWorktree({ both: true });
    const path = join(nested, "src", "both.py");
    const { bridge, auditGeneratedEdits } = await openedOn(root, nested, extra, earlier);
    await shellInWorktree(bridge, () => git(nested, "switch", "--quiet", "other"));
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
    // Measured from `other`'s blob only by the `HEAD` the switch left: read before the worktrees' walk, each time.
    await shellInWorktree(bridge, async () => {
      await writeFile(path, "theirs\n2\n3\n4\n5\n6\n7\nmine\n");
      git(nested, "commit", "--quiet", "-am", "mine");
    });
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => [batch.edits, batch.mode])).toEqual([
      [[{ path: repositoryPath(root, path), language: "python", previousText: "theirs\n2\n3\n4\n5\n6\n7\n", generatedText: "theirs\n2\n3\n4\n5\n6\n7\nmine\n" }], "log"],
    ]);
  });

  /** `extra` with a `worktreeRoots` that throws while `failing.on`, as a Git that cannot list the worktrees does. */
  function failingRoots(extra: Partial<CodeRuleAuditor>) {
    const failing = { on: false };
    const worktreeRoots = () => {
      if (failing.on) throw new Error("git worktree list failed");
      return extra.worktreeRoots!();
    };
    return { extra: { ...extra, worktreeRoots }, failing };
  }
  /** Walks while the session cannot list its worktrees, so it takes no part, and the shared walk drops what it kept of them. */
  async function walkUnlisted(bridge: CodeRuleBridge, failing: { on: boolean }): Promise<void> {
    failing.on = true;
    try {
      await shellInWorktree(bridge, () => undefined);
    } finally {
      failing.on = false;
    }
  }

  /** Runs `run` as a shell call, and the one walk of the other worktrees its finish asks for, as a call followed by another in one turn has. */
  async function shellThenWalk(bridge: CodeRuleBridge, run: () => void | Promise<void>): Promise<void> {
    await shell(bridge, run);
    await bridge.settleWorktreeWalk();
  }

  it.each(eitherWay)("records a shell edit of a file a modify/delete conflict left as it was, in a checkout inside another worktree whose walk dropped what it kept%s", async (_name, earlier) => {
    const { root, nested, extra: listed } = await checkoutInWorktree();
    const { extra, failing } = failingRoots(listed);
    const path = join(nested, "src", "gone.py");
    const { bridge, auditGeneratedEdits } = await openedOn(root, nested, extra, earlier);
    await walkUnlisted(bridge, failing);
    await shellThenWalk(bridge, stopMerge(nested));
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
    await shellThenWalk(bridge, () => writeFile(path, "gone\nours\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => [batch.edits, batch.mode])).toEqual([
      [[{ path: repositoryPath(root, path), language: "python", previousText: "gone\nours\n", generatedText: "gone\nours\nmine\n" }], "log"],
    ]);
  });

  it.each(eitherWay)("records nothing a branch switch wrote in a checkout inside another worktree whose walk dropped what it kept%s, and a file written and committed after it from the branch's text", async (_name, earlier) => {
    const { root, nested, extra: listed } = await checkoutInWorktree({ both: true });
    const { extra, failing } = failingRoots(listed);
    const path = join(nested, "src", "both.py");
    const { bridge, auditGeneratedEdits } = await openedOn(root, nested, extra, earlier);
    await walkUnlisted(bridge, failing);
    await shellThenWalk(bridge, () => git(nested, "switch", "--quiet", "other"));
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
    await shellThenWalk(bridge, async () => {
      await writeFile(path, "theirs\n2\n3\n4\n5\n6\n7\nmine\n");
      git(nested, "commit", "--quiet", "-am", "mine");
    });
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => [batch.edits, batch.mode])).toEqual([
      [[{ path: repositoryPath(root, path), language: "python", previousText: "theirs\n2\n3\n4\n5\n6\n7\n", generatedText: "theirs\n2\n3\n4\n5\n6\n7\nmine\n" }], "log"],
    ]);
  });

  it("starts no Git process for a checkout inside another worktree until its index file changes, and reads its `HEAD` before one walk only", async () => {
    const { root, nested, extra } = await checkoutInWorktree();
    const { bridge } = await opened(root, extra);
    const bothWalks = async () => {
      await bridge.checkpoint("turn");
      await bridge.settleWorktreeWalk();
    };
    // Walks that find nothing read the session checkout's `HEAD` and the worktree's, and nothing of the one inside it.
    expect(await gitReads(bothWalks)).toEqual({ heads: 2, entries: 0 });
    // The walks after a write to its index list its entries once, and those after that do not.
    stopMerge(nested)();
    expect(await gitReads(bothWalks)).toEqual({ heads: 2, entries: 1 });
    expect(await gitReads(bothWalks)).toEqual({ heads: 2, entries: 0 });
    // Once a walk has found a change there, the worktrees' walk reads its `HEAD` too, and the session folder's does not.
    await writeFile(join(nested, "src", "gone.py"), "gone\nours\nmine\n");
    await bothWalks();
    expect(await gitReads(bothWalks)).toEqual({ heads: 3, entries: 0 });
  });

  it("audits a shell edit of a file a modify/delete conflict left as it was, in a checkout inside a session folder that another worktree's folder holds", async () => {
    // The session is in a worktree kept inside the main checkout, which is then one of its other worktrees.
    const main = await twoBranches();
    git(main, "worktree", "add", "--quiet", "wt", "other");
    const root = await realpath(join(main, "wt"));
    const nested = join(root, "lib");
    await modifyDeleteBranches(nested, false);
    await awaitClockAfter(join(nested, "src", "gone.py"));
    const within = (folder: string, path: string) => resolve(root, path).startsWith(`${folder}${sep}`);
    const { bridge, auditGeneratedEdits } = await opened(root, {
      sourceRoot: path => within(nested, path) ? nested : within(root, path) ? root : main,
      worktreeRoots: () => [main],
    });
    // The checkout is the session folder's walk's, though it lies in the main checkout's folder too.
    await shellInWorktree(bridge, stopMerge(nested));
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
    await shellInWorktree(bridge, () => writeFile(join(nested, "src", "gone.py"), "gone\nours\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => [batch.edits, batch.mode])).toEqual([
      [[{ path: "lib/src/gone.py", language: "python", previousText: "gone\nours\n", generatedText: "gone\nours\nmine\n" }], undefined],
    ]);
  });

  it("audits a shell write in a worktree in a folder the session's checkout ignores", async () => {
    const root = await twoBranches();
    const { worktree, extra } = await withWorktree(root, join(root, ".ignored", "w"));
    const { bridge, auditGeneratedEdits } = await opened(root, {
      ...extra, ignoresPath: (path, directory) => directory && path === ".ignored",
    });
    await shellInWorktree(bridge, () => writeFile(join(worktree, "src", "a.py"), "old\nviolation\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits.map(edit => edit.path))).toEqual([[".ignored/w/src/a.py"]]);
  });

  it("answers a command without waiting for the walk of the other worktrees", async () => {
    const root = await twoBranches();
    const { worktree, extra } = await withWorktree(root, join(await workspace(), "w"));
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    await writeFile(join(worktree, "src", "a.py"), "old\nviolation\nmine\n");
    // The session's own walk answers the command; the worktree's change is found behind it.
    expect(await shell(bridge, () => undefined)).toEqual([]);
    await bridge.settleWorktreeWalk();
    await bridge.checkpoint("turn");
    await bridge.settleWorktreeWalk();
    expect(auditGeneratedEdits.mock.calls.flatMap(([, batch]) => batch.edits.map(edit => edit.path))).toEqual([repositoryPath(root, join(worktree, "src", "a.py"))]);
  });

  it("takes a worktree's first walk as its baseline, though `git worktree add` wrote it after the session opened, and records a later change", async () => {
    const root = await twoBranches();
    const roots: string[] = [];
    const changes: IgnoreRulesChange[] = [];
    const bridge = createBridge(root, {
      ...auditor(vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch))),
      sourceRoot: path => roots.find(known => resolve(root, path).startsWith(`${known}${sep}`)) ?? root,
      worktreeRoots: () => roots,
    }, { worktreeWalkGapMs: 0, onIgnoreRulesChanged: change => { changes.push(change); } });
    await bridge.openWorkspace("session-1");
    await bridge.checkpoint("turn");
    const place = join(await workspace(), "w");
    git(root, "worktree", "add", "--quiet", place, "other");
    const worktree = await realpath(place);
    await writeFile(join(worktree, ".gitignore"), "dist/\n");
    roots.push(worktree);

    await shellInWorktree(bridge, () => undefined);
    expect(changes).toEqual([]);
    await shellInWorktree(bridge, () => appendFile(join(worktree, ".gitignore"), "scratch/\n"));
    await shell(bridge, () => undefined);
    expect(changes).toEqual([{ path: join(worktree, ".gitignore"), via: "observed", ignoresMore: ["scratch/"] }]);
  });

  it("walks a worktree inside the session's folder once", async () => {
    const root = await twoBranches();
    const { worktree, extra } = await withWorktree(root, join(root, "wt"));
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    await shellInWorktree(bridge, () => writeFile(join(worktree, "src", "a.py"), "old\nviolation\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.flatMap(([, batch]) => batch.edits.map(edit => edit.path))).toEqual(["wt/src/a.py"]);
  });

  it("keeps no observation of its own of the other worktrees' sources while it lives, and of files a tool wrote outside its folder once it closes", async () => {
    const root = await twoBranches();
    const roots: string[] = [];
    for (const name of ["w1", "w2", "w3"]) {
      const place = join(await workspace(), name);
      git(root, "worktree", "add", "--quiet", "--detach", place, "other");
      roots.push(await realpath(place));
    }
    const sibling = join(await workspace(), "sibling");
    await mkdir(sibling);
    const observations: SourceObservationCache = new Map();
    const bridge = createBridge(root, {
      ...auditor(vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch))),
      sourceRoot: path => [...roots, sibling].find(known => resolve(root, path).startsWith(`${known}${sep}`)) ?? root,
      worktreeRoots: () => roots,
    }, { worktreeWalkGapMs: 0, observations });
    await bridge.openWorkspace("session-1");
    await bridge.checkpoint("turn");
    await bridge.begin("call-1", "write", { path: join(sibling, "x.py") });
    await writeFile(join(sibling, "x.py"), "after\n");
    await bridge.finish("call-1");
    await bridge.checkpoint("turn");
    await bridge.settleWorktreeWalk();
    const own = ["src/a.py", "src/gone.py"];
    const outside = relative(root, join(sibling, "x.py")).split(sep).join("/");
    // The bridge shares every observation it keeps of its own. The other worktrees' sources are the shared walk's: a
    // live session that kept one of each held every file of every worktree, and the daemon's heap grew with the live
    // sessions times the worktrees times their files.
    expect([...observations.get(root)?.keys() ?? []].sort()).toEqual([...own, outside].sort());
    await bridge.close();
    // The daemon keeps a folder's observations while it serves the folder, so each folder sessions started in held every
    // file of every worktree: its heap grew with the folders times the files of all the repository's worktrees.
    expect([...observations.get(root)?.keys() ?? []].sort()).toEqual(own);
  });

  it("checks a later session's edits in its folder, in a worktree and outside the folder as it did before an earlier session there closed", async () => {
    const root = await twoBranches();
    const { worktree, roots } = await withWorktree(root, join(await workspace(), "w"));
    const sibling = join(await workspace(), "sibling");
    await mkdir(sibling);
    await writeFile(join(sibling, "x.py"), "outside\n");
    // Every session in the folder starts from one observation cache, as with the runtime that serves the folder.
    const observations: SourceObservationCache = new Map();
    const open = async (sessionId: string) => {
      const audit = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
      const bridge = createBridge(root, {
        ...auditor(audit),
        sourceRoot: path => [...roots, sibling].find(known => resolve(root, path).startsWith(`${known}${sep}`)) ?? root,
        worktreeRoots: () => roots,
      }, { worktreeWalkGapMs: 0, observations });
      await bridge.openWorkspace(sessionId);
      await bridge.checkpoint("turn");
      await bridge.settleWorktreeWalk();
      // Each audit as its edits and its mode: `undefined` steers the session, `log` only records.
      return { bridge, audited: () => audit.mock.calls.map(([, batch]) => [batch.edits, batch.mode]) };
    };
    let calls = 0;
    const toolWrite = async (bridge: CodeRuleBridge, path: string, text: string) => {
      const id = `write-${++calls}`;
      await bridge.begin(id, "write", { path });
      await writeFile(path, text);
      await bridge.finish(id);
    };
    // The walk of the other worktrees, which a shell call does not wait for, finds what it wrote there.
    const shellWrite = async (bridge: CodeRuleBridge, path: string, text: string) => {
      const id = `shell-${++calls}`;
      await bridge.begin(id, "shell", { command: "./run.sh" });
      await writeFile(path, text);
      await bridge.finish(id);
      await bridge.settleWorktreeWalk();
      await bridge.checkpoint("turn");
      await bridge.settleWorktreeWalk();
    };
    const named = (absolute: string) => relative(root, absolute).split(sep).join("/");
    const edit = (path: string, previousText: string, generatedText: string) => ({ path, language: "python", previousText, generatedText });

    const first = await open("session-1");
    await toolWrite(first.bridge, join(sibling, "x.py"), "outside\nfirst\n");
    await shellWrite(first.bridge, join(worktree, "src", "new.py"), "violation\nfirst\n");
    await first.bridge.close();
    expect(first.audited()).toEqual([
      [[edit(named(join(sibling, "x.py")), "outside\n", "outside\nfirst\n")], undefined],
      [[edit(named(join(worktree, "src", "new.py")), "violation\n", "violation\nfirst\n")], "log"],
    ]);

    // Another process changes a file in each place while no session runs.
    await writeFile(join(worktree, "src", "a.py"), "old\nviolation\nbetween\n");
    await writeFile(join(root, "src", "a.py"), "old\nbetween\n");
    await writeFile(join(sibling, "x.py"), "outside\nfirst\nbetween\n");
    await awaitClockAfter(join(worktree, "src", "a.py"), join(root, "src", "a.py"), join(sibling, "x.py"));

    const second = await open("session-2");
    // What changed while no session ran is no session's edit.
    expect(second.audited()).toEqual([]);
    await toolWrite(second.bridge, join(worktree, "src", "a.py"), "old\nviolation\nbetween\nmine\n");
    await shellWrite(second.bridge, join(worktree, "src", "new.py"), "violation\nfirst\nmine\n");
    await shellWrite(second.bridge, join(root, "src", "a.py"), "old\nbetween\nmine\n");
    await toolWrite(second.bridge, join(sibling, "x.py"), "outside\nfirst\nbetween\nmine\n");
    expect(second.audited()).toEqual([
      [[edit(named(join(worktree, "src", "a.py")), "old\nviolation\nbetween\n", "old\nviolation\nbetween\nmine\n")], undefined],
      [[edit(named(join(worktree, "src", "new.py")), "violation\nfirst\n", "violation\nfirst\nmine\n")], "log"],
      [[edit("src/a.py", "old\nbetween\n", "old\nbetween\nmine\n")], undefined],
      [[edit(named(join(sibling, "x.py")), "outside\nfirst\nbetween\n", "outside\nfirst\nbetween\nmine\n")], undefined],
    ]);
  });

  it("keeps its own sources, and none of the worktrees inside its folder, in the observations it shares once it closes", async () => {
    const root = await twoBranches();
    // A folder of its own whose path starts with a worktree's: `wt/w1-notes` beside `wt/w1`.
    await mkdir(join(root, "wt", "w1-notes"), { recursive: true });
    await writeFile(join(root, "wt", "w1-notes", "a.py"), "notes\n");
    git(root, "add", ".");
    git(root, "commit", "--quiet", "-m", "notes");
    const roots = ["w1", "w2"].map(name => join(root, "wt", name));
    for (const place of roots) git(root, "worktree", "add", "--quiet", "--detach", place, "other");
    const observations: SourceObservationCache = new Map();
    const bridge = createBridge(root, {
      ...auditor(vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch))),
      sourceRoot: path => roots.find(known => resolve(root, path).startsWith(`${known}${sep}`)) ?? root,
      worktreeRoots: () => roots,
    }, { worktreeWalkGapMs: 0, observations });
    await bridge.openWorkspace("session-1");
    await bridge.checkpoint("turn");
    await bridge.settleWorktreeWalk();
    const own = ["src/a.py", "src/gone.py", "wt/w1-notes/a.py"];
    // The worktrees inside the folder are the shared walk's: the session keeps no observation of their sources.
    expect([...observations.get(root)?.keys() ?? []].sort()).toEqual(own);
    await bridge.close();
    expect([...observations.get(root)?.keys() ?? []].sort()).toEqual(own);
  });

  it("audits nothing when a worktree is removed, or of one Git adds later", async () => {
    const root = await twoBranches();
    const { worktree, roots, extra } = await withWorktree(root, join(await workspace(), "w"));
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    await shellInWorktree(bridge, () => {
      git(root, "worktree", "remove", worktree);
      roots.length = 0;
    });
    const later = join(await workspace(), "later");
    await shellInWorktree(bridge, async () => {
      git(root, "worktree", "add", "--quiet", later, "other");
      roots.push(await realpath(later));
    });
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
  });

  /** The paths `audit` was given, over every call. */
  const auditedPaths = (audit: ReturnType<typeof vi.fn>): string[] =>
    audit.mock.calls.flatMap(([, batch]) => (batch as CodeRuleEditBatch).edits.map(edit => edit.path));

  it("walks again for a boundary that came while a walk was running, so a write the walk had passed is seen", async () => {
    const root = await twoBranches();
    const { worktree, extra } = await withWorktree(root, join(await workspace(), "w"));
    await mkdir(join(worktree, "deep", "deeper"), { recursive: true });
    await writeFile(join(worktree, "top.py"), "old\n");
    await awaitClockAfter(join(worktree, "top.py"));
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    // The walk reads each level whole before the next, so this gate holds it after it has read `top.py`.
    const walk = holdDiscovery("directories", join(worktree, "deep", "deeper"));
    await bridge.checkpoint("turn");
    await walk.entered;
    await writeFile(join(worktree, "top.py"), "old\nviolation\n");
    await bridge.checkpoint("turn");
    walk.release();
    await bridge.settleWorktreeWalk();
    expect(auditedPaths(auditGeneratedEdits)).toEqual([repositoryPath(root, join(worktree, "top.py"))]);
  });

  it("takes what a walk could not read in a worktree as a baseline once it can, but for what was written since", async () => {
    const root = await twoBranches();
    const { worktree, extra } = await withWorktree(root, join(await workspace(), "w"));
    await mkdir(join(worktree, "deep"));
    await writeFile(join(worktree, "deep", "old.py"), "violation\n");
    await awaitClockAfter(join(worktree, "deep", "old.py"));
    discovery.directories.set(join(worktree, "deep"), [async () => { throw new Error("EMFILE: too many open files"); }]);
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    // The gap is the session's, as the session folder's own walk reports one.
    expect((await bridge.checkpoint("turn")).failures).toEqual([
      expect.objectContaining({ code: "source-incomplete", message: expect.stringContaining("EMFILE") }),
    ]);
    await bridge.settleWorktreeWalk();
    await writeFile(join(worktree, "deep", "new.py"), "violation\n");
    await bridge.checkpoint("turn");
    await bridge.settleWorktreeWalk();
    expect(auditedPaths(auditGeneratedEdits)).toEqual([repositoryPath(root, join(worktree, "deep", "new.py"))]);
    expect((await bridge.checkpoint("turn")).failures).toEqual([]);
  });

  it("makes no text of the files a worktree's first walk takes as its baseline, and records a later change from Git's text", async () => {
    const root = await twoBranches();
    const { worktree, extra } = await withWorktree(root, join(await workspace(), "w"));
    const files = Array.from({ length: 20 }, (_, index) => join(worktree, "src", `f${index}.py`));
    await Promise.all(files.map((file, index) => writeFile(file, `x = ${index}\n`)));
    git(worktree, "add", ".");
    git(worktree, "commit", "--quiet", "-m", "files");
    await awaitClockAfter(...files);
    decodedReads.count = 0;
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    expect(decodedReads.count).toBe(0);
    await shellInWorktree(bridge, () => writeFile(files[0]!, "x = 0\nmine\n"));
    const path = relative(root, files[0]!).split(sep).join("/");
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => [batch.edits, batch.mode])).toEqual([
      [[{ path, language: "python", previousText: "x = 0\n", generatedText: "x = 0\nmine\n" }], "log"],
    ]);
    // The one file that changed is the one read made into text.
    expect(decodedReads.count).toBe(1);
  });

  it("reads the files of a worktree's first walk into buffers it reuses, not a buffer of their own each", async () => {
    const root = await twoBranches();
    const { worktree, extra } = await withWorktree(root, join(await workspace(), "w"));
    const files = Array.from({ length: 64 }, (_, index) => join(worktree, "src", `f${index}.py`));
    await Promise.all(files.map((file, index) => writeFile(file, `x = ${index}\n`)));
    git(worktree, "add", ".");
    git(worktree, "commit", "--quiet", "-m", "files");
    await awaitClockAfter(...files);
    readBuffers.watching = true;
    await opened(root, extra);
    readBuffers.watching = false;
    // A walk runs 16 reads at once (OBSERVATION_CONCURRENCY), and two walked here: the session folder and the worktree.
    expect(readBuffers.seen.size).toBeGreaterThan(0);
    expect(readBuffers.seen.size).toBeLessThanOrEqual(2 * 16);
  });

  it("makes no text of the files a later worktree walk first sees, as its baseline, in a folder the last one could not read", async () => {
    const root = await twoBranches();
    const { worktree, extra } = await withWorktree(root, join(await workspace(), "w"));
    await mkdir(join(worktree, "deep"));
    const files = Array.from({ length: 20 }, (_, index) => join(worktree, "deep", `f${index}.py`));
    await Promise.all(files.map((file, index) => writeFile(file, `x = ${index}\n`)));
    git(worktree, "add", ".");
    git(worktree, "commit", "--quiet", "-m", "files");
    await awaitClockAfter(...files);
    discovery.directories.set(join(worktree, "deep"), [async () => { throw new Error("EMFILE: too many open files"); }]);
    decodedReads.count = 0;
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    // The walk that reads the folder comes after the worktree's first: only that folder's failure makes these a baseline.
    await bridge.checkpoint("turn");
    await bridge.settleWorktreeWalk();
    expect(decodedReads.count).toBe(0);
    await shellInWorktree(bridge, () => writeFile(files[0]!, "x = 0\nmine\n"));
    const path = relative(root, files[0]!).split(sep).join("/");
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => [batch.edits, batch.mode])).toEqual([
      [[{ path, language: "python", previousText: "x = 0\n", generatedText: "x = 0\nmine\n" }], "log"],
    ]);
  });

  it("records another writer's change in a worktree Git added during the session, made before that worktree's first walk", async () => {
    const root = await twoBranches();
    const roots: string[] = [];
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, {
      ...auditor(auditGeneratedEdits),
      sourceRoot: path => roots.find(known => resolve(root, path).startsWith(`${known}${sep}`)) ?? root,
      worktreeRoots: () => roots,
    }, { worktreeWalkGapMs: 0 });
    await bridge.openWorkspace("session-1");
    await bridge.checkpoint("turn");
    await bridge.settleWorktreeWalk();
    const place = join(await workspace(), "w");
    git(root, "worktree", "add", "--quiet", place, "other");
    const worktree = await realpath(place);
    await writeFile(join(worktree, "src", "a.py"), "old\nviolation\nmine\n");
    roots.push(worktree);
    await shellInWorktree(bridge, () => undefined);
    const path = relative(root, join(worktree, "src", "a.py")).split(sep).join("/");
    // Whether Git's own write of src/new.py is recorded too depends on how its checkout wrote line breaks.
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => [batch.edits, batch.mode])).toEqual([[expect.arrayContaining([
      { path, language: "python", previousReadError: "Source changed during initial discovery without a captured preimage", generatedText: "old\nviolation\nmine\n" },
    ]), "log"]]);
  });

  it("records a file a shell command moved into another worktree, stamped before the session, and measures its next edit from that text", async () => {
    const root = await twoBranches();
    const { worktree, extra } = await withWorktree(root, join(await workspace(), "w"));
    const outside = await workspace();
    await mkdir(join(outside, "pkg"));
    const moved = join(outside, "pkg", "m.py");
    await writeFile(moved, "moved\n");
    await awaitClockAfter(moved);
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    await shellInWorktree(bridge, () => rename(join(outside, "pkg"), join(worktree, "src", "pkg")));
    await shellInWorktree(bridge, () => writeFile(join(worktree, "src", "pkg", "m.py"), "moved\nmine\n"));
    const path = relative(root, join(worktree, "src", "pkg", "m.py")).split(sep).join("/");
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => [batch.edits, batch.mode])).toEqual([
      [[{ path, language: "python", generatedText: "moved\n" }], "log"],
      [[{ path, language: "python", previousText: "moved\n", generatedText: "moved\nmine\n" }], "log"],
    ]);
  });

  it("reports a worktree walk that never ran as pending when the session closes, and none with no worktree to walk", async () => {
    const root = await twoBranches();
    const { extra } = await withWorktree(root, join(await workspace(), "w"));
    const pending: string[] = [];
    const bridge = createBridge(root, { ...auditor(vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch))), ...extra }, {
      worktreeWalkGapMs: 60_000, onDiscoveryPending: boundary => { pending.push(boundary); },
    });
    await bridge.openWorkspace("session-1");
    await bridge.checkpoint("turn");
    await bridge.settleWorktreeWalk();
    // The next walk is timed a minute out; the session closes first.
    await bridge.checkpoint("turn");
    await bridge.close();
    expect(pending).toEqual(["close"]);

    const alone = createBridge(await twoBranches(), { ...auditor(vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch))), worktreeRoots: () => [] }, {
      worktreeWalkGapMs: 60_000, onDiscoveryPending: boundary => { pending.push(`alone:${boundary}`); },
    });
    await alone.openWorkspace("session-2");
    await alone.checkpoint("turn");
    await alone.checkpoint("turn");
    await alone.close();
    expect(pending).toEqual(["close"]);
  });

  it("reports no deletions for a nested worktree the other walk dropped while the session's walk ran", async () => {
    const root = await twoBranches();
    const { worktree, roots, extra } = await withWorktree(root, join(root, "wt"));
    const { bridge, auditGeneratedEdits } = await opened(root, extra, 200);
    // The worktree walk this turn times runs only when the test moves the clock, once the next session walk has started.
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      await bridge.checkpoint("turn");
      await bridge.begin("shell-1", "shell", { command: "./write.sh &" });
      await writeFile(join(root, "src", "hold.py"), "mine\n");
      const walk = holdDiscovery("reads", join(root, "src", "hold.py"));
      await requestDiscovery(bridge, walk);
      git(root, "worktree", "remove", worktree);
      roots.length = 0;
      // The walk is timed the gap or the last walk's length after that walk, whichever is longer: on a slow machine, as
      // Git on Windows is, longer than the gap. The checkpoint's wait and Git's timeouts come later, so it is the next timer.
      await vi.advanceTimersToNextTimerAsync();
      await bridge.settleWorktreeWalk();
      walk.release();
      await bridge.checkpoint("turn");
    } finally {
      vi.useRealTimers();
    }
    await expect(bridge.finish("shell-1")).resolves.toEqual([]);
    expect(auditedPaths(auditGeneratedEdits)).toEqual(["src/hold.py"]);
  });

  it("reports no deletions for a worktree the session's walk dropped while the worktree walk ran", async () => {
    const root = await twoBranches();
    const { worktree, roots, extra } = await withWorktree(root, join(await workspace(), "w"));
    await mkdir(join(worktree, "deep"));
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    const walk = holdDiscovery("directories", join(worktree, "deep"));
    await bridge.checkpoint("turn");
    await walk.entered;
    git(root, "worktree", "remove", "--force", worktree);
    roots.length = 0;
    await bridge.checkpoint("turn");
    walk.release();
    await bridge.settleWorktreeWalk();
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
  });

  it("records nothing of a worktree nested in another walked worktree and removed during its first walk", async () => {
    const root = await twoBranches();
    const { worktree, roots, extra } = await withWorktree(root, join(await workspace(), "w"));
    git(root, "worktree", "add", "--quiet", "-b", "nested", join(worktree, "nest"), "main");
    const nested = await realpath(join(worktree, "nest"));
    // A walk opens one level of folders at a time, so it opens this folder of the outer worktree after it has read the
    // nested worktree's files: a gate here holds the walk between those reads and the audit of what it found.
    const deepest = join(worktree, "deep", "deeper", "deepest");
    await mkdir(deepest, { recursive: true });
    await awaitClockAfter(join(nested, "src", "a.py"), join(nested, "src", "gone.py"));
    const nest = relative(root, nested).split(sep).join("/");
    const { bridge, auditGeneratedEdits } = await opened(root, {
      ...extra,
      sourceRoot: path => roots.filter(known => resolve(root, path).startsWith(`${known}${sep}`)).sort((a, b) => b.length - a.length)[0] ?? root,
      // The outer worktree ignores the folder that holds the nested one, as a main checkout often ignores its
      // worktrees' folder, so the outer worktree's walk never reads the nested worktree's files.
      ignoresPath: (path, directory) => directory && path === nest,
    });
    roots.push(nested);
    const walk = holdDiscovery("directories", deepest);
    await bridge.checkpoint("turn");
    await walk.entered;
    git(root, "worktree", "remove", "--force", nested);
    roots.splice(roots.indexOf(nested), 1);
    // The session folder's walk finds the worktree gone: the held walk no longer takes its files as a baseline.
    await bridge.checkpoint("turn");
    walk.release();
    await bridge.settleWorktreeWalk();
    await bridge.checkpoint("turn");
    await bridge.settleWorktreeWalk();
    // No one wrote these files. A walk that kept their text recorded each one created, then deleted; a session that kept
    // the removed worktree's first walk as its own observations recorded each one deleted. The shared walk's snapshot of
    // a worktree gone before the session took it is never the session's, so a removed worktree stays silent.
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
  });

  it("reports no deletions for a worktree removed while its own walk ran, with no other walk between", async () => {
    const root = await twoBranches();
    const { worktree, roots, extra } = await withWorktree(root, join(await workspace(), "w"));
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    const walk = holdDiscovery("directories", join(worktree, "src"));
    await bridge.checkpoint("turn");
    await walk.entered;
    git(root, "worktree", "remove", worktree);
    roots.length = 0;
    walk.release();
    await bridge.settleWorktreeWalk();
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
  });
});

describe("CodeRuleBridge and Git's own writes under core.autocrlf", () => {
  /** `twoBranches` with `core.autocrlf=true`, as Git for Windows' installer sets it, its files checked out again in CRLF. */
  async function crlfBranches(): Promise<string> {
    const root = await twoBranches();
    git(root, "config", "core.autocrlf", "true");
    await rm(join(root, "src", "a.py"));
    await rm(join(root, "src", "gone.py"));
    git(root, "checkout", "--", ".");
    await awaitClockAfter(join(root, "src", "a.py"), join(root, "src", "gone.py"));
    return root;
  }

  it("audits nothing a checkout wrote with CRLF line breaks", async () => {
    const root = await crlfBranches();
    const { bridge, auditGeneratedEdits } = await opened(root);
    await expect(shell(bridge, () => git(root, "checkout", "--quiet", "other"))).resolves.toEqual([]);
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
  });

  it("audits nothing restored from another branch with CRLF line breaks", async () => {
    const root = await crlfBranches();
    const { bridge, auditGeneratedEdits } = await opened(root);
    await expect(shell(bridge, () => git(root, "checkout", "other", "--", "src/a.py"))).resolves.toEqual([]);
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
  });

  it("audits a shell write with an LF line a checkout never writes, though another branch holds its text with LF", async () => {
    const root = await crlfBranches();
    const { bridge, auditGeneratedEdits } = await opened(root);
    // `other` holds "old\nviolation\n", which a checkout here writes with a CRLF after each line.
    await shell(bridge, () => writeFile(join(root, "src", "a.py"), "old\r\nviolation\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/a.py", language: "python", previousText: "old\r\n", generatedText: "old\r\nviolation\n" }],
    ]);
  });

  it("gives a shell write the preimage Git holds, in the CRLF line breaks the file held it in", async () => {
    const root = await crlfBranches();
    const { bridge, auditGeneratedEdits } = await opened(root);
    await shell(bridge, () => writeFile(join(root, "src", "a.py"), "old\r\nmine\r\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/a.py", language: "python", previousText: "old\r\n", generatedText: "old\r\nmine\r\n" }],
    ]);
  });

  it("audits a shell write that gives an LF file CRLF line breaks where Git would never write them", async () => {
    const root = await twoBranches();
    // As on macOS and Linux, and on Windows set up to check files out as they are stored.
    git(root, "config", "core.autocrlf", "false");
    git(root, "config", "core.eol", "lf");
    const { bridge, auditGeneratedEdits } = await opened(root);
    await shell(bridge, () => writeFile(join(root, "src", "a.py"), "old\r\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/a.py", language: "python", previousText: "old\n", generatedText: "old\r\n" }],
    ]);
  });

  it("credits Git with CRLF line breaks an eol attribute or a text file under core.eol=crlf has it write", async () => {
    for (const setup of [
      { attributes: "*.py eol=crlf\n", eol: "lf" },
      { attributes: "*.py text\n", eol: "crlf" },
    ]) {
      const root = await twoBranches();
      git(root, "config", "core.autocrlf", "false");
      git(root, "config", "core.eol", setup.eol);
      await writeFile(join(root, ".git", "info", "attributes"), setup.attributes);
      await rm(join(root, "src", "a.py"));
      git(root, "checkout", "--", "src/a.py");
      await awaitClockAfter(join(root, "src", "a.py"));
      const { bridge, auditGeneratedEdits } = await opened(root);
      await expect(shell(bridge, () => git(root, "checkout", "--quiet", "other")), setup.attributes).resolves.toEqual([]);
      expect(auditGeneratedEdits, setup.attributes).not.toHaveBeenCalled();
    }
  });

  it("credits Git with the CRLF line breaks a text or eol=crlf attribute has it write into a blob that already holds a CR", async () => {
    for (const setup of [
      { attributes: "*.py eol=crlf\n", autocrlf: "false" },
      // Git for Windows' default core.autocrlf with a text attribute.
      { attributes: "*.py text\n", autocrlf: "true" },
    ]) {
      const root = await twoBranches();
      // Committed with no attribute, so the blobs keep their CRs: a CRLF beside an LF, and a lone CR.
      git(root, "switch", "--quiet", "other");
      await writeFile(join(root, "src", "a.py"), "old\r\nviolation\n");
      await writeFile(join(root, "src", "new.py"), "violation\rmore\n");
      git(root, "commit", "--quiet", "-am", "carriage returns");
      git(root, "switch", "--quiet", "main");
      git(root, "config", "core.autocrlf", setup.autocrlf);
      await writeFile(join(root, ".git", "info", "attributes"), setup.attributes);
      await rm(join(root, "src", "a.py"));
      git(root, "checkout", "--", "src/a.py");
      await awaitClockAfter(join(root, "src", "a.py"));
      const { bridge, auditGeneratedEdits } = await opened(root);
      // A checkout writes every LF without its CR as CRLF here, whatever else the blob holds.
      await expect(shell(bridge, () => git(root, "checkout", "--quiet", "other")), setup.attributes).resolves.toEqual([]);
      expect(auditGeneratedEdits, setup.attributes).not.toHaveBeenCalled();
    }
  });
});

describe("CodeRuleBridge and Git's own writes", () => {
  it.each([
    ["checkout", ["checkout", "--quiet", "other"]],
    ["switch", ["switch", "--quiet", "other"]],
    ["a fast-forward merge", ["merge", "--quiet", "--ff-only", "other"]],
    ["a rebase", ["rebase", "--quiet", "other"]],
    ["a hard reset", ["reset", "--quiet", "--hard", "other"]],
  ])("audits nothing %s rewrote", async (_name, command) => {
    const root = await twoBranches();
    const { bridge, auditGeneratedEdits } = await opened(root);
    await expect(shell(bridge, () => git(root, ...command))).resolves.toEqual([]);
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
  });

  it.each([
    ["a shell redirect of its blob", (root: string) => writeFile(join(root, "src", "a.py"), execFileSync("git", ["show", "other:src/a.py"], { cwd: root }))],
    ["a shell redirect of a remote-tracking branch's blob", async (root: string) => {
      git(root, "update-ref", "refs/remotes/origin/other", "other");
      git(root, "branch", "--quiet", "-D", "other");
      await writeFile(join(root, "src", "a.py"), execFileSync("git", ["show", "origin/other:src/a.py"], { cwd: root }));
    }],
    ["a checkout of its path", (root: string) => git(root, "checkout", "other", "--", "src/a.py")],
    ["a restore from its source", (root: string) => git(root, "restore", "--source", "other", "src/a.py")],
    // Rerail cannot tell a write of the same bytes from a restore, and takes it for one (docs/reference.md).
    ["a plain write of exactly its text", (root: string) => writeFile(join(root, "src", "a.py"), "old\nviolation\n")],
  ])("audits nothing restored from another branch by %s, and measures the next shell edit from it", async (_name, restore) => {
    const root = await twoBranches();
    const { bridge, auditGeneratedEdits } = await opened(root);
    await expect(shell(bridge, () => restore(root))).resolves.toEqual([]);
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
    await shell(bridge, () => writeFile(join(root, "src", "a.py"), "old\nviolation\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/a.py", language: "python", previousText: "old\nviolation\n", generatedText: "old\nviolation\nmine\n" }],
    ]);
  });

  it("audits a file restored from a branch its own command committed it to", async () => {
    const root = await twoBranches();
    const { bridge, auditGeneratedEdits } = await opened(root);
    await shell(bridge, async () => {
      await writeFile(join(root, "src", "a.py"), "old\nmine\n");
      git(root, "commit", "--quiet", "-am", "mine");
      git(root, "branch", "backup");
    });
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/a.py", language: "python", previousText: "old\n", generatedText: "old\nmine\n" }],
    ]);
  });

  it("audits text a branch holds only at another path", async () => {
    const root = await twoBranches();
    const { bridge, auditGeneratedEdits } = await opened(root);
    await shell(bridge, () => writeFile(join(root, "src", "b.py"), "old\nviolation\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/b.py", language: "python", generatedText: "old\nviolation\n" }],
    ]);
  });

  it("reports only its own line for a shell edit after a checkout", async () => {
    const root = await twoBranches();
    const { bridge, auditGeneratedEdits } = await opened(root);
    await shell(bridge, () => git(root, "checkout", "--quiet", "other"));
    await shell(bridge, () => writeFile(join(root, "src", "a.py"), "old\nviolation\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/a.py", language: "python", previousText: "old\nviolation\n", generatedText: "old\nviolation\nmine\n" }],
    ]);
  });

  it("does not take another session's checkout for this session's writes", async () => {
    const root = await twoBranches();
    const { bridge, auditGeneratedEdits } = await opened(root);
    git(root, "checkout", "--quiet", "other");
    await bridge.checkpoint("turn");
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
  });

  it("keeps auditing a file written and committed in one command, against the tree before the commit", async () => {
    const root = await twoBranches();
    const { bridge, auditGeneratedEdits } = await opened(root);
    await shell(bridge, async () => {
      await writeFile(join(root, "src", "a.py"), "old\nmine\n");
      git(root, "commit", "--quiet", "-am", "mine");
    });
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/a.py", language: "python", previousText: "old\n", generatedText: "old\nmine\n" }],
    ]);
  });

  it("measures a file written in the command that moved the checkout from what Git wrote", async () => {
    const root = await twoBranches();
    const { bridge, auditGeneratedEdits } = await opened(root);
    await shell(bridge, async () => {
      git(root, "checkout", "--quiet", "other");
      await writeFile(join(root, "src", "a.py"), "old\nviolation\nmine\n");
    });
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/a.py", language: "python", previousText: "old\nviolation\n", generatedText: "old\nviolation\nmine\n" }],
    ]);
  });

  it("still tells the auditor, with no generated lines and in log mode, of a file this session had changed", async () => {
    const root = await twoBranches();
    const { bridge, auditGeneratedEdits } = await opened(root);
    await shell(bridge, () => writeFile(join(root, "src", "a.py"), "old\nmine\n"));
    await shell(bridge, () => git(root, "restore", "src/a.py"));
    expect(auditGeneratedEdits.mock.calls.at(-1)?.[1]).toMatchObject({
      mode: "log",
      edits: [{ path: "src/a.py", language: "python", previousText: "old\n", generatedText: "old\n" }],
    });
    expect(auditGeneratedEdits).toHaveBeenCalledTimes(2);
  });

  it("audits nothing a merge wrote when it stopped on a conflict, and a later shell edit from the conflicted text", async () => {
    const root = await twoBranches();
    await writeFile(join(root, "src", "a.py"), "old\nours\n");
    git(root, "commit", "--quiet", "-am", "ours");
    await awaitClockAfter(join(root, "src", "a.py"));
    const { bridge, auditGeneratedEdits } = await opened(root);
    await shell(bridge, () => { try { git(root, "merge", "--quiet", "other"); } catch { /* The conflict is the point. */ } });
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
    await shell(bridge, () => writeFile(join(root, "src", "a.py"), "old\nresolved\n"));
    const [edit] = auditGeneratedEdits.mock.calls.at(-1)![1].edits;
    expect(edit).toMatchObject({ path: "src/a.py", generatedText: "old\nresolved\n" });
    expect(edit?.previousText).toContain("<<<<<<<");
    expect(edit?.previousReadError).toBeUndefined();
  });

  it("audits nothing a merge wrote when it stopped on a modify/delete conflict, and a later shell edit from our text", async () => {
    const root = await twoBranches();
    await writeFile(join(root, "src", "gone.py"), "gone\nours\n");
    git(root, "commit", "--quiet", "-am", "ours");
    await awaitClockAfter(join(root, "src", "gone.py"));
    const { bridge, auditGeneratedEdits } = await opened(root);
    await shell(bridge, () => { try { git(root, "merge", "--quiet", "other"); } catch { /* The conflict is the point. */ } });
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
    await shell(bridge, () => writeFile(join(root, "src", "gone.py"), "gone\nours\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/gone.py", language: "python", previousText: "gone\nours\n", generatedText: "gone\nours\nmine\n" }],
    ]);
  });

  it("audits a shell edit of a conflicted file that holds our text again, from that text", async () => {
    const root = await twoBranches();
    await writeFile(join(root, "src", "a.py"), "old\nours\n");
    git(root, "commit", "--quiet", "-am", "ours");
    await awaitClockAfter(join(root, "src", "a.py"));
    const { bridge, auditGeneratedEdits } = await opened(root);
    await shell(bridge, () => { try { git(root, "merge", "--quiet", "other"); } catch { /* The conflict is the point. */ } });
    await shell(bridge, () => git(root, "checkout", "--ours", "src/a.py"));
    await shell(bridge, () => writeFile(join(root, "src", "a.py"), "old\nours\nmine\n"));
    const batch = auditGeneratedEdits.mock.calls.at(-1)![1];
    expect(batch.mode).not.toBe("log");
    expect(batch.edits).toEqual([{ path: "src/a.py", language: "python", previousText: "old\nours\n", generatedText: "old\nours\nmine\n" }]);
  });

  it("keeps the last unmerged entries it read through a pass where Git could not list them", async () => {
    const root = await twoBranches();
    await writeFile(join(root, "src", "a.py"), "old\nours\n");
    git(root, "commit", "--quiet", "-am", "ours");
    await awaitClockAfter(join(root, "src", "a.py"));
    const { bridge, auditGeneratedEdits } = await opened(root);
    await shell(bridge, () => { try { git(root, "merge", "--quiet", "other"); } catch { /* The conflict is the point. */ } });
    // The index changes, so the pass asks Git for the entries, and Git fails.
    asyncGitCalls.failing = "--unmerged";
    try {
      await shell(bridge, async () => {
        git(root, "checkout", "--ours", "src/a.py");
        const later = new Date(Date.now() + 5_000);
        await utimes(join(root, ".git", "index"), later, later);
      });
    } finally {
      asyncGitCalls.failing = undefined;
    }
    expect(asyncGitCalls.args.some(args => args.includes("--no-such-option"))).toBe(true);
    await shell(bridge, () => writeFile(join(root, "src", "a.py"), "old\nours\nmine\n"));
    const batch = auditGeneratedEdits.mock.calls.at(-1)![1];
    expect(batch.mode).not.toBe("log");
    expect(batch.edits).toEqual([{ path: "src/a.py", language: "python", previousText: "old\nours\n", generatedText: "old\nours\nmine\n" }]);
  });

  it("takes a second conflict on a file still unmerged since the last walk as Git's", async () => {
    const root = await twoBranches();
    git(root, "switch", "--quiet", "-c", "second");
    await writeFile(join(root, "src", "a.py"), "old\nsecond\n");
    git(root, "commit", "--quiet", "-am", "second");
    git(root, "switch", "--quiet", "main");
    await writeFile(join(root, "src", "a.py"), "old\nours\n");
    git(root, "commit", "--quiet", "-am", "ours");
    await awaitClockAfter(join(root, "src", "a.py"));
    const { bridge, auditGeneratedEdits } = await opened(root);
    await shell(bridge, () => { try { git(root, "merge", "--quiet", "other"); } catch { /* The conflict is the point. */ } });
    await shell(bridge, () => writeFile(join(root, "src", "a.py"), "old\nresolved\n"));
    await shell(bridge, () => {
      git(root, "add", ".");
      git(root, "commit", "--quiet", "-m", "merged");
      try { git(root, "merge", "--quiet", "second"); } catch { /* The conflict is the point. */ }
    });
    const batch = auditGeneratedEdits.mock.calls.at(-1)![1];
    expect(batch.mode).toBe("log");
    expect(batch.edits[0]?.generatedText).toContain("<<<<<<<");
  });

  it("starts no Git process for the unmerged entries until the index file changes", async () => {
    const root = await twoBranches();
    const { bridge, auditGeneratedEdits } = await opened(root);
    // A pass that finds nothing, then one that finds a shell edit: neither wrote the index.
    expect(await gitReads(() => bridge.checkpoint("turn"))).toEqual({ heads: 1, entries: 0 });
    expect((await gitReads(() => shell(bridge, () => writeFile(join(root, "src", "a.py"), "old\nmine\n")))).entries).toBe(0);
    expect(auditGeneratedEdits).toHaveBeenCalledTimes(1);
    // The pass after an index write reads it once, and the one after that does not.
    expect((await gitReads(() => shell(bridge, () => git(root, "add", "src/a.py")))).entries).toBe(1);
    expect(await gitReads(() => bridge.checkpoint("turn"))).toEqual({ heads: 1, entries: 0 });
  });

  it("reads one checkout each pass for a session opened in a subfolder of it, once a walk has found a change there too", async () => {
    const root = await twoBranches();
    const { bridge, auditGeneratedEdits } = await opened(join(root, "src"), { sourceRoot: () => root });
    await shell(bridge, () => writeFile(join(root, "src", "a.py"), "old\nmine\n"));
    expect(auditGeneratedEdits).toHaveBeenCalledTimes(1);
    asyncGitCalls.args.length = 0;
    await bridge.checkpoint("turn");
    expect(asyncGitCalls.args.filter(args => args.includes("--verify"))).toHaveLength(1);
  });

  it("audits a shell edit of a file a modify/delete conflict left as it was, where the conflict's pass found no change", async () => {
    const root = await workspace();
    await writeFile(join(root, "src", "gone.py"), "gone\n");
    git(root, "init", "--quiet", "--initial-branch=main");
    git(root, "add", ".");
    git(root, "commit", "--quiet", "-m", "seed");
    git(root, "switch", "--quiet", "-c", "other");
    await rm(join(root, "src", "gone.py"));
    git(root, "commit", "--quiet", "-am", "other");
    git(root, "switch", "--quiet", "main");
    await writeFile(join(root, "src", "gone.py"), "gone\nours\n");
    git(root, "commit", "--quiet", "-am", "ours");
    await awaitClockAfter(join(root, "src", "gone.py"));
    const { bridge, auditGeneratedEdits } = await opened(root);
    await shell(bridge, () => { try { git(root, "merge", "--quiet", "other"); } catch { /* The conflict is the point. */ } });
    await shell(bridge, () => writeFile(join(root, "src", "gone.py"), "gone\nours\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/gone.py", language: "python", previousText: "gone\nours\n", generatedText: "gone\nours\nmine\n" }],
    ]);
    expect(auditGeneratedEdits.mock.calls[0]?.[1].mode).not.toBe("log");
  });

  it("audits a shell edit of a file a modify/delete conflict left as it was, in a session opened in a subfolder of the checkout", async () => {
    const root = await workspace();
    await writeFile(join(root, "src", "gone.py"), "gone\n");
    git(root, "init", "--quiet", "--initial-branch=main");
    git(root, "add", ".");
    git(root, "commit", "--quiet", "-m", "seed");
    git(root, "switch", "--quiet", "-c", "other");
    await rm(join(root, "src", "gone.py"));
    git(root, "commit", "--quiet", "-am", "other");
    git(root, "switch", "--quiet", "main");
    await writeFile(join(root, "src", "gone.py"), "gone\nours\n");
    git(root, "commit", "--quiet", "-am", "ours");
    await awaitClockAfter(join(root, "src", "gone.py"));
    const { bridge, auditGeneratedEdits } = await opened(join(root, "src"), { sourceRoot: () => root });
    await shell(bridge, () => { try { git(root, "merge", "--quiet", "other"); } catch { /* The conflict is the point. */ } });
    await shell(bridge, () => writeFile(join(root, "src", "gone.py"), "gone\nours\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "gone.py", language: "python", previousText: "gone\nours\n", generatedText: "gone\nours\nmine\n" }],
    ]);
  });

  it("takes no conflict a checkout already had when it is first read for a merge that just stopped", async () => {
    const root = await twoBranches();
    git(root, "switch", "--quiet", "-c", "ours");
    await writeFile(join(root, "src", "a.py"), "old\nours\n");
    git(root, "commit", "--quiet", "-am", "ours");
    git(root, "switch", "--quiet", "main");
    // A checkout inside the session's folder, stopped on a conflict in `src/a.py` before the session opens.
    const worktree = join(root, "wt");
    git(root, "worktree", "add", "--quiet", "wt", "ours");
    try { git(worktree, "merge", "--quiet", "other"); } catch { /* The conflict is the point. */ }
    await writeFile(join(worktree, "src", "new.py"), "staged\n");
    git(worktree, "add", "src/new.py");
    await writeFile(join(worktree, "src", "new.py"), "staged\nlocal\n");
    await awaitClockAfter(join(worktree, "src", "new.py"));
    // Git cannot list the entries while the session opens, so no walk has that checkout's until the pass that finds a change in it.
    asyncGitCalls.failing = "--unmerged";
    const { bridge, auditGeneratedEdits } = await opened(root, {
      sourceRoot: path => resolve(root, path).startsWith(`${worktree}${sep}`) ? worktree : root,
    }).finally(() => { asyncGitCalls.failing = undefined; });
    // That pass is the first read of its entries: the file holding its index blob is no work of the merge.
    await shell(bridge, () => writeFile(join(worktree, "src", "new.py"), "staged\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "wt/src/new.py", language: "python", previousText: "staged\nlocal\n", generatedText: "staged\n" }],
    ]);
  });

  it("audits a shell edit of a file a modify/delete conflict left as it was, in a checkout inside the session's folder", async () => {
    const { root, nested, extra } = await nestedCheckout();
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    await shell(bridge, () => { try { git(nested, "merge", "--quiet", "other"); } catch { /* The conflict is the point. */ } });
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
    await shell(bridge, () => writeFile(join(nested, "src", "gone.py"), "gone\nours\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "lib/src/gone.py", language: "python", previousText: "gone\nours\n", generatedText: "gone\nours\nmine\n" }],
    ]);
    expect(auditGeneratedEdits.mock.calls[0]?.[1].mode).not.toBe("log");
  });

  it("audits nothing a merge wrote beside a modify/delete conflict, in a worktree of another repository inside the session's folder", async () => {
    const { root, nested, extra } = await nestedCheckout({ both: true, linked: true });
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    await shell(bridge, () => { try { git(nested, "merge", "--quiet", "other"); } catch { /* The conflict is the point. */ } });
    expect(await readFile(join(nested, "src", "both.py"), "utf8")).toBe("theirs\n2\n3\n4\n5\n6\nours\n");
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
  });

  it("starts no Git process for a checkout inside the session's folder until its index file changes", async () => {
    const { root, nested, extra } = await nestedCheckout();
    const { bridge } = await opened(root, extra);
    // A pass that finds nothing reads the session checkout's `HEAD`, and nothing of the one inside it.
    expect(await gitReads(() => bridge.checkpoint("turn"))).toEqual({ heads: 1, entries: 0 });
    // The pass after a write to its index lists its entries once, and the one after that does not.
    const stopped = () => shell(bridge, () => { try { git(nested, "merge", "--quiet", "other"); } catch { /* The conflict is the point. */ } });
    expect(await gitReads(stopped)).toEqual({ heads: 1, entries: 1 });
    expect(await gitReads(() => bridge.checkpoint("turn"))).toEqual({ heads: 1, entries: 0 });
  });

  it("audits nothing of a worktree added inside the session's folder", async () => {
    const root = await twoBranches();
    const worktree = join(root, "wt");
    const { bridge, auditGeneratedEdits } = await opened(root, {
      sourceRoot: path => resolve(root, path).startsWith(`${worktree}${sep}`) ? worktree : root,
    });
    await shell(bridge, () => git(root, "worktree", "add", "--quiet", "wt", "other"));
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
  });

  it("reports only its own line for a shell edit in a worktree inside the session's folder, from that worktree's blob", async () => {
    const root = await twoBranches();
    const worktree = join(root, "wt");
    const { bridge, auditGeneratedEdits } = await opened(root, {
      sourceRoot: path => resolve(root, path).startsWith(`${worktree}${sep}`) ? worktree : root,
    });
    await shell(bridge, () => git(root, "worktree", "add", "--quiet", "wt", "other"));
    await shell(bridge, () => writeFile(join(worktree, "src", "a.py"), "old\nviolation\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "wt/src/a.py", language: "python", previousText: "old\nviolation\n", generatedText: "old\nviolation\nmine\n" }],
    ]);
  });
});

describe("CodeRuleBridge and another session's writes", () => {
  /** Two sessions open on `root` in this process, sharing its observations as the runtime does, with their baselines taken. */
  async function twoSessions(root: string) {
    const observations: SourceObservationCache = new Map();
    const open = async (sessionId: string) => {
      const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
      const bridge = createBridge(root, auditor(auditGeneratedEdits), { observations });
      await bridge.openWorkspace(sessionId);
      await bridge.checkpoint("turn");
      return { bridge, auditGeneratedEdits };
    };
    return [await open("session-a"), await open("session-b")] as const;
  }
  const mine = { path: "src/a.py", language: "python", previousText: "old\n", generatedText: "old\nmine\n" };
  /**
   * Waits until the filesystem clock has passed every window sampled so far: on Linux a window's end can share the next
   * write's tick, so what is written next is stamped strictly after the last call ended.
   */
  async function awaitClockPastNow(): Promise<void> {
    const probe = await mkdtemp(join(tmpdir(), "rerail-policy-now-"));
    directories.push(probe);
    await awaitClockAfter(probe);
  }

  it("checks a shell write for the session that ran it, and not for one whose read-only command or turn end came after", async () => {
    const root = await twoBranches();
    const [a, b] = await twoSessions(root);
    await shell(a.bridge, () => appendFile(join(root, "src", "a.py"), "mine\n"));
    await awaitClockPastNow();
    await expect(shell(b.bridge, () => undefined)).resolves.toEqual([]);
    await b.bridge.checkpoint("turn");
    expect(a.auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([[mine]]);
    expect(b.auditGeneratedEdits).not.toHaveBeenCalled();
  });

  it("does not take a write made while another session's command runs for its own", async () => {
    const root = await twoBranches();
    const [a, b] = await twoSessions(root);
    await a.bridge.begin("shell-a", "shell", { command: "./long.sh" });
    await appendFile(join(root, "src", "a.py"), "mine\n");
    await awaitClockPastNow();
    await shell(b.bridge, () => undefined);
    await a.bridge.finish("shell-a");
    expect(b.auditGeneratedEdits).not.toHaveBeenCalled();
    expect(a.auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([[mine]]);
  });

  it("tells nobody of a file only the other session changed, however often it changes it", async () => {
    const root = await twoBranches();
    const [a, b] = await twoSessions(root);
    for (const line of ["first\n", "second\n", "third\n"]) {
      await shell(a.bridge, () => appendFile(join(root, "src", "a.py"), line));
      await awaitClockPastNow();
      await shell(b.bridge, () => undefined);
    }
    await b.bridge.checkpoint("turn");
    expect(b.auditGeneratedEdits).not.toHaveBeenCalled();
  });

  it("measures a later shell edit of the file from the text the other session left once a walk has seen it", async () => {
    const root = await twoBranches();
    const [a, b] = await twoSessions(root);
    await shell(a.bridge, () => appendFile(join(root, "src", "a.py"), "mine\n"));
    await awaitClockPastNow();
    await shell(b.bridge, () => undefined);
    await shell(b.bridge, () => appendFile(join(root, "src", "a.py"), "theirs\n"));
    expect(b.auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/a.py", language: "python", previousText: "old\nmine\n", generatedText: "old\nmine\ntheirs\n" }],
    ]);
  });

  it("tells the auditor, with no generated lines and in log mode, of a file it had changed that the other session then changed", async () => {
    const root = await twoBranches();
    const [a, b] = await twoSessions(root);
    await shell(b.bridge, () => appendFile(join(root, "src", "a.py"), "theirs\n"));
    await awaitClockPastNow();
    await shell(a.bridge, () => appendFile(join(root, "src", "a.py"), "mine\n"));
    await b.bridge.checkpoint("turn");
    expect(b.auditGeneratedEdits.mock.calls.at(-1)?.[1]).toMatchObject({
      mode: "log",
      edits: [{ path: "src/a.py", language: "python", previousText: "old\ntheirs\nmine\n", generatedText: "old\ntheirs\nmine\n" }],
    });
    expect(b.auditGeneratedEdits).toHaveBeenCalledTimes(2);
  });

  it("takes a removal for the session whose command made it, by its folder's stamp", async () => {
    const root = await twoBranches();
    const [a, b] = await twoSessions(root);
    await shell(a.bridge, () => rm(join(root, "src", "gone.py")));
    await awaitClockPastNow();
    await shell(b.bridge, () => undefined);
    expect(a.auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/gone.py", language: "python", previousReadError: "Source was deleted without a captured preimage", deleted: true }],
    ]);
    expect(b.auditGeneratedEdits).not.toHaveBeenCalled();
  });

  it("takes nothing written while none of its calls ran for its own, and measures its next shell edit of the file from it", async () => {
    const root = await twoBranches();
    const [, b] = await twoSessions(root);
    // A process a command left running, or an editor: no call of the session ran.
    await appendFile(join(root, "src", "a.py"), "background\n");
    await awaitClockPastNow();
    await b.bridge.checkpoint("turn");
    expect(b.auditGeneratedEdits).not.toHaveBeenCalled();
    await shell(b.bridge, () => appendFile(join(root, "src", "a.py"), "mine\n"));
    expect(b.auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/a.py", language: "python", previousText: "old\nbackground\n", generatedText: "old\nbackground\nmine\n" }],
    ]);
  });

  it("takes what a call whose result never came wrote before the turn's end, and nothing written after it", async () => {
    const root = await twoBranches();
    const [, b] = await twoSessions(root);
    // Interrupted: begun, never finished.
    await b.bridge.begin("shell-1", "shell", { command: "./run.sh" });
    await appendFile(join(root, "src", "a.py"), "mine\n");
    await b.bridge.checkpoint("turn");
    expect(b.auditGeneratedEdits.mock.calls.map(([, batch]) => [batch.mode, ...batch.edits])).toEqual([[undefined, mine]]);
    await awaitClockPastNow();
    await appendFile(join(root, "src", "a.py"), "theirs\n");
    await awaitClockPastNow();
    await b.bridge.checkpoint("turn");
    expect(b.auditGeneratedEdits.mock.calls.at(-1)?.[1]).toMatchObject({
      mode: "log",
      edits: [{ path: "src/a.py", language: "python", previousText: "old\nmine\ntheirs\n", generatedText: "old\nmine\ntheirs\n" }],
    });
  });

  it("credits a call answered while the evaluator opened with what was written as it ran, and nothing written after", async () => {
    const root = await twoBranches();
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const indexing = Promise.withResolvers<void>();
    const entered = Promise.withResolvers<void>();
    const bridge = createBridge(root, {
      ...auditor(auditGeneratedEdits),
      openWorkspace: async () => {
        entered.resolve();
        await indexing.promise;
      },
    });
    const open = bridge.openWorkspace("session-1");
    passes.push(open);
    // The runtime answers the agent's calls without the bridge until the evaluator opens.
    await entered.promise;
    await bridge.unwatchedCall("shell-1");
    await appendFile(join(root, "src", "a.py"), "mine\n");
    await bridge.unwatchedResult("shell-1");
    await awaitClockPastNow();
    await writeFile(join(root, "src", "new.py"), "theirs\n");
    await awaitClockPastNow();
    indexing.resolve();
    await open;
    await bridge.checkpoint("turn");
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/a.py", language: "python", previousReadError: "Source changed during initial discovery without a captured preimage", generatedText: "old\nmine\n" }],
    ]);
  });
});

describe("ToolCallWindows", () => {
  it("forgets the calls that ended before a walk began, never one that ended since or still runs, and ends an interrupted call at a turn's end", () => {
    const windows = new ToolCallWindows();
    windows.begin("answered", 10);
    windows.finish("answered", 20);
    const walk = windows.ends;
    // Ended after the walk began, though the walk may read the clock after this end.
    windows.begin("late", 25);
    windows.finish("late", 28);
    windows.begin("running", 30);
    windows.answered(walk);
    expect(windows.ranDuring(15, 15)).toBe(false);
    expect(windows.ranDuring(26, 26)).toBe(true);
    expect(windows.ranDuring(35, 35)).toBe(true);
    expect(windows.ranDuring(1_000, 1_000)).toBe(true);
    windows.finishAll(50);
    expect(windows.ranDuring(45, 45)).toBe(true);
    expect(windows.ranDuring(60, 60)).toBe(false);
  });

  it("says when the first command still running as a walk began started, ordering ends and the walk by count, not by the clock", () => {
    const windows = new ToolCallWindows();
    windows.begin("edit", 5);
    windows.begin("before", 10, true);
    windows.finish("before", 20);
    const walk = windows.ends;
    expect(windows.commandSince(walk)).toBeUndefined();
    windows.begin("running", 30, true);
    expect(windows.commandSince(walk)).toBe(30);
    // Its end reads the same on the clock as the end of the call before the walk, yet it ended after the walk began.
    windows.begin("ending", 15, true);
    windows.finish("ending", 20);
    expect(windows.commandSince(walk)).toBe(15);
    windows.finish("running", 40);
    expect(windows.commandSince(walk)).toBe(15);
    expect(windows.commandSince(windows.ends)).toBeUndefined();
  });

  it(`keeps the newest ${MAX_FINISHED_CALLS} finished calls`, () => {
    const windows = new ToolCallWindows();
    for (let call = 0; call <= MAX_FINISHED_CALLS; call += 1) {
      windows.begin(`call-${call}`, call * 10);
      windows.finish(`call-${call}`, call * 10 + 1);
    }
    expect(windows.ranDuring(0, 0)).toBe(false);
    expect(windows.ranDuring(10, 10)).toBe(true);
    expect(windows.ranDuring(MAX_FINISHED_CALLS * 10, MAX_FINISHED_CALLS * 10)).toBe(true);
  });
});

describe("the file system's clock", () => {
  afterEach(() => {
    vi.mocked(rm).mockReset();
    vi.useRealTimers();
  });

  it("is sampled from its probe folder even while Windows holds that folder for a moment", async () => {
    const { rm: actualRm } = await vi.importActual<typeof FileSystem>("node:fs/promises");
    // Date.now() far from the file system's clock, so a sample that fell back to it shows.
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(0);
    const refused = holdRemovals(vi.mocked(rm), actualRm, 2, path => /rerail-clock-[^\\/]+$/u.test(path));

    const clock = await filesystemClock();

    expect(refused).toHaveLength(2);
    expect(clock).toBeGreaterThan(0);
    expect(existsSync(refused[0]!)).toBe(false);
  });
});

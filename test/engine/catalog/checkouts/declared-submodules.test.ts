// A checkout's declared submodules while Git runs past its time limit reading them: the first listing of the checkout's
// nested checkouts waits on Git once, and each listing after it answers at once as Git gave no answer, with no Git on its
// thread, while Git is asked again off it, one ask at a time, each begun by a listing. What a lookup of a file under the
// submodule decides meanwhile is as before.
import type { ChildProcess } from "node:child_process";
import { execFileSync } from "node:child_process";
import { once } from "node:events";
import { mkdirSync, realpathSync, renameSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

/**
 * What a Git asked for the declared submodules of `folder` (`config --file .gitmodules`, then `ls-files`) does: on the
 * caller's thread (`onThread`), answer, or run past its limit as `spawnSync` ends a Git that runs past Rerail's (at
 * 200 ms); off the thread (`offThread`), answer, run past a limit of 200 ms (`pastLimit`), end as a Git that cannot load
 * its libraries ends (`cannotStart`, exit 127), fail as a read that throws when no Git is found (`fails`), or stay until
 * Rerail's own limit or a stop ends it (`held`). Each Git started on the thread is counted (`blocking`), and each ask
 * off it at its first question (`asks`); every process started for an ask is kept until it has exited (`children`), and
 * `heldStarted` settles with the first one `held` starts.
 */
type Asked = "answers" | "pastLimit" | "cannotStart" | "fails" | "held";
const git = vi.hoisted(() => {
  const asked = {
    folder: "", onThread: "answers" as Asked, offThread: "answers" as Asked, blocking: 0, asks: 0,
    children: [] as ChildProcess[], heldStarted: Promise.withResolvers<ChildProcess>(),
  };
  /** Whether Git's arguments ask which submodules `folder` declares. */
  const reading = (args: unknown): boolean =>
    Array.isArray(args) && asked.folder !== "" && args[args.indexOf("-C") + 1] === asked.folder && (args.includes(".gitmodules") || args.includes("ls-files"));
  return Object.assign(asked, { reading });
});
const STAYS = "setTimeout(() => {}, 60_000)";
vi.mock("../../../../src/engine/child-process.js", async () => {
  const original = await vi.importActual<typeof import("../../../../src/engine/child-process.js")>("../../../../src/engine/child-process.js");
  const standIn = (parameters: Parameters<typeof original.spawn>): ChildProcess => {
    if (git.offThread === "cannotStart") return original.spawn(process.execPath, ["-e", "process.exit(127)"], parameters[2]!);
    if (git.offThread === "pastLimit" || git.offThread === "held") return original.spawn(process.execPath, ["-e", STAYS], parameters[2]!);
    return original.spawn(...parameters);
  };
  const spawn = ((...parameters: Parameters<typeof original.spawn>) => {
    if (!git.reading(parameters[1])) return original.spawn(...parameters);
    const child = standIn(parameters);
    git.children.push(child);
    if (git.offThread === "held") git.heldStarted.resolve(child);
    return child;
  }) as typeof original.spawn;
  const spawnSync = ((...parameters: Parameters<typeof original.spawnSync>) => {
    if (!git.reading(parameters[1])) return original.spawnSync(...parameters);
    git.blocking += 1;
    if (git.onThread === "pastLimit") return original.spawnSync(process.execPath, ["-e", STAYS], { encoding: "utf8", timeout: 200 });
    return original.spawnSync(...parameters);
  }) as typeof original.spawnSync;
  return { ...original, spawn, spawnSync };
});
vi.mock("../../../../src/engine/git.js", async () => {
  const original = await vi.importActual<typeof import("../../../../src/engine/git.js")>("../../../../src/engine/git.js");
  type Options = Parameters<typeof original.runGitAsync>[2];
  const runGitAsync = async (cwd: string, args: readonly string[], options: Options = {}): ReturnType<typeof original.runGitAsync> => {
    if (!git.reading(["-C", cwd, ...args])) return await original.runGitAsync(cwd, args, options);
    if (args.includes(".gitmodules")) git.asks += 1;
    if (git.offThread === "fails") throw new Error("a stand-in read that failed");
    return await original.runGitAsync(cwd, args, git.offThread === "pastLimit" ? { ...options, timeout: 200 } : options);
  };
  return { ...original, runGitAsync };
});

import { CheckoutOwners } from "../../../../src/engine/catalog/checkout-owner.js";
import { forgetGoneSubmodules, keptSubmoduleCount, learnDeclaredSubmodules, stopAskingSubmodules, submodulesAskedAgain } from "../../../../src/engine/catalog/checkouts/declared-submodules.js";
import type { AuditRecorder, AuditInput, AuditResult } from "../../../../src/engine/audit/audit.js";
import { CheckoutCodeRuleRouter } from "../../../../src/engine/code-rule/checkout-code-rule-router.js";
import { CheckoutCatalogs, startingCheckout } from "../../../../src/engine/code-rule/code-rule-dsl-catalog.js";
import { type CodeRule, codeRuleSchema } from "../../../../src/engine/code-rule/code-rule-schema.js";
import { closeNativeTestDaemons, startNativeTestDaemon } from "../../../support/native-daemon.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";
import { setUpTestRepository } from "../../../support/repository-rules.js";

const cleanups: Array<() => Promise<void>> = [];
afterEach(async () => {
  // Each Git asked off the thread has exited before its folder goes.
  await Promise.all(git.children.splice(0).map(async child => child.exitCode === null && child.signalCode === null ? await once(child, "exit") : undefined));
  vi.unstubAllEnvs();
  Object.assign(git, { folder: "", onThread: "answers", offThread: "answers", blocking: 0, asks: 0, heldStarted: Promise.withResolvers<ChildProcess>() });
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

function gitIn(cwd: string, ...args: string[]): string {
  return execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, encoding: "utf8" });
}

/**
 * A committed checkout `alpha` whose `.gitmodules` names `sub`, a repository checked out in it that its index holds as
 * a gitlink, as the issue's steps make it; the stand-ins answer for `alpha`'s submodule reads from now on.
 */
async function checkoutWithSubmodule(): Promise<{ root: string; alpha: string; sub: string }> {
  const root = realpathSync.native(await mkdtemp(join(tmpdir(), "rerail-declared-submodules-")));
  cleanups.push(() => removeFolder(root));
  const [alpha, sub] = [join(root, "alpha"), join(root, "alpha", "sub")];
  mkdirSync(join(sub, "api"), { recursive: true });
  gitIn(alpha, "init", "--quiet");
  writeFileSync(join(sub, "notes.txt"), "sub\n");
  gitIn(sub, "init", "--quiet");
  gitIn(sub, "add", "notes.txt");
  gitIn(sub, "commit", "--quiet", "-m", "sub");
  writeFileSync(join(alpha, ".gitmodules"), "[submodule \"sub\"]\n\tpath = sub\n\turl = ./sub\n");
  writeFileSync(join(alpha, "notes.txt"), "alpha\n");
  gitIn(alpha, "update-index", "--add", "--cacheinfo", `160000,${gitIn(sub, "rev-parse", "HEAD").trim()},sub`);
  gitIn(alpha, "add", "notes.txt", ".gitmodules");
  gitIn(alpha, "commit", "--quiet", "-m", "alpha");
  git.folder = alpha;
  return { root, alpha, sub };
}

describe("a checkout's declared submodules while Git runs past its time limit reading them", () => {
  it("waits on Git once: the listings after it answer as Git gave no answer with no Git on the thread, and one ask off it runs at a time", async () => {
    const { alpha } = await checkoutWithSubmodule();
    Object.assign(git, { onThread: "pastLimit", offThread: "pastLimit" });
    const owners = new CheckoutOwners();
    for (let listing = 0; listing < 6; listing += 1) expect(owners.declaredSubmodules(alpha)).toEqual([]);
    expect(git.blocking).toBe(1);
    // The second listing asked off the thread; the four after it found that ask in flight.
    expect(git.asks).toBe(1);

    // That ask ran past its limit again: the checkout stays kept, and nothing asks again until a listing does.
    await submodulesAskedAgain(alpha);
    expect(git.asks).toBe(1);
    expect(owners.declaredSubmodules(alpha)).toEqual([]);
    expect([git.blocking, git.asks]).toEqual([1, 2]);
    await submodulesAskedAgain(alpha);
  });

  it("takes Git's answer off the thread at the next listing, with no Git on the thread", async () => {
    const { alpha, sub } = await checkoutWithSubmodule();
    Object.assign(git, { onThread: "pastLimit", offThread: "answers" });
    const owners = new CheckoutOwners();
    expect([owners.declaredSubmodules(alpha), owners.declaredSubmodules(alpha)]).toEqual([[], []]);
    await submodulesAskedAgain(alpha);

    expect(owners.declaredSubmodules(alpha)).toEqual([sub]);
    expect([git.blocking, git.asks]).toEqual([1, 1]);
  });

  it.each([
    ["cannot start (exit 127)", "cannotStart"],
    ["fails as a read that finds no Git", "fails"],
  ] as const)("leaves the checkout to the next listing's Git, as before, when the ask's Git %s", async (_, offThread) => {
    const { alpha } = await checkoutWithSubmodule();
    Object.assign(git, { onThread: "pastLimit", offThread });
    const owners = new CheckoutOwners();
    expect([owners.declaredSubmodules(alpha), owners.declaredSubmodules(alpha)]).toEqual([[], []]);
    await submodulesAskedAgain(alpha);
    expect([git.blocking, git.asks]).toEqual([1, 1]);

    expect(owners.declaredSubmodules(alpha)).toEqual([]);
    expect(git.blocking).toBe(2);
  });

  it("ends the ask in flight with its Git at the stop, keeps the checkout as it was, and asks again at a listing after it", async () => {
    const { alpha, sub } = await checkoutWithSubmodule();
    Object.assign(git, { onThread: "pastLimit", offThread: "held" });
    const owners = new CheckoutOwners();
    expect([owners.declaredSubmodules(alpha), owners.declaredSubmodules(alpha)]).toEqual([[], []]);
    // The ask started its Git with the listing that began it.
    expect(git.asks).toBe(1);
    const held = await git.heldStarted.promise;

    await stopAskingSubmodules();
    expect(held.exitCode !== null || held.signalCode !== null).toBe(true);
    await submodulesAskedAgain(alpha);
    expect(git.asks).toBe(1);

    git.offThread = "answers";
    expect(owners.declaredSubmodules(alpha)).toEqual([]);
    expect([git.blocking, git.asks]).toEqual([1, 2]);
    await submodulesAskedAgain(alpha);
    expect(owners.declaredSubmodules(alpha)).toEqual([sub]);
    expect(git.blocking).toBe(1);
  });

  it("waits once after the index changes, where Git answered before it", async () => {
    const { alpha, sub } = await checkoutWithSubmodule();
    const owners = new CheckoutOwners();
    // Git answered both questions on the thread: the `.gitmodules` paths, then which the index holds.
    expect(owners.declaredSubmodules(alpha)).toEqual([sub]);
    expect(git.blocking).toBe(2);
    writeFileSync(join(alpha, "notes.txt"), "alpha, changed\n");
    gitIn(alpha, "add", "notes.txt");

    Object.assign(git, { onThread: "pastLimit", offThread: "pastLimit" });
    for (let listing = 0; listing < 4; listing += 1) expect(owners.declaredSubmodules(alpha)).toEqual([]);
    expect(git.blocking).toBe(3);
    expect(git.asks).toBe(1);
    await submodulesAskedAgain(alpha);
  });

  it("answers a placement's learning at once, through the one ask, while the checkout is kept past its limit", async () => {
    const { alpha } = await checkoutWithSubmodule();
    Object.assign(git, { onThread: "pastLimit", offThread: "pastLimit" });
    expect(new CheckoutOwners().declaredSubmodules(alpha)).toEqual([]);
    expect(await Promise.all([learnDeclaredSubmodules(alpha), learnDeclaredSubmodules(alpha)])).toEqual([[], []]);
    expect([git.blocking, git.asks]).toEqual([1, 1]);
    await submodulesAskedAgain(alpha);
  });

  it("stays kept when an ask runs past its limit again with no fact of the checkout read", async () => {
    const { alpha } = await checkoutWithSubmodule();
    Object.assign(git, { onThread: "pastLimit", offThread: "pastLimit" });
    const owners = new CheckoutOwners();
    expect(owners.declaredSubmodules(alpha)).toEqual([]);
    // Git may read another index: no fact is read for the ask that this listing begins.
    vi.stubEnv("GIT_INDEX_FILE", join(alpha, ".git", "other-index"));
    expect(owners.declaredSubmodules(alpha)).toEqual([]);
    await submodulesAskedAgain(alpha);
    vi.unstubAllEnvs();

    expect(owners.declaredSubmodules(alpha)).toEqual([]);
    expect([git.blocking, git.asks]).toEqual([1, 2]);
    await submodulesAskedAgain(alpha);
  });

  it("keeps nothing for a checkout that went while its ask ran", async () => {
    const { root, alpha } = await checkoutWithSubmodule();
    Object.assign(git, { onThread: "pastLimit", offThread: "pastLimit" });
    const before = keptSubmoduleCount();
    const owners = new CheckoutOwners();
    expect([owners.declaredSubmodules(alpha), owners.declaredSubmodules(alpha)]).toEqual([[], []]);
    expect(keptSubmoduleCount() - before).toBe(1);
    const asking = submodulesAskedAgain(alpha);
    const common = realpathSync.native(join(alpha, ".git"));
    renameSync(alpha, join(root, "moved"));
    forgetGoneSubmodules(common);
    expect(keptSubmoduleCount() - before).toBe(0);

    await asking;
    expect(keptSubmoduleCount() - before).toBe(0);
  });

  it("ends the ask in flight with its Git as the daemon closes", async () => {
    const { root, alpha } = await checkoutWithSubmodule();
    const daemon = await startNativeTestDaemon(root);
    cleanups.push(closeNativeTestDaemons);
    Object.assign(git, { onThread: "pastLimit", offThread: "held" });
    const owners = new CheckoutOwners();
    expect([owners.declaredSubmodules(alpha), owners.declaredSubmodules(alpha)]).toEqual([[], []]);
    expect(git.asks).toBe(1);
    const held = await git.heldStarted.promise;

    await daemon.close();
    expect(held.exitCode !== null || held.signalCode !== null).toBe(true);
  });
});

/** A rule that forbids `NEEDLE` in every Python file and reports `code`. */
function needle(id: string, code: string): CodeRule {
  return codeRuleSchema.parse({
    format: "rerail.policy/v1", id, revision: 1, mode: "block",
    scope: { include: ["**/*.py"], exclude: [] },
    clauses: [{ id: "needle", violation: { code, anchor: "witness.span", message: `No ${code}` }, forbid: { regex: { pattern: "NEEDLE", source: "file.content" } } }],
  });
}

/** A router for sessions in `alpha`, and each violation its audits found: the checkout that judged it, its path and code. */
function routerIn(alpha: string, databasePath: string) {
  const audits: Array<AuditInput & { result?: AuditResult }> = [];
  const recorder: AuditRecorder = {
    start(input) {
      audits.push({ ...input, paths: [...input.paths] });
      return String(audits.length - 1);
    },
    finish(auditId, result) { audits[Number(auditId)]!.result = result; },
    progress(auditId, result) { audits[Number(auditId)]!.result = { status: "completed", ...result }; },
    interrupt() { /* No audit of these cases is interrupted. */ },
  };
  const owners = new CheckoutOwners();
  const catalogs = new CheckoutCatalogs(databasePath, {});
  const router = new CheckoutCodeRuleRouter({
    sessionRoot: alpha, startingCheckout: startingCheckout(owners, alpha).checkoutRoot, owners, catalogs, steerDelivery: { requestSteer: async () => undefined }, audits: recorder,
  });
  cleanups.push(async () => {
    await router.close();
    catalogs.close();
  });
  const violations = () => audits.flatMap(audit => (audit.result?.violations ?? []).map(found => ({ checkout: audit.workspaceRoot, path: found.path, code: found.code })));
  return { router, violations };
}

/** Writes `NEEDLE` into `path` as a session's edit at `revision`, and waits for its checks. */
async function edit(router: CheckoutCodeRuleRouter, revision: number, path: string): Promise<void> {
  await router.auditGeneratedEdits("session", { revision, checkpoints: ["edit"], edits: [{ path, language: "python", generatedText: "NEEDLE\n" }] });
  await router.settleGeneratedEdits();
}

// Which checkout judges such a file while Git gives no answer for the read is the lookup's own decision, made as before
// this keep: these cases pass on the source before it too.
describe("a file under a declared submodule that was never set up", () => {
  /** Three edits of a file under `sub` in a session in `alpha`, the stand-ins as `stand`: which checkout judged each, and how. */
  async function judged(stand: Asked): Promise<{ alpha: string; found: Array<{ checkout: string; path: string | undefined; code: string }> }> {
    const { root, alpha } = await checkoutWithSubmodule();
    const databasePath = join(root, "rerail.sqlite");
    setUpTestRepository(alpha, { library: [{ kind: "code", rule: needle("test:alpha", "ALPHA") }] }, databasePath);
    Object.assign(git, { onThread: stand, offThread: stand });
    const { router, violations } = routerIn(alpha, databasePath);
    await router.openWorkspace("session");
    for (let revision = 1; revision <= 3; revision += 1) await edit(router, revision, "sub/api/b.py");
    return { alpha, found: violations() };
  }

  it("runs no rules with Git answering: the submodule owns it", async () => {
    expect((await judged("answers")).found).toEqual([]);
  });

  it("is judged by the checkout around it at each lookup while Git runs past its limit reading the submodules", async () => {
    const { alpha, found } = await judged("pastLimit");
    expect(found).toEqual(Array.from({ length: 3 }, () => ({ checkout: alpha, path: "sub/api/b.py", code: "ALPHA" })));
  });
});

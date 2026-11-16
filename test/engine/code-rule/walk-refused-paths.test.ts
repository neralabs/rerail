/**
 * A walk beside a file or folder the user may not read. On Windows the native realpath refuses such a path (`EPERM`)
 * while `lstat` answers for it, and every checkout lookup goes through that realpath; Linux and macOS answer for it.
 * Off Windows the tests refuse the realpath of each path they deny the same way (`refused`, below), so they run where
 * the continuous integration runs; on Windows they deny it for real and Windows refuses it.
 */
import { execFileSync } from "node:child_process";
import { closeSync, openSync, readdirSync, type PathLike } from "node:fs";
import { chmod, mkdir, mkdtemp, realpath, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi, type TestContext } from "vitest";
import { CheckoutOwners } from "../../../src/engine/catalog/checkout-owner.js";
import { CheckoutCodeRuleRouter } from "../../../src/engine/code-rule/checkout-code-rule-router.js";
import { CodeRuleBridge } from "../../../src/engine/code-rule/code-rule-bridge.js";
import { CheckoutCatalogs, startingCheckout } from "../../../src/engine/code-rule/code-rule-dsl-catalog.js";
import type { AuditInput, AuditRecorder, AuditResult } from "../../../src/engine/audit/audit.js";
import { codeRuleSchema } from "../../../src/engine/code-rule/code-rule-schema.js";
import { setUpTestRepository } from "../../support/repository-rules.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

/** The paths whose realpath, and every realpath below them, fails with `EPERM`, as Windows fails one the user is denied. */
const refused = vi.hoisted(() => new Set<string>());
vi.mock("node:fs", async importOriginal => {
  const fs = await importOriginal<typeof import("node:fs")>();
  const refuse = (path: PathLike): void => {
    const named = String(path);
    // Filled off Windows only, so the separator is `/`.
    for (const denied of refused) {
      if (named === denied || named.startsWith(`${denied}/`)) throw Object.assign(new Error(`EPERM: operation not permitted, realpath '${named}'`), { code: "EPERM" });
    }
  };
  // The casts keep each realpath's overloads: the wrappers pass every argument on.
  const realpathSync = Object.assign(
    ((...args: Parameters<typeof fs.realpathSync>) => {
      refuse(args[0]);
      return fs.realpathSync(...args);
    }) as typeof fs.realpathSync,
    {
      native: ((...args: Parameters<typeof fs.realpathSync.native>) => {
        refuse(args[0]);
        return fs.realpathSync.native(...args);
      }) as typeof fs.realpathSync.native,
    },
  );
  return { ...fs, realpathSync };
});

const WINDOWS = process.platform === "win32";
const cleanups: Array<() => Promise<void>> = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

const systemTool = (name: string): string => join(process.env.SystemRoot ?? "C:\\Windows", "System32", name);
let sid: string | undefined;
function userSid(): string {
  sid ??= execFileSync(systemTool("whoami.exe"), ["/user", "/fo", "csv", "/nh"], { encoding: "utf8" }).trim().split(",").at(-1)?.replaceAll('"', "");
  if (sid === undefined || sid === "") throw new Error("whoami named no user");
  return sid;
}

function refuses(look: () => void): boolean {
  try {
    look();
    return false;
  } catch {
    return true;
  }
}

/**
 * Takes this user's rights to the file or `folder` at `path` away until the test ends: a deny rule on Windows, mode 000
 * elsewhere, with its realpath refused as Windows refuses it. Skips the test when this process still reads it: an
 * administrator's backup privilege reads past a deny rule, and root past a mode.
 */
async function deny(context: Pick<TestContext, "skip">, path: string, folder: boolean): Promise<void> {
  if (WINDOWS) execFileSync(systemTool("icacls.exe"), [path, "/deny", `*${userSid()}:${folder ? "(OI)(CI)F" : "F"}`], { stdio: "ignore" });
  else {
    await chmod(path, 0o000);
    refused.add(path);
  }
  cleanups.push(async () => {
    if (WINDOWS) execFileSync(systemTool("icacls.exe"), [path, "/remove:d", `*${userSid()}`], { stdio: "ignore" });
    else await chmod(path, folder ? 0o755 : 0o644);
    refused.delete(path);
  });
  if (!refuses(() => (folder ? readdirSync(path) : closeSync(openSync(path, "r"))))) context.skip("this process reads past the denial");
}

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "-c", "protocol.file.allow=always", ...args], { cwd, stdio: "ignore" });
}

const NEEDLE = codeRuleSchema.parse({
  format: "rerail.policy/v1", id: "test:needle", revision: 1, mode: "block", scope: { include: ["**/*.py"], exclude: [] },
  clauses: [{ id: "needle", violation: { code: "NEEDLE", anchor: "witness.span", message: "No NEEDLE" }, forbid: { regex: { pattern: "NEEDLE", source: "file.content" } } }],
});

/** A committed repository at `path` holding `files` (path to text), with NEEDLE set up in `databasePath`; physical. */
async function repository(path: string, files: Readonly<Record<string, string>>, databasePath?: string): Promise<string> {
  for (const [file, text] of Object.entries(files)) {
    await mkdir(join(path, file, ".."), { recursive: true });
    await writeFile(join(path, file), text);
  }
  git(path, "init", "--quiet");
  git(path, "add", ".");
  git(path, "commit", "--quiet", "-m", "seed");
  if (databasePath !== undefined) setUpTestRepository(path, { library: [{ kind: "code", rule: NEEDLE }] }, databasePath);
  return await realpath(path);
}

async function temporaryRoot(): Promise<string> {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-walk-refused-")));
  cleanups.push(() => removeFolder(root));
  return root;
}

type RecordedRun = AuditInput & { result?: AuditResult };

/** The router a session at `session` runs, recording each audit it runs in `audits`. */
function router(session: string, databasePath: string, audits: RecordedRun[]): CheckoutCodeRuleRouter {
  const recorder: AuditRecorder = {
    start(input) {
      audits.push({ ...input, paths: [...input.paths] });
      return String(audits.length - 1);
    },
    finish(auditId, result) { audits[Number(auditId)]!.result = result; },
    progress(auditId, result) { audits[Number(auditId)]!.result = { status: "completed", ...result }; },
    interrupt() {},
  };
  const owners = new CheckoutOwners();
  const catalogs = new CheckoutCatalogs(databasePath);
  const auditor = new CheckoutCodeRuleRouter({
    sessionRoot: session, startingCheckout: startingCheckout(owners, session).checkoutRoot, owners, catalogs,
    steerDelivery: { requestSteer: async () => undefined }, audits: recorder,
  });
  cleanups.push(async () => {
    await auditor.close();
    catalogs.close();
  });
  return auditor;
}

/** The path of each violation the audits found, in the order found. */
function found(audits: readonly RecordedRun[]): string[] {
  return audits.flatMap(audit => (audit.result?.violations ?? []).map(violation => violation.path ?? "(no path)"));
}

/** A session's bridge over `session`, open and past its first walk. */
async function openedBridge(session: string, databasePath: string, audits: RecordedRun[]): Promise<CodeRuleBridge> {
  const bridge = new CodeRuleBridge(session, router(session, databasePath, audits));
  cleanups.push(() => bridge.close());
  await bridge.openWorkspace("session");
  await bridge.checkpoint("turn");
  return bridge;
}

/** A shell command that writes NEEDLE into `path`, then the turn's end: what its walks could not check, as the turn end answers it. */
async function shellWrite(bridge: CodeRuleBridge, session: string, path: string, call: string): Promise<string[]> {
  await bridge.begin(call, "shell", { command: `write ${path}` });
  await writeFile(join(session, path), "def marked():\n    return 1  # NEEDLE\n");
  await bridge.finish(call);
  const answer = await bridge.checkpoint("turn");
  await bridge.settle();
  return answer.failures.filter(failure => failure.clauseId === "source-discovery").map(failure => failure.message);
}

const SOURCES = {
  "src/app/models.py": "def model():\n    return 1\n",
  "src/app/secret.py": "SECRET = 1\n",
  "src/app/locked/hidden.py": "def hidden():\n    return 1\n",
  "lib/tools.py": "def tool():\n    return 1\n",
};

describe("a walk beside a path the user may not read", () => {
  it("checks every shell edit beside a file the user is denied, and names that file alone", async context => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const session = await repository(join(root, "session"), SOURCES, databasePath);
    await deny(context, join(session, "src", "app", "secret.py"), false);
    const audits: RecordedRun[] = [];
    const bridge = await openedBridge(session, databasePath, audits);

    const unchecked = await shellWrite(bridge, session, "src/app/models.py", "write-1");
    expect(found(audits)).toEqual(["src/app/models.py"]);
    // The denied file is the walk's one gap, as a file Linux refuses to open is.
    expect(unchecked).toEqual([expect.stringMatching(/^src\/app\/secret\.py: E(PERM|ACCES)\b[^;]*$/u)]);

    await shellWrite(bridge, session, "lib/tools.py", "write-2");
    expect(found(audits)).toEqual(["src/app/models.py", "lib/tools.py"]);
  });

  it("checks a shell edit to a file listed after a folder the user is denied, and names that folder alone", async context => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const session = await repository(join(root, "session"), SOURCES, databasePath);
    await deny(context, join(session, "src", "app", "locked"), true);
    const audits: RecordedRun[] = [];
    const bridge = await openedBridge(session, databasePath, audits);

    // Windows lists `locked` before `models.py`; its listing of `src/app` used to stop at `locked`.
    const unchecked = await shellWrite(bridge, session, "src/app/models.py", "write-1");
    expect(found(audits)).toEqual(["src/app/models.py"]);
    expect(unchecked).toEqual([expect.stringMatching(/^src\/app\/locked: E(PERM|ACCES)\b[^;]*$/u)]);
  });

  it("checks a shell edit while another worktree of the repository is denied to the user", async context => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const session = await repository(join(root, "session"), SOURCES, databasePath);
    const other = join(root, "other");
    git(session, "worktree", "add", "--quiet", "--detach", other);
    await deny(context, other, true);
    const audits: RecordedRun[] = [];
    const bridge = await openedBridge(session, databasePath, audits);

    // Every walk asks for the repository's worktrees first; the real path of the denied one used to fail it whole.
    const unchecked = await shellWrite(bridge, session, "src/app/models.py", "write-1");
    expect(found(audits)).toEqual(["src/app/models.py"]);
    expect(unchecked).toEqual([]);
  });

  it("audits the whole repository while a submodule it declares is denied to the user", async context => {
    const root = await temporaryRoot();
    const databasePath = join(root, "rerail.sqlite");
    const source = await repository(join(root, "sub-source"), { "lib.py": "x = 1\n" });
    const parent = await repository(join(root, "parent"), { "app.py": "NEEDLE = 1\n" });
    git(parent, "submodule", "add", "--quiet", source, "sub");
    git(parent, "commit", "--quiet", "-m", "submodule");
    setUpTestRepository(parent, { library: [{ kind: "code", rule: NEEDLE }] }, databasePath);
    await deny(context, join(parent, "sub"), true);
    const audits: RecordedRun[] = [];
    const auditor = router(parent, databasePath, audits);
    await auditor.openWorkspace("audit");

    // The audit leaves out the checkouts nested in it; the real path of the denied one used to fail the audit whole.
    await auditor.audit("audit", { scope: "repository", mode: "log" });
    expect(found(audits)).toEqual(["app.py"]);
  });
});

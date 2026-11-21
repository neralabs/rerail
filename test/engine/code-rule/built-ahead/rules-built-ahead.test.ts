import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, realpath, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AuditRecorder } from "../../../../src/engine/audit/audit.js";
import { CheckoutOwners } from "../../../../src/engine/catalog/checkout-owner.js";
import { CheckoutCodeRuleRouter } from "../../../../src/engine/code-rule/checkout-code-rule-router.js";
import { CheckoutCatalogs, startingCheckout } from "../../../../src/engine/code-rule/code-rule-dsl-catalog.js";
import { codeRuleSchema } from "../../../../src/engine/code-rule/code-rule-schema.js";
import type { NativeBinding } from "../../../../src/engine/native/native.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";
import { setUpTestRepository } from "../../../support/repository-rules.js";

/** The engine's compiles off the thread: one of the rule `id` names starts (`started`), then waits for `released`. */
const held = vi.hoisted(() => ({
  id: undefined as string | undefined,
  started: () => {},
  released: Promise.resolve(),
}));

vi.mock("../../../../src/engine/native/native.js", async importOriginal => {
  const actual = await importOriginal<{ loadNativeBinding: () => NativeBinding }>();
  let holding: NativeBinding | undefined;
  const hold = (binding: NativeBinding): NativeBinding => {
    const engine = binding.CodeRuleDslRuntime;
    const runtime = {
      pythonAstBatch: (documents: string) => engine.pythonAstBatch(documents),
      testRr: (paths: string, filter?: string | null) => engine.testRr(paths, filter),
      compile: (...args: Parameters<typeof engine.compile>) => engine.compile(...args),
      compileAsync: async (...args: Parameters<typeof engine.compile>) => {
        if ((JSON.parse(args[0]) as { id: string }).id === held.id) {
          held.started();
          await held.released;
        }
        return await engine.compileAsync!(...args);
      },
    };
    return new Proxy(binding, { get: (target, key) => (key === "CodeRuleDslRuntime" ? runtime : Reflect.get(target, key)) });
  };
  return { ...actual, loadNativeBinding: () => (holding ??= hold(actual.loadNativeBinding())) };
});

const cleanups: Array<() => Promise<void>> = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

/** A library rule that forbids `NEEDLE` in every Python file and reports `code`. */
function needle(id: string, code: string) {
  return {
    kind: "code" as const,
    rule: codeRuleSchema.parse({
      format: "rerail.policy/v1", id, revision: 1, mode: "block",
      scope: { include: ["**/*.py"], exclude: [] },
      clauses: [{ id: "needle", violation: { code, anchor: "witness.span", message: `No ${code}` }, forbid: { regex: { pattern: "NEEDLE", source: "file.content" } } }],
    }),
  };
}

/** A committed Git repository at `path`; returns its physical path. */
async function repository(path: string): Promise<string> {
  await mkdir(join(path, "api"), { recursive: true });
  await writeFile(join(path, "README.md"), "seed\n");
  const git = (...args: string[]) => execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd: path, stdio: "ignore" });
  git("init", "--quiet");
  git("add", "--", "README.md");
  git("commit", "--quiet", "-m", "seed");
  return await realpath(path);
}

const audits: AuditRecorder = { start: () => "0", finish: () => undefined, progress: () => undefined, interrupt: () => undefined };

/**
 * A session folder holding two started repositories in a folder it ignores (`used`, running `usedRules`, and `unused`),
 * each running rules of its own, so a session opened there builds their rules ahead; `open` gives a session's router
 * over the same catalogs.
 */
async function sessionWithNested(usedRules = [needle("test:used", "USED")]) {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-built-ahead-")));
  cleanups.push(() => removeFolder(root));
  const databasePath = join(root, "rerail.sqlite");
  const session = await repository(join(root, "session"));
  await writeFile(join(session, ".gitignore"), ".scratch/\n");
  const used = await repository(join(session, ".scratch", "used"));
  const unused = await repository(join(session, ".scratch", "unused"));
  setUpTestRepository(session, { library: [needle("test:session", "SESSION")] }, databasePath);
  setUpTestRepository(used, { library: usedRules }, databasePath);
  setUpTestRepository(unused, { library: [needle("test:unused", "UNUSED")] }, databasePath);
  const owners = new CheckoutOwners();
  const catalogs = new CheckoutCatalogs(databasePath);
  cleanups.push(async () => catalogs.close());
  const open = () => {
    const auditor = new CheckoutCodeRuleRouter({
      sessionRoot: session, startingCheckout: startingCheckout(owners, session).checkoutRoot, owners, catalogs,
      steerDelivery: { requestSteer: async () => undefined }, audits,
    });
    cleanups.push(async () => await auditor.close());
    return auditor;
  };
  return { session, used, unused, catalogs, open };
}

/** What the test's code writes to stderr from now on, kept from the terminal. */
function stderrLines(): string[] {
  const lines: string[] = [];
  const write = vi.spyOn(process.stderr, "write").mockImplementation(chunk => (lines.push(String(chunk)), true));
  cleanups.push(async () => write.mockRestore());
  return lines;
}

describe("rules a session builds ahead", () => {
  it("are let go of when the session closes, unless one of its tool calls used them", async () => {
    const { session, used, unused, catalogs, open } = await sessionWithNested();
    const auditor = open();
    await auditor.openWorkspace("closes");
    await auditor.rulesBuiltAhead();
    expect([catalogs.holdsCatalog(used), catalogs.holdsCatalog(unused)]).toEqual([true, true]);
    await auditor.auditGeneratedEdits("closes", {
      revision: 1, checkpoints: ["edit"], edits: [{ path: join(used, "api", "x.py"), language: "python", generatedText: "NEEDLE\n" }],
    });
    await auditor.settleGeneratedEdits();

    await auditor.close();
    expect({ session: catalogs.holdsCatalog(session), used: catalogs.holdsCatalog(used), unused: catalogs.holdsCatalog(unused) })
      .toEqual({ session: true, used: true, unused: false });
  });

  it("are kept while another open session built them ahead too", async () => {
    const { unused, catalogs, open } = await sessionWithNested();
    const first = open();
    const second = open();
    await first.openWorkspace("first");
    await first.rulesBuiltAhead();
    // Opened once the first session's are built: it asks ahead for the catalogs the first one holds.
    await second.openWorkspace("second");
    await second.rulesBuiltAhead();

    await first.close();
    expect(catalogs.holdsCatalog(unused)).toBe(true);
    await second.close();
    expect(catalogs.holdsCatalog(unused)).toBe(false);
  });

  it("are built for a session that joined their build when the session that started it closes first", async () => {
    const { used, catalogs, open } = await sessionWithNested([needle("test:joined", "JOINED"), needle("test:joined-next", "NEXT")]);
    // `used` comes first in each session's builds ahead, so the second session joins the first one's build of it.
    vi.spyOn(catalogs, "checkoutsEditedBy").mockReturnValue([used]);
    const started = Promise.withResolvers<void>();
    const released = Promise.withResolvers<void>();
    Object.assign(held, { id: "test:joined", started: started.resolve, released: released.promise });
    cleanups.push(async () => {
      held.id = undefined;
    });
    const first = open();
    const second = open();
    await first.openWorkspace("first");
    // The first session's build of `used` compiles its first rule; the next one waits behind it.
    await started.promise;
    await second.openWorkspace("second");

    const closed = first.close();
    released.resolve();
    await closed;
    await second.rulesBuiltAhead();
    expect(catalogs.holdsCatalog(used)).toBe(true);
  });

  it("are built for the other repositories when one checkout's record cannot be read", async () => {
    const { session, used, unused, catalogs, open } = await sessionWithNested();
    const gone = join(session, "gone");
    vi.spyOn(catalogs, "checkoutsEditedBy").mockReturnValue([gone]);
    const checkout = catalogs.checkout.bind(catalogs);
    vi.spyOn(catalogs, "checkout").mockImplementation(root => {
      if (root === gone) throw new Error("its record cannot be read");
      return checkout(root);
    });
    const written = stderrLines();
    const auditor = open();

    await auditor.openWorkspace("one-unreadable");
    await auditor.rulesBuiltAhead();
    expect([catalogs.holdsCatalog(used), catalogs.holdsCatalog(unused)]).toEqual([true, true]);
    expect(written).toContain(`rerail: rules not built ahead for ${gone}: its record cannot be read\n`);
  });

  it("say on stderr why when the session's checkouts cannot be read, or a build of one fails, and not for a call's build", async () => {
    const { used, catalogs, open } = await sessionWithNested();
    const written = stderrLines();
    const editedBy = vi.spyOn(catalogs, "checkoutsEditedBy").mockImplementation(() => {
      throw new Error("database is locked");
    });
    const unreadable = open();
    await unreadable.openWorkspace("unreadable");
    await unreadable.rulesBuiltAhead();
    expect(written).toContain("rerail: rules not built ahead for session unreadable: database is locked\n");

    editedBy.mockReturnValue([used]);
    const prepare = catalogs.prepare.bind(catalogs);
    const failingPrepare = vi.spyOn(catalogs, "prepare").mockImplementation(async (root, aheadFor) => {
      if (root === used) throw new Error(aheadFor === undefined ? "a call's rules cannot be read" : "its rules cannot be read");
      await prepare(root, aheadFor);
    });
    const failing = open();
    await failing.openWorkspace("failing");
    await failing.rulesBuiltAhead();
    expect(written).toContain(`rerail: rules not built ahead for ${used}: its rules cannot be read\n`);

    // A call's build that fails is left to its lane's open, which reports it there.
    await failing.prepareRules([join(used, "api", "x.py")]);
    expect(failingPrepare).toHaveBeenCalledWith(used, undefined);
    expect(written.filter(line => line.includes("a call's rules cannot be read"))).toEqual([]);
  });
});

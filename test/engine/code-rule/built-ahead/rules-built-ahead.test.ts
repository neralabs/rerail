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
import { removeFolder } from "../../../support/platform/remove-folder.js";
import { setUpTestRepository } from "../../../support/repository-rules.js";

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
 * A session folder holding two started repositories in a folder it ignores (`used`, `unused`), each running a rule of
 * its own, so a session opened there builds their rules ahead; `open` gives a session's router over the same catalogs.
 */
async function sessionWithNested() {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-built-ahead-")));
  cleanups.push(() => removeFolder(root));
  const databasePath = join(root, "rerail.sqlite");
  const session = await repository(join(root, "session"));
  await writeFile(join(session, ".gitignore"), ".scratch/\n");
  const used = await repository(join(session, ".scratch", "used"));
  const unused = await repository(join(session, ".scratch", "unused"));
  setUpTestRepository(session, { library: [needle("test:session", "SESSION")] }, databasePath);
  setUpTestRepository(used, { library: [needle("test:used", "USED")] }, databasePath);
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
    await second.openWorkspace("second");
    await first.rulesBuiltAhead();
    await second.rulesBuiltAhead();

    await first.close();
    expect(catalogs.holdsCatalog(unused)).toBe(true);
    await second.close();
    expect(catalogs.holdsCatalog(unused)).toBe(false);
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
    const auditor = open();

    await auditor.openWorkspace("one-unreadable");
    await auditor.rulesBuiltAhead();
    expect([catalogs.holdsCatalog(used), catalogs.holdsCatalog(unused)]).toEqual([true, true]);
  });
});

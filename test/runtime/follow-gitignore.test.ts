import { execFileSync } from "node:child_process";
import { mkdir, realpath, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { eq } from "drizzle-orm";
import { describe, expect, it, vi } from "vitest";
import { codeRuleSchema, NativeCodeRuleEnforcer, openRerailDatabase, repositories, SqliteAuditStore } from "../../src/engine/index.js";
import { RerailRuntime } from "../../src/runtime/index.js";
import { temporaryDirectories, testEmbedder } from "../support/hook-events.js";
import { testCodeRuleCatalog } from "../support/repository-rules.js";
import { startTestRepository } from "../support/started-repository.js";

const temporaryDirectory = temporaryDirectories("rerail-follow-gitignore-");

function needleCatalog() {
  return testCodeRuleCatalog([codeRuleSchema.parse({
    format: "rerail.policy/v1", id: "test:needle", revision: 1, mode: "steer",
    scope: { include: ["**/*.py"], exclude: [] },
    clauses: [{
      id: "needle",
      violation: { code: "NEEDLE", anchor: "witness.span", message: "No needle" },
      forbid: { regex: { pattern: "NEEDLE", source: "file.content" } },
    }],
  })]);
}

/**
 * A session in a Git checkout whose repository follows `.gitignore` or not runs one shell command that writes a
 * source in the checkout and one in its gitignored `gen/`; returns the paths the session's audits found the needle in.
 */
async function shellWriteFindings(followGitignore: boolean): Promise<string[]> {
  const root = await realpath(await temporaryDirectory({ started: false }));
  const databasePath = join(root, "rerail.sqlite");
  await writeFile(join(root, ".gitignore"), "gen/\nsessions/\nhome/\nrerail.sqlite*\n");
  execFileSync("git", ["init", "--quiet"], { cwd: root });
  const repositoryKey = startTestRepository(root, databasePath);
  const database = openRerailDatabase(databasePath);
  try {
    database.update(repositories).set({ followGitignore }).where(eq(repositories.repositoryKey, repositoryKey)).run();
  } finally {
    database.$client.close();
  }
  const runtime = new RerailRuntime({
    storageRoot: join(root, "sessions"), databasePath, workspaceRoot: root, autostart: false, embedder: testEmbedder,
    createAuditor: options => new NativeCodeRuleEnforcer(needleCatalog(), options),
  });
  const audits = new SqliteAuditStore(databasePath);
  const sessionId = "follow-gitignore";
  try {
    await runtime.ingest({ kind: "session-start", eventId: "start", session: { host: "codex", sessionId, cwd: root } });
    await runtime.codeRulesOpened(sessionId);
    // The baseline walk has run before the shell writes, so both sources are new to the session.
    await runtime.ingest({ kind: "turn-end", eventId: "baseline", sessionId, text: "" });
    await runtime.ingest({
      kind: "call", eventId: "shell", sessionId, toolCallId: "shell-1", toolName: "shell",
      input: { command: ["bash", "-lc", "python3 generate.py"] },
    });
    await mkdir(join(root, "gen"));
    await writeFile(join(root, "gen", "client.py"), "NEEDLE\n");
    await writeFile(join(root, "kept.py"), "NEEDLE\n");
    await runtime.ingest({ kind: "result", eventId: "shell-result", sessionId, toolCallId: "shell-1", toolName: "shell", output: "" });
    const findings = (): string[] => audits.list({ sessionId }).audits
      .filter(audit => audit.scope === "ai-output" && audit.status === "completed")
      .flatMap(audit => audits.get(audit.auditId)?.violations ?? [])
      .map(violation => violation.path ?? "")
      .toSorted();
    await vi.waitFor(() => expect(findings()).toContain("kept.py"), { timeout: 10_000 });
    return findings();
  } finally {
    await runtime.dispose();
    audits.close();
  }
}

describe("following .gitignore", () => {
  it("never audits a shell write under a gitignored folder while the repository follows .gitignore", async () => {
    expect(await shellWriteFindings(true)).toEqual(["kept.py"]);
  }, 30_000);

  it("audits it like any other source once the repository stops following .gitignore", async () => {
    expect(await shellWriteFindings(false)).toEqual(["gen/client.py", "kept.py"]);
  }, 30_000);
});

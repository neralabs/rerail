import { execFileSync } from "node:child_process";
import { realpath, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";
import { CodeRuleBridge, codeRuleSchema, NativeCodeRuleEnforcer, SqliteAuditStore } from "../../src/engine/index.js";
import { RerailRuntime } from "../../src/runtime/index.js";
import { temporaryDirectories, testEmbedder } from "../support/hook-events.js";
import { testCodeRuleCatalog } from "../support/repository-rules.js";
import { startTestRepository } from "../support/started-repository.js";

// A tool call whose result never comes: Claude Code asks before a write, and when the person refuses it sends no
// PostToolUse; a hook's result is also lost while the daemon is down. The call must not hold its file out of the
// session's later checks, nor come back after a reopen, and what it did write is still checked.

const temporaryDirectory = temporaryDirectories("rerail-unresolved-calls-");

const needleCatalog = () => testCodeRuleCatalog([codeRuleSchema.parse({
  format: "rerail.policy/v1", id: "test:needle", revision: 1, mode: "steer",
  scope: { include: ["**/*.py"], exclude: [] },
  clauses: [{
    id: "needle",
    violation: { code: "NEEDLE", anchor: "witness.span", message: "No needle" },
    forbid: { regex: { pattern: "NEEDLE", source: "file.content" } },
  }],
})]);

interface Checkout {
  root: string;
  databasePath: string;
  /** A Claude Code session's runtime in the checkout, its evaluator open and past its baseline walk. */
  open(sessionId: string): Promise<RerailRuntime>;
  /** The paths the checkout's completed ai-output audits found the needle in. */
  findings(): string[];
}

async function checkout(): Promise<Checkout> {
  const root = await realpath(await temporaryDirectory({ started: false }));
  const databasePath = join(root, "rerail.sqlite");
  await writeFile(join(root, ".gitignore"), "sessions/\nhome/\nrerail.sqlite*\n");
  execFileSync("git", ["init", "--quiet"], { cwd: root });
  startTestRepository(root, databasePath);
  return {
    root,
    databasePath,
    async open(sessionId) {
      const runtime = new RerailRuntime({
        storageRoot: join(root, "sessions"), databasePath, workspaceRoot: root, autostart: false, embedder: testEmbedder,
        createAuditor: options => new NativeCodeRuleEnforcer(needleCatalog(), options),
      });
      await runtime.ingest({ kind: "session-start", eventId: `start-${sessionId}`, session: { host: "claude-code", sessionId, cwd: root } });
      await runtime.codeRulesOpened(sessionId);
      await runtime.ingest({ kind: "turn-end", eventId: `baseline-${sessionId}`, sessionId, text: "" });
      return runtime;
    },
    findings() {
      const audits = new SqliteAuditStore(databasePath);
      try {
        return audits.list({}).audits
          .filter(audit => audit.scope === "ai-output" && audit.status === "completed")
          .flatMap(audit => audits.get(audit.auditId)?.violations ?? [])
          .map(violation => violation.path ?? "")
          .toSorted();
      } finally {
        audits.close();
      }
    },
  };
}

/** A Write of `a.py`: its call only, as the test never sends its result. */
const writeCall = (root: string, sessionId: string) => ({
  kind: "call" as const, eventId: "write-call", sessionId, toolCallId: "write", toolName: "Write",
  input: { file_path: join(root, "a.py"), content: "NEEDLE\n" },
});

describe("a tool call whose result never comes", () => {
  it("leaves its file to the next walk once the turn ends, so a shell write to it in the next turn is checked", async () => {
    const { root, open, findings } = await checkout();
    const runtime = await open("refused");
    try {
      await runtime.ingest(writeCall(root, "refused"));
      await runtime.ingest({ kind: "turn-end", eventId: "turn-1", sessionId: "refused", text: "" });
      // Refused, the write left the file as it was: there is nothing to check.
      expect(findings()).toEqual([]);

      await runtime.ingest({ kind: "call", eventId: "shell-call", sessionId: "refused", toolCallId: "shell", toolName: "Bash", input: { command: "echo NEEDLE > a.py" } });
      await writeFile(join(root, "a.py"), "NEEDLE\n");
      await runtime.ingest({ kind: "result", eventId: "shell-result", sessionId: "refused", toolCallId: "shell", toolName: "Bash", output: "" });

      await vi.waitFor(() => expect(findings()).toEqual(["a.py"]), { timeout: 10_000 });
    } finally {
      await runtime.dispose();
    }
  }, 30_000);

  it("is not begun again when the session opens again", async () => {
    const { root, open } = await checkout();
    const first = await open("reopened");
    try {
      await first.ingest(writeCall(root, "reopened"));
      await first.ingest({ kind: "turn-end", eventId: "turn-1", sessionId: "reopened", text: "" });
    } finally {
      await first.dispose();
    }

    const begun = vi.spyOn(CodeRuleBridge.prototype, "begin");
    try {
      const reopened = await open("reopened");
      await reopened.dispose();
      expect(begun).not.toHaveBeenCalled();
    } finally {
      begun.mockRestore();
    }
  }, 30_000);

  it("has what it wrote checked against what it captured at the next stop, when the result was lost while the daemon was down", async () => {
    const { root, open, findings } = await checkout();
    await writeFile(join(root, "a.py"), "x = 1\n");
    const first = await open("lost");
    try {
      await first.ingest(writeCall(root, "lost"));
    } finally {
      await first.dispose();
    }
    // The write lands while no runtime runs, so the next one's walks take the file as they find it.
    await writeFile(join(root, "a.py"), "NEEDLE\n");

    // Opening ends with a turn end: the session's next stop.
    const reopened = await open("lost");
    try {
      await vi.waitFor(() => expect(findings()).toEqual(["a.py"]), { timeout: 10_000 });
    } finally {
      await reopened.dispose();
    }
  }, 30_000);
});

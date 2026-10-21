import { createHash } from "node:crypto";
import { readFile, realpath, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";
import {
  codeRuleSchema,
  NativeCodeRuleEnforcer,
  SqliteAuditControlStore,
  type AdapterDelivery,
  type CodeRuleAuditor,
  type RerailHost,
  type SteerStrategy,
} from "../../src/engine/index.js";
import { RerailRuntime, type RerailRuntimeOptions } from "../../src/runtime/runtime.js";
import { temporaryDirectories, testEmbedder } from "../support/hook-events.js";
import { testCodeRuleCatalog } from "../support/repository-rules.js";

const temporaryDirectory = temporaryDirectories("rerail-steer-strategy-");

/** One steer rule flagging `NEEDLE` in any Python source. */
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
 * A session in a fresh checkout whose saved steer strategy is `strategy` (none saved when undefined), with `files`
 * there before it starts. `edit` writes a file through a tool call and answers with the result's deliveries.
 */
async function strategySession(options: {
  strategy?: SteerStrategy;
  host: RerailHost;
  files?: Readonly<Record<string, string>>;
  createAuditor?: RerailRuntimeOptions["createAuditor"];
}) {
  const root = await realpath(await temporaryDirectory());
  for (const [path, text] of Object.entries(options.files ?? {})) await writeFile(join(root, path), text);
  const databasePath = join(root, "rerail.sqlite");
  if (options.strategy !== undefined) {
    // Saved before any session exists there, as the benchmark seeds a checkout.
    const controls = new SqliteAuditControlStore(databasePath);
    controls.updateWorkspaceSettings(root, { steerStrategy: options.strategy });
    controls.close();
  }
  const runtime = new RerailRuntime({
    storageRoot: join(root, "sessions"), databasePath, workspaceRoot: root, autostart: false, embedder: testEmbedder,
    createAuditor: options.createAuditor ?? (auditorOptions => new NativeCodeRuleEnforcer(needleCatalog(), auditorOptions)),
  });
  const sessionId = "strategy";
  await runtime.ingest({ kind: "session-start", eventId: "start", session: { host: options.host, sessionId, cwd: root } });
  // A boundary reached while the evaluator is still opening goes on without code rules.
  await runtime.codeRulesOpened(sessionId);
  let event = 0;
  const call = (toolCallId: string, path: string) => runtime.ingest({
    kind: "call", eventId: `call-${++event}`, sessionId, toolCallId, toolName: "write", input: { path, content: "" },
  });
  const result = (toolCallId: string) => runtime.ingest({
    kind: "result", eventId: `result-${++event}`, sessionId, toolCallId, toolName: "write", output: "written",
  });
  return {
    root,
    runtime,
    call,
    result,
    edit: async (toolCallId: string, path: string, text: string) => {
      await call(toolCallId, path);
      await writeFile(join(root, path), text);
      return await result(toolCallId);
    },
    turnEnd: () => runtime.ingest({ kind: "turn-end", eventId: `turn-${++event}`, sessionId, text: "done" }),
    prompt: () => runtime.ingest({ kind: "user-prompt", eventId: `prompt-${++event}`, sessionId, text: "next" }),
    /** The deliveries the session's ledger recorded. */
    ledger: async (): Promise<AdapterDelivery[]> => {
      const directory = createHash("sha256").update(sessionId).digest("hex");
      const lines = (await readFile(join(root, "sessions", directory, "events.jsonl"), "utf8")).split("\n").filter(Boolean);
      return lines.map(line => JSON.parse(line) as { type: string; delivery?: AdapterDelivery })
        .flatMap(entry => (entry.type === "delivery" && entry.delivery !== undefined ? [entry.delivery] : []));
    },
  };
}

describe("The rewind steer strategy", () => {
  for (const host of ["omp", "claude-code"] as const) {
    it(`rolls back a new file that breaks a rule, and steers in the edit's own result (${host})`, async () => {
      const session = await strategySession({ strategy: "rewind", host });
      try {
        const deliveries = await session.edit("w1", "created.py", "NEEDLE = 1\n");
        expect(deliveries).toEqual([expect.objectContaining({
          strategy: "rewind",
          toolCallId: "w1",
          rewound: ["created.py"],
          ruleIds: ["test:needle"],
          content: expect.stringMatching(/^Rerail rolled back this edit because it breaks the rule below: `created\.py`, which the edit created, is deleted\. Redo it so it follows the rule; if you decide the rule does not apply, redo it as it was and tell the user in one line why\.\n\nRerail found 1 problem in your last edit:\n- created\.py:1:1 — No needle \[test:needle\]\nFix it, or tell the user in one line why you won't\.$/u),
        })]);
        await expect(readFile(join(session.root, "created.py"))).rejects.toThrow(/ENOENT/u);
        expect(await session.turnEnd()).toEqual([]);
        expect(await session.ledger()).toEqual([expect.objectContaining({ strategy: "rewind", toolCallId: "w1", rewound: ["created.py"] })]);
      } finally {
        await session.runtime.dispose();
      }
    });
  }

  it("puts an existing file back as it was, and the violations it already had steer nothing more", async () => {
    const before = "NEEDLE = 0\n";
    const session = await strategySession({ strategy: "rewind", host: "omp", files: { "existing.py": before } });
    try {
      const deliveries = await session.edit("w1", "existing.py", `${before}NEEDLE = 1\n`);
      expect(deliveries).toEqual([expect.objectContaining({
        strategy: "rewind", toolCallId: "w1", rewound: ["existing.py"],
        content: expect.stringContaining("Rerail rolled back this edit because it breaks the rule below: `existing.py` is back as before the edit."),
      })]);
      expect(await readFile(join(session.root, "existing.py"), "utf8")).toBe(before);
      // The turn's walk sees Rerail's own write, not an uncaptured edit, and the restored file's own violation is not steered.
      expect(await session.turnEnd()).toEqual([]);
      expect(await session.prompt()).toEqual([]);
      // A later edit is judged against the restored file.
      expect(await session.edit("w2", "existing.py", `${before}NEEDLE = 2\n`)).toEqual([expect.objectContaining({ toolCallId: "w2", rewound: ["existing.py"] })]);
      expect(await readFile(join(session.root, "existing.py"), "utf8")).toBe(before);
    } finally {
      await session.runtime.dispose();
    }
  });

  describe("from a one-shot hook", () => {
    /** A Claude Code session whose edit check steers for `a.ts` only once the test opens `check`. */
    async function hookSession() {
      const check = Promise.withResolvers<void>();
      const checking = Promise.withResolvers<void>();
      const steered = Promise.withResolvers<void>();
      const session = await strategySession({
        strategy: "rewind",
        host: "claude-code",
        files: { "a.ts": "export const a = 1;\n" },
        createAuditor: ({ steerDelivery }) => {
          let revision = 0;
          const auditor: CodeRuleAuditor = {
            openWorkspace: async () => undefined,
            acceptsPath: () => true,
            auditGeneratedEdits: async (_sessionId, batch) => {
              // Rerail's own rollback is the only batch audited in log mode.
              if (batch.mode === undefined) revision = batch.revision;
              return { revision: batch.revision, disposition: "continue", violations: [], failures: [] };
            },
            settleGeneratedEdits: async () => {
              if (revision === 0) return;
              const steering = revision;
              revision = 0;
              checking.resolve();
              await check.promise;
              await steerDelivery.requestSteer("strategy", "steer", "Checked the edit.", {
                revision: steering, scope: ["a.ts"], background: true, generatedEdit: { violations: [], sourceOf: () => undefined },
              });
              steered.resolve();
            },
            close: async () => undefined,
          };
          return auditor;
        },
      });
      await session.call("t1", "a.ts");
      await writeFile(join(session.root, "a.ts"), "export const a: any = 1;\n");
      return { ...session, check, checking, steered };
    }

    it("returns the rolled-back edit's steer from the tool result", async () => {
      const session = await hookSession();
      try {
        const answering = session.result("t1");
        session.check.resolve();
        expect(await answering).toEqual([expect.objectContaining({
          strategy: "rewind", toolCallId: "t1", rewound: ["a.ts"], content: expect.stringMatching(/\n\nChecked the edit\.$/u),
        })]);
        expect(await readFile(join(session.root, "a.ts"), "utf8")).toBe("export const a = 1;\n");
      } finally {
        session.check.resolve();
        await session.runtime.dispose();
      }
    });

    it("sends a steer that arrives after the result answered as a message, and leaves the edit in place", async () => {
      const session = await hookSession();
      vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
      try {
        const answering = session.result("t1");
        await session.checking.promise;
        await vi.advanceTimersByTimeAsync(1_000);
        expect(await answering).toEqual([]);
        vi.useRealTimers();
        session.check.resolve();
        await session.steered.promise;
        const next = await session.prompt();
        expect(next).toEqual([{
          sessionId: "strategy", requested: "steer", mechanism: "additional-context", content: "Checked the edit.", ruleIds: [], rules: [], strategy: "message",
        }]);
        expect(await readFile(join(session.root, "a.ts"), "utf8")).toBe("export const a: any = 1;\n");
      } finally {
        vi.useRealTimers();
        session.check.resolve();
        await session.runtime.dispose();
      }
    });
  });
});

describe("The message steer strategy", () => {
  it("steers as Rerail always has, leaving the edit in place", async () => {
    const session = await strategySession({ host: "omp" });
    try {
      const deliveries = await session.edit("w1", "created.py", "NEEDLE = 1\n");
      expect(deliveries).toEqual([{
        sessionId: "strategy", requested: "steer", mechanism: "native-queue", ruleIds: ["test:needle"], strategy: "message",
        rules: [{ id: "test:needle", state: "steered", origin: "From your rule library." }],
        content: expect.stringMatching(/^Rerail found 1 problem in your last edit:\n- created\.py:1:1 — No needle \[test:needle\]\nFix it, or tell the user in one line why you won't\.$/u),
      }]);
      expect(await readFile(join(session.root, "created.py"), "utf8")).toBe("NEEDLE = 1\n");
    } finally {
      await session.runtime.dispose();
    }
  });
});

describe("The subagent steer strategy", () => {
  it("steers with the instruction to hand the fix to the host's subagent tool", async () => {
    const session = await strategySession({ strategy: "subagent", host: "pi" });
    try {
      const deliveries = await session.edit("w1", "created.py", "NEEDLE = 1\n");
      expect(deliveries).toEqual([expect.objectContaining({
        strategy: "subagent",
        content: expect.stringMatching(/^Rerail found 1 problem in your last edit:\n- created\.py:1:1 — No needle \[test:needle\]\nFix it, or tell the user in one line why you won't\.\n\nThis note comes from Rerail, not the user, and your task is unchanged\. Hand the fix to a subagent with the `rerail_fix` tool/u),
      })]);
      expect(deliveries[0]).not.toHaveProperty("toolCallId");
      expect(await readFile(join(session.root, "created.py"), "utf8")).toBe("NEEDLE = 1\n");
    } finally {
      await session.runtime.dispose();
    }
  });
});

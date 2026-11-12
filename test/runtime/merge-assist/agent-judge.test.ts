import { existsSync } from "node:fs";
import { realpath } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { z } from "zod";
import { mergeAssistJudgments, openRerailDatabase, type RerailDatabase } from "../../../src/engine/index.js";
import { createAgentJudge, JUDGE_OUTPUT_LIMIT_BYTES, type JudgeOptions, type JudgeTask } from "../../../src/runtime/merge-assist/agent-judge.js";
import { temporaryDirectories } from "../../support/hook-events.js";
import { agentBinDirectories, fakeAgentCli, type FakeRun } from "./support/fake-agent-cli.js";

const temporaryDirectory = temporaryDirectories("rerail-agent-judge-");
const binDirectory = agentBinDirectories();
const databases: RerailDatabase[] = [];
afterEach(() => {
  for (const database of databases.splice(0)) database.$client.close();
});

/**
 * Real output recorded from `codex exec` 0.158.0 with the judge's flags, a prompt of `Reply with {"ok":true}` and the
 * schema `{"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"],"additionalProperties":false}`:
 * the final message goes to stdout and, without a trailing newline, to the `-o` file; the banner goes to stderr.
 */
const CODEX_RECORDED = {
  stdout: "{\"ok\":true}\n",
  answer: "{\"ok\":true}",
  stderr: [
    "OpenAI Codex v0.158.0", "--------", "workdir: /private/var/folders/x/T/rerail-judge-abc", "model: gpt-6-luna", "provider: openai",
    "approval: never", "sandbox: read-only", "reasoning effort: none", "reasoning summaries: none", "session id: 01a19de4-f31b-71b0-acf3-36a825cf26e1",
    "--------", "user", "Reply with {\"ok\":true}", "codex", "{\"ok\":true}", "tokens used", "13,626", "",
  ].join("\n"),
} as const;

/** Real failure recorded from `codex exec` 0.158.0 when the configured model is not available to the account (exit 1). */
const CODEX_RECORDED_FAILURE = {
  code: 1,
  stderr: [
    "OpenAI Codex v0.158.0", "--------", "model: gpt-5.6-sol", "--------",
    "ERROR: {\"type\":\"error\",\"status\":400,\"error\":{\"type\":\"invalid_request_error\",\"message\":\"The 'gpt-5.6-sol' model is not supported when using Codex with a ChatGPT account.\",\"code\":\"model_not_found\"}}",
    "",
  ].join("\n"),
} as const;

/**
 * `claude -p --output-format json --json-schema` prints one result object with the answer in `structured_output`
 * (Claude Code headless and structured output docs). Claude Code was not installed where these tests were written,
 * so this shape is documented rather than recorded.
 */
function claudeResult(fields: Record<string, unknown>): string {
  return JSON.stringify({
    type: "result", subtype: "success", is_error: false, duration_ms: 2140, num_turns: 1, result: "",
    session_id: "5f0c0d8e-0000-4000-8000-000000000000", total_cost_usd: 0.01, ...fields,
  });
}

const okSchema = z.object({ ok: z.boolean() }).strict();
const okJsonSchema = { type: "object", properties: { ok: { type: "boolean" } }, required: ["ok"], additionalProperties: false };

function task(overrides: Partial<JudgeTask<{ ok: boolean }>> = {}): JudgeTask<{ ok: boolean }> {
  return {
    kind: "summary",
    repositoryKey: "local:judge-test",
    prompt: "Reply with {\"ok\":true}",
    jsonSchema: okJsonSchema,
    parse: value => okSchema.parse(value),
    cacheKey: "a".repeat(64),
    ...overrides,
  };
}

async function setup(cli: readonly string[], behave: (call: { command: string; args: string[] }) => FakeRun, options: Partial<JudgeOptions> = {}) {
  const root = await realpath(await temporaryDirectory({ started: false }));
  const database = openRerailDatabase(join(root, "rerail.sqlite"));
  databases.push(database);
  const env: NodeJS.ProcessEnv = { PATH: binDirectory(cli), HOME: root };
  const fake = fakeAgentCli(behave);
  const judge = createAgentJudge({ database, env, spawn: fake.spawn, ...options });
  return { judge, calls: fake.calls, env, database, bin: env.PATH! };
}

function rows(database: RerailDatabase) {
  return database.select().from(mergeAssistJudgments).all();
}

describe("agent judge invocations", () => {
  it("runs claude without a shell in a private temporary directory, with the prompt on stdin, and removes the directory", async () => {
    const { judge, calls, env, bin } = await setup(["claude", "codex"], () => ({ stdout: claudeResult({ structured_output: { ok: true } }) }));

    const outcome = await judge.judge(task());

    expect(outcome).toEqual({ ok: true, value: { ok: true }, runner: "claude", cached: false });
    const [call] = calls;
    expect(call!.command).toBe(join(bin, "claude"));
    expect(call!.args).toEqual([
      "-p", "--output-format", "json", "--json-schema", JSON.stringify(okJsonSchema),
      "--tools", "", "--disallowedTools", "mcp__*", "--strict-mcp-config",
      "--settings", "{\"disableAllHooks\":true}", "--setting-sources", "user",
      "--no-session-persistence", "--max-turns", "2",
    ]);
    expect(call!.options.shell).toBe(false);
    expect(call!.options.stdio).toEqual(["pipe", "pipe", "pipe"]);
    expect(call!.options.env).toBe(env);
    expect(call!.stdin).toBe("Reply with {\"ok\":true}");
    const cwd = call!.options.cwd!;
    expect(dirname(cwd)).toBe(tmpdir());
    expect(cwd.startsWith(process.cwd())).toBe(false);
    expect(call!.cwdMode).toBe(0o700);
    expect(existsSync(cwd)).toBe(false);
  });

  it("runs codex with tools, user config and rules off, the schema in a file and the recorded answer read from -o", async () => {
    const { judge, calls, bin } = await setup(["codex"], () => CODEX_RECORDED);

    const outcome = await judge.judge(task());

    expect(outcome).toEqual({ ok: true, value: { ok: true }, runner: "codex", cached: false });
    const [call] = calls;
    const cwd = call!.options.cwd!;
    expect(call!.command).toBe(join(bin, "codex"));
    expect(call!.args).toEqual([
      "exec", "--sandbox", "read-only", "--skip-git-repo-check", "--ephemeral", "--ignore-user-config", "--ignore-rules",
      "--disable", "shell_tool", "--disable", "unified_exec", "--disable", "code_mode_host", "--disable", "multi_agent",
      "--output-schema", join(cwd, "schema.json"), "-o", join(cwd, "answer.json"), "-",
    ]);
    expect(JSON.parse(call!.schemaText!)).toEqual(okJsonSchema);
    expect(call!.stdin).toBe("Reply with {\"ok\":true}");
    expect(call!.options.shell).toBe(false);
    expect(dirname(cwd)).toBe(tmpdir());
    expect(call!.cwdMode).toBe(0o700);
    expect(existsSync(cwd)).toBe(false);
  });

  it("reports codex's recorded failure as failed with a short stderr tail free of escape sequences", async () => {
    const { judge } = await setup(["codex"], () => ({ ...CODEX_RECORDED_FAILURE, stderr: `\u001B[31m${CODEX_RECORDED_FAILURE.stderr}\u001B[0m` }));

    const outcome = await judge.judge(task());

    expect(outcome.ok).toBe(false);
    if (outcome.ok) return;
    expect(outcome.reason).toBe("failed");
    expect(outcome.detail).toMatch(/^Codex failed \(exit 1\): /u);
    expect(outcome.detail).toContain("model_not_found");
    expect(outcome.detail).not.toContain("\u001B");
    expect(outcome.detail.length).toBeLessThanOrEqual("Codex failed (exit 1): ".length + 300);
  });

  it("treats codex exiting without a final message, or with one that is not JSON, as invalid", async () => {
    const answers = [undefined, "Sure! Here is the JSON you asked for."];
    const { judge } = await setup(["codex"], () => {
      const answer = answers.shift();
      return answer === undefined ? { stdout: "" } : { answer };
    });

    const none = await judge.judge(task({ cacheKey: "1".repeat(64) }));
    const prose = await judge.judge(task({ cacheKey: "2".repeat(64) }));

    expect(none).toMatchObject({ ok: false, reason: "invalid" });
    expect(prose).toMatchObject({ ok: false, reason: "invalid" });
  });

  it("reads claude's structured_output and treats a missing one as invalid and an error result as failed", async () => {
    const outputs = [
      claudeResult({ result: "{\"ok\":true}" }),
      claudeResult({ subtype: "error_max_turns", is_error: true }),
      "not json at all",
    ];
    const { judge } = await setup(["claude"], () => ({ stdout: outputs.shift()! }));

    const missing = await judge.judge(task({ cacheKey: "1".repeat(64) }));
    const error = await judge.judge(task({ cacheKey: "2".repeat(64) }));
    const garbage = await judge.judge(task({ cacheKey: "3".repeat(64) }));

    expect(missing).toEqual({ ok: false, reason: "invalid", detail: "Claude Code returned no structured output." });
    expect(error).toMatchObject({ ok: false, reason: "failed" });
    expect(!error.ok && error.detail).toContain("error_max_turns");
    expect(garbage).toMatchObject({ ok: false, reason: "invalid" });
  });

  it("rejects an answer that does not match the schema as invalid and records the attempt", async () => {
    const { judge, database } = await setup(["codex"], () => ({ answer: "{\"ok\":\"yes\",\"extra\":1}" }));

    const outcome = await judge.judge(task());

    expect(outcome).toMatchObject({ ok: false, reason: "invalid" });
    expect(!outcome.ok && outcome.detail).toMatch(/^Codex's answer did not match the schema: /u);
    expect(rows(database).map(row => row.task)).toEqual(["summary:failed"]);
  });

  it("stops a run that outlives the timeout with SIGTERM, then SIGKILL, and reports timeout", async () => {
    const { judge, calls } = await setup(["codex"], () => ({ hang: true, ignoreTerm: true }), { timeoutMs: 50 });

    const outcome = await judge.judge(task());

    expect(outcome).toMatchObject({ ok: false, reason: "timeout" });
    expect(calls[0]!.signals).toEqual(["SIGTERM", "SIGKILL"]);
    expect(existsSync(calls[0]!.options.cwd!)).toBe(false);
  });

  it("stops a run that writes more than the stdout cap and reports it failed", async () => {
    const { judge, calls } = await setup(["claude"], () => ({ hang: true, stdout: Buffer.alloc(JUDGE_OUTPUT_LIMIT_BYTES + 1024, 0x61) }));

    const outcome = await judge.judge(task());

    expect(outcome).toMatchObject({ ok: false, reason: "failed" });
    expect(!outcome.ok && outcome.detail).toContain("more than 1 MiB");
    expect(calls[0]!.signals).toEqual(["SIGTERM"]);
  });

  it("reports a CLI that cannot start as failed", async () => {
    const { judge } = await setup(["codex"], () => ({ spawnError: "spawn codex EACCES" }));

    const outcome = await judge.judge(task());

    expect(outcome).toMatchObject({ ok: false, reason: "failed" });
    expect(!outcome.ok && outcome.detail).toContain("EACCES");
  });
});

describe("agent judge budget and cache", () => {
  it("answers a repeated task from the cache without a call and without using budget", async () => {
    const { judge, calls, database } = await setup(["codex"], () => CODEX_RECORDED);

    const first = await judge.judge(task());
    const second = await judge.judge(task());

    expect(first).toMatchObject({ ok: true, cached: false });
    expect(second).toEqual({ ok: true, value: { ok: true }, runner: "codex", cached: true });
    expect(calls).toHaveLength(1);
    expect(judge.budget("local:judge-test")).toEqual({ used: 1, limit: 40, windowMinutes: 60 });
    expect(rows(database)).toEqual([expect.objectContaining({ key: "a".repeat(64), task: "summary", runner: "codex", resultJson: "{\"ok\":true}" })]);
  });

  it("answers from the cache even when no agent CLI is on PATH", async () => {
    const { judge, database } = await setup(["codex"], () => CODEX_RECORDED);
    await judge.judge(task());

    const offline = createAgentJudge({ database, env: { PATH: "" }, spawn: fakeAgentCli(() => ({})).spawn });

    expect(await offline.judge(task())).toEqual({ ok: true, value: { ok: true }, runner: "codex", cached: true });
  });

  it("re-asks when a cached answer no longer passes the task's parser", async () => {
    const { judge, calls } = await setup(["codex"], () => CODEX_RECORDED);
    await judge.judge(task());

    const stricter = await judge.judge(task({ parse: value => z.object({ ok: z.literal(true), why: z.string() }).parse(value) }));

    expect(calls).toHaveLength(2);
    expect(stricter).toMatchObject({ ok: false, reason: "invalid" });
  });

  it("refuses without spawning once the hour's calls, failed ones included, reach the limit, and allows them again an hour later", async () => {
    let clock = 1_800_000_000_000;
    const runs: FakeRun[] = [CODEX_RECORDED, CODEX_RECORDED_FAILURE, CODEX_RECORDED];
    const { judge, calls } = await setup(["codex"], () => runs.shift()!, { limitPerHour: 2, now: () => clock });

    expect(await judge.judge(task({ cacheKey: "1".repeat(64) }))).toMatchObject({ ok: true });
    expect(await judge.judge(task({ cacheKey: "2".repeat(64) }))).toMatchObject({ ok: false, reason: "failed" });
    const refused = await judge.judge(task({ cacheKey: "3".repeat(64) }));

    expect(refused).toMatchObject({ ok: false, reason: "budget" });
    expect(calls).toHaveLength(2);
    expect(judge.budget("local:judge-test")).toEqual({ used: 2, limit: 2, windowMinutes: 60 });
    expect(judge.budget("local:other")).toMatchObject({ used: 0 });

    clock += 61 * 60_000;
    expect(await judge.judge(task({ cacheKey: "3".repeat(64) }))).toMatchObject({ ok: true });
    expect(calls).toHaveLength(3);
  });

  it("counts calls still running against the budget", async () => {
    const { judge, calls } = await setup(["codex"], () => CODEX_RECORDED, { limitPerHour: 1 });

    const [first, second] = await Promise.all([judge.judge(task({ cacheKey: "1".repeat(64) })), judge.judge(task({ cacheKey: "2".repeat(64) }))]);

    expect(first).toMatchObject({ ok: true });
    expect(second).toMatchObject({ ok: false, reason: "budget" });
    expect(calls).toHaveLength(1);
  });
});

describe("agent judge readiness", () => {
  it("prefers the session's own agent CLI and otherwise uses Claude Code, then Codex", async () => {
    const runner = async (cli: readonly string[], preferredHost?: JudgeOptions["preferredHost"]) =>
      (await setup(cli, () => ({}), preferredHost === undefined ? {} : { preferredHost })).judge.readiness().runner;

    expect(await runner(["claude", "codex"])).toBe("claude");
    expect(await runner(["claude", "codex"], "codex")).toBe("codex");
    expect(await runner(["claude", "codex"], "claude-code")).toBe("claude");
    expect(await runner(["claude", "codex"], "omp")).toBe("claude");
    expect(await runner(["claude", "codex"], "pi")).toBe("claude");
    expect(await runner(["codex"], "claude-code")).toBe("codex");
  });

  it("is unavailable without spawning when neither CLI is on PATH", async () => {
    const { judge, calls } = await setup([], () => ({}));

    expect(judge.readiness()).toEqual({
      available: false,
      runner: null,
      problem: "Merge Assist judges with Claude Code or Codex, and neither `claude` nor `codex` is on PATH.",
    });
    expect(await judge.judge(task())).toEqual({ ok: false, reason: "unavailable", detail: judge.readiness().problem });
    expect(calls).toHaveLength(0);
  });
});

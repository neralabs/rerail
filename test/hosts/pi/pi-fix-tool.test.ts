import { describe, expect, it } from "vitest";
import { piFixInvocation, readPiFixRun } from "../../../src/hosts/pi/pi-fix-tool.js";

describe("rerail_fix", () => {
  it("runs Pi's own entry point on the parent's model and thinking level, without a session or extensions", () => {
    const invocation = piFixInvocation(
      "Rule no-any: src/a.ts:3 uses `any`.",
      { provider: "anthropic", model: "claude-sonnet-4-5", thinkingLevel: "high" },
      "/usr/local/bin/node",
      import.meta.filename,
    );
    expect(invocation.command).toBe("/usr/local/bin/node");
    expect(invocation.args.slice(0, -1)).toEqual([
      import.meta.filename, "--mode", "json", "-p", "--no-session", "--no-extensions",
      "--provider", "anthropic", "--model", "claude-sonnet-4-5", "--thinking", "high",
    ]);
    const prompt = invocation.args.at(-1)!;
    expect(prompt).toContain("smallest change");
    expect(prompt).toContain("Do not touch unrelated code");
    expect(prompt).toContain("one sentence");
    expect(prompt.endsWith("Rule no-any: src/a.ts:3 uses `any`.")).toBe(true);
  });

  it("leaves the model to the child's defaults when the parent has none, and falls back to `pi` on PATH", () => {
    expect(piFixInvocation("Fix it.", {}, "/usr/local/bin/node", "/nonexistent/pi.js")).toEqual({
      command: "pi",
      args: ["--mode", "json", "-p", "--no-session", "--no-extensions", expect.stringContaining("Fix it.")],
    });
    expect(piFixInvocation("Fix it.", {}, "/opt/pi/bin/pi", undefined).command).toBe("/opt/pi/bin/pi");
  });

  it("sums usage over the child's assistant messages and returns its last text", () => {
    const assistant = (text: string, usage: object, extra: object = {}) => JSON.stringify({
      type: "message_end",
      message: { role: "assistant", model: "claude-sonnet-4-5", content: [{ type: "text", text }], usage, ...extra },
    });
    const stdout = [
      JSON.stringify({ type: "message_end", message: { role: "user", content: "task" } }),
      "not json",
      assistant("Looking.", { input: 100, output: 10, cacheRead: 50, cacheWrite: 5, cost: { total: 0.01 } }),
      JSON.stringify({ type: "tool_result_end", message: { role: "toolResult", content: [] } }),
      assistant("Replaced `any` with `unknown` in src/a.ts.", { input: 200, output: 20, cacheRead: 0, cacheWrite: 0, cost: { total: 0.02 } }, { stopReason: "stop" }),
      "",
    ].join("\n");
    const run = readPiFixRun(stdout, 0, "fallback-model");
    expect(run).toEqual({
      text: "Replaced `any` with `unknown` in src/a.ts.",
      failed: false,
      details: { exitCode: 0, model: "claude-sonnet-4-5", usage: { input: 300, output: 30, cacheRead: 50, cacheWrite: 5, cost: expect.closeTo(0.03), turns: 2 } },
    });
  });

  it("fails a child that exits nonzero or whose last message errored", () => {
    expect(readPiFixRun("", 1, "claude-sonnet-4-5")).toEqual({
      text: "",
      failed: true,
      details: { exitCode: 1, model: "claude-sonnet-4-5", usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0, turns: 0 } },
    });
    const errored = JSON.stringify({ type: "message_end", message: { role: "assistant", content: [], stopReason: "error", errorMessage: "rate limited" } });
    expect(readPiFixRun(errored, 0)).toMatchObject({ failed: true, errorMessage: "rate limited", details: { exitCode: 0, usage: { turns: 1 } } });
  });
});

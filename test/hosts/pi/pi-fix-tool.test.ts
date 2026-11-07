import { describe, expect, it } from "vitest";
import { PI_FIX_CHILD_TOOLS, PiFixReader, STDERR_TAIL_CHARS, piFixInvocation, runPiFix, type PiFixRun } from "../../../src/hosts/pi/pi-fix-tool.js";

/** `stdout` read as a child streams it: in small chunks that split lines. */
function readStreamed(stdout: string, exitCode: number, model?: string): PiFixRun {
  const reader = new PiFixReader();
  for (let start = 0; start < stdout.length; start += 7) reader.write(stdout.slice(start, start + 7));
  return reader.finish(exitCode, model);
}

/** A child `node -e` script in place of `pi`. */
const node = (script: string): { command: string; args: string[] } => ({ command: process.execPath, args: ["-e", script] });

describe("rerail_fix", () => {
  it("runs Pi's own entry point on the parent's model and thinking level, without a session, extensions or a shell", () => {
    const invocation = piFixInvocation(
      "Rule no-any: src/a.ts:3 uses `any`.",
      { provider: "anthropic", model: "claude-sonnet-4-5", thinkingLevel: "high" },
      "/usr/local/bin/node",
      import.meta.filename,
    );
    expect(invocation.command).toBe("/usr/local/bin/node");
    expect(invocation.args.slice(0, -1)).toEqual([
      import.meta.filename, "--mode", "json", "-p", "--no-session", "--no-extensions", "--tools", "read,edit,write,grep,find,ls",
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
      args: ["--mode", "json", "-p", "--no-session", "--no-extensions", "--tools", "read,edit,write,grep,find,ls", expect.stringContaining("Fix it.")],
    });
    expect(piFixInvocation("Fix it.", {}, "/opt/pi/bin/pi", undefined).command).toBe("/opt/pi/bin/pi");
  });

  it("gives the child Pi's file tools only, never bash", () => {
    const args = piFixInvocation("Fix it.", {}, "/usr/local/bin/node", undefined).args;
    const tools = args[args.indexOf("--tools") + 1]!.split(",");
    expect(tools).toEqual([...PI_FIX_CHILD_TOOLS]);
    expect(tools).not.toContain("bash");
  });

  it("sums usage over the child's streamed assistant messages and returns its last text", () => {
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
    const run = readStreamed(stdout, 0, "fallback-model");
    expect(run).toEqual({
      text: "Replaced `any` with `unknown` in src/a.ts.",
      failed: false,
      details: { exitCode: 0, model: "claude-sonnet-4-5", usage: { input: 300, output: 30, cacheRead: 50, cacheWrite: 5, cost: expect.closeTo(0.03), turns: 2 } },
    });
  });

  it("fails a child that exits nonzero or whose last message errored", () => {
    expect(readStreamed("", 1, "claude-sonnet-4-5")).toEqual({
      text: "",
      failed: true,
      details: { exitCode: 1, model: "claude-sonnet-4-5", usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0, turns: 0 } },
    });
    const errored = JSON.stringify({ type: "message_end", message: { role: "assistant", content: [], stopReason: "error", errorMessage: "rate limited" } });
    expect(readStreamed(errored, 0)).toMatchObject({ failed: true, errorMessage: "rate limited", details: { exitCode: 0, usage: { turns: 1 } } });
  });

  it("returns a finished child's last assistant text", async () => {
    const event = JSON.stringify({ type: "message_end", message: { role: "assistant", content: [{ type: "text", text: "Done." }] } });
    await expect(runPiFix(node(`process.stdout.write(${JSON.stringify(`${event}\n`)})`), process.cwd(), undefined, { model: "m" }))
      .resolves.toEqual({ text: "Done.", details: { exitCode: 0, model: "m", usage: expect.objectContaining({ turns: 1 }) } });
  });

  it("stops a child that outlives its timeout and fails with a timeout error", async () => {
    // A real child process: fake timers cannot drive it, so the timeout is real and short. The tool settles only once
    // the child has closed, so the rejection means it was stopped.
    await expect(runPiFix(node("setTimeout(() => {}, 1e9)"), process.cwd(), undefined, { timeoutMs: 100 }))
      .rejects.toThrow("rerail_fix timed out after 0.1s and was stopped");
  });

  it("fails with only the tail of a failed child's stderr", async () => {
    const script = "process.stderr.write('head' + 'x'.repeat(100000) + 'tail'); process.exitCode = 3;";
    const failure = await runPiFix(node(script), process.cwd(), undefined).catch((error: unknown) => error as Error);
    expect(failure).toBeInstanceOf(Error);
    const message = (failure as Error).message;
    expect(message.startsWith("rerail_fix failed (exit 3): x")).toBe(true);
    expect(message.endsWith("tail")).toBe(true);
    expect(message).not.toContain("head");
    expect(message.length).toBeLessThanOrEqual("rerail_fix failed (exit 3): ".length + STDERR_TAIL_CHARS);
  });
});

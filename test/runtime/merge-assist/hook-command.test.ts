import { tmpdir } from "node:os";
import { describe, expect, it } from "vitest";
import { MAX_HOOK_STATUS, runHookCommand, type HookAnswer, type HookPayload } from "../../../src/runtime/merge-assist/hook-command.js";
import { sampleConflict } from "../../support/merge-assist/status.js";

const payload: HookPayload = {
  version: 1, event: "detected", conflict: sampleConflict(), checkout: { path: tmpdir(), branch: "mine", base: "main" }, session: null,
};

/** The person's command answering `answer` on stdout. */
async function answering(answer: unknown): Promise<HookAnswer> {
  const script = `process.stdout.write(${JSON.stringify(JSON.stringify(answer))})`;
  return await runHookCommand({ command: [process.execPath, "-e", script], timeoutSeconds: 10 }, payload, { cwd: tmpdir(), env: process.env });
}

describe("the conflict hook command's answer", () => {
  it("reads a null note or status as none, as most serializers write an absent field", async () => {
    expect(await answering({ note: null, status: "Tim is on it" })).toEqual({ ok: true, error: null, note: null, status: "Tim is on it" });
    expect(await answering({ note: "Ask Jordan first.", status: null })).toEqual({ ok: true, error: null, note: "Ask Jordan first.", status: null });
    expect(await answering({ note: 1 })).toMatchObject({ ok: false });
  });

  it("cuts a long status between characters, never inside one", async () => {
    const status = `${"a".repeat(MAX_HOOK_STATUS - 2)}🔥🔥🔥`;
    const answer = await answering({ status });
    expect([...answer.status!]).toHaveLength(MAX_HOOK_STATUS);
    expect(answer.status).toBe(`${"a".repeat(MAX_HOOK_STATUS - 2)}🔥…`);
  });
});

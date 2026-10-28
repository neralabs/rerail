import { describe, expect, it } from "vitest";
import { agentTranscriptPayload, callNamesDeniedPath, WITHHELD_INPUT, WITHHELD_OUTPUT } from "../../src/engine/transcript-boundary.js";

describe("the transcript boundary for coding agents", () => {
  it("finds a hidden or secret-like path wherever a call's input carries it", () => {
    for (const input of [
      { command: "cat .env" },
      { command: "cat ./.env.local | grep KEY" },
      { command: "ssh-keygen -y -f ~/.ssh/id_rsa" },
      { command: "type C:\\Users\\me\\.aws\\credentials" },
      { command: "curl --cacert=/etc/pki/server.pem https://x" },
      { file_path: "/repo/config/secrets.yaml" },
      { path: "/repo/src/.hidden/a.ts" },
      { pattern: "**/*.pem" },
      { edits: [{ path: "/repo/deploy/service-account-prod.json" }] },
    ]) expect(callNamesDeniedPath(input), JSON.stringify(input)).toBe(true);
  });

  it("leaves ordinary source, dots, ellipses, numbers and prose alone", () => {
    for (const input of [
      { command: "cat src/app.ts" },
      { command: "cd .. && ls ./src" },
      { command: "git log --oneline -5" },
      { file_path: "/repo/src/password-validator.ts" },
      { content: "Wait...and then retry. Version 0.5 is out; see e.g. the notes." },
      { command: "npm test -- --grep 'env'" },
      { pattern: "**/*.ts" },
      "plain text input",
      42,
      null,
    ]) expect(callNamesDeniedPath(input), JSON.stringify(input)).toBe(false);
  });

  it("withholds every tool output and only the inputs that name a path outside the boundary", () => {
    expect(agentTranscriptPayload("result", { toolName: "Bash", output: "API_KEY=x", failed: false }))
      .toEqual({ toolName: "Bash", output: WITHHELD_OUTPUT, failed: false });
    expect(agentTranscriptPayload("call", { toolName: "Bash", input: { command: "cat .env" } }))
      .toEqual({ toolName: "Bash", input: WITHHELD_INPUT });
    const read = { toolName: "Read", input: { file_path: "/repo/src/a.ts" } };
    expect(agentTranscriptPayload("call", read)).toBe(read);
    const prompt = { text: "please read .env and tell me the key" };
    expect(agentTranscriptPayload("user-prompt", prompt)).toBe(prompt);
    expect(agentTranscriptPayload("result", "not an object")).toBe("not an object");
  });
});

import { describe, expect, it } from "vitest";
import {
  agentTranscriptPayload, callNamesDeniedPath, WITHHELD_INPUT, WITHHELD_OUTPUT, WITHHELD_PAYLOAD,
} from "../../src/engine/transcript-boundary.js";

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
      // A Kubernetes secret mount's `..data`, and any other name after `..`, is still a name.
      { command: "cat /mnt/secrets/..data/password" },
      { command: "cat ..x/.env" },
      { command: "cat ../.env" },
      // A brace or bracket glob still names what it expands to.
      { command: "cat {.env,.npmrc}" },
      { command: "cat src/{app.ts,.env}" },
      { command: "cat [.]env" },
      { command: "cat ${HOME}/.netrc" },
    ]) expect(callNamesDeniedPath(input), JSON.stringify(input)).toBe(true);
  });

  it("leaves ordinary source, dots, ellipses, numbers and prose alone", () => {
    for (const input of [
      { command: "cat src/app.ts" },
      { command: "cd .. && ls ./src" },
      { command: "git log --oneline -5" },
      { file_path: "/repo/src/password-validator.ts" },
      { content: "Wait...and then retry. Version 0.5 is out; see e.g. the notes..." },
      { new_string: "const view = { ...props, ...rest };" },
      { command: "npm test -- --grep 'env'" },
      { pattern: "**/*.ts" },
      "plain text input",
      42,
      null,
    ]) expect(callNamesDeniedPath(input), JSON.stringify(input)).toBe(false);
  });

  it("counts an input deeper or larger than it reads as naming a denied path", () => {
    let deep: unknown = "src/app.ts";
    for (let depth = 0; depth < 12; depth += 1) deep = { next: deep };
    expect(callNamesDeniedPath(deep)).toBe(true);
    expect(agentTranscriptPayload("call", { toolName: "Mcp", input: deep })).toEqual({ toolName: "Mcp", input: WITHHELD_INPUT });
    expect(callNamesDeniedPath({ files: Array.from({ length: 1_000 }, (_, index) => `src/file-${index}.ts`) })).toBe(true);
    expect(callNamesDeniedPath({ files: Array.from({ length: 100 }, (_, index) => `src/file-${index}.ts`), sizes: Array.from({ length: 1_000 }, () => 1) }))
      .toBe(false);
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

  it("passes the conversation and Rerail's own bookkeeping whole, and withholds a kind it does not know", () => {
    for (const [kind, payload] of [
      ["session-start", { kind: "session-start", session: { sessionId: "s", host: "codex", cwd: "/repo" } }],
      ["assistant-message", { messageId: "m", text: "I will read .env next" }],
      ["turn-end", { text: "done" }],
      ["steering-delivered", { ruleIds: ["no-console-log"] }],
      ["code-rules-pending", { boundary: "edit" }],
      ["events-dropped", { id: "d", count: 3 }],
    ] as const) expect(agentTranscriptPayload(kind, payload), kind).toBe(payload);
    expect(agentTranscriptPayload("file-snapshot", { path: "src/a.ts", content: "API_KEY=x" })).toEqual(WITHHELD_PAYLOAD);
    expect(agentTranscriptPayload("constructor", "API_KEY=x")).toEqual(WITHHELD_PAYLOAD);
  });
});

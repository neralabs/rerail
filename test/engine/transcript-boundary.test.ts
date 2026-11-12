import { type EncodingOption, mkdirSync, mkdtempSync, type PathLike, realpathSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { isAbsolute, join, relative, sep } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  agentTranscriptPayload, callNamesDeniedPath, callNamesSecretPath, WITHHELD_INPUT, WITHHELD_OUTPUT, WITHHELD_PAYLOAD,
} from "../../src/engine/transcript-boundary.js";

const roots: string[] = [];
afterEach(() => {
  vi.restoreAllMocks();
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

/** An empty checkout folder, removed after the test. */
function checkout(prefix: string): string {
  const root = realpathSync(mkdtempSync(join(tmpdir(), prefix)));
  roots.push(root);
  return root;
}

/** URLs and a network share, as agents name them; 192.0.2.1 is a documentation address no host answers on. */
const NETWORK_INPUTS = [
  { command: "curl -s https://192.0.2.1/api/v1 | jq ." },
  { url: "https://192.0.2.1/docs/a.md" },
  { command: "type \\\\192.0.2.1\\share\\notes.txt" },
];

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
    // Rerail's records of what Git's ignore rules left unchecked quote hidden ignore files: never an agent's to read.
    expect(agentTranscriptPayload("ignored-edit", { path: "scratch/a.ts", source: ".git/info/exclude", line: 7, pattern: "scratch/" }))
      .toEqual(WITHHELD_PAYLOAD);
    expect(agentTranscriptPayload("ignore-rules-changed", { path: ".git/info/exclude", via: "observed" })).toEqual(WITHHELD_PAYLOAD);
  });

  it("withholds a call that names a file linked to a secret in the session's checkout", () => {
    const root = realpathSync(mkdtempSync(join(tmpdir(), "rerail-transcript-boundary-")));
    roots.push(root);
    writeFileSync(join(root, ".env"), "API_KEY=x\n");
    mkdirSync(join(root, "docs"));
    writeFileSync(join(root, "docs", "guide.md"), "# Guide\n");
    symlinkSync("../.env", join(root, "docs", "a.md"));
    const linked = { toolName: "Bash", input: { command: "cat docs/a.md" } };
    expect(agentTranscriptPayload("call", linked)).toBe(linked);
    expect(agentTranscriptPayload("call", linked, root)).toEqual({ toolName: "Bash", input: WITHHELD_INPUT });
    expect(callNamesDeniedPath({ file_path: join(root, "docs", "a.md") }, root)).toBe(true);
    expect(callNamesSecretPath({ command: "cat ./docs/a.md" }, root)).toBe(true);
    // A link at the checkout's root is resolved when it is named like a file, and a bare word is judged by its name.
    symlinkSync(".env", join(root, "notes.txt"));
    expect(callNamesSecretPath({ command: "cat notes.txt" }, root)).toBe(true);
    expect(callNamesSecretPath({ command: "cat token" }, root)).toBe(true);
    const plain = { toolName: "Read", input: { file_path: join(root, "docs", "guide.md") } };
    expect(agentTranscriptPayload("call", plain, root)).toBe(plain);
    expect(callNamesDeniedPath({ command: "cat docs/missing.md" }, root)).toBe(false);
  });

  it("passes a call that names a link into a hidden folder that is not secret-like, as pnpm's node_modules are", () => {
    const root = realpathSync(mkdtempSync(join(tmpdir(), "rerail-transcript-pnpm-")));
    roots.push(root);
    const store = join(root, "node_modules", ".pnpm", "left-pad@1.3.0", "node_modules", "left-pad");
    mkdirSync(store, { recursive: true });
    writeFileSync(join(store, "index.js"), "module.exports = pad;\n");
    symlinkSync(join(".pnpm", "left-pad@1.3.0", "node_modules", "left-pad"), join(root, "node_modules", "left-pad"));
    const read = { toolName: "Bash", input: { command: "cat node_modules/left-pad/index.js" } };
    expect(agentTranscriptPayload("call", read, root)).toBe(read);
    // The hidden path itself, named, is still outside.
    expect(callNamesDeniedPath({ command: "cat node_modules/.pnpm/left-pad@1.3.0/node_modules/left-pad/index.js" }, root)).toBe(true);
  });

  it("looks up on disk only paths in the session's checkout, never a URL's host or a network share", () => {
    const root = checkout("rerail-transcript-lookups-");
    const outside = (path: string): boolean => {
      const named = relative(root, path);
      return named === ".." || named.startsWith(`..${sep}`) || isAbsolute(named);
    };
    const lookUp = realpathSync.native;
    const lookups: string[] = [];
    vi.spyOn(realpathSync, "native").mockImplementation((path: PathLike, options?: EncodingOption) => {
      lookups.push(String(path));
      // Refused, not made: on Windows a `\\host\share` path waits about 21 s for the host to answer.
      if (outside(String(path))) throw Object.assign(new Error(`ENOENT: no such file or directory, realpath '${String(path)}'`), { code: "ENOENT" });
      return lookUp(path, options);
    });
    for (const input of [...NETWORK_INPUTS, { command: `cat ${join(tmpdir(), "notes.txt")}` }]) {
      expect(callNamesSecretPath(input, root), JSON.stringify(input)).toBe(false);
      // A coding agent reading the session's events has each stored call judged in the same checkout.
      const call = { toolName: "Bash", input };
      expect(agentTranscriptPayload("call", call, root), JSON.stringify(input)).toBe(call);
    }
    // A path in the checkout is still looked up, to see where it leads.
    expect(callNamesSecretPath({ command: "cat docs/guide.md" }, root)).toBe(false);
    expect(lookups.filter(outside)).toEqual([]);
    expect(lookups).toContain(join(root, "docs", "guide.md"));
  });

  it.runIf(process.platform === "win32")("answers at once for a URL on Windows, where its host part names a network share", () => {
    const root = checkout("rerail-transcript-share-");
    for (const input of NETWORK_INPUTS) {
      const started = performance.now();
      expect(callNamesSecretPath(input, root), JSON.stringify(input)).toBe(false);
      expect(performance.now() - started, JSON.stringify(input)).toBeLessThan(50);
    }
  });

  it("judges only secret-like names as secret, not every hidden path", () => {
    expect(callNamesSecretPath({ command: "cat .env" })).toBe(true);
    expect(callNamesSecretPath({ file_path: "/repo/deploy/gcp-key.json" })).toBe(true);
    expect(callNamesSecretPath({ path: "/repo/.github/workflows/ci.yml" })).toBe(false);
    expect(callNamesDeniedPath({ path: "/repo/.github/workflows/ci.yml" })).toBe(true);
    expect(callNamesSecretPath({ command: "cat src/app.ts" })).toBe(false);
  });
});

import { type EncodingOption, mkdirSync, mkdtempSync, type PathLike, realpathSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join, relative, resolve, sep } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  agentTranscriptPayload, callNamesDeniedPath, callNamesSecretPath, WITHHELD_INPUT, WITHHELD_OUTPUT, WITHHELD_PAYLOAD,
} from "../../src/engine/transcript-boundary.js";

const roots: string[] = [];
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllEnvs();
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

/** An empty checkout folder, removed after the test. */
function checkout(prefix: string): string {
  const root = realpathSync(mkdtempSync(join(tmpdir(), prefix)));
  roots.push(root);
  return root;
}

/** Calls that name URLs as agents do; 192.0.2.1 is a documentation address (RFC 5737) no host answers on. */
const URL_INPUTS = [
  { command: "curl -s https://192.0.2.1/api/v1 | jq ." },
  { url: "https://192.0.2.1/docs/a.md" },
  { command: "git clone ssh://git@192.0.2.1:2222/team/app.git" },
  { command: "curl http://user:pw@192.0.2.1:8080/notes.txt?page=2" },
];

/**
 * A checkout `repo`, with a session folder `repo/packages/app` in it, and a secret folder `home/.aws` beside it. Links
 * to that folder sit in the checkout (`repo/docs/cfg`), in the home folder (`home/cfg`), in the temp folder
 * (`temp/cfg`) and in another folder (`out/cfg`); for the rest of the test `homedir()` and `tmpdir()` name `home` and
 * `temp`. A link to a folder is a junction on Windows, which needs no privilege to make.
 */
function linksToSecretFolder(): { base: string; repo: string; app: string } {
  const base = checkout("rerail-transcript-links-");
  const repo = join(base, "repo");
  const app = join(repo, "packages", "app");
  const secrets = join(base, "home", ".aws");
  for (const folder of [app, join(repo, "docs"), join(base, "temp"), join(base, "out"), secrets]) mkdirSync(folder, { recursive: true });
  writeFileSync(join(secrets, "config"), "[default]\n");
  for (const folder of [join(repo, "docs"), join(base, "home"), join(base, "temp"), join(base, "out")]) {
    symlinkSync(secrets, join(folder, "cfg"), "junction");
  }
  for (const name of ["HOME", "USERPROFILE"]) vi.stubEnv(name, join(base, "home"));
  for (const name of ["TMPDIR", "TEMP", "TMP"]) vi.stubEnv(name, join(base, "temp"));
  return { base, repo, app };
}

/** `path` as a `file:` URL, spelled as an agent writes one: `file:///home/me/a.txt`, `file:///C:/Users/me/a.txt`. */
const fileUrl = (path: string): string => `file://${path.startsWith("/") ? "" : "/"}${path.split(sep).join("/")}`;

/**
 * Records each path `realpathSync.native` is asked for. A `\\` path is refused, not looked up: on Windows it names a
 * network share, and the lookup waits about 21 s for a host that does not answer.
 */
function recordLookups(): string[] {
  const lookUp = realpathSync.native;
  const lookups: string[] = [];
  vi.spyOn(realpathSync, "native").mockImplementation((path: PathLike, options?: EncodingOption) => {
    lookups.push(String(path));
    if (String(path).startsWith("\\\\")) throw Object.assign(new Error(`ENOENT: no such file or directory, realpath '${String(path)}'`), { code: "ENOENT" });
    return lookUp(path, options);
  });
  return lookups;
}

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

  it("withholds a call that names a link to a secret in the temp folder, the home folder or another folder outside the checkout", () => {
    const { base, repo } = linksToSecretFolder();
    for (const link of [join(tmpdir(), "cfg"), join(homedir(), "cfg"), join(base, "out", "cfg")]) {
      const named = join(link, "config");
      for (const input of [
        { command: `cat ${named}` },
        { file_path: named },
        { command: `cat ${relative(repo, named)}` },
        { command: `curl ${fileUrl(named)}` },
      ]) {
        expect(callNamesSecretPath(input, repo), JSON.stringify(input)).toBe(true);
        expect(agentTranscriptPayload("call", { toolName: "Bash", input }, repo), JSON.stringify(input))
          .toEqual({ toolName: "Bash", input: WITHHELD_INPUT });
      }
    }
  });

  it("judges a link elsewhere in the checkout when the session runs in a subfolder of it", () => {
    const { repo, app } = linksToSecretFolder();
    for (const input of [{ command: "cat ../../docs/cfg/config" }, { command: `cat ${join(repo, "docs", "cfg", "config")}` }]) {
      expect(callNamesSecretPath(input, app), JSON.stringify(input)).toBe(true);
    }
  });

  it("starts no lookup for a URL's host and path, judges it by name, and looks up every other path-shaped word", () => {
    const base = checkout("rerail-transcript-lookups-");
    const root = join(base, "repo");
    mkdirSync(root);
    const lookups = recordLookups();
    for (const input of URL_INPUTS) {
      expect(callNamesSecretPath(input, root), JSON.stringify(input)).toBe(false);
      // A coding agent reading the session's events has each stored call judged in the same checkout.
      const call = { toolName: "Bash", input };
      expect(agentTranscriptPayload("call", call, root), JSON.stringify(input)).toBe(call);
    }
    expect(callNamesDeniedPath({ url: "https://192.0.2.1/app/.env" }, root)).toBe(true);
    expect(callNamesSecretPath({ url: "https://192.0.2.1/config/secrets.json" }, root)).toBe(true);
    expect(lookups.filter(path => path.includes("192.0.2.1"))).toEqual([]);
    // Every other path-shaped word is looked up as before: in the checkout, out of it, a `\\server\share` path, and a
    // path a `:` joins to a URL that has a path already, as in a list.
    const share = "\\\\198.51.100.1\\share\\notes.txt";
    const listed = join(base, "bin", "tool.sh");
    for (const command of ["cat docs/guide.md", `cat ${join(base, "notes.txt")}`, `type ${share}`, `PATH=https://192.0.2.1/bin:${listed}`]) {
      expect(callNamesSecretPath({ command }, root), command).toBe(false);
    }
    expect(lookups).toEqual(expect.arrayContaining([join(root, "docs", "guide.md"), join(base, "notes.txt"), resolve(root, share), listed]));
  });

  it("spends none of a call's lookups on a URL, so a link named after many URLs is still followed", () => {
    const { repo } = linksToSecretFolder();
    recordLookups();
    const urls = Array.from({ length: 80 }, (_, index) => `https://192.0.2.1/v${index}`);
    expect(callNamesSecretPath({ content: [...urls, "cat docs/cfg/config"].join("\n") }, repo)).toBe(true);
  });

  it.runIf(process.platform === "win32")("answers at once for a URL on Windows, where its host and path would name a network share", () => {
    const root = checkout("rerail-transcript-share-");
    for (const input of URL_INPUTS) {
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

import { execFileSync } from "node:child_process";
import { mkdirSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { declarations } from "../../../src/engine/merge-assist/alignment/symbols.js";
import { baseDocs, compatibleChange, contractBreaks, declaresBreak, reservedClassesOf, sideChanges, sideDiff } from "../../../src/engine/merge-assist/conflict-evidence.js";

const roots: string[] = [];
afterEach(async () => {
  await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })));
});

function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "-c", "commit.gpgsign=false", ...args], { cwd, encoding: "utf8" }).trim();
}

function commit(root: string, files: Record<string, string | null>, message: string): string {
  for (const [path, content] of Object.entries(files)) {
    if (content === null) rmSync(join(root, path));
    else {
      mkdirSync(dirname(join(root, path)), { recursive: true });
      writeFileSync(join(root, path), content);
    }
  }
  git(root, "add", "-A");
  git(root, "commit", "--quiet", "-m", message);
  return git(root, "rev-parse", "HEAD");
}

const API = "export function fetchUser(id: string): User {\n  return load(id);\n}\n\nexport interface User {\n  id: string;\n  name: string;\n}\n";

async function repository(): Promise<{ root: string; base: string }> {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-conflict-evidence-")));
  roots.push(root);
  git(root, "init", "--quiet", "-b", "main");
  const base = commit(root, {
    "src/api.ts": API,
    "src/app.ts": "import { fetchUser } from './api';\nfetchUser('1');\n",
    "test/api.test.ts": "import { fetchUser } from '../src/api';\n",
    "README.md": "# App\nThe api module loads users.\n",
    "docs/design/users.md": "# Users\nfetchUser is the one way to load a user; it never retries.\n",
    "docs/other.md": "# Other\nNothing here.\n",
    "package-lock.json": "{}\n",
  }, "seed");
  return { root, base };
}

function declaration(language: "typescript" | "python" | "go", text: string) {
  return declarations(language, text)[0]!;
}

describe("sideChanges", () => {
  it("counts source lines from where the side left the base, without tests or lockfiles", async () => {
    const { root, base } = await repository();
    git(root, "checkout", "--quiet", "-b", "side");
    const head = commit(root, { "src/app.ts": "import { fetchUser } from './api';\nfetchUser('1');\nfetchUser('2');\n", "test/api.test.ts": "// more\n", "package-lock.json": "{\"a\":1}\n" }, "side");
    git(root, "checkout", "--quiet", "main");
    const later = commit(root, { "src/other.ts": "export const x = 1;\n" }, "main moves on");
    const changes = sideChanges(root, later, head)!;
    expect(changes.from).toBe(base);
    expect(changes.files.map(file => file.path).sort()).toEqual(["package-lock.json", "src/app.ts", "test/api.test.ts"]);
    expect(changes.sourceLines).toBe(1);
    expect(changes.sourceFiles).toBe(1);
    expect(sideChanges(root, base, "f".repeat(40))).toBeNull();
  });
});

describe("contractBreaks", () => {
  it("finds an exported signature change with its dependents, and declares it from the description or changelog", async () => {
    const { root, base } = await repository();
    const head = commit(root, { "src/api.ts": API.replace("fetchUser(id: string)", "fetchUser(id: number)") }, "numeric ids");
    const changes = sideChanges(root, base, head)!;
    const [change] = await contractBreaks(root, head, changes, { intent: "Use numeric ids.", declared: [] });
    expect(change).toMatchObject({ path: "src/api.ts", name: "fetchUser", change: "changed", public: true, declared: false });
    expect([...change!.dependents].sort()).toEqual(["docs/design/users.md", "src/app.ts", "test/api.test.ts"]);
    expect((await contractBreaks(root, head, changes, { intent: "BREAKING CHANGE: ids are numbers.", declared: [] }))[0]!.declared).toBe(true);
    expect((await contractBreaks(root, head, changes, { intent: "", declared: ["fetchUser"] }))[0]!.declared).toBe(true);
    const logged = commit(root, { "CHANGELOG.md": "- fetchUser takes a number now\n" }, "log it");
    expect((await contractBreaks(root, logged, sideChanges(root, base, logged)!, { intent: "", declared: [] }))[0]!.declared).toBe(true);
  });

  it("does not count an optional parameter, a grown type, or a move between files", async () => {
    const { root, base } = await repository();
    const grown = commit(root, { "src/api.ts": API.replace("fetchUser(id: string)", "fetchUser(id: string, retries?: number)").replace("  name: string;\n", "  name: string;\n  email: string;\n") }, "grow");
    expect(await contractBreaks(root, grown, sideChanges(root, base, grown)!, { intent: "", declared: [] })).toEqual([]);
    const moved = commit(root, { "src/api.ts": "export interface User {\n  id: string;\n  name: string;\n}\n", "src/users.ts": "export function fetchUser(id: string): User {\n  return load(id);\n}\n" }, "move");
    expect(await contractBreaks(root, moved, sideChanges(root, base, moved)!, { intent: "", declared: [] })).toEqual([]);
  });

  it("reports a removal, and an internal path as not public", async () => {
    const { root } = await repository();
    commit(root, { "src/internal/cache.ts": "export function evictAll(): void {}\n", "src/app.ts": "import { evictAll } from './internal/cache';\nevictAll();\n" }, "cache");
    const start = git(root, "rev-parse", "HEAD");
    const head = commit(root, { "src/internal/cache.ts": "export const unused = 1;\n", "src/api.ts": "export interface User {\n  id: string;\n  name: string;\n}\n" }, "remove");
    const breaks = await contractBreaks(root, head, sideChanges(root, start, head)!, { intent: "", declared: [] });
    expect(breaks.map(change => [change.name, change.change, change.public])).toEqual([["fetchUser", "removed", true], ["evictAll", "removed", false]]);
  });
});

describe("compatibleChange", () => {
  it("follows each language's optional parameters and keeps Go receivers out of the parameters", () => {
    expect(compatibleChange("python", declaration("python", "def run(a, b):\n  pass\n"), declaration("python", "def run(a, b, c=None):\n  pass\n"))).toBe(true);
    expect(compatibleChange("python", declaration("python", "def run(a, b):\n  pass\n"), declaration("python", "def run(a, c):\n  pass\n"))).toBe(false);
    expect(compatibleChange("typescript", declaration("typescript", "export function f(cb: () => void) {}\n"), declaration("typescript", "export function f(cb: () => void, more: () => void) {}\n"))).toBe(false);
    expect(compatibleChange("go", declaration("go", "func (s *Server) Serve(addr string) error {\n}\n"), declaration("go", "func (s *Server) Serve(addr string, opts Options) error {\n}\n"))).toBe(false);
    expect(compatibleChange("go", declaration("go", "type Config struct {\n  Addr string\n}\n"), declaration("go", "type Config struct {\n  Addr string\n  Timeout int\n}\n"))).toBe(true);
    expect(compatibleChange("typescript", declaration("typescript", "export const LIMIT = 3;\n"), declaration("typescript", "export const LIMIT = 4;\n"))).toBe(true);
  });
});

describe("reservedClassesOf, sideDiff and baseDocs", () => {
  it("classes security and data-model paths and public breaks", async () => {
    const { root, base } = await repository();
    const head = commit(root, { "src/auth/session.ts": "export const ttl = 1;\n", "migrations/002_users.sql": "ALTER TABLE users ADD x int;\n", "test/auth_test.ts": "x\n" }, "auth");
    const changes = sideChanges(root, base, head)!;
    expect(reservedClassesOf(changes, [], false)).toEqual(["security", "data-model"]);
    expect(reservedClassesOf(changes, [], true)).toEqual(["public-api", "security", "data-model"]);
    expect(reservedClassesOf(changes, [{ path: "src/api.ts", name: "f", change: "removed", before: "", after: null, public: true, dependents: [], declared: true }], false)).toEqual(["public-api", "security", "data-model"]);
  });

  it("knows a side declares a break from its description or a breaking changelog fragment", async () => {
    const { root, base } = await repository();
    const head = commit(root, { "src/app.ts": "x\n" }, "change");
    expect(await declaresBreak(root, head, sideChanges(root, base, head)!, "Potentially breaking change: continuations need a space.")).toBe(true);
    expect(await declaresBreak(root, head, sideChanges(root, base, head)!, "Fix the formatter.")).toBe(false);
    const fragment = commit(root, { "changelog/123.breaking.rst": "Continuations change.\n" }, "fragment");
    expect(await declaresBreak(root, fragment, sideChanges(root, base, fragment)!, "Fix the formatter.")).toBe(true);
  });

  it("shows source before tests, leaves lockfiles out and names what it cut", async () => {
    const { root, base } = await repository();
    const head = commit(root, { "test/api.test.ts": "// t\n", "src/app.ts": "x\n", "package-lock.json": "{\"b\":2}\n", "src/big.ts": "y\n".repeat(400) }, "diff");
    const diff = sideDiff(root, head, sideChanges(root, base, head)!, 600);
    expect(diff.indexOf("b/src/app.ts")).toBeLessThan(diff.indexOf("b/test/api.test.ts"));
    expect(diff).not.toContain("package-lock.json");
    expect(diff).toContain("[left out for length: src/big.ts]");
  });

  it("reads the top-level docs and the design docs that name the conflict's files or symbols", async () => {
    const { root, base } = await repository();
    const docs = await baseDocs(root, base, { files: ["src/api.ts"], symbols: ["fetchUser"] }, 10_000);
    expect(docs.map(doc => doc.path)).toEqual(["README.md", "docs/design/users.md"]);
    expect(docs[1]!.text).toContain("it never retries");
  });
});

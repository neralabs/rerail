import { execFileSync } from "node:child_process";
import { appendFile, mkdir, mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir, userInfo } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/**
 * The arguments of each Git process Rerail started; the test's own `git` uses `execFileSync`, which is not counted. A
 * `git var` of a variable in `failing` fails once, as one in a folder just removed does.
 */
const started = vi.hoisted(() => ({ git: [] as string[][], failing: new Set<string>() }));
vi.mock("node:child_process", async importOriginal => {
  const original = await importOriginal<typeof import("node:child_process")>();
  const spawn = ((file: string, args: readonly string[], ...rest: never[]) => {
    if (!/(^|[\\/])git(\.exe)?$/iu.test(file)) return original.spawn(file, args, ...rest);
    started.git.push([...args]);
    const variable = args.at(-1) ?? "";
    if (args.at(-2) !== "var" || !started.failing.delete(variable)) return original.spawn(file, args, ...rest);
    return original.spawn(file, [...args.slice(0, -2), "-C", join(tmpdir(), "rerail-no-such-folder"), "var", variable], ...rest);
  }) as typeof original.spawn;
  return { ...original, spawn };
});

import { gitCrlfPaths, lineEndingAttributesFrom } from "../../src/engine/git.js";
import { gitCrlfReader, type GitCrlfReader } from "../../src/engine/git-crlf-reader.js";
import { removeFolder } from "../support/platform/remove-folder.js";

/** An input must be this old before a reader keeps an answer by its stamp (the reader's 2 s, and a margin). */
const SETTLED_MS = 2_100;
/**
 * How far the clock the reader measures an input's age on runs ahead of the real one: a test moves it on in place of
 * waiting, so every input written before then reads as that much older, as after a wait.
 */
const clock = { aheadMs: 0 };
const realNow = Date.now.bind(Date);
beforeEach(() => {
  vi.spyOn(Date, "now").mockImplementation(() => realNow() + clock.aheadMs);
});
const PATHS = ["a.txt", "src/b.py", "src/deep/c.bin", "d.md"];

const folders: string[] = [];
const environment = new Map<string, string | undefined>();
afterEach(async () => {
  vi.mocked(Date.now).mockRestore();
  clock.aheadMs = 0;
  started.failing.clear();
  for (const [name, value] of environment) {
    if (value === undefined) delete process.env[name];
    else process.env[name] = value;
  }
  environment.clear();
  await Promise.all(folders.splice(0).map(async folder => await removeFolder(folder)));
});

/** Sets an environment variable for this test only. */
function setVariable(name: string, value: string): void {
  if (!environment.has(name)) environment.set(name, process.env[name]);
  process.env[name] = value;
}

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}

/** A repository holding `PATHS`, and `.gitattributes` with `attributes` when given, committed. */
async function repository(attributes?: string): Promise<string> {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-crlf-reader-")));
  folders.push(root);
  await mkdir(join(root, "src", "deep"), { recursive: true });
  for (const path of PATHS) await writeFile(join(root, path), "line\n");
  if (attributes !== undefined) await writeFile(join(root, ".gitattributes"), attributes);
  git(root, "init", "--quiet");
  git(root, "config", "core.autocrlf", "false");
  git(root, "add", ".");
  git(root, "commit", "--quiet", "-m", "seed");
  return root;
}

/** The reader's rules for `PATHS`, `gitCrlfPaths`' right after, and the Git processes the reader started, by command. */
async function read(reader: GitCrlfReader, root: string): Promise<{ rules: unknown; expected: unknown; git: string[] }> {
  const before = started.git.length;
  const rules = Object.fromEntries(await reader(root, PATHS));
  const git = started.git.slice(before).map(args => args.find(arg => !arg.startsWith("-") && !arg.includes("=") && arg !== root) ?? "");
  return { rules, expected: Object.fromEntries(await gitCrlfPaths(root, PATHS)), git };
}

describe("gitCrlfReader", () => {
  it("answers as gitCrlfPaths does, asking Git again only once a file it reads attributes or settings from changed", async () => {
    const root = await repository("*.txt text eol=crlf\n*.bin binary\n");
    const reader = gitCrlfReader();
    clock.aheadMs += SETTLED_MS;
    const changes: Array<[string, () => Promise<void> | void]> = [
      ["nothing", () => undefined],
      ["the top .gitattributes", async () => await writeFile(join(root, ".gitattributes"), "*.txt text eol=crlf\n*.py text eol=crlf\n")],
      ["a .gitattributes in a folder", async () => await writeFile(join(root, "src", ".gitattributes"), "*.py -text\n")],
      ["info/attributes", async () => await writeFile(join(root, ".git", "info", "attributes"), "*.md text eol=crlf\n")],
      ["core.autocrlf", () => void git(root, "config", "core.autocrlf", "true")],
      ["core.eol", () => void git(root, "config", "core.eol", "crlf")],
      ["the top .gitattributes removed while the index tracks it", async () => await rm(join(root, ".gitattributes"))],
      ["the removal staged", () => void git(root, "rm", "--quiet", "--cached", ".gitattributes")],
    ];
    for (const [name, change] of changes) {
      await change();
      clock.aheadMs += SETTLED_MS;
      const first = await read(reader, root);
      expect(first.rules, `after ${name}`).toEqual(first.expected);
      const again = await read(reader, root);
      expect(again.rules, `after ${name}, again`).toEqual(again.expected);
      expect(again.git, `after ${name}, again`).toEqual([]);
      if (name !== "nothing") expect(first.git.length, `after ${name}`).toBeGreaterThan(0);
    }
  }, 60_000);

  it("asks Git for no attribute where no file can set one", async () => {
    const root = await repository();
    // No system file, and a global one's default place that holds none.
    setVariable("GIT_ATTR_NOSYSTEM", "1");
    setVariable("XDG_CONFIG_HOME", join(root, ".git", "no-config-home"));
    const reader = gitCrlfReader();
    clock.aheadMs += SETTLED_MS;
    const first = await read(reader, root);
    expect(first.rules).toEqual(first.expected);
    expect(first.git).not.toContain("check-attr");
    git(root, "config", "core.autocrlf", "true");
    clock.aheadMs += SETTLED_MS;
    const autocrlf = await read(reader, root);
    expect(autocrlf.rules).toEqual(autocrlf.expected);
    expect(Object.keys(autocrlf.rules as object)).toHaveLength(PATHS.length);
    expect(autocrlf.git).not.toContain("check-attr");
  }, 30_000);

  it("asks Git on every call while an input is younger than the settle time, or GIT_ATTR_SOURCE moves where Git looks", async () => {
    const root = await repository("*.txt text eol=crlf\n");
    const reader = gitCrlfReader();
    await writeFile(join(root, ".gitattributes"), "*.txt text eol=crlf\n# fresh\n");
    for (const time of [1, 2]) {
      const answer = await read(reader, root);
      expect(answer.rules).toEqual(answer.expected);
      expect(answer.git, `fresh, call ${time}`).toContain("check-attr");
    }
    clock.aheadMs += SETTLED_MS;
    setVariable("GIT_ATTR_SOURCE", "HEAD");
    for (const time of [1, 2]) {
      const answer = await read(reader, root);
      expect(answer.rules).toEqual(answer.expected);
      expect(answer.git, `GIT_ATTR_SOURCE, call ${time}`).toContain("check-attr");
    }
  }, 30_000);

  it("asks git var for the system attributes file again at the next call after it failed, then answers from Git's files", async () => {
    const root = await repository("*.txt text eol=crlf\n");
    const reader = gitCrlfReader();
    clock.aheadMs += SETTLED_MS;
    started.failing.add("GIT_ATTR_SYSTEM");
    const failed = await read(reader, root);
    expect(failed.rules).toEqual(failed.expected);
    const again = await read(reader, root);
    expect(again.rules).toEqual(again.expected);
    const after = await read(reader, root);
    expect(after.rules).toEqual(after.expected);
    expect(after.git).toEqual([]);
  }, 30_000);

  it("asks nothing of the system attributes file for a checkout whose configuration cannot be named", async () => {
    const root = await repository("*.txt text eol=crlf\n");
    const reader = gitCrlfReader();
    clock.aheadMs += SETTLED_MS;
    const before = started.git.length;
    expect(Object.fromEntries(await reader(join(root, "removed"), PATHS))).toEqual({});
    expect(started.git.slice(before).filter(args => args.includes("GIT_ATTR_SYSTEM"))).toEqual([]);
    const first = await read(reader, root);
    expect(first.rules).toEqual(first.expected);
    expect((await read(reader, root)).git).toEqual([]);
  }, 30_000);

  // Git reads `HEAD` for an include only where an `includeIf "onbranch:"` is set; here the reader cannot read it at all,
  // as `HEAD` is past the size it reads.
  it("answers from Git's files for a checkout with an include on no branch whose HEAD the reader cannot read", async () => {
    const root = await repository("*.txt text eol=crlf\n");
    await writeFile(join(root, ".git", "extra.conf"), "[core]\n\teol = lf\n");
    git(root, "config", "include.path", "extra.conf");
    await appendFile(join(root, ".git", "HEAD"), "\n".repeat(70_000));
    const reader = gitCrlfReader();
    clock.aheadMs += SETTLED_MS;
    const first = await read(reader, root);
    expect(first.rules).toEqual(first.expected);
    const again = await read(reader, root);
    expect(again.rules).toEqual(again.expected);
    expect(again.git).toEqual([]);
  }, 30_000);

  it("asks Git on every call while core.attributesFile is spelled so that only Git can name its file", async () => {
    setVariable("GIT_ATTR_NOSYSTEM", "1");
    const spellings = ["~rerail-no-such-user/attributes", `~${userInfo().username}/.rerail-no-such-attributes`, "%(prefix)/rerail-no-such-attributes", ":(optional)no-such-attributes"];
    for (const spelling of spellings) {
      const root = await repository();
      git(root, "config", "core.autocrlf", "true");
      git(root, "config", "core.attributesFile", spelling);
      const reader = gitCrlfReader();
      clock.aheadMs += SETTLED_MS;
      for (const time of [1, 2]) {
        const answer = await read(reader, root);
        expect(answer.rules, `${spelling}, call ${time}`).toEqual(answer.expected);
        expect(answer.git, `${spelling}, call ${time}`).toContain("check-attr");
      }
    }
  }, 60_000);

  it("answers no path whose check-attr output was cut before its three attributes and the NUL closing the last, so none is kept", () => {
    const whole = "a.txt\0text\0set\0a.txt\0crlf\0unspecified\0a.txt\0eol\0crlf\0";
    // Output cut at the timeout: `b.txt` got its `text` and no `crlf` or `eol`, which a `.gitattributes` may set; or its
    // `eol` value was cut before its closing NUL, before any of it or inside it (`cr` of `crlf`).
    for (const cut of ["b.txt\0text\0unset\0b.txt\0crlf\0", "b.txt\0text\0set\0b.txt\0crlf\0unspecified\0b.txt\0eol\0", "b.txt\0text\0set\0b.txt\0crlf\0unspecified\0b.txt\0eol\0cr"]) {
      expect([...lineEndingAttributesFrom(`${whole}${cut}`)], JSON.stringify(cut)).toEqual([["a.txt", { text: "set", crlf: "unspecified", eol: "crlf" }]]);
    }
  });
});

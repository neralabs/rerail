// scripts/s3/pack-fixtures.mjs: which paths are pack test data, the aws filters and messages built from them, and pull
// and push against a temporary git checkout with a fake `aws` whose bucket is a local directory.

import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  BUCKET,
  KEY_PREFIX,
  literalFilter,
  isPackFixture,
  packFixturesWritten,
  PACK_FIXTURE_PATTERNS,
  pull,
  push,
  requireAws,
  syncFilters,
} from "../../scripts/s3/pack-fixtures.mjs";

describe("isPackFixture", () => {
  it("matches a tool's cases and recorded answers, and nothing else under upstream/<tool>/", () => {
    const data = [
      "packs/php/upstream/phpcs/fixtures/Generic/x.inc",
      "packs/react/upstream/eslint-plugin-react/tests/lib/rules/a.js",
      "packs/node/upstream/eslint-plugin-n/fixtures.expected.json",
      "packs/blade/upstream/blade-parser/corpus-bagisto.expected.json",
      "packs/vue/upstream/eslint-plugin-vue/cases.json",
      "packs/sql/upstream/sqlfluff/fixtures.cases.json",
      "packs/rust/upstream/clippy/fixtures.upstream-check.json",
      "packs/complexity/upstream/lizard/lizard-1.24.0.tsv",
      "packs/next/upstream/eslint-plugin-next/fixtures/pages/[id].tsx",
    ];
    const kept = [
      "packs/python/upstream/ruff/gates.yaml",
      "packs/python/upstream/ruff/generate.py",
      "packs/python/upstream/ruff/UPSTREAM.md",
      "packs/python/upstream/ruff/goldens.tsv",
      "packs/python/upstream/ruff/recording.lock",
      "packs/python/upstream/ruff/recording.json",
      "packs/php/upstream/phpcs/ruleset.xml",
      "packs/blade/upstream/bladestan/composer-laravel.lock",
      "packs/python/upstream/common.py",
      "packs/python/upstream/fixtures.expected.json",
      "packs/python/rules/a.json",
      "fixtures/packs/valid/upstream/tool/fixtures.expected.json",
    ];
    expect(data.filter((path) => !isPackFixture(path))).toEqual([]);
    expect(kept.filter((path) => isPackFixture(path))).toEqual([]);
  });

  it("builds an aws include per pattern, below packs/, with aws's path-crossing `*`", () => {
    expect(syncFilters()).toEqual([
      "--exclude", "*",
      ...PACK_FIXTURE_PATTERNS.flatMap((pattern) => ["--include", pattern.slice("packs/".length).replaceAll("**", "*")]),
    ]);
    expect(syncFilters()).toContain("*/upstream/*/fixtures/*");
  });

  it("brackets fnmatch's special characters in a literal filter", () => {
    expect(literalFilter("fixtures/pages/[id]/a?b*.tsx")).toBe("fixtures/pages/[[]id]/a[?]b[*].tsx");
  });

  it("tells a contributor how to add what a recorder wrote, quoting paths the shell would split", () => {
    expect(packFixturesWritten(["packs/a/upstream/t/fixtures", "packs/a/upstream/t/fixtures/p [x].js"]))
      .toContain("git add -f -- packs/a/upstream/t/fixtures 'packs/a/upstream/t/fixtures/p [x].js'");
  });
});

describe("requireAws", () => {
  it("says to install the CLI when aws is missing, and to sign in when it is not authenticated", () => {
    const missing = Object.assign(new Error("spawn aws ENOENT"), { code: "ENOENT" });
    expect(() => requireAws(() => ({ status: null, error: missing }))).toThrow(/AWS CLI \(aws\) is not installed/u);
    expect(() => requireAws(() => ({ status: 255, stderr: "\nUnable to locate credentials\n" }))).toThrow(
      /aws is not signed in \(aws sts get-caller-identity: Unable to locate credentials\)/u,
    );
    expect(() => requireAws(() => ({ status: 0, stdout: "{}" }))).not.toThrow();
  });
});

const scratch = [];
afterEach(() => {
  for (const directory of scratch.splice(0)) rmSync(directory, { recursive: true, force: true });
});

const files = (root) =>
  readdirSync(root, { recursive: true, withFileTypes: true })
    .filter((entry) => entry.isFile())
    .map((entry) => relative(root, join(entry.parentPath, entry.name)).split("\\").join("/"))
    .sort();

const git = (root, ...args) => execFileSync("git", args, { cwd: root, encoding: "utf8" });

/** A git checkout and a bucket directory (`<bucket>/<key>`), with the fake `aws` that reads and writes it. */
function setUp() {
  const base = mkdtempSync(join(tmpdir(), "pack-fixtures-test-"));
  scratch.push(base);
  const root = join(base, "checkout");
  const bucket = join(base, "bucket");
  mkdirSync(root);
  mkdirSync(bucket);
  git(root, "init", "-q");
  git(root, "config", "user.email", "test@example.com");
  git(root, "config", "user.name", "test");
  const write = (directory, path, text) => {
    mkdirSync(dirname(join(directory, path)), { recursive: true });
    writeFileSync(join(directory, path), text);
  };
  const calls = [];
  const s3 = (url) => join(bucket, url.slice(`s3://${BUCKET}/`.length));
  const aws = (args) => {
    calls.push(args);
    if (args[0] === "sts") return { status: 0, stdout: "{}" };
    const [, command, source, target] = args;
    const includes = args.flatMap((arg, index) => (args[index - 1] === "--include" ? [arg.replace(/\[(.)\]/gu, "$1")] : []));
    if (args.includes("--dryrun")) return { status: 0 };
    if (command === "sync") {
      cpSync(s3(source), target, { recursive: true });
    } else if (source.startsWith("s3://")) {
      for (const path of files(s3(source)).filter((path) => includes.includes(path))) write(target, path, readFileSync(join(s3(source), path)));
    } else {
      cpSync(source, s3(target), { recursive: true });
    }
    return { status: 0 };
  };
  return { root, bucket, aws, calls, write };
}

describe("pull", () => {
  it("fetches every file, then gives tracked files their committed content and keeps working-tree edits", () => {
    const { root, aws, calls, write, bucket } = setUp();
    const key = (path) => join(KEY_PREFIX, path);
    write(root, "packs/p/upstream/t/gates.yaml", "gates\n");
    write(root, "packs/p/upstream/t/fixtures/committed.js", "committed\n");
    write(root, "packs/p/upstream/t/fixtures/edited.js", "committed\n");
    write(root, "packs/p/upstream/t/fixtures/deleted.js", "committed\n");
    git(root, "add", ".");
    git(root, "commit", "-qm", "pack");
    write(root, "packs/p/upstream/t/fixtures/edited.js", "edited\n");
    rmSync(join(root, "packs/p/upstream/t/fixtures/deleted.js"));
    for (const path of ["committed.js", "edited.js", "deleted.js", "only-in-s3.js"]) write(bucket, key(`packs/p/upstream/t/fixtures/${path}`), "from s3\n");
    const lines = [];

    expect(pull({ root, aws, log: (line) => lines.push(line) })).toEqual({ restored: 1, kept: 2 });

    const read = (path) => readFileSync(join(root, "packs/p/upstream/t/fixtures", path), "utf8");
    expect(read("committed.js")).toBe("committed\n");
    expect(read("edited.js")).toBe("edited\n");
    expect(existsSync(join(root, "packs/p/upstream/t/fixtures/deleted.js"))).toBe(false);
    expect(read("only-in-s3.js")).toBe("from s3\n");
    expect(calls.map((args) => args.slice(0, 2))).toEqual([["sts", "get-caller-identity"], ["s3", "sync"]]);
    expect(calls[1].slice(2, 4)).toEqual([`s3://${BUCKET}/${KEY_PREFIX}packs/`, join(root, "packs")]);
  });

  it("changes nothing on a dry run", () => {
    const { root, aws, calls, write } = setUp();
    write(root, "packs/p/upstream/t/fixtures/a.js", "a\n");
    git(root, "add", ".");
    git(root, "commit", "-qm", "pack");
    write(root, "packs/p/upstream/t/fixtures/a.js", "edited\n");
    pull({ root, aws, dryRun: true, log: () => {} });
    expect(calls[1]).toContain("--dryrun");
    expect(readFileSync(join(root, "packs/p/upstream/t/fixtures/a.js"), "utf8")).toBe("edited\n");
  });
});

describe("push", () => {
  const tracked = (root) => git(root, "ls-files").split("\n").filter((path) => path !== "");

  function committed() {
    const context = setUp();
    const { root, write } = context;
    write(root, "packs/p/upstream/t/gates.yaml", "gates\n");
    write(root, "packs/p/upstream/t/fixtures.expected.json", "{}\n");
    write(root, "packs/p/upstream/t/fixtures/pages/[id].tsx", "page\n");
    git(root, "add", ".");
    git(root, "commit", "-qm", "pack");
    return context;
  }

  it("uploads the tracked pack test data, verifies it, and untracks it, leaving the files on disk", () => {
    const { root, bucket, aws } = committed();
    const lines = [];
    expect(push({ root, aws, log: (line) => lines.push(line) }).pushed).toEqual([
      "packs/p/upstream/t/fixtures.expected.json",
      "packs/p/upstream/t/fixtures/pages/[id].tsx",
    ]);
    expect(files(join(bucket, KEY_PREFIX))).toEqual(["packs/p/upstream/t/fixtures.expected.json", "packs/p/upstream/t/fixtures/pages/[id].tsx"]);
    expect(tracked(root)).toEqual(["packs/p/upstream/t/gates.yaml"]);
    expect(git(root, "diff", "--cached", "--name-only").trim().split("\n")).toHaveLength(2);
    expect(readFileSync(join(root, "packs/p/upstream/t/fixtures/pages/[id].tsx"), "utf8")).toBe("page\n");
    expect(lines).toContain("removed from the index: packs/p/upstream/t/fixtures/pages/[id].tsx");
  });

  it("removes nothing when the round trip changes a file", () => {
    const { root, aws } = committed();
    const corrupting = (args, options) => {
      const run = aws(args, options);
      if (args[1] === "cp" && args[2].startsWith("s3://")) writeFileSync(join(args[3], "fixtures.expected.json"), "other\n");
      return run;
    };
    expect(() => push({ root, aws: corrupting, log: () => {} })).toThrow(/round trip through S3 changed 1 file\(s\); nothing was removed from git:\n {2}packs\/p\/upstream\/t\/fixtures.expected.json/u);
    expect(tracked(root)).toHaveLength(3);
  });

  it("refuses staged changes and edited pack test data, and uploads nothing on a dry run", () => {
    const { root, bucket, aws, write } = committed();
    write(root, "packs/p/upstream/t/gates.yaml", "staged\n");
    git(root, "add", ".");
    expect(() => push({ root, aws, log: () => {} })).toThrow(/the index has staged changes/u);
    git(root, "commit", "-qm", "gates");
    write(root, "packs/p/upstream/t/fixtures.expected.json", "edited\n");
    expect(() => push({ root, aws, log: () => {} })).toThrow(/differs from the index in the working tree[^]*fixtures\.expected\.json/u);
    git(root, "checkout", "--", ".");
    const lines = [];
    push({ root, aws, dryRun: true, log: (line) => lines.push(line) });
    expect(lines.at(-1)).toBe("dry run: would upload, verify and untrack 2 file(s)");
    expect(files(bucket)).toEqual([]);
    expect(tracked(root)).toHaveLength(3);
  });
});

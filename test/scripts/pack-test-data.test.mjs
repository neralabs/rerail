// scripts/s3/pack-fixtures.mjs: which paths are pack test data and which of it is authored, the aws filters and
// messages built from them, and pull and push against a temporary git checkout with a fake `aws` whose bucket is a local
// directory.

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  authoredListing,
  authoredSyncs,
  BUCKET,
  KEY_PREFIX,
  literalFilter,
  isPackFixture,
  packFixtureKind,
  packFixturesWritten,
  pull,
  push,
  requireAws,
} from "../../scripts/s3/pack-fixtures.mjs";

/** A tool's UPSTREAM.md listing `files` (pack-relative) as Rerail's. */
const record = (files) => `# cases Rerail wrote\n\n| File | Origin |\n|---|---|\n${files.map((file) => `| \`${file}\` | written by Rerail |\n`).join("")}`;

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
      "packs/node/upstream/eslint-plugin-n/authored/corpus/src/a.js",
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

  it("brackets fnmatch's special characters in a literal filter", () => {
    expect(literalFilter("fixtures/pages/[id]/a?b*.tsx")).toBe("fixtures/pages/[[]id]/a[?]b[*].tsx");
  });
});

describe("authored and derived", () => {
  it("takes what the tool's own UPSTREAM.md lists, and upstream/<tool>/authored/, as authored; the rest is derived", () => {
    const { root, write } = setUp();
    write(root, "packs/p/upstream/t/UPSTREAM.md", record(["upstream/t/fixtures/rerail/a.py", "upstream/other/fixtures/b.py"]));
    const authored = authoredListing(root);
    expect([...authored]).toEqual(["packs/p/upstream/t/fixtures/rerail/a.py"]);
    expect(packFixtureKind("packs/p/upstream/t/fixtures/rerail/a.py", authored)).toBe("authored");
    expect(packFixtureKind("packs/p/upstream/t/authored/corpus/x.py", authored)).toBe("authored");
    expect(packFixtureKind("packs/p/upstream/t/fixtures/upstream/a.py", authored)).toBe("derived");
    expect(packFixtureKind("packs/p/upstream/t/fixtures.expected.json", authored)).toBe("derived");
    expect(packFixtureKind("packs/p/upstream/t/gates.yaml", authored)).toBeUndefined();
  });

  it("syncs the authored/ directories and, per tool directory, the listed files as literal filters", () => {
    const { root, write } = setUp();
    write(root, "packs/p/upstream/t/UPSTREAM.md", record(["upstream/t/fixtures/pages/[id].tsx", "upstream/t/fixtures/rerail/a.py"]));
    expect(authoredSyncs(root)).toEqual([
      { prefix: "packs/", filters: ["--exclude", "*", "--include", "*/upstream/*/authored/*"] },
      { prefix: "packs/p/upstream/t/", filters: ["--exclude", "*", "--include", "fixtures/pages/[[]id].tsx", "--include", "fixtures/rerail/a.py"] },
    ]);
  });

  it("tells a contributor to add only the authored files a recorder wrote, quoting paths the shell would split", () => {
    const { root, write } = setUp();
    write(root, "packs/a/upstream/t/UPSTREAM.md", record(["upstream/t/fixtures/p [x].js"]));
    write(root, "packs/a/upstream/t/fixtures/p [x].js", "mine\n");
    write(root, "packs/a/upstream/t/fixtures/upstream.js", "theirs\n");
    write(root, "packs/a/upstream/t/fixtures.expected.json", "{}\n");
    const message = packFixturesWritten([join(root, "packs/a/upstream/t/fixtures"), join(root, "packs/a/upstream/t/fixtures.expected.json")], root);
    expect(message).toContain("Wrote 2 derived pack test data file(s)");
    expect(message).toContain("git add -f -- 'packs/a/upstream/t/fixtures/p [x].js'\n");
    expect(packFixturesWritten([join(root, "packs/a/upstream/t/fixtures.expected.json")], root)).not.toContain("git add");
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
  existsSync(root)
    ? readdirSync(root, { recursive: true, withFileTypes: true })
      .filter((entry) => entry.isFile())
      .map((entry) => relative(root, join(entry.parentPath, entry.name)).split("\\").join("/"))
      .sort()
    : [];

const git = (root, ...args) => execFileSync("git", args, { cwd: root, encoding: "utf8" });

/** An aws filter as a matcher: fnmatch, whose `*` crosses `/`, with `[c]` a literal c. */
const filterMatcher = (filter) =>
  new RegExp(`^${[...filter.matchAll(/\[(.)\]|(\*)|(.)/gsu)].map(([, bracketed, star, other]) => (star === undefined ? (bracketed ?? other).replace(/[.*+?^${}()|[\]\\/]/gu, "\\$&") : ".*")).join("")}$`, "u");

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
    const includes = args.flatMap((arg, index) => (args[index - 1] === "--include" ? [filterMatcher(arg)] : []));
    if (args.includes("--dryrun")) return { status: 0 };
    if (source.startsWith("s3://")) {
      for (const path of files(s3(source)).filter((path) => includes.some((include) => include.test(path)))) write(target, path, readFileSync(join(s3(source), path)));
    } else if (command === "cp") {
      for (const path of files(source)) write(s3(target), path, readFileSync(join(source, path)));
    }
    return { status: 0 };
  };
  return { root, bucket, aws, calls, write };
}

describe("pull", () => {
  it("fetches only authored data, then gives tracked files their committed content and keeps working-tree edits", () => {
    const { root, aws, calls, write, bucket } = setUp();
    const key = (path) => join(KEY_PREFIX, path);
    const listed = ["committed.js", "edited.js", "deleted.js", "only-in-s3.js"];
    write(root, "packs/p/upstream/t/UPSTREAM.md", record(listed.map((path) => `upstream/t/fixtures/rerail/${path}`)));
    write(root, "packs/p/upstream/t/fixtures/rerail/committed.js", "committed\n");
    write(root, "packs/p/upstream/t/fixtures/rerail/edited.js", "committed\n");
    write(root, "packs/p/upstream/t/fixtures/rerail/deleted.js", "committed\n");
    git(root, "add", ".");
    git(root, "commit", "-qm", "pack");
    write(root, "packs/p/upstream/t/fixtures/rerail/edited.js", "edited\n");
    rmSync(join(root, "packs/p/upstream/t/fixtures/rerail/deleted.js"));
    for (const path of listed) write(bucket, key(`packs/p/upstream/t/fixtures/rerail/${path}`), "from s3\n");
    write(bucket, key("packs/p/upstream/t/fixtures/upstream.js"), "derived\n");
    write(bucket, key("packs/p/upstream/t/fixtures.expected.json"), "{}\n");
    write(bucket, key("packs/q/upstream/u/authored/corpus/a.py"), "corpus\n");
    const lines = [];

    expect(pull({ root, aws, log: (line) => lines.push(line) })).toEqual({ restored: 1, kept: 2 });

    const read = (path) => readFileSync(join(root, "packs/p/upstream/t/fixtures/rerail", path), "utf8");
    expect(read("committed.js")).toBe("committed\n");
    expect(read("edited.js")).toBe("edited\n");
    expect(existsSync(join(root, "packs/p/upstream/t/fixtures/rerail/deleted.js"))).toBe(false);
    expect(read("only-in-s3.js")).toBe("from s3\n");
    expect(readFileSync(join(root, "packs/q/upstream/u/authored/corpus/a.py"), "utf8")).toBe("corpus\n");
    expect(existsSync(join(root, "packs/p/upstream/t/fixtures/upstream.js"))).toBe(false);
    expect(existsSync(join(root, "packs/p/upstream/t/fixtures.expected.json"))).toBe(false);
    expect(calls.map((args) => args.slice(0, 4))).toEqual([
      ["sts", "get-caller-identity", "--output", "json"],
      ["s3", "sync", `s3://${BUCKET}/${KEY_PREFIX}packs/`, join(root, "packs/")],
      ["s3", "sync", `s3://${BUCKET}/${KEY_PREFIX}packs/p/upstream/t/`, join(root, "packs/p/upstream/t/")],
    ]);
  });

  it("changes nothing on a dry run", () => {
    const { root, aws, calls, write } = setUp();
    write(root, "packs/p/upstream/t/UPSTREAM.md", record(["upstream/t/fixtures/a.js"]));
    write(root, "packs/p/upstream/t/fixtures/a.js", "a\n");
    git(root, "add", ".");
    git(root, "commit", "-qm", "pack");
    write(root, "packs/p/upstream/t/fixtures/a.js", "edited\n");
    pull({ root, aws, dryRun: true, log: () => {} });
    expect(calls.slice(1).every((args) => args.includes("--dryrun"))).toBe(true);
    expect(readFileSync(join(root, "packs/p/upstream/t/fixtures/a.js"), "utf8")).toBe("edited\n");
  });
});

describe("push", () => {
  const tracked = (root) => git(root, "ls-files").split("\n").filter((path) => path !== "");

  function committed() {
    const context = setUp();
    const { root, write } = context;
    write(root, "packs/p/upstream/t/gates.yaml", "gates\n");
    write(root, "packs/p/upstream/t/UPSTREAM.md", record(["upstream/t/fixtures/rerail/case.py", "upstream/t/fixtures/pages/[id].tsx"]));
    write(root, "packs/p/upstream/t/fixtures/rerail/case.py", "case\n");
    write(root, "packs/p/upstream/t/fixtures/pages/[id].tsx", "page\n");
    git(root, "add", ".");
    git(root, "commit", "-qm", "pack");
    return context;
  }

  it("uploads the tracked authored data, verifies it, and untracks it, leaving the files on disk", () => {
    const { root, bucket, aws } = committed();
    const lines = [];
    expect(push({ root, aws, log: (line) => lines.push(line) }).pushed).toEqual([
      "packs/p/upstream/t/fixtures/pages/[id].tsx",
      "packs/p/upstream/t/fixtures/rerail/case.py",
    ]);
    expect(files(join(bucket, KEY_PREFIX))).toEqual(["packs/p/upstream/t/fixtures/pages/[id].tsx", "packs/p/upstream/t/fixtures/rerail/case.py"]);
    expect(tracked(root)).toEqual(["packs/p/upstream/t/UPSTREAM.md", "packs/p/upstream/t/gates.yaml"]);
    expect(git(root, "diff", "--cached", "--name-only").trim().split("\n")).toHaveLength(2);
    expect(readFileSync(join(root, "packs/p/upstream/t/fixtures/pages/[id].tsx"), "utf8")).toBe("page\n");
    expect(lines).toContain("removed from the index: packs/p/upstream/t/fixtures/pages/[id].tsx");
  });

  it("refuses tracked derived data, which is never uploaded", () => {
    const { root, bucket, aws, write } = committed();
    write(root, "packs/p/upstream/t/fixtures.expected.json", "{}\n");
    git(root, "add", "-f", ".");
    git(root, "commit", "-qm", "derived");
    expect(() => push({ root, aws, log: () => {} })).toThrow(/tracked pack test data is derived[^]*\n {2}packs\/p\/upstream\/t\/fixtures\.expected\.json$/u);
    expect(files(bucket)).toEqual([]);
  });

  it("removes nothing when the round trip changes a file", () => {
    const { root, aws } = committed();
    const corrupting = (args, options) => {
      const run = aws(args, options);
      if (args[1] === "cp" && args[2].startsWith("s3://")) {
        for (const path of files(args[3]).filter((path) => path.endsWith("/case.py"))) writeFileSync(join(args[3], path), "other\n");
      }
      return run;
    };
    expect(() => push({ root, aws: corrupting, log: () => {} })).toThrow(/round trip through S3 changed 1 file\(s\); nothing was removed from git:\n {2}packs\/p\/upstream\/t\/fixtures\/rerail\/case.py/u);
    expect(tracked(root)).toHaveLength(4);
  });

  it("refuses staged changes and edited pack test data, and uploads nothing on a dry run", () => {
    const { root, bucket, aws, write } = committed();
    write(root, "packs/p/upstream/t/gates.yaml", "staged\n");
    git(root, "add", ".");
    expect(() => push({ root, aws, log: () => {} })).toThrow(/the index has staged changes/u);
    git(root, "commit", "-qm", "gates");
    write(root, "packs/p/upstream/t/fixtures/rerail/case.py", "edited\n");
    expect(() => push({ root, aws, log: () => {} })).toThrow(/differs from the index in the working tree[^]*case\.py/u);
    git(root, "checkout", "--", ".");
    const lines = [];
    push({ root, aws, dryRun: true, log: (line) => lines.push(line) });
    expect(lines.at(-1)).toBe("dry run: would upload, verify and untrack 2 file(s)");
    expect(files(bucket)).toEqual([]);
    expect(tracked(root)).toHaveLength(4);
  });
});

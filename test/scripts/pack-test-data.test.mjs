// scripts/s3/pack-fixtures.mjs: which paths are pack test data and which of it is authored, and pull, push (from the
// data on disk, or from a pull request's commit), pack, upload, recordings (the pull of the one recording archive
// and its tree, and recordings push), check and the CI cache entry against a temporary git checkout with a fake
// `aws` whose bucket is a local directory.

import { execFileSync, spawnSync } from "node:child_process";
import { chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, renameSync, rmSync, symlinkSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import {
  ARCHIVE_CACHE,
  archiveCache,
  archiveCachePath,
  archiveKey,
  authoredListing,
  BUCKET,
  check,
  isPackFixture,
  PACK_FIXTURE_PATTERNS,
  pack,
  packFixtureKind,
  packFixturesWritten,
  pull,
  push,
  RECORDING_CACHE,
  RECORDING_TREE_MARKER,
  recordingArchiveCacheFile,
  recordingKey,
  recordingTreeDirectory,
  recordings,
  recordingsPush,
  readLock,
  readSamplesLock,
  requireAws,
  SAMPLES_LOCK,
  samples,
  samplesArchiveCachePath,
  samplesKey,
  samplesPush,
  samplesTree,
  samplesTreeDirectory,
  upload,
} from "../../scripts/s3/pack-fixtures.mjs";
import { SAMPLE_ID } from "../../scripts/samples/format.ts";
import { archiveSizeBound, buildArchive, compressArchive, decompressArchive, lockText, readArchive, sha256 } from "../../scripts/s3/test-data-archive.mjs";

/** A tool's UPSTREAM.md listing `files` (pack-relative) as Rerail's. */
const record = (files) => `# cases Rerail wrote\n\n| File | Origin |\n|---|---|\n${files.map((file) => `| \`${file}\` | written by Rerail |\n`).join("")}`;

const scratch = [];
afterEach(() => {
  for (const directory of scratch.splice(0)) rmSync(directory, { recursive: true, force: true });
});

/** Every file under `root`, relative to it, sorted. */
const files = (root) =>
  existsSync(root)
    ? readdirSync(root, { recursive: true, withFileTypes: true })
      .filter((entry) => entry.isFile())
      .map((entry) => relative(root, join(entry.parentPath, entry.name)).split("\\").join("/"))
      .sort()
    : [];

/** Every file under packs/ of the checkout, with its content, or only the pack test data with `dataOnly`. */
const tree = (root, dataOnly = false) =>
  Object.fromEntries(
    files(join(root, "packs"))
      .map((path) => `packs/${path}`)
      .filter((path) => !dataOnly || isPackFixture(path))
      .map((path) => [path, readFileSync(join(root, path), "utf8")]),
  );

const git = (root, ...args) => execFileSync("git", args, { cwd: root, encoding: "utf8" });
const lines = (text) => text.split("\n").filter((line) => line !== "");

const LOCKS = ["packs/p/upstream/t/test-data.lock", "packs/p/upstream/w/test-data.lock", "packs/q/upstream/u/test-data.lock"];
const T = { pack: "p", tool: "t", directory: "packs/p/upstream/t" };
const U = { pack: "q", tool: "u", directory: "packs/q/upstream/u" };

/**
 * A git checkout whose .gitignore covers pack test data and the archive cache, as the repository's does, and a bucket
 * directory holding each key as a file, with the fake `aws` that serves it and records its calls.
 */
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
  const write = (path, content) => {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), content);
  };
  write(".gitignore", `${["/.rerail/", ...PACK_FIXTURE_PATTERNS.map((pattern) => `/${pattern}`)].join("\n")}\n`);
  const object = (key) => join(bucket, key);
  const calls = [];
  const aws = (args) => {
    calls.push(args);
    const option = (name) => (args.includes(name) ? args[args.indexOf(name) + 1] : undefined);
    if (args[0] === "sts" && args[1] === "get-caller-identity") return { status: 0, stdout: "{}" };
    if (args[0] === "s3" && args[1] === "cp" && args[2].startsWith(`s3://${BUCKET}/`)) {
      const source = object(args[2].slice(`s3://${BUCKET}/`.length));
      if (!existsSync(source)) return { status: 1, stderr: `fatal error: An error occurred (404) when calling the HeadObject operation: Key "${args[2]}" does not exist\n` };
      copyFileSync(source, args[3]);
      return { status: 0, stdout: "" };
    }
    if (args[0] === "s3api" && args[1] === "head-object" && option("--bucket") === BUCKET) {
      if (!existsSync(object(option("--key")))) return { status: 254, stderr: "\nAn error occurred (404) when calling the HeadObject operation: Not Found\n" };
      return { status: 0, stdout: "{}" };
    }
    if (args[0] === "s3api" && args[1] === "put-object" && option("--bucket") === BUCKET && option("--if-none-match") === "*") {
      const target = object(option("--key"));
      if (existsSync(target)) {
        return { status: 254, stderr: "\nAn error occurred (PreconditionFailed) when calling the PutObject operation: At least one of the pre-conditions you specified did not hold\n" };
      }
      mkdirSync(dirname(target), { recursive: true });
      copyFileSync(option("--body"), target);
      return { status: 0, stdout: "{}" };
    }
    throw new Error(`the fake aws does not implement: aws ${args.join(" ")}`);
  };
  /** The calls of one kind, e.g. "put-object" or "cp". */
  const called = (command) => calls.filter((args) => args[1] === command);
  return { root, bucket, object, aws, calls, called, write };
}

/** setUp with three tool-packs' test data on disk, untracked, and everything else committed. */
function withData() {
  const context = setUp();
  const { root, write } = context;
  write("packs/p/upstream/t/gates.yaml", "gates\n");
  write("packs/p/upstream/t/UPSTREAM.md", record(["upstream/t/fixtures/rerail/a.py"]));
  write("packs/p/upstream/t/fixtures/rerail/a.py", "authored\n");
  write("packs/p/upstream/t/fixtures/upstream/b.py", "derived\n");
  write("packs/p/upstream/t/x y.expected.json", "{}\n");
  write("packs/p/upstream/w/gates.yaml", "gates\n");
  write("packs/p/upstream/w/cases.json", "[]\n");
  write("packs/q/upstream/u/gates.yaml", "gates\n");
  write("packs/q/upstream/u/authored/corpus/c.py", "corpus\n");
  git(root, "add", ".");
  git(root, "commit", "-qm", "packs");
  context.data = tree(root, true);
  return context;
}

/** withData pushed and its locks committed; the fake aws's calls so far are forgotten. */
function published() {
  const context = withData();
  push({ root: context.root, aws: context.aws, log: () => {} });
  git(context.root, "add", ".");
  git(context.root, "commit", "-qm", "locks");
  context.calls.length = 0;
  return context;
}

/** Removes the pack test data from disk, and the archive cache unless `keepCache`: a fresh checkout. */
function fresh(root, keepCache = false) {
  for (const path of Object.keys(tree(root, true))) rmSync(join(root, path));
  if (!keepCache) rmSync(join(root, ARCHIVE_CACHE), { recursive: true, force: true });
}

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
      "packs/python/upstream/ruff/recording.lock",
      "packs/python/upstream/ruff/test-data.lock",
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
});

describe("authored and derived", () => {
  it("takes what the tool's own UPSTREAM.md lists, and upstream/<tool>/authored/, as authored; the rest is derived", () => {
    const { root, write } = setUp();
    write("packs/p/upstream/t/UPSTREAM.md", record(["upstream/t/fixtures/rerail/a.py", "upstream/other/fixtures/b.py"]));
    const authored = authoredListing(root);
    expect([...authored]).toEqual(["packs/p/upstream/t/fixtures/rerail/a.py"]);
    expect(packFixtureKind("packs/p/upstream/t/fixtures/rerail/a.py", authored)).toBe("authored");
    expect(packFixtureKind("packs/p/upstream/t/authored/corpus/x.py", authored)).toBe("authored");
    expect(packFixtureKind("packs/p/upstream/t/fixtures/upstream/a.py", authored)).toBe("derived");
    expect(packFixtureKind("packs/p/upstream/t/fixtures.expected.json", authored)).toBe("derived");
    expect(packFixtureKind("packs/p/upstream/t/gates.yaml", authored)).toBeUndefined();
  });

  it("tells a contributor to add only the authored files a recorder wrote, quoting paths the shell would split", () => {
    const { root, write } = setUp();
    write("packs/a/upstream/t/UPSTREAM.md", record(["upstream/t/fixtures/p [x].js"]));
    write("packs/a/upstream/t/fixtures/p [x].js", "mine\n");
    write("packs/a/upstream/t/fixtures/upstream.js", "theirs\n");
    write("packs/a/upstream/t/fixtures.expected.json", "{}\n");
    const message = packFixturesWritten([join(root, "packs/a/upstream/t/fixtures"), join(root, "packs/a/upstream/t/fixtures.expected.json")], root);
    expect(message).toContain("npm run fixtures:push");
    expect(message).toContain("git add -f -- 'packs/a/upstream/t/fixtures/p [x].js'\n");
    expect(packFixturesWritten([join(root, "packs/a/upstream/t/fixtures.expected.json")], root)).not.toContain("git add");
  });
});

describe("requireAws", () => {
  it("says to install the CLI when aws is missing, and to sign in when it is not authenticated", () => {
    const missing = Object.assign(new Error("spawn aws ENOENT"), { code: "ENOENT" });
    expect(() => requireAws(() => ({ status: null, error: missing }))).toThrow(/AWS CLI \(aws\) is not installed[^]*s3:\/\/nera-test-fixtures\/rerail\/archives\//u);
    expect(() => requireAws(() => ({ status: 255, stderr: "\nUnable to locate credentials\n" }))).toThrow(
      /aws is not signed in \(aws sts get-caller-identity: Unable to locate credentials\)/u,
    );
    expect(() => requireAws(() => ({ status: 0, stdout: "{}" }))).not.toThrow();
  });
});

describe("pull", () => {
  it("fetches each locked archive from S3 into the cache and extracts it", () => {
    const { root, aws, called, data } = published();
    fresh(root);
    const result = pull({ root, aws, log: () => {} });
    expect(result.toolPacks.map(({ directory, source }) => [directory, source])).toEqual([
      ["packs/p/upstream/t", "S3"],
      ["packs/p/upstream/w", "S3"],
      ["packs/q/upstream/u", "S3"],
    ]);
    expect(tree(root, true)).toEqual(data);
    expect(called("cp")).toHaveLength(3);
    expect(files(join(root, ARCHIVE_CACHE))).toHaveLength(3);
  });

  it("gives tracked files their committed content, keeps their working-tree edits, and overwrites untracked ones", () => {
    const { root, aws, write } = published();
    write("packs/p/upstream/t/fixtures/rerail/a.py", "committed\n");
    write("packs/p/upstream/t/fixtures/rerail/new.py", "new\n");
    write("packs/p/upstream/t/x y.expected.json", "committed\n");
    git(root, "add", "-f", "--", "packs/p/upstream/t/fixtures/rerail", "packs/p/upstream/t/x y.expected.json");
    git(root, "commit", "-qm", "pull request data");
    write("packs/p/upstream/t/x y.expected.json", "edited\n");
    write("packs/p/upstream/t/fixtures/upstream/b.py", "untracked\n");

    expect(pull({ root, aws, log: () => {} })).toMatchObject({ restored: 2, kept: 1 });

    const read = (path) => readFileSync(join(root, "packs/p/upstream/t", path), "utf8");
    expect(read("fixtures/rerail/a.py")).toBe("committed\n");
    expect(read("fixtures/rerail/new.py")).toBe("new\n");
    expect(read("x y.expected.json")).toBe("edited\n");
    expect(read("fixtures/upstream/b.py")).toBe("derived\n");
  });

  it("deletes an untracked derived file the archive lacks and keeps an untracked authored one", () => {
    const { root, aws, write, data } = published();
    write("packs/p/upstream/t/fixtures/upstream/stale.py", "stale\n");
    write("packs/q/upstream/u/authored/corpus/new.py", "new\n");
    pull({ root, aws, log: () => {} });
    expect(tree(root, true)).toEqual({ ...data, "packs/q/upstream/u/authored/corpus/new.py": "new\n" });
  });

  it("uses the cached archives without aws on a second pull", () => {
    const { root, aws, calls, data } = published();
    fresh(root);
    pull({ root, aws, log: () => {} });
    fresh(root, true);
    calls.length = 0;
    const result = pull({ root, aws, log: () => {} });
    expect(calls).toEqual([]);
    expect(result.toolPacks.map(({ source }) => source)).toEqual(["cache", "cache", "cache"]);
    expect(tree(root, true)).toEqual(data);
  });

  it("selects one pack's tool-packs with --pack, and one tool-pack with --tool", () => {
    const { root, aws, called, data } = published();
    fresh(root);
    const only = (prefix) => Object.fromEntries(Object.entries(data).filter(([path]) => path.startsWith(prefix)));
    expect(pull({ root, aws, pack: "p", log: () => {} }).toolPacks.map(({ directory }) => directory)).toEqual(["packs/p/upstream/t", "packs/p/upstream/w"]);
    expect(tree(root, true)).toEqual(only("packs/p/"));
    expect(called("cp").every((args) => args[2].startsWith(`s3://${BUCKET}/rerail/archives/p/`))).toBe(true);
    fresh(root);
    pull({ root, aws, pack: "p", tool: "w", log: () => {} });
    expect(tree(root, true)).toEqual(only("packs/p/upstream/w/"));
  });

  it("refuses a symlinked directory an entry goes in, naming it, with nothing written through it or anywhere else", () => {
    const { root, aws } = published();
    fresh(root);
    const outside = join(dirname(root), "outside");
    mkdirSync(outside);
    rmSync(join(root, "packs/p/upstream/t/fixtures"), { recursive: true });
    symlinkSync(outside, join(root, "packs/p/upstream/t/fixtures"));
    expect(failure(() => pull({ root, aws, log: () => {} }))).toContain("packs/p/upstream/t/fixtures is a symlink");
    expect(files(outside)).toEqual([]);
    expect(tree(root, true)).toEqual({});
  });

  it("refuses a symlinked directory above the tool directory, naming it, with nothing written through it", () => {
    const { root, aws } = published();
    fresh(root);
    const elsewhere = join(dirname(root), "elsewhere");
    renameSync(join(root, "packs/q/upstream"), elsewhere);
    symlinkSync(elsewhere, join(root, "packs/q/upstream"));
    const before = files(elsewhere);
    expect(failure(() => pull({ root, aws, pack: "q", log: () => {} }))).toContain("packs/q/upstream is a symlink");
    expect(files(elsewhere)).toEqual(before);
    expect(before.filter((path) => isPackFixture(`packs/q/upstream/${path}`))).toEqual([]);
  });

  describe("fails, naming the lock and changing nothing in the checkout, on", () => {
    const label = "packs/q/upstream/u/test-data.lock";
    const objectUrl = (digest) => `s3://${BUCKET}/rerail/archives/q/u/${digest}.tar.zst`;
    const breaks = [
      [
        "a lock whose sha256 was edited, naming the key it expects",
        ({ root, write }) => {
          const lock = readLock(root, U);
          const digest = `${lock.sha256.slice(0, -1)}${lock.sha256.endsWith("0") ? "1" : "0"}`;
          write(label, lockText({ ...lock, sha256: digest }));
          return [label, objectUrl(digest)];
        },
      ],
      [
        "an archive S3 does not hold, naming its key",
        ({ root, object }) => {
          const { sha256: digest } = readLock(root, U);
          rmSync(object(archiveKey(U, digest)));
          return [label, objectUrl(digest)];
        },
      ],
      [
        "a corrupted object",
        ({ root, object }) => {
          const path = object(archiveKey(U, readLock(root, U).sha256));
          const bytes = readFileSync(path);
          for (let index = 8; index < bytes.length - 4; index += 7) bytes[index] ^= 0xff;
          writeFileSync(path, bytes);
          return [label];
        },
      ],
      [
        "a lock whose file count was edited",
        ({ root, write }) => {
          const lock = readLock(root, U);
          write(label, lockText({ ...lock, files: lock.files + 1 }));
          return [label];
        },
      ],
      [
        "a lock whose byte count was edited",
        ({ root, write }) => {
          const lock = readLock(root, U);
          write(label, lockText({ ...lock, bytes: lock.bytes - 1 }));
          return [label];
        },
      ],
      [
        "a malformed lock",
        ({ write }) => {
          write(label, '{ "format": 1 }\n');
          return [label];
        },
      ],
    ];

    it.each(breaks)("%s", (_, breakIt) => {
      const context = published();
      const { root, aws, write } = context;
      fresh(root);
      // The tool-pack sorted first verifies; a pull that went on would extract it and delete this stale derived file.
      write("packs/p/upstream/t/fixtures/upstream/stale.py", "stale\n");
      const named = breakIt(context);
      const before = tree(root);
      let message = "";
      try {
        pull({ root, aws, log: () => {} });
      } catch (error) {
        message = error.message;
      }
      for (const text of named) expect(message).toContain(text);
      expect(tree(root)).toEqual(before);
      expect(files(join(root, ARCHIVE_CACHE)).filter((path) => path.startsWith("q/"))).toEqual([]);
    });
  });
});

describe("push", () => {
  it("archives each tool-pack, uploads the archive under its sha256 and writes the lock", () => {
    const { root, aws, object, data } = withData();
    const result = push({ root, aws, log: () => {} });
    expect(result.changedLocks).toEqual(LOCKS);
    const toolPacks = LOCKS.map((lock) => {
      const [, pack, , tool] = lock.split("/");
      return { pack, tool, directory: dirname(lock) };
    });
    expect(result.uploaded).toEqual(toolPacks.map((toolPack) => archiveKey(toolPack, readLock(root, toolPack).sha256)));
    for (const toolPack of toolPacks) {
      const lock = readLock(root, toolPack);
      const tar = decompressArchive(readFileSync(object(archiveKey(toolPack, lock.sha256))));
      expect(sha256(tar)).toBe(lock.sha256);
      const entries = readArchive(tar);
      expect({ files: lock.files, bytes: lock.bytes }).toEqual({ files: entries.length, bytes: entries.reduce((total, entry) => total + entry.bytes.length, 0) });
      expect(Object.fromEntries(entries.map(({ path, bytes }) => [`${toolPack.directory}/${path}`, bytes.toString("utf8")]))).toEqual(
        Object.fromEntries(Object.entries(data).filter(([path]) => path.startsWith(`${toolPack.directory}/`))),
      );
    }
  });

  it("uploads nothing and leaves the locks byte-identical when the data is unchanged", () => {
    const { root, aws, called } = published();
    const locks = LOCKS.map((lock) => readFileSync(join(root, lock)));
    expect(push({ root, aws, log: () => {} })).toMatchObject({ changedLocks: [], uploaded: [] });
    expect(called("put-object")).toEqual([]);
    expect(LOCKS.map((lock) => readFileSync(join(root, lock)))).toEqual(locks);
  });

  it("prints the git commands that commit the locks and untrack exactly the tracked pack test data", () => {
    const { root, aws } = withData();
    const trackedData = [
      "packs/p/upstream/t/fixtures/rerail/a.py",
      "packs/p/upstream/t/fixtures/upstream/b.py",
      "packs/p/upstream/t/x y.expected.json",
      "packs/q/upstream/u/authored/corpus/c.py",
    ];
    git(root, "add", "-f", "--", ...trackedData);
    git(root, "commit", "-qm", "pull request data");
    const printed = [];
    expect(push({ root, aws, log: (line) => printed.push(...line.split("\n")) }).tracked).toEqual(trackedData);
    const command = (verb) => printed.filter((line) => line.startsWith(`  git ${verb} `)).map((line) => line.trim());
    expect(command("add")).toEqual([`git add -- ${LOCKS.join(" ")}`]);
    expect(command("rm")).toHaveLength(1);
    for (const line of [...command("add"), ...command("rm")]) execFileSync("sh", ["-c", line], { cwd: root });
    const staged = lines(git(root, "diff", "--cached", "--name-status", "--no-renames")).map((line) => line.split("\t"));
    expect(staged.filter(([status]) => status === "D").map(([, path]) => path)).toEqual(trackedData);
    expect(staged.filter(([status]) => status !== "D")).toEqual(LOCKS.map((lock) => ["A", lock]));
    expect(lines(git(root, "ls-files", "packs")).filter(isPackFixture)).toEqual([]);
    for (const path of trackedData) expect(existsSync(join(root, path))).toBe(true);
  });

  it("refuses tracked pack test data edited in the working tree", () => {
    const { root, aws, bucket, calls, write } = withData();
    git(root, "add", "-f", "--", "packs/p/upstream/t/fixtures/rerail/a.py");
    git(root, "commit", "-qm", "pull request data");
    write("packs/p/upstream/t/fixtures/rerail/a.py", "edited\n");
    expect(() => push({ root, aws, log: () => {} })).toThrow("packs/p/upstream/t/fixtures/rerail/a.py");
    expect(calls).toEqual([]);
    expect(files(bucket)).toEqual([]);
    expect(LOCKS.filter((lock) => existsSync(join(root, lock)))).toEqual([]);
  });
});

describe("upload", () => {
  const other = () => compressArchive(buildArchive([{ path: "cases.json", bytes: Buffer.from("other\n") }]).tar);

  it("refuses a cached archive that does not match its lock", () => {
    const { root, aws, bucket, called } = withData();
    pack({ root, log: () => {} });
    const cached = join(root, ARCHIVE_CACHE, "p/t", `${readLock(root, T).sha256}.tar.zst`);
    writeFileSync(cached, other());
    expect(() => upload({ root, aws, log: () => {} })).toThrow("packs/p/upstream/t/test-data.lock");
    expect(called("put-object")).toEqual([]);
    expect(files(bucket)).toEqual([]);
  });

  it("never overwrites a key S3 holds", () => {
    const { root, aws, object, called } = withData();
    pack({ root, log: () => {} });
    const key = object(archiveKey(T, readLock(root, T).sha256));
    mkdirSync(dirname(key), { recursive: true });
    const held = other();
    writeFileSync(key, held);
    expect(() => upload({ root, pack: "p", tool: "t", aws, log: () => {} })).toThrow("packs/p/upstream/t/test-data.lock");
    expect(called("put-object")).toEqual([]);
    // S3 says the key is absent, then holds it by the time the put lands: the conditional put is refused.
    const racing = (args) => (args[1] === "head-object" ? { status: 254, stderr: "An error occurred (404) when calling the HeadObject operation: Not Found\n" } : aws(args));
    expect(() => upload({ root, pack: "p", tool: "t", aws: racing, log: () => {} })).toThrow("packs/p/upstream/t/test-data.lock");
    expect(called("put-object")).toHaveLength(1);
    expect(readFileSync(key).equals(held)).toBe(true);
  });
});

describe("pack", () => {
  it("makes no aws call, and writes the same lock whatever the data's on-disk mode, mtime or write order", () => {
    const { root, data, write } = withData();
    const base = dirname(root);
    const bin = join(base, "bin");
    mkdirSync(bin);
    writeFileSync(join(bin, "aws"), `#!/bin/sh\necho "$@" >> ${JSON.stringify(join(base, "aws-calls"))}\nexit 1\n`, { mode: 0o755 });
    const path = process.env.PATH;
    process.env.PATH = `${bin}${delimiter}${path}`;
    try {
      expect(spawnSync("aws", ["--version"]).status).toBe(1);
      rmSync(join(base, "aws-calls"), { force: true });
      expect(pack({ root, log: () => {} }).changedLocks).toEqual(LOCKS);
      const locks = LOCKS.map((lock) => readFileSync(join(root, lock), "utf8"));
      expect(files(join(root, ARCHIVE_CACHE))).toEqual(LOCKS.map((lock, index) => `${lock.split("/")[1]}/${lock.split("/")[3]}/${JSON.parse(locks[index]).sha256}.tar.zst`));

      for (const file of Object.keys(data)) rmSync(join(root, file));
      for (const [file, text] of Object.entries(data).reverse()) {
        write(file, text);
        chmodSync(join(root, file), 0o755);
        utimesSync(join(root, file), new Date("2001-02-03T04:05:06Z"), new Date("2001-02-03T04:05:06Z"));
      }
      expect(pack({ root, log: () => {} }).changedLocks).toEqual([]);
      expect(LOCKS.map((lock) => readFileSync(join(root, lock), "utf8"))).toEqual(locks);
      expect(existsSync(join(base, "aws-calls"))).toBe(false);
    } finally {
      process.env.PATH = path;
    }
  });

  it("refuses a symlink among the pack test data rather than leave it out, before writing any lock", () => {
    const { root } = withData();
    const outside = join(dirname(root), "outside.expected.json");
    writeFileSync(outside, "{}\n");
    symlinkSync(outside, join(root, "packs/q/upstream/u/linked.expected.json"));
    expect(failure(() => pack({ root, log: () => {} }))).toContain("packs/q/upstream/u/linked.expected.json is a symlink");
    expect(LOCKS.filter((lock) => existsSync(join(root, lock)))).toEqual([]);
  });
});

describe("archiveCache", () => {
  it("keys one tool-pack's entry by its lock's sha256", () => {
    const { root } = published();
    const { sha256: digest } = readLock(root, T);
    expect(archiveCache({ root, pack: "p", tool: "t" })).toEqual({ key: `pack-test-data-p-t-${digest}`, paths: [`${ARCHIVE_CACHE}/p/t/${digest}.tar.zst`] });
  });

  it("changes the key for several tool-packs when any lock changes", () => {
    const { root, write } = published();
    const original = archiveCache({ root }).key;
    const keys = [original];
    for (const toolPack of [U, T]) {
      const lock = readLock(root, toolPack);
      write(`${toolPack.directory}/test-data.lock`, lockText({ ...lock, sha256: sha256(Buffer.from(lock.sha256)) }));
      keys.push(archiveCache({ root }).key);
    }
    expect(new Set(keys).size).toBe(3);
    expect(archiveCache({ root, pack: "p" }).key).not.toBe(archiveCache({ root }).key);
  });

  it("lists the archives' paths sorted by tool-pack", () => {
    const { root, write } = setUp();
    const digest = (seed) => sha256(Buffer.from(seed));
    for (const directory of ["packs/b/upstream/a", "packs/a/upstream/b", "packs/a/upstream/Z"]) {
      write(`${directory}/test-data.lock`, lockText({ sha256: digest(directory), files: 1, bytes: 1 }));
    }
    expect(archiveCache({ root }).paths).toEqual([
      `${ARCHIVE_CACHE}/a/Z/${digest("packs/a/upstream/Z")}.tar.zst`,
      `${ARCHIVE_CACHE}/a/b/${digest("packs/a/upstream/b")}.tar.zst`,
      `${ARCHIVE_CACHE}/b/a/${digest("packs/b/upstream/a")}.tar.zst`,
    ]);
  });
});

const putFile = (path, content) => {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, content);
};

/** The message `run` throws; fails the test when it does not throw. */
function failure(run) {
  try {
    run();
  } catch (error) {
    return error.message;
  }
  throw new Error("it did not fail");
}

describe("an archive that is not the canonical archive of its tool directory's pack test data", () => {
  const label = "packs/q/upstream/u/test-data.lock";
  const corpus = { path: "authored/corpus/c.py", bytes: Buffer.from("corpus\n") };
  /** The tar and the lock that names it truthfully: sha256, file count and bytes all match. */
  const truthful = (tar) => {
    const entries = readArchive(tar);
    return { tar, lock: { sha256: sha256(tar), files: entries.length, bytes: entries.reduce((total, entry) => total + entry.bytes.length, 0) } };
  };
  const big = buildArchive([{ path: "authored/corpus/big.py", bytes: Buffer.alloc(1 << 20, 0x61) }]).tar;
  const cases = [
    ["holding a generator", () => truthful(buildArchive([corpus, { path: "generate.py", bytes: Buffer.from("import os\n") }]).tar), "not pack test data of packs/q/upstream/u, e.g. generate.py"],
    ["holding a lock", () => truthful(buildArchive([corpus, { path: "test-data.lock", bytes: Buffer.from("{}\n") }]).tar), "not pack test data of packs/q/upstream/u, e.g. test-data.lock"],
    [
      "whose entries are unsorted",
      () => truthful(Buffer.concat([buildArchive([{ path: "authored/corpus/z.py", bytes: Buffer.from("z\n") }]).tar.subarray(0, -1024), buildArchive([corpus]).tar])),
      "is not the canonical archive of its own files",
    ],
    ["that decompresses past the size its lock allows", () => ({ tar: big, lock: { sha256: sha256(big), files: 1, bytes: 16 } }), "is not a zstd archive of the size the lock allows"],
  ];

  it("decompresses past archiveSizeBound in the size case", () => {
    expect(big.length).toBeGreaterThan(archiveSizeBound({ files: 1, bytes: 16 }));
  });

  it.each(cases)("%s is refused by pull, naming the lock and key, with nothing written", (_, make, reason) => {
    const { root, aws, object, write } = published();
    fresh(root);
    const { tar, lock } = make();
    write(label, lockText(lock));
    putFile(object(archiveKey(U, lock.sha256)), compressArchive(tar));
    const before = tree(root);
    const message = failure(() => pull({ root, aws, log: () => {} }));
    for (const text of [reason, label, `s3://${BUCKET}/${archiveKey(U, lock.sha256)}`]) expect(message).toContain(text);
    expect(tree(root)).toEqual(before);
    expect(files(join(root, ARCHIVE_CACHE)).filter((path) => path.startsWith("q/"))).toEqual([]);
  });

  it.each(cases)("%s is refused by upload, naming the lock and key, with nothing put", (_, make, reason) => {
    const { root, aws, bucket, called, write } = withData();
    pack({ root, log: () => {} });
    const { tar, lock } = make();
    write(label, lockText(lock));
    putFile(archiveCachePath(root, U, lock.sha256), compressArchive(tar));
    const message = failure(() => upload({ root, aws, log: () => {} }));
    for (const text of [reason, label, `s3://${BUCKET}/${archiveKey(U, lock.sha256)}`]) expect(message).toContain(text);
    expect(called("put-object")).toEqual([]);
    expect(files(bucket)).toEqual([]);
  });

  it("a corrupt cached archive where S3 cannot be read says to delete the Actions cache entry", () => {
    const { root } = published();
    fresh(root, true);
    const cached = archiveCachePath(root, U, readLock(root, U).sha256);
    writeFileSync(cached, "not zstd");
    const signedOut = () => ({ status: 255, stderr: "\nUnable to locate credentials\n" });
    const message = failure(() => pull({ root, aws: signedOut, log: () => {} }));
    expect(message).toContain(relative(root, cached));
    expect(message).toContain("gh cache delete");
    expect(existsSync(cached)).toBe(false);
  });

  it("a dry run deletes no corrupt cached archive, and says it would", () => {
    const { root, aws } = published();
    fresh(root, true);
    const cached = archiveCachePath(root, U, readLock(root, U).sha256);
    writeFileSync(cached, "not zstd");
    const printed = [];
    pull({ root, aws, dryRun: true, log: (line) => printed.push(line) });
    expect(readFileSync(cached, "utf8")).toBe("not zstd");
    expect(printed.join("\n")).toContain(`would delete ${relative(root, cached)}`);
  });
});

const RECORDING_LOCK_V4 = "rerail.port-readiness-recording-lock/v4";
/** A recording tree, as archive path → content, in the recording archive's layout (conformance/port-readiness/recording/). */
const ANSWERS = {
  "config/c0.json": '{\n  "format": 1\n}\n',
  "tool/c0/identity.json": '{\n  "name": "t"\n}\n',
  "rules/c0/no-null/rule.json": '{\n  "cases": []\n}\n',
  "rules/c0/no-null/texts.json": "[]\n",
  "texts/0f.json": '"a"\n',
  "rules/c0/no-null/cases/a0f.json": '{\n  "findings": [\n    [1, 1, 1, 5]\n  ]\n}\n',
  "rules/c0/no-null/cases/b1e.json": '{\n  "findings": []\n}\n',
  "verdicts/v0/0f.js.json": "[]\n",
};

/**
 * Writes, with `write`, a v4 recording.lock into `toolPack`'s directory naming the canonical archive of `answers`, or
 * `archive` when given (truthfully: its tar's sha256, file count and bytes); returns `recording` as
 * `{ name, bytes, sha256, key, local(root), tree(root) }` with `bytes` compressed as stored, and as `named`.
 */
function lockRecordings(write, toolPack = T, answers = ANSWERS, archive = buildArchive(Object.entries(answers).map(([path, content]) => ({ path, bytes: Buffer.from(content) })))) {
  const recording = {
    name: "recording.tar.zst",
    bytes: compressArchive(archive.tar),
    sha256: archive.sha256,
    key: recordingKey(toolPack, archive.sha256),
    local: (root) => recordingArchiveCacheFile(root, archive.sha256),
    tree: (root) => recordingTreeDirectory(root, archive.sha256),
  };
  const locked = { "recording.tar.zst": { sha256: archive.sha256, files: archive.files, bytes: archive.bytes, format: "rerail.port-readiness-recording-archive/v1" } };
  write(`${toolPack.directory}/recording.lock`, `${JSON.stringify({ format: RECORDING_LOCK_V4, tool: toolPack.tool, files: locked }, null, 2)}\n`);
  return { recording, named: [recording] };
}

/**
 * An archive holding two answers whose paths differ only by case, which buildArchive refuses to make: each file's
 * canonical tar, the first without its end blocks, then the second.
 */
function caseCollidingArchive() {
  const content = Buffer.from('{\n  "findings": []\n}\n');
  const [upper, lower] = ["A", "a"].map((name) => buildArchive([{ path: `rules/c0/no-null/cases/${name}0f.json`, bytes: content }]).tar);
  const tar = Buffer.concat([upper.subarray(0, upper.length - 1024), lower]);
  return { tar, sha256: sha256(tar), files: 2, bytes: 2 * content.length };
}

const keys = (named) => named.map((recording) => recording.key).sort();
/** What a pull extracted for `recording`, as path → content (the marker aside): ANSWERS when it extracted the whole archive. */
const extractedTree = (root, recording) => Object.fromEntries(files(recording.tree(root)).filter((path) => path !== RECORDING_TREE_MARKER).map((path) => [path, readFileSync(join(recording.tree(root), path), "utf8")]));
const treeLabel = (root, recording) => relative(root, recording.tree(root)).split("\\").join("/");

describe("recordings", () => {
  const quiet = () => {};
  /** withData, with a recording.lock for packs/p/upstream/t naming the archive of `answers`, or `archive`. */
  function locked(answers = ANSWERS, archive = undefined) {
    const context = withData();
    return { ...context, ...lockRecordings(context.write, T, answers, archive) };
  }

  it("fetches the recording archive from S3, verified, and extracts its tree", () => {
    const { root, aws, object, called, recording } = locked();
    putFile(object(recording.key), recording.bytes);
    expect(recording.key).toBe(`rerail/recordings/p/t/${recording.sha256}.tar.zst`);
    expect(recordings({ root, aws, log: quiet })).toEqual({ kept: [], downloaded: [recording.key], missing: [], extracted: [treeLabel(root, recording)] });
    expect(readFileSync(recording.local(root))).toEqual(recording.bytes);
    expect(treeLabel(root, recording)).toBe(`.rerail/port-readiness/recordings/tree/${recording.sha256}`);
    expect(extractedTree(root, recording)).toEqual(ANSWERS);
    // The marker names the tar the tree was extracted from.
    expect(readFileSync(join(recording.tree(root), RECORDING_TREE_MARKER), "utf8")).toBe(`${recording.sha256}\n`);
    expect(called("cp")).toHaveLength(1);
  });

  it("keeps a valid cached archive and extracts it, calling no aws", () => {
    const { root, aws, calls, recording } = locked();
    putFile(recording.local(root), recording.bytes);
    expect(recordings({ root, aws, log: quiet })).toEqual({ kept: [recording.key], downloaded: [], missing: [], extracted: [treeLabel(root, recording)] });
    expect(extractedTree(root, recording)).toEqual(ANSWERS);
    expect(calls).toEqual([]);
  });

  it("reuses a tree whose files rebuild the locked tar, without its archive, and extracts again a tree missing a file", () => {
    const { root, aws, calls, recording } = locked();
    putFile(recording.local(root), recording.bytes);
    recordings({ root, aws, log: quiet });
    rmSync(recording.local(root));
    expect(recordings({ root, aws, log: quiet })).toEqual({ kept: [recording.key], downloaded: [], missing: [], extracted: [] });

    putFile(recording.local(root), recording.bytes);
    rmSync(join(recording.tree(root), "rules/c0/no-null/rule.json"));
    expect(recordings({ root, aws, log: quiet }).extracted).toEqual([treeLabel(root, recording)]);
    expect(extractedTree(root, recording)).toEqual(ANSWERS);
    expect(calls).toEqual([]);
  });

  it("extracts again a tree whose marker names another archive, though its files and bytes count the same", () => {
    const { root, aws, recording } = locked();
    putFile(recording.local(root), recording.bytes);
    recordings({ root, aws, log: quiet });
    writeFileSync(join(recording.tree(root), "verdicts/v0/0f.js.json"), "{}\n");
    writeFileSync(join(recording.tree(root), RECORDING_TREE_MARKER), `${"0".repeat(64)}\n`);
    expect(recordings({ root, aws, log: quiet }).extracted).toEqual([treeLabel(root, recording)]);
    expect(extractedTree(root, recording)).toEqual(ANSWERS);
    // A tree without a marker is no finished extraction either.
    rmSync(join(recording.tree(root), RECORDING_TREE_MARKER));
    expect(recordings({ root, aws, log: quiet }).extracted).toEqual([treeLabel(root, recording)]);
    expect(existsSync(join(recording.tree(root), RECORDING_TREE_MARKER))).toBe(true);
  });

  it("extracts again a tree whose marker names the archive but whose files were edited or renamed, the counts kept", () => {
    const { root, aws, calls, recording } = locked();
    putFile(recording.local(root), recording.bytes);
    recordings({ root, aws, log: quiet });
    // A finding's coordinate moved: the same file count and bytes, other content.
    const answer = join(recording.tree(root), "rules/c0/no-null/cases/a0f.json");
    writeFileSync(answer, ANSWERS["rules/c0/no-null/cases/a0f.json"].replace("[1, 1, 1, 5]", "[2, 1, 1, 5]"));
    expect(recordings({ root, aws, log: quiet }).extracted).toEqual([treeLabel(root, recording)]);
    expect(extractedTree(root, recording)).toEqual(ANSWERS);
    // An answer filed under another name.
    renameSync(answer, answer.replace("a0f", "c0f"));
    expect(recordings({ root, aws, log: quiet }).extracted).toEqual([treeLabel(root, recording)]);
    expect(extractedTree(root, recording)).toEqual(ANSWERS);
    // An untouched tree is kept, its archive unread.
    expect(recordings({ root, aws, log: quiet }).extracted).toEqual([]);
    expect(calls).toEqual([]);
  });

  it("fills the recording cache it is given, the recorder's --cache, rather than the checkout's", () => {
    const { root, aws, object, recording } = locked();
    putFile(object(recording.key), recording.bytes);
    const cache = join(root, "elsewhere", "recordings");
    expect(recordings({ root, cache, aws, log: quiet }).downloaded).toEqual([recording.key]);
    expect(files(cache)).toContain(`${recording.sha256}.tar.zst`);
    expect(files(cache)).toContain(`tree/${recording.sha256}/${RECORDING_TREE_MARKER}`);
    expect(files(join(root, RECORDING_CACHE))).toEqual([]);
  });

  it("notes an archive S3 does not hold, naming its sha256, key and the command that records it, and succeeds", () => {
    const { root, aws, recording } = locked();
    const printed = [];
    expect(recordings({ root, aws, log: (line) => printed.push(line) })).toEqual({ kept: [], downloaded: [], missing: [recording.key], extracted: [] });
    for (const text of [recording.sha256, `s3://${BUCKET}/${recording.key}`, "npm run port-readiness:record -- --pack p --tool t"]) expect(printed.join("\n")).toContain(text);
    expect(files(join(root, RECORDING_CACHE))).toEqual([]);
    expect(existsSync(recording.tree(root))).toBe(false);
  });

  const corrupt = [
    ["is not zstd", () => locked(), () => Buffer.from("not zstd"), "is not a zstd archive of the size the lock allows"],
    ["holds another tar than the locked one (tampered)", () => locked(), () => compressArchive(buildArchive([{ path: "config/c0.json", bytes: Buffer.from("{}\n") }]).tar), "holds a tar whose sha256 is"],
    ["holds a file outside the tree's roots", () => locked({ ...ANSWERS, "run.sh": "rm -rf /\n" }), (recording) => recording.bytes, "e.g. run.sh"],
    ["holds two files that differ only by case", () => locked(ANSWERS, caseCollidingArchive()), (recording) => recording.bytes, "which differ only in case"],
  ];
  it.each(corrupt)("refuses a recording archive in S3 that %s, naming the lock and key, and extracts nothing", (_, make, stored, reason) => {
    const { root, aws, object, recording } = make();
    putFile(object(recording.key), stored(recording));
    const message = failure(() => recordings({ root, aws, log: quiet }));
    for (const text of [reason, "packs/p/upstream/t/recording.lock", `s3://${BUCKET}/${recording.key}`, "nothing was extracted"]) expect(message).toContain(text);
    expect(files(join(root, RECORDING_CACHE))).toEqual([]);
  });

  it("deletes a corrupt cached archive, and says to delete the Actions cache entry when S3 cannot be read", () => {
    const { root, recording } = locked();
    putFile(recording.local(root), "not zstd");
    const signedOut = () => ({ status: 255, stderr: "\nUnable to locate credentials\n" });
    const message = failure(() => recordings({ root, aws: signedOut, log: quiet }));
    expect(message).toContain(relative(root, recording.local(root)));
    expect(message).toContain("gh cache delete");
    expect(existsSync(recording.local(root))).toBe(false);
    expect(existsSync(recording.tree(root))).toBe(false);
  });

  const digest = sha256(Buffer.from("{}"));
  const oldLocks = [
    ["v2", { format: "rerail.port-readiness-recording-lock/v2", tool: "t", files: { "corpus.json": { sha256: digest, size: 2 }, "recording.json": { sha256: digest, size: 2 } } }],
    ["v3", { format: "rerail.port-readiness-recording-lock/v3", tool: "t", files: { "corpus.tar.zst": { sha256: digest, files: 1, bytes: 2 }, "recording.json": { sha256: digest, size: 2 } } }],
  ];
  it.each(oldLocks)("refuses a %s recording.lock, saying to delete it and record again, fetching nothing", (_, old) => {
    const { root, aws, calls, write } = withData();
    write("packs/p/upstream/t/recording.lock", `${JSON.stringify(old)}\n`);
    const message = failure(() => recordings({ root, aws, log: quiet }));
    for (const text of ["packs/p/upstream/t/recording.lock", `not ${RECORDING_LOCK_V4}`, "delete it and record the tool-pack again", "npm run port-readiness:record -- --pack p --tool t"]) expect(message).toContain(text);
    expect(failure(() => recordingsPush({ root, pack: "p", aws, log: quiet }))).toContain(`not ${RECORDING_LOCK_V4}`);
    expect(calls).toEqual([]);
  });

  it("refuses a v4 lock naming anything but the one archive", () => {
    const { root, aws, write } = withData();
    const entry = { sha256: sha256(Buffer.from("{}")), files: 1, bytes: 2, format: "rerail.port-readiness-recording-archive/v1" };
    write("packs/p/upstream/t/recording.lock", `${JSON.stringify({ format: RECORDING_LOCK_V4, tool: "t", files: { "recording.tar.zst": entry, "recording.json": entry } })}\n`);
    expect(failure(() => recordings({ root, aws, log: quiet }))).toContain("must name exactly recording.tar.zst");
  });

  it("--dry-run names what it would fetch, and fetches, extracts and deletes nothing", () => {
    const { root, aws, calls, recording } = locked();
    putFile(recording.local(root), "not zstd");
    const printed = [];
    expect(recordings({ root, aws, dryRun: true, log: (line) => printed.push(line) })).toEqual({ kept: [], downloaded: [], missing: [], extracted: [] });
    expect(printed.join("\n")).toContain(`would fetch s3://${BUCKET}/${recording.key}`);
    expect(printed.join("\n")).toContain(`would delete ${relative(root, recording.local(root))}`);
    expect(calls).toEqual([]);
    expect(readFileSync(recording.local(root), "utf8")).toBe("not zstd");
    expect(existsSync(recording.tree(root))).toBe(false);
  });
});

describe("recordings push", () => {
  const quiet = () => {};
  /** withData, with a recording.lock for packs/p/upstream/t. */
  function locked() {
    const context = withData();
    return { ...context, ...lockRecordings(context.write) };
  }

  it("puts the archive from the recordings cache, reads it back, and puts nothing when run again", () => {
    const { root, aws, object, called, recording } = locked();
    putFile(recording.local(root), recording.bytes);
    const printed = [];
    expect(recordingsPush({ root, pack: "p", aws, log: (line) => printed.push(line) })).toEqual({ uploaded: [recording.key], locks: ["packs/p/upstream/t/recording.lock"] });
    expect(readFileSync(object(recording.key))).toEqual(recording.bytes);
    expect(printed.at(-1)).toContain("git add -- packs/p/upstream/t/recording.lock");

    const again = [];
    expect(recordingsPush({ root, pack: "p", aws, log: (line) => again.push(line) }).uploaded).toEqual([]);
    expect(again.filter((line) => line.includes("S3 already holds"))).toHaveLength(1);
    expect(called("put-object")).toHaveLength(1);
  });

  it("refuses when neither the recordings cache nor S3 holds the archive, naming it and both remedies, before putting anything", () => {
    const { root, aws, bucket, called, recording } = locked();
    const message = failure(() => recordingsPush({ root, pack: "p", tool: "t", aws, log: quiet }));
    for (const text of [
      "packs/p/upstream/t/recording.lock",
      recording.sha256,
      `s3://${BUCKET}/${recording.key}`,
      "npm run port-readiness:record -- --pack p --tool t",
      "gh run download <run-id> -n recordings-p--t -D .rerail/port-readiness/recordings",
    ]) {
      expect(message).toContain(text);
    }
    expect(called("put-object")).toEqual([]);
    expect(files(bucket)).toEqual([]);
  });

  it("refuses a cached archive that is not the locked one, calling no aws", () => {
    const { root, aws, bucket, calls, recording } = locked();
    putFile(recording.local(root), compressArchive(buildArchive([{ path: "config/c0.json", bytes: Buffer.from("{}\n") }]).tar));
    const message = failure(() => recordingsPush({ root, pack: "p", aws, log: quiet }));
    expect(message).toContain(relative(root, recording.local(root)));
    expect(message).toContain("packs/p/upstream/t/recording.lock");
    expect(calls).toEqual([]);
    expect(files(bucket)).toEqual([]);
  });
});

describe("upload with recordings", () => {
  const quiet = () => {};
  /** withData packed, with a recording.lock for packs/p/upstream/t. */
  function packed() {
    const context = withData();
    pack({ root: context.root, log: quiet });
    return { ...context, ...lockRecordings(context.write) };
  }
  const putKeys = (called) => called("put-object").map((args) => args[args.indexOf("--key") + 1]);

  it("uploads the local recording archive under its sha256 and reads it back", () => {
    const { root, aws, object, called, recording } = packed();
    putFile(recording.local(root), recording.bytes);
    const { uploaded } = upload({ root, pack: "p", tool: "t", aws, log: quiet });
    expect(uploaded).toEqual([archiveKey(T, readLock(root, T).sha256), recording.key]);
    expect(readFileSync(object(recording.key))).toEqual(recording.bytes);
    expect(called("cp").map((args) => args[2])).toContain(`s3://${BUCKET}/${recording.key}`);
  });

  it("does not upload a recording archive S3 holds, whether or not there is a local copy", () => {
    const { root, aws, object, called, recording } = packed();
    putFile(object(recording.key), recording.bytes);
    putFile(recording.local(root), recording.bytes);
    const archive = archiveKey(T, readLock(root, T).sha256);
    expect(upload({ root, pack: "p", tool: "t", aws, log: quiet }).uploaded).toEqual([archive]);
    expect(putKeys(called)).toEqual([archive]);
  });

  it("fails on a recording archive in neither place, naming its sha256, its key and both remedies, before putting anything", () => {
    const { root, aws, bucket, called, recording } = packed();
    const message = failure(() => upload({ root, pack: "p", tool: "t", aws, log: quiet }));
    for (const text of [
      "packs/p/upstream/t/recording.lock",
      recording.sha256,
      `s3://${BUCKET}/${recording.key}`,
      "npm run port-readiness:record -- --pack p --tool t",
      "gh run download <run-id> -n recordings-p--t -D .rerail/port-readiness/recordings",
    ]) {
      expect(message).toContain(text);
    }
    expect(called("put-object")).toEqual([]);
    expect(files(bucket)).toEqual([]);
  });

  it("refuses a local recording archive that is not the locked one, calling no aws", () => {
    const { root, aws, bucket, calls, recording } = packed();
    putFile(recording.local(root), "tampered\n");
    const message = failure(() => upload({ root, pack: "p", tool: "t", aws, log: quiet }));
    expect(message).toContain("packs/p/upstream/t/recording.lock");
    expect(message).toContain(relative(root, recording.local(root)));
    expect(calls).toEqual([]);
    expect(files(bucket)).toEqual([]);
  });


  it("verifies an archive S3 holds when the archive cache lacks it, and fails naming the key when neither holds it", () => {
    const { root, aws, object, called } = published();
    rmSync(join(root, ARCHIVE_CACHE), { recursive: true });
    expect(upload({ root, aws, log: quiet }).uploaded).toEqual([]);
    expect(called("cp")).toHaveLength(3);
    const key = archiveKey(U, readLock(root, U).sha256);
    rmSync(object(key));
    const message = failure(() => upload({ root, aws, log: quiet }));
    expect(message).toContain("packs/q/upstream/u/test-data.lock");
    expect(message).toContain(`s3://${BUCKET}/${key}`);
    expect(called("put-object")).toEqual([]);
  });
});

const SAMPLE = "demo-src-a-js-0123456789ab";
/** A samples tree, as archive path → content, in the samples archive's layout (scripts/samples/format.ts). */
const SAMPLES = {
  "LICENSES.md": "# Licences\n\n- example/demo: MIT\n",
  [`${SAMPLE}/src/a.js`]: "var a = null;\n",
  [`${SAMPLE}/src/a.js.expected.json`]: '{\n  "expected": {\n    "p-no-null": [{ "line": 1 }]\n  }\n}\n',
};
const archiveOf = (content) => buildArchive(Object.entries(content).map(([path, text]) => ({ path, bytes: Buffer.from(text) })));

/**
 * Writes, with `write`, a samples.lock into `toolPack`'s directory naming `archive` (by default the canonical archive of
 * `content`) truthfully; returns it as `{ bytes, sha256, key, local(root), tree(root) }`, `bytes` compressed as stored.
 */
function lockSamples(write, toolPack = T, content = SAMPLES, archive = archiveOf(content)) {
  write(`${toolPack.directory}/${SAMPLES_LOCK}`, lockText(archive));
  return {
    bytes: compressArchive(archive.tar),
    sha256: archive.sha256,
    key: samplesKey(toolPack, archive.sha256),
    local: (root) => samplesArchiveCachePath(root, toolPack, archive.sha256),
    tree: (root) => samplesTreeDirectory(root, toolPack, archive.sha256),
  };
}

/** What a samples tree holds besides its marker, as path → content. */
const samplesIn = (directory) => Object.fromEntries(files(directory).filter((path) => path !== RECORDING_TREE_MARKER).map((path) => [path, readFileSync(join(directory, path), "utf8")]));
const shown = (root, path) => relative(root, path).split("\\").join("/");

describe("samples", () => {
  const quiet = () => {};
  /** withData, with a samples.lock for packs/p/upstream/t naming the archive of `content`. */
  function locked(content = SAMPLES) {
    const context = withData();
    return { ...context, sample: lockSamples(context.write, T, content) };
  }

  it("names the key, cache file and tree by pack, tool and the tar's sha256", () => {
    const { root, sample } = locked();
    expect(sample.key).toBe(`rerail/archives/samples/p/t/${sample.sha256}.tar.zst`);
    expect(shown(root, sample.local(root))).toBe(`.rerail/pack-data/archives/samples/p/t/${sample.sha256}.tar.zst`);
    expect(shown(root, sample.tree(root))).toBe(`.rerail/pack-data/samples/p/t/${sample.sha256}`);
    expect(readSamplesLock(root, T)).toEqual({ label: "packs/p/upstream/t/samples.lock", sha256: sample.sha256, files: 3, bytes: Object.values(SAMPLES).join("").length });
  });

  it("samplesTree fetches the archive from S3, verifies it, and extracts its tree with the marker", () => {
    const { root, aws, object, called, sample } = locked();
    putFile(object(sample.key), sample.bytes);
    expect(samplesTree({ root, pack: "p", tool: "t", aws })).toBe(sample.tree(root));
    expect(samplesIn(sample.tree(root))).toEqual(SAMPLES);
    expect(readFileSync(join(sample.tree(root), RECORDING_TREE_MARKER), "utf8")).toBe(`${sample.sha256}\n`);
    expect(readFileSync(sample.local(root))).toEqual(sample.bytes);
    expect(called("cp")).toHaveLength(1);
    // No partial file or directory is left behind.
    expect(files(join(root, ".rerail")).filter((path) => path.includes(".partial"))).toEqual([]);
  });

  it("samplesTree reuses a cached archive, then a complete tree with no archive, calling no aws", () => {
    const { root, aws, calls, sample } = locked();
    putFile(sample.local(root), sample.bytes);
    expect(samplesTree({ root, pack: "p", tool: "t", aws })).toBe(sample.tree(root));
    rmSync(sample.local(root));
    expect(samplesTree({ root, pack: "p", tool: "t", aws })).toBe(sample.tree(root));
    expect(samplesIn(sample.tree(root))).toEqual(SAMPLES);
    expect(calls).toEqual([]);
  });

  it("samplesTree extracts again a tree whose file was edited, its size kept", () => {
    const { root, aws, calls, sample } = locked();
    putFile(sample.local(root), sample.bytes);
    samplesTree({ root, pack: "p", tool: "t", aws });
    writeFileSync(join(sample.tree(root), SAMPLE, "src/a.js"), "var b = null;\n");
    samplesTree({ root, pack: "p", tool: "t", aws });
    expect(samplesIn(sample.tree(root))).toEqual(SAMPLES);
    expect(calls).toEqual([]);
  });

  it("samplesTree is undefined for a tool-pack with no samples.lock, calling no aws", () => {
    const { root, aws, calls } = withData();
    expect(samplesTree({ root, pack: "p", tool: "t", aws })).toBeUndefined();
    expect(samplesTree({ root, pack: "p", tool: "absent", aws })).toBeUndefined();
    expect(calls).toEqual([]);
  });

  it("samplesTree refuses a pack or tool that is not one path segment", () => {
    const { root, aws } = withData();
    expect(() => samplesTree({ root, pack: "..", tool: "t", aws })).toThrow('pack ".." is not a pack id');
    expect(() => samplesTree({ root, pack: "p", tool: "t/../../x", aws })).toThrow("is not a tool id");
  });

  const other = { "LICENSES.md": "other\n" };
  /** Each case: the archive the lock names, the bytes S3 holds under the lock's key, and the reason the refusal gives. */
  const corrupt = [
    ["is not zstd", () => archiveOf(SAMPLES), () => Buffer.from("not zstd"), /is not a zstd archive/u],
    ["holds another tar than the lock names", () => archiveOf(SAMPLES), () => compressArchive(archiveOf(other).tar), /holds a tar whose sha256 is [0-9a-f]{64}, not the locked/u],
    ["holds more files than a lock edited by hand says", () => ({ ...archiveOf(SAMPLES), files: 2 }), () => compressArchive(archiveOf(SAMPLES).tar), /the lock says 2 of/u],
    ["holds a file outside LICENSES.md and <sample-id>/", () => archiveOf({ ...SAMPLES, "notes.txt": "x\n" }), (archive) => compressArchive(archive.tar), /holds 1 file\(s\) that are not real-code samples [^]*notes\.txt/u],
    ["holds the tree's marker", () => archiveOf({ ...SAMPLES, [RECORDING_TREE_MARKER]: "x\n" }), (archive) => compressArchive(archive.tar), /that are not real-code samples [^]*\.tar-sha256/u],
    ["holds a sample whose id is not a sample id", () => archiveOf({ ...SAMPLES, "Demo-a-0123456789ab/a.js": "x\n" }), (archive) => compressArchive(archive.tar), /that are not real-code samples [^]*Demo-a-0123456789ab\/a\.js/u],
  ];
  it.each(corrupt)("refuses an archive that %s, naming the lock and key, and extracts and keeps nothing", (_, lockFor, stored, reason) => {
    const { root, aws, object, write } = withData();
    const archive = lockFor();
    const sample = lockSamples(write, T, undefined, archive);
    putFile(object(sample.key), stored(archive));
    const message = failure(() => samplesTree({ root, pack: "p", tool: "t", aws }));
    expect(message).toMatch(reason);
    expect(message).toContain(`packs/p/upstream/t/samples.lock: s3://${BUCKET}/${sample.key}`);
    expect(message).toContain("nothing was extracted");
    expect(files(join(root, ".rerail"))).toEqual([]);
  });

  it("checks sample ids as scripts/samples/format.ts's SAMPLE_ID does", () => {
    expect(SAMPLE_ID.test(SAMPLE)).toBe(true);
    expect(SAMPLE_ID.test("Demo-a-0123456789ab")).toBe(false);
    expect(SAMPLE_ID.test(RECORDING_TREE_MARKER)).toBe(false);
  });

  it("deletes a cached archive that is not the locked one and fetches the locked one from S3", () => {
    const { root, aws, object, called, sample } = locked();
    putFile(sample.local(root), "not zstd");
    putFile(object(sample.key), sample.bytes);
    samplesTree({ root, pack: "p", tool: "t", aws });
    expect(readFileSync(sample.local(root))).toEqual(sample.bytes);
    expect(called("cp")).toHaveLength(1);
  });

  it("fails naming the key and the push command when S3 does not hold the archive", () => {
    const { root, aws, sample } = locked();
    const message = failure(() => samplesTree({ root, pack: "p", tool: "t", aws }));
    expect(message).toBe(`packs/p/upstream/t/samples.lock names s3://${BUCKET}/${sample.key}, which S3 does not hold: a maintainer puts the built samples archive there with \`node scripts/s3/pack-fixtures.mjs samples push --pack p --tool t\``);
    expect(existsSync(sample.tree(root))).toBe(false);
  });

  it("the samples command fetches and extracts every selected lock, skipping tool-packs without one", () => {
    const { root, aws, object, write } = withData();
    const t = lockSamples(write, T);
    const u = lockSamples(write, U, { ...SAMPLES, "LICENSES.md": "u\n" });
    putFile(t.local(root), t.bytes);
    putFile(object(u.key), u.bytes);
    expect(samples({ root, aws, log: quiet })).toEqual({ kept: [t.key], downloaded: [u.key], extracted: [shown(root, t.tree(root)), shown(root, u.tree(root))] });
    expect(samplesIn(u.tree(root))).toEqual({ ...SAMPLES, "LICENSES.md": "u\n" });
    // A second run keeps both archives and trees.
    expect(samples({ root, aws, log: quiet })).toEqual({ kept: [t.key, u.key], downloaded: [], extracted: [] });
    expect(samples({ root, pack: "q", aws, log: quiet })).toEqual({ kept: [u.key], downloaded: [], extracted: [] });
  });

  it("the samples command's --dry-run names what it would fetch, and fetches, extracts and deletes nothing", () => {
    const { root, aws, calls, sample } = locked();
    putFile(sample.local(root), "not zstd");
    const printed = [];
    expect(samples({ root, aws, dryRun: true, log: (line) => printed.push(line) })).toEqual({ kept: [], downloaded: [], extracted: [] });
    expect(printed.join("\n")).toContain(`would delete ${shown(root, sample.local(root))}, which is not the archive packs/p/upstream/t/samples.lock names, and would fetch s3://${BUCKET}/${sample.key}`);
    expect(readFileSync(sample.local(root), "utf8")).toBe("not zstd");
    expect(existsSync(sample.tree(root))).toBe(false);
    expect(calls).toEqual([]);
  });

  it("pull brings each selected tool-pack's samples with its test data", () => {
    const { root, aws, object, write, data } = published();
    const sample = lockSamples(write, T);
    putFile(object(sample.key), sample.bytes);
    fresh(root);
    const result = pull({ root, aws, log: quiet });
    expect(tree(root, true)).toEqual(data);
    expect(result.samples).toEqual([{ ...T, source: "S3" }]);
    expect(samplesIn(sample.tree(root))).toEqual(SAMPLES);
    expect(readFileSync(sample.local(root))).toEqual(sample.bytes);
    expect(pull({ root, aws, pack: "p", tool: "t", log: quiet }).samples).toEqual([{ ...T, source: "cache" }]);
  });

  it("pull refuses a corrupt samples archive before extracting any test data", () => {
    const { root, aws, object, write } = published();
    const sample = lockSamples(write, T);
    putFile(object(sample.key), "not zstd");
    fresh(root);
    expect(failure(() => pull({ root, aws, log: quiet }))).toMatch(/samples\.lock: [^]*is not a zstd archive/u);
    expect(tree(root, true)).toEqual({});
    expect(existsSync(sample.tree(root))).toBe(false);
  });
});

describe("samples push", () => {
  const quiet = () => {};

  it("puts the cached archive, reads it back, prints the git add, and puts nothing when run again", () => {
    const { root, aws, object, called, write } = withData();
    const sample = lockSamples(write, T);
    putFile(sample.local(root), sample.bytes);
    const printed = [];
    expect(samplesPush({ root, aws, log: (line) => printed.push(line) })).toEqual({ uploaded: [sample.key], locks: ["packs/p/upstream/t/samples.lock"] });
    expect(readFileSync(object(sample.key))).toEqual(sample.bytes);
    expect(printed.at(-1)).toBe("S3 holds the samples archive(s) the lock(s) name; commit them:\n  git add -- packs/p/upstream/t/samples.lock");
    expect(called("put-object")).toHaveLength(1);
    expect(samplesPush({ root, aws, log: quiet })).toEqual({ uploaded: [], locks: ["packs/p/upstream/t/samples.lock"] });
    expect(called("put-object")).toHaveLength(1);
    expect(called("cp")).toHaveLength(2);
  });

  it("refuses a cached archive that is not the locked one, calling no aws", () => {
    const { root, aws, bucket, calls, write } = withData();
    const sample = lockSamples(write, T);
    putFile(sample.local(root), compressArchive(archiveOf({ "LICENSES.md": "other\n" }).tar));
    const message = failure(() => samplesPush({ root, aws, log: quiet }));
    expect(message).toContain(`${shown(root, sample.local(root))} is not the samples archive packs/p/upstream/t/samples.lock names`);
    expect(message).toContain("Nothing was uploaded");
    expect(calls).toEqual([]);
    expect(files(bucket)).toEqual([]);
  });

  it("refuses an archive in neither the cache nor S3, before putting anything", () => {
    const { root, aws, called, write } = withData();
    const sample = lockSamples(write, T);
    const message = failure(() => samplesPush({ root, aws, log: quiet }));
    expect(message).toContain(`names samples archive ${sample.sha256}, which is in neither`);
    expect(message).toContain(`s3://${BUCKET}/${sample.key}`);
    expect(called("put-object")).toEqual([]);
  });

  it("refuses a selection with no samples.lock, and --dry-run", () => {
    const { root, aws } = withData();
    expect(() => samplesPush({ root, aws, log: quiet })).toThrow("no selected tool-pack has a samples.lock");
    expect(() => samplesPush({ root, aws, dryRun: true, log: quiet })).toThrow("samples push has no --dry-run");
  });
});

describe("archiveCache with samples", () => {
  it("keys one tool-pack's entry by its test data and samples sha256s, and lists both archives", () => {
    const { root, write } = published();
    const sample = lockSamples(write, T);
    const { sha256: digest } = readLock(root, T);
    expect(archiveCache({ root, pack: "p", tool: "t" })).toEqual({
      key: `pack-test-data-p-t-${digest}-samples-${sample.sha256}`,
      paths: [`${ARCHIVE_CACHE}/p/t/${digest}.tar.zst`, `${ARCHIVE_CACHE}/samples/p/t/${sample.sha256}.tar.zst`],
    });
  });

  it("changes the key for several tool-packs when a samples.lock is added or changes, and lists its archive", () => {
    const { root, write } = published();
    const before = archiveCache({ root });
    const first = lockSamples(write, U);
    const added = archiveCache({ root });
    const second = lockSamples(write, U, { ...SAMPLES, "LICENSES.md": "changed\n" });
    const changed = archiveCache({ root });
    expect(new Set([before.key, added.key, changed.key]).size).toBe(3);
    expect(added.paths).toEqual([...before.paths, `${ARCHIVE_CACHE}/samples/q/u/${first.sha256}.tar.zst`]);
    expect(changed.paths).toContain(`${ARCHIVE_CACHE}/samples/q/u/${second.sha256}.tar.zst`);
  });
});

const SCRIPTS = fileURLToPath(new URL("../../scripts/s3/", import.meta.url));

/** setUp's fake `aws` as an executable, serving the bucket directory FAKE_BUCKET names. */
const FAKE_AWS = `#!/usr/bin/env node
const { copyFileSync, existsSync, mkdirSync } = require("node:fs");
const { dirname, join } = require("node:path");
const args = process.argv.slice(2);
const prefix = "s3://${BUCKET}/";
const object = (key) => join(process.env.FAKE_BUCKET, key);
const option = (name) => args[args.indexOf(name) + 1];
const exit = (status, stdout, stderr = "") => {
  process.stdout.write(stdout);
  process.stderr.write(stderr);
  process.exit(status);
};
if (args[0] === "sts") exit(0, "{}");
if (args[0] === "s3" && args[1] === "cp" && args[2].startsWith(prefix)) {
  const source = object(args[2].slice(prefix.length));
  if (!existsSync(source)) exit(1, "", "fatal error: An error occurred (404) when calling the HeadObject operation: Key does not exist\\n");
  copyFileSync(source, args[3]);
  exit(0, "");
}
if (args[1] === "head-object") exit(existsSync(object(option("--key"))) ? 0 : 254, "{}", "An error occurred (404) when calling the HeadObject operation: Not Found\\n");
if (args[1] === "put-object" && option("--if-none-match") === "*") {
  const target = object(option("--key"));
  if (existsSync(target)) exit(254, "", "An error occurred (PreconditionFailed) when calling the PutObject operation\\n");
  mkdirSync(dirname(target), { recursive: true });
  copyFileSync(option("--body"), target);
  exit(0, "{}");
}
exit(2, "", "the fake aws does not implement: aws " + args.join(" ") + "\\n");
`;

/**
 * published, with this repository's pack-fixtures scripts committed on main, and a pull request branch `feature`
 * checked out in the worktree `pr`. The branch tracks a changed and a new file of packs/p/upstream/t and a new one of
 * packs/q/upstream/u, and carries code that writes `marker` if anything runs it: package.json scripts, a tool's
 * generator, and its own scripts/s3/pack-fixtures.mjs.
 */
function pullRequest() {
  const context = published();
  const { root } = context;
  const base = dirname(root);
  for (const name of ["pack-fixtures.mjs", "test-data-archive.mjs"]) putFile(join(root, "scripts/s3", name), readFileSync(join(SCRIPTS, name)));
  git(root, "add", "scripts");
  git(root, "commit", "-qm", "scripts");
  const pr = join(base, "pr");
  git(root, "worktree", "add", "-q", "-b", "feature", pr);
  const marker = join(base, "marker");
  const changes = {
    "packs/p/upstream/t/fixtures/rerail/a.py": "changed\n",
    "packs/p/upstream/t/fixtures/rerail/new.py": "new\n",
    "packs/q/upstream/u/authored/corpus/d.py": "d\n",
  };
  for (const [path, text] of Object.entries(changes)) putFile(join(pr, path), text);
  git(pr, "add", "-f", "--", ...Object.keys(changes));
  putFile(join(pr, "package.json"), `${JSON.stringify({ scripts: { preinstall: `touch ${marker}`, "fixtures:push": `touch ${marker}` } }, null, 2)}\n`);
  putFile(join(pr, "packs/p/upstream/t/generate.py"), `open(${JSON.stringify(marker)}, "w").write("ran")\n`);
  putFile(join(pr, "scripts/s3/pack-fixtures.mjs"), `import { writeFileSync } from "node:fs";\nwriteFileSync(${JSON.stringify(marker)}, "ran\\n");\n`);
  git(pr, "add", "--", "package.json", "packs/p/upstream/t/generate.py", "scripts/s3/pack-fixtures.mjs");
  git(pr, "commit", "-qm", "pull request");
  return { ...context, pr, marker, changes };
}

/** What `toolPack`'s archive should hold after push --from: the published data overlaid with the pull request's. */
const overlaid = (data, changes, toolPack) =>
  Object.fromEntries(
    Object.entries({ ...data, ...changes })
      .filter(([path]) => path.startsWith(`${toolPack.directory}/`))
      .map(([path, text]) => [path.slice(toolPack.directory.length + 1), text]),
  );

/** What S3 holds for the archive `toolPack`'s lock in the checkout at `root` names. */
const archived = (object, root, toolPack) =>
  Object.fromEntries(
    readArchive(decompressArchive(readFileSync(object(archiveKey(toolPack, readLock(root, toolPack).sha256))))).map(({ path, bytes }) => [path, bytes.toString("utf8")]),
  );

/** The pull request's index after its printed commands ran: the two locks rewritten and its pack test data untracked. */
const STAGED = [
  ["D", "packs/p/upstream/t/fixtures/rerail/a.py"],
  ["D", "packs/p/upstream/t/fixtures/rerail/new.py"],
  ["M", "packs/p/upstream/t/test-data.lock"],
  ["D", "packs/q/upstream/u/authored/corpus/d.py"],
  ["M", "packs/q/upstream/u/test-data.lock"],
];
const staged = (root) => lines(git(root, "diff", "--cached", "--name-status", "--no-renames")).map((line) => line.split("\t"));

describe("push --from", () => {
  it("archives a pull request's tracked data from its commit's blobs alone, running none of its code", () => {
    const { root, pr, bucket, object, marker, data, changes } = pullRequest();
    const bin = join(dirname(root), "bin");
    putFile(join(bin, "aws"), FAKE_AWS);
    chmodSync(join(bin, "aws"), 0o755);
    const head = git(root, "rev-parse", "HEAD");
    const run = spawnSync(process.execPath, ["scripts/s3/pack-fixtures.mjs", "push", "--from", "feature"], {
      cwd: root,
      encoding: "utf8",
      env: { ...process.env, PATH: `${bin}${delimiter}${process.env.PATH}`, FAKE_BUCKET: bucket },
    });
    expect(run.stderr).toBe("");
    expect(run.status).toBe(0);

    // The trusted checkout is as it was: still on main, nothing of the branch on disk, nothing run.
    expect(existsSync(marker)).toBe(false);
    expect(git(root, "rev-parse", "HEAD")).toBe(head);
    expect(git(root, "status", "--porcelain")).toBe("");
    expect(existsSync(join(root, "packs/p/upstream/t/generate.py"))).toBe(false);
    expect(existsSync(join(root, "packs/p/upstream/t/fixtures/rerail/new.py"))).toBe(false);
    expect(readFileSync(join(root, "packs/p/upstream/t/fixtures/rerail/a.py"), "utf8")).toBe("authored\n");

    const script = run.stdout.split(/run in a checkout of [^\n]*:\n/u).at(-1);
    expect(script).toMatch(/^cat > packs\/p\/upstream\/t\/test-data\.lock <<'EOF'\n\{\n {2}"format": 1,/u);
    expect(script).not.toMatch(/\bnpm\b|\bnode\b/u);
    execFileSync("sh", ["-c", script], { cwd: pr });
    expect(staged(pr)).toEqual(STAGED);
    expect(lines(git(pr, "ls-files", "packs")).filter(isPackFixture)).toEqual([]);
    for (const path of Object.keys(changes)) expect(existsSync(join(pr, path))).toBe(true);
    for (const toolPack of [T, U]) expect(archived(object, pr, toolPack)).toEqual(overlaid(data, changes, toolPack));
    expect(existsSync(marker)).toBe(false);
  });

  it("--into writes the new locks into a checkout of the commit, prints only git commands, and uploads its recordings", () => {
    const { root, pr, aws, object, data, changes } = pullRequest();
    const { named } = lockRecordings((path, content) => putFile(join(pr, path), content));
    git(pr, "add", "--", "packs/p/upstream/t/recording.lock");
    git(pr, "commit", "-qm", "recording");
    for (const each of named) putFile(each.local(root), each.bytes);
    const printed = [];
    const result = push({ root, aws, from: "feature", into: pr, log: (line) => printed.push(line) });

    expect(result.changedLocks).toEqual(["packs/p/upstream/t/test-data.lock", "packs/q/upstream/u/test-data.lock"]);
    expect(result.script).toBe("git add -- packs/p/upstream/t/test-data.lock packs/q/upstream/u/test-data.lock\ngit rm -r -q --cached -- packs/p/upstream/t/fixtures packs/q/upstream/u/authored");
    expect(printed.at(-1).endsWith(`:\n${result.script}`)).toBe(true);
    expect(result.uploaded).toEqual([...[T, U].map((toolPack) => archiveKey(toolPack, readLock(pr, toolPack).sha256)), ...keys(named)]);
    for (const each of named) expect(readFileSync(object(each.key))).toEqual(each.bytes);
    for (const toolPack of [T, U]) expect(archived(object, pr, toolPack)).toEqual(overlaid(data, changes, toolPack));
    execFileSync("sh", ["-c", result.script], { cwd: pr });
    expect(staged(pr)).toEqual(STAGED);
  });

  it("--into refuses a directory that is no checkout of the commit, before uploading anything", () => {
    const { root, aws, bucket, called } = pullRequest();
    const held = files(bucket);
    const message = failure(() => push({ root, aws, from: "feature", into: root, log: () => {} }));
    expect(message).toContain("is no checkout of feature");
    expect(called("put-object")).toEqual([]);
    expect(files(bucket)).toEqual(held);
  });

  /** A branch `name` off the checkout's HEAD, in a worktree beside it, committing `edits` (path → text) with `git add -f`. */
  function branch(root, name, edits) {
    const worktree = join(dirname(root), name);
    git(root, "worktree", "add", "-q", "-b", name, worktree);
    for (const [path, text] of Object.entries(edits)) putFile(join(worktree, path), text);
    git(worktree, "add", "-f", "--", ...Object.keys(edits));
    git(worktree, "commit", "-qm", name);
    return worktree;
  }

  /** push --from `from` with `options`, its printed script run in `worktree`; the result and what it logged. */
  function pushInto(context, worktree, from, options = {}) {
    const printed = [];
    const result = push({ root: context.root, aws: context.aws, from, ...options, log: (line) => printed.push(line) });
    execFileSync("sh", ["-c", result.script], { cwd: worktree });
    return { result, printed: printed.join("\n") };
  }

  it("drops the files a removed UPSTREAM.md row listed, naming them, and keeps the rest of the archive", () => {
    const context = published();
    const pr = branch(context.root, "unlist", { "packs/p/upstream/t/UPSTREAM.md": record([]) });
    const { result, printed } = pushInto(context, pr, "unlist");
    expect(result.changedLocks).toEqual(["packs/p/upstream/t/test-data.lock"]);
    expect(result.dropped).toEqual(["packs/p/upstream/t/fixtures/rerail/a.py"]);
    expect(printed).toMatch(/packs\/p\/upstream\/t: archive [0-9a-f]{12} drops 1 file\(s\) of archive [0-9a-f]{12} \(the UPSTREAM\.md at unlist no longer lists them as Rerail's\):\n {2}fixtures\/rerail\/a\.py/u);
    expect(archived(context.object, pr, T)).toEqual({ "fixtures/upstream/b.py": "derived\n", "x y.expected.json": "{}\n" });
  });

  it("keeps the archive's files the commit does not track", () => {
    const context = published();
    const changes = { "packs/p/upstream/t/fixtures/rerail/a.py": "changed\n" };
    const pr = branch(context.root, "overlay", changes);
    const { result, printed } = pushInto(context, pr, "overlay");
    expect(result.dropped).toEqual([]);
    expect(printed).not.toContain("drops");
    expect(archived(context.object, pr, T)).toEqual(overlaid(context.data, changes, T));
  });

  it("--replace archives the commit's tracked data alone, dropping and naming every file of the archive it does not track", () => {
    const context = published();
    const pr = branch(context.root, "replace", { "packs/p/upstream/t/fixtures/rerail/a.py": "changed\n" });
    const { result, printed } = pushInto(context, pr, "replace", { replace: true });
    expect(result.changedLocks).toEqual(["packs/p/upstream/t/test-data.lock"]);
    expect(result.dropped).toEqual(["packs/p/upstream/t/fixtures/upstream/b.py", "packs/p/upstream/t/x y.expected.json"]);
    expect(printed).toContain("(replace does not track them):\n  fixtures/upstream/b.py\n  x y.expected.json");
    expect(archived(context.object, pr, T)).toEqual({ "fixtures/rerail/a.py": "changed\n" });
  });

  it("--replace refuses a selected tool-pack that tracks no pack test data, uploading nothing", () => {
    const context = published();
    branch(context.root, "untracked", { "packs/p/upstream/t/UPSTREAM.md": record([]) });
    const message = failure(() => push({ root: context.root, aws: context.aws, from: "untracked", replace: true, log: () => {} }));
    expect(message).toContain("--replace: packs/p/upstream/t tracks no pack test data");
    expect(context.called("put-object")).toEqual([]);
  });

  it("selects a tool-pack whose only change is its samples.lock and uploads the samples archive it names", () => {
    const context = published();
    const archive = archiveOf(SAMPLES);
    branch(context.root, "samples", { [`packs/p/upstream/t/${SAMPLES_LOCK}`]: lockText(archive) });
    putFile(samplesArchiveCachePath(context.root, T, archive.sha256), compressArchive(archive.tar));
    const result = push({ root: context.root, aws: context.aws, from: "samples", log: () => {} });
    expect(result.uploaded).toEqual([samplesKey(T, archive.sha256)]);
    expect(result.changedLocks).toEqual([]);
    expect(readFileSync(context.object(samplesKey(T, archive.sha256)))).toEqual(compressArchive(archive.tar));
  });
});

describe("check", () => {
  /** published, with a recording.lock for packs/p/upstream/t and a samples.lock for packs/q/upstream/u whose archives S3 holds. */
  function locked() {
    const context = published();
    const { named } = lockRecordings(context.write);
    for (const recording of named) putFile(context.object(recording.key), recording.bytes);
    const sample = lockSamples(context.write, U);
    putFile(context.object(sample.key), sample.bytes);
    return { ...context, named, sample };
  }
  const archiveKeys = (root) => [T, { pack: "p", tool: "w", directory: "packs/p/upstream/w" }, U].map((toolPack) => archiveKey(toolPack, readLock(root, toolPack).sha256));

  it("passes when S3 holds every archive, recording and samples archive the locks name, asking head-object alone", () => {
    const { root, aws, calls, named, sample } = locked();
    expect(check({ root, aws, log: () => {} }).checked).toEqual([...archiveKeys(root), ...keys(named), sample.key].sort());
    expect(calls.filter((args) => args[0] !== "sts" && args[1] !== "head-object")).toEqual([]);
  });

  it("fails listing each missing archive with its lock and remedy, downloading nothing", () => {
    const { root, aws, object, calls, named, sample } = locked();
    const [recording] = named;
    const archive = archiveKey(U, readLock(root, U).sha256);
    rmSync(object(archive));
    rmSync(object(recording.key));
    rmSync(object(sample.key));
    const message = failure(() => check({ root, aws, log: () => {} }));
    expect(message.split("\n")).toEqual([
      'S3 lacks 3 of the 5 object(s) the locks name (CONTRIBUTING.md, "Pack test data"):',
      `  packs/q/upstream/u/test-data.lock names s3://${BUCKET}/${archive}, which S3 does not hold: a maintainer archives the pull request's pack test data with \`npm run fixtures:push -- --from <ref> --pack q --tool u\``,
      `  packs/q/upstream/u/samples.lock names s3://${BUCKET}/${sample.key}, which S3 does not hold: a maintainer must re-pack the set from a script with packSamples and writeSampleSet (scripts/samples/pack.ts), which write the archive into .rerail/pack-data/archives/samples/ and the lock and SAMPLES.md, then put the archive there with \`node scripts/s3/pack-fixtures.mjs samples push --pack q --tool u\``,
      `  packs/p/upstream/t/recording.lock names recording.tar.zst at s3://${BUCKET}/${recording.key}, which S3 does not hold: a maintainer downloads the pull request's CI recording with \`gh run download <run> -n recordings-p--t -D .rerail/port-readiness/recordings\`, then puts it in S3 with \`npm run fixtures:push -- --from <ref> --pack p --tool t\``,
    ]);
    expect(calls.filter((args) => args[0] !== "sts" && args[1] !== "head-object")).toEqual([]);
  });

  it("exits 0 when S3 holds everything and 1 when it lacks anything, as a command", () => {
    const { root, bucket, object, named } = locked();
    const bin = join(dirname(root), "bin");
    putFile(join(bin, "aws"), FAKE_AWS);
    chmodSync(join(bin, "aws"), 0o755);
    for (const name of ["pack-fixtures.mjs", "test-data-archive.mjs"]) putFile(join(root, "scripts/s3", name), readFileSync(join(SCRIPTS, name)));
    const run = () => spawnSync(process.execPath, ["scripts/s3/pack-fixtures.mjs", "check"], {
      cwd: root,
      encoding: "utf8",
      env: { ...process.env, PATH: `${bin}${delimiter}${process.env.PATH}`, FAKE_BUCKET: bucket },
    });
    const passed = run();
    expect(passed.stderr).toBe("");
    expect(passed.stdout).toBe("S3 holds every object the locks name: 3 archive(s), 1 recording archive(s) and 1 samples archive(s)\n");
    expect(passed.status).toBe(0);
    rmSync(object(named[0].key));
    const failed = run();
    expect(failed.status).toBe(1);
    expect(failed.stderr).toContain(`fixtures:check: S3 lacks 1 of the 5 object(s)`);
    expect(failed.stderr).toContain(`s3://${BUCKET}/${named[0].key}`);
  });
});

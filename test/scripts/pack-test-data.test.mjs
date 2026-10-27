// scripts/s3/pack-fixtures.mjs: which paths are pack test data and which of it is authored, and pull (with a tool-pack's
// publish/ laid over it), stage, push, pack (from the data on disk, or a pull request's publish/ from its commit), upload
// (and the publish workflow's upload --objects and confirm), samples, check and the CI cache entry against a temporary
// git checkout with a fake `aws` whose bucket is a local directory.

import { execFileSync, spawnSync } from "node:child_process";
import { chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, renameSync, rmSync, statSync, symlinkSync, utimesSync, writeFileSync } from "node:fs";
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
  confirm,
  isPackFixture,
  licenceAllowed,
  objectTagging,
  PACK_FIXTURE_PATTERNS,
  pack,
  packFixtureKind,
  packFixturesWritten,
  packFrom,
  publishPath,
  pull,
  push,
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
  stage,
  TREE_MARKER,
  upload,
  uploadPublished,
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
  /** The `--tagging` of each key put. */
  const tagging = new Map();
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
      return { status: 0, stdout: JSON.stringify({ ContentLength: statSync(object(option("--key"))).size }) };
    }
    if (args[0] === "s3api" && args[1] === "put-object" && option("--bucket") === BUCKET && option("--if-none-match") === "*") {
      const target = object(option("--key"));
      if (existsSync(target)) {
        return { status: 254, stderr: "\nAn error occurred (PreconditionFailed) when calling the PutObject operation: At least one of the pre-conditions you specified did not hold\n" };
      }
      mkdirSync(dirname(target), { recursive: true });
      copyFileSync(option("--body"), target);
      tagging.set(option("--key"), option("--tagging"));
      return { status: 0, stdout: "{}" };
    }
    throw new Error(`the fake aws does not implement: aws ${args.join(" ")}`);
  };
  /** The calls of one kind, e.g. "put-object" or "cp". */
  const called = (command) => calls.filter((args) => args[1] === command);
  return { root, bucket, object, aws, calls, called, write, tagging };
}

/** Every upload's tags, as a maintainer's push gives them. */
const TAGS = { commit: "0".repeat(40), pr: "none", runId: "local", uploader: "maintainer" };

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
  push({ root: context.root, aws: context.aws, tags: TAGS, log: () => {} });
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
      "packs/javascript/upstream/eslint/provenance.json",
      "packs/javascript/upstream/eslint/LICENSES.md",
      "packs/next/upstream/eslint-plugin-next/fixtures/pages/[id].tsx",
      "packs/node/upstream/eslint-plugin-n/authored/corpus/src/a.js",
    ];
    const kept = [
      "packs/python/upstream/ruff/gates.yaml",
      "packs/python/upstream/ruff/generate.py",
      "packs/python/upstream/ruff/UPSTREAM.md",
      "packs/python/upstream/ruff/test-data.lock",
      "packs/php/upstream/phpcs/ruleset.xml",
      "packs/blade/upstream/bladestan/composer-laravel.lock",
      "packs/python/upstream/common.py",
      "packs/python/upstream/provenance.json",
      "packs/python/licenses/ruff/LICENSES.md",
      "packs/python/upstream/fixtures.expected.json",
      "packs/python/rules/a.json",
      "fixtures/packs/valid/upstream/tool/fixtures.expected.json",
    ];
    expect(data.filter((path) => !isPackFixture(path))).toEqual([]);
    expect(kept.filter((path) => isPackFixture(path))).toEqual([]);
  });

  it("never matches a publish/ path, so pulled data and a publish request never mix", () => {
    const publish = [
      "packs/php/upstream/phpcs/publish/fixtures/Generic/x.inc",
      "packs/node/upstream/eslint-plugin-n/publish/fixtures.expected.json",
      "packs/node/upstream/eslint-plugin-n/publish/authored/corpus/a.js",
      "packs/sql/upstream/sqlfluff/publish/samples/s-0123456789ab/a.sql",
      "packs/sql/upstream/sqlfluff/publish/gates.yaml",
    ];
    expect(publish.filter((path) => isPackFixture(path))).toEqual([]);
    expect(publish.map((path) => publishPath(path)?.path)).toEqual(["fixtures/Generic/x.inc", "fixtures.expected.json", "authored/corpus/a.js", "samples/s-0123456789ab/a.sql", "gates.yaml"]);
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

  it("tells a contributor to stage what a generator wrote into its tool-pack's publish/, once per tool-pack", () => {
    const { root, write } = setUp();
    write("packs/a/upstream/t/fixtures/p [x].js", "mine\n");
    write("packs/a/upstream/t/fixtures/upstream.js", "theirs\n");
    write("packs/a/upstream/t/fixtures.expected.json", "{}\n");
    const message = packFixturesWritten([join(root, "packs/a/upstream/t/fixtures"), join(root, "packs/a/upstream/t/fixtures.expected.json")], root);
    expect(message.split("\n")).toHaveLength(1);
    expect(message).toContain("Wrote 3 pack test data file(s) under packs/a/upstream/t/");
    expect(message).toContain("`npm run fixtures:stage -- --pack a --tool t`");
    expect(message).not.toContain("git add -f");
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

  it("lays the tool-pack's publish/ over the archive, overwriting and adding files, and leaves publish/samples/ out", () => {
    const { root, aws, write, data } = published();
    write("packs/p/upstream/t/publish/fixtures/rerail/a.py", "published\n");
    write("packs/p/upstream/t/publish/fixtures/rerail/new.py", "new\n");
    write("packs/p/upstream/t/publish/samples/s-0123456789ab/a.py", "sample\n");
    write("packs/p/upstream/t/publish/gates.yaml", "not data\n");
    const printed = [];
    const result = pull({ root, aws, log: (line) => printed.push(line) });
    expect(result.toolPacks.map(({ directory, published: count }) => [directory, count])).toEqual([
      ["packs/p/upstream/t", 2],
      ["packs/p/upstream/w", 0],
      ["packs/q/upstream/u", 0],
    ]);
    expect(tree(root, true)).toEqual({
      ...data,
      "packs/p/upstream/t/fixtures/rerail/a.py": "published\n",
      "packs/p/upstream/t/fixtures/rerail/new.py": "new\n",
    });
    expect(existsSync(join(root, "packs/p/upstream/t/s-0123456789ab"))).toBe(false);
    expect(printed.join("\n")).toContain("left out 1 publish/ file(s) that are no pack test data, which publishing refuses: gates.yaml");
  });

  it("takes a tool-pack with no lock yet from its publish/ alone", () => {
    const { root, aws, write } = published();
    write("packs/n/upstream/v/gates.yaml", "gates\n");
    write("packs/n/upstream/v/publish/cases.json", "[1]\n");
    const result = pull({ root, aws, pack: "n", log: () => {} });
    expect(result.toolPacks).toEqual([{ pack: "n", tool: "v", directory: "packs/n/upstream/v", source: "publish/", published: 1 }]);
    expect(readFileSync(join(root, "packs/n/upstream/v/cases.json"), "utf8")).toBe("[1]\n");
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
  it("archives each tool-pack, uploads the archive under its sha256, tagged, and writes the lock", () => {
    const { root, aws, object, data, tagging } = withData();
    const result = push({ root, aws, tags: TAGS, log: () => {} });
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
    for (const key of result.uploaded) expect(tagging.get(key)).toBe(`rerail%3Acommit=${"0".repeat(40)}&rerail%3Apr=none&rerail%3Arun-id=local&rerail%3Auploader=maintainer`);
  });

  it("uploads nothing and leaves the locks byte-identical when the data is unchanged", () => {
    const { root, aws, called } = published();
    const locks = LOCKS.map((lock) => readFileSync(join(root, lock)));
    expect(push({ root, aws, tags: TAGS, log: () => {} })).toMatchObject({ changedLocks: [], uploaded: [] });
    expect(called("put-object")).toEqual([]);
    expect(LOCKS.map((lock) => readFileSync(join(root, lock)))).toEqual(locks);
  });

  it("refuses to upload untagged, before packing anything", () => {
    const { root, aws, calls } = withData();
    expect(() => push({ root, aws, tags: { ...TAGS, uploader: "" }, log: () => {} })).toThrow('rerail:uploader ""');
    expect(() => push({ root, aws, log: () => {} })).toThrow("rerail:commit undefined");
    expect(calls).toEqual([]);
    expect(LOCKS.filter((lock) => existsSync(join(root, lock)))).toEqual([]);
  });
});

describe("objectTagging", () => {
  it("URL-encodes the four rerail: tags, and refuses a value S3 cannot carry", () => {
    expect(objectTagging({ commit: "a".repeat(40), pr: "177", runId: "123", uploader: "jeffreysnell" })).toBe(
      `rerail%3Acommit=${"a".repeat(40)}&rerail%3Apr=177&rerail%3Arun-id=123&rerail%3Auploader=jeffreysnell`,
    );
    expect(() => objectTagging({ commit: "a", pr: "1", runId: "1", uploader: "bot[bot]" })).toThrow('rerail:uploader "bot[bot]"');
  });
});

describe("upload", () => {
  const other = () => compressArchive(buildArchive([{ path: "cases.json", bytes: Buffer.from("other\n") }]).tar);

  it("refuses a cached archive that does not match its lock", () => {
    const { root, aws, bucket, called } = withData();
    pack({ root, log: () => {} });
    const cached = join(root, ARCHIVE_CACHE, "p/t", `${readLock(root, T).sha256}.tar.zst`);
    writeFileSync(cached, other());
    expect(() => upload({ root, aws, tags: TAGS, log: () => {} })).toThrow("packs/p/upstream/t/test-data.lock");
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
    expect(() => upload({ root, pack: "p", tool: "t", aws, tags: TAGS, log: () => {} })).toThrow("packs/p/upstream/t/test-data.lock");
    expect(called("put-object")).toEqual([]);
    // S3 says the key is absent, then holds it by the time the put lands: the conditional put is refused.
    const racing = (args) => (args[1] === "head-object" ? { status: 254, stderr: "An error occurred (404) when calling the HeadObject operation: Not Found\n" } : aws(args));
    expect(() => upload({ root, pack: "p", tool: "t", aws: racing, tags: TAGS, log: () => {} })).toThrow("packs/p/upstream/t/test-data.lock");
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
    const message = failure(() => upload({ root, aws, tags: TAGS, log: () => {} }));
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

describe("upload without a local archive", () => {
  const quiet = () => {};

  it("verifies an archive S3 holds when the archive cache lacks it, and fails naming the key when neither holds it", () => {
    const { root, aws, object, called } = published();
    rmSync(join(root, ARCHIVE_CACHE), { recursive: true });
    expect(upload({ root, aws, tags: TAGS, log: quiet }).uploaded).toEqual([]);
    expect(called("cp")).toHaveLength(3);
    const key = archiveKey(U, readLock(root, U).sha256);
    rmSync(object(key));
    const message = failure(() => upload({ root, aws, tags: TAGS, log: quiet }));
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
const samplesIn = (directory) => Object.fromEntries(files(directory).filter((path) => path !== TREE_MARKER).map((path) => [path, readFileSync(join(directory, path), "utf8")]));
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
    expect(readFileSync(join(sample.tree(root), TREE_MARKER), "utf8")).toBe(`${sample.sha256}\n`);
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
    ["holds the tree's marker", () => archiveOf({ ...SAMPLES, [TREE_MARKER]: "x\n" }), (archive) => compressArchive(archive.tar), /that are not real-code samples [^]*\.tar-sha256/u],
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
    expect(SAMPLE_ID.test(TREE_MARKER)).toBe(false);
  });

  it("deletes a cached archive that is not the locked one and fetches the locked one from S3", () => {
    const { root, aws, object, called, sample } = locked();
    putFile(sample.local(root), "not zstd");
    putFile(object(sample.key), sample.bytes);
    samplesTree({ root, pack: "p", tool: "t", aws });
    expect(readFileSync(sample.local(root))).toEqual(sample.bytes);
    expect(called("cp")).toHaveLength(1);
  });

  it("fails naming the key and how to publish the set when S3 does not hold the archive", () => {
    const { root, aws, sample } = locked();
    const message = failure(() => samplesTree({ root, pack: "p", tool: "t", aws }));
    expect(message).toBe(`packs/p/upstream/t/samples.lock names s3://${BUCKET}/${sample.key}, which S3 does not hold: stage the set with writeSampleSet (scripts/samples/pack.ts) into the tool-pack's publish/samples/ and run the publish workflow (https://github.com/neralabs/rerail/actions/workflows/pack-data-publish.yml) for the pull request`);
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
    expect(samplesPush({ root, aws, tags: TAGS, log: (line) => printed.push(line) })).toEqual({ uploaded: [sample.key], locks: ["packs/p/upstream/t/samples.lock"] });
    expect(readFileSync(object(sample.key))).toEqual(sample.bytes);
    expect(printed.at(-1)).toBe("S3 holds the samples archive(s) the lock(s) name; commit them:\n  git add -- packs/p/upstream/t/samples.lock");
    expect(called("put-object")).toHaveLength(1);
    expect(samplesPush({ root, aws, tags: TAGS, log: quiet })).toEqual({ uploaded: [], locks: ["packs/p/upstream/t/samples.lock"] });
    expect(called("put-object")).toHaveLength(1);
    expect(called("cp")).toHaveLength(2);
  });

  it("refuses a cached archive that is not the locked one, calling no aws", () => {
    const { root, aws, bucket, calls, write } = withData();
    const sample = lockSamples(write, T);
    putFile(sample.local(root), compressArchive(archiveOf({ "LICENSES.md": "other\n" }).tar));
    const message = failure(() => samplesPush({ root, aws, tags: TAGS, log: quiet }));
    expect(message).toContain(`${shown(root, sample.local(root))} is not the samples archive packs/p/upstream/t/samples.lock names`);
    expect(message).toContain("Nothing was uploaded");
    expect(calls).toEqual([]);
    expect(files(bucket)).toEqual([]);
  });

  it("refuses an archive in neither the cache nor S3, before putting anything", () => {
    const { root, aws, called, write } = withData();
    const sample = lockSamples(write, T);
    const message = failure(() => samplesPush({ root, aws, tags: TAGS, log: quiet }));
    expect(message).toContain(`names samples archive ${sample.sha256}, which is in neither`);
    expect(message).toContain(`s3://${BUCKET}/${sample.key}`);
    expect(called("put-object")).toEqual([]);
  });

  it("refuses a selection with no samples.lock, and --dry-run", () => {
    const { root, aws } = withData();
    expect(() => samplesPush({ root, aws, tags: TAGS, log: quiet })).toThrow("no selected tool-pack has a samples.lock");
    expect(() => samplesPush({ root, aws, dryRun: true, tags: TAGS, log: quiet })).toThrow("samples push has no --dry-run");
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

const REPOSITORY = fileURLToPath(new URL("../../", import.meta.url));
/** The files scripts/s3/pack-fixtures.mjs runs from, by repository path. */
const SCRIPT_FILES = [
  "scripts/s3/pack-fixtures.mjs",
  "scripts/s3/test-data-archive.mjs",
  "scripts/check/licence-allowlist.json",
  "scripts/check/check-structure/metrics/07-pack-licence-annotations.mjs",
  "scripts/check/check-structure/lib.mjs",
  "scripts/check/check-structure/repo.mjs",
];
/** Copies this repository's pack-fixtures scripts into the checkout at `root`. */
const copyScripts = (root) => {
  for (const path of SCRIPT_FILES) putFile(join(root, path), readFileSync(join(REPOSITORY, path)));
};

/** setUp's fake `aws` as an executable, serving the bucket directory FAKE_BUCKET names. */
const FAKE_AWS = `#!/usr/bin/env node
const { copyFileSync, existsSync, mkdirSync, statSync } = require("node:fs");
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
if (args[1] === "head-object") {
  if (!existsSync(object(option("--key")))) exit(254, "", "An error occurred (404) when calling the HeadObject operation: Not Found\\n");
  exit(0, JSON.stringify({ ContentLength: statSync(object(option("--key"))).size }));
}
if (args[1] === "put-object" && option("--if-none-match") === "*") {
  const target = object(option("--key"));
  if (existsSync(target)) exit(254, "", "An error occurred (PreconditionFailed) when calling the PutObject operation\\n");
  mkdirSync(dirname(target), { recursive: true });
  copyFileSync(option("--body"), target);
  exit(0, "{}");
}
exit(2, "", "the fake aws does not implement: aws " + args.join(" ") + "\\n");
`;

/** A bin directory beside the checkout at `root` whose `aws` is FAKE_AWS, and the environment that runs it on `bucket`. */
function fakeAwsEnvironment(root, bucket) {
  const bin = join(dirname(root), "bin");
  putFile(join(bin, "aws"), FAKE_AWS);
  chmodSync(join(bin, "aws"), 0o755);
  return { ...process.env, PATH: `${bin}${delimiter}${process.env.PATH}`, FAKE_BUCKET: bucket };
}

/** A branch `name` off the checkout's HEAD, in a worktree beside it, committing `edits` (path → text, publish/ paths included). */
function branch(root, name, edits) {
  const worktree = join(dirname(root), name);
  git(root, "worktree", "add", "-q", "-b", name, worktree);
  for (const [path, text] of Object.entries(edits)) putFile(join(worktree, path), text);
  git(worktree, "add", "--", ...Object.keys(edits));
  git(worktree, "commit", "-qm", name);
  return worktree;
}

/**
 * published, with this repository's pack-fixtures scripts committed on main, and a pull request branch `feature`
 * checked out in the worktree `pr`. Its publish/ directories hold a changed and a new file of packs/p/upstream/t and a
 * new one of packs/q/upstream/u, and it carries code that writes `marker` if anything runs it: package.json scripts, a
 * tool's generator, and its own scripts/s3/pack-fixtures.mjs.
 */
function pullRequest() {
  const context = published();
  const { root } = context;
  copyScripts(root);
  git(root, "add", "scripts");
  git(root, "commit", "-qm", "scripts");
  const marker = join(dirname(root), "marker");
  const changes = {
    "packs/p/upstream/t/fixtures/rerail/a.py": "changed\n",
    "packs/p/upstream/t/fixtures/rerail/new.py": "new\n",
    "packs/q/upstream/u/authored/corpus/d.py": "d\n",
  };
  const publishRequest = Object.fromEntries(Object.entries(changes).map(([path, text]) => [path.replace(/^(packs\/[^/]+\/upstream\/[^/]+)\//u, "$1/publish/"), text]));
  const pr = branch(root, "feature", {
    ...publishRequest,
    "package.json": `${JSON.stringify({ scripts: { preinstall: `touch ${marker}`, "fixtures:stage": `touch ${marker}` } }, null, 2)}\n`,
    "packs/p/upstream/t/generate.py": `open(${JSON.stringify(marker)}, "w").write("ran")\n`,
    "scripts/s3/pack-fixtures.mjs": `import { writeFileSync } from "node:fs";\nwriteFileSync(${JSON.stringify(marker)}, "ran\\n");\n`,
  });
  return { ...context, pr, marker, changes, publishRequest };
}

/** What `toolPack`'s archive should hold: the published data overlaid with `changes` (pulled-layout paths). */
const overlaid = (data, changes, toolPack) =>
  Object.fromEntries(
    Object.entries({ ...data, ...changes })
      .filter(([path]) => path.startsWith(`${toolPack.directory}/`))
      .map(([path, text]) => [path.slice(toolPack.directory.length + 1), text]),
  );

/** What the compressed archive file `file` holds, path → text. */
const archivedIn = (file) => Object.fromEntries(readArchive(decompressArchive(readFileSync(file))).map(({ path, bytes }) => [path, bytes.toString("utf8")]));
/** What the archive packFrom wrote under `out` for the manifest object `object` holds, path → text. */
const archivedObject = (out, object) => archivedIn(join(out, object.key.slice("rerail/archives/".length)));

/** packFrom of `from` in the checkout of `context`, into a scratch directory: the manifest, the out directory and the log. */
async function packed(context, from, options = {}) {
  const out = join(dirname(context.root), `out-${from}`);
  const printed = [];
  const manifest = await packFrom({ root: context.root, from, out, aws: context.aws, ...options, log: (line) => printed.push(line) });
  return { manifest, out, printed: printed.join("\n") };
}

/** packFrom's message when it throws; fails the test when it does not. */
async function packFailure(context, from, options = {}) {
  try {
    await packed(context, from, options);
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
  throw new Error("expected packFrom to throw");
}

describe("pack --from (the publish workflow's prep)", () => {
  it("archives a pull request's publish/ from its commit's blobs alone, running none of its code, and writes the manifest", () => {
    const { root, pr, bucket, marker, data, changes, publishRequest } = pullRequest();
    const head = git(root, "rev-parse", "HEAD");
    const out = join(dirname(root), "out");
    const run = spawnSync(process.execPath, ["scripts/s3/pack-fixtures.mjs", "pack", "--from", "feature", "--out", out, "--manifest", join(out, "publish.json")], {
      cwd: root,
      encoding: "utf8",
      env: fakeAwsEnvironment(root, bucket),
    });
    expect(run.stderr).toBe("");
    expect(run.status).toBe(0);

    // The trusted checkout is as it was: still on main, nothing of the branch on disk, nothing run.
    expect(existsSync(marker)).toBe(false);
    expect(git(root, "rev-parse", "HEAD")).toBe(head);
    expect(git(root, "status", "--porcelain")).toBe("");
    expect(existsSync(join(root, "packs/p/upstream/t/generate.py"))).toBe(false);
    expect(existsSync(join(root, "packs/p/upstream/t/publish"))).toBe(false);

    const manifest = JSON.parse(readFileSync(join(out, "publish.json"), "utf8"));
    expect(manifest).toMatchObject({ format: "rerail.pack-data-publish/v1", commit: git(pr, "rev-parse", "HEAD").trim(), replace: false, remove: Object.keys(publishRequest).sort() });
    expect(manifest.objects.map(({ kind, pack: id, tool, added, changed, dropped }) => [kind, id, tool, added, changed, dropped])).toEqual([
      ["test-data", "p", "t", ["fixtures/rerail/new.py"], ["fixtures/rerail/a.py"], []],
      ["test-data", "q", "u", ["authored/corpus/d.py"], [], []],
    ]);
    for (const [object, toolPack] of [[manifest.objects[0], T], [manifest.objects[1], U]]) {
      const file = join(out, object.key.slice("rerail/archives/".length));
      expect(object.key).toBe(archiveKey(toolPack, object.sha256));
      expect(object.compressedBytes).toBe(statSync(file).size);
      expect(archivedIn(file)).toEqual(overlaid(data, changes, toolPack));
      expect(object.lockText).toBe(lockText({ sha256: object.sha256, files: object.files, bytes: object.bytes }));
      expect(object.licences).toEqual({ "Apache-2.0": object.files });
    }
    expect(run.stdout).toContain("| test-data | `p/t` |");
    expect(existsSync(marker)).toBe(false);
  });

  it("drops the files a removed UPSTREAM.md row listed, and keeps the rest of the archive", async () => {
    const context = published();
    branch(context.root, "unlist", { "packs/p/upstream/t/UPSTREAM.md": record([]), "packs/p/upstream/t/publish/cases.json": "[]\n" });
    const { manifest, out } = await packed(context, "unlist");
    const [object] = manifest.objects;
    expect(object.dropped).toEqual(["fixtures/rerail/a.py"]);
    expect(archivedObject(out, object)).toEqual({ "cases.json": "[]\n", "fixtures/upstream/b.py": "derived\n", "x y.expected.json": "{}\n" });
  });

  it("with replace, archives publish/ alone and lists every file of the locked archive it drops", async () => {
    const context = published();
    branch(context.root, "replace", { "packs/p/upstream/t/publish/fixtures/rerail/a.py": "changed\n" });
    const { manifest, out } = await packed(context, "replace", { replace: true });
    const [object] = manifest.objects;
    expect(manifest.replace).toBe(true);
    expect(object.dropped).toEqual(["fixtures/upstream/b.py", "x y.expected.json"]);
    expect(archivedObject(out, object)).toEqual({ "fixtures/rerail/a.py": "changed\n" });
  });

  it("refuses a publish/ path no archive takes, listing each, and writes nothing", async () => {
    const context = published();
    branch(context.root, "foreign", {
      "packs/p/upstream/t/publish/gates.yaml": "gates\n",
      "packs/p/upstream/t/publish/test-data.lock": "{}\n",
      "packs/p/upstream/t/publish/fixtures/rerail/a.py": "changed\n",
    });
    const message = await packFailure(context, "foreign");
    expect(message).toContain("publish/ holds 2 path(s) no archive takes, so nothing was packed");
    expect(message).toContain("packs/p/upstream/t/publish/gates.yaml (neither pack test data of packs/p/upstream/t nor under publish/samples/)");
    expect(message).toContain("packs/p/upstream/t/publish/test-data.lock");
    expect(existsSync(join(dirname(context.root), "out-foreign"))).toBe(false);
  });

  it("refuses a symlink in publish/, a file no archive can hold", async () => {
    const context = published();
    const pr = join(dirname(context.root), "link");
    git(context.root, "worktree", "add", "-q", "-b", "link", pr);
    mkdirSync(join(pr, "packs/p/upstream/t/publish"), { recursive: true });
    symlinkSync("../../../../../../etc/passwd", join(pr, "packs/p/upstream/t/publish/cases.json"));
    git(pr, "add", "--", "packs/p/upstream/t/publish");
    git(pr, "commit", "-qm", "link");
    expect(await packFailure(context, "link")).toContain("packs/p/upstream/t/publish/cases.json (mode 120000: not a regular file)");
  });

  it("fails with nothing to publish when the commit has no publish/", async () => {
    const context = published();
    expect(await packFailure(context, "HEAD")).toMatch(/^HEAD \([0-9a-f]{12}\) has no publish\/ in any packs\/<pack>\/upstream\/<tool>\/: nothing to publish$/u);
  });

  it("refuses, before writing anything, a file whose licence the allowlist does not take, listing it with its licence", async () => {
    const context = published();
    const upstream = (licence) => `# Tool\n\nLicense: ${licence}\n\n| File | Upstream path |\n|---|---|\n| \`upstream/t/fixtures/upstream/b.py\` | b.py |\n`;
    branch(context.root, "gpl", { "packs/p/licenses/tool/UPSTREAM.md": upstream("GPL-3.0-only"), "packs/p/upstream/t/publish/cases.json": "[]\n" });
    const message = await packFailure(context, "gpl");
    expect(message).toContain("1 file(s) going to S3 are under a licence scripts/check/licence-allowlist.json does not take");
    expect(message).toContain("  packs/p/upstream/t/fixtures/upstream/b.py: GPL-3.0-only");
    expect(existsSync(join(dirname(context.root), "out-gpl"))).toBe(false);

    branch(context.root, "mit", { "packs/p/licenses/tool/UPSTREAM.md": upstream("MIT"), "packs/p/upstream/t/publish/cases.json": "[]\n" });
    const { manifest } = await packed(context, "mit");
    expect(manifest.objects[0].licences).toEqual({ "Apache-2.0": 3, MIT: 1 });
  });
});

describe("licenceAllowed", () => {
  it("passes A OR B when either side is allowed, A AND B only when both are", () => {
    const allowed = new Set(["MIT", "Apache-2.0"]);
    expect(licenceAllowed("MIT", allowed)).toBe(true);
    expect(licenceAllowed("Unlicense OR MIT", allowed)).toBe(true);
    expect(licenceAllowed("GPL-3.0-only", allowed)).toBe(false);
    expect(licenceAllowed("MIT AND GPL-3.0-only", allowed)).toBe(false);
    expect(licenceAllowed("(MIT AND Apache-2.0) OR GPL-3.0-only", allowed)).toBe(true);
  });
});

describe("upload --objects and confirm (the publish workflow's upload)", () => {
  /** A pull request's publish/ packed by prep: the context, its objects (as prep's job output) and the archives. */
  async function prepared() {
    const context = published();
    branch(context.root, "feature", { "packs/p/upstream/t/publish/fixtures/rerail/a.py": "changed\n", "packs/q/upstream/u/publish/cases.json": "[]\n" });
    const { manifest, out } = await packed(context, "feature");
    const objects = manifest.objects.map(({ kind, pack: id, tool, key, sha256: digest, files: count, bytes, compressedBytes }) => ({ kind, pack: id, tool, key, sha256: digest, files: count, bytes, compressedBytes }));
    context.calls.length = 0;
    return { ...context, objects, archives: out };
  }
  const RUN_TAGS = { commit: "1".repeat(40), pr: "177", runId: "42", uploader: "approver" };

  it("puts each archive create-only with the run's tags, and confirm verifies every object afresh", async () => {
    const { aws, object, objects, archives, tagging } = await prepared();
    const { uploaded } = uploadPublished({ objects, archives, tags: RUN_TAGS, aws, log: () => {} });
    expect(uploaded).toEqual(objects.map(({ key }) => key).sort());
    for (const { key } of objects) {
      expect(existsSync(object(key))).toBe(true);
      expect(tagging.get(key)).toBe(`rerail%3Acommit=${"1".repeat(40)}&rerail%3Apr=177&rerail%3Arun-id=42&rerail%3Auploader=approver`);
    }
    expect(confirm({ objects, uploaded, aws, log: () => {} })).toEqual({ confirmed: objects.map(({ key }) => key), failed: [] });
    // Run again: S3 holds every object, so nothing is put, and confirm accepts them.
    expect(uploadPublished({ objects, archives, tags: RUN_TAGS, aws, log: () => {} }).uploaded).toEqual([]);
    expect(confirm({ objects, aws, log: () => {} }).failed).toEqual([]);
  });

  it("refuses a tampered archive, or one that is not the object prep named, before any put", async () => {
    const { aws, called, bucket, objects, archives } = await prepared();
    const held = files(bucket);
    const file = join(archives, objects[0].key.slice("rerail/archives/".length));
    const bytes = readFileSync(file);
    bytes[bytes.length - 1] ^= 1;
    writeFileSync(file, bytes);
    const message = failure(() => uploadPublished({ objects, archives, tags: RUN_TAGS, aws, log: () => {} }));
    expect(message).toContain("1 archive(s) are not what prep named, so nothing was uploaded");
    expect(message).toContain(objects[0].key);
    expect(called("put-object")).toEqual([]);
    expect(files(bucket)).toEqual(held);
    expect(() => publishedObjectsOf([{ ...objects[1], key: objects[0].key }])).toThrow("which its kind, pack, tool and sha256 make");
    expect(() => publishedObjectsOf([{ ...objects[1], pack: "../x" }])).toThrow('pack "../x"');
  });

  it("confirm reports each object S3 lacks or holds at another size than this run put, and confirms the rest", async () => {
    const { aws, object, objects, archives } = await prepared();
    const { uploaded } = uploadPublished({ objects, archives, tags: RUN_TAGS, aws, log: () => {} });
    rmSync(object(objects[1].key));
    const wrong = [{ ...objects[0], compressedBytes: objects[0].compressedBytes + 1 }, objects[1]];
    const result = confirm({ objects: wrong, uploaded, aws, log: () => {} });
    expect(result.confirmed).toEqual([]);
    expect(result.failed).toEqual([
      { key: objects[0].key, reason: `S3 holds ${objects[0].compressedBytes} byte(s), not the ${objects[0].compressedBytes + 1} this run put` },
      { key: objects[1].key, reason: "S3 does not hold it" },
    ]);
  });
});

/** publishedObjects, through uploadPublished's own check (it throws before reading any archive). */
const publishedObjectsOf = (objects) => uploadPublished({ objects, archives: "/nonexistent", tags: TAGS, aws: () => ({ status: 0, stdout: "{}" }), log: () => {} });

describe("stage", () => {
  it("copies the data on disk that the locked archive lacks or holds differently into publish/, and pull lays it back", () => {
    const { root, aws, write, data } = published();
    write("packs/p/upstream/t/fixtures/rerail/a.py", "changed\n");
    write("packs/p/upstream/t/fixtures/rerail/new.py", "new\n");
    rmSync(join(root, "packs/p/upstream/t/x y.expected.json"));
    const printed = [];
    const result = stage({ root, aws, pack: "p", tool: "t", log: (line) => printed.push(line) });
    expect(result.toolPacks).toEqual([{ ...T, staged: ["fixtures/rerail/a.py", "fixtures/rerail/new.py"], removed: [], missing: ["x y.expected.json"] }]);
    const publishDirectory = join(root, "packs/p/upstream/t/publish");
    expect(files(publishDirectory)).toEqual(result.toolPacks[0].staged);
    expect(printed.join("\n")).toContain("an overlay cannot delete a file: to publish exactly the data on disk, stage with --all and publish with replace");
    expect(printed.join("\n")).toContain("git add -- packs/p/upstream/t/publish");

    // Reverting a file unstages it; --all stages the whole set.
    write("packs/p/upstream/t/fixtures/rerail/a.py", data["packs/p/upstream/t/fixtures/rerail/a.py"]);
    expect(stage({ root, aws, pack: "p", tool: "t", log: () => {} }).toolPacks[0]).toMatchObject({ staged: ["fixtures/rerail/new.py"], removed: ["fixtures/rerail/a.py"] });
    expect(stage({ root, aws, pack: "p", tool: "t", all: true, log: () => {} }).toolPacks[0].staged).toEqual(["fixtures/rerail/a.py", "fixtures/rerail/new.py", "fixtures/upstream/b.py"]);

    pull({ root, aws, log: () => {} });
    expect(readFileSync(join(root, "packs/p/upstream/t/fixtures/rerail/new.py"), "utf8")).toBe("new\n");
    expect(stage({ root, aws, pack: "p", tool: "t", all: true, dryRun: true, log: () => {} }).toolPacks[0].staged).toHaveLength(4);
  });
});

describe("check", () => {
  /** published, with a samples.lock for packs/q/upstream/u whose archive S3 holds. */
  function locked() {
    const context = published();
    const sample = lockSamples(context.write, U);
    putFile(context.object(sample.key), sample.bytes);
    return { ...context, sample };
  }
  const archiveKeys = (root) => [T, { pack: "p", tool: "w", directory: "packs/p/upstream/w" }, U].map((toolPack) => archiveKey(toolPack, readLock(root, toolPack).sha256));

  it("passes when S3 holds every archive and samples archive the locks name, asking head-object alone", () => {
    const { root, aws, calls, sample } = locked();
    expect(check({ root, aws, log: () => {} }).checked).toEqual([...archiveKeys(root), sample.key].sort());
    expect(calls.filter((args) => args[0] !== "sts" && args[1] !== "head-object")).toEqual([]);
  });

  it("fails listing each missing archive with its lock and remedy, downloading nothing", () => {
    const { root, aws, object, calls, sample } = locked();
    const archive = archiveKey(U, readLock(root, U).sha256);
    rmSync(object(archive));
    rmSync(object(sample.key));
    const message = failure(() => check({ root, aws, log: () => {} }));
    expect(message.split("\n")).toEqual([
      'S3 lacks 2 of the 4 object(s) the locks name (CONTRIBUTING.md, "Pack test data"):',
      `  packs/q/upstream/u/test-data.lock names s3://${BUCKET}/${archive}, which S3 does not hold: publish the pull request's data through the publish workflow (https://github.com/neralabs/rerail/actions/workflows/pack-data-publish.yml), which commits the lock naming what it uploaded`,
      `  packs/q/upstream/u/samples.lock names s3://${BUCKET}/${sample.key}, which S3 does not hold: stage the set with writeSampleSet (scripts/samples/pack.ts) into the tool-pack's publish/samples/ and run the publish workflow (https://github.com/neralabs/rerail/actions/workflows/pack-data-publish.yml) for the pull request`,
    ]);
    expect(calls.filter((args) => args[0] !== "sts" && args[1] !== "head-object")).toEqual([]);
  });

  it("exits 0 when S3 holds everything and 1 when it lacks anything, as a command", () => {
    const { root, bucket, object, sample } = locked();
    const bin = join(dirname(root), "bin");
    putFile(join(bin, "aws"), FAKE_AWS);
    chmodSync(join(bin, "aws"), 0o755);
    copyScripts(root);
    const run = () => spawnSync(process.execPath, ["scripts/s3/pack-fixtures.mjs", "check"], {
      cwd: root,
      encoding: "utf8",
      env: { ...process.env, PATH: `${bin}${delimiter}${process.env.PATH}`, FAKE_BUCKET: bucket },
    });
    const passed = run();
    expect(passed.stderr).toBe("");
    expect(passed.stdout).toBe("S3 holds every object the locks name: 3 archive(s) and 1 samples archive(s)\n");
    expect(passed.status).toBe(0);
    rmSync(object(sample.key));
    const failed = run();
    expect(failed.status).toBe(1);
    expect(failed.stderr).toContain(`fixtures:check: S3 lacks 1 of the 4 object(s)`);
    expect(failed.stderr).toContain(`s3://${BUCKET}/${sample.key}`);
  });
});

// One-off: snapshots the per-file provenance rows of packs/*/licenses/*/UPSTREAM.md, as they stand at a base commit,
// into each tool-pack's test-data archive as `provenance.json` and `LICENSES.md`, so the tracked records can become per
// repository. Every entry of each locked archive is kept byte for byte and exactly those two files are added; any entry
// the rows, the tool's authored listing or the recorded-file names cannot classify, and any row whose file the archive
// lacks, stop the run. Kept in history only: the generators write these files once the follow-up lands.
//
//   node scripts/pack-data/snapshot-provenance.mjs [--base <rev>] [--write]   build (and with --write cache and lock)
//   node scripts/pack-data/snapshot-provenance.mjs --check <rev>              identity check of the locks against <rev>'s
//
// It reads the archives from ARCHIVE_CACHE (run `npm run fixtures:pull` first) and never touches S3.
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { parseArgs } from "node:util";

import { archiveCachePath, listedAuthored, ROOT, toolPacks } from "../s3/pack-fixtures.mjs";
import { buildArchive, compressArchive, decompressArchive, lockText, parseLock, readArchive, TEST_DATA_LOCK } from "../s3/test-data-archive.mjs";

const FORMAT = "rerail.pack-provenance/v1";
const PROVENANCE = "provenance.json";
const LICENSES = "LICENSES.md";
const RERAIL = "rerail";
const RERAIL_LICENCE = "Apache-2.0";
const RECORDED = [/^[^/]*\.expected\.json$/u, /^cases\.json$/u, /^[^/]*\.cases\.json$/u, /^fixtures\.upstream-check\.json$/u, /^lizard-[^/]*\.tsv$/u];
const AUTHORED_DIRECTORY = /^authored\//u;
const TABLE = /^packs\/([^/]+)\/licenses\/([^/]+)\/UPSTREAM\.md$/u;
const RENAMED = /^derived\b.*\(`([^`]+)` with its names and literals renamed\)$/u;
const GENERIC_COPY = [/^verbatim$/u, /^derived$/u, /^extracted \(its `code`, verbatim\)$/u, /^derived \(`[^`]+` with its names and literals renamed\)$/u];
const HOW_ORDER = ["verbatim", "extracted", "adapted", "renamed", "recorded", "authored"];

const byBytes = (a, b) => Buffer.compare(Buffer.from(a, "utf8"), Buffer.from(b, "utf8"));

function git(args) {
  const result = spawnSync("git", args, { cwd: ROOT, encoding: "buffer", maxBuffer: 1 << 30 });
  if (result.status !== 0) throw new Error(`git ${args.join(" ")}: ${result.stderr.toString("utf8")}`);
  return result.stdout;
}
const gitText = (args) => git(args).toString("utf8");
const show = (rev, path) => gitText(["show", `${rev}:${path}`]);
const listed = (rev, prefix) => gitText(["ls-tree", "-r", "--name-only", rev, "--", prefix]).split("\n").filter(Boolean);

/** A markdown table row's cells, splitting on `|` outside backticks. */
function cells(line) {
  const out = [];
  let current = "";
  let quoted = false;
  for (const char of line.trim().replace(/^\|/u, "").replace(/\|$/u, "")) {
    if (char === "`") quoted = !quoted;
    if (char === "|" && !quoted) {
      out.push(current.trim());
      current = "";
    } else current += char;
  }
  out.push(current.trim());
  return out;
}

/** The source of one licenses/<project>/UPSTREAM.md: repository, tag, commit and licence, from its `Upstream:` line. */
function sourceOf(label, text, rev, pack) {
  const line = text.split("\n").find((candidate) => candidate.startsWith("Upstream: "));
  if (line === undefined) throw new Error(`${label}: no Upstream: line`);
  const link = /\]\((https:\/\/github\.com\/[^/]+\/[^/)]+)\/tree\/([^)]+)\)/u.exec(line);
  if (link === null) throw new Error(`${label}: no GitHub tree link`);
  const licence = /\bLicense: (.+?)(?: \(|;)/u.exec(line)?.[1];
  if (licence === undefined) throw new Error(`${label}: no License:`);
  const tag = /\bat tag `([^`]+)`/u.exec(line)?.[1] ?? (/^[0-9a-f]{40}$/u.test(link[2]) ? undefined : link[2]);
  let commit = /\bcommit `([0-9a-f]{40})`/u.exec(line)?.[1];
  if (commit === undefined) {
    // Bandit's and Ruff's records name a tag alone; their generators pin the commit (`ORIGIN = "<repo>@<sha>"`).
    const project = label.split("/")[3];
    const generator = `packs/${pack}/upstream/${project}/generate.py`;
    const origin = new RegExp(`^ORIGIN = "${link[1].replaceAll(".", "\\.")}@([0-9a-f]{40})"$`, "mu").exec(show(rev, generator))?.[1];
    if (origin === undefined) throw new Error(`${label}: no commit, and ${generator} pins none for ${link[1]}`);
    commit = origin;
  }
  return { repository: link[1], ...(tag === undefined ? {} : { tag }), commit, license: licence };
}

/** Splits an upstream cell into its path and the case locator after it. */
function upstreamCell(label, cell) {
  if (cell === "none") return { from: undefined, locator: undefined };
  const match = /^`([^`]+)`(.*)$/u.exec(cell);
  if (match === null) throw new Error(`${label}: upstream cell ${JSON.stringify(cell)} is not a backticked path`);
  const [path, ...rest] = match[1].split(" ");
  const locator = [rest.join(" "), match[2].replace(/^,\s*/u, "").trim()].filter(Boolean).join(" ");
  return { from: path, locator: locator === "" ? undefined : locator };
}

/** One row's `how` (and renamedFrom), from its Copy cell and whether it lies under fixtures/renamed/. */
function howOf(label, path, copy, from) {
  const renamed = path.startsWith("fixtures/renamed/");
  const variant = RENAMED.exec(copy);
  const copyOf = /^a copy of `[^`]+`/u.test(copy);
  if (from === undefined) {
    // No upstream path: Rerail's own file, or a copy of one (node's fixtures/renamed/package.json).
    if (!/^Rerail's own, not copied\b/u.test(copy) && !copyOf) throw new Error(`${label}: ${path} has no upstream path but is not Rerail's`);
    return { how: "authored", source: RERAIL };
  }
  if (renamed && copyOf) return { how: "verbatim" };
  if (renamed !== (variant !== null)) throw new Error(`${label}: ${path} is ${renamed ? "" : "not "}under fixtures/renamed/ but its copy is ${JSON.stringify(copy)}`);
  if (variant !== null) return { how: "renamed", renamedFrom: variant[1] };
  if (copy === "derived") return { how: "extracted" }; // the ESLint family's originals: one case's `code`, verbatim
  if (/^derived: wrapped in /u.test(copy)) return { how: "adapted" };
  if (/^verbatim\b/u.test(copy)) return { how: "verbatim" };
  if (/^extracted\b/u.test(copy) || /^the `code` of /u.test(copy) || /^`(pass_str|fail_str)`, verbatim$/u.test(copy) || /, captured verbatim$/u.test(copy)) return { how: "extracted" };
  throw new Error(`${label}: ${path}: cannot classify copy ${JSON.stringify(copy)}`);
}

/** Every row of the base commit's licenses/*\/UPSTREAM.md, keyed by repository path, and each project's source. */
function readTables(rev) {
  const rows = new Map();
  const projects = new Map();
  for (const label of listed(rev, "packs").filter((path) => TABLE.test(path))) {
    const [, pack, project] = TABLE.exec(label);
    const text = show(rev, label);
    const directory = `packs/${pack}/licenses/${project}`;
    const texts = listed(rev, directory)
      .filter((path) => path !== label)
      .sort(byBytes)
      .map((path) => ({ file: path.slice(directory.length + 1), text: show(rev, path) }));
    projects.set(`${pack}/${project}`, { pack, project, label, source: sourceOf(label, text, rev, pack), texts });
    let header;
    for (const line of text.split("\n")) {
      if (!line.startsWith("|")) {
        header = undefined;
        continue;
      }
      const row = cells(line);
      if (header === undefined) {
        header = row;
        continue;
      }
      if (/^-+$/u.test(row[0] ?? "")) continue;
      if (row.length !== header.length) throw new Error(`${label}: row of ${row.length} cell(s) under a header of ${header.length}: ${line}`);
      if (header[0] !== "File" || !/^Upstream (path|case)$/u.test(header[1]) || header[2] !== "Copy") throw new Error(`${label}: unexpected header ${header.join(" | ")}`);
      const file = /^`(upstream\/([^/]+)\/[^`]+)`$/u.exec(row[0]);
      if (file === null) throw new Error(`${label}: file cell ${row[0]}`);
      const repositoryPath = `packs/${pack}/${file[1]}`;
      if (rows.has(repositoryPath)) throw new Error(`${label}: ${file[1]} listed twice`);
      const path = file[1].slice(`upstream/${file[2]}/`.length);
      const { from, locator } = upstreamCell(label, row[1]);
      const copy = row[2];
      const kind = howOf(label, path, copy, from);
      const extra = header.slice(3).map((name, index) => `${name}: ${row[3 + index]}`);
      const detail = [locator, GENERIC_COPY.some((pattern) => pattern.test(copy)) ? undefined : copy, ...extra].filter(Boolean).join("; ");
      rows.set(repositoryPath, {
        tool: file[2],
        path,
        source: kind.source ?? project,
        project: `${pack}/${project}`,
        ...(from === undefined || kind.source === RERAIL ? {} : { from }),
        how: kind.how,
        ...(kind.renamedFrom === undefined ? {} : { renamedFrom: kind.renamedFrom }),
        ...(detail === "" ? {} : { detail }),
      });
    }
  }
  // A case extracted from a test file the pack also copies whole names that copy (`tests/…`); its upstream path is the
  // copy's.
  for (const [key, row] of rows) {
    if (row.from === undefined || row.how === "verbatim") continue;
    const copied = rows.get(`${key.split("/upstream/")[0]}/upstream/${row.tool}/${row.from}`);
    if (copied !== undefined && copied.how === "verbatim" && copied.from !== undefined) rows.set(key, { ...row, from: copied.from });
  }
  return { rows, projects };
}

/** The archive entries of `toolPack`'s lock at `rev`, from ARCHIVE_CACHE. */
function lockedEntries(rev, toolPack) {
  const label = `${toolPack.directory}/${TEST_DATA_LOCK}`;
  const lock = parseLock(show(rev, label), label);
  const cached = archiveCachePath(ROOT, toolPack, lock.sha256);
  if (!existsSync(cached)) throw new Error(`${label} at ${rev}: ${cached} is missing; run npm run fixtures:pull at that commit`);
  return { lock, entries: readArchive(decompressArchive(readFileSync(cached))) };
}

const fence = (text) => "`".repeat(Math.max(2, ...[...text.matchAll(/`+/gu)].map((match) => match[0].length)) + 1);
const counts = (entries) => {
  const tally = new Map();
  for (const entry of entries) tally.set(entry.how, (tally.get(entry.how) ?? 0) + 1);
  return HOW_ORDER.filter((how) => tally.has(how)).map((how) => `${tally.get(how)} ${how}`).join(", ");
};

function provenanceJson(toolPack, snapshot, sources, files) {
  const sortedSources = Object.fromEntries([...sources].sort(([a], [b]) => byBytes(a, b)));
  const sortedFiles = Object.fromEntries([...files].sort(([a], [b]) => byBytes(a, b)));
  return `${JSON.stringify({ format: FORMAT, snapshot, pack: toolPack.pack, tool: toolPack.tool, sources: sortedSources, files: sortedFiles }, null, 2)}\n`;
}

function licensesMarkdown(toolPack, snapshot, sources, projects, files) {
  const entries = [...files.values()];
  const lines = [
    `# Licences of the ${toolPack.tool} test data`,
    "",
    `This archive is the test data of \`${toolPack.directory}/\`. \`${PROVENANCE}\` beside this file lists every file in it with its source, its upstream path and how it was made (${snapshot}). ${sources.size > 1 ? `Each upstream source below is listed with its licence, how many files come from it, and its licence text as the repository tracks it under \`packs/${toolPack.pack}/licenses/\`; the rest is Rerail's.` : "Every file in it is Rerail's."}`,
    "",
  ];
  for (const [id, source] of [...sources].sort(([a], [b]) => byBytes(a, b))) {
    if (id === RERAIL) continue;
    const taken = entries.filter((entry) => entry.source === id);
    const project = projects.get(`${toolPack.pack}/${id}`);
    lines.push(
      `## ${id}`,
      "",
      `Repository ${source.repository}${source.tag === undefined ? "" : `, tag \`${source.tag}\``}, commit \`${source.commit}\`. License: ${source.license}.`,
      "",
      `${taken.length} file(s): ${counts(taken)}. They remain under ${id}'s licence. Files marked \`extracted\`, \`adapted\` or \`renamed\` were changed by Rerail as \`${PROVENANCE}\` describes: \`extracted\` is one test case cut out of the upstream file it names (\`detail\` locates the case), \`adapted\` is such a case wrapped so the tool reads it, and \`renamed\` is the file \`renamedFrom\` names with its names and literals renamed. \`verbatim\` files are unchanged copies.`,
      "",
    );
    for (const { file, text } of project.texts) lines.push(`### ${file}`, "", fence(text), text.trimEnd(), fence(text), "");
  }
  const own = entries.filter((entry) => entry.source === RERAIL);
  if (own.length > 0) {
    lines.push(
      "## Rerail",
      "",
      `${own.length} file(s): ${counts(own)}. Rerail's, under ${RERAIL_LICENCE} (the pack's \`LICENSE\`): \`authored\` files were written by Rerail, and \`recorded\` files are a tool's output on this data, recorded by the pack's generators.`,
      "",
    );
  }
  return `${lines.join("\n").trimEnd()}\n`;
}

/** The new entries of one tool-pack's archive and the lock text, refusing anything it cannot classify. */
function snapshotToolPack(toolPack, rev, tables, snapshot) {
  const { lock, entries } = lockedEntries(rev, toolPack);
  const clash = entries.filter((entry) => entry.path === PROVENANCE || entry.path === LICENSES);
  if (clash.length > 0) throw new Error(`${toolPack.directory}: the locked archive already holds ${clash.map((entry) => entry.path).join(", ")}`);
  const authoredRecord = `${toolPack.directory}/UPSTREAM.md`;
  const authored = new Set(listed(rev, authoredRecord).length === 0 ? [] : listedAuthored(authoredRecord, show(rev, authoredRecord)));
  const present = new Set(entries.map((entry) => entry.path));
  const files = new Map();
  const sources = new Map([[RERAIL, { license: RERAIL_LICENCE }]]);
  const unknown = [];
  for (const { path } of entries) {
    const repositoryPath = `${toolPack.directory}/${path}`;
    const row = tables.rows.get(repositoryPath);
    if (row !== undefined && authored.has(repositoryPath)) throw new Error(`${repositoryPath}: both an upstream row and an authored listing`);
    if (row !== undefined) {
      const { tool, path: _path, project, ...entry } = row;
      if (entry.renamedFrom !== undefined && !present.has(entry.renamedFrom)) throw new Error(`${repositoryPath}: renamed from ${entry.renamedFrom}, which the archive lacks`);
      if (entry.source !== RERAIL) sources.set(entry.source, tables.projects.get(project).source);
      files.set(path, entry);
    } else if (authored.has(repositoryPath) || AUTHORED_DIRECTORY.test(path)) files.set(path, { source: RERAIL, how: "authored" });
    else if (RECORDED.some((pattern) => pattern.test(path))) files.set(path, { source: RERAIL, how: "recorded" });
    else unknown.push(path);
  }
  if (unknown.length > 0) throw new Error(`${toolPack.directory}: ${unknown.length} archive entr(ies) no row, authored listing or recorded name covers, e.g. ${unknown.slice(0, 5).join(", ")}`);
  const absent = [...tables.rows.entries()].filter(([key]) => key.startsWith(`${toolPack.directory}/`) && !present.has(key.slice(toolPack.directory.length + 1)));
  if (absent.length > 0) throw new Error(`${toolPack.directory}: ${absent.length} row(s) name files the archive lacks, e.g. ${absent.slice(0, 5).map(([key]) => key).join(", ")}`);
  const added = [
    { path: PROVENANCE, bytes: Buffer.from(provenanceJson(toolPack, snapshot, sources, files), "utf8") },
    { path: LICENSES, bytes: Buffer.from(licensesMarkdown(toolPack, snapshot, sources, tables.projects, files), "utf8") },
  ];
  const archive = buildArchive([...entries, ...added]);
  return { lock, archive, files };
}

/** Old entries byte-identical, the file count up by exactly two, and exactly provenance.json and LICENSES.md added. */
function identityCheck(rev) {
  let failures = 0;
  for (const toolPack of toolPacks(ROOT)) {
    const before = lockedEntries(rev, toolPack);
    const lock = parseLock(readFileSync(join(ROOT, toolPack.directory, TEST_DATA_LOCK), "utf8"), toolPack.directory);
    const cached = archiveCachePath(ROOT, toolPack, lock.sha256);
    const after = readArchive(decompressArchive(readFileSync(cached)));
    const old = new Map(before.entries.map((entry) => [entry.path, entry.bytes]));
    const changed = after.filter((entry) => old.has(entry.path) && !old.get(entry.path).equals(entry.bytes)).map((entry) => entry.path);
    const added = after.filter((entry) => !old.has(entry.path)).map((entry) => entry.path).sort(byBytes);
    const kept = new Set(after.map((entry) => entry.path));
    const lost = [...old.keys()].filter((path) => !kept.has(path));
    const ok = after.length === before.entries.length + 2 && changed.length === 0 && lost.length === 0 && added.join(",") === `${LICENSES},${PROVENANCE}`;
    if (!ok) failures += 1;
    console.log(`${ok ? "ok  " : "FAIL"} ${toolPack.directory}: ${before.entries.length} -> ${after.length} file(s); added ${added.join(", ") || "none"}; changed ${changed.length}; lost ${lost.length}`);
  }
  if (failures > 0) throw new Error(`${failures} tool-pack(s) fail the identity check`);
}

function main() {
  const { values } = parseArgs({ options: { base: { type: "string", default: "HEAD" }, write: { type: "boolean", default: false }, check: { type: "string" } } });
  if (values.check !== undefined) return identityCheck(values.check);
  const base = gitText(["rev-parse", values.base]).trim();
  const tablesCommit = gitText(["log", "-1", "--format=%H", base, "--", "packs/*/licenses/*/UPSTREAM.md"]).trim();
  const tables = readTables(base);
  for (const toolPack of toolPacks(ROOT)) {
    const snapshot = `bootstrapped from packs/${toolPack.pack}/licenses/*/UPSTREAM.md at commit ${tablesCommit}; generators do not write this file yet`;
    const { lock, archive, files } = snapshotToolPack(toolPack, base, tables, snapshot);
    console.log(`${toolPack.directory}: ${lock.files} -> ${archive.files} file(s), ${archive.bytes} byte(s), sha256 ${archive.sha256}; ${counts([...files.values()])}`);
    if (!values.write) continue;
    const cached = archiveCachePath(ROOT, toolPack, archive.sha256);
    mkdirSync(dirname(cached), { recursive: true });
    writeFileSync(`${cached}.partial`, compressArchive(archive.tar));
    renameSync(`${cached}.partial`, cached);
    writeFileSync(join(ROOT, toolPack.directory, TEST_DATA_LOCK), lockText(archive));
  }
}

main();

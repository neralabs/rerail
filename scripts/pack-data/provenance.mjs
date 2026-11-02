// A tool-pack's per-file provenance: `packs/<pack>/upstream/<tool>/provenance.json` (every pack test data file, its
// source, its upstream path and how it was made) and `LICENSES.md` beside it (each upstream source's licence and
// licence text, and Rerail's), both archived with the test data (docs/pack-format.md). The generators write them,
// through writeProvenance (JavaScript) or `write_provenance` (scripts/s3/pack_fixtures.py, through this file's CLI):
//
//   node scripts/pack-data/provenance.mjs write --pack <id> --tool <tool> [--root <checkout>] < {"entries":…,"commits":…}
//
// A generator names the files it copied from upstream; the writer classifies Rerail's own files itself (authored by the
// tool's UPSTREAM.md listing or authored/, recorded by RECORDED), reads each source from its tracked
// packs/<pack>/licenses/<project>/UPSTREAM.md, and refuses, writing nothing, anything that disagrees with those records.
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { parseArgs } from "node:util";

import { isPackFixture, listedAuthored, pathGlob, PROVENANCE_FORMAT, readUpstreamSources, ROOT } from "../s3/pack-fixtures.mjs";

/** The provenance file's name in the tool directory. */
export const PROVENANCE = "provenance.json";
/** The licences file's name in the tool directory. */
export const LICENSES = "LICENSES.md";
/** The `source` of Rerail's own files. */
export const RERAIL = "rerail";
const RERAIL_LICENCE = "Apache-2.0";
/** Tool-relative names of a tool's output recorded on its data, which are Rerail's (`how: "recorded"`). */
const RECORDED = [/^[^/]*\.expected\.json$/u, /^cases\.json$/u, /^[^/]*\.cases\.json$/u, /^fixtures\.upstream-check\.json$/u, /^lizard-[^/]*\.tsv$/u];
/** Rerail's authored corpus, by its place (pack-fixtures.mjs AUTHORED_CORPUS_PATTERN). */
const AUTHORED_DIRECTORY = /^authored\//u;
/** The `how` of an entry, least changed first: the order LICENSES.md counts them in. */
const HOW_ORDER = ["verbatim", "extracted", "adapted", "renamed", "recorded", "authored"];
/** An entry's keys, in the order provenance.json writes them. */
const ENTRY_KEYS = ["source", "from", "how", "renamedFrom", "detail"];
/** A Source-row tag cell meaning "no tag". */
const NO_TAG = "—";
/** Where a tool's rename variants sit under fixtures/ (eslint-expected.mjs RENAMED_DIRECTORY). */
const FIXTURES = "fixtures";

const byBytes = (a, b) => Buffer.compare(Buffer.from(a, "utf8"), Buffer.from(b, "utf8"));
const pairs = (record) => (record instanceof Map ? [...record] : Object.entries(record ?? {}));

/** Every file under `directory` (absolute), as paths relative to it, `/`-separated. */
function filesBelow(directory) {
  if (!existsSync(directory)) return [];
  return readdirSync(directory, { recursive: true, withFileTypes: true })
    .filter((entry) => entry.isFile())
    .map((entry) => join(entry.parentPath, entry.name).slice(directory.length + 1).split("\\").join("/"));
}

/** The tool directory's pack test data on disk, tool-relative, without the two outputs. */
function toolFiles(root, directory) {
  return new Set(
    filesBelow(join(root, directory)).filter((path) => path !== PROVENANCE && path !== LICENSES && isPackFixture(`${directory}/${path}`)),
  );
}

/** The tool-relative files the tool's own UPSTREAM.md lists as Rerail's. */
function authoredFiles(root, directory) {
  const record = `${directory}/UPSTREAM.md`;
  if (!existsSync(join(root, record))) return new Set();
  return new Set(listedAuthored(record, readFileSync(join(root, record), "utf8")).map((path) => path.slice(directory.length + 1)));
}

/** Each licenses/<project>/ of the pack holding an UPSTREAM.md: its directory and what that record says. */
function projectRecords(root, pack) {
  const licenses = join(root, "packs", pack, "licenses");
  if (!existsSync(licenses)) return new Map();
  return new Map(
    readdirSync(licenses, { withFileTypes: true })
      .filter((entry) => entry.isDirectory() && existsSync(join(licenses, entry.name, "UPSTREAM.md")))
      .map((entry) => [entry.name, { directory: join(licenses, entry.name), ...readUpstreamSources(readFileSync(join(licenses, entry.name, "UPSTREAM.md"), "utf8")) }]),
  );
}

/** A Source row as provenance.json's `sources.<project>`: no `tag` when the row has none. */
function sourceOf({ repository, tag, commit, licence }) {
  return { repository, ...(tag === undefined || tag === "" || tag === NO_TAG ? {} : { tag }), commit, license: licence };
}

/** What is wrong with an entry's shape, or undefined. */
function shapeProblem(path, entry) {
  if (entry === null || typeof entry !== "object" || Array.isArray(entry)) return `${path}: the entry is not an object`;
  const unknown = Object.keys(entry).filter((key) => !ENTRY_KEYS.includes(key));
  if (unknown.length > 0) return `${path}: the entry has unknown key(s) ${unknown.join(", ")} (an entry holds ${ENTRY_KEYS.join(", ")})`;
  if (typeof entry.source !== "string") return `${path}: the entry has no \`source\``;
  if (!HOW_ORDER.includes(entry.how)) return `${path}: \`how\` ${JSON.stringify(entry.how)} is none of ${HOW_ORDER.join(", ")}`;
  const notText = ["from", "renamedFrom", "detail"].filter((key) => entry[key] !== undefined && typeof entry[key] !== "string");
  return notText.length > 0 ? `${path}: ${notText.join(", ")} must be text` : undefined;
}

/** The problems of one upstream entry against its source's record (refusals 1, 4 and 5). */
function upstreamProblems(path, entry, record, { tool, authored, recordLabel }) {
  const problems = [];
  if (authored.has(path) || AUTHORED_DIRECTORY.test(path)) problems.push(`${path}: an entry from ${entry.source}, but the tool's UPSTREAM.md or authored/ makes it Rerail's`);
  const packPath = `upstream/${tool}/${path}`;
  if (!record.taken.some(({ glob }) => pathGlob(glob).test(packPath))) problems.push(`${packPath}: matches none of the "Taken from it" globs of ${recordLabel}`);
  const refused = entry.from === undefined ? undefined : record.notCopied.find(({ prefix }) => entry.from.startsWith(prefix));
  if (refused !== undefined) problems.push(`${path}: its upstream path ${entry.from} is under \`${refused.prefix}\`, which ${recordLabel} lists as Not copied`);
  return problems;
}

/** Each entry checked against the files and the records; returns the files map's entries and the projects they use. */
function checkedEntries(entries, context) {
  const { pack, files, projects, problems } = context;
  const checked = new Map();
  const used = new Set();
  for (const [path, entry] of pairs(entries)) {
    const shape = shapeProblem(path, entry);
    if (shape !== undefined) {
      problems.push(shape);
      continue;
    }
    if (!files.has(path)) problems.push(`${path}: an entry for a file the tool directory does not hold`);
    if (entry.renamedFrom !== undefined && !files.has(entry.renamedFrom)) problems.push(`${path}: renamed from ${entry.renamedFrom}, which the tool directory does not hold`);
    // Keys in ENTRY_KEYS order, undefined ones left out.
    checked.set(path, Object.fromEntries(ENTRY_KEYS.filter((key) => entry[key] !== undefined).map((key) => [key, entry[key]])));
    if (entry.source === RERAIL) continue;
    const record = projects.get(entry.source);
    if (record === undefined) {
      problems.push(`${path}: source ${JSON.stringify(entry.source)} is neither ${RERAIL} nor a directory of packs/${pack}/licenses/ holding an UPSTREAM.md`);
      continue;
    }
    used.add(entry.source);
    problems.push(...upstreamProblems(path, entry, record, { ...context, recordLabel: `packs/${pack}/licenses/${entry.source}/UPSTREAM.md` }));
  }
  return { checked, used };
}

/** Rerail's files without an entry: authored (listed, or under authored/) or recorded; anything else is a problem. */
function rerailEntries(files, checked, authored, problems) {
  const own = new Map();
  for (const path of files) {
    if (checked.has(path)) continue;
    if (authored.has(path) || AUTHORED_DIRECTORY.test(path)) own.set(path, { source: RERAIL, how: "authored" });
    else if (RECORDED.some((pattern) => pattern.test(path))) own.set(path, { source: RERAIL, how: "recorded" });
    else problems.push(`${path}: pack test data with no entry, not listed as Rerail's in the tool's UPSTREAM.md, not under authored/ and no recorded name`);
  }
  return own;
}

/** The sources of the projects used, checking each one's globs under the tool and its commit (refusals 6 and 7). */
function usedSources(used, { pack, tool, files, projects, commits, problems }) {
  const sources = new Map();
  const commitOf = new Map(pairs(commits));
  for (const project of [...used].sort(byBytes)) {
    const record = projects.get(project);
    const label = `packs/${pack}/licenses/${project}/UPSTREAM.md`;
    if (record.source === undefined) {
      problems.push(`${label}: no Source row (a repository URL row under \`## Source\`)`);
      continue;
    }
    for (const { glob } of record.taken.filter(({ glob }) => glob.startsWith(`upstream/${tool}/`))) {
      const matcher = pathGlob(glob);
      if (![...files].some((path) => matcher.test(`upstream/${tool}/${path}`))) problems.push(`${label}: its "Taken from it" glob \`${glob}\` matches no file`);
    }
    const commit = commitOf.get(project);
    if (commit === undefined) problems.push(`${project}: no commit given for it (the commit the generator checked out)`);
    else if (commit !== record.source.commit) problems.push(`${project}: the generator checked out ${commit}, but ${label}'s Source row names ${record.source.commit}`);
    sources.set(project, sourceOf(record.source));
  }
  sources.set(RERAIL, { license: RERAIL_LICENCE });
  return sources;
}

/** Each licence file of a licenses/<project>/ (all but UPSTREAM.md), byte-sorted, with its text. */
function licenceTexts(directory) {
  return filesBelow(directory)
    .filter((file) => file !== "UPSTREAM.md")
    .sort(byBytes)
    .map((file) => ({ file, text: readFileSync(join(directory, file), "utf8") }));
}

/** A fence one backtick longer than the longest run in `text`, and at least three. */
const fence = (text) => "`".repeat(Math.max(2, ...[...text.matchAll(/`+/gu)].map((match) => match[0].length)) + 1);

/** "2 verbatim, 1 renamed": how many entries of each `how`, in HOW_ORDER. */
function counts(entries) {
  const tally = new Map();
  for (const entry of entries) tally.set(entry.how, (tally.get(entry.how) ?? 0) + 1);
  return HOW_ORDER.filter((how) => tally.has(how))
    .map((how) => `${tally.get(how)} ${how}`)
    .join(", ");
}

/** provenance.json's text: format, pack, tool, sources and files, keys byte-sorted. */
function provenanceJson(pack, tool, sources, files) {
  const sortedSources = Object.fromEntries([...sources].sort(([a], [b]) => byBytes(a, b)));
  const sortedFiles = Object.fromEntries([...files].sort(([a], [b]) => byBytes(a, b)));
  return `${JSON.stringify({ format: PROVENANCE_FORMAT, pack, tool, sources: sortedSources, files: sortedFiles }, null, 2)}\n`;
}

/** LICENSES.md's text: each upstream source with its counts and licence texts, then Rerail's files. */
function licensesMarkdown(pack, tool, sources, projects, files) {
  const entries = [...files.values()];
  const lines = [
    `# Licences of the ${tool} test data`,
    "",
    `This archive is the test data of \`packs/${pack}/upstream/${tool}/\`. \`${PROVENANCE}\` beside this file lists every file in it with its source, its upstream path and how it was made. ${sources.size > 1 ? `Each upstream source below is listed with its licence, how many files come from it, and its licence text as the repository tracks it under \`packs/${pack}/licenses/\`; the rest is Rerail's.` : "Every file in it is Rerail's."}`,
    "",
  ];
  for (const [id, source] of [...sources].sort(([a], [b]) => byBytes(a, b))) {
    if (id === RERAIL) continue;
    const taken = entries.filter((entry) => entry.source === id);
    lines.push(
      `## ${id}`,
      "",
      `Repository ${source.repository}${source.tag === undefined ? "" : `, tag \`${source.tag}\``}, commit \`${source.commit}\`. License: ${source.license}.`,
      "",
      `${taken.length} file(s): ${counts(taken)}. They remain under ${id}'s licence. Files marked \`extracted\`, \`adapted\` or \`renamed\` were changed by Rerail as \`${PROVENANCE}\` describes: \`extracted\` is one test case cut out of the upstream file it names (\`detail\` locates the case), \`adapted\` is such a case wrapped so the tool reads it, and \`renamed\` is the file \`renamedFrom\` names with its names renamed (literals and the names' order kept). \`verbatim\` files are unchanged copies.`,
      "",
    );
    for (const { file, text } of licenceTexts(projects.get(id).directory)) lines.push(`### ${file}`, "", fence(text), text.trimEnd(), fence(text), "");
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

/**
 * The texts of `packs/<pack>/upstream/<tool>/`'s provenance.json and LICENSES.md, from the generator's `entries`
 * (tool-relative path → `{ source, from?, how, renamedFrom?, detail? }`, a Map or an object) and `commits` (project →
 * the commit it checked out), against the pack test data on disk under `root` and the pack's tracked licence records.
 * Throws one error listing every problem.
 */
export function provenanceTexts({ root = ROOT, pack, tool, entries, commits }) {
  const directory = `packs/${pack}/upstream/${tool}`;
  const context = { pack, tool, commits, problems: [], files: toolFiles(root, directory), authored: authoredFiles(root, directory), projects: projectRecords(root, pack) };
  const { checked, used } = checkedEntries(entries, context);
  const own = rerailEntries(context.files, checked, context.authored, context.problems);
  const sources = usedSources(used, context);
  if (context.problems.length > 0) {
    throw new Error(`${directory}: refusing to write ${PROVENANCE} and ${LICENSES}, ${context.problems.length} problem(s):\n${context.problems.map((problem) => `  - ${problem}`).join("\n")}`);
  }
  const files = new Map([...checked, ...own]);
  return { provenance: provenanceJson(pack, tool, sources, files), licenses: licensesMarkdown(pack, tool, sources, context.projects, files) };
}

/** Writes provenanceTexts' two files into `packs/<pack>/upstream/<tool>/` under `root`; on any problem it writes neither. */
export function writeProvenance(options) {
  const { provenance, licenses } = provenanceTexts(options);
  const directory = join(options.root ?? ROOT, "packs", options.pack, "upstream", options.tool);
  writeFileSync(join(directory, PROVENANCE), provenance);
  writeFileSync(join(directory, LICENSES), licenses);
}

/**
 * The entries of a tool's rename variants: for every unit of withOrigins' result (`{ files, units }`) whose
 * `units[key].renamedFrom` names its original, each of its paths (relative to fixtures/) gets its original's entry
 * from `originals` (tool-relative keys, `fixtures/<path>`) with `how: "renamed"` and `renamedFrom` the original's
 * tool-relative path; `override(path, entry)` may return another entry for a path (a manifest copied verbatim).
 * Throws when an original has no entry.
 */
export function variantEntries(originals, recorded, override = (_path, entry) => entry) {
  const variants = new Map();
  for (const unit of recorded.files) {
    const paths = typeof unit === "string" ? [unit] : unit;
    const of = recorded.units[paths[0]]?.renamedFrom;
    if (of === undefined) continue;
    if (!paths[0].endsWith(of)) throw new Error(`fixtures/${paths[0]}: a variant whose path does not end in its original's, ${of}`);
    const prefix = paths[0].slice(0, paths[0].length - of.length);
    for (const path of paths) {
      const renamedFrom = `${FIXTURES}/${path.slice(prefix.length)}`;
      const original = originals.get(renamedFrom);
      if (original === undefined) throw new Error(`${FIXTURES}/${path}: a rename variant of ${renamedFrom}, which has no entry`);
      const variant = `${FIXTURES}/${path}`;
      variants.set(variant, override(variant, { ...original, how: "renamed", renamedFrom }));
    }
  }
  return variants;
}

/** Fails a derivation whose generator wrote no provenance: either output missing from `directory` (repository path) under `root`. */
export function assertProvenanceWritten(directory, root = ROOT) {
  const missing = [PROVENANCE, LICENSES].filter((name) => !existsSync(join(root, directory, name)));
  if (missing.length > 0) throw new Error(`${directory}: the generator wrote no provenance (${missing.join(" and ")} missing): it must call writeProvenance or write_provenance`);
}

/** The CLI's input: `{ entries, commits }`, both objects. */
function parsedInput(text) {
  const input = JSON.parse(text);
  const isRecord = (value) => value !== null && typeof value === "object" && !Array.isArray(value);
  if (!isRecord(input) || !isRecord(input.entries) || !isRecord(input.commits)) throw new Error('stdin must be {"entries": {…}, "commits": {…}}');
  return input;
}

const USAGE = "usage: node scripts/pack-data/provenance.mjs write --pack <id> --tool <tool> [--root <checkout>] < {\"entries\":…,\"commits\":…}";

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    const { values, positionals } = parseArgs({ options: { pack: { type: "string" }, tool: { type: "string" }, root: { type: "string" } }, allowPositionals: true });
    if (positionals.join(" ") !== "write" || values.pack === undefined || values.tool === undefined) throw new Error(USAGE);
    const { entries, commits } = parsedInput(readFileSync(0, "utf8"));
    writeProvenance({ root: resolve(values.root ?? ROOT), pack: values.pack, tool: values.tool, entries, commits });
  } catch (error) {
    console.error(`provenance: ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 1;
  }
}

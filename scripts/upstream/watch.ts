/**
 * `npm run upstream:watch`: the nightly report (upstream-releases.yml). For each gated tool-pack it compares Rerail's
 * pinned upstream commit with upstream's latest stable release and says what changed there that Rerail uses. It only
 * reports; a maintainer starts a bump by hand (upstream-bump-start.yml).
 *
 * Per tool-pack:
 * 1. The latest stable release, from the registry `upstream:releases` reads (releases.ts), and how many releases and
 *    what kind of gap (breaking or not) lie between it and the pin.
 * 2. The tag and commit of the pin and of that release (bump.ts's resolveRelease, which writes nothing).
 * 3. The files the tool-pack's `watch:` block names (gates.yaml, conformance/languages/watch.ts), read at both commits
 *    through GitHub's trees API (`git/trees/<commit>:<dir>`) and compared by blob sha, so an added, changed, deleted or
 *    renamed file shows however many files the release touched (watch-compare.ts):
 *    - (a) the tests the generator reads, an ESLint-family file with its RuleTester case diff (rule-tester.ts);
 *    - (b) the ported rules' code, and which rules;
 *    - (c) the code the rules share, a weaker signal of its own: listed, but alone it leaves the status
 *      `behind-irrelevant`;
 *    - (d) tests upstream added for a ported rule that the generator does not read.
 * 4. Its status: `current` (at the latest release), `behind-irrelevant` (behind, and nothing it watches changed but
 *    shared code), `changed`, or `error` when it could not be read; one tool-pack's error never stops the others.
 *
 * Writes the Markdown summary (one row per tool-pack, then the details) to `--summary <file>`, or appends it to
 * `$GITHUB_STEP_SUMMARY`, and to stdout; `--json <file>` gets one entry per tool-pack. `--pack <p> --tool <t>` reports
 * one. A token (`GH_TOKEN` or `GITHUB_TOKEN`) raises GitHub's rate limit. Exit 0 whatever upstream did, 2 on bad
 * arguments or the report's own failure.
 */
import { appendFileSync, mkdirSync, realpathSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { z } from "zod";

import { gatedToolPacks, readGatesManifest } from "../../conformance/languages/manifest.js";
import { expandWatch } from "../../conformance/languages/watch.js";
import { resolveRelease, type Release, type Sources } from "./bump.js";
import { githubClient, type FileChange, type GitHubClient } from "./github.js";
import { BREAKING_GAPS, compareReleases, gatedPin, registry, releaseGap, releaseParts, type Ecosystem, type Fetch } from "./releases.js";
import type { CaseCounts } from "./rule-tester.js";
import { compareWatch, type WatchChanges } from "./watch-compare.js";

export type WatchStatus = "current" | "behind-irrelevant" | "changed" | "error";

/** One tool-pack's report: the JSON entry of `--json`, and a row of the summary. */
export interface WatchReport extends WatchChanges {
  toolPack: string;
  status: WatchStatus;
  /** The pin; its tag is null when the check failed before reading it. */
  pinned: { version: string; tag: string | null; commit: string };
  /** Upstream's latest stable release; null when it could not be read. */
  latest: Release | null;
  /** How many stable releases are newer than the pin, up to the latest; null when unknown. */
  releasesBehind: number | null;
  breaking: boolean;
  /** How a release was chosen (Clippy's source at a Rust point release). */
  notes: string[];
  error?: string;
}

/** How long one registry request may take. */
const REQUEST_TIMEOUT_MS = 60_000;

const npmVersionsSchema = z.object({ versions: z.record(z.string(), z.unknown()) });
const pypiVersionsSchema = z.object({ releases: z.record(z.string(), z.array(z.object({ yanked: z.boolean().default(false) }))) });
const packagistVersionsSchema = z.object({ packages: z.record(z.string(), z.array(z.object({ version: z.string() }))) });

/**
 * How many stable releases a registry's answer (`body`, as releases.ts's `registry` fetches it) lists after `pinned` up to
 * `latest`. Rust's channel manifest names one release, and Clippy follows Rust's minor releases: the minors between.
 */
export function releasesBetween(ecosystem: Ecosystem, name: string, body: string, pinned: string, latest: string): number {
  const [from, to] = [releaseParts(pinned), releaseParts(latest)];
  if (from === null || to === null) throw new Error(`${pinned} or ${latest} is not a release version`);
  if (ecosystem === "rust") return Math.max(0, (to[0]! - from[0]!) * 1000 + (to[1]! - from[1]!));
  const versions = ecosystem === "npm" ? Object.keys(npmVersionsSchema.parse(JSON.parse(body)).versions)
    : ecosystem === "pypi" ? Object.entries(pypiVersionsSchema.parse(JSON.parse(body)).releases).filter(([, files]) => files.some((file) => !file.yanked)).map(([version]) => version)
      : (packagistVersionsSchema.parse(JSON.parse(body)).packages[name] ?? []).map((entry) => entry.version);
  const between = new Set<string>();
  for (const version of versions) {
    const parts = releaseParts(version);
    if (parts !== null && compareReleases(parts, from) > 0 && compareReleases(parts, to) <= 0) between.add(parts.join("."));
  }
  return between.size;
}

/** The registry's latest stable release of a tool-pack's upstream, and how many releases it is past the pin. */
async function latestRelease(fetch: Fetch, ecosystem: Ecosystem, name: string, pinned: string): Promise<{ version: string; behind: number }> {
  const source = registry(ecosystem, name);
  const response = await fetch(source.url, { headers: { accept: source.accept }, signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS) });
  if (!response.ok) throw new Error(`${source.url}: HTTP ${response.status}`);
  const body = await response.text();
  const version = source.latest(body);
  return { version, behind: releasesBetween(ecosystem, name, body, pinned, version) };
}

/** What watchToolPack reads beyond the checkout. */
export interface WatchSources extends Sources {
  github: GitHubClient;
}

const NO_CHANGES: WatchChanges = { tests: { files: [], cases: null }, code: { rules: [], files: [] }, shared: { files: [] }, newTests: [] };

/** One tool-pack's report; an error is its row's, never thrown. */
export async function watchToolPack(root: string, pack: string, tool: string, sources: WatchSources): Promise<WatchReport> {
  const toolPack = `${pack}/${tool}`;
  let report: WatchReport = { toolPack, status: "error", pinned: { version: "", tag: null, commit: "" }, latest: null, releasesBehind: null, breaking: false, notes: [], ...NO_CHANGES };
  try {
    const manifest = readGatesManifest(root, pack, tool);
    const pin = gatedPin(root, pack, tool);
    report = { ...report, pinned: { version: pin.version, tag: null, commit: manifest.upstream.commit } };
    const watch = expandWatch(root, pack, tool, manifest);
    if (watch === undefined) throw new Error(`packs/${pack}/upstream/${tool}/gates.yaml has no watch: block`);
    const latest = await latestRelease(sources.fetch, pin.ecosystem, pin.package, pin.version);
    const [pinnedParts, latestParts] = [releaseParts(pin.version), releaseParts(latest.version)];
    if (pinnedParts === null || latestParts === null) throw new Error(`${pin.version} or ${latest.version} is not a release version`);
    const gap = releaseGap(pin.ecosystem, pinnedParts, latestParts);
    const current = gap === "current";
    const resolved = await resolveRelease(root, pack, tool, current ? pin.version : latest.version, sources);
    report = { ...report, pinned: resolved.from, latest: resolved.to, releasesBehind: current ? 0 : latest.behind, breaking: BREAKING_GAPS[gap], notes: resolved.notes };
    if (current) return { ...report, status: "current" };
    const changes = await compareWatch(sources.github, { repository: resolved.repository, ruleTester: manifest.adapter === "eslint", watch, from: resolved.from.commit, to: resolved.to.commit });
    const relevant = changes.tests.files.length + changes.code.files.length + changes.newTests.length > 0;
    return { ...report, ...changes, status: relevant ? "changed" : "behind-irrelevant" };
  } catch (error) {
    return { ...report, status: "error", error: error instanceof Error ? error.message : String(error) };
  }
}

/** Every gated tool-pack's report, in tool-pack order. */
export async function watchUpstreams(root: string, toolPacks: { pack: string; tool: string }[], sources: WatchSources): Promise<WatchReport[]> {
  return await Promise.all(toolPacks.map(({ pack, tool }) => watchToolPack(root, pack, tool, sources)));
}

const STATUS_LABELS: Record<WatchStatus, string> = {
  current: "up to date",
  "behind-irrelevant": "behind, nothing relevant changed",
  changed: "changed",
  error: "error",
};

/** The most items one list of the details shows. */
const DETAIL_LIMIT = 40;
/** The most rules the table's rule code cell names. */
const CELL_RULES = 6;

function casesText(cases: CaseCounts): string {
  return `+${cases.added} −${cases.removed} ~${cases.changed}${cases.unresolved > 0 ? ` (${cases.unresolved} unresolved)` : ""}`;
}

function filesText(count: number): string {
  return count === 1 ? "1 file" : `${count} files`;
}

/** A report's table row. */
function row(report: WatchReport): string {
  const version = report.latest === null || report.latest.version === report.pinned.version ? `${report.pinned.version || "?"}` : `${report.pinned.version} → ${report.latest.version}`;
  const behind = report.releasesBehind === null ? "?" : `${report.releasesBehind}${report.breaking ? " (breaking)" : ""}`;
  if (report.status === "error" || report.status === "current") return `| \`${report.toolPack}\` | ${STATUS_LABELS[report.status]} | ${version} | ${behind} | | | | |`;
  const tests = report.tests.files.length === 0 ? "—" : `${filesText(report.tests.files.length)}${report.tests.cases === null ? "" : `, ${casesText(report.tests.cases)}`}`;
  const named = report.code.rules.slice(0, CELL_RULES).map((rule) => `\`${rule}\``).join(", ");
  const code = report.code.files.length === 0 ? "—" : report.code.rules.length === 0 ? filesText(report.code.files.length) : `${named}${report.code.rules.length > CELL_RULES ? `, … ${report.code.rules.length - CELL_RULES} more` : ""}`;
  const shared = report.shared.files.length === 0 ? "—" : filesText(report.shared.files.length);
  const fresh = report.newTests.length === 0 ? "—" : filesText(new Set(report.newTests.map((entry) => entry.path)).size);
  return `| \`${report.toolPack}\` | ${STATUS_LABELS[report.status]} | ${version} | ${behind} | ${tests} | ${code} | ${shared} | ${fresh} |`;
}

function changeText(file: FileChange): string {
  return `\`${file.path}\` (${file.change})`;
}

/** A details list: its title and count, then its first DETAIL_LIMIT items nested below. */
function list<T>(title: string, items: T[], render: (item: T) => string): string[] {
  if (items.length === 0) return [];
  const more = items.length > DETAIL_LIMIT ? [`  - … ${items.length - DETAIL_LIMIT} more`] : [];
  return [`- ${title} (${items.length}):`, ...items.slice(0, DETAIL_LIMIT).map((item) => `  - ${render(item)}`), ...more];
}

/** A report's details: what changed, what upstream added, how its release was chosen, or what went wrong. */
function details(report: WatchReport): string[] {
  return [
    `#### \`${report.toolPack}\`: ${STATUS_LABELS[report.status]}`,
    "",
    ...(report.error === undefined ? [] : [`- Error: ${report.error.replace(/\s+/gu, " ")}`]),
    ...(report.latest === null || report.pinned.tag === null ? [] : [`- ${report.pinned.tag} (\`${report.pinned.commit.slice(0, 12)}\`) → ${report.latest.tag} (\`${report.latest.commit.slice(0, 12)}\`)`]),
    ...report.notes.map((note) => `- ${note}`),
    ...list("Tests the generator reads", report.tests.files, (file) => `${changeText(file)}${file.cases === null ? "" : ` ${casesText(file.cases)}`}`),
    ...list("Rule code", report.code.files, (file) => `${changeText(file)}${file.rules.length === 0 ? "" : ` for ${file.rules.map((rule) => `\`${rule}\``).join(", ")}`}`),
    ...list("Shared code", report.shared.files, changeText),
    ...list("New tests the generator does not read", report.newTests, (entry) => `\`${entry.path}\`${entry.rule === null ? "" : ` for \`${entry.rule}\``}`),
    "",
  ];
}

/** The reports as Markdown: one table row per tool-pack, then the details of each that is not current. */
export function renderWatch(reports: WatchReport[]): string {
  const lines = [
    "## Upstream releases",
    "",
    "What changed upstream since each tool-pack's pin, in what Rerail reads (gates.yaml `watch:`). Report only: start a bump by hand (docs/upstream.md).",
    "",
    "| Tool-pack | Status | Pinned → latest | Releases behind | Tests read (cases +/−/~) | Rule code | Shared code | New tests not read |",
    "|---|---|---|---|---|---|---|---|",
    ...reports.map(row),
    "",
  ];
  const noted = reports.filter((report) => report.status !== "current");
  if (noted.length > 0) lines.push("### Details", "", ...noted.flatMap(details));
  return `${lines.join("\n")}\n`;
}

const USAGE = "usage: npm run upstream:watch -- [--pack <pack> --tool <tool>] [--json <file>] [--summary <file>]\n";

/** Run the report over the checkout at `root`; the exit code. */
export async function main(argv: string[], root: string, fetch: Fetch, env: NodeJS.ProcessEnv = process.env, github?: GitHubClient): Promise<number> {
  let values;
  try {
    ({ values } = parseArgs({ args: argv, options: { pack: { type: "string" }, tool: { type: "string" }, json: { type: "string" }, summary: { type: "string" }, help: { type: "boolean", default: false } }, strict: true }));
  } catch (error) {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n${USAGE}`);
    return 2;
  }
  if (values.help || (values.pack === undefined) !== (values.tool === undefined)) {
    process.stderr.write(USAGE);
    return values.help ? 0 : 2;
  }
  const toolPacks = gatedToolPacks(root).filter(({ pack, tool }) => values.pack === undefined || (pack === values.pack && tool === values.tool));
  if (toolPacks.length === 0) {
    process.stderr.write(`no gated tool-pack ${values.pack}/${values.tool}\n`);
    return 2;
  }
  const token = env.GH_TOKEN ?? env.GITHUB_TOKEN;
  const base: Sources = { fetch, token };
  const reports = await watchUpstreams(root, toolPacks, { ...base, github: github ?? githubClient(base) });
  const markdown = renderWatch(reports);
  process.stdout.write(markdown);
  const write = (file: string, text: string): void => {
    mkdirSync(dirname(resolve(file)), { recursive: true });
    writeFileSync(resolve(file), text);
  };
  if (values.json !== undefined) write(values.json, `${JSON.stringify(reports, null, 2)}\n`);
  if (values.summary !== undefined) write(values.summary, markdown);
  else if (env.GITHUB_STEP_SUMMARY !== undefined && env.GITHUB_STEP_SUMMARY !== "") appendFileSync(env.GITHUB_STEP_SUMMARY, markdown);
  if (env.GITHUB_ACTIONS === "true") {
    // A workflow command's data escapes `%`, CR and LF.
    for (const report of reports.filter((entry) => entry.status === "error")) process.stdout.write(`::warning title=Upstream watch (${report.toolPack})::${(report.error ?? "").replace(/%/gu, "%25").replace(/\r/gu, "%0D").replace(/\n/gu, "%0A")}\n`);
  }
  return 0;
}

// Imported by its tests, run as a script by the nightly: only the second runs.
const invoked = (() => {
  try {
    return process.argv[1] !== undefined && realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
})();
if (invoked) {
  main(process.argv.slice(2), resolve(dirname(fileURLToPath(import.meta.url)), "..", ".."), globalThis.fetch).then((code) => {
    process.exitCode = code;
  }, (error: unknown) => {
    process.stderr.write(`scripts/upstream/watch.ts: ${error instanceof Error ? error.message : String(error)}\n`);
    process.exitCode = 2;
  });
}

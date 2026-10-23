/**
 * `npm run port-readiness:bump`: move one tool-pack's pins to a newer upstream release. With every tool-pack
 * `port-readiness:upstream-releases` finds behind, the nightly (port-readiness-nightly.yml) dispatches
 * port-readiness-bump.yml once on the work branch auto/bump-run/upstream, whose leg per tool-pack runs it, then on Linux
 * derives and records again every tool-pack the bump made stale and packs each one's test data into the archive its
 * test-data.lock names; that upstream code at an unreviewed release runs with no OIDC or write token, and outside main's
 * cache scope. port-readiness-bump-publish.yml then, on main, runs it again for each leg, composes the verified legs into
 * a commit per tool-pack carrying its pins, test-data.lock and recording.lock files together, uploads the archives to S3
 * and opens one pull request, which a maintainer reviews and merges. A maintainer can run it the same way, and
 * regenerate the rest as the pull request's "Regenerate locally" section says.
 *
 * A gated tool-pack (`packs/<pack>/upstream/<tool>/gates.yaml`) pins its upstream in several places, all moved together:
 * - gates.yaml's `upstream.version` and `upstream.commit`, the commit being the one the release's tag names on GitHub
 *   (an annotated tag is followed to its commit), and an ESLint upstream's `eslint.release.tag`;
 * - the runtime's pin file under conformance/recorders/port-readiness/runtimes/<upstream.runtime>/ (package.json,
 *   pyproject.toml or composer.json): upstream's package, and on npm every package of the same scope pinned at the same
 *   release (typescript-eslint publishes its packages together). A runtime directory named `<name>-<version>` is
 *   renamed to the new version, and every gates.yaml naming it follows;
 * - conformance.json's `tools[<tool>].version`;
 * - the generators' pins (`packs/<pack>/upstream/<tool>/*` and `packs/<pack>/upstream/*` scripts): the old commit, the
 *   npm `gitHead` of the old release where a generator pins that instead, a `...VERSION = "<old>"` constant of the
 *   tool's own scripts, and an object entry keyed by upstream's package (`"eslint-plugin-react": "7.37.5"`).
 *
 * The Rust pack's Clippy ships with Rust: its pins are packs/rust/upstream/common.py's `TOOLCHAIN`, `RUSTC_VERSION`,
 * `CLIPPY_VERSION` and `CLIPPY_COMMIT` (rust-lang/rust-clippy at tag `rust-<release>`), read from the release's channel
 * manifest, conformance.json's `tools[clippy].version`, gates.yaml's `upstream` commit, version and runtime, and the
 * runtime's `rust-toolchain.toml` (its directory named `clippy-<release>`).
 *
 * How upstream tags a release is learnt from the pin itself: the first of `eslint.release.tag`'s form, `v<version>` and
 * `<version>` whose tag of the pinned release names the pinned commit. Every edit is checked by reading the file back,
 * so a pin in an unexpected shape stops the bump instead of being half-moved. Prose still naming the old release (READMEs,
 * comments, UPSTREAM.md headers) is listed, not rewritten: the pull request's reviewer decides it.
 *
 * Exit 0 when bumped or already at the release, 1 when the bump cannot be made, 2 on bad arguments.
 */
import { existsSync, mkdirSync, readFileSync, readdirSync, realpathSync, renameSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { isDeepStrictEqual, parseArgs } from "node:util";
import { parse as parseToml } from "smol-toml";
import { z } from "zod";

import { parseGatesManifest, readGatesManifest, recordedRuntimes, type GatesManifest } from "../manifest.js";
import { RUNTIMES_DIRECTORY } from "../recorded-adapter.js";
import { gatedToolPacks } from "../run.js";
import {
  ECOSYSTEM_NAMES,
  PIN_FILES,
  PYTHON_PIN,
  clippyPin,
  compareReleases,
  gatedPin,
  normalPythonName,
  registry,
  releaseParts,
  type Ecosystem,
  type Fetch,
} from "./releases.js";

/** A release as the pins name it: the version, the GitHub tag it was cut from, and the commit the tag names. */
export interface Release {
  version: string;
  tag: string;
  commit: string;
}

/** One file the bump rewrites, at its path after any runtime directory rename. */
export interface BumpFile {
  path: string;
  before: string;
  after: string;
}

/** A tool-pack's bump, planned but not yet written. */
export interface Bump {
  /** `<pack>/<tool>`. */
  toolPack: string;
  pack: string;
  tool: string;
  ecosystem: Ecosystem;
  package: string;
  /** `<owner>/<name>` on GitHub. */
  repository: string;
  from: Release;
  to: Release;
  files: BumpFile[];
  /** Runtime directories renamed, repository-relative. */
  renames: { from: string; to: string }[];
  /** The gated tool-packs (`<pack>/<tool>`) whose recordings the bump makes stale: each installs a runtime it changed. */
  record: string[];
  /** Lines that still name the old release once the bump is written: `<path>:<line>: <text>`. */
  leftovers: string[];
  /** Anything the reviewer should know about how a pin was chosen. */
  notes: string[];
}

/** How the bump reaches GitHub, the registries and Rust's channel manifests; tests pass recorded answers. */
export interface Sources {
  fetch: Fetch;
  /** A GitHub token for the API's higher rate limit; public repositories need none. */
  token?: string | undefined;
}

/** How long one request may take. */
const REQUEST_TIMEOUT_MS = 60_000;

const GITHUB_API = "https://api.github.com";

/** A regular expression matching `text` literally. */
function literal(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
}

/** `text` with each match of `pattern` replaced; throws unless there were exactly `expected` matches (any number when omitted). */
function replaceCounted(text: string, pattern: RegExp, replace: (...groups: string[]) => string, label: string, expected?: number): string {
  let count = 0;
  const result = text.replace(pattern, (...groups: string[]) => {
    count += 1;
    return replace(...groups);
  });
  if (expected !== undefined && count !== expected) throw new Error(`${label}: expected ${expected} match(es) of ${pattern.source}, found ${count}`);
  return result;
}

/** The JSON a URL answers, or null on 404. */
async function getText(sources: Sources, url: string, headers: Record<string, string>): Promise<string | null> {
  const response = await sources.fetch(url, { headers, signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS) });
  if (response.status === 404) return null;
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
  return await response.text();
}

function githubHeaders(sources: Sources): Record<string, string> {
  return {
    accept: "application/vnd.github+json",
    "x-github-api-version": "2022-11-28",
    ...(sources.token === undefined || sources.token === "" ? {} : { authorization: `Bearer ${sources.token}` }),
  };
}

const gitObjectSchema = z.object({ object: z.object({ type: z.string(), sha: z.string().regex(/^[0-9a-f]{40}$/u) }) });

/** The commit `tag` names in `repository` (`<owner>/<name>`), an annotated tag followed to its commit; null when there is no such tag. */
export async function tagCommit(sources: Sources, repository: string, tag: string): Promise<string | null> {
  const ref = tag.split("/").map(encodeURIComponent).join("/");
  const body = await getText(sources, `${GITHUB_API}/repos/${repository}/git/ref/tags/${ref}`, githubHeaders(sources));
  if (body === null) return null;
  const answer: unknown = JSON.parse(body);
  // Refs that only start with the name come back as a list: not this tag.
  if (Array.isArray(answer)) return null;
  let { object } = gitObjectSchema.parse(answer);
  for (let depth = 0; object.type === "tag"; depth += 1) {
    if (depth >= 5) throw new Error(`${repository} tag ${tag}: more than 5 annotated tags deep`);
    const annotated = await getText(sources, `${GITHUB_API}/repos/${repository}/git/tags/${object.sha}`, githubHeaders(sources));
    if (annotated === null) throw new Error(`${repository} tag ${tag}: its tag object ${object.sha} is missing`);
    object = gitObjectSchema.parse(JSON.parse(annotated)).object;
  }
  if (object.type !== "commit") throw new Error(`${repository} tag ${tag} names a ${object.type}, not a commit`);
  return object.sha;
}

/** `<owner>/<name>` of a gates.yaml repository URL. */
export function githubRepository(url: string): string {
  const match = /^https:\/\/github\.com\/([^/]+\/[^/]+?)(?:\.git)?\/?$/u.exec(url);
  if (match === null) throw new Error(`${url} is not a GitHub repository`);
  return match[1]!;
}

/** The forms of a release's tag to try, `{version}` standing for the version: an ESLint upstream's own tag's form first. */
export function tagTemplates(manifest: GatesManifest): string[] {
  const templates: string[] = [];
  const tag = manifest.eslint?.release.tag;
  if (tag !== undefined && tag.includes(manifest.upstream.version)) templates.push(tag.replace(manifest.upstream.version, "{version}"));
  templates.push("v{version}", "{version}");
  return [...new Set(templates)];
}

function fillTag(template: string, version: string): string {
  return template.replace("{version}", version);
}

/** The tag form whose tag of the pinned release names the pinned commit: how this upstream tags releases. */
async function pinnedTagTemplate(sources: Sources, repository: string, templates: string[], pinned: { version: string; commit: string }): Promise<string> {
  const tried: string[] = [];
  for (const template of templates) {
    const tag = fillTag(template, pinned.version);
    const commit = await tagCommit(sources, repository, tag);
    if (commit === pinned.commit) return template;
    tried.push(`${tag} (${commit ?? "no such tag"})`);
  }
  throw new Error(`no tag of ${repository} names the pinned commit ${pinned.commit} of ${pinned.version}, so how it tags releases is unknown: tried ${tried.join(", ")}`);
}

/** The `gitHead` npm recorded for a release, or null. */
async function npmGitHead(sources: Sources, name: string, version: string): Promise<string | null> {
  const body = await getText(sources, `https://registry.npmjs.org/${name.replace("/", "%2f")}/${version}`, { accept: "application/json" });
  if (body === null) throw new Error(`npm has no ${name}@${version}`);
  const gitHead = z.object({ gitHead: z.string().regex(/^[0-9a-f]{40}$/u).optional() }).parse(JSON.parse(body)).gitHead;
  return gitHead ?? null;
}

/** Throws unless `version` is a release newer than `pinned`; false when it is the pinned release. */
function isNewer(pinned: string, version: string, label: string): boolean {
  const target = releaseParts(version);
  if (target === null) throw new Error(`${version} is not a release version`);
  const current = releaseParts(pinned);
  if (current === null) throw new Error(`${label} pins ${pinned}, which is not a release version`);
  const order = compareReleases(target, current);
  if (order < 0) throw new Error(`${label} pins ${pinned}, newer than ${version}`);
  return order > 0;
}

/** The files a bump reads and rewrites, keyed by their path before any rename. */
class Edits {
  private readonly texts = new Map<string, { before: string; after: string }>();

  constructor(private readonly root: string) {}

  read(path: string): string {
    const known = this.texts.get(path);
    if (known !== undefined) return known.after;
    const text = readFileSync(join(this.root, path), "utf8");
    this.texts.set(path, { before: text, after: text });
    return text;
  }

  write(path: string, text: string): void {
    this.read(path);
    this.texts.get(path)!.after = text;
  }

  /** The changed files, renamed directories applied to their paths. */
  changed(renames: { from: string; to: string }[]): BumpFile[] {
    return [...this.texts].filter(([, text]) => text.before !== text.after).map(([path, text]) => {
      const rename = renames.find((entry) => path.startsWith(`${entry.from}/`));
      return { path: rename === undefined ? path : `${rename.to}${path.slice(rename.from.length)}`, ...text };
    }).sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
  }
}

/** The top-level `upstream:` block of a gates.yaml rewritten by `edit`; the rest of the file is left as it is. */
function withUpstreamBlock(text: string, label: string, edit: (block: string) => string): string {
  const lines = text.split("\n");
  const start = lines.findIndex((line) => /^upstream:\s*$/u.test(line));
  if (start < 0) throw new Error(`${label} has no upstream: block`);
  let end = start + 1;
  while (end < lines.length && !/^\S/u.test(lines[end]!)) end += 1;
  return [...lines.slice(0, start), edit(lines.slice(start, end).join("\n")), ...lines.slice(end)].join("\n");
}

/** A YAML scalar line `<indent><key>: <value>`, the value plain or quoted. */
function yamlScalar(indent: string, key: string, value: string): RegExp {
  return new RegExp(`^(${indent}${key}: )(["']?)${literal(value)}\\2[ \\t]*$`, "gmu");
}

/** A gates.yaml's parsed manifest with every `runtime` naming `from` renamed `to`. */
function renamedRuntime(manifest: GatesManifest, from: string, to: string): GatesManifest {
  const copy = structuredClone(manifest);
  if (copy.upstream.runtime === from) copy.upstream.runtime = to;
  if (copy.eslint?.environment?.runtime === from) copy.eslint.environment.runtime = to;
  return copy;
}

/** Checks that `after` reads as `expected`: an edit that moved anything else, or nothing, stops the bump. */
function verify(label: string, after: unknown, expected: unknown): void {
  // Through JSON, so a parser's own object prototypes (TOML's) do not count as a difference.
  if (!isDeepStrictEqual(JSON.parse(JSON.stringify(after)), JSON.parse(JSON.stringify(expected)))) throw new Error(`${label}: the rewritten file does not read back as the bumped pins; bump it by hand`);
}

/**
 * Bump upstream's package (and, on npm, its same-scope siblings at the same release) in a runtime's pin file: the new
 * text, and the packages it moved.
 */
export function bumpRuntimePin(ecosystem: Exclude<Ecosystem, "rust">, file: string, text: string, name: string, from: string, to: string): { text: string; packages: string[] } {
  if (ecosystem === "npm") {
    const before = z.looseObject({ dependencies: z.record(z.string(), z.string()) }).parse(JSON.parse(text));
    if (before.dependencies[name] !== from) throw new Error(`${file} pins ${name} ${before.dependencies[name] ?? "nowhere"}, not ${from}`);
    const scope = name.startsWith("@") ? `${name.split("/")[0]}/` : null;
    const packages = Object.keys(before.dependencies).filter((key) => key === name || (scope !== null && key.startsWith(scope) && before.dependencies[key] === from));
    let after = text;
    for (const key of packages) after = replaceCounted(after, new RegExp(`("${literal(key)}"\\s*:\\s*")${literal(from)}(")`, "gu"), (_, head, tail) => `${head}${to}${tail}`, `${file} ${key}`);
    verify(file, JSON.parse(after), { ...before, dependencies: { ...before.dependencies, ...Object.fromEntries(packages.map((key) => [key, to])) } });
    return { text: after, packages };
  }
  if (ecosystem === "packagist") {
    const before = z.looseObject({ require: z.record(z.string(), z.string()) }).parse(JSON.parse(text));
    const entry = Object.entries(before.require).find(([key]) => key.toLowerCase() === name);
    if (entry === undefined || entry[1].replace(/^v/u, "") !== from) throw new Error(`${file} requires ${name} ${entry?.[1] ?? "nowhere"}, not ${from}`);
    const [key, value] = entry;
    const bumped = value.startsWith("v") ? `v${to}` : to;
    const after = replaceCounted(text, new RegExp(`("${literal(key)}"\\s*:\\s*")${literal(value)}(")`, "gu"), (_, head, tail) => `${head}${bumped}${tail}`, `${file} ${key}`, 1);
    verify(file, JSON.parse(after), { ...before, require: { ...before.require, [key]: bumped } });
    return { text: after, packages: [name] };
  }
  const pyproject = z.looseObject({ project: z.looseObject({ dependencies: z.array(z.string()) }) });
  const before = pyproject.parse(parseToml(text));
  const dependency = before.project.dependencies.find((entry) => {
    const match = PYTHON_PIN.exec(entry.trim());
    return match !== null && normalPythonName(match[1]!) === normalPythonName(name) && match[2] === from;
  });
  if (dependency === undefined) throw new Error(`${file} does not pin ${name}==${from}`);
  const bumped = dependency.replace(/(==\s*)\S+$/u, (_, operator: string) => `${operator}${to}`);
  const after = replaceCounted(text, new RegExp(`(["'])${literal(dependency)}\\1`, "gu"), (_, quote) => `${quote}${bumped}${quote}`, `${file} ${name}`, 1);
  verify(file, parseToml(after), { ...before, project: { ...before.project, dependencies: before.project.dependencies.map((entry) => (entry === dependency ? bumped : entry)) } });
  return { text: after, packages: [name] };
}

/** conformance.json with `tools[<tool>].version` moved from `from` to `to`. */
export function bumpConformanceTool(file: string, text: string, tool: string, from: string, to: string): string {
  const after = replaceCounted(text, new RegExp(`("name"\\s*:\\s*"${literal(tool)}"\\s*,\\s*"version"\\s*:\\s*")${literal(from)}(")`, "gu"), (_, head, tail) => `${head}${to}${tail}`, `${file} tools[${tool}]`, 1);
  const before = z.looseObject({ tools: z.array(z.looseObject({ name: z.string(), version: z.string() })) }).parse(JSON.parse(text));
  verify(file, JSON.parse(after), { ...before, tools: before.tools.map((entry) => (entry.name === tool ? { ...entry, version: to } : entry)) });
  return after;
}

/** The scripts that pin a tool-pack's upstream for its generators: the tool's own, then the pack's shared ones. */
function generatorScripts(root: string, pack: string, tool: string): { own: string[]; shared: string[] } {
  const scripts = (directory: string): string[] => existsSync(join(root, directory))
    ? readdirSync(join(root, directory)).filter((name) => /\.(?:py|mjs|cjs|js|ts|php)$/u.test(name) && statSync(join(root, directory, name)).isFile()).sort().map((name) => `${directory}/${name}`)
    : [];
  return { own: scripts(`packs/${pack}/upstream/${tool}`), shared: scripts(`packs/${pack}/upstream`) };
}

/** An object entry keyed by `name` (quoted, or bare when it is an identifier) whose value is `version`. */
function packageEntry(name: string, version: string): RegExp {
  const key = /^[A-Za-z_$][\w$]*$/u.test(name) ? `(?:"${literal(name)}"|'${literal(name)}'|(?<=[\\s{,])${literal(name)})` : `(?:"${literal(name)}"|'${literal(name)}')`;
  return new RegExp(`(${key}\\s*:\\s*)(["'])${literal(version)}\\2`, "gu");
}

/**
 * `version` as a whole token, a tag's `v` allowed before it (group 1): `v10.11.0` and `eslint@10.11.0` name 10.11.0,
 * `10.11.0.1` and `110.11.0` do not.
 */
function versionToken(version: string, flags: string): RegExp {
  return new RegExp(`(?<![\\w.-])(v?)${literal(version)}(?![\\w-]|\\.\\d)`, flags);
}

/** The most leftover lines the summary lists, and the most characters of each it shows. */
const LEFTOVER_LIMIT = 60;
const LEFTOVER_WIDTH = 200;

/** Every line of `files` that still names `version`. */
function findLeftovers(root: string, edits: Edits, files: string[], version: string, renames: { from: string; to: string }[]): string[] {
  const token = versionToken(version, "u");
  const found: string[] = [];
  for (const path of [...new Set(files)]) {
    if (!existsSync(join(root, path))) continue;
    const rename = renames.find((entry) => path.startsWith(`${entry.from}/`));
    const shown = rename === undefined ? path : `${rename.to}${path.slice(rename.from.length)}`;
    edits.read(path).split("\n").forEach((line, index) => {
      if (token.test(line)) found.push(`${shown}:${index + 1}: ${line.trim().length > LEFTOVER_WIDTH ? `${line.trim().slice(0, LEFTOVER_WIDTH)}…` : line.trim()}`);
    });
  }
  return found;
}

/** The files in `directory` (repository-relative), not descending into fixtures/ or tests/. */
function filesIn(root: string, directory: string): string[] {
  if (!existsSync(join(root, directory))) return [];
  return readdirSync(join(root, directory)).filter((name) => statSync(join(root, directory, name)).isFile()).sort().map((name) => `${directory}/${name}`);
}

/** Plan the bump of a gated tool-pack to `version`; null when it already pins it. */
export async function planGatedBump(root: string, pack: string, tool: string, version: string, sources: Sources): Promise<Bump | null> {
  const toolPack = `${pack}/${tool}`;
  const manifest = readGatesManifest(root, pack, tool);
  const pin = gatedPin(root, pack, tool);
  if (pin.disagreements.length > 0) throw new Error(`${toolPack}: the pins disagree before the bump (${pin.disagreements.join("; ")}); line them up by hand first`);
  if (pin.ecosystem === "rust") throw new Error(`${toolPack} is not pinned on a package registry`);
  const ecosystem = pin.ecosystem;
  if (!isNewer(pin.version, version, toolPack)) return null;

  const repository = githubRepository(manifest.upstream.repository);
  const template = await pinnedTagTemplate(sources, repository, tagTemplates(manifest), { version: pin.version, commit: manifest.upstream.commit });
  const from: Release = { version: pin.version, tag: fillTag(template, pin.version), commit: manifest.upstream.commit };
  const toTag = fillTag(template, version);
  const toCommit = await tagCommit(sources, repository, toTag);
  if (toCommit === null) throw new Error(`${repository} has no tag ${toTag}: ${ECOSYSTEM_NAMES[ecosystem]} publishes ${pin.package} ${version}, but the commit it was cut from is unknown`);
  const to: Release = { version, tag: toTag, commit: toCommit };
  const notes: string[] = [];

  const edits = new Edits(root);
  const renames: { from: string; to: string }[] = [];
  const oldRuntime = manifest.upstream.runtime;
  const newRuntime = oldRuntime.endsWith(`-${from.version}`) ? `${oldRuntime.slice(0, -from.version.length)}${version}` : oldRuntime;
  if (newRuntime !== oldRuntime) {
    if (existsSync(join(root, RUNTIMES_DIRECTORY, newRuntime))) throw new Error(`${RUNTIMES_DIRECTORY}/${newRuntime} already exists`);
    renames.push({ from: `${RUNTIMES_DIRECTORY}/${oldRuntime}`, to: `${RUNTIMES_DIRECTORY}/${newRuntime}` });
  }

  // gates.yaml: the upstream block's pins, the ESLint release tag, and every gates.yaml's name for a renamed runtime.
  const gatesPath = `packs/${pack}/upstream/${tool}/gates.yaml`;
  let gates = withUpstreamBlock(edits.read(gatesPath), gatesPath, (block) => {
    let edited = replaceCounted(block, yamlScalar("  ", "commit", from.commit), (_, head, quote) => `${head}${quote}${to.commit}${quote}`, `${gatesPath} upstream.commit`, 1);
    edited = replaceCounted(edited, yamlScalar("  ", "version", from.version), (_, head, quote) => `${head}${quote}${version}${quote}`, `${gatesPath} upstream.version`, 1);
    return edited;
  });
  const releaseTag = manifest.eslint?.release.tag;
  let newReleaseTag: string | undefined;
  if (releaseTag !== undefined) {
    if (!releaseTag.includes(from.version)) throw new Error(`${gatesPath}: eslint.release.tag ${releaseTag} does not name ${from.version}`);
    newReleaseTag = releaseTag.replace(from.version, version);
    gates = replaceCounted(gates, yamlScalar("    ", "tag", releaseTag), (_, head, quote) => `${head}${quote}${newReleaseTag!}${quote}`, `${gatesPath} eslint.release.tag`, 1);
  }
  edits.write(gatesPath, gates);
  const { id: _id, ...pinned } = manifest;
  const expected = renamedRuntime({
    ...pinned,
    upstream: { ...pinned.upstream, commit: to.commit, version },
    ...(pinned.eslint === undefined || newReleaseTag === undefined ? {} : { eslint: { ...pinned.eslint, release: { ...pinned.eslint.release, tag: newReleaseTag } } }),
  }, oldRuntime, newRuntime);
  const manifests = new Map<string, GatesManifest>();
  for (const other of gatedToolPacks(root)) {
    const path = `packs/${other.pack}/upstream/${other.tool}/gates.yaml`;
    const original = parseGatesManifest(readFileSync(join(root, path), "utf8"), path);
    if (newRuntime !== oldRuntime) edits.write(path, replaceCounted(edits.read(path), new RegExp(`^(\\s+runtime: )(["']?)${literal(oldRuntime)}\\2[ \\t]*$`, "gmu"), (_, head, quote) => `${head}${quote}${newRuntime}${quote}`, `${path} runtime`));
    const after = parseGatesManifest(edits.read(path), path);
    verify(path, after, path === gatesPath ? expected : renamedRuntime(original, oldRuntime, newRuntime));
    manifests.set(`${other.pack}/${other.tool}`, after);
  }

  // The runtime's pin file.
  const pinFile = `${RUNTIMES_DIRECTORY}/${oldRuntime}/${PIN_FILES[ecosystem]}`;
  const runtime = bumpRuntimePin(ecosystem, pinFile, edits.read(pinFile), pin.package, from.version, version);
  edits.write(pinFile, runtime.text);

  // conformance.json.
  const conformancePath = `packs/${pack}/conformance.json`;
  edits.write(conformancePath, bumpConformanceTool(conformancePath, edits.read(conformancePath), tool, from.version, version));

  // The generators' pins.
  const scripts = generatorScripts(root, pack, tool);
  const gitHeads = ecosystem === "npm" ? await (async () => {
    const old = await npmGitHead(sources, pin.package, from.version);
    if (old === null || old === from.commit || ![...scripts.own, ...scripts.shared].some((path) => edits.read(path).includes(old))) return null;
    const fresh = await npmGitHead(sources, pin.package, version);
    if (fresh === null) throw new Error(`a generator pins npm's gitHead of ${pin.package} ${from.version} (${old}), and npm records none for ${version}`);
    notes.push(`The generators pin npm's \`gitHead\` of ${pin.package}: ${old} for ${from.version}, ${fresh} for ${version}.`);
    return { old, fresh };
  })() : null;
  // A line naming the pinned commit (or gitHead) describes the pin: its release moves with it (`at v10.11.0 (<commit>)`).
  const commits: [string, string][] = [[from.commit, to.commit]];
  if (gitHeads !== null) commits.push([gitHeads.old, gitHeads.fresh]);
  const release = versionToken(from.version, "gu");
  for (const path of [...scripts.own, ...scripts.shared]) {
    let text = edits.read(path).split("\n").map((line) => (commits.some(([old]) => line.includes(old))
      ? commits.reduce((moved, [old, fresh]) => moved.replaceAll(old, fresh), line).replace(release, (_, v: string) => `${v}${version}`)
      : line)).join("\n");
    if (scripts.own.includes(path)) {
      text = replaceCounted(text, new RegExp(`^((?:export\\s+)?(?:const\\s+)?(?:[A-Z][A-Z0-9_]*_)?VERSION\\s*=\\s*)(["'])${literal(from.version)}\\2`, "gmu"), (_, head, quote) => `${head}${quote}${version}${quote}`, path);
    }
    for (const name of runtime.packages) text = replaceCounted(text, packageEntry(name, from.version), (_, head, quote) => `${head}${quote}${version}${quote}`, path);
    edits.write(path, text);
  }

  // The tool-packs whose recordings install the changed runtime.
  const record = [toolPack, ...[...manifests].filter(([id, other]) => id !== toolPack && recordedRuntimes(other).includes(newRuntime)).map(([id]) => id)];

  const project = manifest.rerail.project;
  const leftovers = findLeftovers(root, edits, [
    gatesPath,
    ...filesIn(root, `packs/${pack}/upstream/${tool}`).filter((path) => !path.endsWith("/recording.lock") && !path.endsWith(".expected.json")),
    ...scripts.shared,
    conformancePath,
    `packs/${pack}/README.md`,
    `packs/${pack}/licenses/${project}/UPSTREAM.md`,
    ...filesIn(root, `${RUNTIMES_DIRECTORY}/${oldRuntime}`),
  ], from.version, renames);

  return { toolPack, pack, tool, ecosystem, package: pin.package, repository, from, to, files: edits.changed(renames), renames, record, leftovers, notes };
}

const channelSchema = z.object({
  pkg: z.object({
    rust: z.object({ version: z.string(), git_commit_hash: z.string().regex(/^[0-9a-f]{40}$/u) }),
    "clippy-preview": z.object({ version: z.string() }),
  }),
});

/** What a Rust release's channel manifest says its toolchain prints: `rustc --version` and `cargo clippy --version`. */
export function rustToolchain(body: string): { rustc: string; clippy: string; clippyVersion: string } {
  const { pkg } = channelSchema.parse(parseToml(body));
  const match = /^(\S+) \(([0-9a-f]+) (\d{4}-\d{2}-\d{2})\)$/u.exec(pkg.rust.version);
  if (match === null || !pkg.rust.git_commit_hash.startsWith(match[2]!)) throw new Error(`[pkg.rust] version ${pkg.rust.version} is not \`<release> (<commit> <date>)\``);
  return {
    rustc: `rustc ${pkg.rust.version}`,
    clippy: `clippy ${pkg["clippy-preview"].version} (${pkg.rust.git_commit_hash.slice(0, 10)} ${match[3]!})`,
    clippyVersion: pkg["clippy-preview"].version,
  };
}

/** A release's channel manifest: what rustup installs as that toolchain. */
async function channelManifest(sources: Sources, version: string): Promise<string> {
  const url = `https://static.rust-lang.org/dist/channel-rust-${version}.toml`;
  const body = await getText(sources, url, { accept: "application/toml" });
  if (body === null) throw new Error(`${url} does not exist`);
  return body;
}

/** The tree src/tools/clippy is at a rust-lang/rust tag. */
async function clippySubtree(sources: Sources, tag: string): Promise<string> {
  const body = await getText(sources, `${GITHUB_API}/repos/rust-lang/rust/contents/src/tools?ref=${encodeURIComponent(tag)}`, githubHeaders(sources));
  const entry = body === null ? undefined : z.array(z.object({ name: z.string(), sha: z.string() })).parse(JSON.parse(body)).find((item) => item.name === "clippy");
  if (entry === undefined) throw new Error(`rust-lang/rust has no src/tools/clippy at ${tag}`);
  return entry.sha;
}

/**
 * The rust-lang/rust-clippy tag and commit a Rust release's Clippy comes from. rust-clippy tags `rust-<minor>.0` releases
 * only; a point release whose src/tools/clippy in rust-lang/rust is the tree its `.0` release had comes from that tag.
 */
async function clippySource(sources: Sources, version: string): Promise<{ tag: string; commit: string; note: string | null }> {
  const tag = `rust-${version}`;
  const commit = await tagCommit(sources, "rust-lang/rust-clippy", tag);
  if (commit !== null) return { tag, commit, note: null };
  const parts = releaseParts(version)!;
  if ((parts[2] ?? 0) === 0) throw new Error(`rust-lang/rust-clippy has no tag ${tag}`);
  const base = `${parts[0]}.${parts[1]}.0`;
  const [ours, theirs] = [await clippySubtree(sources, version), await clippySubtree(sources, base)];
  if (ours !== theirs) throw new Error(`rust-lang/rust-clippy has no tag ${tag}, and src/tools/clippy changed between Rust ${base} and ${version} (${theirs} to ${ours}): pin Clippy's source by hand`);
  const baseCommit = await tagCommit(sources, "rust-lang/rust-clippy", `rust-${base}`);
  if (baseCommit === null) throw new Error(`rust-lang/rust-clippy has no tag rust-${version} or rust-${base}`);
  return { tag: `rust-${base}`, commit: baseCommit, note: `rust-lang/rust-clippy tags no \`${tag}\`; Rust ${version}'s src/tools/clippy is the tree ${base} has (${ours}), so Clippy's source stays at \`rust-${base}\`.` };
}

/** A `NAME = "<value>"` constant of common.py. */
function pythonConstant(text: string, name: string, path: string): string {
  const value = new RegExp(`^${name} = "([^"]+)"$`, "mu").exec(text)?.[1];
  if (value === undefined) throw new Error(`${path} sets no ${name} = "<value>"`);
  return value;
}

/** Plan the bump of the Rust pack's Clippy to Rust `version`; null when it already pins it. */
export async function planClippyBump(root: string, version: string, sources: Sources): Promise<Bump | null> {
  const manifest = readGatesManifest(root, "rust", "clippy");
  const pin = clippyPin(root);
  if (pin.disagreements.length > 0) throw new Error(`rust/clippy: the pins disagree before the bump (${pin.disagreements.join("; ")}); line them up by hand first`);
  if (!isNewer(pin.version, version, "rust/clippy")) return null;
  const common = "packs/rust/upstream/common.py";
  const edits = new Edits(root);
  const text = edits.read(common);
  const pinned = {
    toolchain: pythonConstant(text, "TOOLCHAIN", common),
    rustc: pythonConstant(text, "RUSTC_VERSION", common),
    clippy: pythonConstant(text, "CLIPPY_VERSION", common),
    commit: pythonConstant(text, "CLIPPY_COMMIT", common),
  };
  // The pinned strings must be what the pinned release's manifest says, or the new ones cannot be read the same way.
  const old = rustToolchain(await channelManifest(sources, pinned.toolchain));
  if (old.rustc !== pinned.rustc || old.clippy !== pinned.clippy) throw new Error(`${common} pins \`${pinned.rustc}\` and \`${pinned.clippy}\`, but Rust ${pinned.toolchain}'s channel manifest says \`${old.rustc}\` and \`${old.clippy}\``);
  const oldTag = /at tag `(rust-[0-9.]+)`/u.exec(text)?.[1] ?? null;
  const fresh = rustToolchain(await channelManifest(sources, version));
  const source = await clippySource(sources, version);
  const minor = releaseParts(version)![1]!;
  if (fresh.clippyVersion !== `0.1.${minor}`) throw new Error(`Rust ${version}'s channel manifest names Clippy ${fresh.clippyVersion}, not 0.1.${minor}`);

  let after = text;
  for (const [name, value] of [["TOOLCHAIN", version], ["RUSTC_VERSION", fresh.rustc], ["CLIPPY_VERSION", fresh.clippy], ["CLIPPY_COMMIT", source.commit]] as const) {
    after = replaceCounted(after, new RegExp(`^(${name} = ")[^"]+(")$`, "gmu"), (_, head, tail) => `${head}${value}${tail}`, `${common} ${name}`, 1);
  }
  // The docstring names the same pins.
  after = after.replaceAll(pinned.rustc, fresh.rustc).replaceAll(pinned.clippy, fresh.clippy).replaceAll(pinned.commit, source.commit);
  if (oldTag !== null) after = after.replaceAll(`\`${oldTag}\``, `\`${source.tag}\``);
  after = after.replace(new RegExp(`(\\+|toolchain )${literal(pinned.toolchain)}(?![\\w.])`, "gu"), (_, head: string) => `${head}${version}`);
  edits.write(common, after);
  verify(common, ["TOOLCHAIN", "RUSTC_VERSION", "CLIPPY_VERSION", "CLIPPY_COMMIT"].map((name) => pythonConstant(after, name, common)), [version, fresh.rustc, fresh.clippy, source.commit]);

  const conformancePath = "packs/rust/conformance.json";
  const clippyVersion = `0.1.${releaseParts(pinned.toolchain)![1]!}`;
  if (clippyVersion !== fresh.clippyVersion) edits.write(conformancePath, bumpConformanceTool(conformancePath, edits.read(conformancePath), "clippy", clippyVersion, fresh.clippyVersion));

  // gates.yaml (upstream commit, Clippy's version, the runtime it names) and the recorder's runtime, named for the Rust
  // release whose toolchain it pins (runtimes/clippy-<release>/rust-toolchain.toml).
  const renames: { from: string; to: string }[] = [];
  const oldRuntime = manifest.upstream.runtime;
  const newRuntime = oldRuntime.endsWith(`-${pinned.toolchain}`) ? `${oldRuntime.slice(0, -pinned.toolchain.length)}${version}` : oldRuntime;
  if (newRuntime !== oldRuntime) {
    if (existsSync(join(root, RUNTIMES_DIRECTORY, newRuntime))) throw new Error(`${RUNTIMES_DIRECTORY}/${newRuntime} already exists`);
    renames.push({ from: `${RUNTIMES_DIRECTORY}/${oldRuntime}`, to: `${RUNTIMES_DIRECTORY}/${newRuntime}` });
  }
  const gatesPath = "packs/rust/upstream/clippy/gates.yaml";
  edits.write(gatesPath, withUpstreamBlock(edits.read(gatesPath), gatesPath, (block) => {
    let edited = block;
    if (source.commit !== pinned.commit) edited = replaceCounted(edited, yamlScalar("  ", "commit", pinned.commit), (_, head, quote) => `${head}${quote}${source.commit}${quote}`, `${gatesPath} upstream.commit`, 1);
    if (fresh.clippyVersion !== clippyVersion) edited = replaceCounted(edited, yamlScalar("  ", "version", clippyVersion), (_, head, quote) => `${head}${quote}${fresh.clippyVersion}${quote}`, `${gatesPath} upstream.version`, 1);
    if (newRuntime !== oldRuntime) edited = replaceCounted(edited, yamlScalar("  ", "runtime", oldRuntime), (_, head, quote) => `${head}${quote}${newRuntime}${quote}`, `${gatesPath} upstream.runtime`, 1);
    return edited;
  }));
  const { id: _id, ...gated } = manifest;
  verify(gatesPath, parseGatesManifest(edits.read(gatesPath), gatesPath), renamedRuntime({ ...gated, upstream: { ...gated.upstream, commit: source.commit, version: fresh.clippyVersion } }, oldRuntime, newRuntime));
  const toolchainPath = `${RUNTIMES_DIRECTORY}/${oldRuntime}/rust-toolchain.toml`;
  const toolchainBefore = edits.read(toolchainPath);
  const toolchain = replaceCounted(toolchainBefore, new RegExp(`^(channel\\s*=\\s*")${literal(pinned.toolchain)}(")\\s*$`, "gmu"), (_, head, tail) => `${head}${version}${tail}`, `${toolchainPath} channel`, 1);
  edits.write(toolchainPath, toolchain);
  // The file reads back as the pinned one with only its channel moved.
  const toolchainSchema = z.object({ toolchain: z.object({ channel: z.string() }).passthrough() }).passthrough();
  const original = toolchainSchema.parse(parseToml(toolchainBefore));
  verify(toolchainPath, toolchainSchema.parse(parseToml(toolchain)), { ...original, toolchain: { ...original.toolchain, channel: version } });

  const leftovers = findLeftovers(root, edits, [
    common,
    ...filesIn(root, "packs/rust/upstream/clippy").filter((path) => !path.endsWith("/recording.lock") && !path.endsWith(".expected.json")),
    conformancePath,
    "packs/rust/README.md",
    ...(existsSync(join(root, "packs/rust/licenses")) ? readdirSync(join(root, "packs/rust/licenses")).map((project) => `packs/rust/licenses/${project}/UPSTREAM.md`) : []),
    ...filesIn(root, `${RUNTIMES_DIRECTORY}/${oldRuntime}`),
  ], pinned.toolchain, renames);
  return {
    toolPack: "rust/clippy",
    pack: "rust",
    tool: "clippy",
    ecosystem: "rust",
    package: "rust",
    repository: "rust-lang/rust-clippy",
    from: { version: pinned.toolchain, tag: oldTag ?? `rust-${pinned.toolchain}`, commit: pinned.commit },
    to: { version, tag: source.tag, commit: source.commit },
    files: edits.changed(renames),
    renames,
    // Clippy is gated: its tool-pack is recorded again with the new toolchain (on Linux, as CI records it).
    record: ["rust/clippy"],
    leftovers,
    notes: source.note === null ? [] : [source.note],
  };
}

/** Plan a tool-pack's bump to `version`. */
export async function planBump(root: string, pack: string, tool: string, version: string, sources: Sources): Promise<Bump | null> {
  if (pack === "rust" && tool === "clippy") return await planClippyBump(root, version, sources);
  return await planGatedBump(root, pack, tool, version, sources);
}

/** Write a planned bump into `root`: rename the runtime directories, then write every changed file. */
export function applyBump(root: string, bump: Bump): void {
  for (const rename of bump.renames) renameSync(join(root, rename.from), join(root, rename.to));
  for (const file of bump.files) writeFileSync(join(root, file.path), file.after);
}

/** The changed lines of each file, as a diff; every bump edit keeps a file's line count. */
export function pinDiff(bump: Bump): string {
  const out: string[] = [];
  for (const rename of bump.renames) out.push(`rename ${rename.from}/ → ${rename.to}/`);
  for (const file of bump.files) {
    const before = file.before.split("\n");
    const after = file.after.split("\n");
    out.push(`--- ${file.path}`);
    if (before.length !== after.length) {
      out.push("(rewritten)");
      continue;
    }
    before.forEach((line, index) => {
      if (line !== after[index]) out.push(`@@ ${index + 1}`, `-${line}`, `+${after[index]!}`);
    });
  }
  return out.join("\n");
}

/**
 * Where a release is published, for the pull request: its tag, upstream's changes since the pinned one, and its registry
 * page. Rust's are rust-lang/rust's release tags; Clippy's source commit is in the bump's commit line.
 */
export function releaseLinks(bump: Bump): { release: string; releaseUrl: string; compare: string; compareUrl: string; registryUrl: string } {
  const { version } = bump.to;
  const [repository, fromTag, toTag] = bump.ecosystem === "rust" ? ["rust-lang/rust", bump.from.version, version] : [bump.repository, bump.from.tag, bump.to.tag];
  const github = `https://github.com/${repository}`;
  const registryUrl = {
    npm: `https://www.npmjs.com/package/${bump.package}/v/${version}`,
    pypi: `https://pypi.org/project/${bump.package}/${version}/`,
    packagist: `https://packagist.org/packages/${bump.package}#v${version}`,
    rust: `https://static.rust-lang.org/dist/channel-rust-${version}.toml`,
  }[bump.ecosystem];
  return {
    release: `${repository} ${toTag}`,
    releaseUrl: `${github}/releases/tag/${encodeURIComponent(toTag)}`,
    compare: `${fromTag}...${toTag}`,
    compareUrl: `${github}/compare/${encodeURIComponent(fromTag)}...${encodeURIComponent(toTag)}`,
    registryUrl,
  };
}

/**
 * The bump as Markdown, for the pull request's body: the release, the pin diff, what the nightly derived and recorded
 * again, how to do that locally, and what still names the old release. The nightly adds the test-data.lock files that
 * changed, which only exist once it has packed the derived data.
 */
export function renderBump(bump: Bump): string {
  const links = releaseLinks(bump);
  const name = bump.ecosystem === "rust" ? "Rust" : `\`${bump.package}\``;
  const lines = [
    `## \`${bump.toolPack}\`: ${name} ${bump.from.version} → ${bump.to.version}`,
    "",
    `- Release: [${links.release}](${links.releaseUrl}) (${ECOSYSTEM_NAMES[bump.ecosystem]}: [${bump.to.version}](${links.registryUrl}))`,
    `- Upstream changes: [${links.compare}](${links.compareUrl})`,
    `- Commit${bump.ecosystem === "rust" ? " (rust-lang/rust-clippy)" : ""}: \`${bump.from.commit}\` → \`${bump.to.commit}\``,
    ...bump.notes.map((note) => `- ${note}`),
    "",
    "### Pins",
    "",
    "```diff",
    pinDiff(bump),
    "```",
    "",
    "### Derived and recorded again",
    "",
    "The nightly derived each tool-pack's test data from the new pins and recorded its upstream answers again, on Linux with the runtimes the port readiness gates install, then packed the test data into the archive its `test-data.lock` names:",
    "",
    ...bump.record.map((id) => `- \`${id}\``),
    "",
    "### Regenerate locally",
    "",
    "From this branch, with the current test data pulled (`npm run fixtures:pull`), for each tool-pack above:",
    "",
    "```sh",
    ...bump.record.map((id) => {
      const [pack, tool] = id.split("/");
      const selector = `--pack ${pack!} --tool ${tool!}`;
      return `npm run fixtures:derive -- ${selector} --update && npm run port-readiness:record -- ${selector} --full && npm run fixtures:push -- ${selector}`;
    }),
    "```",
    "",
    "then commit each tool-pack's `test-data.lock` and `recording.lock`.",
    ...(bump.record.includes("rust/clippy") ? ["", "Derive and record `rust/clippy` on Linux: Clippy's answers on macOS differ from the ones CI checks."] : []),
    "",
    `### Still naming ${bump.from.version}`,
    "",
  ];
  if (bump.leftovers.length === 0) {
    lines.push("Nothing else in the tool-pack's pins, scripts, README, conformance.json or UPSTREAM.md names it.");
  } else {
    lines.push("Prose and pins the bump does not rewrite: review each before merging.", "");
    lines.push(...bump.leftovers.slice(0, LEFTOVER_LIMIT).map((line) => `- \`${line.replaceAll("`", "'")}\``));
    if (bump.leftovers.length > LEFTOVER_LIMIT) lines.push(`- ... and ${bump.leftovers.length - LEFTOVER_LIMIT} more`);
  }
  return `${lines.join("\n")}\n`;
}

/** What the nightly reads back: the bump without the file texts. */
export function bumpJson(bump: Bump | null, toolPack: string, version: string): unknown {
  if (bump === null) return { toolPack, version, changed: false };
  const { files, ...rest } = bump;
  return { ...rest, version: bump.to.version, changed: true, files: files.map((file) => file.path), record: bump.record.map((id) => ({ pack: id.split("/")[0], tool: id.split("/")[1] })) };
}

const USAGE = `usage:
  npm run port-readiness:bump -- --pack <pack> --tool <tool> [--version <release>] [--root <dir>] [--summary <file>] [--json <file>] [--dry-run]

Without --version the registry's latest stable release is the target. GH_TOKEN or GITHUB_TOKEN, when set, is sent to
GitHub's API.
`;

/** The latest stable release of a tool-pack's package, as port-readiness:upstream-releases reads it. */
async function latestRelease(root: string, pack: string, tool: string, fetch: Fetch): Promise<string> {
  const pin = gatedPin(root, pack, tool);
  const source = registry(pin.ecosystem, pin.package);
  const response = await fetch(source.url, { headers: { accept: source.accept }, signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS) });
  if (!response.ok) throw new Error(`${source.url}: HTTP ${response.status}`);
  return source.latest(await response.text());
}

/** Run the CLI; the exit code. */
export async function main(argv: string[], repositoryRoot: string, fetch: Fetch, env: NodeJS.ProcessEnv = process.env): Promise<number> {
  let values;
  try {
    ({ values } = parseArgs({
      args: argv,
      options: {
        pack: { type: "string" },
        tool: { type: "string" },
        version: { type: "string" },
        root: { type: "string" },
        summary: { type: "string" },
        json: { type: "string" },
        "dry-run": { type: "boolean", default: false },
        help: { type: "boolean", default: false },
      },
      strict: true,
    }));
  } catch (error) {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n${USAGE}`);
    return 2;
  }
  if (values.help || values.pack === undefined || values.tool === undefined) {
    process.stderr.write(USAGE);
    return values.help ? 0 : 2;
  }
  const { pack, tool } = values;
  const root = resolve(values.root ?? repositoryRoot);
  const sources: Sources = { fetch, token: env.GH_TOKEN ?? env.GITHUB_TOKEN };
  const version = values.version ?? await latestRelease(root, pack, tool, fetch);
  const bump = await planBump(root, pack, tool, version, sources);
  const write = (file: string | undefined, text: string): void => {
    if (file === undefined) return;
    mkdirSync(dirname(resolve(file)), { recursive: true });
    writeFileSync(resolve(file), text);
  };
  write(values.json, `${JSON.stringify(bumpJson(bump, `${pack}/${tool}`, version), null, 2)}\n`);
  if (bump === null) {
    process.stdout.write(`${pack}/${tool} already pins ${version}\n`);
    write(values.summary, `## \`${pack}/${tool}\` already pins ${version}\n`);
    return 0;
  }
  if (!values["dry-run"]) applyBump(root, bump);
  const markdown = renderBump(bump);
  write(values.summary, markdown);
  process.stdout.write(`${values["dry-run"] ? "would bump" : "bumped"} ${bump.toolPack} ${bump.from.version} → ${bump.to.version} in ${root}\n\n${markdown}`);
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
  main(process.argv.slice(2), resolve(dirname(fileURLToPath(import.meta.url)), "..", "..", ".."), globalThis.fetch).then((code) => {
    process.exitCode = code;
  }, (error: unknown) => {
    process.stderr.write(`conformance/port-readiness/upstream/bump.ts: ${error instanceof Error ? error.message : String(error)}\n`);
    process.exitCode = 1;
  });
}

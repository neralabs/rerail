import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { dirname, join, matchesGlob, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { tableFromIPC, type Table } from "apache-arrow";
import { z } from "zod";

import { PACKAGE_ROOT } from "../../src/engine/index.js";
import { runChecked, type CommandRunner } from "./process.js";
import type { Check } from "./types.js";

/**
 * The corpus goldens are read from: the code a SWE Atlas/Evo task ends with. For every pinned task
 * (corpus-tasks.json), its reference solution (the vetted code diff at the end of the task, pinned by sha256, read
 * from a pinned GitHub file or a pinned Hugging Face dataset cell) is applied to the files it touches at the task's
 * base commit, and each resulting file of a type the pack lints is one corpus file. The corpora a rerail pack pins in
 * its conformance manifests add every file under their pinned paths. Everything fetched is cached under the cache
 * root's `corpus/`, keyed by those pins, and reused only while it still has the digests recorded when it was built.
 */

/** Where fetched corpora, results and scratch files are cached unless a run names another directory (gitignored). */
export const DEFAULT_CACHE_ROOT = join(PACKAGE_ROOT, ".rerail", "port-readiness");

/** The pinned task list beside this module. */
export const CORPUS_TASKS = fileURLToPath(new URL("./corpus-tasks.json", import.meta.url));

const sha256 = (value: string | Buffer): string => createHash("sha256").update(value).digest("hex");

const corpusTasksSchema = z.strictObject({
  /** One entry per catalog task, its latest revision, sorted by task id. */
  tasks: z.array(z.strictObject({
    /** The catalog task's id. */
    task: z.string().min(1),
    /** The benchmark the task comes from: `swe-atlas` or `swe-evo`. */
    substrate: z.string().min(1),
    /** The task's GitHub repository, `<owner>/<name>`. */
    repository: z.string().regex(/^[A-Za-z0-9._-]+\/[A-Za-z0-9._-]+$/u),
    /** The commit the task starts from, which its reference solution applies to. */
    baseSha: z.string().regex(/^[0-9a-f]{40}$/u),
    /** The task's reference solution: where its patch is pinned, and its sha256; null when the task has none. */
    reference: z.strictObject({ locator: z.string().min(1), sha256: z.string().regex(/^[0-9a-f]{64}$/u) }).nullable(),
  })),
});

/** One pinned catalog task, as corpus-tasks.json records it. */
export type PinnedTask = z.infer<typeof corpusTasksSchema>["tasks"][number];

/** The pinned catalog tasks: corpus-tasks.json, or another file of the same form. */
export function readCorpusTasks(path: string = CORPUS_TASKS): PinnedTask[] {
  return corpusTasksSchema.parse(JSON.parse(readFileSync(path, "utf8"))).tasks;
}

export interface CorpusFile {
  task: string;
  repository: string;
  path: string;
  source: string;
  sha256: string;
}

export interface CorpusTask {
  task: string;
  substrate: string;
  repository: string;
  baseSha: string;
  patchSha256: string | null;
  /**
   * `skipped`: the task has nothing the linter reads, or its solution does not apply; the same on every run.
   * `failed`: a pinned input could not be fetched (a download or git fetch failed, or the bytes differ from the pin), so
   * this run lacks files a complete run has.
   */
  status: "applied" | "skipped" | "failed";
  reason: string;
  files: number;
}

export interface Corpus {
  tasks: CorpusTask[];
  files: CorpusFile[];
}

interface PatchedPath {
  before: string | null;
  after: string | null;
}

/** The files a git patch touches: what each was before (null when created) and after (null when deleted). */
export function patchedPaths(patch: string): PatchedPath[] {
  const paths: PatchedPath[] = [];
  for (const block of patch.split(/^(?=diff --git )/mu).filter((part) => part.startsWith("diff --git "))) {
    const header = /^diff --git a\/(.+?) b\/(.+)$/mu.exec(block);
    const before = /^--- (?:a\/(.+)|\/dev\/null)$/mu.exec(block);
    const after = /^\+\+\+ (?:b\/(.+)|\/dev\/null)$/mu.exec(block);
    const renamedFrom = /^rename from (.+)$/mu.exec(block)?.[1];
    const renamedTo = /^rename to (.+)$/mu.exec(block)?.[1];
    const created = /^new file mode /mu.test(block);
    const deleted = /^deleted file mode /mu.test(block);
    const beforePath = created ? null : renamedFrom ?? before?.[1] ?? header?.[1] ?? null;
    const afterPath = deleted ? null : renamedTo ?? after?.[1] ?? header?.[2] ?? null;
    paths.push({ before: beforePath, after: afterPath });
  }
  return paths;
}

/** A GitHub blob locator pinned at a commit, as the raw file URL; undefined for any other locator. */
function rawGithubUrl(locator: string): string | undefined {
  const match = /^https:\/\/github\.com\/([^/]+)\/([^/]+)\/blob\/([0-9a-f]{40})\/(.+)$/u.exec(locator);
  return match === null ? undefined : `https://raw.githubusercontent.com/${match[1]}/${match[2]}/${match[3]}/${match[4]}`;
}

/** A Hugging Face dataset cell pinned at a revision: `hf://datasets/<owner>/<name>@<revision>/<file>#row=<n>&column=<c>`. */
function huggingFaceCell(locator: string): { url: string; cache: string; row: number; column: string } | undefined {
  const match = /^hf:\/\/datasets\/([^/]+)\/([^@/]+)@([0-9a-f]{40})\/([^#]+\.arrow)#row=(\d+)&column=([A-Za-z_]+)$/u.exec(locator);
  if (match === null) return undefined;
  const [, owner, name, revision, file, row, column] = match;
  return {
    url: `https://huggingface.co/datasets/${owner}/${name}/resolve/${revision}/${file}`,
    cache: join("hf", owner!, name!, revision!, file!),
    row: Number(row),
    column: column!,
  };
}

const arrowTables = new Map<string, Table>();

/** The text of one cell of a pinned Hugging Face arrow file, downloading the file into the cache once. */
async function huggingFaceText(cacheRoot: string, cell: NonNullable<ReturnType<typeof huggingFaceCell>>): Promise<string | undefined> {
  const path = join(cacheRoot, "corpus", cell.cache);
  if (!existsSync(path)) {
    const response = await fetch(cell.url);
    if (!response.ok) return undefined;
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, Buffer.from(await response.arrayBuffer()));
  }
  let table = arrowTables.get(path);
  if (table === undefined) {
    table = tableFromIPC(readFileSync(path));
    arrowTables.set(path, table);
  }
  const value: unknown = table.getChild(cell.column)?.get(cell.row);
  return typeof value === "string" ? value : undefined;
}

type Unfetched = { status: "skipped" | "failed"; reason: string };

async function referencePatch(cacheRoot: string, reference: NonNullable<PinnedTask["reference"]>): Promise<{ text: string; sha256: string } | Unfetched> {
  const expected = reference.sha256;
  const cached = join(cacheRoot, "corpus", "patches", `${expected}.patch`);
  if (existsSync(cached)) {
    const bytes = await readFile(cached);
    if (sha256(bytes) === expected) return { text: bytes.toString("utf8"), sha256: expected };
  }
  const locator = reference.locator;
  const github = rawGithubUrl(locator);
  const cell = huggingFaceCell(locator);
  let bytes: Buffer;
  if (github !== undefined) {
    const response = await fetch(github);
    if (!response.ok) return { status: "failed", reason: `downloading its reference patch failed with HTTP ${response.status}` };
    bytes = Buffer.from(await response.arrayBuffer());
  } else if (cell !== undefined) {
    const text = await huggingFaceText(cacheRoot, cell);
    if (text === undefined) return { status: "failed", reason: `its reference patch is not readable at ${locator}` };
    bytes = Buffer.from(text, "utf8");
  } else {
    return { status: "skipped", reason: `its reference patch is neither a pinned GitHub file nor a pinned Hugging Face dataset cell (${locator.split("#")[0]})` };
  }
  if (sha256(bytes) !== expected) return { status: "failed", reason: `its reference patch does not match the sha256 the catalog pins (${expected})` };
  await mkdir(join(cacheRoot, "corpus", "patches"), { recursive: true });
  await writeFile(cached, bytes);
  return { text: bytes.toString("utf8"), sha256: expected };
}

/** True when a cached checkout is exactly the commit with an unmodified working tree (ignored build outputs aside). */
async function isPristine(directory: string, commit: string, runner: CommandRunner): Promise<boolean> {
  if (!existsSync(join(directory, ".git"))) return false;
  const head = await runChecked(runner, ["git", "-C", directory, "rev-parse", "HEAD"]).catch(() => "");
  if (head.trim() !== commit) return false;
  const status = await runChecked(runner, ["git", "-C", directory, "status", "--porcelain"]).catch(() => "?");
  return status.trim() === "";
}

/**
 * A sparse checkout of `paths` of a repository at one exact commit, in `directory`. A cached checkout is reused only
 * while it is pristine; anything else is fetched again.
 */
export async function checkoutPinned(repository: string, commit: string, directory: string, paths: string[], runner: CommandRunner, options: { blobless?: boolean } = {}): Promise<string> {
  if (await isPristine(directory, commit, runner)) {
    // A checkout cached with fewer paths only needs its sparse set widened: the shallow fetch holds every file of the
    // commit. Widened, never narrowed, so tool-packs that share a corpus keep each other's files.
    const sparse = (await runChecked(runner, ["git", "-C", directory, "sparse-checkout", "list"]).catch(() => "")).trim().split("\n");
    if (paths.every((path) => sparse.includes(path))) return directory;
    await runChecked(runner, ["git", "-C", directory, "sparse-checkout", "add", ...paths.filter((path) => !sparse.includes(path))]);
    if (await isPristine(directory, commit, runner)) return directory;
  }
  await rm(directory, { recursive: true, force: true });
  await mkdir(directory, { recursive: true });
  const git = (...args: string[]) => runChecked(runner, ["git", "-C", directory, ...args]);
  await git("init", "-q");
  await git("remote", "add", "origin", `${repository}.git`);
  await git("sparse-checkout", "set", "--no-cone", ...paths);
  // Blobless, the checkout fetches only the blobs of the sparse paths, and a widened sparse set fetches the rest on demand.
  await git("fetch", "-q", "--depth", "1", ...(options.blobless === true ? ["--filter=blob:none"] : []), "origin", commit);
  await git("checkout", "-q", "--force", "FETCH_HEAD");
  if (!(await isPristine(directory, commit, runner))) throw new Error(`checkout of ${repository} ${commit} is not clean after checkout`);
  return directory;
}

/** A reference solution that does not apply to its base commit: the same on every run, unlike a failed fetch. */
class PatchDoesNotApply extends Error {}

/**
 * The patched files of one task, read from a cached sparse checkout of its base commit with the patch applied. The
 * cache is reused only while every patched file still has the digest recorded when it was built.
 */
async function patchedFiles(cacheRoot: string, task: string, repository: string, baseSha: string, patch: { text: string; sha256: string }, wanted: string[], runner: CommandRunner): Promise<Map<string, string>> {
  // One checkout per set of kept files, so tool-packs for different languages never rebuild each other's.
  const directory = resolve(cacheRoot, "corpus", "checkouts", `${task}-${sha256(wanted.join("\n")).slice(0, 12)}`);
  const record = `${directory}.json`;
  if (existsSync(record)) {
    const recorded = JSON.parse(await readFile(record, "utf8")) as { patchSha256: string; baseSha: string; files: Record<string, string> };
    const files = new Map<string, string>();
    let intact = recorded.patchSha256 === patch.sha256 && recorded.baseSha === baseSha;
    for (const path of wanted) {
      const file = join(directory, path);
      const text = intact && existsSync(file) ? await readFile(file, "utf8") : undefined;
      if (text === undefined || sha256(text) !== recorded.files[path]) intact = false;
      else files.set(path, text);
    }
    if (intact) return files;
  }
  await rm(directory, { recursive: true, force: true });
  await mkdir(directory, { recursive: true });
  const git = (...args: string[]) => runChecked(runner, ["git", "-C", directory, ...args]);
  const touched = [...new Set(patchedPaths(patch.text).flatMap(({ before, after }) => [before, after]).filter((path): path is string => path !== null))];
  await git("init", "-q");
  await git("remote", "add", "origin", `https://github.com/${repository}.git`);
  await git("sparse-checkout", "set", "--no-cone", ...touched.map((path) => `/${path}`));
  await git("fetch", "-q", "--depth", "1", "--filter=blob:none", "origin", baseSha);
  await git("checkout", "-q", "--force", "FETCH_HEAD");
  // Only the kept files' hunks are applied, so a hunk no golden can name (a CRLF batch file, a binary)
  // cannot drop the task.
  const kept = patchedPaths(patch.text).filter(({ after }) => after !== null && wanted.includes(after))
    .flatMap(({ before, after }) => [before, after]).filter((path): path is string => path !== null);
  try {
    await runChecked(runner, ["git", "-C", directory, "apply", ...[...new Set(kept)].map((path) => `--include=${path}`), "-"], { input: patch.text });
  } catch (error) {
    throw new PatchDoesNotApply(error instanceof Error ? error.message : String(error));
  }
  const files = new Map<string, string>();
  for (const path of wanted) files.set(path, await readFile(join(directory, path), "utf8"));
  await writeFile(record, `${JSON.stringify({ patchSha256: patch.sha256, baseSha, files: Object.fromEntries([...files].map(([path, text]) => [path, sha256(text)])) })}\n`);
  return files;
}

/**
 * corpus-complete: every task and corpus a golden names was fetched. A task that failed to fetch (a download, a git
 * fetch) would otherwise fail each golden that names one of its files as though the rule differed there; this says why,
 * and keeps the run's results out of the cache. A golden whose file the fetched corpus lacks, or has with other bytes,
 * is the golden's own failure: the rule's identical-on-goldens names it.
 */
export function corpusCompleteCheck(goldens: readonly { task: string }[], tasks: readonly CorpusTask[]): Check {
  const named = new Set(goldens.map((golden) => golden.task));
  const failed = tasks.filter((task) => named.has(task.task) && task.status === "failed");
  if (failed.length === 0) return { id: "corpus-complete", ok: true, detail: `every task and corpus the ${goldens.length} golden(s) name was fetched` };
  return { id: "corpus-complete", ok: false, detail: `${failedTasksMessage(failed)}: run the gates again` };
}

/** The tasks a run failed to fetch, for an error or a check's detail. */
export function failedTasksMessage(failed: readonly CorpusTask[]): string {
  return `${failed.length} corpus task(s) could not be fetched (${failed.slice(0, 3).map((task) => `${task.task}: ${task.reason}`).join("; ")})`;
}

/**
 * The places (`task`, `path`) a corpus is built for: the files goldens name. A task or corpus no golden names is not
 * fetched at all, and of the rest only the named files are read.
 */
export type Wanted = ReadonlySet<string>;

export const wantedKey = (task: string, path: string): string => `${task}\0${path}`;

/** The places the goldens name. */
export function wantedPlaces(goldens: Iterable<{ task: string; path: string }>): Wanted {
  return new Set([...goldens].map((golden) => wantedKey(golden.task, golden.path)));
}

/**
 * Build the corpus of files the pack's linter reads (`inCorpus`) from the pinned tasks' reference solutions: the files
 * of `wanted`, and only the tasks that name one.
 */
export async function buildCorpus(cacheRoot: string, inCorpus: (path: string) => boolean, runner: CommandRunner, wanted: Wanted, pinned: PinnedTask[] = readCorpusTasks()): Promise<Corpus> {
  const tasks: CorpusTask[] = [];
  const files: CorpusFile[] = [];
  for (const { task, substrate, repository, baseSha, reference } of pinned) {
    const entry = { task, substrate, repository, baseSha };
    if (![...wanted].some((place) => place.startsWith(`${task}\0`))) {
      tasks.push({ ...entry, patchSha256: reference?.sha256 ?? null, status: "skipped", reason: "no golden names a file of it", files: 0 });
      continue;
    }
    if (reference === null) {
      tasks.push({ ...entry, patchSha256: null, status: "skipped", reason: "it has no reference solution", files: 0 });
      continue;
    }
    const patch = await referencePatch(cacheRoot, reference);
    if ("status" in patch) {
      tasks.push({ ...entry, patchSha256: reference.sha256, ...patch, files: 0 });
      continue;
    }
    const named = patchedPaths(patch.text)
      .map(({ after }) => after)
      .filter((path): path is string => path !== null && inCorpus(path) && wanted.has(wantedKey(task, path)))
      .sort();
    if (named.length === 0) {
      tasks.push({ ...entry, patchSha256: patch.sha256, status: "skipped", reason: "its solution changes no file a golden names", files: 0 });
      continue;
    }
    try {
      const patched = await patchedFiles(cacheRoot, task, repository, baseSha, patch, named, runner);
      for (const [path, source] of patched) files.push({ task, repository, path, source, sha256: sha256(source) });
      tasks.push({ ...entry, patchSha256: patch.sha256, status: "applied", reason: "its solution applies to the base commit", files: patched.size });
    } catch (error) {
      const last = (error instanceof Error ? error.message : String(error)).split("\n").slice(-1)[0];
      tasks.push(error instanceof PatchDoesNotApply
        ? { ...entry, patchSha256: patch.sha256, status: "skipped", reason: `its solution does not apply: ${last}`, files: 0 }
        : { ...entry, patchSha256: patch.sha256, status: "failed", reason: `fetching its base commit failed: ${last}`, files: 0 });
    }
  }
  return { tasks, files };
}

/** The files under a directory, recursively, as `/`-separated paths relative to `root`; symbolic links are not followed. */
async function filesUnder(root: string, relative: string): Promise<string[]> {
  const entries = await readdir(join(root, relative), { withFileTypes: true }).catch(() => null);
  if (entries === null) return existsSync(join(root, relative)) ? [relative] : [];
  const nested = await Promise.all(entries.map((entry) => {
    const path = `${relative}/${entry.name}`;
    if (entry.isDirectory()) return filesUnder(root, path);
    return Promise.resolve(entry.isFile() ? [path] : []);
  }));
  return nested.flat();
}

/** One corpus a rerail pack pins in its conformance manifest: a repository at a commit, the paths of it to read. */
export interface RerailCorpus {
  name: string;
  repository: string;
  commit: string;
  paths: string[];
  exclude: string[];
}

/** The conformance manifests of a pack that pin corpora: the pack's own, and its candidates'. */
export function conformancePaths(pack: string): string[] {
  return [`packs/${pack}/conformance.json`, `packs/${pack}/candidates/conformance.json`];
}

/**
 * Every corpus a pack pins, by name: the `corpora` of its conformance manifests under `root`. A name pinned twice must
 * name the same repository, commit and paths.
 */
export async function readRerailCorpora(root: string, pack: string): Promise<RerailCorpus[]> {
  const byName = new Map<string, RerailCorpus>();
  for (const path of conformancePaths(pack)) {
    const text = await readFile(join(root, path), "utf8").catch(() => null);
    if (text === null) continue;
    const corpora = (JSON.parse(text) as { corpora?: unknown }).corpora;
    if (!Array.isArray(corpora)) continue;
    for (const entry of corpora as Partial<RerailCorpus>[]) {
      const valid = typeof entry.name === "string" && /^[a-z0-9][a-z0-9._-]*$/u.test(entry.name)
        && typeof entry.repository === "string" && /^https:\/\/github\.com\/[A-Za-z0-9._-]+\/[A-Za-z0-9._-]+$/u.test(entry.repository)
        && typeof entry.commit === "string" && /^[0-9a-f]{40}$/u.test(entry.commit)
        && Array.isArray(entry.paths) && entry.paths.length > 0 && entry.paths.every((item) => typeof item === "string" && item !== "" && !item.split("/").includes(".."))
        && (entry.exclude === undefined || (Array.isArray(entry.exclude) && entry.exclude.every((item) => typeof item === "string")));
      if (!valid) throw new Error(`${path} pins a corpus the gates cannot read: ${JSON.stringify(entry)}`);
      const corpus: RerailCorpus = { name: entry.name!, repository: entry.repository!, commit: entry.commit!, paths: [...entry.paths!].sort(), exclude: [...(entry.exclude ?? [])].sort() };
      const pinned = byName.get(corpus.name);
      if (pinned !== undefined && JSON.stringify(pinned) !== JSON.stringify(corpus)) throw new Error(`pack ${pack} pins two different corpora named ${corpus.name}`);
      byName.set(corpus.name, corpus);
    }
  }
  return [...byName.values()].sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
}

/** The corpus id a pack's pinned corpus takes in place of a task: `rerail:<name>`. */
export function rerailCorpusId(corpus: RerailCorpus): string {
  return `rerail:${corpus.name}`;
}

/**
 * The corpus from the corpora a pack pins (its conformance manifests' `corpora`): each repository at its pinned
 * commit, and of the files under the pinned paths of a type the pack's linter reads that no `exclude` glob matches,
 * those `wanted` names, read from a sparse checkout of just them. Each file's task is the corpus id (`rerail:<name>`),
 * so goldens name it like a task's file. A corpus no golden names is not fetched.
 */
export async function buildRerailCorpora(corpora: RerailCorpus[], cacheRoot: string, inCorpus: (path: string) => boolean, runner: CommandRunner, wanted: Wanted): Promise<Corpus> {
  const tasks: CorpusTask[] = [];
  const files: CorpusFile[] = [];
  for (const corpus of corpora) {
    const task = rerailCorpusId(corpus);
    const repository = corpus.repository.replace(/^https:\/\/github\.com\//u, "");
    const named = [...wanted].filter((place) => place.startsWith(`${task}\0`)).map((place) => place.slice(task.length + 1))
      .filter((path) => corpus.paths.some((root) => path === root || path.startsWith(`${root}/`)))
      .sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
    if (named.length === 0) {
      tasks.push({ task, substrate: "rerail", repository, baseSha: corpus.commit, patchSha256: null, status: "skipped", reason: "no golden names a file of it", files: 0 });
      continue;
    }
    const checkout = await checkoutPinned(corpus.repository, corpus.commit, resolve(cacheRoot, "corpus", "rerail", corpus.name, corpus.commit), named.map((path) => `/${path}`), runner);
    const paths = (await Promise.all(named.map((path) => filesUnder(checkout, path))))
      .flat()
      .filter((path) => inCorpus(path) && !corpus.exclude.some((glob) => matchesGlob(path, glob)))
      .sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
    for (const path of [...new Set(paths)]) {
      const source = await readFile(join(checkout, path), "utf8");
      files.push({ task, repository, path, source, sha256: sha256(source) });
    }
    tasks.push({
      task, substrate: "rerail", repository, baseSha: corpus.commit, patchSha256: null,
      status: paths.length > 0 ? "applied" : "skipped",
      reason: paths.length > 0 ? `the rerail pack pins it (${corpus.paths.join(", ")}), and goldens name ${new Set(paths).size} of its files` : "goldens name files of it, but none is a file of a type the linter reads",
      files: new Set(paths).size,
    });
  }
  return { tasks, files };
}

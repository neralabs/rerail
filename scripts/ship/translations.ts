// The translation suite: for each pack with `.rerail` translations, whether each rule's translation reports the same
// findings as its original JSON rule, rule by rule, and whether that matches what the pack declares
// (docs/rule-syntax/packs/<pack>/equivalence.json). The decisions are translations-model.ts's; this runs the engines.
//
//   node --import tsx scripts/ship/translations.ts [check] (--pack <pack> | --packs a,b | --all) [--rules a,b]
//       [--corpora <dir>] [--engine <rerail-engine>] [--out <dir>] [--jobs 8] [--timeout 3600] [--write-differences]
//       (npm run translations:check -- …)
//   node --import tsx scripts/ship/translations.ts pin <corpus> [--corpora <dir>]
//
// The original of a rule is packs/<pack>/rules/<id>.json, through the product's path (product-run.ts: the pack's rules
// compiled together over a corpus in one production workspace). Its translation is every `.rerail` file of
// docs/rule-syntax/packs/<pack>/ whose `.policy` line names the rule's id (a rule whose clauses span languages is one
// policy per language; a file without a `.policy` line is a table others include), checked by `rerail-engine
// check-rr` and run by `rerail-engine run-policies <corpus> <out> <rule.rerail>...`: every translation of the pack in
// one run, and when that run fails, each rule's files alone (--jobs at once), a rule failing alone reported with the
// engine's message. Findings are compared per rule as sets of (rule, path, line, code), and column where both sides
// give one; messages are not compared. The engine reads the library translations include from this checkout's
// docs/rule-syntax/lib (RERAIL_RR_LIB). A rule is `failed` when check-rr refuses it, a side's run fails, or its
// original does not compile; the original's failures on single files (syntax errors, a spent work budget) are
// recorded beside the comparison, as the product reports them and carries on.
//
// The corpora, each compared on its own:
//   examples   pack.json's `rules[].examples` that are files, laid out as few directories as hold them;
//   fixtures   the pack's recorded upstream fixtures, <corpora>/recorded/<pack>/files, as the shippability bar's
//              `prepare` makes them (checked against <corpora>/corpora.lock.json when it is there);
//   <name>     each real-code corpus of scripts/ship/corpora.json for the pack: its `translations[<pack>]`, else the
//              benchmark's `packs[<pack>]`, at <corpora>/real/<name>. A git corpus missing there is exported from this
//              repository (its commit fetched from origin when the clone lacks it); each is checked against its pin.
// --corpora defaults to .rerail/translations/corpora; --engine to the checkout's release `rerail-engine`.
//
// Writes <out>/<pack>.json (translations-model.ts PackReport; --out defaults to .rerail/translations/out) and a summary
// on stdout. Exits 1 when a pack declaring `equivalent: true` disagrees with its run (gateProblems: a corpus not
// compared counts too), 2 when it cannot run; a pack declaring `equivalent: false` is reported only.
// --write-differences rewrites each pack's equivalence.json `different` and `untranslated` from the run
// (writeDifferences), creating the file (`equivalent: false`) where there is none.
//
// `pin <corpus>` exports a git corpus of corpora.json and writes its file count and hash there.

import { spawn, spawnSync } from "node:child_process";
import { availableParallelism } from "node:os";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { hashTree } from "./corpus-tree.js";
import { engineJson } from "./engine-json.js";
import { engineBinary } from "./metric.js";
import {
  byRule,
  type Declaration,
  emptyDeclaration,
  EXAMPLES_CORPUS,
  exampleFiles,
  type Finding,
  FIXTURES_CORPUS,
  gateProblems,
  layOutExamples,
  type PackReport,
  parseDeclaration,
  renderDeclaration,
  renderPack,
  type RuleInput,
  ruleResult,
  summarize,
  writeDifferences,
} from "./translations-model.js";

const HERE = dirname(fileURLToPath(import.meta.url));
const SELF = resolve(HERE, "..", "..");
const MANIFEST = join(HERE, "corpora.json");
const TRANSLATIONS = join(SELF, "docs", "rule-syntax", "packs");
/**
 * The engine's environment: the `.rerail` library its translations include is this checkout's (RERAIL_RR_LIB), not
 * the one the engine binary was built from.
 */
const ENGINE_ENV = { ...process.env, RERAIL_RR_LIB: join(SELF, "docs", "rule-syntax", "lib") };

interface Manifest {
  format: 1;
  corpora: Record<string, ({ kind: "git"; commit: string; paths: string[] } | { kind: "samples"; pack: string; exclude: string[] }) & { files: number | null; sha256: string | null }>;
  packs: Record<string, string>;
  /** The translation suite's real-code corpora of a pack, where they are not the benchmark's (`packs`). */
  translations?: Record<string, string[]>;
}

class Refusal extends Error {}

function refuse(message: string): never {
  throw new Refusal(message);
}

function readJson<T>(path: string): T {
  return JSON.parse(readFileSync(path, "utf8")) as T;
}

function git(args: readonly string[]): { status: number; stderr: string } {
  const result = spawnSync("git", ["-C", SELF, ...args], { encoding: "utf8", maxBuffer: 1 << 26 });
  return { status: result.status ?? 1, stderr: (result.stderr ?? String(result.error ?? "")).trim() };
}

/**
 * A git corpus of the manifest, exported from this repository at its commit into `destination`: the commit fetched
 * alone from origin when the clone lacks it (a shallow CI checkout), Git LFS pointers kept as pointers so the tree is
 * the same wherever it is made.
 */
function exportGitCorpus(name: string, spec: { commit: string; paths: string[] }, destination: string): void {
  if (git(["cat-file", "-e", `${spec.commit}^{commit}`]).status !== 0) {
    const fetched = git(["fetch", "--no-tags", "--depth=1", "origin", spec.commit]);
    if (fetched.status !== 0) refuse(`corpus ${name}: commit ${spec.commit} is not in this clone and could not be fetched: ${fetched.stderr.split("\n").at(-1)}`);
  }
  rmSync(destination, { recursive: true, force: true });
  mkdirSync(destination, { recursive: true });
  const tar = `${destination}.tar`;
  const archived = git(["-c", "filter.lfs.smudge=cat", "-c", "filter.lfs.process=", "-c", "filter.lfs.required=false", "-c", "core.autocrlf=false", "archive", "-o", tar, spec.commit, "--", ...spec.paths]);
  if (archived.status !== 0) refuse(`corpus ${name}: git archive ${spec.commit}: ${archived.stderr.split("\n").at(-1)}`);
  if (spawnSync("tar", ["-xf", tar, "-C", destination]).status !== 0) refuse(`corpus ${name}: could not extract ${tar}`);
  rmSync(tar);
}

// ---------------------------------------------------------------- running a side

interface Spawned {
  status: number | null;
  stdout: string;
  stderr: string;
  timedOut: boolean;
}

function spawnAsync(command: string, args: readonly string[], options: { cwd: string; env?: NodeJS.ProcessEnv; timeoutS: number }): Promise<Spawned> {
  const { promise, resolve: done } = Promise.withResolvers<Spawned>();
  const child = spawn(command, args, { cwd: options.cwd, env: options.env ?? process.env, stdio: ["ignore", "pipe", "pipe"] });
  const out: Buffer[] = [];
  const err: Buffer[] = [];
  let timedOut = false;
  const timer = setTimeout(() => {
    timedOut = true;
    child.kill("SIGKILL");
  }, options.timeoutS * 1000);
  child.stdout.on("data", (chunk: Buffer) => out.push(chunk));
  child.stderr.on("data", (chunk: Buffer) => err.push(chunk));
  child.on("error", (error) => err.push(Buffer.from(String(error))));
  child.on("close", (status) => {
    clearTimeout(timer);
    done({ status, stdout: Buffer.concat(out).toString("utf8"), stderr: Buffer.concat(err).toString("utf8"), timedOut });
  });
  return promise;
}

/** What a failed process said: its stderr less the engine's timing lines, or its exit. */
function failureText(run: Spawned, timeoutS: number): string {
  if (run.timedOut) return `timed out after ${timeoutS} s`;
  const lines = run.stderr.split("\n").map((line) => line.trim()).filter((line) => line !== "" && !/^(after the run|before the command)\b/.test(line) && !/^shipbar: phase /.test(line));
  // Paths in this checkout as the repository names them, so a message reads the same on every machine.
  return `${lines.slice(-4).join(" | ").replaceAll(`${SELF}/`, "").slice(0, 600) || "no message"} (exit ${run.status ?? "signal"})`;
}

/** One side's findings on one corpus directory, and the rules it failed, each with why. */
interface SideRun {
  findings: Finding[];
  /** Rules the run leaves without findings to compare (its process failed, the rule does not compile). */
  failures: Map<string, string>;
  /** The original's failures on some files of a rule it ran (the product reports them and carries on). */
  incidents: Map<string, string[]>;
  /** What the run says that is no rule's (the original's failures without a rule; findings of no known policy). */
  notes: string[];
}

/** The original side: the rules' JSON documents in `rulesDir`, through product-run.ts. */
async function runOriginal(dir: string, rulesDir: string, work: string, ruleIds: readonly string[], timeoutS: number): Promise<SideRun> {
  const result = join(work, "original.json");
  const tmp = join(work, "original.tmp");
  mkdirSync(tmp, { recursive: true });
  const run = await spawnAsync("node", ["--import", "tsx", join(HERE, "product-run.ts"), SELF, dir, result, rulesDir], { cwd: SELF, env: { ...process.env, SHIPBAR_ENGINE_TMPDIR: tmp }, timeoutS });
  rmSync(tmp, { recursive: true, force: true });
  if (run.status !== 0 || !existsSync(result)) {
    const why = `the original's run failed: ${failureText(run, timeoutS)}`;
    return { findings: [], failures: new Map(ruleIds.map((rule) => [rule, why])), incidents: new Map(), notes: [] };
  }
  const output = readJson<{ findings: [string, string, number, string, number?][]; compileErrors: { rule: string; error: string }[]; failures: { rule: string; code: string; message: string }[] }>(result);
  rmSync(result);
  const failures = new Map<string, string>();
  for (const e of output.compileErrors) failures.set(e.rule, `the original does not compile: ${e.error}`);
  const incidents = new Map<string, string[]>();
  const unattributed: string[] = [];
  for (const f of output.failures) {
    if (f.rule === "") unattributed.push(`${f.code}: ${f.message}`);
    else if (incidents.has(f.rule)) incidents.get(f.rule)!.push(`${f.code}: ${f.message}`);
    else incidents.set(f.rule, [`${f.code}: ${f.message}`]);
  }
  const notes = unattributed.length === 0 ? [] : [`the original's run had ${unattributed.length} failure(s) of no rule; the first: ${unattributed[0]}`];
  const findings = output.findings.map(([rule, path, line, code, column]): Finding => ({ rule, path, line, code, ...(column === undefined || column === 0 ? {} : { column }) }));
  return { findings, failures, incidents, notes };
}

/** A run-policies document's findings, each mapped to its rule by its policy's name. */
function translationFindings(stdout: string, policies: ReadonlySet<string>, notes: string[]): Finding[] {
  const report = engineJson<{ diagnostics: { policy: string; path: string; line: number; code: string; column?: unknown }[] }>(stdout);
  const findings: Finding[] = [];
  let unknown = 0;
  for (const d of report.diagnostics) {
    if (!policies.has(d.policy)) unknown += 1;
    else findings.push({ rule: d.policy, path: d.path, line: d.line, code: d.code, ...(typeof d.column === "number" && d.column > 0 ? { column: d.column } : {}) });
  }
  if (unknown > 0) notes.push(`the translations' run reported ${unknown} finding(s) of a policy that names no rule of the pack`);
  return findings;
}

/** The translation side: every rule's `.rerail` files in one run, else each rule's alone, `jobs` at once. */
async function runTranslations(engine: string, dir: string, rules: ReadonlyMap<string, readonly string[]>, work: string, jobs: number, timeoutS: number): Promise<Omit<SideRun, "incidents">> {
  const notes: string[] = [];
  const failures = new Map<string, string>();
  if (rules.size === 0) return { findings: [], failures, notes };
  const policies = new Set(rules.keys());
  const attempt = async (paths: readonly string[], out: string): Promise<{ findings: Finding[] } | { failure: string }> => {
    // Each run its own store, removed with its output: the default store, one per working directory, would hold every
    // pack's tapes and memo entries across runs until its hard limit refused new ones.
    const store = `${out}.store`;
    mkdirSync(store, { recursive: true });
    const run = await spawnAsync(engine, ["run-policies", dir, out, ...paths], { cwd: SELF, env: { ...ENGINE_ENV, RERAIL_TAPE_CACHE: join(store, "store.sqlite") }, timeoutS });
    rmSync(out, { recursive: true, force: true });
    rmSync(store, { recursive: true, force: true });
    if (run.status !== 0) return { failure: failureText(run, timeoutS) };
    try {
      return { findings: translationFindings(run.stdout, policies, notes) };
    } catch (error) {
      return { failure: `run-policies wrote no report: ${error instanceof Error ? error.message : String(error)}` };
    }
  };
  const together = await attempt([...rules.values()].flat(), join(work, "together"));
  if ("findings" in together) return { findings: together.findings, failures, notes };
  notes.push(`the translations' run of all ${rules.size} rules failed (${together.failure}); each rule ran alone`);
  const queue = [...rules.keys()];
  const findings: Finding[] = [];
  const worker = async (): Promise<void> => {
    for (let rule = queue.shift(); rule !== undefined; rule = queue.shift()) {
      const alone = await attempt(rules.get(rule)!, join(work, `alone-${rule}`));
      if ("findings" in alone) findings.push(...alone.findings.filter((f) => f.rule === rule));
      else failures.set(rule, `the translation failed: ${alone.failure}`);
    }
  };
  await Promise.all(Array.from({ length: Math.max(1, jobs) }, worker));
  return { findings, failures, notes };
}

// ---------------------------------------------------------------- a pack

interface Options {
  corpora: string;
  engine: string;
  out: string;
  jobs: number;
  timeoutS: number;
  rules: string[] | undefined;
  writeDifferences: boolean;
}

/** A corpus to compare on: its directories (several for examples that cannot share one), each finding's path prefix. */
interface Corpus {
  name: string;
  dirs: { dir: string; prefix: string }[];
  files: number;
}

/** The packs with translations: each packs/<id>/ with a pack.json whose docs/rule-syntax/packs/<id>/ has `.rerail` files or an equivalence.json. */
function translatedPacks(): { packs: string[]; notPacks: string[] } {
  const packs: string[] = [];
  const notPacks: string[] = [];
  for (const entry of readdirSync(TRANSLATIONS, { withFileTypes: true })) {
    if (!entry.isDirectory() || !readdirSync(join(TRANSLATIONS, entry.name)).some((name) => name.endsWith(".rerail") || name === "equivalence.json")) continue;
    (existsSync(join(SELF, "packs", entry.name, "pack.json")) ? packs : notPacks).push(entry.name);
  }
  return { packs: packs.sort(), notPacks: notPacks.sort() };
}

/** Each policy's `.rerail` files of a pack's translations directory, by the name its `.policy` line gives. */
function policyFiles(pack: string): Map<string, string[]> {
  const out = new Map<string, string[]>();
  const dir = join(TRANSLATIONS, pack);
  if (!existsSync(dir)) return out;
  for (const name of readdirSync(dir).filter((n) => n.endsWith(".rerail")).sort()) {
    const policy = /^\.policy\s+"([^"]+)"/m.exec(readFileSync(join(dir, name), "utf8"))?.[1];
    if (policy === undefined) continue;
    const list = out.get(policy) ?? [];
    list.push(join(dir, name));
    out.set(policy, list);
  }
  return out;
}

/** check-rr's refusal of each rule some file of which it does not pass. */
function checkRr(engine: string, rules: ReadonlyMap<string, readonly string[]>, work: string): Map<string, string> {
  const refused = new Map<string, string>();
  const paths = [...rules.values()].flat();
  if (paths.length === 0) return refused;
  const store = join(work, "check-rr.store");
  mkdirSync(store, { recursive: true });
  const checked = spawnSync(engine, ["check-rr", ...paths], { cwd: SELF, env: { ...ENGINE_ENV, RERAIL_TAPE_CACHE: join(store, "store.sqlite") }, encoding: "utf8", maxBuffer: 1 << 28 });
  rmSync(store, { recursive: true, force: true });
  let rows: { path: string; status: string; error?: string }[];
  try {
    rows = engineJson(checked.stdout ?? "");
  } catch {
    const why = `check-rr gave no report: ${(checked.stderr ?? String(checked.error ?? "")).trim().split("\n").at(-1)}`;
    for (const rule of rules.keys()) refused.set(rule, why);
    return refused;
  }
  const byPath = new Map(rows.map((row) => [resolve(SELF, row.path), row]));
  for (const [rule, files] of rules) {
    const problems = files.flatMap((file) => {
      const row = byPath.get(resolve(file));
      const name = file.slice(file.lastIndexOf("/") + 1);
      if (row === undefined) return [`check-rr did not report ${name}`];
      return row.status === "reader-passed" ? [] : [`check-rr ${row.status} ${name}: ${(row.error ?? "").replaceAll(`${SELF}/`, "")}`];
    });
    if (problems.length > 0) refused.set(rule, problems.join("; "));
  }
  return refused;
}

/** A real-code corpus at <corpora>/real/<name>, exported first when it is a git corpus not there yet, checked against its pin. */
function realCorpus(manifest: Manifest, name: string, corpora: string): { dir: string; files: number } | { missing: string } {
  const spec = manifest.corpora[name];
  if (spec === undefined) refuse(`scripts/ship/corpora.json names no corpus ${name}`);
  if (spec.sha256 === null || spec.files === null) refuse(`corpus ${name} is not pinned in scripts/ship/corpora.json: run \`translations.ts pin ${name}\``);
  const dir = join(corpora, "real", name);
  if (!existsSync(dir)) {
    if (spec.kind !== "git") return { missing: `${dir} is not prepared: run \`npm run bench:ship -- prepare\` with --corpora ${corpora}` };
    console.log(`translations: exporting corpus ${name} (${spec.commit.slice(0, 10)}) into ${dir}`);
    exportGitCorpus(name, spec, dir);
  }
  const now = hashTree(dir);
  if (now.sha256 !== spec.sha256 || now.files !== spec.files) refuse(`corpus ${dir} is ${now.files} files, ${now.sha256}; corpora.json pins ${spec.files} files, ${spec.sha256}`);
  return { dir, files: now.files };
}

/** The pack's corpora, and why any it should have is not compared. */
function packCorpora(pack: string, o: Options, work: string, notes: string[]): { corpora: Corpus[]; incomplete: Record<string, string> } {
  const corpora: Corpus[] = [];
  const incomplete: Record<string, string> = {};
  const examples = exampleFiles(readJson(join(SELF, "packs", pack, "pack.json")));
  for (const skipped of examples.skipped) notes.push(`example not compared: ${skipped}`);
  const bins = layOutExamples(examples.files);
  if (bins.length > 0) {
    const dirs = bins.map((bin, i) => {
      const dir = join(work, "examples", String(i + 1));
      for (const [path, text] of bin) {
        mkdirSync(dirname(join(dir, path)), { recursive: true });
        writeFileSync(join(dir, path), text);
      }
      return { dir, prefix: bins.length === 1 ? "" : `${i + 1}/` };
    });
    corpora.push({ name: EXAMPLES_CORPUS, dirs, files: bins.reduce((n, bin) => n + bin.size, 0) });
  } else {
    notes.push("the pack's examples hold no file");
  }

  if (!existsSync(join(SELF, "packs", pack, "conformance.json"))) {
    notes.push("no fixtures corpus: the pack has no upstream recording (no conformance.json)");
  } else {
    const dir = join(o.corpora, "recorded", pack, "files");
    const lockPath = join(o.corpora, "corpora.lock.json");
    if (!existsSync(dir)) {
      incomplete[FIXTURES_CORPUS] = `${dir} is not prepared: run \`npm run bench:ship -- prepare\` with --corpora ${o.corpora}`;
    } else {
      const tree = hashTree(dir);
      const locked = existsSync(lockPath) ? readJson<{ fixtures?: Record<string, { sha256?: string }> }>(lockPath).fixtures?.[pack] : undefined;
      if (locked?.sha256 === undefined) notes.push(`fixtures corpus not checked: ${lockPath} records none of ${pack}`);
      else if (locked.sha256 !== tree.sha256) refuse(`${dir} changed since the benchmark prepared it (corpora.lock.json)`);
      corpora.push({ name: FIXTURES_CORPUS, dirs: [{ dir, prefix: "" }], files: tree.files });
    }
  }

  const manifest = readJson<Manifest>(MANIFEST);
  const names = manifest.translations?.[pack] ?? (manifest.packs[pack] === undefined ? [] : [manifest.packs[pack]!]);
  if (names.length === 0) notes.push("no real-code corpus: scripts/ship/corpora.json names none for the pack");
  for (const name of names) {
    const real = realCorpus(manifest, name, o.corpora);
    if ("missing" in real) incomplete[name] = real.missing;
    else corpora.push({ name, dirs: [{ dir: real.dir, prefix: "" }], files: real.files });
  }
  return { corpora, incomplete };
}

async function checkPack(pack: string, o: Options): Promise<PackReport> {
  const started = performance.now();
  const shipped = readdirSync(join(SELF, "packs", pack, "rules")).filter((name) => name.endsWith(".json")).map((name) => name.slice(0, -".json".length)).sort();
  const unknown = (o.rules ?? []).filter((rule) => !shipped.includes(rule));
  if (unknown.length > 0) refuse(`pack ${pack} has no rule ${unknown.join(", ")}`);
  const ruleIds = o.rules === undefined ? shipped : shipped.filter((rule) => o.rules!.includes(rule));
  const declarationPath = join(TRANSLATIONS, pack, "equivalence.json");
  const declaration: Declaration = existsSync(declarationPath) ? parseDeclaration(readFileSync(declarationPath, "utf8"), pack) : emptyDeclaration(pack);
  const notes: string[] = [];
  if (!existsSync(declarationPath)) notes.push(`${declarationPath} is missing: read as equivalent: false`);

  const files = policyFiles(pack);
  for (const policy of files.keys()) if (!shipped.includes(policy)) notes.push(`translation of no rule of the pack: .policy "${policy}" (${files.get(policy)!.map((f) => f.slice(f.lastIndexOf("/") + 1)).join(", ")})`);
  const translated = new Map(ruleIds.flatMap((rule) => (files.has(rule) ? [[rule, files.get(rule)!] as const] : [])));

  const work = join(o.out, "work", pack);
  rmSync(work, { recursive: true, force: true });
  mkdirSync(work, { recursive: true });
  const refused = checkRr(o.engine, translated, work);
  const runnable = new Map([...translated].filter(([rule]) => !refused.has(rule)));
  const rulesDir = join(work, "rules");
  mkdirSync(rulesDir);
  for (const rule of ruleIds) writeFileSync(join(rulesDir, `${rule}.json`), readFileSync(join(SELF, "packs", pack, "rules", `${rule}.json`)));
  const { corpora, incomplete } = packCorpora(pack, o, work, notes);

  const inputs = new Map<string, RuleInput>(ruleIds.map((rule) => {
    const refusal = refused.get(rule);
    return [rule, { rule, translation: translated.get(rule)?.join(", "), ...(refusal === undefined ? {} : { refused: refusal }), corpora: {} }];
  }));
  for (const corpus of corpora) {
    const original: Finding[] = [];
    const translation: Finding[] = [];
    const failures = new Map<string, string>();
    const incidents = new Map<string, string[]>();
    for (const [i, { dir, prefix }] of corpus.dirs.entries()) {
      const runWork = join(work, "runs", `${corpus.name}-${i + 1}`);
      mkdirSync(runWork, { recursive: true });
      console.log(`translations: ${pack} on ${corpus.name}${corpus.dirs.length > 1 ? ` (${i + 1}/${corpus.dirs.length})` : ""}: ${ruleIds.length} originals, ${runnable.size} translations`);
      const [o1, t1] = await Promise.all([runOriginal(dir, rulesDir, runWork, ruleIds, o.timeoutS), runTranslations(o.engine, dir, runnable, runWork, o.jobs, o.timeoutS)]);
      for (const [rule, why] of [...o1.failures, ...t1.failures]) if (!failures.has(rule)) failures.set(rule, why);
      for (const [rule, list] of o1.incidents) incidents.set(rule, [...(incidents.get(rule) ?? []), ...list]);
      for (const note of [...o1.notes, ...t1.notes]) notes.push(`${corpus.name}: ${note}`);
      original.push(...o1.findings.map((f) => ({ ...f, path: prefix + f.path })));
      translation.push(...t1.findings.map((f) => ({ ...f, path: prefix + f.path })));
    }
    const originalBy = byRule(original);
    const translationBy = byRule(translation);
    for (const rule of runnable.keys()) {
      const why = failures.get(rule);
      const run = { original: originalBy.get(rule) ?? [], translation: translationBy.get(rule) ?? [], ...(incidents.has(rule) ? { originalFailures: incidents.get(rule)! } : {}) };
      inputs.get(rule)!.corpora[corpus.name] = why === undefined ? run : { failure: why };
    }
  }

  const results = ruleIds.map((rule) => ruleResult(inputs.get(rule)!));
  const ran = corpora.map((c) => c.name);
  const problems = gateProblems(declaration, results, ran, shipped);
  if (declaration.equivalent) for (const [corpus, why] of Object.entries(incomplete)) problems.push(`not compared on ${corpus}: ${why}`);
  if (o.writeDifferences) {
    writeFileSync(declarationPath, renderDeclaration(writeDifferences(declaration, results, ran, incomplete)));
    console.log(`translations: wrote ${declarationPath}`);
  }
  const report: PackReport = {
    pack,
    equivalent: declaration.equivalent,
    corpora: corpora.map((c) => ({ name: c.name, dir: c.dirs.map((d) => d.dir).join(", "), files: c.files })),
    incomplete,
    notes,
    summary: summarize(results),
    rules: results,
    problems,
    failed: declaration.equivalent && problems.length > 0,
  };
  writeFileSync(join(o.out, `${pack}.json`), `${JSON.stringify({ ...report, seconds: Math.round((performance.now() - started) / 1000) }, null, 1)}\n`);
  rmSync(work, { recursive: true, force: true });
  return report;
}

// ---------------------------------------------------------------- commands

function pin(name: string, corpora: string): void {
  const manifest = readJson<Manifest>(MANIFEST);
  const spec = manifest.corpora[name];
  if (spec === undefined) refuse(`scripts/ship/corpora.json names no corpus ${name}`);
  if (spec.kind !== "git") refuse(`corpus ${name} is not a git corpus; the benchmark's prepare --record pins it`);
  const dir = join(corpora, "real", name);
  exportGitCorpus(name, spec, dir);
  const tree = hashTree(dir);
  spec.files = tree.files;
  spec.sha256 = tree.sha256;
  writeFileSync(MANIFEST, `${JSON.stringify(manifest, null, 2)}\n`);
  console.log(`translations: pinned ${name}: ${tree.files} files, ${tree.sha256}`);
}

async function main(): Promise<number> {
  const { values, positionals } = parseArgs({
    allowPositionals: true,
    options: {
      pack: { type: "string" },
      packs: { type: "string" },
      all: { type: "boolean", default: false },
      rules: { type: "string" },
      corpora: { type: "string" },
      engine: { type: "string" },
      out: { type: "string" },
      jobs: { type: "string" },
      timeout: { type: "string" },
      "write-differences": { type: "boolean", default: false },
    },
  });
  const corpora = resolve(values.corpora ?? join(SELF, ".rerail", "translations", "corpora"));
  const [command = "check", ...rest] = positionals;
  if (command === "pin") {
    if (rest.length !== 1) refuse("usage: translations.ts pin <corpus> [--corpora <dir>]");
    pin(rest[0]!, corpora);
    return 0;
  }
  if (command !== "check" || rest.length > 0) refuse(`unknown command ${[command, ...rest].join(" ")}: check or pin`);

  const { packs: available, notPacks } = translatedPacks();
  const chosen = [values.pack !== undefined, values.packs !== undefined, values.all].filter(Boolean).length;
  if (chosen !== 1) refuse("name the packs: --pack <pack>, --packs a,b or --all");
  const packs = values.all ? available : values.pack !== undefined ? [values.pack] : values.packs!.split(",").map((p) => p.trim()).filter((p) => p !== "");
  for (const pack of packs) if (!available.includes(pack)) refuse(`${pack} is not a pack with translations (those: ${available.join(", ")})`);
  if (values.all && notPacks.length > 0) console.log(`translations: not packs (no packs/<id>/pack.json), not checked: ${notPacks.join(", ")}`);
  const rules = values.rules?.split(",").map((r) => r.trim()).filter((r) => r !== "");
  if (rules !== undefined && packs.length !== 1) refuse("--rules takes one --pack");
  if (rules !== undefined && values["write-differences"]) refuse("--write-differences rewrites a whole pack's lists: run it without --rules");
  const engine = values.engine === undefined ? engineBinary(SELF) : resolve(values.engine);
  if (engine === undefined || !existsSync(engine)) refuse(`no rerail-engine${values.engine === undefined ? " in target/release" : ` at ${engine}`}: build it (cargo build --release --manifest-path native/Cargo.toml --bin rerail-engine) or pass --engine`);
  const out = resolve(values.out ?? join(SELF, ".rerail", "translations", "out"));
  mkdirSync(out, { recursive: true });
  const o: Options = {
    corpora,
    engine,
    out,
    jobs: values.jobs === undefined ? Math.max(1, Math.min(8, Math.floor(availableParallelism() / 2))) : Number(values.jobs),
    timeoutS: values.timeout === undefined ? 3600 : Number(values.timeout),
    rules,
    writeDifferences: values["write-differences"],
  };
  if (!Number.isInteger(o.jobs) || o.jobs < 1 || !(o.timeoutS > 0)) refuse("--jobs takes a whole number from 1, --timeout seconds above 0");

  let failed = false;
  for (const pack of packs) {
    const report = await checkPack(pack, o);
    console.log(renderPack(report));
    failed ||= report.failed;
  }
  console.log(`translations: results in ${out}`);
  return failed ? 1 : 0;
}

try {
  process.exitCode = await main();
} catch (error) {
  if (!(error instanceof Refusal)) throw error;
  console.error(`translations: ${error.message}`);
  process.exitCode = 2;
}

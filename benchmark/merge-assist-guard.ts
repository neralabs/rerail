/**
 * Merge Assist guard forms eval: how the context eval's guard sources (signals, embeddings, both) fire on labelled code,
 * offline, with no agent and no judge. See benchmark/merge-assist-context.ts (the index and the runs) and
 * benchmark/merge-assist-context-hook.ts (the guard).
 *
 *   node --import tsx benchmark/merge-assist-guard.ts --index index.json [--thresholds 0.3,0.35,...] [--hook-calls N]
 *
 * Each labelled teammate file in cases.json and each unchanged project file is a simulated patch: its whole content
 * added at its path. The index's real decisions and distractors (the conflict tasks' teammate decisions are left out:
 * they are only in conflict runs) are scored on each, as the hook scores a patch. A patch breaks a decision when it is
 * that decision's "caught" teammate file or the decision's oracle flags it; a fire on a patch that breaks nothing it
 * fires for is noise. It prints:
 * - the integrity of the index: the check every run makes, real decisions drafted from their own capture case only,
 *   and no teammate file line in any prompt the judge saw (a leak fails the eval);
 * - per real decision and source: recall on its own caught files, fires on its own passed files (near misses), and
 *   fires on every other patch it does not break (cross noise);
 * - distractor fires per patch, per source;
 * - an embedding threshold sweep on these forms, and the threshold with the best recall at no more than 0.5 false
 *   fires per patch. That threshold is reported only: the frozen one comes from `--sweep` on the dev split's real
 *   patches, so these forms stay held out;
 * - the hook's own cost per call, from `--hook-calls` runs of the real hook process on project-file patches.
 */
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { performance } from "node:perf_hooks";
import { fileURLToPath } from "node:url";
import {
  bestThreshold,
  type BuiltIndex,
  clip,
  type Dataset,
  EMBED_GRID,
  EMBED_THRESHOLD,
  fixed,
  grade,
  hookEntry,
  loadDataset,
  loadIndex,
  MODEL_PATH,
  NOISE_BUDGET,
  printSweep,
  rate,
  ratio,
  type SweepRow,
  sweepRows,
  table,
  teammateLeaks,
} from "./merge-assist-context.js";
import { fires, guardBlocks, type GuardScore, type GuardSource, type HookConfig, type HookLogEntry, type IndexEntry, loadEmbedder, scorePath } from "./merge-assist-context-hook.js";

const HOOK = fileURLToPath(new URL("./merge-assist-context-hook.ts", import.meta.url));
const HOOK_TIMEOUT_MS = 60_000;
const USAGE = "Usage: node --import tsx benchmark/merge-assist-guard.ts --index index.json [--thresholds 0.3,0.35,...] [--hook-calls N]";

interface Options {
  index: string;
  thresholds: readonly number[];
  hookCalls: number;
}

function parseOptions(argv: readonly string[]): Options {
  let index: string | undefined;
  let thresholds: readonly number[] = EMBED_GRID;
  let hookCalls = 5;
  for (let position = 0; position < argv.length; position++) {
    const flag = argv[position]!;
    const value = argv[++position];
    if (flag === "--index" && value !== undefined) index = value;
    else if (flag === "--thresholds" && value !== undefined) {
      thresholds = value.split(",").map(Number);
      if (thresholds.some(threshold => !Number.isFinite(threshold) || threshold < 0 || threshold > 1)) throw new Error(`--thresholds takes numbers from 0 to 1\n${USAGE}`);
    } else if (flag === "--hook-calls" && value !== undefined && Number.isInteger(Number(value)) && Number(value) >= 0) hookCalls = Number(value);
    else throw new Error(USAGE);
  }
  if (index === undefined) throw new Error(USAGE);
  return { index, thresholds, hookCalls };
}

/** A simulated patch: a labelled teammate file (with its decision and label) or an unchanged project file. */
interface Form {
  readonly path: string;
  readonly content: string;
  readonly owner: string | null;
  readonly expect: "caught" | "passed" | null;
  /** The decisions it breaks: its own when labelled caught, and every decision whose oracle flags it. */
  readonly breaks: readonly string[];
  readonly scores: readonly GuardScore[];
}

/** Every simulated patch, scored against `index` as the hook scores a patch (one model load, one batch). */
async function scoredForms(dataset: Dataset, index: readonly IndexEntry[], embeddings: HookConfig["embeddings"]): Promise<{ forms: Form[]; loadMs: number; embedMs: number; blocks: number }> {
  const raw = [
    ...dataset.labelled.flatMap(entry => (entry.teammate ?? []).map(file => ({ path: file.path, content: file.content, owner: entry.id, expect: file.expect }))),
    ...Object.entries(dataset.project).map(([path, content]) => ({ path, content, owner: null, expect: null })),
  ];
  const blocks = raw.map(form => guardBlocks(form.path, form.content));
  const { embedder, loadMs } = loadEmbedder(MODEL_PATH);
  const started = performance.now();
  const vectors = await embedder.embed(blocks.flat());
  const embedMs = performance.now() - started;
  let next = 0;
  const forms = raw.map((form, position) => {
    const own = vectors.slice(next, next + blocks[position]!.length);
    next += blocks[position]!.length;
    const hits = grade([{ path: form.path, status: "added", added: form.content.split("\n").map((text, line) => ({ line: line + 1, text })) }], dataset.decisions);
    const breaks = [...new Set([...(form.expect === "caught" ? [form.owner!] : []), ...Object.keys(hits).filter(id => hits[id]!.length > 0)])];
    return { ...form, breaks, scores: scorePath(index, embeddings, form.path, form.content, own) };
  });
  return { forms, loadMs, embedMs, blocks: vectors.length };
}

function fired(form: Form, decisionId: string, source: GuardSource, threshold: number | null): boolean {
  return form.scores.some(score => score.ids.includes(decisionId) && fires(score, source, threshold));
}

/** The index's integrity, as this eval needs it: real decisions drafted from their own case only, no teammate line ever judged. */
function checkProvenance(dataset: Dataset, built: BuiltIndex): boolean {
  const reals = built.entries.filter(entry => dataset.decisions.some(decision => decision.id === entry.id));
  const wrong = reals.filter(entry => entry.provenance.by !== "draftDecision" || entry.provenance.case !== entry.id).map(entry => entry.id);
  const strays = built.judgeCalls.filter(call => dataset.decisions.some(decision => decision.id === call.entryId) && call.kind !== "capture").map(call => `${call.entryId} ${call.kind}`);
  const leaks = teammateLeaks(dataset, built.judgeCalls);
  console.log(`\nReal decisions in the index: ${reals.length}/${dataset.decisions.length}${built.absent.length === 0 ? "" : `; absent: ${built.absent.map(entry => `${entry.id} (${entry.outcome})`).join(", ")}`}`);
  console.log(`Drafted by draftDecision from their own capture case: ${wrong.length === 0 ? "all" : `NOT ${wrong.join(", ")}`}`);
  console.log(`Real decisions' judge calls other than capture: ${strays.length === 0 ? "none" : strays.join(", ")}`);
  console.log(`Teammate file lines in the ${built.judgeCalls.length} prompts the judge saw: ${leaks.length === 0 ? "none" : `${leaks.length} LEAKED`}`);
  for (const leak of leaks) console.log(`  LEAK ${leak}`);
  return wrong.length === 0 && leaks.length === 0;
}

/** Per real decision and source: recall on its caught files, fires on its passed files, and cross noise. */
function printDecisions(dataset: Dataset, reals: readonly IndexEntry[], forms: readonly Form[], threshold: number | null): void {
  const sources: Array<{ source: GuardSource; threshold: number | null; label: string }> = [
    { source: "signals", threshold: null, label: "signals" },
    ...(threshold === null ? [] : [{ source: "embed" as const, threshold, label: `embed ≥${threshold}` }, { source: "both" as const, threshold, label: `both ≥${threshold}` }]),
  ];
  const rows = dataset.decisions.flatMap(decision => {
    const entry = reals.find(candidate => candidate.id === decision.id);
    if (entry === undefined) return [[decision.id, "ABSENT", "", "", "", ""]];
    const own = forms.filter(form => form.owner === decision.id);
    const caught = own.filter(form => form.expect === "caught");
    const passed = own.filter(form => form.expect === "passed");
    const others = forms.filter(form => form.owner !== decision.id);
    return sources.map(({ source, threshold: at, label }) => {
      const noise = others.filter(form => !form.breaks.includes(decision.id) && fired(form, decision.id, source, at));
      const crossTrue = others.filter(form => form.breaks.includes(decision.id) && fired(form, decision.id, source, at)).length;
      return [
        decision.id,
        label,
        caught.length === 0 ? "no labelled files" : rate(ratio(caught.filter(form => fired(form, decision.id, source, at)).length, caught.length)),
        passed.length === 0 ? "-" : rate(ratio(passed.filter(form => fired(form, decision.id, source, at)).length, passed.length)),
        `${rate(ratio(noise.length, others.length))}${crossTrue === 0 ? "" : ` (+${crossTrue} on files its oracle flags)`}`,
        clip(noise.map(form => (form.owner === null ? form.path : `${form.path}@${form.owner}`)).join(" "), 90),
      ];
    });
  });
  console.log("\nPer real decision and source (fires need the index's own scope to cover the path)");
  console.log(table(["decision", "source", "recall on own caught", "fires on own passed", "cross noise", "noise files"], rows));
}

function printDistractors(distractors: readonly IndexEntry[], forms: readonly Form[], threshold: number | null): void {
  const sources: Array<[GuardSource, number | null]> = [["signals", null], ...(threshold === null ? [] : [["embed", threshold], ["both", threshold]] as Array<[GuardSource, number]>)];
  console.log(`\nDistractor fires per patch (${distractors.length} distractors, ${forms.length} patches)`);
  console.log(table(["source", "threshold", "fires per patch", "most firing"], sources.map(([source, at]) => {
    const counts = distractors.map(entry => ({ id: entry.id, fires: forms.filter(form => fired(form, entry.id, source, at)).length })).sort((left, right) => right.fires - left.fires);
    const total = counts.reduce((sum, entry) => sum + entry.fires, 0);
    return [source, at === null ? "-" : String(at), fixed(total / forms.length), counts.filter(entry => entry.fires > 0).slice(0, 4).map(entry => `${entry.id} ${entry.fires}`).join(", ")];
  })));
}

/** `count` runs of the real hook process, as Codex runs it, on project-file patches under the control: its cost per call. */
async function hookCost(built: BuiltIndex, index: readonly IndexEntry[], dataset: Dataset, count: number): Promise<void> {
  if (count === 0) return;
  const folder = await realpath(await mkdtemp(join(tmpdir(), "rerail-ma-guard-hook-")));
  try {
    const log = join(folder, "hook.log");
    const config: HookConfig = {
      variant: null,
      index,
      prompt: "",
      embeddings: Object.fromEntries(index.map(entry => [entry.id, built.embeddings[entry.id]!])),
      modelPath: MODEL_PATH,
      files: Object.keys(dataset.project),
      root: folder,
      folder,
      log,
      state: join(folder, "hook-state.json"),
    };
    const configPath = join(folder, "hook-config.json");
    await writeFile(configPath, JSON.stringify(config));
    const loader = fileURLToPath(import.meta.resolve("tsx"));
    const files = Object.entries(dataset.project).filter(([path]) => path.startsWith("src/"));
    const walls: number[] = [];
    for (let call = 0; call < count; call++) {
      const [path, content] = files[call % files.length]!;
      const patch = `*** Begin Patch\n*** Add File: ${path}\n${content.split("\n").map(line => `+${line}`).join("\n")}\n*** End Patch`;
      const input = { hook_event_name: "PreToolUse", tool_name: "apply_patch", cwd: folder, tool_input: { command: ["apply_patch", patch] } };
      const started = performance.now();
      const result = spawnSync(process.execPath, ["--import", loader, HOOK], { input: JSON.stringify(input), env: { ...process.env, MA_CONTEXT_CONFIG: configPath }, timeout: HOOK_TIMEOUT_MS, maxBuffer: 1024 * 1024 });
      walls.push(performance.now() - started);
      if (result.status !== 0) throw new Error(`The hook exited ${result.status}: ${result.stderr.toString("utf8").slice(0, 400)}`);
    }
    const entries = readFileSync(log, "utf8").split("\n").filter(line => line.trim() !== "").map(line => JSON.parse(line) as HookLogEntry);
    const errors = entries.flatMap(entry => (entry.error === undefined ? [] : [entry.error]));
    if (errors.length > 0) throw new Error(`The hook logged errors: ${errors.join("; ")}`);
    console.log(`\nHook cost per apply_patch call (${count} runs of the hook process, control config, ${index.length} decisions)`);
    console.log(table(["call", "path", "model load ms", "embed ms", "blocks", "scored items", "process wall ms"], entries.map((entry, position) => [
      String(position + 1),
      entry.paths.join(" "),
      fixed(entry.embedding?.loadMs ?? Number.NaN, 1),
      fixed(entry.embedding?.embedMs ?? Number.NaN, 1),
      String(entry.embedding?.blocks ?? 0),
      String(entry.guard?.length ?? 0),
      fixed(walls[position]!, 0),
    ])));
  } finally {
    await rm(folder, { recursive: true, force: true });
  }
}

async function main(): Promise<void> {
  const options = parseOptions(process.argv.slice(2));
  const dataset = loadDataset();
  const built = loadIndex(options.index);
  const honest = checkProvenance(dataset, built);
  const reals = built.entries.filter(entry => entry.provenance.by === "draftDecision").map(hookEntry);
  const distractors = built.entries.filter(entry => entry.provenance.by === "deriveSignals" && entry.conflictsWith === undefined).map(hookEntry);
  const index = [...reals, ...distractors];
  const { forms, loadMs, embedMs, blocks } = await scoredForms(dataset, index, built.embeddings);
  const teammate = forms.filter(form => form.owner !== null).length;
  console.log(`\n${forms.length} simulated patches: ${teammate} labelled teammate files, ${forms.length - teammate} project files; model load ${fixed(loadMs, 1)} ms, ${blocks} blocks embedded in ${fixed(embedMs, 1)} ms`);
  const rows = sweepRows(forms, options.thresholds);
  const noise = (row: SweepRow) => row.falsePerPatch;
  console.log(`\nEmbedding threshold sweep on the forms (recall over the decisions each patch breaks; noise: fires per patch on decisions it does not break)`);
  printSweep(rows, "false fires per patch", noise);
  const formsBest = bestThreshold(rows, "embed", noise);
  const formsLine = formsBest === undefined ? `no embed threshold keeps false fires per patch at or below ${NOISE_BUDGET}` : `${formsBest.threshold} (recall ${rate(formsBest.recall)}, ${fixed(formsBest.falsePerPatch)} false fires per patch)`;
  console.log(`\nForms-chosen embed threshold (reported only, never frozen from here): ${formsLine}`);
  console.log(`Frozen embed threshold (from --sweep on the dev split's real patches): ${EMBED_THRESHOLD ?? "not frozen yet"}`);
  const shown = EMBED_THRESHOLD ?? formsBest?.threshold ?? null;
  if (EMBED_THRESHOLD === null && shown !== null) console.log(`The tables below use the forms-chosen ${shown} for embed and both, since none is frozen yet.`);
  printDecisions(dataset, reals, forms, shown);
  printDistractors(distractors, forms, shown);
  await hookCost(built, index, dataset, options.hookCalls);
  if (!honest) process.exitCode = 1;
}

await main();

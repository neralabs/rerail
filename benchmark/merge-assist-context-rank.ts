/**
 * The Merge Assist context eval's session-focus ranker, started detached by the hook under `orient: focus` (see
 * benchmark/merge-assist-context-hook.ts).
 *
 *   node --import <absolute tsx loader> <absolute path of this file> <hook config>
 *
 * It asks the product's judge (`createAgentJudge`, Codex first, a scratch database in the run folder) which of the
 * index's decisions the task needs, with `rankPrompt`: the task prompt and every decision whose scope covers a project
 * file, under opaque ids (`rankerInput`). It writes `focus.json` in the run folder, atomically: the picked index ids,
 * most relevant first, how long it took, and why it failed when it did.
 */
import { createHash, randomBytes } from "node:crypto";
import { readFileSync, renameSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { performance } from "node:perf_hooks";
import { openRerailDatabase } from "../src/engine/index.js";
import { createAgentJudge, rankJsonSchemaFor, rankPrompt, rankVerdictSchema } from "../src/runtime/index.js";
import { type FocusFile, focusPath, type HookConfig, rankerInput } from "./merge-assist-context-hook.js";

/** The repository key the ranker's judge calls are budgeted and recorded under, in its scratch database. */
const RANKER_REPOSITORY = "eval:merge-assist-context";

async function rank(config: HookConfig): Promise<Omit<FocusFile, "ms">> {
  const input = rankerInput(config);
  const prompt = rankPrompt({ task: input.task, decisions: input.decisions }, randomBytes(16).toString("hex"));
  const base = { promptChars: prompt.length, candidates: input.decisions.length };
  const database = openRerailDatabase(join(config.folder, "ranker.sqlite"));
  try {
    const judge = createAgentJudge({ database, preferredHost: "codex" });
    const outcome = await judge.judge({
      kind: "rank",
      repositoryKey: RANKER_REPOSITORY,
      prompt,
      jsonSchema: rankJsonSchemaFor(input.decisions.map(decision => decision.id)),
      parse: value => rankVerdictSchema.parse(value),
      cacheKey: createHash("sha256").update(JSON.stringify({ task: input.task, decisions: input.decisions })).digest("hex"),
    });
    if (!outcome.ok) return { ...base, ok: false, ids: [], error: `${outcome.reason}: ${outcome.detail}`, runner: null };
    const known = new Map(input.decisions.map((decision, position) => [decision.id, input.entries[position]!]));
    const ids = [...new Set(outcome.value.ids.flatMap(id => (known.has(id) ? [known.get(id)!] : [])))];
    return { ...base, ok: true, ids, error: null, runner: outcome.runner };
  } finally {
    database.$client.close();
  }
}

const configPath = process.argv[2];
if (configPath === undefined) throw new Error("Usage: merge-assist-context-rank.ts <hook config>");
const config = JSON.parse(readFileSync(configPath, "utf8")) as HookConfig;
const started = performance.now();
let result: Omit<FocusFile, "ms">;
try {
  result = await rank(config);
} catch (error) {
  result = { ok: false, ids: [], error: error instanceof Error ? error.message : String(error), runner: null, promptChars: 0, candidates: 0 };
}
const out = focusPath(config.folder);
const focus: FocusFile = { ...result, ms: Math.round(performance.now() - started) };
writeFileSync(`${out}.tmp`, `${JSON.stringify(focus)}\n`);
renameSync(`${out}.tmp`, out);

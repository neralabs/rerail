/**
 * The Merge Assist context eval's focus ranker, started detached by the hook under `orient: focus` or `area-focus`
 * (see benchmark/merge-assist-context-hook.ts).
 *
 *   node --import <absolute tsx loader> <absolute path of this file> <hook config> [<touched file>]
 *
 * It asks the product's judge (`createAgentJudge`, Codex first, a scratch database in the run folder) which of the
 * index's decisions the task needs, with `rankPrompt`: the task prompt and every decision whose scope covers a project
 * file, under opaque ids (`rankerInput`), answered as at most the variant's `rankMax` ids (the session's under
 * `area-focus`: `sessionRankMax`). Without a touched file it ranks for the session and writes `focus.json` in the run
 * folder; with one, it ranks the decisions covering that file or a project file in its area and writes
 * `focus-<area>.json` (`areaFocusPath`), under `sessionRankMax` after waiting up to `SESSION_WAIT_MS` for `focus.json`
 * and listing the session's picks first (`sessionFirst`). Either is written atomically: the picked index ids, most
 * relevant first, how long it took, and why it failed when it did.
 */
import { createHash, randomBytes } from "node:crypto";
import { readFileSync, renameSync, writeFileSync } from "node:fs";
import { performance } from "node:perf_hooks";
import { setTimeout as sleep } from "node:timers/promises";
import { openRerailDatabase } from "../src/engine/index.js";
import { createAgentJudge, MAX_RANKED, rankJsonSchemaFor, rankPrompt, rankVerdictSchemaFor } from "../src/runtime/index.js";
import {
  areaFocusPath,
  areaOf,
  FOCUS_POLL_MS,
  type FocusFile,
  focusPath,
  type HookConfig,
  rankerInput,
  readFocusFile,
  SESSION_WAIT_MS,
  sessionFirst,
} from "./merge-assist-context-hook.js";

/** The repository key the ranker's judge calls are budgeted and recorded under, in its scratch database. */
const RANKER_REPOSITORY = "eval:merge-assist-context";

/** The session ranker's picks once `focus.json` is there, at most {@link SESSION_WAIT_MS} from now; none if it failed or is late. */
async function sessionPicks(folder: string): Promise<string[]> {
  const started = performance.now();
  let focus = readFocusFile(focusPath(folder));
  while (focus === undefined && performance.now() - started < SESSION_WAIT_MS) {
    await sleep(FOCUS_POLL_MS);
    focus = readFocusFile(focusPath(folder));
  }
  return focus?.ok === true ? [...focus.ids] : [];
}

async function rank(config: HookConfig, path: string | null, out: string): Promise<Omit<FocusFile, "ms">> {
  const sessionMax = config.variant?.orient === "area-focus" ? config.variant.sessionRankMax : null;
  const session = path !== null && sessionMax !== null ? await sessionPicks(config.folder) : undefined;
  const input = session === undefined ? rankerInput(config, path) : sessionFirst(rankerInput(config, path), session);
  const max = (path === null ? sessionMax : null) ?? config.variant?.rankMax ?? MAX_RANKED;
  const prompt = rankPrompt({ task: input.task, decisions: input.decisions }, randomBytes(16).toString("hex"), max);
  const base = { promptChars: prompt.length, candidates: input.decisions.length, ...(session === undefined ? {} : { session }) };
  // Each ranker its own database: area rankers run at once, in separate processes.
  const database = openRerailDatabase(out.replace(/\.json$/u, ".sqlite"));
  try {
    const judge = createAgentJudge({ database, preferredHost: "codex" });
    const verdict = rankVerdictSchemaFor(max);
    const outcome = await judge.judge({
      kind: "rank",
      repositoryKey: RANKER_REPOSITORY,
      prompt,
      jsonSchema: rankJsonSchemaFor(input.decisions.map(decision => decision.id), max),
      parse: value => verdict.parse(value),
      cacheKey: createHash("sha256").update(JSON.stringify({ task: input.task, decisions: input.decisions, max })).digest("hex"),
    });
    if (!outcome.ok) return { ...base, ok: false, ids: [], error: `${outcome.reason}: ${outcome.detail}`, runner: null };
    const known = new Map(input.decisions.map((decision, position) => [decision.id, input.entries[position]!]));
    const ids = [...new Set(outcome.value.ids.flatMap(id => (known.has(id) ? [known.get(id)!] : [])))];
    return { ...base, ok: true, ids, error: null, runner: outcome.runner };
  } finally {
    database.$client.close();
  }
}

const [configPath, touched] = process.argv.slice(2);
if (configPath === undefined) throw new Error("Usage: merge-assist-context-rank.ts <hook config> [<touched file>]");
const config = JSON.parse(readFileSync(configPath, "utf8")) as HookConfig;
const path = touched ?? null;
const out = path === null ? focusPath(config.folder) : areaFocusPath(config.folder, areaOf(path));
const started = performance.now();
let result: Omit<FocusFile, "ms">;
try {
  result = await rank(config, path, out);
} catch (error) {
  result = { ok: false, ids: [], error: error instanceof Error ? error.message : String(error), runner: null, promptChars: 0, candidates: 0 };
}
const focus: FocusFile = { ...result, ms: Math.round(performance.now() - started) };
writeFileSync(`${out}.tmp`, `${JSON.stringify(focus)}\n`);
renameSync(`${out}.tmp`, out);

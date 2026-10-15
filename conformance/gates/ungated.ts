import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

import { GATES_MANIFEST } from "./manifest.js";
import type { CommandRunner } from "./process.js";
import type { Check } from "./types.js";

/** The frozen list of shipped rules in packs without an upstream adapter, and the tool-packs still validated elsewhere. */
export const UNGATED_RULES = "conformance/gates/ungated-rules.json";
export const PENDING = "conformance/gates/pending.json";

const byText = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0);

export interface UngatedResult {
  checks: Check[];
  /** Packs with shipped rules whose upstream tool is neither gated nor pending, with the count of those rules. */
  notGated: { pack: string; rules: number }[];
  /** Pending tool-packs, with the reason and the count of shipped rules of that tool. */
  pending: { toolPack: string; reason: string; rules: number }[];
}

function readJson<T>(path: string): T {
  return JSON.parse(readFileSync(path, "utf8")) as T;
}

export interface PackState {
  pack: string;
  /** The pack's upstream tools (conformance.json tools[]). */
  tools: string[];
  /** The ids of the rules pack.json ships. */
  shipped: string[];
  /** The tools with a gates.yaml: their rules are judged by the gates. */
  gatedTools: string[];
  /** Each shipped rule's upstream tool, from its conformance.json entry; absent when it has none. */
  toolOf: Map<string, string>;
}

/** Every pack under `root` with its tools, its shipped rule ids, its gated tools and each shipped rule's tool. */
export function packStates(root: string): PackState[] {
  const packs = join(root, "packs");
  return readdirSync(packs, { withFileTypes: true })
    .filter((entry) => entry.isDirectory() && existsSync(join(packs, entry.name, "pack.json")))
    .map((entry) => entry.name)
    .sort(byText)
    .map((pack) => {
      const conformance = join(packs, pack, "conformance.json");
      const manifest = existsSync(conformance) ? readJson<{ tools?: { name: string }[]; rules?: { id: string; upstream?: { tool?: string } }[] }>(conformance) : {};
      const tools = (manifest.tools ?? []).map((tool) => tool.name);
      const shipped = (readJson<{ rules?: { id: string }[] }>(join(packs, pack, "pack.json")).rules ?? []).map((rule) => rule.id).sort(byText);
      const gatedTools = tools.filter((tool) => existsSync(join(packs, pack, "upstream", tool, GATES_MANIFEST)));
      const toolOf = new Map((manifest.rules ?? []).filter((rule) => typeof rule.upstream?.tool === "string").map((rule) => [rule.id, rule.upstream!.tool!]));
      return { pack, tools, shipped, gatedTools, toolOf };
    });
}

type Frozen = Record<string, string[]>;
export type Pending = Record<string, string>;

/**
 * What the base commit holds at `path`: `missing` when the file is not there (the guard is skipped), its parsed JSON,
 * or an error (the guard fails closed). A base commit not present locally is fetched first.
 */
export async function readAtBase(root: string, base: string, path: string, runner: CommandRunner): Promise<{ kind: "missing" } | { kind: "ok"; value: unknown } | { kind: "error"; message: string }> {
  if (!/^[0-9a-f]{7,40}$/u.test(base)) return { kind: "error", message: `--base ${base} is not a commit sha` };
  const present = await runner.run(["git", "cat-file", "-e", `${base}^{commit}`], { cwd: root });
  if (present.exitCode !== 0) {
    const fetched = await runner.run(["git", "fetch", "--depth=1", "origin", base], { cwd: root });
    if (fetched.exitCode !== 0) return { kind: "error", message: `the base commit ${base} is not present and git fetch failed: ${fetched.stderr.trim().split("\n")[0] ?? ""}` };
  }
  const tree = await runner.run(["git", "ls-tree", "--name-only", base, "--", path], { cwd: root });
  if (tree.exitCode !== 0) return { kind: "error", message: `cannot list ${path} at ${base}: ${tree.stderr.trim()}` };
  if (tree.stdout.trim() === "") return { kind: "missing" };
  const shown = await runner.run(["git", "show", `${base}:${path}`], { cwd: root });
  if (shown.exitCode !== 0) return { kind: "error", message: `cannot read ${path} at ${base}: ${shown.stderr.trim()}` };
  try {
    return { kind: "ok", value: JSON.parse(shown.stdout) };
  } catch (error) {
    return { kind: "error", message: `${path} at ${base} is not JSON: ${error instanceof Error ? error.message : String(error)}` };
  }
}

/**
 * How the gates treat one shipped rule, by its upstream tool: `gated` when the tool has a gates.yaml (the tool-pack run
 * judges it; a rule with no conformance entry in a gated pack is reported there as unmapped), `pending` when pending.json
 * names the tool, else `frozen`: it must be on the frozen list. Decided per (pack, tool), so a gated pack's rule of
 * another tool is never waved through.
 */
export function shippedRuleTreatment(state: PackState, id: string, pending: Pending): { kind: "gated" | "frozen" } | { kind: "pending"; toolPack: string } {
  const tool = state.toolOf.get(id);
  if (tool === undefined) {
    // No conformance entry: a gated tool-pack run reports it as unmapped; in a pack only pending, its tool decides later.
    if (state.gatedTools.length > 0) return { kind: "gated" };
    const pendingTool = state.tools.find((name) => `${state.pack}/${name}` in pending);
    return pendingTool === undefined ? { kind: "frozen" } : { kind: "pending", toolPack: `${state.pack}/${pendingTool}` };
  }
  if (state.gatedTools.includes(tool)) return { kind: "gated" };
  if (`${state.pack}/${tool}` in pending) return { kind: "pending", toolPack: `${state.pack}/${tool}` };
  return { kind: "frozen" };
}

/**
 * The not-gated and pending checks: every shipped rule whose tool is neither gated nor pending is on the frozen list (a
 * new one is blocked), every listed rule still ships ungated (a stale entry is removed), every pending tool-pack is not
 * gated yet, and, with `base`, both lists only shrink against the base commit's.
 */
export async function ungatedChecks(root: string, options: { base?: string; runner: CommandRunner }): Promise<UngatedResult> {
  const frozen = readJson<Frozen>(join(root, UNGATED_RULES));
  const pending = readJson<Pending>(join(root, PENDING));
  const states = packStates(root);
  const checks: Check[] = [];
  const notGated: UngatedResult["notGated"] = [];
  const pendingOut: UngatedResult["pending"] = [];
  const ungatedBy = new Map<string, Set<string>>();

  for (const state of states) {
    const ungated: string[] = [];
    const pendingRules = new Map<string, number>();
    for (const id of state.shipped) {
      const treatment = shippedRuleTreatment(state, id, pending);
      if (treatment.kind === "frozen") ungated.push(id);
      else if (treatment.kind === "pending") pendingRules.set(treatment.toolPack, (pendingRules.get(treatment.toolPack) ?? 0) + 1);
    }
    ungatedBy.set(state.pack, new Set(ungated));
    for (const tool of state.tools.filter((name) => `${state.pack}/${name}` in pending)) {
      const toolPack = `${state.pack}/${tool}`;
      pendingOut.push({ toolPack, reason: pending[toolPack]!, rules: pendingRules.get(toolPack) ?? 0 });
    }
    if (ungated.length === 0 && !(state.pack in frozen)) continue;
    notGated.push({ pack: state.pack, rules: ungated.length });
    const listed = new Set(frozen[state.pack] ?? []);
    const added = ungated.filter((id) => !listed.has(id));
    checks.push({
      id: `not-gated:${state.pack}`,
      ok: added.length === 0,
      detail: added.length === 0
        ? `${ungated.length} shipped rule(s) whose upstream tool has no adapter, all on the frozen list`
        : `${added.length} new shipped rule(s) (${added.slice(0, 5).map((id) => `${id}${state.toolOf.has(id) ? ` of ${state.toolOf.get(id)}` : ""}`).join(", ")}) whose upstream tool is neither gated nor pending, so the rule cannot be gated: keep it in candidates/ until the tool has a gates.yaml`,
    });
  }
  for (const [pack, ids] of Object.entries(frozen).sort(([a], [b]) => byText(a, b))) {
    const ungated = ungatedBy.get(pack) ?? new Set<string>();
    const stale = ids.filter((id) => !ungated.has(id));
    checks.push({
      id: `frozen-list:${pack}`,
      ok: stale.length === 0,
      detail: stale.length === 0 ? `${ids.length} listed rule(s), all still shipped and not gated` : `${stale.length} listed rule(s) no longer shipped, or now gated or pending (${stale.slice(0, 5).join(", ")}): remove it from the list`,
    });
  }
  for (const toolPack of Object.keys(pending).sort(byText)) {
    const [pack, tool] = toolPack.split("/") as [string, string];
    const gated = existsSync(join(root, "packs", pack, "upstream", tool ?? "", GATES_MANIFEST));
    const known = states.some((state) => state.pack === pack && state.tools.includes(tool));
    checks.push({ id: `pending:${toolPack}`, ok: !gated && known, detail: gated ? "it has a gates.yaml now: remove it from pending.json" : known ? pending[toolPack]! : "no such tool in the pack's conformance.json: remove it from pending.json" });
  }

  if (options.base !== undefined) {
    for (const [path, current] of [[UNGATED_RULES, frozen], [PENDING, pending]] as const) {
      const atBase = await readAtBase(root, options.base, path, options.runner);
      if (atBase.kind === "missing") {
        checks.push({ id: `shrinks:${path}`, ok: true, detail: `the base commit has no ${path}: the guard is skipped` });
        continue;
      }
      if (atBase.kind === "error") {
        checks.push({ id: `shrinks:${path}`, ok: false, detail: `fails closed: ${atBase.message}` });
        continue;
      }
      const value = atBase.value;
      if (value === null || typeof value !== "object" || Array.isArray(value)) {
        checks.push({ id: `shrinks:${path}`, ok: false, detail: `fails closed: ${path} at the base is not a JSON object` });
        continue;
      }
      const base = value as Record<string, unknown>;
      const added = path === UNGATED_RULES
        ? Object.entries(current as Frozen).flatMap(([pack, ids]) => {
            const before = new Set(Array.isArray(base[pack]) ? (base[pack] as string[]) : []);
            return ids.filter((id) => !before.has(id)).map((id) => `${pack}/${id}`);
          })
        : Object.keys(current).filter((toolPack) => !(toolPack in base));
      checks.push({ id: `shrinks:${path}`, ok: added.length === 0, detail: added.length === 0 ? `only shrinks against ${options.base.slice(0, 12)}` : `${added.length} entr(ies) the base lacks (${added.slice(0, 5).join(", ")}): the list may only shrink` });
    }
  }
  return { checks, notGated, pending: pendingOut };
}

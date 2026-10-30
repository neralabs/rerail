/**
 * The per-rule gate of every gated tool-pack (a `packs/<pack>/upstream/<tool>/gates.yaml`), judged on the pack's
 * conformance report: a rule ships when it loads, is compact and passes the conformance checks (`checks/conformance.ts`);
 * upstream-status, from gates.yaml's declared facts, only decides whether it ships on or off. Every rule is judged,
 * shipped or candidate: a shipped rule that fails fails the check, and a candidate that passes is ready to promote.
 */
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import type { Check } from "./checks/check.js";
import { compactGate } from "./checks/compact.js";
import { conformanceChecks, conformanceOf } from "./checks/conformance.js";
import type { Engine } from "./checks/engine.js";
import { loadAsBundled, loadsGate, type RuleDocument } from "./checks/loads.js";
import { defaultOn, upstreamStatusChecks } from "./checks/upstream-status.js";
import { lintedPaths, lintedTypes, readGatesManifest } from "./manifest.js";
import { compareDeterministicStrings } from "../../src/engine/index.js";
import { documentPath, type LoadedPack } from "./pack.js";
import type { T4Report } from "./report.js";

/** The gates a rule must pass to ship, in the order a verdict reads them. */
export const BLOCKING_GATES = [
  { id: "loads", title: "Compiles and loads on this engine" },
  { id: "compact", title: "Compact (no written-out expansion)" },
  { id: "conformance", title: "Passes the conformance checks" },
] as const;
export type BlockingGateId = (typeof BLOCKING_GATES)[number]["id"];

/** Where a document sits: `rules/` ships and blocks; `candidates/**` is reported only. */
export type Location = "shipped" | "candidate";

export interface RuleVerdict {
  id: string;
  /** Repository-relative path of the document. */
  file: string;
  location: Location;
  /** The upstream rule the document ports, as conformance.json maps it. */
  upstreamRule: string;
  /** The rule passes every blocking gate and its tool-pack's checks hold. */
  pass: boolean;
  /** The first blocking gate the rule failed, or `tool-pack` when only a failing tool-pack check blocks it. */
  stoppedAt: BlockingGateId | "tool-pack" | null;
  /** The first failing check of that gate, and why. */
  failure: { check: string; why: string } | null;
  /** upstream-status's call: on when upstream has it, does not deprecate it and turns it on. */
  defaultOn: boolean;
  defaultOnReason: string;
  /** pack.json's `on` for a shipped rule (absent means on); null for a candidate. */
  on: boolean | null;
  gates: { "upstream-status": Check[] } & Record<BlockingGateId, Check[]>;
}

export interface ToolPackVerdict {
  pack: string;
  tool: string;
  toolPack: string;
  /** Tool-pack checks: a failing one fails every rule of the tool-pack. */
  checks: Check[];
  /** Documents of the pack with no conformance.json entry (reported once, by the pack's first gated tool-pack). */
  unmapped: { file: string; location: Location }[];
  rules: RuleVerdict[];
}

function walk(root: string, directory: string): string[] {
  const absolute = join(root, directory);
  if (!existsSync(absolute)) return [];
  return readdirSync(absolute, { withFileTypes: true }).flatMap((entry) => {
    const path = `${directory}/${entry.name}`;
    return entry.isDirectory() ? walk(root, path) : [path];
  });
}

/**
 * The pack's rule documents in the working tree: `rules/*.json` (shipped) and every JSON under `candidates/` except
 * hand-labelled probes (`candidates/probes/**`) and conformance manifests (candidate).
 */
export function packDocuments(root: string, pack: string): { path: string; location: Location }[] {
  const shipped = walk(root, `packs/${pack}/rules`).filter((path) => path.endsWith(".json") && path.split("/").length === 4);
  const candidates = walk(root, `packs/${pack}/candidates`)
    .filter((path) => path.endsWith(".json") && !path.startsWith(`packs/${pack}/candidates/probes/`) && !path.endsWith("/conformance.json"));
  return [
    ...shipped.map((path) => ({ path, location: "shipped" as const })),
    ...candidates.map((path) => ({ path, location: "candidate" as const })),
  ].sort((a, b) => compareDeterministicStrings(a.path, b.path));
}

function readDocument(root: string, path: string): RuleDocument {
  const buffer = readFileSync(join(root, path));
  return { file: path.slice(path.lastIndexOf("/") + 1), path, text: buffer.toString("utf8"), bytes: buffer.length };
}

function packOn(root: string, pack: string): Map<string, boolean> {
  const manifest = JSON.parse(readFileSync(join(root, "packs", pack, "pack.json"), "utf8")) as { rules?: { document: string; on?: boolean }[] };
  return new Map((manifest.rules ?? []).map((rule) => [`packs/${pack}/${rule.document}`, rule.on !== false]));
}

const failing = (checks: readonly Check[]) => checks.find((check) => !check.ok);

/**
 * Judge every rule of one gated tool-pack on the pack's conformance report. `unmapped` lists the pack's documents no
 * conformance.json entry maps; the caller reports them once per pack.
 */
export function judgeToolPack(root: string, engine: Engine, loaded: LoadedPack, report: T4Report, tool: string): ToolPackVerdict {
  const pack = loaded.pack.pack;
  const manifest = readGatesManifest(root, pack, tool);
  const conformance = conformanceOf(loaded, report);
  const entries = new Map(loaded.pack.rules.map((rule) => [`packs/${pack}/${documentPath(rule.policy)}`, rule]));
  const documents = packDocuments(root, pack);
  const mine = documents.filter((document) => entries.get(document.path)?.upstream.tool === tool);
  const on = packOn(root, pack);
  const scratch = mkdtempSync(join(tmpdir(), "rerail-gates-"));
  try {
    // bundled-load: rerail loads every document in the pack's rules/ together, whatever its upstream tool, so a
    // conflict between them (a duplicate id) fails here, where each rule's own loads check reads one document.
    const shipped = documents.filter((document) => document.location === "shipped").map((document) => readDocument(root, document.path));
    const checks: Check[] = [];
    try {
      const loadedIds = loadAsBundled(engine, scratch, pack, shipped);
      const ok = loadedIds.ids.length === shipped.length;
      checks.push({ id: "bundled-load", ok, detail: ok ? `rerail's bundled-rule loader loads all ${shipped.length} shipped document(s) of the pack together` : `the loader returned ${loadedIds.ids.length} rule(s) for ${shipped.length} shipped document(s)` });
    } catch (error) {
      checks.push({ id: "bundled-load", ok: false, detail: `the pack's shipped rules do not load together through rerail's bundled-rule loader: ${error instanceof Error ? error.message : String(error)}` });
    }
    const pinned = loaded.pack.tools.find((entry) => entry.name === tool)?.version;
    checks.push({
      id: "declared-pins",
      ok: pinned === manifest.upstream.version,
      detail: pinned === manifest.upstream.version
        ? `conformance.json pins ${tool} ${pinned}, as gates.yaml's upstream.version does`
        : `conformance.json pins ${tool} ${pinned ?? "(not listed)"}, but gates.yaml's upstream.version is ${manifest.upstream.version}`,
    });
    // A port's own file types (conformance.json's `lints`) narrow the tool-pack's; one beyond them names a type the
    // tool does not lint, and that port's in-scope check would ask for nothing upstream compares.
    const declared = lintedTypes(manifest);
    const stray = mine.flatMap(({ path }) => {
      const entry = entries.get(path)!;
      return (entry.lints ?? []).filter((type) => !declared.includes(type)).map((type) => `${entry.id} ${type}`);
    });
    checks.push({
      id: "declared-lints",
      ok: stray.length === 0,
      detail: stray.length === 0
        ? "every port's own file types (conformance.json's lints) are ones gates.yaml declares the tool lints"
        : `conformance.json gives ports file types gates.yaml's upstream.lints (narrowed by rerail.in_scope) does not declare: ${stray.join(", ")}`,
    });
    // An error naming another tool of the pack (its fixtures or samples) is that tool-pack's, not this one's.
    const runErrors = [...conformance.errors, ...(conformance.toolErrors.get(tool) ?? [])];
    checks.push({ id: "conformance-runs", ok: runErrors.length === 0, detail: runErrors.length === 0 ? "the pack's conformance check runs without errors" : runErrors.slice(0, 3).join("; ") });
    const blockedBy = failing(checks);

    const rules = mine.map(({ path, location }): RuleVerdict => {
      const entry = entries.get(path)!;
      const document = readDocument(root, path);
      const status = upstreamStatusChecks(entry.upstream.rule, manifest.rules[entry.upstream.rule], conformance.rows.get(entry.id));
      const loads = loadsGate(engine, document, pack, scratch, lintedPaths(manifest, entry.lints));
      const parsed = loads.resolved ?? loads.document;
      const compact = parsed === undefined ? [{ id: "not-run", ok: false, detail: "the document is not JSON" }] : compactGate(document.bytes, parsed).checks;
      const gates = { "upstream-status": status, loads: loads.checks, compact, conformance: conformanceChecks(conformance, entry.id) };
      const stopped = BLOCKING_GATES.find((gate) => failing(gates[gate.id]) !== undefined);
      const first = stopped === undefined ? undefined : failing(gates[stopped.id]);
      const start = defaultOn(status);
      const blocked = stopped === undefined && blockedBy !== undefined;
      return {
        id: entry.id,
        file: path,
        location,
        upstreamRule: entry.upstream.rule,
        pass: stopped === undefined && !blocked,
        stoppedAt: stopped?.id ?? (blocked ? "tool-pack" : null),
        failure: first !== undefined ? { check: first.id, why: first.detail } : blocked ? { check: blockedBy.id, why: blockedBy.detail } : null,
        defaultOn: start.on,
        defaultOnReason: start.reason,
        on: location === "shipped" ? (on.get(path) ?? true) : null,
        gates,
      };
    });
    const unmapped = documents.filter((document) => !entries.has(document.path)).map(({ path, location }) => ({ file: path, location }));
    return { pack, tool, toolPack: `${pack}/${tool}`, checks, unmapped, rules };
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}

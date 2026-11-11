/**
 * The per-rule gate of every gated tool-pack (a `packs/<pack>/upstream/<tool>/gates.yaml`), judged on the pack's
 * conformance report: a rule ships when it loads, is compact and passes the conformance checks (`checks/conformance.ts`);
 * upstream-status, from gates.yaml's declared facts, only decides whether it ships on or off. Every rule is judged,
 * shipped or candidate: a shipped rule that fails fails the check, and a candidate that passes is ready to promote.
 * A pack's own rules (conformance.json `own`) have no upstream tool, so they pass `loads` and `compact` alone, judged
 * once per pack (`judgeOwnRules`), not by each of its tool-packs.
 */
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { z } from "zod";

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

/** The blocking gates an own rule passes: the ones that read the document alone, with no upstream to compare. */
const OWN_GATES = BLOCKING_GATES.filter((gate): gate is Extract<(typeof BLOCKING_GATES)[number], { id: OwnGateId }> => gate.id === "loads" || gate.id === "compact");
export type OwnGateId = Extract<BlockingGateId, "loads" | "compact">;

/** Where a document sits: `rules/` ships and blocks; `candidates/**` is reported only. */
export type Location = "shipped" | "candidate";

interface Verdict {
  id: string;
  /** Repository-relative path of the document. */
  file: string;
  location: Location;
  /** The rule passes every blocking gate and its tool-pack's checks hold. */
  pass: boolean;
  /** The first blocking gate the rule failed, or `tool-pack` when only a failing tool-pack check blocks it. */
  stoppedAt: BlockingGateId | "tool-pack" | null;
  /** The first failing check of that gate, and why. */
  failure: { check: string; why: string } | null;
  /** pack.json's `on` for a shipped rule (absent means on); null for a candidate. */
  on: boolean | null;
}

/** A port's verdict, on every gate. */
export interface PortVerdict extends Verdict {
  /** The upstream rule the document ports, as conformance.json maps it. */
  upstreamRule: string;
  /** upstream-status's call: on when upstream has it, does not deprecate it and turns it on. */
  defaultOn: boolean;
  defaultOnReason: string;
  gates: { "upstream-status": Check[] } & Record<BlockingGateId, Check[]>;
}

/** An own rule's verdict (conformance.json `own`): no upstream rule, so `loads` and `compact` alone. */
export interface OwnVerdict extends Verdict {
  upstreamRule: null;
  stoppedAt: OwnGateId | null;
  gates: Record<OwnGateId, Check[]>;
}

export type RuleVerdict = PortVerdict | OwnVerdict;

export interface ToolPackVerdict {
  pack: string;
  /** The upstream tool; null for the pack's own rules (`judgeOwnRules`), which no tool gates. */
  tool: string | null;
  /** `<pack>/<tool>`, or `<pack>/own` for the pack's own rules. */
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

const examplePathSchema = z.object({ path: z.string().optional() }).optional();

/** A rule pack.json ships, as the gates read it: its document, its `on`, and its examples' paths. */
const packJsonSchema = z.object({
  rules: z.array(z.object({ id: z.string(), document: z.string(), on: z.boolean().optional(), examples: z.object({ caught: examplePathSchema, passes: examplePathSchema }).optional() })).default([]),
});
type PackJsonRule = z.infer<typeof packJsonSchema>["rules"][number];

function packJsonRules(root: string, pack: string): PackJsonRule[] {
  return packJsonSchema.parse(JSON.parse(readFileSync(join(root, "packs", pack, "pack.json"), "utf8"))).rules;
}

/** A document's repository-relative path from its pack-relative one (pack.json's `document`, a port's `policy`). */
const packPath = (pack: string, document: string): string => `packs/${pack}/${documentPath(document)}`;

/** The pack's own rules (conformance.json `own`) pack.json ships, by their documents' repository-relative paths. */
function ownRules(root: string, loaded: LoadedPack): Map<string, PackJsonRule> {
  const own = new Set(loaded.pack.own);
  return new Map(packJsonRules(root, loaded.pack.pack).filter((rule) => own.has(rule.id)).map((rule) => [packPath(loaded.pack.pack, rule.document), rule]));
}

const failing = (checks: readonly Check[]) => checks.find((check) => !check.ok);

/**
 * What judging a tool-pack reads of the pack's documents alone, before any report: whether its shipped rules load
 * together (`bundled-load`), and each of its rules' `loads` and `compact` gates. It compiles every document, most of a
 * judgement's work, so it can run while the report is being made.
 */
export interface DocumentGates {
  bundledLoad: Check;
  rules: Record<string, { loads: Check[]; compact: Check[] }>;
}

/**
 * The pack's documents of `tool`, by repository-relative path, with the conformance.json entries mapping them, and the
 * pack's own rules (`own`): they have no upstream, so they are neither the tool's nor unmapped.
 */
function toolDocuments(root: string, loaded: LoadedPack, tool: string) {
  const pack = loaded.pack.pack;
  const entries = new Map(loaded.pack.rules.map((rule) => [packPath(pack, rule.policy), rule]));
  const own = new Set(ownRules(root, loaded).keys());
  const documents = packDocuments(root, pack);
  return { entries, documents, own, mine: documents.filter((document) => entries.get(document.path)?.upstream.tool === tool) };
}

/** One document's `loads` and `compact` gates; `linted` and `outOfScope` as `loadsGate` takes them. */
function judgeDocument(engine: Engine, document: RuleDocument, pack: string, scratch: string, linted: readonly string[], outOfScope?: string): Record<OwnGateId, Check[]> {
  const loads = loadsGate(engine, document, pack, scratch, linted, outOfScope);
  const parsed = loads.resolved ?? loads.document;
  return { loads: loads.checks, compact: parsed === undefined ? [{ id: "not-run", ok: false, detail: "the document is not JSON" }] : compactGate(document.bytes, parsed).checks };
}

export function judgeDocuments(root: string, engine: Engine, loaded: LoadedPack, tool: string): DocumentGates {
  const pack = loaded.pack.pack;
  const manifest = readGatesManifest(root, pack, tool);
  const { entries, documents, mine } = toolDocuments(root, loaded, tool);
  const scratch = mkdtempSync(join(tmpdir(), "rerail-gates-"));
  try {
    // bundled-load: rerail loads every document in the pack's rules/ together, whatever its upstream tool, so a
    // conflict between them (a duplicate id) fails here, where each rule's own loads check reads one document.
    const shipped = documents.filter((document) => document.location === "shipped").map((document) => readDocument(root, document.path));
    let bundledLoad: Check;
    try {
      const loadedIds = loadAsBundled(engine, scratch, pack, shipped);
      const ok = loadedIds.ids.length === shipped.length;
      bundledLoad = { id: "bundled-load", ok, detail: ok ? `rerail's bundled-rule loader loads all ${shipped.length} shipped document(s) of the pack together` : `the loader returned ${loadedIds.ids.length} rule(s) for ${shipped.length} shipped document(s)` };
    } catch (error) {
      bundledLoad = { id: "bundled-load", ok: false, detail: `the pack's shipped rules do not load together through rerail's bundled-rule loader: ${error instanceof Error ? error.message : String(error)}` };
    }
    const rules: DocumentGates["rules"] = {};
    for (const { path } of mine) rules[path] = judgeDocument(engine, readDocument(root, path), pack, scratch, lintedPaths(manifest, entries.get(path)!.lints));
    return { bundledLoad, rules };
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}

/**
 * Judge every rule of one gated tool-pack on the pack's conformance report, its documents' gates (`judgeDocuments`)
 * made already or here. `unmapped` lists the pack's documents no conformance.json entry maps; the caller reports them
 * once per pack.
 */
export function judgeToolPack(
  root: string,
  engine: Engine,
  loaded: LoadedPack,
  report: T4Report,
  tool: string,
  documentGates: DocumentGates = judgeDocuments(root, engine, loaded, tool),
): ToolPackVerdict {
  const pack = loaded.pack.pack;
  const manifest = readGatesManifest(root, pack, tool);
  const conformance = conformanceOf(loaded, report);
  const { entries, documents, own, mine } = toolDocuments(root, loaded, tool);
  const on = new Map(packJsonRules(root, pack).map((rule) => [packPath(pack, rule.document), rule.on !== false]));
  const checks: Check[] = [documentGates.bundledLoad];
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

  const rules = mine.map(({ path, location }): PortVerdict => {
    const entry = entries.get(path)!;
    const status = upstreamStatusChecks(entry.upstream.rule, manifest.rules[entry.upstream.rule], conformance.rows.get(entry.id));
    const { loads, compact } = documentGates.rules[path]!;
    const gates = { "upstream-status": status, loads, compact, conformance: conformanceChecks(conformance, entry.id) };
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
  const unmapped = documents.filter((document) => !entries.has(document.path) && !own.has(document.path)).map(({ path, location }) => ({ file: path, location }));
  return { pack, tool, toolPack: `${pack}/${tool}`, checks, unmapped, rules };
}

/**
 * Judge the pack's own rules (conformance.json `own`) on `loads` and `compact`, the gates that read a document alone:
 * `upstream-status` and `conformance` compare a port with its upstream tool, which an own rule has none of. An own rule
 * has no tool-pack whose file types `loads`'s in-scope check could ask for, so it asks the rule to apply to its
 * pack.json examples' paths: the files its proof says it catches and passes, as fixtures are a port's. The pack's file
 * types would ask more than an own rule claims (one written for JSX alone), and the rule's scope against itself would
 * ask nothing. Each own rule is judged here once, whatever the number of the pack's tool-packs. An own id pack.json
 * does not ship, or whose document is missing, has nothing to judge: pack validation's checks 1 and 2 fail it.
 */
export function judgeOwnRules(root: string, engine: Engine, loaded: LoadedPack): ToolPackVerdict {
  const pack = loaded.pack.pack;
  const own = ownRules(root, loaded);
  const scratch = mkdtempSync(join(tmpdir(), "rerail-gates-"));
  try {
    const rules = packDocuments(root, pack).filter((document) => own.has(document.path)).map(({ path, location }): OwnVerdict => {
      const { id, on, examples } = own.get(path)!;
      const paths = [...new Set([examples?.caught?.path, examples?.passes?.path].filter((example) => example !== undefined))];
      const gates = judgeDocument(engine, readDocument(root, path), pack, scratch, paths, "pack.json gives the rule an example there: widen the rule's scope, or move the example into it");
      const stopped = OWN_GATES.find((gate) => failing(gates[gate.id]) !== undefined);
      const first = stopped === undefined ? undefined : failing(gates[stopped.id]);
      return {
        id,
        file: path,
        location,
        upstreamRule: null,
        pass: stopped === undefined,
        stoppedAt: stopped?.id ?? null,
        failure: first === undefined ? null : { check: first.id, why: first.detail },
        on: location === "shipped" ? on !== false : null,
        gates,
      };
    });
    return { pack, tool: null, toolPack: `${pack}/own`, checks: [], unmapped: [], rules };
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}

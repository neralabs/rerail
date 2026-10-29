import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import type { Check } from "./check.js";
import type { CompiledPolicy, Engine } from "./engine.js";

/** One rule document of a pack, read as raw bytes. */
export interface RuleDocument {
  /** The file name, e.g. `no-debugger.json`. */
  file: string;
  /** Repository-relative path, under the pack's `rules/` or `candidates/`. */
  path: string;
  text: string;
  bytes: number;
}

export interface LoadsResult {
  checks: Check[];
  /** The engine's compiled form of the shipped bytes, when they compile. */
  compiled?: CompiledPolicy;
  /** The parsed document, when it is JSON. */
  document?: Record<string, unknown>;
  /** The document after rerail resolves its options: what the engine evaluates, when the schema accepts it. */
  resolved?: Record<string, unknown>;
}

function message(error: unknown): string {
  return (error instanceof Error ? error.message : String(error)).split("\n")[0]!;
}

/**
 * Whether a document is written for the current rule contract: `rerail.policy/v1`, `clauses`
 * whose findings are `violation`s with no severity, and one rule `mode`.
 */
export function currentContract(document: Record<string, unknown>): Check {
  const clauses = (Array.isArray(document.clauses) ? document.clauses : [])
    .filter((clause): clause is Record<string, unknown> => clause !== null && typeof clause === "object");
  const severity = (finding: unknown) => finding !== null && typeof finding === "object" && "severity" in finding;
  const departures = [
    ...(document.format === "rerail.policy/v1" ? [] : [`format ${JSON.stringify(document.format)}`]),
    ...("checks" in document ? ["checks instead of clauses"] : []),
    ...("mode" in document ? [] : ["no rule mode"]),
    ...(clauses.some((clause) => "finding" in clause) ? ["a finding instead of a violation"] : []),
    ...(clauses.some((clause) => severity(clause.violation)) ? ["a violation with a severity"] : []),
  ];
  return {
    id: "current-contract",
    ok: departures.length === 0,
    detail: departures.length === 0
      ? "written for the current rule contract (rerail.policy/v1 with clauses, each with a violation, and a rule mode)"
      : `not written for the current rule contract (rerail.policy/v1 with clauses, each with a violation, and a rule mode): ${departures.join(", ")}`,
  };
}

function hasLlmClause(document: Record<string, unknown>): boolean {
  const clauses = Array.isArray(document.clauses) ? document.clauses : [];
  return clauses.some((clause) => typeof clause === "object" && clause !== null && "llm" in clause);
}

/** Load documents the way rerail reads a pack it ships: `<pack>/rules/*.json`, through its own bundled-rule loader. */
export function loadAsBundled(engine: Engine, scratch: string, packId: string, rules: readonly RuleDocument[]): { ids: string[]; kinds: string[] } {
  const directory = mkdtempSync(join(scratch, "bundled-"));
  try {
    const packDirectory = join(directory, packId);
    mkdirSync(join(packDirectory, "rules"), { recursive: true });
    for (const rule of rules) writeFileSync(join(packDirectory, "rules", rule.file), rule.text);
    const loaded = engine.rerail.loadPackRuleDocuments([packDirectory]);
    return { ids: loaded.map((document) => document.rule.id), kinds: loaded.map((document) => document.kind) };
  } catch (error) {
    // The loader names a file it rejects by absolute path; this directory is new on every run, so the report names the
    // file from the pack (`<pack>/rules/<file>`) and stays byte-identical across runs.
    throw new Error((error instanceof Error ? error.message : String(error)).replaceAll(`${directory}/`, ""));
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

/**
 * loads: the shipped bytes parse, pass rerail's own schema, compile on this checkout's engine both as written and in
 * the stable form the rule catalog compiles, load through rerail's bundled-rule loader, and apply to an example path
 * of every file type upstream lints (`linted`: gates.yaml's `upstream.lints`, narrowed by `rerail.in_scope` and by the
 * port's own `lints` in conformance.json).
 */
export function loadsGate(engine: Engine, rule: RuleDocument, packId: string, scratch: string, linted: readonly string[]): LoadsResult {
  const checks: Check[] = [];
  let document: Record<string, unknown>;
  try {
    const duplicates = engine.rerail.findDuplicateJsonKeys(rule.text);
    if (duplicates.length > 0) throw new Error(`duplicate key \`${duplicates[0]!.key}\` at ${duplicates[0]!.pointer || "/"}`);
    const parsed: unknown = JSON.parse(rule.text);
    if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) throw new Error("the document is not a JSON object");
    document = parsed as Record<string, unknown>;
    checks.push({ id: "json", ok: true, detail: "valid JSON with no repeated keys" });
  } catch (error) {
    checks.push({ id: "json", ok: false, detail: message(error) });
    return { checks };
  }

  checks.push(currentContract(document));
  const schema = engine.rerail.codeRuleSchema.safeParse(document);
  checks.push(schema.success
    ? { id: "schema", ok: true, detail: "passes RERAIL's policy rule schema" }
    : { id: "schema", ok: false, detail: schema.error.issues.slice(0, 3).map((issue) => `/${issue.path.map(String).join("/")}: ${issue.message}`).join("; ") });
  checks.push({
    id: "deterministic",
    ok: !hasLlmClause(document),
    detail: hasLlmClause(document) ? "an llm clause needs a model at evaluation time" : "no llm clauses",
  });

  let compiled: CompiledPolicy | undefined;
  try {
    compiled = engine.compile(rule.text);
    const stable = schema.success ? engine.rerail.stableJsonText(schema.data) : undefined;
    if (stable !== undefined) engine.compile(stable);
    checks.push({ id: "engine-compile", ok: schema.success, detail: schema.success ? "compiles as written and as the catalog's stable JSON" : "compiles as written; the schema rejected it, so the catalog never would" });
  } catch (error) {
    checks.push({ id: "engine-compile", ok: false, detail: message(error) });
    return { checks, document, ...(schema.success ? { resolved: schema.data as Record<string, unknown> } : {}) };
  }

  const canonicalId = (JSON.parse(compiled.canonicalJson) as { id?: unknown }).id;
  checks.push({
    id: "identity",
    ok: typeof document.id === "string" && canonicalId === document.id,
    detail: `document id ${JSON.stringify(document.id)}, compiled id ${JSON.stringify(canonicalId)}`,
  });

  try {
    const loaded = loadAsBundled(engine, scratch, packId, [rule]);
    const ok = loaded.ids.length === 1 && loaded.ids[0] === document.id && loaded.kinds[0] === "code";
    checks.push({ id: "bundled-load", ok, detail: ok ? "rerail's bundled-rule loader loads it from a pack's rules/ as a code rule" : `the loader returned ${JSON.stringify(loaded.ids)} as ${JSON.stringify(loaded.kinds)}` });
  } catch (error) {
    checks.push({ id: "bundled-load", ok: false, detail: message(error) });
  }

  const outOfScope = linted.filter((path) => !compiled.inScope(path));
  checks.push({
    id: "in-scope",
    ok: outOfScope.length === 0,
    detail: outOfScope.length === 0
      ? `applies to ${linted.join(", ")}`
      : `does not apply to ${outOfScope.join(", ")}: gates.yaml declares upstream lints that file type (upstream.lints); widen the rule's scope, or narrow the tool-pack with rerail.in_scope or the port with its lints in conformance.json`,
  });
  return { checks, compiled, document, ...(schema.success ? { resolved: schema.data as Record<string, unknown> } : {}) };
}

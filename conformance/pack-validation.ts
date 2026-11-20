/**
 * The pack validation contract (docs/pack-format.md, "The validation contract"): checks 1 to 8 over one
 * `packs/<id>/` directory. Test support, not runtime: test/engine/catalog/packs.test.ts calls it for every pack, and it reuses the
 * conformance runner for ported rules, so it lives beside that runner and stays out of the published core.
 */
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { isDeepStrictEqual } from "node:util";
import { basename, dirname, extname, join, matchesGlob, posix, relative, resolve, sep } from "node:path";
import semver from "semver";
import { z } from "zod";
import {
  languageFor,
  languageNotSupportedOf,
  codeRuleSchema,
  parseJsonWithoutDuplicateKeys,
  loadNativeBinding,
  type NativeCompiledCodeRuleDsl,
  rulePackFileSchema,
  rulePackPackageName,
  rulePackPackageSchema,
  RULE_PACK_RULE_ID,
  type RulePackFile,
  compareDeterministicStrings,
  LANGUAGE_REGISTRY,
  RULE_PACK_LIST,
  ConversationRuleRegistry,
  type ConversationRule,
  engineRegexIssues,
  type RegexIssue,
  parseConversationRule,
  RERAIL_DOCUMENT_EXTENSION,
  type RerailDescription,
  type RrDescription,
} from "../src/engine/index.js";
import { resolveModelPath } from "../src/runtime/index.js";
import { PACK_LIST_MODULE_PATH, PACKS_DIRECTORY, renderPackListModule } from "../scripts/generate/packs/pack-list.js";
import { CONFORMANCE_FILE, documentPath, loadPack, packSchema, RERAIL_ROOT, type LoadedPack } from "./languages/pack.js";
import { reportFailures, runPack } from "./languages/runner.js";
import { decidingPattern, inToolSubdirectory, licenceAllowed, licenceAlternatives, packFixtureAbsence, pathGlob, PUBLISH_DIRECTORY, readUpstreamSources } from "../scripts/s3/pack-fixtures.mjs";
import { isPrivatePack } from "../scripts/release/packs.mjs";
import { admitsRefusingCore, HELD_OFF_SINCE, heldOffRules, LANGUAGE_SINCE, raisedCoreRange, RERAIL_DOCUMENTS_SINCE, rerailRules, shippedLanguages } from "../scripts/release/core.mjs";
import { describeListed, includedPackFiles, rerailExamplePairFailures, rerailListed, rerailProofFailures, rerailTestFailures } from "./rerail-pack-rules.js";
import { capabilitiesJson } from "./engine.js";
import { BIOME_SAME_SOURCES } from "./biome-same-sources.generated.js";

export const PACK_CHECKS = [
  { number: 1, name: "rules" },
  { number: 2, name: "proof" },
  { number: 3, name: "examples" },
  { number: 4, name: "ports" },
  { number: 5, name: "package" },
  { number: 6, name: "licences" },
  { number: 7, name: "manifest" },
  { number: 8, name: "text" },
] as const;
export type PackCheckName = (typeof PACK_CHECKS)[number]["name"];

/** `not-supported-yet`: the rule compiles to `language-not-supported`; check 1 passes it and checks 3 and 4 skip it. */
export type PackRuleStatus = "ok" | "failed" | "not-supported-yet";

export interface PackRuleResult {
  /** The rule id, as pack.json lists it. */
  id: string;
  document: string;
  status: PackRuleStatus;
  /** For `failed`, the first failure; for `not-supported-yet`, the engine's reason. */
  reason?: string;
}

export interface PackValidation {
  id: string;
  directory: string;
  /** Each check's failures, in a stable order; an empty list passes. */
  checks: Record<PackCheckName, string[]>;
  rules: PackRuleResult[];
  pass: boolean;
}

export interface ValidatePackOptions {
  /**
   * Whether core's generated pack list (`src/engine/catalog/pack-list.generated.ts`) must be current and agree with
   * the pack: name it when published, leave it out when its package.json says `"private": true`. Default: when
   * `directory` is one of the repository's `packs/` directories; a fixture pack elsewhere is published nowhere.
   */
  listed?: boolean;
  /**
   * Whether check 4 scores the ported rules on the tools' fixtures. Default true. Off, check 4 keeps the rest (rules
   * that do not compile, stale exemptions, uncovered capabilities, suppression comments) and leaves the scoring to the
   * conformance check (`npm run test:conformance`), which scores every port on the same fixtures through the same
   * runner, and fails on every failure the scoring here reports: CI's unit tests turn it off, so the fixtures are
   * scored once, in the conformance job.
   */
  fixtures?: boolean;
  /**
   * The core version check 5 holds the pack's core range to (`heldOff`, and a language core first reads after 0.1.0).
   * Default: the repository's package.json version.
   */
  coreVersion?: string;
}

/** Validation material: kept in the repository, never published. */
const VALIDATION_MATERIAL = [CONFORMANCE_FILE, "divergences.json", "upstream", "candidates"] as const;
/** Rerail's own licence for its material in a pack. */
const RERAIL_LICENSE = "Apache-2.0";
/** A row of a tool's own UPSTREAM.md, which lists Rerail's files: its first cell names a pack-relative path. */
const UPSTREAM_ROW = /^\|\s*`([^`]+)`/u;

let allowlist: ReadonlySet<string> | undefined;
/** The repository's licence allowlist, the one `scripts/check/check-licenses.mjs` enforces. */
function allowedLicenses(): ReadonlySet<string> {
  allowlist ??= new Set(z.array(z.string()).parse(JSON.parse(readFileSync(join(RERAIL_ROOT, "scripts", "check", "licence-allowlist.json"), "utf8"))));
  return allowlist;
}

const evaluationSchema = z.object({ violations: z.array(z.object({ code: z.string() })).default([]), failures: z.array(z.unknown()).default([]) });

/** A code rule's violation codes and failures on one file, evaluated as a one-file repository snapshot. */
export function evaluateExample(compiled: NativeCompiledCodeRuleDsl, path: string, source: string): { codes: string[]; failures: unknown[] } {
  const language = languageFor(path) ?? "text";
  const input = JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, facts: [], files: [{ language, path, source }] });
  const result = evaluationSchema.parse(JSON.parse(compiled.evaluate(input, "report")));
  return { codes: result.violations.map(violation => violation.code), failures: result.failures };
}

const documentSchema = z.object({
  id: z.string(),
  clauses: z.array(z.looseObject({ violation: z.object({ code: z.string() }).optional() })),
});

type Compiled =
  | { kind: "compiled"; compiled: NativeCompiledCodeRuleDsl; codes: ReadonlySet<string> }
  | { kind: "conversation"; rule: ConversationRule }
  | { kind: "rerail"; described: RerailDescription }
  | { kind: "not-supported-yet"; reason: string }
  | { kind: "failed"; message: string };

/**
 * Check 3 on one rule's example pair: each path is in the rule's scope and the engine evaluates it, the caught example
 * produces one of the rule's violation codes and the passes example none.
 */
function exampleFailures(id: string, result: Extract<Compiled, { kind: "compiled" }>, examples: Record<"caught" | "passes", { path?: string | undefined; text: string }>): string[] {
  const failures: string[] = [];
  for (const side of ["caught", "passes"] as const) {
    const example = examples[side];
    if (example.path === undefined) {
      failures.push(`${id}: the ${side} example has no path`);
      continue;
    }
    if (!result.compiled.inScope(example.path)) {
      failures.push(`${id}: the ${side} example's path ${example.path} is outside the rule's scope`);
      continue;
    }
    const evaluation = evaluateExample(result.compiled, example.path, example.text);
    if (evaluation.failures.length > 0) failures.push(`${id}: the ${side} example (${example.path}) fails in the engine: ${JSON.stringify(evaluation.failures)}`);
    const hits = evaluation.codes.filter(code => result.codes.has(code));
    if (side === "caught" && hits.length === 0) failures.push(`${id}: the caught example (${example.path}) produces no ${[...result.codes].join("/")} violation`);
    if (side === "passes" && hits.length > 0) failures.push(`${id}: the passes example (${example.path}) produces ${hits.join(", ")}`);
  }
  return failures;
}

/**
 * Check 3 on a conversation rule's example pair: the caught example fires the rule, the passes example does not. An
 * example's text is what the rule samples: for a tool-call rule, the command the coding agent runs (`tool:*` is tried
 * as Bash); otherwise the text itself.
 */
function conversationExampleFailures(
  rule: ConversationRule,
  examples: Record<"caught" | "passes", { path?: string | undefined; text: string }>,
  registry: () => ConversationRuleRegistry,
): string[] {
  let engine: ConversationRuleRegistry;
  try {
    engine = registry();
    engine.replace([rule]);
  } catch (error) {
    return [`${rule.id}: the conversation engine cannot load the rule: ${message(error)}`];
  }
  const failures: string[] = [];
  const source = rule.sample.source === "tool:*" ? "tool:Bash" : rule.sample.source;
  for (const side of ["caught", "passes"] as const) {
    const { text } = examples[side];
    const input = JSON.stringify({ command: text });
    const sample = rule.sample.boundary === "tool-call" ? { source, boundary: rule.sample.boundary, text: input, input } : { source, boundary: rule.sample.boundary, text };
    const fired = engine.match(sample, { respectRepeat: false }).some(match => match.ruleId === rule.id);
    if (side === "caught" && !fired) failures.push(`${rule.id}: the caught example does not fire the rule: ${text}`);
    if (side === "passes" && fired) failures.push(`${rule.id}: the passes example fires the rule: ${text}`);
  }
  return failures;
}

const message = (error: unknown): string => (error instanceof Error ? error.message : String(error));

const headerSchema = z.object({ format: z.string().optional(), id: z.string().optional() });

/** The `format` and `id` a document's text carries, where it is a JSON object with them. */
function headerOf(text: string): z.infer<typeof headerSchema> {
  try {
    const parsed = headerSchema.safeParse(JSON.parse(text));
    return parsed.success ? parsed.data : {};
  } catch {
    return {};
  }
}

/** The `id` a code-rule document's text carries, if it is JSON with one. */
const documentIdOf = (text: string): string | undefined => headerOf(text).id;

const CODE_RULE_FORMAT = "rerail.policy/v1";
/** A rule document in some format: `rerail.policy/v1`, or an older `<name>.policy/v<n>`. */
const POLICY_FORMAT = /\.policy\/v\d+$/u;

const inRulesDirectory = (file: string): boolean => posix.basename(posix.dirname(file)) === "rules";

/**
 * The rule documents among `candidates`, the pack-relative files under `candidates/` (ports under evaluation and
 * probes): every JSON file in a `rules/` directory there, and any other JSON file there whose `format` names a rule
 * document (which check 1 fails, since a candidate is in a `rules/` directory).
 */
function candidateDocuments(directory: string, candidates: readonly string[]): string[] {
  return candidates.filter(file => file.endsWith(".json") && (inRulesDirectory(file) || POLICY_FORMAT.test(headerOf(readFileSync(join(directory, file), "utf8")).format ?? "")));
}

/**
 * The id rule (docs/pack-format.md, check 1): one id style in every pack. A rule's id is lowercase letters, digits and
 * dashes and names its document, shipped (`rules/<id>.json`) or candidate (`candidates/…/rules/<id>.json`); a candidate
 * is a `rerail.policy/v1` document or a conversation rule, like a shipped rule; and a port in any `conformance.json` of the pack has its
 * document's id. So the id users turn a rule on and off by is the one its file, its listing and its scores carry, and a
 * candidate keeps it when it ships.
 */
function idFailures(directory: string): string[] {
  const failures: string[] = [];
  const candidates = filesUnder(directory, join(directory, "candidates"));
  for (const file of candidateDocuments(directory, candidates)) {
    const text = readFileSync(join(directory, file), "utf8");
    const { format, id } = headerOf(text);
    if (format !== CODE_RULE_FORMAT && !isConversationDocument(text))
      failures.push(`${file}: format is ${format ?? "missing"}; a rule document is ${CODE_RULE_FORMAT} or a conversation rule`);
    if (!inRulesDirectory(file))
      failures.push(`${file}: a candidate is in a rules/ directory; move it to ${posix.join(posix.dirname(file), "rules", `${id ?? posix.basename(file, ".json")}.json`)}`);
    if (id === undefined) failures.push(`${file}: has no id`);
    else if (!RULE_PACK_RULE_ID.test(id)) failures.push(`${file}: id ${id} is not lowercase letters, digits and dashes`);
    else if (posix.basename(file, ".json") !== id) failures.push(`${file}: has id ${id}; name it ${posix.join(posix.dirname(file), `${id}.json`)}`);
  }
  // The root manifest is the conformance runner's pack: check 4 loads it, and reports one that does not parse or a port
  // whose policy is missing. Nothing else reads the manifests under candidates/ (probes), so check 1 reports those.
  const manifests = [CONFORMANCE_FILE, ...candidates.filter(file => posix.basename(file) === CONFORMANCE_FILE)];
  for (const manifest of manifests.filter(file => existsSync(join(directory, file)))) {
    const nested = manifest !== CONFORMANCE_FILE;
    let parsed: z.infer<typeof packSchema>;
    try {
      const result = packSchema.safeParse(JSON.parse(readFileSync(join(directory, manifest), "utf8")));
      if (!result.success) {
        if (nested) failures.push(`${manifest}: ${z.prettifyError(result.error)}`);
        continue;
      }
      parsed = result.data;
    } catch (error) {
      if (nested) failures.push(`${manifest}: is not JSON: ${message(error)}`);
      continue;
    }
    for (const port of parsed.rules) {
      const policy = posix.join(posix.dirname(manifest), port.policy);
      if (!existsSync(join(directory, policy))) {
        if (nested) failures.push(`${manifest}: port ${port.id} scores ${policy}, which does not exist`);
        continue;
      }
      const id = documentIdOf(readFileSync(join(directory, policy), "utf8"));
      if (id !== undefined && id !== port.id) failures.push(`${manifest}: port ${port.id} scores ${policy}, whose id is ${id}`);
    }
  }
  return failures;
}

/**
 * A code-rule document compiled as the catalog would take it: valid `rerail.policy/v1`, then compiled by the engine
 * against core's capabilities. `language-not-supported` is not a failure: the rule is stored inactive until core reads
 * its language.
 */
function compileCodeRule(text: string): Compiled {
  let document: unknown;
  try {
    document = JSON.parse(text);
  } catch (error) {
    return { kind: "failed", message: `is not JSON: ${message(error)}` };
  }
  const valid = codeRuleSchema.safeParse(document);
  if (!valid.success) return { kind: "failed", message: `is not a valid code rule (rerail.policy/v1): ${z.prettifyError(valid.error)}` };
  try {
    const compiled = loadNativeBinding().CodeRuleDslRuntime.compile(text, "json", capabilitiesJson());
    const codes = new Set(documentSchema.parse(document).clauses.flatMap(clause => (clause.violation === undefined ? [] : [clause.violation.code])));
    return { kind: "compiled", compiled, codes };
  } catch (error) {
    const notSupported = languageNotSupportedOf(message(error));
    return notSupported === undefined ? { kind: "failed", message: `does not compile: ${message(error)}` } : { kind: "not-supported-yet", reason: notSupported.reason };
  }
}

/** Whether a document's text is shaped as a conversation rule: a JSON object with a `match` and no `format`. */
function isConversationDocument(text: string): boolean {
  try {
    const document: unknown = JSON.parse(text);
    return typeof document === "object" && document !== null && !Array.isArray(document) && !("format" in document) && "match" in document;
  } catch {
    return false;
  }
}

/**
 * A rule document checked as the catalog would take it: a conversation rule parsed and its patterns accepted by the
 * engine, any other document compiled as a code rule.
 */
function compileRuleDocument(text: string): Compiled {
  if (!isConversationDocument(text)) return compileCodeRule(text);
  let rule: ConversationRule;
  try {
    rule = parseConversationRule(JSON.parse(text));
  } catch (error) {
    return { kind: "failed", message: `is not a valid conversation rule: ${message(error)}` };
  }
  let issues: RegexIssue[];
  try {
    issues = engineRegexIssues(rule);
  } catch (error) {
    return { kind: "failed", message: `cannot be checked: ${message(error)}` };
  }
  const [issue] = issues;
  if (issue !== undefined) return { kind: "failed", message: `has a pattern the engine refuses at ${issue.path.join(".")}: ${issue.message}` };
  return { kind: "conversation", rule };
}

/** The `violation.suppress` key and list that name an upstream rule (code-rule DSL reference, `suppress`). */
interface Suppression {
  key: string;
  list: string;
}
const SUPPRESSION_BY_TOOL: Record<string, Suppression> = {
  ruff: { key: "noqa", list: "codes" },
  bandit: { key: "nosec", list: "codes" },
  phpstan: { key: "phpstan", list: "identifiers" },
  psalm: { key: "psalm", list: "issues" },
  phpcs: { key: "phpcs", list: "sniffs" },
};

/**
 * The suppression a tool's own inline comments give its findings (`// eslint-disable-line <rule>`, `# noqa: <code>`,
 * `# nosec <id>`, `@phpstan-ignore <identifier>`, `@psalm-suppress <issue>`, `phpcs:ignore <sniff>`), which a port of
 * its rule must honour as the tool does. Other tools (Bladestan, tlint, blade-parser, lizard) read none Rerail models.
 */
function suppressionOf(tool: string): Suppression | undefined {
  if (tool === "eslint" || tool === "typescript-eslint" || tool.startsWith("eslint-plugin-")) return { key: "eslint", list: "rules" };
  return SUPPRESSION_BY_TOOL[tool];
}

const suppressingDocumentSchema = z.object({
  clauses: z.array(
    z.looseObject({
      id: z.string(),
      forbid: z.looseObject({ ast: z.looseObject({ language: z.string() }).optional() }).optional(),
      violation: z.looseObject({ suppress: z.record(z.string(), z.record(z.string(), z.unknown())).optional() }).optional(),
    }),
  ),
});

/** The languages whose registry entry honours Biome's `biome-ignore` comments (`suppressions`). */
const BIOME_LANGUAGES: ReadonlySet<string> = new Set(LANGUAGE_REGISTRY.filter(entry => (entry.suppressions as readonly string[]).includes("biome")).map(entry => entry.id));

/** The string items of a suppression's list (`suppress.eslint.rules`), or none. */
function listed(suppression: Record<string, unknown> | undefined, list: string): string[] {
  const names = suppression?.[list];
  return Array.isArray(names) ? names.filter((name): name is string => typeof name === "string") : [];
}

/**
 * In a port's document, the ids of the clauses whose violation does not name the upstream `rule` under `suppression`
 * (`unsuppressed`); once any violation carries `suppress.oxlint`, of those that carry only one of `suppress.eslint`
 * and `suppress.oxlint` (`oxlintApart`): Oxlint reads `eslint-disable` comments too, so a port of a rule Oxlint
 * implements honours both on the same violations; of those whose `suppress.eslint` names a rule Biome has a rule the
 * same as, without that Biome rule under `suppress.biome` (`biomeMissing`, with the missing rules), and of those that
 * carry `suppress.biome` without `suppress.eslint` (`biomeAlone`). A query of a language whose registry entry does not
 * honour Biome's comments (Vue's) needs no `suppress.biome`. `undefined` when the document cannot be read (check
 * 1 reports that). A PHP_CodeSniffer port names its sniff's message codes, `<sniff>.<code>`, as `phpcs:ignore` does.
 */
function suppressionGaps(
  text: string,
  suppression: Suppression,
  tool: string,
  rule: string,
): { unsuppressed: string[]; oxlintApart: string[]; biomeMissing: { clauses: string[]; rules: string[] }; biomeAlone: string[] } | undefined {
  let document: unknown;
  try {
    document = JSON.parse(text);
  } catch {
    return undefined;
  }
  const parsed = suppressingDocumentSchema.safeParse(document);
  if (!parsed.success) return undefined;
  const violations = parsed.data.clauses.flatMap(clause =>
    clause.violation === undefined ? [] : [{ id: clause.id, language: clause.forbid?.ast?.language, suppress: clause.violation.suppress ?? {} }],
  );
  const unsuppressed = violations
    .filter(({ suppress }) => {
      const names = suppress[suppression.key]?.[suppression.list];
      return !(Array.isArray(names) && names.some(name => name === rule || (tool === "phpcs" && typeof name === "string" && name.startsWith(`${rule}.`))));
    })
    .map(violation => violation.id);
  const eslintFamily = suppression.key === "eslint";
  const oxlint = eslintFamily && violations.some(({ suppress }) => suppress.oxlint !== undefined);
  const oxlintApart = oxlint ? violations.filter(({ suppress }) => (suppress.eslint === undefined) !== (suppress.oxlint === undefined)).map(violation => violation.id) : [];
  const biomeMissing = { clauses: [] as string[], rules: [] as string[] };
  // Only a query of a language that honours Biome's comments may carry suppress.biome (Vue's does not).
  for (const { id, language, suppress } of eslintFamily ? violations.filter(({ language }) => language === undefined || BIOME_LANGUAGES.has(language)) : []) {
    const carried = new Set(listed(suppress.biome, "rules"));
    const missing = listed(suppress.eslint, "rules").flatMap(name => BIOME_SAME_SOURCES[name] ?? []).filter(name => !carried.has(name));
    if (missing.length === 0) continue;
    biomeMissing.clauses.push(id);
    for (const name of missing) if (!biomeMissing.rules.includes(name)) biomeMissing.rules.push(name);
  }
  const biomeAlone = violations.filter(({ suppress }) => suppress.biome !== undefined && suppress.eslint === undefined).map(violation => violation.id);
  return { unsuppressed, oxlintApart, biomeMissing, biomeAlone };
}

/** The ids of a rule document's clauses whose violation sets `suppress`; none when the text is no such document (check 1 reports that). */
function suppressingClauses(text: string): string[] {
  try {
    const parsed = suppressingDocumentSchema.safeParse(JSON.parse(text));
    return parsed.success ? parsed.data.clauses.filter(clause => clause.violation?.suppress !== undefined).map(clause => clause.id) : [];
  } catch {
    return [];
  }
}

/** Every file under `directory`, as posix paths relative to `root`, sorted. */
function filesUnder(root: string, directory: string): string[] {
  if (!existsSync(directory)) return [];
  const files: string[] = [];
  const walk = (current: string): void => {
    for (const entry of readdirSync(current, { withFileTypes: true })) {
      const path = join(current, entry.name);
      if (entry.isDirectory()) walk(path);
      else if (entry.isFile()) files.push(relative(root, path).split(sep).join("/"));
    }
  };
  walk(directory);
  return files.sort(compareDeterministicStrings);
}

/**
 * Whether a `files` entry of package.json publishes `path`. npm publishes a matched directory whole, so the entry
 * covers `path` when it names or matches `path` or any folder above it (`rules/*` publishes `rules/group/a.json`), or
 * when it matches something under `path` (`upstream/**` publishes part of `upstream`).
 */
function publishes(entry: string, path: string): boolean {
  const pattern = entry.replace(/^\.\//u, "").replace(/\/+$/u, "");
  const segments = path.split("/");
  const self = segments.map((_, index) => segments.slice(0, index + 1).join("/")).some(prefix => prefix === pattern || matchesGlob(prefix, pattern));
  return self || matchesGlob(`${path}/x`, pattern);
}

/**
 * What is wrong with a packed pack: each entry of its tarball (a path relative to the package root, without npm's
 * `package/` prefix) must be `package.json` or something `files` publishes, and never validation material, whatever
 * `files` says. The publish gate runs it on the real `npm pack` tarball.
 */
export function packedFileFailures(files: readonly string[], entries: readonly string[]): string[] {
  const failures: string[] = [];
  for (const entry of [...entries].sort(compareDeterministicStrings)) {
    if (VALIDATION_MATERIAL.some(material => entry === material || entry.startsWith(`${material}/`))) failures.push(`${entry}: validation material, never published`);
    else if (entry !== "package.json" && !files.some(file => publishes(file, entry))) failures.push(`${entry}: package.json files does not publish it`);
  }
  return failures;
}

/**
 * The `### ` headings of a markdown file's `## Rule details` section, each with the backticked names it holds; none
 * when the file or the section is missing. A line in a fenced code block is not a heading.
 */
function ruleDetailHeadings(path: string): { heading: string; names: string[] }[] {
  if (!existsSync(path)) return [];
  const headings: { heading: string; names: string[] }[] = [];
  let section = false;
  let fenced = false;
  for (const line of readFileSync(path, "utf8").split("\n")) {
    if (line.startsWith("```")) fenced = !fenced;
    if (fenced) continue;
    if (line.startsWith("## ")) section = line === "## Rule details";
    else if (section && line.startsWith("### ")) headings.push({ heading: line.slice(4), names: [...line.matchAll(/`([^`]+)`/gu)].map(match => match[1]!) });
  }
  return headings;
}

/** The root package.json, for the repository every pack is published from and core's version. */
const rootPackage = (): { repository: { url: string }; version: string } =>
  z.object({ repository: z.object({ url: z.string() }), version: z.string() }).parse(JSON.parse(readFileSync(join(RERAIL_ROOT, "package.json"), "utf8")));

/** The repository every pack is published from, as the root package.json names it: npm's trusted publishing checks it. */
const repositoryUrl = (): string => rootPackage().repository.url;

/**
 * Why a pack's core range `range` admits a core that refuses its pack.json, or undefined. Once core is at
 * `HELD_OFF_SINCE` (`coreVersion`, the Release pull request's bump), a pack holding rules off (`heldOff`) must leave out
 * every older core, which refuses the field and would drop the whole pack as invalid; before then the range stays, so
 * this checkout's core still loads the pack. The Release pull request raises the range of a pack it releases
 * (scripts/release/core.mjs holds the releases and the rule, for both).
 */
export function heldOffRangeFailure(range: string, heldOff: readonly string[], coreVersion: string): string | undefined {
  if (heldOff.length === 0 || !admitsRefusingCore(range, HELD_OFF_SINCE, coreVersion)) return undefined;
  return `package.json: the core range ${range} admits Rerail older than ${HELD_OFF_SINCE}, which refuses heldOff (${heldOff.join(", ")}); raise it to ${raisedCoreRange(HELD_OFF_SINCE)}`;
}

/**
 * Why a pack's core range `range` admits a core that refuses one of its rule documents, or undefined. `languages` are
 * the ones its shipped documents name. As for `heldOff`: once core is at the release that first reads one of them
 * (`LANGUAGE_SINCE`), the pack must leave out every older core, which refuses the document and would drop the whole
 * pack as invalid; before then the range stays, so this checkout's core still loads the pack.
 */
export function languageRangeFailure(range: string, languages: readonly string[], coreVersion: string): string | undefined {
  const refused = languages.flatMap(language => {
    const since = LANGUAGE_SINCE.get(language);
    return since !== undefined && admitsRefusingCore(range, since, coreVersion) ? [{ language, since }] : [];
  });
  const since = refused.map(entry => entry.since).sort(semver.rcompare)[0];
  if (since === undefined) return undefined;
  return `package.json: the core range ${range} admits Rerail older than ${since}, which reads no ${refused.map(entry => entry.language).join(" or ")} rule; raise it to ${raisedCoreRange(since)}`;
}

/**
 * Why a pack's core range `range` admits a core that cannot read its rules written in `.rerail` (`rerail`, their ids),
 * or undefined: as for `heldOff`, once core is at `RERAIL_DOCUMENTS_SINCE`.
 */
export function rerailRangeFailure(range: string, rerail: readonly string[], coreVersion: string): string | undefined {
  if (rerail.length === 0 || !admitsRefusingCore(range, RERAIL_DOCUMENTS_SINCE, coreVersion)) return undefined;
  return `package.json: the core range ${range} admits Rerail older than ${RERAIL_DOCUMENTS_SINCE}, which reads no rule written in .rerail (${rerail.join(", ")}); raise it to ${raisedCoreRange(RERAIL_DOCUMENTS_SINCE)}`;
}

/** The SPDX ids an SPDX expression names. */
const spdxIds = (expression: string): string[] =>
  expression
    .split(/\s+(?:AND|OR|WITH)\s+|[()]/u)
    .map(part => part.trim())
    .filter(part => part.length > 0);

/**
 * What is wrong with a package's `license` against the licences `recorded` (per project, each licenses/<project>/UPSTREAM.md's
 * SPDX expression), or undefined: some way to satisfy the package's licence must name exactly Apache-2.0 plus one way to
 * satisfy each record (licenceAlternatives). A record's `OR` lets the package elect either side; its `AND` must be
 * named whole.
 */
function packageLicenceFailure(license: string, recorded: ReadonlyMap<string, string>): string | undefined {
  const named = [...new Set(spdxIds(license))].sort(compareDeterministicStrings);
  let carried: string[][] = [[RERAIL_LICENSE]];
  for (const [project, licence] of recorded) {
    const alternatives = licenceAlternatives(licence);
    if (alternatives === undefined) return `package.json: license ${JSON.stringify(license)} names ${named.join(", ")}; licenses/${project}/UPSTREAM.md records ${licence}, which is no SPDX expression`;
    carried = carried.flatMap(terms => alternatives.map(alternative => [...new Set([...terms, ...alternative])].sort(compareDeterministicStrings)));
  }
  const choices = new Set(carried.map(terms => terms.join(", ")));
  if ((licenceAlternatives(license) ?? []).some(terms => choices.has([...terms].sort(compareDeterministicStrings).join(", ")))) return undefined;
  return `package.json: license ${JSON.stringify(license)} names ${named.join(", ")}; the pack carries ${[...choices].join(" or ")} (${RERAIL_LICENSE} and what licenses/*/UPSTREAM.md records)`;
}

/** A pack-path glob a `licenses/<project>/UPSTREAM.md` declares, with the record declaring it. */
interface SourcePattern {
  glob: string;
  record: string;
  matcher: RegExp;
}

/** A "Taken from it" glob, with the licence its record records and its copy kind (copyKind), which decidingPattern weighs. */
interface TakenPattern extends SourcePattern {
  licence: string | undefined;
  how: string;
}

interface Sources {
  /** The licence each `licenses/<project>/UPSTREAM.md` records. */
  recorded: Map<string, string>;
  /** The "Taken from it" globs: what each upstream repository's files are, by pack path. */
  taken: TakenPattern[];
  /** The "Not copied" globs: pack paths no file may have, with the upstream prefix each stands for. */
  notCopied: (SourcePattern & { prefix: string })[];
  failures: string[];
}

/** Per upstream repository (`licenses/<project>/UPSTREAM.md`): its licence, its Source row, and its patterns, compiled once. */
function readSources(directory: string): Sources {
  const recorded = new Map<string, string>();
  const taken: TakenPattern[] = [];
  const notCopied: (SourcePattern & { prefix: string })[] = [];
  const failures: string[] = [];
  const licenses = join(directory, "licenses");
  if (!existsSync(licenses)) return { recorded, taken, notCopied, failures };
  for (const project of readdirSync(licenses, { withFileTypes: true }).filter(entry => entry.isDirectory()).map(entry => entry.name).sort(compareDeterministicStrings)) {
    const record = `licenses/${project}/UPSTREAM.md`;
    const path = join(directory, record);
    if (!existsSync(path)) {
      failures.push(`licenses/${project}/: no UPSTREAM.md`);
      continue;
    }
    if (!readdirSync(join(licenses, project)).some(name => name !== "UPSTREAM.md")) failures.push(`licenses/${project}/: no upstream licence text beside UPSTREAM.md`);
    const sources = readUpstreamSources(readFileSync(path, "utf8"));
    if (sources.licence === undefined) failures.push(`${record}: no "License: <SPDX id>" line`);
    else recorded.set(project, sources.licence);
    if (sources.source === undefined) failures.push(`${record}: no Source row (| <repository URL> | <tag> | <commit> | <licence> |)`);
    else {
      const { repository, tag, commit, licence } = sources.source;
      const lacking = [...(tag ? [] : ["a tag"]), ...(commit !== undefined && /^[0-9a-f]{40}$/u.test(commit) ? [] : ["a 40-hex commit"]), ...(licence ? [] : ["a licence"])];
      if (lacking.length > 0) failures.push(`${record}: Source row ${repository} lacks ${lacking.join(" and ")} (| <repository URL> | <tag> | <commit> | <licence> |)`);
    }
    for (const line of sources.malformed) failures.push(`${record}: "${line}" is not a \`- Not copied: \`<prefix>\` as \`<glob>\`\` line`);
    /** The glob's matcher, or undefined (and a failure) when pathGlob refuses it. */
    const compiled = (glob: string): RegExp | undefined => {
      try {
        return pathGlob(glob);
      } catch (error) {
        failures.push(`${record}: ${message(error)}`);
        return undefined;
      }
    };
    for (const { glob, how } of sources.taken) {
      const matcher = compiled(glob);
      if (matcher !== undefined) taken.push({ glob, record, matcher, licence: sources.licence, how });
    }
    for (const { prefix, glob } of sources.notCopied) {
      if (glob === undefined) failures.push(`${record}: "Not copied: \`${prefix}\`" names no pack-path glob (\`… as \`<glob>\`\`)`);
      else {
        const matcher = compiled(glob);
        if (matcher !== undefined) notCopied.push({ prefix, glob, record, matcher });
      }
    }
  }
  return { recorded, taken, notCopied, failures };
}

/**
 * Check 6's per-file checks over the files deeper than `upstream/<tool>/` (`files`, pack-relative): (c) none matches a
 * "Not copied" glob; (b) each is Rerail's (`authored`, first) or matches some "Taken from it" glob, and equal-length
 * longest globs covering it agree on licence and copy kind (decidingPattern); (a) each glob matches at least one copied
 * file. A glob matching nothing because the checkout lacks its tool's test data gives one failure per tool, saying to
 * pull it.
 */
function sourceFailures(directory: string, sources: Sources, authored: ReadonlySet<string>, files: readonly string[]): string[] {
  const failures: string[] = [];
  const matched = new Set<SourcePattern>();
  for (const file of files) {
    for (const refused of sources.notCopied.filter(pattern => pattern.matcher.test(file)))
      failures.push(`${file} matches \`${refused.glob}\`, which ${refused.record} lists as not copied (${refused.prefix})`);
    if (authored.has(file)) continue;
    const covering = sources.taken.filter(pattern => pattern.matcher.test(file));
    for (const pattern of covering) matched.add(pattern);
    if (covering.length === 0)
      failures.push(`${file} is under upstream/<tool>/ but no licenses/*/UPSTREAM.md pattern covers it (copied) and its tool's UPSTREAM.md does not list it (Rerail's)`);
    const { tie } = decidingPattern(covering);
    if (tie !== undefined)
      failures.push(`${file} is covered by equal-length globs that disagree: ${tie.map(({ record, glob, licence, how }) => `${record}: \`${glob}\`, ${licence ?? "no licence"}, ${how || "no copy kind"}`).join("; ")}`);
  }
  const absent = new Set<string>();
  for (const pattern of sources.taken.filter(pattern => !matched.has(pattern))) {
    const absence = packFixtureAbsence(join(directory, pattern.glob.replaceAll("*", "x")));
    if (absence === undefined) failures.push(`${pattern.record}: \`${pattern.glob}\` matches no copied file`);
    else if (!absent.has(absence)) {
      absent.add(absence);
      failures.push(`${pattern.record}: \`${pattern.glob}\` matches no file: ${absence}`);
    }
  }
  return failures;
}

/**
 * The failures for listed files (file, the UPSTREAM.md listing it) the pack directory lacks. Pack test data is not in
 * git, so a checkout without it lacks every listed copy: one failure per record says to pull and derive it.
 */
function missingListed(directory: string, listed: Iterable<readonly [file: string, record: string]>): string[] {
  const failures: string[] = [];
  const absent = new Map<string, string[]>();
  for (const [file, record] of listed) {
    const path = join(directory, file);
    if (existsSync(path)) continue;
    if (packFixtureAbsence(path) !== undefined) absent.set(record, [...(absent.get(record) ?? []), file]);
    else failures.push(`${record} lists ${file}, which does not exist`);
  }
  for (const [record, files] of absent) {
    const more = files.length === 1 ? "" : ` (and ${files.length - 1} more listed file(s))`;
    failures.push(`${record} lists ${files[0]}${more}: ${packFixtureAbsence(join(directory, files[0]!))}`);
  }
  return failures;
}

/**
 * Files Rerail wrote under `upstream/<tool>/` subdirectories (a corpus authored for the tool to score), as the tool's own
 * `upstream/<tool>/UPSTREAM.md` lists them, pack-relative, and every file under `upstream/<tool>/authored/`, the place
 * of a contributor's corpus and its ground truth (AUTHORED_CORPUS_PATTERN, scripts/s3/pack-fixtures.mjs). A record may
 * list only its tool's files.
 */
function readAuthored(directory: string): { authored: Set<string>; failures: string[] } {
  const authored = new Set<string>();
  const failures: string[] = [];
  const missing: [file: string, record: string][] = [];
  const upstream = join(directory, "upstream");
  if (!existsSync(upstream)) return { authored, failures };
  for (const tool of readdirSync(upstream, { withFileTypes: true }).filter(entry => entry.isDirectory()).map(entry => entry.name).sort(compareDeterministicStrings)) {
    for (const file of filesUnder(directory, join(upstream, tool, "authored"))) authored.add(file);
    const record = `upstream/${tool}/UPSTREAM.md`;
    if (!existsSync(join(directory, record))) continue;
    for (const line of readFileSync(join(directory, record), "utf8").split("\n")) {
      const file = UPSTREAM_ROW.exec(line)?.[1];
      if (file === undefined) continue;
      if (!file.startsWith(`upstream/${tool}/`)) failures.push(`${record} lists ${file}, which is not under upstream/${tool}/`);
      else if (!existsSync(join(directory, file))) missing.push([file, record]);
      else authored.add(file);
    }
  }
  failures.push(...missingListed(directory, missing));
  return { authored, failures };
}

const PACK_LIST_MODULE = "src/engine/catalog/pack-list.generated.ts";

/** The longest description, for people browsing the catalog (docs/pack-format.md, "Rule text"). */
export const DESCRIPTION_CHARS = 200;
/** Upstream tools a rule's text must not name; a tool's plugins share its name (`eslint-plugin-react`, `typescript-eslint`). */
const TOOL_NAMES = /\b(?:eslint|oxlint|biome|ruff|bandit|pylint|flake8|clippy|sqlfluff|phpstan|psalm|phpcs|tlint|bladestan|lizard)\b/iu;
/** A sentence end inside the text: a full stop or question mark, then a capital. */
const SENTENCE_BREAK = /[.?]\s+\p{Lu}/u;
/** Text the rule quotes, a configured value or code, which may hold any word. */
const QUOTED = /'[^']*'|`[^`]*`/gu;
/** Abbreviations whose full stop ends no sentence. */
const ABBREVIATION = /\b(?:e\.g|i\.e|etc|vs)\./giu;

/**
 * Check 8 (text): a rule document's description meets the description criteria a machine can check. It is one
 * sentence ending in a period, at most DESCRIPTION_CHARS, with no backticks and no tool name. Violation messages are
 * left to review (docs/pack-format.md, "Message"): no check reads them.
 */
export function textFailures(file: string, text: string): string[] {
  // A conversation rule carries no description: its schema has none.
  if (isConversationDocument(text)) return [];
  let document: unknown;
  try {
    document = JSON.parse(text);
  } catch {
    return []; // Check 1 reports a document that does not parse.
  }
  const description = typeof document === "object" && document !== null && "description" in document ? document.description : undefined;
  return descriptionFailures(file, description);
}

/** Check 8 on a rule's description, from its JSON document or its `.rerail` file's `.description`. */
export function descriptionFailures(file: string, description: unknown): string[] {
  const failures: string[] = [];
  const sentence = (what: string, value: string, limit: number, form: boolean): void => {
    const plain = value.replace(QUOTED, "''").replace(ABBREVIATION, "");
    if (value.length > limit) failures.push(`${file}: ${what} is ${value.length} characters, over ${limit}: ${value}`);
    if (form && !value.endsWith(".")) failures.push(`${file}: ${what} does not end with a period: ${value}`);
    if (form && SENTENCE_BREAK.test(plain)) failures.push(`${file}: ${what} is more than one sentence: ${value}`);
    if (TOOL_NAMES.test(plain)) failures.push(`${file}: ${what} names an upstream tool: ${value}`);
  };
  if (typeof description !== "string" || description.trim() === "") failures.push(`${file}: has no description`);
  else {
    sentence("description", description, DESCRIPTION_CHARS, true);
    if (description.includes("`")) failures.push(`${file}: description holds code (backticks): ${description}`);
  }
  return failures;
}

/**
 * Whether the pack's package.json says `"private": true`: never published. A package.json that does not parse, or whose
 * `private` is not a boolean, is check 5's to report.
 */
function declaresPrivate(directory: string): boolean {
  try {
    return isPrivatePack(JSON.parse(readFileSync(join(directory, "package.json"), "utf8")));
  } catch {
    return false;
  }
}

/**
 * Check 7 (manifest): `pack.json` has a `match`, so set-up can recommend the pack; and, when `listed`, core's
 * generated pack list is current and names the pack, or leaves it out when it is private.
 */
function manifestFailures(directory: string, packJson: unknown, listed: boolean): string[] {
  const failures: string[] = [];
  const id = basename(directory);
  if (listed) {
    if (readFileSync(PACK_LIST_MODULE_PATH, "utf8") !== renderPackListModule()) failures.push(`${PACK_LIST_MODULE} is stale; run npm run generate:packs`);
    const named = RULE_PACK_LIST.some(entry => entry.id === id);
    if (declaresPrivate(directory)) {
      if (named) failures.push(`${PACK_LIST_MODULE} lists ${id}, which is private: set-up would fetch it from npm, where it is never published; run npm run generate:packs`);
    } else if (!named) failures.push(`${PACK_LIST_MODULE} does not list ${id}; run npm run generate:packs`);
  }
  if (packJson === undefined) return [...failures, "pack.json does not parse; see check 1"];
  const raw = z.looseObject({ match: z.unknown() }).safeParse(packJson);
  if (!raw.success || raw.data.match === undefined) return [...failures, "pack.json has no match: set-up could never recommend this pack"];
  if (!rulePackFileSchema.shape.match.safeParse(raw.data.match).success) return [...failures, "pack.json: match does not pass the schema; see check 1"];
  return failures;
}

/** Records a failure under `check`, and under `rule` (a pack.json rule id) for that rule's status. */
type Fail = (check: PackCheckName, text: string, rule?: string) => void;

/** What every check reads: the pack's directory and id, the options validatePack was given, and where a failure goes. */
interface Validation {
  directory: string;
  id: string;
  options: ValidatePackOptions;
  fail: Fail;
}

/** What check 1 reads, which later checks read too. */
interface ReadPack {
  pack: RulePackFile | undefined;
  /** pack.json as JSON, before the schema: check 7 reads its `match` even when another field fails check 1. */
  packJson: unknown;
  /** Each listed rule's document, compiled, by rule id. */
  compiled: Map<string, Compiled>;
}

type ListedRule = RulePackFile["rules"][number];
type Port = LoadedPack["pack"]["rules"][number];
type PackPackage = z.infer<typeof rulePackPackageSchema>;
type SuppressionGaps = NonNullable<ReturnType<typeof suppressionGaps>>;

/** pack.json, parsed and then checked against the schema; check 1 fails one that is missing or does not pass. */
function readPackJson({ directory, fail }: Validation): Pick<ReadPack, "pack" | "packJson"> {
  const packPath = join(directory, "pack.json");
  if (!existsSync(packPath)) {
    fail("rules", "pack.json is missing");
    return { pack: undefined, packJson: undefined };
  }
  let packJson: unknown;
  try {
    packJson = parseJsonWithoutDuplicateKeys(readFileSync(packPath, "utf8"), "pack.json");
    const parsed = rulePackFileSchema.safeParse(packJson);
    if (parsed.success) return { pack: parsed.data, packJson };
    fail("rules", `pack.json: ${z.prettifyError(parsed.error)}`);
  } catch (error) {
    fail("rules", `pack.json: ${message(error)}`);
  }
  return { pack: undefined, packJson };
}

/** Each listed rule by its document, failing a rule listed twice or a document listed for two rules. */
function listedDocuments({ fail }: Validation, pack: RulePackFile): ReadonlyMap<string, string> {
  const byDocument = new Map<string, string>();
  const seen = new Set<string>();
  for (const rule of pack.rules) {
    if (seen.has(rule.id)) fail("rules", `${rule.id}: listed twice`, rule.id);
    seen.add(rule.id);
    const other = byDocument.get(rule.document);
    if (other !== undefined) fail("rules", `${rule.id}: ${rule.document} is also ${other}'s document`, rule.id);
    byDocument.set(rule.document, rule.id);
  }
  return byDocument;
}

/**
 * Check 1 on pack.json's listing: no rule listed twice, no document listed for two rules, every rules/ file listed or
 * included by a listed `.rerail` rule (`included`, the pack's own library files).
 */
function checkListing(validation: Validation, pack: RulePackFile, included: ReadonlySet<string>): void {
  const byDocument = listedDocuments(validation, pack);
  for (const file of filesUnder(validation.directory, join(validation.directory, "rules"))) {
    if (!byDocument.has(file) && !included.has(file)) validation.fail("rules", `${file} is not listed in pack.json`);
  }
}

/** Check 1 on a listed JSON document's text: it carries the listed id and compiles. */
function compileJsonListed({ fail }: Validation, rule: ListedRule, text: string): Compiled {
  const documentId = documentIdOf(text);
  if (documentId !== undefined && documentId !== rule.id) fail("rules", `${rule.id}: ${rule.document} has id ${documentId}`, rule.id);
  return compileRuleDocument(text);
}

/**
 * Check 1 on one listed rule: its document is named for its id, exists, carries the id and compiles (one written in
 * `.rerail`: the engine reads and checks it, `described`, by path); undefined when it is missing.
 */
function compileListed(validation: Validation, rule: ListedRule, described: ReadonlyMap<string, RrDescription>): Compiled | undefined {
  const { directory, fail } = validation;
  const extension = extname(rule.document);
  if (basename(rule.document, extension) !== rule.id) fail("rules", `${rule.id}: its document is ${rule.document}; name it rules/${rule.id}${extension}`, rule.id);
  const path = join(directory, rule.document);
  if (!existsSync(path)) {
    fail("rules", `${rule.id}: ${rule.document} does not exist`, rule.id);
    return undefined;
  }
  const result = extension === RERAIL_DOCUMENT_EXTENSION ? rerailListed(rule.id, described.get(path), rule.mode) : compileJsonListed(validation, rule, readFileSync(path, "utf8"));
  if (result.kind === "failed") fail("rules", `${rule.id}: ${rule.document} ${result.message}`, rule.id);
  return result;
}

/** The engine's reading of every listed `.rerail` document that exists, by its path. */
function describeRerailDocuments(directory: string, pack: RulePackFile): ReadonlyMap<string, RrDescription> {
  const paths = pack.rules
    .filter(rule => extname(rule.document) === RERAIL_DOCUMENT_EXTENSION)
    .map(rule => join(directory, rule.document))
    .filter(path => existsSync(path));
  return paths.length === 0 ? new Map() : describeListed(paths);
}

/**
 * Check 1: rules. pack.json parses, every rules/ file is listed or included by a listed `.rerail` rule, every listed
 * document is named for its id, exists, carries the listed id and compiles against core's capabilities (a
 * conversation rule: parses, and the engine accepts its patterns; a `.rerail` rule: the engine reads and checks it);
 * candidates and ports follow the same id rule.
 */
function checkRules(validation: Validation): ReadPack {
  const { directory, id, fail } = validation;
  const { pack, packJson } = readPackJson(validation);
  const compiled = new Map<string, Compiled>();
  if (pack !== undefined) {
    if (pack.id !== id) fail("rules", `pack.json: id is ${pack.id}; the directory is ${id}`);
    const described = describeRerailDocuments(directory, pack);
    checkListing(validation, pack, includedPackFiles(directory, described.values()));
    for (const rule of pack.rules) {
      const result = compileListed(validation, rule, described);
      if (result !== undefined) compiled.set(rule.id, result);
    }
  }
  for (const failure of idFailures(directory)) fail("rules", failure);
  return { pack, packJson, compiled };
}

/**
 * The ported rules: the pack's conformance.json, the conformance runner's pack (conformance/languages/pack.ts), or
 * undefined without one. One that does not load is check 4's failure.
 */
function loadPorts({ directory, fail }: Validation): LoadedPack | undefined {
  if (!existsSync(join(directory, CONFORMANCE_FILE))) return undefined;
  try {
    return loadPack(directory);
  } catch (error) {
    fail("ports", `${CONFORMANCE_FILE}: ${message(error).replace(`${directory}${sep}`, "")}`);
    return undefined;
  }
}

/**
 * Check 2 on an own rule (conformance.json `own`), which pack.json must ship. It has no port entry, so check 2 already
 * asks it for examples. It has no upstream tool whose comments it honours either: every port carries
 * `violation.suppress`, so one in an own rule's document is a port moved into `own`, out of the gates that compare it
 * with its upstream.
 */
function checkOwnRule({ directory, fail }: Validation, shipped: ReadonlyMap<string, ListedRule>, id: string): void {
  const rule = shipped.get(id);
  if (rule === undefined) {
    fail("proof", `${CONFORMANCE_FILE} lists own rule ${id}, which pack.json does not ship`);
    return;
  }
  const path = join(directory, rule.document);
  // A missing document is check 1's failure.
  const suppressing = existsSync(path) ? suppressingClauses(readFileSync(path, "utf8")) : [];
  if (suppressing.length > 0) fail("proof", `${id}: an own rule whose clause(s) ${suppressing.join(", ")} set violation.suppress; an own rule has no upstream tool's comments to honour, so this is a port: give it a port entry in ${CONFORMANCE_FILE}`, id);
}

/** Check 2: proof. Every rule has examples or a port entry, and every own rule is shipped and sets no violation.suppress. */
function checkProof(validation: Validation, pack: RulePackFile | undefined, loaded: LoadedPack | undefined): void {
  const { fail } = validation;
  if (pack === undefined) {
    fail("proof", "pack.json does not parse; see check 1");
    return;
  }
  const ported = new Set((loaded?.pack.rules ?? []).map(port => port.policy.replace(/^\.\//u, "")));
  for (const rule of pack.rules) {
    if (rule.examples === undefined && !ported.has(rule.document)) fail("proof", `${rule.id}: no examples and no port entry in ${CONFORMANCE_FILE}`, rule.id);
  }
  const shipped = new Map(pack.rules.map(rule => [rule.id, rule]));
  for (const id of loaded?.pack.own ?? []) checkOwnRule(validation, shipped, id);
}

/** Check 3 on a pack.json rule's example pair, when it has one. */
function checkRuleExamples({ fail }: Validation, rule: ListedRule, result: Compiled | undefined, registry: () => ConversationRuleRegistry): void {
  if (rule.examples === undefined) return;
  if (result?.kind === "conversation") for (const failure of conversationExampleFailures(result.rule, rule.examples, registry)) fail("examples", failure, rule.id);
  if (result?.kind !== "compiled") return;
  for (const failure of exampleFailures(rule.id, result, rule.examples)) fail("examples", failure, rule.id);
}

/** Check 3 on the example pair a port carries in conformance.json, as a candidate does until it ships. */
function checkPortExamples({ directory, fail }: Validation, port: Port): void {
  if (port.examples === undefined) return;
  const path = join(directory, documentPath(port.policy));
  // A missing candidate document is check 4's failure (the conformance runner loads every policy).
  if (!existsSync(path)) return;
  const result = compileCodeRule(readFileSync(path, "utf8"));
  if (result.kind === "failed") fail("examples", `${port.id}: ${documentPath(port.policy)} ${result.message}`);
  else if (result.kind === "compiled") for (const failure of exampleFailures(port.id, result, port.examples)) fail("examples", failure);
}

/**
 * Check 3: examples. A caught example produces a violation of the rule (fires it, for a conversation rule), a passes
 * example none: pack.json's, and the pairs candidates carry in conformance.json until they ship.
 */
function checkExamples(validation: Validation, { pack, compiled }: ReadPack, loaded: LoadedPack | undefined): void {
  const { fail } = validation;
  let registry: ConversationRuleRegistry | undefined;
  const conversationRegistry = (): ConversationRuleRegistry => (registry ??= new ConversationRuleRegistry({ modelPath: resolveModelPath(process.cwd()) }));
  if (pack === undefined) fail("examples", "pack.json does not parse; see check 1");
  for (const rule of pack?.rules ?? []) checkRuleExamples(validation, rule, compiled.get(rule.id), conversationRegistry);
  for (const port of loaded?.pack.rules ?? []) checkPortExamples(validation, port);
}

/**
 * Checks 2 and 3 on the rules written in `.rerail`: a pack's own rule sets no `suppress` and carries a `.test` block;
 * its examples, run through the `.rerail` runner, catch and pass as they say; and its `.test` blocks pass.
 */
async function checkRerailRules({ directory, fail }: Validation, { pack, compiled }: ReadPack): Promise<void> {
  for (const rule of pack?.rules ?? []) {
    const result = compiled.get(rule.id);
    if (result?.kind !== "rerail") continue;
    for (const failure of rerailProofFailures(rule.id, result.described)) fail("proof", failure, rule.id);
    const examples = rule.examples === undefined ? [] : await rerailExamplePairFailures(rule.id, result.described, rule.examples);
    for (const failure of [...examples, ...rerailTestFailures(rule.id, join(directory, rule.document))]) fail("examples", failure, rule.id);
  }
}

/** Per port id, the pack.json rule that ships the port (`ruleOfPort`), and why the port is not supported yet (`notSupported`). */
interface PortRules {
  ruleOfPort: Map<string, string>;
  notSupported: Map<string, string>;
}

/** Why a port is not supported yet (its document compiles to `language-not-supported`), or undefined. */
function notSupportedReason({ directory }: Validation, port: Port, result: Compiled | undefined): string | undefined {
  if (result?.kind === "not-supported-yet") return result.reason;
  if (result !== undefined || !existsSync(join(directory, port.policy))) return undefined;
  const candidate = compileCodeRule(readFileSync(join(directory, port.policy), "utf8"));
  return candidate.kind === "not-supported-yet" ? candidate.reason : undefined;
}

/** Each port's pack.json rule and, if it is not supported yet, why; a candidate's document is compiled to tell. */
function portRules(validation: Validation, loaded: LoadedPack, { pack, compiled }: ReadPack): PortRules {
  const ports: PortRules = { ruleOfPort: new Map(), notSupported: new Map() };
  for (const port of loaded.pack.rules) {
    const listed = pack?.rules.find(rule => rule.document === port.policy.replace(/^\.\//u, ""));
    if (listed !== undefined) ports.ruleOfPort.set(port.id, listed.id);
    const reason = notSupportedReason(validation, port, listed === undefined ? undefined : compiled.get(listed.id));
    if (reason !== undefined) ports.notSupported.set(port.id, reason);
  }
  return ports;
}

/** Check 4's scoring: the conformance runner's fixture tier over the ports supported yet, and its capability coverage. */
async function scorePorts({ options, fail }: Validation, loaded: LoadedPack, { ruleOfPort, notSupported }: PortRules): Promise<void> {
  const report = await runPack(loaded, {
    rules: loaded.pack.rules.filter(port => !notSupported.has(port.id)).map(port => port.id),
    fixtures: options.fixtures ?? true,
    samples: false,
    date: new Date().toISOString().slice(0, 10),
  });
  for (const failure of reportFailures(report)) {
    const port = loaded.pack.rules.find(entry => failure.startsWith(`${entry.id}:`) || failure.startsWith(`${entry.id} (`));
    fail("ports", failure, port === undefined ? undefined : ruleOfPort.get(port.id));
  }
  for (const capability of report.coverage.uncovered) fail("ports", `capability ${capability}: no ported rule exercises it and it has no exemption`);
}

/** A port's suppression gaps (suppressionGaps), with what its failures name. */
interface PortSuppression {
  port: Port;
  /** The port's document, pack-relative. */
  document: string;
  suppression: Suppression | undefined;
  gaps: SuppressionGaps;
  /** The pack.json rule that ships the port, if any. */
  owner: string | undefined;
}

/**
 * Check 4 on a port's upstream rule in its suppression: a port not exempt names it on every violation, and an
 * exemption the port does not need is stale.
 */
function checkSuppressionExemption({ fail }: Validation, { port, document, suppression, gaps, owner }: PortSuppression, exempt: boolean): void {
  const { tool, rule } = port.upstream;
  if (exempt) {
    if (suppression === undefined) fail("ports", `${port.id}: stale suppression exemption: ${tool} has no suppression comment a port must honour`, owner);
    else if (gaps.unsuppressed.length === 0) fail("ports", `${port.id}: stale suppression exemption: every violation of ${document} names ${rule} in suppress.${suppression.key}.${suppression.list}`, owner);
  } else if (suppression !== undefined && gaps.unsuppressed.length > 0) {
    fail(
      "ports",
      `${port.id}: ${document} reports ${tool}'s ${rule} without suppress.${suppression.key}.${suppression.list} naming it (clause ${gaps.unsuppressed.join(", ")}), so the comments that silence ${rule} are ignored; add it, or exempt the port in suppressionExemptions with a reason`,
      owner,
    );
  }
}

/** Check 4 on the suppressions beside ESLint's in a port: Oxlint's on the same violations, Biome's naming the same rules. */
function checkLinterSuppressions({ fail }: Validation, { port, document, gaps, owner }: PortSuppression): void {
  if (gaps.oxlintApart.length > 0)
    fail("ports", `${port.id}: ${document} carries suppress.eslint and suppress.oxlint on different violations (clause ${gaps.oxlintApart.join(", ")}); Oxlint reads eslint-disable comments too, so both go on the same violations`, owner);
  if (gaps.biomeMissing.clauses.length > 0)
    fail(
      "ports",
      `${port.id}: ${document} carries suppress.eslint without suppress.biome naming ${gaps.biomeMissing.rules.join(", ")} (clause ${gaps.biomeMissing.clauses.join(", ")}), the Biome 2.5.15 rule${gaps.biomeMissing.rules.length === 1 ? "" : "s"} the same as the ESLint rule it names, so the biome-ignore comments that silence it are ignored; run scripts/generate/packs/biome-suppression.mjs`,
      owner,
    );
  if (gaps.biomeAlone.length > 0)
    fail("ports", `${port.id}: ${document} carries suppress.biome without suppress.eslint (clause ${gaps.biomeAlone.join(", ")}); a port names the Biome rules the same as the ESLint rule it names`, owner);
}

/** Check 4 on one port's suppression comments, when its document exists and reads as a rule document. */
function checkPortSuppression(validation: Validation, port: Port, exempt: boolean, owner: string | undefined): void {
  const { directory } = validation;
  const path = join(directory, port.policy);
  if (!existsSync(path)) return;
  const { tool, rule } = port.upstream;
  const suppression = suppressionOf(tool);
  const gaps =
    suppression === undefined
      ? { unsuppressed: [], oxlintApart: [], biomeMissing: { clauses: [], rules: [] }, biomeAlone: [] }
      : suppressionGaps(readFileSync(path, "utf8"), suppression, tool, rule);
  if (gaps === undefined) return;
  const found: PortSuppression = { port, document: documentPath(port.policy), suppression, gaps, owner };
  checkSuppressionExemption(validation, found, exempt);
  checkLinterSuppressions(validation, found);
}

/**
 * Check 4: ports. The fixture tier of the conformance runner, with rules not supported yet skipped, plus capability
 * coverage and each port's suppression comments.
 */
async function checkPorts(validation: Validation, read: ReadPack, loaded: LoadedPack | undefined): Promise<void> {
  if (loaded === undefined) return;
  const ports = portRules(validation, loaded, read);
  await scorePorts(validation, loaded, ports);
  // A port of a tool that honours its own suppression comments names the upstream rule in that suppression on every
  // violation, unless conformance.json exempts it with a reason; an exemption the port does not need is stale. A
  // port that also honours Oxlint's comments does so on the same violations as ESLint's, and one whose ESLint rule
  // Biome has a rule the same as names that rule under suppress.biome where it names the ESLint rule.
  const exempt = new Set(loaded.pack.suppressionExemptions.map(exemption => exemption.rule));
  for (const port of loaded.pack.rules) checkPortSuppression(validation, port, exempt.has(port.id), ports.ruleOfPort.get(port.id));
}

/** package.json as JSON, or undefined (check 5's failure) when it does not read or parse. */
function parsePackageJson({ fail }: Validation, path: string): unknown {
  try {
    return parseJsonWithoutDuplicateKeys(readFileSync(path, "utf8"), "package.json");
  } catch (error) {
    fail("package", `package.json: ${message(error)}`);
    return undefined;
  }
}

/** package.json, checked against the schema; check 5 fails one that is missing, has dependencies or does not pass. */
function readPackageManifest(validation: Validation): PackPackage | undefined {
  const { directory, fail } = validation;
  const packagePath = join(directory, "package.json");
  if (!existsSync(packagePath)) {
    fail("package", "package.json is missing");
    return undefined;
  }
  const raw = parsePackageJson(validation, packagePath);
  const dependencies = z.looseObject({ dependencies: z.record(z.string(), z.unknown()).optional() }).safeParse(raw);
  const named = dependencies.success ? Object.keys(dependencies.data.dependencies ?? {}) : [];
  if (named.length > 0) fail("package", `package.json: has dependencies (${named.join(", ")}); a pack has none`);
  if (raw === undefined) return undefined;
  const parsed = rulePackPackageSchema.safeParse(raw);
  if (parsed.success) return parsed.data;
  fail("package", `package.json: ${z.prettifyError(parsed.error)}`);
  return undefined;
}

/** Check 5 on where the package comes from and goes: named for the pack, from this repository's packs/<id>/, public or else private. */
function checkPublishing({ id, fail }: Validation, manifest: PackPackage): void {
  if (manifest.name !== rulePackPackageName(id)) fail("package", `package.json: name is ${manifest.name}, not ${rulePackPackageName(id)}`);
  const repository = { type: "git", url: repositoryUrl(), directory: `packs/${id}` };
  if (!isDeepStrictEqual(manifest.repository, repository))
    fail("package", `package.json: repository is ${JSON.stringify(manifest.repository) ?? "missing"}, not ${JSON.stringify(repository)}`);
  if (manifest.private === false)
    fail("package", "package.json: private is false; omit private for a published pack, so there is one way to say it");
  if (!isPrivatePack(manifest) && manifest.publishConfig?.access !== "public")
    fail("package", `package.json: publishConfig.access is ${manifest.publishConfig?.access ?? "missing"}; a published pack is public`);
}

/** Check 5 on `files`: it publishes no validation material, and publishes pack.json, the rules and the licences. */
function checkPublishedFiles({ directory, fail }: Validation, manifest: PackPackage): void {
  for (const material of VALIDATION_MATERIAL) {
    const entry = manifest.files.find(file => publishes(file, material));
    if (entry !== undefined) fail("package", `package.json: files entry ${entry} publishes ${material}, which is validation material`);
  }
  const required = ["pack.json", "rules", "LICENSE", ...(existsSync(join(directory, "licenses")) ? ["licenses"] : [])];
  for (const path of required) {
    if (!manifest.files.some(file => publishes(file, path))) fail("package", `package.json: files does not publish ${path}`);
  }
}

/** Check 5 on the core range (`peerDependencies["@neralabs/rerail"]`): it admits no core that refuses the pack. */
function checkCoreRange({ directory, options, fail }: Validation, manifest: PackPackage, pack: RulePackFile | undefined): void {
  const range = manifest.peerDependencies["@neralabs/rerail"];
  const coreVersion = options.coreVersion ?? rootPackage().version;
  const failures = [
    heldOffRangeFailure(range, heldOffRules(pack), coreVersion),
    languageRangeFailure(range, shippedLanguages(directory), coreVersion),
    rerailRangeFailure(range, rerailRules(pack), coreVersion),
  ];
  for (const failure of failures) if (failure !== undefined) fail("package", failure);
}

/**
 * Check 5 on the rule details. README.md is published; candidates/README.md, which holds the candidates' rule
 * details, is not. A rule's details move to README.md when it ships.
 */
function checkRuleDetails({ directory, fail }: Validation, pack: RulePackFile): void {
  const listed = new Set(pack.rules.map(rule => rule.id));
  for (const { heading, names } of ruleDetailHeadings(join(directory, "README.md"))) {
    if (names.length === 0) fail("package", `README.md: the Rule details heading "${heading}" names no rule`);
    for (const name of names.filter(name => !listed.has(name)))
      fail("package", `README.md: Rule details describes ${name}, which pack.json does not list; a candidate's details go in candidates/README.md`);
  }
  for (const { names } of ruleDetailHeadings(join(directory, "candidates", "README.md"))) {
    for (const name of names.filter(name => listed.has(name)))
      fail("package", `candidates/README.md: Rule details describes ${name}, which pack.json lists; a shipped rule's details go in README.md`, name);
  }
}

/**
 * Check 5: package. Named for the pack, from this repository's packs/<id>/, public or else private (never
 * published), no dependencies, no validation material published, licence agreeing with what licenses/ records, and a
 * README whose rule details describe only the rules pack.json lists.
 */
function checkPackage(validation: Validation, pack: RulePackFile | undefined, sources: Sources): void {
  const { fail } = validation;
  const manifest = readPackageManifest(validation);
  if (manifest !== undefined) {
    checkPublishing(validation, manifest);
    checkPublishedFiles(validation, manifest);
    const failure = packageLicenceFailure(manifest.license, sources.recorded);
    if (failure !== undefined) fail("package", failure);
    checkCoreRange(validation, manifest, pack);
  }
  if (pack !== undefined) checkRuleDetails(validation, pack);
}

/** Check 6 on each licence licenses/ records: an SPDX expression some choice of whose alternatives is on the allowlist. */
function checkRecordedLicences({ fail }: Validation, recorded: ReadonlyMap<string, string>): void {
  for (const [project, licence] of recorded) {
    if (licenceAlternatives(licence) === undefined) fail("licences", `licenses/${project}/UPSTREAM.md: ${licence} is no SPDX expression`);
    else if (!licenceAllowed(licence, allowedLicenses())) fail("licences", `licenses/${project}/UPSTREAM.md: ${licence} is not on the repository licence allowlist`);
  }
}

/**
 * Check 6: licences. Per upstream repository, licenses/<project>/UPSTREAM.md records its licence (an SPDX expression
 * some choice of whose alternatives is on the repository allowlist), its Source row, the pack-path globs of the files
 * taken from it and the globs nothing may be copied to (readSources). Under upstream/<tool>/, the expected findings and
 * the generator (files directly in it) are Rerail's; every file in its subdirectories (fixtures/, tests/;
 * inToolSubdirectory, which the publish licence check shares) must match no "Not copied" glob, and is then Rerail's
 * when the tool's own upstream/<tool>/UPSTREAM.md lists it, and otherwise must match a "Taken from it" glob
 * (sourceFailures). Per-file provenance is in each test-data archive's provenance.json, which no check reads. A
 * tool's publish/ is left out: a pull lays it over the tool directory, where its test data is judged, and its staged
 * samples are licensed through scripts/samples/licences.ts and checked by the publish's prep (publishedSamples).
 */
function checkLicences(validation: Validation, sources: Sources): void {
  const { directory, fail } = validation;
  for (const failure of sources.failures) fail("licences", failure);
  if (!existsSync(join(directory, "LICENSE"))) fail("licences", "LICENSE is missing");
  checkRecordedLicences(validation, sources.recorded);
  const { authored, failures: authoredFailures } = readAuthored(directory);
  for (const failure of authoredFailures) fail("licences", failure);
  const deeper = filesUnder(directory, join(directory, "upstream")).filter(path => inToolSubdirectory(path) && path.split("/")[2] !== PUBLISH_DIRECTORY);
  for (const failure of sourceFailures(directory, sources, authored, deeper)) fail("licences", failure);
}

/**
 * Check 7: manifest. Set-up can recommend the pack: its match is there, and core's pack list names it unless it is
 * private (never published, so set-up must not fetch it from npm).
 */
function checkManifest({ directory, options, fail }: Validation, packJson: unknown): void {
  const listed = options.listed ?? resolve(dirname(directory)) === PACKS_DIRECTORY;
  for (const failure of manifestFailures(directory, packJson, listed)) fail("manifest", failure);
}

/**
 * Check 8: text. Every rule document's description, shipped and candidate, meets the parts of the description
 * criteria a machine can check; violation messages are left to review. Probes are facts the ports are built on, read
 * by no one, so are left out.
 */
function checkText({ directory, fail }: Validation, { pack, compiled }: Pick<ReadPack, "pack" | "compiled">): void {
  const listedBy = new Map((pack?.rules ?? []).map(rule => [rule.document, rule.id]));
  const documents = [...filesUnder(directory, join(directory, "rules")), ...candidateDocuments(directory, filesUnder(directory, join(directory, "candidates"))).filter(file => !file.startsWith("candidates/probes/"))];
  for (const file of documents) {
    const id = listedBy.get(file);
    const result = id === undefined ? undefined : compiled.get(id);
    const failures = result?.kind === "rerail" ? descriptionFailures(file, result.described.description) : textFailures(file, readFileSync(join(directory, file), "utf8"));
    for (const failure of failures) fail("text", failure, id);
  }
}

/** Each pack.json rule's status: failed, with its first failure; not supported yet, with the engine's reason; or ok. */
function ruleResults({ pack, compiled }: ReadPack, ruleFailures: ReadonlyMap<string, string[]>): PackRuleResult[] {
  return (pack?.rules ?? []).map((rule): PackRuleResult => {
    const failures = ruleFailures.get(rule.id) ?? [];
    const result = compiled.get(rule.id);
    if (failures.length > 0) return { id: rule.id, document: rule.document, status: "failed", reason: failures[0]! };
    if (result?.kind === "not-supported-yet") return { id: rule.id, document: rule.document, status: "not-supported-yet", reason: result.reason };
    return { id: rule.id, document: rule.document, status: "ok" };
  });
}

/**
 * Validates `packs/<id>/` against the contract. Check 4 runs the conformance runner (the engine's production workspace
 * path), so a pack that ports rules takes as long as its fixture tier.
 */
export async function validatePack(directory: string, options: ValidatePackOptions = {}): Promise<PackValidation> {
  const id = basename(directory);
  const checks: Record<PackCheckName, string[]> = { rules: [], proof: [], examples: [], ports: [], package: [], licences: [], manifest: [], text: [] };
  /** Failures per pack.json rule id, for the rule's status. */
  const ruleFailures = new Map<string, string[]>();
  const fail: Fail = (check, text, rule) => {
    checks[check].push(text);
    if (rule !== undefined) ruleFailures.set(rule, [...(ruleFailures.get(rule) ?? []), text]);
  };
  const validation: Validation = { directory, id, options, fail };
  const read = checkRules(validation);
  const loaded = loadPorts(validation);
  // A rule's status names its first failure, and an own rule's example failures come before its check 2 suppression
  // failure, so check 3 records its failures first.
  checkExamples(validation, read, loaded);
  checkProof(validation, read.pack, loaded);
  await checkRerailRules(validation, read);
  await checkPorts(validation, read, loaded);
  const sources = readSources(directory);
  checkPackage(validation, read.pack, sources);
  checkLicences(validation, sources);
  checkManifest(validation, read.packJson);
  checkText(validation, read);
  return { id, directory, checks, rules: ruleResults(read, ruleFailures), pass: Object.values(checks).every(failures => failures.length === 0) };
}

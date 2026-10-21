/**
 * The pack validation contract (docs/pack-format.md, "The validation contract"): checks 1 to 8 over one
 * `packs/<id>/` directory. Test support, not runtime: test/engine/catalog/packs.test.ts calls it for every pack, and it reuses the
 * conformance runner for ported rules, so it lives beside that runner and stays out of the published core.
 */
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { isDeepStrictEqual } from "node:util";
import { basename, dirname, join, matchesGlob, posix, relative, resolve, sep } from "node:path";
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
} from "../src/engine/index.js";
import { PACK_LIST_MODULE_PATH, PACKS_DIRECTORY, renderPackListModule } from "../scripts/generate/packs/pack-list.js";
import { CONFORMANCE_FILE, defaultCorpusRoot, documentPath, loadPack, packSchema, RERAIL_ROOT, type LoadedPack } from "./languages/pack.js";
import { reportFailures, runPack } from "./languages/runner.js";
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
   * Check 4 always scores the ported rules on the tools' fixtures. Corpora: `true` scores every pinned corpus (a
   * missing checkout fails), `false` none, and by default those checked out under `corpusRoot`.
   */
  corpora?: boolean;
  /** Corpus checkouts, `<corpusRoot>/<corpus name>`; default the nearest `.rerail/corpora/<pack>`. */
  corpusRoot?: string;
  /**
   * Whether the pack must be in core's generated pack list (`src/engine/catalog/pack-list.generated.ts`), and that
   * list current. Default: when `directory` is one of the repository's `packs/` directories, the packs Rerail
   * publishes; a fixture pack elsewhere is published nowhere, so no list names it.
   */
  listed?: boolean;
}

/** Validation material: kept in the repository, never published. */
const VALIDATION_MATERIAL = [CONFORMANCE_FILE, "divergences.json", "upstream", "candidates"] as const;
/** Rerail's own licence for its material in a pack. */
const RERAIL_LICENSE = "Apache-2.0";
const UPSTREAM_LICENSE = /\bLicense: ([A-Za-z0-9][A-Za-z0-9.+-]*)/u;
const UPSTREAM_ROW = /^\|\s*`([^`]+)`/u;
/** A row's upstream path, its second column. */
const UPSTREAM_SOURCE = /^\|\s*`[^`]+`\s*\|\s*`([^`]+)`/u;
/** An upstream path prefix an UPSTREAM.md declares nothing may be copied from (a copyleft project's material). */
const NOT_COPIED = /^- Not copied: `([^`]+)`/gmu;

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
  | { kind: "not-supported-yet"; reason: string }
  | { kind: "failed"; message: string };

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
/** A rule document in some format: `rerail.policy/v1`, or an older one such as `tempo.policy/v1`. */
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
 * is a `rerail.policy/v1` document like a shipped rule; and a port in any `conformance.json` of the pack has its
 * document's id. So the id users turn a rule on and off by is the one its file, its listing and its scores carry, and a
 * candidate keeps it when it ships.
 */
function idFailures(directory: string): string[] {
  const failures: string[] = [];
  const candidates = filesUnder(directory, join(directory, "candidates"));
  for (const file of candidateDocuments(directory, candidates)) {
    const { format, id } = headerOf(readFileSync(join(directory, file), "utf8"));
    if (format !== CODE_RULE_FORMAT) failures.push(`${file}: format is ${format ?? "missing"}; a rule document is ${CODE_RULE_FORMAT}`);
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

/** The repository every pack is published from, as the root package.json names it: npm's trusted publishing checks it. */
const repositoryUrl = (): string =>
  z.object({ repository: z.object({ url: z.string() }) }).parse(JSON.parse(readFileSync(join(RERAIL_ROOT, "package.json"), "utf8"))).repository.url;

/** The SPDX ids an SPDX expression names. */
const spdxIds = (expression: string): string[] =>
  expression
    .split(/\s+(?:AND|OR|WITH)\s+|[()]/u)
    .map(part => part.trim())
    .filter(part => part.length > 0);

interface Licences {
  /** The licence each `licenses/<project>/UPSTREAM.md` records. */
  recorded: Map<string, string>;
  /** Pack-relative paths the UPSTREAM.md tables list. */
  listed: Map<string, string>;
  failures: string[];
}

function readLicences(directory: string): Licences {
  const recorded = new Map<string, string>();
  const listed = new Map<string, string>();
  const failures: string[] = [];
  const licenses = join(directory, "licenses");
  if (!existsSync(licenses)) return { recorded, listed, failures };
  for (const project of readdirSync(licenses, { withFileTypes: true }).filter(entry => entry.isDirectory()).map(entry => entry.name).sort(compareDeterministicStrings)) {
    const record = `licenses/${project}/UPSTREAM.md`;
    const path = join(directory, record);
    if (!existsSync(path)) {
      failures.push(`licenses/${project}/: no UPSTREAM.md`);
      continue;
    }
    if (!readdirSync(join(licenses, project)).some(name => name !== "UPSTREAM.md")) failures.push(`licenses/${project}/: no upstream licence text beside UPSTREAM.md`);
    const text = readFileSync(path, "utf8");
    const licence = UPSTREAM_LICENSE.exec(text)?.[1];
    if (licence === undefined) failures.push(`${record}: no "License: <SPDX id>" line`);
    else recorded.set(project, licence);
    const notCopied = [...text.matchAll(NOT_COPIED)].map(match => match[1]!);
    for (const line of text.split("\n")) {
      const file = UPSTREAM_ROW.exec(line)?.[1];
      if (file !== undefined && !listed.has(file)) listed.set(file, record);
      const source = UPSTREAM_SOURCE.exec(line)?.[1];
      const refused = source === undefined ? undefined : notCopied.find(prefix => source.startsWith(prefix));
      if (refused !== undefined) failures.push(`${record} lists ${file} as copied from ${source}, under ${refused}, which it lists as not copied`);
    }
  }
  return { recorded, listed, failures };
}

/**
 * Files Rerail wrote under `upstream/<tool>/` subdirectories (a corpus authored for the tool to score), as the tool's own
 * `upstream/<tool>/UPSTREAM.md` lists them, pack-relative. A record may list only its tool's files.
 */
function readAuthored(directory: string): { authored: Set<string>; failures: string[] } {
  const authored = new Set<string>();
  const failures: string[] = [];
  const upstream = join(directory, "upstream");
  if (!existsSync(upstream)) return { authored, failures };
  for (const tool of readdirSync(upstream, { withFileTypes: true }).filter(entry => entry.isDirectory()).map(entry => entry.name).sort(compareDeterministicStrings)) {
    const record = `upstream/${tool}/UPSTREAM.md`;
    if (!existsSync(join(directory, record))) continue;
    for (const line of readFileSync(join(directory, record), "utf8").split("\n")) {
      const file = UPSTREAM_ROW.exec(line)?.[1];
      if (file === undefined) continue;
      if (!file.startsWith(`upstream/${tool}/`)) failures.push(`${record} lists ${file}, which is not under upstream/${tool}/`);
      else if (!existsSync(join(directory, file))) failures.push(`${record} lists ${file}, which does not exist`);
      else authored.add(file);
    }
  }
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
  let document: unknown;
  try {
    document = JSON.parse(text);
  } catch {
    return []; // Check 1 reports a document that does not parse.
  }
  const description = (document as { description?: unknown }).description;
  if (typeof description !== "string" || description.trim() === "") return [`${file}: has no description`];
  const failures: string[] = [];
  const plain = description.replace(QUOTED, "''").replace(ABBREVIATION, "");
  if (description.length > DESCRIPTION_CHARS) failures.push(`${file}: description is ${description.length} characters, over ${DESCRIPTION_CHARS}: ${description}`);
  if (!description.endsWith(".")) failures.push(`${file}: description does not end with a period: ${description}`);
  if (SENTENCE_BREAK.test(plain)) failures.push(`${file}: description is more than one sentence: ${description}`);
  if (TOOL_NAMES.test(plain)) failures.push(`${file}: description names an upstream tool: ${description}`);
  if (description.includes("`")) failures.push(`${file}: description holds code (backticks): ${description}`);
  return failures;
}

/**
 * Check 7 (manifest): `pack.json` has a `match`, so set-up can recommend the pack; and, when `listed`, core's
 * generated pack list is current and names the pack.
 */
function manifestFailures(directory: string, packJson: unknown, listed: boolean): string[] {
  const failures: string[] = [];
  const id = basename(directory);
  if (listed) {
    if (readFileSync(PACK_LIST_MODULE_PATH, "utf8") !== renderPackListModule()) failures.push(`${PACK_LIST_MODULE} is stale; run npm run generate:packs`);
    if (!RULE_PACK_LIST.some(entry => entry.id === id)) failures.push(`${PACK_LIST_MODULE} does not list ${id}; run npm run generate:packs`);
  }
  if (packJson === undefined) return [...failures, "pack.json does not parse; see check 1"];
  const raw = z.looseObject({ match: z.unknown() }).safeParse(packJson);
  if (!raw.success || raw.data.match === undefined) return [...failures, "pack.json has no match: set-up could never recommend this pack"];
  if (!rulePackFileSchema.shape.match.safeParse(raw.data.match).success) return [...failures, "pack.json: match does not pass the schema; see check 1"];
  return failures;
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
  const fail = (check: PackCheckName, text: string, rule?: string): void => {
    checks[check].push(text);
    if (rule !== undefined) ruleFailures.set(rule, [...(ruleFailures.get(rule) ?? []), text]);
  };

  // Check 1: rules. pack.json parses, every rules/*.json is listed, every listed document is named for its id, exists,
  // carries the listed id and compiles against core's capabilities; candidates and ports follow the same id rule.
  let pack: RulePackFile | undefined;
  /** pack.json as JSON, before the schema: check 7 reads its `match` even when another field fails check 1. */
  let packJson: unknown;
  const packPath = join(directory, "pack.json");
  if (!existsSync(packPath)) fail("rules", "pack.json is missing");
  else {
    try {
      packJson = parseJsonWithoutDuplicateKeys(readFileSync(packPath, "utf8"), "pack.json");
      const parsed = rulePackFileSchema.safeParse(packJson);
      if (parsed.success) pack = parsed.data;
      else fail("rules", `pack.json: ${z.prettifyError(parsed.error)}`);
    } catch (error) {
      fail("rules", `pack.json: ${message(error)}`);
    }
  }
  const compiled = new Map<string, Compiled>();
  if (pack !== undefined) {
    if (pack.id !== id) fail("rules", `pack.json: id is ${pack.id}; the directory is ${id}`);
    const byDocument = new Map<string, string>();
    const seen = new Set<string>();
    for (const rule of pack.rules) {
      if (seen.has(rule.id)) fail("rules", `${rule.id}: listed twice`, rule.id);
      seen.add(rule.id);
      const other = byDocument.get(rule.document);
      if (other !== undefined) fail("rules", `${rule.id}: ${rule.document} is also ${other}'s document`, rule.id);
      byDocument.set(rule.document, rule.id);
    }
    for (const file of filesUnder(directory, join(directory, "rules"))) {
      if (!byDocument.has(file)) fail("rules", `${file} is not listed in pack.json`);
    }
    for (const rule of pack.rules) {
      if (basename(rule.document, ".json") !== rule.id) fail("rules", `${rule.id}: its document is ${rule.document}; name it rules/${rule.id}.json`, rule.id);
      const path = join(directory, rule.document);
      if (!existsSync(path)) {
        fail("rules", `${rule.id}: ${rule.document} does not exist`, rule.id);
        continue;
      }
      const text = readFileSync(path, "utf8");
      const documentId = documentIdOf(text);
      if (documentId !== undefined && documentId !== rule.id) fail("rules", `${rule.id}: ${rule.document} has id ${documentId}`, rule.id);
      const result = compileCodeRule(text);
      compiled.set(rule.id, result);
      if (result.kind === "failed") fail("rules", `${rule.id}: ${rule.document} ${result.message}`, rule.id);
    }
  }

  for (const failure of idFailures(directory)) fail("rules", failure);

  // The ported rules, by the document they port: the pack's conformance.json, the conformance runner's pack
  // (conformance/languages/pack.ts).
  const conformancePath = join(directory, CONFORMANCE_FILE);
  let loaded: LoadedPack | undefined;
  if (existsSync(conformancePath)) {
    try {
      loaded = loadPack(directory);
    } catch (error) {
      fail("ports", `${CONFORMANCE_FILE}: ${message(error).replace(`${directory}${sep}`, "")}`);
    }
  }
  const portByDocument = new Map((loaded?.pack.rules ?? []).map(port => [port.policy.replace(/^\.\//u, ""), port]));

  // Check 2: proof. Every rule has examples or a port entry.
  // Check 3: examples. A caught example produces a violation of the rule, a passes example none.
  if (pack === undefined) {
    fail("proof", "pack.json does not parse; see check 1");
    fail("examples", "pack.json does not parse; see check 1");
  } else {
    for (const rule of pack.rules) {
      if (rule.examples === undefined && !portByDocument.has(rule.document)) fail("proof", `${rule.id}: no examples and no port entry in ${CONFORMANCE_FILE}`, rule.id);
      const result = compiled.get(rule.id);
      if (rule.examples === undefined || result?.kind !== "compiled") continue;
      for (const side of ["caught", "passes"] as const) {
        const example = rule.examples[side];
        if (example.path === undefined) {
          fail("examples", `${rule.id}: the ${side} example has no path`, rule.id);
          continue;
        }
        if (!result.compiled.inScope(example.path)) {
          fail("examples", `${rule.id}: the ${side} example's path ${example.path} is outside the rule's scope`, rule.id);
          continue;
        }
        const evaluation = evaluateExample(result.compiled, example.path, example.text);
        if (evaluation.failures.length > 0) fail("examples", `${rule.id}: the ${side} example (${example.path}) fails in the engine: ${JSON.stringify(evaluation.failures)}`, rule.id);
        const hits = evaluation.codes.filter(code => result.codes.has(code));
        if (side === "caught" && hits.length === 0) fail("examples", `${rule.id}: the caught example (${example.path}) produces no ${[...result.codes].join("/")} violation`, rule.id);
        if (side === "passes" && hits.length > 0) fail("examples", `${rule.id}: the passes example (${example.path}) produces ${hits.join(", ")}`, rule.id);
      }
    }
  }

  // Check 4: ports. The fixture tier (and corpora) of the conformance runner, with rules not supported yet skipped,
  // plus capability coverage and each port's suppression comments.
  const notSupported = new Map<string, string>();
  if (loaded !== undefined) {
    const ruleOfPort = new Map<string, string>();
    for (const port of loaded.pack.rules) {
      const listed = pack?.rules.find(rule => rule.document === port.policy.replace(/^\.\//u, ""));
      if (listed !== undefined) ruleOfPort.set(port.id, listed.id);
      const result = listed === undefined ? undefined : compiled.get(listed.id);
      if (result?.kind === "not-supported-yet") notSupported.set(port.id, result.reason);
      else if (result === undefined && existsSync(join(directory, port.policy))) {
        const candidate = compileCodeRule(readFileSync(join(directory, port.policy), "utf8"));
        if (candidate.kind === "not-supported-yet") notSupported.set(port.id, candidate.reason);
      }
    }
    const corpusRoot = options.corpusRoot ?? defaultCorpusRoot(loaded.pack.pack);
    const corpora = loaded.pack.corpora
      .filter(corpus => (options.corpora === undefined ? existsSync(join(corpusRoot, corpus.name)) : options.corpora))
      .map(corpus => corpus.name);
    const report = await runPack(loaded, {
      rules: loaded.pack.rules.filter(port => !notSupported.has(port.id)).map(port => port.id),
      fixtures: true,
      corpora,
      corpusRoot,
      date: new Date().toISOString().slice(0, 10),
    });
    for (const failure of reportFailures(report)) {
      const port = loaded.pack.rules.find(entry => failure.startsWith(`${entry.id}:`) || failure.startsWith(`${entry.id} (`));
      fail("ports", failure, port === undefined ? undefined : ruleOfPort.get(port.id));
    }
    for (const capability of report.coverage.uncovered) fail("ports", `capability ${capability}: no ported rule exercises it and it has no exemption`);
    // A port of a tool that honours its own suppression comments names the upstream rule in that suppression on every
    // violation, unless conformance.json exempts it with a reason; an exemption the port does not need is stale. A
    // port that also honours Oxlint's comments does so on the same violations as ESLint's, and one whose ESLint rule
    // Biome has a rule the same as names that rule under suppress.biome where it names the ESLint rule.
    const exempt = new Set(loaded.pack.suppressionExemptions.map(exemption => exemption.rule));
    for (const port of loaded.pack.rules) {
      const path = join(directory, port.policy);
      if (!existsSync(path)) continue;
      const { tool, rule } = port.upstream;
      const suppression = suppressionOf(tool);
      const gaps =
        suppression === undefined
          ? { unsuppressed: [], oxlintApart: [], biomeMissing: { clauses: [], rules: [] }, biomeAlone: [] }
          : suppressionGaps(readFileSync(path, "utf8"), suppression, tool, rule);
      if (gaps === undefined) continue;
      const document = documentPath(port.policy);
      const owner = ruleOfPort.get(port.id);
      if (exempt.has(port.id)) {
        if (suppression === undefined) fail("ports", `${port.id}: stale suppression exemption: ${tool} has no suppression comment a port must honour`, owner);
        else if (gaps.unsuppressed.length === 0) fail("ports", `${port.id}: stale suppression exemption: every violation of ${document} names ${rule} in suppress.${suppression.key}.${suppression.list}`, owner);
      } else if (suppression !== undefined && gaps.unsuppressed.length > 0) {
        fail(
          "ports",
          `${port.id}: ${document} reports ${tool}'s ${rule} without suppress.${suppression.key}.${suppression.list} naming it (clause ${gaps.unsuppressed.join(", ")}), so the comments that silence ${rule} are ignored; add it, or exempt the port in suppressionExemptions with a reason`,
          owner,
        );
      }
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
  }

  // Check 5: package. Named for the pack, published from this repository's packs/<id>/ and restricted, no
  // dependencies, no validation material published, licence agreeing with what licenses/ records.
  const licences = readLicences(directory);
  const packagePath = join(directory, "package.json");
  if (!existsSync(packagePath)) fail("package", "package.json is missing");
  else {
    let raw: unknown;
    try {
      raw = parseJsonWithoutDuplicateKeys(readFileSync(packagePath, "utf8"), "package.json");
    } catch (error) {
      fail("package", `package.json: ${message(error)}`);
    }
    const dependencies = z.looseObject({ dependencies: z.record(z.string(), z.unknown()).optional() }).safeParse(raw);
    const named = dependencies.success ? Object.keys(dependencies.data.dependencies ?? {}) : [];
    if (named.length > 0) fail("package", `package.json: has dependencies (${named.join(", ")}); a pack has none`);
    const parsed = raw === undefined ? undefined : rulePackPackageSchema.safeParse(raw);
    if (parsed !== undefined && !parsed.success) fail("package", `package.json: ${z.prettifyError(parsed.error)}`);
    if (parsed?.success === true) {
      const manifest = parsed.data;
      if (manifest.name !== rulePackPackageName(id)) fail("package", `package.json: name is ${manifest.name}, not ${rulePackPackageName(id)}`);
      const repository = { type: "git", url: repositoryUrl(), directory: `packs/${id}` };
      if (!isDeepStrictEqual(manifest.repository, repository))
        fail("package", `package.json: repository is ${JSON.stringify(manifest.repository) ?? "missing"}, not ${JSON.stringify(repository)}`);
      if (manifest.publishConfig?.access !== "restricted")
        fail("package", `package.json: publishConfig.access is ${manifest.publishConfig?.access ?? "missing"}; packs stay restricted until launch`);
      for (const material of VALIDATION_MATERIAL) {
        const entry = manifest.files.find(file => publishes(file, material));
        if (entry !== undefined) fail("package", `package.json: files entry ${entry} publishes ${material}, which is validation material`);
      }
      const required = ["pack.json", "rules", "LICENSE", ...(existsSync(join(directory, "licenses")) ? ["licenses"] : [])];
      for (const path of required) {
        if (!manifest.files.some(file => publishes(file, path))) fail("package", `package.json: files does not publish ${path}`);
      }
      const named = [...new Set(spdxIds(manifest.license))].sort(compareDeterministicStrings);
      const carried = [...new Set([RERAIL_LICENSE, ...licences.recorded.values()])].sort(compareDeterministicStrings);
      if (named.join(",") !== carried.join(","))
        fail("package", `package.json: license ${JSON.stringify(manifest.license)} names ${named.join(", ")}; the pack carries ${carried.join(", ")} (${RERAIL_LICENSE} and what licenses/*/UPSTREAM.md records)`);
    }
  }

  // Check 6: licences. Every copied file is listed in an UPSTREAM.md, every listed file exists, no listed file is
  // copied from an upstream path its UPSTREAM.md lists as "Not copied" (readLicences), and every recorded licence is
  // on the repository allowlist. Under upstream/<tool>/, the recordings and the recorder (files directly in
  // it) are Rerail's; everything in its subdirectories (fixtures/, tests/) is copied from the tool, unless the tool's
  // own upstream/<tool>/UPSTREAM.md lists it as Rerail's.
  for (const failure of licences.failures) fail("licences", failure);
  if (!existsSync(join(directory, "LICENSE"))) fail("licences", "LICENSE is missing");
  for (const [project, licence] of licences.recorded) {
    if (!allowedLicenses().has(licence)) fail("licences", `licenses/${project}/UPSTREAM.md: ${licence} is not on the repository licence allowlist`);
  }
  const { authored, failures: authoredFailures } = readAuthored(directory);
  for (const failure of authoredFailures) fail("licences", failure);
  for (const file of filesUnder(directory, join(directory, "upstream"))) {
    if (file.split("/").length > 3 && !licences.listed.has(file) && !authored.has(file))
      fail("licences", `${file} is under upstream/<tool>/ but neither licenses/*/UPSTREAM.md (copied) nor its tool's UPSTREAM.md (Rerail's) lists it`);
  }
  for (const [file, record] of licences.listed) {
    if (!existsSync(join(directory, file))) fail("licences", `${record} lists ${file}, which does not exist`);
  }

  // Check 7: manifest. Set-up can recommend the pack: its match is there, and core's pack list names it.
  for (const failure of manifestFailures(directory, packJson, options.listed ?? resolve(dirname(directory)) === PACKS_DIRECTORY)) fail("manifest", failure);

  // Check 8: text. Every rule document's description, shipped and candidate, meets the parts of the description
  // criteria a machine can check; violation messages are left to review. Probes are facts the ports are built on, read
  // by no one, so are left out.
  const listedBy = new Map((pack?.rules ?? []).map(rule => [rule.document, rule.id]));
  const documents = [...filesUnder(directory, join(directory, "rules")), ...candidateDocuments(directory, filesUnder(directory, join(directory, "candidates"))).filter(file => !file.startsWith("candidates/probes/"))];
  for (const file of documents) {
    for (const failure of textFailures(file, readFileSync(join(directory, file), "utf8"))) fail("text", failure, listedBy.get(file));
  }

  const rules = (pack?.rules ?? []).map((rule): PackRuleResult => {
    const failures = ruleFailures.get(rule.id) ?? [];
    const result = compiled.get(rule.id);
    if (failures.length > 0) return { id: rule.id, document: rule.document, status: "failed", reason: failures[0]! };
    if (result?.kind === "not-supported-yet") return { id: rule.id, document: rule.document, status: "not-supported-yet", reason: result.reason };
    return { id: rule.id, document: rule.document, status: "ok" };
  });
  return { id, directory, checks, rules, pass: Object.values(checks).every(failures => failures.length === 0) };
}

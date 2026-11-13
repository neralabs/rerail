/**
 * Writes `src/engine/catalog/lint-config/generated/phpcs-standards.generated.ts`: for each coding standard PHP_CodeSniffer 4.0.4
 * ships, the sniffs `<rule ref="<Standard>"/>` includes and the severities its ruleset sets, in the order it sets them.
 * It fetches the standards' `ruleset.xml` files and the list of their sniffs from PHPCSStandards/PHP_CodeSniffer at the
 * commit the php pack ports (`packs/php/upstream/phpcs/gates.yaml`), and replays `Ruleset::processRuleset` (src/Ruleset.php
 * l.528-890: own sniffs, `<rule ref>` expanded, `<exclude>` of a message code as severity 0 and of anything else
 * expanded and left out, the single-message-code inclusion of l.719-748, and `processRule`'s `<severity>` for the ref
 * and every sniff it newly includes, l.1136-1173) at the sniff level, refusing what the built-in rulesets do not use.
 *
 * Run: `node --import tsx scripts/generate/lint-config/phpcs-standards.ts`. Needs the network; never run in CI.
 */
import { writeFileSync } from "node:fs";
import { parseXml, xmlChildren, type XmlElement } from "../../../src/engine/index.js";

const COMMIT = "bbdc3d0532623e21838b7041a4364383a8126f96";
const VERSION = "4.0.4";
const OUTPUT = new URL("../../../src/engine/catalog/lint-config/generated/phpcs-standards.generated.ts", import.meta.url);
const RAW = `https://raw.githubusercontent.com/PHPCSStandards/PHP_CodeSniffer/${COMMIT}`;
const TREE = `https://api.github.com/repos/PHPCSStandards/PHP_CodeSniffer/git/trees/${COMMIT}?recursive=1`;
/** `src/Standards/<Standard>/Sniffs/<Category>/<Name>Sniff.php`. */
const SNIFF_FILE = /^src\/Standards\/([^/]+)\/Sniffs\/([^/]+)\/([^/]+)Sniff\.php$/u;
const RULESET_FILE = /^src\/Standards\/([^/]+)\/ruleset\.xml$/u;

interface Upstream {
  /** Per standard, its own sniffs' codes. */
  readonly own: ReadonlyMap<string, readonly string[]>;
  readonly rulesets: ReadonlyMap<string, XmlElement>;
}

/** What processing one ruleset gives: the sniffs it includes, in order, and the severities it set. */
interface Processed {
  readonly sniffs: string[];
  readonly severities: [string, number][];
}

async function fetchText(url: string): Promise<string> {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
  return response.text();
}

async function readUpstream(): Promise<Upstream> {
  const tree = JSON.parse(await fetchText(TREE)) as { truncated: boolean; tree: { path: string }[] };
  if (tree.truncated) throw new Error("the Git tree listing is truncated");
  const own = new Map<string, string[]>();
  const standards: string[] = [];
  for (const { path } of tree.tree) {
    const sniff = SNIFF_FILE.exec(path);
    if (sniff !== null) own.set(sniff[1]!, [...own.get(sniff[1]!) ?? [], `${sniff[1]}.${sniff[2]}.${sniff[3]}`]);
    const ruleset = RULESET_FILE.exec(path);
    if (ruleset !== null) standards.push(ruleset[1]!);
  }
  const rulesets = new Map(await Promise.all(standards.map(async (name): Promise<[string, XmlElement]> => [name, parseXml(await fetchText(`${RAW}/src/Standards/${name}/ruleset.xml`))])));
  return { own, rulesets };
}

/** The sniffs `ref` names, as `expandRulesetReference` expands it: a standard (processed), a category, a sniff, a message code's sniff. */
function expand(upstream: Upstream, ref: string, severities: [string, number][]): string[] {
  const parts = ref.split(".");
  if (parts.length === 1) {
    const processed = processStandard(upstream, ref);
    severities.push(...processed.severities);
    return processed.sniffs;
  }
  const sniffs = upstream.own.get(parts[0]!) ?? [];
  const prefix = parts.slice(0, 3).join(".");
  const found = parts.length === 2 ? sniffs.filter(sniff => sniff.startsWith(`${prefix}.`)) : sniffs.filter(sniff => sniff === prefix);
  if (found.length === 0) throw new Error(`${ref} names no sniff`);
  return found;
}

/** The `<rule>` children that change where a rule reports, which this replay does not model. */
const UNMODELLED_RULE_CHILDREN = new Set(["exclude-pattern", "include-pattern"]);
/** The ruleset children this replay does not model. */
const UNMODELLED_RULESET_CHILDREN = new Set(["exclude-pattern", "arg", "ini", "autoload"]);
/** The one `<arg>` modelled: `tab-width`, which sets no rule. */
const MODELLED_ARGUMENT = "tab-width";

/** A ruleset state while its rules are applied. */
interface RulesetState {
  readonly included: string[];
  readonly excluded: string[];
  readonly severities: [string, number][];
}

/** Refuses a rule this replay does not model: a pattern, or a `phpcs-only`/`phpcbf-only` element. */
function checkModelled(standard: string, rule: XmlElement): void {
  const limited = [rule, ...rule.children].some(element => element.attributes.has("phpcs-only") || element.attributes.has("phpcbf-only"));
  if (limited || rule.children.some(child => UNMODELLED_RULE_CHILDREN.has(child.localName))) {
    throw new Error(`${standard}: <rule ref="${rule.attributes.get("ref")}"> uses what this replay does not model`);
  }
}

/** Including one message code (l.719-748): back on if its sniff was turned off, else its sniff off but for it. */
function applyMessageCodeRef(upstream: Upstream, standard: string, ref: string, newSniffs: readonly string[], state: RulesetState): void {
  const sniff = ref.split(".").slice(0, 3).join(".");
  if (state.severities.findLast(([code]) => code === sniff)?.[1] === 0) state.severities.push([ref, 5]);
  else if (newSniffs.length > 0 && !(upstream.own.get(standard) ?? []).includes(newSniffs[0]!)) state.severities.push([sniff, 0], [ref, 5]);
}

/** A rule's `<exclude>`s: a message code's severity 0, anything else left out of the ruleset's sniffs. */
function applyExcludes(upstream: Upstream, rule: XmlElement, state: RulesetState): void {
  for (const exclude of xmlChildren(rule, "exclude")) {
    const name = exclude.attributes.get("name") ?? "";
    if (name.split(".").length === 4) state.severities.push([name, 0]);
    else state.excluded.push(...expand(upstream, name, state.severities));
  }
}

/** One `<rule>` of `standard`'s ruleset, applied to `state`. */
function applyRule(upstream: Upstream, standard: string, rule: XmlElement, state: RulesetState): void {
  checkModelled(standard, rule);
  const ref = rule.attributes.get("ref") ?? "";
  const newSniffs = expand(upstream, ref, state.severities).filter(sniff => !state.included.includes(sniff));
  state.included.push(...newSniffs);
  const parts = ref.split(".");
  if (parts.length === 4) applyMessageCodeRef(upstream, standard, ref, newSniffs, state);
  applyExcludes(upstream, rule, state);
  const severity = xmlChildren(rule, "severity")[0];
  if (severity === undefined) return;
  // processRule: a standard's or a category's severity is the ref's and each sniff it newly included.
  const todo = parts.length <= 2 ? [ref, ...newSniffs] : [ref];
  for (const code of todo) state.severities.push([code, Number.parseInt(severity.text.trim(), 10)]);
}

function processStandard(upstream: Upstream, standard: string): Processed {
  const ruleset = upstream.rulesets.get(standard);
  if (ruleset === undefined) throw new Error(`${standard} is not a standard PHP_CodeSniffer ${VERSION} ships`);
  const unmodelled = ruleset.children.find(child => UNMODELLED_RULESET_CHILDREN.has(child.localName)
    && !(child.localName === "arg" && child.attributes.get("name") === MODELLED_ARGUMENT));
  if (unmodelled !== undefined) throw new Error(`${standard}: <${unmodelled.localName}> is not modelled`);
  const state: RulesetState = { included: [], excluded: [], severities: [] };
  for (const rule of xmlChildren(ruleset, "rule")) applyRule(upstream, standard, rule, state);
  const sniffs = [...new Set([...upstream.own.get(standard) ?? [], ...state.included])].filter(sniff => !state.excluded.includes(sniff));
  return { sniffs: sniffs.toSorted(), severities: state.severities };
}

function render(standards: ReadonlyMap<string, Processed>): string {
  const entries = [...standards].map(([name, { sniffs, severities }]) => [
    `  ${JSON.stringify(name)}: {`,
    "    sniffs: [",
    ...sniffs.map(sniff => `      ${JSON.stringify(sniff)},`),
    "    ],",
    "    severities: [",
    ...severities.map(([code, severity]) => `      [${JSON.stringify(code)}, ${severity}],`),
    "    ],",
    "  },",
  ].join("\n"));
  return [
    "// Generated by scripts/generate/lint-config/phpcs-standards.ts; do not edit.",
    "",
    `/** A coding standard PHP_CodeSniffer ${VERSION} ships (PHPCSStandards/PHP_CodeSniffer@${COMMIT.slice(0, 12)}, src/Standards/<name>/ruleset.xml). */`,
    "export interface PhpcsStandard {",
    "  /** The sniffs `<rule ref=\"<name>\"/>` includes, its own and those its ruleset references, less those it excludes. */",
    "  readonly sniffs: readonly string[];",
    "  /** The severities its ruleset sets, by message code, sniff, category or standard, in the order it sets them. */",
    "  readonly severities: readonly (readonly [string, number])[];",
    "}",
    "",
    "/** The standards by name. */",
    "export const PHPCS_STANDARDS: Readonly<Record<string, PhpcsStandard>> = {",
    ...entries,
    "};",
    "",
  ].join("\n");
}

const upstream = await readUpstream();
const standards = new Map([...upstream.rulesets.keys()].toSorted().map(name => [name, processStandard(upstream, name)]));
writeFileSync(OUTPUT, render(standards));

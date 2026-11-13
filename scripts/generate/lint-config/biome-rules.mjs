// Reads Biome's source at @biomejs/biome@2.5.15 and writes src/engine/catalog/lint-config/generated/biome-rules.generated.ts:
// every lint rule's metadata as Biome's configuration reads it, so set-up can tell which rules a `biome.json` turns on
// without running Biome. For each `<group>/<rule>`, one entry per language analyzer that declares it, in the order
// `lint_group_structs!` visits them (crates/biome_configuration_macros/src/lib.rs `collect_lint_rules`: JavaScript,
// JSON, CSS, GraphQL, HTML, Markdown): whether it is `recommended` and its `domains`, by their config names.
// Run offline, not in CI:
//   git clone --depth 1 --branch @biomejs/biome@2.5.15 https://github.com/biomejs/biome.git <dir>
//   node scripts/generate/lint-config/biome-rules.mjs <dir>
// Any other Biome version or commit is refused; bump the pins and regenerate to move.
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const PIN = "2.5.15";
const TAG = `@biomejs/biome@${PIN}`;
const COMMIT = "0b4c11f553a7e702f9851af139662e1de45dc131";
const GENERATOR = "scripts/generate/lint-config/biome-rules.mjs";
const OUTPUT = "src/engine/catalog/lint-config/generated/biome-rules.generated.ts";
const ROOT = join(import.meta.dirname, "../../..");
/** The analyzers in the order the configuration macro visits their registries. */
const ANALYZERS = ["js", "json", "css", "graphql", "html", "markdown"];

/** Code-unit order, as Rust's `str` ordering for these ASCII names. */
const byText = (a, b) => (a < b ? -1 : a > b ? 1 : 0);

/** The checkout at `biome`, refused unless it is Biome `PIN` at `COMMIT`. */
function checkedCrates(biome) {
  if (biome === undefined) throw new Error(`usage: node ${GENERATOR} <checkout of biomejs/biome at ${TAG}>`);
  const { version } = JSON.parse(readFileSync(join(biome, "packages/@biomejs/biome/package.json"), "utf8"));
  if (version !== PIN) throw new Error(`Biome ${version} found; the data is pinned to Biome ${PIN}`);
  const head = execFileSync("git", ["-C", biome, "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
  if (head !== COMMIT) throw new Error(`the checkout is at ${head}; ${TAG} is ${COMMIT}`);
  return join(biome, "crates");
}

/**
 * `RuleDomain` variant → its config name (`impl Display for RuleDomain` in crates/biome_analyze/src/rule.rs, which
 * the configuration's `domains` keys match).
 */
function domainNames(crates) {
  const source = readFileSync(join(crates, "biome_analyze/src/rule.rs"), "utf8");
  const display = source.slice(source.indexOf("impl Display for RuleDomain"), source.indexOf("impl Ord for RuleDomain"));
  const names = new Map([...display.matchAll(/Self::(\w+)\s*=>\s*fmt\.write_str\("([^"]+)"\)/gu)].map(match => [match[1], match[2]]));
  if (names.get("ReactNative") !== "reactNative") throw new Error("RuleDomain's Display was not read");
  return names;
}

/** The metadata block of the rule file at `path` (`pub <Rule> { ... }` in `declare_lint_rule!`). */
function ruleBody(path) {
  const text = readFileSync(path, "utf8");
  const declaration = text.search(/declare_lint_rule!\s*\{/u);
  if (declaration < 0) throw new Error(`${path} declares no lint rule`);
  const body = text.slice(declaration).match(/\n\s*pub\s+\w+\s*\{([\s\S]*?)\n\s{4}\}/u)?.[1];
  if (body === undefined) throw new Error(`${path}: no rule metadata`);
  if (/^\s*rule_presets:/mu.test(body)) throw new Error(`${path}: rule_presets are not read`);
  return body;
}

/** A rule's `recommended`: false when left out, as `RuleMetadata::new` has it. */
function recommendedOf(path, body) {
  const recommended = body.match(/^\s*recommended:\s*(\w+)\s*,/mu)?.[1] ?? "false";
  if (recommended !== "true" && recommended !== "false") throw new Error(`${path}: recommended is ${recommended}`);
  return recommended === "true";
}

/**
 * One rule file's metadata (`declare_lint_rule!`, crates/biome_analyze/src/rule.rs `RuleMetadata`): its `name`,
 * `recommended` and `domains`.
 */
function ruleMetadata(path, domains) {
  const body = ruleBody(path);
  const name = body.match(/^\s*name:\s*"([^"]+)",/mu)?.[1];
  if (name === undefined) throw new Error(`${path}: no rule name`);
  const listed = body.match(/^\s*domains:\s*&\[([^\]]*)\]/mu)?.[1] ?? "";
  const ruleDomains = [...listed.matchAll(/RuleDomain::(\w+)/gu)].map(match => {
    if (!domains.has(match[1])) throw new Error(`${path}: RuleDomain::${match[1]} has no config name`);
    return domains.get(match[1]);
  });
  return { name, recommended: recommendedOf(path, body), domains: ruleDomains };
}

/**
 * Every lint rule, `<group>/<rule>` → its metadata per analyzer, in visiting order. `declare_group_from_fs!` makes
 * every `.rs` file directly in a group's directory one rule (crates/biome_analyze_macros/src/group_macro.rs).
 */
function lintRules(crates, domains) {
  const rules = new Map();
  for (const analyzer of ANALYZERS) {
    const directory = join(crates, `biome_${analyzer}_analyze/src/lint`);
    if (!existsSync(directory)) throw new Error(`${directory} is missing`);
    for (const group of readdirSync(directory, { withFileTypes: true }).filter(entry => entry.isDirectory()).map(entry => entry.name)) {
      for (const file of readdirSync(join(directory, group)).filter(name => name.endsWith(".rs") && name !== "mod.rs")) {
        const { name, ...metadata } = ruleMetadata(join(directory, group, file), domains);
        const key = `${group}/${name}`;
        rules.set(key, [...rules.get(key) ?? [], metadata]);
      }
    }
  }
  return rules;
}

/** The rule names the generated configuration enum lists (`pub enum RuleName` in crates/biome_configuration/src/analyzer/linter/rules.rs). */
function configurationRuleNames(crates) {
  const source = readFileSync(join(crates, "biome_configuration/src/analyzer/linter/rules.rs"), "utf8");
  const start = source.indexOf("pub enum RuleName {");
  const body = source.slice(start, source.indexOf("}", start));
  return new Set([...body.matchAll(/^\s*(\w+),$/gmu)].map(match => match[1][0].toLowerCase() + match[1].slice(1)));
}

const crates = checkedCrates(process.argv[2]);
const domains = domainNames(crates);
const rules = lintRules(crates, domains);
const configured = configurationRuleNames(crates);
const read = new Set([...rules.keys()].map(key => key.split("/")[1]));
const unmatched = [...configured].filter(name => !read.has(name)).concat([...read].filter(name => !configured.has(name)));
if (unmatched.length > 0) throw new Error(`the rule files and RuleName disagree on ${unmatched.join(", ")}`);

const entry = ({ recommended, domains: ruleDomains }) => `[${recommended}, ${JSON.stringify(ruleDomains).replaceAll(",", ", ")}]`;
writeFileSync(
  join(ROOT, OUTPUT),
  [
    `// Generated by ${GENERATOR}; do not edit. Biome ${PIN} (${TAG}, ${COMMIT}).`,
    "",
    "/**",
    ` * Biome ${PIN}'s lint domains, by the names \`linter.domains\` takes (\`RuleDomain\`'s \`Display\`,`,
    " * `crates/biome_analyze/src/rule.rs`).",
    " */",
    `export const BIOME_DOMAINS: readonly string[] = ${JSON.stringify([...domains.values()].sort(byText)).replaceAll(",", ", ")};`,
    "",
    "/** One analyzer's declaration of a rule: whether it is `recommended`, and its `domains` by config name. */",
    "export type BiomeRuleDeclaration = readonly [recommended: boolean, domains: readonly string[]];",
    "",
    "/**",
    ` * Every Biome ${PIN} lint rule, \`<group>/<rule>\`, with each language analyzer's declaration of it in the order`,
    " * Biome's configuration visits them (JavaScript, JSON, CSS, GraphQL, HTML, Markdown), read from",
    " * `crates/biome_<language>_analyze/src/lint/<group>/<rule>.rs`. The configuration's presets take the last one.",
    " */",
    "export const BIOME_RULES: Readonly<Record<string, readonly BiomeRuleDeclaration[]>> = {",
    ...[...rules.keys()].sort(byText).map(key => `  ${JSON.stringify(key)}: [${rules.get(key).map(entry).join(", ")}],`),
    "};",
    "",
    "/** The upstream version the table comes from. */",
    `export const UPSTREAM_VERSIONS: Readonly<Record<string, string>> = { biome: ${JSON.stringify(PIN)} };`,
    "",
  ].join("\n"),
);
console.log(`${rules.size} rules, ${domains.size} domains`);

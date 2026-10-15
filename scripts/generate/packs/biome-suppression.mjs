// Reads Biome's source at @biomejs/biome@2.5.15 and writes what Rerail needs to honour its `biome-ignore` comments:
//   - native/src/code_rule_dsl/suppress/biome_registry.generated.rs: every diagnostic category (a suppression comment
//     naming any other is refused) and the rules of the JavaScript analyzer's registry (a comment naming another rule
//     or group suppresses nothing), which the engine's port of Biome's suppressions reads;
//   - conformance/biome-same-sources.generated.ts: for each ESLint-family rule name (`no-debugger`,
//     `react-hooks/rules-of-hooks`, `@typescript-eslint/x`), the Biome lint rules whose `sources` mark it `.same()`;
//   - and, from that mapping, `suppress.biome.rules` on every violation of a pack port that carries `suppress.eslint`
//     (packs/<id>/rules and packs/<id>/candidates), edited in place in each document's own formatting; a shipped
//     document (packs/<id>/rules) that changes bumps its revision. A second run changes nothing.
// Run offline, not in CI:
//   git clone --depth 1 --branch @biomejs/biome@2.5.15 https://github.com/biomejs/biome.git <dir>
//   node scripts/generate/packs/biome-suppression.mjs <dir>
// Any other Biome version or commit is refused; bump the pins and regenerate to move.
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { isDeepStrictEqual } from "node:util";
import { join, relative } from "node:path";

const PIN = "2.5.15";
const TAG = `@biomejs/biome@${PIN}`;
const COMMIT = "0b4c11f553a7e702f9851af139662e1de45dc131";
const GENERATOR = "scripts/generate/packs/biome-suppression.mjs";
const ROOT = join(import.meta.dirname, "../../..");

const biome = process.argv[2];
if (biome === undefined) throw new Error(`usage: node ${GENERATOR} <checkout of biomejs/biome at ${TAG}>`);
const { version } = JSON.parse(readFileSync(join(biome, "packages/@biomejs/biome/package.json"), "utf8"));
if (version !== PIN) throw new Error(`Biome ${version} found; the data is pinned to Biome ${PIN}`);
const head = execFileSync("git", ["-C", biome, "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
if (head !== COMMIT) throw new Error(`the checkout is at ${head}; ${TAG} is ${COMMIT}`);
const crates = join(biome, "crates");

/** Code-unit order, as Rust's `str` ordering for these ASCII names. */
const byText = (a, b) => (a < b ? -1 : a > b ? 1 : 0);

// Every category `define_categories!` declares (crates/biome_diagnostics_categories/src/categories.rs): the keys of
// its `"name": "link",` entries and its bare `"name",` entries. `"<category>".parse::<&Category>()` accepts these only.
const categoriesSource = readFileSync(join(crates, "biome_diagnostics_categories/src/categories.rs"), "utf8");
const body = categoriesSource.slice(categoriesSource.indexOf("define_categories! {"));
const categories = [...body.matchAll(/^\s*"([^"]+)"\s*[:,]/gmu)].map(match => match[1]).sort(byText);
if (new Set(categories).size !== categories.length) throw new Error("a category repeats");

// The ESLint-family `RuleSource` variants and the namespace each puts before its rule name (`RuleSource::namespace`
// and `to_namespaced_rule_name` in crates/biome_analyze/src/rule.rs): `Eslint("no-debugger")` is `no-debugger`,
// `EslintReactHooks("rules-of-hooks")` is `react-hooks/rules-of-hooks`.
const ruleSource = readFileSync(join(crates, "biome_analyze/src/rule.rs"), "utf8");
const namespaceBody = ruleSource.slice(ruleSource.indexOf("pub const fn namespace(&self)"), ruleSource.indexOf("pub fn to_namespaced_rule_name"));
const namespaces = new Map();
for (const arm of namespaceBody.matchAll(/((?:\|?\s*Self::\w+(?:\([^)]*\))?\s*)+)=>\s*"([^"]*)"/gu)) {
  for (const variant of arm[1].matchAll(/Self::(\w+)/gu)) namespaces.set(variant[1], arm[2]);
}
const eslintFamily = variant => variant.startsWith("Eslint") || variant === "HtmlEslint";
if (namespaces.get("Eslint") !== "" || namespaces.get("EslintReactHooks") !== "react-hooks") throw new Error("RuleSource::namespace was not read");

/** The text of the bracketed list starting at `open` (an index of `[`). */
function bracketed(text, open) {
  let depth = 0;
  for (let index = open; index < text.length; index += 1) {
    if (text[index] === "[") depth += 1;
    else if (text[index] === "]" && --depth === 0) return text.slice(open + 1, index);
  }
  throw new Error("unclosed list");
}

// The JavaScript analyzer's registry (crates/biome_js_analyze/src/registry.rs records its assist, lint and syntax
// categories; `declare_group_from_fs!` makes every `.rs` file directly in a group's directory one rule,
// crates/biome_analyze_macros/src/group_macro.rs l.77-117): each rule's group, `name`, category and `sources`.
const rules = [];
for (const category of ["assist", "lint", "syntax"]) {
  const directory = join(crates, "biome_js_analyze/src", category);
  for (const group of readdirSync(directory, { withFileTypes: true }).filter(entry => entry.isDirectory()).map(entry => entry.name)) {
    for (const file of readdirSync(join(directory, group)).filter(name => name.endsWith(".rs") && name !== "mod.rs")) {
      const text = readFileSync(join(directory, group, file), "utf8");
      const declaration = text.search(/declare_(?:lint|source|syntax)_rule!\s*\{/u);
      if (declaration < 0) throw new Error(`${category}/${group}/${file} declares no rule`);
      const metadata = text.slice(declaration).match(/\n\s*pub\s+\w+\s*\{([\s\S]*?)\n\s{4}\}/u);
      const name = metadata?.[1].match(/^\s*name:\s*"([^"]+)",/mu)?.[1];
      if (name === undefined) throw new Error(`${category}/${group}/${file}: no rule name`);
      const sourcesAt = metadata[1].search(/sources:\s*&\[/u);
      const sources = sourcesAt < 0 ? "" : bracketed(metadata[1], metadata[1].indexOf("[", sourcesAt));
      const same = [...sources.matchAll(/RuleSource::(\w+)\(\s*"([^"]+)"\s*\)\s*\.\s*(same|inspired)\(\)/gu)]
        .filter(([, variant, , kind]) => kind === "same" && eslintFamily(variant))
        .map(([, variant, rule]) => {
          if (!namespaces.has(variant)) throw new Error(`RuleSource::${variant} has no namespace`);
          const namespace = namespaces.get(variant);
          return namespace === "" ? rule : `${namespace}/${rule}`;
        });
      rules.push({ category, group, name, same });
    }
  }
}
rules.sort((a, b) => byText(a.group, b.group) || byText(a.name, b.name) || byText(a.category, b.category));
for (const [index, rule] of rules.entries()) {
  const previous = rules[index - 1];
  if (previous !== undefined && previous.group === rule.group && previous.name === rule.name) throw new Error(`${rule.group}/${rule.name} repeats`);
  if (rule.category === "lint" && !categories.includes(`lint/${rule.group}/${rule.name}`)) throw new Error(`lint/${rule.group}/${rule.name} is no category`);
}

const rust = [
  `// Generated by ${GENERATOR}; do not edit. Biome ${PIN} (${TAG}, ${COMMIT}).`,
  `//! Biome ${PIN}'s diagnostic categories (\`crates/biome_diagnostics_categories/src/categories.rs\`) and the rules of`,
  "//! its JavaScript analyzer (`crates/biome_js_analyze/src/{assist,lint,syntax}/<group>/<rule>.rs`), sorted.",
  "",
  "/// Every diagnostic category: a suppression comment's category must be one of them (or",
  "/// `lint/plugin/<name>`).",
  "pub(super) const CATEGORIES: &[&str] = &[",
  ...categories.map(name => `    ${JSON.stringify(name)},`),
  "];",
  "",
  "/// The JavaScript analyzer's `MetadataRegistry`: each rule's group, name and category (`assist`,",
  "/// `lint` or `syntax`), sorted by group and name.",
  "pub(super) const JS_RULES: &[(&str, &str, &str)] = &[",
  ...rules.map(rule => `    (${JSON.stringify(rule.group)}, ${JSON.stringify(rule.name)}, ${JSON.stringify(rule.category)}),`),
  "];",
  "",
].join("\n");
writeFileSync(join(ROOT, "native/src/code_rule_dsl/suppress/biome_registry.generated.rs"), rust);

/** ESLint-family rule name → the Biome lint rules (`lint/<group>/<rule>`) whose sources mark it `.same()`. */
const sameSources = new Map();
for (const rule of rules.filter(entry => entry.category === "lint")) {
  for (const eslint of rule.same) {
    const names = sameSources.get(eslint) ?? [];
    names.push(`lint/${rule.group}/${rule.name}`);
    sameSources.set(eslint, names);
  }
}
const mapping = Object.fromEntries([...sameSources.keys()].sort(byText).map(eslint => [eslint, [...new Set(sameSources.get(eslint))].sort(byText)]));
writeFileSync(
  join(ROOT, "conformance/biome-same-sources.generated.ts"),
  [
    `// Generated by ${GENERATOR}; do not edit. Biome ${PIN} (${TAG}, ${COMMIT}).`,
    "",
    "/**",
    ` * For each ESLint-family rule name, the Biome ${PIN} lint rules (\`lint/<group>/<rule>\`) whose \`sources\` mark it the same`,
    " * rule (`.same()`, not `.inspired()`), read from `crates/biome_js_analyze/src/lint/<group>/<rule>.rs`.",
    " */",
    "export const BIOME_SAME_SOURCES: Readonly<Record<string, readonly string[]>> = {",
    ...Object.entries(mapping).map(([eslint, names]) => `  ${JSON.stringify(eslint)}: ${JSON.stringify(names).replaceAll(",", ", ")},`),
    "};",
    "",
  ].join("\n"),
);

// ---- The pack documents, edited as text so each keeps its formatting. ----

/** A JSON value with its byte offsets: objects keep their members' key offsets, arrays their items. */
function parseJson(text) {
  let at = 0;
  const space = () => {
    while (at < text.length && /\s/u.test(text[at])) at += 1;
  };
  const string = () => {
    const start = at;
    at += 1;
    while (text[at] !== '"') at += text[at] === "\\" ? 2 : 1;
    at += 1;
    return { type: "string", start, end: at, value: JSON.parse(text.slice(start, at)) };
  };
  const value = () => {
    space();
    const start = at;
    if (text[at] === "{") {
      at += 1;
      const members = [];
      space();
      if (text[at] === "}") at += 1;
      else
        for (;;) {
          space();
          const key = string();
          space();
          at += 1; // :
          members.push({ key: key.value, keyStart: key.start, value: value() });
          space();
          if (text[at++] === "}") break;
        }
      return { type: "object", start, end: at, members };
    }
    if (text[at] === "[") {
      at += 1;
      const items = [];
      space();
      if (text[at] === "]") at += 1;
      else
        for (;;) {
          items.push(value());
          space();
          if (text[at++] === "]") break;
        }
      return { type: "array", start, end: at, items };
    }
    if (text[at] === '"') return string();
    const literal = text.slice(at).match(/^(?:true|false|null|-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)/u)[0];
    at += literal.length;
    return { type: "literal", start, end: at };
  };
  return value();
}

const member = (object, key) => object.members.find(entry => entry.key === key);

/**
 * The text of a `"biome": { "rules": [...] }` member written as `template` (the `"eslint"` member) is written: its key
 * renamed and its rules list holding `names`, its items separated as the template's would be.
 */
function biomeMember(text, template, names) {
  const value = template.value;
  const list = member(value, "rules").value;
  const before = text.slice(list.start + 1, list.items[0].start);
  const after = text.slice(list.items.at(-1).end, list.end - 1);
  const separator = list.items.length > 1 ? text.slice(list.items[0].end, list.items[1].start) : before.includes("\n") ? `,${before}` : ", ";
  const items = `${before}${names.map(name => JSON.stringify(name)).join(separator)}${after}`;
  return `"biome"${text.slice(template.keyStart + '"eslint"'.length, list.start + 1)}${items}${text.slice(list.end - 1, value.end)}`;
}

/** The documents under `directory`, recursively. */
function jsonFiles(directory) {
  if (!existsSync(directory)) return [];
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry =>
    entry.isDirectory() ? jsonFiles(join(directory, entry.name)) : entry.name.endsWith(".json") ? [join(directory, entry.name)] : [],
  );
}

const summary = new Map();
for (const pack of readdirSync(join(ROOT, "packs"), { withFileTypes: true }).filter(entry => entry.isDirectory()).map(entry => entry.name)) {
  for (const [kind, directory] of [["shipped", join(ROOT, "packs", pack, "rules")], ["candidate", join(ROOT, "packs", pack, "candidates")]]) {
    for (const path of jsonFiles(directory)) {
      const original = readFileSync(path, "utf8");
      const document = JSON.parse(original);
      if (document?.format !== "rerail.policy/v1" || !Array.isArray(document.clauses)) continue;
      const root = parseJson(original);
      // Edits as [start, end, replacement], applied from the end.
      const edits = [];
      let violations = 0;
      for (const clause of member(root, "clauses").value.items) {
        const suppressMember = member(member(clause, "violation")?.value ?? { members: [] }, "suppress");
        if (suppressMember === undefined) continue;
        const suppress = suppressMember.value;
        const eslint = member(suppress, "eslint");
        const existing = member(suppress, "biome");
        const wanted = eslint === undefined ? [] : [...new Set(JSON.parse(original.slice(eslint.value.start, eslint.value.end)).rules.flatMap(name => mapping[name] ?? []))].sort(byText);
        const current = existing === undefined ? [] : JSON.parse(original.slice(existing.value.start, existing.value.end)).rules;
        if (isDeepStrictEqual(current, wanted)) continue;
        violations += 1;
        if (existing !== undefined) {
          // Replace or drop the stale member, with the separator before it.
          const previous = suppress.members[suppress.members.indexOf(existing) - 1];
          if (previous === undefined) throw new Error(`${relative(ROOT, path)}: suppress.biome comes first`);
          const separator = original.slice(previous.value.end, existing.keyStart);
          edits.push([previous.value.end, existing.value.end, wanted.length === 0 ? "" : `${separator}${biomeMember(original, eslint, wanted)}`]);
        } else {
          // After `oxlint` (or `eslint`), separated as the members before it are (or as `{` is from its first member).
          const after = member(suppress, "oxlint") ?? eslint;
          const index = suppress.members.indexOf(after);
          const separator =
            index > 0
              ? original.slice(suppress.members[index - 1].value.end, after.keyStart)
              : suppress.members.length > 1
                ? original.slice(suppress.members[0].value.end, suppress.members[1].keyStart)
                : `,${original.slice(suppress.start + 1, suppress.members[0].keyStart).includes("\n") ? original.slice(suppress.start + 1, suppress.members[0].keyStart) : " "}`;
          edits.push([after.value.end, after.value.end, `${separator}${biomeMember(original, eslint, wanted)}`]);
        }
      }
      if (edits.length === 0) continue;
      if (kind === "shipped") {
        const revision = member(root, "revision").value;
        edits.push([revision.start, revision.end, String(Number(original.slice(revision.start, revision.end)) + 1)]);
      }
      let text = original;
      for (const [start, end, replacement] of edits.sort((a, b) => b[0] - a[0])) text = text.slice(0, start) + replacement + text.slice(end);
      // The edit changes nothing but `suppress.biome` (and the revision).
      const expected = structuredClone(document);
      if (kind === "shipped") expected.revision += 1;
      for (const clause of expected.clauses) {
        const suppress = clause.violation?.suppress;
        if (suppress === undefined) continue;
        const wanted = [...new Set((suppress.eslint?.rules ?? []).flatMap(name => mapping[name] ?? []))].sort(byText);
        if (wanted.length === 0) delete suppress.biome;
        else suppress.biome = { rules: wanted };
      }
      const edited = JSON.parse(text);
      if (!isDeepStrictEqual(edited, expected)) throw new Error(`${relative(ROOT, path)}: the edit changed more than suppress.biome`);
      writeFileSync(path, text);
      const counts = summary.get(pack) ?? { shipped: 0, candidate: 0, violations: 0 };
      counts[kind] += 1;
      counts.violations += violations;
      summary.set(pack, counts);
    }
  }
}
for (const [pack, counts] of summary) console.log(`${pack}: ${counts.shipped} shipped and ${counts.candidate} candidate documents, ${counts.violations} violations`);

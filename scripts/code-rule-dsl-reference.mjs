#!/usr/bin/env node
// Generator and checker for docs/code-rule-dsl-reference.html, the code-rule DSL reference.
//
//   node scripts/code-rule-dsl-reference.mjs fill
//     Refills every example figure of the reference: `<figure class="ex-fig" data-src="repo:PATH#PTR">`
//     takes its JSON from the committed document in this repository, so the reference quotes it
//     exactly; a figure without `data-src` keeps its own JSON. Both are reformatted to one width.
//
//   node scripts/code-rule-dsl-reference.mjs figures
//     Redraws the relations figure between `<!-- figure:relations:begin -->` and `…:end -->`.
//
//   node scripts/code-rule-dsl-reference.mjs check
//     Checks every `data-zod` annotation names an existing zod object, that each grammar
//     production lists exactly that object's keys, and that a key the schema reads with
//     `z.enum(...)` lists exactly its values as literals. `grammar` is the same check; reads only.
//     `npm run check:reference` runs it in `check:all`.
//
//   node scripts/code-rule-dsl-reference.mjs coverage [--ruff REF|DIR] [--eslint REF|DIR] [--write]
//     Counts, per suite, the committed code-rule documents that use each DSL construct. Prints the
//     table; with --write, regenerates the reference's coverage heatmap between its
//     `<!-- coverage:begin -->` and `<!-- coverage:end -->` markers.
//
//   node --import tsx scripts/code-rule-dsl-reference.mjs examples [--engine PATH] [--verbose]
//     Compiles every example of the reference with the native engine (`CodeRuleDslRuntime.compile`),
//     wrapping rule, query, clause and define examples into documents. PATH is a built engine:
//     `native/*.node`, or a debug build (`target/debug/librerail_native.dylib`), which Node
//     loads the same way; without it, the addon the package loads (`resolveNativeBindingPath`).
//     Examples marked `data-kind="fragment"` are fragments on purpose. Reads only;
//     `npm run test:reference` runs it in `test:all`, after the engine is built.
//
//   node scripts/code-rule-dsl-reference.mjs locate FEATURE [--suite ID] [--whole clause] [--limit N]
//     Prints the smallest committed uses of a feature (see `features`), with files and JSON pointers:
//     the search behind the reference's examples. `features` lists every feature with its count;
//     `show SUITE:PATH[#PTR]` prints a document's clauses, or the value at a pointer.
//
// The TSLint pack's candidates and shipped rules (the bundled code rules) and the engine's worked examples are read
// from this checkout.
// Ruff and ESLint conformance ports are not part of this repository; `--ruff` and `--eslint` add
// them as a directory of ports, or as a git ref of this repository that has them under
// `conformance/<suite>/rules`. Regenerate in this order: fill, figures, coverage --write; then
// check with grammar and examples.

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const RERAIL = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const REFERENCE = join(RERAIL, "docs/code-rule-dsl-reference.html");
const CAPABILITIES = join(RERAIL, "code-rule-dsl/capabilities.json");
// The rule document format.
const FORMAT = "rerail.policy/v1";
const CLAUSE_KINDS = ["forbid", "require_each", "budget", "llm", "define"];

function args(argv) {
  const out = { _: [] };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg.startsWith("--")) {
      const key = arg.slice(2);
      const next = argv[i + 1];
      if (next === undefined || next.startsWith("--")) out[key] = true;
      else {
        out[key] = next;
        i += 1;
      }
    } else out._.push(arg);
  }
  return out;
}

function* jsonFiles(dir) {
  if (!existsSync(dir)) return;
  for (const name of readdirSync(dir).sort()) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) yield* jsonFiles(path);
    else if (name.endsWith(".json")) yield path;
  }
}

const git = (...argv) => execFileSync("git", argv, { cwd: RERAIL, maxBuffer: 1 << 30 }); // NOSONAR - a developer script: runs the developer's own git from PATH

/** `[path, text]` for every JSON file under `path` at a git ref, read in one `cat-file --batch`. */
function gitJsonFiles(ref, path) {
  const names = git("ls-tree", "-r", "-z", "--name-only", ref, "--", path)
    .toString("utf8")
    .split("\0")
    .filter(name => name.endsWith(".json"))
    .sort();
  if (names.length === 0) return [];
  const input = names.map(name => `${ref}:${name}`).join("\n") + "\n";
  const out = execFileSync("git", ["cat-file", "--batch"], { cwd: RERAIL, input, maxBuffer: 1 << 30 }); // NOSONAR - a developer script: runs the developer's own git from PATH
  const files = [];
  let offset = 0;
  for (const name of names) {
    const newline = out.indexOf(10, offset);
    const size = Number(out.subarray(offset, newline).toString("utf8").split(" ")[2]);
    files.push([name, out.subarray(newline + 1, newline + 1 + size).toString("utf8")]);
    offset = newline + 1 + size + 1;
  }
  return files;
}

/**
 * The committed code-rule documents of a suite: JSON files whose `format` is the DSL's.
 */
function documents(suite) {
  const files =
    suite.ref === undefined ? [...jsonFiles(suite.dir)].map(path => [path, readFileSync(path, "utf8")]) : gitJsonFiles(suite.ref, suite.dir);
  const docs = [];
  for (const [path, text] of files) {
    let doc;
    try {
      doc = JSON.parse(text);
    } catch {
      continue;
    }
    if (isObject(doc) && doc.format === FORMAT) docs.push({ path, doc });
  }
  return docs;
}

/**
 * An optional external suite: absent without `override`; read from `override` when it is a directory,
 * otherwise from `conformance/<id>/rules` at the git ref `override` names.
 */
function source(id, label, override) {
  if (typeof override !== "string") return undefined;
  if (existsSync(override)) return { id, label, dir: resolve(override) };
  return { id, label, ref: override, dir: `conformance/${id}/rules` };
}

function suites(options) {
  const list = [
    source("ruff", "Ruff", options.ruff),
    source("eslint", "ESLint", options.eslint),
    { id: "tslint", label: "TSLint", dir: join(RERAIL, "packs/tslint/candidates") },
    { id: "bundled", label: "Bundled", dir: join(RERAIL, "packs/tslint/rules") },
    { id: "worked", label: "Worked", dir: join(RERAIL, "native/src/code_rule_dsl/compile/examples") },
  ].filter(suite => suite !== undefined);
  for (const suite of list) {
    if (suite.ref !== undefined) suite.commit = git("rev-parse", "--short=9", suite.ref).toString("utf8").trim();
  }
  return list;
}

// ---------------------------------------------------------------------------------------------
// Feature extraction: a walk over a document that records every construct it uses, with the
// JSON pointer of the rule object (or other owner) where it appears.

const RELATIONS = ["inside", "has", "precedes", "follows"];

class Walk {
  constructor() {
    this.features = new Map(); // feature -> [{ pointer, owner }]
    this.patterns = [];
    this.regexes = [];
    this.transformSources = [];
  }
  add(feature, pointer, owner) {
    if (!this.features.has(feature)) this.features.set(feature, []);
    this.features.get(feature).push({ pointer, owner });
  }
}

const esc = token => String(token).replace(/~/g, "~0").replace(/\//g, "~1");
const at = (pointer, ...keys) => pointer + keys.map(key => `/${esc(key)}`).join("");
const isObject = value => value !== null && typeof value === "object" && !Array.isArray(value);

function walkOptions(walk, value, pointer) {
  if (Array.isArray(value)) value.forEach((item, index) => walkOptions(walk, item, at(pointer, index)));
  else if (isObject(value)) {
    if ("$option" in value) {
      walk.add("option.ref", pointer, value);
      for (const key of ["cases", "escape", "map", "join", "template", "spread"]) if (key in value) walk.add(`option.ref.${key}`, pointer, value);
    }
    for (const [key, child] of Object.entries(value)) walkOptions(walk, child, at(pointer, key));
  }
}

function walkFilter(walk, filter, pointer, prefix) {
  if (!isObject(filter)) return;
  for (const key of Object.keys(filter)) walk.add(`${prefix}.${key}`, pointer, filter);
  for (const key of ["site", "value", "annotation", "occurrence"]) if (isObject(filter[key])) walkRule(walk, filter[key], at(pointer, key));
}

function walkPattern(walk, rule, pointer) {
  if (rule.pattern === undefined) return;
  walk.add(isObject(rule.pattern) ? "rule.pattern.context" : "rule.pattern", pointer, rule);
  if (typeof rule.pattern === "string") walk.patterns.push(rule.pattern);
  else if (isObject(rule.pattern)) walk.patterns.push(String(rule.pattern.context));
}

function walkRuleChildren(walk, rule, pointer, grammar) {
  for (const key of ["all", "any"]) {
    if (!Array.isArray(rule[key])) continue;
    walk.add(`rule.${key}`, pointer, rule);
    rule[key].forEach((item, index) => walkRule(walk, item, at(pointer, key, index), grammar));
  }
  if (rule.not !== undefined) {
    walk.add("rule.not", pointer, rule);
    walkRule(walk, rule.not, at(pointer, "not"), grammar);
  }
}

function walkRuleRelations(walk, rule, pointer, grammar) {
  for (const key of RELATIONS) {
    const relation = rule[key];
    if (!isObject(relation)) continue;
    walk.add(`rule.${key}`, pointer, rule);
    walkRelation(walk, relation, at(pointer, key), grammar);
  }
  if (!isObject(rule.count)) return;
  walk.add("rule.count", pointer, rule);
  for (const key of RELATIONS) {
    if (!isObject(rule.count[key])) continue;
    walk.add(`count.${key}`, pointer, rule);
    walkRelation(walk, rule.count[key], at(pointer, "count", key), grammar);
  }
}

function walkResolution(walk, rule, pointer) {
  if (!isObject(rule.resolves)) return;
  walk.add("rule.resolves", pointer, rule);
  for (const [key, value] of Object.entries(rule.resolves)) {
    if (key === "global" || key === "import") walk.add(`resolves.${key}${value === true ? ".any" : ""}`, pointer, rule);
    else if (key === "unresolved") walk.add(`resolves.unresolved${value === "star" ? ".star" : ""}`, pointer, rule);
    else if (key === "this") walk.add(`resolves.this.${value}`, pointer, rule);
    else if (key !== "binding") walk.add(`resolves.${key}`, pointer, rule);
  }
  if (isObject(rule.resolves.binding)) {
    walk.add("resolves.binding", pointer, rule);
    walkFilter(walk, rule.resolves.binding, at(pointer, "resolves", "binding"), "binding");
  }
}

function walkValueAndArguments(walk, rule, pointer) {
  if (isObject(rule.value)) {
    walk.add("rule.value", pointer, rule);
    for (const key of Object.keys(rule.value)) walk.add(`value.${key}`, pointer, rule);
  }
  if (!isObject(rule.arguments)) return;
  walk.add("rule.arguments", pointer, rule);
  for (const key of ["bind", "absent"]) if (rule.arguments[key] !== undefined) walk.add(`arguments.${key}`, pointer, rule);
  walk.add(`arguments.mode.${rule.arguments.mode ?? "language"}`, pointer, rule);
  if (rule.arguments.signature?.some?.(entry => entry === "/" || entry === "*")) walk.add("arguments.markers", pointer, rule);
}

function walkBindsAndParse(walk, rule, pointer, grammar) {
  if (isObject(rule.binds)) {
    walk.add("rule.binds", pointer, rule);
    walkFilter(walk, rule.binds, at(pointer, "binds"), "binds");
    if (isObject(rule.binds.redefines)) walkFilter(walk, rule.binds.redefines, at(pointer, "binds", "redefines"), "redefines");
  }
  if (isObject(rule.parse)) {
    walk.add("rule.parse", pointer, rule);
    walk.add(`parse.${rule.parse.grammar}`, pointer, rule);
    walkRule(walk, rule.parse, at(pointer, "parse"), rule.parse.grammar);
  }
  if (rule.capture !== undefined) walk.add("rule.capture", pointer, rule);
  if (isObject(rule.host)) {
    walk.add("rule.host", pointer, rule);
    walkRule(walk, rule.host, at(pointer, "host"));
  }
}

function walkFormatArguments(walk, rule, pointer, grammar) {
  if (!isObject(rule.formatArgs)) return;
  walk.add("rule.formatArgs", pointer, rule);
  walk.add(`formatArgs.${rule.formatArgs.style}`, pointer, rule);
  for (const key of ["unused", "missing"]) {
    if (!isObject(rule.formatArgs[key])) continue;
    walk.add(`formatArgs.${key}`, pointer, rule);
    walkRule(walk, rule.formatArgs[key], at(pointer, "formatArgs", key), key === "missing" ? "format" : grammar);
  }
}

function walkRuleOptions(walk, rule, pointer, grammar) {
  walkValueAndArguments(walk, rule, pointer);
  walkBindsAndParse(walk, rule, pointer, grammar);
  walkFormatArguments(walk, rule, pointer, grammar);
}

function walkLayout(walk, rule, pointer) {
  if (!isObject(rule.layout)) return;
  walk.add("rule.layout", pointer, rule);
  for (const key of ["span", "start", "before", "after"]) {
    const value = rule.layout[key];
    if (value === undefined) continue;
    walk.add(`layout.${key}`, pointer, rule);
    if (!isObject(value) || typeof value.to !== "string") continue;
    walk.add(`layout.to.${value.to.startsWith("capture.") ? "capture" : value.to}`, pointer, rule);
    for (const measure of ["breaks", "blank", "comments", "edge"]) if (value[measure] !== undefined) walk.add(`layout.${measure}`, pointer, rule);
  }
}

function walkNthChild(walk, rule, pointer, grammar, add) {
  if (rule.nthChild === undefined) return;
  add("rule.nthChild");
  if (!isObject(rule.nthChild)) return;
  if (rule.nthChild.ofRule !== undefined) add("nthChild.ofRule");
  if (rule.nthChild.reverse !== undefined) add("nthChild.reverse");
  walkRule(walk, rule.nthChild.ofRule, at(pointer, "nthChild", "ofRule"), grammar);
}

function walkRuleBasics(walk, rule, pointer, grammar) {
  const add = feature => walk.add(feature, pointer, rule);
  walkPattern(walk, rule, pointer);
  if (rule.kind !== undefined) add("rule.kind");
  if (rule.regex !== undefined) {
    add("rule.regex");
    if (typeof rule.regex === "string") walk.regexes.push(rule.regex);
  }
  walkNthChild(walk, rule, pointer, grammar, add);
  walkRuleChildren(walk, rule, pointer, grammar);
  if (rule.matches !== undefined) add(isObject(rule.matches) ? "matches.call" : "rule.matches");
}

function walkRuleComparison(walk, rule, pointer) {
  if (!isObject(rule.compare)) return;
  walk.add("rule.compare", pointer, rule);
  walk.add(`compare.text.${rule.compare.text ?? "source"}`, pointer, rule);
  if (rule.compare.collation !== undefined) walk.add(`compare.collation.${rule.compare.collation}`, pointer, rule);
  if (rule.compare.commute !== undefined) walk.add("compare.commute", pointer, rule);
}

function walkRule(walk, rule, pointer, grammar) {
  if (!isObject(rule)) return;
  walkRuleBasics(walk, rule, pointer, grammar);
  walkRuleRelations(walk, rule, pointer, grammar);
  walkRuleComparison(walk, rule, pointer);
  walkResolution(walk, rule, pointer);
  walkRuleOptions(walk, rule, pointer, grammar);
  walkLayout(walk, rule, pointer);
}

function walkRelation(walk, relation, pointer, grammar) {
  if (relation.stopBy !== undefined) {
    const stop = isObject(relation.stopBy) ? "rule" : relation.stopBy;
    walk.add(`stopBy.${stop}`, pointer, relation);
    if (isObject(relation.stopBy)) walkRule(walk, relation.stopBy, at(pointer, "stopBy"), grammar);
  }
  if (relation.field !== undefined) walk.add("relation.field", pointer, relation);
  walkRule(walk, relation, pointer, grammar);
}

function walkUtility(walk, util, pointer) {
  if (isObject(util) && Array.isArray(util.params) && isObject(util.rule)) {
    walk.add("utils.params", pointer, util);
    walkRule(walk, util.rule, at(pointer, "rule"));
  } else walkRule(walk, util, pointer);
}

function walkAstRules(walk, ast, pointer) {
  if (isObject(ast.utils)) {
    walk.add("ast.utils", pointer, ast);
    for (const [id, util] of Object.entries(ast.utils)) walkUtility(walk, util, at(pointer, "utils", id));
  }
  if (isObject(ast.constraints)) {
    walk.add("ast.constraints", pointer, ast);
    for (const [name, rule] of Object.entries(ast.constraints)) walkRule(walk, rule, at(pointer, "constraints", name));
  }
}

function walkAstProperties(walk, ast, pointer) {
  for (const [property, feature] of [["languageOptions", "languageOptions"], ["when", "when"], ["file", "file"]]) {
    if (!isObject(ast[property])) continue;
    walk.add(`ast.${feature}`, pointer, ast);
    for (const key of Object.keys(ast[property])) walk.add(`${feature}.${key}`, pointer, ast);
  }
}

function walkTransformBody(walk, body, pointer, transformation, transforms) {
  if (!isObject(body)) return;
  if (body.default !== undefined) walk.add("transform.default", pointer, transformation);
  if (body.value === true) walk.add("transform.value", pointer, transformation);
  if (typeof body.source === "string") walk.transformSources.push({ name: body.source.replace(/^\$+/u, ""), pointer, transformation, transforms });
}

function walkAstTransforms(walk, ast, pointer) {
  if (!isObject(ast.transform)) return;
  walk.add("ast.transform", pointer, ast);
  for (const [name, transformation] of Object.entries(ast.transform)) {
    const transformPointer = at(pointer, "transform", name);
    for (const [kind, body] of Object.entries(transformation ?? {})) {
      walk.add(`transform.${kind}`, transformPointer, transformation);
      walkTransformBody(walk, body, transformPointer, transformation, ast.transform);
    }
  }
}

function walkAst(walk, ast, pointer) {
  walk.add(`language.${ast.language}`, pointer, ast);
  walkRule(walk, ast.rule, at(pointer, "rule"));
  walkAstRules(walk, ast, pointer);
  walkAstProperties(walk, ast, pointer);
  walkAstTransforms(walk, ast, pointer);
}

function walkQuery(walk, query, pointer) {
  if (!isObject(query)) return;
  if (isObject(query.regex)) walk.add(`query.regex.${query.regex.source}`, pointer, query);
  if (isObject(query.ast)) {
    walk.add("query.ast", pointer, query);
    walkAst(walk, query.ast, at(pointer, "ast"));
  }
  if (isObject(query.fact)) walk.add("query.fact", pointer, query);
  if (isObject(query.select_when)) {
    walk.add("query.select_when", pointer, query);
    walkQuery(walk, query.select_when.select, at(pointer, "select_when", "select"));
    walkQuery(walk, query.select_when.exists, at(pointer, "select_when", "exists"));
  }
}

function walkSuppression(walk, violation, pointer) {
  if (!isObject(violation.suppress)) return;
  if (violation.suppress.noqa !== undefined) walk.add("suppress.noqa", pointer, violation);
  if (violation.suppress.noqa?.parent !== undefined) walk.add("suppress.noqa.parent", pointer, violation);
  if (violation.suppress.eslint !== undefined) walk.add("suppress.eslint", pointer, violation);
}

function walkViolation(walk, violation, pointer) {
  if (!isObject(violation)) return;
  const anchor = String(violation.anchor ?? "");
  walk.add(`anchor.${anchor.startsWith("capture.") ? "capture" : anchor}`, pointer, violation);
  const message = String(violation.message ?? "");
  if (/(?<!\\)\$(?:\$\$)?[A-Z_][A-Z0-9_]*/u.test(message)) walk.add("message.var", pointer, violation);
  if (/(?<!\\)\$(?:\$\$)?\{[A-Z0-9_]+\}/u.test(message)) walk.add("message.braced", pointer, violation);
  if (violation.each !== undefined) walk.add("violation.each", pointer, violation);
  if (typeof violation.fix === "string") walk.add("fix.string", pointer, violation);
  if (isObject(violation.fix)) walk.add(violation.fix.range === "witness.span" ? "fix.object" : "fix.range", pointer, violation);
  walkSuppression(walk, violation, pointer);
}

function extract(doc) {
  const walk = new Walk();
  walkOptions(walk, doc, "");
  if (isObject(doc.scope)) walk.add("doc.scope", "", doc);
  (doc.clauses ?? []).forEach((clause, index) => {
    const pointer = at("", "clauses", index);
    for (const form of CLAUSE_KINDS) if (clause[form] !== undefined) walk.add(`clause.${form}`, pointer, clause);
    walkQuery(walk, clause.forbid, at(pointer, "forbid"));
    walkViolation(walk, clause.violation, at(pointer, "violation"));
    if (isObject(clause.define)) {
      const define = clause.define;
      for (const key of ["utils", "options", "config"]) if (define[key] !== undefined) walk.add(`define.${key}`, at(pointer, "define"), define);
      for (const [id, util] of Object.entries(define.utils ?? {})) walkUtility(walk, util, at(pointer, "define", "utils", id));
    }
  });
  // A transform whose source is a named regex group of the witness: no pattern binds `$NAME`,
  // another transform does not produce it, and some regex of the document names the group.
  const patternText = walk.patterns.join("\n");
  for (const source of walk.transformSources) {
    const boundByPattern = new RegExp(`\\$(?:\\$\\$)?${source.name}(?![A-Z0-9_])`, "u").test(patternText);
    const isTransform = Object.hasOwn(source.transforms, source.name);
    const isGroup = walk.regexes.some(regex => regex.includes(`(?<${source.name}>`) || regex.includes(`(?P<${source.name}>`));
    if (!boundByPattern && !isTransform && isGroup) walk.add("transform.groupSource", source.pointer, source.transformation);
  }
  return walk.features;
}

// ---------------------------------------------------------------------------------------------
// The constructs the reference documents, with the features that count as a use of each. The
// order and grouping are the reference's; `id` is the construct card's anchor.

const any = (...features) => ({ features });
export const CONSTRUCTS = [
  ["Document", [
    ["clause-forms", "forbid", any("clause.forbid")],
    ["clause-forms", "require_each", any("clause.require_each")],
    ["clause-forms", "budget", any("clause.budget")],
    ["define", "define", any("clause.define")],
    ["queries", "regex query", any("query.regex.file.content", "query.regex.file.path")],
    ["queries", "fact query", any("query.fact")],
    ["queries", "select_when", any("query.select_when")],
    ["queries", "ast query", any("query.ast")],
    ["languages", "python", any("language.python")],
    ["languages", "typescript", any("language.typescript")],
    ["scope", "scope", any("doc.scope")],
  ]],
  ["Atomic", [
    ["kind", "kind", any("rule.kind")],
    ["pattern", "pattern", any("rule.pattern")],
    ["pattern", "pattern context/selector", any("rule.pattern.context")],
    ["regex", "regex", any("rule.regex")],
    ["value", "value", any("rule.value")],
    ["value", "value.follow", any("value.follow")],
    ["nthChild", "nthChild", any("rule.nthChild")],
    ["nthChild", "nthChild.ofRule", any("nthChild.ofRule")],
    ["layout", "layout", any("rule.layout")],
    ["layout", "layout.before/after", any("layout.before", "layout.after")],
    ["layout", "layout.span/start", any("layout.span", "layout.start")],
  ]],
  ["Relations", [
    ["relations", "has", any("rule.has")],
    ["relations", "inside", any("rule.inside")],
    ["relations", "precedes", any("rule.precedes")],
    ["relations", "follows", any("rule.follows")],
    ["stopBy", "stopBy: end", any("stopBy.end")],
    ["stopBy", "stopBy: neighbor", any("stopBy.neighbor")],
    ["stopBy", "stopBy: rule", any("stopBy.rule")],
    ["field", "field", any("relation.field")],
  ]],
  ["Combinators", [
    ["combinators", "all", any("rule.all")],
    ["combinators", "any", any("rule.any")],
    ["combinators", "not", any("rule.not")],
  ]],
  ["Reuse", [
    ["matches", "matches", any("rule.matches")],
    ["matches", "matches call", any("matches.call")],
    ["utils", "utils (query)", any("ast.utils")],
    ["define", "define.utils", any("define.utils")],
    ["params", "params", any("utils.params")],
    ["options", "define.options", any("define.options")],
    ["options", "$option", any("option.ref")],
    ["options", "when.option", any("when.option")],
    ["constraints", "constraints", any("ast.constraints")],
  ]],
  ["compare", [
    ["compare", "compare", any("rule.compare")],
    ...["source", "unquoted", "value", "key", "member", "name", "reference", "reference-no-static-keys", "tokens", "structure", "structure-hash", "structure-parts", "node", "index"].map(text => [`compare-${text}`, `text: ${text}`, any(`compare.text.${text}`)]),
    ["compare-tokens", "commute", any("compare.commute")],
    ["collations", "collation (non-ordinal)", any("compare.collation.fold-lower", "compare.collation.fold-upper", "compare.collation.lowercase-first", "compare.collation.natural", "compare.collation.natural-fold-lower")],
  ]],
  ["count", [["count", "count", any("rule.count")]]],
  ["resolves", [
    ["resolves", "resolves", any("rule.resolves")],
    ["resolves-global", "global: [names]", any("resolves.global")],
    ["resolves-global", "global: true", any("resolves.global.any")],
    ["resolves-import", "import: [names]", any("resolves.import")],
    ["resolves-import", "import: true", any("resolves.import.any")],
    ["resolves-imported", "imported", any("resolves.imported")],
    ["resolves-imported", "prefix", any("resolves.prefix")],
    ["resolves-imported", "relative", any("resolves.relative")],
    ["resolves-imported", "seenBefore", any("resolves.seenBefore")],
    ["resolves-this", "this", any("resolves.this.global", "resolves.this.globalScope")],
    ["resolves-this", "sloppyFunctions", any("resolves.sloppyFunctions")],
    ["resolves-binding", "binding", any("resolves.binding")],
    ["resolves-binding", "binding.site/value/annotation", any("binding.site", "binding.value", "binding.annotation")],
    ["resolves-binding", "binding.only", any("binding.only")],
    ["resolves-binding", "binding.stopAtGlobal", any("binding.stopAtGlobal")],
    ["resolves-binding", "binding.destructuredValue", any("binding.destructuredValue")],
    ["resolves-unresolved", "unresolved", any("resolves.unresolved", "resolves.unresolved.star")],
    ["resolves-misc", "beforeLocal", any("resolves.beforeLocal")],
    ["resolves-misc", "write: last", any("resolves.write")],
    ["resolves-global", "declared", any("resolves.declared")],
    ["resolves-global", "directives", any("resolves.directives")],
    ["resolves-global", "scriptDeclarations", any("resolves.scriptDeclarations")],
  ]],
  ["binds", [
    ["binds", "binds", any("rule.binds")],
    ["binds", "form", any("binds.form", "binding.form", "redefines.form")],
    ["binds", "scope", any("binds.scope", "binding.scope", "redefines.scope")],
    ["binds", "used / referenced", any("binds.used", "binding.used", "redefines.used", "binds.referenced", "binding.referenced", "redefines.referenced")],
    ["binds", "exported", any("binds.exported", "binding.exported", "redefines.exported")],
    ["binds", "latest / outer", any("binds.latest", "binding.latest", "binds.outer", "binding.outer", "redefines.latest", "redefines.outer")],
    ["binds", "nestedUse", any("binds.nestedUse", "binding.nestedUse", "redefines.nestedUse")],
    ["binds", "redefines", any("binds.redefines")],
    ["binds", "bindings (count)", any("binds.bindings", "binding.bindings", "redefines.bindings")],
    ["binds", "occurrence", any("binds.occurrence", "binding.occurrence")],
    ["binds", "builtin", any("binds.builtin", "binding.builtin", "redefines.builtin")],
  ]],
  ["arguments", [
    ["arguments", "arguments", any("rule.arguments")],
    ["arguments-syntactic", "mode: syntactic", any("arguments.mode.syntactic")],
    ["arguments-estree", "mode: estree", any("arguments.mode.estree")],
  ]],
  ["Sub-grammars", [
    ["parse", "parse: percentFormat", any("parse.percentFormat")],
    ["parse", "parse: braceFormat", any("parse.braceFormat")],
    ["parse", "parse: annotation", any("parse.annotation")],
    ["parse", "parse: eslintDirective", any("parse.eslintDirective")],
    ["capture", "capture", any("rule.capture")],
    ["host", "host", any("rule.host")],
    ["formatArgs", "formatArgs", any("rule.formatArgs")],
  ]],
  ["transform", [
    ["transform", "transform", any("ast.transform")],
    ...["replace", "substring", "convert", "line", "lines", "count"].map(kind => ["transform", kind, any(`transform.${kind}`)]),
    ["transform", "default", any("transform.default")],
    ["transform", "value: true", any("transform.value")],
    ["transform", "regex-group source", any("transform.groupSource")],
  ]],
  ["Query options", [
    ["languageOptions", "languageOptions", any("ast.languageOptions")],
    ...["ecmaVersion", "sourceType", "globals", "parserOptions", "lib", "jsxPragma", "pythonVersion", "typingModules", "extendGenerics"].map(key => ["languageOptions", key, any(`languageOptions.${key}`)]),
    ["when", "when", any("ast.when")],
    ["file", "file", any("ast.file")],
    ["file", "file.path / declaration", any("file.path", "file.declaration")],
    ["file", "file module keys", any("file.module", "file.moduleName", "file.package", "file.inPackage")],
  ]],
  ["violation", [
    ["violation", "anchor: capture.NAME", any("anchor.capture")],
    ["violation", "anchor: witness.start/end", any("anchor.witness.start", "anchor.witness.end")],
    ["violation", "anchor: file.start", any("anchor.file.start")],
    ["violation", "message $VAR", any("message.var")],
    ["violation", "message ${VAR}", any("message.braced")],
    ["violation-each", "each", any("violation.each")],
    ["violation-fix", "fix (template)", any("fix.string", "fix.object")],
    ["violation-fix", "fix range", any("fix.range")],
    ["suppress", "suppress.noqa", any("suppress.noqa")],
    ["suppress", "suppress.eslint", any("suppress.eslint")],
  ]],
];

function coverage(options) {
  const rows = CONSTRUCTS.flatMap(([group, items]) => items.map(([anchor, label, { features }]) => ({ group, anchor, label, features, counts: {} })));
  const totals = {};
  const walked = [];
  for (const suite of suites(options)) {
    const docs = documents(suite);
    totals[suite.id] = docs.length;
    walked.push({ ...suite, docs: docs.length });
    const used = docs.map(({ doc }) => new Set(extract(doc).keys()));
    for (const row of rows) row.counts[suite.id] = used.filter(set => row.features.some(feature => set.has(feature))).length;
  }
  return { rows, totals, walked };
}

function printCoverage({ rows, totals, walked }) {
  for (const suite of walked) {
    const location = suite.ref === undefined ? suite.dir : `${suite.ref}@${suite.commit}:${suite.dir}`;
    console.log(`${suite.label.padEnd(8)} ${String(suite.docs).padStart(5)} documents  ${location}`);
  }
  console.log();
  const ids = Object.keys(totals);
  const header = ids.map(id => id.padStart(9)).join("");
  console.log(`${"construct".padEnd(34)}${header}`);
  for (const row of rows) {
    const label = `${row.group} / ${row.label}`.slice(0, 33).padEnd(34);
    const counts = ids.map(id => String(row.counts[id]).padStart(9)).join("");
    console.log(`${label}${counts}`);
  }
}

// ---------------------------------------------------------------------------------------------
// The coverage section: a heatmap in inline SVG plus the table behind it.

const htmlEscape = text => String(text).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

function heatmap({ rows, totals, walked }) {
  const ids = Object.keys(totals);
  const labelWidth = 230;
  const cell = 64;
  const rowHeight = 17;
  const groupGap = 14;
  const header = 44;
  let y = header;
  const parts = [];
  let lastGroup = null;
  for (const row of rows) {
    if (row.group !== lastGroup) {
      if (lastGroup !== null) y += groupGap;
      parts.push(`<text class="hm-group" x="0" y="${y + 11}">${htmlEscape(row.group)}</text>`);
      y += rowHeight;
      lastGroup = row.group;
    }
    parts.push(`<a href="#${htmlEscape(row.anchor)}"><text class="hm-label" x="10" y="${y + 12}">${htmlEscape(row.label)}</text></a>`);
    ids.forEach((id, index) => {
      const count = row.counts[id];
      const share = totals[id] === 0 ? 0 : count / totals[id];
      // Log-ish ramp: any use is visible; a construct in most ports is saturated.
      const level = count === 0 ? 0 : Math.min(6, 1 + Math.floor(Math.log2(1 + share * 63)));
      const x = labelWidth + index * cell;
      const title = `${row.label}: ${count} of ${totals[id]} ${walked[index].label} documents (${(share * 100).toFixed(1)}%)`;
      parts.push(`<g><title>${htmlEscape(title)}</title><rect class="hm-${level}" x="${x}" y="${y}" width="${cell - 4}" height="${rowHeight - 3}" rx="2"/><text class="hm-count${level >= 4 ? " hm-inv" : ""}" x="${x + (cell - 4) / 2}" y="${y + 11}">${count === 0 ? "·" : count}</text></g>`);
    });
    y += rowHeight;
  }
  const width = labelWidth + ids.length * cell;
  const heads = walked.map((suite, index) => `<text class="hm-head" x="${labelWidth + index * cell + (cell - 4) / 2}" y="16">${htmlEscape(suite.label)}</text><text class="hm-sub" x="${labelWidth + index * cell + (cell - 4) / 2}" y="32">n=${suite.docs}</text>`).join("");
  return `<svg class="figure heatmap" viewBox="0 0 ${width} ${y + 8}" role="img" aria-label="Coverage heatmap: committed ports using each construct, per suite">${heads}${parts.join("")}</svg>`;
}

function suiteLocation(suite) {
  const dir = suite.dir.startsWith(RERAIL) ? relative(RERAIL, suite.dir) : suite.dir;
  if (suite.ref === undefined) return `<code>${htmlEscape(dir)}</code>`;
  return `<code>${htmlEscape(dir)}</code> on <code>${htmlEscape(suite.ref.replace(/^origin\//u, ""))}</code> @ <code>${suite.commit}</code>`;
}

function coverageSection(result) {
  const suitesLine = result.walked.map(suite => `<li><b>${htmlEscape(suite.label)}</b>: ${suite.docs} documents in ${suiteLocation(suite)}</li>`).join("");
  return [
    "<!-- coverage:begin (generated by scripts/code-rule-dsl-reference.mjs coverage --write) -->",
    `<ul class="scanned">${suitesLine}</ul>`,
    `<div class="figure-wrap">${heatmap(result)}</div>`,
    "<!-- coverage:end -->",
  ].join("\n");
}

function writeCoverage(result) {
  const html = readFileSync(REFERENCE, "utf8");
  const begin = html.indexOf("<!-- coverage:begin");
  const end = html.indexOf("<!-- coverage:end -->");
  if (begin < 0 || end < 0) throw new Error("The reference has no coverage markers.");
  writeFileSync(REFERENCE, html.slice(0, begin) + coverageSection(result) + html.slice(end + "<!-- coverage:end -->".length));
}

// ---------------------------------------------------------------------------------------------
// Example checking.

/** `text` without its tags: what `text.replace(/<[^>]+>/g, "")` returns, in one linear pass. */
function stripTags(text) {
  let out = "";
  let from = 0;
  let open = text.indexOf("<");
  while (open >= 0) {
    const close = text.indexOf(">", open + 1);
    if (close < 0) break;
    if (close > open + 1) {
      out += text.slice(from, open);
      from = close + 1;
      open = text.indexOf("<", from);
    } else open = text.indexOf("<", open + 1);
  }
  return out + text.slice(from);
}

const htmlDecode = text => stripTags(text).replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&quot;/g, '"').replace(/&#39;/g, "'").replace(/&amp;/g, "&");

function examplesOf(html) {
  const out = [];
  const re = /<pre class="ex"([^>]*)>([\s\S]*?)<\/pre>/g;
  for (const match of html.matchAll(re)) {
    const attrs = Object.fromEntries([...match[1].matchAll(/data-([a-z]+)="([^"]*)"/g)].map(([, key, value]) => [key, value]));
    const sectionStart = html.lastIndexOf("<section", match.index);
    const section = sectionStart < 0 ? "" : html.slice(sectionStart, html.indexOf(">", sectionStart) + 1);
    const figureStart = html.lastIndexOf("<figure", match.index);
    const beforeFigure = html.slice(Math.max(html.lastIndexOf("</figure>", figureStart), sectionStart), figureStart);
    if (/data-deferred="[^"]+"/u.test(section)) attrs.deferred = "section";
    else if (/data-deferred-example/u.test(beforeFigure)) attrs.deferred = "figure";
    const line = html.slice(0, match.index).split("\n").length;
    out.push({ attrs, text: htmlDecode(match[2]), line });
  }
  return out;
}

const VIOLATION = { code: "M13", anchor: "witness.span", message: "example" };

function wrap(example) {
  const value = JSON.parse(example.text);
  const kind = example.attrs.kind;
  const document = clauses => ({ format: FORMAT, id: "m13-example", revision: 1, mode: "steer", clauses });
  if (kind === "document") return value;
  if (kind === "clause") return document([value]);
  if (kind === "clauses") return document(value);
  // A define reports nothing, so a document needs a clause beside it.
  if (kind === "define") return document([value, { id: "example", forbid: { ast: { language: value.define?.language ?? "typescript", rule: { kind: "comment" } } }, violation: VIOLATION }]);
  if (kind === "query") return document([{ id: "example", forbid: { ast: value }, violation: VIOLATION }]);
  if (kind === "rule") {
    const language = example.attrs.lang;
    if (language !== "python" && language !== "typescript") throw new Error("a rule example needs data-lang");
    return document([{ id: "example", forbid: { ast: { language, rule: value } }, violation: VIOLATION }]);
  }
  throw new Error(`unknown data-kind ${kind}`);
}

/** A napi addon at any path and extension (a `.node`, or a `cargo build` dylib). */
function loadEngine(path) {
  const module = { exports: {} };
  process.dlopen(module, resolve(path));
  return module.exports;
}

function checkExamples(options, engine) {
  const { CodeRuleDslRuntime } = loadEngine(engine);
  const capabilities = readFileSync(CAPABILITIES, "utf8");
  const examples = examplesOf(readFileSync(REFERENCE, "utf8"));
  let compiled = 0;
  let fragments = 0;
  let deferred = 0;
  const failures = [];
  for (const example of examples) {
    if (example.attrs.kind === "fragment" || example.attrs.deferred !== undefined) {
      if (example.attrs.kind === "fragment") fragments += 1;
      else deferred += 1;
      continue;
    }
    try {
      CodeRuleDslRuntime.compile(JSON.stringify(wrap(example)), "json", capabilities);
      compiled += 1;
      if (options.verbose) console.log(`ok   line ${example.line} (${example.attrs.kind})`);
    } catch (error) {
      failures.push({ line: example.line, kind: example.attrs.kind, message: String(error.message ?? error).split("\n")[0] });
    }
  }
  for (const failure of failures) console.log(`FAIL line ${failure.line} (${failure.kind}): ${failure.message}`);
  console.log(`${examples.length} examples: ${compiled} compile, ${failures.length} fail, ${fragments} fragments and ${deferred} deferred (not compiled)`);
  return failures.length === 0;
}

// ---------------------------------------------------------------------------------------------
// Locating real examples.

function locateHits(options, feature) {
  const hits = [];
  for (const suite of suites(options)) {
    for (const { path, doc } of documents(suite)) {
      for (const { pointer, owner } of extract(doc).get(feature) ?? []) {
        const clausePointer = /^\/clauses\/\d+/u.exec(pointer)?.[0];
        const value = options.whole === "clause" && clausePointer !== undefined ? doc.clauses[Number(clausePointer.slice(9))] : owner;
        const text = JSON.stringify(value);
        hits.push({ suite: suite.id, path, pointer, size: text.length, text });
      }
    }
  }
  return hits.sort((left, right) => left.size - right.size);
}

function printLocateHits(hits, options, limit) {
  const seen = new Set();
  let shown = 0;
  for (const hit of hits) {
    if (options.suite !== undefined && hit.suite !== options.suite) continue;
    if (seen.has(hit.text)) continue;
    seen.add(hit.text);
    console.log(`# ${hit.suite} ${hit.path.replace(/^.*\/(rerail\/)/u, "$1")}#${hit.pointer} (${hit.size} B)`);
    console.log(hit.text);
    shown += 1;
    if (shown >= limit) break;
  }
}

function locate(options) {
  const hits = locateHits(options, options._[1]);
  printLocateHits(hits, options, Number(options.limit ?? 5));
  console.log(`(${hits.length} uses)`);
}

// ---------------------------------------------------------------------------------------------
// Filling examples: `<figure class="ex-fig" data-src="SUITE:PATH#POINTER[|...]" data-kind=...>`
// takes its JSON from the committed file (a suite's branch, or this checkout for `repo:`), so the
// reference quotes ports exactly; a figure without `data-src` keeps its own JSON, reformatted.

const WIDTH = 96;

function inlineEntry([key, child]) {
  return `${JSON.stringify(key)}: ${inline(child)}`;
}

function inline(value) {
  if (Array.isArray(value)) return value.length === 0 ? "[]" : `[${value.map(inline).join(", ")}]`;
  if (isObject(value)) {
    const entries = Object.entries(value);
    return entries.length === 0 ? "{}" : `{ ${entries.map(inlineEntry).join(", ")} }`;
  }
  return JSON.stringify(value);
}

/** JSON with every object or array that fits the width on one line, the others one entry a line. */
function pretty(value, indent = 0, prefix = 0) {
  const flat = inline(value);
  if (indent + prefix + flat.length <= WIDTH || (!Array.isArray(value) && !isObject(value))) return flat;
  const pad = " ".repeat(indent + 2);
  if (Array.isArray(value)) return `[\n${value.map(item => pad + pretty(item, indent + 2)).join(",\n")}\n${" ".repeat(indent)}]`;
  const entries = Object.entries(value).map(([key, child]) => {
    const head = `${JSON.stringify(key)}: `;
    return pad + head + pretty(child, indent + 2, head.length);
  });
  return `{\n${entries.join(",\n")}\n${" ".repeat(indent)}}`;
}

const SOURCES = {
  repo: { dir: "", label: "this repository" },
};
const fileCache = new Map();

function resolvePointer(doc, pointer) {
  let value = doc;
  for (const token of pointer.split("/").slice(1)) {
    const key = token.replace(/~1/g, "/").replace(/~0/g, "~");
    if (value === undefined || value === null) break;
    value = Array.isArray(value) ? value[Number(key)] : value[key];
  }
  if (value === undefined) throw new Error(`no value at ${pointer}`);
  return value;
}

function loadSource(spec) {
  const match = /^(\w+):([^#]+)(#.*)?$/u.exec(spec.trim());
  if (match === null || SOURCES[match[1]] === undefined) throw new Error(`bad data-src ${spec}`);
  const [, suite, file, hash] = match;
  const source = SOURCES[suite];
  const path = source.dir === "" ? file : `${source.dir}/${file}`;
  const key = `${suite}:${path}`;
  if (!fileCache.has(key)) {
    const text = source.ref === undefined ? readFileSync(join(RERAIL, path), "utf8") : git("show", `${source.ref}:${path}`).toString("utf8");
    fileCache.set(key, JSON.parse(text));
  }
  const pointer = hash === undefined ? "" : hash.slice(1);
  const commit = source.ref === undefined ? undefined : git("rev-parse", "--short=9", source.ref).toString("utf8").trim();
  return { value: pointer === "" ? fileCache.get(key) : resolvePointer(fileCache.get(key), pointer), path, pointer, source, commit };
}

function fill() {
  const html = readFileSync(REFERENCE, "utf8");
  let count = 0;
  const out = html.replace(/<figure class="ex-fig"([^>]*)>([\s\S]*?)<\/figure>/g, (_, attrText, body) => {
    const attrs = Object.fromEntries([...attrText.matchAll(/data-([a-z]+)="([^"]*)"/g)].map(([, key, value]) => [key, value]));
    const caption = /<figcaption>[\s\S]*?<\/figcaption>/u.exec(body)?.[0] ?? "";
    let value;
    let label = "";
    if (attrs.src !== undefined) {
      const loaded = attrs.src.split("|").map(spec => {
        try {
          return loadSource(spec);
        } catch (error) {
          throw new Error(`${spec}: ${error.message}`);
        }
      });
      value = loaded.length === 1 ? loaded[0].value : loaded.map(item => item.value);
      const first = loaded[0];
      const where = loaded.map(item => `<code>${htmlEscape(item.pointer === "" ? "(whole document)" : item.pointer)}</code>`).join(" + ");
      const origin = first.commit === undefined ? first.source.label : `${first.source.label} @ <code>${first.commit}</code>`;
      label = `<div class="src"><span class="src-path">${htmlEscape(first.path)}</span> <span class="src-at">${where} · ${origin}</span></div>`;
    } else {
      const pre = /<pre class="ex"[^>]*>([\s\S]*?)<\/pre>/u.exec(body);
      if (pre === null) throw new Error(`figure without data-src or <pre>: ${attrText}`);
      value = JSON.parse(htmlDecode(pre[1]));
      const note = /<div class="src">[\s\S]*?<\/div>/u.exec(body)?.[0];
      if (note !== undefined) label = note;
    }
    count += 1;
    const lang = attrs.lang === undefined ? "" : ` data-lang="${attrs.lang}"`;
    return `<figure class="ex-fig"${attrText}>${caption}${label}<pre class="ex" data-kind="${attrs.kind}"${lang}>${pretty(value).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;")}</pre></figure>`;
  });
  writeFileSync(REFERENCE, out);
  console.log(`filled ${count} examples`);
}

// ---------------------------------------------------------------------------------------------
// Grammar check: every production `<div class="prod" data-zod="NAME[.key...]">` lists its keys as
// `<b class="k">key</b>`; they must be exactly the keys of that zod object in code-rule-schema.ts or
// code-rule-schema-semantic.ts (`SCHEMA_SOURCES`). A key whose zod value is `z.enum(...)` lists its
// values as `<span class="lit">"value"</span>` after the key, exactly the enum's.

/**
 * The index after a string, template, comment or regex literal starting at `i` in TypeScript
 * source, or `i` itself when none starts there (enough of a lexer for the schema's object literals).
 */
function skipQuoted(text, i) {
  const char = text[i];
  for (let index = i + 1; index < text.length; index += 1) {
    if (text[index] === "\\") index += 1;
    else if (text[index] === char) return index + 1;
  }
  return text.length;
}

function skipRegex(text, i) {
  let before = i - 1;
  while (before >= 0 && /\s/u.test(text[before])) before -= 1;
  if (before >= 0 && !"(,=:[!&|?{};".includes(text[before])) return i;
  let inClass = false;
  for (let index = i + 1; index < text.length; index += 1) {
    if (text[index] === "\\") index += 1;
    else if (text[index] === "[") inClass = true;
    else if (text[index] === "]") inClass = false;
    else if (text[index] === "/" && !inClass) {
      let end = index + 1;
      while (/[a-z]/u.test(text[end] ?? "")) end += 1;
      return end;
    }
  }
  return text.length;
}

function skipLiteral(text, i) {
  const char = text[i];
  if (char === '"' || char === "'" || char === "`") return skipQuoted(text, i);
  if (char !== "/") return i;
  if (text[i + 1] === "/") {
    const end = text.indexOf("\n", i);
    return end < 0 ? text.length : end;
  }
  if (text[i + 1] === "*") return text.indexOf("*/", i + 2) + 2;
  return skipRegex(text, i);
}

function balanced(text, open) {
  const pairs = { "(": ")", "{": "}", "[": "]" };
  const stack = [];
  let index = open;
  while (index < text.length) {
    const next = skipLiteral(text, index);
    if (next !== index) {
      index = next;
      continue;
    }
    const char = text[index];
    if (pairs[char] !== undefined) stack.push(pairs[char]);
    else if (char === ")" || char === "}" || char === "]") {
      stack.pop();
      if (stack.length === 0) return text.slice(open, index + 1);
    }
    index += 1;
  }
  throw new Error("unbalanced");
}

/** Top-level `key: value` entries of an object literal's source (spreads resolved by name). */
function entryPieces(literal) {
  let depth = 0;
  let start = 1;
  const pieces = [];
  let index = 0;
  while (index < literal.length) {
    const next = skipLiteral(literal, index);
    if (next !== index) {
      index = next;
      continue;
    }
    const char = literal[index];
    if ("({[".includes(char)) depth += 1;
    else if (")}]".includes(char)) {
      depth -= 1;
      if (depth === 0) pieces.push(literal.slice(start, index));
    } else if (char === "," && depth === 1) {
      pieces.push(literal.slice(start, index));
      start = index + 1;
    }
    index += 1;
  }
  return pieces.map(text => text.replace(/\/\*\*[\s\S]*?\*\//g, "").trim()).filter(Boolean);
}

/** The zod sources a production may name: the code-rule schema and its semantic fields. */
const SCHEMA_SOURCES = ["src/code-rule-schema.ts", "src/code-rule-schema-semantic.ts"];

/** The entries a spread piece brings in, or undefined when `piece` is no spread. */
function spreadEntries(schema, piece) {
  const spread = /^\.\.\.(\w+)$/u.exec(piece);
  if (spread !== null) {
    const at = schema.search(new RegExp(`const ${spread[1]} = \\{`, "u"));
    return entries(schema, balanced(schema, schema.indexOf("{", at)));
  }
  // A spread shape function (`...semanticRuleShape({ ... })`): the keys of the object it returns.
  const call = /^\.\.\.(\w+)\(/u.exec(piece);
  if (call === null) return undefined;
  const at = schema.search(new RegExp(`function ${call[1]}\\b`, "u"));
  if (at < 0) throw new Error(`no shape function ${call[1]}`);
  const returned = schema.indexOf("return {", at);
  return entries(schema, balanced(schema, schema.indexOf("{", returned)));
}

function entries(schema, literal) {
  const out = new Map();
  for (const piece of entryPieces(literal)) {
    const spread = spreadEntries(schema, piece);
    if (spread !== undefined) {
      for (const [key, value] of spread) out.set(key, value);
      continue;
    }
    const entry = /^"?(\w+)"?\s*:([\s\S]*)$/u.exec(piece);
    if (entry !== null) out.set(entry[1], entry[2].trimStart());
  }
  return out;
}

/**
 * The entries of a zod object (key → value source): `NAME` is the first `.object({ ... })` after
 * `const NAME`, `NAME@N` the N-th (0-based, in source order: a union's variants and their bodies),
 * and `.key` descends into that key's own object.
 */
function zodEntries(schema, spec) {
  const [head, ...path] = spec.split(".");
  const [name, nth] = head.split("@");
  const at = schema.search(new RegExp(`const ${name}\\b`, "u"));
  if (at < 0) throw new Error(`no zod schema ${name}`);
  let text = schema.slice(at);
  let skip = Number(nth ?? 0);
  const object = () => {
    const opens = [...text.matchAll(/\.object\(\s*\{/gu)];
    const open = opens[skip]?.index;
    skip = 0;
    if (open === undefined) throw new Error(`no object in ${spec}`);
    return entries(schema, balanced(text, text.indexOf("{", open)));
  };
  let keys = object();
  for (const key of path) {
    text = keys.get(key);
    if (text === undefined) throw new Error(`no key ${key} in ${spec}`);
    keys = object();
  }
  return keys;
}

function zodKeys(schema, spec) {
  return [...zodEntries(schema, spec).keys()];
}

/** Where an enum's named value list may be declared: the schema sources and the language tables they import. */
const ENUM_SOURCES = [...SCHEMA_SOURCES, "src/code-rule-languages.ts", "src/code-rule-language-registry.generated.ts", "src/rule-mode.ts"];

/** The string values of an array literal's source, its `...NAME` spreads resolved in `sources`. */
function arrayValues(sources, list) {
  return entryPieces(list).flatMap(piece => {
    const spread = /^\.\.\.(\w+)$/u.exec(piece);
    if (spread !== null) return namedValues(sources, spread[1]);
    const literal = /^"([^"]*)"$/u.exec(piece);
    if (literal === null) throw new Error(`enum value ${piece} is no string literal`);
    return [literal[1]];
  });
}

function namedValues(sources, name) {
  const at = sources.search(new RegExp(`const ${name}\\b[^=]*=\\s*\\[`, "u"));
  if (at < 0) throw new Error(`no enum values ${name}`);
  return arrayValues(sources, balanced(sources, sources.indexOf("[", at)));
}

/**
 * The values of a zod value written `z.enum([...])` or `z.enum(NAME)` (NAME a `const NAME = [...]`
 * of `ENUM_SOURCES`), optionally `.optional()`; undefined for any other value.
 */
function zodEnumValues(sources, value) {
  const match = /^z\.enum\((\[[^\]]*\]|\w+)\)(?:\.optional\(\))?$/u.exec(value.trim());
  if (match === null) return undefined;
  return match[1].startsWith("[") ? arrayValues(sources, match[1]) : namedValues(sources, match[1]);
}

/**
 * The literals a production lists for `key`, from its `<b class="k">` to the next key or comment;
 * a range `"a".."b"` stands for `values` from a to b.
 */
function documentedLiterals(production, key, values) {
  const start = production.indexOf(`<b class="k">${key}</b>`);
  const rest = production.slice(start + key.length + 17);
  const end = rest.search(/<b class="k">|<span class="c">/u);
  const literals = [];
  const pattern = /<span class="lit">"([^"]*)"<\/span>(?:\.\.<span class="lit">"([^"]*)"<\/span>)?/gu;
  for (const [, from, to] of (end < 0 ? rest : rest.slice(0, end)).matchAll(pattern)) {
    const first = values.indexOf(from);
    const last = to === undefined ? -1 : values.indexOf(to);
    literals.push(...(first >= 0 && last >= first ? values.slice(first, last + 1) : [from, ...(to === undefined ? [] : [to])]));
  }
  return literals;
}

/**
 * A production's enum keys whose literals differ from the schema's values, as messages. A key
 * written with no literal names a nonterminal (`Case`) and is not compared here.
 */
function enumDifferences(schema, sources, spec, production, documented) {
  const differences = [];
  for (const [key, value] of zodEntries(schema, spec)) {
    const values = zodEnumValues(sources, value);
    if (values === undefined || !documented.has(key)) continue;
    const literals = new Set(documentedLiterals(production, key, values));
    if (literals.size === 0) continue;
    const missing = values.filter(literal => !literals.has(literal));
    const extra = [...literals].filter(literal => !values.includes(literal));
    if (missing.length > 0 || extra.length > 0) {
      differences.push(`${key} values: missing ${missing.join(", ") || "-"}; not in zod ${extra.join(", ") || "-"}`);
    }
  }
  return differences;
}

function acceptedCardIds(schema, compiler) {
  const astRuleKeys = /const AST_RULE_KEYS:.*?= &\[(.*?)\];/su.exec(compiler)?.[1] ?? "";
  const ast = new Set([...astRuleKeys.matchAll(/"([^"]+)"/gu)].map(([, key]) => key));
  const define = new Set(zodKeys(schema, "codeRuleDefineSchema"));
  const utility = new Set(zodKeys(schema, "codeRuleUtilitySchema"));
  const query = new Set(zodKeys(schema, "codeRuleQuerySchema.ast"));
  const violation = new Set(zodKeys(schema, "codeRuleViolationSpecSchema"));
  const cards = new Set();
  if (ast.has("matches")) cards.add("matches");
  if (ast.has("compare")) cards.add("compare");
  if (define.has("language")) cards.add("define");
  if (define.has("options") || define.has("config")) cards.add("options");
  if (utility.has("params")) cards.add("params");
  if (query.has("file")) cards.add("file");
  if (violation.size > 0) cards.add("violation");
  return cards;
}

function deferredSectionFailures(html, accepted) {
  const deferred = [];
  for (const [, attrs] of html.matchAll(/<section\b([^>]*)>/gu)) {
    if (!/data-deferred="[^"]+"/u.test(attrs)) continue;
    const id = /\bid="([^"]+)"/u.exec(attrs)?.[1];
    if (id !== undefined && accepted.has(id)) deferred.push(id);
  }
  return deferred;
}

function checkGrammar() {
  const schema = SCHEMA_SOURCES.map(path => readFileSync(join(RERAIL, path), "utf8")).join("\n");
  const sources = ENUM_SOURCES.map(path => readFileSync(join(RERAIL, path), "utf8")).join("\n");
  const compiler = readFileSync(join(RERAIL, "native/src/code_rule_dsl/compile/ast_rule.rs"), "utf8");
  const html = readFileSync(REFERENCE, "utf8");
  let failures = 0;
  let productions = 0;
  for (const match of html.matchAll(/<div class="prod"[^>]*data-zod="([^"]+)"[^>]*>([\s\S]*?)<\/div>/g)) {
    productions += 1;
    const documented = new Set([...match[2].matchAll(/<b class="k">([\w$]+)<\/b>/g)].map(([, key]) => key));
    let expected;
    let enums;
    try {
      expected = new Set(zodKeys(schema, match[1]));
      enums = enumDifferences(schema, sources, match[1], match[2], documented);
    } catch (error) {
      failures += 1;
      console.log(`${match[1]}: ${String(error.message ?? error)}`);
      continue;
    }
    const missing = [...expected].filter(key => !documented.has(key));
    const extra = [...documented].filter(key => !expected.has(key));
    if (missing.length > 0 || extra.length > 0) {
      failures += 1;
      console.log(`${match[1]}: missing ${missing.join(", ") || "-"}; not in zod ${extra.join(", ") || "-"}`);
    }
    for (const difference of enums) {
      failures += 1;
      console.log(`${match[1]}: ${difference}`);
    }
  }
  const deferred = deferredSectionFailures(html, acceptedCardIds(schema, compiler));
  if (deferred.length > 0) {
    failures += deferred.length;
    console.log(`supported sections marked deferred: ${deferred.join(", ")}`);
  }
  console.log(`${productions} productions checked against ${SCHEMA_SOURCES.join(" and ")}, ${failures} differ`);
  return failures === 0 && productions > 0;
}

// ---------------------------------------------------------------------------------------------
// Figures: the relation walks, drawn from one tree so every panel reads the same way.

const TREE = {
  program: { x: 150, y: 0, label: "program" },
  fn: { x: 150, y: 42, label: "function", parent: "program" },
  name: { x: 62, y: 84, label: "name", parent: "fn", field: "name" },
  body: { x: 150, y: 84, label: "body", parent: "fn", field: "body" },
  ret: { x: 238, y: 84, label: "type", parent: "fn", field: "return_type" },
  a: { x: 42, y: 126, label: "stmt", parent: "body" },
  b: { x: 96, y: 126, label: "stmt", parent: "body" },
  x: { x: 150, y: 126, label: "X", parent: "body" },
  c: { x: 204, y: 126, label: "stmt", parent: "body" },
  d: { x: 258, y: 126, label: "stmt", parent: "body" },
  x1: { x: 118, y: 168, label: "child", parent: "x" },
  x2: { x: 182, y: 168, label: "child", parent: "x" },
  x3: { x: 182, y: 210, label: "child", parent: "x2" },
};

const PANELS = [
  { title: "has", rule: '"has": { … }', witness: "x", near: ["x1", "x2"], far: ["x3"], note: "children; end: every descendant" },
  { title: "inside", rule: '"inside": { … }', witness: "x", near: ["body"], far: ["fn", "program"], note: "parent; end: every ancestor" },
  { title: "precedes", rule: '"precedes": { … }', witness: "x", near: ["c"], far: ["d"], note: "next sibling; end: every later one" },
  { title: "follows", rule: '"follows": { … }', witness: "x", near: ["b"], far: ["a"], note: "previous sibling; end: every earlier one" },
  { title: "field", rule: '"has": { "field": "body" }', witness: "fn", near: ["body"], far: ["a", "b", "x", "c", "d", "x1", "x2", "x3"], note: "the body child; end: its subtree" },
  { title: "stopBy rule", rule: '"inside": { "stopBy": { "kind": "function" } }', witness: "x", near: ["body"], far: ["fn"], skipped: ["program"], stop: "fn", note: "walk ends at the stop node, which is tested" },
];

function edgeOffset(parent, node) {
  if (node.x < parent.x) return -34;
  if (node.x > parent.x) return 4;
  return 6;
}

function nodeClass(panel, id) {
  if (id === panel.witness) return "bx hi";
  if (panel.near.includes(id)) return "bx hi3";
  if (panel.far.includes(id)) return "bx hi2";
  return "bx";
}

function nodeStyle(panel, id) {
  if (id === panel.witness) return ' style="stroke:var(--accent);stroke-width:2.4"';
  if (panel.skipped?.includes(id)) return ' style="stroke-dasharray:4 3;opacity:.55"';
  return "";
}

function drawPanel(parts, panel, index, panelWidth, panelHeight) {
  const ox = 8 + (index % 3) * (panelWidth + 8);
  const oy = 8 + Math.floor(index / 3) * (panelHeight + 8);
  const dy = 54;
  parts.push(`<g transform="translate(${ox} ${oy})">`, `<rect class="bx soft" x="0" y="0" width="${panelWidth}" height="${panelHeight}" rx="10"/>`, `<text class="t" x="12" y="22">${htmlEscape(panel.title)}</text>`, `<text class="m" x="12" y="40" style="font-size:10.5px">${htmlEscape(panel.rule)}</text>`);
  for (const node of Object.values(TREE)) {
    if (node.parent === undefined) continue;
    const parent = TREE[node.parent];
    parts.push(`<path class="ln" d="M${parent.x} ${parent.y + dy + 22} L${node.x} ${node.y + dy}"/>`);
    if (node.field !== undefined) parts.push(`<text class="s" x="${(parent.x + node.x) / 2 + edgeOffset(parent, node)}" y="${(parent.y + node.y) / 2 + dy + 16}">${node.field === "return_type" ? "" : node.field}</text>`);
  }
  for (const [id, node] of Object.entries(TREE)) {
    const width = node.label.length * 6.4 + 12;
    parts.push(`<rect class="${nodeClass(panel, id)}" x="${node.x - width / 2}" y="${node.y + dy}" width="${width}" height="22" rx="5"${nodeStyle(panel, id)}/>`, `<text class="m" x="${node.x}" y="${node.y + dy + 15}" text-anchor="middle">${htmlEscape(node.label)}</text>`);
    if (panel.stop === id) parts.push(`<text class="s warn" x="${node.x + width / 2 + 6}" y="${node.y + dy + 15}">stop</text>`);
  }
  parts.push(`<text class="s" x="12" y="${panelHeight - 12}">${htmlEscape(panel.note)}</text>`, "</g>");
}

function relationsFigure() {
  const panelWidth = 310;
  const panelHeight = 316;
  const legendGap = 18;
  const parts = [];
  PANELS.forEach((panel, index) => drawPanel(parts, panel, index, panelWidth, panelHeight));
  const legendY = 2 * (panelHeight + 8) + 14;
  const legend = [["bx hi", "witness", ' style="stroke:var(--accent);stroke-width:2.4"'], ["bx hi3", "tested with neighbor (default)", ""], ["bx hi2", "also tested with end", ""], ["bx", "not tested", ""], ["bx", "past the stop node", ' style="stroke-dasharray:4 3;opacity:.55"']];
  let x = 12;
  legend.forEach(([cls, label, style]) => {
    parts.push(`<rect class="${cls}" x="${x}" y="${legendY}" width="22" height="14" rx="3"${style}/><text class="s" x="${x + 30}" y="${legendY + 11}">${label}</text>`);
    x += 30 + label.length * 6 + legendGap;
  });
  const width = 3 * (panelWidth + 8) + 8;
  return `<div class="figure-wrap"><svg class="figure" viewBox="0 0 ${width} ${legendY + 26}" role="img" aria-label="Which nodes has, inside, precedes, follows, field and a stop rule test from a witness">${parts.join("")}</svg></div>`;
}

function figures() {
  const html = readFileSync(REFERENCE, "utf8");
  const begin = "<!-- figure:relations:begin -->";
  const end = "<!-- figure:relations:end -->";
  const from = html.indexOf(begin);
  const to = html.indexOf(end);
  if (from < 0 || to < 0) throw new Error("The reference has no relations figure markers.");
  writeFileSync(REFERENCE, `${html.slice(0, from + begin.length)}\n${relationsFigure()}\n${html.slice(to)}`);
  console.log("drew the relations figure");
}

function show(options) {
  for (const spec of options._.slice(1)) {
    const { value } = loadSource(spec);
    if (isObject(value) && Array.isArray(value.clauses)) {
      value.clauses.forEach((clause, index) => {
        const forms = Object.keys(clause).filter(key => key !== "id" && key !== "violation").join(",");
        console.log(`${spec} /clauses/${index} ${clause.id} ${forms} ${JSON.stringify(clause).length} B`);
      });
    } else console.log(pretty(value));
  }
}

function features(options) {
  const counts = new Map();
  for (const suite of suites(options)) for (const { doc } of documents(suite)) for (const feature of extract(doc).keys()) counts.set(feature, (counts.get(feature) ?? 0) + 1);
  for (const [feature, count] of [...counts].sort(([left], [right]) => left.localeCompare(right))) console.log(`${String(count).padStart(6)} ${feature}`);
}

const COMMANDS = {
  show,
  figures,
  fill,
  check: () => { process.exitCode = checkGrammar() ? 0 : 1; },
  grammar: () => { process.exitCode = checkGrammar() ? 0 : 1; },
  coverage: options => {
    const result = coverage(options);
    printCoverage(result);
    if (options.write) writeCoverage(result);
  },
  examples: async options => {
    const engine = typeof options.engine === "string" ? options.engine : (await import("../src/native.ts")).resolveNativeBindingPath();
    process.exitCode = checkExamples(options, engine) ? 0 : 1;
  },
  locate,
  features,
};

function main() {
  const options = args(process.argv.slice(2));
  const command = options._[0];
  const handler = COMMANDS[command];
  if (handler !== undefined) {
    handler(options);
    return;
  }
  console.log("usage: code-rule-dsl-reference.mjs fill | figures | check | grammar | coverage [--write] | examples [--engine PATH] | locate FEATURE | features | show SPEC");
  process.exitCode = 2;
}

main();

/**
 * The RuleTester cases of an ESLint-family test file, read statically: every `<tester>.run(name, rule, { valid, invalid })`
 * call, each case's `code` evaluated from literals, templates (`String.raw`, `noFormat` and `unIndent` tags), `+`, in-file
 * bindings, spreads, `Object.assign`, single-expression helper functions and `map`/`flatMap`/`concat`/`join`. Upstream's
 * code is parsed, never run. A case whose code cannot be read this way is kept as an unresolved case keyed on its source
 * text, so an edit to it still shows.
 *
 * Mirrors the ESLint-family generators' filters where they are visible in the source: a run of another rule, `skip: true`,
 * a custom parser or type information (`parserOptions.project`, `projectService`, `programs`), and cases over 64 KiB.
 * The nightly report (scripts/upstream/watch.ts) diffs a test file's cases between two releases (compareCases).
 */
import { parse } from "@babel/parser";
import type * as t from "@babel/types";
import { diffArrays } from "diff";

/** One case of a test file: valid or invalid, its code (or, unresolved, its source text), in file order. */
export interface TestCase {
  kind: "valid" | "invalid" | "run" | "file";
  key: string;
  unresolved: boolean;
}

/** How two releases of a test file's cases differ: added, removed and changed cases, and how many of those are unresolved. */
export interface CaseCounts {
  added: number;
  removed: number;
  changed: number;
  unresolved: number;
}

/** The largest case the generators keep; a longer one is a stress test they skip. */
const MAX_CASE_BYTES = 64 * 1024;

const UNRESOLVED: unique symbol = Symbol("unresolved");
type Unresolved = typeof UNRESOLVED;

interface Env {
  vars: Map<string, Value>;
  parent: Env | null;
}
/** A source array whose elements are evaluated when read. */
interface ArrayValue {
  type: "array";
  elements: ({ spread: Value } | { unresolvedSpread: string } | { node: t.Node | null; env: Env | null })[];
}
/** An object literal: each property's node and the scope it is evaluated in. */
interface ObjectValue {
  type: "object";
  props: Map<string, { node: t.Node; env: Env | null }>;
  unresolvedSpread: boolean;
}
interface FunctionValue {
  type: "function";
  node: t.ArrowFunctionExpression | t.FunctionExpression | t.FunctionDeclaration;
  env: Env | null;
}
type Value = string | number | boolean | null | undefined | RegExp | Unresolved | Value[] | ArrayValue | ObjectValue | FunctionValue;

function isArrayValue(value: Value): value is ArrayValue {
  return typeof value === "object" && value !== null && !Array.isArray(value) && !(value instanceof RegExp) && value.type === "array";
}
function isObjectValue(value: Value): value is ObjectValue {
  return typeof value === "object" && value !== null && !Array.isArray(value) && !(value instanceof RegExp) && value.type === "object";
}
function isFunctionValue(value: Value): value is FunctionValue {
  return typeof value === "object" && value !== null && !Array.isArray(value) && !(value instanceof RegExp) && value.type === "function";
}
/** A value `+` and template substitution can use: a string, number, boolean or undefined. */
function isPrimitive(value: Value): value is string | number | boolean | undefined {
  return value === undefined || ["string", "number", "boolean"].includes(typeof value);
}

/** Whitespace collapsed, for unresolved keys. */
function collapse(text: string): string {
  return text.replace(/\s+/gu, " ").trim();
}

/** ESLint's `unIndent` tag: the template without its first and last line breaks and its common indentation. */
function unIndent(strings: string[], values: Value[]): string {
  const text = strings.map((part, index) => (index === 0 ? part : `${String(values[index - 1])}${part}`)).join("");
  const lines = text.replace(/^\n/u, "").replace(/\n\s*$/u, "").split("\n");
  const indents = lines.filter((line) => line.trim() !== "").map((line) => /^ */u.exec(line)![0].length);
  const indent = indents.length === 0 ? 0 : Math.min(...indents);
  return lines.map((line) => line.slice(indent)).join("\n");
}

/** Every child node of `node`, in source order. */
function children(node: t.Node): t.Node[] {
  const out: t.Node[] = [];
  for (const [key, child] of Object.entries(node)) {
    if (key === "loc" || key === "leadingComments" || key === "trailingComments" || key === "innerComments") continue;
    if (Array.isArray(child)) out.push(...child.filter((item): item is t.Node => typeof (item as { type?: unknown } | null)?.type === "string"));
    else if (typeof (child as { type?: unknown } | null)?.type === "string") out.push(child as t.Node);
  }
  return out;
}

function walk(node: t.Node, visit: (node: t.Node) => void): void {
  visit(node);
  for (const child of children(node)) walk(child, visit);
}

/** A TypeScript wrapper's expression (`x as T`, `x satisfies T`, `x!`, `<T>x`), unwrapped. */
function unwrap(node: t.Node): t.Node {
  let current = node;
  while (current.type === "TSAsExpression" || current.type === "TSSatisfiesExpression" || current.type === "TSNonNullExpression" || current.type === "TSTypeAssertion" || current.type === "ParenthesizedExpression") current = current.expression;
  return current;
}

/** The static evaluator over one file's source. */
class Evaluator {
  private readonly bindings = new Map<string, t.Node[]>();
  private readonly resolving = new Set<string>();

  constructor(private readonly source: string, program: t.Program) {
    walk(program, (node) => {
      if (node.type === "VariableDeclarator" && node.id.type === "Identifier") this.bind(node.id.name, node.init ?? null);
      if (node.type === "FunctionDeclaration" && node.id) this.bind(node.id.name, node);
    });
  }

  private bind(name: string, node: t.Node | null): void {
    // A binding without an initializer still counts, so a name declared twice stays ambiguous.
    this.bindings.set(name, [...(this.bindings.get(name) ?? []), ...(node === null ? [{ type: "Noop" } as t.Noop] : [node])]);
  }

  text(node: t.Node): string {
    return this.source.slice(node.start ?? 0, node.end ?? 0);
  }

  private lookup(name: string, env: Env | null): Value {
    for (let scope = env; scope !== null; scope = scope.parent) if (scope.vars.has(name)) return scope.vars.get(name);
    const nodes = this.bindings.get(name);
    if (nodes === undefined || nodes.length !== 1 || this.resolving.has(name)) return UNRESOLVED;
    this.resolving.add(name);
    try {
      const node = nodes[0]!;
      return node.type === "FunctionDeclaration" ? { type: "function", node, env: null } : this.ev(node, null);
    } finally {
      this.resolving.delete(name);
    }
  }

  private call(fn: Value, args: Value[]): Value {
    if (!isFunctionValue(fn)) return UNRESOLVED;
    const vars = new Map<string, Value>();
    fn.node.params.forEach((param, index) => {
      if (param.type === "Identifier") vars.set(param.name, args[index]);
      else if (param.type === "AssignmentPattern" && param.left.type === "Identifier") vars.set(param.left.name, args[index] === undefined ? this.ev(param.right, fn.env) : args[index]);
    });
    const env: Env = { vars, parent: fn.env };
    if (fn.node.body.type !== "BlockStatement") return this.ev(fn.node.body, env);
    const [only, ...rest] = fn.node.body.body;
    return only?.type === "ReturnStatement" && only.argument && rest.length === 0 ? this.ev(only.argument, env) : UNRESOLVED;
  }

  private args(nodes: t.Node[], env: Env | null): Value[] | Unresolved {
    const out: Value[] = [];
    for (const node of nodes) {
      if (node.type !== "SpreadElement") {
        out.push(this.ev(node, env));
        continue;
      }
      const spread = this.materialize(this.ev(node.argument, env));
      if (spread === null) return UNRESOLVED;
      out.push(...spread);
    }
    return out;
  }

  ev(node: t.Node | null | undefined, env: Env | null): Value {
    if (node === null || node === undefined) return UNRESOLVED;
    const bare = unwrap(node);
    switch (bare.type) {
      case "StringLiteral":
      case "NumericLiteral":
      case "BooleanLiteral":
        return bare.value;
      case "NullLiteral":
        return null;
      case "RegExpLiteral":
        try {
          return new RegExp(bare.pattern, bare.flags);
        } catch {
          return UNRESOLVED;
        }
      case "TemplateLiteral":
        return this.template(bare, env);
      case "TaggedTemplateExpression":
        return this.tagged(bare, env);
      case "BinaryExpression": {
        if (bare.operator !== "+") return UNRESOLVED;
        const left: Value = this.ev(bare.left, env);
        const right: Value = this.ev(bare.right, env);
        if (!isPrimitive(left) || !isPrimitive(right)) return UNRESOLVED;
        return typeof left === "number" && typeof right === "number" ? left + right : `${String(left)}${String(right)}`;
      }
      case "Identifier":
        return bare.name === "undefined" ? undefined : this.lookup(bare.name, env);
      case "ArrayExpression":
        return {
          type: "array",
          elements: bare.elements.map((element) => {
            if (element?.type !== "SpreadElement") return { node: element, env };
            const spread = this.ev(element.argument, env);
            return Array.isArray(spread) || isArrayValue(spread) ? { spread } : { unresolvedSpread: this.text(element) };
          }),
        };
      case "ObjectExpression":
        return this.object(bare, env);
      case "ArrowFunctionExpression":
      case "FunctionExpression":
        return { type: "function", node: bare, env };
      case "CallExpression":
        return this.callExpression(bare, env);
      default:
        return UNRESOLVED;
    }
  }

  private template(node: t.TemplateLiteral, env: Env | null): Value {
    let out = node.quasis[0]!.value.cooked;
    if (out === null || out === undefined) return UNRESOLVED;
    for (const [index, expression] of node.expressions.entries()) {
      const value = this.ev(expression, env);
      const next = node.quasis[index + 1]!.value.cooked;
      if (!isPrimitive(value) || next === null || next === undefined) return UNRESOLVED;
      out += `${String(value)}${next}`;
    }
    return out;
  }

  private tagged(node: t.TaggedTemplateExpression, env: Env | null): Value {
    const tag = this.text(node.tag);
    const values = node.quasi.expressions.map((expression) => this.ev(expression, env));
    if (!values.every(isPrimitive)) return UNRESOLVED;
    // typescript-eslint's noFormat is String.raw under another name.
    if (tag === "String.raw" || tag === "noFormat") return node.quasi.quasis.map((quasi, index) => (index === 0 ? quasi.value.raw : `${String(values[index - 1])}${quasi.value.raw}`)).join("");
    if (tag === "unIndent") {
      const cooked = node.quasi.quasis.map((quasi) => quasi.value.cooked);
      return cooked.every((part): part is string => typeof part === "string") ? unIndent(cooked, values) : UNRESOLVED;
    }
    return UNRESOLVED;
  }

  private object(node: t.ObjectExpression, env: Env | null): ObjectValue {
    const value: ObjectValue = { type: "object", props: new Map(), unresolvedSpread: false };
    for (const property of node.properties) {
      if (property.type === "ObjectProperty" && !property.computed) {
        const key = property.key.type === "Identifier" ? property.key.name : property.key.type === "StringLiteral" || property.key.type === "NumericLiteral" ? String(property.key.value) : null;
        if (key !== null) value.props.set(key, { node: property.value, env });
      } else if (property.type === "SpreadElement") {
        const spread = this.ev(property.argument, env);
        if (isObjectValue(spread)) {
          for (const [key, entry] of spread.props) value.props.set(key, entry);
          value.unresolvedSpread ||= spread.unresolvedSpread;
        } else value.unresolvedSpread = true;
      }
    }
    return value;
  }

  private callExpression(node: t.CallExpression, env: Env | null): Value {
    const callee = node.callee;
    if (callee.type !== "MemberExpression" || callee.computed || callee.property.type !== "Identifier") {
      const args = this.args(node.arguments, env);
      return args === UNRESOLVED ? UNRESOLVED : this.call(this.ev(callee, env), args);
    }
    const method = callee.property.name;
    if (/RuleTester$/u.test(this.text(callee.object)) && method === "only") return this.ev(node.arguments[0], env);
    const args = this.args(node.arguments, env);
    if (args === UNRESOLVED) return UNRESOLVED;
    if (this.text(callee) === "Object.assign") {
      if (!args.every(isObjectValue)) return UNRESOLVED;
      const merged: ObjectValue = { type: "object", props: new Map(), unresolvedSpread: args.some((arg) => arg.unresolvedSpread) };
      for (const arg of args) for (const [key, entry] of arg.props) merged.props.set(key, entry);
      return merged;
    }
    const target = this.ev(callee.object, env);
    if (target === UNRESOLVED) return UNRESOLVED;
    if (typeof target === "string") return stringMethod(target, method, args);
    const array = this.materialize(target);
    if (array === null) return UNRESOLVED;
    switch (method) {
      case "join":
        return array.every((item) => typeof item === "string" || typeof item === "number") ? array.join(typeof args[0] === "string" ? args[0] : ",") : UNRESOLVED;
      case "concat":
        return [...array, ...args.flatMap((arg): Value[] => this.materialize(arg) ?? [arg])];
      case "map":
      case "flatMap": {
        const mapped = array.map((item, index) => this.call(args[0], [item, index]));
        if (mapped.includes(UNRESOLVED)) return UNRESOLVED;
        return method === "map" ? mapped : mapped.flatMap((item) => this.materialize(item) ?? [item]);
      }
      default:
        return UNRESOLVED;
    }
  }

  /** An array value's elements, evaluated; null when it is not an array or holds a spread that cannot be read. */
  materialize(value: Value): Value[] | null {
    if (Array.isArray(value)) return value;
    if (!isArrayValue(value)) return null;
    const out: Value[] = [];
    for (const element of value.elements) {
      if ("spread" in element) out.push(...(this.materialize(element.spread) ?? [UNRESOLVED]));
      else if ("unresolvedSpread" in element) return null;
      else out.push(this.ev(element.node, element.env));
    }
    return out;
  }

  /** The case elements of an array value, each with its source text; a spread that cannot be read is one unresolved case. */
  caseElements(value: Value, source: string): { value: Value; source: string }[] {
    if (Array.isArray(value)) return value.map((item) => ({ value: item, source }));
    if (!isArrayValue(value)) return [{ value: UNRESOLVED, source }];
    return value.elements.flatMap((element) => {
      if ("spread" in element) return this.caseElements(element.spread, source);
      if ("unresolvedSpread" in element) return [{ value: UNRESOLVED, source: element.unresolvedSpread }];
      return [{ value: this.ev(element.node, element.env), source: element.node === null ? "<hole>" : this.text(element.node) }];
    });
  }

  prop(object: ObjectValue, key: string): Value {
    const entry = object.props.get(key);
    if (entry === undefined) return object.unresolvedSpread ? UNRESOLVED : undefined;
    return this.ev(entry.node, entry.env);
  }

  propText(object: ObjectValue, key: string): string {
    const entry = object.props.get(key);
    return entry === undefined ? "" : this.text(entry.node);
  }
}

/** `target.<method>(...args)` for the string methods test files build code with. */
function stringMethod(target: string, method: string, args: Value[]): Value {
  const [first, second] = args;
  if (method === "repeat" && typeof first === "number") return target.repeat(first);
  if (method === "trim") return target.trim();
  if (method === "replace" && (typeof first === "string" || first instanceof RegExp) && typeof second === "string") return target.replace(first, second);
  return UNRESOLVED;
}

/** How a `languageOptions` value parses: with a TypeScript parser, a custom one, with type information, or as `current` says. */
function parsing(evaluator: Evaluator, options: Value, current: "ts" | "custom" | "typeinfo" | null): "ts" | "custom" | "typeinfo" | null {
  if (!isObjectValue(options)) return current;
  const parserOptions = evaluator.prop(options, "parserOptions");
  if (isObjectValue(parserOptions) && ["project", "projectService", "programs"].some((key) => parserOptions.props.has(key))) return "typeinfo";
  const parser = evaluator.propText(options, "parser");
  if (parser === "") return current;
  return /typescript-eslint|tsParser|typescript/iu.test(parser) ? "ts" : "custom";
}

/** The parser plugins a test file needs, by its name (a recorded copy may end in `.txt`). */
function parserPlugins(path: string): ("jsx" | "typescript")[] {
  const name = path.replace(/\.txt$/u, "");
  if (/\.tsx$/u.test(name)) return ["typescript", "jsx"];
  if (/\.[cm]?ts$/u.test(name)) return ["typescript"];
  return ["jsx"];
}

/**
 * The cases of the test file at `path` for `rule`: runs named after another rule are left out, unless no run names
 * `rule`. A file with no run is one unresolved case keyed on its whole text, so any edit to it shows.
 */
export function extractCases(source: string, path: string, rule: string): TestCase[] {
  const ast = parse(source, { sourceType: "unambiguous", errorRecovery: true, plugins: parserPlugins(path) });
  const evaluator = new Evaluator(source, ast.program);
  const testers = new Map<string, Value>();
  const runs: t.CallExpression[] = [];
  walk(ast.program, (node) => {
    if (node.type === "VariableDeclarator" && node.id.type === "Identifier" && node.init?.type === "NewExpression" && /RuleTester$/u.test(evaluator.text(node.init.callee))) {
      const config = node.init.arguments[0];
      testers.set(node.id.name, config === undefined ? null : evaluator.ev(config, null));
    }
    if (node.type === "CallExpression" && node.callee.type === "MemberExpression" && node.callee.property.type === "Identifier" && node.callee.property.name === "run" && node.arguments.length >= 3) runs.push(node);
  });
  if (runs.length === 0) return [{ kind: "file", key: `UNRESOLVED-FILE:${collapse(source)}`, unresolved: true }];
  const named = runs.map((run) => ({ run, name: evaluator.ev(run.arguments[0]!, null) }));
  const own = named.some(({ name }) => name === rule);
  const cases: TestCase[] = [];
  for (const { run, name } of named) {
    if (own && typeof name === "string" && name !== rule) continue;
    cases.push(...runCases(evaluator, run, testers));
  }
  return cases;
}

/** The cases of one `run` call. */
function runCases(evaluator: Evaluator, run: t.CallExpression, testers: Map<string, Value>): TestCase[] {
  const testsNode = run.arguments[2]!;
  const tests = evaluator.ev(testsNode, null);
  if (!isObjectValue(tests)) return [{ kind: "run", key: `UNRESOLVED:${collapse(evaluator.text(testsNode))}`, unresolved: true }];
  const tester = run.callee.type === "MemberExpression" ? run.callee.object : null;
  let config: Value = null;
  if (tester?.type === "Identifier") config = testers.get(tester.name) ?? null;
  else if (tester?.type === "NewExpression" && tester.arguments[0] !== undefined) config = evaluator.ev(tester.arguments[0], null);
  const runParsing = isObjectValue(config) ? parsing(evaluator, evaluator.prop(config, "languageOptions"), null) : null;
  const cases: TestCase[] = [];
  for (const kind of ["valid", "invalid"] as const) {
    if (!tests.props.has(kind)) continue;
    for (const { value, source } of evaluator.caseElements(evaluator.prop(tests, kind), evaluator.propText(tests, kind))) {
      const found = caseOf(evaluator, value, source, runParsing);
      if (found !== null) cases.push({ kind, ...found });
    }
  }
  return cases;
}

/** One case element's key, or null when the generators leave it out. */
function caseOf(evaluator: Evaluator, value: Value, source: string, runParsing: "ts" | "custom" | "typeinfo" | null): { key: string; unresolved: boolean } | null {
  const unresolved = (text: string): { key: string; unresolved: boolean } => ({ key: `UNRESOLVED:${collapse(text)}`, unresolved: true });
  const resolved = (code: string): { key: string; unresolved: boolean } | null => (Buffer.byteLength(code) > MAX_CASE_BYTES ? null : { key: code, unresolved: false });
  if (typeof value === "string") return runParsing === "custom" || runParsing === "typeinfo" ? null : resolved(value);
  if (!isObjectValue(value)) return unresolved(source);
  if (evaluator.prop(value, "skip") === true) return null;
  const caseParsing = parsing(evaluator, evaluator.prop(value, "languageOptions"), runParsing);
  if (caseParsing === "custom" || caseParsing === "typeinfo") return null;
  const code = evaluator.prop(value, "code");
  return typeof code === "string" ? resolved(code) : unresolved(evaluator.propText(value, "code") || source);
}

/** Items of `a` not matched one for one in `b`. */
function multisetMinus(a: string[], b: string[]): string[] {
  const counts = new Map<string, number>();
  for (const key of b) counts.set(key, (counts.get(key) ?? 0) + 1);
  return a.filter((key) => {
    const count = counts.get(key) ?? 0;
    if (count === 0) return true;
    counts.set(key, count - 1);
    return false;
  });
}

/**
 * The cases added, removed and changed from `before` to `after`; a reorder is no change. A case counts as changed when an
 * in-place edit pairs a removed case with an added one (a run of removals directly followed by additions), as added or
 * removed otherwise.
 */
export function compareCases(before: TestCase[], after: TestCase[]): CaseCounts {
  const id = (entry: TestCase): string => `${entry.unresolved ? "U" : "R"}${entry.kind}\u0000${entry.key}`;
  const [oldIds, newIds] = [before.map(id), after.map(id)];
  const added = multisetMinus(newIds, oldIds);
  const removed = multisetMinus(oldIds, newIds);
  const hunks = diffArrays(oldIds, newIds);
  let paired = 0;
  for (let index = 0; index < hunks.length; index += 1) {
    const [hunk, next] = [hunks[index]!, hunks[index + 1]];
    if (hunk.removed && next?.added) {
      paired += Math.min(hunk.count ?? hunk.value.length, next.count ?? next.value.length);
      index += 1;
    }
  }
  const changed = Math.min(paired, added.length, removed.length);
  const unresolved = [...added, ...removed].filter((key) => key.startsWith("U")).length;
  return { added: added.length - changed, removed: removed.length - changed, changed, unresolved };
}

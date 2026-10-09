import { posix } from "node:path";

import { closingIndex, lexRust, splitTopLevel, type RustToken } from "./rust-source.js";

/**
 * Which fixture files Ruff's own tests lint, and with which settings: a reader of the test modules of
 * `crates/ruff_linter/src/rules/<linter>/mod.rs` and `crates/ruff_linter/src/linter.rs`, ported from
 * nera-rule-validation's src/validation/adapters/ruff-tests.ts. It understands the shapes those modules use:
 * `#[test_case(...)]` arguments bound to parameters, `let` bindings, the module's own one-expression path helpers
 * (`notebook_path`), and fixture paths built from literals with `Path::new`, `join` and `format!`. A call it cannot
 * evaluate is kept as unresolved, never guessed.
 */

/** One file a Ruff test lints. Paths are relative to `crates/ruff_linter`, the directory cargo runs the tests from. */
export interface RuffTestLint {
  /** `<linter module>::<test function>`. */
  test: string;
  /** The linted file, e.g. `resources/test/fixtures/pycodestyle/E711.py`; undefined when the path is not a constant. */
  fixture: string | undefined;
  /** The settings expression, with test-case parameters and `let` bindings substituted. */
  settings: RustToken[];
}

const FIXTURES = "resources/test/fixtures";

/** The tokens as compact text, for matching expression shapes. */
export function tokenText(tokens: readonly RustToken[]): string {
  return tokens.map((token) => token.text).join(" ").replace(/ ?(::|\.|,|\(|\)|\[|\]|!) ?/gu, "$1").replace(/,/gu, ", ");
}

interface TestFunction {
  name: string;
  params: string[];
  /** The argument lists of each `#[test_case(...)]`; one empty list for a plain `#[test]`. */
  cases: RustToken[][][];
  body: RustToken[];
}

/** The functions of a `mod tests`: those that carry `#[test]` or `#[test_case(...)]`, and the helpers. */
function testFunctions(tokens: readonly RustToken[]): { tests: TestFunction[]; helpers: Helpers } {
  const start = tokens.findIndex((token, index) => token.text === "mod" && tokens[index + 1]?.text === "tests" && tokens[index + 2]?.text === "{");
  const helpers: Helpers = new Map();
  if (start === -1) return { tests: [], helpers };
  const end = closingIndex(tokens, start + 2);
  const functions: TestFunction[] = [];
  let attributes: RustToken[][] = [];
  for (let index = start + 3; index < end; index += 1) {
    const token = tokens[index]!;
    if (token.text === "#" && tokens[index + 1]?.text === "[") {
      const close = closingIndex(tokens, index + 1);
      attributes.push(tokens.slice(index + 2, close));
      index = close;
    } else if (token.text === "fn" && tokens[index + 1]?.kind === "ident" && tokens[index + 2]?.text === "(") {
      const paramsClose = closingIndex(tokens, index + 2);
      const params = splitTopLevel(tokens.slice(index + 3, paramsClose), ",").map((part) => part[0]?.text ?? "");
      let open = paramsClose + 1;
      while (open < end && tokens[open]!.text !== "{") open += 1;
      const bodyClose = closingIndex(tokens, open);
      const isTest = attributes.some((attribute) => attribute.length === 1 && attribute[0]!.text === "test");
      const cases = attributes
        .filter((attribute) => attribute[0]?.text === "test_case" && attribute[1]?.text === "(")
        .map((attribute) => splitTopLevel(splitTopLevel(attribute.slice(2, closingIndex(attribute, 1)), ";")[0] ?? [], ","));
      const body = tokens.slice(open + 1, bodyClose);
      if (isTest || cases.length > 0) functions.push({ name: tokens[index + 1]!.text, params, cases: cases.length > 0 ? cases : [[]], body });
      // A helper whose body is one expression (`notebook_path(path)`) is inlined where a test calls it.
      else if (body.length > 0 && splitTopLevel(body, ";").length === 1) helpers.set(tokens[index + 1]!.text, { params, body });
      attributes = [];
      index = bodyClose;
    } else if (token.text === "{") {
      index = closingIndex(tokens, index);
      attributes = [];
    } else if (token.text === ";") {
      attributes = [];
    }
  }
  return { tests: functions, helpers };
}

type Bindings = Map<string, RustToken[]>;
type Helpers = Map<string, { params: string[]; body: RustToken[] }>;

const PUNCT = (text: string): RustToken => ({ kind: "punct", text });

/** `tokens` with every bound name replaced by its (parenthesized) value; struct field names and paths are not uses. */
function substitute(tokens: readonly RustToken[], bindings: Bindings, depth = 0): RustToken[] {
  if (depth > 8) return [...tokens];
  const out: RustToken[] = [];
  const brackets: string[] = [];
  tokens.forEach((token, index) => {
    if (token.kind === "punct" && "([{".includes(token.text)) brackets.push(token.text);
    if (token.kind === "punct" && ")]}".includes(token.text)) brackets.pop();
    const previous = tokens[index - 1]?.text;
    const next = tokens[index + 1]?.text;
    const field = brackets[brackets.length - 1] === "{" && (previous === "{" || previous === ",");
    const bound = token.kind === "ident" && previous !== "." && previous !== "::" && next !== "::" && !(field && next === ":") ? bindings.get(token.text) : undefined;
    if (bound === undefined) {
      out.push(token);
      return;
    }
    // A shorthand struct field (`Settings { strict, .. }`) is the field name with the binding's value.
    if (field && (next === "," || next === "}")) out.push(token, PUNCT(":"));
    out.push(PUNCT("("), ...substitute(bound, bindings, depth + 1), PUNCT(")"));
  });
  return out;
}

/** `tokens` with every call of a one-expression helper replaced by its (parenthesized) body, arguments bound. */
function inlineHelpers(tokens: readonly RustToken[], helpers: Helpers, depth = 0): RustToken[] {
  if (depth > 8 || helpers.size === 0) return [...tokens];
  const out: RustToken[] = [];
  for (let index = 0; index < tokens.length; index += 1) {
    const token = tokens[index]!;
    const helper = token.kind === "ident" && tokens[index + 1]?.text === "(" && ![".", "::", "fn"].includes(tokens[index - 1]?.text ?? "") ? helpers.get(token.text) : undefined;
    if (helper === undefined) {
      out.push(token);
      continue;
    }
    const close = closingIndex(tokens, index + 1);
    const args = splitTopLevel(tokens.slice(index + 2, close), ",");
    const bindings: Bindings = new Map(helper.params.flatMap((param, at) => (args[at] === undefined ? [] : [[param, args[at]!] as const])));
    out.push(PUNCT("("), ...inlineHelpers(substitute(helper.body, bindings), helpers, depth + 1), PUNCT(")"));
    index = close;
  }
  return out;
}

/** The expression without a leading `&`/`*`, redundant parentheses, `?`, and conversions that keep a path's value. */
function unwrap(tokens: readonly RustToken[]): RustToken[] {
  let current = [...tokens];
  for (;;) {
    if (current[0]?.text === "&" || current[0]?.text === "*") {
      current = current.slice(1);
      continue;
    }
    if (current[0]?.text === "(" && closingIndex(current, 0) === current.length - 1) {
      current = current.slice(1, -1);
      continue;
    }
    if (current.length > 1 && current[current.length - 1]?.text === "?") {
      current = current.slice(0, -1);
      continue;
    }
    const tail = tokenText(current.slice(-4));
    if (current.length > 4 && [".to_string()", ".into_owned()", ".as_path()", ".as_str()", ".to_owned()", ".into()", ".as_ref()"].includes(tail)) {
      current = current.slice(0, -4);
      continue;
    }
    return current;
  }
}

/** The index of the last `.` outside brackets, or -1. */
function lastTopLevelDot(tokens: readonly RustToken[]): number {
  let depth = 0;
  for (let index = tokens.length - 1; index >= 0; index -= 1) {
    const token = tokens[index]!;
    if (token.kind !== "punct") continue;
    if (")]}".includes(token.text)) depth += 1;
    else if ("([{".includes(token.text)) depth -= 1;
    else if (depth === 0 && token.text === ".") return index;
  }
  return -1;
}

/** What `Rule::<Variant>.name()` and `.noqa_code()` evaluate to. */
export interface RuleVariants {
  nameOf(variant: string): string | undefined;
  codeOf(variant: string): string | undefined;
}

/** A string- or path-valued expression as a constant; undefined for any shape this reader does not understand. */
export function evaluatePath(expression: readonly RustToken[], variants: RuleVariants): string | undefined {
  const tokens = unwrap(expression);
  if (tokens.length === 0) return undefined;
  if (tokens.length === 1) return tokens[0]!.kind === "string" ? tokens[0]!.value : undefined;
  const head = tokenText(tokens.slice(0, 4));
  // format!("...", args): positional `{}` placeholders filled in order.
  if (tokens[0]!.text === "format" && tokens[1]?.text === "!" && tokens[2]?.text === "(" && closingIndex(tokens, 2) === tokens.length - 1) {
    const [template, ...args] = splitTopLevel(tokens.slice(3, -1), ",");
    if (template?.length !== 1 || template[0]!.kind !== "string") return undefined;
    const values = args.map((arg) => evaluatePath(arg, variants));
    let position = 0;
    let failed = false;
    const result = template[0]!.value!.replace(/\{\{|\}\}|\{([^{}]*)\}/gu, (match, inner: string | undefined) => {
      if (match === "{{") return "{";
      if (match === "}}") return "}";
      if ((inner ?? "").split(":")[0] !== "") {
        failed = true;
        return "";
      }
      const value = values[position];
      position += 1;
      if (value === undefined) failed = true;
      return value ?? "";
    });
    return failed ? undefined : result;
  }
  // Path::new(x), PathBuf::from(x), String::from(x), test_resource_path(x)
  if (/^(?:std::path::)?(?:Path::new|PathBuf::from|String::from)\($/u.test(head) && closingIndex(tokens, 3) === tokens.length - 1) {
    return evaluatePath(tokens.slice(4, -1), variants);
  }
  // PathBuf::from_iter(["a", "b", path])
  if (/^(?:std::path::)?PathBuf::from_iter\($/u.test(head) && tokens[4]?.text === "[" && closingIndex(tokens, 4) === tokens.length - 2 && closingIndex(tokens, 3) === tokens.length - 1) {
    const parts = splitTopLevel(tokens.slice(5, -2), ",").map((part) => evaluatePath(part, variants));
    return parts.some((part) => part === undefined) ? undefined : parts.join("/");
  }
  if (tokens[0]!.text === "test_resource_path" && tokens[1]?.text === "(" && closingIndex(tokens, 1) === tokens.length - 1) {
    const inner = evaluatePath(tokens.slice(2, -1), variants);
    return inner === undefined ? undefined : `resources/test/${inner}`;
  }
  const dot = lastTopLevelDot(tokens);
  if (dot === -1) return undefined;
  const receiver = tokens.slice(0, dot);
  const member = tokens[dot + 1]?.text ?? "";
  if (tokens[dot + 2]?.text !== "(" || closingIndex(tokens, dot + 2) !== tokens.length - 1) return undefined;
  const args = tokens.slice(dot + 3, -1);
  if (member === "join") {
    const base = evaluatePath(receiver, variants);
    const part = evaluatePath(args, variants);
    return base === undefined || part === undefined ? undefined : `${base.replace(/\/$/u, "")}/${part}`;
  }
  if (["to_string_lossy", "display", "to_str", "unwrap"].includes(member) && args.length === 0) return evaluatePath(receiver, variants);
  const variant = /^(?:crate::)?(?:registry::)?(?:codes::)?Rule::(\w+)$/u.exec(tokenText(unwrap(receiver)))?.[1];
  if (variant !== undefined && args.length === 0) {
    if (member === "name" || member === "as_ref") return variants.nameOf(variant);
    if (member === "noqa_code") return variants.codeOf(variant);
  }
  return undefined;
}

/** `;`-separated statements, with a leading `if`/`for`/`while`/`loop`/`match` block split off from what follows it. */
function statements(body: readonly RustToken[]): RustToken[][] {
  const out: RustToken[][] = [];
  for (const statement of splitTopLevel(body, ";")) {
    let rest = statement;
    while (["if", "for", "while", "loop", "match"].includes(rest[0]?.text ?? "")) {
      const open = rest.findIndex((token) => token.text === "{");
      if (open === -1) break;
      let close = closingIndex(rest, open);
      while (rest[close + 1]?.text === "else") {
        const next = rest.findIndex((token, index) => index > close && token.text === "{");
        if (next === -1) break;
        close = closingIndex(rest, next);
      }
      out.push(rest.slice(0, close + 1));
      rest = rest.slice(close + 1);
    }
    if (rest.length > 0) out.push(rest);
  }
  return out;
}

/**
 * The file a source expression reads: the first argument of `SourceKind::from_path`, `Notebook::from_path` or
 * `fs::read_to_string`; undefined for a source written inline.
 */
function readFile(source: readonly RustToken[]): RustToken[] | undefined {
  const read = source.findIndex((token, at) => ["from_path", "read_to_string"].includes(token.text) && source[at - 1]?.text === "::" && source[at + 1]?.text === "(");
  return read === -1 ? undefined : splitTopLevel(source.slice(read + 2, closingIndex(source, read + 1)), ",")[0];
}

/**
 * The calls in `tokens` that lint a file: `test_path`, `assert_notebook_path`, the settings-diff helpers, and
 * `test_contents` (or linter.rs's `test_contents_syntax_errors`) of a source read from a file.
 */
function lintCalls(tokens: readonly RustToken[], variants: RuleVariants): { fixture: string | undefined; settings: RustToken[] }[] {
  const calls: { fixture: string | undefined; settings: RustToken[] }[] = [];
  tokens.forEach((token, index) => {
    if (token.kind !== "ident") return;
    const macro = tokens[index + 1]?.text === "!";
    const open = index + (macro ? 2 : 1);
    if (tokens[open]?.text !== "(" || tokens[index - 1]?.text === "fn") return;
    const args = splitTopLevel(tokens.slice(open + 1, closingIndex(tokens, open)), ",");
    const fixture = (path: RustToken[] | undefined) => {
      const value = path === undefined ? undefined : evaluatePath(path, variants);
      return value === undefined ? undefined : posix.normalize(value);
    };
    const underFixtures = (path: RustToken[] | undefined) => {
      const value = fixture(path);
      return value === undefined ? undefined : posix.join(FIXTURES, value);
    };
    if (!macro && token.text === "test_path" && args.length === 2) {
      calls.push({ fixture: underFixtures(args[0]), settings: unwrap(args[1]!) });
    } else if (!macro && token.text === "test_path_with_settings_diff" && args.length === 3) {
      calls.push({ fixture: underFixtures(args[0]), settings: unwrap(args[1]!) }, { fixture: underFixtures(args[0]), settings: unwrap(args[2]!) });
    } else if (macro && token.text === "assert_diagnostics_diff" && (args.length === 3 || args.length === 4)) {
      const [path, before, after] = args.slice(args.length - 3);
      calls.push({ fixture: underFixtures(path), settings: unwrap(before!) }, { fixture: underFixtures(path), settings: unwrap(after!) });
    } else if (!macro && token.text === "assert_notebook_path" && args.length === 3) {
      calls.push({ fixture: fixture(args[0]), settings: unwrap(args[2]!) });
    } else if (!macro && (token.text === "test_contents" || token.text === "test_contents_syntax_errors") && args.length === 3) {
      // A source read from a file lints that file; an inline source has none.
      const path = readFile(args[0]!);
      if (path !== undefined) calls.push({ fixture: fixture(path), settings: unwrap(args[2]!) });
    }
  });
  return calls;
}

/** Every file the test module of one Rust source (`rules/<module>/mod.rs`, `linter.rs`) lints, per test function and test case. */
export function ruffTestLints(module: string, source: string, variants: RuleVariants): RuffTestLint[] {
  const lints: RuffTestLint[] = [];
  const { tests, helpers } = testFunctions(lexRust(source));
  for (const fn of tests) {
    for (const args of fn.cases) {
      const bindings: Bindings = new Map();
      fn.params.forEach((param, index) => {
        if (args[index] !== undefined) bindings.set(param, args[index]!);
      });
      for (const statement of statements(fn.body)) {
        const equals = statement[0]?.text === "let" ? statement.findIndex((token) => token.text === "=") : -1;
        const name = equals === -1 ? undefined : statement[statement[1]?.text === "mut" ? 2 : 1]?.text;
        const value = inlineHelpers(substitute(equals === -1 ? statement : statement.slice(equals + 1), bindings), helpers);
        const calls = lintCalls(value, variants);
        for (const call of calls) lints.push({ test: `${module}::${fn.name}`, ...call });
        // A binding that holds a lint's diagnostics is not expanded where it is used, so each lint counts once.
        if (name !== undefined && /^[A-Za-z_]\w*$/u.test(name) && calls.length === 0) bindings.set(name, value);
      }
    }
  }
  return lints;
}

/** The `Rule::<Variant>` names and `Linter::<Variant>.rules()` selections a settings expression enables. */
export function enabledSelections(settings: readonly RustToken[]): { rules: string[]; linters: string[] } {
  const rules = new Set<string>();
  const linters = new Set<string>();
  settings.forEach((token, index) => {
    if (token.text === "Rule" && settings[index + 1]?.text === "::" && settings[index + 2]?.kind === "ident") rules.add(settings[index + 2]!.text);
    if (token.text === "Linter" && settings[index + 1]?.text === "::" && settings[index + 2]?.kind === "ident"
      && settings[index + 3]?.text === "." && settings[index + 4]?.text === "rules") linters.add(settings[index + 2]!.text);
  });
  return { rules: [...rules].sort(), linters: [...linters].sort() };
}

/** Ruff's `Rule` variant name as the kebab-case rule name `ruff rule` prints (`NonPEP585Annotation` is `non-pep585-annotation`). */
export function kebabCase(variant: string): string {
  return variant.replace(/([a-z0-9])([A-Z])/gu, "$1-$2").replace(/([A-Z]+)([A-Z][a-z])/gu, "$1-$2").toLowerCase();
}

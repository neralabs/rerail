/**
 * Static half of the identifier-poisoning audit. Every identifier-like token a port hardcodes (AST `pattern`,
 * `context`, `selector`, `regex`, file-content regex `pattern`, `text` and `fix` templates; `kind` values are grammar
 * names and are skipped) that also appears in that rule's fixtures gets a status:
 *
 * - `suspicious`: a fixture identifier with no justification; the port may lean on a name only the fixtures use.
 * - `fixture-text`: same, but the word appears only in fixture strings or comments (e.g. a baked header text).
 * - `option-default`: the default-options doc hardcodes a word from some variant's options (an option list baked in).
 * - `context`: in an ast-grep `context`, which is parse scaffolding; only the `selector` node is matched.
 * - `protected` / `rule-source` / `option` / `language`: a keyword or standard-library name (`rename.ts`), a word in
 *   TSLint's rule source literals, a word in the fixtures' options, or source syntax (triple-slash, JSDoc tag, tslint:).
 *
 *   node --import tsx conformance/tslint/cli.ts literals [--rule <name>]   rules with `suspicious` or `fixture-text` tokens
 *   node --import tsx conformance/tslint/cli.ts literals --all             every token that occurs in the fixtures, with its status
 */
import { readFileSync } from "node:fs";
import { relative } from "node:path";
import ts from "typescript";
import { HARNESS_DIRECTORY, loadScoredRules, loadVariants } from "./fixtures.js";
import { globalProtectedNames, optionNames, ruleSourceWords } from "./rename.js";
import { findPort } from "./runner.js";

/**
 * Words that name source syntax rather than identifiers: triple-slash directives, JSDoc tags and TSLint's own
 * `tslint:disable` comment directives. A port may match them in comment text.
 */
const LANGUAGE_WORDS = new Set([
  "reference", "path", "types", "lib", "amd", "module", "param", "returns", "return", "typedef", "template", "type", "tslint", "enable", "disable",
  "line", "next",
]);

const REVIEW: ReadonlySet<LiteralViolation["status"]> = new Set(["suspicious", "fixture-text", "option-default"]);

/** Keys whose string values are matched against source text; `kind`, `stopBy`, `id`, messages and `matches` are not. */
const LITERAL_KEYS = new Set(["pattern", "context", "selector", "regex", "text", "fix"]);

export interface PortLiteral {
  /** Port file relative to the harness directory. */
  file: string;
  key: string;
  isRegex: boolean;
  token: string;
  value: string;
}

export interface LiteralViolation extends PortLiteral {
  /** `suspicious`, or why the literal is fine. */
  status: "suspicious" | "fixture-text" | "option-default" | "context" | "language" | "protected" | "rule-source" | "option" | "not-in-fixtures";
}

/**
 * Identifier-like tokens in one literal. AST patterns and templates drop metavariables (`$A`, `$$$B`); regexes drop
 * escapes (`\s`, `\x{..}`), group syntax and character classes (`[A-Z]` names no identifier).
 */
export function literalTokens(value: string, isRegex: boolean): string[] {
  const text = isRegex ? value.replace(/\[(?:\\.|[^\]\\])*\]/g, " ") : value.replace(/\$\$?\$?[A-Z_][A-Z0-9_]*/g, " ");
  // `\\n` in a regex matches a source escape sequence: the letter is escape syntax, not a name.
  const unescaped = text.replace(/\\\\[A-Za-z]/g, " ").replace(/\\x\{[0-9A-Fa-f]+\}|\\u\{?[0-9A-Fa-f]+\}?|\\[pP]\{\w+\}|\\./g, " ").replace(/\(\?<\w+>|\(\?P<\w+>|\\k<\w+>|\(\?[a-z]+[:)]/g, " ");
  return [...new Set(unescaped.match(/[A-Za-z_][\w$]*/g) ?? [])];
}

export function portLiterals(rule: string): PortLiteral[] {
  const port = findPort(rule);
  const literals: PortLiteral[] = [];
  for (const path of [port.document, ...port.overrides.values()]) {
    if (path === undefined) continue;
    const file = relative(HARNESS_DIRECTORY, path);
    // A file-content/path regex is `{ source, pattern }`; an AST `pattern` is ast-grep code.
    const visit = (value: unknown, key: string, parent: object | undefined): void => {
      if (typeof value === "string") {
        if (!LITERAL_KEYS.has(key)) return;
        const isRegex = key === "regex" || (key === "pattern" && parent !== undefined && "source" in parent);
        for (const token of literalTokens(value, isRegex)) literals.push({ file, key, isRegex, token, value });
      } else if (Array.isArray(value)) value.forEach(entry => visit(entry, key, parent));
      else if (typeof value === "object" && value !== null) for (const [child, entry] of Object.entries(value)) visit(entry, child, value);
    };
    visit(JSON.parse(readFileSync(path, "utf8")) as unknown, "", undefined);
  }
  return literals;
}

/**
 * Identifier texts across a rule's evaluated fixtures, every other word in their text (strings, comments), and every
 * word in the variants' options.
 */
export function fixtureIdentifiers(rule: string): { identifiers: Set<string>; words: Set<string>; options: Set<string> } {
  const identifiers = new Set<string>();
  const words = new Set<string>();
  const options = new Set<string>();
  for (const variant of loadVariants(rule)) {
    for (const word of optionNames(variant.config.options)) options.add(word);
    for (const file of variant.files) {
      for (const word of optionNames(file.config.options)) options.add(word);
      if (file.parsed.skipped) continue;
      for (const word of file.parsed.source.match(/[A-Za-z_][\w$]*/g) ?? []) words.add(word);
      const kind = file.language === "tsx" ? ts.ScriptKind.TSX : ts.ScriptKind.TS;
      const visit = (node: ts.Node): void => {
        if (ts.isIdentifier(node) || ts.isPrivateIdentifier(node)) identifiers.add(node.text.replace(/^#/, ""));
        ts.forEachChild(node, visit);
      };
      visit(ts.createSourceFile(file.path, file.parsed.source, ts.ScriptTarget.ESNext, true, kind));
    }
  }
  return { identifiers, words, options };
}

export function scanRule(rule: string): LiteralViolation[] {
  const literals = portLiterals(rule);
  if (literals.length === 0) return [];
  const { identifiers, words, options } = fixtureIdentifiers(rule);
  const protectedNames = globalProtectedNames();
  const sourceNames = ruleSourceWords(rule);
  return literals.map(literal => {
    const inFixtures = identifiers.has(literal.token) || words.has(literal.token);
    const status: LiteralViolation["status"] = !inFixtures
      ? "not-in-fixtures"
      : protectedNames.has(literal.token)
        ? "protected"
        : sourceNames.has(literal.token)
          ? "rule-source"
          : options.has(literal.token)
            ? literal.file === `rules/${rule}.json`
              ? "option-default"
              : "option"
            : LANGUAGE_WORDS.has(literal.token)
              ? "language"
              : literal.key === "context"
                ? "context"
                : identifiers.has(literal.token)
              ? "suspicious"
              : "fixture-text";
    return { ...literal, status };
  });
}

/** Text report: suspicious literals per rule (with `all`, every literal that names a fixture identifier). */
export function literalScanReport(options: { all: boolean; rule?: string | undefined }): string {
  const rules = loadScoredRules().filter(rule => options.rule === undefined || rule.name === options.rule);
  const lines: string[] = [];
  let flagged = 0;
  for (const { name } of rules) {
    const violations = scanRule(name).filter(violation => (options.all ? violation.status !== "not-in-fixtures" : REVIEW.has(violation.status)));
    if (violations.length === 0) continue;
    flagged += 1;
    lines.push(name);
    const seen = new Set<string>();
    for (const violation of violations) {
      const key = `${violation.file} ${violation.key} ${violation.token} ${violation.status}`;
      if (seen.has(key)) continue;
      seen.add(key);
      const shown = violation.value.length > 100 ? `${violation.value.slice(0, 97)}...` : violation.value;
      lines.push(`  ${violation.status.padEnd(15)} ${violation.token.padEnd(16)} ${violation.file} ${violation.key}: ${shown}`);
    }
  }
  lines.push(`${flagged} rule(s) with ${options.all ? "fixture-identifier" : "suspicious"} literals`);
  return `${lines.join("\n")}\n`;
}

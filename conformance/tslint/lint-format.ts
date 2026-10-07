import { format } from "node:util";
import ts from "typescript";

/**
 * TSLint `.lint` test-fixture parser. Mirrors palantir/tslint `src/verify/{lines,parse,lintError}.ts` at the pinned
 * SHA (see fixtures.ts) line for line, so expected failures are exactly what TSLint's own test runner compares against.
 * Positions are 0-based lines and 0-based UTF-16 columns, as TSLint's `getLineAndCharacter` reports them.
 */

/** TypeScript version the fixture directives (`[typescript]: <range>`, `#if typescript <range>`) are evaluated against. TSLint's last devDependency was `~4.0.0`. */
export const FIXTURE_TYPESCRIPT_VERSION = "4.0.2";

export interface ExpectedFailure {
  startLine: number;
  startCol: number;
  endLine: number;
  endCol: number;
  message: string;
}

export type ParsedLintFile =
  | { skipped: true; requirement: string }
  | { skipped: false; source: string; failures: ExpectedFailure[] };

type Line =
  | { kind: "code"; contents: string }
  | { kind: "substitution"; key: string; message: string }
  | { kind: "multiline"; startCol: number }
  | { kind: "end"; startCol: number; endCol: number; message: string };
type ErrorLine = Extract<Line, { kind: "multiline" | "end" }>;

const multilineErrorRegex = /^\s*(~+|~nil)$/;
const endErrorRegex = /^\s*(~+|~nil)\s*\[(.+)\]\s*$/;
const messageSubstitutionRegex = /^\[([-\w]+?)]: \s*(.+?)\s*$/;
const ZERO_LENGTH_ERROR = "~nil";

export class LintSyntaxError extends Error {
  constructor(message: string) {
    super(`Lint File Syntax Error: ${message}`);
  }
}

export function parseLine(text: string): Line {
  if (multilineErrorRegex.test(text)) return { kind: "multiline", startCol: text.indexOf("~") };
  const endError = endErrorRegex.exec(text);
  if (endError !== null) {
    const startCol = text.indexOf("~");
    const endCol = endError[1] === ZERO_LENGTH_ERROR ? startCol : text.lastIndexOf("~") + 1;
    return { kind: "end", startCol, endCol, message: endError[2]! };
  }
  const substitution = messageSubstitutionRegex.exec(text);
  if (substitution !== null) return { kind: "substitution", key: substitution[1]!, message: substitution[2]! };
  return { kind: "code", contents: text };
}

export function typescriptVersionRequirement(text: string): string | undefined {
  const first = parseLine(text.split(/\r?\n/)[0] ?? "");
  return first.kind === "substitution" && first.key === "typescript" ? first.message : undefined;
}

export function preprocessDirectives(text: string, typescriptVersion: string = FIXTURE_TYPESCRIPT_VERSION): string {
  if (!/^#(?:if|else|endif)\b/m.test(text)) return text;
  let state: "initial" | "if" | "else" = "initial";
  let collecting = true;
  const result: string[] = [];
  for (const line of text.split(/\n/)) {
    if (line.startsWith("#if typescript")) {
      if (state !== "initial") throw new LintSyntaxError("#if directives cannot be nested");
      state = "if";
      collecting = satisfiesRange(typescriptVersion, line.slice("#if typescript".length).trim());
    } else if (/^#else\s*$/.test(line)) {
      if (state !== "if") throw new LintSyntaxError("unexpected #else");
      state = "else";
      collecting = !collecting;
    } else if (/^#endif\s*$/.test(line)) {
      if (state === "initial") throw new LintSyntaxError("unexpected #endif");
      state = "initial";
      collecting = true;
    } else if (collecting) {
      result.push(line);
    }
  }
  if (state !== "initial") throw new LintSyntaxError("expected #endif");
  return result.join("\n");
}

export function removeErrorMarkup(text: string): string {
  return text
    .split("\n")
    .map(parseLine)
    .flatMap(line => (line.kind === "code" ? [line.contents] : []))
    .join("\n");
}

export function parseErrorsFromMarkup(text: string): ExpectedFailure[] {
  const lines = text.split("\n").map(parseLine);
  if (lines.length > 0 && lines[0]!.kind !== "code") throw new LintSyntaxError("text cannot start with an error mark line.");

  const substitutions = new Map<string, string>();
  for (const line of lines) {
    if (line.kind === "substitution") substitutions.set(line.key, formatMessage(substitutions, line.message));
  }

  const errorLinesForCodeLines: ErrorLine[][] = [];
  for (const line of lines) {
    if (line.kind === "code") errorLinesForCodeLines.push([]);
    else if (line.kind !== "substitution") errorLinesForCodeLines[errorLinesForCodeLines.length - 1]!.push(line);
  }

  const failures: ExpectedFailure[] = [];
  const add = (end: Extract<Line, { kind: "end" }>, startLine: number, startCol: number, endLine: number): void => {
    failures.push({ startLine, startCol, endLine, endCol: end.endCol, message: substituteMessage(substitutions, end.message) });
  };
  errorLinesForCodeLines.forEach((errorLines, lineNo) => {
    while (errorLines.length > 0) {
      const errorLine = errorLines.shift()!;
      if (errorLine.kind === "end") {
        add(errorLine, lineNo, errorLine.startCol, lineNo);
        continue;
      }
      for (let next = lineNo + 1; ; next += 1) {
        const continuation = errorLinesForCodeLines[next];
        if (continuation === undefined || continuation.length === 0 || continuation[0]!.startCol !== 0) {
          throw new LintSyntaxError(`Error mark starting at ${lineNo}:${errorLine.startCol} does not end correctly.`);
        }
        const nextLine = continuation.shift()!;
        if (nextLine.kind === "end") {
          add(nextLine, lineNo, errorLine.startCol, next);
          break;
        }
      }
    }
  });
  return failures.sort(compareFailures);
}

export function compareFailures(a: ExpectedFailure, b: ExpectedFailure): number {
  return (
    a.startLine - b.startLine ||
    a.startCol - b.startCol ||
    a.endLine - b.endLine ||
    a.endCol - b.endCol ||
    a.message.localeCompare(b.message)
  );
}

/** Parses a whole `.lint` file the way TSLint's `runTest` does: version gate, `#if` directives, then markup. */
export function parseLintFile(text: string, typescriptVersion: string = FIXTURE_TYPESCRIPT_VERSION): ParsedLintFile {
  let body = text;
  const requirement = typescriptVersionRequirement(body);
  if (requirement !== undefined) {
    if (!satisfiesRange(typescriptVersion, requirement)) return { skipped: true, requirement };
    const lineBreak = body.search(/\n/);
    body = lineBreak === -1 ? "" : body.slice(lineBreak + 1);
  }
  body = preprocessDirectives(body, typescriptVersion);
  return { skipped: false, source: removeErrorMarkup(body), failures: parseErrorsFromMarkup(body) };
}

function substituteMessage(templates: Map<string, string>, message: string): string {
  return templates.get(message) ?? formatMessage(templates, message);
}

function formatMessage(templates: Map<string, string>, message: string): string {
  const match = /^([-\w]+) % \((.+)\)$/.exec(message);
  if (match === null) return message;
  const template = templates.get(match[1]!);
  if (template === undefined) return message;
  const args = parseFormatArguments(match[2]!);
  return args === undefined ? message : format(template, ...args);
}

let scanner: ts.Scanner | undefined;

function parseFormatArguments(text: string): string[] | undefined {
  scanner ??= ts.createScanner(ts.ScriptTarget.Latest, false);
  scanner.setText(text);
  const result: string[] = [];
  let expectValue = true;
  for (let token = scanner.scan(); token !== ts.SyntaxKind.EndOfFileToken; token = scanner.scan()) {
    if (token === ts.SyntaxKind.StringLiteral) {
      if (!expectValue) return undefined;
      result.push(scanner.getTokenValue());
      expectValue = false;
    } else if (token === ts.SyntaxKind.CommaToken) {
      if (expectValue) return undefined;
      expectValue = true;
    } else if (token !== ts.SyntaxKind.WhitespaceTrivia) {
      return undefined;
    }
  }
  return result.length === 0 ? undefined : result;
}

type Version = [number, number, number];

function parseVersion(text: string): Version | undefined {
  const match = /^v?(\d+)(?:\.(\d+|x|\*))?(?:\.(\d+|x|\*))?/.exec(text.trim());
  if (match === null) return undefined;
  const part = (value: string | undefined): number => (value === undefined || value === "x" || value === "*" ? 0 : Number(value));
  return [part(match[1]), part(match[2]), part(match[3])];
}

function compareVersions(a: Version, b: Version): number {
  return a[0] - b[0] || a[1] - b[1] || a[2] - b[2];
}

/** Minimal semver range check covering the comparator forms TSLint fixtures use: `>= 2.4.0 < 3.5.0`, `<=3.4.5`, `0.0.0`, `a || b`. */
export function satisfiesRange(version: string, range: string): boolean {
  const current = parseVersion(version);
  if (current === undefined) throw new Error(`invalid version ${version}`);
  return range.split("||").some(alternative => {
    const comparators = alternative.trim().match(/(<=|>=|<|>|=)?\s*[^\s<>=]+/g) ?? [];
    if (comparators.length === 0) return true;
    return comparators.every(comparator => {
      const [, operator = "=", rest = ""] = /^(<=|>=|<|>|=)?\s*(.+)$/.exec(comparator) ?? [];
      const target = parseVersion(rest);
      if (target === undefined) return false;
      const order = compareVersions(current, target);
      switch (operator) {
        case "<":
          return order < 0;
        case "<=":
          return order <= 0;
        case ">":
          return order > 0;
        case ">=":
          return order >= 0;
        default:
          return order === 0;
      }
    });
  });
}

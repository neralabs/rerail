import { readFileSync } from "node:fs";
import { extname } from "node:path";

import { regexWords } from "../../../port-readiness/policy-literals.js";
import type { ParserVerdict } from "../../../port-readiness/types.js";

/**
 * How the gates read JavaScript and TypeScript, with the TypeScript compiler an adapter's pinned runtime installs:
 * the language's keywords and comment vocabulary, the words of test files, the names and regexes upstream's
 * implementation checks, and independent parsers' verdicts on source the engine cannot parse.
 */

// Minimal shapes of the TypeScript compiler API (4.x to 6.x) and Babel's parser used here.
export interface TsNode { kind: number; getChildren(): TsNode[]; text?: string; name?: { kind: number; text?: string }; modifiers?: { kind: number }[]; parent?: TsNode }
export interface TsSourceFile extends TsNode {
  getLineAndCharacterOfPosition(position: number): { line: number; character: number };
  parseDiagnostics: TsDiagnostic[];
}
export interface TsDiagnostic { code: number; file?: { fileName: string }; messageText: unknown; start?: number }
export interface BabelParser { parse(source: string, options: { sourceType: string; plugins: unknown[]; errorRecovery: boolean }): { errors?: { message: string }[] } }
export interface TypeScript {
  version: string;
  SyntaxKind: Record<string, number>;
  ScriptTarget: { ESNext: number };
  ScriptKind: { TS: number; TSX: number; JS: number; JSX: number };
  tokenToString(kind: number): string | undefined;
  LanguageVariant: { Standard: number; JSX: number };
  createScanner(target: number, skipTrivia: boolean, variant: number, text: string): { scan(): number; getTokenText(): string };
  commentPragmas: Record<string, { args?: { name: string }[] }>;
  createSourceFile(name: string, text: string, target: number, setParents: boolean, kind?: number): TsSourceFile;
  createCompilerHost(options: object): { getSourceFile(name: string, version: number): unknown };
  createProgram(names: string[], options: object, host: object): {
    getSyntacticDiagnostics(): TsDiagnostic[];
    getSemanticDiagnostics(): TsDiagnostic[];
  };
  flattenDiagnosticMessageText(text: unknown, newLine: string): string;
  sys: { readFile(path: string): string | undefined; readDirectory: unknown };
  readConfigFile(path: string, read: (path: string) => string | undefined): { config?: unknown; error?: unknown };
  parseJsonConfigFileContent(config: unknown, host: object, basePath: string): { options: object };
  getDefaultLibFileName(options: object): string;
  getDefaultLibFilePath(options: object): string;
}

const WORD = /[A-Za-z_$][\w$]*/gu;

export function words(text: string): string[] {
  return text.match(WORD) ?? [];
}

/** TypeScript's keywords, read from the pinned compiler: syntax wherever a rule spells them. */
export function keywords(ts: TypeScript): Set<string> {
  const found = new Set<string>();
  for (let kind = ts.SyntaxKind.FirstKeyword!; kind <= ts.SyntaxKind.LastKeyword!; kind += 1) {
    const token = ts.tokenToString(kind);
    if (token !== undefined) found.add(token);
  }
  return found;
}

/**
 * The words TypeScript gives meaning to inside comments, read from the pinned compiler: the JSDoc tag names its
 * parser recognizes and its `///` pragmas with their arguments. They are syntax only where a rule matches comment text.
 */
export function commentVocabulary(ts: TypeScript, compilerSource: string): Set<string> {
  const found = new Set<string>();
  const switches = [...compilerSource.matchAll(/switch \(tagName\.escapedText\) \{/gu)];
  for (const match of switches) {
    const body = compilerSource.slice(match.index, compilerSource.indexOf("default:", match.index));
    for (const tag of body.matchAll(/case "([A-Za-z]+)":/gu)) found.add(tag[1]!);
  }
  if (switches.length === 0) throw new Error("cannot find the JSDoc tag names in the pinned TypeScript compiler");
  for (const [pragma, definition] of Object.entries(ts.commentPragmas)) {
    for (const word of words(pragma)) found.add(word);
    for (const argument of definition.args ?? []) for (const word of words(argument.name)) found.add(word);
  }
  return found;
}

/**
 * The words of TypeScript source as its scanner sees them: identifiers, keywords, and the words inside strings,
 * templates, comments and JSX text. Numbers contribute nothing, so `100n` and `0x1F` yield no words.
 */
export function sourceWords(ts: TypeScript, text: string, jsx: boolean): string[] {
  const scanner = ts.createScanner(ts.ScriptTarget.ESNext, false, jsx ? ts.LanguageVariant.JSX : ts.LanguageVariant.Standard, text);
  const kinds = ts.SyntaxKind;
  const found: string[] = [];
  for (let token = scanner.scan(); token !== kinds.EndOfFileToken; token = scanner.scan()) {
    if (token === kinds.NumericLiteral || token === kinds.BigIntLiteral) continue;
    if (token === kinds.Identifier || token === kinds.PrivateIdentifier || (token >= kinds.FirstKeyword! && token <= kinds.LastKeyword!)) {
      found.push(scanner.getTokenText().replace(/^#/u, ""));
    } else {
      found.push(...words(scanner.getTokenText()));
    }
  }
  return found;
}

/** The script kind a path's extension gives a file; anything else reads as TypeScript. */
export function scriptKind(ts: TypeScript, path: string): number {
  const extension = extname(path).toLowerCase();
  if (extension === ".tsx") return ts.ScriptKind.TSX;
  if (extension === ".jsx") return ts.ScriptKind.JSX;
  if ([".js", ".mjs", ".cjs"].includes(extension)) return ts.ScriptKind.JS;
  return ts.ScriptKind.TS;
}

/** Whether a node is a rule's documentation block (descriptions, messages, examples) rather than what it checks. */
export type DocumentationNode = (node: TsNode) => boolean;

/** Every node of each file outside its documentation blocks, depth first. */
function visitSources(ts: TypeScript, files: string[], isDocumentation: DocumentationNode, visitor: (node: TsNode) => void): void {
  for (const file of files) {
    const sourceFile = ts.createSourceFile(file, readFileSync(file, "utf8"), ts.ScriptTarget.ESNext, true, scriptKind(ts, file));
    const visit = (node: TsNode): void => {
      if (isDocumentation(node)) return;
      visitor(node);
      for (const child of node.getChildren()) visit(child);
    };
    visit(sourceFile);
  }
}

/** The regex literals of sources outside their documentation blocks, as JavaScript regexes. */
export function literalRegexes(ts: TypeScript, files: string[], isDocumentation: DocumentationNode): RegExp[] {
  const found: RegExp[] = [];
  visitSources(ts, files, isDocumentation, (node) => {
    if (node.kind === ts.SyntaxKind.RegularExpressionLiteral && node.text !== undefined) {
      const end = node.text.lastIndexOf("/");
      try {
        found.push(new RegExp(node.text.slice(1, end), node.text.slice(end + 1).replace("g", "")));
      } catch {
        // A literal TypeScript accepts but this runtime rejects explains nothing.
      }
    }
  });
  return found;
}

/**
 * Names a source checks: the words of string literals without whitespace (a name, or a token the code matches against,
 * such as `javascript:` or `eslint-disable-line`; prose has spaces), quoted words in other strings, and regex words.
 * With `everyStringWord`, every word of every string counts: for sources whose prose all sits in the documentation
 * block, as an ESLint rule's messages sit in `meta`, so the strings left are data the code builds, such as the
 * `'parameter property'` a message names. With `objectKeys`, the keys of object literals count too: the lookup tables
 * a source checks names against, such as ESLint's `{ Math: { pow: { [CALL]: true } } }` trace map or the
 * `{ Array: false, ... }` globals tables.
 */
export function literalNames(ts: TypeScript, files: string[], isDocumentation: DocumentationNode, options: { everyStringWord?: boolean; objectKeys?: boolean } = {}): Set<string> {
  const names = new Set<string>();
  const literals = [ts.SyntaxKind.StringLiteral, ts.SyntaxKind.NoSubstitutionTemplateLiteral, ts.SyntaxKind.TemplateHead,
    ts.SyntaxKind.TemplateMiddle, ts.SyntaxKind.TemplateTail];
  const members = [ts.SyntaxKind.PropertyAssignment, ts.SyntaxKind.ShorthandPropertyAssignment, ts.SyntaxKind.MethodDeclaration,
    ts.SyntaxKind.GetAccessor, ts.SyntaxKind.SetAccessor];
  visitSources(ts, files, isDocumentation, (node) => {
    if (literals.includes(node.kind) && node.text !== undefined) {
      if (options.everyStringWord === true || !/\s/u.test(node.text)) for (const word of words(node.text)) names.add(word);
      for (const match of node.text.matchAll(/["'`]([A-Za-z_$][\w$]*)["'`]/gu)) names.add(match[1]!);
    }
    // A quoted key is a string literal, read above; a computed key is an expression.
    if (options.objectKeys === true && members.includes(node.kind) && node.parent?.kind === ts.SyntaxKind.ObjectLiteralExpression
      && node.name?.kind === ts.SyntaxKind.Identifier && node.name?.text !== undefined) {
      names.add(node.name.text);
    }
    if (node.kind === ts.SyntaxKind.RegularExpressionLiteral && node.text !== undefined) {
      for (const word of regexWords(node.text.slice(1, node.text.lastIndexOf("/")))) names.add(word);
    }
  });
  return names;
}

/**
 * Syntax and grammar errors (codes 1xxx and 17xxx-18xxx) from a TypeScript, without type checking. Decorators are
 * enabled, so the experimental-decorators notice (TS1219) is not reported as an error.
 */
export function compilerErrors(compiler: TypeScript, path: string, source: string): { syntax: string[]; grammar: string[] } {
  const name = `/fixture${extname(path) || ".ts"}`;
  const host = compiler.createCompilerHost({});
  const readSourceFile = host.getSourceFile.bind(host);
  const probeHost = {
    ...host,
    getSourceFile: (file: string, version: number) =>
      file === name ? compiler.createSourceFile(file, source, version, true, scriptKind(compiler, path)) : readSourceFile(file, version),
  };
  const program = compiler.createProgram([name], { noEmit: true, noLib: true, types: [], allowJs: true, jsx: 1, experimentalDecorators: true, target: compiler.ScriptTarget.ESNext }, probeHost);
  const format = (diagnostics: readonly TsDiagnostic[]) => diagnostics
    .filter((diagnostic) => diagnostic.file?.fileName === name && (diagnostic.code < 2000 || (diagnostic.code >= 17000 && diagnostic.code < 19000)))
    .map((diagnostic) => `TS${diagnostic.code} ${compiler.flattenDiagnosticMessageText(diagnostic.messageText, " ")}`);
  const syntax = format(program.getSyntacticDiagnostics());
  return { syntax, grammar: format(program.getSemanticDiagnostics()).filter((error) => !syntax.includes(error)) };
}

/** The independent parsers that give a second opinion on source the engine cannot parse. */
export interface IndependentParsers {
  /** The TypeScript upstream's fixtures target: the one the adapter's pinned runtime installs. */
  fixtures: TypeScript;
  /** Current TypeScript, from runtimes/parsers. */
  current: TypeScript;
  babel: BabelParser;
  babelVersion: string;
}

/**
 * What independent parsers say about a file: the fixtures' TypeScript parser, its checker's grammar errors, the
 * current TypeScript parser, and Babel's TypeScript parser. A file every parser rejects is invalid source; a file only
 * the grammar checker rejects is syntax every parser reads.
 */
export function parserVerdicts(parsers: IndependentParsers, path: string, source: string, script = false): ParserVerdict[] {
  return [
    { parser: `typescript ${parsers.fixtures.version}`, ...compilerErrors(parsers.fixtures, path, source) },
    { parser: `typescript ${parsers.current.version}`, ...compilerErrors(parsers.current, path, source) },
    { parser: `@babel/parser ${parsers.babelVersion}`, ...babelErrors(parsers.babel, source, { typescript: /\.[mc]?tsx?$/iu.test(path), jsx: /\.[jt]sx$/iu.test(path), goal: script ? "script" : /\.cjs$/iu.test(path) ? "commonjs" : "module" }) },
  ];
}

/**
 * Babel's verdict on a file, read with the syntax its type implies (TypeScript, with the `accessor` class members
 * TypeScript has accepted since 4.9, only for a TypeScript file; JSX only for a .jsx or .tsx one), as the goal upstream
 * reads it with: a module, a script (no strict mode, so `yield` can be a label), or CommonJS (a .cjs file). With error recovery, Babel throws only when it cannot build a tree, and returns the early
 * errors it recovered from as grammar errors.
 */
export function babelErrors(babel: BabelParser, source: string, syntax: { typescript: boolean; jsx: boolean; goal: "module" | "script" | "commonjs" }): { syntax: string[]; grammar: string[] } {
  try {
    const plugins = [...(syntax.typescript ? [["typescript", {}], "decoratorAutoAccessors"] : []), ...(syntax.jsx ? ["jsx"] : []), "decorators-legacy"];
    const errors = babel.parse(source, { sourceType: syntax.goal, errorRecovery: true, plugins }).errors ?? [];
    return { syntax: [], grammar: errors.map((error) => error.message) };
  } catch (error) {
    return { syntax: [(error instanceof Error ? error.message : String(error)).split("\n")[0]!], grammar: [] };
  }
}

/**
 * A `.rerail` rule file in the editor, read by the engine's own parser: its tokens as semantic tokens (highlighting)
 * and its statements that do not parse, over the text as it stands, saved or not (`highlightRr`); and, for a rule file
 * as saved, the engine's check of it with its includes, as the catalog reads it (`describeRr`).
 */
import { fileURLToPath } from "node:url";
import {
  DiagnosticSeverity,
  SemanticTokenModifiers,
  SemanticTokenTypes,
  type Diagnostic,
  type SemanticTokensLegend,
} from "vscode-languageserver/node";
import type { TextDocument } from "vscode-languageserver-textdocument";
import { loadNativeBinding, type RrHighlight, type RrTokenClass } from "../engine/native/native.js";
import { describeRerailPolicies } from "../engine/rr/rr-runner.js";
import { DIAGNOSTIC_SOURCE, JudgedText } from "./diagnostics.js";

const TOKEN_TYPES = [
  SemanticTokenTypes.comment,
  SemanticTokenTypes.keyword,
  SemanticTokenTypes.string,
  SemanticTokenTypes.regexp,
  SemanticTokenTypes.macro,
  SemanticTokenTypes.number,
  SemanticTokenTypes.operator,
  SemanticTokenTypes.function,
  SemanticTokenTypes.variable,
  SemanticTokenTypes.type,
  SemanticTokenTypes.parameter,
  SemanticTokenTypes.property,
  SemanticTokenTypes.enumMember,
  SemanticTokenTypes.namespace,
];
const DECLARATION = 1 << 0;

/** The legend the server announces: each class's token type, and `declaration` where a name is declared. */
export const RERAIL_TOKEN_LEGEND: SemanticTokensLegend = { tokenTypes: TOKEN_TYPES, tokenModifiers: [SemanticTokenModifiers.declaration] };

/** Each class's token type (an index in the legend) and modifiers. A relation is a function, as Soufflé's editors show it. */
const CLASSES: Record<RrTokenClass, readonly [type: number, modifiers: number]> = {
  "comment": [TOKEN_TYPES.indexOf(SemanticTokenTypes.comment), 0],
  "keyword": [TOKEN_TYPES.indexOf(SemanticTokenTypes.keyword), 0],
  "string": [TOKEN_TYPES.indexOf(SemanticTokenTypes.string), 0],
  "regexp": [TOKEN_TYPES.indexOf(SemanticTokenTypes.regexp), 0],
  "code": [TOKEN_TYPES.indexOf(SemanticTokenTypes.macro), 0],
  "number": [TOKEN_TYPES.indexOf(SemanticTokenTypes.number), 0],
  "operator": [TOKEN_TYPES.indexOf(SemanticTokenTypes.operator), 0],
  "relation": [TOKEN_TYPES.indexOf(SemanticTokenTypes.function), 0],
  "relation-declaration": [TOKEN_TYPES.indexOf(SemanticTokenTypes.function), DECLARATION],
  "function": [TOKEN_TYPES.indexOf(SemanticTokenTypes.function), 0],
  "function-declaration": [TOKEN_TYPES.indexOf(SemanticTokenTypes.function), DECLARATION],
  "variable": [TOKEN_TYPES.indexOf(SemanticTokenTypes.variable), 0],
  "type": [TOKEN_TYPES.indexOf(SemanticTokenTypes.type), 0],
  "type-declaration": [TOKEN_TYPES.indexOf(SemanticTokenTypes.type), DECLARATION],
  "parameter": [TOKEN_TYPES.indexOf(SemanticTokenTypes.parameter), DECLARATION],
  "property": [TOKEN_TYPES.indexOf(SemanticTokenTypes.property), 0],
  "value": [TOKEN_TYPES.indexOf(SemanticTokenTypes.enumMember), 0],
  "test": [TOKEN_TYPES.indexOf(SemanticTokenTypes.namespace), DECLARATION],
};

/** Whether `document` is a `.rerail` file: by the editor's language id, or its name. */
export function isRerailDocument(document: { readonly uri: string; readonly languageId: string }): boolean {
  return document.languageId === "rerail" || document.uri.endsWith(".rerail");
}

/** A `.rerail` text as the editor gets it: its semantic tokens' data, and its syntax errors. */
export interface RerailText {
  readonly tokens: number[];
  readonly errors: Diagnostic[];
}

/** `document`'s text read by the engine's parser: its tokens encoded as LSP semantic tokens, and its syntax errors. */
export function readRerailText(document: TextDocument): RerailText {
  const found = JSON.parse(loadNativeBinding().CodeRuleDslRuntime.highlightRr(document.getText())) as RrHighlight;
  const text = new JudgedText(document);
  return { tokens: encodeTokens(document, text, found.tokens), errors: found.errors.map(error => syntaxError(text, error)) };
}

function syntaxError(text: JudgedText, error: RrHighlight["errors"][number]): Diagnostic {
  return {
    range: { start: text.position(error.start), end: text.position(error.end) },
    severity: DiagnosticSeverity.Error,
    source: DIAGNOSTIC_SOURCE,
    message: error.message,
  };
}

/**
 * The tokens as LSP's relative encoding (`[Δline, Δstart, length, type, modifiers]` each), a token over several lines
 * one per line: an editor need not support multi-line tokens.
 */
function encodeTokens(document: TextDocument, text: JudgedText, tokens: RrHighlight["tokens"]): number[] {
  const source = document.getText();
  const out: number[] = [];
  let [lastLine, lastStart] = [0, 0];
  const push = (line: number, start: number, length: number, type: number, modifiers: number): void => {
    if (length <= 0) return;
    out.push(line - lastLine, line === lastLine ? start - lastStart : start, length, type, modifiers);
    [lastLine, lastStart] = [line, start];
  };
  for (const [startByte, endByte, cls] of tokens) {
    const [type, modifiers] = CLASSES[cls];
    const start = text.position(startByte);
    const end = text.position(endByte);
    for (let line = start.line; line <= end.line; line++) {
      const from = line === start.line ? start.character : 0;
      const to = line === end.line ? end.character : lineLength(document, source, line);
      push(line, from, to - from, type, modifiers);
    }
  }
  return out;
}

/** Line `line`'s length in UTF-16 units, without its line break. */
function lineLength(document: TextDocument, source: string, line: number): number {
  const start = document.offsetAt({ line, character: 0 });
  let end = document.offsetAt({ line: line + 1, character: 0 });
  if (end > start && source[end - 1] === "\n") end--;
  if (end > start && source[end - 1] === "\r") end--;
  return end - start;
}

/**
 * The engine's check of the `.rerail` rule file `document` names, as saved, with its includes (`describeRr`, as the
 * catalog reads it): its error, at the line it names. Nothing for a file that states no `.policy` (a library file,
 * which is no rule on its own) or is not on disk.
 */
export async function checkRerailRule(document: TextDocument): Promise<Diagnostic[]> {
  if (!document.uri.startsWith("file:") || !/^\s*\.policy\b/m.test(document.getText())) return [];
  const [row] = await describeRerailPolicies([fileURLToPath(document.uri)]);
  if (row === undefined || row.ok) return [];
  const line = /\bline (\d+)\b/.exec(row.error)?.[1];
  const at = line === undefined ? 0 : Math.max(Number(line) - 1, 0);
  const range = { start: { line: at, character: 0 }, end: { line: at, character: lineLength(document, document.getText(), at) } };
  return [{ range, severity: DiagnosticSeverity.Error, source: DIAGNOSTIC_SOURCE, message: row.error }];
}

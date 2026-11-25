/**
 * A Violation as an editor diagnostic. The engine's spans count UTF-8 bytes; LSP positions count UTF-16 code units
 * by default, so each byte offset is converted against the exact text the audit judged.
 */
import { DiagnosticSeverity, type Diagnostic, type Position, type Range } from "vscode-languageserver/node";
import type { TextDocument } from "vscode-languageserver-textdocument";
import type { RuleMode } from "../engine/catalog/rule-mode.js";
import type { CodeRuleSourceSpan, CodeRuleViolation } from "../engine/code-rule/code-rule.js";
import { ruleUrl } from "../engine/code-rule/rerail-steer.js";

/** The `source` every Rerail diagnostic carries, so code actions pick out their own. */
export const DIAGNOSTIC_SOURCE = "rerail";

/** Carried on a diagnostic whose violation suggests a replacement: the quick fix applies it. */
export interface DiagnosticFix {
  readonly range: Range;
  readonly text: string;
}

const SEVERITY: Record<RuleMode, DiagnosticSeverity> = {
  block: DiagnosticSeverity.Error,
  steer: DiagnosticSeverity.Warning,
  log: DiagnosticSeverity.Information,
};

/** The judged text, held once per publish: its UTF-8 bytes map the engine's offsets to the editor's. */
export class JudgedText {
  readonly #document: TextDocument;
  readonly #bytes: number;
  /** Each UTF-8 byte offset's UTF-16 offset, for a text that is not ASCII (where the two are the same). */
  readonly #utf16: Uint32Array | undefined;

  constructor(document: TextDocument) {
    this.#document = document;
    const text = document.getText();
    this.#bytes = Buffer.byteLength(text, "utf8");
    this.#utf16 = this.#bytes === text.length ? undefined : utf16Offsets(text, this.#bytes);
  }

  range(span: CodeRuleSourceSpan): Range {
    return { start: this.position(span.startByte), end: this.position(span.endByte) };
  }

  /** Byte offset `byte` as an editor position; a byte inside a character is that character's start. */
  position(byte: number): Position {
    const clamped = Math.min(Math.max(byte, 0), this.#bytes);
    return this.#document.positionAt(this.#utf16?.[clamped] ?? clamped);
  }
}

/** The UTF-16 offset of each UTF-8 byte offset of `text` (`bytes` long), as `Buffer.from(text, "utf8")` encodes it. */
function utf16Offsets(text: string, bytes: number): Uint32Array {
  const out = new Uint32Array(bytes + 1);
  let byte = 0;
  for (let at = 0; at < text.length; at++) {
    const code = text.codePointAt(at) ?? 0;
    // A lone surrogate encodes as U+FFFD: three bytes, as any other code below U+10000.
    const width = code < 0x80 ? 1 : code < 0x800 ? 2 : code < 0x10000 ? 3 : 4;
    out.fill(at, byte, byte + width);
    byte += width;
    if (code >= 0x10000) at++;
  }
  out[bytes] = text.length;
  return out;
}

const WHOLE_FILE: Range = { start: { line: 0, character: 0 }, end: { line: 0, character: 0 } };

/** `violation` on `judged`; `dashboard` links its rule's page as the diagnostic's `why?`. */
export function violationDiagnostic(violation: CodeRuleViolation, judged: JudgedText, dashboard: string | undefined): Diagnostic {
  const fix: DiagnosticFix | undefined = violation.fix === undefined ? undefined : { range: judged.range(violation.fix.span), text: violation.fix.text };
  return {
    range: violation.span === undefined ? WHOLE_FILE : judged.range(violation.span),
    severity: SEVERITY[violation.mode],
    source: DIAGNOSTIC_SOURCE,
    code: violation.ruleId,
    ...(dashboard === undefined ? {} : { codeDescription: { href: ruleUrl(dashboard, violation.ruleId) } }),
    message: violation.message,
    ...(fix === undefined ? {} : { data: { fix } }),
  };
}

/** The suggested replacement a Rerail diagnostic carries, if any. */
export function diagnosticFix(diagnostic: Diagnostic): DiagnosticFix | undefined {
  if (diagnostic.source !== DIAGNOSTIC_SOURCE) return undefined;
  const data: unknown = diagnostic.data;
  if (typeof data !== "object" || data === null || !("fix" in data)) return undefined;
  return data.fix as DiagnosticFix;
}

/**
 * A Violation as an editor diagnostic. The engine's spans count UTF-8 bytes; LSP positions count UTF-16 code units
 * by default, so each byte offset is converted against the exact text the audit judged.
 */
import { DiagnosticSeverity, type Diagnostic, type Range } from "vscode-languageserver/node";
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
  readonly #bytes: Buffer;
  readonly #ascii: boolean;

  constructor(document: TextDocument) {
    this.#document = document;
    const text = document.getText();
    this.#bytes = Buffer.from(text, "utf8");
    this.#ascii = this.#bytes.length === text.length;
  }

  range(span: CodeRuleSourceSpan): Range {
    return { start: this.#position(span.startByte), end: this.#position(span.endByte) };
  }

  #position(byte: number) {
    const clamped = Math.min(Math.max(byte, 0), this.#bytes.length);
    const offset = this.#ascii ? clamped : this.#bytes.subarray(0, clamped).toString("utf8").length;
    return this.#document.positionAt(offset);
  }
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

/**
 * Which of a file's violations an edit generated: in `ai-output` scope those on the lines the edit wrote since an
 * earlier text of the file (its preimage), in any other scope the whole file's. Every path that turns an edit's
 * findings into what the agent is told shares this choice: the native evaluation and the `.rerail` runs.
 */
import { diffLines } from "diff";
import type { AuditScope } from "../audit/audit.js";
import type { CodeRuleEdit, CodeRuleViolation } from "./code-rule.js";

type ByteRange = { start: number; end: number };

/** The earlier source an ai-output edit's generated lines are measured from. */
export type ReportBase = Pick<CodeRuleEdit, "previousText" | "previousReadError">;

function generatedRanges(previousText: string, generatedText: string): ByteRange[] {
  const ranges: ByteRange[] = [];
  let offset = 0;
  for (const change of diffLines(previousText, generatedText)) {
    const bytes = Buffer.byteLength(change.value);
    if (change.added) ranges.push({ start: offset, end: offset + bytes });
    if (!change.removed) offset += bytes;
  }
  return ranges;
}

/**
 * Whether an ai-output edit generated `violation`: its span meets a generated range. A violation with no span is about
 * the whole file (a `file.path` match, a `file` query), which the edit generated only by creating the file.
 */
function generatedBy(violation: CodeRuleViolation, ranges: readonly ByteRange[], created: boolean): boolean {
  if (violation.span === undefined) return created;
  const { startByte, endByte } = violation.span;
  return ranges.some(({ start, end }) =>
    startByte === endByte ? start <= startByte && startByte <= end : startByte < end && start < endByte,
  );
}

/**
 * Whether the edit reports its whole file: in touched-files scope, for a deleted file, and where the text it wrote or
 * the earlier one is unknown (a shell write to a file the session never saw change and Git cannot supply).
 */
function reportsWholeFile(scope: AuditScope, edit: CodeRuleEdit, base: ReportBase): boolean {
  return scope === "touched-files" || edit.deleted === true || edit.generatedText === undefined || base.previousReadError !== undefined;
}

/**
 * An edit's violations among `violations`: in touched-files scope every one in its file, otherwise those in the lines
 * it generated since `base` (its own preimage unless given).
 */
export function editViolations(
  scope: AuditScope,
  edit: CodeRuleEdit,
  violations: readonly CodeRuleViolation[],
  base: ReportBase = edit,
): CodeRuleViolation[] {
  const pathViolations = violations.filter((violation) => violation.path === edit.path);
  if (reportsWholeFile(scope, edit, base)) return pathViolations;
  const ranges = generatedRanges(base.previousText ?? "", edit.generatedText!);
  // A new file has no earlier text and no error reading one.
  const created = base.previousText === undefined;
  return pathViolations.filter((violation) => generatedBy(violation, ranges, created));
}

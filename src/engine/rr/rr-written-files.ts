/**
 * Whether a written file still holds what its edit wrote. A run after edits reads the files from disk, and the spans of
 * its findings are offsets in the file as the run read it; an edit's generated lines are offsets in the text the edit
 * wrote. The two compare only while the file holds the edit's bytes: once another session, a formatter or the person's
 * editor wrote it again, the edit's lines can no longer be told apart in it.
 */
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import type { CodeRuleEdit } from "../code-rule/code-rule.js";

/**
 * Whether the file of `edit` (relative to `root`) is as the edit left it: absent for a deletion, else holding the text
 * it wrote. An edit whose text could not be read reports its whole file whatever it holds, so it never counts as
 * changed.
 */
export function asWritten(root: string, edit: CodeRuleEdit): boolean {
  const path = join(root, edit.path);
  if (edit.deleted === true) return !existsSync(path);
  if (edit.generatedText === undefined) return true;
  try {
    return readFileSync(path, "utf8") === edit.generatedText;
  } catch {
    return false;
  }
}

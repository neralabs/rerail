/**
 * Independent parsers' verdicts on the sources the gates read (upstream's case sources and the known gaps' probes), in
 * a tool-pack's recording archive (archive.ts says how the archive is named, packed and cached):
 *
 *   verdicts/<parsers-id>/<sha256><type>[.script].json        independent parsers' verdicts on that content
 *
 * - `<parsers-id>` is the sha256 of the recorded parser list (the tool's `parsers`, canonical JSON), of the adapter
 *   code's digest, which runs the parsers, and of archive.ts's RECORDING_FORMAT, the version of the files the recorder
 *   writes their verdicts as. A verdict is named by the source's sha256, its file type (engine.ts's fileTypeOf) and
 *   whether it is read as an ECMAScript script: the rest of the path never decides a verdict, so two sources with the
 *   same text share one.
 *
 * A verdict file is the verdict list, `[]` when every parser was silent. A verdict exists exactly while its file does,
 * so the recorder asks the parsers only about the names a tree lacks, and the gates check by existence.
 */
import { z } from "zod";

import { sha256 } from "../../../scripts/s3/test-data-archive.mjs";
import { fileTypeOf } from "../engine.js";
import type { ParserVerdict } from "../types.js";
import { canonicalText, cleanVerdicts, isCleanVerdict, parserVerdictSchema, type RecordedParser } from "./schema.js";

const byText = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0);

/** The sha256 of the sorted rules, one per line: what an answer asked with every recorded rule is keyed by. */
export function rulesDigest(rules: readonly string[]): string {
  return sha256([...new Set(rules)].sort(byText).map((rule) => `${rule}\n`).join(""));
}

export function verdictPath(parsers: string, file: { path: string; sha256: string; script: boolean }): string {
  return `verdicts/${parsers}/${file.sha256}${fileTypeOf(file.path)}${file.script ? ".script" : ""}.json`;
}

/** A verdict file's text: the verdict list, `[]` when every parser was silent. */
export function verdictText(verdicts: readonly ParserVerdict[]): string {
  return canonicalText(verdicts.every(isCleanVerdict) ? [] : verdicts);
}

/** A tree file's value: valid under `schema`, and written in canonical form (one way to record a given answer). */
export function parseJson<T>(schema: z.ZodType<T>, text: string, label: string): T {
  const parsed = schema.safeParse(JSON.parse(text));
  if (!parsed.success) throw new Error(`${label}: ${parsed.error.issues.slice(0, 3).map((issue) => `/${issue.path.map(String).join("/")}: ${issue.message}`).join("; ")}`);
  if (canonicalText(parsed.data) !== text) throw new Error(`${label} is not in canonical form`);
  return parsed.data;
}

/** A verdict file's list, `[]` read as every one of `parsers` silent. */
export function parseVerdicts(text: string, label: string, parsers: readonly RecordedParser[]): ParserVerdict[] {
  const verdicts = parseJson(z.array(parserVerdictSchema), text, label);
  if (verdicts.length > 0 && verdicts.every(isCleanVerdict)) throw new Error(`${label}: every parser is silent, which is written []`);
  return verdicts.length === 0 ? cleanVerdicts(parsers) : verdicts;
}

/**
 * The one-off rewrite of every committed recording from `rerail.gates-recording/v1` to `/v2`: name the independent
 * parsers once, in `parsers`, and leave `verdicts` off every entry none of them objected to (conformance/gates/recording.ts).
 *
 * It is a rewrite and not a re-recording because re-recording needs every upstream runtime installed — uv, nine pinned
 * linters, two TypeScript versions, Babel — and would also re-fetch three real-code corpora. The migration reads only
 * what is committed, so it says nothing new about upstream: it drops bytes the v2 reader rebuilds and writes the rest
 * through the same `recordingText` the recorder writes, which is what makes the result the bytes a fresh
 * `gates:record` now produces. It is deterministic (no clock, no machine path) and idempotent: a v2 recording is
 * already in canonical form, so a second run rewrites the same bytes.
 */
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";

import {
  GATES_CORPUS, GATES_CORPUS_FORMAT, GATES_RECORDING, GATES_RECORDING_FORMAT, gatesCorpusSchema, gatesRecordingSchema,
  isCleanVerdict, recordedParsers, recordingText, type GatesCorpusRecording, type GatesRecording, type RecordedParser,
} from "../../gates/recording.js";
import { toolPackDirectory } from "../../gates/manifest.js";

/** A v1 or v2 entry: `verdicts` is written on every v1 entry and only on a v2 entry some parser objected to. */
interface Entry {
  verdicts?: { parser: string; syntax: string[]; grammar: string[] | null }[];
}

/** A recording as it is on disk, before the v2 schema has seen it. */
interface RawRecording {
  format?: unknown;
  parsers?: RecordedParser[];
  parserVerdicts?: Entry[];
  files?: Entry[];
}

/** An entry with a clean `verdicts` written out loses it; every other entry is returned as it is. */
function withoutSilence<T extends Entry>(entry: T): T {
  if (entry.verdicts === undefined || !entry.verdicts.every(isCleanVerdict)) return entry;
  const { verdicts: _silent, ...rest } = entry;
  return rest as T;
}

/**
 * The parser list to write: a v2 recording already has it, and it is kept rather than re-derived because a recording
 * whose every entry was silent (eslint-plugin-unicorn, eslint-plugin-vitest) has no verdict list left to read it off.
 * A v1 recording's lists all name the same parsers, which `recordedParsers` checks.
 */
function parsersOf(raw: RawRecording): RecordedParser[] {
  if (raw.parsers !== undefined) return raw.parsers;
  return recordedParsers((raw.parserVerdicts ?? raw.files ?? []).map((entry) => entry.verdicts ?? []));
}

/**
 * One tool-pack's two recordings in v2. The corpus recording takes gates.json's parsers: one recorder run asks one set
 * of parsers about cases and corpus files alike, and a tool upstream cannot lint real code for (javascript/eslint) has
 * no corpus file to read them off at all.
 */
export function migrateToV2(recording: unknown, corpus: unknown): { recording: GatesRecording; corpus: GatesCorpusRecording } {
  const rawRecording = recording as RawRecording;
  const rawCorpus = corpus as RawRecording;
  const parsers = parsersOf(rawRecording);
  return {
    recording: gatesRecordingSchema.parse({
      ...rawRecording,
      format: GATES_RECORDING_FORMAT,
      parsers,
      parserVerdicts: (rawRecording.parserVerdicts ?? []).map(withoutSilence),
    }),
    corpus: gatesCorpusSchema.parse({
      ...rawCorpus,
      format: GATES_CORPUS_FORMAT,
      parsers,
      files: (rawCorpus.files ?? []).map(withoutSilence),
    }),
  };
}

/** Every tool-pack with a committed recording, as `<pack>/<tool>`, sorted. */
export function recordedToolPacks(root: string): { pack: string; tool: string }[] {
  const packs = join(root, "packs");
  return readdirSync(packs, { withFileTypes: true }).filter((entry) => entry.isDirectory()).flatMap((entry) => {
    const upstream = join(packs, entry.name, "upstream");
    if (!existsSync(upstream)) return [];
    return readdirSync(upstream, { withFileTypes: true })
      .filter((tool) => tool.isDirectory() && existsSync(join(upstream, tool.name, GATES_RECORDING)))
      .map((tool) => ({ pack: entry.name, tool: tool.name }));
  }).sort((a, b) => (`${a.pack}/${a.tool}` < `${b.pack}/${b.tool}` ? -1 : 1));
}

/** What the migration did to one tool-pack: the bytes before and after, so the caller can report the saving. */
export interface MigrationResult {
  pack: string;
  tool: string;
  before: number;
  after: number;
  /** True when both files already held the bytes the migration writes. */
  unchanged: boolean;
}

/** Rewrite every committed recording under `root` in place, and report what each one cost before and after. */
export async function migrateToolPacks(root: string, write: boolean): Promise<MigrationResult[]> {
  const results: MigrationResult[] = [];
  for (const { pack, tool } of recordedToolPacks(root)) {
    const directory = toolPackDirectory(root, pack, tool);
    const paths = { recording: join(directory, GATES_RECORDING), corpus: join(directory, GATES_CORPUS) };
    const before = readFileSync(paths.recording, "utf8");
    const beforeCorpus = existsSync(paths.corpus) ? readFileSync(paths.corpus, "utf8") : null;
    if (beforeCorpus === null) throw new Error(`packs/${pack}/upstream/${tool}/${GATES_CORPUS} is missing: migrate a complete recording`);
    const migrated = migrateToV2(JSON.parse(before), JSON.parse(beforeCorpus));
    const after = recordingText(migrated.recording);
    const afterCorpus = recordingText(migrated.corpus);
    const unchanged = after === before && afterCorpus === beforeCorpus;
    if (write && !unchanged) {
      await writeFile(paths.recording, after);
      await writeFile(paths.corpus, afterCorpus);
    }
    results.push({
      pack,
      tool,
      before: Buffer.byteLength(before) + Buffer.byteLength(beforeCorpus),
      after: Buffer.byteLength(after) + Buffer.byteLength(afterCorpus),
      unchanged,
    });
  }
  return results;
}

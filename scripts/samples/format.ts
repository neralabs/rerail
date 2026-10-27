/**
 * A real-code sample set: real files labelled with where each pack rule's intended violations are, or labelled
 * clean, so the conformance check can show a rule catches what it is meant to catch and stays silent on correct code.
 * Labels mark intent; they are not a contract to reproduce the tool that produced them, whose name and version are
 * kept as provenance only.
 *
 * One set per tool-pack is one canonical archive in S3 (`samples.lock`, scripts/s3/pack-fixtures.mjs). Inside it:
 *
 *   LICENSES.md                                  each source repository, its licence, and the files taken from it
 *   <sample-id>/<source.path>                    the file, byte for byte, at its path in its repository
 *   <sample-id>/<source.path>.expected.json      its labels (`sampleLabelsSchema`)
 *   <sample-id>/<other path>                     a context file the sample needs beside it (Ruff's `__init__.py`), never judged
 *
 * A sample is judged under `<sample-id>/<source.path>`, so rule scopes see the file where its repository has it.
 * A sample is judged only for the rules it has a key for: `[]` is clean for that rule; no key is not judged.
 */
import { z } from "zod";

/** The labels file beside a sample's file. */
export const LABELS_SUFFIX = ".expected.json";
export const LICENSES_FILE = "LICENSES.md";

/** `<source>-<path slug>-<sha256 first 12>`: unique within a set, stable for the same file. */
export const SAMPLE_ID = /^[a-z0-9][a-z0-9._-]*-[0-9a-f]{12}$/u;

const nonEmpty = z.string().min(1);

/** One labelled violation: the line the rule should report on (1-based). Columns are provenance, never compared. */
export const sampleLabelSchema = z.strictObject({
  line: z.number().int().positive(),
  column: z.number().int().positive().optional(),
});
export type SampleLabel = z.infer<typeof sampleLabelSchema>;

export const sampleLabelsSchema = z.strictObject({
  source: z.strictObject({
    /** `owner/name`, as on its host. */
    repo: nonEmpty,
    commit: z.string().regex(/^[0-9a-f]{40}$/u),
    /** The file's path in its repository at `commit`; posix, relative. */
    path: nonEmpty,
    /** SPDX id of the licence the file is under at `commit`, read from the repository's licence file. */
    license: nonEmpty,
    /** sha256 of the file's bytes. */
    sha256: z.string().regex(/^[0-9a-f]{64}$/u),
  }),
  /** Where the labels came from: provenance only. */
  labelledBy: z.strictObject({
    tool: nonEmpty,
    version: nonEmpty,
    how: z.enum(["port-readiness recording", "corpus recording", "eslint test set", "hand-labelled"]),
  }),
  /** Pack rule id -> its labelled violations; `[]` is clean for that rule. Keys sorted. */
  expected: z.record(nonEmpty, z.array(sampleLabelSchema)),
});
export type SampleLabels = z.infer<typeof sampleLabelsSchema>;

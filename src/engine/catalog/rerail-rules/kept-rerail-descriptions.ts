/**
 * The engine's reading of each `.rerail` pack rule, kept beside the packs' compile verdicts (`pack-compile-checks.ts`),
 * so that a process reading unchanged packs starts no engine child to learn what their rules say. A reading holds for
 * one engine build and for the bytes of the rule file and of every file it includes: each is hashed again on every
 * read, so an edit to any of them, core's library after an update among them, asks the engine again.
 */
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { z } from "zod";
import type { RrDescription } from "../../rr/rr-runner.js";
import { describeRerailRules, type RerailDescriber, type RerailDescription, type RerailRuleFile } from "./rerail-rule.js";
import { RULE_MODES } from "../rule-mode.js";

/** A kept reading, as the engine gave it for a rule it accepts. */
export const keptDescriptionSchema = z.object({
  path: z.string(),
  ok: z.literal(true),
  id: z.string(),
  revision: z.number(),
  description: z.string().nullable(),
  mode: z.enum(RULE_MODES),
  language: z.string().nullable(),
  scope: z.object({ include: z.array(z.string()), exclude: z.array(z.string()), text: z.array(z.string()), toolCalls: z.boolean() }),
  codes: z.array(z.string()),
  tests: z.number(),
  suppresses: z.boolean(),
  fixes: z.boolean(),
  files: z.array(z.object({ path: z.string(), sha256: z.string() })).min(1),
}) satisfies z.ZodType<RerailDescription>;

/** The SHA-256 of the file at `path` in hex, or undefined when it cannot be read. */
export function sha256Of(path: string): string | undefined {
  try {
    return createHash("sha256").update(readFileSync(path)).digest("hex");
  } catch {
    return undefined;
  }
}

/** Whether a file still has the bytes it was read with, each file hashed once per call however many rules include it. */
function unchangedFiles(): (file: RerailRuleFile) => boolean {
  const digests = new Map<string, string | undefined>();
  return file => {
    if (!digests.has(file.path)) digests.set(file.path, sha256Of(file.path));
    return digests.get(file.path) === file.sha256;
  };
}

/**
 * `describe`, answered from `kept` (a reading by the path asked) where none of the files a reading came from changed,
 * and by the engine for the rest, in one call. `keep` is given each reading answered, kept or new, so the caller can
 * write back what this read used; a rule the engine refuses is not kept, and is asked about again.
 */
export function keptRerailDescriber(
  kept: ReadonlyMap<string, RerailDescription>,
  keep: (path: string, description: RerailDescription) => void,
  describe: RerailDescriber = describeRerailRules,
): RerailDescriber {
  return paths => {
    const unchanged = unchangedFiles();
    const found = new Map<string, RrDescription>(paths.flatMap(path => {
      const description = kept.get(path);
      return description !== undefined && description.files.every(unchanged) ? [[path, description] as const] : [];
    }));
    const missing = paths.filter(path => !found.has(path));
    if (missing.length > 0) describe(missing).forEach((row, at) => found.set(missing[at]!, row));
    return paths.map(path => {
      const row = found.get(path)!;
      if (row.ok) keep(path, row);
      return row;
    });
  };
}

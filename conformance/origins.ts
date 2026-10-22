/**
 * Where each rule document of a pack comes from (docs/pack-format.md, "origins.json"): a port of an upstream linter's
 * rule, or an original, Rerail's own rule. A conformance.json entry, the root one's or candidates/conformance.json's,
 * declares its document a port the conformance runner scores; origins.json declares the rest, an original with what it
 * is, or a port no conformance.json entry maps yet with why. Every rule document is declared once: the pack checks
 * (check 1) and the conformance stage of the rule gates fail one declared nowhere. Validation material, never published.
 */
import { existsSync, readFileSync } from "node:fs";
import { join, posix } from "node:path";
import { z } from "zod";
import { CONFORMANCE_FILE, documentPath } from "./languages/pack.js";

export const ORIGINS_FILE = "origins.json";

/** Why a document declared nowhere fails, after its path or rule. */
export const UNDECLARED = `declares no origin: map it in ${CONFORMANCE_FILE} if the conformance runner can score it, or declare it in ${ORIGINS_FILE}, as an original or as a port not mapped yet`;

/** The manifests whose entries declare ports, pack-relative: the conformance runner's pack and candidates/'s. */
const MANIFESTS = [CONFORMANCE_FILE, `candidates/${CONFORMANCE_FILE}`];

const nonEmpty = z.string().min(1);
const upstreamSchema = z.strictObject({ tool: nonEmpty, rule: nonEmpty });
export type Upstream = z.infer<typeof upstreamSchema>;

const entrySchema = z
  .strictObject({
    /** Pack-relative, as pack.json spells it: `rules/<id>.json` or `candidates/…/rules/<id>.json`. */
    document: nonEmpty,
    /** An original: what the rule is, and what it is modelled on, if anything. */
    original: z.string().trim().min(20).optional(),
    /** A port no conformance.json entry maps yet: the upstream rule, and `unmapped`, why no entry maps it. */
    upstream: upstreamSchema.optional(),
    unmapped: z.string().trim().min(20).optional(),
  })
  .refine(
    entry => (entry.original === undefined ? entry.upstream !== undefined && entry.unmapped !== undefined : entry.upstream === undefined && entry.unmapped === undefined),
    "declare an original with `original`, or a port no conformance.json entry maps with `upstream` and `unmapped`",
  );

/** What a conformance manifest declares: its tools, and each entry's document and upstream rule. Check 4 reads the rest. */
const manifestSchema = z.looseObject({
  tools: z.array(z.looseObject({ name: nonEmpty })).default([]),
  rules: z.array(z.looseObject({ policy: nonEmpty, upstream: z.looseObject({ tool: nonEmpty, rule: nonEmpty }) })).default([]),
});

/** A rule document's origin. A port's `unmapped` says why no conformance.json entry maps it; it is absent when one does. */
export type Origin = { kind: "port"; upstream: Upstream; unmapped?: string } | { kind: "original"; why: string };

export interface PackOrigins {
  /** Each declared document's origin, by pack-relative path; the first declaration of one declared twice. */
  origins: Map<string, Origin>;
  /** The documents given that declare no origin, in their order. */
  undeclared: string[];
  /** What is wrong with origins.json: an entry that does not parse, names no document, or names a tool the pack lacks; a document declared twice. */
  failures: string[];
}

const message = (error: unknown): string => (error instanceof Error ? error.message : String(error));

/**
 * The origins of the pack in `directory`, against `documents`, its rule documents (pack-relative). A conformance
 * manifest that does not parse declares nothing here: check 4, check 1 and the gates report it.
 */
export function packOrigins(directory: string, documents: readonly string[]): PackOrigins {
  const origins = new Map<string, Origin>();
  const declaredBy = new Map<string, string>();
  const failures: string[] = [];
  const declare = (document: string, origin: Origin, by: string): void => {
    const first = declaredBy.get(document);
    if (first !== undefined) {
      failures.push(`${document}: declared twice, by ${first} and by ${by}; declare it once`);
      return;
    }
    declaredBy.set(document, by);
    origins.set(document, origin);
  };

  const tools = new Set<string>();
  for (const manifest of MANIFESTS) {
    const path = join(directory, manifest);
    if (!existsSync(path)) continue;
    let parsed;
    try {
      parsed = manifestSchema.safeParse(JSON.parse(readFileSync(path, "utf8")));
    } catch {
      continue;
    }
    if (!parsed.success) continue;
    for (const tool of parsed.data.tools) tools.add(tool.name);
    for (const entry of parsed.data.rules) {
      declare(posix.join(posix.dirname(manifest), documentPath(entry.policy)), { kind: "port", upstream: { tool: entry.upstream.tool, rule: entry.upstream.rule } }, manifest);
    }
  }

  const path = join(directory, ORIGINS_FILE);
  if (existsSync(path)) {
    let entries: unknown[] = [];
    try {
      const value: unknown = JSON.parse(readFileSync(path, "utf8"));
      const result = z.array(entrySchema).safeParse(value);
      if (!result.success) failures.push(`${ORIGINS_FILE}: ${z.prettifyError(result.error)}`);
      // Every entry that parses still declares its document, so one bad entry fails only itself.
      entries = Array.isArray(value) ? value : [];
    } catch (error) {
      failures.push(`${ORIGINS_FILE}: is not JSON: ${message(error)}`);
    }
    const known = new Set(documents);
    for (const value of entries) {
      const parsed = entrySchema.safeParse(value);
      if (!parsed.success) continue;
      const entry = parsed.data;
      const document = documentPath(entry.document);
      if (!known.has(document)) {
        failures.push(`${ORIGINS_FILE}: ${document} is not a rule document of the pack`);
        continue;
      }
      if (entry.upstream !== undefined && !tools.has(entry.upstream.tool)) {
        failures.push(`${ORIGINS_FILE}: ${document} ports ${entry.upstream.tool}'s ${entry.upstream.rule}, but no conformance.json of the pack lists ${entry.upstream.tool} in tools`);
      }
      declare(document, entry.upstream !== undefined ? { kind: "port", upstream: entry.upstream, unmapped: entry.unmapped! } : { kind: "original", why: entry.original! }, ORIGINS_FILE);
    }
  }

  return { origins, undeclared: documents.filter(document => !origins.has(document)), failures };
}

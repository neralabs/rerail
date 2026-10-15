import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";

import { shardName, type Shard } from "./shard.js";

/**
 * The gate layer's own code: every tracked file under it, hashed from the working tree, is part of every rule's key.
 * The files that only render finished results (the merged summary, the pull request comment, the README) decide no
 * verdict, so changing them re-judges nothing.
 */
export const GATE_CODE = [
  "conformance/gates",
  ":(exclude)conformance/gates/summary.ts",
  ":(exclude)conformance/gates/comment.ts",
  ":(exclude)conformance/gates/README.md",
] as const;

/**
 * The gate code's own dependencies: a lockfile bump can change what a verdict reads (the schema validator, the YAML
 * reader), so it is part of every rule's key too.
 */
export const GATE_DEPENDENCIES = ["package.json", "package-lock.json"] as const;

/** Tags every cached entry, so an entry written by another layout of the cache is a miss. */
export const CACHE_FORMAT = "rerail.rule-gates-cache/v1";

const sha256 = (bytes: string | Buffer): string => createHash("sha256").update(bytes).digest("hex");

/** sha256 of every file `git ls-files` lists under `pathspec` at `root`, by path, hashed from the working tree. */
export function trackedDigest(root: string, pathspec: string | readonly string[]): string {
  const listed = execFileSync("git", ["ls-files", "-z", "--", ...(typeof pathspec === "string" ? [pathspec] : pathspec)], { cwd: root, encoding: "utf8", maxBuffer: 1 << 28 })
    .split("\0")
    .filter((path) => path !== "")
    .sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
  return sha256([...new Set(listed)].map((path) => {
    const absolute = join(root, path);
    return `${path}\0${existsSync(absolute) ? sha256(readFileSync(absolute)) : "deleted"}`;
  }).join("\n"));
}

/** The inputs every rule of a tool-pack shares; each is a digest or the text itself. */
export interface SharedKeyInputs {
  gateCode: string;
  dependencies: string;
  engine: string;
  gatesYaml: string;
  gatesJson: string;
  gatesCorpus: string;
  corpusTasks: string;
  corpora: string;
}

/** The inputs one rule adds. */
export interface RuleKeyInputs {
  document: string;
  conformanceEntry: string;
  goldens: string;
  caseSources: string[];
}

/**
 * A rule's result key: sha256 over the gate code and its dependencies, the engine identity, the tool-pack's manifest and recordings, the
 * corpus pins, and the rule's own document, conformance entry, goldens and case sources, each digested in that order.
 */
export function ruleKey(shared: SharedKeyInputs, rule: RuleKeyInputs): string {
  const parts = [
    shared.gateCode, shared.dependencies, shared.engine, shared.gatesYaml, shared.gatesJson, shared.gatesCorpus, shared.corpusTasks, shared.corpora,
    rule.document, rule.conformanceEntry, rule.goldens, ...rule.caseSources,
  ];
  return sha256(parts.map((part) => sha256(part)).join("\n"));
}

/**
 * Where one rule's cached result lives: `<cache>/results/<pack>/<tool>/<key>.json`, or under the shard's directory
 * (`<tool>/shard-<index>of<count>/`) when the tool-pack is sharded, so each CI leg caches its own shard.
 */
export function resultPath(cacheRoot: string, pack: string, tool: string, key: string, shard?: Shard): string {
  return join(cacheRoot, "results", pack, tool, ...(shard === undefined ? [] : [shardName(shard)]), `${key}.json`);
}

/**
 * A cached entry, or undefined when there is none or it is not one `valid` accepts: an entry that is not JSON, was
 * written by another cache layout, or does not have the shape the run reads is a miss, never a verdict. The shape check
 * guards against corruption, not forgery; pull request runs never save the cache (rule-gates.yml).
 */
export async function readCached<T>(cacheRoot: string, pack: string, tool: string, key: string, valid: (value: unknown) => value is T, shard?: Shard): Promise<T | undefined> {
  const path = resultPath(cacheRoot, pack, tool, key, shard);
  if (!existsSync(path)) return undefined;
  try {
    const entry: unknown = JSON.parse(await readFile(path, "utf8"));
    if (entry === null || typeof entry !== "object" || (entry as { format?: unknown }).format !== CACHE_FORMAT) return undefined;
    const value = (entry as { value?: unknown }).value;
    return valid(value) ? value : undefined;
  } catch {
    return undefined;
  }
}

export async function writeCached(cacheRoot: string, pack: string, tool: string, key: string, value: unknown, shard?: Shard): Promise<void> {
  const path = resultPath(cacheRoot, pack, tool, key, shard);
  await mkdir(dirname(path), { recursive: true });
  const temporary = `${path}.${process.pid}.tmp`;
  await writeFile(temporary, `${JSON.stringify({ format: CACHE_FORMAT, value })}\n`);
  await rename(temporary, path);
}

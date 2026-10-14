import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";

import { shardName, type Shard } from "./shard.js";

/** The gate layer's own code: every tracked file under it, hashed from the working tree, is part of every rule's key. */
export const GATE_CODE = "conformance/gates";

const sha256 = (bytes: string | Buffer): string => createHash("sha256").update(bytes).digest("hex");

/** sha256 of every file `git ls-files` lists under `pathspec` at `root`, by path, hashed from the working tree. */
export function trackedDigest(root: string, pathspec: string): string {
  const listed = execFileSync("git", ["ls-files", "-z", "--", pathspec], { cwd: root, encoding: "utf8", maxBuffer: 1 << 28 })
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
 * A rule's result key: sha256 over the gate code, the engine identity, the tool-pack's manifest and recordings, the
 * corpus pins, and the rule's own document, conformance entry, goldens and case sources, each digested in that order.
 */
export function ruleKey(shared: SharedKeyInputs, rule: RuleKeyInputs): string {
  const parts = [
    shared.gateCode, shared.engine, shared.gatesYaml, shared.gatesJson, shared.gatesCorpus, shared.corpusTasks, shared.corpora,
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

export async function readCached<T>(cacheRoot: string, pack: string, tool: string, key: string, shard?: Shard): Promise<T | undefined> {
  const path = resultPath(cacheRoot, pack, tool, key, shard);
  if (!existsSync(path)) return undefined;
  try {
    return JSON.parse(await readFile(path, "utf8")) as T;
  } catch {
    return undefined;
  }
}

export async function writeCached(cacheRoot: string, pack: string, tool: string, key: string, value: unknown, shard?: Shard): Promise<void> {
  const path = resultPath(cacheRoot, pack, tool, key, shard);
  await mkdir(dirname(path), { recursive: true });
  const temporary = `${path}.${process.pid}.tmp`;
  await writeFile(temporary, `${JSON.stringify(value)}\n`);
  await rename(temporary, path);
}

/**
 * Merge Assist's policy file, `.rerail/policy.yaml`, versioned and reviewed like code: how loud an open conflict gets
 * (the dimmer, `dimmer.ts`). The policy that governs a checkout's conflicts is the one at its base branch, never either
 * side's own.
 */
import { parse as parseYaml } from "yaml";
import { z } from "zod";
import { runGit } from "../git.js";
import { revisionArgument } from "./branch-git.js";

export const POLICY_PATH = ".rerail/policy.yaml";

export const policySchema = z.object({
  version: z.literal(1),
  /** How loud an open conflict gets (`dimmer.ts`). */
  dimmer: z.object({
    /** What one interruption of the person costs, in lines of rework: the checkpoint threshold. */
    interruptCost: z.number().positive().max(1_000_000).default(300),
    /** Lines per hour on the contested files above which a lock-in ×4 conflict may stop the line. */
    stopGrowthPerHour: z.number().nonnegative().max(1_000_000).default(50),
    /** Checkpoint prompts one person gets per day; the rest wait, queued. */
    checkpointsPerDay: z.number().int().min(0).max(100).default(3),
  }).strict().prefault({}),
}).strict();

export type Policy = z.output<typeof policySchema>;
export type DimmerConfig = Policy["dimmer"];

/** The policy of a repository with no `.rerail/policy.yaml`: the dimmer's defaults. */
export function defaultPolicy(): Policy {
  return policySchema.parse({ version: 1 });
}

export type PolicyParse = { readonly ok: true; readonly policy: Policy } | { readonly ok: false; readonly problems: string[] };

/** `text` as a policy: YAML (duplicate keys refused) that matches the schema; every problem otherwise. */
export function parsePolicy(text: string): PolicyParse {
  let value: unknown;
  try {
    value = parseYaml(text, { uniqueKeys: true, maxAliasCount: 0 });
  } catch (error) {
    return { ok: false, problems: [`not YAML: ${error instanceof Error ? error.message.split("\n")[0] : String(error)}`] };
  }
  const parsed = policySchema.safeParse(value ?? {});
  if (parsed.success) return { ok: true, policy: parsed.data };
  return { ok: false, problems: parsed.error.issues.map(issue => `${issue.path.length === 0 ? "policy" : issue.path.join(".")}: ${issue.message}`) };
}

export type LoadedPolicy =
  | { readonly source: "file" | "default"; readonly policy: Policy }
  | { readonly source: "invalid"; readonly problems: string[] };

/** The policy at `revision` (the base branch a checkout's conflicts are levelled at): its file, the default when it has none, or why it is invalid. */
export function loadPolicy(root: string, revision: string): LoadedPolicy {
  if (!revisionArgument(revision)) return { source: "invalid", problems: [`not a revision: ${revision}`] };
  const result = runGit(root, ["cat-file", "blob", `${revision}:${POLICY_PATH}`], { maxBuffer: 1024 * 1024 });
  if (result.status !== 0) return { source: "default", policy: defaultPolicy() };
  const parsed = parsePolicy(result.stdout);
  return parsed.ok ? { source: "file", policy: parsed.policy } : { source: "invalid", problems: parsed.problems };
}

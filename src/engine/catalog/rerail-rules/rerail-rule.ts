/**
 * A pack rule written in `.rerail` (docs/pack-format.md, "Rules written in .rerail"): the file is the rule. The engine
 * reads it with every file it includes and states what it says (`describeRerailRules`): its id and revision, mode,
 * description, language, scope and violation codes, none of which `pack.json` restates. Such a rule runs through the
 * `.rerail` runner (`src/engine/rr/`), never through the JSON engine, and takes no options.
 */
import { describeRerailPoliciesSync, type RrDescription } from "../../rr/rr-runner.js";
import type { RuleMode } from "../rule-mode.js";

/** The extension of a rule document written in `.rerail`. */
export const RERAIL_DOCUMENT_EXTENSION = ".rerail";

/** A file a `.rerail` rule was read from, with the SHA-256 of its bytes as the engine read them. */
export interface RerailRuleFile {
  readonly path: string;
  readonly sha256: string;
}

export interface RerailRule {
  /** The file's `.policy` id, which is also its file name in the pack. */
  readonly id: string;
  readonly revision: number;
  /** The mode the file's `.mode` names; an effective rule carries the mode its settings give it instead. */
  readonly mode: RuleMode;
  readonly description: string | null;
  readonly language: string | null;
  readonly scope: { readonly include: readonly string[]; readonly exclude: readonly string[] };
  /** The violation codes the file's own `.violation`s report. */
  readonly codes: readonly string[];
  /** The file's text. */
  readonly source: string;
  /** The absolute path of the installed file. */
  readonly path: string;
  /** The SHA-256 (hex) of the file's bytes as the engine read them. */
  readonly sha256: string;
  /** Every file the rule includes, directly or not, as the engine resolved it: a pack's own, or core's library. */
  readonly includes: readonly RerailRuleFile[];
}

/** The engine's reading of a `.rerail` file it accepts. */
export type RerailDescription = Extract<RrDescription, { ok: true }>;

/** Reads `.rerail` rule files and states each one's facts, one row per path in the order given. */
export type RerailDescriber = (paths: readonly string[]) => readonly RrDescription[];

/** Every path described by the engine, in a child process that runs nothing (`describeRerailPoliciesSync`). */
export const describeRerailRules: RerailDescriber = paths => (paths.length === 0 ? [] : describeRerailPoliciesSync([...paths]));

/**
 * Why a `.rerail` rule cannot be one of a pack's rules yet, or undefined: what a finding of it carries or where it
 * reads must be something the product runs. `entryMode` is the mode `pack.json` starts it in, when it names one.
 */
export function unsupportedRerailRule(described: RerailDescription, entryMode: RuleMode | undefined): string | undefined {
  if (described.mode === "block" || entryMode === "block") return "block mode is not supported yet for a .rerail rule; use steer or log";
  if (described.scope.toolCalls) return "a .rerail rule that reads tool calls (.scope tool_calls) is not supported yet in a pack";
  if (described.fixes) return "a .rerail rule whose findings carry a fix is not supported yet";
  return undefined;
}

/** The rule `described` states, read from `source`, the text of the file at `described.path`. */
export function rerailRuleOf(described: RerailDescription, source: string): RerailRule {
  return {
    id: described.id,
    revision: described.revision,
    mode: described.mode,
    description: described.description,
    language: described.language,
    scope: { include: described.scope.include, exclude: described.scope.exclude },
    codes: described.codes,
    source,
    path: described.path,
    // The engine lists the rule file first, then every file it included.
    sha256: described.files[0]!.sha256,
    includes: described.files.slice(1),
  };
}

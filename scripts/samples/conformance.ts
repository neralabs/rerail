/**
 * What sample labels need of a pack: its conformance manifest's rules, to key labels by pack rule id (D4). Only the
 * rules of `packs/<pack>/conformance.json` count: the conformance check judges samples for them, so a label key from
 * another manifest (node's `candidates/`) would name no rule it loads.
 */
import { byText } from "./samples.js";

export interface PackRule {
  id: string;
  /** The rule document, relative to the pack directory. */
  policy: string;
  upstream: { tool: string; rule: string };
}

export interface PackConformance {
  pack: string;
  family: string;
  rules: PackRule[];
}

/** The pack rule ids porting one upstream rule of one tool (D4: each gets the upstream rule's labels). */
export function packRuleIds(conformance: PackConformance, tool: string, upstreamRule: string): string[] {
  return conformance.rules.filter((rule) => rule.upstream.tool === tool && rule.upstream.rule === upstreamRule).map((rule) => rule.id).sort(byText);
}

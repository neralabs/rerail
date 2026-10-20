// Types of record-plan.mjs, for its tests.
import type { ToolPack } from "../plan.mjs";

export type { ToolPack };

/** A tool-pack and the first reason it was chosen, or why it is not recorded. */
export interface Chosen extends ToolPack {
  why: string;
}

export interface RecordPlan {
  event: "label" | "dispatch";
  /** The labelled head; empty for a dispatch. */
  head: string;
  /** Every tool-pack, because git could not compare the commits. */
  every: boolean;
  legs: ToolPack[];
  chosen: Chosen[];
  dropped: Chosen[];
  warnings: string[];
  lines: string[];
}

export const ROOT: string;
export const NOT_COPIED: RegExp;
export function parseToolPacks(text: string): ToolPack[];
export function notCopiedOf(text: string): string[];
export function dispatchPlan(toolPacks: readonly ToolPack[], pack: string, tool: string): RecordPlan;
export function labelPlan(toolPacks: readonly ToolPack[], base: string, head: string, root?: string): RecordPlan;
export function commentBody(planned: RecordPlan): string;
export function writeOutputs(file: string, planned: Pick<RecordPlan, "legs">): void;

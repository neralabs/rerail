// Types of plan.mjs, for its tests.
export interface ToolPack {
  pack: string;
  tool: string;
}

export interface Leg extends ToolPack {
  shard?: number;
  shards?: number;
}

export interface Plan {
  legs: Leg[];
  toolPacks: string[];
  lines: string[];
}

export const ROOT: string;
export const SHARDS: Readonly<Record<string, number>>;
export const ROOTS: readonly string[];
export const REPORT_ONLY: readonly string[];
export function gatedToolPacks(root?: string): ToolPack[];
export function legsOf(toolPacks: readonly ToolPack[]): Leg[];
export function classify(path: string, toolPacks: readonly ToolPack[]): ToolPack[];
export function plan(toolPacks: readonly ToolPack[], paths?: readonly string[]): Plan;
export function changedPaths(from: string, to: string, root?: string): string[];
export function writeOutputs(file: string, planned: Pick<Plan, "legs" | "toolPacks">): void;

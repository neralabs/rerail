// Types of core.mjs's core-range exports, for conformance/pack-validation.ts.
export const HELD_OFF_SINCE: string;
export const LANGUAGE_SINCE: ReadonlyMap<string, string>;
export const RULE_KEY_SINCE: ReadonlyMap<string, string>;
export function heldOffRules(packJson: unknown): string[];
export const RERAIL_DOCUMENTS_SINCE: string;
export function rerailRules(packJson: unknown): string[];
export function shippedLanguages(directory: string): string[];
export function shippedRuleKeys(directory: string): string[];
export function coreFloor(heldOff: readonly string[], languages: readonly string[], ruleKeys: readonly string[], rerail: readonly string[]): string | undefined;
export function admitsRefusingCore(range: string, floor: string | undefined, coreVersion: string): boolean;
export function coreBeforeFloor(coreVersion: string, floor: string | undefined): boolean;
export function isOpenRange(range: string): boolean;
export const FIRST_CHECKING_CORE: string;
export function releasedCoreVersions(published: readonly string[]): string[];
export function publishedPackDocuments(packs: readonly { id: string; private?: boolean }[], root: string): Record<string, Record<string, unknown>>;
export function releasedCoreRefusals(documents: Record<string, Record<string, unknown>>): Record<string, Record<string, string[]>>;
export function acceptedFloor(
  versions: readonly string[],
  refusedBy: (version: string) => readonly string[],
  coreVersion: string,
): { floor: string | undefined; refusals: { version: string; rules: readonly string[] }[] };
export function newerFloor(a: string | undefined, b: string | undefined): string | undefined;
export function packRuleDocuments(directory: string): Record<string, unknown>;
export function coreRefusals(version: string, documents: Record<string, Record<string, unknown>>): Record<string, string[]>;
export function raisedCoreRange(floor: string): string;

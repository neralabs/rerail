// Types of core.mjs's core-range exports, for conformance/pack-validation.ts.
export const HELD_OFF_SINCE: string;
export const LANGUAGE_SINCE: ReadonlyMap<string, string>;
export function heldOffRules(packJson: unknown): string[];
export function shippedLanguages(directory: string): string[];
export function coreFloor(heldOff: readonly string[], languages: readonly string[]): string | undefined;
export function admitsRefusingCore(range: string, floor: string | undefined, coreVersion: string): boolean;
export function raisedCoreRange(floor: string): string;

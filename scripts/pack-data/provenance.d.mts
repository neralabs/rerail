// Types of provenance.mjs, for its TypeScript consumers (scripts/pack-data/derive.ts).
export type ProvenanceHow = "verbatim" | "extracted" | "adapted" | "renamed" | "recorded" | "authored";
export interface ProvenanceEntry {
  /** A project directory of packs/<pack>/licenses/, or "rerail". */
  source: string;
  from?: string;
  how: ProvenanceHow;
  renamedFrom?: string;
  detail?: string;
}
export type ProvenanceEntries = ReadonlyMap<string, ProvenanceEntry> | Readonly<Record<string, ProvenanceEntry>>;
export interface ProvenanceOptions {
  root?: string;
  pack: string;
  tool: string;
  entries: ProvenanceEntries;
  commits: ReadonlyMap<string, string> | Readonly<Record<string, string>>;
}
/** withOrigins' result (scripts/pack-data/eslint-expected.mjs), as variantEntries reads it. */
export interface RecordedUnits {
  files: readonly (string | readonly string[])[];
  units: Readonly<Record<string, { renamedFrom?: string }>>;
}

export const PROVENANCE: "provenance.json";
export const LICENSES: "LICENSES.md";
export const RERAIL: "rerail";
export function provenanceTexts(options: ProvenanceOptions): { provenance: string; licenses: string };
export function writeProvenance(options: ProvenanceOptions): void;
export function variantEntries(
  originals: ReadonlyMap<string, ProvenanceEntry>,
  recorded: RecordedUnits,
  override?: (path: string, entry: ProvenanceEntry) => ProvenanceEntry,
): Map<string, ProvenanceEntry>;
export function assertProvenanceWritten(directory: string, root?: string): void;

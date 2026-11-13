/**
 * What ESLint's and oxlint's readers share: a JSON config file at the root read as an object, a rule entry's severity,
 * and how an entry is named to the person.
 */
import { z } from "zod";
import { parseJsonc, readFormat } from "./formats/lint-config-formats.js";
import { NOT_A_FILE, readRootFile, type LintConfigLinter, type LinterNote } from "./linter-config.js";

/** A JSON object; arrays and `null` are not. Zod leaves out `__proto__` keys. */
export const jsonObject = z.record(z.string(), z.unknown());
export const configFile = z.looseObject({ rules: z.unknown().optional(), overrides: z.unknown().optional() });
export type ConfigFile = z.infer<typeof configFile>;

/** A JSONC config file at the root as an object; undefined (with a note when present) otherwise. */
export function jsoncFile(root: string, name: string, linter: LintConfigLinter, notes: LinterNote[]): ConfigFile | undefined {
  const read = readRootFile(root, name);
  if (!read.present) return undefined;
  const { text } = read;
  if (text === undefined) {
    notes.push({ linter, text: `${name} is not read: ${NOT_A_FILE}` });
    return undefined;
  }
  const parsed = readFormat(() => parseJsonc(text));
  const config = parsed.ok ? configFile.safeParse(parsed.value) : undefined;
  if (config?.success === true) return config.data;
  notes.push({ linter, text: `${name} is not read: it is not a JSON object (Rerail reads JSON lint configs only).` });
  return undefined;
}

/** How a rule entry's severity is read: a flat config's, the eslintrc loader's (any case), oxlint's. */
export type SeverityReading = "eslint" | "eslintrc" | "oxlint";

export interface Severity {
  readonly off: boolean;
  readonly options: unknown[];
}

/** Per reading, the levels that turn a rule off and on. */
const LEVELS: { readonly [K in SeverityReading]: { readonly off: readonly unknown[]; readonly on: readonly unknown[] } } = {
  eslint: { off: ["off", 0], on: ["warn", "error", 1, 2] },
  eslintrc: { off: ["off", 0], on: ["warn", "error", 1, 2] },
  oxlint: { off: ["off", "allow", 0], on: ["warn", "error", "deny", 1, 2] },
};

/**
 * A rule entry's severity: `off`, `on`, or undefined when it is not one the linter takes; with the options after it.
 * `eslint` is a flat config's, `eslintrc` the eslintrc loader's, which takes a severity string in any case.
 */
export function severityOf(value: unknown, linter: SeverityReading): Severity | undefined {
  const [written, ...options] = Array.isArray(value) ? value : [value];
  const level: unknown = linter === "eslintrc" && typeof written === "string" ? written.toLowerCase() : written;
  if (LEVELS[linter].off.includes(level)) return { off: true, options };
  return LEVELS[linter].on.includes(level) ? { off: false, options } : undefined;
}

const MAX_VALUE_TEXT = 60;

/** A rule entry as the person reads it: `.eslintrc.json: "no-console": "off"`. */
export function entryText(file: string, key: string, value: unknown): string {
  const text = JSON.stringify(value);
  return `${file}: ${JSON.stringify(key)}: ${text.length > MAX_VALUE_TEXT ? `${text.slice(0, MAX_VALUE_TEXT - 1)}…` : text}`;
}

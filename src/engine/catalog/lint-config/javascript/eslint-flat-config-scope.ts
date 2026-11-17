/**
 * Where a flat config's rule entry applies, from the `files` and `ignores` of the config objects it is in: whole folders
 * where every pattern names one, else some files, with the globs Rerail matches as ESLint does where it can.
 */
import type { EslintPresetEntry } from "../generated/eslint-presets.generated.js";
import type { FilePatterns, FlatConfigScope } from "./eslint-flat-config.js";

/** `<folder>/**` or `<folder>/**\/*`: every file under one folder. A `./` start is allowed. */
export function folderOfGlob(pattern: string): string | undefined {
  const match = /^(?:\.\/)?((?:[A-Za-z0-9_@.-]+\/)*[A-Za-z0-9_@.-]+)\/\*\*(?:\/\*)?$/u.exec(pattern);
  if (match === null) return undefined;
  const folder = match[1]!;
  return folder.split("/").some(part => part === "." || part === "..") ? undefined : folder;
}

/** A segment of a glob Rerail matches as minimatch does: names, `*`, `?`, and `{a,b}` alternatives of names. */
const EXACT_SEGMENT = /^(?:[\p{L}\p{N}_@.+*?-]|\{[\p{L}\p{N}_@.+-]*(?:,[\p{L}\p{N}_@.+-]*)+\})+$/u;
/** A segment that is, or has an alternative that is, `.` or `..`, which minimatch takes as a path step. */
const DOT_SEGMENT = /(?:^|[{,])\.\.?(?:[,}]|$)/u;

/**
 * `glob` as the pack matcher (`globPattern`) matches it the same as ESLint's minimatch (`dot: true`), its leading `./`
 * dropped as ESLint drops it; undefined for any other: a character class, an extglob, an escape, a `.`, `..` or empty
 * segment, or a brace minimatch expands otherwise (`{a}`, `{1..3}`, nested, with a glob inside).
 */
function exactGlob(glob: unknown): string | undefined {
  if (typeof glob !== "string") return undefined;
  const relative = glob.startsWith("./") ? glob.slice(2) : glob;
  return relative.split("/").every(segment => EXACT_SEGMENT.test(segment) && !DOT_SEGMENT.test(segment)) ? relative : undefined;
}

/**
 * `files` less `ignores` (JSON) as globs, where each is a list of globs Rerail matches files against as ESLint does
 * (`exactGlob`): no `files` is every file; a nested list (every glob of which must match), a negated glob or one
 * Rerail cannot match as ESLint does is not read.
 */
function filePatterns(files: unknown, ignores: unknown): FilePatterns | undefined {
  const globs = (value: unknown, none: readonly string[]): readonly string[] | undefined => {
    if (value === undefined) return none;
    const exact = Array.isArray(value) ? value.map(exactGlob) : [undefined];
    return exact.every(glob => glob !== undefined) ? exact as string[] : undefined;
  };
  const included = globs(files, ["**/*"]);
  const excluded = globs(ignores, []);
  return included === undefined || excluded === undefined ? undefined : { files: [included], ignores: excluded };
}

/**
 * The scope of `files` patterns less `ignores` (JSON; `files` as written, `text`) for an element every file gets
 * otherwise: whole folders where every pattern names one and none is ignored, or some files.
 */
export function filesScope(files: unknown, text: string, ignores?: unknown): FlatConfigScope {
  const folders = Array.isArray(files) && ignores === undefined ? files.map(pattern => (typeof pattern === "string" ? folderOfGlob(pattern) : undefined)) : [];
  const patterns = filePatterns(files, ignores);
  const read = patterns === undefined ? {} : { patterns };
  return folders.length > 0 && folders.every(folder => folder !== undefined)
    ? { kind: "folders", folders: folders as string[], files: text, ...read }
    : { kind: "some", files: text, ...read };
}

/**
 * Scope `scope`, limited to some files, narrowed by `files` less `ignores` (JSON), named `text`: the files both match,
 * as `defineConfig` extends an element's `files` and `ignores` with those of the configs it extends.
 */
export function narrowScope(scope: Exclude<FlatConfigScope, { kind: "all" }>, files: unknown, ignores: unknown, text: string): FlatConfigScope {
  const inner = filePatterns(files, ignores);
  const outer = scope.patterns;
  if (outer === undefined || inner === undefined) return { kind: "some", files: text };
  return { kind: "some", files: text, patterns: { files: [...outer.files, ...inner.files], ignores: [...outer.ignores, ...inner.ignores] } };
}

/** Where a shared config's entry applies in an element for `scope`: its own `files` and `ignores` narrow it. */
export function presetScope(entry: EslintPresetEntry, scope: FlatConfigScope): FlatConfigScope {
  if (entry.files === undefined && entry.ignores === undefined) return scope;
  const text = entry.files === undefined ? "some files" : JSON.stringify(entry.files);
  return scope.kind === "all" ? filesScope(entry.files, text, entry.ignores) : narrowScope(scope, entry.files, entry.ignores, `${scope.files} and ${text}`);
}

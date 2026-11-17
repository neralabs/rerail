/**
 * Biome's globs, as Biome 2.5.15 reads them (`validate_glob`, over globset with `literal_separator`) turned into regular
 * expressions, and matched against a repository's paths as Biome matches them: the last glob matching a path decides,
 * and a file is held only where Biome visits every folder above it.
 */

/** A list of Biome globs, each a pattern and whether it is negated (`!` or `!!`). */
export type Globs = readonly { readonly negated: boolean; readonly pattern: RegExp }[];

/** A glob's piece at one index: its regular expression's source, how many characters after it it takes, and how it nests alternates. */
interface GlobToken {
  readonly source: string;
  readonly skip: number;
  readonly depth: number;
}

const literal = (text: string, skip = 0): GlobToken => ({ source: text.replace(/[$()*+./?[\\\]^{|}]/gu, String.raw`\$&`), skip, depth: 0 });

/** `{`, `,` and `}` by the alternates' depth they are at: Biome refuses nested alternates and a `}` closing none. */
const ALTERNATES: Readonly<Record<string, GlobToken>> = {
  "{0": { source: "(?:", skip: 0, depth: 1 },
  ",0": literal(","),
  ",1": { source: "|", skip: 0, depth: 0 },
  "}1": { source: ")", skip: 0, depth: -1 },
};
/** The characters `\` escapes in a Biome glob; any other after it, or none, Biome refuses. */
const ESCAPED = [..."!*?{}[]\\"];

/** Whether `**` at `index` of `glob` starts a segment, not right after another `**` (`validate_glob`'s `allow_globstar`). */
const segmentStart = (glob: string, index: number): boolean => index === 0 || (glob[index - 1] === "/" && !glob.slice(0, index).endsWith("**/"));

/** `*` at `index` of `glob`, within a segment; or `**`, any segments, where it is a whole segment. */
function starToken(glob: string, index: number): GlobToken | undefined {
  if (glob[index + 1] !== "*") return { source: "[^/]*", skip: 0, depth: 0 };
  const after = glob[index + 2];
  if (!segmentStart(glob, index) || (after !== undefined && after !== "/")) return undefined;
  return after === undefined ? { source: ".*", skip: 1, depth: 0 } : { source: "(?:[^/]*/)*", skip: 2, depth: 0 };
}

/** The token at `index` of `glob`, `depth` alternates in; undefined where Biome refuses it. */
function globToken(glob: string, index: number, depth: number): GlobToken | undefined {
  const character = glob[index]!;
  if (character === "\\") return ESCAPED.includes(glob.charAt(index + 1)) ? literal(glob.charAt(index + 1), 1) : undefined;
  if (character === "*") return starToken(glob, index);
  if ("?[]".includes(character)) return undefined;
  return "{,}".includes(character) ? ALTERNATES[`${character}${depth}`] : literal(character);
}

/**
 * Glob `glob` (without its `!`) as a regular expression's source, as Biome 2.5.15 reads it (`validate_glob`,
 * crates/biome_glob/src/lib.rs l.638-704, over globset with `literal_separator`); undefined where Biome refuses it.
 */
export function globSource(glob: string): string | undefined {
  let source = "";
  let depth = 0;
  for (let index = 0; index < glob.length; index += 1) {
    const token = globToken(glob, index, depth);
    if (token === undefined) return undefined;
    source += token.source;
    depth += token.depth;
    index += token.skip;
  }
  return depth === 0 ? source : undefined;
}

/** Whether `path` matches `globs`: the last glob matching it decides, `fallback` where none does (`matches_with_exceptions_or`). */
export function matchesGlobs(globs: Globs, path: string, fallback: boolean): boolean {
  const last = globs.findLast(glob => glob.pattern.test(path));
  return last === undefined ? fallback : !last.negated;
}

/** Whether `globs` hold file `path`: Biome visits no folder holding it that they leave out (`is_dir_included`), and the file matches (`is_file_included`). */
export function holdsFile(globs: Globs, path: string): boolean {
  const segments = path.split("/");
  const folders = segments.slice(1).map((_, index) => segments.slice(0, index + 1).join("/"));
  return folders.every(folder => matchesGlobs(globs, folder, true)) && matchesGlobs(globs, path, false);
}

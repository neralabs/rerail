/**
 * Repository-relative globs as Rerail reads them, matched by an automaton rather than a backtracking regular expression.
 * Rule packs and Merge Assist scopes use it; peer scopes are other people's text, so matching must stay linear.
 */

/** How many characters the glob token at `index` spans: `**\/`, `**`, a whole `{…}` group, or one code point. */
function globTokenLength(glob: string, index: number): number {
  if (glob.startsWith("**/", index)) return 3;
  if (glob.startsWith("**", index)) return 2;
  if (glob[index] !== "{") return (glob.codePointAt(index) ?? 0) > 0xffff ? 2 : 1;
  const close = glob.indexOf("}", index);
  if (close === -1) throw new Error(`Unclosed { in glob ${glob}`);
  const nested = glob.indexOf("{", index + 1);
  if (nested !== -1 && nested < close) throw new Error(`Nested { in glob ${glob}: braces do not nest`);
  return close + 1 - index;
}

/** What one automaton step reads: one given character, any character but `/`, or any character but a line break. */
type GlobRead = { readonly char: string } | "segment" | "line";

interface GlobState {
  /** States reached without reading a character. */
  readonly empty: number[];
  readonly reads: Array<{ readonly read: GlobRead; readonly to: number }>;
}

function reads(read: GlobRead, char: string): boolean {
  if (read === "segment") return char !== "/";
  if (read === "line") return char !== "\n" && char !== "\r" && char !== "\u2028" && char !== "\u2029";
  return read.char === char;
}

/** A nondeterministic automaton for a list of globs, run over a path in one pass, so no glob can make a match backtrack. */
class GlobAutomaton {
  readonly #states: GlobState[] = [];
  readonly #start: number;
  readonly #accept: number;

  constructor(globs: readonly string[]) {
    this.#accept = this.#state();
    this.#start = this.#state();
    const firsts = globs.map(glob => this.#glob(glob, this.#accept));
    this.#states[this.#start]!.empty.push(...firsts);
  }

  #state(): number {
    this.#states.push({ empty: [], reads: [] });
    return this.#states.length - 1;
  }

  #reading(read: GlobRead, to: number): number {
    const state = this.#state();
    this.#states[state]!.reads.push({ read, to });
    return state;
  }

  /** `[^/]*` (or `.*` reading `line`): stay on each character read, or go on to `next`. */
  #repeat(read: GlobRead, next: number): number {
    const state = this.#state();
    this.#states[state]!.empty.push(next);
    this.#states[state]!.reads.push({ read, to: state });
    return state;
  }

  /** The first state of `glob`, whose match continues at `next`. Tokens are built last first, each leading to the next. */
  #glob(glob: string, next: number): number {
    const tokens: Array<{ token: string; index: number }> = [];
    for (let index = 0; index < glob.length; index += tokens.at(-1)!.token.length) {
      tokens.push({ token: glob.slice(index, index + globTokenLength(glob, index)), index });
    }
    return tokens.reduceRight((after, { token, index }) => this.#token(token, index, glob, after), next);
  }

  #token(token: string, index: number, glob: string, next: number): number {
    const segmentStart = index === 0 || glob[index - 1] === "/";
    if (token === "**/") return segmentStart ? this.#folders(next) : this.#repeat("segment", this.#reading({ char: "/" }, next));
    if (token === "**") return this.#repeat(segmentStart && index + token.length === glob.length ? "line" : "segment", next);
    if (token === "*") return this.#repeat("segment", next);
    if (token === "?") return this.#reading("segment", next);
    if (token.startsWith("{")) {
      const state = this.#state();
      this.#states[state]!.empty.push(...token.slice(1, -1).split(",").map(alternative => this.#glob(alternative, next)));
      return state;
    }
    return this.#reading({ char: token }, next);
  }

  /** `(?:[^/]*\/)*`: any number of whole folders, each ending in `/`, then `next`. */
  #folders(next: number): number {
    const between = this.#state();
    const inside = this.#state();
    this.#states[between]!.empty.push(next);
    this.#states[between]!.reads.push({ read: { char: "/" }, to: between }, { read: "segment", to: inside });
    this.#states[inside]!.reads.push({ read: { char: "/" }, to: between }, { read: "segment", to: inside });
    return between;
  }

  /** `states` and every state reached from them without reading, each once: `seen` holds `mark` for each state taken. */
  #closure(states: readonly number[], seen: Uint32Array, mark: number): number[] {
    const reached: number[] = [];
    const pending = [...states];
    while (pending.length > 0) {
      const state = pending.pop()!;
      if (seen[state] === mark) continue;
      seen[state] = mark;
      reached.push(state);
      pending.push(...this.#states[state]!.empty);
    }
    return reached;
  }

  /** Whether some glob matches all of `path`: time grows with the path's length times the automaton's size, never more. */
  test(path: string): boolean {
    const seen = new Uint32Array(this.#states.length);
    let mark = 1;
    let current = this.#closure([this.#start], seen, mark);
    for (const char of path) {
      const next: number[] = [];
      for (const state of current) {
        for (const { read, to } of this.#states[state]!.reads) if (reads(read, char)) next.push(to);
      }
      if (next.length === 0) return false;
      current = this.#closure(next, seen, ++mark);
    }
    return current.includes(this.#accept);
  }
}

/** A compiled list of globs; `test` tells whether a path matches any of them. */
export interface GlobMatcher {
  test(path: string): boolean;
}

/**
 * A matcher for any of `globs`: `**` spans directories when it is a whole segment (`**\/x` also matches `x`) and acts as
 * `*` otherwise (`**.ts` matches `a.ts`, not `src/a.ts`), as in minimatch; `*` and `?` stay inside one segment; `{a,b}`
 * alternates, without nesting. A malformed glob throws; an empty list matches nothing. Matching runs an automaton in one
 * pass over the path rather than a backtracking regular expression, so globs from other people (Merge Assist's peer
 * scopes) cannot make a match take more than linear time in the path's length.
 */
export function globPattern(globs: readonly string[]): GlobMatcher {
  return new GlobAutomaton(globs);
}

import { fileURLToPath } from "node:url";
import { run } from "./upstream-source.js";

/** The worker that reads Python for the recorders, run isolated under CPython 3.14 (python_toolkit.py). */
const WORKER = fileURLToPath(new URL("./python_toolkit.py", import.meta.url));

export interface PythonLiterals {
  /** Names the source checks: identifier-shaped and dotted string literals, quoted words, words of its regexes. */
  names: string[];
  /** The patterns it passes to `re` functions. */
  regexes: string[];
  /** The modules it imports by absolute name. */
  imports: string[];
}

/** How the recorders read Python. Ported from nera-rule-validation's src/validation/languages/python.ts. */
export interface PythonToolkit {
  keywords: string[];
  /**
   * Words Python itself gives meaning to inside comments: PEP 263's `coding` declaration and PEP 484's `type:` and
   * `type: ignore` comments, which CPython's parser reads. Linter directives such as `noqa` belong to the linter.
   */
  commentVocabulary: string[];
  /** Every word of every token but numbers, over all the texts. */
  words(texts: string[]): string[];
  literals(files: string[]): PythonLiterals[];
}

/** The toolkit run by `python` (CPython 3.14), isolated from the environment, user site and working directory. */
export function createPythonToolkit(python: string): PythonToolkit {
  const ask = <T>(request: object): T => JSON.parse(run(python, ["-I", "-B", WORKER], { input: JSON.stringify(request) })) as T;
  return {
    keywords: ask<string[]>({ op: "keywords" }),
    commentVocabulary: ["coding", "ignore", "type"],
    words: texts => (texts.length === 0 ? [] : ask<string[]>({ op: "words", texts })),
    literals: files => (files.length === 0 ? [] : ask<PythonLiterals[]>({ op: "literals", files })),
  };
}

/** Python's `re` flag bits and their JavaScript equivalents; any other bit has none. */
const PYTHON_FLAGS: [number, string][] = [[2, "i"], [8, "m"], [16, "s"]];
const IGNORED_FLAGS = 32 | 256; // re.UNICODE is the default for str patterns; re.ASCII narrows \w, which JavaScript's \w already is.

/**
 * A compiled Python regex as a JavaScript one, when it has a faithful translation: named groups renamed, and no flag
 * JavaScript lacks (verbose, locale). Undefined otherwise; an implementation regex that cannot run explains nothing.
 */
export function pythonRegex(pattern: string, flags: number): RegExp | undefined {
  let remaining = flags & ~IGNORED_FLAGS;
  let jsFlags = "";
  for (const [bit, flag] of PYTHON_FLAGS) {
    if ((remaining & bit) !== 0) {
      jsFlags += flag;
      remaining &= ~bit;
    }
  }
  if (remaining !== 0 || /\(\?[aiLmsux-]+[):]/u.test(pattern) || /\\[AZ]/u.test(pattern)) return undefined;
  try {
    return new RegExp(pattern.replaceAll("(?P<", "(?<").replace(/\(\?P=(\w+)\)/gu, "\\k<$1>"), jsFlags);
  } catch {
    return undefined;
  }
}

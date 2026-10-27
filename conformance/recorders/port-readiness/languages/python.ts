import { fileURLToPath } from "node:url";

import { runChecked, type CommandRunner } from "../../../process.js";
import type { ParserVerdict } from "../types.js";
import { uvRuntime } from "../runtimes.js";

/** The worker that reads Python for the gates, run in the pinned environment of runtimes/python. */
const WORKER = fileURLToPath(new URL("./python_toolkit.py", import.meta.url));

export interface PythonLiterals {
  /** Names the source checks: identifier-shaped and dotted string literals, quoted words, words of its regexes. */
  names: string[];
  /** The patterns it passes to `re` functions. */
  regexes: string[];
  /** The modules it imports by absolute name. */
  imports: string[];
}

/** How the gates read Python: one pinned CPython, the reference implementation, with libcst as an independent parser. */
export interface PythonToolkit {
  /** The pinned interpreter, e.g. `Python 3.14.8`. */
  version: string;
  keywords: Set<string>;
  /**
   * Words Python itself gives meaning to inside comments: PEP 263's `coding` declaration and PEP 484's `type:` and
   * `type: ignore` comments, which CPython's parser reads. Linter directives such as `noqa` belong to the linter.
   */
  commentVocabulary: Set<string>;
  /** Per text, every word of every token but numbers. */
  words(texts: string[]): Promise<string[][]>;
  literals(files: string[]): Promise<PythonLiterals[]>;
  verdicts(files: { path: string; source: string }[]): Promise<ParserVerdict[][]>;
}

export async function createPythonToolkit(cacheRoot: string, runner: CommandRunner): Promise<PythonToolkit> {
  const runtime = await uvRuntime("python", cacheRoot, runner);
  const ask = async <T>(request: object): Promise<T> =>
    JSON.parse(await runChecked(runner, [runtime.python, "-I", WORKER], { input: JSON.stringify(request) })) as T;
  return {
    version: runtime.version,
    keywords: new Set(await ask<string[]>({ op: "keywords" })),
    commentVocabulary: new Set(["coding", "type", "ignore"]),
    words: async (texts) => (texts.length === 0 ? [] : await ask<string[][]>({ op: "words", texts })),
    literals: async (files) => (files.length === 0 ? [] : await ask<PythonLiterals[]>({ op: "literals", files })),
    verdicts: async (files) => (files.length === 0 ? [] : await ask<ParserVerdict[][]>({ op: "verdicts", files })),
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

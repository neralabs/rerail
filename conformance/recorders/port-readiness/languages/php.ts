import { fileURLToPath } from "node:url";

import { runChecked, type CommandRunner } from "../../../process.js";
import type { ParserVerdict } from "../types.js";
import { composerRuntime, PHP_INI, type PhpRuntime } from "../runtimes.js";

/** The worker that reads PHP for the gates, run on the pinned interpreter with runtimes/php's vendor/ (nikic/php-parser). */
const WORKER = fileURLToPath(new URL("./php_toolkit.php", import.meta.url));

export interface PhpLiterals {
  /**
   * Names the source checks: identifier-shaped string literals, the segments of qualified names, paths and dotted or
   * `::`-joined strings (`Foo\Bar`, `Foo::bar`, `/var/tmp`), quoted words inside strings, and the words of the regexes it
   * passes to `preg_*` functions. Docblocks and comments are not code and are left out.
   */
  names: string[];
  /** The patterns it passes to `preg_*` functions as string literals, as PHP writes them (delimiters and modifiers included). */
  regexes: string[];
  /** The fully qualified names it imports with `use` (classes, functions and constants alike), without a leading `\`. */
  imports: string[];
  /** The fully qualified names of the classes it declares, and of the classes they extend. */
  classes: { name: string; extends: string | null }[];
}

/**
 * How the gates read PHP: the pinned interpreter (runtimes/php's `.php-version`), which tokenizes and lints with PHP's
 * own compiler, and nikic/php-parser as an independent parser. Every PHP-reading adapter (phpcs, phpstan, psalm, and
 * the Blade adapters for the PHP a template embeds) builds on it.
 */
export interface PhpToolkit {
  /** The pinned interpreter and runtimes/php, for drivers that need PHP-Parser too. */
  runtime: PhpRuntime;
  /** The interpreter, e.g. `PHP 8.5.11`, as the report names it. */
  version: string;
  /** nikic/php-parser's installed version, e.g. `5.9.0`. */
  parser: string;
  /**
   * PHP's keywords, read off the interpreter's tokenizer (every word `token_get_all` reads as a token of its own, magic
   * constants included) and PHP's other reserved words (`true`, `false`, `null`, the scalar type names, `self`,
   * `parent`, and `this` of the pseudo-variable `$this`). PHP reads keywords in any case, so each is listed lower case
   * and upper case, the two spellings code uses.
   */
  keywords: Set<string>;
  /**
   * Words the PHP ecosystem gives meaning to inside comments: phpDoc tags (`@param`, `@return`, `@var`, …), the
   * `@phpstan-`, `@psalm-` and `@template` annotations the static analysers read, and PHP_CodeSniffer's `phpcs:`
   * directives and its older `@codingStandards…` annotations, each as the words a tag splits into.
   */
  commentVocabulary: Set<string>;
  /** Per text, every word of every PHP token but numbers (`$name` gives `name`); a text without `<?php` is all inline HTML. */
  words(texts: string[]): Promise<string[][]>;
  /** Per PHP file (an absolute path), what its code checks: string literals, `preg_*` patterns, imports and classes. */
  literals(files: string[]): Promise<PhpLiterals[]>;
  /**
   * Per source, two independent verdicts: `php -l` on the pinned interpreter (its parse errors are `syntax`, the
   * compile errors it reports after building the tree are `grammar`) and nikic/php-parser (`syntax` only, `grammar`
   * null: it checks nothing after it builds a tree).
   */
  verdicts(files: { path: string; source: string }[]): Promise<ParserVerdict[][]>;
}

/**
 * The comment tags the PHP ecosystem reads: phpDocumentor's standard tags, the `@phpstan-` and `@psalm-` tags of the
 * static analysers (PHPStan's and Psalm's documentation), PHPStan's and Psalm's inline directives, and PHP_CodeSniffer's
 * `phpcs:` directives and the `@codingStandards…` annotations it still honours. Not-overfit reads words, so each is
 * split into the words it is made of.
 */
const COMMENT_TAGS = [
  "@api", "@author", "@category", "@copyright", "@deprecated", "@example", "@filesource", "@final", "@global", "@ignore",
  "@inheritDoc", "@inheritdoc", "@internal", "@license", "@link", "@method", "@mixin", "@package", "@param", "@param-out",
  "@property", "@property-read", "@property-write", "@readonly", "@return", "@see", "@since", "@source", "@subpackage",
  "@template", "@template-covariant", "@template-contravariant", "@extends", "@implements", "@use", "@throws", "@todo",
  "@uses", "@used-by", "@var", "@version", "@pure", "@immutable", "@no-named-arguments", "@phpstan-param",
  "@phpstan-return", "@phpstan-var", "@phpstan-type", "@phpstan-import-type", "@phpstan-assert", "@phpstan-assert-if-true",
  "@phpstan-assert-if-false", "@phpstan-self-out", "@phpstan-this-out", "@phpstan-require-extends",
  "@phpstan-require-implements", "@phpstan-sealed", "@phpstan-pure", "@phpstan-impure", "@phpstan-allow-private-mutation",
  "@phpstan-ignore", "@phpstan-ignore-line", "@phpstan-ignore-next-line", "@psalm-param", "@psalm-return", "@psalm-var",
  "@psalm-type", "@psalm-import-type", "@psalm-assert", "@psalm-assert-if-true", "@psalm-assert-if-false",
  "@psalm-this-out", "@psalm-self-out", "@psalm-pure", "@psalm-mutation-free", "@psalm-external-mutation-free",
  "@psalm-immutable", "@psalm-readonly", "@psalm-suppress", "@psalm-taint-sink", "@psalm-taint-source",
  "@psalm-taint-escape", "@psalm-taint-unescape", "@psalm-taint-specialize", "@psalm-flow", "@psalm-check-type",
  "@psalm-consistent-constructor", "@psalm-seal-properties", "@psalm-seal-methods", "@psalm-yield", "@psalm-internal",
  "phpcs:ignore", "phpcs:ignoreFile", "phpcs:disable", "phpcs:enable", "phpcs:set", "@codingStandardsIgnoreFile",
  "@codingStandardsIgnoreStart", "@codingStandardsIgnoreEnd", "@codingStandardsIgnoreLine", "@codingStandardsChangeSetting",
];

export async function createPhpToolkit(cacheRoot: string, runner: CommandRunner): Promise<PhpToolkit> {
  const runtime = await composerRuntime("php", cacheRoot, runner);
  const ask = async <T>(request: object): Promise<T> =>
    JSON.parse(await runChecked(runner, runtime.command(WORKER, runtime.directory), { input: JSON.stringify(request), cwd: cacheRoot })) as T;
  const facts = await ask<{ keywords: string[]; parser: string }>({ op: "facts" });
  return {
    runtime,
    version: `PHP ${runtime.version}`,
    parser: facts.parser,
    keywords: new Set(facts.keywords),
    commentVocabulary: new Set(COMMENT_TAGS.flatMap((tag) => tag.match(/[A-Za-z_]\w*/gu) ?? [])),
    words: async (texts) => (texts.length === 0 ? [] : await ask<string[][]>({ op: "words", texts })),
    literals: async (files) => (files.length === 0 ? [] : await ask<PhpLiterals[]>({ op: "literals", files })),
    verdicts: async (files) => (files.length === 0 ? [] : await ask<ParserVerdict[][]>({ op: "verdicts", files, ini: PHP_INI })),
  };
}

/** PCRE modifiers with a JavaScript flag; `u` is always set, as PHP code points read like JavaScript's `u` mode. */
const PCRE_FLAGS: Record<string, string> = { i: "i", m: "m", s: "s", u: "" };

/** The closing delimiter of each PCRE bracket delimiter. */
const CLOSING: Record<string, string> = { "(": ")", "{": "}", "[": "]", "<": ">" };

/**
 * A PCRE pattern as PHP writes it (`/^foo$/i`, `#a|b#`, `{x}u`) as a JavaScript regex, when it has a faithful
 * translation: modifiers JavaScript has (`i`, `m`, `s`, `u`) and no PCRE-only syntax (possessive quantifiers, atomic
 * groups, inline options, `\A`, `\z`, `\Z`, `\h`, POSIX classes, recursion). Undefined otherwise: an implementation
 * regex that cannot run explains nothing.
 */
export function phpRegex(pattern: string): RegExp | undefined {
  const trimmed = pattern.replace(/^\s+/u, "");
  const open = trimmed[0];
  if (open === undefined || /[A-Za-z0-9\\\s]/u.test(open)) return undefined;
  const close = CLOSING[open] ?? open;
  const end = trimmed.lastIndexOf(close);
  if (end <= 0) return undefined;
  const body = trimmed.slice(1, end);
  const modifiers = trimmed.slice(end + 1);
  let flags = "u";
  for (const modifier of modifiers) {
    const flag = PCRE_FLAGS[modifier];
    if (flag === undefined) return undefined;
    if (!flags.includes(flag)) flags += flag;
  }
  if (/\(\?[>|#]|\(\?[A-Za-z-]+[):]|\(\?R\)|\(\?\d|[*+?}]\+|\\[AzZhHKRX]|\[:[a-z]+:\]|\\[pP][A-Z][a-z&]?(?![{])/u.test(body)) return undefined;
  try {
    return new RegExp(body, flags);
  } catch {
    return undefined;
  }
}

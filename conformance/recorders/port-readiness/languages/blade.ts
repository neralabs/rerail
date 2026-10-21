import { fileURLToPath } from "node:url";

import { runChecked, type CommandRunner } from "../../../port-readiness/process.js";
import type { ParserVerdict } from "../../../port-readiness/types.js";
import { composerRuntime, type PhpRuntime } from "../runtimes.js";
import { createPhpToolkit, type PhpLiterals, type PhpToolkit } from "./php.js";

/** The worker that reads Blade for the gates, run on the pinned interpreter with runtimes/blade's vendor/ (laravel/framework, stillat/blade-parser). */
const WORKER = fileURLToPath(new URL("./blade_toolkit.php", import.meta.url));

/**
 * How the gates read Blade templates: Laravel's own BladeCompiler, whose compiled PHP the pinned interpreter lints, and
 * stillat/blade-parser as an independent parser. Neither shares code with the engine's port of Forte
 * (native/src/code_rule_dsl/blade/forte), so when the engine cannot parse a template they say whether the template is
 * the problem. The PHP a template embeds is PHP, read by the PHP toolkit.
 */
export interface BladeToolkit {
  /** The pinned interpreter and runtimes/blade, for drivers that need laravel/framework or stillat/blade-parser too. */
  runtime: PhpRuntime;
  /** The PHP toolkit the compiled views are linted with and implementation files are read with. */
  php: PhpToolkit;
  /** The parsers and their versions, e.g. `blade-compiler 13.34.0, stillat/blade-parser 2.1.0, PHP 8.5.11`. */
  version: string;
  /**
   * PHP's keywords and the directives BladeCompiler compiles, read off the compiler: each `compile<Name>` method it
   * calls for `@<name>` and the `@verbatim`/`@php` blocks it lifts out first, spelled `includeIf` and `includeif`, as
   * Blade reads a directive in either. Blade's echo and comment delimiters (`{{ }}`, `{!! !!}`, `{{-- --}}`) have no words.
   */
  keywords: Set<string>;
  /** PHP's: a Blade comment (`{{-- --}}`) is never read, and the PHP comments a template embeds are PHP's. */
  commentVocabulary: Set<string>;
  /**
   * Per template, every word of its text but numbers. A template is HTML with directives and echoes, which PHP's
   * tokenizer reads as one stretch of inline HTML, and its compiled PHP holds words the compiler wrote, so the template
   * itself is read with a regex, leaving out PHP number literals as PHP's tokenizer would.
   */
  words(texts: string[]): Promise<string[][]>;
  /** Per PHP file (an absolute path): the PHP toolkit's literals, as upstream's implementations are PHP. */
  literals(files: string[]): Promise<PhpLiterals[]>;
  /**
   * Per template, two independent verdicts. `blade-compiler <version>`: BladeCompiler::compileString, then `php -l` on
   * the compiled PHP on the pinned interpreter. A template the compiler throws on, and `php -l`'s parse errors, are
   * `syntax`; the compile errors `php -l` reports after building a tree are `grammar`, at lines of the compiled view.
   * `stillat/blade-parser <version>`: its document parser's fatal errors are `syntax`, and `grammar` is null, as it
   * checks nothing after it builds the document.
   */
  verdicts(files: { path: string; source: string }[]): Promise<ParserVerdict[][]>;
}

/** What the worker returns per template: the compiled PHP (null when the compiler threw, with what it threw) and stillat/blade-parser's verdict. */
interface Compiled {
  compiled: string | null;
  errors: string[];
  parser: ParserVerdict;
}

/** A PHP number literal (`42`, `0x1F`, `0b101`, `0o17`, `1_000`, `1.5e3`). */
const NUMBER = /\b(?:0[xob][\da-f_]+|\d[\d_]*(?:\.\d*)?(?:e[+-]?\d+)?)\b/giu;
const WORD = /[A-Za-z_]\w*/gu;

export async function createBladeToolkit(cacheRoot: string, runner: CommandRunner, php?: PhpToolkit): Promise<BladeToolkit> {
  const reader = php ?? (await createPhpToolkit(cacheRoot, runner));
  const runtime = await composerRuntime("blade", cacheRoot, runner);
  const ask = async <T>(request: object): Promise<T> =>
    JSON.parse(await runChecked(runner, runtime.command(WORKER, runtime.directory), { input: JSON.stringify(request), cwd: cacheRoot })) as T;
  const facts = await ask<{ compiler: string; parser: string; directives: string[] }>({ op: "facts" });
  const compiler = `blade-compiler ${facts.compiler}`;
  return {
    runtime,
    php: reader,
    version: `${compiler}, stillat/blade-parser ${facts.parser}, ${reader.version}`,
    keywords: new Set([...reader.keywords, ...facts.directives]),
    commentVocabulary: reader.commentVocabulary,
    words: async (texts) => texts.map((text) => text.replace(NUMBER, " ").match(WORD) ?? []),
    literals: async (files) => await reader.literals(files),
    verdicts: async (files) => {
      if (files.length === 0) return [];
      const read = await ask<Compiled[]>({ op: "verdicts", files });
      const views = read.flatMap((file, index) => (file.compiled === null ? [] : [{ path: files[index]!.path, source: file.compiled }]));
      const linted = (await reader.verdicts(views)).map((verdicts) => {
        const lint = verdicts.find((verdict) => verdict.parser.startsWith("php -l "));
        if (lint === undefined) throw new Error(`the PHP toolkit returned no php -l verdict, only ${verdicts.map((verdict) => verdict.parser).join(", ")}`);
        // php -l reads the compiled view, whose lines are not the template's.
        const [syntax = [], grammar = []] = [lint.syntax, lint.grammar ?? []].map((messages) =>
          messages.map((message) => message.replace(/\(line (\d+)\)$/u, "(line $1 of the compiled view)")));
        return { syntax, grammar };
      });
      let next = 0;
      return read.map((file) => [
        { parser: compiler, ...(file.compiled === null ? { syntax: file.errors, grammar: [] } : linted[next++]!) },
        file.parser,
      ]);
    },
  };
}

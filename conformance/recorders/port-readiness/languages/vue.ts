import type { ParserVerdict } from "../../../port-readiness/types.js";
import { parserVerdicts, type IndependentParsers } from "./ecmascript.js";

/**
 * How the gates read Vue single-file components: @vue/compiler-sfc, from the runtime gates.yaml's `eslint.vue.runtime`
 * pins, splits a `.vue` file into its blocks, and the ECMAScript parsers read its scripts.
 */

interface SfcBlock { content: string; lang?: string }

/**
 * What @vue/compiler-sfc's parse returns, as the gates read it. An error with a `code` is the template parser's (an
 * unclosed element); one without is a check of the component it built (no `<template>` or `<script>` at all, two
 * `<script>` elements).
 */
export interface SfcParseResult { errors: { message: string; code?: unknown }[]; descriptor: { script: SfcBlock | null; scriptSetup: SfcBlock | null } }

/** The part of @vue/compiler-sfc's API the gates use. */
export interface SfcCompiler {
  version: string;
  parse(source: string, options: { filename: string }): SfcParseResult;
}

/** The path a `<script lang>` block is read at: TypeScript for `ts` and `tsx`, JSX for `jsx` and `tsx`, JavaScript otherwise. */
const SCRIPT_PATHS: Record<string, string> = { ts: "script.ts", tsx: "script.tsx", jsx: "script.jsx" };

/** A module-level declaration of vue-eslint-parser's bundle, as the text of its initializer up to the line closing it. */
function declaration(parserSource: string, opening: string, closing: string): string {
  const start = parserSource.indexOf(opening);
  const end = start === -1 ? -1 : parserSource.indexOf(closing, start);
  if (end === -1) throw new Error(`vue-eslint-parser's bundle declares no \`${opening.trim()}\`: the Vue vocabulary must be read again for this version`);
  return parserSource.slice(start + opening.length, end + closing.length);
}

/**
 * The words the Vue template language defines, as vue-eslint-parser (the parser upstream lints templates with, from the
 * tool's pinned runtime) reads them, which are syntax wherever a rule spells them: its directive syntax
 * (`DIRECTIVE_NAME`, the `v-` prefix and the `.`, `:`, `@` and `#` shorthands), the built-in directive names it
 * knows (`BUILTIN_DIRECTIVES`), and the names of the HTML character references its tokenizer (`entitySets`, the HTML
 * standard's table) decodes to whitespace alone (`&nbsp;`, `&Tab;`, `&NewLine;`), whitespace as
 * `String.prototype.trim` strips it. Only those: a rule spells a reference name where upstream sees the decoded text
 * and trims it (no-child-content's whitespace-only children), and the table's other names are English words (`check`,
 * `star`, `copy`, `and`) that would explain a fixture's word in every rule of the tool-pack. None of the three is
 * exported, so they are read from the bundle's source, and a version that declares them otherwise fails the recording.
 */
export function templateVocabulary(parserSource: string): string[] {
  const directiveName = declaration(parserSource, "const DIRECTIVE_NAME = /", "/u;").slice(0, -"/u;".length);
  const builtins = JSON.parse(declaration(parserSource, "const BUILTIN_DIRECTIVES = new Set(", "\n]").trim()) as string[];
  const entitySets = JSON.parse(declaration(parserSource, "const entitySets = ", "\n]")) as { entities: Record<string, number[]> }[];
  return [
    ...(directiveName.match(/[A-Za-z]\w*/gu) ?? []),
    ...builtins.flatMap((name) => name.split("-")),
    ...entitySets.flatMap((set) => Object.entries(set.entities)
      .filter(([, codePoints]) => String.fromCodePoint(...codePoints).trim() === "")
      .map(([name]) => name.replace(/;$/u, ""))),
  ];
}

/**
 * Independent parsers' verdicts on a file of a tool-pack that lints Vue single-file components. Every file gets the
 * same parsers, as a recording requires: the ECMAScript parsers of `parserVerdicts`, each named with the component
 * parser that feeds it. A `.vue` file is split by @vue/compiler-sfc, and every parser reports what it found: a parse
 * error of the component (an unclosed element, or a parse that throws) as a syntax error, a check of the component it
 * built (no block at all, two `<script>` elements) as a grammar error, and then its own errors on the `<script>` and
 * `<script setup>` blocks, read in their `lang`. Any other file is read by the ECMAScript parsers as it is.
 */
export function componentParserVerdicts(parsers: IndependentParsers, sfc: SfcCompiler, path: string, source: string, script: boolean): ParserVerdict[] {
  const named = (verdicts: ParserVerdict[]) => verdicts.map((verdict) => ({ ...verdict, parser: `${verdict.parser} (.vue through @vue/compiler-sfc ${sfc.version})` }));
  if (!/\.vue$/iu.test(path)) return named(parserVerdicts(parsers, path, source, script));
  let parsed: SfcParseResult | undefined;
  let thrown: string | undefined;
  try {
    parsed = sfc.parse(source, { filename: path });
  } catch (error) {
    thrown = (error instanceof Error ? error.message : String(error)).split("\n")[0]!;
  }
  const message = (error: { message: string }) => `@vue/compiler-sfc: ${error.message.split("\n")[0]}`;
  const syntax = parsed === undefined ? [`@vue/compiler-sfc: ${thrown}`] : parsed.errors.filter((error) => error.code !== undefined).map(message);
  const grammar = parsed === undefined ? [] : parsed.errors.filter((error) => error.code === undefined).map(message);
  const blocks = parsed === undefined ? [] : [parsed.descriptor.script, parsed.descriptor.scriptSetup].filter((block) => block !== null);
  const blockVerdicts = blocks.map((block) => parserVerdicts(parsers, SCRIPT_PATHS[block.lang ?? ""] ?? "script.js", block.content));
  // The parsers in parserVerdicts' order and with its names, each reporting what it found in the component.
  return named(parserVerdicts(parsers, "script.js", "").map((verdict, index) => ({
    parser: verdict.parser,
    syntax: [...syntax, ...blockVerdicts.flatMap((verdicts) => verdicts[index]!.syntax)],
    grammar: [...grammar, ...blockVerdicts.flatMap((verdicts) => verdicts[index]!.grammar ?? [])],
  })));
}

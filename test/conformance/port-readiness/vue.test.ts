import assert from "node:assert/strict";
import { test } from "vitest";

import * as babel from "@babel/parser";
import ts from "typescript";

import {
  fileType, parserLanguage, vueReadsAsComponent, vueScriptLangs, vueScriptParser, wrappedScript,
} from "../../../conformance/recorders/port-readiness/adapters/eslint/eslint-upstream.js";
import type { BabelParser, TypeScript } from "../../../conformance/recorders/port-readiness/languages/ecmascript.js";
import { componentParserVerdicts, type SfcCompiler, type SfcParseResult } from "../../../conformance/recorders/port-readiness/languages/vue.js";

test("vue-eslint-parser's trees are Vue components, read at a .vue path", () => {
  assert.equal(parserLanguage("vue-eslint-parser", "vue-eslint-parser", {}), "vue");
  assert.equal(parserLanguage(undefined, "vue-eslint-parser-fork", { "vue-eslint-parser-fork": "vue" }), "vue");
  assert.equal(fileType({ language: "vue", jsx: true, sourceType: "module" }), ".vue");
});

test("vue-eslint-parser reads a .vue file, or code that starts with a tag, as a component, and other code as one script", () => {
  assert.equal(vueReadsAsComponent("test.vue", "export default {}"), true);
  // RuleTester names a case that names no file `<input>`: the code decides.
  assert.equal(vueReadsAsComponent(undefined, "\n  <template><div/></template>"), true);
  assert.equal(vueReadsAsComponent(undefined, "export default {}"), false);
  assert.equal(vueReadsAsComponent("test.js", "Vue.component('x', {})"), false);
  assert.equal(vueReadsAsComponent("test.js", "<div/>"), true);
});

test("a script vue-eslint-parser reads takes the parser its parserOptions.parser picks for the file's extension", () => {
  const tsParser = { parseForESLint: () => ({}) };
  assert.deepEqual(vueScriptLangs("a.tsx"), ["tsx", "ts"]);
  assert.deepEqual(vueScriptLangs("a.js"), ["js"]);
  assert.deepEqual(vueScriptLangs(undefined), ["js"]);
  assert.equal(vueScriptParser(tsParser, ["js"]), tsParser);
  assert.equal(vueScriptParser({ ts: tsParser }, vueScriptLangs("a.tsx")), tsParser);
  assert.equal(vueScriptParser({ ts: "@typescript-eslint/parser" }, ["ts"]), "@typescript-eslint/parser");
  // A map with no entry for the lang falls back to its `js`, and no parser at all means espree.
  assert.equal(vueScriptParser({ ts: tsParser, js: "espree" }, ["js"]), "espree");
  assert.equal(vueScriptParser({ ts: tsParser }, ["js"]), undefined);
  assert.equal(vueScriptParser(undefined, ["js"]), undefined);
});

test("a script case becomes the component holding it: a <script> in its lang one line down, at a .vue name", () => {
  // eslint-plugin-vue's script case at test.vue keeps its name.
  assert.deepEqual(wrappedScript("export default {}", ".js", "test.vue", "r/input.vue"), { code: "<script>\nexport default {}\n</script>\n", filename: "test.vue", lineOffset: 1 });
  // A case naming another file takes its name with .vue; one naming none, the nameless path.
  assert.equal(wrappedScript("let a: number", ".ts", "src/a.ts", "r/input.vue").code, "<script lang=\"ts\">\nlet a: number\n</script>\n");
  assert.equal(wrappedScript("x", ".ts", "src/a.ts", "r/input.vue").filename, "src/a.vue");
  assert.equal(wrappedScript("x", ".tsx", undefined, "r/input.vue").code, "<script lang=\"tsx\">\nx\n</script>\n");
  assert.equal(wrappedScript("x", ".jsx", undefined, "r/input.vue").filename, "r/input.vue");
});

test("a component's verdicts are its own parse errors and each parser's on its scripts, under the same parsers as every other file", () => {
  const parsers = { fixtures: ts as unknown as TypeScript, current: ts as unknown as TypeScript, babel: babel as BabelParser, babelVersion: "7" };
  const blocks = new Map<string, SfcParseResult>([
    ["clean.vue", { errors: [], descriptor: { script: { content: "export default {}" }, scriptSetup: { content: "const n: number = 1", lang: "ts" } } }],
    ["broken.vue", { errors: [{ message: "Element is missing end tag.", code: 24 }], descriptor: { script: null, scriptSetup: { content: "let x = ", lang: "ts" } } }],
    ["empty.vue", { errors: [{ message: "At least one <template> or <script> is required in a single file component." }], descriptor: { script: null, scriptSetup: null } }],
  ]);
  const sfc: SfcCompiler = { version: "3.5.43", parse: (_source, { filename }) => blocks.get(filename)! };
  const names = (path: string, source = "") => componentParserVerdicts(parsers, sfc, path, source, false).map((verdict) => verdict.parser);

  // Every file names the same parsers in the same order, as one recording needs.
  assert.deepEqual(names("clean.vue"), names("a.js", "let x = 1"));
  assert.match(names("a.js")[0]!, /^typescript .* \(\.vue through @vue\/compiler-sfc 3\.5\.43\)$/u);

  for (const verdict of componentParserVerdicts(parsers, sfc, "clean.vue", "", false)) assert.deepEqual([verdict.syntax, verdict.grammar], [[], []]);
  for (const verdict of componentParserVerdicts(parsers, sfc, "broken.vue", "", false)) {
    assert.equal(verdict.syntax[0], "@vue/compiler-sfc: Element is missing end tag.");
    assert.ok(verdict.syntax.length > 1, `${verdict.parser} reports the unfinished <script setup lang="ts">`);
  }
  // A check of the component it built (no block at all) is a grammar error, not a parse failure.
  for (const verdict of componentParserVerdicts(parsers, sfc, "empty.vue", "", false)) {
    assert.deepEqual(verdict.syntax, []);
    assert.deepEqual(verdict.grammar, ["@vue/compiler-sfc: At least one <template> or <script> is required in a single file component."]);
  }
});

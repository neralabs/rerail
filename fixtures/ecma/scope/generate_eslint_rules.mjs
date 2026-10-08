// Writes eslint-rules.expected.json: where ESLint's own no-obj-calls, no-implied-eval, no-new-wrappers, no-eval, no-alert, no-extend-native and strict report on a few sources
// under different languageOptions, as byte offsets. Run offline, not in CI:
//   node generate_eslint_rules.mjs <node_modules with eslint@10.11.0>
import { writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";

const CASES = [
  { rule: "no-obj-calls", code: "Reflect();\nJSON();\nvar x = new Atomics();\n", languageOptions: { ecmaVersion: 5, sourceType: "script" } },
  { rule: "no-obj-calls", code: "Reflect();\nJSON();\nvar x = new Atomics();\n", languageOptions: { ecmaVersion: 2015, sourceType: "script" } },
  { rule: "no-obj-calls", code: "Reflect();\nJSON();\nvar x = new Atomics();\n", languageOptions: { ecmaVersion: 2017, sourceType: "script" } },
  { rule: "no-obj-calls", code: "Reflect();\nJSON();\nvar x = new Atomics();\n", languageOptions: { ecmaVersion: 2017, globals: { JSON: "off" } } },
  { rule: "no-obj-calls", code: "/* global Reflect */\nReflect();\n", languageOptions: { ecmaVersion: 5, sourceType: "script" } },
  // Calls through a variable initialized to the global; a parameter of the same name shadows it. (ESLint's
  // ReferenceTracker also follows a variable some other write assigns, which a binding value does not.)
  { rule: "no-obj-calls", code: "var foo = JSON; function f(foo) { foo(); }\nvar a = JSON, b = a; b();\nconst d = Math; new d();\nvar e = e; e();\n", languageOptions: { ecmaVersion: 2022, sourceType: "script" } },
  // A string argument, directly or through getStaticValue with the call's scope.
  { rule: "no-implied-eval", code: "const s = 'alert(1)'; setTimeout(s);\nlet t = 'a'; t = 'b'; setTimeout(t);\nsetTimeout(() => 1);\nsetTimeout('a' + x);\nconst n = 1; setTimeout(n);\nsetTimeout(`x`);\nvar u = 'x'; function f(u) { setTimeout(u); }\n", languageOptions: { ecmaVersion: 2022, sourceType: "script", globals: { setTimeout: "readonly" } } },
  { rule: "no-new-wrappers", code: "new String('a');\nnew Number(1);\nnew Boolean(true);\n", languageOptions: {} },
  { rule: "no-new-wrappers", code: "new String('a');\nnew Number(1);\nnew Boolean(true);\n", languageOptions: { globals: { String: "off" } } },
  { rule: "no-new-wrappers", code: "new String('a');\nnew Number(1);\nnew Boolean(true);\n", languageOptions: { ecmaVersion: 3, sourceType: "script", globals: { Number: false, Boolean: "off" } } },
  { rule: "no-eval", code: "this.eval('a');\n", languageOptions: { sourceType: "script" } },
  { rule: "no-eval", code: "this.eval('a');\n", languageOptions: { sourceType: "module" } },
  { rule: "no-eval", code: "'use strict'; this.eval('a');\n", languageOptions: { sourceType: "script" } },
  { rule: "no-eval", code: "(() => this.eval('a'))();\n", languageOptions: { sourceType: "script" } },
  // A non-strict ordinary function's `this` is the global object for no-eval. Strictness comes from a directive
  // prologue (ignored at ES3), a module, impliedStrict or a class; globalReturn makes the top level a function.
  { rule: "no-eval", code: "function f() { this.eval('a'); }\nfunction g() { 'use strict'; this.eval('b'); }\nfunction h() { function i() { this.eval('c'); } 'use strict'; }\nfunction j() { 'a'; 'use strict'; function k() { this.eval('d'); } }\nfunction l() { ('use strict'); this.eval('e'); }\n", languageOptions: { ecmaVersion: 5, sourceType: "script" } },
  { rule: "no-eval", code: "'use strict';\nthis.eval('a');\nfunction f() { this.eval('b'); }\n", languageOptions: { ecmaVersion: 5, sourceType: "script" } },
  { rule: "no-eval", code: "function f() { 'use strict'; this.eval('a'); }\n", languageOptions: { ecmaVersion: 3, sourceType: "script" } },
  { rule: "no-eval", code: "function f() { this.eval('a'); }\nthis.eval('b');\n", languageOptions: { ecmaVersion: 5, sourceType: "script", parserOptions: { ecmaFeatures: { impliedStrict: true } } } },
  { rule: "no-eval", code: "function f() { 'use strict'; this.eval('a'); }\n", languageOptions: { ecmaVersion: 3, sourceType: "script", parserOptions: { ecmaFeatures: { impliedStrict: true } } } },
  { rule: "no-eval", code: "function f() { this.eval('a'); }\n", languageOptions: { ecmaVersion: 2015, sourceType: "module" } },
  { rule: "no-eval", code: "var j = () => { 'use strict'; function k() { this.eval('a'); } };\nclass A extends function () { this.eval('b'); } { m() { this.eval('c'); } static { this.eval('d'); } }\nvar o = { m() { this.eval('e'); }, get p() { return this.eval('f'); } };\nfunction* g() { this.eval('g'); }\n", languageOptions: { ecmaVersion: 2022, sourceType: "script" } },
  { rule: "no-eval", code: "this.eval('a');\nfunction f() { this.eval('b'); }\n", languageOptions: { ecmaVersion: 5, sourceType: "script", parserOptions: { ecmaFeatures: { globalReturn: true } } } },
  { rule: "no-eval", code: "'use strict';\nthis.eval('a');\nfunction f() { this.eval('b'); }\n", languageOptions: { ecmaVersion: 5, sourceType: "script", parserOptions: { ecmaFeatures: { globalReturn: true } } } },
  { rule: "no-eval", code: "this.eval('a');\n", languageOptions: { ecmaVersion: 5, sourceType: "script", parserOptions: { ecmaFeatures: { globalReturn: true, impliedStrict: true } } } },
  { rule: "no-eval", code: "function f() { this.eval('a'); }\n", languageOptions: { sourceType: "commonjs" } },
  { rule: "no-eval", code: "'use strict';\nfunction f() { this.eval('a'); }\n", languageOptions: { sourceType: "commonjs" } },
  { rule: "no-eval", code: "this.eval('a');\n", languageOptions: { sourceType: "commonjs", parserOptions: { ecmaFeatures: { globalReturn: true } } } },
  // no-eval reads a CommonJS file's top-level `this` as the global object: its Program is no module and the
  // configuration sets no globalReturn (eslint-scope's function scope for CommonJS does not matter to it).
  { rule: "no-eval", code: "this.eval('a');\n'use strict';\n{ this.eval('b'); }\n(() => this.eval('c'))();\n", languageOptions: { sourceType: "commonjs" } },
  // no-alert tests shadowing, not existence: `window` counts without browser globals, `off` changes nothing.
  { rule: "no-alert", code: "alert(1);\nwindow.alert(2);\nwindow.confirm(3);\nfunction f(alert) { alert(4); }\nprompt(5);\n", languageOptions: { ecmaVersion: 5, sourceType: "script" } },
  { rule: "no-alert", code: "alert(1);\nwindow.alert(2);\nwindow.confirm(3);\nfunction f(alert) { alert(4); }\nprompt(5);\n", languageOptions: { ecmaVersion: 2015, sourceType: "module", globals: { window: "readonly", alert: "off" } } },
  { rule: "no-alert", code: "var window = {};\nwindow.alert(1);\nconfirm(2);\n", languageOptions: { ecmaVersion: 5, sourceType: "script" } },
  // no-alert takes `this` as the global object only where eslint-scope's scope is the global scope: blocks count
  // from ES2015, catch and with clauses always; never in a module or CommonJS file.
  ...[[3, "script"], [5, "script"], [2015, "script"], [2015, "module"], [2015, "commonjs"]].map(([ecmaVersion, sourceType]) => ({
    rule: "no-alert",
    code: "this.alert(1);\n{ this.alert(2); }\nif (a) { this.alert(3); } else this.alert(4);\nswitch (a) { case 1: this.alert(5); }\ntry { this.alert(6); } catch (e) { this.alert(7); }\nfor (var i in this.alert(8)) this.alert(9);\nfunction f() { this.alert(10); }\nx = { a: this.alert(11) };\n",
    languageOptions: { ecmaVersion, sourceType },
  })),
  { rule: "no-alert", code: "with (a) this.alert(1);\nwith (this.alert(2)) {}\n", languageOptions: { ecmaVersion: 5, sourceType: "script" } },
  { rule: "no-alert", code: "for (let i = this.alert(1);;) {}\nfor (const k of this.alert(2)) this.alert(3);\nclass A extends this.alert(4) {}\n() => this.alert(5);\nlabel: this.alert(6);\n", languageOptions: { ecmaVersion: 2022, sourceType: "script" } },
  // Rules that follow the global variable's references count a script's top-level declarations as the global.
  ...["script", "module", "commonjs"].map(sourceType => ({
    rule: "no-extend-native",
    code: "var Array;\nArray.prototype.a = 1;\nfunction Object() {}\nObject.prototype.b = 2;\nlet Map;\nMap.prototype.c = 3;\nfunction f(Set) { Set.prototype.d = 4; }\nSet.prototype.e = 5;\n",
    languageOptions: { ecmaVersion: 2015, sourceType },
  })),
  { rule: "no-extend-native", code: "var Promise;\nPromise.prototype.a = 1;\n", languageOptions: { ecmaVersion: 5, sourceType: "script" } },
  // Before ES2015 blocks are no scopes: a function declared in one binds in the enclosing function or global scope.
  ...[5, 2015].map(ecmaVersion => ({
    rule: "no-extend-native",
    code: "{ function Array() {} }\nArray.prototype.a = 1;\nif (a) { function Object() {} }\nObject.prototype.b = 2;\n",
    languageOptions: { ecmaVersion, sourceType: "script" },
  })),
  { rule: "no-eval", code: "var window;\nwindow.eval('a');\nfunction f() { window.eval('b'); }\n", languageOptions: { ecmaVersion: 2015, sourceType: "script" } },
  { rule: "no-eval", code: "var window;\nwindow.eval('a');\nfunction f() { window.eval('b'); }\n", languageOptions: { ecmaVersion: 2015, sourceType: "module" } },
  // strict ("safe") picks its mode from the configuration the file does not show: "module" or "implied" (every
  // directive is unnecessary), "global" under CommonJS or globalReturn, else "function".
  ...["'use strict';\nfunction f() { 'use strict'; return 1; }\nfunction g() { return 2; }\n", "function g() { return 2; }\n"].flatMap(code =>
    [
      { sourceType: "script" },
      { sourceType: "module" },
      { sourceType: "commonjs" },
      { sourceType: "script", parserOptions: { ecmaFeatures: { impliedStrict: true } } },
      { sourceType: "script", parserOptions: { ecmaFeatures: { globalReturn: true } } },
      { sourceType: "commonjs", parserOptions: { ecmaFeatures: { impliedStrict: true } } },
    ].map(languageOptions => ({ rule: "strict", code, languageOptions })),
  ),
];

const require = createRequire(join(process.argv[2], "noop.js"));
const { Linter } = require("eslint");
const { version } = require("eslint/package.json");
const cases = CASES.map(({ rule, code, languageOptions }) => {
  const lines = code.split("\n");
  const offset = (line, column) => Buffer.byteLength(lines.slice(0, line - 1).map(text => `${text}\n`).join("") + lines[line - 1].slice(0, column - 1));
  const messages = new Linter({ configType: "flat" }).verify(code, [{ files: ["**/*.js"], languageOptions, rules: { [rule]: "error" } }], "fixture.js");
  const fatal = messages.find(message => message.fatal);
  if (fatal !== undefined) throw new Error(`${rule}: ${fatal.message}`);
  return { rule, code, languageOptions, reported: messages.map(message => offset(message.line, message.column)).sort((a, b) => a - b) };
});
writeFileSync(join(import.meta.dirname, "eslint-rules.expected.json"), `${JSON.stringify({ oracle: `eslint ${version}`, cases }, null, 1)}\n`);

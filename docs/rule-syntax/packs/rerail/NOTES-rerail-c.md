# rerail pack, batch C

Five rules, each with tests in its file. Regexes and messages are the JSON's, copied with fill.py; a JSON `regex` on a
node is ast-grep's (an unanchored search) and is written as a whole-text `match` with `(?s:.*)` where it may go on.

## rerail-git-no-lazy-fetch

- One TypeScript policy, three clauses. The JSON's file conditions (`path.regex` / `notRegex` on src/engine/git.ts)
  are `git_module(n)`, a match on the document's path.
- `nthChild` with `ofRule: not comment` is `arg(c, a, i)`: the arguments' named children without comments.
- `resolves.binding.value` (a name bound to `promisify(spawn | execFile | exec)`) uses tsx/names' `ref_site` and
  `binding_value`, as the engine's translator does.
- `has` with `field` and `stopBy: end` on the arguments is `has(arguments, p, "pair")`; the key and value tests read
  the pair's `key`/`value` fields only (the machine translation also read anonymous tokens and source gaps; a pair's
  key is always its field, so they add nothing).
- A message `{ … }` is written `{{ … }}` (braces are interpolation in a message).

## rerail-provenance-writers

- The JSON's scope spans TypeScript, JavaScript and Python; five clauses are regexes over the file, one is a
  TypeScript tree clause. Two files declare the policy: `rerail-provenance-writers.rerail` (no language; every scoped
  file as text, the five regex clauses) and `rerail-provenance-writers.typescript.rerail` (the tree clause, over the
  .ts/.mts/.mjs/.js globs). Both report LICENCE_RECORD_WRITE; a key both find is one finding.
- Text, not `.language python`, for the regex clauses: the original searches a file that does not parse; a
  language policy would withhold it.
- The tree clause's `inside`/`has` declaration lookups (`$D`, `$P`) are `declared(n, x, v)`: a declarator of that
  name anywhere in an enclosing function, or a top-level `const`/`let` (`lexical_declaration` directly under the
  program, as the JSON's direct `has`). Like the JSON, it does not resolve which declaration the name refers to.

## rerail-pack-data-runtime-names

- TypeScript; the text clause is `source`/`match_all`, the literal clause a whole-text match on `string` and
  `template_string`. No difference.

## rerail-pack-data-helper-copies

- The scope has .mjs/.js/.cjs and .ts files; the JSON's two regex clauses read all of them, its pattern clause is
  `language: javascript`. `.language typescript` runs on the JavaScript dialect too, so the policy covers every file,
  and the pattern clause is restricted to non-.ts files (`javascript(n)`). Checked on the box: the original does not
  report `process.chdir(x)` in a .ts file under the scope; without the restriction the translation did.
- The patterns are tree facts: `process.chdir(…)` a call whose callee is that member chain, `process.cwd = …` an
  assignment, `Object.defineProperty(process, "cwd", …)` with the second argument's text exactly `"cwd"` (ast-grep
  compares the pattern's quotes, so `'cwd'` is reported by neither side).

## rerail-rule-documents-in-packs

- JSON has a grammar label (`json`, languages/json.json), but the three clauses are regexes from the file's start
  (`\A`), so the policy reads `**/*.json` as text (`.scope include` + `.scope text`): no tree is needed, and a JSON
  file that does not parse is still searched. Anchors: `witness.start` is the match's start (`match_all`), the two
  `capture.*` anchors are `match_group` spans.

## Library additions

None. `arg`, `writer`, `declared` and the path tests are local; no second rule uses them yet.

## Validation (nera-port-1, `box-check.sh rpt-trc`, plus each test's fixtures run through both sides)

| rule | test-rr | pack examples caught / passes | repo corpus 7efcac9c67 | test fixtures, original vs translation |
| --- | --- | --- | --- | --- |
| rerail-git-no-lazy-fetch | 6/6 | same 1 / same 0 | same 0 | same (12 findings, 6 corpora) |
| rerail-provenance-writers | 6/6 (3 + 3) | same 1 / same 0 | same 1 | same (15 findings, 6 corpora) |
| rerail-pack-data-runtime-names | 3/3 | same 1 / same 0 | same 0 | same (5 findings, 3 corpora) |
| rerail-pack-data-helper-copies | 3/3 | same 1 / same 0 | same 0 | same (6 findings, 3 corpora) |
| rerail-rule-documents-in-packs | 3/3 | same 1 / same 0 | same 537 | same (3 findings, 3 corpora) |

No rule is blocked.

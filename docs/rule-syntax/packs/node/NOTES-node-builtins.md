# n-no-unsupported-features-node-builtins, hand-written

> The core additions proposed below are now in lib/core.rr (2026-11-05).

The JSON policy has 932 clauses and 3,053 utilities. The generated `.rr` is about 29,000 code lines. This version has
four parts:

| file | lines | what |
|---|---|---|
| `n-no-unsupported-features-node-builtins.rr` | 56 | the rule |
| `n-no-unsupported-features-node-builtins.facts.rr` | 3,848 | generated table: 1,876 APIs, 119 module specifiers, 77 globals, 1,752 traced names |
| `../../lib/tsx/values.rr` | 218 | which built-in an expression denotes (`api_ref`, `api_use`, `node_range`) |
| `gen-node-builtins.py`, `check-node-builtins.py` | | regenerate the table from the JSON; check it by a round trip |

## Assumptions

- **Where the data comes from.** The JSON is eslint-plugin-n 18.4.1's trace map, lowered by
  `scripts/generate/packs/node_policies.py`. `gen-node-builtins.py` inverts that lowering. The plugin itself is not
  installed here.
  - The plugin has experimental versions for each API. The JSON was lowered with allowExperimental off, so it
    dropped them, and the table cannot hold them.
  - `api_experimental(api, how)` means no release supports that use. Either the API is experimental everywhere, or
    that form is never supported, for example `zlib.Gzip()` called without `new`.
- **The version test.** The JSON's test is "the configured range is not a subset of `^v1 || ^v2 || >=v1`". The rule
  tests `>=since || ^backport…`. Both ranges admit the same versions.
- **API names.** A module is named by its specifier as written (`"fs"`, `"node:fs"`). A module member is written
  module `#` path (`"fs#promises.readFile"`). A global is written as a plain dotted path (`"crypto.subtle"`). Globals
  and modules stay apart because global `crypto` (23.0.0) and module `crypto` (0.1.92) differ. `how` is `load`,
  `read`, `call` or `new`.
- **The Node range** follows the engine (`package_manifest.rs`, `configured_node_version`):
  1. `engines.node`, when it is a string and a valid range;
  2. else the first `devEngines.runtime` entry named node that has a string version;
  3. else `>=16.0.0`. This also applies when that version is invalid, and when there is no package.json.

  A valid `version` option overrides this. An invalid one, such as `"lts"`, is ignored.
- **Other options.** `option("ignores", name)` gives one row per list entry, in the plugin's spelling: `#` is
  written `.`, and `node:` is dropped.
- **Module system.** The policy asks for `moduleSystem: package-type`. `values.rr`'s `package_type(d, t)` implements
  it:
  - `.mjs` and `.mts` files are ES modules, and `.cjs` and `.cts` files are CommonJS;
  - any other file is an ES module only where its package.json says `"type": "module"`;
  - no syntax is read.

  This differs from `modules.rr`'s `source_type`, which follows the engine's file mode and does read syntax, so I
  did not reuse it.

  `require`, `module`, `exports`, `__dirname` and `__filename` are globals only in CommonJS files, unless a
  `/* global */` comment declares them.
- **Which globals are traced.** `node_global` holds the globals the trace map has, plus `globalThis`. It is shipped
  with the table and replaces `es_global`, which was unspecified.
- **`node_module`, `api_known` and `node_global`** are relations declared in `values.rr`, with their rows
  generated into the facts file.
- **Includes.** `#include` resolves from `lib/`, so the facts file is included as `"../packs/node/…facts.rr"`.

## Core additions proposed

They are at the top of `values.rr`:

- `@semver_valid(r): number` is node-semver's loose `validRange`.
- `@semver_subset(a, b): number` is node-semver's `subset`. It is defined only for two valid ranges: on an invalid
  one it is an evaluation error, so the row is Unknown rather than 0. Callers guard with `@semver_valid`.
- **The package.json reader.** There is no new relation. `values.rr` includes `modules.rr` and uses its proposed
  `package_for(path, dir)` and `package_field(dir, key, v)`, with one extension:
  - `key` may be a nested path, segments joined by `/` and escaped as JSON pointer tokens are (`~0`, `~1`), with
    array indices from 0: `"engines/node"`, `"devEngines/runtime/0/version"`;
  - only string values give rows, so `"engines": {"node": 18}` gives none, as the engine ignores it;
  - core's `package(dir, name)` is unchanged.

## Changes to names.rr, after review

1. **`star_export` now requires an `export_statement`.** Before this, every `import … from "m"` counted as
   `export *`. Its only other user, `exports`, already tested the statement kind.
2. **Destructured names now resolve.**
   - A declaration's kind comes from the declarator's parent statement, not from the nearest ancestor. So
     `var f = () => { let x }` no longer declares `x` with `var`.
   - Pattern names are declared from that kind: function scope for `var`, block scope for `let` and `const`.
   - Shorthand names in destructured parameters are now declared.
   - `bound_name` keeps keys and default values out of the bound names.
3. **A shorthand assignment pattern counts as a reference.** In `({ promises } = require("fs"))`, a shorthand key
   that declares nothing is now a (write) reference. `values.rr` carries a value written by a pattern back to the
   variable's declaration.

## Possible differences from the JSON

Every finding has the same code, so a finding's key is (path, line). These differences matter only where they change
a line's set of findings.

1. **allowExperimental is not ported** (see Assumptions).
2. **Small supersets.**
   - `global` and `globalThis` are also followed through assignments.
   - `const { fetch } = globalThis` is traced. The JSON has no destructuring pattern for the global object.
3. **Known gaps in `names.rr`, unchanged.**
   - A parameter's default value expression, `function f(a = g(x))`, still declares `x`: the old parameter rule
     matches every identifier under the parameters.
   - A `catch ({ e })` pattern is not declared.
4. **Messages differ.** They are free text. An `export * from "m"` gives one row per member at the statement, which
   is the same key as the JSON's.
5. **ESLint settings.** `settings.n.version` and `settings.node.version` are not read.

## The es-builtins rule

The es-builtins JSON has the same shape, with globals only. The same rule text and generator would serve it.

## Checks

- **`python3 site/check_rr.py`** reports ok. It now covers `packs/`.
- **`python3 packs/node/check-node-builtins.py`** reports ok. It works as a round trip, after the reviewer's
  `roundtrip.py`:
  1. rebuild the trace map from the table;
  2. run the JSON's own generator on it;
  3. compare the result with the shipped JSON, with compact utilities inlined.

  On 756 utilities, 537 clause groups and 3,720 alternatives it found 0 differences. It also checks `node_module`,
  `node_global` and `api_known`.

  Two mutations fail it, as they should:
  - turning `fs#Stats` call into new;
  - changing `fetch`'s version.
- **The reviewer's Soufflé harness** passes the new cases:
  - a named import, which is no longer `export *`;
  - a destructured parameter, `const { fetch } = require("node-fetch")`, and `const { promises } = require("fs")`;
  - a shorthand assignment pattern;
  - `require` in a `.mjs` file;
  - a comment before the specifier;
  - `devEngines`: the first node entry, an invalid version, and a numeric `engines.node`;
  - `version=lts`.

  To run them I adapted its `lower.py` to the `package_field` reader and to the capture and suffix forms.

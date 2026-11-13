# n-no-unsupported-features-node-builtins, hand-written

The JSON policy has 932 clauses and 3,053 utilities. The generated `.rr` is about 29,000 code lines. This version has
four parts:

| file | lines | what |
|---|---|---|
| `n-no-unsupported-features-node-builtins.rr` | 69 | the rule |
| `n-no-unsupported-features-node-builtins.facts.rr` | 3,853 | generated table: 1,876 APIs (312 with experimental versions), 119 module specifiers, 77 globals, 1,752 traced names |
| `../../lib/tsx/values.rr` | 201 | which built-in an expression denotes (`api_ref`, `api_use`, `node_range`) |
| `gen-node-builtins.py`, `check-node-builtins.py` | | regenerate the table; check it by a round trip and against the plugin |

## Where the data comes from

The table has two sources, and both scripts need both.

**The JSON.** `packs/node/rules/…node-builtins.json` is eslint-plugin-n 18.4.1's trace map, lowered by
`scripts/generate/packs/node_policies.py`. `gen-node-builtins.py` inverts that lowering.

**The plugin.** The plugin's own data is in `lib/unsupported-features/*.js`. No checkout or `node_modules` on this
machine has it, so I fetched the pinned tarball:

```
npm pack eslint-plugin-n@18.4.1
```

Then I unpacked it and passed the directory as `--plugin DIR` (or `$ESLINT_PLUGIN_N`). The version is checked
against the pin.

- **No JavaScript runs.** The data files use a small subset of JavaScript: imports, `const`, member assignments,
  object and array literals, spreads, and computed `[READ]` keys. The generator's `PluginData` evaluates that subset
  in Python, so the table regenerates without Node. `Symbol.*` keys are skipped, because `Object.entries` skips them
  too.
- **The two sources agree.** On all 1,876 APIs, and their supported versions, the plugin and the JSON match in both
  directions. Only then does the generator take the experimental versions, which the JSON never had, from the
  plugin.

**What the rows mean.**

| relation | meaning |
|---|---|
| `api_since`, `api_backport` | the supported releases |
| `api_experimental_since`, `api_experimental_backport` | the releases where the API is experimental |
| `api_never(api, how)` | no release supports that use: the API is experimental only, or that form is never supported, such as `zlib.Gzip()` without `new` (formerly `api_experimental`) |

## Assumptions

- **The version test.**
  - The JSON's test is "the configured range is not a subset of `^v1 || ^v2 || >=v1`". The rule tests
    `>=since || ^backport…`. Both ranges admit the same versions.
  - **allowExperimental**, as the plugin implements it (`check-unsupported-builtins.js`). When the option is on and
    an API has experimental versions, those versions alone decide: the API is fine if the configured range is
    inside them, and it is reported (kind "experimental") if not. The supported versions are then not consulted.
    Every other API is judged by its supported versions.
- **API names.**
  - A module is named by its specifier as written (`"fs"`, `"node:fs"`).
  - A module member is written module `#` path (`"fs#promises.readFile"`).
  - A global is a plain dotted path (`"crypto.subtle"`).
  - Globals and modules stay apart: global `crypto` (23.0.0) and module `crypto` (0.1.92) are different APIs.
  - `how` is `load`, `read`, `call` or `new`.
- **The Node range** follows the engine (`package_manifest.rs`, `configured_node_version`):
  1. `engines.node`, when it is a string and a valid range;
  2. else the first `devEngines.runtime` entry named node with a string version;
  3. else `>=16.0.0`, which is also used when that version is invalid and when there is no package.json.

  A valid `version` option overrides all of this. An invalid one, such as `"lts"`, is ignored.
- **Options.**
  - `option("ignores", name)` is one row per list entry, in the plugin's spelling: `#` is written `.`, and `node:`
    is dropped.
  - `option("allowExperimental", "true")` turns on the experimental basis above.
- **Module system** (`moduleSystem: package-type`). `values.rr`'s `package_type(d, t)` implements it:
  - `.mjs` and `.mts` files are ES modules;
  - `.cjs` and `.cts` files are CommonJS;
  - any other file is an ES module only where its package.json says `"type": "module"`.

  No syntax is read. In ES modules, the CommonJS globals (`require`, `module`, `exports`, `__dirname`,
  `__filename`) are not globals, unless a `/* global */` comment declares them.
- **Which globals are traced.** `node_global` holds the globals the trace map has, plus `globalThis`. It ships with
  the table.
- **Includes.** `#include` resolves from `lib/`, so the facts file is included as `"../packs/node/…facts.rr"`.
- **Core.** The semver functors and the `package_for` / `package_field` reader now live in `lib/core.rr`.
  `values.rr` no longer includes `modules.rr`.

## Changes to names.rr

1. **`star_export`** now requires an `export_statement`. Previously every `import … from "m"` counted as
   `export *`.
2. **Variable declarations.**
   - A declaration's kind (`var`, or `let`/`const`) comes from the declarator's parent statement.
   - Pattern names are declared with that kind's scope.
   - `bound_name` keeps keys and default values out of the bound names.
   - A pattern inside a function nested in a default value is not declared in the outer scope.
3. **Parameters.**
   - Only bound names are declared: a name listed directly, the pattern of a TypeScript parameter, or a name bound
     by a destructuring pattern or a default's left side.
   - In `function f(a = g(x))`, `x` is a reference again.
   - A bare arrow parameter (`x => x`) is now declared.
4. **`catch (e)` and `catch ({ e })`** declare in the catch clause, with form `"catch"`.
5. **A shorthand assignment pattern** (`({ promises } = …)`) that declares nothing is a reference.

Before these changes I checked the other users of `names.rr`, and none of them reads the declaration relations
directly:

- `jsx.rr` uses `sym_of` and `ref_site`, which these changes make more precise.
- `modules.rr`, `eslint-strict.rr` and `eslint-sort-imports.rr` use `import_binding`, which is unchanged.
- `nested-routers.b2.rr`

## Possible differences from the JSON

Every finding has the same code, so a finding's key is (path, line). These differences matter only where they change
a line's set of findings.

1. **Small supersets.**
   - `global` and `globalThis` are also followed through assignments.
   - `const { fetch } = globalThis` is traced. The JSON has no destructuring pattern for the global object.
2. **Messages differ.** They are free text. An `export * from "m"` gives one row per member at the statement, which
   is the same key as the JSON's.
3. **ESLint settings.** `settings.n.version` and `settings.node.version` are not read.

## The es-builtins rule

The es-builtins JSON has the same shape, with globals only. The same rule text and generator would serve it, with
the plugin's es-builtins trace map as the second source.

## Checks

- **`python3 site/check_rr.py`** reports ok.
- **`python3 packs/node/gen-node-builtins.py --plugin DIR`** reports that the plugin and the JSON agree on all 1,876
  APIs. Regenerating gives the same file, byte for byte.
- **`python3 packs/node/check-node-builtins.py --plugin DIR`** reports ok.
  - **Round trip:** it rebuilds the trace map from the table, runs the JSON's own generator, and compares the result
    with the shipped JSON, compact utilities inlined. That is 756 utilities, 537 clause groups and 3,720
    alternatives, with 0 differences.
  - **Plugin:** every row's supported and experimental versions equal the plugin's, and the table holds exactly the
    plugin's traced APIs.
  - **Mutations it catches:**
    - `fs#Stats` changed from call to new;
    - `fetch`'s version changed;
    - `fs#cp`'s experimental version changed.

  Without `--plugin`, the script runs the round trip and says the experimental columns went unchecked.
- **The reviewer's Soufflé harness** covers cases 1 to 21:
  - Cases 18 to 21 are new:
    - a default value is a reference;
    - `catch ({ fetch })` and `catch (process)` shadow the globals;
    - `fs.promises.cp` with allowExperimental, under `>=16.7.0` and `>=14.0.0`.
  - Cases 1 to 17 are unchanged; only the output order differs.
  - With `allowExperimental=true`, the `experimental` kind replaces `supported` exactly where the plugin would.

  I adapted the harness's `lower.py` to the core functors and to the `range_of` join.

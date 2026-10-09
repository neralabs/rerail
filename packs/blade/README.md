# Rerail rule pack: Blade

15 code rules for Laravel Blade templates, ported from Bladestan 0.11.7 (8 PHPStan 2.2.16 rules over the compiled
template), tlint 9.7.0 (3 lints) and stillat/blade-parser 2.1.0 (4 validators). Each is scored against the tool's own
findings on the tool's test fixtures and on three pinned corpora, through the engine's production workspace path, with
every disagreement declared in `divergences.json`.

It is checked by `npx vitest run test/engine/catalog/packs.test.ts -t "pack blade"` ([pack structure and validation
contract](../../docs/pack-format.md)).

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): each rule's document, title and group; eight rules carry examples |
| `rules/` | The code-rule documents (`rerail.policy/v1`) |
| `conformance.json` | Each ported rule's upstream tool and rule, how findings compare, and the capabilities it exercises; the pinned tools, the pinned corpora (BookStack, Filament, Bagisto) and the `calls` exemption with its evidence |
| `divergences.json` | Declared disagreements with the tools |
| `upstream/<tool>/` | The tool's copied tests (`fixtures/`), its recordings (`fixtures.expected.json`, `corpus-<name>.expected.json`) and the recorder (`generate.py` with its PHP helpers, sharing `upstream/common.py` and `upstream/capture.php`) |
| `licenses/<tool>/` | The tool's licence and `UPSTREAM.md`, listing every copied file |
| `candidates/rules/` | Ports scored but never shipped: tlint's NoDumpDirectives and NoRayDirective. tree-sitter-blade reads `@dump(...)`, `@dd(...)` and `@ray(...)` with arguments as an unclosed paired directive, so every template using them has a syntax-error region and neither port can show a caught case on valid Blade |

Re-recording runs offline against the pinned tool versions, with PHP 8.5.11 (each recorder's docstring has the setup):

```sh
python3 upstream/bladestan/generate.py fixtures <bladestan checkout at 0.11.7>
python3 upstream/tlint/generate.py fixtures <tlint checkout at v9.7.0> <composer project with tightenco/tlint 9.7.0>
python3 upstream/blade-parser/generate.py fixtures <blade-parser checkout at v2.1.0> <composer project with stillat/blade-parser 2.1.0>
python3 upstream/bladestan/generate.py corpus <bladestan checkout> bookstack=<checkout> filament=<checkout> bagisto=<checkout>
python3 upstream/tlint/generate.py corpus <tlint composer project> bookstack=<checkout> filament=<checkout> bagisto=<checkout>
python3 upstream/blade-parser/generate.py corpus <blade-parser composer project> bookstack=<checkout> filament=<checkout> bagisto=<checkout>
```

Scoring one rule with every false positive and false negative printed:

```sh
node --import tsx conformance/languages/cli.ts --pack blade --rule tlint-space-after-blade-directives --fixtures
```

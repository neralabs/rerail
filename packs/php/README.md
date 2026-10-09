# Rerail rule pack: PHP

18 code rules for PHP, ported from PHP_CodeSniffer 4.0.4 (8 sniffs), PHPStan 2.2.16 (8 rules) and Psalm 6.19.1 (2
issues). Each is scored against the tool's own findings on the tool's tests, through the engine's production
workspace path, with every disagreement declared in `divergences.json`.

It follows the [pack structure and validation contract](../../docs/pack-format.md): it is checked by
`npx vitest run test/packs.test.ts -t "pack php"`.

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): each rule's document, title and group; two rules carry examples |
| `rules/` | The code-rule documents (`rerail.policy/v1`) |
| `conformance.json` | Each ported rule's upstream tool and rule, how findings compare, and the capabilities it exercises; the pinned tools and the pinned corpora (laravel, symfony) |
| `divergences.json` | Declared disagreements with the tools |
| `upstream/<tool>/` | The tool's copied tests (`fixtures/`), its recordings (`fixtures.expected.json`, `corpus-<name>.expected.json`) and the recorder (`generate.py`, sharing `upstream/common.py`); Psalm's corpus dependencies are pinned by `upstream/psalm/composer-<name>.lock` |
| `licenses/<tool>/` | The tool's licence and `UPSTREAM.md`, listing every copied file |

Re-recording runs offline against the pinned tool versions, with PHP 8.5.11:

```sh
python3 upstream/phpcs/generate.py fixtures <PHP_CodeSniffer checkout at 4.0.4> <composer project with squizlabs/php_codesniffer 4.0.4>
python3 upstream/phpstan/generate.py fixtures <phpstan-src checkout at 2.2.16> <composer project with phpstan/phpstan 2.2.16>
python3 upstream/psalm/generate.py fixtures <Psalm checkout at 6.19.1> <composer project with vimeo/psalm 6.19.1>
python3 upstream/<tool>/generate.py corpus <composer project> laravel=<laravel checkout> symfony=<symfony checkout>
```

The Psalm recorder reads which code Psalm leaves unanalysed with the usage-corpus generator it shares with the engine's
scope oracle, `fixtures/php/scope/php/generate_usage_corpus.py`.

Scoring one rule with every false positive and false negative printed:

```sh
node --import tsx conformance/languages/cli.ts --pack php --rule psalm-unused-variable --fixtures
```

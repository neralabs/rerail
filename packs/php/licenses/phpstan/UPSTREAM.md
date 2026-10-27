# PHPStan 2.2.16

Upstream: [phpstan/phpstan-src](https://github.com/phpstan/phpstan-src/tree/08e3736efd29631ab8f32596c5dd2362dec8dc26) at tag `2.2.16` (commit `08e3736efd29631ab8f32596c5dd2362dec8dc26`). License: MIT (Copyright (c) 2016 Ondřej Mirtes, Copyright (c) 2025 PHPStan s.r.o.); [`LICENSE`](LICENSE) is the upstream license file, copied verbatim at that tag.

The files listed below are test data of the rules the PHP pack ports (the pack's `conformance.json`), analysed by their RuleTestCase tests, and remain under PHPStan's license. Every other file in this
pack is Rerail's. The rule documents in `rules/` and `candidates/rules/` are Rerail's re-implementations of PHP_CodeSniffer's, PHPStan's and
Psalm's rules, published under the pack's licence expression, `Apache-2.0 AND BSD-3-Clause AND MIT`, as `REUSE.toml`
annotates them. The rest is under the Apache License 2.0 (the pack's `LICENSE`): the generators `upstream/common.py`
and `upstream/phpstan/generate.py`, `upstream/phpstan/read_tests.php` and the expected findings they write from the tests'
expectations and by running PHPStan.

Paths are relative to the pack directory, `packs/php/`.

| File | Upstream path | Copy |
|---|---|---|
| `upstream/phpstan/fixtures/Analyser/nsrt/bug-10090.php` | `tests/PHPStan/Analyser/nsrt/bug-10090.php` | verbatim |
| `upstream/phpstan/fixtures/Analyser/nsrt/bug-14019.php` | `tests/PHPStan/Analyser/nsrt/bug-14019.php` | verbatim |
| `upstream/phpstan/fixtures/Analyser/nsrt/bug-6822.php` | `tests/PHPStan/Analyser/nsrt/bug-6822.php` | verbatim |
| `upstream/phpstan/fixtures/Analyser/nsrt/bug-6833.php` | `tests/PHPStan/Analyser/nsrt/bug-6833.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Arrays/data/bug-13013.php` | `tests/PHPStan/Rules/Arrays/data/bug-13013.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Arrays/data/bug-13022.php` | `tests/PHPStan/Rules/Arrays/data/bug-13022.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Arrays/data/bug-14041.php` | `tests/PHPStan/Rules/Arrays/data/bug-14041.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Arrays/data/bug-15244.php` | `tests/PHPStan/Rules/Arrays/data/bug-15244.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Arrays/data/bug-15247-php81.php` | `tests/PHPStan/Rules/Arrays/data/bug-15247-php81.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Arrays/data/bug-15247.php` | `tests/PHPStan/Rules/Arrays/data/bug-15247.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Arrays/data/bug-15248.php` | `tests/PHPStan/Rules/Arrays/data/bug-15248.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Arrays/data/duplicate-keys.php` | `tests/PHPStan/Rules/Arrays/data/duplicate-keys.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Comparison/data/bug-10561.php` | `tests/PHPStan/Rules/Comparison/data/bug-10561.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Comparison/data/bug-10903.php` | `tests/PHPStan/Rules/Comparison/data/bug-10903.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Comparison/data/bug-11417.php` | `tests/PHPStan/Rules/Comparison/data/bug-11417.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Comparison/data/bug-2499.php` | `tests/PHPStan/Rules/Comparison/data/bug-2499.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Comparison/data/bug-4043.php` | `tests/PHPStan/Rules/Comparison/data/bug-4043.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Comparison/data/bug-4284.php` | `tests/PHPStan/Rules/Comparison/data/bug-4284.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Comparison/data/bug-4302.php` | `tests/PHPStan/Rules/Comparison/data/bug-4302.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Comparison/data/bug-4864.php` | `tests/PHPStan/Rules/Comparison/data/bug-4864.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Comparison/data/bug-4912.php` | `tests/PHPStan/Rules/Comparison/data/bug-4912.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Comparison/data/bug-5020.php` | `tests/PHPStan/Rules/Comparison/data/bug-5020.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Comparison/data/bug-5370.php` | `tests/PHPStan/Rules/Comparison/data/bug-5370.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Comparison/data/bug-6211.php` | `tests/PHPStan/Rules/Comparison/data/bug-6211.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Comparison/data/bug-6902.php` | `tests/PHPStan/Rules/Comparison/data/bug-6902.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Comparison/data/bug-7491.php` | `tests/PHPStan/Rules/Comparison/data/bug-7491.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Comparison/data/bug-7699.php` | `tests/PHPStan/Rules/Comparison/data/bug-7699.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Comparison/data/bug-8485.php` | `tests/PHPStan/Rules/Comparison/data/bug-8485.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Comparison/data/bug-8926.php` | `tests/PHPStan/Rules/Comparison/data/bug-8926.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Comparison/data/bug-8980.php` | `tests/PHPStan/Rules/Comparison/data/bug-8980.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Comparison/data/call-condition-cross-file-a.php` | `tests/PHPStan/Rules/Comparison/data/call-condition-cross-file-a.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Comparison/data/call-condition-cross-file-b.php` | `tests/PHPStan/Rules/Comparison/data/call-condition-cross-file-b.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Comparison/data/call-condition-cross-file-marker-a.php` | `tests/PHPStan/Rules/Comparison/data/call-condition-cross-file-marker-a.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Comparison/data/call-condition-cross-file-marker-b.php` | `tests/PHPStan/Rules/Comparison/data/call-condition-cross-file-marker-b.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Comparison/data/constant-condition-function-call.php` | `tests/PHPStan/Rules/Comparison/data/constant-condition-function-call.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Comparison/data/if-condition-in-trait.php` | `tests/PHPStan/Rules/Comparison/data/if-condition-in-trait.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Comparison/data/if-condition-not-phpdoc.php` | `tests/PHPStan/Rules/Comparison/data/if-condition-not-phpdoc.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Comparison/data/if-condition.php` | `tests/PHPStan/Rules/Comparison/data/if-condition.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Functions/data/bug-10260.php` | `tests/PHPStan/Rules/Functions/data/bug-10260.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Functions/data/bug-13831.php` | `tests/PHPStan/Rules/Functions/data/bug-13831.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Functions/data/bug-14468.php` | `tests/PHPStan/Rules/Functions/data/bug-14468.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Functions/data/bug-14567.php` | `tests/PHPStan/Rules/Functions/data/bug-14567.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Functions/data/bug-1889.php` | `tests/PHPStan/Rules/Functions/data/bug-1889.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Functions/data/bug-2342.php` | `tests/PHPStan/Rules/Functions/data/bug-2342.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Functions/data/bug-4717.php` | `tests/PHPStan/Rules/Functions/data/bug-4717.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Functions/data/bug-6361.php` | `tests/PHPStan/Rules/Functions/data/bug-6361.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Functions/data/bug-8774.php` | `tests/PHPStan/Rules/Functions/data/bug-8774.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Functions/data/missing-function-return-typehint.php` | `tests/PHPStan/Rules/Functions/data/missing-function-return-typehint.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Functions/data/printf-format-parser.php` | `tests/PHPStan/Rules/Functions/data/printf-format-parser.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Functions/data/printf.php` | `tests/PHPStan/Rules/Functions/data/printf.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Functions/data/random-int.php` | `tests/PHPStan/Rules/Functions/data/random-int.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/TooWideTypehints/data/bug-13384b.php` | `tests/PHPStan/Rules/TooWideTypehints/data/bug-13384b.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/boolean-op-truthy-falsey.php` | `tests/PHPStan/Rules/Variables/data/boolean-op-truthy-falsey.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-1016.php` | `tests/PHPStan/Rules/Variables/data/bug-1016.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-1016b.php` | `tests/PHPStan/Rules/Variables/data/bug-1016b.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-10228.php` | `tests/PHPStan/Rules/Variables/data/bug-10228.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-10245.php` | `tests/PHPStan/Rules/Variables/data/bug-10245.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-10418.php` | `tests/PHPStan/Rules/Variables/data/bug-10418.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-10657.php` | `tests/PHPStan/Rules/Variables/data/bug-10657.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-10729.php` | `tests/PHPStan/Rules/Variables/data/bug-10729.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-10909.php` | `tests/PHPStan/Rules/Variables/data/bug-10909.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-11218.php` | `tests/PHPStan/Rules/Variables/data/bug-11218.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-11984.php` | `tests/PHPStan/Rules/Variables/data/bug-11984.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-12364.php` | `tests/PHPStan/Rules/Variables/data/bug-12364.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-12373.php` | `tests/PHPStan/Rules/Variables/data/bug-12373.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-12597.php` | `tests/PHPStan/Rules/Variables/data/bug-12597.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-12944.php` | `tests/PHPStan/Rules/Variables/data/bug-12944.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-12992.php` | `tests/PHPStan/Rules/Variables/data/bug-12992.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-1306.php` | `tests/PHPStan/Rules/Variables/data/bug-1306.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-13353.php` | `tests/PHPStan/Rules/Variables/data/bug-13353.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-13694.php` | `tests/PHPStan/Rules/Variables/data/bug-13694.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-13920.php` | `tests/PHPStan/Rules/Variables/data/bug-13920.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-13981.php` | `tests/PHPStan/Rules/Variables/data/bug-13981.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-14117.php` | `tests/PHPStan/Rules/Variables/data/bug-14117.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-14227.php` | `tests/PHPStan/Rules/Variables/data/bug-14227.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-14274.php` | `tests/PHPStan/Rules/Variables/data/bug-14274.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-14318.php` | `tests/PHPStan/Rules/Variables/data/bug-14318.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-14323.php` | `tests/PHPStan/Rules/Variables/data/bug-14323.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-2032.php` | `tests/PHPStan/Rules/Variables/data/bug-2032.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-2748.php` | `tests/PHPStan/Rules/Variables/data/bug-2748.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-3283.php` | `tests/PHPStan/Rules/Variables/data/bug-3283.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-3515.php` | `tests/PHPStan/Rules/Variables/data/bug-3515.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-3601.php` | `tests/PHPStan/Rules/Variables/data/bug-3601.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-393.php` | `tests/PHPStan/Rules/Variables/data/bug-393.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-4173.php` | `tests/PHPStan/Rules/Variables/data/bug-4173.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-4352.php` | `tests/PHPStan/Rules/Variables/data/bug-4352.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-4412.php` | `tests/PHPStan/Rules/Variables/data/bug-4412.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-5191.php` | `tests/PHPStan/Rules/Variables/data/bug-5191.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-5266.php` | `tests/PHPStan/Rules/Variables/data/bug-5266.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-5326.php` | `tests/PHPStan/Rules/Variables/data/bug-5326.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-5401.php` | `tests/PHPStan/Rules/Variables/data/bug-5401.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-5477.php` | `tests/PHPStan/Rules/Variables/data/bug-5477.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-5805.php` | `tests/PHPStan/Rules/Variables/data/bug-5805.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-5919.php` | `tests/PHPStan/Rules/Variables/data/bug-5919.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-6112.php` | `tests/PHPStan/Rules/Variables/data/bug-6112.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-6688.php` | `tests/PHPStan/Rules/Variables/data/bug-6688.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-6830.php` | `tests/PHPStan/Rules/Variables/data/bug-6830.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-7291.php` | `tests/PHPStan/Rules/Variables/data/bug-7291.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-7705.php` | `tests/PHPStan/Rules/Variables/data/bug-7705.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-8142.php` | `tests/PHPStan/Rules/Variables/data/bug-8142.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-8212.php` | `tests/PHPStan/Rules/Variables/data/bug-8212.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-8430.php` | `tests/PHPStan/Rules/Variables/data/bug-8430.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-8430b.php` | `tests/PHPStan/Rules/Variables/data/bug-8430b.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-8467c.php` | `tests/PHPStan/Rules/Variables/data/bug-8467c.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-8719.php` | `tests/PHPStan/Rules/Variables/data/bug-8719.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-9023.php` | `tests/PHPStan/Rules/Variables/data/bug-9023.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-9349.php` | `tests/PHPStan/Rules/Variables/data/bug-9349.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-9392.php` | `tests/PHPStan/Rules/Variables/data/bug-9392.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/bug-9474.php` | `tests/PHPStan/Rules/Variables/data/bug-9474.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/cli-arguments-variables.php` | `tests/PHPStan/Rules/Variables/data/cli-arguments-variables.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/dead-branches.php` | `tests/PHPStan/Rules/Variables/data/dead-branches.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/define-variables-class.php` | `tests/PHPStan/Rules/Variables/data/define-variables-class.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/defined-variables-anonymous-function-use.php` | `tests/PHPStan/Rules/Variables/data/defined-variables-anonymous-function-use.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/defined-variables-array-destructuring-short-syntax.php` | `tests/PHPStan/Rules/Variables/data/defined-variables-array-destructuring-short-syntax.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/defined-variables-arrow-functions.php` | `tests/PHPStan/Rules/Variables/data/defined-variables-arrow-functions.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/defined-variables-closures.php` | `tests/PHPStan/Rules/Variables/data/defined-variables-closures.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/defined-variables-coalesce-assign.php` | `tests/PHPStan/Rules/Variables/data/defined-variables-coalesce-assign.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/defined-variables-enum.php` | `tests/PHPStan/Rules/Variables/data/defined-variables-enum.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/defined-variables.php` | `tests/PHPStan/Rules/Variables/data/defined-variables.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/discussion-10252.php` | `tests/PHPStan/Rules/Variables/data/discussion-10252.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/dynamic-access.php` | `tests/PHPStan/Rules/Variables/data/dynamic-access.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/first-class-callables.php` | `tests/PHPStan/Rules/Variables/data/first-class-callables.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/foreach.php` | `tests/PHPStan/Rules/Variables/data/foreach.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/global-variables.php` | `tests/PHPStan/Rules/Variables/data/global-variables.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/isstring-certainty.php` | `tests/PHPStan/Rules/Variables/data/isstring-certainty.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/pass-by-reference-into-not-nullable.php` | `tests/PHPStan/Rules/Variables/data/pass-by-reference-into-not-nullable.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/property-hooks.php` | `tests/PHPStan/Rules/Variables/data/property-hooks.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/root-scope-maybe.php` | `tests/PHPStan/Rules/Variables/data/root-scope-maybe.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/this.php` | `tests/PHPStan/Rules/Variables/data/this.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/variable-nullsafe-isset.php` | `tests/PHPStan/Rules/Variables/data/variable-nullsafe-isset.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/variable-variable-name.php` | `tests/PHPStan/Rules/Variables/data/variable-variable-name.php` | verbatim |
| `upstream/phpstan/fixtures/Rules/Variables/data/yield-key-throw-point.php` | `tests/PHPStan/Rules/Variables/data/yield-key-throw-point.php` | verbatim |

# PHP_CodeSniffer 4.0.4

Upstream: [PHPCSStandards/PHP_CodeSniffer](https://github.com/PHPCSStandards/PHP_CodeSniffer/tree/bbdc3d0532623e21838b7041a4364383a8126f96) at tag `4.0.4` (commit `bbdc3d0532623e21838b7041a4364383a8126f96`). License: BSD-3-Clause (Copyright (c) 2012, Squiz Pty Ltd (ABN 77 084 670 600), Copyright (c) 2023, PHPCSStandards and contributors); [`licence.txt`](licence.txt) is the upstream license file, copied verbatim at that tag.

The files listed below are the unit-test fixtures of the sniffs the PHP pack ports (the pack's `conformance.json`) and remain under PHP_CodeSniffer's license. Every other file in this
pack is Rerail's. The rule documents in `rules/` and `candidates/rules/` are Rerail's re-implementations of PHP_CodeSniffer's, PHPStan's and
Psalm's rules, published under the pack's licence expression, `Apache-2.0 AND BSD-3-Clause AND MIT`, as `REUSE.toml`
annotates them. The rest is under the Apache License 2.0 (the pack's `LICENSE`): the generators `upstream/common.py`
and `upstream/phpcs/generate.py`, `upstream/phpcs/error_lists.php`, `upstream/phpcs/ruleset.xml` and the expected findings
they write by running PHP_CodeSniffer.

Paths are relative to the pack directory, `packs/php/`.

| File | Upstream path | Copy |
|---|---|---|
| `upstream/phpcs/fixtures/Generic/CodeAnalysis/EmptyStatementUnitTest.inc.php` | `src/Standards/Generic/Tests/CodeAnalysis/EmptyStatementUnitTest.inc` | verbatim, renamed with a `.php` suffix so the host routes it to PHP |
| `upstream/phpcs/fixtures/Generic/PHP/BacktickOperatorUnitTest.inc.php` | `src/Standards/Generic/Tests/PHP/BacktickOperatorUnitTest.inc` | verbatim, renamed with a `.php` suffix so the host routes it to PHP |
| `upstream/phpcs/fixtures/Generic/PHP/DeprecatedFunctionsUnitTest.inc.php` | `src/Standards/Generic/Tests/PHP/DeprecatedFunctionsUnitTest.inc` | verbatim, renamed with a `.php` suffix so the host routes it to PHP |
| `upstream/phpcs/fixtures/Generic/PHP/ForbiddenFunctionsUnitTest.inc.php` | `src/Standards/Generic/Tests/PHP/ForbiddenFunctionsUnitTest.inc` | verbatim, renamed with a `.php` suffix so the host routes it to PHP |
| `upstream/phpcs/fixtures/Generic/PHP/LowerCaseConstantUnitTest.1.inc.php` | `src/Standards/Generic/Tests/PHP/LowerCaseConstantUnitTest.1.inc` | verbatim, renamed with a `.php` suffix so the host routes it to PHP |
| `upstream/phpcs/fixtures/Generic/PHP/LowerCaseConstantUnitTest.2.inc.php` | `src/Standards/Generic/Tests/PHP/LowerCaseConstantUnitTest.2.inc` | verbatim, renamed with a `.php` suffix so the host routes it to PHP |
| `upstream/phpcs/fixtures/Generic/PHP/NoSilencedErrorsUnitTest.inc.php` | `src/Standards/Generic/Tests/PHP/NoSilencedErrorsUnitTest.inc` | verbatim, renamed with a `.php` suffix so the host routes it to PHP |
| `upstream/phpcs/fixtures/Squiz/PHP/DisallowSizeFunctionsInLoopsUnitTest.inc.php` | `src/Standards/Squiz/Tests/PHP/DisallowSizeFunctionsInLoopsUnitTest.inc` | verbatim, renamed with a `.php` suffix so the host routes it to PHP |
| `upstream/phpcs/fixtures/Squiz/PHP/EvalUnitTest.inc.php` | `src/Standards/Squiz/Tests/PHP/EvalUnitTest.inc` | verbatim, renamed with a `.php` suffix so the host routes it to PHP |

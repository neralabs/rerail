# fortephp/forte 1.1.0

Upstream: [fortephp/forte](https://github.com/fortephp/forte/tree/v1.1.0) at tag `v1.1.0` (commit `9c9597d0b7080d16c25f53a940a0449ff1aab19b`). License: MIT (Copyright (c) Johnathon Koster); [`LICENSE.md`](LICENSE.md) is the upstream license file (`license.md`), copied verbatim at that commit.

The files listed below contain material copied from fortephp/forte and remain under its license. Every other file in
this pack is Rerail's, under the Apache License 2.0 (this repository's `LICENSE`): the `forte/*.php` scripts and the
trees they record by running Forte.

Paths are relative to `blade/`.

| File | Upstream path | Copy |
|---|---|---|
| `blade/forte/suite.expected.json` | `tests/**/*Test.php` and `tests/Fixtures/**` | derived: every template Forte's own test suite (Pest, 5,348 tests) hands its lexer, recorded by `blade/forte/record_suite.php` while the suite runs, each verbatim as its case's `source` (or `source_base64`) with the test files that lexed it; templates the suite lexes prefix by prefix are recorded once, whole. The digests are Rerail's recording. |

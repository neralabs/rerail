# Ruff 0.16.9

Upstream: [Ruff 0.16.9](https://github.com/astral-sh/ruff/tree/0.16.9). License: MIT (Copyright (c) 2022 Charles Marsh); [`LICENSE`](LICENSE) is the upstream license file, copied verbatim at that tag.

Source: https://github.com/astral-sh/ruff
Revision: 0be08a206f9c3180afd3e93bcc792ed5cb1f4db1

The files listed below contain material copied from Ruff and remain under its license. Every other file in this
pack is Rerail's. The rule documents in `rules/` are Rerail's re-implementations of Bandit's and Ruff's rules,
published under the pack's licence expression, `Apache-2.0 AND MIT`, as `REUSE.toml` annotates them. The rest is under
the Apache License 2.0 (the pack's `LICENSE`): the recorders `upstream/common.py` and `upstream/ruff/generate.py`,
and the `*.expected.json` outputs recorded by running Ruff. `upstream/ruff/source.json`, the facts `conformance/recorders/python-source.ts` records from the upstream source and tests at the revision above, holds names, words and regexes extracted from them: it is Ruff's material and Rerail's, under `MIT AND Apache-2.0`, as `REUSE.toml` annotates it, and the table lists it.

Ruff's `LICENSE` also carries the notices of the projects Ruff derives from, Pyflakes among them. Ruff's pyflakes fixtures come with that notice.

Paths are relative to the pack directory, `packs/python/`.

| File | Upstream path | Copy |
|---|---|---|
| `upstream/ruff/source.json` | the rules' implementations and tests, and the helpers they delegate to, at the revision above | extracted: the names, words and regexes `source.json` records |
| `upstream/ruff/fixtures/flake8_bugbear/B011.py` | `crates/ruff_linter/resources/test/fixtures/flake8_bugbear/B011.py` | verbatim |
| `upstream/ruff/fixtures/flake8_bugbear/B028.py` | `crates/ruff_linter/resources/test/fixtures/flake8_bugbear/B028.py` | verbatim |
| `upstream/ruff/fixtures/flake8_bugbear/B905.py` | `crates/ruff_linter/resources/test/fixtures/flake8_bugbear/B905.py` | verbatim |
| `upstream/ruff/fixtures/flake8_pie/PIE796.py` | `crates/ruff_linter/resources/test/fixtures/flake8_pie/PIE796.py` | verbatim |
| `upstream/ruff/fixtures/flake8_pie/PIE796.pyi` | `crates/ruff_linter/resources/test/fixtures/flake8_pie/PIE796.pyi` | verbatim |
| `upstream/ruff/fixtures/flake8_simplify/SIM222.py` | `crates/ruff_linter/resources/test/fixtures/flake8_simplify/SIM222.py` | verbatim |
| `upstream/ruff/fixtures/pycodestyle/E711.py` | `crates/ruff_linter/resources/test/fixtures/pycodestyle/E711.py` | verbatim |
| `upstream/ruff/fixtures/pycodestyle/E722.py` | `crates/ruff_linter/resources/test/fixtures/pycodestyle/E722.py` | verbatim |
| `upstream/ruff/fixtures/pycodestyle/constant_literals.py` | `crates/ruff_linter/resources/test/fixtures/pycodestyle/constant_literals.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F50x.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F50x.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F523.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F523.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F524.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F524.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F601.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F601.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F632.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F632.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F722.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F722.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F722_1.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F722_1.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_0.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_0.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_1.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_1.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_10.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_10.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_11.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_11.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_12.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_12.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_13.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_13.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_14.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_14.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_15.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_15.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_16.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_16.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_17.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_17.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_18.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_18.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_19.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_19.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_2.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_2.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_20.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_20.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_21.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_21.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_22.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_22.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_23.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_23.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_24.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_24.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_25.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_25.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_26.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_26.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_27.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_27.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_28.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_28.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_29.pyi` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_29.pyi` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_3.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_3.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_30.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_30.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_31.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_31.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_32.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_32.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_33.pyi` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_33.pyi` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_35.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_35.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_36.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_36.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_4.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_4.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_5.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_5.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_6.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_6.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_7.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_7.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_8.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_8.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F811_9.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F811_9.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_0.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_0.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_1.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_1.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_10.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_10.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_11.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_11.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_11.pyi` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_11.pyi` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_12.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_12.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_13.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_13.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_14.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_14.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_15.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_15.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_16.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_16.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_17.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_17.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_18.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_18.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_19.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_19.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_2.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_2.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_20.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_20.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_21.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_21.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_23.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_23.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_24.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_24.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_25.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_25.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_26.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_26.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_26.pyi` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_26.pyi` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_27.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_27.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_28.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_28.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_29.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_29.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_3.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_3.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_30.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_30.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_31.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_31.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_32.pyi` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_32.pyi` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_33.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_33.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_34.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_34.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_34.pyi` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_34.pyi` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_4.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_4.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_5.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_5.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_5.pyi` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_5.pyi` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_6.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_6.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_7.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_7.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_8.pyi` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_8.pyi` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F821_9.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F821_9.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F823.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F823.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F841_0.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F841_0.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F841_1.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F841_1.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F841_2.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F841_2.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F841_3.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F841_3.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/__init__.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/__init__.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/builtins.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/builtins.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/future_annotations.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/future_annotations.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/typing_modules.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/typing_modules.py` | verbatim |
| `upstream/ruff/fixtures/ruff/RUF100_3.py` | `crates/ruff_linter/resources/test/fixtures/ruff/RUF100_3.py` | verbatim |
| `upstream/ruff/fixtures/ruff/RUF100_7.py` | `crates/ruff_linter/resources/test/fixtures/ruff/RUF100_7.py` | verbatim |
| `upstream/ruff/fixtures/ruff/flake8_noqa.py` | `crates/ruff_linter/resources/test/fixtures/ruff/flake8_noqa.py` | verbatim |
| `upstream/ruff/fixtures/ruff/noqa.py` | `crates/ruff_linter/resources/test/fixtures/ruff/noqa.py` | verbatim |
| `upstream/ruff/fixtures/ruff/ruff_noqa_all.py` | `crates/ruff_linter/resources/test/fixtures/ruff/ruff_noqa_all.py` | verbatim |
| `upstream/ruff/fixtures/ruff/ruff_noqa_codes.py` | `crates/ruff_linter/resources/test/fixtures/ruff/ruff_noqa_codes.py` | verbatim |
| `upstream/ruff/fixtures/ruff/ruff_noqa_invalid.py` | `crates/ruff_linter/resources/test/fixtures/ruff/ruff_noqa_invalid.py` | verbatim |

# Ruff 0.16.9

Upstream: [Ruff 0.16.9](https://github.com/astral-sh/ruff/tree/0.16.9). License: MIT (Copyright (c) 2022 Charles Marsh); [`LICENSE`](LICENSE) is the upstream license file, copied verbatim at that tag.

The files listed below contain material copied from Ruff and remain under its license. Every other file in this pack
is Rerail's. The rule documents in `rules/` and `candidates/rules/` are Rerail's re-implementations of Bandit's and
Ruff's rules, published under the pack's licence expression, `Apache-2.0 AND MIT`, as `REUSE.toml` annotates them. The
rest is under the Apache License 2.0 (the pack's `LICENSE`): the recorders `upstream/common.py` and
`upstream/ruff/generate.py`, and the `*.expected.json` outputs recorded by running Ruff.

Ruff's `LICENSE` also carries the notices of the projects Ruff derives from, Pyflakes among them. Ruff's pyflakes fixtures come with that notice.

Paths are relative to the pack directory, `packs/python/`.

| File | Upstream path | Copy |
|---|---|---|
| `upstream/ruff/fixtures/flake8_async/ASYNC100.py` | `crates/ruff_linter/resources/test/fixtures/flake8_async/ASYNC100.py` | verbatim |
| `upstream/ruff/fixtures/flake8_async/ASYNC105.py` | `crates/ruff_linter/resources/test/fixtures/flake8_async/ASYNC105.py` | verbatim |
| `upstream/ruff/fixtures/flake8_async/ASYNC115.py` | `crates/ruff_linter/resources/test/fixtures/flake8_async/ASYNC115.py` | verbatim |
| `upstream/ruff/fixtures/flake8_async/ASYNC116.py` | `crates/ruff_linter/resources/test/fixtures/flake8_async/ASYNC116.py` | verbatim |
| `upstream/ruff/fixtures/flake8_async/ASYNC210.py` | `crates/ruff_linter/resources/test/fixtures/flake8_async/ASYNC210.py` | verbatim |
| `upstream/ruff/fixtures/flake8_async/ASYNC22x.py` | `crates/ruff_linter/resources/test/fixtures/flake8_async/ASYNC22x.py` | verbatim |
| `upstream/ruff/fixtures/flake8_async/ASYNC230.py` | `crates/ruff_linter/resources/test/fixtures/flake8_async/ASYNC230.py` | verbatim |
| `upstream/ruff/fixtures/flake8_async/ASYNC251.py` | `crates/ruff_linter/resources/test/fixtures/flake8_async/ASYNC251.py` | verbatim |
| `upstream/ruff/fixtures/flake8_bugbear/B002.py` | `crates/ruff_linter/resources/test/fixtures/flake8_bugbear/B002.py` | verbatim |
| `upstream/ruff/fixtures/flake8_bugbear/B003.py` | `crates/ruff_linter/resources/test/fixtures/flake8_bugbear/B003.py` | verbatim |
| `upstream/ruff/fixtures/flake8_bugbear/B004.py` | `crates/ruff_linter/resources/test/fixtures/flake8_bugbear/B004.py` | verbatim |
| `upstream/ruff/fixtures/flake8_bugbear/B005.py` | `crates/ruff_linter/resources/test/fixtures/flake8_bugbear/B005.py` | verbatim |
| `upstream/ruff/fixtures/flake8_bugbear/B009_B010.py` | `crates/ruff_linter/resources/test/fixtures/flake8_bugbear/B009_B010.py` | verbatim |
| `upstream/ruff/fixtures/flake8_bugbear/B013.py` | `crates/ruff_linter/resources/test/fixtures/flake8_bugbear/B013.py` | verbatim |
| `upstream/ruff/fixtures/flake8_bugbear/B014.py` | `crates/ruff_linter/resources/test/fixtures/flake8_bugbear/B014.py` | verbatim |
| `upstream/ruff/fixtures/flake8_bugbear/B016.py` | `crates/ruff_linter/resources/test/fixtures/flake8_bugbear/B016.py` | verbatim |
| `upstream/ruff/fixtures/flake8_bugbear/B017_0.py` | `crates/ruff_linter/resources/test/fixtures/flake8_bugbear/B017_0.py` | verbatim |
| `upstream/ruff/fixtures/flake8_bugbear/B017_1.py` | `crates/ruff_linter/resources/test/fixtures/flake8_bugbear/B017_1.py` | verbatim |
| `upstream/ruff/fixtures/flake8_bugbear/B019.py` | `crates/ruff_linter/resources/test/fixtures/flake8_bugbear/B019.py` | verbatim |
| `upstream/ruff/fixtures/flake8_bugbear/B020.py` | `crates/ruff_linter/resources/test/fixtures/flake8_bugbear/B020.py` | verbatim |
| `upstream/ruff/fixtures/flake8_bugbear/B021.py` | `crates/ruff_linter/resources/test/fixtures/flake8_bugbear/B021.py` | verbatim |
| `upstream/ruff/fixtures/flake8_bugbear/B022.py` | `crates/ruff_linter/resources/test/fixtures/flake8_bugbear/B022.py` | verbatim |
| `upstream/ruff/fixtures/flake8_bugbear/B023.py` | `crates/ruff_linter/resources/test/fixtures/flake8_bugbear/B023.py` | verbatim |
| `upstream/ruff/fixtures/flake8_bugbear/B025.py` | `crates/ruff_linter/resources/test/fixtures/flake8_bugbear/B025.py` | verbatim |
| `upstream/ruff/fixtures/flake8_bugbear/B026.py` | `crates/ruff_linter/resources/test/fixtures/flake8_bugbear/B026.py` | verbatim |
| `upstream/ruff/fixtures/flake8_bugbear/B029.py` | `crates/ruff_linter/resources/test/fixtures/flake8_bugbear/B029.py` | verbatim |
| `upstream/ruff/fixtures/flake8_bugbear/B032.py` | `crates/ruff_linter/resources/test/fixtures/flake8_bugbear/B032.py` | verbatim |
| `upstream/ruff/fixtures/flake8_bugbear/B033.py` | `crates/ruff_linter/resources/test/fixtures/flake8_bugbear/B033.py` | verbatim |
| `upstream/ruff/fixtures/flake8_bugbear/B039.py` | `crates/ruff_linter/resources/test/fixtures/flake8_bugbear/B039.py` | verbatim |
| `upstream/ruff/fixtures/flake8_debugger/T100.py` | `crates/ruff_linter/resources/test/fixtures/flake8_debugger/T100.py` | verbatim |
| `upstream/ruff/fixtures/flake8_pie/PIE790.py` | `crates/ruff_linter/resources/test/fixtures/flake8_pie/PIE790.py` | verbatim |
| `upstream/ruff/fixtures/flake8_pie/PIE794.py` | `crates/ruff_linter/resources/test/fixtures/flake8_pie/PIE794.py` | verbatim |
| `upstream/ruff/fixtures/flake8_pie/PIE796.py` | `crates/ruff_linter/resources/test/fixtures/flake8_pie/PIE796.py` | verbatim |
| `upstream/ruff/fixtures/flake8_pie/PIE796.pyi` | `crates/ruff_linter/resources/test/fixtures/flake8_pie/PIE796.pyi` | verbatim |
| `upstream/ruff/fixtures/flake8_pie/PIE800.py` | `crates/ruff_linter/resources/test/fixtures/flake8_pie/PIE800.py` | verbatim |
| `upstream/ruff/fixtures/flake8_pie/PIE804.py` | `crates/ruff_linter/resources/test/fixtures/flake8_pie/PIE804.py` | verbatim |
| `upstream/ruff/fixtures/flake8_pie/PIE807.py` | `crates/ruff_linter/resources/test/fixtures/flake8_pie/PIE807.py` | verbatim |
| `upstream/ruff/fixtures/flake8_pie/PIE808.py` | `crates/ruff_linter/resources/test/fixtures/flake8_pie/PIE808.py` | verbatim |
| `upstream/ruff/fixtures/flake8_pie/PIE810.py` | `crates/ruff_linter/resources/test/fixtures/flake8_pie/PIE810.py` | verbatim |
| `upstream/ruff/fixtures/flake8_pyi/PYI021_1.pyi` | `crates/ruff_linter/resources/test/fixtures/flake8_pyi/PYI021_1.pyi` | verbatim |
| `upstream/ruff/fixtures/flake8_return/RET501.py` | `crates/ruff_linter/resources/test/fixtures/flake8_return/RET501.py` | verbatim |
| `upstream/ruff/fixtures/flake8_simplify/SIM101.py` | `crates/ruff_linter/resources/test/fixtures/flake8_simplify/SIM101.py` | verbatim |
| `upstream/ruff/fixtures/flake8_simplify/SIM107.py` | `crates/ruff_linter/resources/test/fixtures/flake8_simplify/SIM107.py` | verbatim |
| `upstream/ruff/fixtures/flake8_simplify/SIM113.py` | `crates/ruff_linter/resources/test/fixtures/flake8_simplify/SIM113.py` | verbatim |
| `upstream/ruff/fixtures/flake8_simplify/SIM115.py` | `crates/ruff_linter/resources/test/fixtures/flake8_simplify/SIM115.py` | verbatim |
| `upstream/ruff/fixtures/flake8_simplify/SIM201.py` | `crates/ruff_linter/resources/test/fixtures/flake8_simplify/SIM201.py` | verbatim |
| `upstream/ruff/fixtures/flake8_simplify/SIM202.py` | `crates/ruff_linter/resources/test/fixtures/flake8_simplify/SIM202.py` | verbatim |
| `upstream/ruff/fixtures/flake8_simplify/SIM208.py` | `crates/ruff_linter/resources/test/fixtures/flake8_simplify/SIM208.py` | verbatim |
| `upstream/ruff/fixtures/flake8_simplify/SIM210.py` | `crates/ruff_linter/resources/test/fixtures/flake8_simplify/SIM210.py` | verbatim |
| `upstream/ruff/fixtures/flake8_simplify/SIM211.py` | `crates/ruff_linter/resources/test/fixtures/flake8_simplify/SIM211.py` | verbatim |
| `upstream/ruff/fixtures/flake8_simplify/SIM220.py` | `crates/ruff_linter/resources/test/fixtures/flake8_simplify/SIM220.py` | verbatim |
| `upstream/ruff/fixtures/flake8_simplify/SIM221.py` | `crates/ruff_linter/resources/test/fixtures/flake8_simplify/SIM221.py` | verbatim |
| `upstream/ruff/fixtures/flake8_simplify/SIM905.py` | `crates/ruff_linter/resources/test/fixtures/flake8_simplify/SIM905.py` | verbatim |
| `upstream/ruff/fixtures/pycodestyle/E722.py` | `crates/ruff_linter/resources/test/fixtures/pycodestyle/E722.py` | verbatim |
| `upstream/ruff/fixtures/pycodestyle/constant_literals.py` | `crates/ruff_linter/resources/test/fixtures/pycodestyle/constant_literals.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F404_0.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F404_0.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F404_1.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F404_1.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F407.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F407.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F502.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F502.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F503.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F503.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F504.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F504.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F50x.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F50x.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F521.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F521.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F522.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F522.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F523.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F523.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F524.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F524.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F525.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F525.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F601.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F601.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F602.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F602.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F622.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F622.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F631.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F631.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F632.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F632.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F633.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F633.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F634.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F634.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F701.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F701.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F702.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F702.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F706.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F706.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F707.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F707.py` | verbatim |
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
| `upstream/ruff/fixtures/pyflakes/F823.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F823.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F842.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F842.py` | verbatim |
| `upstream/ruff/fixtures/pyflakes/F901.py` | `crates/ruff_linter/resources/test/fixtures/pyflakes/F901.py` | verbatim |
| `upstream/ruff/fixtures/syntax_errors/late_future_import.py` | `crates/ruff_linter/resources/test/fixtures/syntax_errors/late_future_import.py` | verbatim |
| `upstream/ruff/fixtures/syntax_errors/return_outside_function.py` | `crates/ruff_linter/resources/test/fixtures/syntax_errors/return_outside_function.py` | verbatim |

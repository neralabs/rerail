# rust-clippy (Rust 1.99.0)

Upstream: [rust-lang/rust-clippy](https://github.com/rust-lang/rust-clippy/tree/49e2f89a2605a9a340e04955fdbefa9803e6027a) at tag `rust-1.99.0` (commit `49e2f89a2605a9a340e04955fdbefa9803e6027a`), the clippy shipped as `clippy 0.1.99 (b940084d7e 2026-09-28)` with Rust 1.99.0. License: MIT OR Apache-2.0 (Copyright (c) The Rust Project Contributors); [`LICENSE-MIT`](LICENSE-MIT), [`LICENSE-APACHE`](LICENSE-APACHE) and [`COPYRIGHT`](COPYRIGHT) are the upstream license files, copied verbatim at that tag.

The files listed below are UI tests of the clippy lints the Rust pack ports (`packs/rust/conformance.json`), the auxiliary crates those tests build, and the `clippy.toml` files they read. They remain under clippy's license. Every other file under `upstream/clippy/` is Rerail's, under the Apache License 2.0 (this repository's `LICENSE`; `upstream/clippy/UPSTREAM.md` lists the cases under `fixtures/rerail/`): `generate.py`, `../common.py` and the recordings they write by running clippy (`fixtures.expected.json`, `fixtures.upstream-check.json`, `corpus-*.expected.json`), and the port readiness gates' `gates.yaml`, `recording.lock` and `goldens.tsv`. The port readiness recording itself is not in the tree and holds no clippy test text: it names each test file by path and sha256, and the gates fetch them from rust-clippy at the pinned commit.

Paths are relative to `packs/rust/`; each file keeps its upstream path under `upstream/clippy/fixtures/`.

| File | Upstream path | Copy | Role |
|---|---|---|---|
| `upstream/clippy/fixtures/tests/clippy.toml` | `tests/clippy.toml` | verbatim | clippy configuration of every tests/ui test (`CLIPPY_CONF_DIR=tests`) |
| `upstream/clippy/fixtures/tests/ui-toml/decimal_literal_representation/clippy.toml` | `tests/ui-toml/decimal_literal_representation/clippy.toml` | verbatim | clippy configuration of the tests/ui-toml test beside it |
| `upstream/clippy/fixtures/tests/ui-toml/decimal_literal_representation/decimal_literal_representation.rs` | `tests/ui-toml/decimal_literal_representation/decimal_literal_representation.rs` | verbatim | UI test of `clippy::decimal_literal_representation` |
| `upstream/clippy/fixtures/tests/ui-toml/lint_decimal_readability/clippy.toml` | `tests/ui-toml/lint_decimal_readability/clippy.toml` | verbatim | clippy configuration of the tests/ui-toml test beside it |
| `upstream/clippy/fixtures/tests/ui-toml/lint_decimal_readability/test.rs` | `tests/ui-toml/lint_decimal_readability/test.rs` | verbatim | UI test of `clippy::inconsistent_digit_grouping`, `clippy::unreadable_literal` |
| `upstream/clippy/fixtures/tests/ui-toml/needless_raw_string_hashes_one_allowed/clippy.toml` | `tests/ui-toml/needless_raw_string_hashes_one_allowed/clippy.toml` | verbatim | clippy configuration of the tests/ui-toml test beside it |
| `upstream/clippy/fixtures/tests/ui-toml/needless_raw_string_hashes_one_allowed/needless_raw_string_hashes.rs` | `tests/ui-toml/needless_raw_string_hashes_one_allowed/needless_raw_string_hashes.rs` | verbatim | UI test of `clippy::needless_raw_string_hashes` |
| `upstream/clippy/fixtures/tests/ui/allow_attributes.rs` | `tests/ui/allow_attributes.rs` | verbatim | UI test of `clippy::allow_attributes` |
| `upstream/clippy/fixtures/tests/ui/allow_attributes_without_reason.rs` | `tests/ui/allow_attributes_without_reason.rs` | verbatim | UI test of `clippy::allow_attributes_without_reason` |
| `upstream/clippy/fixtures/tests/ui/auxiliary/macro_rules.rs` | `tests/ui/auxiliary/macro_rules.rs` | verbatim | aux crate (`//@aux-build`) of `double_parens.rs` |
| `upstream/clippy/fixtures/tests/ui/auxiliary/proc_macro_attr.rs` | `tests/ui/auxiliary/proc_macro_attr.rs` | verbatim | aux crate (`//@aux-build`) of `needless_arbitrary_self_type_unfixable.rs` |
| `upstream/clippy/fixtures/tests/ui/auxiliary/proc_macro_derive.rs` | `tests/ui/auxiliary/proc_macro_derive.rs` | verbatim | aux crate (`//@aux-build`) of `allow_attributes.rs`, `double_parens.rs` |
| `upstream/clippy/fixtures/tests/ui/auxiliary/proc_macros.rs` | `tests/ui/auxiliary/proc_macros.rs` | verbatim | aux crate (`//@aux-build`) of `allow_attributes.rs`, `allow_attributes_without_reason.rs`, `double_parens.rs`, `option_env_unwrap.rs`, `redundant_at_rest_pattern.rs`, `redundant_field_names.rs`, `unneeded_wildcard_pattern.rs` |
| `upstream/clippy/fixtures/tests/ui/builtin_type_shadow.rs` | `tests/ui/builtin_type_shadow.rs` | verbatim | UI test of `clippy::builtin_type_shadow` |
| `upstream/clippy/fixtures/tests/ui/decimal_literal_representation.rs` | `tests/ui/decimal_literal_representation.rs` | verbatim | UI test of `clippy::decimal_literal_representation` |
| `upstream/clippy/fixtures/tests/ui/double_parens.rs` | `tests/ui/double_parens.rs` | verbatim | UI test of `clippy::double_parens` |
| `upstream/clippy/fixtures/tests/ui/else_if_without_else.rs` | `tests/ui/else_if_without_else.rs` | verbatim | UI test of `clippy::else_if_without_else` |
| `upstream/clippy/fixtures/tests/ui/inconsistent_digit_grouping.rs` | `tests/ui/inconsistent_digit_grouping.rs` | verbatim | UI test of `clippy::inconsistent_digit_grouping`, `clippy::unreadable_literal` |
| `upstream/clippy/fixtures/tests/ui/int_plus_one.rs` | `tests/ui/int_plus_one.rs` | verbatim | UI test of `clippy::int_plus_one` |
| `upstream/clippy/fixtures/tests/ui/large_digit_groups.rs` | `tests/ui/large_digit_groups.rs` | verbatim | UI test of `clippy::large_digit_groups`, `clippy::unusual_byte_groupings` |
| `upstream/clippy/fixtures/tests/ui/literals.rs` | `tests/ui/literals.rs` | verbatim | UI test of `clippy::inconsistent_digit_grouping`, `clippy::mixed_case_hex_literals`, `clippy::unusual_byte_groupings`, `clippy::zero_prefixed_literal` |
| `upstream/clippy/fixtures/tests/ui/many_single_char_names.rs` | `tests/ui/many_single_char_names.rs` | verbatim | UI test of `clippy::many_single_char_names` |
| `upstream/clippy/fixtures/tests/ui/needless_arbitrary_self_type.rs` | `tests/ui/needless_arbitrary_self_type.rs` | verbatim | UI test of `clippy::needless_arbitrary_self_type` |
| `upstream/clippy/fixtures/tests/ui/needless_arbitrary_self_type_unfixable.rs` | `tests/ui/needless_arbitrary_self_type_unfixable.rs` | verbatim | UI test of `clippy::needless_arbitrary_self_type` |
| `upstream/clippy/fixtures/tests/ui/needless_else.rs` | `tests/ui/needless_else.rs` | verbatim | UI test of `clippy::needless_else` |
| `upstream/clippy/fixtures/tests/ui/needless_raw_string.rs` | `tests/ui/needless_raw_string.rs` | verbatim | UI test of `clippy::needless_raw_strings` |
| `upstream/clippy/fixtures/tests/ui/needless_raw_string_hashes.rs` | `tests/ui/needless_raw_string_hashes.rs` | verbatim | UI test of `clippy::needless_raw_string_hashes` |
| `upstream/clippy/fixtures/tests/ui/octal_escapes.rs` | `tests/ui/octal_escapes.rs` | verbatim | UI test of `clippy::octal_escapes` |
| `upstream/clippy/fixtures/tests/ui/option_env_unwrap.rs` | `tests/ui/option_env_unwrap.rs` | verbatim | UI test of `clippy::option_env_unwrap` |
| `upstream/clippy/fixtures/tests/ui/partial_pub_fields.rs` | `tests/ui/partial_pub_fields.rs` | verbatim | UI test of `clippy::partial_pub_fields` |
| `upstream/clippy/fixtures/tests/ui/precedence.rs` | `tests/ui/precedence.rs` | verbatim | UI test of `clippy::precedence` |
| `upstream/clippy/fixtures/tests/ui/precedence_bits.rs` | `tests/ui/precedence_bits.rs` | verbatim | UI test of `clippy::precedence_bits` |
| `upstream/clippy/fixtures/tests/ui/redundant_at_rest_pattern.rs` | `tests/ui/redundant_at_rest_pattern.rs` | verbatim | UI test of `clippy::redundant_at_rest_pattern` |
| `upstream/clippy/fixtures/tests/ui/redundant_field_names.rs` | `tests/ui/redundant_field_names.rs` | verbatim | UI test of `clippy::redundant_field_names` |
| `upstream/clippy/fixtures/tests/ui/redundant_static_lifetimes.rs` | `tests/ui/redundant_static_lifetimes.rs` | verbatim | UI test of `clippy::redundant_static_lifetimes` |
| `upstream/clippy/fixtures/tests/ui/redundant_static_lifetimes_multiple.rs` | `tests/ui/redundant_static_lifetimes_multiple.rs` | verbatim | UI test of `clippy::redundant_static_lifetimes` |
| `upstream/clippy/fixtures/tests/ui/single_char_lifetime_names.rs` | `tests/ui/single_char_lifetime_names.rs` | verbatim | UI test of `clippy::single_char_lifetime_names` |
| `upstream/clippy/fixtures/tests/ui/single_component_path_imports.rs` | `tests/ui/single_component_path_imports.rs` | verbatim | UI test of `clippy::single_component_path_imports` |
| `upstream/clippy/fixtures/tests/ui/single_component_path_imports_macro.rs` | `tests/ui/single_component_path_imports_macro.rs` | verbatim | UI test of `clippy::single_component_path_imports` |
| `upstream/clippy/fixtures/tests/ui/single_component_path_imports_nested_first.rs` | `tests/ui/single_component_path_imports_nested_first.rs` | verbatim | UI test of `clippy::single_component_path_imports` |
| `upstream/clippy/fixtures/tests/ui/single_component_path_imports_self_after.rs` | `tests/ui/single_component_path_imports_self_after.rs` | verbatim | UI test of `clippy::single_component_path_imports` |
| `upstream/clippy/fixtures/tests/ui/single_component_path_imports_self_before.rs` | `tests/ui/single_component_path_imports_self_before.rs` | verbatim | UI test of `clippy::single_component_path_imports` |
| `upstream/clippy/fixtures/tests/ui/unneeded_wildcard_pattern.rs` | `tests/ui/unneeded_wildcard_pattern.rs` | verbatim | UI test of `clippy::unneeded_wildcard_pattern` |
| `upstream/clippy/fixtures/tests/ui/unnested_or_patterns.rs` | `tests/ui/unnested_or_patterns.rs` | verbatim | UI test of `clippy::unnested_or_patterns` |
| `upstream/clippy/fixtures/tests/ui/unnested_or_patterns2.rs` | `tests/ui/unnested_or_patterns2.rs` | verbatim | UI test of `clippy::unnested_or_patterns` |
| `upstream/clippy/fixtures/tests/ui/unreadable_literal.rs` | `tests/ui/unreadable_literal.rs` | verbatim | UI test of `clippy::unreadable_literal` |

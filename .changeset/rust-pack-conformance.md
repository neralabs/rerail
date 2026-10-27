---
"@neralabs/rerail-pack-rust": patch
---

Rust pack: eighteen Clippy 0.1.99 lints pass the conformance check and ship. On: `builtin_type_shadow`, `inconsistent_digit_grouping`, `int_plus_one`, `mixed_case_hex_literals`, `needless_arbitrary_self_type`, `needless_else`, `octal_escapes`, `precedence`, `redundant_at_rest_pattern`, `single_component_path_imports`, `unneeded_wildcard_pattern`, `unusual_byte_groupings` and `zero_prefixed_literal`. Off, as Clippy's default lint levels leave them (pedantic or restriction): `large_digit_groups`, `many_single_char_names`, `needless_raw_string_hashes`, `partial_pub_fields` and `precedence_bits`. 16 ports remain candidates.

// Rerail-authored fixture (Apache-2.0). Rust literals rustc 1.99.0 accepts, one `case!(name, type: literal)` each;
// `_` keeps the literal's own type (suffix or default). generate.py compiles this file as a module of a
// harness that defines `case!` and prints every value.
#![allow(unused)]

pub fn run() {
    // Integers: decimal, leading zeros (still decimal), separators anywhere after the first digit.
    case!(int_zero, _: 0);
    case!(int_decimal, _: 123);
    case!(int_leading_zeros, _: 007);
    case!(int_separators, _: 1_000_000);
    case!(int_separators_odd, _: 1__2_);
    case!(int_i32_max, _: 2147483647);
    // Hexadecimal, octal, binary; separators after the prefix; digits are case-insensitive.
    case!(int_hex_lower, _: 0xff);
    case!(int_hex_upper, _: 0xFF);
    case!(int_hex_mixed, _: 0xDEAD_beef_u32);
    case!(int_hex_separator_first, _: 0x_ff);
    case!(int_octal, _: 0o777);
    case!(int_octal_separator_first, _: 0o_7);
    case!(int_binary, _: 0b1010);
    case!(int_binary_separators, _: 0b_1_0);
    // Suffixes, with and without a separator.
    case!(int_u8_max, _: 255u8);
    case!(int_i8_max, _: 127i8);
    case!(int_u16, _: 65535u16);
    case!(int_i16, _: 32767_i16);
    case!(int_u32, _: 4294967295u32);
    case!(int_i64, _: 9223372036854775807i64);
    case!(int_u64_max, _: 18446744073709551615u64);
    case!(int_u128_max, _: 340282366920938463463374607431768211455u128);
    case!(int_i128_max, _: 170141183460469231731687303715884105727i128);
    case!(int_usize, _: 1usize);
    case!(int_isize, _: 1_isize);
    case!(int_binary_u8, _: 0b1111_1111u8);
    case!(int_octal_u16, _: 0o177777u16);
    case!(int_hex_u128_max, _: 0xffff_ffff_ffff_ffff_ffff_ffff_ffff_ffffu128);
    // A hex literal whose digits look like a float suffix stays an integer; a decimal one becomes a float.
    case!(int_hex_digits_f32, _: 0x1f32);
    case!(float_from_int_suffix_f32, _: 1f32);
    case!(float_from_int_suffix_f64, _: 1_f64);
    // Unsuffixed literals typed by context.
    case!(int_context_u128, u128: 0xffff_ffff_ffff_ffff_ffff_ffff_ffff_ffff);
    case!(int_context_u64, u64: 0o1777777777777777777777);
    case!(int_context_i8, i8: 0b0111_1111);

    // Floats.
    case!(float_simple, _: 1.0);
    case!(float_trailing_dot, _: 1.);
    case!(float_tenth, _: 0.1);
    case!(float_tenth_f32, _: 0.1f32);
    case!(float_exponent, _: 1e10);
    case!(float_exponent_upper, _: 1E10);
    case!(float_exponent_plus, _: 1e+10);
    case!(float_exponent_minus, _: 1e-10);
    case!(float_fraction_exponent, _: 1.5e3);
    case!(float_separators, _: 1_000.000_1);
    case!(float_exponent_separators, _: 1e1_0);
    case!(float_exponent_separator_first, _: 1e_3);
    case!(float_suffix_f32, _: 2.5f32);
    case!(float_suffix_separator, _: 2.5_f64);
    case!(float_exponent_f32, _: 1e3f32);
    case!(float_f64_max, _: 1.7976931348623157e308);
    case!(float_subnormal_min, _: 5e-324);
    case!(float_round_half_even, _: 9007199254740993.0);
    case!(float_f32_rounding, _: 16777217.0f32);
    case!(float_context_f32, f32: 0.1);

    // Characters: plain, non-ASCII, every escape.
    case!(char_plain, _: 'a');
    case!(char_latin, _: 'é');
    case!(char_astral, _: '😀');
    case!(char_double_quote, _: '"');
    case!(char_escape_newline, _: '\n');
    case!(char_escape_return, _: '\r');
    case!(char_escape_tab, _: '\t');
    case!(char_escape_backslash, _: '\\');
    case!(char_escape_nul, _: '\0');
    case!(char_escape_quote, _: '\'');
    case!(char_escape_double_quote, _: '\"');
    case!(char_escape_hex_zero, _: '\x00');
    case!(char_escape_hex_max, _: '\x7F');
    case!(char_escape_unicode_zero, _: '\u{0}');
    case!(char_escape_unicode, _: '\u{1F600}');
    case!(char_escape_unicode_max, _: '\u{10FFFF}');
    case!(char_escape_unicode_padded, _: '\u{0041}');
    case!(char_escape_unicode_separators, _: '\u{00_41}');

    // Bytes.
    case!(byte_plain, _: b'a');
    case!(byte_escape_newline, _: b'\n');
    case!(byte_escape_return, _: b'\r');
    case!(byte_escape_tab, _: b'\t');
    case!(byte_escape_backslash, _: b'\\');
    case!(byte_escape_nul, _: b'\0');
    case!(byte_escape_quote, _: b'\'');
    case!(byte_escape_double_quote, _: b'"');
    case!(byte_escape_hex_zero, _: b'\x00');
    case!(byte_escape_hex_high, _: b'\xff');
    case!(byte_escape_hex_upper, _: b'\xAB');

    // Strings.
    case!(str_empty, _: "");
    case!(str_plain, _: "abc");
    case!(str_non_ascii, _: "é😀");
    case!(str_escapes, _: "\n\r\t\\\0\'\"");
    case!(str_single_quote, _: "'");
    case!(str_escape_hex, _: "\x41\x7f");
    case!(str_escape_unicode, _: "\u{1F600}\u{0}\u{10_FFFF}");
    case!(str_continuation, _: "a\
        b");
    case!(str_continuation_keeps_escaped_space, _: "a\
        \x20b");
    case!(str_literal_newline, _: "a
b");
    case!(str_continuation_at_end, _: "a\
");

    // Raw strings: no escapes, hashes delimit.
    case!(raw_plain, _: r"a\nb");
    case!(raw_empty, _: r"");
    case!(raw_hash, _: r#"a"b"#);
    case!(raw_two_hashes, _: r##"a"#b"##);
    case!(raw_backslash_at_end, _: r"a\");
    case!(raw_newline, _: r"a
b");

    // Byte strings, raw byte strings.
    case!(bstr_plain, _: b"abc");
    case!(bstr_escapes, _: b"\xff\x00\n\\\"");
    case!(bstr_continuation, _: b"a\
        b");
    case!(bstr_raw, _: br"a\n");
    case!(bstr_raw_hash, _: br#"a"b"#);

    // C strings (bytes without the trailing NUL), raw C strings.
    case!(cstr_plain, _: c"abc");
    case!(cstr_escape_hex_high, _: c"\xff");
    case!(cstr_escape_unicode, _: c"\u{1F600}");
    case!(cstr_non_ascii, _: c"é");
    case!(cstr_continuation, _: c"a\
        b");
    case!(cstr_raw, _: cr"a\n");
    case!(cstr_raw_hash, _: cr#"a"b"#);

    // Booleans.
    case!(bool_true, _: true);
    case!(bool_false, _: false);

    // concat! and stringify! render literals as text.
    case!(concat_mixed, _: concat!("a", 'b', 1, 2.5, true, -3));
    case!(concat_float_exponent, _: concat!(1e3, 1.50, 2.0f32));
    case!(concat_int_suffix_and_base, _: concat!(1u8, 0x10, 0b11, 1_000));
    case!(concat_escapes, _: concat!("\x41", '\u{42}', r"\n"));
    case!(stringify_literals, _: stringify!(0x1_0u8, "a\n", 'c', r#"x"#, 1.5e3));
}

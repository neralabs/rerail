// Rerail-authored fixture (MIT). Literals at or past the edge of the language, compiled one case at a time
// (each `case!` alone, at its own line): rustc 1.99.0's error codes and lint names where it rejects them, the
// value where it accepts them (possibly with a warning).
#![allow(unused)]

pub fn run() {
    // Out of range for the type: deny-by-default `overflowing_literals`, or a hard error past u128.
    case!(int_u8_overflow, _: 256u8);
    case!(int_i8_overflow, _: 128i8);
    case!(int_i8_hex_wraps, _: 0xffi8);
    case!(int_i32_default_overflow, _: 2147483648);
    case!(int_too_large_for_u128, _: 340282366920938463463374607431768211456);
    case!(float_f64_overflow, _: 1e309);
    case!(float_f32_overflow, _: 1e39f32);
    case!(float_underflow_to_zero, _: 1e-400);
    case!(float_f32_subnormal, _: 1e-45f32);

    // Malformed numbers.
    case!(int_binary_bad_digit, _: 0b102);
    case!(int_octal_bad_digit, _: 0o8);
    case!(int_hex_empty, _: 0x);
    case!(int_hex_only_separator, _: 0x_);
    case!(int_bad_suffix, _: 1u7);
    case!(int_float_suffix_on_hex, _: 0x10f64);
    case!(float_empty_exponent, _: 1e);
    case!(float_hex, _: 0x1.0);
    case!(float_binary_exponent, _: 0b1e2);
    case!(float_int_suffix, _: 1.0u8);
    case!(float_f16_unstable, _: 1.0f16);

    // Malformed characters, bytes and escapes.
    case!(char_surrogate, _: '\u{D800}');
    case!(char_beyond_max, _: '\u{110000}');
    case!(char_empty_unicode, _: '\u{}');
    case!(char_long_unicode, _: '\u{0000041}');
    case!(char_hex_above_7f, _: '\x80');
    case!(char_two_chars, _: 'ab');
    case!(char_unknown_escape, _: '\q');
    case!(byte_non_ascii, _: b'é');
    case!(byte_unicode_escape, _: b'\u{41}');
    // Accepted with a warning: a line continuation skips blank lines too.
    case!(str_continuation_blank_lines, _: "a\

        b");
    case!(str_hex_above_7f, _: "\x80");
    case!(str_unknown_escape, _: "\q");
    case!(bstr_non_ascii, _: b"é");
    case!(cstr_nul_escape, _: c"a\0b");
    case!(cstr_hex_nul, _: c"\x00");
    case!(raw_unbalanced_hashes, _: r#"a"##);
    case!(raw_too_few_hashes, _: r##"a"#);
    case!(concat_byte_literal, _: concat!(b'a'));
}

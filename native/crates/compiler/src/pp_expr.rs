//! `#if`'s expressions (C17 §6.10.1, C++ [cpp.cond]): integer constant expressions over
//! `intmax_t` and `uintmax_t`, after macro expansion and `defined`. The operators' and primaries'
//! meanings, which the preprocessing words' `EXPR` applies (decision 0348): its program's rules are
//! the precedence climbing of the lean pass's evaluator (`rerail-c-pp`'s `expr`).
//!
//! Every operand is 64 bits, signed unless an operand made it unsigned (the usual arithmetic
//! conversions). `&&`, `||` and `?:` evaluate only the operands they take, and an operand not
//! evaluated raises no error (a division by zero there is none). An identifier left after expansion
//! is 0; `true` is 1 where it is a keyword (C++), and 0 where it is not.

use crate::pp_words::{IDENT, LITERAL, NUMBER, PUNCT};

/// A value: its bits and whether it is unsigned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Value {
    pub bits: u64,
    pub unsigned: bool,
}

impl Value {
    pub fn int(v: i64) -> Self {
        Value { bits: v as u64, unsigned: false }
    }

    fn truth(b: bool) -> Self {
        Value::int(b as i64)
    }

    pub fn is_true(self) -> bool {
        self.bits != 0
    }
}

/// A token as the evaluator reads it: its kind (`pp_words`' kinds), its operator (a
/// punctuator's spelling, or, where the dialect has them, an alternative token's: `and` is `&&`),
/// and its spelling.
#[derive(Clone, Copy)]
pub struct Item<'a> {
    pub kind: u8,
    pub op: Option<&'a [u8]>,
    pub text: &'a [u8],
}

/// C++'s alternative tokens and their punctuators (the last three no operator of an expression:
/// an error there, not an identifier's 0).
pub const ALT: &[(&[u8], &[u8])] = &[(b"and", b"&&"), (b"or", b"||"), (b"not", b"!"), (b"bitand", b"&"), (b"bitor", b"|"), (b"xor", b"^"), (b"compl", b"~"), (b"not_eq", b"!="), (b"and_eq", b"&="), (b"or_eq", b"|="), (b"xor_eq", b"^=")];

/// A token's item: a punctuator's operator is its spelling, an alternative token's (`alt`) its
/// operator's.
pub fn item<'a>(kind: u8, text: &'a [u8], alt: bool) -> Item<'a> {
    let op = match kind {
        PUNCT => Some(text),
        IDENT if alt => ALT.iter().find(|a| a.0 == text).map(|a| a.1),
        _ => None,
    };
    Item { kind: if op.is_some() { PUNCT } else { kind }, op, text }
}

/// A binary operator's precedence (higher binds tighter).
pub fn binary(t: &[u8]) -> Option<u8> {
    Some(match t {
        b"||" => 1,
        b"&&" => 2,
        b"|" => 3,
        b"^" => 4,
        b"&" => 5,
        b"==" | b"!=" => 6,
        b"<" | b">" | b"<=" | b">=" => 7,
        b"<<" | b">>" => 8,
        b"+" | b"-" => 9,
        b"*" | b"/" | b"%" => 10,
        _ => return None,
    })
}

/// A prefix operator (`+ - ~ !`) applied.
pub fn prefix(op: &[u8], v: Value) -> Value {
    match op {
        b"-" => Value { bits: v.bits.wrapping_neg(), unsigned: v.unsigned },
        b"~" => Value { bits: !v.bits, unsigned: v.unsigned },
        b"!" => Value::truth(!v.is_true()),
        _ => v,
    }
}

/// A primary's value: a number, a character constant, an identifier (0, or `true`'s 1 where
/// it is a keyword); `None` for any other token, or a number or character constant that is none.
pub fn primary(it: &Item, true_is_one: bool) -> Option<Value> {
    match it.kind {
        NUMBER => number(it.text),
        LITERAL if it.text.iter().find(|&&c| c == b'"' || c == b'\'') == Some(&b'\'') => char_value(it.text),
        IDENT => Some(Value::truth(true_is_one && it.text == b"true")),
        _ => None,
    }
}

/// `l op r`, the usual conversions applied; `None` for a division or remainder by zero evaluated
/// (`live`: one not evaluated is 0).
pub fn apply(op: &[u8], l: Value, r: Value, live: bool) -> Option<Value> {
    let u = l.unsigned || r.unsigned;
    let (a, b) = (l.bits, r.bits);
    let (sa, sb) = (a as i64, b as i64);
    let v = |bits: u64| Value { bits, unsigned: u };
    Some(match op {
        b"*" => v(a.wrapping_mul(b)),
        b"/" | b"%" => {
            if b == 0 {
                return if live { None } else { Some(v(0)) };
            }
            match (op, u) {
                (b"/", true) => v(a / b),
                (b"/", false) => v(sa.wrapping_div(sb) as u64),
                (_, true) => v(a % b),
                (_, false) => v(sa.wrapping_rem(sb) as u64),
            }
        }
        b"+" => v(a.wrapping_add(b)),
        b"-" => v(a.wrapping_sub(b)),
        // A shift's type is its left operand's.
        b"<<" => Value { bits: if b >= 64 { 0 } else { a << b }, unsigned: l.unsigned },
        b">>" => Value {
            bits: match (l.unsigned, b >= 64) {
                (true, true) => 0,
                (true, false) => a >> b,
                (false, true) => (sa >> 63) as u64,
                (false, false) => (sa >> b) as u64,
            },
            unsigned: l.unsigned,
        },
        b"<" => Value::truth(if u { a < b } else { sa < sb }),
        b">" => Value::truth(if u { a > b } else { sa > sb }),
        b"<=" => Value::truth(if u { a <= b } else { sa <= sb }),
        b">=" => Value::truth(if u { a >= b } else { sa >= sb }),
        b"==" => Value::truth(a == b),
        b"!=" => Value::truth(a != b),
        b"&" => v(a & b),
        b"^" => v(a ^ b),
        b"|" => v(a | b),
        _ => unreachable!("not a binary operator"),
    })
}

/// An integer constant's value (C's and C++'s forms: decimal, octal, `0x`, `0b`, digit
/// separators, suffixes `u`, `l`, `ll`, `z`, `wb`, `uwb`, in either case and order); `None` for a
/// floating constant or a value past 64 bits.
pub fn number(t: &[u8]) -> Option<Value> {
    let s: Vec<u8> = t.iter().copied().filter(|&c| c != b'\'').collect();
    let (radix, digits) = match s.as_slice() {
        [b'0', b'x' | b'X', r @ ..] => (16, r),
        [b'0', b'b' | b'B', r @ ..] => (2, r),
        [b'0', r @ ..] => (8, r),
        r => (10, r),
    };
    let n = digits.iter().take_while(|&&c| (c as char).is_digit(radix)).count();
    let (ds, suffix) = digits.split_at(n);
    if radix != 10 && radix != 8 && ds.is_empty() {
        return None;
    }
    let mut v: u64 = 0;
    for &c in ds {
        v = v.checked_mul(radix as u64)?.checked_add((c as char).to_digit(radix)? as u64)?;
    }
    let sfx: Vec<u8> = suffix.to_ascii_lowercase();
    let mut unsigned = match sfx.as_slice() {
        b"" | b"l" | b"ll" | b"z" | b"wb" => false,
        b"u" | b"ul" | b"lu" | b"ull" | b"llu" | b"uz" | b"zu" | b"uwb" | b"wbu" => true,
        _ => return None,
    };
    // A decimal past `intmax_t` is unsigned (the standard says it has no type; compilers take it).
    if radix == 10 && v > i64::MAX as u64 {
        unsigned = true;
    }
    Some(Value { bits: v, unsigned })
}

/// A character constant's value: its prefix (`L`, `u`, `U`, `u8`), its characters with escapes
/// read; several characters (`'ab'`) packed as GCC and clang pack them. A plain one's character
/// past ASCII is read as a signed `char`'s.
pub fn char_value(t: &[u8]) -> Option<Value> {
    let q = t.iter().position(|&c| c == b'\'')?;
    let prefix = &t[..q];
    let body = t.get(q + 1..t.len().checked_sub(1)?)?;
    let mut chars = Vec::new();
    let mut i = 0;
    while i < body.len() {
        let c = body[i];
        if c != b'\\' {
            // A UTF-8 character: its code point where the prefix is wide, else its bytes.
            if c >= 0x80 && !prefix.is_empty() {
                let s = std::str::from_utf8(&body[i..]).ok().or_else(|| {
                    let n = (1..=4).rev().find(|&n| std::str::from_utf8(&body[i..(i + n).min(body.len())]).is_ok())?;
                    std::str::from_utf8(&body[i..i + n]).ok()
                })?;
                let ch = s.chars().next()?;
                chars.push(ch as u64);
                i += ch.len_utf8();
            } else {
                chars.push(c as u64);
                i += 1;
            }
            continue;
        }
        let e = *body.get(i + 1)?;
        i += 2;
        let v = match e {
            b'n' => 10,
            b't' => 9,
            b'r' => 13,
            b'a' => 7,
            b'b' => 8,
            b'f' => 12,
            b'v' => 11,
            b'e' | b'E' => 27,
            b'0'..=b'7' => {
                let mut v = (e - b'0') as u64;
                let mut k = 0;
                while k < 2 && matches!(body.get(i), Some(b'0'..=b'7')) {
                    v = v * 8 + (body[i] - b'0') as u64;
                    i += 1;
                    k += 1;
                }
                v
            }
            b'x' => {
                let mut v = 0u64;
                let s = i;
                while let Some(d) = body.get(i).and_then(|c| (*c as char).to_digit(16)) {
                    v = v.wrapping_mul(16) + d as u64;
                    i += 1;
                }
                if i == s {
                    return None;
                }
                v
            }
            b'u' | b'U' => {
                let n = if e == b'u' { 4 } else { 8 };
                let h = std::str::from_utf8(body.get(i..i + n)?).ok()?;
                i += n;
                u64::from_str_radix(h, 16).ok()?
            }
            c => c as u64,
        };
        chars.push(v);
    }
    if chars.is_empty() {
        return None;
    }
    Some(match prefix {
        b"" => {
            if chars.len() == 1 {
                // `char` is signed on the targets read here.
                Value::int(chars[0] as u8 as i8 as i64)
            } else {
                // GCC's and clang's packing: each character a byte, the last lowest, as `int`.
                let v = chars.iter().fold(0u32, |a, &c| (a << 8) | (c as u8) as u32);
                Value::int(v as i32 as i64)
            }
        }
        b"u8" => Value::int(chars[0] as u8 as i64),
        b"u" => Value::int(chars[0] as u16 as i64),
        // `wchar_t` (`L`) is signed 32 bits on the targets read here; `char32_t` unsigned.
        b"L" => Value::int(chars[0] as u32 as i32 as i64),
        b"U" => Value::int(chars[0] as u32 as i64),
        _ => return None,
    })
}


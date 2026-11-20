//! `#if`'s expressions (C17 §6.10.1, C++ [cpp.cond]): integer constant expressions over `intmax_t`
//! and `uintmax_t`, after macro expansion and `defined`.
//!
//! Every operand is 64 bits, signed unless an operand made it unsigned (the usual arithmetic
//! conversions). `&&`, `||` and `?:` evaluate only the operands they take, and an operand not
//! evaluated raises no error (a division by zero there is none). An identifier left after expansion
//! is 0; `true` is 1 where it is a keyword (C++, C23), and 0 where it is not.

use rerail_c_lean::T;
use rerail_c_lean::pp::P;

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

/// What an expression gets wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExprError {
    /// No expression (`#if` alone).
    Empty,
    /// A token where an operand or operator was wanted, by its index.
    Unexpected(usize),
    /// Tokens left after the expression.
    Trailing(usize),
    /// A division or remainder by zero, evaluated.
    DivByZero(usize),
    /// A number that is no integer (a floating constant), or out of range.
    BadNumber(usize),
    /// A character constant that is not one.
    BadChar(usize),
}

/// A token as the evaluator reads it: its kind, punctuator and spelling.
#[derive(Clone, Copy)]
pub struct Item<'a> {
    pub kind: P,
    pub punct: T,
    pub text: &'a [u8],
}

/// The value of `items` (an `#if`'s expanded tokens, `defined` and the `__has_*` family answered).
pub fn eval(items: &[Item], true_is_one: bool) -> Result<Value, ExprError> {
    if items.is_empty() {
        return Err(ExprError::Empty);
    }
    let mut e = Eval { items, at: 0, true_is_one };
    let v = e.cond(true)?;
    if e.at < items.len() {
        return Err(ExprError::Trailing(e.at));
    }
    Ok(v)
}

struct Eval<'a, 'b> {
    items: &'b [Item<'a>],
    at: usize,
    true_is_one: bool,
}

/// A binary operator's precedence (higher binds tighter) and kind.
fn binary(t: T) -> Option<u8> {
    use T::*;
    Some(match t {
        PipePipe => 1,
        AmpAmp => 2,
        Pipe => 3,
        Caret => 4,
        Amp => 5,
        EqEq | Ne => 6,
        Lt | Gt | Le | Ge => 7,
        LtLt | GtGt => 8,
        Plus | Minus => 9,
        Star | Slash | Percent => 10,
        _ => return None,
    })
}

impl Eval<'_, '_> {
    fn peek(&self) -> Option<T> {
        let i = self.items.get(self.at)?;
        (i.kind == P::Punct).then_some(i.punct)
    }

    /// `cond (',' cond)*`: C forbids the comma outside an unevaluated operand, but compilers take
    /// it; its value is the right's.
    fn cond(&mut self, live: bool) -> Result<Value, ExprError> {
        let mut v = self.ternary(live)?;
        while self.peek() == Some(T::Comma) {
            self.at += 1;
            v = self.ternary(live)?;
        }
        Ok(v)
    }

    fn ternary(&mut self, live: bool) -> Result<Value, ExprError> {
        let c = self.binary(1, live)?;
        if self.peek() != Some(T::Question) {
            return Ok(c);
        }
        self.at += 1;
        let a = self.cond(live && c.is_true())?;
        if self.peek() != Some(T::Colon) {
            return Err(ExprError::Unexpected(self.at));
        }
        self.at += 1;
        let b = self.ternary(live && !c.is_true())?;
        // The usual conversions over both arms.
        let unsigned = a.unsigned || b.unsigned;
        let v = if c.is_true() { a } else { b };
        Ok(Value { bits: v.bits, unsigned })
    }

    /// Precedence climbing from `min`.
    fn binary(&mut self, min: u8, live: bool) -> Result<Value, ExprError> {
        let mut l = self.unary(live)?;
        while let Some(op) = self.peek() {
            let Some(p) = binary(op) else { break };
            if p < min {
                break;
            }
            let at = self.at;
            self.at += 1;
            l = match op {
                T::AmpAmp => {
                    let r = self.binary(p + 1, live && l.is_true())?;
                    Value::truth(l.is_true() && r.is_true())
                }
                T::PipePipe => {
                    let r = self.binary(p + 1, live && !l.is_true())?;
                    Value::truth(l.is_true() || r.is_true())
                }
                _ => {
                    let r = self.binary(p + 1, live)?;
                    apply(op, l, r, live, at)?
                }
            };
        }
        Ok(l)
    }

    fn unary(&mut self, live: bool) -> Result<Value, ExprError> {
        let at = self.at;
        let Some(it) = self.items.get(at) else { return Err(ExprError::Unexpected(at)) };
        self.at += 1;
        match it.kind {
            P::Punct => match it.punct {
                T::Plus => self.unary(live),
                T::Minus => self.unary(live).map(|v| Value { bits: v.bits.wrapping_neg(), unsigned: v.unsigned }),
                T::Tilde => self.unary(live).map(|v| Value { bits: !v.bits, unsigned: v.unsigned }),
                T::Bang => self.unary(live).map(|v| Value::truth(!v.is_true())),
                T::LParen => {
                    let v = self.cond(live)?;
                    if self.peek() != Some(T::RParen) {
                        return Err(ExprError::Unexpected(self.at));
                    }
                    self.at += 1;
                    Ok(v)
                }
                _ => Err(ExprError::Unexpected(at)),
            },
            P::Number => number(it.text).ok_or(ExprError::BadNumber(at)),
            P::Char => char_value(it.text).ok_or(ExprError::BadChar(at)),
            // Any other identifier (`false` too) is 0.
            P::Ident => Ok(Value::truth(self.true_is_one && it.text == b"true")),
            _ => Err(ExprError::Unexpected(at)),
        }
    }
}

/// `l op r`, the usual conversions applied.
fn apply(op: T, l: Value, r: Value, live: bool, at: usize) -> Result<Value, ExprError> {
    use T::*;
    let u = l.unsigned || r.unsigned;
    let (a, b) = (l.bits, r.bits);
    let (sa, sb) = (a as i64, b as i64);
    let v = |bits: u64| Value { bits, unsigned: u };
    Ok(match op {
        Star => v(a.wrapping_mul(b)),
        Slash | Percent => {
            if b == 0 {
                return if live { Err(ExprError::DivByZero(at)) } else { Ok(v(0)) };
            }
            match (op, u) {
                (Slash, true) => v(a / b),
                (Slash, false) => v(sa.wrapping_div(sb) as u64),
                (_, true) => v(a % b),
                (_, false) => v(sa.wrapping_rem(sb) as u64),
            }
        }
        Plus => v(a.wrapping_add(b)),
        Minus => v(a.wrapping_sub(b)),
        // A shift's type is its left operand's.
        LtLt => Value { bits: if b >= 64 { 0 } else { a << b }, unsigned: l.unsigned },
        GtGt => Value {
            bits: match (l.unsigned, b >= 64) {
                (true, true) => 0,
                (true, false) => a >> b,
                (false, true) => (sa >> 63) as u64,
                (false, false) => (sa >> b) as u64,
            },
            unsigned: l.unsigned,
        },
        Lt => Value::truth(if u { a < b } else { sa < sb }),
        Gt => Value::truth(if u { a > b } else { sa > sb }),
        Le => Value::truth(if u { a <= b } else { sa <= sb }),
        Ge => Value::truth(if u { a >= b } else { sa >= sb }),
        EqEq => Value::truth(a == b),
        Ne => Value::truth(a != b),
        Amp => v(a & b),
        Caret => v(a ^ b),
        Pipe => v(a | b),
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

#[cfg(test)]
mod tests {
    use super::*;
    use rerail_c_lean::pp::{Options, PpLexer};

    fn ev(src: &str) -> Result<Value, ExprError> {
        let mut lx = PpLexer::new(src.as_bytes(), Options::c(23, false));
        let mut items = Vec::new();
        loop {
            let t = lx.next_token();
            if t.kind == P::Eof {
                break;
            }
            items.push(Item { kind: t.kind, punct: t.punct, text: &src.as_bytes()[t.start as usize..t.end as usize] });
        }
        eval(&items, true)
    }

    fn v(src: &str) -> i64 {
        ev(src).unwrap().bits as i64
    }

    #[test]
    fn arithmetic_and_precedence() {
        assert_eq!(v("1 + 2 * 3"), 7);
        assert_eq!(v("(1 + 2) * 3"), 9);
        assert_eq!(v("10 - 2 - 3"), 5);
        assert_eq!(v("1 << 4 >> 2"), 4);
        assert_eq!(v("7 & 3 | 8 ^ 1"), 11);
        assert_eq!(v("1 ? 2 : 3"), 2);
        assert_eq!(v("0 ? 2 : 0 ? 3 : 4"), 4);
        assert_eq!(v("-5 / 2"), -2);
        assert_eq!(v("-5 % 2"), -1);
        assert_eq!(v("!0 + ~0"), 0);
        assert_eq!(v("1, 2"), 2);
        assert_eq!(v("0x10 + 010 + 0b11 + 1'0"), 16 + 8 + 3 + 10);
    }

    #[test]
    fn unsigned_conversions() {
        // -1 converted to unsigned is the largest value.
        assert_eq!(v("-1 > 0u"), 1);
        assert_eq!(v("-1 > 0"), 0);
        assert_eq!(v("(0u - 1) >> 63"), 1);
        assert_eq!(v("-1 >> 63"), -1);
        assert_eq!(v("18446744073709551615 == -1"), 1);
        // The arms of `?:` convert together.
        assert_eq!(v("(1 ? -1 : 0u) > 0"), 1);
        assert!(ev("0xFFFFFFFFFFFFFFFFull").unwrap().unsigned);
        assert_eq!(ev("99999999999999999999"), Err(ExprError::BadNumber(0)));
    }

    #[test]
    fn short_circuit_raises_no_error() {
        assert_eq!(v("0 && 1 / 0"), 0);
        assert_eq!(v("1 || 1 % 0"), 1);
        assert_eq!(v("0 ? 1 / 0 : 5"), 5);
        assert_eq!(ev("1 / 0"), Err(ExprError::DivByZero(1)));
        assert_eq!(ev("1 && 1 / 0"), Err(ExprError::DivByZero(3)));
    }

    #[test]
    fn identifiers_and_characters() {
        assert_eq!(v("UNDEFINED + 1"), 1);
        assert_eq!(v("true + false"), 1);
        let t = [Item { kind: P::Ident, punct: T::Eof, text: b"true" }];
        assert_eq!(eval(&t, false), Ok(Value::int(0)));
        assert_eq!(v("'a'"), 97);
        assert_eq!(v("'\\n' + '\\x41' + '\\101'"), 10 + 65 + 65);
        assert_eq!(v("'\\377'"), -1);
        assert_eq!(v("'ab'"), 0x6162);
        assert_eq!(v("L'\\xffffffff'"), -1);
        assert_eq!(v("U'\\xffffffff'"), 0xffff_ffff);
        assert_eq!(v("u'é'"), 0xe9);
    }

    #[test]
    fn errors() {
        assert_eq!(ev(""), Err(ExprError::Empty));
        assert_eq!(ev("1 +"), Err(ExprError::Unexpected(2)));
        assert_eq!(ev("1 2"), Err(ExprError::Trailing(1)));
        assert_eq!(ev("(1"), Err(ExprError::Unexpected(2)));
        assert_eq!(ev("1.0"), Err(ExprError::BadNumber(0)));
    }
}

//! Tier 1's passes as UKL (decision 0726): the lines tier 1's Rust is made of, printed as UKL C, a thread a chunk.
//!
//! Tier 1's passes are written in a narrow form: words of 64 bits a block, the 64-byte blocks' operations of
//! `rerail_simd8::portable` (`s::`, which `<ukl/blx64.h>` defines byte by byte), loops over the blocks and the lanes,
//! and a few shapes of their own (a lane met, a lane's end). The printer reads each function of the passes line by line
//! and prints each line as UKL, its expressions parsed and printed with every operation in parentheses (Rust's `&` binds
//! tighter than `==`, C's does not). A line it does not know is an error, so the form stays closed: a change to tier
//! 1's Rust that the printer does not take fails `json_rust_is_current`, never a kernel.
//!
//! Each name of the tier's (its functions, tables, constants, `Carries`) takes the prefix the caller gives, so several
//! tiers share a program. The kernels: `{p}ends` (pass 1, a chunk's end from each lane's entry) and `{p}from` (pass 2,
//! counting or writing, each type's records in one list from the offsets the caller gives).

use std::collections::HashMap;
use std::fmt::Write;

/// A value's type, as the printer keeps it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Ty {
    U64,
    U32,
    Bool,
    B64,
    Carries,
    /// A pointer, its C type.
    Ptr(String),
    /// The types' lists, one buffer (`types[t]` is it).
    Lists,
    /// An array of these.
    Arr(Box<Ty>),
    /// What tier 2's helpers write through (`Out`), its C struct.
    Out(String),
    /// A `Result<usize, u32>` (`{p}R`).
    Res,
    /// A pair of `u32`s (`marks`').
    Pair,
    /// A struct by value, its C name.
    Named(String),
}

impl Ty {
    fn c(&self, p: &str) -> String {
        match self {
            Ty::U64 => "uint64_t".into(),
            Ty::U32 => "uint32_t".into(),
            Ty::Bool => "bool".into(),
            Ty::B64 => "UklB64".into(),
            Ty::Carries => format!("{p}Carries"),
            Ty::Ptr(c) => c.clone(),
            Ty::Lists => "UKL_GLOBAL uint32_t*".into(),
            Ty::Arr(t) => t.c(p),
            Ty::Out(c) => format!("UKL_PRIVATE {c}*"),
            Ty::Res => format!("{p}R"),
            Ty::Pair => "uint32_t".into(),
            Ty::Named(c) => c.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Tok {
    Id(String),
    Num(String),
    Str,
    /// A byte string's bytes (`b"…"`).
    Bytes(Vec<u8>),
    P(&'static str),
}

const PUNCT: &[&str] = &[
    "<<=", ">>=", "..=", "..", "::", "=>", "?", "<<", ">>", "<=", ">=", "==", "!=", "&&", "||", "+=", "-=", "|=", "&=", "^=", "*=", "(", ")", "[", "]", "{", "}", ",", ".", ";", ":", "&", "|",
    "^", "!", "+", "-", "*", "/", "%", "<", ">", "=",
];

pub(crate) fn lex(s: &str) -> Result<Vec<Tok>, String> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    'next: while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c == b'b' && b.get(i + 1) == Some(&b'"') {
            let j = i + 2 + b[i + 2..].iter().position(|c| *c == b'"').ok_or("an open byte string")?;
            out.push(Tok::Bytes(b[i + 2..j].to_vec()));
            i = j + 1;
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let j = i + b[i..].iter().take_while(|c| c.is_ascii_alphanumeric() || **c == b'_').count();
            out.push(Tok::Id(s[i..j].into()));
            i = j;
            continue;
        }
        if c.is_ascii_digit() {
            let j = i + b[i..].iter().take_while(|c| c.is_ascii_alphanumeric() || **c == b'_').count();
            out.push(Tok::Num(s[i..j].into()));
            i = j;
            continue;
        }
        if c == b'"' {
            let j = i + 1 + b[i + 1..].iter().position(|c| *c == b'"').ok_or("an open string")?;
            out.push(Tok::Str);
            i = j + 1;
            continue;
        }
        for p in PUNCT {
            if s[i..].starts_with(p) {
                out.push(Tok::P(p));
                i += p.len();
                continue 'next;
            }
        }
        return Err(format!("a character the printer does not take: {:?}", c as char));
    }
    Ok(out)
}

#[derive(Clone, Debug)]
pub(crate) enum E {
    Num(String),
    Id(String),
    Path(String, String),
    Un(&'static str, Box<E>),
    Bin(&'static str, Box<E>, Box<E>),
    Cast(Box<E>, String),
    Call(Box<E>, Vec<E>),
    Method(Box<E>, String, Vec<E>),
    Field(Box<E>, String),
    Index(Box<E>, Box<E>),
    Array(Vec<E>),
    If(Box<E>, Box<E>, Box<E>),
    /// `a..b` or `a..=b` (the last `true`).
    Range(Box<E>, Box<E>, bool),
    /// `matches!(x, p | q | a..=b)`: its patterns, each a low and a high.
    Matches(Box<E>, Vec<(E, E)>),
    /// `x?`.
    Try(Box<E>),
    Tuple(Vec<E>),
    /// `x[a..b]`, `x[a..]`.
    Slice(Box<E>, Box<E>, Option<Box<E>>),
    Bytes(Vec<u8>),
}

pub(crate) struct Parser {
    pub(crate) t: Vec<Tok>,
    pub(crate) i: usize,
}

fn binding(op: &str) -> Option<u8> {
    Some(match op {
        "||" => 1,
        "&&" => 2,
        "==" | "!=" | "<" | ">" | "<=" | ">=" => 3,
        "|" => 4,
        "^" => 5,
        "&" => 6,
        "<<" | ">>" => 7,
        "+" | "-" => 8,
        "*" | "/" | "%" => 9,
        _ => return None,
    })
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn is(&self, p: &str) -> bool {
        matches!(self.peek(), Some(Tok::P(q)) if *q == p)
    }
    fn eat(&mut self, p: &str) -> Result<(), String> {
        if self.is(p) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected `{p}` at {:?}", self.peek()))
        }
    }
    fn id(&mut self) -> Result<String, String> {
        match self.peek().cloned() {
            Some(Tok::Id(s)) => {
                self.i += 1;
                Ok(s)
            }
            t => Err(format!("expected a name at {t:?}")),
        }
    }
    pub(crate) fn expr(&mut self, min: u8) -> Result<E, String> {
        let mut a = self.unary()?;
        loop {
            if matches!(self.peek(), Some(Tok::Id(k)) if k == "as") {
                self.i += 1;
                let t = self.id()?;
                a = E::Cast(Box::new(a), t);
                continue;
            }
            let Some(Tok::P(op)) = self.peek().cloned() else { break };
            let Some(bp) = binding(op) else { break };
            if bp < min {
                break;
            }
            self.i += 1;
            let b = self.expr(bp + 1)?;
            a = E::Bin(op, Box::new(a), Box::new(b));
        }
        Ok(a)
    }
    fn unary(&mut self) -> Result<E, String> {
        for op in ["!", "-", "*", "&"] {
            if self.is(op) {
                self.i += 1;
                if op == "&" && matches!(self.peek(), Some(Tok::Id(k)) if k == "mut") {
                    self.i += 1;
                }
                let e = self.unary()?;
                return Ok(E::Un(op, Box::new(e)));
            }
        }
        self.postfix()
    }
    fn args(&mut self) -> Result<Vec<E>, String> {
        let mut v = Vec::new();
        self.eat("(")?;
        while !self.is(")") {
            if matches!(self.peek(), Some(Tok::Str)) {
                self.i += 1;
            } else {
                v.push(self.expr(0)?);
            }
            if self.is(",") {
                self.i += 1;
            }
        }
        self.eat(")")?;
        Ok(v)
    }
    fn postfix(&mut self) -> Result<E, String> {
        let mut a = self.primary()?;
        loop {
            if self.is(".") {
                self.i += 1;
                let m = match self.peek().cloned() {
                    Some(Tok::Id(s)) => s,
                    Some(Tok::Num(s)) => s,
                    t => return Err(format!("expected a field at {t:?}")),
                };
                self.i += 1;
                if self.is("(") {
                    let args = self.args()?;
                    a = E::Method(Box::new(a), m, args);
                } else {
                    a = E::Field(Box::new(a), m);
                }
            } else if self.is("[") {
                self.i += 1;
                if self.is("..") {
                    self.i += 1;
                    self.eat("]")?;
                    a = E::Slice(Box::new(a), Box::new(E::Num("0".into())), None);
                    continue;
                }
                let k = self.expr(0)?;
                if self.is("..") {
                    self.i += 1;
                    let end = if self.is("]") { None } else { Some(Box::new(self.expr(0)?)) };
                    self.eat("]")?;
                    a = E::Slice(Box::new(a), Box::new(k), end);
                    continue;
                }
                self.eat("]")?;
                a = E::Index(Box::new(a), Box::new(k));
            } else if self.is("?") {
                self.i += 1;
                a = E::Try(Box::new(a));
            } else if self.is("(") {
                let args = self.args()?;
                a = E::Call(Box::new(a), args);
            } else {
                break;
            }
        }
        Ok(a)
    }
    fn primary(&mut self) -> Result<E, String> {
        match self.peek().cloned() {
            Some(Tok::Num(n)) => {
                self.i += 1;
                Ok(E::Num(n))
            }
            Some(Tok::Id(k)) if k == "unsafe" && matches!(self.t.get(self.i + 1), Some(Tok::P("{"))) => {
                self.i += 2;
                let e = self.expr(0)?;
                self.eat("}")?;
                Ok(e)
            }
            Some(Tok::Id(k)) if k == "if" => {
                self.i += 1;
                let c = self.expr(0)?;
                self.eat("{")?;
                let a = self.expr(0)?;
                self.eat("}")?;
                if !matches!(self.peek(), Some(Tok::Id(k)) if k == "else") {
                    return Err("an `if` value without `else`".into());
                }
                self.i += 1;
                self.eat("{")?;
                let b = self.expr(0)?;
                self.eat("}")?;
                Ok(E::If(Box::new(c), Box::new(a), Box::new(b)))
            }
            Some(Tok::Id(k)) if k == "matches" && matches!(self.t.get(self.i + 1), Some(Tok::P("!"))) => {
                self.i += 2;
                self.eat("(")?;
                let x = self.expr(0)?;
                self.eat(",")?;
                let mut pats = Vec::new();
                loop {
                    let lo = self.expr(5)?;
                    let hi = if self.is("..=") {
                        self.i += 1;
                        self.expr(5)?
                    } else {
                        lo.clone()
                    };
                    pats.push((lo, hi));
                    if self.is("|") {
                        self.i += 1;
                        continue;
                    }
                    break;
                }
                self.eat(")")?;
                Ok(E::Matches(Box::new(x), pats))
            }
            Some(Tok::Bytes(v)) => {
                self.i += 1;
                Ok(E::Bytes(v))
            }
            Some(Tok::Id(k)) => {
                self.i += 1;
                if self.is("::") {
                    self.i += 1;
                    let f = self.id()?;
                    return Ok(E::Path(k, f));
                }
                Ok(E::Id(k))
            }
            Some(Tok::P("(")) => {
                self.i += 1;
                let e = self.expr(0)?;
                if self.is("..=") || self.is("..") {
                    let incl = self.is("..=");
                    self.i += 1;
                    let b = self.expr(0)?;
                    self.eat(")")?;
                    return Ok(E::Range(Box::new(e), Box::new(b), incl));
                }
                if self.is(",") {
                    let mut v = vec![e];
                    while self.is(",") {
                        self.i += 1;
                        v.push(self.expr(0)?);
                    }
                    self.eat(")")?;
                    return Ok(E::Tuple(v));
                }
                self.eat(")")?;
                Ok(e)
            }
            Some(Tok::P("[")) => {
                self.i += 1;
                let mut v = Vec::new();
                while !self.is("]") {
                    v.push(self.expr(0)?);
                    if self.is(",") {
                        self.i += 1;
                    }
                }
                self.eat("]")?;
                Ok(E::Array(v))
            }
            t => Err(format!("an expression the printer does not take, at {t:?}")),
        }
    }
}

/// `s` parsed whole as one expression.
pub(crate) fn parse(s: &str) -> Result<E, String> {
    let mut p = Parser { t: lex(s)?, i: 0 };
    let e = p.expr(0)?;
    if p.i != p.t.len() {
        return Err(format!("`{s}`: more after its expression ({:?})", p.peek()));
    }
    Ok(e)
}

/// A function's names, types and constants, as it is printed.
pub(crate) struct Fx<'a> {
    pub(crate) p: &'a str,
    /// The tier's names: functions, tables, constants, with their prefix.
    pub(crate) names: &'a HashMap<String, String>,
    /// The tier's functions' return types.
    pub(crate) returns: &'a HashMap<String, Ty>,
    /// The scopes' names and types, innermost last.
    pub(crate) scopes: Vec<Vec<(String, Ty)>>,
    /// The function's own constants, their values and types.
    pub(crate) consts: HashMap<String, (String, Ty)>,
    /// The statements a line's expression needs before it (a `?`'s test, an array argument's room).
    pub(crate) pre: std::cell::RefCell<String>,
    pub(crate) tmp: std::cell::Cell<u32>,
    /// Whether the walk's `Out` keeps the token pass's answers (`known`), and its C struct.
    pub(crate) known: bool,
    pub(crate) out: String,
}

impl Fx<'_> {
    /// The statements hoisted for the line printed last, taken.
    pub(crate) fn take_pre(&self) -> String {
        std::mem::take(&mut *self.pre.borrow_mut())
    }
    fn fresh(&self) -> String {
        let k = self.tmp.get();
        self.tmp.set(k + 1);
        format!("t{k}_")
    }
    pub(crate) fn var(&self, n: &str) -> Option<&Ty> {
        self.scopes.iter().rev().flat_map(|s| s.iter().rev()).find(|(k, _)| k == n).map(|(_, t)| t)
    }
    pub(crate) fn in_this_scope(&self, n: &str) -> bool {
        self.scopes.last().is_some_and(|s| s.iter().any(|(k, _)| k == n))
    }
    pub(crate) fn declare(&mut self, n: &str, t: Ty) {
        self.scopes.last_mut().expect("a scope").push((n.into(), t));
    }
    pub(crate) fn name(&self, n: &str) -> String {
        self.names.get(n).cloned().unwrap_or_else(|| n.into())
    }

    pub(crate) fn ty(&self, e: &E) -> Ty {
        match e {
            E::Num(n) => {
                let d = n.replace('_', "");
                let d = ["u64", "usize", "u32", "u8"].iter().fold(d.as_str(), |d, s| d.strip_suffix(s).unwrap_or(d)).to_string();
                let wide = d.strip_prefix("0x").map_or_else(|| d.parse::<u64>().ok(), |h| u64::from_str_radix(h, 16).ok()).is_some_and(|v| v > u64::from(u32::MAX));
                if n.ends_with("u64") || wide { Ty::U64 } else { Ty::U32 }
            }
            E::Id(n) => match self.consts.get(n) {
                Some((_, t)) => t.clone(),
                None => self.var(n).cloned().unwrap_or(Ty::U32),
            },
            E::Path(_, f) => match f.as_str() {
                "eq_mask" | "to_bitmask" | "nonzero" => Ty::U64,
                _ => Ty::B64,
            },
            E::Un("!", a) => self.ty(a),
            E::Un("*", a) => match self.ty(a) {
                Ty::Ptr(c) if c.contains("uint64_t") => Ty::U64,
                _ => Ty::U32,
            },
            E::Un(_, a) => self.ty(a),
            E::Bin(op, a, b) => match *op {
                "==" | "!=" | "<" | ">" | "<=" | ">=" | "&&" | "||" => Ty::Bool,
                "<<" | ">>" => self.ty(a),
                _ => {
                    let (x, y) = (self.ty(a), self.ty(b));
                    if x == Ty::U64 || y == Ty::U64 {
                        Ty::U64
                    } else if x == Ty::Bool && y == Ty::Bool {
                        Ty::Bool
                    } else {
                        Ty::U32
                    }
                }
            },
            E::Cast(_, t) => cast_ty(t),
            E::Call(f, _) => match &**f {
                E::Id(n) => self.returns.get(n).cloned().unwrap_or(Ty::U32),
                E::Path(m, _) if m != "s" => cast_ty(m),
                E::Path(..) => self.ty(f),
                _ => Ty::U32,
            },
            E::Method(a, m, _) => match m.as_str() {
                "count_ones" | "trailing_zeros" => Ty::U32,
                "is_some" | "is_none" | "starts_with" | "contains" => Ty::Bool,
                "get_unchecked" => Ty::U32,
                _ => self.ty(a),
            },
            E::Range(..) | E::Bytes(_) => Ty::U32,
            E::Matches(..) => Ty::Bool,
            E::Try(_) => Ty::U32,
            E::Tuple(_) => Ty::Pair,
            E::Slice(a, ..) => match self.ty(a) {
                Ty::Ptr(c) => Ty::Ptr(c),
                _ => Ty::Ptr("UKL_GLOBAL const uint8_t*".into()),
            },
            E::Field(a, f) => match self.ty(a) {
                Ty::Named(c) if c.ends_with("Lx") && f == "starts" => Ty::Ptr("UKL_GLOBAL const uint32_t*".into()),
                Ty::Named(_) => Ty::U32,
                _ => Ty::U64,
            },
            E::Index(a, _) => match self.ty(a) {
                Ty::Arr(t) => *t,
                Ty::Ptr(c) if c.contains("uint64_t") => Ty::U64,
                _ => Ty::U32,
            },
            E::Array(_) => Ty::U32,
            E::If(_, a, _) => self.ty(a),
        }
    }

    pub(crate) fn c(&self, e: &E) -> Result<String, String> {
        Ok(match e {
            E::Num(n) => {
                let d: String = n.replace('_', "");
                let digits = ["u64", "usize", "u32", "u8"].iter().fold(d.as_str(), |d, s| d.strip_suffix(s).unwrap_or(d)).to_string();
                if self.ty(e) == Ty::U64 { format!("{digits}ull") } else { format!("{digits}u") }
            }
            E::Id(n) if n == "None" => "UKL_NONE".into(),
            E::Id(n) => match self.consts.get(n) {
                Some((v, _)) => v.clone(),
                // A local or a parameter is itself, whatever a function of its name is printed as.
                None if self.var(n).is_some() => n.clone(),
                None => self.name(n),
            },
            E::Call(f, args) if matches!(&**f, E::Id(n) if n == "Ok" || n == "Err" || n == "Some") => {
                let E::Id(n) = &**f else { unreachable!("a name") };
                let x = self.c(args.first().ok_or("a value of nothing")?)?;
                match n.as_str() {
                    "Some" => x,
                    "Ok" => format!("{}ok({x})", self.p),
                    _ => format!("{}err({x})", self.p),
                }
            }
            // `u32::from(x)` and the like: a cast.
            E::Call(f, args) if matches!(&**f, E::Path(m, n) if m != "s" && n == "from") => {
                let E::Path(m, _) = &**f else { unreachable!("a path") };
                let x = args.first().ok_or("`from` of nothing")?;
                format!("(({})({}))", cast_ty(m).c(self.p), self.c(x)?)
            }
            E::Path(m, f) => {
                if m != "s" {
                    return Err(format!("a path the printer does not take: {m}::{f}"));
                }
                format!("ukl_b64_{f}")
            }
            E::Un("!", a) => {
                if self.ty(a) == Ty::Bool {
                    format!("(!{})", self.c(a)?)
                } else {
                    format!("(~{})", self.c(a)?)
                }
            }
            // A table's address is the table; a local's is its address.
            E::Un("&", a) => match &**a {
                E::Id(n) if self.names.contains_key(n) => self.name(n),
                E::Slice(..) => self.c(a)?,
                _ => format!("(&{})", self.c(a)?),
            },
            E::Un(op, a) => format!("({op}{})", self.c(a)?),
            // `x[a..b] == *b"…"`: each byte.
            E::Bin("==", a, b) if matches!((&**a, &**b), (E::Slice(_, _, Some(_)), E::Un("*", y)) if matches!(&**y, E::Bytes(_))) => {
                let (E::Slice(x, k, _), E::Un(_, y)) = (&**a, &**b) else { unreachable!("a slice") };
                let E::Bytes(bytes) = &**y else { unreachable!("bytes") };
                let (x, k) = (self.c(x)?, self.c(k)?);
                let v: Vec<String> = bytes.iter().enumerate().map(|(j, b)| format!("{x}[{k} + {j}u] == {b:#04x}u")).collect();
                format!("({})", v.join(" && "))
            }
            E::Bin(op, a, b) => format!("({} {op} {})", self.c(a)?, self.c(b)?),
            E::Cast(a, t) => format!("(({})({}))", cast_ty(t).c(self.p), self.c(a)?),
            // A word of the line index ORed (`or_word`): the device's atomic OR.
            E::Call(f, args) if matches!(&**f, E::Id(n) if n == "or_word") && args.len() == 3 => {
                format!("ukl_atomic_or(&({})[{}], {})", self.c(&args[0])?, self.c(&args[1])?, self.c(&args[2])?)
            }
            E::Call(f, args) => {
                // `s::load` of a table: the table's.
                let f = match &**f {
                    E::Path(m, n) if m == "s" && n == "load" && matches!(args.first(), Some(E::Un("&", _))) => "ukl_b64_load_t".to_string(),
                    _ => self.c(f)?,
                };
                // An array argument: its room, before the line.
                let mut v = Vec::new();
                for a in args {
                    if let E::Array(xs) = a {
                        let t = self.fresh();
                        let xs = xs.iter().map(|x| self.c(x)).collect::<Result<Vec<_>, _>>()?;
                        let _ = writeln!(self.pre.borrow_mut(), "uint32_t {t}[{}] = {{{}}};", xs.len(), xs.join(", "));
                        v.push(t);
                    } else {
                        v.push(self.c(a)?);
                    }
                }
                format!("{f}({})", v.join(", "))
            }
            // `x?`: its result's test before the line, its value in it.
            E::Try(x) => {
                let t = self.fresh();
                let x = self.c(x)?;
                let _ = writeln!(self.pre.borrow_mut(), "{p}R {t} = {x};\nif (!{t}.ok)\nreturn {t};", p = self.p);
                format!("{t}.v")
            }
            E::Matches(x, pats) => {
                let x = self.c(x)?;
                let mut v = Vec::new();
                for (lo, hi) in pats {
                    let (l, h) = (self.c(lo)?, self.c(hi)?);
                    v.push(if l == h { format!("{x} == {l}") } else { format!("({x} >= {l} && {x} <= {h})") });
                }
                format!("({})", v.join(" || "))
            }
            E::Slice(a, k, None) if matches!(&**k, E::Num(z) if z == "0") => self.c(a)?,
            E::Slice(a, k, None) => format!("({} + {})", self.c(a)?, self.c(k)?),
            E::Range(..) | E::Tuple(_) | E::Slice(..) | E::Bytes(_) => return Err("a range, tuple, slice or byte string the printer does not take here".into()),
            E::Method(a, m, args) if matches!(self.ty(a), Ty::Out(_)) => {
                let Ty::Out(c) = self.ty(a) else { unreachable!("an out") };
                let mut v = vec![self.c(a)?];
                for x in args {
                    v.push(self.c(x)?);
                }
                format!("{c}_{m}({})", v.join(", "))
            }
            E::Method(a, m, args) => {
                let x = if matches!(&**a, E::Range(..)) { String::new() } else { self.c(a)? };
                let arg = |k: usize| args.get(k).ok_or_else(|| format!("`{m}` without its argument")).and_then(|e| self.c(e));
                let wide = self.ty(a) == Ty::U64;
                match m.as_str() {
                    "wrapping_sub" => format!("({x} - {})", arg(0)?),
                    "wrapping_add" => format!("({x} + {})", arg(0)?),
                    "wrapping_mul" => format!("({x} * {})", arg(0)?),
                    "trailing_zeros" => format!("{}({x})", if wide { "ukl_ctz64" } else { "ukl_ctz32" }),
                    "count_ones" => format!("{}({x})", if wide { "ukl_popcount64" } else { "ukl_popcount32" }),
                    "div_ceil" => {
                        let k = arg(0)?;
                        format!("(({x} + {k} - 1u) / {k})")
                    }
                    "min" => {
                        let y = arg(0)?;
                        format!("({x} < {y} ? {x} : {y})")
                    }
                    "max" => {
                        let y = arg(0)?;
                        format!("({x} > {y} ? {x} : {y})")
                    }
                    "saturating_sub" => {
                        let y = arg(0)?;
                        format!("({x} > {y} ? {x} - {y} : 0u)")
                    }
                    "is_some" => format!("({x} != UKL_NONE)"),
                    "is_none" => format!("({x} == UKL_NONE)"),
                    "add" => format!("({x} + {})", arg(0)?),
                    "get_unchecked" => format!("(&{x}[{}])", arg(0)?),
                    // `(a..=b).contains(&x)`.
                    "contains" => {
                        let E::Range(lo, hi, incl) = &**a else { return Err("`contains` of no range".into()) };
                        let y = match args.first() {
                            Some(E::Un("&", y)) => self.c(y)?,
                            Some(y) => self.c(y)?,
                            None => return Err("`contains` of nothing".into()),
                        };
                        format!("({y} >= {} && {y} {} {})", self.c(lo)?, if *incl { "<=" } else { "<" }, self.c(hi)?)
                    }
                    // `src.starts_with(&[a, b, c])`: its first bytes.
                    "starts_with" => {
                        let Some(E::Un("&", arr)) = args.first() else { return Err("`starts_with` of no bytes".into()) };
                        let E::Array(bytes) = &**arr else { return Err("`starts_with` of no bytes".into()) };
                        let mut v = Vec::new();
                        for (k, b) in bytes.iter().enumerate() {
                            v.push(format!("{x}[{k}] == {}", self.c(b)?));
                        }
                        format!("({})", v.join(" && "))
                    }
                    _ => return Err(format!("a method the printer does not take: `{m}`")),
                }
            }
            E::Field(a, f) => format!("{}.{f}", self.c(a)?),
            E::Index(a, k) => {
                if self.ty(a) == Ty::Lists {
                    self.c(a)?
                } else {
                    format!("{}[{}]", self.c(a)?, self.c(k)?)
                }
            }
            E::Array(_) => return Err("an array value the printer does not take".into()),
            E::If(c, a, b) => format!("({} ? {} : {})", self.c(c)?, self.c(a)?, self.c(b)?),
        })
    }
}

pub(crate) fn cast_ty(t: &str) -> Ty {
    match t {
        "u64" => Ty::U64,
        "bool" => Ty::Bool,
        _ => Ty::U32,
    }
}

/// A parameter's type, from its Rust type.
fn param_ty(t: &str) -> Result<Ty, String> {
    param_ty_of(t, "")
}

/// A parameter's type, `&mut D` the walk's `Out`, `out` (its C struct).
fn param_ty_of(t: &str, out: &str) -> Result<Ty, String> {
    if t == "&mut D" && !out.is_empty() {
        return Ok(Ty::Out(out.into()));
    }
    // The levels' places: the chunk's room.
    if t.starts_with("&mut [u8; ") {
        return Ok(Ty::Ptr("UKL_GLOBAL uint8_t*".into()));
    }
    Ok(match t {
        "u64" => Ty::U64,
        "u32" | "usize" => Ty::U32,
        "bool" => Ty::Bool,
        "Carries" => Ty::Carries,
        "&[u8]" => Ty::Ptr("UKL_GLOBAL const uint8_t*".into()),
        "&[u32]" => Ty::Ptr("UKL_GLOBAL const uint32_t*".into()),
        "u8" => Ty::U32,
        _ if t.starts_with("[u32; ") => Ty::Ptr("UKL_PRIVATE const uint32_t*".into()),
        "*mut u32" => Ty::Ptr("UKL_GLOBAL uint32_t*".into()),
        // A phase's record words (`<ukl/phases_run.h>`'s trees): what other groups of the launch read, coherent.
        "&[Rec]" => Ty::Ptr("UKL_COHERENT const uint32_t*".into()),
        "*mut Rec" => Ty::Ptr("UKL_COHERENT uint32_t*".into()),
        "&mut usize" => Ty::Ptr("UKL_PRIVATE uint32_t*".into()),
        _ if t.starts_with("&mut [usize;") => Ty::Ptr("UKL_PRIVATE uint32_t*".into()),
        _ if t.starts_with("[*mut u32;") => Ty::Lists,
        _ => return Err(format!("a parameter type the printer does not take: `{t}`")),
    })
}

/// What a frame closes to.
enum Frame {
    Plain,
    /// A block whose value is the named variable's.
    Value(String),
    /// A struct's fields, each `{target}.field = …`, then the line `after`.
    Fields(String, String),
    /// A `match`, as tests in turn of its value's variable: whether an arm is printed yet.
    Match(String, bool),
    /// An arm of a `match`: its block (a loop's, closed with it, where `true`).
    Arm(bool),
    /// A loop, its label.
    Loop(Option<String>),
    /// A `match` of a `Result` as a value: its target and the result's variable.
    ResMatch(String, String),
}

/// Function `name`'s body (`lines`, after its signature line, to its closing brace), printed.
#[allow(clippy::too_many_lines)]
fn body(fx: &mut Fx<'_>, lines: &[&str], ret: &str, out: &mut String) -> Result<(), String> {
    let mut frames: Vec<Frame> = vec![Frame::Plain];
    let lanes = fx.name("LANES");
    // Where the line before was printed, for the statements its expressions hoisted; the braces of a block skipped.
    let mut mark = 0usize;
    let mut skip = 0i32;
    let mut matches = 0u32;
    // The labels of the loops met, a jump's word one past its place.
    let mut labels: Vec<String> = Vec::new();
    for raw in lines {
        let pre = fx.take_pre();
        if !pre.is_empty() {
            let line = &out[mark..];
            if line.starts_with('}') || line.starts_with("while") {
                return Err(format!("as UKL: a test hoisted before `{}`", line.lines().next().unwrap_or("")));
            }
            out.insert_str(mark, &pre);
        }
        mark = out.len();
        // A walk's stop as its constructors' calls.
        let stopped = raw
            .trim()
            .replace("Stop::At(Entry { i, e, depth, st })", "stop_at(i, e, depth, st)")
            .replace("Stop::Read(Entry { i, e, depth, st })", "stop_read(i, e, depth, st)")
            .replace("Stop::Done(Ok(()))", "stop_ok()")
            .replace("Stop::Done(Err(", "stop_err((");
        let l = stopped.as_str();
        let why = |e: String| format!("as UKL: `{l}`: {e}");
        if skip > 0 {
            skip += l.matches('{').count() as i32 - l.matches('}').count() as i32;
            continue;
        }
        if l.is_empty() || l.starts_with("debug_assert") || l.starts_with("let _ = ") {
            continue;
        }
        // A `match` of a result as a value: its error's arm returns, its value's is the target's.
        if let Some(Frame::ResMatch(_, r)) = frames.last() {
            let r = r.clone();
            if let Some(v) = l.strip_prefix("Ok(").and_then(|x| x.split_once(") => ")) {
                if v.1.trim_end_matches(',') != v.0 {
                    return Err(why("an `Ok` arm that is not its value".into()));
                }
                continue;
            }
            if let Some((x, rest)) = l.strip_prefix("Err(").and_then(|x| x.split_once(") => ")) {
                let stmt = statement(fx, rest.trim_end_matches(',')).map_err(why)?;
                let _ = writeln!(out, "if (!{r}.ok) {{\nuint32_t {x} = {r}.v;\n{stmt}\n}}");
                continue;
            }
        }
        if let Some((target, x)) = l.strip_suffix(" {").and_then(|r| r.split_once(" = match ")) {
            let e = parse(x).map_err(why)?;
            let r = fx.fresh();
            let _ = writeln!(out, "{}R {r} = {};", fx.p, fx.c(&e).map_err(why)?);
            frames.push(Frame::ResMatch(target.into(), r));
            continue;
        }
        // A labelled loop's `continue`: the innermost loop's own, else the jump word set and each loop left up to it.
        if let Some(label) = l.strip_prefix("continue '").and_then(|r| r.strip_suffix(';')) {
            let inner = frames.iter().rev().find_map(|f| if let Frame::Loop(x) = f { Some(x.clone()) } else { None }).ok_or_else(|| why("a `continue` outside a loop".into()))?;
            if inner.as_deref() == Some(label) {
                out.push_str("continue;\n");
            } else {
                let id = labels.iter().position(|x| x == label).ok_or_else(|| why("a label not open".into()))? + 1;
                let _ = writeln!(out, "jmp_ = {id}u;\nbreak;");
            }
            continue;
        }
        // The token pass's answer, where the walk's `Out` keeps none: the walk reads the token itself. Where it keeps
        // them, its `known` takes the record and gives the token's result.
        if l.starts_with("if let Some(t) = d.known(") {
            if fx.known {
                let o = fx.out.clone();
                let _ = writeln!(out, "{{\n{p}R k_;\nif ({o}_known(d, starts, ip, &k_))\nreturn k_;\n}}", p = fx.p);
            }
            skip = 1;
            continue;
        }
        // A `match`'s arms.
        if let Some(Frame::Match(v, armed)) = frames.last() {
            let (v, armed) = (v.clone(), *armed);
            if l != "}" && !l.starts_with("//") {
                let (pat, rest) = l.split_once(" => ").ok_or_else(|| why("an arm".into()))?;
                let (pat, guard) = match pat.split_once(" if ") {
                    Some((p, g)) => (p, Some(g)),
                    None => (pat, None),
                };
                let mut cond = if pat == "_" {
                    None
                } else if let Some(x) = pat.strip_prefix("Some(").and_then(|r| r.strip_suffix(')')) {
                    if armed {
                        return Err(why("a `Some` arm after the first".into()));
                    }
                    let _ = writeln!(out, "uint32_t {x} = {v};");
                    fx.declare(x, Ty::U32);
                    Some(format!("({v} != UKL_NONE)"))
                } else {
                    Some(fx.c(&parse(&format!("matches!({v}, {pat})")).map_err(why)?).map_err(why)?)
                };
                if let Some(g) = guard {
                    let g = fx.c(&parse(g).map_err(why)?).map_err(why)?;
                    cond = Some(match cond {
                        Some(c) => format!("({c} && {g})"),
                        None => g,
                    });
                }
                let head = match (&cond, armed) {
                    (Some(c), false) => format!("if ({c})"),
                    (Some(c), true) => format!("else if ({c})"),
                    (None, false) => String::new(),
                    (None, true) => "else".into(),
                };
                if let Some(Frame::Match(_, a)) = frames.last_mut() {
                    *a = true;
                }
                if rest == "{" {
                    let _ = writeln!(out, "{head} {{");
                    frames.push(Frame::Arm(false));
                    fx.scopes.push(Vec::new());
                } else if let Some(label) = rest.strip_prefix('\'').and_then(|r| r.strip_suffix(": loop {")) {
                    let _ = writeln!(out, "{head} {{\nwhile (true) {{");
                    frames.push(Frame::Arm(true));
                    fx.scopes.push(Vec::new());
                    if !labels.iter().any(|x| x == label) {
                        labels.push(label.into());
                    }
                    frames.push(Frame::Loop(Some(label.into())));
                    fx.scopes.push(Vec::new());
                } else if rest == "unreachable!()," {
                    let _ = writeln!(out, "{head} {{}}");
                } else if rest == "{}" {
                    let _ = writeln!(out, "{head} {{}}");
                } else {
                    let x = rest.strip_suffix(',').ok_or_else(|| why("an arm's end".into()))?;
                    let stmt = if x.starts_with("return ") || x.starts_with("continue") || x.starts_with("break") {
                        statement(fx, x).map_err(why)?
                    } else {
                        format!("return {};", fx.c(&parse(x).map_err(why)?).map_err(why)?)
                    };
                    let _ = writeln!(out, "{head} {{\n{stmt}\n}}");
                }
                continue;
            }
        }
        if l.starts_with("//") {
            let _ = writeln!(out, "{l}");
            continue;
        }
        // The block's ends.
        if l == "}" || l == "};" || l == "})" || l == "}," {
            let f = frames.pop().ok_or_else(|| why("a brace too many".into()))?;
            if !matches!(f, Frame::Fields(..) | Frame::ResMatch(..)) {
                fx.scopes.pop();
            }
            match f {
                Frame::Fields(_, after) => {
                    let _ = writeln!(out, "{after}");
                }
                Frame::ResMatch(target, r) => {
                    let _ = writeln!(out, "{target} = {r}.v;");
                }
                Frame::Loop(_) => {
                    out.push_str("}\n");
                    // A jump past it: taken by the loop it names, else on out.
                    if !labels.is_empty() {
                        match frames.iter().rev().find_map(|f| if let Frame::Loop(x) = f { Some(x.clone()) } else { None }) {
                            Some(Some(outer)) => {
                                let id = labels.iter().position(|x| *x == outer).map_or(0, |k| k + 1);
                                let _ = writeln!(out, "if (jmp_ != 0u) {{\nif (jmp_ == {id}u) {{\njmp_ = 0u;\ncontinue;\n}}\nbreak;\n}}");
                            }
                            Some(None) => out.push_str("if (jmp_ != 0u)\nbreak;\n"),
                            None => {}
                        }
                    }
                    // A loop that was its arm's body: the arm too.
                    if l == "}," && matches!(frames.last(), Some(Frame::Arm(true))) {
                        frames.pop();
                        fx.scopes.pop();
                        out.push_str("}\n");
                    }
                }
                _ => out.push_str("}\n"),
            }
            continue;
        }
        if let Some(Frame::Fields(target, _)) = frames.last() {
            let (f, v) = l.trim_end_matches(',').split_once(": ").ok_or_else(|| why("a field without its value".into()))?;
            let v = parse(v).map_err(why)?;
            let _ = writeln!(out, "{target}.{f} = {};", fx.c(&v).map_err(why)?);
            continue;
        }
        if l == "{" {
            frames.push(Frame::Plain);
            fx.scopes.push(Vec::new());
            out.push_str("{\n");
            continue;
        }
        if l == "} else {" {
            fx.scopes.pop();
            fx.scopes.push(Vec::new());
            out.push_str("} else {\n");
            continue;
        }
        if l == "continue;" || l == "break;" {
            let _ = writeln!(out, "{l}");
            continue;
        }
        if l == "loop {" {
            out.push_str("while (true) {\n");
            frames.push(Frame::Loop(None));
            fx.scopes.push(Vec::new());
            continue;
        }
        if let Some(label) = l.strip_prefix('\'').and_then(|r| r.strip_suffix(": loop {")) {
            if labels.is_empty() {
                out.push_str("uint32_t jmp_ = 0u;\n");
            }
            if !labels.iter().any(|x| x == label) {
                labels.push(label.into());
            }
            out.push_str("while (true) {\n");
            frames.push(Frame::Loop(Some(label.into())));
            fx.scopes.push(Vec::new());
            continue;
        }
        if let Some(x) = l.strip_prefix("match ").and_then(|r| r.strip_suffix(" {")) {
            let e = parse(x).map_err(why)?;
            let v = format!("m{matches}_");
            matches += 1;
            let _ = writeln!(out, "{{\n{} {v} = {};", fx.ty(&e).c(fx.p), fx.c(&e).map_err(why)?);
            frames.push(Frame::Match(v, false));
            fx.scopes.push(Vec::new());
            continue;
        }
        if let Some(x) = l.strip_prefix("return ").and_then(|r| r.strip_suffix(';')) {
            let _ = writeln!(out, "return {};", fx.c(&parse(x).map_err(why)?).map_err(why)?);
            continue;
        }
        // Two values bound at once: a pair's, or the bytes' marks (`marks`).
        if let Some((lhs, rhs)) = l.strip_prefix("let ").unwrap_or(l).split_once(" = ").filter(|(lhs, _)| lhs.starts_with('(')) {
            let names: Vec<&str> = lhs.trim_matches(|c| c == '(' || c == ')').split(", ").map(|n| n.strip_prefix("mut ").unwrap_or(n)).collect();
            let rhs = rhs.strip_suffix(';').ok_or_else(|| why("a pair without its `;`".into()))?;
            let e = parse(rhs).map_err(why)?;
            match e {
                E::Call(f, args) if matches!(&*f, E::Id(n) if n == "marks") && args.len() == 4 && names.len() == 2 => {
                    for n in &names {
                        if l.starts_with("let ") && !fx.in_this_scope(n) {
                            let _ = writeln!(out, "uint32_t {n};");
                            fx.declare(n, Ty::U32);
                        }
                    }
                    let a = args.iter().map(|a| fx.c(a)).collect::<Result<Vec<_>, _>>().map_err(why)?;
                    let _ = writeln!(out, "ukl_b32_marks({} + {}, {}, {}, &{}, &{});", a[0], a[1], a[2], a[3], names[0], names[1]);
                }
                E::Tuple(vals) if vals.len() == names.len() => {
                    let cs = vals.iter().map(|v| fx.c(v)).collect::<Result<Vec<_>, _>>().map_err(why)?;
                    for ((n, v), c) in names.iter().zip(&vals).zip(&cs) {
                        if fx.in_this_scope(n) || !l.starts_with("let ") {
                            let _ = writeln!(out, "{n} = {c};");
                        } else {
                            let t = fx.ty(v);
                            let _ = writeln!(out, "{} {n} = {c};", t.c(fx.p));
                            fx.declare(n, t);
                        }
                    }
                }
                _ => return Err(why("a pair the printer does not take".into())),
            }
            continue;
        }
        // A struct's value: the function's, a lane's end, or the pass's end with its verdict.
        if l == "Carries {" {
            let c = fx.name("Carries");
            match ret {
                "ends" => frames.push(Frame::Fields("ends_out[l]".into(), String::new())),
                _ => {
                    let _ = writeln!(out, "{c} r_;");
                    frames.push(Frame::Fields("r_".into(), "return r_;".into()));
                }
            }
            continue;
        }
        if let Some(v) = l.strip_prefix("(").and_then(|r| r.strip_suffix(", Carries {")) {
            let v = parse(v).map_err(why)?;
            frames.push(Frame::Fields("(*end_out)".into(), format!("return {};", fx.c(&v).map_err(why)?)));
            continue;
        }
        if l == "core::array::from_fn(|l| {" {
            frames.push(Frame::Plain);
            fx.scopes.push(vec![("l".into(), Ty::U32)]);
            let _ = writeln!(out, "for (uint32_t l = 0u; l < {lanes}; l++) {{");
            continue;
        }
        if let Some(r) = l.strip_prefix("const ") {
            let (n, v) = r.split_once(": ").and_then(|(n, r)| r.split_once(" = ").map(|(_, v)| (n, v))).ok_or_else(|| why("a constant".into()))?;
            let v = v.trim_end_matches(';');
            let e = parse(v).map_err(why)?;
            let c = fx.c(&e).map_err(why)?;
            let t = fx.ty(&e);
            fx.consts.insert(n.into(), (c, t));
            continue;
        }
        if let Some(r) = l.strip_prefix("for ").and_then(|r| r.strip_suffix(" {")) {
            let (v, range) = r.split_once(" in ").ok_or_else(|| why("a loop".into()))?;
            let (a, b) = range.split_once("..").ok_or_else(|| why("a loop's range".into()))?;
            let (a, b) = (parse(a).map_err(why)?, parse(b).map_err(why)?);
            let _ = writeln!(out, "for (uint32_t {v} = {}; {v} < {}; {v}++) {{", fx.c(&a).map_err(why)?, fx.c(&b).map_err(why)?);
            frames.push(Frame::Loop(None));
            fx.scopes.push(vec![(v.into(), Ty::U32)]);
            continue;
        }
        if let Some(r) = l.strip_prefix("while let Some(").and_then(|r| r.strip_suffix(" {")) {
            let (v, x) = r.split_once(") = ").ok_or_else(|| why("a `while let`".into()))?;
            let x = fx.c(&parse(x).map_err(why)?).map_err(why)?;
            let _ = writeln!(out, "while ({x} != UKL_NONE) {{\nuint32_t {v} = {x};");
            frames.push(Frame::Loop(None));
            fx.scopes.push(vec![(v.into(), Ty::U32)]);
            continue;
        }
        if let Some(c) = l.strip_prefix("while ").and_then(|r| r.strip_suffix(" {")) {
            let _ = writeln!(out, "while ({}) {{", fx.c(&parse(c).map_err(why)?).map_err(why)?);
            frames.push(Frame::Loop(None));
            fx.scopes.push(Vec::new());
            continue;
        }
        if let Some(c) = l.strip_prefix("if ").and_then(|r| r.strip_suffix(" {")) {
            let _ = writeln!(out, "if ({}) {{", fx.c(&parse(c).map_err(why)?).map_err(why)?);
            frames.push(Frame::Plain);
            fx.scopes.push(Vec::new());
            continue;
        }
        if let Some(c) = l.strip_prefix("} else if ").and_then(|r| r.strip_suffix(" {")) {
            fx.scopes.pop();
            fx.scopes.push(Vec::new());
            let _ = writeln!(out, "}} else if ({}) {{", fx.c(&parse(c).map_err(why)?).map_err(why)?);
            continue;
        }
        // `x = (0..l).find(|&k| c);`: the first `k` below `l` where `c` holds, else none.
        if let Some((x, r)) = l.split_once(" = (0..") {
            let (bound, r) = r.split_once(").find(|&").ok_or_else(|| why("a search".into()))?;
            let (k, c) = r.split_once("| ").ok_or_else(|| why("a search's test".into()))?;
            let c = c.strip_suffix(");").ok_or_else(|| why("a search's end".into()))?;
            let x = fx.c(&parse(x).map_err(why)?).map_err(why)?;
            let bound = fx.c(&parse(bound).map_err(why)?).map_err(why)?;
            fx.scopes.push(vec![(k.into(), Ty::U32)]);
            let c = fx.c(&parse(c).map_err(why)?).map_err(why)?;
            fx.scopes.pop();
            let _ = writeln!(out, "{x} = UKL_NONE;\nfor (uint32_t {k} = 0u; {k} < {bound}; {k}++) {{\nif ({c}) {{\n{x} = {k};\nbreak;\n}}\n}}");
            continue;
        }
        if let Some(r) = l.strip_prefix("let ") {
            let r = r.strip_prefix("mut ").unwrap_or(r);
            let (lhs, rhs) = r.split_once(" = ").ok_or_else(|| why("a `let` without its value".into()))?;
            let (name, annot) = match lhs.split_once(": ") {
                Some((n, t)) => (n, Some(t)),
                None => (lhs, None),
            };
            // A block's value.
            if rhs == "{" {
                let _ = writeln!(out, "uint64_t {name};\n{{");
                fx.declare(name, Ty::U64);
                frames.push(Frame::Value(name.into()));
                fx.scopes.push(Vec::new());
                continue;
            }
            let rhs = rhs.strip_suffix(';').ok_or_else(|| why("a `let` without its `;`".into()))?;
            // A block of the padded source, where it lies.
            if annot == Some("&[u8; 64]") {
                let (from, _) = rhs.strip_prefix("src[").and_then(|r| r.split_once("..")).ok_or_else(|| why("a block's bytes".into()))?;
                let from = fx.c(&parse(from).map_err(why)?).map_err(why)?;
                let _ = writeln!(out, "UKL_GLOBAL const uint8_t* {name} = src + {from};");
                fx.declare(name, Ty::Ptr("UKL_GLOBAL const uint8_t*".into()));
                continue;
            }
            // A lane's: none yet.
            if let Some(n) = annot.and_then(|a| a.strip_prefix("[Option<usize>; ")).and_then(|a| a.strip_suffix(']')) {
                let n = fx.c(&parse(n).map_err(why)?).map_err(why)?;
                let _ = writeln!(out, "uint32_t {name}[{n}];\nfor (uint32_t i_ = 0u; i_ < {n}; i_++)\n{name}[i_] = UKL_NONE;");
                fx.declare(name, Ty::Arr(Box::new(Ty::U32)));
                continue;
            }
            // `[x; n]`: an array of `x`.
            if let Some((x, n)) = rhs.strip_prefix('[').and_then(|r| r.strip_suffix(']')).and_then(|r| r.split_once("; ")) {
                let (x, n) = (parse(x).map_err(why)?, parse(n).map_err(why)?);
                let t = fx.ty(&x);
                let (xc, nc) = (fx.c(&x).map_err(why)?, fx.c(&n).map_err(why)?);
                let _ = writeln!(out, "{} {name}[{nc}];\nfor (uint32_t i_ = 0u; i_ < {nc}; i_++)\n{name}[i_] = {xc};", t.c(fx.p));
                fx.declare(name, Ty::Arr(Box::new(t)));
                continue;
            }
            let e = parse(rhs).map_err(why)?;
            let t = match annot {
                Some(a) => param_ty(a).map_err(why)?,
                None => fx.ty(&e),
            };
            let c = fx.c(&e).map_err(why)?;
            if fx.in_this_scope(name) {
                let _ = writeln!(out, "{name} = {c};");
            } else {
                let _ = writeln!(out, "{} {name} = {c};", t.c(fx.p));
                fx.declare(name, t);
            }
            continue;
        }
        // A statement: `unsafe { … };` is its call.
        let s = l.strip_prefix("unsafe { ").and_then(|r| r.strip_suffix(" };")).map_or_else(|| l.to_string(), |r| format!("{r};"));
        if s.ends_with(';') {
            let st = statement(fx, &s).map_err(why)?;
            let _ = writeln!(out, "{st}");
            continue;
        }
        // A block's value: its variable's.
        if let Some(Frame::Value(v)) = frames.last() {
            let e = parse(l).map_err(why)?;
            let _ = writeln!(out, "{v} = {};", fx.c(&e).map_err(why)?);
            continue;
        }
        // The function's value.
        if frames.len() == 1 {
            let e = parse(l).map_err(why)?;
            let c = fx.c(&e).map_err(why)?;
            if ret == "void" {
                let _ = writeln!(out, "{c};");
            } else {
                let _ = writeln!(out, "return {c};");
            }
            continue;
        }
        return Err(why("a line the printer does not take".into()));
    }
    let pre = fx.take_pre();
    out.insert_str(mark, &pre);
    if frames.len() != 1 {
        return Err(format!("as UKL: {} blocks left open", frames.len() - 1));
    }
    Ok(())
}

/// One statement (`s`, its `;` with it): an assignment, a `return`, a `continue` or `break`, or a call.
fn statement(fx: &Fx<'_>, s: &str) -> Result<String, String> {
    let s = s.strip_suffix(';').unwrap_or(s);
    if s == "continue" || s == "break" {
        return Ok(format!("{s};"));
    }
    if let Some(x) = s.strip_prefix("return ") {
        return Ok(format!("return {};", fx.c(&parse(x)?)?));
    }
    let toks = lex(s)?;
    let at = toks.iter().position(|t| matches!(t, Tok::P(p) if ["=", "+=", "-=", "|=", "&=", "^=", "<<=", ">>=", "*="].contains(p)));
    Ok(match at {
        Some(k) => {
            let Tok::P(op) = toks[k] else { unreachable!("an assignment") };
            let mut p = Parser { t: toks[..k].to_vec(), i: 0 };
            let lhs = p.expr(0)?;
            let mut q = Parser { t: toks[k + 1..].to_vec(), i: 0 };
            let rhs = q.expr(0)?;
            if p.i != p.t.len() || q.i != q.t.len() {
                return Err("an assignment the printer does not take".into());
            }
            format!("{} {op} {};", fx.c(&lhs)?, fx.c(&rhs)?)
        }
        None => format!("{};", fx.c(&parse(s)?)?),
    })
}

/// The text of the item that starts at line `at` (its signature) to its closing brace at the line's start.
fn item<'a, 'b>(lines: &'b [&'a str], at: usize) -> Result<&'b [&'a str], String> {
    // Its braces counted, outside comments and strings.
    let mut depth = 0i32;
    for (k, l) in lines.iter().enumerate().skip(at) {
        let code = l.split("//").next().unwrap_or("");
        let mut in_str = false;
        for ch in code.chars() {
            match ch {
                '"' => in_str = !in_str,
                '{' if !in_str => depth += 1,
                '}' if !in_str => depth -= 1,
                _ => {}
            }
        }
        if depth == 0 && k > at {
            return Ok(&lines[at + 1..k]);
        }
    }
    Err("an item without its end".into())
}

/// Tier 1's Rust (`rust`, as `bitlex_lex_rust` writes it) as UKL: its tables, its `Carries`, its passes' functions and
/// their kernels, each name prefixed with `p`.
pub fn tier1(rust: &str, p: &str) -> Result<String, String> {
    let lines: Vec<&str> = rust.lines().collect();
    let mut names: HashMap<String, String> = HashMap::new();
    let mut returns: HashMap<String, Ty> = HashMap::new();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "// Generated by native/crates/compiler/src/bitlex_lex_rust/ukl.rs from tier 1's Rust; do not edit. Tier 1's passes as UKL,\n// its passes a\n// chunk at a time, a launch a phase (decision 0726).\n\n#pragma once\n#include <ukl/blx64.h>\n#include <ukl/memory.h>\n"
    );
    // The constants, the tables and the carries.
    let mut fields: Vec<String> = Vec::new();
    let (mut lanes, mut types) = (None, None);
    for (k, l) in lines.iter().enumerate() {
        if let Some(r) = l.strip_prefix("pub const ").and_then(|r| r.strip_suffix(';')) {
            // The counts (the names' and the like are the host's).
            let Some((n, v)) = r.split_once(": usize = ") else { continue };
            if v.parse::<u32>().is_ok() {
                names.insert(n.into(), format!("{p}{n}"));
                let _ = writeln!(out, "#define {p}{n} {v}u");
                match n {
                    "LANES" => lanes = Some(v.to_string()),
                    "TYPES" => types = Some(v.to_string()),
                    _ => {}
                }
            }
        } else if let Some(r) = l.strip_prefix("static ").and_then(|r| r.strip_suffix("];")) {
            let (n, r) = r.split_once(": [u8; ").ok_or_else(|| format!("tier 1 as UKL: a table `{n}`", n = &l[..l.len().min(40)]))?;
            let (len, vals) = r.split_once("] = [").ok_or("tier 1 as UKL: a table's bytes")?;
            names.insert(n.into(), format!("{p}{n}"));
            let _ = writeln!(out, "UKL_TABLE uint8_t {p}{n}[{len}] = {{{vals}}};");
        } else if *l == "pub struct Carries {" {
            for f in item(&lines, k)? {
                let f = f.trim().strip_prefix("pub ").and_then(|f| f.strip_suffix(": u64,")).ok_or_else(|| format!("tier 1 as UKL: a carry `{f}`"))?;
                fields.push(f.into());
            }
        }
    }
    let (Some(lanes), Some(types)) = (lanes, types) else { return Err("tier 1 as UKL: no LANES or TYPES".into()) };
    if fields.is_empty() {
        return Err("tier 1 as UKL: no carries".into());
    }
    names.insert("Carries".into(), format!("{p}Carries"));
    let _ = writeln!(out, "\ntypedef struct {p}Carries {{");
    for f in &fields {
        let _ = writeln!(out, "    uint64_t {f};");
    }
    let _ = writeln!(out, "}} {p}Carries;\n");
    // The functions of the passes, in their order.
    let wanted = ["lane_carries", "write_exact", "write_numbers", "put_line", "stage1_ends", "stage1_from"];
    for f in wanted {
        names.insert(f.into(), format!("{p}{f}"));
    }
    returns.insert("lane_carries".into(), Ty::Carries);
    for f in wanted {
        let at = lines
            .iter()
            .position(|l| (l.starts_with("fn ") || l.starts_with("pub fn ") || l.starts_with("unsafe fn ")) && l.contains(&format!("fn {f}(")))
            .ok_or_else(|| format!("tier 1 as UKL: no `{f}`"))?;
        let sig = lines[at];
        // Its parameters: to the parenthesis that closes the first; then its return type.
        let open = sig.find('(').ok_or("tier 1 as UKL: a signature")?;
        let mut d = 0;
        let close = sig[open..]
            .char_indices()
            .find(|&(_, ch)| {
                d += match ch {
                    '(' => 1,
                    ')' => -1,
                    _ => 0,
                };
                d == 0
            })
            .map(|(k, _)| open + k)
            .ok_or("tier 1 as UKL: a signature's end")?;
        let ps = &sig[open + 1..close];
        let rett = sig[close..].strip_prefix(") -> ").map(|r| r.trim_end_matches(" {"));
        let mut params: Vec<(String, Ty)> = Vec::new();
        // Its parameters (split at the commas outside brackets).
        let mut depth = 0;
        let mut cur = String::new();
        let mut parts = Vec::new();
        for ch in ps.chars() {
            match ch {
                '[' | '(' | '<' => depth += 1,
                ']' | ')' | '>' => depth -= 1,
                ',' if depth == 0 => {
                    parts.push(std::mem::take(&mut cur));
                    continue;
                }
                _ => {}
            }
            cur.push(ch);
        }
        parts.push(cur);
        for part in parts {
            let part = part.trim();
            let part = part.strip_prefix("mut ").unwrap_or(part);
            let (n, t) = part.split_once(": ").ok_or_else(|| format!("tier 1 as UKL: a parameter `{part}`"))?;
            params.push((n.into(), param_ty(t)?));
        }
        let (cret, kind) = match rett {
            None => ("void".to_string(), "void"),
            Some("Carries") => (format!("{p}Carries"), "value"),
            Some(r) if r.starts_with("[Carries;") => {
                params.push(("ends_out".into(), Ty::Ptr(format!("UKL_PRIVATE {p}Carries*"))));
                ("void".to_string(), "ends")
            }
            Some("(bool, Carries)") => {
                params.push(("end_out".into(), Ty::Ptr(format!("UKL_PRIVATE {p}Carries*"))));
                ("bool".to_string(), "verdict")
            }
            Some(r) => return Err(format!("tier 1 as UKL: a return type `{r}`")),
        };
        let plist = params.iter().map(|(n, t)| format!("{} {n}", t.c(p))).collect::<Vec<_>>().join(", ");
        let _ = writeln!(out, "UKL_INLINE {cret} {p}{f}({plist})\n{{");
        let mut fx = Fx { p, names: &names, returns: &returns, scopes: vec![params], consts: HashMap::new(), pre: Default::default(), tmp: Default::default(), known: false, out: String::new() };
        let mut text = String::new();
        body(&mut fx, item(&lines, at)?, kind, &mut text)?;
        out.push_str(&text);
        out.push_str("}\n\n");
    }
    // The program (`<ukl/lex_run.h>`): its hooks, from the passes, and its kernel.
    let c = format!("{p}Carries");
    let lane_of = fields.iter().enumerate().map(|(k, f)| format!("((uint32_t)(e[l].{f} & 1u) << {k}u)")).collect::<Vec<_>>().join(" | ");
    let _ = write!(
        out,
        r#"#define UKL_LEX_LANES {p}LANES
#define UKL_LEX_TYPES {p}TYPES
#include <ukl/lex_run.h>

// Pass 1 (`stage1_ends`): each entry lane's end lane (`lane_of`).
UKL_INLINE void ukl_lex_ends(UKL_GLOBAL const uint8_t* src, uint32_t n, uint32_t from, uint32_t to, UKL_PRIVATE uint32_t* ends)
{{
    {c} e[{p}LANES];
    {p}stage1_ends(src, n, from, to, e);
    for (uint32_t l = 0u; l < {p}LANES; l++)
        ends[l] = {lane_of};
}}

// Pass 2 (`stage1_from`) from entry lane `lane` (`lane_carries`).
UKL_INLINE bool ukl_lex_from(UKL_GLOBAL const uint8_t* src, uint32_t n, uint32_t from, uint32_t to, uint32_t lane, UKL_PRIVATE uint32_t* at,
                             UKL_GLOBAL uint32_t* starts, UKL_GLOBAL uint32_t* types, bool write, UKL_GLOBAL uint32_t* lines,
                             UKL_PRIVATE const uint32_t* shape)
{{
    {c} end;
    return {p}stage1_from(src, n, from, to, {p}lane_carries(lane), at, starts, types, write, lines, shape, &end);
}}

typedef struct {p}LArgs {{
    uint32_t n;
    uint32_t phase;
}} {p}LArgs;
UKL_REFLECT_STRUCT({p}LArgs, n, phase)

// Tier 1 from the bytes (`<ukl/lex_run.h>`), its phase `args.phase`: its SIMD groups take the phase's chunks by
// ticket. The pool's header is the host's layout and what the phases say (the records, the refusal, the room they
// need past their room); `starts` takes the records and their three words after, `types` each type's records'
// numbers, `tokens` each record's token none; `meet` the trees' arrivals.
DEFINE_KERNEL2(tier1, {p}lex,
               ARGS,     {p}LArgs, args,   ,
               ROBUFFER, uint8_t,  src,    "[n + 64]",
               RWBUFFER, uint32_t, pool,   "[pool]",
               RWBUFFER, uint32_t, starts, "[room + 3]",
               RWBUFFER, uint32_t, types,  "[room]",
               RWBUFFER, uint32_t, tokens, "[room * 5]",
               RWBUFFER, uint32_t, meet,   "[2 * tn]",
               LANE,     uint16_t, lane,   )
{{
    UklLex r;
    r.src = src;
    r.m = (UklLexPool)pool;
    r.starts = starts;
    r.types = types;
    r.tokens = tokens;
    r.meet = meet;
    r.phase = args.phase;
    ukl_lex_worker(&r, lane __UKL_ARGS);
}}
"#
    );
    Ok(out)
}

/// The line where function `f` starts (`fn f(` or `fn f<`).
fn find_fn(lines: &[&str], f: &str) -> Result<usize, String> {
    lines
        .iter()
        .position(|l| {
            let l = l.trim_start_matches("pub ").trim_start_matches("unsafe ");
            l.starts_with(&format!("fn {f}(")) || l.starts_with(&format!("fn {f}<"))
        })
        .ok_or_else(|| format!("as UKL: no `{f}`"))
}

/// Function `f` of `lines` printed as `{name}` (its walk's `Out` the C struct `out`, where it takes one).
fn print_fn(lines: &[&str], f: &str, name: &str, out_c: &str, p: &str, names: &HashMap<String, String>, returns: &HashMap<String, Ty>, out: &mut String) -> Result<(), String> {
    let at = find_fn(lines, f)?;
    let sig = lines[at];
    let open = sig.find('(').ok_or("as UKL: a signature")?;
    let mut d = 0;
    let close = sig[open..]
        .char_indices()
        .find(|&(_, ch)| {
            d += match ch {
                '(' | '[' => 1,
                ')' | ']' => -1,
                _ => 0,
            };
            d == 0
        })
        .map(|(k, _)| open + k)
        .ok_or("as UKL: a signature's end")?;
    let rett = sig[close..].strip_prefix(") -> ").map(|r| r.trim_end_matches(" {"));
    let mut params: Vec<(String, Ty)> = Vec::new();
    let mut depth = 0;
    let mut cur = String::new();
    let mut parts = Vec::new();
    for ch in sig[open + 1..close].chars() {
        match ch {
            '[' | '(' | '<' => depth += 1,
            ']' | ')' | '>' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(ch);
    }
    parts.push(cur);
    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let part = part.strip_prefix("mut ").unwrap_or(part);
        let (n, t) = part.split_once(": ").ok_or_else(|| format!("as UKL: a parameter `{part}`"))?;
        // The walk's: tier 1's records and its place, by value.
        let ty = match t {
            "&crate::tier1::Lexed" => Ty::Named(format!("{p}Lx")),
            "Entry" => Ty::Named(format!("{p}Entry")),
            _ => param_ty_of(t, out_c)?,
        };
        params.push((n.into(), ty));
    }
    let (cret, kind) = match rett {
        Some("Stop") => (format!("{p}Stop"), "stop"),
        None => ("void".to_string(), "void"),
        Some("Result<usize, u32>") => (format!("{p}R"), "value"),
        Some("Option<usize>" | "usize" | "u32" | "u8") => ("uint32_t".to_string(), "value"),
        Some("bool") => ("bool".to_string(), "value"),
        Some(r) => return Err(format!("as UKL: `{f}`'s return type `{r}`")),
    };
    let plist = params.iter().map(|(n, t)| format!("{} {n}", t.c(p))).collect::<Vec<_>>().join(", ");
    let _ = writeln!(out, "UKL_INLINE {cret} {name}({plist})\n{{");
    let mut fx = Fx { p, names, returns, scopes: vec![params], consts: HashMap::new(), pre: Default::default(), tmp: Default::default(), known: out_c.ends_with("Sh"), out: out_c.to_string() };
    body(&mut fx, item(lines, at)?, kind, out).map_err(|e| format!("`{f}`: {e}"))?;
    // Past the walk's loop, which every way out of returns from.
    if kind == "stop" {
        let _ = writeln!(out, "return {p}stop_err(UKL_NONE);");
    }
    out.push_str("}\n\n");
    Ok(())
}

/// The tables of `lines` (`static X: [u8 | u32; N] = [...];`), each `UKL_TABLE`, its name prefixed.
fn tables(lines: &[&str], p: &str, names: &mut HashMap<String, String>, out: &mut String) -> Result<(), String> {
    for l in lines {
        let Some(r) = l.strip_prefix("static ").and_then(|r| r.strip_suffix("];")) else { continue };
        let (n, r) = r.split_once(": [").ok_or("as UKL: a table")?;
        let (t, r) = r.split_once("; ").ok_or("as UKL: a table's type")?;
        let (len, vals) = r.split_once("] = [").ok_or("as UKL: a table's values")?;
        let (ct, suffix) = match t {
            "u8" => ("uint8_t", ""),
            "u32" => ("uint32_t", "u"),
            _ => return Err(format!("as UKL: a table of `{t}`")),
        };
        let vals: Vec<String> = vals.split(", ").map(|v| format!("{}{suffix}", v.replace('_', ""))).collect();
        names.insert(n.into(), format!("{p}{n}"));
        let _ = writeln!(out, "UKL_TABLE {ct} {p}{n}[{len}] = {{{}}};", vals.join(", "));
    }
    Ok(())
}

/// Tier 2's token pass (stage 2, decision 0726) as UKL, from tier 2's Rust (`rust`, as `one_loop` writes it): the
/// primitive's helpers printed for the pass's `Out` (`{p}Tok`, the first node's number, the nodes, the last one's
/// start) and its kernel `{p}tokens`, a thread a record: the token at each record of `list` (tier 1's type lists, one
/// buffer), its five words (`end`, `kind`, `nodes`, `last`, `err`) at the record's place in `tokens`.
pub fn tier2(rust: &str, p: &str) -> Result<String, String> {
    let lines: Vec<&str> = rust.lines().collect();
    let mut names: HashMap<String, String> = HashMap::new();
    let mut returns: HashMap<String, Ty> = HashMap::new();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "// Generated by native/crates/compiler/src/bitlex_lex_rust/ukl.rs from tier 2's Rust; do not edit. Tier 2's passes as\n// UKL (decision 0726): the token pass, a thread a record, and the passes from the middle as one program.\n\n#pragma once\n#include <ukl/blx64.h>\n#include <ukl/memory.h>\n#include <ukl/writer_mid.h>\n#include <ukl/middle.h>\n#include <ukl/middle_run.h>\n#include <ukl/z16_run.h>\n#include <ukl/lex_pool.h>\n#include <ukl/t2_phases.h>\n"
    );
    tables(&lines, p, &mut names, &mut out)?;
    let depth = lines
        .iter()
        .find_map(|l| l.strip_prefix("const DEPTH: usize = ").and_then(|r| r.strip_suffix(';')))
        .ok_or("as UKL: no DEPTH")?;
    names.insert("DEPTH".into(), format!("{p}DEPTH"));
    let _ = writeln!(out, "#define {p}DEPTH {depth}u");
    // The primitive: its name and its numbers' count; the tokens' first bytes.
    let prim = lines
        .iter()
        .find_map(|l| l.strip_prefix("unsafe fn ").and_then(|r| r.split_once("<D: Out>(")).map(|(n, _)| n).filter(|n| n.ends_with("_primitive")))
        .ok_or("as UKL: no primitive")?;
    let arity: usize = lines[find_fn(&lines, prim)?]
        .split_once("n: [u32; ")
        .and_then(|(_, r)| r.split_once(']'))
        .and_then(|(k, _)| k.parse().ok())
        .ok_or("as UKL: the primitive's numbers")?;
    let firsts = lines
        .iter()
        .find_map(|l| l.strip_prefix("pub const TOKEN_FIRSTS: &[(u8, u8)] = &[").and_then(|r| r.strip_suffix("];")))
        .ok_or("as UKL: no TOKEN_FIRSTS")?;
    let mut tests = Vec::new();
    for pair in firsts.trim_start_matches('(').trim_end_matches(')').split("), (") {
        let (a, z) = pair.split_once(", ").ok_or("as UKL: a first byte's range")?;
        tests.push(if a == z { format!("b == {a}u") } else { format!("(b >= {a}u && b <= {z}u)") });
    }
    let _ = write!(
        out,
        r#"
#define {p}NONE 4294967295u

// A `Result<usize, u32>`: its value, or the byte it is refused at.
typedef struct {p}R {{
    bool ok;
    uint32_t v;
}} {p}R;
UKL_INLINE {p}R {p}ok(uint32_t v)
{{
    {p}R r;
    r.ok = true;
    r.v = v;
    return r;
}}
UKL_INLINE {p}R {p}err(uint32_t v)
{{
    {p}R r;
    r.ok = false;
    r.v = v;
    return r;
}}

// The token pass's `Out`: the first node's number, the nodes, the last one's start.
typedef struct {p}Tok {{
    uint32_t kind;
    uint32_t nodes;
    uint32_t last;
}} {p}Tok;
UKL_INLINE void {p}Tok_node(UKL_PRIVATE {p}Tok* d, uint32_t number, uint64_t start)
{{
    if (d->nodes == 0u)
        d->kind = number;
    d->nodes += 1u;
    d->last = (uint32_t)start;
}}
UKL_INLINE void {p}Tok_open(UKL_PRIVATE {p}Tok* d, uint32_t number, uint64_t start) {{ {p}Tok_node(d, number, start); }}
UKL_INLINE void {p}Tok_leaf(UKL_PRIVATE {p}Tok* d, uint32_t number, uint64_t start, uint64_t end) {{ {p}Tok_node(d, number, start); }}
UKL_INLINE void {p}Tok_leaf_in(UKL_PRIVATE {p}Tok* d, uint32_t number, uint32_t child, uint64_t start, uint64_t cs, uint64_t ce, uint64_t end)
{{
    {p}Tok_node(d, number, start);
    {p}Tok_node(d, number, cs);
}}
UKL_INLINE void {p}Tok_close(UKL_PRIVATE {p}Tok* d, uint64_t end) {{}}

// Pass 1's `Out` (`Shadow`): the writer's frames kept as the cursor keeps them, without writing: the nodes written
// from the range's first (`n`), the frames the range opened (`own`, `UKL_MID_FRAME` words each, at most `most_own`),
// what it wrote into those open at its entry, innermost first (`touch`, two words each: children, last child or
// `NONE`; at most `most_touch`), how many of those it closed, and the start of its last node. `full` where it passed
// its room. It reads the token pass's answers (`toks`, five words a record) where it has them.
typedef struct {p}Sh {{
    UKL_GLOBAL const uint32_t* toks;
    uint32_t ntoks;
    uint32_t n;
    UKL_GLOBAL uint32_t* own;
    uint32_t nown;
    uint32_t most_own;
    UKL_GLOBAL uint32_t* touch;
    uint32_t ntouch;
    uint32_t most_touch;
    uint32_t closed;
    uint64_t last_start;
    bool has_last;
    bool full;
}} {p}Sh;
UKL_INLINE uint32_t {p}Sh_node(UKL_PRIVATE {p}Sh* d, uint64_t start)
{{
    uint32_t i = d->n;
    if (d->nown > 0u) {{
        UKL_GLOBAL uint32_t* f = d->own + UKL_MID_FRAME * (d->nown - 1u);
        f[1] += 1u;
        f[2] = i;
    }} else if (d->closed >= d->most_touch) {{
        d->full = true;
    }} else {{
        uint32_t k = d->closed;
        while (d->ntouch <= k) {{
            d->touch[2u * d->ntouch] = 0u;
            d->touch[2u * d->ntouch + 1u] = {p}NONE;
            d->ntouch += 1u;
        }}
        d->touch[2u * k] += 1u;
        d->touch[2u * k + 1u] = i;
    }}
    d->n += 1u;
    d->last_start = start;
    d->has_last = true;
    return i;
}}
UKL_INLINE void {p}Sh_open(UKL_PRIVATE {p}Sh* d, uint32_t number, uint64_t start)
{{
    uint32_t i = {p}Sh_node(d, start);
    if (d->nown >= d->most_own) {{
        d->full = true;
        return;
    }}
    UKL_GLOBAL uint32_t* f = d->own + UKL_MID_FRAME * d->nown;
    f[0] = i;
    f[1] = 0u;
    f[2] = {p}NONE;
    f[3] = (uint32_t)start;
    f[4] = (uint32_t)(start >> 32);
    d->nown += 1u;
}}
UKL_INLINE void {p}Sh_leaf(UKL_PRIVATE {p}Sh* d, uint32_t number, uint64_t start, uint64_t end) {{ {p}Sh_node(d, start); }}
UKL_INLINE void {p}Sh_leaf_in(UKL_PRIVATE {p}Sh* d, uint32_t number, uint32_t child, uint64_t start, uint64_t cs, uint64_t ce, uint64_t end)
{{
    {p}Sh_node(d, start);
    d->n += 1u;
    d->last_start = cs;
}}
UKL_INLINE void {p}Sh_close(UKL_PRIVATE {p}Sh* d, uint64_t end)
{{
    if (d->nown > 0u)
        d->nown -= 1u;
    else
        d->closed += 1u;
}}
// The token pass's answer at record `*ip` (`known`, then `skip`): where it has one, the token's nodes as one child of
// the node open, the record past, and its end or the byte it is refused at in `*k`.
UKL_INLINE bool {p}Sh_known(UKL_PRIVATE {p}Sh* d, UKL_GLOBAL const uint32_t* starts, UKL_PRIVATE uint32_t* ip, UKL_PRIVATE {p}R* k)
{{
    uint32_t i = *ip;
    if (i >= d->ntoks)
        return false;
    UKL_GLOBAL const uint32_t* t = d->toks + 5u * i;
    if (t[4] != {p}NONE) {{
        *ip = i + 1u;
        *k = {p}err(t[4]);
        return true;
    }}
    if (t[1] == {p}NONE)
        return false;
    {p}Sh_node(d, (uint64_t)starts[i]);
    *ip = i + 1u;
    d->n += t[2] - 1u;
    d->last_start = (uint64_t)t[3];
    *k = {p}ok(t[0]);
    return true;
}}

// Tier 1's records: their starts (then the index's three last entries), their count, the file's length.
typedef struct {p}Lx {{
    UKL_GLOBAL const uint32_t* starts;
    uint32_t count;
    uint32_t len;
}} {p}Lx;

// The walk's state at a place: the next record, the last token's end, the levels open, the place.
typedef struct {p}Entry {{
    uint32_t i;
    uint32_t e;
    uint32_t depth;
    uint32_t st;
}} {p}Entry;

// Where a walk stops (`Stop`): `kind` 0 at a place at or past its end, 1 where it closes a level it does not know,
// 2 the file's end, 3 its refusal at byte `err`.
typedef struct {p}Stop {{
    uint32_t kind;
    {p}Entry en;
    uint32_t err;
}} {p}Stop;
UKL_INLINE {p}Stop {p}stop_of(uint32_t kind, uint32_t i, uint32_t e, uint32_t depth, uint32_t st, uint32_t err)
{{
    {p}Stop r;
    r.kind = kind;
    r.en.i = i;
    r.en.e = e;
    r.en.depth = depth;
    r.en.st = st;
    r.err = err;
    return r;
}}
UKL_INLINE {p}Stop {p}stop_at(uint32_t i, uint32_t e, uint32_t depth, uint32_t st) {{ return {p}stop_of(0u, i, e, depth, st, 0u); }}
UKL_INLINE {p}Stop {p}stop_read(uint32_t i, uint32_t e, uint32_t depth, uint32_t st) {{ return {p}stop_of(1u, i, e, depth, st, 0u); }}
UKL_INLINE {p}Stop {p}stop_ok() {{ return {p}stop_of(2u, 0u, 0u, 0u, 0u, 0u); }}
UKL_INLINE {p}Stop {p}stop_err(uint32_t x) {{ return {p}stop_of(3u, 0u, 0u, 0u, 0u, x); }}

"#
    );
    // The helpers: those of no walk once, the walk's for its `Out`.
    let plain = ["ends", "check_number", "number_end", "at", "peek"];
    let walk = ["open", "leaf", "leaf_in", "close", "token", "string", prim];
    for f in plain {
        names.insert(f.into(), format!("{p}{f}"));
    }
    for f in walk {
        names.insert(f.into(), format!("{p}{f}_Tok"));
    }
    for f in ["number_end", "string", prim] {
        returns.insert(f.into(), Ty::Res);
    }
    returns.insert("ends".into(), Ty::Bool);
    for f in ["stop_at", "stop_read", "stop_ok", "stop_err"] {
        names.insert(f.into(), format!("{p}{f}"));
    }
    for f in plain {
        print_fn(&lines, f, &format!("{p}{f}"), "", p, &names, &returns, &mut out)?;
    }
    for f in walk {
        print_fn(&lines, f, &format!("{p}{f}_Tok"), &format!("{p}Tok"), p, &names, &returns, &mut out)?;
    }
    // Pass 2's: each walk helper, then the walk from a place, on the writer's cursor from the middle.
    let mut mid = names.clone();
    for f in walk.iter().chain(&["run_range"]) {
        mid.insert((*f).into(), format!("{p}{f}_Mid"));
    }
    for f in walk.iter().chain(&["run_range"]) {
        print_fn(&lines, f, &format!("{p}{f}_Mid"), "UklWMid", p, &mid, &returns, &mut out)?;
    }
    // Pass 1's: the walk helpers and the walk from a place, on the shadow of the writer's frames.
    let mut sh = names.clone();
    for f in walk.iter().chain(&["run_range"]) {
        sh.insert((*f).into(), format!("{p}{f}_Sh"));
    }
    for f in walk.iter().chain(&["run_range"]) {
        print_fn(&lines, f, &format!("{p}{f}_Sh"), &format!("{p}Sh"), p, &sh, &returns, &mut out)?;
    }
    // The places a chunk's first record can be at, those a level returns to, and each level kind's return place and
    // cycle place, as functions of their index.
    for (name, c) in [("CYCLES", "cycles"), ("RETS", "rets"), ("CONT", "cont"), ("CYCLE", "cycle")] {
        let v: Vec<&str> = lines
            .iter()
            .find_map(|l| l.strip_prefix(&format!("pub const {name}: &[u8] = &[")).and_then(|r| r.strip_suffix("];")))
            .ok_or(format!("as UKL: no {name}"))?
            .split(", ")
            .collect();
        let mut body = format!("{}u", v[v.len() - 1]);
        for (k, x) in v.iter().enumerate().rev().skip(1) {
            body = format!("k == {k}u ? {x}u : {body}");
        }
        let _ = writeln!(out, "#define {p}N{name} {}u\nUKL_INLINE uint32_t {p}{c}(uint32_t k) {{ return {body}; }}", v.len());
    }
    // A record's bracket by its first byte: its kind twice, plus one where it opens; `NONE` for none.
    let at = lines.iter().position(|l| l.trim_start().starts_with("fn bracket(b: u8)")).ok_or("as UKL: no bracket")?;
    let mut bt = String::new();
    for l in &lines[at..] {
        let t = l.trim();
        if t.starts_with("_ => None") {
            break;
        }
        if let Some((byte, rest)) = t.split_once(" => Some((")
            && let Some((k, open)) = rest.trim_end_matches("),").trim_end_matches(')').split_once(", ")
        {
            let code = 2 * k.parse::<u32>().map_err(|_| "as UKL: a bracket's kind")? + u32::from(open == "true");
            let _ = write!(bt, "b == {byte}u ? {code}u : ");
        }
    }
    let _ = writeln!(out, "#define {p}ROOT ({p}NCONT - 1u)\nUKL_INLINE uint32_t {p}bracket(uint32_t b) {{ return {bt}{p}NONE; }}");
    // A list's separators, the bytes a chunk starts after.
    let seps = lines
        .iter()
        .find_map(|l| l.strip_prefix("pub const SEPS: &[u8] = &[").and_then(|r| r.strip_suffix("];")))
        .ok_or("as UKL: no SEPS")?
        .split(", ")
        .map(|b| format!("b == {b}u"))
        .collect::<Vec<_>>()
        .join(" || ");
    let nums = (0..arity).map(|k| format!("{k}u")).collect::<Vec<_>>().join(", ");
    let _ = write!(
        out,
        r#"typedef struct {p}TArgs {{
    uint32_t n;
    uint32_t count;
}} {p}TArgs;
UKL_REFLECT_STRUCT({p}TArgs, n, count)

// The token at record `list[j]` (the token pass's), written; its nodes.
UKL_INLINE uint32_t {p}token_at(UKL_GLOBAL const uint8_t* src, UKL_GLOBAL const uint32_t* starts, UKL_GLOBAL const uint32_t* list, UKL_GLOBAL uint32_t* tokens, uint32_t j)
{{
    uint32_t r = list[j];
    uint32_t b = src[starts[r]];
    uint32_t t[5] = {{0u, {p}NONE, 0u, 0u, {p}NONE}};
    if ({firsts}) {{
        {p}Tok d;
        d.kind = 0u;
        d.nodes = 0u;
        d.last = 0u;
        uint32_t i = r;
        uint32_t nn[{arity}] = {{{nums}}};
        {p}R x = {p}{prim}_Tok(src, starts, &d, &i, nn);
        if (x.ok) {{
            t[0] = x.v;
            t[1] = d.kind;
            t[2] = d.nodes;
            t[3] = d.last;
        }} else {{
            t[4] = x.v;
        }}
    }}
    for (uint32_t k = 0u; k < 5u; k++)
        tokens[r * 5u + k] = t[k];
    return t[2];
}}

// Stage 2: the token at record `list[j]`, read on its own (`token_at`): its end, its first node's number among the
// primitive's alternatives, its nodes, the last one's start, or the byte it is refused at (`err`, else `NONE`). The
// list is tier 1's types' (`<ukl/lex_run.h>`), as many as its pool says (`lexed`); the grid `args.n` threads, at
// least that many. A record that starts no token keeps the none tier 1 wrote. The tokens' nodes are summed in the
// pool (`UKL_LEX_NODES`, a SIMD group's sum an add): with the records, tier 2's room for the tape's nodes.
DEFINE_KERNEL2(tier2, {p}tokens,
               ARGS,     {p}TArgs, args,   ,
               ROBUFFER, uint8_t,  src,    "[n + 64]",
               ROBUFFER, uint32_t, starts, "[count + 3]",
               ROBUFFER, uint32_t, list,   "[n]",
               RWBUFFER, uint32_t, lexed,  "[32]",
               RWBUFFER, uint32_t, tokens, "[count * 5]",
               GID0,     uint32_t, j,      ,
               LANE,     uint16_t, lane,   )
{{
    // Every lane goes on to the group's sum.
    bool mine = j < args.n && j < lexed[UKL_LEX_TYPED] && lexed[UKL_LEX_REFUSED] == 0u && lexed[UKL_LEX_SHORT] == 0u;
    uint32_t nodes = mine ? {p}token_at(src, starts, list, tokens, j) : 0u;
    uint32_t sum = ukl_sg_sum(nodes __UKL_ARGS);
    if (lane == 0u && sum != 0u)
        ukl_atomic_add(&((UklLexPool)lexed)[UKL_LEX_NODES], sum);
}}

// Pass 2 (`write_range`), chunk `c` from its state `a` (pass 3's, in the pool), through a cursor from the middle on
// the writer's region (its state at 0, its blocks made): counting its side-list entries, or writing them. Its frames'
// room is placed by its first record and its number (`5 * (span + 1)` frames: those open at its entry, and as many as
// its records open); its levels in its row of `rets`. Where it stopped: its two words of `out`, its stop's kind and
// record (4: no state, 5: past its room, 6: after the end).
UKL_ONCE void ukl_run_write(UKL_PRIVATE UklRun* r, uint32_t c, uint32_t a, uint32_t start, uint32_t stop, bool count)
{{
    UklMidPool pool = r->m;
    UklMidPool out = pool + pool[UKL_RUN_O_OUT];
    out[c * 2u + 1u] = 0u;
    if (a >= UKL_MID_FULL || pool[a + 5u] != 0u) {{
        out[c * 2u] = a >= UKL_MID_FULL ? 4u : 6u;
        return;
    }}
    uint32_t span = stop - start;
    uint32_t nret = pool[a + 9u], nf = pool[a + 10u];
    if (nf + 3u * span + 4u > 5u * (span + 1u)) {{
        out[c * 2u] = 5u;
        return;
    }}
    UKL_GLOBAL uint32_t* room = r->f2 + UKL_WMID_FRAME * 5u * (start + c);
    for (uint32_t k = 0u; k < nf * UKL_WMID_FRAME; k++)
        room[k] = pool[a + UKL_AT_HEAD + nret + k];
    UKL_GLOBAL uint8_t* ret = r->rets + c * {p}DEPTH;
    uint32_t base = pool[a + 2u] - nret;
    for (uint32_t k = 0u; k < nret; k++)
        ret[base + k] = (uint8_t)pool[a + UKL_AT_HEAD + k];
    {p}Lx lx;
    lx.starts = r->starts;
    lx.count = r->count;
    lx.len = r->len;
    {p}Entry en;
    en.i = pool[a];
    en.e = pool[a + 1u];
    en.depth = pool[a + 2u];
    en.st = pool[a + 3u];
    uint64_t prev = (uint64_t)pool[a + 7u] | ((uint64_t)pool[a + 8u] << 32);
    UklWMid d = ukl_wmid_resume(r->t, 0u, pool[a + 4u], prev, room, nf, count);
    {p}Stop x = {p}run_range_Mid(r->src, lx, &d, en, ret, 0u, stop);
    out[c * 2u] = x.kind;
    out[c * 2u + 1u] = x.en.i;
}}

// The span of a walk that stopped at `r` (`Span::of`), from its shadow `d`: its offset in the pool, or `UKL_MID_FULL`.
UKL_INLINE uint32_t {p}span_of(UklMidPool m, {p}Stop r, UKL_PRIVATE const {p}Sh* d)
{{
    bool end = r.kind >= 2u;
    uint32_t nt = d->closed + 1u;
    uint32_t npf = end ? 0u : d->nown;
    uint32_t at = ukl_mid_take(m, UKL_SPAN_HEAD + 2u * nt + UKL_MID_FRAME * npf);
    if (at == UKL_MID_FULL)
        return at;
    m[at] = r.kind == 0u ? r.en.i : 0u;
    m[at + 1u] = r.kind == 0u ? r.en.e : 0u;
    m[at + 2u] = r.kind == 0u ? r.en.depth : 0u;
    m[at + 3u] = r.kind == 0u ? r.en.st : 0u;
    m[at + 4u] = d->n;
    m[at + 5u] = r.kind == 2u ? 1u : r.kind == 3u ? 2u : 0u;
    m[at + 6u] = r.kind == 3u ? r.err : 0u;
    m[at + 7u] = d->closed;
    m[at + 8u] = nt;
    m[at + 9u] = npf;
    m[at + 10u] = d->has_last ? (uint32_t)d->last_start : 0u;
    m[at + 11u] = d->has_last ? (uint32_t)(d->last_start >> 32) : 0u;
    m[at + 12u] = d->has_last ? 1u : 0u;
    uint32_t w = at + UKL_SPAN_HEAD;
    for (uint32_t k = 0u; k < nt; k++) {{
        bool has = !end && k < d->ntouch;
        m[w++] = has ? d->touch[2u * k] : 0u;
        m[w++] = has ? d->touch[2u * k + 1u] : {p}NONE;
    }}
    for (uint32_t k = 0u; k < UKL_MID_FRAME * npf; k++)
        m[w++] = d->own[k];
    return at;
}}

// Pass 1 (`span`): chunk `c`'s one walk, from the place and the levels' places its open levels' kinds `k` give (the
// levels it closes, its word `w`'s anti-opens), so it never reads a level it does not know. Kinds of none are after
// the file's refusal, as one deeper than the walk goes: the span ends at its first record. Its frames are kept in
// `f1` (`2 * span + 2`), its writes into its entry's in `touch` (`span + 1`), each placed by its first record and its
// number; its levels' places in its row of `rets`.
UKL_ONCE uint32_t ukl_run_span(UKL_PRIVATE UklRun* r, uint32_t c, uint32_t start, uint32_t stop, uint32_t depth0, uint32_t k, uint32_t w)
{{
    UklMidPool pool = r->m;
    uint32_t span = stop - start;
    {p}Sh d;
    d.toks = r->toks;
    d.ntoks = r->ntoks;
    d.n = 0u;
    d.own = r->f1 + UKL_MID_FRAME * 2u * (start + c);
    d.nown = 0u;
    d.most_own = 2u * span + 2u;
    d.touch = r->touch + 2u * (start + c);
    d.ntouch = 0u;
    d.most_touch = span + 1u;
    d.closed = 0u;
    d.last_start = 0u;
    d.has_last = false;
    d.full = false;
    {p}Stop x = {p}stop_err(r->starts[start]);
    bool walk = k != UKL_MID_NONE && depth0 <= {p}DEPTH;
    uint32_t st = 0u, known = 0u;
    UKL_GLOBAL uint8_t* ret = r->rets + c * {p}DEPTH;
    if (walk) {{
        uint32_t nk = pool[k], anti = pool[w];
        uint32_t inner = pool[k + nk];
        known = anti < nk - 1u ? anti : nk - 1u;
        known = known < depth0 ? known : depth0;
        if (start != 0u) {{
            walk = inner != {p}ROOT;
            st = walk ? {p}cycle(inner) : 0u;
        }}
        for (uint32_t j = 0u; j < known; j++)
            ret[depth0 - 1u - j] = (uint8_t){p}cont(pool[k + 1u + nk - 2u - j]);
    }}
    if (walk) {{
        {p}Lx lx;
        lx.starts = r->starts;
        lx.count = r->count;
        lx.len = r->len;
        {p}Entry en;
        en.i = start;
        en.e = 0u;
        en.depth = depth0;
        en.st = st;
        x = {p}run_range_Sh(r->src, lx, &d, en, ret, depth0 - known, stop);
        // A level its word does not close: the walk closes a level only at a closer.
        if (x.kind == 1u) {{
            x = {p}stop_err(r->starts[start]);
            d.n = 0u;
            d.nown = 0u;
            d.ntouch = 0u;
            d.closed = 0u;
            d.has_last = false;
        }}
    }}
    return d.full ? UKL_MID_FULL : {p}span_of(pool, x, &d);
}}

UKL_INLINE uint32_t ukl_run_bracket(uint32_t b) {{ return {p}bracket(b); }}
UKL_INLINE bool ukl_run_sep(uint32_t b) {{ return {seps}; }}
UKL_INLINE uint32_t ukl_run_root(void) {{ return {p}ROOT; }}
UKL_INLINE uint32_t ukl_run_cont(uint32_t k) {{ return {p}cont(k); }}

"#,
        firsts = tests.join(" || "),
    );
    tier2_kernels(&mut out, p);
    // The phases' block operators (decision 0726 step 4), printed from the Rust the CPU's phases run.
    let mut ph = names.clone();
    ph.insert("bracket_byte".into(), format!("{p}bracket"));
    ph.insert("NONE".into(), format!("{p}NONE"));
    ph.insert("D0".into(), format!("{p}D0"));
    ph.insert("KEYS".into(), format!("{p}KEYS"));
    for f in PHASE_FNS {
        ph.insert((*f).into(), format!("{p}{f}"));
    }
    let _ = writeln!(out, "#define {p}D0 {}u\n#define {p}KEYS 256u", 1u32 << 30);
    for f in PHASE_FNS {
        print_fn(&lines, f, &format!("{p}{f}"), "", p, &ph, &returns, &mut out)?;
    }
    // The program's (`<ukl/t2_phases.h>`) names for them: the merges and units by op, each phase's operators.
    let _ = write!(
        out,
        r#"UKL_INLINE void ukl_ph_merge(uint32_t op, UKL_COHERENT const uint32_t* a, UKL_COHERENT const uint32_t* b, UKL_COHERENT uint32_t* out)
{{
    if (op == UKL_T2_OP_P1)
        {p}p1_merge(a, b, out);
    else if (op == UKL_T2_OP_ADD1)
        {p}add1_merge(a, b, out);
    else if (op == UKL_T2_OP_KEYS)
        {p}p2_merge(a, b, out);
}}
UKL_INLINE void ukl_ph_unit(uint32_t op, UKL_COHERENT uint32_t* out)
{{
    if (op == UKL_T2_OP_P1)
        {p}p1_unit(out);
    else if (op == UKL_T2_OP_ADD1)
        {p}add1_unit(out);
    else if (op == UKL_T2_OP_KEYS)
        {p}p2_unit(out);
}}
UKL_INLINE void ukl_t2_p1_run(UKL_GLOBAL const uint8_t* src, UKL_GLOBAL const uint32_t* starts, uint32_t n, uint32_t a, uint32_t b, UKL_COHERENT uint32_t* out) {{ {p}p1_run(src, starts, n, a, b, out); }}
UKL_INLINE void ukl_t2_p2_run(UKL_GLOBAL const uint32_t* items, uint32_t a, uint32_t b, uint32_t shift, UKL_COHERENT uint32_t* out) {{ {p}p2_run(items, a, b, shift, out); }}
UKL_INLINE void ukl_t2_p2_write(UKL_GLOBAL const uint32_t* items, uint32_t a, uint32_t b, uint32_t shift, UKL_COHERENT uint32_t* sum, UKL_GLOBAL const uint32_t* keys, UKL_GLOBAL uint32_t* out) {{ {p}p2_write(items, a, b, shift, sum, keys, out); }}
UKL_INLINE void ukl_t2_p1_write(UKL_GLOBAL const uint8_t* src, UKL_GLOBAL const uint32_t* starts, uint32_t n, uint32_t a, uint32_t b, UKL_COHERENT const uint32_t* sum, UKL_GLOBAL uint32_t* depth, UKL_GLOBAL uint32_t* items) {{ {p}p1_write(src, starts, n, a, b, sum, depth, items); }}
"#
    );
    Ok(out)
}

/// The phases' block operators the printer prints, in order (each after the ones it calls).
const PHASE_FNS: &[&str] = &["bracket_code", "empty_close_at", "p1_unit", "p1_run", "p1_merge", "p1_write", "add1_unit", "add1_merge", "p2_unit", "p2_merge", "p2_run", "p2_write"];

/// Tier 2's two programs as UKL kernels, launched a phase at a time in order (decision 0726). `<ukl/middle_run.h>`'s
/// phases are three kernels, each its phases a constant mask, so it holds their code alone (its compile and its
/// registers its own): `{p}run_walk` pass 1's walks, `{p}run_write` pass 2's (its count and its writes, one walk),
/// `{p}run_scan` the rest. Compiled cold on the M3 Max (2026-11-27), a kernel a phase took 2 s each but pass 2's two,
/// 19 and 20 s. `<ukl/z16_run.h>`'s are `{p}z_<phase>`.
fn tier2_kernels(out: &mut String, p: &str) {
    let _ = write!(
        out,
        r#"
typedef struct {p}RArgs {{
    uint32_t count;
    uint32_t len;
    uint32_t ntoks;
    uint32_t size;
    uint32_t runs;
    uint32_t most;
    uint32_t phase;
    uint32_t pad;
}} {p}RArgs;
UKL_REFLECT_STRUCT({p}RArgs, count, len, ntoks, size, runs, most, phase, pad)

// The 16z program's kernels take nothing but their buffers.
typedef struct {p}ZArgs {{
    uint32_t pad;
    uint32_t pad2;
}} {p}ZArgs;
UKL_REFLECT_STRUCT({p}ZArgs, pad, pad2)
"#
    );
    // Each kernel's phases (`UKL_RUN_PH_*`'s bits).
    let walk = 1u32 << 4;
    let write = (1u32 << 6) | (1 << 7);
    for (name, mask) in [("scan", ((1u32 << 11) - 1) & !walk & !write), ("walk", walk), ("write", write)] {
        let _ = write!(
            out,
            r#"
// Tier 2 from the middle (`<ukl/middle_run.h>`), its {name} phases (the mask {mask:#x}), the phase `args.phase`: its
// SIMD groups take the phase's items by ticket until none is left. The pool holds every value an item hands on (its header the host's layout); `t` is the writer's
// region, its state at 0, room reserved for the tape and its tail.
DEFINE_KERNEL2(tier2, {p}run_{name},
               ARGS,     {p}RArgs, args,    ,
               ROBUFFER, uint8_t,  src,     "[len + 64]",
               ROBUFFER, uint32_t, starts,  "[count + 3]",
               ROBUFFER, uint32_t, toks,    "[ntoks * 5]",
               RWBUFFER, uint32_t, pool,    "[pool]",
               RWBUFFER, uint8_t,  t,       "[region]",
               RWBUFFER, uint8_t,  rets,    "[runs * {p}DEPTH]",
               RWBUFFER, uint32_t, f1,      "[(count + 1 + runs) * 10]",
               RWBUFFER, uint32_t, touch,   "[(count + 1 + runs) * 2]",
               RWBUFFER, uint32_t, f2,      "[(count + 1 + runs) * 25]",
               ROBUFFER, uint32_t, lexed,   "[32]",
               RWBUFFER, uint32_t, meet,    "[5 * tn]",
               LANE,     uint16_t, lane,    )
{{
    UklRun r;
    r.src = src;
    r.starts = starts;
    r.toks = toks;
    r.m = (UklMidPool)pool;
    r.t = t;
    r.rets = rets;
    r.f1 = f1;
    r.touch = touch;
    r.f2 = f2;
    r.meet = meet;
    r.phase = args.phase;
    // The records tier 1 found (`args.count` and `args.runs` bound them: the buffers' room).
    r.count = lexed[UKL_LEX_COUNT];
    r.refused = lexed[UKL_LEX_REFUSED];
    r.len = args.len;
    r.ntoks = r.count;
    r.size = args.size;
    r.runs = (r.count + args.size - 1u) / args.size;
    r.runs = r.runs == 0u ? 1u : r.runs;
    r.most = args.most;
    ukl_run_worker(&r, {mask}u, lane __UKL_ARGS);
}}
"#
        );
    }
    let _ = write!(
        out,
        r#"
// Tier 2 as phases (`<ukl/t2_phases.h>`), its phase `args.phase`: SIMD groups take the phase's items by ticket until
// none is left. Tier 1's pool says the records (`lexed`); the phases' pool holds the host's layout words, the trees and
// the arrays the phases write.
typedef struct {p}PArgs {{
    uint32_t phase;
    uint32_t pad;
}} {p}PArgs;
UKL_REFLECT_STRUCT({p}PArgs, phase, pad)
DEFINE_KERNEL2(tier2, {p}phases,
               ARGS,     {p}PArgs, args,    ,
               ROBUFFER, uint8_t,  src,     "[len + 64]",
               ROBUFFER, uint32_t, starts,  "[count + 3]",
               ROBUFFER, uint32_t, lexed,   "[32]",
               RWBUFFER, uint32_t, pool,    "[pool]",
               RWBUFFER, uint32_t, meet,    "[trees * ms]",
               RWBUFFER, uint32_t, arrays,  "[arrays]",
               LANE,     uint16_t, lane,    )
{{
    UklPh r;
    r.m = (UklPhPool)pool;
    r.meet = meet;
    r.phase = args.phase;
    r.src = src;
    r.starts = starts;
    r.arrays = arrays;
    r.n = lexed[UKL_LEX_COUNT];
    r.refused = lexed[UKL_LEX_REFUSED] != 0u || lexed[UKL_LEX_SHORT] != 0u;
    ukl_ph_worker(&r, lane __UKL_ARGS);
}}
"#
    );
    for (ph, name) in ["count", "write"].iter().enumerate() {
        let _ = write!(
            out,
            r#"
// The finished tape stored as 16z where it lies (`<ukl/z16_run.h>`), its phase {name}: SIMD groups, a group a
// threadgroup, each its LZ4 table in threadgroup memory, taking the phase's items by ticket.
DEFINE_KERNEL2(tier2, {p}z_{name},
               ARGS,     {p}ZArgs, args,    ,
               ROBUFFER, uint8_t,  t,       "[region]",
               RWBUFFER, uint8_t,  z,       "[zroom]",
               RWBUFFER, uint32_t, pool,    "[pool]",
               SHARED,   uint32_t, table,   UKL_LZ4E_TABLE,
               LANE,     uint16_t, lane,    )
{{
    UklZ16 r;
    // The tape: the writer's region's OUT part, where the chain finished it.
    r.t = t + ukl_z16_u32(t, ukl_w_part_field(CLIF_W_PART_OUT, CLIF_W_AT_P_AT));
    r.z = z;
    r.m = (UklZ16Pool)pool;
    r.phase = {ph}u;
    r.table = table;
    ukl_z16_worker(&r, lane __UKL_ARGS);
}}
"#
        );
    }
}

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
enum Ty {
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
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Id(String),
    Num(String),
    Str,
    P(&'static str),
}

const PUNCT: &[&str] = &[
    "<<=", ">>=", "..", "::", "<<", ">>", "<=", ">=", "==", "!=", "&&", "||", "+=", "-=", "|=", "&=", "^=", "*=", "(", ")", "[", "]", "{", "}", ",", ".", ";", ":", "&", "|",
    "^", "!", "+", "-", "*", "/", "%", "<", ">", "=",
];

fn lex(s: &str) -> Result<Vec<Tok>, String> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    'next: while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
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
enum E {
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
}

struct Parser {
    t: Vec<Tok>,
    i: usize,
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
    fn expr(&mut self, min: u8) -> Result<E, String> {
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
                let k = self.expr(0)?;
                self.eat("]")?;
                a = E::Index(Box::new(a), Box::new(k));
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
fn parse(s: &str) -> Result<E, String> {
    let mut p = Parser { t: lex(s)?, i: 0 };
    let e = p.expr(0)?;
    if p.i != p.t.len() {
        return Err(format!("`{s}`: more after its expression ({:?})", p.peek()));
    }
    Ok(e)
}

/// A function's names, types and constants, as it is printed.
struct Fx<'a> {
    p: &'a str,
    /// The tier's names: functions, tables, constants, with their prefix.
    names: &'a HashMap<String, String>,
    /// The tier's functions' return types.
    returns: &'a HashMap<String, Ty>,
    /// The scopes' names and types, innermost last.
    scopes: Vec<Vec<(String, Ty)>>,
    /// The function's own constants, their values and types.
    consts: HashMap<String, (String, Ty)>,
}

impl Fx<'_> {
    fn var(&self, n: &str) -> Option<&Ty> {
        self.scopes.iter().rev().flat_map(|s| s.iter().rev()).find(|(k, _)| k == n).map(|(_, t)| t)
    }
    fn in_this_scope(&self, n: &str) -> bool {
        self.scopes.last().is_some_and(|s| s.iter().any(|(k, _)| k == n))
    }
    fn declare(&mut self, n: &str, t: Ty) {
        self.scopes.last_mut().expect("a scope").push((n.into(), t));
    }
    fn name(&self, n: &str) -> String {
        self.names.get(n).cloned().unwrap_or_else(|| n.into())
    }

    fn ty(&self, e: &E) -> Ty {
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
                E::Path(..) => self.ty(f),
                _ => Ty::U32,
            },
            E::Method(a, m, _) => match m.as_str() {
                "count_ones" | "trailing_zeros" => Ty::U32,
                "is_some" | "is_none" | "starts_with" => Ty::Bool,
                _ => self.ty(a),
            },
            E::Field(_, _) => Ty::U64,
            E::Index(a, _) => match self.ty(a) {
                Ty::Arr(t) => *t,
                Ty::Ptr(c) if c.contains("uint64_t") => Ty::U64,
                _ => Ty::U32,
            },
            E::Array(_) => Ty::U32,
            E::If(_, a, _) => self.ty(a),
        }
    }

    fn c(&self, e: &E) -> Result<String, String> {
        Ok(match e {
            E::Num(n) => {
                let d: String = n.replace('_', "");
                let digits = ["u64", "usize", "u32", "u8"].iter().fold(d.as_str(), |d, s| d.strip_suffix(s).unwrap_or(d)).to_string();
                if self.ty(e) == Ty::U64 { format!("{digits}ull") } else { format!("{digits}u") }
            }
            E::Id(n) => match self.consts.get(n) {
                Some((v, _)) => v.clone(),
                None => self.name(n),
            },
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
                _ => format!("(&{})", self.c(a)?),
            },
            E::Un(op, a) => format!("({op}{})", self.c(a)?),
            E::Bin(op, a, b) => format!("({} {op} {})", self.c(a)?, self.c(b)?),
            E::Cast(a, t) => format!("(({})({}))", cast_ty(t).c(self.p), self.c(a)?),
            E::Call(f, args) => {
                // `s::load` of a table: the table's.
                let f = match &**f {
                    E::Path(m, n) if m == "s" && n == "load" && matches!(args.first(), Some(E::Un("&", _))) => "ukl_b64_load_t".to_string(),
                    _ => self.c(f)?,
                };
                let args = args.iter().map(|a| self.c(a)).collect::<Result<Vec<_>, _>>()?;
                format!("{f}({})", args.join(", "))
            }
            E::Method(a, m, args) => {
                let x = self.c(a)?;
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

fn cast_ty(t: &str) -> Ty {
    match t {
        "u64" => Ty::U64,
        "bool" => Ty::Bool,
        _ => Ty::U32,
    }
}

/// A parameter's type, from its Rust type.
fn param_ty(t: &str) -> Result<Ty, String> {
    Ok(match t {
        "u64" => Ty::U64,
        "u32" | "usize" => Ty::U32,
        "bool" => Ty::Bool,
        "Carries" => Ty::Carries,
        "&[u8]" => Ty::Ptr("UKL_GLOBAL const uint8_t*".into()),
        "*mut u32" => Ty::Ptr("UKL_GLOBAL uint32_t*".into()),
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
}

/// Function `name`'s body (`lines`, after its signature line, to its closing brace), printed.
#[allow(clippy::too_many_lines)]
fn body(fx: &mut Fx<'_>, lines: &[&str], ret: &str, out: &mut String) -> Result<(), String> {
    let mut frames: Vec<Frame> = vec![Frame::Plain];
    let lanes = fx.name("LANES");
    for raw in lines {
        let l = raw.trim();
        let why = |e: String| format!("tier 1 as UKL: `{l}`: {e}");
        if l.is_empty() {
            continue;
        }
        if l.starts_with("//") {
            let _ = writeln!(out, "{l}");
            continue;
        }
        // The block's ends.
        if l == "}" || l == "};" || l == "})" {
            let f = frames.pop().ok_or_else(|| why("a brace too many".into()))?;
            if !matches!(f, Frame::Fields(..)) {
                fx.scopes.pop();
            }
            match f {
                Frame::Fields(_, after) => {
                    let _ = writeln!(out, "{after}");
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
            frames.push(Frame::Plain);
            fx.scopes.push(vec![(v.into(), Ty::U32)]);
            continue;
        }
        if let Some(r) = l.strip_prefix("while let Some(").and_then(|r| r.strip_suffix(" {")) {
            let (v, x) = r.split_once(") = ").ok_or_else(|| why("a `while let`".into()))?;
            let x = fx.c(&parse(x).map_err(why)?).map_err(why)?;
            let _ = writeln!(out, "while ({x} != UKL_NONE) {{\nuint32_t {v} = {x};");
            frames.push(Frame::Plain);
            fx.scopes.push(vec![(v.into(), Ty::U32)]);
            continue;
        }
        if let Some(c) = l.strip_prefix("while ").and_then(|r| r.strip_suffix(" {")) {
            let _ = writeln!(out, "while ({}) {{", fx.c(&parse(c).map_err(why)?).map_err(why)?);
            frames.push(Frame::Plain);
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
        if let Some(s) = s.strip_suffix(';') {
            let toks = lex(s).map_err(why)?;
            let at = toks.iter().position(|t| matches!(t, Tok::P(p) if ["=", "+=", "-=", "|=", "&=", "^=", "<<=", ">>=", "*="].contains(p)));
            match at {
                Some(k) => {
                    let Tok::P(op) = toks[k] else { unreachable!("an assignment") };
                    let mut p = Parser { t: toks[..k].to_vec(), i: 0 };
                    let lhs = p.expr(0).map_err(why)?;
                    let mut q = Parser { t: toks[k + 1..].to_vec(), i: 0 };
                    let rhs = q.expr(0).map_err(why)?;
                    if p.i != p.t.len() || q.i != q.t.len() {
                        return Err(why("an assignment the printer does not take".into()));
                    }
                    let _ = writeln!(out, "{} {op} {};", fx.c(&lhs).map_err(why)?, fx.c(&rhs).map_err(why)?);
                }
                None => {
                    let e = parse(s).map_err(why)?;
                    let _ = writeln!(out, "{};", fx.c(&e).map_err(why)?);
                }
            }
            continue;
        }
        // A block's value: its variable's.
        if let Some(Frame::Value(v)) = frames.last() {
            let e = parse(l).map_err(why)?;
            let _ = writeln!(out, "{v} = {};", fx.c(&e).map_err(why)?);
            continue;
        }
        return Err(why("a line the printer does not take".into()));
    }
    if frames.len() != 1 {
        return Err(format!("tier 1 as UKL: {} blocks left open", frames.len() - 1));
    }
    Ok(())
}

/// The text of the item that starts at line `at` (its signature) to its closing brace at the line's start.
fn item<'a, 'b>(lines: &'b [&'a str], at: usize) -> Result<&'b [&'a str], String> {
    let end = (at + 1..lines.len()).find(|&k| lines[k] == "}").ok_or("an item without its end")?;
    Ok(&lines[at + 1..end])
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
        "// Generated by native/crates/compiler/src/bitlex_lex_rust/ukl.rs from tier 1's Rust; do not edit. Tier 1's passes as UKL,\n// a thread a chunk (decision 0726).\n\n#pragma once\n#include <ukl/blx64.h>\n#include <ukl/memory.h>\n"
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
    let wanted = ["lane_carries", "write_exact", "write_numbers", "stage1_ends", "stage1_from"];
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
        let mut fx = Fx { p, names: &names, returns: &returns, scopes: vec![params], consts: HashMap::new() };
        let mut text = String::new();
        body(&mut fx, item(&lines, at)?, kind, &mut text)?;
        out.push_str(&text);
        out.push_str("}\n\n");
    }
    // The kernels.
    let c = format!("{p}Carries");
    let put = fields.iter().enumerate().map(|(k, f)| format!("        ends[(c * {p}LANES + l) * {n}u + {k}u] = e[l].{f};", n = fields.len())).collect::<Vec<_>>().join("\n");
    let get = fields.iter().enumerate().map(|(k, f)| format!("    e.{f} = entries[c * {n}u + {k}u];", n = fields.len())).collect::<Vec<_>>().join("\n");
    let _ = write!(
        out,
        r#"typedef struct {p}Args {{
    uint32_t n;
    uint32_t blocks;
    uint32_t chunks;
    uint32_t write;
}} {p}Args;
UKL_REFLECT_STRUCT({p}Args, n, blocks, chunks, write)

// Pass 1: chunk `c`'s end from each lane's entry ({lanes} lanes, each its carries' words in turn).
DEFINE_KERNEL2(tier1, {p}ends,
               ARGS,     {p}Args, args,    ,
               ROBUFFER, uint8_t,  src,     "[n + 64]",
               RWBUFFER, uint64_t, ends,    "[chunks * {lanes} * {nf}]",
               GID0,     uint32_t, c,       )
{{
    if (c >= args.chunks)
        return;
    {c} e[{p}LANES];
    {p}stage1_ends(src, args.n, c * args.blocks, (c + 1u) * args.blocks, e);
    for (uint32_t l = 0u; l < {p}LANES; l++) {{
{put}
    }}
}}

// Pass 2 of chunk `c` from its entry (`entries`): counting (`args.write` 0: its records and each type's, at `at`), or
// writing each record at its index (`at`'s first) and each type's records' numbers in `types` (from `at`'s others:
// each type's list's start in `types` and the type's records before the chunk). `refused`: whether it refuses.
DEFINE_KERNEL2(tier1, {p}from,
               ARGS,     {p}Args, args,    ,
               ROBUFFER, uint8_t,  src,     "[n + 64]",
               ROBUFFER, uint64_t, entries, "[chunks * {nf}]",
               RWBUFFER, uint32_t, at,      "[chunks * ({types} + 1)]",
               RWBUFFER, uint32_t, starts,  "[records]",
               RWBUFFER, uint32_t, types,   "[records]",
               RWBUFFER, uint32_t, refused, "[chunks]",
               GID0,     uint32_t, c,       )
{{
    if (c >= args.chunks)
        return;
    {c} e;
{get}
    uint32_t a[{p}TYPES + 1u];
    for (uint32_t k = 0u; k < {p}TYPES + 1u; k++)
        a[k] = args.write != 0u ? at[c * ({p}TYPES + 1u) + k] : 0u;
    {c} end;
    bool r = {p}stage1_from(src, args.n, c * args.blocks, (c + 1u) * args.blocks, e, a, starts, types, args.write != 0u, &end);
    if (args.write == 0u)
        for (uint32_t k = 0u; k < {p}TYPES + 1u; k++)
            at[c * ({p}TYPES + 1u) + k] = a[k];
    refused[c] = r ? 1u : 0u;
}}
"#,
        nf = fields.len(),
    );
    Ok(out)
}

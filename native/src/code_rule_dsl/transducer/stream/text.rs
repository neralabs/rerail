//! The stream program's text form, in the manner of `packed::rir`: one instruction per line, no
//! addresses, a content digest in the header, so two dumps diff cleanly. `parse(print(p))` is
//! `p`, and parsing builds through the verifier, so a hand-written program cannot bypass it.
//!
//! ```text
//! stream program sha256:<digest of the lines below>
//! table 0 lo=10000000000000000000000000000000 hi=00100000000000000000000000000000
//! %0 = class t0 0x01            ; '\'
//! %1 = prev %0, 1
//! %2 = const 0xaaaaaaaaaaaaaaaa
//! %3 = or %1, %2
//! %4 = sub %3, %0
//! %5 = rounds 4 settle
//!   %6 = feed %4
//!   %7 = toggle %6
//! %8 = end %6 <- %7
//! output escaped %4
//! ```
//!
//! A header of `stream program -` takes no digest (a hand-written program). Text after `;` is a
//! comment. Any mnemonic not listed here is refused: there is no way to write a loop, a branch
//! or a call.

use super::program::{ClassTable, Op, Program, Value};
use std::fmt::Write as _;

/// The header's first words.
const HEADER: &str = "stream program";

/// Print a program, digest first.
pub(in crate::code_rule_dsl) fn print(p: &Program) -> String {
    format!("{HEADER} sha256:{}\n{}", hex(&p.digest()), print_body(p))
}

/// The lines after the header: what the digest covers.
pub(in crate::code_rule_dsl) fn print_body(p: &Program) -> String {
    let mut out = String::new();
    for (i, t) in p.tables().iter().enumerate() {
        writeln!(out, "table {i} lo={} hi={}", hex(&t.lo), hex(&t.hi)).unwrap();
    }
    let mut depth = 0usize;
    for (i, op) in p.insts().iter().enumerate() {
        if let Op::End(_) = op {
            depth -= 1;
        }
        let pad = "  ".repeat(depth);
        writeln!(out, "{pad}%{i} = {}", print_op(op)).unwrap();
        if let Op::Rounds { .. } = op {
            depth += 1;
        }
    }
    for (name, v) in p.outputs() {
        writeln!(out, "output {name} %{v}").unwrap();
    }
    out
}

fn print_op(op: &Op) -> String {
    let v = |x: &Value| format!("%{x}");
    let two = |name: &str, a: &Value, b: &Value| format!("{name} {}, {}", v(a), v(b));
    match op {
        Op::Starts => "starts".into(),
        Op::Const(c) => format!("const {c:#018x}"),
        Op::Class { table, bits } => format!("class t{table} {bits:#04x}"),
        Op::Not(a) => format!("not {}", v(a)),
        Op::And(a, b) => two("and", a, b),
        Op::Or(a, b) => two("or", a, b),
        Op::Xor(a, b) => two("xor", a, b),
        Op::AndNot(a, b) => two("andnot", a, b),
        Op::Select(c, a, b) => format!("select {}, {}, {}", v(c), v(a), v(b)),
        Op::Prev(a, k) => format!("prev {}, {k}", v(a)),
        Op::Next(a, k) => format!("next {}, {k}", v(a)),
        Op::Add(a, b) => two("add", a, b),
        Op::Sub(a, b) => two("sub", a, b),
        Op::AddBack(a, b) => two("add_back", a, b),
        Op::Toggle(a) => format!("toggle {}", v(a)),
        Op::ScanMod { n, up, down, reset } => {
            format!("scan_mod {n} {}, {}, {}", v(up), v(down), v(reset))
        }
        Op::ScanClamp { max, up, down, reset } => {
            format!("scan_clamp {max} {}, {}, {}", v(up), v(down), v(reset))
        }
        Op::Plane(a, p) => format!("plane {}, {p}", v(a)),
        Op::PlaneEq(a, c) => format!("plane_eq {}, {c}", v(a)),
        Op::Count(a) => format!("count {}", v(a)),
        Op::Rounds { n, settle } => format!("rounds {n}{}", if *settle { " settle" } else { "" }),
        Op::Feed(a) => format!("feed {}", v(a)),
        Op::End(feeds) => {
            let fs: Vec<String> = feeds.iter().map(|(f, x)| format!("{} <- {}", v(f), v(x))).collect();
            format!("end {}", fs.join(", ")).trim_end().into()
        }
    }
}

/// Why text did not parse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum ParseError {
    /// Line `n` (from 1) is not a table, an instruction or an output.
    Line(usize, String),
    /// The header is missing or malformed.
    Header,
    /// The header's digest is not the program's.
    Digest,
    /// The program does not verify.
    Refused(super::verify::Refusal),
}

/// Parse a printed program, through the verifier.
pub(in crate::code_rule_dsl) fn parse(text: &str) -> Result<Program, ParseError> {
    let mut lines = text.lines();
    let header = lines.next().ok_or(ParseError::Header)?;
    let digest = match header.strip_prefix(HEADER).map(str::trim) {
        Some("-") => None,
        Some(d) => Some(d.strip_prefix("sha256:").ok_or(ParseError::Header)?.to_owned()),
        None => return Err(ParseError::Header),
    };
    let mut tables = vec![];
    let mut insts = vec![];
    let mut outputs = vec![];
    for (n, raw) in lines.enumerate() {
        let line = raw.split(';').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let bad = || ParseError::Line(n + 2, raw.to_owned());
        if let Some(rest) = line.strip_prefix("table ") {
            let t = parse_table(rest, tables.len()).ok_or_else(bad)?;
            tables.push(t);
        } else if let Some(rest) = line.strip_prefix("output ") {
            let (name, v) = rest.split_once(' ').ok_or_else(bad)?;
            outputs.push((name.to_owned(), value(v.trim()).ok_or_else(bad)?));
        } else {
            let (lhs, rhs) = line.split_once('=').ok_or_else(bad)?;
            if value(lhs.trim()) != Some(insts.len() as Value) {
                return Err(bad());
            }
            insts.push(parse_op(rhs.trim()).ok_or_else(bad)?);
        }
    }
    let program = Program::new(tables, insts, outputs).map_err(ParseError::Refused)?;
    if let Some(d) = digest {
        if d != hex(&program.digest()) {
            return Err(ParseError::Digest);
        }
    }
    Ok(program)
}

fn parse_table(rest: &str, index: usize) -> Option<ClassTable> {
    let mut words = rest.split_whitespace();
    if words.next()?.parse::<usize>().ok()? != index {
        return None;
    }
    let lo = unhex16(words.next()?.strip_prefix("lo=")?)?;
    let hi = unhex16(words.next()?.strip_prefix("hi=")?)?;
    words.next().is_none().then_some(ClassTable { lo, hi })
}

fn parse_op(rhs: &str) -> Option<Op> {
    let (name, args) = rhs.split_once(' ').unwrap_or((rhs, ""));
    let args: Vec<&str> = args.split(',').map(str::trim).filter(|a| !a.is_empty()).collect();
    let v = |i: usize| args.get(i).and_then(|a| value(a));
    let num = |s: &str| s.parse::<u8>().ok();
    // `scan_mod 3 %a, %b, %c`: a parameter, then three values.
    let param_then = |want: usize| -> Option<(u8, Vec<Value>)> {
        let (p, first) = args.first()?.split_once(' ')?;
        let mut vs = vec![value(first.trim())?];
        for a in &args[1..] {
            vs.push(value(a)?);
        }
        (vs.len() == want).then_some((num(p)?, vs))
    };
    let arity = |n: usize| args.len() == n;
    Some(match name {
        "starts" if arity(0) => Op::Starts,
        "const" if arity(1) => Op::Const(u64::from_str_radix(args[0].strip_prefix("0x")?, 16).ok()?),
        "class" if arity(1) => {
            let (t, b) = args[0].split_once(' ')?;
            let table = t.strip_prefix('t')?.parse().ok()?;
            let bits = u8::from_str_radix(b.trim().strip_prefix("0x")?, 16).ok()?;
            Op::Class { table, bits }
        }
        "not" if arity(1) => Op::Not(v(0)?),
        "and" if arity(2) => Op::And(v(0)?, v(1)?),
        "or" if arity(2) => Op::Or(v(0)?, v(1)?),
        "xor" if arity(2) => Op::Xor(v(0)?, v(1)?),
        "andnot" if arity(2) => Op::AndNot(v(0)?, v(1)?),
        "select" if arity(3) => Op::Select(v(0)?, v(1)?, v(2)?),
        "prev" if arity(2) => Op::Prev(v(0)?, num(args[1])?),
        "next" if arity(2) => Op::Next(v(0)?, num(args[1])?),
        "add" if arity(2) => Op::Add(v(0)?, v(1)?),
        "sub" if arity(2) => Op::Sub(v(0)?, v(1)?),
        "add_back" if arity(2) => Op::AddBack(v(0)?, v(1)?),
        "toggle" if arity(1) => Op::Toggle(v(0)?),
        "scan_mod" => {
            let (n, vs) = param_then(3)?;
            Op::ScanMod { n, up: vs[0], down: vs[1], reset: vs[2] }
        }
        "scan_clamp" => {
            let (max, vs) = param_then(3)?;
            Op::ScanClamp { max, up: vs[0], down: vs[1], reset: vs[2] }
        }
        "plane" if arity(2) => Op::Plane(v(0)?, num(args[1])?),
        "plane_eq" if arity(2) => Op::PlaneEq(v(0)?, num(args[1])?),
        "count" if arity(1) => Op::Count(v(0)?),
        "rounds" => {
            let mut w = rhs.split_whitespace().skip(1);
            let n = num(w.next()?)?;
            let settle = match w.next() {
                None => false,
                Some("settle") => true,
                Some(_) => return None,
            };
            if w.next().is_some() {
                return None;
            }
            Op::Rounds { n, settle }
        }
        "feed" if arity(1) => Op::Feed(v(0)?),
        "end" => {
            let mut feeds = vec![];
            for a in &args {
                let (f, x) = a.split_once("<-")?;
                feeds.push((value(f.trim())?, value(x.trim())?));
            }
            Op::End(feeds)
        }
        _ => return None,
    })
}

fn value(s: &str) -> Option<Value> {
    s.strip_prefix('%')?.parse().ok()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex16(s: &str) -> Option<[u8; 16]> {
    if s.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

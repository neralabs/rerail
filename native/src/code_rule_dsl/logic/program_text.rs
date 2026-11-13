//! A `front::Program` as `.rr` text, and that text read back as the same program (decisions 0155, 0162).
//!
//! This is how a run keeps what it compiled (a prepared program, a plan choice, a combined program): the program's
//! own rules as text, read back into the program's parts by one walk over the lean parser's tape image
//! (`from_tape`), with nothing lowered and nothing joined. `rr::print` is another thing: it prints a program as source to lower again.
//!
//! The text, in order:
//!
//! ```text
//! .decl r(c: int, d: sym)            // each declaration, as written
//! .input a, b                        // the inputs, then the outputs, in order
//! .output r
//! r(1, "x").                         // the program's facts, each relation's rows in order
//! .rules
//! r(x, y) :- a(x), !b(y), x < 3.     // the rules in order: an atom, `!` its negation, `@f(…)` a call,
//! n = count : { a(x) }               //   an aggregate as `v = op over by (k, …) sep "s" : { atom }`,
//! nearest(n, a, "set")               //   the nearest-ancestor and `ast` literals as atoms of those names
//! ast(d, n, "matcher", "NAME", v)
//! .about
//! program("a").                      // what has no syntax, as `program_facts` rows of program "a" (`write_about`)
//! gated(14, 2).                      // rule 14's literal 2 is a gated negation (-1: its head)
//! ```
//!
//! Variables the parser reads as words (`top`, `count`, …) and relations named as the literals here (`nearest`,
//! `ast`) are written with a trailing `_`, as is a name that is one of those with `_`s after it, so it reads back.

use super::front::{AggOp, Atom, Literal, Program, Rule, Term, Value};
use super::program_facts::{self, Facts};
use rerail_rr_syntax::syntax::quote;
use crate::content_store::{ContentStore, HashKey, SourceId, StoreResult};
use sha2::{Digest as _, Sha256};
use std::fmt::Write as _;

/// The program the `.about` rows are written as (`program_facts`).
const ABOUT: &str = "a";

pub mod from_tape;

use from_tape::{Parts, REL_WORDS, VAR_WORDS};

fn bad(what: impl std::fmt::Display) -> String {
    format!("program text: {what}")
}

fn ident(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty() && s != "_" && (b[0].is_ascii_alphabetic() || b[0] == b'_') && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_')
}

/// `name` as written: one more `_` when it is a word of `words` with `_`s after it, or none.
fn escape(name: &str, words: &[&str]) -> Result<String, String> {
    if !ident(name) {
        return Err(bad(format!("{name:?} is no identifier")));
    }
    Ok(if words.contains(&name.trim_end_matches('_')) { format!("{name}_") } else { name.to_owned() })
}

fn var(v: &str) -> Result<String, String> {
    escape(v, &VAR_WORDS)
}

fn int(n: i64, out: &mut String) {
    // The lexer reads a number's digits as an `i64`, so the least one is written as a difference.
    if n == i64::MIN {
        out.push_str("(-9223372036854775807 - 1)");
    } else {
        write!(out, "{n}").expect("a string");
    }
}

fn term(t: &Term, out: &mut String) -> Result<(), String> {
    match t {
        Term::Var(v) => out.push_str(&var(v)?),
        Term::Wild => out.push('_'),
        Term::Int(n) => int(*n, out),
        Term::Str(s) => out.push_str(&quote(s)),
        Term::Top => out.push_str("top"),
        Term::Call(f, args) => {
            if !ident(f) {
                return Err(bad(format!("function {f:?} is no identifier")));
            }
            write!(out, "@{f}(").expect("a string");
            terms(args, out)?;
            out.push(')');
        }
    }
    Ok(())
}

fn terms(ts: &[Term], out: &mut String) -> Result<(), String> {
    for (i, t) in ts.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        term(t, out)?;
    }
    Ok(())
}

fn atom(a: &Atom, out: &mut String) -> Result<(), String> {
    if a.neg {
        out.push('!');
    }
    out.push_str(&escape(a.rel.as_str(), &REL_WORDS)?);
    out.push('(');
    terms(&a.args, out)?;
    out.push(')');
    Ok(())
}

fn literal(l: &Literal, out: &mut String) -> Result<(), String> {
    match l {
        Literal::Atom(a) => atom(a, out)?,
        Literal::Cmp { l, op, r } => {
            if !matches!(*op, "=" | "!=" | "<" | "<=" | ">" | ">=") {
                return Err(bad(format!("comparison {op:?}")));
            }
            term(l, out)?;
            write!(out, " {op} ").expect("a string");
            term(r, out)?;
        }
        Literal::Agg { var: v, op, over, atom: a } => {
            write!(out, "{} = ", var(v)?).expect("a string");
            let over = match over {
                Some(o) => var(o)?,
                None => "_".to_owned(),
            };
            match op {
                AggOp::Count if over == "_" => out.push_str("count"),
                AggOp::Count => return Err(bad("a count over a variable")),
                AggOp::Min => write!(out, "min {over}").expect("a string"),
                AggOp::Max => write!(out, "max {over}").expect("a string"),
                AggOp::Sum => write!(out, "sum {over}").expect("a string"),
                AggOp::Product => write!(out, "product {over}").expect("a string"),
                AggOp::Join { by, sep } => {
                    write!(out, "join {over}").expect("a string");
                    if !by.is_empty() {
                        let by = by.iter().map(|k| var(k)).collect::<Result<Vec<_>, _>>()?;
                        write!(out, " by ({})", by.join(", ")).expect("a string");
                    }
                    write!(out, " sep {}", quote(sep)).expect("a string");
                }
            }
            out.push_str(" : { ");
            atom(a, out)?;
            out.push_str(" }");
        }
        Literal::Nearest { n, a, kindset } => write!(out, "nearest({}, {}, {})", var(n)?, var(a)?, quote(kindset)).expect("a string"),
        Literal::Ast { document, node, matcher, captures } => {
            write!(out, "ast({}, {}, {}", var(document)?, var(node)?, quote(matcher)).expect("a string");
            for (name, v) in captures {
                write!(out, ", {}, {}", quote(name), var(v)?).expect("a string");
            }
            out.push(')');
        }
    }
    Ok(())
}

fn value(v: &Value, out: &mut String) {
    match v {
        Value::Int(n) => int(*n, out),
        Value::Str(s) => out.push_str(&quote(s)),
    }
}

/// A fact `rel(values…).` on its own line.
fn fact(rel: &str, row: &[Value], out: &mut String) -> Result<(), String> {
    out.push_str(&escape(rel, &REL_WORDS)?);
    out.push('(');
    for (i, v) in row.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        value(v, out);
    }
    out.push_str(").\n");
    Ok(())
}

/// `prog` as its text. A program with parts the text has no form for (maintenance's `again`, `together` and
/// `shapes`) is refused, not written in part.
pub fn print(prog: &Program) -> Result<String, String> {
    if !prog.again.is_empty() || !prog.together.is_empty() || !prog.shapes.is_empty() {
        return Err(bad("maintenance's parts (again, together, shapes) have no text"));
    }
    let mut out = String::new();
    for (rel, cols) in &prog.decls {
        write!(out, ".decl {}(", escape(rel, &REL_WORDS)?).expect("a string");
        for (i, (c, t)) in cols.iter().enumerate() {
            if !ident(c) || !ident(t) {
                return Err(bad(format!("{rel}'s column {c:?}: {t:?}")));
            }
            write!(out, "{}{c}: {t}", if i > 0 { ", " } else { "" }).expect("a string");
        }
        out.push_str(")\n");
    }
    for (directive, names) in [("input", &prog.inputs), ("output", &prog.outputs)] {
        if !names.is_empty() {
            let names = names.iter().map(|n| escape(n, &REL_WORDS)).collect::<Result<Vec<_>, _>>()?;
            writeln!(out, ".{directive} {}", names.join(", ")).expect("a string");
        }
    }
    for (rel, rows) in &prog.facts {
        for row in rows {
            fact(rel, row, &mut out)?;
        }
    }
    out.push_str(".rules\n");
    let mut gated = Vec::new();
    for (r, rule) in prog.rules.iter().enumerate() {
        if rule.head.gated {
            gated.push((r, -1));
        }
        atom(&rule.head, &mut out)?;
        for (k, l) in rule.body.iter().enumerate() {
            out.push_str(if k == 0 { " :- " } else { ", " });
            literal(l, &mut out)?;
            if let Literal::Atom(a) | Literal::Agg { atom: a, .. } = l
                && a.gated
            {
                gated.push((r, k as i64));
            }
        }
        out.push_str(".\n");
    }
    out.push_str(".about\n");
    let mut about = Facts::new();
    program_facts::write_about(&mut about, ABOUT, prog);
    for (rel, rows) in &about {
        for row in rows {
            fact(rel, row, &mut out)?;
        }
    }
    for (r, k) in gated {
        writeln!(out, "gated({r}, {k}).").expect("a string");
    }
    Ok(out)
}

/// The program `text` (`print`) holds: its tape image (the lean parser's) walked straight into the program's parts.
/// Its lists are its own; a run reads with `read_in`, so that its programs share theirs.
pub fn read(text: &str) -> Result<Program, String> {
    read_in(text, &crate::code_rule_dsl::logic::front::Lists::default())
}

/// `read`, each list taken from the run's `lists` (decision 0172).
pub fn read_in(text: &str, lists: &crate::code_rule_dsl::logic::front::Lists) -> Result<Program, String> {
    assemble(from_tape::parts(text, &rerail_rr_syntax::tape(text), lists)?)
}

/// The program of `parts`: the gated atoms marked, the rest of it read from the `.about` rows.
fn assemble(parts: Parts) -> Result<Program, String> {
    let Parts { decls, inputs, outputs, facts, mut rules, mut about } = parts;
    let gated = about.remove("gated").unwrap_or_default();
    let mut prog = program_facts::program(&about, ABOUT)?;
    for row in gated {
        let [Value::Int(r), Value::Int(k)] = row.as_slice() else { return Err(bad("a gated row")) };
        let rule = rules.get_mut(*r as usize).ok_or_else(|| bad(format!("gated: no rule {r}")))?;
        let atom = match *k {
            -1 => &mut rule.head,
            k => match rule.body.make_mut().get_mut(k as usize) {
                Some(Literal::Atom(a) | Literal::Agg { atom: a, .. }) => a,
                _ => return Err(bad(format!("gated: rule {r} has no atom {k}"))),
            },
        };
        atom.gated = true;
    }
    // The relations of facts `about` named, rows or none, with the rows read.
    for (rel, rows) in facts {
        prog.facts.entry(rel).or_default().extend(rows);
    }
    prog.decls = decls;
    prog.inputs = inputs;
    prog.outputs = outputs;
    prog.rules = rules;
    Ok(prog)
}

/// Whether stored programs are kept and read as images too (decision 0173): `RERAIL_PROGRAM_IMAGE=off` keeps and
/// reads texts only; `require` reads images only, refusing a program kept as text alone (a validation aid).
fn images() -> &'static str {
    match std::env::var("RERAIL_PROGRAM_IMAGE").as_deref() {
        Ok("off") => "off",
        Ok("require") => "require",
        _ => "on",
    }
}

/// The alias the store keeps the image of the program whose text has sha256 `digest` under: the program's identity
/// stays its text's digest, which the rows that name a program hold, and the image's source is its own bytes' sha256,
/// as every source's is. The format is in the key, so another format's image is never found for it.
pub fn image_key(digest: &[u8; 32]) -> HashKey {
    HashKey::sha256(&[b"rerail program image 1\0".as_slice(), digest.as_slice()].concat())
}

/// What the store keeps of a program, as read: its image (decision 0173), or its text where it has no image.
pub enum Stored {
    Image(Vec<u8>),
    Text(String),
}

/// The program `stored` holds, its lists taken from the run's `lists`.
pub fn read_stored(stored: &Stored, lists: &crate::code_rule_dsl::logic::front::Lists) -> Result<Program, String> {
    match stored {
        Stored::Image(bytes) => crate::code_rule_dsl::logic::program_image::decode(bytes, lists),
        Stored::Text(text) => read_in(text, lists),
    }
}

/// Keeps `prog` in `store` as its text, a source by its sha256 (held, so the run keeps it, O95), and as its image,
/// a source by `image_key` (decision 0173): the digest, and the sources written, which the caller names so collection
/// keeps them. `None` when the text does not read back as `prog`: nothing is kept. An image that does not read back as
/// `prog` is not kept; the text is.
pub fn store(store: &ContentStore, prog: &Program) -> StoreResult<Option<([u8; 32], Vec<SourceId>)>> {
    let Ok(text) = print(prog) else { return Ok(None) };
    let want = program_facts::debug_digest(prog);
    if !read(&text).is_ok_and(|back| program_facts::debug_digest(&back) == want) {
        return Ok(None);
    }
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, text.len() as u64);
    let hash = HashKey::sha256(text.as_bytes());
    let source = match store.held_source_by_hash(hash)? {
        Some(id) => id,
        None => store.put_source_hashed(text.as_bytes(), hash, &[])?,
    };
    let digest: [u8; 32] = Sha256::digest(text.as_bytes()).into();
    let mut sources = vec![source];
    if images() != "off" {
        let key = image_key(&digest);
        if let Some(id) = store.source_by_alias(key)? {
            store.pin_held_sources(&[id])?;
            sources.push(id);
        } else if let Ok(bytes) = crate::code_rule_dsl::logic::program_image::encode(prog)
            && crate::code_rule_dsl::logic::program_image::decode(&bytes, &crate::code_rule_dsl::logic::front::Lists::default()).is_ok_and(|back| program_facts::debug_digest(&back) == want)
        {
            crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, bytes.len() as u64);
            // An image the store refuses (its alias taken by other bytes) is not kept; the text is.
            if let Ok(id) = store.put_source(&bytes, &[key]) {
                store.pin_held_sources(&[id])?;
                sources.push(id);
            }
        }
    }
    Ok(Some((digest, sources)))
}

/// What `store` keeps of the programs of sha256 `digests` (`store`), by digest, with their sources, in two reads: for
/// a caller that must read before its run's waves (decision 0104) and parse later. Each is its image where it has
/// one, else its text; one the store has lost is left out.
pub fn texts(store: &ContentStore, digests: &[[u8; 32]]) -> StoreResult<std::collections::HashMap<[u8; 32], (Stored, Vec<SourceId>)>> {
    let mode = images();
    let hashes: Vec<HashKey> = digests.iter().map(|d| HashKey::from_sha256_digest(*d)).collect();
    let text_ids = store.held_sources_by_hash(&hashes)?;
    let mut out = std::collections::HashMap::new();
    let mut want_text = Vec::new();
    for (k, d) in digests.iter().enumerate() {
        let image = if mode == "off" { None } else { store.source_by_alias(image_key(d))? };
        match image {
            Some(id) => {
                store.pin_held_sources(&[id])?;
                let bytes = store.get_source(id)?;
                out.insert(*d, (Stored::Image(bytes), text_ids[k].into_iter().chain([id]).collect()));
            }
            None if mode != "require" => want_text.push(k),
            None => {}
        }
    }
    let requests: Vec<(SourceId, HashKey)> = want_text.iter().filter_map(|&k| Some((text_ids[k]?, hashes[k]))).collect();
    let mut bytes = store.get_sources_by_hash_bulk(&requests, usize::MAX)?;
    for k in want_text {
        let Some(id) = text_ids[k] else { continue };
        let Some(text) = bytes.remove(&id).and_then(|b| String::from_utf8(b).ok()) else { continue };
        out.insert(digests[k], (Stored::Text(text), vec![id]));
    }
    Ok(out)
}

/// The program `store` keeps of sha256 `digest` (`store`), from its image where it has one, else its text, and the
/// sources it is kept as; `None` when the store has lost it, or it no longer reads. Its lists are taken from the
/// run's `lists`.
pub fn load(store: &ContentStore, digest: &[u8; 32], lists: &crate::code_rule_dsl::logic::front::Lists) -> StoreResult<Option<(Program, Vec<SourceId>)>> {
    let mut found = texts(store, &[*digest])?;
    let Some((stored, sources)) = found.remove(digest) else { return Ok(None) };
    Ok(read_stored(&stored, lists).ok().map(|p| (p, sources)))
}

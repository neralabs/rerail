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
        // A lowered program's atom (decision 0270) is the store's: a lowered program is kept as its image.
        Term::Branded(b) => return Err(bad(format!("the atom {b}: no text"))),
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

fn value(v: &Value, out: &mut String) -> Result<(), String> {
    match v {
        Value::Int(n) => int(*n, out),
        Value::Str(s) => out.push_str(&quote(s)),
        Value::Branded(b) => return Err(bad(format!("the atom {b}: no text"))),
    }
    Ok(())
}

/// A fact `rel(values…).` on its own line.
fn fact(rel: &str, row: &[Value], out: &mut String) -> Result<(), String> {
    out.push_str(&escape(rel, &REL_WORDS)?);
    out.push('(');
    for (i, v) in row.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        value(v, out)?;
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
    let format = crate::code_rule_dsl::logic::program_image::FORMAT;
    HashKey::sha256(&[format!("rerail program image {format}\0").as_bytes(), digest.as_slice()].concat())
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

/// The program `stored` holds, as `read_stored` reads it, and its version where it is kept as an image (decision
/// 0244: the parts of a prepared program are made from it).
pub fn read_stored_version(stored: &Stored, lists: &crate::code_rule_dsl::logic::front::Lists) -> Result<(Program, Option<std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>>), String> {
    match stored {
        Stored::Image(bytes) => {
            let v = crate::code_rule_dsl::logic::program_image::Version::open(bytes)?;
            Ok((v.program(lists)?, Some(v)))
        }
        Stored::Text(text) => Ok((read_in(text, lists)?, None)),
    }
}

/// A program as the store keeps it, made where the program is (the run's thread, decision 0306), kept by the store's
/// writer (`store_encoded`): its image (decision 0173), or with `RERAIL_PROGRAM_IMAGE=off` its text, checked to read
/// back as the program. The digest is of its bytes, which is the source's own hash.
pub struct Encoded {
    pub digest: [u8; 32],
    /// Its image's bytes (else its text's).
    pub image: bool,
    pub bytes: Vec<u8>,
}

/// `prog` encoded as the store keeps it (`Encoded`); `None` where its text does not read back as it (nothing is kept).
pub fn encode(prog: &Program) -> Option<Encoded> {
    // The stored form is the image (decision 0246 §6): encoded once, kept under the digest of its bytes. Not printed,
    // read back, encoded and decoded again, and digested by `{:?}` to check each.
    if images() != "off" {
        let bytes = crate::code_rule_dsl::logic::program_image::encode(prog).ok()?;
        crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, bytes.len() as u64);
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        // A run building the engine's bundle (`RERAIL_EXPORT_IMAGES`, decision 0264) ships it.
        crate::code_rule_dsl::logic::rr::images::export(image_key(&digest), &bytes);
        return Some(Encoded { digest, image: true, bytes });
    }
    let text = print(prog).ok()?;
    let want = program_facts::debug_digest(prog);
    if !read(&text).is_ok_and(|back| program_facts::debug_digest(&back) == want) {
        return None;
    }
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, text.len() as u64);
    Some(Encoded { digest: Sha256::digest(text.as_bytes()).into(), image: false, bytes: text.into_bytes() })
}

/// Programs `encode` made, and a facts entry after them, as a record's item carries them (decision 0306): their
/// digests and kinds in its body, their bytes, then the entry's, in its byte area.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Packed {
    programs: Vec<([u8; 32], bool, usize)>,
}

/// Packs `programs` and `entry` (`Packed`): the body and the byte area.
pub fn pack(programs: Vec<Encoded>, entry: Vec<u8>) -> (Packed, Vec<u8>) {
    let mut bytes = Vec::with_capacity(programs.iter().map(|p| p.bytes.len()).sum::<usize>() + entry.len());
    let mut packed = Packed { programs: Vec::with_capacity(programs.len()) };
    for p in programs {
        packed.programs.push((p.digest, p.image, p.bytes.len()));
        bytes.extend_from_slice(&p.bytes);
    }
    bytes.extend_from_slice(&entry);
    (packed, bytes)
}

/// Keeps the programs `pack` packed (`store_encoded`): the sources written, and the entry's bytes.
pub fn store_packed<'b>(store: &ContentStore, packed: &Packed, bytes: &'b [u8]) -> StoreResult<(Vec<SourceId>, &'b [u8])> {
    let (mut at, mut sources) = (0, Vec::new());
    for (digest, image, len) in &packed.programs {
        let program = bytes.get(at..at + len).ok_or_else(|| crate::content_store::StoreError::Invalid("a record's program: past its bytes".into()))?;
        sources.extend(store_encoded(store, digest, *image, program)?);
        at += len;
    }
    Ok((sources, &bytes[at..]))
}

/// Keeps a program `encode` made in `store`, a source by its bytes' sha256 (an image by `image_key` too), held so the
/// run keeps it (O95): the sources written, which the caller names so collection keeps them.
pub fn store_encoded(store: &ContentStore, digest: &[u8; 32], image: bool, bytes: &[u8]) -> StoreResult<Vec<SourceId>> {
    if image {
        let key = image_key(digest);
        let id = match store.source_by_alias(key)? {
            Some(id) => id,
            None => store.put_source(bytes, &[key])?,
        };
        store.pin_held_sources(&[id])?;
        return Ok(vec![id]);
    }
    let hash = HashKey::sha256(bytes);
    let source = match store.held_source_by_hash(hash)? {
        Some(id) => id,
        None => store.put_source_hashed(bytes, hash, &[])?,
    };
    Ok(vec![source])
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
                // The image's digest is its bytes' (`store`), so it is its own text's source too: named once.
                out.insert(*d, (Stored::Image(bytes), text_ids[k].into_iter().filter(|t| *t != id).chain([id]).collect()));
            }
            // A program the engine ships (decision 0264): its image from the bundle, kept by no source of the store.
            None if let Some(bytes) = (mode != "off").then(|| crate::code_rule_dsl::logic::rr::images::shipped().and_then(|b| b.get(image_key(d)))).flatten() => {
                out.insert(*d, (Stored::Image(bytes.to_vec()), text_ids[k].into_iter().collect()));
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
/// The sources `texts` names for `digests` (each program's text and image), with none read: a run that holds the
/// programs in memory names what it would have read (the manifest's), so collection keeps them while it keeps the run.
pub fn sources(store: &ContentStore, digests: &[[u8; 32]]) -> StoreResult<Vec<SourceId>> {
    let mode = images();
    let hashes: Vec<HashKey> = digests.iter().map(|d| HashKey::from_sha256_digest(*d)).collect();
    let mut out: Vec<SourceId> = store.held_sources_by_hash(&hashes)?.into_iter().flatten().collect();
    if mode != "off" {
        for d in digests {
            if let Some(id) = store.source_by_alias(image_key(d))? {
                store.pin_held_sources(&[id])?;
                out.push(id);
            }
        }
    }
    Ok(out)
}

pub fn load(store: &ContentStore, digest: &[u8; 32], lists: &crate::code_rule_dsl::logic::front::Lists) -> StoreResult<Option<(Program, Vec<SourceId>)>> {
    let mut found = texts(store, &[*digest])?;
    let Some((stored, sources)) = found.remove(digest) else { return Ok(None) };
    Ok(read_stored(&stored, lists).ok().map(|p| (p, sources)))
}

/// `load`, with the program's version where it is kept as an image (`read_stored_version`).
pub fn load_version(store: &ContentStore, digest: &[u8; 32], lists: &crate::code_rule_dsl::logic::front::Lists) -> StoreResult<Option<(Program, Option<std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>>, Vec<SourceId>)>> {
    let mut found = texts(store, &[*digest])?;
    let Some((stored, sources)) = found.remove(digest) else { return Ok(None) };
    Ok(read_stored_version(&stored, lists).ok().map(|(p, v)| (p, v, sources)))
}

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
use crate::code_rule_dsl::logic::program_image::manager::VersionAnalyses as _;

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

/// A program as the store keeps it, made where the program is (the run's thread, decision 0306), kept by the store's
/// writer (`store_encoded`): its image (decision 0173), or with `RERAIL_PROGRAM_IMAGE=off` its text, checked to read
/// back as the program. The digest is of its bytes, which is the source's own hash.
pub struct Encoded {
    pub digest: [u8; 32],
    /// Its image's bytes (else its text's).
    pub image: bool,
    pub bytes: Vec<u8>,
}

/// A program's version as the store keeps it: a base's bytes as they are, with no encode of a tree (a prepared
/// program's version, `Version::base_bytes`, decision 0281), any other version folded into one first (Typing-support's
/// `store_version`, taken at the merge of e829e72a7); `None` where images are off (a version has no text form) or the
/// fold fails.
pub fn encode_version(v: &crate::code_rule_dsl::logic::program_image::Version) -> Option<Encoded> {
    if images() == "off" {
        return None;
    }
    let folded;
    let bytes = match v.base_bytes() {
        Some(bytes) => bytes,
        None => {
            folded = v.fold().ok()?;
            folded.base_bytes()?
        }
    };
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, bytes.len() as u64);
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    // A run building the engine's bundle (`RERAIL_EXPORT_IMAGES`, decision 0264) ships it.
    crate::code_rule_dsl::logic::rr::images::export(image_key(&digest), bytes);
    Some(Encoded { digest, image: true, bytes: bytes.to_vec() })
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

/// Image `bytes` kept in `store` under the digest of its bytes (decision 0246 §6), on the store's writer (an item's apply,
/// decision 0306): the digest, and the source, which the item names so collection keeps it. No pin (0306 step 4).
fn store_image(store: &ContentStore, bytes: &[u8]) -> StoreResult<([u8; 32], Vec<SourceId>)> {
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, bytes.len() as u64);
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    let ids = store_encoded(store, &digest, true, bytes)?;
    Ok((digest, ids))
}

/// Keeps a program `encode` made in `store`, a source by its bytes' sha256 (an image by `image_key` too): the sources
/// written, which the caller names so collection keeps them.
pub fn store_encoded(store: &ContentStore, digest: &[u8; 32], image: bool, bytes: &[u8]) -> StoreResult<Vec<SourceId>> {
    if image {
        let key = image_key(digest);
        let id = match store.source_by_alias(key)? {
            Some(id) => id,
            None => store.put_source(bytes, &[key])?,
        };
        return Ok(vec![id]);
    }
    let hash = HashKey::sha256(bytes);
    let source = match store.source_by_hash(hash)? {
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
    let text_ids = store.sources_by_hashes(&hashes)?;
    let mut out = std::collections::HashMap::new();
    let mut want_text = Vec::new();
    for (k, d) in digests.iter().enumerate() {
        let image = if mode == "off" { None } else { store.source_by_alias(image_key(d))? };
        match image {
            Some(id) => {
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
    let mut out: Vec<SourceId> = store.sources_by_hashes(&hashes)?.into_iter().flatten().collect();
    if mode != "off" {
        for d in digests {
            if let Some(id) = store.source_by_alias(image_key(d))? {
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

/// The version of the program `store` keeps of sha256 `digest`, opened from its image with no tree decoded (a
/// program kept as text, with images off, imaged from it), and the sources it is kept as; `None` as `load` gives.
pub fn load_image(store: &ContentStore, digest: &[u8; 32], lists: &crate::code_rule_dsl::logic::front::Lists) -> StoreResult<Option<(std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>, Vec<SourceId>)>> {
    let mut found = texts(store, &[*digest])?;
    let Some((stored, sources)) = found.remove(digest) else { return Ok(None) };
    let version = match &stored {
        Stored::Image(bytes) => crate::code_rule_dsl::logic::program_image::Version::open(bytes),
        Stored::Text(text) => read_in(text, lists).and_then(|p| crate::code_rule_dsl::logic::program_image::Builder::of_program(&p)?.finish().map(|(v, _)| v)),
    };
    Ok(version.ok().map(|v| (v, sources)))
}


/// A keyed pipeline's steps (decision 0281 §6), kept in `store` as facts beside the program images they name. A
/// step is two relations in 0103's form, its key and the step it names: `pipeline_step_key(key, pass, version)` (the
/// pass's version is the tag) and `pipeline_step(key, image, size, digest)`, with its decisions
/// (`pipeline_step_decision(key, k, pass, what, detail)`, 0269). They are one source under its key's alias, and the
/// output is an image as a prepared program's is (`store_image`), so a step whose output another step or program
/// stored already shares its image. A run's collection lease keeps what it reads and writes (decision 0306 step 4), and
/// a step is written by the store's writer, an item of the run's record (0306 step 2).
pub struct StoredSteps<'s>(pub &'s ContentStore);

/// The alias a step's facts are kept under: its key, with the image format, so another format's step is not found.
fn step_alias(key: &[u8; 32]) -> HashKey {
    let format = crate::code_rule_dsl::logic::program_image::FORMAT;
    HashKey::sha256(&[format!("rerail pipeline step {format}\0").as_bytes(), key.as_slice()].concat())
}

fn hex32(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex32(text: &str) -> Option<[u8; 32]> {
    let mut out = [0u8; 32];
    if text.len() != 64 {
        return None;
    }
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(text.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

impl crate::code_rule_dsl::logic::program_image::manager::Steps for StoredSteps<'_> {
    fn find(&self, key: &[u8; 32]) -> Result<Option<crate::code_rule_dsl::logic::program_image::manager::StepRecord>, String> {
        use crate::code_rule_dsl::logic::program_image::manager::{Decided, StepRecord};
        let e = |e: crate::content_store::StoreError| e.to_string();
        let Some(id) = self.0.source_by_alias(step_alias(key)).map_err(e)? else { return Ok(None) };
        let facts = program_facts::decode(&self.0.get_source(id).map_err(e)?)?;
        let k = hex32(key);
        let rows = |rel: &str| facts.get(rel).into_iter().flatten().filter(|r| matches!(r.first(), Some(Value::Str(s)) if **s == *k));
        let int = |v: &Value| match v {
            Value::Int(n) => Ok(*n),
            _ => Err(bad("a step's number is not one")),
        };
        let text = |v: &Value| match v {
            Value::Str(s) => Ok(s.to_string()),
            _ => Err(bad("a step's text is not one")),
        };
        let Some(head) = rows("pipeline_step_key").next() else { return Ok(None) };
        let Some(out) = rows("pipeline_step").next() else { return Ok(None) };
        let (pass, version) = (text(&head[1])?, int(&head[2])? as u32);
        let image = unhex32(&text(&out[1])?).ok_or_else(|| bad("a step's image is not a digest"))?;
        // A step whose image the store lost is not found: the step runs again.
        if self.0.source_by_alias(image_key(&image)).map_err(e)?.is_none() {
            return Ok(None);
        }
        let mut decided: Vec<(i64, Decided)> = Vec::new();
        for r in rows("pipeline_step_decision") {
            decided.push((int(&r[1])?, Decided { pass: text(&r[2])?.into(), what: text(&r[3])?.into(), detail: text(&r[4])? }));
        }
        decided.sort_by_key(|(k, _)| *k);
        Ok(Some(StepRecord { pass, version, image, output: (int(&out[2])? as usize, int(&out[3])? as u64), decided: decided.into_iter().map(|(_, d)| d).collect() }))
    }

    fn open(&self, image: &[u8; 32]) -> Result<Option<std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>>, String> {
        let e = |e: crate::content_store::StoreError| e.to_string();
        let Some(id) = self.0.source_by_alias(image_key(image)).map_err(e)? else { return Ok(None) };
        crate::code_rule_dsl::logic::program_image::Version::open(&self.0.get_source(id).map_err(e)?).map(Some)
    }

    fn keep(&self, key: &[u8; 32], record: &crate::code_rule_dsl::logic::program_image::manager::StepRecord, output: &crate::code_rule_dsl::logic::program_image::Version) -> Result<(), String> {
        let folded;
        let bytes = match output.base_bytes() {
            Some(b) => b,
            None => {
                folded = output.fold()?;
                folded.base_bytes().ok_or("a folded version is a base")?
            }
        };
        crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, bytes.len() as u64);
        let image: [u8; 32] = Sha256::digest(bytes).into();
        // A run building the engine's bundle (`RERAIL_EXPORT_IMAGES`, decision 0264) ships it.
        crate::code_rule_dsl::logic::rr::images::export(image_key(&image), bytes);
        let k = Value::Str(hex32(key).into());
        let mut facts = Facts::new();
        facts.entry("pipeline_step_key".into()).or_default().push([k.clone(), str_value(&record.pass), Value::Int(i64::from(record.version))].into());
        facts.entry("pipeline_step".into()).or_default().push([k.clone(), Value::Str(hex32(&image).into()), Value::Int(record.output.0 as i64), Value::Int(record.output.1 as i64)].into());
        for (i, d) in record.decided.iter().enumerate() {
            facts.entry("pipeline_step_decision".into()).or_default().push([k.clone(), Value::Int(i as i64), str_value(&d.pass), str_value(&d.what), str_value(&d.detail)].into());
        }
        // An item of the run's record (decision 0306): the image's bytes, then the step's facts, in its byte area;
        // each stored by the writer unless the store holds it already (an image another step or program stored).
        fn apply(cx: &crate::content_store::record::Apply<'_>, body: crate::content_store::record::codec::Body) -> StoreResult<()> {
            let (key, image, at): ([u8; 32], [u8; 32], usize) = crate::content_store::record::codec::from_body(STEP, body)?;
            let (image_bytes, facts) = cx.bytes.split_at_checked(at).ok_or_else(|| crate::content_store::StoreError::Invalid("a step's record: past its bytes".into()))?;
            if cx.store.source_by_alias(image_key(&image))?.is_none() {
                cx.store.put_source(image_bytes, &[image_key(&image)])?;
            }
            let alias = step_alias(&key);
            if cx.store.source_by_alias(alias)?.is_none() {
                cx.store.put_source(facts, &[alias])?;
            }
            Ok(())
        }
        let mut area = bytes.to_vec();
        area.extend_from_slice(&program_facts::encode(&facts));
        self.0.record(STEP, Vec::new(), &(*key, image, bytes.len()), Some(area), apply);
        Ok(())
    }
}

/// A pipeline step's item of the run's record (`StoredSteps::keep`).
const STEP: &str = "a pipeline step";

fn str_value(s: &str) -> Value {
    Value::Str(s.to_owned().into())
}

/// A pipeline's last run (decision 0333 §2), kept in `store` as facts beside the images of its versions, under its
/// key (`Runs::keep`): `pipeline_run(input, output)`, `pipeline_run_version(i, kind, parent, image, removed)` (a base
/// or an overlay's delta by its image, a view by the rules it leaves out), `pipeline_run_step(j, pass, input, output,
/// form, select kind, select ids, asked, own end)`, `pipeline_run_decision(j, pass, what, detail)`, and each step
/// input's analyses, `pipeline_run_analysis(version, name, tag, …row)`, read with the run and handed to its version.
/// 0103's two rules: the key names the run, and the run its versions. A run whose image the store lost is not found.
pub struct StoredRuns<'s>(pub &'s ContentStore);

fn run_alias(key: &[u8; 32]) -> HashKey {
    let format = crate::code_rule_dsl::logic::program_image::FORMAT;
    HashKey::sha256(&[format!("rerail pipeline run {format}\0").as_bytes(), key.as_slice()].concat())
}

fn ids_text(ids: &[u32]) -> Value {
    str_value(&ids.iter().map(u32::to_string).collect::<Vec<_>>().join(","))
}

fn text_ids(v: &Value) -> Result<Vec<u32>, String> {
    let Value::Str(s) = v else { return Err(bad("a stored run's ids are not text")) };
    if s.is_empty() {
        return Ok(Vec::new());
    }
    s.split(',').map(|x| x.parse::<u32>().map_err(|e| bad(e))).collect()
}

fn kept_run(run: &crate::code_rule_dsl::logic::program_image::manager::Run) -> (Vec<Vec<u8>>, Vec<u8>) {
    use crate::code_rule_dsl::logic::program_image::manager::StoredVersion;
    let mut images: Vec<Vec<u8>> = Vec::new();
    let mut image = |bytes: &[u8]| -> [u8; 32] {
        images.push(bytes.to_vec());
        Sha256::digest(bytes).into()
    };
    let parts = run.parts();
    let mut facts = Facts::new();
    let int = |n: i64| Value::Int(n);
    facts.entry("pipeline_run".into()).or_default().push([int(parts.input as i64), int(parts.output as i64)].into());
    for (i, v) in parts.versions.iter().enumerate() {
        let row: [Value; 5] = match v {
            StoredVersion::Base(bytes) => {
                let image = image(bytes);
                [int(i as i64), int(0), int(-1), str_value(&hex32(&image)), str_value("")]
            }
            StoredVersion::Overlay { parent, delta } => {
                let image = image(delta);
                [int(i as i64), int(1), int(*parent as i64), str_value(&hex32(&image)), str_value("")]
            }
            StoredVersion::View { parent, removed } => [int(i as i64), int(2), int(*parent as i64), str_value(""), ids_text(removed)],
        };
        facts.entry("pipeline_run_version".into()).or_default().push(row.into());
    }
    for (j, s) in parts.steps.iter().enumerate() {
        let (kind, ids, asked, own_end) = match &s.rule {
            Some(r) => (i64::from(r.select.0), ids_text(&r.select.1), str_value(&r.asked.join(",")), i64::from(r.own_end)),
            None => (-1, str_value(""), str_value(""), -1),
        };
        facts.entry("pipeline_run_step".into()).or_default().push(
            [int(j as i64), str_value(&s.pass), int(s.input as i64), int(s.output as i64), int(i64::from(s.form.code())), int(kind), ids, asked, int(own_end)].into(),
        );
    }
    for (j, d) in parts.decided.iter().enumerate() {
        facts.entry("pipeline_run_decision".into()).or_default().push([int(j as i64), str_value(&d.pass), str_value(&d.what), str_value(&d.detail)].into());
    }
    for (i, name, tag, rows) in &parts.analyses {
        let head = [int(*i as i64), str_value(name), int(i64::from(*tag))];
        facts.entry("pipeline_run_analysis".into()).or_default().extend(rows.iter().map(|r| head.iter().cloned().chain(r.iter().cloned()).collect::<Vec<_>>().into()));
    }
    (images, program_facts::encode(&facts))
}

/// Keeps a run (`kept_run`'s images and facts) under `key`: each image under its digest, the facts under the run's
/// alias (once). The sources, which the caller names so collection keeps them (decision 0306 step 4: no pins).
fn store_run(store: &ContentStore, key: &[u8; 32], images: &[&[u8]], facts: &[u8]) -> StoreResult<Vec<SourceId>> {
    let mut ids = Vec::new();
    for bytes in images {
        ids.extend(store_image(store, bytes)?.1);
    }
    let alias = run_alias(key);
    ids.push(match store.source_by_alias(alias)? {
        Some(id) => id,
        None => store.put_source(facts, &[alias])?,
    });
    Ok(ids)
}

/// The sources a pipeline run kept under `key` lives in (`store_run`'s): its versions' images and its facts. `None`
/// when the store does not hold it whole. A run that finds its prepared program names a kept run's sources by these,
/// as the run that kept it named them (decision 0333).
pub fn run_sources(store: &ContentStore, key: &[u8; 32]) -> Result<Option<Vec<SourceId>>, String> {
    let e = |e: crate::content_store::StoreError| e.to_string();
    let Some(id) = store.source_by_alias(run_alias(key)).map_err(e)? else { return Ok(None) };
    let facts = program_facts::decode(&store.get_source(id).map_err(e)?)?;
    let mut ids = Vec::new();
    for r in facts.get("pipeline_run_version").into_iter().flatten() {
        let (Some(Value::Int(kind)), Some(Value::Str(image))) = (r.get(1), r.get(3)) else { return Err(bad("a stored run's version is not one")) };
        if !matches!(kind, 0 | 1) {
            continue;
        }
        let image = unhex32(image).ok_or_else(|| bad("a stored run's image is not a digest"))?;
        let Some(image_id) = store.source_by_alias(image_key(&image)).map_err(e)? else { return Ok(None) };
        ids.push(image_id);
    }
    ids.push(id);
    Ok(Some(ids))
}

/// One run of `record_runs`' item: its key, and its images' and facts' lengths in the item's byte area.
#[derive(serde::Serialize, serde::Deserialize)]
struct RecordedRun {
    key: [u8; 32],
    images: Vec<usize>,
    facts: usize,
}

/// Keeps pipeline runs behind the run, as an item of its record (decision 0306): each encoded here (`kept_run`, on
/// the run's thread), stored by the writer, the sources named by the run's manifest. Replaces Typing-support's job of
/// code on the writer (`StoredRuns::keep` in a closure), at the merge of e829e72a7.
pub fn record_runs(store: &ContentStore, runs: &[([u8; 32], &crate::code_rule_dsl::logic::program_image::manager::Run)]) {
    let mut body = Vec::new();
    let mut bytes = Vec::new();
    for (key, run) in runs {
        let (images, facts) = kept_run(run);
        body.push(RecordedRun { key: *key, images: images.iter().map(Vec::len).collect(), facts: facts.len() });
        images.iter().for_each(|i| bytes.extend_from_slice(i));
        bytes.extend_from_slice(&facts);
    }
    fn apply(cx: &crate::content_store::record::Apply<'_>, body: crate::content_store::record::codec::Body) -> StoreResult<()> {
        let runs: Vec<RecordedRun> = crate::content_store::record::codec::from_body("the pipelines' runs", body)?;
        let mut at = 0;
        let mut take = |n: usize| {
            let b = &cx.bytes[at..at + n];
            at += n;
            b
        };
        for r in runs {
            let images: Vec<&[u8]> = r.images.iter().map(|&n| take(n)).collect();
            let facts = take(r.facts);
            cx.name(store_run(cx.store, &r.key, &images, facts)?);
        }
        Ok(())
    }
    store.record("the pipelines' runs", Vec::new(), &body, Some(bytes), apply);
}

impl crate::code_rule_dsl::logic::program_image::manager::Runs for StoredRuns<'_> {
    fn keep(&self, key: &[u8; 32], run: &crate::code_rule_dsl::logic::program_image::manager::Run) -> Result<(), String> {
        let (images, facts) = kept_run(run);
        store_run(self.0, key, &images.iter().map(Vec::as_slice).collect::<Vec<_>>(), &facts).map(drop).map_err(|e| e.to_string())
    }

    fn open(&self, key: &[u8; 32], name: &'static str) -> Result<Option<crate::code_rule_dsl::logic::program_image::manager::Run>, String> {
        use crate::code_rule_dsl::logic::program_image::manager::{Decided, Form, Run, StoredRule, StoredStep};
        use crate::code_rule_dsl::logic::program_image::Version;
        let e = |e: crate::content_store::StoreError| e.to_string();
        let store = self.0;
        let Some(id) = store.source_by_alias(run_alias(key)).map_err(e)? else { return Ok(None) };
        let facts = program_facts::decode(&store.get_source(id).map_err(e)?)?;
        let rows = |rel: &str| facts.get(rel).cloned().unwrap_or_default();
        let int = |v: &Value| match v {
            Value::Int(n) => Ok(*n),
            _ => Err(bad("a stored run's number is not one")),
        };
        let text = |v: &Value| match v {
            Value::Str(s) => Ok(s.to_string()),
            _ => Err(bad("a stored run's text is not one")),
        };
        let mut by_index: Vec<crate::code_rule_dsl::logic::front::List<Value>> = rows("pipeline_run_version");
        by_index.sort_by_key(|r| int(&r[0]).unwrap_or(i64::MAX));
        let mut versions: Vec<std::sync::Arc<Version>> = Vec::new();
        for r in &by_index {
            let (kind, parent) = (int(&r[1])?, int(&r[2])?);
            let parent = || usize::try_from(parent).ok().and_then(|p| versions.get(p).cloned()).ok_or_else(|| bad("a stored run's version names no parent before it"));
            let bytes = || -> Result<Option<Vec<u8>>, String> {
                let image = unhex32(&text(&r[3])?).ok_or_else(|| bad("a stored run's image is not a digest"))?;
                let Some(image_id) = store.source_by_alias(image_key(&image)).map_err(e)? else { return Ok(None) };
                Ok(Some(store.get_source(image_id).map_err(e)?))
            };
            let v = match kind {
                0 => match bytes()? {
                    Some(b) => Version::open(&b)?,
                    None => return Ok(None),
                },
                1 => match bytes()? {
                    Some(b) => Version::over(&parent()?, &b)?,
                    None => return Ok(None),
                },
                2 => Version::view_without(&parent()?, &text_ids(&r[4])?)?,
                _ => return Err(bad("a stored run's version is of no kind")),
            };
            versions.push(v);
        }
        // Each version's analyses, by name and tag, handed to it to read before computing one.
        let mut analyses: std::collections::BTreeMap<i64, std::collections::HashMap<(String, u32), Vec<Vec<Value>>>> = Default::default();
        for r in rows("pipeline_run_analysis") {
            let (at, name, tag) = (int(&r[0])?, text(&r[1])?, u32::try_from(int(&r[2])?).map_err(|e| bad(e))?);
            analyses.entry(at).or_default().entry((name, tag)).or_default().push(r[3..].to_vec());
        }
        for (at, held) in analyses {
            let v = usize::try_from(at).ok().and_then(|i| versions.get(i)).ok_or_else(|| bad("a stored analysis names no version"))?;
            let held = std::sync::Arc::new(held);
            v.analyses().read_stored(std::sync::Arc::new(move |name: &str, tag: u32| held.get(&(name.to_owned(), tag)).cloned()));
        }
        let mut steps: Vec<(i64, StoredStep)> = Vec::new();
        for r in rows("pipeline_run_step") {
            let kind = int(&r[5])?;
            let rule = (kind >= 0).then(|| -> Result<StoredRule, String> {
                let asked = text(&r[7])?;
                Ok(StoredRule {
                    select: (u8::try_from(kind).map_err(|e| bad(e))?, text_ids(&r[6])?),
                    asked: if asked.is_empty() { Vec::new() } else { asked.split(',').map(str::to_owned).collect() },
                    own_end: u32::try_from(int(&r[8])?).map_err(|e| bad(e))?,
                })
            });
            let form = Form::of_code(u8::try_from(int(&r[4])?).map_err(|e| bad(e))?).ok_or_else(|| bad("a stored step's form is not one"))?;
            steps.push((int(&r[0])?, StoredStep { pass: text(&r[1])?, input: int(&r[2])? as usize, output: int(&r[3])? as usize, form, rule: rule.transpose()? }));
        }
        steps.sort_by_key(|(j, _)| *j);
        let mut decided: Vec<(i64, Decided)> = Vec::new();
        for r in rows("pipeline_run_decision") {
            decided.push((int(&r[0])?, Decided { pass: text(&r[1])?.into(), what: text(&r[2])?.into(), detail: text(&r[3])? }));
        }
        decided.sort_by_key(|(j, _)| *j);
        let Some(head) = rows("pipeline_run").into_iter().next() else { return Ok(None) };
        // No pin (0306 step 4): the run that reads it keeps its own run, which its record names (`record_runs`).
        Run::assemble(name, &versions, int(&head[0])? as usize, int(&head[1])? as usize, steps.into_iter().map(|(_, s)| s).collect(), decided.into_iter().map(|(_, d)| d).collect()).map(Some)
    }
}

/// `facts` as a `.rr` document: the schema's declarations and every row as a fact. `rr::parse` reads it back as the
/// same rows.
pub fn document(facts: &super::program_facts::Facts, extra: &std::collections::BTreeMap<super::front::name::Name, crate::code_rule_dsl::logic::front::name::List<(super::front::name::Name, super::front::name::Name)>>) -> Result<String, String> {
    let mut decls = super::program_facts::schema();
    decls.extend(extra.iter().map(|(k, v)| (k.clone(), v.clone())));
    let prog = super::front::ast::Program { decls, facts: facts.clone(), ..super::front::ast::Program::default() };
    super::rr::print(&prog).map_err(|e| format!("program facts: {e:?}"))
}

//! The asks of a program's exported relations, recorded (`RERAIL_ASKS`, a measurement aid for callable per-file
//! relations, decision 0725): what the cross part needs of each relation its files make, reader by reader.
//!
//! For each rule of the cross part (its head not a file's relation) and each relation of the files it reads at the
//! top of its body, two relations are added and made outputs:
//!
//! - `rr_key__<n>__<rel>__<reader>__<pattern>`: the keys the reader uses, its columns those `pattern` marks `b` (as
//!   the rest of the body binds them, `callable::request_body`); none where it binds no column. Made from the reader's whole body, so a key that
//!   finds no row is not counted. A negated read's are every key it tests: its body without the negation.
//! - `rr_used__<n>__<rel>__<reader>__<pattern>`: the rows of `rel` the reader uses (its columns written as
//!   variables), from its whole body. None for a negated read.
//!
//! Against `rel`'s rows made, they are the least a call answering this reader could make: its query shape and its
//! volume. Where `rel` names its file (the split's `R_at(d, ..)`, or a location column of its own: a document, or a
//! node), three more: `rr_kfile__<tag>(key.., path)`, the files answering each key asked;
//! `rr_kpair__<rel>__<pattern>(key.., d)`, every (key, document) pair of its rows, a key-to-file index's entries; and
//! `rr_kall__<rel>__<pattern>(key..)`, its keys. A relation asked by key that names no file is returned
//! (`Recorded::unlocated`). A relay (`R(..) :- R_at(_, ..)`) is looked through: its readers' asks are recorded under its
//! head. Reads inside an aggregate are not recorded.

use super::callable::request_body;
use crate::code_rule_dsl::logic::plan::body::{VarSet, term_vars};
use crate::code_rule_dsl::logic::program_image::fields::FieldsMut;
use crate::code_rule_dsl::logic::program_image::{Builder, Lit, LitRef, T, TermRef, Version, t_of};
use std::collections::BTreeSet;
use std::sync::Arc;

/// The key relations' prefix.
pub const KEY: &str = "rr_key__";
/// The used rows' relations' prefix.
pub const USED: &str = "rr_used__";
/// The files answering each key asked: `(key.., path)`, a located relation's keyed asks alone.
pub const FILES: &str = "rr_kfile__";
/// Every (key, document) pair of a located relation, by the columns an ask binds: a key-to-file index's entries.
pub const PAIRS: &str = "rr_kpair__";
/// Every key of a located relation, by those columns.
pub const KEYS: &str = "rr_kall__";
/// The split's qualifier that makes a relation in the cross part.
const CROSS_FILE: &str = crate::code_rule_dsl::typing::file_local::CROSS_FILE;

/// Whether the run records them: `RERAIL_ASKS` set.
pub fn on() -> bool {
    std::env::var_os("RERAIL_ASKS").is_some()
}

/// `v` with its exported relations' asks recorded (module docs); `file_local`, its files' relations. `None` where
/// no rule of the cross part reads one.
pub fn recorded(v: &Arc<Version>, file_local: &BTreeSet<String>, locations: &std::collections::BTreeMap<String, rerail_blocks::vector::Vector<usize>>) -> Result<Recorded, String> {
    let derived: BTreeSet<&str> = v.rules().map(|r| v.rel_name(r.head().rel())).collect();
    // A relay the split made (`R(x..) :- R_at(_, x..)`: one positive literal, its document wild, then the head's
    // variables in order) passes a recorded relation's rows on as they are: its own asks are its readers', recorded
    // in its stead.
    // Its source (`R_at`) where `r` is one.
    let relay_of = |r: crate::code_rule_dsl::logic::program_image::RuleRef<'_>, from: &BTreeSet<String>| -> Option<String> {
        let lits: Vec<LitRef<'_>> = r.body().collect();
        let [one] = lits[..] else { return None };
        let Lit::Atom(a) = one.view() else { return None };
        let args: Vec<TermRef<'_>> = a.args().collect();
        let head: Vec<TermRef<'_>> = r.head().args().collect();
        let is = !a.neg()
            && from.contains(v.rel_name(a.rel()))
            && args.len() == head.len() + 1
            && matches!(args[0], TermRef::Wild)
            && head.iter().zip(&args[1..]).all(|(h, x)| h.var().is_some() && h.var() == x.var());
        is.then(|| v.rel_name(a.rel()).to_owned())
    };
    let is_relay = |r: crate::code_rule_dsl::logic::program_image::RuleRef<'_>, from: &BTreeSet<String>| relay_of(r, from).is_some();
    let mut recorded: BTreeSet<String> = file_local.iter().filter(|r| derived.contains(r.as_str())).cloned().collect();
    let mut relays: BTreeSet<String> = BTreeSet::new();
    loop {
        let more: BTreeSet<String> = derived
            .iter()
            .filter(|h| !recorded.contains(**h) && !file_local.contains(**h))
            .filter(|h| v.rules().filter(|r| v.rel_name(r.head().rel()) == **h).all(|r| is_relay(r, &recorded)))
            .map(|h| (*h).to_owned())
            .collect();
        if more.is_empty() {
            break;
        }
        relays.extend(more.iter().cloned());
        recorded.extend(more);
    }
    // Each relay head's located source, the split's `R_at(d, ..)`, its document in front.
    let sources: std::collections::BTreeMap<String, String> = v
        .rules()
        .filter(|r| relays.contains(v.rel_name(r.head().rel())))
        .filter_map(|r| relay_of(r, &recorded).map(|at| (v.rel_name(r.head().rel()).to_owned(), at)))
        .collect();
    let mut b = Builder::derive(v);
    for r in v.rules() {
        b.keep(v, r);
    }
    let path_type = v.decl_of("doc").and_then(|d| d.cols().nth(1)).map(|(_, t)| v.text(t).to_owned());
    let docof = v.inputs().any(|a| v.text(a) == "docof");
    let mut indexed: BTreeSet<String> = BTreeSet::new();
    let mut unlocated: BTreeSet<String> = BTreeSet::new();
    let mut how_located: Vec<(String, String)> = Vec::new();
    let mut added: Vec<String> = Vec::new();
    let mut r_count = 0usize;
    for r in v.rules() {
        let reader = v.rel_name(r.head().rel());
        if file_local.contains(reader) || relays.contains(reader) || reader.starts_with(KEY) || reader.starts_with(USED) {
            continue;
        }
        let lits: Vec<LitRef<'_>> = r.body().collect();
        for (i, l) in lits.iter().enumerate() {
            let Lit::Atom(a) = l.view() else { continue };
            let rel = v.rel_name(a.rel());
            if !recorded.contains(rel) {
                continue;
            }
            let Some(decl) = v.decl_of(rel) else { continue };
            let cols: Vec<(String, String)> = decl.cols().map(|(n, t)| (v.text(n).to_owned(), v.text(t).to_owned())).collect();
            let args: Vec<TermRef<'_>> = a.args().collect();
            let (_, vars) = request_body(v, &lits, i, rel);
            let bound: Vec<usize> = (0..args.len())
                .filter(|&k| !matches!(args[k], TermRef::Wild | TermRef::Top))
                .filter(|&k| {
                    let mut vs = VarSet::new();
                    term_vars(args[k], &mut vs);
                    vs.is_subset(&vars)
                })
                .collect();
            let pattern: String = (0..args.len()).map(|k| if bound.contains(&k) { 'b' } else { 'f' }).collect();
            let n = r_count;
            r_count += 1;
            let tag = format!("{n}__{rel}__{reader}__{pattern}{}", if a.neg() { "_neg" } else { "" });
            // The keys: the bound columns' terms (none where no column is bound: its rows used say it all).
            let key = format!("{KEY}{tag}");
            if !bound.is_empty() {
                let key_args: Vec<T<'_>> = bound.iter().map(|&k| t_of(r, args[k])).collect();
                let key_cols: Vec<(&str, &str)> = bound.iter().map(|&k| (cols[k].0.as_str(), cols[k].1.as_str())).collect();
                let mut w = b.rule().head(key.as_str(), &key_args);
                for (j, m) in lits.iter().enumerate() {
                    if !(a.neg() && j == i) {
                        w = w.lit(v, r, *m);
                    }
                }
                w.finish()?;
                b.fields().declare(&key, &key_cols);
                // A located relation's files: its every (key, document) pair (`rr_kpair__`, a key-to-file index's
                // entries) and key (`rr_kall__`), both made in the cross part (`cross_file`), and each asked key's files
                // (`rr_kfile__`). Its document is the split's relay's (`R_at(d, ..)`), or a location column of its own:
                // a document, or a node's (`docof`, else `node`).
                let kv: Vec<String> = (0..bound.len()).map(|j| format!("rr_k{j}")).collect();
                let key_at = |k: usize| bound.iter().position(|&x| x == k).map(|j| kv[j].as_str());
                let own = locations.get(rel).and_then(|ls| ls.first().copied()).filter(|_| !sources.contains_key(rel));
                // The document's source: a relation and its arguments, and a node to read its document off.
                // The document's source: a relation and its arguments, a node to read its document off, the
                // document's variable and its type.
                let type_of = |rel: &str, c: usize| v.decl_of(rel).and_then(|d| d.cols().nth(c)).map(|(_, t)| v.text(t).to_owned());
                let located: Option<(&str, Vec<T<'_>>, Option<&str>, &str, Option<String>)> = if let Some(at) = sources.get(rel) {
                    let mut at_args: Vec<T<'_>> = vec![T::Var("rr_d")];
                    at_args.extend((0..args.len()).map(|k| key_at(k).map_or(T::Wild, T::Var)));
                    Some((at.as_str(), at_args, None, "rr_d", type_of(at, 0)))
                } else if let Some(c) = own {
                    let place = key_at(c).unwrap_or("rr_l");
                    let rel_args: Vec<T<'_>> = (0..args.len()).map(|k| if k == c { T::Var(place) } else { key_at(k).map_or(T::Wild, T::Var) }).collect();
                    let node = cols.get(c).is_some_and(|(_, t)| t == "node");
                    let doc_type = if !node { cols.get(c).map(|(_, t)| t.clone()) } else if docof { type_of("docof", 1) } else { type_of("node", 0) };
                    Some((rel, rel_args, node.then_some(place), if node { "rr_d" } else { place }, doc_type))
                } else {
                    unlocated.insert(rel.to_owned());
                    None
                };
                if let Some((src, src_args, node, d, Some(doc_type))) = located {
                    // How it names its file, and which of its key's columns does, where one does.
                    let how = match own {
                        None => "relay".to_owned(),
                        Some(c) => format!("{} column {c}{}", if node.is_some() { "node" } else { "document" }, bound.iter().position(|&x| x == c).map_or(String::new(), |j| format!(", key {j}"))),
                    };
                    how_located.push((tag.clone(), how));
                    let kt: Vec<T<'_>> = kv.iter().map(|s| T::Var(s.as_str())).collect();
                    let pairs = format!("{PAIRS}{rel}__{pattern}");
                    let keys = format!("{KEYS}{rel}__{pattern}");
                    if indexed.insert(pairs.clone()) {
                        let rel_args: Vec<T<'_>> = (0..args.len()).map(|k| key_at(k).map_or(T::Wild, T::Var)).collect();
                        b.rule().head(keys.as_str(), &kt).atom(rel, &rel_args).finish()?;
                        b.fields().declare(&keys, &key_cols);
                        let mut head = kt.clone();
                        head.push(T::Var(d));
                        let mut w = b.rule().head(pairs.as_str(), &head).atom(keys.as_str(), &kt).atom(src, &src_args);
                        if let Some(n) = node {
                            w = if docof { w.atom("docof", &[T::Var(n), T::Var("rr_d")]) } else { w.atom("node", &[T::Var("rr_d"), T::Var(n), T::Wild, T::Wild, T::Wild]) };
                        }
                        w.finish()?;
                        let mut pair_cols = key_cols.clone();
                        pair_cols.push(("d", doc_type.as_str()));
                        b.fields().declare(&pairs, &pair_cols);
                        b.fields().add_qualifiers(&[(keys.as_str(), CROSS_FILE), (pairs.as_str(), CROSS_FILE)]);
                        added.push(keys);
                        added.push(pairs.clone());
                    }
                    // Each asked key's files: by `doc`, a document's path.
                    if let Some(path_type) = &path_type {
                        let files = format!("{FILES}{tag}");
                        let mut head = kt.clone();
                        head.push(T::Var("rr_p"));
                        let mut pair_args = kt.clone();
                        pair_args.push(T::Var("rr_d"));
                        b.rule().head(files.as_str(), &head).atom(key.as_str(), &kt).atom(pairs.as_str(), &pair_args).atom("doc", &[T::Var("rr_d"), T::Var("rr_p"), T::Wild]).finish()?;
                        let mut files_cols = key_cols.clone();
                        files_cols.push(("p", path_type.as_str()));
                        b.fields().declare(&files, &files_cols);
                        added.push(files);
                    }
                }
            }
            added.push(key);
            // The rows used: the read's variable columns.
            let used = format!("{USED}{tag}");
            let at: Vec<usize> = (0..args.len()).filter(|&k| matches!(args[k], TermRef::Var(_))).collect();
            if !a.neg() && !at.is_empty() {
                let used_args: Vec<T<'_>> = at.iter().map(|&k| t_of(r, args[k])).collect();
                let used_cols: Vec<(&str, &str)> = at.iter().map(|&k| (cols[k].0.as_str(), cols[k].1.as_str())).collect();
                let mut w = b.rule().head(used.as_str(), &used_args);
                for m in &lits {
                    w = w.lit(v, r, *m);
                }
                w.finish()?;
                b.fields().declare(&used, &used_cols);
            }
            added.push(used);
        }
    }
    if added.is_empty() {
        return Ok(Recorded { version: None, unlocated, located: how_located });
    }
    let mut outputs: Vec<String> = v.outputs().map(|o| v.text(o).to_owned()).collect();
    outputs.extend(added.iter().filter(|r| b.declares(r)).cloned());
    b.fields().set_outputs(&outputs);
    let (out, _) = b.finish()?;
    Ok(Recorded { version: Some(out), unlocated, located: how_located })
}

/// What `recorded` made: the program with its asks recorded (`None` where nothing reads a relation of the files),
/// the relations asked by key that name no file (no relay, and no location column), and how each probe's relation
/// names its file: `relay` (the split's `R_at`), or `document column C` or `node column C`, with `, key J` where its
/// key's column J is that column (the key names its file itself).
pub struct Recorded {
    pub version: Option<Arc<Version>>,
    pub unlocated: BTreeSet<String>,
    pub located: Vec<(String, String)>,
}

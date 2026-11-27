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
//! volume. Where `rel` is located (the split's `R_at(d, ..)`), three more: `rr_kfile__<tag>(key.., path)`, the files
//! answering each key asked; `rr_kpair__<rel>__<pattern>(key.., d)`, every (key, document) pair of its rows, a
//! key-to-file index's entries; and `rr_kall__<rel>__<pattern>(key..)`, its keys. A relay (`R(..) :- R_at(_, ..)`) is looked through: its readers' asks are recorded under its head. Reads
//! inside an aggregate are not recorded.

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

/// Whether the run records them: `RERAIL_ASKS` set.
pub fn on() -> bool {
    std::env::var_os("RERAIL_ASKS").is_some()
}

/// `v` with its exported relations' asks recorded (module docs); `file_local`, its files' relations. `None` where
/// no rule of the cross part reads one.
pub fn recorded(v: &Arc<Version>, file_local: &BTreeSet<String>) -> Result<Option<Arc<Version>>, String> {
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
    let mut indexed: BTreeSet<String> = BTreeSet::new();
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
                // A located relation's files: each asked key's, and its every (key, document) pair and key.
                if let Some(at) = sources.get(rel)
                    && let Some(at_decl) = v.decl_of(at)
                    && let Some(path_type) = v.decl_of("doc").and_then(|d| d.cols().nth(1)).map(|(_, t)| v.text(t).to_owned())
                {
                    let doc_type = at_decl.cols().next().map(|(_, t)| v.text(t).to_owned()).unwrap_or_default();
                    let kv: Vec<String> = (0..bound.len()).map(|j| format!("rr_k{j}")).collect();
                    let mut at_args: Vec<T<'_>> = vec![T::Var("rr_d")];
                    at_args.extend((0..args.len()).map(|k| bound.iter().position(|&x| x == k).map_or(T::Wild, |j| T::Var(kv[j].as_str()))));
                    let kt: Vec<T<'_>> = kv.iter().map(|s| T::Var(s.as_str())).collect();
                    let files = format!("{FILES}{tag}");
                    let mut head = kt.clone();
                    head.push(T::Var("rr_p"));
                    b.rule().head(files.as_str(), &head).atom(key.as_str(), &kt).atom(at.as_str(), &at_args).atom("doc", &[T::Var("rr_d"), T::Var("rr_p"), T::Wild]).finish()?;
                    let mut files_cols = key_cols.clone();
                    files_cols.push(("p", path_type.as_str()));
                    b.fields().declare(&files, &files_cols);
                    added.push(files);
                    let shape: String = pattern.clone();
                    let pairs = format!("{PAIRS}{rel}__{shape}");
                    let keys = format!("{KEYS}{rel}__{shape}");
                    // Both read in the cross part (its relay head, then that): its rows are the cross part's to count.
                    if indexed.insert(pairs.clone()) {
                        let rel_args: Vec<T<'_>> = at_args[1..].to_vec();
                        b.rule().head(keys.as_str(), &kt).atom(rel, &rel_args).finish()?;
                        b.fields().declare(&keys, &key_cols);
                        let mut head = kt.clone();
                        head.push(T::Var("rr_d"));
                        b.rule().head(pairs.as_str(), &head).atom(keys.as_str(), &kt).atom(at.as_str(), &at_args).finish()?;
                        let mut pair_cols = key_cols.clone();
                        pair_cols.push(("d", doc_type.as_str()));
                        b.fields().declare(&pairs, &pair_cols);
                        added.push(pairs);
                        added.push(keys);
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
        return Ok(None);
    }
    let mut outputs: Vec<String> = v.outputs().map(|o| v.text(o).to_owned()).collect();
    outputs.extend(added.iter().filter(|r| b.declares(r)).cloned());
    b.fields().set_outputs(&outputs);
    let (out, _) = b.finish()?;
    Ok(Some(out))
}

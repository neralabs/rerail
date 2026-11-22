//! The program texts the combine last read, kept so that an edit reads only those it changed (decision 0373's
//! splice, read before it).
//!
//! The combine reads each text for its library modules (`rr::Modules`): the files its includes resolve to, each
//! with its bytes' digest and the relations it declares, and what the text derives and declares itself. Those give
//! each program's scope and image key, which the splice compares with its last link. The record keeps each text's
//! modules by the text's digest, under the run's shape (`family`). A text whose digest is in the record, and each
//! of whose files still reads as that digest (`rr::file_holds`), is not read again: its modules are the record's,
//! and its files are noted as read (`rr::note_files`), as reading it would note them.

use super::preparation::parts::{hex, keep_facts, kept_facts_alone, rows_of, str, unhex};
use crate::code_rule_dsl::logic::program_facts::Facts;
use crate::code_rule_dsl::logic::rr::Modules;
use crate::content_store::ContentStore;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// A text's digest: what the record knows it by.
fn digest(text: &str) -> [u8; 32] {
    use sha2::Digest as _;
    sha2::Sha256::digest(text.as_bytes()).into()
}

/// The key the record of a run's texts is kept under: the engine and the number of programs.
fn family(programs: usize) -> [u8; 32] {
    use sha2::Digest as _;
    let mut h = sha2::Sha256::new();
    h.update(b"rerail/combine-texts/v1\0");
    h.update(crate::code_rule_dsl::build_ids::engine().as_bytes());
    h.update((programs as u64).to_le_bytes());
    h.finalize().into()
}

fn join<'a>(names: impl IntoIterator<Item = &'a String>) -> String {
    names.into_iter().cloned().collect::<Vec<_>>().join("\u{1f}")
}

fn split(s: &str) -> BTreeSet<String> {
    if s.is_empty() { BTreeSet::new() } else { s.split('\u{1f}').map(str::to_owned).collect() }
}

/// Each text's modules, known from the record where it stands for the text (`None` where the text must be read):
/// `Some(None)` for a text that includes nothing.
pub fn known(store: &ContentStore, programs: &[&str]) -> Vec<Option<Option<Modules>>> {
    let _s = crate::code_rule_dsl::logic::profile::span(|| "the texts the combine last read, from the store".into());
    let mut out = vec![None; programs.len()];
    let Some((facts, source)) = kept_facts_alone(store, &family(programs.len())) else { return out };
    crate::code_rule_dsl::logic::rr::renew_images(store, &[source]);
    let Some(record) = record_of(&facts) else { return out };
    let mut holds: BTreeMap<(PathBuf, [u8; 32]), bool> = BTreeMap::new();
    for (slot, text) in out.iter_mut().zip(programs) {
        let Some(modules) = record.get(&digest(text)) else { continue };
        let stands = modules.as_ref().is_none_or(|m| m.files.iter().all(|(path, d, _)| *holds.entry((path.clone(), *d)).or_insert_with(|| crate::code_rule_dsl::logic::rr::file_holds(path, d))));
        if stands {
            *slot = Some(modules.clone());
        }
    }
    out
}

/// Keeps the record of `programs`' modules (`modular`, in their order), written by the store's writer.
pub fn keep(store: &ContentStore, programs: &[&str], modular: &[Option<Modules>]) {
    let _s = crate::code_rule_dsl::logic::profile::span(|| "keep the texts the combine read".into());
    let mut f = Facts::new();
    let mut put = |rel: &str, rows: Vec<Vec<String>>| {
        f.insert(rel.to_owned(), rows.into_iter().map(|r| r.iter().map(|c| str(c)).collect::<Vec<_>>().into()).collect());
    };
    let mut texts = Vec::new();
    let mut includes = Vec::new();
    let mut files: BTreeMap<(String, String), String> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for (text, modules) in programs.iter().zip(modular) {
        let d = hex(&digest(text));
        if !seen.insert(d.clone()) {
            continue;
        }
        match modules {
            None => texts.push(vec![d, "plain".to_owned(), String::new(), String::new()]),
            Some(m) => {
                texts.push(vec![d.clone(), "modular".to_owned(), join(&m.heads), join(&m.declared)]);
                for (j, (path, fd, names)) in m.files.iter().enumerate() {
                    let path = path.to_string_lossy().into_owned();
                    let fd = hex(fd);
                    includes.push(vec![d.clone(), j.to_string(), path.clone(), fd.clone()]);
                    files.entry((path, fd)).or_insert_with(|| join(names));
                }
            }
        }
    }
    put("texts_text", texts);
    put("texts_include", includes);
    put("texts_file", files.into_iter().map(|((path, d), names)| vec![path, d, names]).collect());
    keep_facts(store, &family(programs.len()), &f);
}

/// The record's modules by text digest.
fn record_of(facts: &Facts) -> Option<BTreeMap<[u8; 32], Option<Modules>>> {
    let names: BTreeMap<(String, String), BTreeSet<String>> = rows_of(facts, "texts_file").map(|r| Some(((r.first()?.clone(), r.get(1)?.clone()), split(r.get(2)?)))).collect::<Option<_>>()?;
    let mut included: BTreeMap<String, Vec<(usize, PathBuf, [u8; 32], BTreeSet<String>)>> = BTreeMap::new();
    for r in rows_of(facts, "texts_include") {
        let (d, j, path, fd) = (r.first()?, r.get(1)?.parse().ok()?, r.get(2)?, r.get(3)?);
        let file_names = names.get(&(path.clone(), fd.clone()))?.clone();
        included.entry(d.clone()).or_default().push((j, PathBuf::from(path), unhex(fd)?, file_names));
    }
    let mut out = BTreeMap::new();
    for r in rows_of(facts, "texts_text") {
        let d = r.first()?;
        let modules = match r.get(1)?.as_str() {
            "plain" => None,
            "modular" => {
                let mut files = included.remove(d).unwrap_or_default();
                files.sort_by_key(|(j, ..)| *j);
                Some(Modules { files: files.into_iter().map(|(_, p, fd, n)| (p, fd, n)).collect(), heads: split(r.get(2)?), declared: split(r.get(3)?) })
            }
            _ => return None,
        };
        out.insert(unhex(d)?, modules);
    }
    Some(out)
}

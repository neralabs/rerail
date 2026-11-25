//! The vocabularies as kept facts (t33 step 7): a key's matching terms over a language's terms, for the prefilter's
//! regexes (`REGEX`, what `vocabulary_predicate`, `vocabulary_term`, `vocabulary_literal` and `vocabulary_pending`
//! held) and the text layer's plans (`TEXT`, what `text_matcher`, `text_term` and `text_pending` held). One mechanism:
//!
//! - A key's terms are a partition of the vocabulary space (by a hash of its kind, language and key):
//!   `voc_head(kind, lang, upto, len)`, the key's bytes `voc_key(at, word)`, its terms `voc_term(term)`. `upto`: the
//!   last publication of the counts (`kept::Sums::head`) its terms were tested up to.
//! - Brought up to date before it is read (`ContentStore::vocabulary_terms`): a key not kept tests every term of the
//!   language; a kept one the terms of the counts' runs past `upto` (`Sums::terms_since`), so a merge across `upto`
//!   tests its run's terms again; a term of those runs no tape holds any more leaves the key's set. A reader (`vtab`)
//!   leaves out a kept term no tape holds.
//! - A key that is a finite set of exact literals (a regex `^(?:a|b)$`, a text plan's whole literals) is its literals
//!   that some tape holds, found by value (never tested) and kept as any key's.
//!
//! Terms are the store's atoms, typed `s`.
use super::super::kept_facts::{KeptPut, KeptSpace, sorted_put};
use super::super::{ContentStore, StoreError, StoreResult};
use super::kept;
use rerail_blocks::kept::{FactKey, Kept, KeptRead};
use sha2::{Digest as _, Sha256};

/// A regex's terms (the prefilter's) and a text plan's (the text layer's): store-base's, which the facts name too.
pub use rerail_store_base::content_store::prefilter::{VOCABULARY_REGEX as REGEX, VOCABULARY_TEXT as TEXT};

const HEAD: &str = "voc_head";
const KEY: &str = "voc_key";
const TERMS: &str = "voc_term";

pub fn space() -> [u8; 32] {
    Sha256::digest(b"rerail.prefilter-vocabulary/v1").into()
}

fn ident(rel: &str) -> [u8; 32] {
    Sha256::digest(format!("rerail.prefilter-vocabulary/v1\0{rel}").as_bytes()).into()
}

fn part(kind: i64, lang: i64, key: &str) -> i64 {
    let mut h = Sha256::new();
    h.update(kind.to_le_bytes());
    h.update(lang.to_le_bytes());
    h.update(key.as_bytes());
    i64::from_le_bytes(h.finalize()[..8].try_into().expect("8 bytes")) & i64::MAX
}

fn entry<'a, R: KeptRead + ?Sized>(view: &'a R, rel: &str, part: i64) -> Option<Kept<'a>> {
    let ident = ident(rel);
    view.get(&FactKey { rel, ident: &ident, part }).filter(|e| *e.key.ident == ident)
}

/// Key `key`'s kept terms and the publication they were tested up to; `None` when not kept (or another key's).
fn kept_key<R: KeptRead + ?Sized>(view: &R, kind: i64, lang: i64, key: &str) -> Option<(i64, Vec<i64>)> {
    let p = part(kind, lang, key);
    let head = entry(view, HEAD, p)?;
    if head.rows.len() != 1 || head.rows.get(0, 0) != kind || head.rows.get(0, 1) != lang || head.rows.get(0, 3) != key.len() as i64 {
        return None;
    }
    let words = entry(view, KEY, p);
    let mut bytes = Vec::with_capacity(key.len());
    if let Some(w) = words {
        for i in 0..w.rows.len() {
            bytes.extend_from_slice(&w.rows.get(i, 1).to_le_bytes());
        }
    }
    bytes.truncate(key.len());
    if bytes != key.as_bytes() {
        return None;
    }
    let terms = entry(view, TERMS, p).map(|e| (0..e.rows.len()).map(|i| e.rows.get(i, 0)).collect()).unwrap_or_default();
    Some((head.rows.get(0, 2), terms))
}

/// Every kept key of kind `kind`: its language, its text and its terms as kept.
pub fn kept_all<R: KeptRead + ?Sized>(view: &R, kind: i64) -> Vec<(i64, String, Vec<i64>)> {
    let head = ident(HEAD);
    let mut out = Vec::new();
    for e in view.of(HEAD).filter(|e| *e.key.ident == head) {
        if e.rows.len() != 1 || e.rows.get(0, 0) != kind {
            continue;
        }
        let (lang, len, p) = (e.rows.get(0, 1), e.rows.get(0, 3), e.key.part);
        let mut bytes = Vec::new();
        if let Some(w) = entry(view, KEY, p) {
            (0..w.rows.len()).for_each(|i| bytes.extend_from_slice(&w.rows.get(i, 1).to_le_bytes()));
        }
        bytes.truncate(usize::try_from(len).unwrap_or(0));
        let Ok(key) = String::from_utf8(bytes) else { continue };
        if part(kind, lang, &key) != p {
            continue;
        }
        let terms = entry(view, TERMS, p).map(|e| (0..e.rows.len()).map(|i| e.rows.get(i, 0)).collect()).unwrap_or_default();
        out.push((lang, key, terms));
    }
    out
}

/// Key `key`'s terms as kept (the store's atoms, ascending).
pub fn kept_terms<R: KeptRead + ?Sized>(view: &R, kind: i64, lang: i64, key: &str) -> Option<Vec<i64>> {
    kept_key(view, kind, lang, key).map(|(_, t)| t)
}

fn key_puts(kind: i64, lang: i64, key: &str, upto: i64, terms: &[i64]) -> StoreResult<Vec<KeptPut>> {
    let p = part(kind, lang, key);
    let put = |rel: &str, types: &[u8], n: usize, fill: &mut dyn FnMut(&mut dyn FnMut(i64))| {
        sorted_put(rel, ident(rel), p, types, n, |push| {
            fill(push);
            Ok::<(), StoreError>(())
        })
    };
    let words: Vec<i64> = key.as_bytes().chunks(8).map(|c| { let mut w = [0u8; 8]; w[..c.len()].copy_from_slice(c); i64::from_le_bytes(w) }).collect();
    Ok(vec![
        put(HEAD, b"iiii", 1, &mut |push| [kind, lang, upto, key.len() as i64].into_iter().for_each(&mut *push))?,
        put(KEY, b"ii", words.len(), &mut |push| words.iter().enumerate().for_each(|(i, &w)| { push(i as i64); push(w) }))?,
        put(TERMS, b"s", terms.len(), &mut |push| terms.iter().for_each(|&t| push(t)))?,
    ])
}

/// What tests terms for a group of keys: given the keys (their indices) and the terms' texts, each key's matching
/// texts (their indices).
pub use rerail_store_base::content_store::prefilter::VocabularyTest as Test;

/// Each key's terms where they need no test: a kept key's tested up to the counts' last publication, or a literal
/// set's; for every other key, where its test starts (`None`: every term) and its terms so far; and the literal sets
/// found, to keep.
type Placed = (Vec<Option<Vec<i64>>>, Vec<(usize, Option<i64>, Vec<i64>)>, Vec<(usize, Vec<i64>)>);

impl ContentStore {
    /// Each of `keys`' matching terms of language `lang` (the store's atoms, ascending), brought up to date and kept
    /// (`kind` `REGEX` or `TEXT`); `test` tests terms for keys, `literals` gives a key whose terms are exactly some
    /// literals' (found by value, never tested). Keys all up to date are read without the write lock.
    pub fn vocabulary_terms(&self, kind: i64, lang: i64, keys: &[String], test: Test<'_>, literals: &dyn Fn(&str) -> Option<Vec<String>>) -> StoreResult<Vec<Vec<i64>>> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        {
            let (counts, kept) = (self.kept_space(&kept::space())?, self.kept_space(&space())?);
            let (out, rest, found) = self.place(kind, lang, keys, literals, counts.as_deref(), kept.as_deref())?;
            if rest.is_empty() && found.is_empty() {
                return Ok(out.into_iter().map(Option::unwrap_or_default).collect());
            }
        }
        let path = self.database_path.clone();
        let (out, written) = super::super::atoms::mapped(|| self.immediate_transaction(|t| {
            let open = |space: &[u8; 32]| -> StoreResult<Option<KeptSpace>> { KeptSpace::read(&path, t, space) };
            let (counts, kept) = (open(&kept::space())?, open(&space())?);
            let (mut out, from, found) = self.place(kind, lang, keys, literals, counts.as_ref(), kept.as_ref())?;
            let counts_view = counts.as_ref().map(KeptSpace::view);
            let sums = counts_view.as_ref().map(|v| kept::sums(v));
            let head = sums.as_ref().map_or(0, kept::Sums::head);
            let mut puts: Vec<KeptPut> = Vec::new();
            for (i, terms) in &found {
                puts.extend(key_puts(kind, lang, &keys[*i], head, terms)?);
            }
            // Keys that start from one place are tested together.
            let mut starts: Vec<Option<i64>> = from.iter().map(|f| f.1).collect();
            starts.sort_unstable();
            starts.dedup();
            for start in starts {
                let group: Vec<&(usize, Option<i64>, Vec<i64>)> = from.iter().filter(|f| f.1 == start).collect();
                // The terms counted since: those some tape still holds are tested; the others leave the keys' sets.
                let (candidates, gone): (Vec<i64>, Vec<i64>) = match (start, sums.as_ref()) {
                    (_, None) => Default::default(),
                    (None, Some(s)) => (s.terms(lang), Vec::new()),
                    (Some(upto), Some(s)) => s.terms_since(lang, upto).into_iter().partition(|&t| s.has_term(lang, t)),
                };
                // Their texts: the store's ids as this process's, read where its atoms lie.
                let mut local = candidates.clone();
                self.atoms.to_local(super::super::StoreIds::as_stored(&mut local))?;
                let texts: Vec<String> = local.iter().map(|&a| u32::try_from(a).ok().and_then(|a| self.atoms.with_str(a, str::to_owned)).unwrap_or_default()).collect();
                let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
                let indices: Vec<usize> = group.iter().map(|g| g.0).collect();
                let found = if refs.is_empty() { vec![Vec::new(); indices.len()] } else { test(&indices, &refs) };
                for (g, found) in group.iter().zip(found) {
                    let mut terms: Vec<i64> = g.2.iter().copied().filter(|t| gone.binary_search(t).is_err()).collect();
                    terms.extend(found.into_iter().map(|j| candidates[j as usize]));
                    terms.sort_unstable();
                    terms.dedup();
                    puts.extend(key_puts(kind, lang, &keys[g.0], head, &terms)?);
                    out[g.0] = Some(terms);
                }
            }
            drop(sums);
            drop(counts_view);
            let written = if puts.is_empty() { None } else { super::super::kept_facts::write_space(&path, t, &space(), puts, None)? };
            Ok((out.into_iter().map(Option::unwrap_or_default).collect::<Vec<_>>(), written))
        }))?;
        self.note_kept(&space(), written);
        Ok(out)
    }

    fn place(&self, kind: i64, lang: i64, keys: &[String], literals: &dyn Fn(&str) -> Option<Vec<String>>, counts: Option<&KeptSpace>, kept: Option<&KeptSpace>) -> StoreResult<Placed> {
        let counts_view = counts.map(KeptSpace::view);
        let sums = counts_view.as_ref().map(|v| kept::sums(v));
        let head = sums.as_ref().map_or(0, kept::Sums::head);
        let kept_view = kept.map(KeptSpace::view);
        let mut out: Vec<Option<Vec<i64>>> = vec![None; keys.len()];
        let (mut from, mut found) = (Vec::new(), Vec::new());
        for (i, key) in keys.iter().enumerate() {
            let was = kept_view.as_ref().and_then(|v| kept_key(v, kind, lang, key));
            if let Some((upto, terms)) = &was {
                if *upto >= head {
                    out[i] = Some(terms.clone());
                    continue;
                }
            }
            if let Some(lits) = literals(key) {
                let texts: Vec<&str> = lits.iter().map(String::as_str).collect();
                let present: Vec<super::super::Sym> = self.atoms.lookup_all(&texts)?.into_iter().flatten().collect();
                let mut terms: Vec<i64> = self.atoms.stored(&present)?.into_iter().flatten().map(i64::from).filter(|&t| sums.as_ref().is_some_and(|s| s.has_term(lang, t))).collect();
                terms.sort_unstable();
                terms.dedup();
                out[i] = Some(terms.clone());
                found.push((i, terms));
                continue;
            }
            match was {
                Some((upto, terms)) => from.push((i, Some(upto), terms)),
                None => from.push((i, None, Vec::new())),
            }
        }
        Ok((out, from, found))
    }
}

impl ContentStore {
    /// Each of `expressions`' terms of language `lang` (the store's atoms, ascending): the terms they match, kept
    /// (`prefilter::vocabulary`, kind `REGEX`) and brought up to date. A finite set of exact literals is its literals
    /// (`literal_set`); any other is tried, its probe's byte tests first, on each term.
    pub fn regex_terms(&self, lang: i64, expressions: &[String]) -> StoreResult<Vec<Vec<i64>>> {
        let compiled = expressions
            .iter()
            .map(|expression| {
                let regex = regex::Regex::new(expression).map_err(|e| StoreError::Invalid(format!("regex {expression:?}: {e}")))?;
                Ok((rerail_store_base::probes::TextProbe::compile(expression), regex))
            })
            .collect::<StoreResult<Vec<_>>>()?;
        let test = |keys: &[usize], terms: &[&str]| -> Vec<Vec<u32>> {
            keys.iter()
                .map(|&key| {
                    let (probe, regex) = &compiled[key];
                    (0..terms.len() as u32).filter(|&i| probe.allows(terms[i as usize]) && regex.is_match(terms[i as usize])).collect()
                })
                .collect()
        };
        self.vocabulary_terms(REGEX, lang, expressions, &test, &rerail_store_base::content_store::vocabulary::literal_set)
    }
}

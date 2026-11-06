//! Tokens from a file's token masks: the reference evaluator's `finish` (`reference.rs`), over
//! the masks a stream program computed. The token stage's own lowering is `03-bitlex.md` §3.3.

use super::mask::Mask;
use super::reference::Token;

type Lits = Vec<(Vec<u8>, Option<String>)>;

pub(in crate::code_rule_dsl) fn finish(
    src: &[u8],
    toks: &[(String, Mask, Option<Mask>)],
    munch: &[(String, Mask, Lits)],
    keywords: &[(String, Lits, Vec<Vec<u8>>, bool)],
    skip: &Mask,
    errors: &mut Vec<String>,
) -> Vec<Token> {
    let n = src.len();
    let mut kind_at: Vec<u32> = vec![0; n]; // 1 + index into kinds
    let mut len_at: Vec<u32> = vec![0; n]; // fixed length (munch), or 0
    let mut kinds: Vec<String> = vec![];
    let mut throughs: Vec<Option<&Mask>> = vec![];
    let mut overlap: Option<String> = None;
    for (kind, start, through) in toks {
        kinds.push(kind.clone());
        throughs.push(through.as_ref());
        let k = kinds.len() as u32;
        for i in start.ones_iter() {
            if kind_at[i] != 0 && overlap.is_none() {
                overlap = Some(format!("tokens {} and {} both start at byte {i}", kinds[kind_at[i] as usize - 1], kind));
            }
            kind_at[i] = k;
        }
    }
    for (_, over, lits) in munch {
        let mut runs = vec![];
        over.runs(|a, b| runs.push((a, b)));
        for (a, b) in runs {
            let mut i = a;
            while i < b {
                let best = lits.iter().filter(|(l, _)| src[i..b].starts_with(l)).max_by_key(|(l, _)| l.len());
                let Some((l, alias)) = best else {
                    errors.push(format!("munch: no operator at byte {i}"));
                    break;
                };
                let kind = alias.clone().unwrap_or_else(|| String::from_utf8_lossy(l).into_owned());
                let k = match kinds.iter().position(|x| *x == kind) {
                    Some(p) => p as u32 + 1,
                    None => {
                        kinds.push(kind.clone());
                        throughs.push(None);
                        kinds.len() as u32
                    }
                };
                if kind_at[i] != 0 && overlap.is_none() {
                    overlap = Some(format!("tokens {} and {kind} both start at byte {i}", kinds[kind_at[i] as usize - 1]));
                }
                kind_at[i] = k;
                len_at[i] = l.len() as u32;
                i += l.len();
            }
        }
    }
    if let Some(o) = overlap {
        errors.push(o);
    }
    let mut out = vec![];
    let mut covered = Mask::zeros(n);
    for i in 0..n {
        if kind_at[i] == 0 {
            continue;
        }
        let k = kind_at[i] as usize - 1;
        let mut end = i + 1;
        if len_at[i] > 0 {
            end = i + len_at[i] as usize;
        } else if let Some(t) = throughs[k] {
            while end < n && t.get(end) && kind_at[end] == 0 {
                end += 1
            }
        }
        for j in i..end {
            covered.set(j)
        }
        out.push(Token { kind: kinds[k].clone(), start: i, end, soft: false });
    }
    let gap = covered.or(skip).not();
    if let Some(i) = gap.first() {
        errors.push(format!("byte {i} is in no token"));
    }
    for (kind, words, soft, required) in keywords {
        for t in out.iter_mut().filter(|t| &t.kind == kind) {
            let text = &src[t.start..t.end];
            if let Some((w, alias)) = words.iter().find(|(w, _)| w == text) {
                t.kind = alias.clone().unwrap_or_else(|| String::from_utf8_lossy(w).into_owned());
            } else if soft.iter().any(|w| w == text) {
                t.soft = true;
            } else if *required {
                errors.push(format!("{kind} {:?} is no keyword (byte {})", String::from_utf8_lossy(text), t.start));
            }
        }
    }
    out
}

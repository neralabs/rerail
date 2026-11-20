//! Where a format 16 run's nodes go to the writer's ops at their words, with no row
//! (`Table::direct`): a program whose tier 2 makes the records, written as one function, its
//! calls nested as its containers (`OneFn::stacked`), and every node's key known at its word
//! (`coded`). Also the byte sets' classes a local check reads (`word_form`) and whether rows
//! carry marks (`marks`).

use crate::native_ir::{FlagOp, On, Structure, StructuralOperation as Op};
use crate::program::LocalOp;
use crate::stream_ir::REFUSE;
use std::collections::HashMap;

/// What the analysis reads: the program, its supertypes' classes, and the program as one function.
struct D<'a> {
    native: &'a Structure,
    classes: Vec<u16>,
    one: Option<OneFn>,
}

/// Where every node's key is known at its word and no leaf is deferred or an extra: each node word's key bits from
/// the words after it (`key_after`: its field, its supertypes' classes), by its place. `None` elsewhere.
pub fn direct_keys(native: &Structure) -> Option<Vec<(u32, u32)>> {
    if !crate::native_cpu_stream::calls_on_stack(native) || !fetched(native).ok()? {
        return None;
    }
    let context = native.structural.iter().any(|b| matches!(b.operation, Op::Remember { .. } | Op::Flag { .. } | Op::Push { flag: Some(_), .. }));
    let plain = native.extras.is_empty() && native.readings.provisional.is_empty() && !native.structural.iter().any(|w| matches!(w.operation, Op::Leaf { deferred: true, .. } | Op::Leaf { extra: true, .. }));
    if context || !plain {
        return None;
    }
    let classes = crate::native_cpu_stream::supertype_classes(native).ok()?;
    let mut x = D { native, classes, one: None };
    let one = OneFn::of(native, &|w| plain_succ(&x, w));
    x.one = Some(one);
    let st = stacked(&x);
    if let Some(one) = &mut x.one {
        one.stacked = st;
    }
    coded(&x).then(|| (0..native.structural.len()).map(|pc| key_after(&x, pc)).collect())
}

/// A program written as one function, as simdjson's `walk_document`: a call (`PUSH`) stores its
/// site's number on a stack of its own (`S::ret`, as `is_array[depth]`) and goes on at the rule's
/// first word; a `RETURN` takes the number off and goes on at that site's place, as `scope_end`
/// goes back to the state of the scope it returns to.
struct OneFn {
    /// Each call's place, and the number it stores: calls that every `RETURN` sends to the same
    /// place store one number, as `is_array` is one bit for a scope's two states.
    sites: Vec<(usize, usize)>,
    /// Each `RETURN`'s numbers (those of the calls whose rule holds it), each with its place.
    returns: HashMap<usize, Vec<(usize, usize)>>,
    /// Each call's places after it: where its rule's returns go back to (its target, and the one
    /// after only where a `RETURN` gives `again`).
    after: HashMap<usize, Vec<usize>>,
    /// The calls nested as the containers (`stacked`): a call's number kept in its container's
    /// row, and no `CLOSE` that can find no node open.
    stacked: bool,
}

/// Whether each node's code can be written as it is made (`Facts::coded`): no word edits a
/// node made (`stacked`'s words; a field only on the node made last), and every `NODE_SUPER` and
/// `NODE_FIELD` is reached only from a node's `OPEN` or `LEAF` by words that make no node
/// (`GOTO`, `SPAN`, another of them), so the node's key is its own and the words after it
/// (`key_after`), known when it is made.
fn coded(x: &D<'_>) -> bool {
    let words = &x.native.structural;
    if !x.one.as_ref().is_some_and(|o| o.stacked) || words.iter().any(|w| matches!(w.operation, Op::NodeField { on, .. } if on != On::Here)) {
        return false;
    }
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); words.len()];
    for pc in 0..words.len() {
        for t in plain_succ(x, pc) {
            preds[t].push(pc);
        }
    }
    // A straight word's way back reaches a node's word on every path.
    let mut ok: Vec<Option<bool>> = vec![None; words.len()];
    fn back(x: &D<'_>, preds: &[Vec<usize>], ok: &mut Vec<Option<bool>>, q: usize) -> bool {
        if let Some(v) = ok[q] {
            return v;
        }
        ok[q] = Some(false);
        let words = &x.native.structural;
        let v = !preds[q].is_empty()
            && preds[q].iter().all(|&p| match words[p].operation {
                Op::Open { .. } | Op::Leaf { .. } => plain_succ(x, p).len() == 1,
                Op::Goto { .. } | Op::Span { .. } | Op::NodeSuper { .. } | Op::NodeField { on: On::Here, .. } => plain_succ(x, p).len() == 1 && back(x, preds, ok, p),
                _ => false,
            });
        ok[q] = Some(v);
        v
    }
    (0..words.len()).filter(|&q| matches!(words[q].operation, Op::NodeSuper { .. } | Op::NodeField { .. })).all(|q| back(x, &preds, &mut ok, q))
}

/// The key bits node word `pc`'s key gets from the words after it that make no node (`coded`):
/// each supertype's class (`native_cpu_tape::commit`'s), and a field set there (in `kf`'s
/// place, above its kind). Of a node with its field in the next word (`field_after`), that one
/// is its own already.
fn key_after(x: &D<'_>, pc: usize) -> (u32, u32) {
    let words = &x.native.structural;
    let (mut bits, mut field) = (0u32, 0u32);
    let mut q = pc;
    for _ in 0..words.len() {
        let &[t] = plain_succ(x, q).as_slice() else { break };
        match words[t].operation {
            Op::NodeSuper { kind } => {
                if let Some(j) = x.classes.iter().position(|&c| c == kind) {
                    bits |= 1 << (16 + j);
                }
            }
            Op::NodeField { field: f, on: On::Here } => field = u32::from(f) << 16,
            Op::Goto { .. } | Op::Span { .. } => {}
            _ => break,
        }
        q = t;
    }
    (field, bits)
}

/// Whether the calls nest as the nodes (`OneFn::stacked`): each `PUSH` right after its
/// container's `OPEN` (a supertype or field word between), each `RETURN` right after the `CLOSE`
/// of that container, and each rule's opens and closes counted alike on every way, a call the
/// close of the container before it: 0 at a `RETURN`'s `CLOSE`, and at least 1 at every other
/// `CLOSE` and every `PUSH`. Then that `CLOSE` closes the node the call's `OPEN` made, the
/// container's row holds the call's number until it (its end is not read before), and no
/// `CLOSE` finds no node open. Only words whose nodes this counts: no wrap, fold or drop.
fn stacked(x: &D<'_>) -> bool {
    let words = &x.native.structural;
    let Some(one) = &x.one else { return false };
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); words.len()];
    for pc in 0..words.len() {
        for t in plain_succ(x, pc) {
            preds[t].push(pc);
        }
    }
    let only_from = |pc: usize, q: usize| preds[pc].as_slice() == [q];
    for &(pc, _) in &one.sites {
        let mut q = pc;
        loop {
            if q == 0 || !only_from(q, q - 1) {
                return false;
            }
            q -= 1;
            match words[q].operation {
                Op::Open { .. } => break,
                Op::NodeSuper { .. } | Op::NodeField { .. } => {}
                _ => return false,
            }
        }
    }
    for &r in one.returns.keys() {
        if r == 0 || !only_from(r, r - 1) || !matches!(words[r - 1].operation, Op::Close { .. }) {
            return false;
        }
    }
    // Each rule from its first word (the document's from word 0): a count a word, the same on
    // every way.
    let mut entries: Vec<usize> = vec![0];
    entries.extend(one.sites.iter().map(|&(pc, _)| crate::native_cpu_stream::callee(x.native, pc)));
    entries.sort_unstable();
    entries.dedup();
    for e in entries {
        let mut at: HashMap<usize, i64> = HashMap::from([(e, 0)]);
        let mut work = vec![e];
        while let Some(pc) = work.pop() {
            let c = at[&pc];
            let after = match words[pc].operation {
                Op::Open { .. } => c + 1,
                Op::Close { .. } => {
                    let last = plain_succ(x, pc).iter().any(|&t| matches!(words[t].operation, Op::Return { .. }));
                    if (last && c != 0 && e != 0) || (!last && c < 1) {
                        return false;
                    }
                    c - 1
                }
                Op::Push { .. } if c < 1 => return false,
                Op::Push { .. } => c - 1,
                Op::Token { .. } | Op::Switch { .. } | Op::Leaf { .. } | Op::NodeSuper { .. } | Op::NodeField { .. } | Op::Span { .. } | Op::Check { .. } | Op::Goto { .. } | Op::Return { .. } | Op::Accept | Op::Stop | Op::Empty { .. } => c,
                _ => return false,
            };
            for t in plain_succ(x, pc) {
                match at.get(&t) {
                    Some(&v) if v != after => return false,
                    Some(_) => {}
                    None => {
                        at.insert(t, after);
                        work.push(t);
                    }
                }
            }
        }
    }
    true
}

impl OneFn {
    fn of(native: &Structure, x: &dyn Fn(usize) -> Vec<usize>) -> OneFn {
        let words = &native.structural;
        let sites: Vec<usize> = (0..words.len()).filter(|&pc| matches!(words[pc].operation, Op::Push { .. })).collect();
        let mut returns: HashMap<usize, Vec<(usize, usize)>> = HashMap::new();
        for (k, &pc) in sites.iter().enumerate() {
            let Op::Push { target, .. } = words[pc].operation else { unreachable!() };
            // The rule's words: from its first, its calls not followed (`plain_succ`).
            let mut seen = vec![false; words.len()];
            let mut work = vec![crate::native_cpu_stream::callee(native, pc)];
            while let Some(w) = work.pop() {
                if std::mem::replace(&mut seen[w], true) {
                    continue;
                }
                if let Op::Return { again } = words[w].operation {
                    returns.entry(w).or_default().push((k, usize::from(target) + usize::from(again)));
                }
                work.extend(x(w));
            }
        }
        let mut after: HashMap<usize, Vec<usize>> = HashMap::new();
        for arms in returns.values() {
            for &(k, t) in arms {
                let v = after.entry(sites[k]).or_default();
                if !v.contains(&t) {
                    v.push(t);
                }
            }
        }
        // A call's number: the places every return sends it to, numbered as they first come.
        let mut rets: Vec<usize> = returns.keys().copied().collect();
        rets.sort_unstable();
        let place = |k: usize| -> Vec<Option<usize>> { rets.iter().map(|r| returns[r].iter().find(|&&(s, _)| s == k).map(|&(_, t)| t)).collect() };
        let mut kinds: Vec<Vec<Option<usize>>> = Vec::new();
        let number: Vec<usize> = (0..sites.len())
            .map(|k| {
                let p = place(k);
                kinds.iter().position(|q| *q == p).unwrap_or_else(|| {
                    kinds.push(p);
                    kinds.len() - 1
                })
            })
            .collect();
        for arms in returns.values_mut() {
            for a in arms.iter_mut() {
                a.0 = number[a.0];
            }
            arms.sort_unstable();
            arms.dedup();
        }
        OneFn { sites: sites.into_iter().zip(number).collect(), returns, after, stacked: false }
    }
}

/// A word's successors, a call's returns where the program is one function.
fn plain_succ(x: &D<'_>, pc: usize) -> Vec<usize> {
    let n = x.native.structural.len();
    let v = match x.native.structural[pc].operation {
        Op::Goto { target } => vec![usize::from(target)],
        Op::Switch { table } => x.native.switches[usize::from(table)].all_targets().filter(|&t| t != REFUSE).map(usize::from).collect(),
        Op::Word { test: true, .. } | Op::Mark { .. } | Op::Row { .. } | Op::Flag { op: FlagOp::Test, .. } => vec![pc + 1, pc + 2],
        // In one function (`OneFn`), only the places its rule's returns go back to.
        Op::Push { .. } if x.one.is_some() => x.one.as_ref().and_then(|o| o.after.get(&pc)).cloned().unwrap_or_default(),
        Op::Push { target, .. } => vec![usize::from(target), usize::from(target) + 1],
        Op::Return { .. } | Op::Accept | Op::Stop => vec![],
        _ => vec![pc + 1],
    };
    v.into_iter().filter(|&t| t < n).collect()
}

fn fetched(native: &Structure) -> Result<bool, String> {
    use crate::records::EndRule;
    let Some(ft) = crate::native_cpu_stream::fetch_of(native).map_err(|e| format!("records made in tier 2: {e:?}"))? else { return Ok(false) };
    let split = crate::native_cpu_stream::split_parts(native).map_err(|e| format!("a region tier 2 splits: {e:?}"))?;
    let seq = match split.and_then(|sp| sp.seq.clone()) {
        Some(q) => {
            let es: Vec<u8> = (0..=255u8).filter(|&b| q.stops[usize::from(b) / 64] >> (b % 64) & 1 == 1 && q.xs[usize::from(b) / 64] >> (b % 64) & 1 == 0).collect();
            let &[e0] = es.as_slice() else { return Err("a region read as `parse_string` whose part begins with more than one byte".into()) };
            if q.codes2.is_none() {
                return Err("a region read as `parse_string` with no part codes".into());
            }
            Some((q, e0))
        }
        None => None,
    };
    for b in 0..256usize {
        match ft.rules.get(usize::from(ft.ends[b])) {
            None => {}
            Some(r) if crate::native_cpu_stream::one_byte(&ft, r) => {}
            Some(&EndRule::Read(i)) if ft.seq[usize::from(i)] && seq.is_some() => {}
            Some(&EndRule::Run(_)) => {
                let class = ft.run_class[usize::from(ft.ends[b])].as_ref().ok_or_else(|| format!("byte {b:#04x}: a run whose output is not a byte class"))?;
                // A word with a byte out of the run's class is never the run's token.
                for (w, _) in ft.words.iter().filter(|(w, _)| w.first() == Some(&(b as u8)) && w.iter().all(|&y| class[usize::from(y)] == 1)) {
                    if w.len() > 8 {
                        return Err(format!("byte {b:#04x}: a word ({w:?}) that is not an atom of its run"));
                    }
                }
            }
            Some(r) => return Err(format!("byte {b:#04x}: records made in tier 2 by {r:?}: not yet ahead of time")),
        }
    }
    let _ = seq;
    Ok(true)
}

/// A local check's word form (`program::local_value` over the 64 places of a word, a bit
/// each): the byte sets' classes, tier 1's form (a class a bit of a byte, by two
/// nibble tables, `lo[b & 15] & hi[b >> 4]`, eight classes a pair of tables), each set's table
/// and class bits.
pub(crate) struct WordForm {
    pub tables: Vec<([u8; 16], [u8; 16])>,
    pub class_of: HashMap<usize, (usize, u8)>,
}

pub(crate) fn word_form(ops: &[LocalOp]) -> Result<WordForm, String> {
    let mut tables: Vec<([u8; 16], [u8; 16])> = Vec::new();
    let mut class_of: HashMap<usize, (usize, u8)> = HashMap::new();
    let mut classes = 0usize;
    for (k, op) in ops.iter().enumerate() {
        let LocalOp::Bytes(set) = *op else { continue };
        let member = |b: usize| set[b / 64] >> (b % 64) & 1 == 1;
        // Each high nibble's low nibbles; each distinct nonempty set of them a class.
        let mut pats: Vec<u16> = Vec::new();
        let mut of_hi = [0u16; 16];
        for (h, slot) in of_hi.iter_mut().enumerate() {
            *slot = (0..16).filter(|&l| member(h * 16 + l)).fold(0u16, |m, l| m | 1 << l);
            if *slot != 0 && !pats.contains(slot) {
                pats.push(*slot);
            }
        }
        if pats.len() > 8 {
            return Err(format!("a byte set of {} classes", pats.len()));
        }
        if classes % 8 + pats.len() > 8 {
            classes = classes.div_ceil(8) * 8;
        }
        let mut bits = 0u8;
        for (pi, &pat) in pats.iter().enumerate() {
            let at = classes + pi;
            while tables.len() <= at / 8 {
                tables.push(([0; 16], [0; 16]));
            }
            let (lo, hi) = &mut tables[at / 8];
            let bit = 1u8 << (at % 8);
            for l in 0..16 {
                if pat >> l & 1 == 1 {
                    lo[l] |= bit;
                }
            }
            for h in 0..16 {
                if of_hi[h] == pat {
                    hi[h] |= bit;
                }
            }
            bits |= bit;
        }
        class_of.insert(k, (classes / 8, bits));
        classes += pats.len();
    }
    Ok(WordForm { tables, class_of })
}

/// Whether rows carry marks (`head`): where a field is set on a wrap's inner node, or a reread
/// may meet a dropped row's place (`native_cpu_stream::TableFns` keeps the same).
pub(crate) fn marks(n: &Structure) -> bool {
    n.structural.iter().any(|b| matches!(b.operation, Op::NodeField { on: On::Inner, .. } | Op::Reread { .. }))
}


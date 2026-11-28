//! Python's Rust forms (`.form r python.…`, `facts::vtab::syntax::forms`): node functions of points-to's terms,
//! registered at load (`boot`), read in place on the batch's tapes (decision 0353).

use super::forms::{each_node, Form, FormCall, FormRows, KindSet};
use crate::code_rule_dsl::facts::core::NONE;
use rustc_hash::FxHashMap;

/// Python's forms.
pub const FORMS: &[&dyn Form] = &[&PlaceTerm];

/// `python.place_term SKIP… / SCOPE… reads scope_tm hosted`: `r(n, t)`, node `n`'s term by its place (python/
/// points-to-file's `pt_tm` of a node no definition names, decision 0677): `q@o+l^c`, `q` the term (`scope_tm`'s) of
/// its nearest ancestor of a SCOPE kind, `o` its start from that one's, `l` its length, `c` how many of its ancestors
/// in an unbroken chain up span what it does (`tape.same_span`). Nodes of a SKIP kind, and those `hosted` holds (a
/// guest's), have none. Keyed by `n`.
pub struct PlaceTerm;

impl Form for PlaceTerm {
    fn name(&self) -> &'static str {
        "python.place_term"
    }
    fn version(&self) -> u32 {
        1
    }
    fn columns(&self) -> &'static [&'static str] {
        &["node", "sym"]
    }
    fn keyed(&self) -> &'static [usize] {
        &[0]
    }
    fn rows(&self, call: &FormCall<'_>, _column: usize, out: &mut FormRows) -> Result<(), String> {
        let Some(cut) = call.args.iter().position(|a| *a == "/") else {
            return Err("python.place_term takes the kinds with no place term, `/`, then the scopes' kinds".into());
        };
        let [scope_tm, hosted] = call.reads else { return Err("python.place_term reads the scopes' terms and the guests' nodes".into()) };
        let (mut skip, mut scope) = (KindSet::new(&call.args[..cut]), KindSet::new(&call.args[cut + 1..]));
        // Each scope's terms, their texts found once.
        let atoms: Vec<i64> = (0..scope_tm.len()).map(|r| scope_tm.value(r, 1)).collect();
        let texts = (call.texts)(&atoms)?;
        let mut terms: FxHashMap<i64, Vec<usize>> = FxHashMap::default();
        for r in 0..scope_tm.len() {
            terms.entry(scope_tm.value(r, 0)).or_default().push(r);
        }
        let mut guests: Vec<i64> = (0..hosted.len()).map(|r| hosted.value(r, 0)).collect();
        guests.sort_unstable();
        let mut push = |out: &mut FormRows, n: i64, s: i64, sb: u32, [b, e]: [u32; 2], same: i64| {
            let Some(rows) = terms.get(&s) else { return };
            for &r in rows {
                let t = out.text(format!("{}@{}+{}^{}", texts[r], i64::from(b) - i64::from(sb), e - b, same));
                out.push(&[n, t]);
            }
        };
        if call.keys.is_none() {
            // Every node: one pass down each tape, a parent before its children, each node's nearest scope and
            // count of its span's ancestors carried down from its parent's (this pass's own, let go after it).
            for (i, p) in (&call.batch.docs).into_iter().enumerate() {
                if call.batch.docs.withheld(i) {
                    continue;
                }
                let f = p.f();
                let mut at: Vec<(u32, [u32; 2], u32, i64, bool)> = Vec::with_capacity(p.len() as usize);
                for local in 0..p.len() {
                    let k = f.kind(p.node(local));
                    let span = p.span(local);
                    let up = p.parent(local as usize);
                    let (scope_of, same) = if up == NONE {
                        (NONE, 0)
                    } else {
                        debug_assert!(up < local, "a tape's parent before its children");
                        let (_, uspan, uscope, usame, is_scope) = at[up as usize];
                        (if is_scope { up } else { uscope }, if uspan == span { usame + 1 } else { 0 })
                    };
                    at.push((local, span, scope_of, same, scope.has(k)));
                    if scope_of == NONE || skip.has(k) {
                        continue;
                    }
                    let n = p.global(local);
                    if guests.binary_search(&n).is_ok() {
                        continue;
                    }
                    let (_, sspan, ..) = at[scope_of as usize];
                    push(out, n, p.global(scope_of), sspan[0], span, same);
                }
            }
            return Ok(());
        }
        each_node(call.batch, call.keys, |p, local| {
            let f = p.f();
            if skip.has(f.kind(p.node(local))) {
                return Ok(());
            }
            let n = p.global(local);
            if guests.binary_search(&n).is_ok() {
                return Ok(());
            }
            let [b, e] = p.span(local);
            let (mut at, mut same, mut chain) = (local, 0i64, true);
            loop {
                let up = p.parent(at as usize);
                if up == NONE {
                    return Ok(());
                }
                chain = chain && p.span(up) == [b, e];
                if chain {
                    same += 1;
                }
                if scope.has(f.kind(p.node(up))) {
                    push(out, n, p.global(up), p.span(up)[0], [b, e], same);
                    return Ok(());
                }
                at = up;
            }
        })
    }
}

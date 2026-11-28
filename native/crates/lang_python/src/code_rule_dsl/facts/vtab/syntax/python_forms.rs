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
                    let Some(rows) = terms.get(&p.global(up)) else { return Ok(()) };
                    let sb = p.span(up)[0];
                    for &r in rows {
                        let t = out.text(format!("{}@{}+{}^{}", texts[r], i64::from(b) - i64::from(sb), e - b, same));
                        out.push(&[n, t]);
                    }
                    return Ok(());
                }
                at = up;
            }
        })
    }
}

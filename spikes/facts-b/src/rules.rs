//! Rule-level record types and the per-record / per-group logic of R2 (public-api-triples,
//! function families). The group functions are applied to a complete group by both evaluators;
//! what the gate checks is that the dataflow recomputes exactly the right groups from base + delta.
use crate::derive::World;
use crate::store::Sym;

/// R1 finding: (path, qualname). Findings are keyed by symbol; the line is presentation, attached
/// at delivery from the current content's `def` facts, so edits above a definition do not churn.
pub type R1F = (Sym, Sym);
/// R3 finding: (path, qualname, family).
pub type R3F = (Sym, Sym, Sym);
/// R2 finding: (issue, owner, base_name, family payload, exemplar path, exemplar name).
/// Issues: 0 invalid_mixed_lazy_execution_name, 1 missing_sync, 2 missing_async, 3 missing_lazy.
pub type R2F = (u8, Sym, Sym, Vec<u32>, Sym, Sym);

/// A resolved export root: (export module, export name, symbol path, symbol name, flags).
pub type Root = (Sym, Sym, Sym, Sym, u8);

/// Re-export hops followed when resolving an export target (see report: R2 semantics).
pub const MAX_HOPS: usize = 4;

/// A function-scope surface: (owner, export name, path, name, slot, base_name).
/// slot: 0 async, 1 lazy, 2 sync (SQL string order).
pub type Surface = (Sym, Sym, Sym, Sym, u8, Sym);

pub enum SurfaceOrIssue {
    Surface(Surface),
    Mixed(R2F),
    None,
}

pub fn surface(w: &mut World, r: &Root) -> SurfaceOrIssue {
    let &(owner, ename, path, name, flags) = r;
    if flags & 1 == 0 {
        return SurfaceOrIssue::None; // not a function: class roots belong to method scope (out of scope)
    }
    let n = w.store.syms.str(name).to_string();
    if n.starts_with('_') || w.info(path).internal {
        return SurfaceOrIssue::None;
    }
    let lower = n.to_lowercase();
    let mixed = lower.contains("lazy") && (lower.contains("async") || lower.contains("sync"));
    if mixed {
        return SurfaceOrIssue::Mixed((0, owner, name, vec![ename], path, name));
    }
    let base = ["_lazy", "_async", "_sync"].iter().find_map(|s| n.strip_suffix(s)).unwrap_or(&n).to_string();
    let slot = if n.ends_with("_lazy") {
        1
    } else if n.ends_with("_async") || flags & 2 != 0 {
        0
    } else {
        2
    };
    let base = w.store.syms.intern(&base);
    SurfaceOrIssue::Surface((owner, ename, path, name, slot, base))
}

/// A family of one owner: (base_name, implementations [(slot, path, name)], exemplar (path, name)).
/// Keyed downstream by (base_name, implementations) for the cross-owner dedupe.
pub type Family = (Sym, Vec<(u8, Sym, Sym)>, (Sym, Sym));

fn eligible(owner: &str, base: &str, ex_path: &str, sync: bool, asy: bool, lazy: bool) -> bool {
    let txn_owner = ["ReadTransaction", "WriteTransaction", "LazyVersionedReadTransaction", "LazyVersionedWriteTransaction"];
    let excluded = (["plan", "query", "query_unfiltered"].contains(&base)
        && ["l2050_transaction/transaction.py", "l7200_versioned_transactions/versioned_transactions.py"].contains(&ex_path)
        && txn_owner.contains(&owner))
        || (owner == "CacheTransaction" && (base == "commit" || base == "rollback"))
        || (owner == "BaseMigration" && base == "rollback")
        || (base == "execute_ddl" && (owner == "SchemaTransaction" || owner == "AsyncSchemaTransaction" || ex_path == "l2000_connection/schema_transaction.py"))
        || (owner.ends_with("Connection") && base == "table_exists")
        || (!sync
            && !asy
            && lazy
            && (base.to_lowercase().contains("bootstrap")
                || owner.to_lowercase().contains("bootstrap")
                || ["check_ddl_lockdown", "column_exists", "physical_table_exists_many", "seal_table_exists_cache"].contains(&base)))
        || ["LazyVersionedReadTransaction", "LazyVersionedWriteTransaction", "SyncSessionHandle", "AsyncSessionHandle", "SyncVersionHandle", "AsyncVersionHandle"]
            .contains(&owner)
        || owner_handle_regex(owner);
    !excluded
}

/// `^(?:Sync|Async|Lazy).*(?:Transaction|SchemaTransaction|SessionHandle|VersionHandle)$`
fn owner_handle_regex(o: &str) -> bool {
    (o.starts_with("Sync") || o.starts_with("Async") || o.starts_with("Lazy"))
        && ["Transaction", "SessionHandle", "VersionHandle"].iter().any(|s| {
            o.ends_with(s) && {
                let pre = if o.starts_with("Sync") { 4 } else if o.starts_with("Async") { 5 } else { 4 };
                o.len() >= pre + s.len()
            }
        })
}

/// Slots, family groups and eligibility of one owner's non-mixed surfaces (Jeremy's stages
/// slots, family_groups, eligible_groups with executionOnly = false).
pub fn owner_families(w: &World, owner: Sym, surfaces: &[Surface]) -> Vec<Family> {
    let s = |x: Sym| w.store.syms.str(x);
    use std::collections::BTreeMap;
    // (base, slot) -> rows
    let mut parts: BTreeMap<(Sym, u8), Vec<&Surface>> = BTreeMap::new();
    for r in surfaces {
        debug_assert_eq!(r.0, owner);
        parts.entry((r.5, r.4)).or_default().push(r);
    }
    // per (base): [(slot, chosen row, first_ordinal)]
    let mut groups: BTreeMap<Sym, Vec<(u8, &Surface, &str)>> = BTreeMap::new();
    for ((base, slot), rows) in &parts {
        let first = rows.iter().map(|r| s(r.1)).min().unwrap();
        // max ordinal; ties (one export name resolving to several symbols) broken by (path, name)
        let chosen = rows.iter().max_by(|a, b| (s(a.1), s(a.2), s(a.3)).cmp(&(s(b.1), s(b.2), s(b.3)))).unwrap();
        groups.entry(*base).or_default().push((*slot, chosen, first));
    }
    let mut out = Vec::new();
    let owner_s = s(owner);
    for (base, slots) in groups {
        let ex = slots.iter().min_by(|a, b| (a.2, s(a.1 .2), s(a.1 .3)).cmp(&(b.2, s(b.1 .2), s(b.1 .3)))).unwrap().1;
        let has = |k: u8| slots.iter().any(|x| x.0 == k);
        if !eligible(owner_s, s(base), s(ex.2), has(2), has(0), has(1)) {
            continue;
        }
        let mut imp: Vec<(u8, Sym, Sym)> = slots.iter().map(|x| (x.0, x.1 .2, x.1 .3)).collect();
        imp.sort_unstable_by(|a, b| (a.0, s(a.1), s(a.2)).cmp(&(b.0, s(b.1), s(b.2))));
        out.push((base, imp, (ex.2, ex.3)));
    }
    out
}

/// Cross-owner dedupe (Jeremy's `families`: first owner by name per (base_name,
/// implementations)) and the issues of the surviving family.
pub fn family_issues(w: &World, base: Sym, imp: &[(u8, Sym, Sym)], owners: &[(Sym, (Sym, Sym))]) -> Vec<R2F> {
    let s = |x: Sym| w.store.syms.str(x);
    let (owner, ex) = owners.iter().min_by(|a, b| (s(a.0), s(a.1 .0), s(a.1 .1)).cmp(&(s(b.0), s(b.1 .0), s(b.1 .1)))).unwrap();
    let has = |k: u8| imp.iter().any(|x| x.0 == k);
    let payload: Vec<u32> = imp.iter().flat_map(|&(k, p, n)| [k as u32, p, n]).collect();
    let mut out = Vec::new();
    let mk = |code: u8| (code, *owner, base, payload.clone(), ex.0, ex.1);
    if !has(2) && has(0) {
        out.push(mk(1));
    }
    if !has(0) && has(2) {
        out.push(mk(2));
    }
    if !has(1) {
        out.push(mk(3));
    }
    out
}

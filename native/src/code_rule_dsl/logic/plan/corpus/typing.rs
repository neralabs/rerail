//! The corpus runner's typing inputs and outputs (O42; 03 §13).
//!
//! - **In:** `rerail.types/v1` declarations (`typing::declare`) and the dependency summaries the corpus's imports
//!   demand (`typing::deps`) become `declared_ret(q, t)` facts of the program, one per member name of each
//!   declared return. The program's rule uses them only for names the corpus does not define (fills gaps only).
//!   User declarations come first; summaries fill what they leave.
//! - **Out:** each node's `type_of` names are one union term (`typing::terms`), written with the term tables
//!   (`x.node_type(n, tid)`, `x.type_term`, `x.type_arg`). Declarations that differ from what the corpus derives
//!   for a name it defines are reported, and so are modules whose summaries are pending.
//! - **Summaries:** with `Typing::summarize`, the corpus is a site directory. Each installed module's functions,
//!   with the return names the program derived and the module's `from` imports, are written to the cache under
//!   the module file's identity.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::super::{Error, Result};
use crate::code_rule_dsl::typing::declare;
use crate::code_rule_dsl::typing::deps::{self, Cache, Installed, State, Universe};
use crate::code_rule_dsl::typing::terms::{Ctor, Terms};
use crate::content_store::ContentStore;
use rusqlite::Connection;

/// The typing inputs of a corpus run.
#[derive(Clone, Debug, Default)]
pub(crate) struct Typing {
    /// `rerail.types/v1` files.
    pub(crate) declarations: Vec<PathBuf>,
    /// An installed site directory, and the summary cache, where imports the corpus does not define are demanded.
    pub(crate) site: Option<(PathBuf, PathBuf)>,
    /// Summarize the corpus (a site directory) into this cache.
    pub(crate) summarize: Option<PathBuf>,
    /// Modules per package (`Universe::demand`); 0 is the default, 64.
    pub(crate) budget: usize,
    /// The language of the files read: `Some(true)` the `.rs` files, `Some(false)` the `.py` files; `None` the more
    /// numerous (`corpus_files`).
    pub(crate) rust: Option<bool>,
}

/// What the typing inputs and outputs did.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct TypingReport {
    /// `declared_ret` facts added.
    pub(crate) declared: usize,
    /// Demanded modules whose summaries are pending (requested), by module.
    pub(crate) pending: Vec<String>,
    /// (name, the corpus's return names, the declared ones) where they differ.
    pub(crate) conflicts: Vec<(String, String, String)>,
    /// Nodes given a term, and the terms.
    pub(crate) typed_nodes: usize,
    pub(crate) terms: usize,
    /// Summaries written.
    pub(crate) summaries: usize,
}

/// A declared callable's return member names: closed unions of named types (and literals' types) only; ⊤ or an
/// open union gives none.
fn ret_names(terms: &Terms, callable: u32, text: &dyn Fn(u32) -> String) -> Option<BTreeSet<String>> {
    let t = terms.get(callable);
    if t.ctor != Ctor::Callable {
        return None;
    }
    let (members, open) = terms.members(*t.args.last()?);
    if open {
        return None;
    }
    members.into_iter().map(|m| (terms.get(m).ctor == Ctor::Named).then(|| text(terms.get(m).name))).collect()
}

/// The declarations, merged (user first, then the demanded summaries), as program facts; and the report so far.
pub(crate) fn facts(typing: &Typing, root: &Path, files: &[String], report: &mut TypingReport) -> Result<(String, BTreeMap<String, BTreeSet<String>>)> {
    let mut names: Vec<String> = Vec::new();
    let mut intern = |s: &str| match names.iter().position(|x| x == s) {
        Some(i) => i as u32,
        None => {
            names.push(s.to_owned());
            (names.len() - 1) as u32
        }
    };
    let mut terms = Terms::default();
    let mut merged: BTreeMap<String, u32> = BTreeMap::new();
    for path in &typing.declarations {
        let text = std::fs::read_to_string(path).map_err(|e| Error::Plan(format!("{}: {e}", path.display())))?;
        let d = declare::parse(&text, &mut terms, &mut intern).map_err(|e| Error::Plan(format!("{}:{}: {}", path.display(), e.line, e.message)))?;
        merged = declare::fill(&merged, &d).0;
    }
    if let Some((site, cache)) = &typing.site {
        let installed = Installed::scan(site, None).map_err(|e| Error::Plan(format!("{}: {e}", site.display())))?;
        let mut imports = Vec::new();
        for f in files.iter().filter(|f| f.ends_with(".py")) {
            if let Ok(text) = std::fs::read_to_string(root.join(f)) {
                imports.extend(deps::imports(&text, false));
            }
        }
        let roots = deps::roots(&imports);
        let roots: Vec<&str> = roots.iter().map(String::as_str).collect();
        let budget = if typing.budget == 0 { 64 } else { typing.budget };
        let universe = Universe::demand(&roots, &installed, &Cache { dir: cache.clone() }, budget, &mut terms, &mut intern);
        for (module, state) in universe.modules {
            match state {
                State::Ready(d) => merged = declare::fill(&merged, &d).0,
                State::Pending => report.pending.push(module),
                State::Top => {}
            }
        }
    }
    let text = |s: u32| names[s as usize].clone();
    let mut declared = BTreeMap::new();
    let mut out = String::new();
    for (q, callable) in merged {
        let Some(rets) = ret_names(&terms, callable, &text) else { continue };
        for t in &rets {
            out.push_str(&format!("declared_ret({}, {}).\n", crate::code_rule_dsl::logic::rr::quote(&q), crate::code_rule_dsl::logic::rr::quote(t)));
            report.declared += 1;
        }
        declared.insert(q, rets);
    }
    Ok((out, declared))
}

/// After the run: `type_of` as terms, the conflicts, and (with `summarize`) the summaries.
#[allow(clippy::too_many_arguments)]
pub(crate) fn outputs(
    typing: &Typing,
    db: &Connection,
    store: &ContentStore,
    root: &Path,
    type_of: Option<&Vec<String>>,
    returns: &[Vec<i64>],
    declared: &BTreeMap<String, BTreeSet<String>>,
    fn_q: Option<&dyn crate::code_rule_dsl::facts::vtab::Source>,
    report: &mut TypingReport,
) -> Result<()> {
    let text = |a: i64| -> Result<String> { store.atom_value(a as u32).map_err(|e| Error::Plan(e.to_string()))?.ok_or_else(|| Error::Plan(format!("no atom {a}"))) };
    // Each node's names, one union term.
    if let Some(rows) = type_of {
        let mut per: BTreeMap<i64, BTreeSet<String>> = BTreeMap::new();
        for row in rows {
            if let Some((n, t)) = row.split_once('\t')
                && let Ok(n) = n.parse::<i64>()
            {
                per.entry(n).or_default().insert(t.to_owned());
            }
        }
        let mut terms = Terms::default();
        let all: Vec<&str> = per.values().flatten().map(String::as_str).collect::<BTreeSet<_>>().into_iter().collect();
        let atoms = store.intern_atoms(&all).map_err(|e| Error::Plan(e.to_string()))?;
        let atom: BTreeMap<&str, u32> = all.iter().copied().zip(atoms).collect();
        // One transaction for the rows: `x` is on disk, and a statement on its own commits (and syncs) by itself.
        let tx = db.is_autocommit().then(|| db.unchecked_transaction()).transpose()?;
        db.execute_batch("DROP TABLE IF EXISTS x.node_type; CREATE TABLE x.node_type(n INTEGER PRIMARY KEY, tid INTEGER)")?;
        let mut ins = db.prepare("INSERT INTO x.node_type VALUES (?1, ?2)")?;
        for (n, names) in &per {
            let members: Vec<u32> = names.iter().map(|s| terms.named(atom[s.as_str()], &[])).collect();
            let t = terms.union(&members, false);
            ins.execute([*n, i64::from(t)])?;
        }
        drop(ins);
        db.execute_batch("DROP TABLE IF EXISTS x.type_term; DROP TABLE IF EXISTS x.type_arg")?;
        terms.write(db, "x")?;
        if let Some(tx) = tx {
            tx.commit()?;
        }
        report.typed_nodes = per.len();
        report.terms = terms.len();
    }
    // The corpus's functions: name → the return names it derived, from the export `fn_q` (`exports`), when the
    // program has one.
    let Some(fn_q) = fn_q else {
        return Ok(());
    };
    let mut rets: BTreeMap<i64, BTreeSet<String>> = BTreeMap::new();
    for r in returns {
        rets.entry(r[0]).or_default().insert(text(r[1])?);
    }
    let mut functions: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let pairs: Vec<(i64, i64)> = (0..fn_q.len()).map(|r| (fn_q.value(r, 0), fn_q.value(r, 1))).collect();
    for (q, f) in pairs {
        functions.entry(text(q)?).or_default().extend(rets.get(&f).cloned().unwrap_or_default());
    }
    for (q, d) in declared {
        if let Some(have) = functions.get(q)
            && have != d
        {
            let join = |s: &BTreeSet<String>| s.iter().cloned().collect::<Vec<_>>().join(" | ");
            report.conflicts.push((q.clone(), join(have), join(d)));
        }
    }
    let Some(cache) = &typing.summarize else { return Ok(()) };
    let installed = Installed::scan(root, None).map_err(|e| Error::Plan(format!("{}: {e}", root.display())))?;
    let cache = Cache { dir: cache.clone() };
    let mut by_module: BTreeMap<&str, BTreeMap<String, BTreeSet<String>>> = BTreeMap::new();
    for (q, r) in &functions {
        if let Some((m, name)) = q.rsplit_once('.') {
            by_module.entry(m).or_default().insert(name.to_owned(), r.clone());
        }
    }
    for (module, id) in &installed.modules {
        let source = std::fs::read_to_string(root.join(&id.path)).unwrap_or_default();
        let reexported: Vec<(String, Vec<String>)> = deps::imports(&source, true).into_iter().filter(|(_, names)| !names.is_empty()).collect();
        let empty = BTreeMap::new();
        let fns = by_module.get(module.as_str()).unwrap_or(&empty);
        cache.put(id, &deps::summary(module, fns, &reexported)).map_err(|e| Error::Plan(e.to_string()))?;
        report.summaries += 1;
    }
    Ok(())
}

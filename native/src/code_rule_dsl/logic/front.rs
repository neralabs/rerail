//! The logic's text syntax: tokens, the AST, stratification and pruning (03 §12; 05 Phase 6 row 2).
//!
//! A port of the front end of Typing-support's `rdl.py` (T, `front/rdl.py` beside this file until the port is
//! done): `tokens`, `parse`, the AST classes, `strata`/`tarjan` and `prune`, with T's grammar. Added for §12:
//! - lattices declared on any relation, `.decl r(k: t; v: L) merge @join widen k`, besides T's `.lattice`;
//! - `min` and `max` aggregates, `$m = min $v : { atom }`, besides `count`;
//! - the matcher atom `ast($D, $P, { … })`, whose matcher text is kept for the transducer lowering (§3), and which
//!   exports matcher metavariables as variables, `ast($D, $P, { … }, NAME: $n)`, each bound to what it captured (a
//!   node, or the text of a regex group or a transform); a match that leaves one unbound gives no row;
//! - JSON strings, and `.policy { … }` for lint-rule documents (`yaml`, the second syntax of B12);
//! - named errors for aggregation in recursion (unless the head is a monotone lattice) and unbounded lattices;
//! - unbounded value cycles (`cycles`, O11 option b): a recursion that creates values must be bounded by the data.
//!
//! §12's other named errors are not here: a non-interned cross-document join needs column types, and the cost gate
//! is the planner's (row 5).
//!
//! Tree structure is never a relation: `nearest($n, $s, set)` is a built-in over the tape (T's `rdl.py` docstring).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub mod caches;
pub use rerail_rr_syntax::code_rule_dsl::logic::front::name;
pub use name::{List, Lists, Name};
pub mod cycles;
pub mod graph;
pub use rerail_rr_syntax::code_rule_dsl::logic::front::ast;
pub use ast::{AXIS, AggOp, Algo, Branded, Class, DESCENDANTS, EVERY_KIND, FEW, Fan, GLOBAL, LIBRARY, LatticeDecl, LayoutFact, NEAREST, Native, OPTION_GLOB, QUALIFIERS, RESERVED, SCOPE_NOT, Shape, ShapeWait, Size as LayoutSize, SpanIn, Value, Values as LayoutValues, add_layout, axis, carried_record, globals, is_axis, is_transduced, layout_facts, readers_of_heads, record_copy, shape_base};

use ast::PUNCT;

pub use ast::operator;

// T's `repr`s, which its transformed programs are compared in.
/// `scopes` with each option entry's values (`OPTION_GLOB`) added beside it, from the run's `options` as
/// `option`/`option_item` read them: a directory under `dirs`, a glob under `include` or `exclude`. An option the run
/// does not set adds nothing, and its entry still holds no path (an entry whose only globs are such reads none).
pub fn resolve_scopes(scopes: &Option<Vec<(Vec<String>, Vec<String>)>>, options: &serde_json::Value) -> Option<Vec<(Vec<String>, Vec<String>)>> {
    let mut rows = crate::code_rule_dsl::facts::repository::Rows::default();
    crate::code_rule_dsl::facts::repository::options(options, &mut rows);
    let values = |name: &str| -> Vec<String> {
        rows.option_item.iter().filter(|(n, _, _)| n == name).map(|(_, _, v)| v.clone()).chain(rows.option.iter().filter(|(n, _)| n == name).map(|(_, v)| v.clone())).collect()
    };
    let resolve = |globs: &Vec<String>| -> Vec<String> {
        let mut out = Vec::new();
        for g in globs {
            out.push(g.clone());
            let Some((form, name)) = g.strip_prefix(OPTION_GLOB).and_then(|r| r.split_once(':')) else { continue };
            for v in values(name) {
                out.push(if form == "dirs" { format!("{}/**", v.trim_end_matches('/')) } else { v });
            }
        }
        out
    };
    scopes.as_ref().map(|entries| entries.iter().map(|(within, out)| (resolve(within), resolve(out))).collect())
}

pub fn in_doc_scopes(scopes: &Option<Vec<(Vec<String>, Vec<String>)>>, path: &str) -> bool {
    let glob = |g: &String| crate::code_rule_dsl::logic::strings::test("glob", path, g);
    scopes.as_ref().is_none_or(|entries| entries.iter().any(|(within, out)| (within.is_empty() || within.iter().any(glob)) && !out.iter().any(glob)))
}

#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    Syntax(String),
    /// A relation depends negatively on another in its own component.
    NotStratifiable { message: String, component: BTreeSet<String>, negated: String },
    /// An aggregate reads its own component, and the head is not a lattice the aggregate is monotone in.
    AggregationInRecursion { head: String, aggregated: String, component: BTreeSet<String> },
    /// A lattice of unbounded height with no `widen`.
    UnboundedLattice { relation: String },
    /// A demanded relation is used with its key unbound: its demand would be everything (magic sets).
    UnboundDemand { message: String, relation: String },
    /// A component creates values (`@cat`, …) the data does not bound (`cycles`): `rule` is one it could not
    /// bound.
    UnboundedValueCycle { component: BTreeSet<String>, rule: String },
    /// `.input r` where no `.decl` gives `r`'s columns (nor core.rerail's, for `.rerail`).
    UndeclaredInput { relation: String },
}

/// The `.rerail` parser's error (`rerail_rr_syntax`) is a syntax error.
impl From<rerail_rr_syntax::Error> for Error {
    fn from(e: rerail_rr_syntax::Error) -> Self {
        match e {
            rerail_rr_syntax::Error::Syntax(m) => Error::Syntax(m),
        }
    }
}

impl Error {
    /// The component and relation of a stratification failure (a negation or an aggregation inside its own
    /// component): T raised `NotStratifiable` for both, and magic sets recovers from either the same way.
    pub fn unstratified(&self) -> Option<(&BTreeSet<String>, &str)> {
        match self {
            Error::NotStratifiable { component, negated, .. } => Some((component, negated)),
            Error::AggregationInRecursion { component, aggregated, .. } => Some((component, aggregated)),
            _ => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Syntax(m) | Error::NotStratifiable { message: m, .. } | Error::UnboundDemand { message: m, .. } => {
                f.write_str(m)
            }
            Error::AggregationInRecursion { head, aggregated, .. } => {
                write!(f, "aggregation in recursion: {head} aggregates {aggregated} in its own component")
            }
            Error::UnboundedLattice { relation } => {
                write!(f, "unbounded lattice: {relation} has no finite height and no `widen k`")
            }
            Error::UnboundedValueCycle { component, rule } => {
                let component = component.iter().cloned().collect::<Vec<_>>().join(", ");
                write!(f, "unbounded value cycle: {{{component}}} creates values the data does not bound ({rule})")
            }
            Error::UndeclaredInput { relation } => write!(f, ".input {relation}: no .decl declares {relation}'s columns"),
        }
    }
}

// A test of a transform written for preorder ids (T's goldens) parses programs with their node comparisons as written,
// on its own thread.
#[cfg(any(test, feature = "test-hooks"))]
thread_local! {pub static KEEP_NODE_COMPARISONS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };}

/// The relation of each node's preorder key (`syntax`'s `node_order(n, k)`).
const NODE_ORDER: &str = "node_order";

/// Whether node comparisons are made by `node_order` (`order_nodes`, and its version form): under node ids per
/// definition, unless a test keeps them as written, or a measurement compares the ids themselves.
pub fn orders_nodes() -> bool {
    if !crate::code_rule_dsl::facts::vtab::syntax::definition_ids() {
        return false;
    }
    #[cfg(any(test, feature = "test-hooks"))]
    if KEEP_NODE_COMPARISONS.get() {
        return false;
    }
    // A measurement, never a mode to run in: comparisons of the node ids themselves, which an edit leaves alone outside
    // the granules it touches but which are not in preorder. What a stable order key could save at most.
    if std::env::var_os("RERAIL_ORDER_RAW").is_some() {
        crate::code_rule_dsl::logic::profile::decision("node comparisons (RERAIL_ORDER_RAW)", || "node ids compared as they are, not by node_order: a measurement only".into());
        return false;
    }
    true
}

/// The relation of each node's preorder key, as `order_nodes` reads it.
pub const NODE_ORDER_REL: &str = NODE_ORDER;

/// `finish`'s checks, of the program its rewrites made, read on its version: no computed term in a body atom, bounded
/// lattices, every input declared, bounded value cycles, the caches' form.
pub fn finish_checks_on(v: &crate::code_rule_dsl::logic::program_image::Version) -> Result<(), Error> {
    finish_checks_given(v, None)
}

/// `finish_checks_on` of a program lowered without the rules of the `library` it reads (`rr::Text::lower_in`): its
/// value cycles are bounded by what the library's relations guarantee there (`cycles::check_given`).
pub fn finish_checks_given(v: &crate::code_rule_dsl::logic::program_image::Version, library: Option<&crate::code_rule_dsl::logic::program_image::Version>) -> Result<(), Error> {
    use crate::code_rule_dsl::logic::program_image::fields::ProgramFields;
    use crate::code_rule_dsl::logic::program_image::handles::{Lit, TermRef};
    for rule in v.rules() {
        // A computed term is a value, not a pattern: an atom never matches on one (the backends would drop it).
        if let Some(a) = rule.body().filter_map(|l| if let Lit::Atom(a) = l.view() { Some(a) } else { None }).find(|a| a.args().any(|t| matches!(t, TermRef::Call(..)))) {
            return Err(Error::Syntax(format!(
                "computed term in body atom {} of the rule for {}: bind it with `v = @f(...)`",
                v.rel_name(a.rel()),
                v.rel_name(rule.head().rel())
            )));
        }
    }
    for (relation, decl) in ProgramFields::lattice(v) {
        if !decl.bounded() {
            return Err(Error::UnboundedLattice { relation });
        }
    }
    // An input's columns are its declaration's: the planner reads them.
    if let Some(relation) = v.input_names().into_iter().find(|rel| !v.declares(rel)) {
        return Err(Error::UndeclaredInput { relation });
    }
    let checking = crate::code_rule_dsl::logic::profile::span(|| "check the program's cycles and caches".to_owned());
    match library {
        Some(library) => cycles::check_given(v, library)?,
        None => cycles::check(v)?,
    };
    // A cache is two rules (decision 0103): a broken form refuses the program.
    caches::caches(v)?;
    drop(checking);
    Ok(())
}

/// `declared`, of a version: each rule's reads declared, and each atom with its relation's columns.
pub fn declared_on(v: &crate::code_rule_dsl::logic::program_image::Version) -> Result<(), Error> {
    use crate::code_rule_dsl::logic::program_image::handles::Lit;
    for rule in v.rules() {
        let head = v.rel_name(rule.head().rel());
        let atoms = || {
            rule.body().filter_map(|l| match l.view() {
                Lit::Atom(a) | Lit::Agg { atom: a, .. } => Some(a),
                _ => None,
            })
        };
        for a in atoms() {
            read_declared(v, v.rel_name(a.rel()), head)?;
        }
        for a in std::iter::once(rule.head()).chain(atoms()) {
            columns_declared(v, v.rel_name(a.rel()), a.arity(), head)?;
        }
    }
    Ok(())
}

/// A read of `rel` in a rule for `head` names a declared relation. The import components' relations (`rr_cx_reach`,
/// decision 0076) are added by the run to a program that reads them: declared there.
pub fn read_declared(prog: &(impl crate::code_rule_dsl::logic::program_image::fields::ProgramFields + ?Sized), rel: &str, head: &str) -> Result<(), Error> {
    if !prog.declares(rel) && !is_transduced(rel) && !rel.starts_with("rr_cx_") && !matches!(rel, "match_all" | "match_group") {
        return Err(Error::Syntax(format!("relation {rel} is read by the rule for {head} and not declared: declare it with `.decl`")));
    }
    Ok(())
}

/// An atom of `rel` with `arity` columns, in a rule for `head`, has the columns `rel` declares: a read of another
/// arity names some other relation (a rule's own `field(n)` beside the core `field(p, f, c)`), which no plan can join.
pub fn columns_declared(prog: &(impl crate::code_rule_dsl::logic::program_image::fields::ProgramFields + ?Sized), rel: &str, arity: usize, head: &str) -> Result<(), Error> {
    if let Some(cols) = prog.columns_of(rel)
        && cols.len() != arity
    {
        return Err(Error::Syntax(format!("relation {rel} is declared with {} columns and read with {arity} in the rule for {head}", cols.len())));
    }
    Ok(())
}

/// Dependency edges: head → {(dependency, negative)}; an aggregate's is negative, a gated negation's is not.
pub type Edges = BTreeMap<String, BTreeSet<(String, bool)>>;

/// `components_of` of a dependency graph read from a version (`program_image::analyses::components_by_name`): the
/// derived relations' names, sorted; each one's dependencies by their places among them, with whether negative, sorted
/// and each once; and the non-monotone aggregation edges. The components, their order and the error are the tree's.
pub fn components_of_graph(names: Vec<&str>, succ: Vec<Vec<(u32, bool)>>, aggs: rustc_hash::FxHashSet<(u32, u32)>) -> Result<Vec<BTreeSet<String>>, Error> {
    Graph { names, succ, aggs }.components()
}

/// The dependency graph of `strata` on numbers: the derived relations in their order, each one's dependencies among
/// them (with whether negative, sorted as `Edges` sorts them), and the non-monotone aggregation edges. The search and the
/// stratification check run on it; the edges by name are made only for a caller that reads
/// them (`edges`). Each edge as a pair of names in a sorted map was most of the strata's cost.
struct Graph<'p> {
    names: Vec<&'p str>,
    succ: Vec<Vec<(u32, bool)>>,
    aggs: rustc_hash::FxHashSet<(u32, u32)>,
}

impl<'p> Graph<'p> {


    /// The components, dependencies first: an error if a negation or a non-monotone aggregate closes a cycle.
    fn components(&self) -> Result<Vec<BTreeSet<String>>, Error> {
        let sccs = tarjan_ids(&self.succ);
        match self.first_unstratified(&sccs, true).into_iter().next() {
            Some(e) => Err(e),
            None => Ok(self.named(&sccs)),
        }
    }

    fn named(&self, sccs: &[Vec<u32>]) -> Vec<BTreeSet<String>> {
        sccs.iter().map(|scc| scc.iter().map(|&r| self.names[r as usize].to_owned()).collect()).collect()
    }

    /// The first error of each component a negative edge closes a cycle in (`first`: only the first component's).
    fn first_unstratified(&self, sccs: &[Vec<u32>], first: bool) -> Vec<Error> {
        let mut at = vec![usize::MAX; self.names.len()];
        for (i, scc) in sccs.iter().enumerate() {
            for &r in scc {
                at[r as usize] = i;
            }
        }
        let mut out = Vec::new();
        for (i, scc) in sccs.iter().enumerate() {
            // In the order of names, as the components by name are read.
            let mut members = scc.clone();
            members.sort_unstable();
            'scc: for &v in &members {
                for &(w, neg) in &self.succ[v as usize] {
                    if !neg || at[w as usize] != i {
                        continue;
                    }
                    let component: BTreeSet<String> = scc.iter().map(|&r| self.names[r as usize].to_owned()).collect();
                    let (vn, wn) = (self.names[v as usize].to_owned(), self.names[w as usize].to_owned());
                    unstratified_trace(&vn, &wn, &component);
                    out.push(if self.aggs.contains(&(v, w)) {
                        Error::AggregationInRecursion { head: vn, aggregated: wn, component }
                    } else {
                        Error::NotStratifiable { message: format!("not stratifiable: {vn} depends negatively on {wn} in its own component"), component, negated: wn }
                    });
                    if first {
                        return out;
                    }
                    break 'scc;
                }
            }
        }
        out
    }
}

/// `RERAIL_STRATA_COUNTS` (a diagnostic): a refused component's members, which the error's message leaves out.
pub fn unstratified_trace(head: &str, dep: &str, component: &BTreeSet<String>) {
    if std::env::var_os("RERAIL_STRATA_COUNTS").is_some() {
        eprintln!("RERAIL_STRATA_COUNTS unstratified (magic sets may recover): {head} -> {dep} in a component of {}: {}", component.len(), component.iter().cloned().collect::<Vec<_>>().join(" "));
    }
}


/// Strongly connected components on numbers (`tarjan`), dependencies first; each component's members in the order
/// they leave the stack.
fn tarjan_ids(succ: &[Vec<(u32, bool)>]) -> Vec<Vec<u32>> {
    const NONE: u32 = u32::MAX;
    let n = succ.len();
    let (mut index, mut low, mut on) = (vec![NONE; n], vec![0u32; n], vec![false; n]);
    let (mut stack, mut sccs, mut counter) = (Vec::<u32>::new(), Vec::new(), 0u32);
    for root in 0..n as u32 {
        if index[root as usize] != NONE {
            continue;
        }
        let mut frames: Vec<(u32, usize)> = vec![(root, 0)];
        index[root as usize] = counter;
        low[root as usize] = counter;
        counter += 1;
        stack.push(root);
        on[root as usize] = true;
        while let Some((v, at)) = frames.last_mut() {
            let v = *v;
            if let Some(&(w, _)) = succ[v as usize].get(*at) {
                *at += 1;
                if index[w as usize] == NONE {
                    index[w as usize] = counter;
                    low[w as usize] = counter;
                    counter += 1;
                    stack.push(w);
                    on[w as usize] = true;
                    frames.push((w, 0));
                } else if on[w as usize] {
                    low[v as usize] = low[v as usize].min(index[w as usize]);
                }
                continue;
            }
            frames.pop();
            if let Some(&(parent, _)) = frames.last() {
                low[parent as usize] = low[parent as usize].min(low[v as usize]);
            }
            if low[v as usize] == index[v as usize] {
                let mut scc = Vec::new();
                loop {
                    let w = stack.pop().unwrap();
                    on[w as usize] = false;
                    scc.push(w);
                    if w == v {
                        break;
                    }
                }
                sccs.push(scc);
            }
        }
    }
    sccs
}

/// Strongly connected components of `nodes` under `edges`, dependencies first, in the same order every run.
/// Iterative, where T's recursed (with a raised recursion limit).
///
/// The nodes are numbered in their order and each edge looked up once, so the search runs on numbers: it looked up
/// each node's index, low link and stack mark by name at every step, a string comparison per level of three maps (the
/// strata are asked of every program a batch prepares, by some thirty passes). Roots and successors are taken in the
/// same order as by name, so the components and their order are the same.
pub fn tarjan(nodes: &BTreeSet<String>, edges: &Edges) -> Vec<BTreeSet<String>> {
    let names: Vec<&String> = nodes.iter().collect();
    let id: rustc_hash::FxHashMap<&str, u32> = names.iter().enumerate().map(|(i, n)| (n.as_str(), i as u32)).collect();
    // Each node's successors among the nodes, in the edges' order.
    let succ: Vec<Vec<u32>> = names
        .iter()
        .map(|v| edges.get(v.as_str()).into_iter().flatten().filter_map(|(w, _)| id.get(w.as_str()).copied()).collect())
        .collect();
    const NONE: u32 = u32::MAX;
    let n = names.len();
    let (mut index, mut low, mut on) = (vec![NONE; n], vec![0u32; n], vec![false; n]);
    let (mut stack, mut sccs, mut counter) = (Vec::<u32>::new(), Vec::new(), 0u32);
    for root in 0..n as u32 {
        if index[root as usize] != NONE {
            continue;
        }
        // Frames: a node and the position of its next successor.
        let mut frames: Vec<(u32, usize)> = vec![(root, 0)];
        index[root as usize] = counter;
        low[root as usize] = counter;
        counter += 1;
        stack.push(root);
        on[root as usize] = true;
        while let Some((v, at)) = frames.last_mut() {
            let v = *v;
            if let Some(&w) = succ[v as usize].get(*at) {
                *at += 1;
                if index[w as usize] == NONE {
                    index[w as usize] = counter;
                    low[w as usize] = counter;
                    counter += 1;
                    stack.push(w);
                    on[w as usize] = true;
                    frames.push((w, 0));
                } else if on[w as usize] {
                    low[v as usize] = low[v as usize].min(index[w as usize]);
                }
                continue;
            }
            frames.pop();
            if let Some(&(parent, _)) = frames.last() {
                low[parent as usize] = low[parent as usize].min(low[v as usize]);
            }
            if low[v as usize] == index[v as usize] {
                let mut scc = BTreeSet::new();
                loop {
                    let w = stack.pop().unwrap();
                    on[w as usize] = false;
                    scc.insert(names[w as usize].clone());
                    if w == v {
                        break;
                    }
                }
                sccs.push(scc);
            }
        }
    }
    sccs
}

// cratify: `tests` is rerail-logic's (it mounts it)
#[cfg(test)]
pub mod data_layer_census_tests;
// cratify: `data_layer_tests` is rerail-logic's (it mounts it)
pub mod yaml;

/// The pragma by which a program asked for bounded rounds (decision 0056), refused: bounded cross-file rounds are retired: a program's cross-file part is one fixpoint (decisions 0065, 0276 §9).
pub const PRAGMA: &str = "cross-rounds";

/// A base relation's document column, including raw text views.
pub fn document_column(relation: &str) -> Option<usize> {
    match relation {
        "node" | "doc" | "pkg" | "path_part" | "file_name" | "syntax_error" | "cpython_syntax_error" |
        "source" | "src_line" | "src_line_lf" | "tape_line" | "split_line" | "lf_line" | "line_count" | "parsed"
        // Decision 0055's: a guest and its host are one file (`corpus::guests` attributes its rows to the host).
        | "doc_role" | "doc_root" | "tape_of" | "guest" | "host_node" => Some(0),
        // Decision 0431: a string's parsed nodes, by the string (its file's node).
        "docof" | "guest_of" | "guest_place" => Some(1),
        _ => None,
    }
}

pub use crate::code_rule_dsl::facts::core::PYTHON_CONTEXTS;

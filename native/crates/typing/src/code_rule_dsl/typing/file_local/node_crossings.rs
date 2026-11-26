//! No node id crosses into the cross-file part (decision 0677): which columns hold a node, inferred over the whole
//! program, and the cross-file rules that carry one.
//!
//! A node id is its document's and its slot's (0094 §2a): a relation of the cross-file part that holds one changes
//! for every reader whenever the file's ids change (a definition moved, a file edited above it), though nothing
//! changed in meaning. Cross-file facts are keyed by symbols, paths, document keys and values; a file's own rules
//! relate its nodes to them.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::code_rule_dsl::logic::plan::fields::Fields;
use crate::code_rule_dsl::logic::program_image::{AggRef, CmpOp, Lit, RuleRef, Slot, TermRef, Version};

/// The declared types whose values are node ids: `node`, and `loc` (`node | span`), which may hold one.
pub(crate) const NODE_TYPES: [&str; 2] = ["node", "loc"];

/// The functions of a node whose answer is what the node says, not which node it is: its document (a document key
/// may cross, 0677 §2), its text and value reads (`NODE_READS`), its subtree's digest and its source again. A value
/// they make from a node does not move when the file's ids do. Every other function of a node (`@to_string`, `+`,
/// `@cat`) makes a value of its id, and carries it.
const VALUES_OF_A_NODE: [&str; 6] = ["doc", "tree_digest", "unparse", "py_unparse", "ast_dump", "place"];

fn value_of_a_node(f: &str) -> bool {
    VALUES_OF_A_NODE.contains(&f) || crate::code_rule_dsl::facts::vtab::syntax::NODE_READS.contains(&f)
}

/// Which columns of each relation hold a node id: its declared `node` and `loc` columns, and each column a rule
/// fills from a node (a variable bound at such a column, by `nearest` or an `ast` pattern, a function of one but
/// `VALUES_OF_A_NODE`, a `min`, `max` or `sum` of one), to a fixpoint over the program's rules: a column's type
/// inferred from what binds it, as a head column's kind is.
pub struct NodeColumns<'v> {
    columns: HashMap<&'v str, Vec<bool>>,
}

impl<'v> NodeColumns<'v> {
    pub fn of(v: &'v Version, f: &Fields<'v>) -> NodeColumns<'v> {
        let mut columns: HashMap<&'v str, Vec<bool>> = HashMap::new();
        for (rel, types) in f.decls_by_name() {
            if types.iter().any(|t| NODE_TYPES.contains(t)) {
                columns.insert(rel, types.iter().map(|t| NODE_TYPES.contains(t)).collect());
            }
        }
        let mut nodes = NodeColumns { columns };
        let mut changed = true;
        while changed {
            changed = false;
            for r in v.rules() {
                let flows = nodes.flowing(v, r, &|_| false);
                let head = r.head();
                let rel = v.rel_name(head.rel());
                for (i, t) in head.args().enumerate() {
                    if term_flows(v, t, &flows.slots) && !nodes.holds(rel, i) {
                        let cols = nodes.columns.entry(rel).or_insert_with(|| vec![false; head.arity()]);
                        if cols.len() <= i {
                            cols.resize(i + 1, false);
                        }
                        cols[i] = true;
                        changed = true;
                    }
                }
            }
        }
        nodes
    }

    /// Whether column `c` of `rel` holds a node id.
    pub fn holds(&self, rel: &str, c: usize) -> bool {
        self.columns.get(rel).is_some_and(|cols| cols.get(c).copied().unwrap_or(false))
    }

    /// `rel`'s columns that hold a node id.
    pub fn of_relation(&self, rel: &str) -> Vec<usize> {
        self.columns.get(rel).map_or_else(Vec::new, |cols| cols.iter().enumerate().filter(|(_, n)| **n).map(|(i, _)| i).collect())
    }

    /// The variables of rule `r` that hold a node id, and what bound the first of them; and which are bound by a node
    /// column of a relation of `inherited` (another cross-file relation's, which carries its node already).
    fn flowing(&self, v: &Version, r: RuleRef<'_>, inherited: &dyn Fn(&str) -> bool) -> Flows {
        let n = r.var_count();
        let mut flows = Flows { slots: vec![false; n], cause: None, inherited: vec![false; n], entered: vec![None; n] };
        for l in r.body() {
            match l.view() {
                Lit::Atom(a) | Lit::Agg { atom: a, .. } => {
                    let rel = v.rel_name(a.rel());
                    for (i, t) in a.args().enumerate() {
                        if let TermRef::Var(s) = t
                            && self.holds(rel, i)
                        {
                            if inherited(rel) {
                                flows.mark(s, || format!("reads {rel}'s column {i}, a node"));
                                flows.inherited[s.index()] = true;
                            } else {
                                flows.enter(s, || format!("reads {rel}'s column {i}, a node"));
                            }
                        }
                    }
                }
                Lit::Nearest { n, a, .. } => {
                    flows.enter(n, || "asks for the nearest of a node".to_owned());
                    flows.enter(a, || "asks for the nearest of a node".to_owned());
                }
                Lit::Ast { node, captures, .. } => {
                    flows.enter(node, || "matches an AST pattern".to_owned());
                    for (_, s) in captures {
                        flows.enter(s, || "matches an AST pattern".to_owned());
                    }
                }
                Lit::Cmp { .. } => {}
            }
        }
        // Equalities and aggregates carry a node to the variables they bind, to a fixpoint within the rule.
        let mut changed = true;
        while changed {
            changed = false;
            for l in r.body() {
                match l.view() {
                    Lit::Cmp { l, op: CmpOp::Eq, r } => {
                        for (from, to) in [(l, r), (r, l)] {
                            if let TermRef::Var(s) = to
                                && !flows.slots[s.index()]
                                && term_flows(v, from, &flows.slots)
                            {
                                flows.mark(s, || "computes a value from a node".to_owned());
                                changed = true;
                            }
                        }
                    }
                    Lit::Agg { var, op, over, .. } if !matches!(op, AggRef::Count) && !flows.slots[var.index()] && over.is_some_and(|o| flows.slots[o.index()]) => {
                        flows.mark(var, || "aggregates a node".to_owned());
                        changed = true;
                    }
                    _ => {}
                }
            }
        }
        flows
    }
}

struct Flows {
    slots: Vec<bool>,
    cause: Option<String>,
    /// The variables bound by a node column of another cross-file relation.
    inherited: Vec<bool>,
    /// What bound each variable to a node first where it entered the rule: a read of a per-file relation's (an
    /// export's, a base relation's) node column, a pattern or `nearest`.
    entered: Vec<Option<String>>,
}

impl Flows {
    fn mark(&mut self, s: Slot, why: impl FnOnce() -> String) {
        if let Some(slot) = self.slots.get_mut(s.index()) {
            *slot = true;
            self.cause.get_or_insert_with(why);
        }
    }

    fn enter(&mut self, s: Slot, why: impl Fn() -> String) {
        self.mark(s, &why);
        if let Some(e) = self.entered.get_mut(s.index()) {
            e.get_or_insert_with(why);
        }
    }

    /// Where a node enters the cross-file part in this rule: a variable bound to a node per file, by no node column of
    /// another cross-file relation (which carries it already, and is named itself).
    fn entry(&self) -> Option<&str> {
        self.entered.iter().zip(&self.inherited).find_map(|(e, inherited)| e.as_deref().filter(|_| !inherited))
    }
}

/// Whether term `t` holds a node id: a variable that does, or a function of one but `VALUES_OF_A_NODE`.
fn term_flows(v: &Version, t: TermRef<'_>, slots: &[bool]) -> bool {
    match t {
        TermRef::Var(s) => slots.get(s.index()).copied().unwrap_or(false),
        TermRef::Call(f, mut args) => !value_of_a_node(v.text(f)) && args.any(|a| term_flows(v, a, slots)),
        _ => false,
    }
}

/// One relation of the cross-file part that holds or reads a node id: its name, the source relation the lowering
/// made it for, and why, with its rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeCrossing {
    pub rel: String,
    pub source: Option<String>,
    pub why: String,
    /// The crossing that puts it in the cross-file part (`crossing_under`, from the split's causes), named as `named`
    /// names; `None` where it is cross-file for a reason of its own, or the causes are not known (a split read from
    /// the store keeps none).
    pub under: Option<String>,
}

impl NodeCrossing {
    /// The relation of the rules as written: the source relation of a helper, without a batch's program prefix
    /// (`p3_`): the name the offenders' list holds.
    pub fn named(&self) -> String {
        unprefixed(self.source.as_deref().unwrap_or(&self.rel))
    }
}

/// `rel` without a batch's program prefix (`p3_`, `q1_`).
fn unprefixed(rel: &str) -> String {
    match rel.split_once('_') {
        Some((p, rest)) if p.len() > 1 && p.starts_with(['p', 'q']) && p[1..].bytes().all(|b| b.is_ascii_digit()) => rest.to_owned(),
        _ => rel.to_owned(),
    }
}

/// The crossing a relation of the cross-file part is there for, read down the split's causes (`FileLocal::causes`,
/// 0652's chain): through each "reads cross-file X" to the first relation cross-file for a reason of its own; that
/// relation, or the input it joins with no location ("reads value_type with no location"). `None` for one
/// cross-file for a reason of its own (it joins two documents itself). A relation that places a cross-file answer on
/// its own nodes (`up018_literal_str(n) :- kind(n, "string"), …`) is under the answer's crossing: handed on by a
/// phase (0665 §6), the answer is a setting, and its reader per file.
fn crossing_under(rel: &str, causes: &BTreeMap<String, String>) -> Option<String> {
    let mut at = rel.to_owned();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    while seen.insert(at.clone()) {
        let Some(cause) = causes.get(&at) else { break };
        let cause = cause.lines().next().unwrap_or("");
        let first = |s: &str| s.split_whitespace().next().unwrap_or("").to_owned();
        if let Some(x) = cause.strip_prefix("reads cross-file ") {
            at = first(x);
            continue;
        }
        if let Some((y, _)) = cause.strip_prefix("reads ").and_then(|c| c.split_once(" with no location")) {
            return Some(first(y));
        }
        break;
    }
    (at != rel).then_some(at)
}

/// Each crossing's `under` (`crossing_under`) from the split's `causes`, named as its relation is (a helper by its
/// source, without a batch's prefix).
pub fn crossings_under(program: &Version, crossings: &mut [NodeCrossing], causes: &BTreeMap<String, String>) {
    let readers = readers_of(program);
    let readers = |rel: &str| readers.get(rel).map_or_else(Vec::new, |hs| hs.iter().map(|h| (*h).to_owned()).collect());
    for c in crossings {
        c.under = crossing_under(&c.rel, causes).map(|u| unprefixed(&super::lowering_source_in(program, &u, &readers).unwrap_or(u)));
    }
}

/// Each relation's readers' heads, once, for the helpers' sources (`lowering_source`).
fn readers_of(program: &Version) -> HashMap<&str, BTreeSet<&str>> {
    let mut readers: HashMap<&str, BTreeSet<&str>> = HashMap::new();
    for r in program.rules() {
        let head = program.rel_name(r.head().rel());
        for a in r.body().filter_map(|l| l.atom()) {
            readers.entry(program.rel_name(a.rel())).or_default().insert(head);
        }
    }
    readers
}

/// Where a node id enters the cross-file part `cross` (of `program`, split at its file-local relations `local`;
/// decision 0677 §1): each derived relation of it whose rule binds a node per file, by a read of a per-file
/// relation's node column (an export's, a base relation's), a pattern or `nearest`, and by no node column of another
/// cross-file relation; and each rule of `program` that reads a node column of a setting or an earlier phase's export
/// (`run_constant`), which crosses from every file to every file.
///
/// A file-local relation the cross-file part derives again for its readers (`kind`) is not named: its rows are each
/// one file's, and the node enters at its cross-file reader. A relation that reads its node from another cross-file
/// relation follows it, and is not named: it holds a node
/// because that relation does, and goes per file with it (`air302_alt_1(n) :- sym_of(n, "…"), kind(n, …)` follows
/// `sym_of`, Python's name resolution, 0677's first family), as 0652 names a crossing and not its readers. An output
/// is not one: its node is the finding's place, written once (0677, Open: which form a finding that pairs two files'
/// nodes takes is the rewrite of its family), and a relation reading an output's node is named. A relation only
/// outputs read is (unlike 0652's `placement_only`): a set of other files' nodes is maintained over every file, and
/// changes with their ids, for every finding that reads it; keyed by a symbol, the finding is placed in its own
/// file's rules.
pub fn node_crossings(program: &Version, f: &Fields<'_>, cross: &Version, local: &BTreeSet<String>) -> Vec<NodeCrossing> {
    let nodes = NodeColumns::of(program, f);
    let mut found: BTreeMap<String, String> = BTreeMap::new();
    let shown = |v: &Version, r: RuleRef<'_>| v.show_rule(r).chars().take(300).collect::<String>();
    // A relation of the cross-file part: derived there, or made natively there (a function of values `locate` moved
    // for its cross-file readers, `value_type`).
    let crossing = |rel: &str| (f.derives(rel) || f.native(rel).is_some()) && !local.contains(rel);
    let inherited = |rel: &str| crossing(rel) && !f.is_output(rel);
    for r in cross.rules() {
        let head = cross.rel_name(r.head().rel());
        // A file-local relation the cross part derives again for its readers (`kind`, a projected export) holds each
        // file's own rows: the node enters at the cross-file rule that reads it, which is named. An output's located
        // form (`locate`'s `violation_at`, read back as `violation`) is the output.
        let output_at = head.strip_suffix("_at").is_some_and(|o| f.is_output(o));
        if found.contains_key(head) || f.is_output(head) || output_at || local.contains(head) {
            continue;
        }
        let flows = nodes.flowing(cross, r, &inherited);
        let Some(cause) = flows.entry().map(str::to_owned) else { continue };
        let held: Vec<String> = r.head().args().enumerate().filter(|(_, t)| term_flows(cross, *t, &flows.slots)).map(|(i, _)| column_name(f, head, i)).collect();
        let what = if held.is_empty() { "reads a node".to_owned() } else { format!("holds a node in {}", held.join(", ")) };
        found.insert(head.to_owned(), format!("{what}: its rule {cause}\n      {}", shown(cross, r)));
    }
    let settings = |rel: &str| super::run_constant(program, rel) && !super::option_input(rel);
    for r in program.rules() {
        let head = program.rel_name(r.head().rel());
        if found.contains_key(head) {
            continue;
        }
        let read = r.body().filter_map(|l| l.atom()).find_map(|a| {
            let rel = program.rel_name(a.rel());
            (settings(rel) && a.args().enumerate().any(|(i, t)| matches!(t, TermRef::Var(_)) && nodes.holds(rel, i))).then_some(rel)
        });
        if let Some(rel) = read {
            found.insert(head.to_owned(), format!("reads a node of {rel}, a setting or an earlier phase's export, which every file reads\n      {}", shown(program, r)));
        }
    }
    // A relation of the cross-file part made natively and read there, with a node column: the node enters where it
    // is made.
    let read_there: BTreeSet<&str> = cross.rules().flat_map(|r| r.body().filter_map(|l| l.atom()).map(|a| cross.rel_name(a.rel())).collect::<Vec<_>>()).collect();
    for (rel, _) in f.decls_by_name() {
        if f.native(rel).is_some() && crossing(rel) && read_there.contains(rel) && !nodes.of_relation(rel).is_empty() {
            let held: Vec<String> = nodes.of_relation(rel).into_iter().map(|c| column_name(f, rel, c)).collect();
            found.entry(rel.to_owned()).or_insert_with(|| format!("holds a node in {}: made natively in the cross-file part", held.join(", ")));
        }
    }
    let readers = readers_of(program);
    let readers = |rel: &str| readers.get(rel).map_or_else(Vec::new, |hs| hs.iter().map(|h| (*h).to_owned()).collect());
    // `under` is the split's to say (`crossings_under`, from its causes).
    found.into_iter().map(|(rel, why)| NodeCrossing { source: super::lowering_source_in(program, &rel, &readers), rel, why, under: None }).collect()
}

/// Column `c` of `rel` by its declared name, else its number.
fn column_name(f: &Fields<'_>, rel: &str, c: usize) -> String {
    f.columns(rel).and_then(|cols| cols.get(c).map(|(n, _)| format!("{rel}.{n}"))).unwrap_or_else(|| format!("{rel}'s column {c}"))
}

/// `RERAIL_STRICT_NODES=1`: a program whose cross-file part holds a node id is refused (decision 0677). Read where a
/// split is made, once per program version (no process-global state, decision 0442).
pub fn strict_nodes() -> bool {
    std::env::var("RERAIL_STRICT_NODES").is_ok_and(|v| !matches!(v.as_str(), "" | "0"))
}

/// The cross-file part's node crossings (`node_crossings`) refused under `strict`, else reported in one line, by the
/// crossing each is under: once per program version, as its split is made once. `RERAIL_NODE_CROSSINGS=FILE`
/// appends each one's name (`NodeCrossing::named`) and its crossing's to FILE, a line each, for the pack test's list
/// of offenders.
pub fn nodes_checked(crossings: &[NodeCrossing], strict: bool) -> Result<(), String> {
    if crossings.is_empty() {
        return Ok(());
    }
    if let Some(path) = std::env::var_os("RERAIL_NODE_CROSSINGS") {
        use std::io::Write as _;
        let lines: String = crossings.iter().map(|c| format!("{} {}\n", c.named(), c.under.as_deref().unwrap_or("-"))).collect();
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            // One write of every line: the runs of a batch append to one file at once.
            let _ = file.write_all(lines.as_bytes());
        }
    }
    if strict {
        let shown: Vec<String> = crossings
            .iter()
            .map(|c| {
                let under = c.under.as_ref().map_or_else(String::new, |u| format!(" (in the cross-file part under {u})"));
                super::crossing_shown_with(c.source.as_deref(), &c.rel, &format!("{}{under}", c.why))
            })
            .collect();
        return Err(format!(
            "RERAIL_STRICT_NODES: {} relation(s) of the cross-file part hold or read a node id (decision 0677). Key each by a \
             symbol (a module, a qualified name), a path, a document key or a value, relating the file's nodes to it in the \
             file's own rules; or keep the rule per file:\n{}",
            crossings.len(),
            shown.join("\n")
        ));
    }
    let mut by: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for c in crossings {
        by.entry(c.under.clone().unwrap_or_default()).or_default().insert(c.named());
    }
    let line: Vec<String> = by
        .into_iter()
        .map(|(under, names)| match under.as_str() {
            "" => names.into_iter().collect::<Vec<_>>().join(", "),
            _ => format!("{} under {under}", names.len()),
        })
        .collect();
    eprintln!("rerail: the cross-file part holds node ids, which every edit of their file moves (decision 0677): {}", line.join("; "));
    Ok(())
}

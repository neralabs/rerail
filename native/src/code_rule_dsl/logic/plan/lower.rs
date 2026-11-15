//! A program lowered to the integers of the base relations (decision 0009; O13).
//!
//! Every column gets a domain. Declared types give `int`, `node` and `sym`; the base relations' symbol columns are
//! names, except `node.kind` (kind atoms) and `field.f` (field atoms); a derived `sym` column takes the domain of
//! the variables written into it, to a fixpoint over the rules, and is a name if nothing says otherwise. String
//! constants then become integers by the domain they meet: name atoms (interned), or kind and field atoms in the
//! program's dialect (`-1`, which matches nothing, for a name the grammar lacks). Planning and SQL see only integers.

use super::super::front::{AggOp, Atom, Literal, Program, Term, Value};
use super::{Error, Result};
use crate::code_rule_dsl::facts::core::kind_atom;
use crate::code_rule_dsl::facts::vtab::syntax::field_atom;
use crate::code_rule_dsl::language::{GrammarDialect, KindCatalog, tape_tag};
use std::collections::BTreeMap;
// FxHash: lowering's variable maps are hot (std's SipHash showed in a cold batch's samples).
use rustc_hash::FxHashMap as HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(Hash)]
#[derive(Ord)]
#[derive(PartialOrd)]
pub enum Domain {
    Int,
    Node,
    Kind,
    Field,
    Name,
}

impl Domain {
    fn symbolic(self) -> bool {
        matches!(self, Domain::Kind | Domain::Field | Domain::Name)
    }
}

/// The domains of a functor's arguments and of its value, where it fixes them: `@cat`'s parts and whole are names,
/// `flat_bin`'s operator is one; arithmetic is over integers; `concat`, `substr` and `to_string` make names,
/// `strlen` an integer, `rx_group` a regex's named group, `pkg_up` a package (`.rr`'s `+ - * /`, `cat`, `substr`, `to_string`, `strlen`).
/// A text test's string is a name, but for `equals`, a membership (`(k = "a" ; k = "b")`), whose string may be a kind
/// or a field (`expand_memberships`).
pub(in crate::code_rule_dsl) fn functor_domains(f: &str, arity: usize) -> (Vec<Option<Domain>>, Option<Domain>) {
    use Domain::{Int, Name};
    match f {
        "cat" | "concat" => (vec![Some(Name); arity], Some(Name)),
        "flat_bin" => (vec![Some(Name), None, None], None),
        "add" | "sub" | "mul" | "div" | "max" | "min" => (vec![Some(Int); arity], Some(Int)),
        "strlen" => (vec![Some(Name)], Some(Int)),
        "doc" => (vec![None], Some(Int)),
        "to_string" => (vec![Some(Int)], Some(Name)),
        "substr" => (vec![Some(Name), Some(Int), Some(Int)], Some(Name)),
        // W's semantics, ahead of `strings::functions`' (`plan::sql_term`'s arms for them).
        "replace_all" | "rx_replace" => (vec![Some(Name); 3], Some(Name)),
        "json_str" | "nfkc" | "py_repr" => (vec![Some(Name)], Some(Name)),
        "ed25519_ok" => (vec![Some(Name); 3], Some(Int)),
        "rx_group" => (vec![Some(Name); 3], Some(Name)),
        // `@place(path, line)` (`rr::lower`'s `PLACE`): a place where a node goes, taken apart again by
        // `place_path` and `place_line`.
        "place" => (vec![Some(Name), Some(Int)], Some(Domain::Node)),
        "place_path" => (vec![Some(Domain::Node)], Some(Name)),
        "place_line" => (vec![Some(Domain::Node)], Some(Int)),
        f if let Some((_, args, ret)) = crate::code_rule_dsl::logic::strings::functions::signature(f) => {
            let domain = |k: &crate::code_rule_dsl::logic::strings::functions::Kind| {
                Some(if *k == crate::code_rule_dsl::logic::strings::functions::Kind::Text { Name } else { Int })
            };
            (args.iter().map(domain).collect(), domain(&ret))
        }
        // `equals` (membership) takes its subject's domain: kinds compare as kind atoms (`lower`, `@in`).
        "equals" | "in" => (vec![None; arity], None),
        f if crate::code_rule_dsl::logic::strings::is_test(f) => (vec![Some(Name); arity], None),
        _ => (vec![None; arity], None),
    }
}

/// `prog` with each membership of a kind or field variable (`equals($k, {"a", "b"})`, from `(k = "a" ; k = "b")`)
/// as comparisons, which take the variable's domain: one rule per member, or (negated) a comparison with each.
/// `None` where `prog` has no membership.
fn expand_memberships(prog: &Program) -> Result<Option<Program>> {
    let membership = |l: &Literal| match l {
        Literal::Cmp { l: Term::Call(f, args), op: "=", r: Term::Int(b @ (0 | 1)) } if f == "equals" => match args.as_slice() {
            [Term::Var(x), Term::Call(set, items)] if set == crate::code_rule_dsl::logic::strings::SET => Some((x.clone(), items.clone(), *b == 1)),
            _ => None,
        },
        _ => None,
    };
    if !prog.rules.iter().flat_map(|r| &r.body).any(|l| membership(l).is_some()) {
        return Ok(None);
    }
    let domains = domains(prog)?;
    let cols = columns_of(&domains);
    let mut out = prog.clone();
    out.rules.clear();
    for rule in &prog.rules {
        if !rule.body.iter().any(|l| membership(l).is_some()) {
            out.rules.push(rule.clone());
            continue;
        }
        let vars = rule_vars(rule, &mut |a: &Atom| cols.get(a.rel.as_str()).map(Vec::as_slice))?;
        let mut bodies = vec![Vec::new()];
        for l in &rule.body {
            match membership(l) {
                Some((x, items, holds)) if matches!(vars.get(x.as_str()), Some(Domain::Kind | Domain::Field)) => {
                    let cmp = |item: &Term, op| Literal::Cmp { l: Term::Var(x.clone().into()), op, r: item.clone() };
                    if holds {
                        bodies = bodies.iter().flat_map(|b| items.iter().map(move |item| b.iter().cloned().chain([cmp(item, "=")]).collect())).collect();
                    } else {
                        bodies.iter_mut().for_each(|b| b.extend(items.iter().map(|item| cmp(item, "!="))));
                    }
                }
                _ => bodies.iter_mut().for_each(|b| b.push(l.clone())),
            }
        }
        out.rules.extend(bodies.into_iter().map(|body| super::super::front::Rule { head: rule.head.clone(), body: body.into() }));
    }
    Ok(Some(out))
}

/// Each relation's column domains.
pub(in crate::code_rule_dsl) type Domains = BTreeMap<String, Vec<Domain>>;

/// Which of relation `rel`'s columns hold atoms (decision 0170: what a forked process maps, what an entry may name):
/// its `sym` columns whose values are names, not a grammar's kind or field ids (`Domain::Kind`, `Domain::Field`:
/// `field_atom`'s `tag << 16 | code`, which no store numbers). A relation `domains` does not cover keeps every `sym`
/// column.
pub(in crate::code_rule_dsl) fn atom_columns(prog: &Program, domains: &Domains, rel: &str) -> Vec<bool> {
    let ds = domains.get(rel);
    prog.decls.get(rel).map_or_else(Vec::new, |cols| {
        cols.iter().enumerate().map(|(c, (_, t))| t == "sym" && ds.and_then(|d| d.get(c)).is_none_or(|d| *d == Domain::Name)).collect()
    })
}

/// A base relation's symbol column that is not a name.
fn base_domain(rel: &str, col: usize) -> Option<Domain> {
    match (rel, col) {
        ("node", 2) => Some(Domain::Kind),
        ("field", 1) => Some(Domain::Field),
        _ => None,
    }
}

fn atoms(l: &Literal) -> impl Iterator<Item = &Atom> {
    match l {
        Literal::Atom(a) | Literal::Agg { atom: a, .. } => Some(a),
        _ => None,
    }
    .into_iter()
}

/// A rule's variables' domains, as they are found.
struct Vars<'a> {
    head: &'a Atom,
    map: HashMap<&'a str, Domain>,
}

impl<'a> Vars<'a> {
    /// Records `$v` in `d`; whether that is new. Two symbol domains for one variable is an error.
    fn set(&mut self, v: &'a str, d: Domain) -> Result<bool> {
        match self.map.get(v) {
            Some(&e) if e.symbolic() && d.symbolic() && e != d => {
                Err(Error::Plan(format!("rule for {}: ${v} is a {e:?} and a {d:?}", self.head)))
            }
            Some(_) => Ok(false),
            None => {
                self.map.insert(v, d);
                Ok(true)
            }
        }
    }

    /// The domains a builtin call gives its variables: `@cat`'s parts are names, `flat_bin`'s operator too.
    fn call(&mut self, t: &'a Term) -> Result<bool> {
        let mut changed = false;
        if let Term::Call(f, args) = t {
            let (domains, _) = functor_domains(f, args.len());
            for (a, d) in args.iter().zip(domains) {
                if let (Term::Var(v), Some(d)) = (a, d) {
                    changed |= self.set(v, d)?;
                }
                changed |= self.call(a)?;
            }
        }
        Ok(changed)
    }
}

/// The domains of a rule's variables, given each of its atoms' columns (`cols`, in the order of `atoms`: the head,
/// then the body's).
fn rule_vars<'r, 'c>(r: &'r super::super::front::Rule, cols: &mut dyn FnMut(&Atom) -> Option<&'c [Option<Domain>]>) -> Result<HashMap<&'r str, Domain>> {
    let mut vars = Vars { head: &r.head, map: HashMap::default() };
    // Each atom's columns, looked up once, not once per argument per pass.
    let atoms: Vec<(&'r Atom, Option<&'c [Option<Domain>]>)> = std::iter::once(&r.head).chain(r.body.iter().flat_map(atoms)).map(|a| (a, cols(a))).collect();
    let mut changed = true;
    while changed {
        changed = false;
        for &(a, ds) in &atoms {
            for (c, t) in a.args.iter().enumerate() {
                if let (Term::Var(v), Some(Some(d))) = (t, ds.and_then(|ds| ds.get(c))) {
                    changed |= vars.set(v, *d)?;
                }
                changed |= vars.call(t)?;
            }
        }
        for l in &r.body {
            match l {
                Literal::Cmp { l, r, .. } => {
                    changed |= vars.call(l)?;
                    changed |= vars.call(r)?;
                    if let (Term::Var(x), Term::Var(y)) = (l, r) {
                        if let Some(&d) = vars.map.get(x.as_str()) {
                            changed |= vars.set(y.as_str(), d)?;
                        }
                        if let Some(&d) = vars.map.get(y.as_str()) {
                            changed |= vars.set(x.as_str(), d)?;
                        }
                    }
                    // `$x = @cat(...)`: the whole is a name; `$x = @add(...)` an integer.
                    if let (Term::Var(x), Term::Call(f, args)) = (l, r)
                        && let (_, Some(d)) = functor_domains(f, args.len())
                    {
                        changed |= vars.set(x, d)?;
                    }
                }
                Literal::Nearest { n, a, .. } => {
                    changed |= vars.set(n, Domain::Node)?;
                    changed |= vars.set(a, Domain::Node)?;
                }
                // `join` makes a name; `count` and `sum` integers.
                Literal::Agg { var, op: AggOp::Join { .. }, .. } => changed |= vars.set(var, Domain::Name)?,
                Literal::Agg { var, op: AggOp::Count | AggOp::Sum, .. } => changed |= vars.set(var, Domain::Int)?,
                _ => {}
            }
        }
    }
    Ok(vars.map)
}

/// Every column's domain (see the module comment).
///
/// On numbers: each declared relation by its place among the declarations, its columns a vector by number, each
/// rule's atoms resolved to numbers once, and the readers by number. Looked up by name in a sorted map at every atom
/// of every step, the columns were a string comparison per level (`memcmp`, the largest share of a cold batch's main
/// thread under `domains`). The worklist's order is the same, so the domains are.
pub fn domains(prog: &Program) -> Result<Domains> {
    let names: Vec<&str> = prog.decls.keys().map(|r| r.as_str()).collect();
    let id: HashMap<&str, u32> = names.iter().enumerate().map(|(i, r)| (*r, i as u32)).collect();
    let inputs: std::collections::HashSet<&str> = prog.inputs.iter().map(String::as_str).collect();
    let mut cols: Vec<Vec<Option<Domain>>> = Vec::with_capacity(names.len());
    for (rel, decl) in &prog.decls {
        let input = inputs.contains(rel.as_str());
        let ds = decl
            .iter()
            .enumerate()
            .map(|(c, (_, t))| match t.as_str() {
                "int" | "flat" | "min" | "max" => Ok(Some(Domain::Int)),
                "node" => Ok(Some(Domain::Node)),
                "sym" if input => Ok(Some(base_domain(rel, c).unwrap_or(Domain::Name))),
                "sym" => Ok(None),
                _ => Err(Error::Plan(format!("{rel}: no domain for {t}"))),
            })
            .collect::<Result<_>>()?;
        cols.push(ds);
    }
    // Each rule's atoms (the head, then the body's) by number: `None` for a relation not declared.
    let rels: Vec<Vec<Option<u32>>> = prog
        .rules
        .iter()
        .map(|r| std::iter::once(&r.head).chain(r.body.iter().flat_map(atoms)).map(|a| id.get(a.rel.as_str()).copied()).collect())
        .collect();
    // To a fixpoint, by a worklist: a rule is read again only when a relation it names gained a column's domain. Every
    // rule in every round was quadratic in a combined pack (O111: 539 rules, a round per step of a chain).
    let mut readers: Vec<Vec<usize>> = vec![Vec::new(); names.len()];
    for (i, atoms) in rels.iter().enumerate() {
        for &d in atoms.iter().flatten() {
            let list = &mut readers[d as usize];
            if list.last() != Some(&i) {
                list.push(i);
            }
        }
    }
    let mut queued = vec![true; prog.rules.len()];
    let mut work: std::collections::VecDeque<usize> = (0..prog.rules.len()).collect();
    while let Some(i) = work.pop_front() {
        queued[i] = false;
        let r = &prog.rules[i];
        let vars = {
            let mut at = rels[i].iter();
            let cols = &cols;
            rule_vars(r, &mut |_| at.next().copied().flatten().map(|d| cols[d as usize].as_slice()))?
        };
        let mut gained: Vec<u32> = Vec::new();
        for (a, d) in std::iter::once(&r.head).chain(r.body.iter().flat_map(atoms)).zip(&rels[i]) {
            let Some(d) = *d else { continue };
            let ds = &mut cols[d as usize];
            for (c, t) in a.args.iter().enumerate() {
                if let (Term::Var(v), Some(slot @ None)) = (t, ds.get_mut(c))
                    && let Some(&dom) = vars.get(v.as_str())
                {
                    *slot = Some(dom);
                    gained.push(d);
                }
            }
        }
        for d in gained {
            for &j in &readers[d as usize] {
                if !queued[j] {
                    queued[j] = true;
                    work.push_back(j);
                }
            }
        }
    }
    Ok(names.into_iter().zip(cols).map(|(rel, ds)| (rel.to_owned(), ds.into_iter().map(|d| d.unwrap_or(Domain::Name)).collect())).collect())
}

/// Each relation's columns as `rule_vars` reads them, from the domains: by name, one lookup per atom.
pub(in crate::code_rule_dsl) fn columns_of(domains: &Domains) -> HashMap<&str, Vec<Option<Domain>>> {
    domains.iter().map(|(r, ds)| (r.as_str(), ds.iter().copied().map(Some).collect())).collect()
}

/// The domains of a new relation's columns past `known` (those it shares with a relation `domains` covers), from
/// `rule`, its one rule: each column's declared type (`types`), else its head variable's domain in the rule's body
/// (`cols`, `columns_of`), else a name, as `domains` finds them. A support relation (`supports`, `h__supN`) holds its
/// head's columns, then the variables its body binds: a kind there (`kind(p, k)`) is a kind id, no atom (decision
/// 0170), and given the head's domains alone it was taken for one.
pub(in crate::code_rule_dsl) fn rule_head_domains(rule: &super::super::front::Rule, types: &[&str], known: &[Domain], cols: &HashMap<&str, Vec<Option<Domain>>>) -> Result<Vec<Domain>> {
    let vars = rule_vars(rule, &mut |a: &Atom| cols.get(a.rel.as_str()).map(Vec::as_slice))?;
    Ok(types
        .iter()
        .enumerate()
        .map(|(c, t)| match *t {
            _ if c < known.len() => known[c],
            "int" | "flat" | "min" | "max" => Domain::Int,
            "node" => Domain::Node,
            _ => match rule.head.args.get(c) {
                Some(Term::Var(v)) => vars.get(v.as_str()).copied().unwrap_or(Domain::Name),
                _ => Domain::Name,
            },
        })
        .collect())
}

/// A lowered program's tree, where a reader still reads one (decision 0173: on the run path, none should): given, or
/// decoded from the program's version the first time a reader asks. Each decode is noted (`trees_decoded`) and,
/// when profiling, named by the reader's place (the decision "a tree decoded from its version"), so what still reads
/// a tree is seen, and a part made of versions alone (`Tree::of`) costs no tree until then. A clone shares the tree
/// (and a decode made through either); an edit of a shared tree copies it, noted too (`trees_copied`).
#[derive(Clone)]
pub struct Tree {
    tree: std::sync::Arc<std::sync::OnceLock<Program>>,
    /// The version it is decoded from, with the maintained parts its tree states (`again`, `together`); `None` for a
    /// tree given (`Tree::given`).
    of: Option<(std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>, Option<std::sync::Arc<super::body::Maint>>)>,
}

/// A thread's count of one kind of tree read (`Tree`), and the place of the last.
type Reads = std::cell::Cell<(usize, Option<&'static std::panic::Location<'static>>)>;

thread_local! {
    /// The trees this thread decoded from their versions, and the shared trees it copied to edit them (`Tree`): a
    /// count and the last place each, beside the profile's decision naming every place.
    static TREE_READS: [Reads; 2] = const { [std::cell::Cell::new((0, None)), std::cell::Cell::new((0, None))] };
}

/// How many trees this thread decoded from their versions so far, and where the last was (`file:line`): a run reads
/// its trees on its own thread, so a check around a run on one thread reads that run's.
pub fn trees_decoded() -> (usize, Option<String>) {
    tree_reads(0)
}

/// How many shared trees this thread copied to edit them so far, and where the last was (`trees_decoded`).
pub fn trees_copied() -> (usize, Option<String>) {
    tree_reads(1)
}

/// Forgets this thread's tree reads (`trees_decoded`, `trees_copied`): a check counts a run's alone.
pub fn forget_tree_reads() {
    TREE_READS.with(|t| t.iter().for_each(|c| c.set((0, None))));
}

fn tree_reads(which: usize) -> (usize, Option<String>) {
    let (n, at) = TREE_READS.with(|t| t[which].get());
    (n, at.map(|l| format!("{}:{}", l.file(), l.line())))
}

fn tree_read(which: usize, at: &'static std::panic::Location<'static>) {
    TREE_READS.with(|t| t[which].set((t[which].get().0 + 1, Some(at))));
}

impl Tree {
    /// A tree the caller holds.
    pub fn given(program: Program) -> Self {
        let tree = std::sync::OnceLock::new();
        let _ = tree.set(program);
        Tree { tree: std::sync::Arc::new(tree), of: None }
    }
    /// The tree of version `v`, decoded only where a reader asks.
    pub fn of(v: std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>) -> Self {
        Tree { tree: Default::default(), of: Some((v, None)) }
    }
    /// The tree of version `v` with maintained parts `m` (`again`, `together`), decoded only where a reader asks.
    pub(in crate::code_rule_dsl) fn maintained(v: std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>, m: Option<std::sync::Arc<super::body::Maint>>) -> Self {
        Tree { tree: Default::default(), of: Some((v, m)) }
    }
    /// The tree, where it was given rather than decoded from a version: what a check against the version reads.
    pub fn given_tree(&self) -> Option<&Program> {
        if self.of.is_some() { None } else { self.tree.get() }
    }
    #[track_caller]
    fn get(&self) -> &Program {
        if let Some(p) = self.tree.get() {
            return p;
        }
        let at = std::panic::Location::caller();
        self.tree.get_or_init(|| {
            let (v, m) = self.of.as_ref().expect("a tree neither given nor of a version");
            tree_read(0, at);
            crate::code_rule_dsl::logic::profile::decision("a tree decoded from its version", || format!("{}:{}", at.file(), at.line()));
            let lists = crate::code_rule_dsl::logic::front::Lists::default();
            let mut p = v.program(&lists).unwrap_or_else(|e| panic!("a version made by the run decodes: {e}"));
            if let Some(m) = m {
                (p.again, p.together) = m.to_tree(&lists);
            }
            p
        })
    }
    /// The tree as the caller's, decoded where it is not held.
    #[track_caller]
    pub fn into_program(self) -> Program {
        self.get();
        std::sync::Arc::try_unwrap(self.tree).map_or_else(|shared| shared.get().expect("decoded above").clone(), |t| t.into_inner().expect("decoded above"))
    }
}

impl std::fmt::Debug for Tree {
    /// The tree's: a check comparing programs reads it (and decodes one held as a version).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&**self, f)
    }
}

impl std::ops::Deref for Tree {
    type Target = Program;
    #[track_caller]
    fn deref(&self) -> &Program {
        self.get()
    }
}

impl std::ops::DerefMut for Tree {
    /// An edit of the tree: the caller states the same edit on the version (`corpus::made_from`).
    #[track_caller]
    fn deref_mut(&mut self) -> &mut Program {
        let at = std::panic::Location::caller();
        self.get();
        if std::sync::Arc::get_mut(&mut self.tree).is_none() {
            tree_read(1, at);
            crate::code_rule_dsl::logic::profile::decision("a shared tree copied to be edited", || format!("{}:{}", at.file(), at.line()));
        }
        // An edited tree is no longer its version's.
        self.of = None;
        std::sync::Arc::make_mut(&mut self.tree).get_mut().expect("decoded above")
    }
}

/// A program lowered for one dialect.
#[derive(Clone)]
pub struct Lowered {
    pub program: Tree,
    /// `program`'s version, where the caller made it from another's (a split's part, decision 0244): `plan::run`
    /// reads it in place of an image made of `program`. `None` once `program` is edited, and for a program made here.
    pub version: Option<std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>>,
    /// A maintained program's parts beside `version` (decision 0215): `None` for every other program.
    pub(in crate::code_rule_dsl) maint: Option<std::sync::Arc<super::body::Maint>>,
    pub domains: Domains,
    /// The name atoms of `flat_bin`'s operators `+`, `-` and `*`.
    pub(in crate::code_rule_dsl) ops: [i64; 3],
    pub dialect: GrammarDialect,
}

impl Lowered {
    /// What the program's run reads beside its version (`plan::run`).
    pub fn lowering(&self) -> Lowering<'_> {
        Lowering { domains: &self.domains, dialect: self.dialect }
    }
    /// The version `plan::run` reads: `version`, else an image made of `program` (`Placed::of_program`).
    pub fn placed(&self) -> super::Result<super::body::Placed> {
        let placed = match &self.version {
            Some(v) if self.maint.is_some() => super::body::Placed::derived(v.clone(), self.maint.clone()),
            Some(v) if !self.tree_maintained() => super::body::Placed::whole(v.clone()),
            _ => return super::body::Placed::of_program(&self.program),
        };
        // A version made from another's is stated beside the tree until the run reads no tree (t36 item 7): in a debug
        // build (every test's), it is checked to be the tree's program, so the two statements cannot drift apart unseen.
        #[cfg(debug_assertions)]
        self.same_as_tree(&placed)?;
        Ok(placed)
    }

    /// Panics unless `placed`, made from `version`, is the program `program` is: each read back through an image (the
    /// image's normal form), with the maintained parts.
    #[cfg(debug_assertions)]
    fn same_as_tree(&self, placed: &super::body::Placed) -> super::Result<()> {
        // A tree decoded from the version is the version's: only one given beside it is checked.
        let Some(tree) = self.program.given_tree() else { return Ok(()) };
        let lists = crate::code_rule_dsl::logic::front::Lists::default();
        let of = super::body::Placed::of_program(tree)?;
        let got = placed.v.program(&lists).map_err(super::Error::Plan)?;
        let want = of.v.program(&lists).map_err(super::Error::Plan)?;
        let maint = (placed.maint.as_ref().map(|m| m.to_tree(&lists)), of.maint.as_ref().map(|m| m.to_tree(&lists)));
        use crate::code_rule_dsl::logic::program_facts::debug_digest as digest;
        if digest(&(&got, &maint.0)) != digest(&(&want, &maint.1)) {
            let rule = got.rules.iter().zip(&want.rules).position(|(a, b)| a != b);
            let mut fields = got.clone();
            fields.rules = want.rules.clone();
            panic!(
                "a version made from another's is not its tree's program: {} rules against {}; first rule that differs: {rule:?} ({:?} against {:?}); fields {}; maintained parts {}",
                got.rules.len(),
                want.rules.len(),
                rule.map(|k| got.rules[k].to_string()),
                rule.map(|k| want.rules[k].to_string()),
                if digest(&fields) == digest(&want) { "the same" } else { "differ" },
                if digest(&maint.0) == digest(&maint.1) { "the same" } else { "differ" },
            );
        }
        // And its composed digest (decision 0173 §7), which the program's keys are made of, is the tree's image's.
        assert_eq!(placed.digest(), of.digest(), "a version made from another's has the program's rules and fields, but not their digest");
        Ok(())
    }

    /// The program's digest (`Placed::digest`): its version's, kept as it was made, with its maintained parts'; the
    /// tree's image's where it has no version.
    pub fn digest(&self) -> super::Result<u64> {
        Ok(match &self.version {
            Some(v) if self.maint.is_some() => super::body::Placed::derived(v.clone(), self.maint.clone()).digest(),
            Some(v) if !self.tree_maintained() => super::body::Placed::whole(v.clone()).digest(),
            _ => super::body::Placed::of_program(&self.program)?.digest(),
        })
    }

    /// Whether a tree given beside the version has maintained parts (`again`, `together`), which a version without
    /// `maint` does not state; a tree of the version has none beyond `maint`.
    pub(in crate::code_rule_dsl) fn tree_maintained(&self) -> bool {
        self.program.given_tree().is_some_and(|p| !p.again.is_empty() || !p.together.is_empty())
    }
}

/// What a lowered program's run reads beside its version (`plan::run`, t36 item 7 step 8): each relation's column
/// domains, and the dialect its kinds are of.
#[derive(Clone, Copy)]
pub struct Lowering<'l> {
    pub(in crate::code_rule_dsl) domains: &'l Domains,
    pub dialect: GrammarDialect,
}

struct Constants<'a> {
    tag: u8,
    kinds: KindCatalog,
    names: &'a HashMap<String, i64>,
}

impl Constants<'_> {
    fn of(&self, s: &str, d: Domain) -> Result<i64> {
        Ok(match d {
            Domain::Name => self.names[s],
            // `"sql:select"`: a kind of another grammar, as a guest's are (decision 0058 §2).
            Domain::Kind | Domain::Field if let Some((label, name)) = super::corpus::qualified(s) => {
                let dialect = crate::code_rule_dsl::language::dialect_by_label(label).expect("a registry label");
                let (Some(tag), kinds) = (tape_tag(dialect), dialect.kinds()) else { return Ok(-1) };
                match d {
                    Domain::Kind => match kinds.kind_id(name) {
                        0 => -1,
                        code => i64::from(kind_atom(tag, code)),
                    },
                    _ => kinds.field_id(name).map_or(-1, |code| i64::from(field_atom(tag, code))),
                }
            }
            Domain::Kind => match self.kinds.kind_id(s) {
                0 => -1,
                code => i64::from(kind_atom(self.tag, code)),
            },
            Domain::Field => self.kinds.field_id(s).map_or(-1, |code| i64::from(field_atom(self.tag, code))),
            Domain::Int | Domain::Node => return Err(Error::Plan(format!("the string \"{s}\" stands where an integer goes"))),
        })
    }
    fn term(&self, t: &mut Term, d: Option<Domain>) -> Result<()> {
        match t {
            Term::Str(s) => *t = Term::Int(self.of(s, d.unwrap_or(Domain::Name))?),
            Term::Call(f, args) => {
                let (domains, _) = functor_domains(f, args.len());
                for (a, d) in args.iter_mut().zip(domains) {
                    self.term(a, d)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

fn strings(t: &Term, out: &mut Vec<String>) {
    match t {
        Term::Str(s) => out.push(s.to_string()),
        Term::Call(_, args) => args.iter().for_each(|a| strings(a, out)),
        _ => {}
    }
}

/// Lowers `prog` for `dialect`; `intern` gives the name atoms of strings, interning new ones.
pub fn lower(
    prog: &Program,
    dialect: GrammarDialect,
    intern: &mut dyn FnMut(&[&str]) -> Result<Vec<u32>>,
) -> Result<Lowered> {
    let expanded = expand_memberships(prog)?;
    let prog = expanded.as_ref().unwrap_or(prog);
    let domains = domains(prog)?;
    let tag = tape_tag(dialect).ok_or_else(|| Error::Plan("the dialect has no tape tag".to_owned()))?;
    // Every string a name could be, interned in one call: interning a kind's name too costs one atom.
    let mut all = vec!["+".to_owned(), "-".to_owned(), "*".to_owned()];
    for r in &prog.rules {
        for a in std::iter::once(&r.head).chain(r.body.iter().flat_map(atoms)) {
            a.args.iter().for_each(|t| strings(t, &mut all));
        }
        for l in &r.body {
            if let Literal::Cmp { l, r, .. } = l {
                strings(l, &mut all);
                strings(r, &mut all);
            }
        }
    }
    for rows in prog.facts.values() {
        for v in rows.iter().flatten() {
            if let Value::Str(s) = v {
                all.push(s.to_string());
            }
        }
    }
    all.sort();
    all.dedup();
    let atoms_of = intern(&all.iter().map(String::as_str).collect::<Vec<_>>())?;
    let names: HashMap<String, i64> = all.into_iter().zip(atoms_of.into_iter().map(i64::from)).collect();
    let k = Constants { tag, kinds: dialect.kinds(), names: &names };
    let mut out = prog.clone();
    // Every column's domain as `rule_vars` reads it, made once: made per rule, it was rules × relations (O111).
    let cols = columns_of(&domains);
    for i in 0..out.rules.len() {
        // A constant that does not fit its column names its rule (`the string "" stands where an integer goes`).
        let head = out.rules[i].head.to_string();
        (|| -> Result<()> {
            let vars = rule_vars(&prog.rules[i], &mut |a: &Atom| cols.get(a.rel.as_str()).map(Vec::as_slice))?;
            let rule = &mut out.rules[i];
            let col = |rel: &str, c: usize| domains.get(rel).and_then(|ds| ds.get(c)).copied();
            for (c, t) in rule.head.args.iter_mut().enumerate() {
                k.term(t, col(&rule.head.rel, c))?;
            }
            for l in &mut rule.body {
                match l {
                    Literal::Atom(a) | Literal::Agg { atom: a, .. } => {
                        for (c, t) in a.args.iter_mut().enumerate() {
                            k.term(t, col(&a.rel, c))?;
                        }
                    }
                    Literal::Cmp { l, op, r } => {
                        let of = |t: &Term| if let Term::Var(v) = t { vars.get(v.as_str()).copied() } else { None };
                        // Symbols are ordered by their text, not their atoms: `$t > $s` is `@str_cmp($t, $s) > 0`.
                        // A call whose value is a symbol (`cat(a, "!")`, a text function's) orders by its text too.
                        let symbol = |t: &Term| {
                            of(t) == Some(Domain::Name)
                                || matches!(t, Term::Call(f, _) if f == "concat" || f == "cat"
                                    || crate::code_rule_dsl::logic::strings::functions::native_call(f).is_some_and(|(_, _, k)| k == crate::code_rule_dsl::logic::strings::functions::Kind::Text))
                        };
                        if matches!(*op, "<" | "<=" | ">" | ">=") && (symbol(l) || symbol(r)) {
                            let mut pair = [std::mem::replace(l, Term::Int(0)), std::mem::replace(r, Term::Int(0))];
                            pair.iter_mut().try_for_each(|t| k.term(t, Some(Domain::Name)))?;
                            *l = Term::Call("str_cmp".into(), pair.into());
                            continue;
                        }
                        // Membership of a kind or field, `@equals($k, {"a", "b"})`: `@in` of their atoms.
                        if let Term::Call(f, args) = l
                            && f == "equals"
                            && let [x, set] = args.as_mut_slice()
                            && let Some(d @ (Domain::Kind | Domain::Field)) = of(x)
                        {
                            match set {
                                Term::Call(g, items) if g == crate::code_rule_dsl::logic::strings::SET => items.iter_mut().try_for_each(|i| k.term(i, Some(d)))?,
                                one => k.term(one, Some(d))?,
                            }
                            *f = "in".into();
                            continue;
                        }
                        let (dl, dr) = (of(r), of(l));
                        k.term(l, dl)?;
                        k.term(r, dr)?;
                    }
                    _ => {}
                }
            }

            Ok(())
        })()
        .map_err(|e| match e {
            Error::Plan(m) => Error::Plan(format!("rule for {head}: {m}")),
            e => e,
        })?;
    }
    for (rel, rows) in &mut out.facts {
        for row in rows {
            for (c, v) in row.iter_mut().enumerate() {
                if let Value::Str(s) = v {
                    let d = domains.get(rel.as_str()).and_then(|ds| ds.get(c)).copied().unwrap_or(Domain::Name);
                    *v = Value::Int(k.of(s, d).map_err(|e| match e {
                        Error::Plan(m) => Error::Plan(format!("fact of {rel}, column {c}: {m}")),
                        e => e,
                    })?);
                }
            }
        }
    }
    for members in out.kindsets.values_mut() {
        // `EVERY_KIND`: the set of every kind (core.rr's `within`).
        if members.iter().any(|m| m == super::super::front::EVERY_KIND) {
            *members = (1..k.kinds.kind_count() as u16).map(|code| i64::from(kind_atom(tag, code)).to_string()).collect();
            continue;
        }
        for m in members.iter_mut() {
            *m = k.of(m, Domain::Kind)?.to_string();
        }
    }
    let ops = [names["+"], names["-"], names["*"]];
    Ok(Lowered { program: Tree::given(out), version: None, maint: None, domains, ops, dialect })
}

impl Lowered {
    /// `nearest($n, $a, set)` read from the transducer's file-local FIX (O12, option a): each literal becomes
    /// the input atom `nearest_<set>($n, $a)`, which the run binds beside the base relations
    /// (`facts::vtab::syntax::with_nearest`). The relations the run must make (`nearest_<set>`, and the tree
    /// axes' `rr_inside_<set>` and `rr_has_<set>`), each with its set's kind atoms.
    pub fn transduce_nearest(&mut self) -> Vec<(String, Vec<i64>)> {
        let prog = &mut self.program;
        let mut used = std::collections::BTreeSet::new();
        for rule in &mut prog.rules {
            for literal in &mut rule.body {
                if let Literal::Nearest { n, a, kindset } = literal {
                    used.insert(kindset.clone());
                    *literal = Literal::Atom(Atom {
                        rel: format!("{}{kindset}", crate::code_rule_dsl::logic::front::NEAREST).into(),
                        args: [Term::Var(n.clone().into()), Term::Var(a.clone().into())].into(),
                        neg: false,
                        gated: false,
                    });
                }
            }
        }
        let mut sets = Vec::new();
        // The tree axes (`rr_inside_<set>` and `rr_has_<set>`, decision 0036), made from the same sets' nearest ancestors.
        use crate::code_rule_dsl::logic::front::{AXIS, DESCENDANTS};
        for rel in prog.inputs.iter().filter(|r| crate::code_rule_dsl::logic::front::is_axis(r)) {
            let set = rel.strip_prefix(AXIS).or_else(|| rel.strip_prefix(DESCENDANTS)).expect("an axis");
            self.domains.insert(rel.clone(), vec![Domain::Node, Domain::Node]);
            let kinds = prog.kindsets.get(set).into_iter().flatten().filter_map(|k| k.parse::<i64>().ok()).collect();
            sets.push((rel.clone(), kinds));
        }
        for set in used {
            let rel = format!("{}{set}", crate::code_rule_dsl::logic::front::NEAREST);
            prog.decls.insert(rel.clone().into(), [("n".into(), "node".into()), ("a".into(), "node".into())].into());
            if !prog.inputs.contains(&rel) {
                prog.inputs.push(rel.clone());
            }
            self.domains.insert(rel.clone(), vec![Domain::Node, Domain::Node]);
            let kinds = prog.kindsets.get(&set).into_iter().flatten().filter_map(|k| k.parse::<i64>().ok()).collect();
            sets.push((rel, kinds));
        }
        sets
    }
}

#[cfg(test)]
pub mod tests;

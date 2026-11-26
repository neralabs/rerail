//! The split's preparation passes on the pass manager (decision 0281's path, step 5), each reading the split from the
//! version (`FileLocal`).

use super::FileLocal;
use crate::code_rule_dsl::logic::plan::fields::{Fields, FieldsData};
use crate::code_rule_dsl::logic::program_image::Builder;
use crate::code_rule_dsl::logic::program_image::build::T;
use crate::code_rule_dsl::logic::program_image::manager::{Cx, ProgramPass, ProgramW};
use crate::code_rule_dsl::logic::plan::body;
use crate::code_rule_dsl::logic::program_image::{Lit as L, LitRef, RuleRef, Slot, TermRef, Version};
use std::collections::{BTreeSet, HashMap, HashSet};

mod factorize;
mod push;
mod requests;
pub use factorize::{FactorCaches, FactorizeExports, factorize_intermediates};
pub use requests::RequestsFileLocal;
pub use push::PushLocalJoins;
#[cfg(any(test, feature = "test-hooks"))]
pub use push::Readers;

/// A term of `v`'s rule as a builder's term: a variable by the atom naming it (a derivation's ids are its input's).
pub(super) fn t_of<'a>(v: &'a Version, rule: RuleRef<'a>, t: TermRef<'a>) -> T<'a> {
    match t {
        TermRef::Var(s) => T::VarAtom(rule.var(s)),
        TermRef::Wild => T::Wild,
        TermRef::Top => T::Top,
        TermRef::Int(n) => T::Int(n),
        TermRef::Str(a) => T::StrAtom(a),
        TermRef::Call(f, args) => T::Call(v.text(f), args.map(|a| t_of(v, rule, a)).collect()),
        TermRef::Branded(b) => T::Branded(b),
    }
}

/// One round of `locate_all`: every relation the chunks hand on (an export, or a file-local output) given a location
/// column, so each of its rows names its document (decision 0052). A relation without one, `r(p, l)`, is derived by
/// the chunks as `r_at(d, p, l)` (`r_at_` where the program has an `r_at` of its own), `d` a document its rules bind
/// (from `node`, `doc`, `tape_line`, …, a document column left unnamed included, a node through `node(d, n, …)`, or a
/// file's path through `doc(d, p, …)`, decision 0665), and the cross-file part reads `r(p, l) :- r_at(_, p, l)`. A relation that file-local rules also read, a lattice, or
/// one whose rule binds no location is left, and named in `blocked`: its rows are not attributable.
#[derive(Default)]
pub struct Locate {
    pub located: Vec<String>,
    pub blocked: Vec<String>,
}

impl Locate {
    /// The round's result: the relations located, or those left.
    pub fn result(&self) -> Result<Vec<String>, Vec<String>> {
        if self.blocked.is_empty() { Ok(self.located.clone()) } else { Err(self.blocked.clone()) }
    }
}

/// Every relation the chunks hand on made to name its document per row, so a file's rows can be memoised (decision
/// 0052): `Locate`, and a relation a round leaves (its rows name no document) moved to the cross-file part
/// (`MoveCrossFile`, decision 0290), with every file-local relation that reads it, and the program located again,
/// until every row handed on is located. The chunks then hand on only located rows, and the per-file memo holds for the
/// rest of the program. The relations located; the `Err` names the moved relations where a round blocks one the moves
/// cannot place, which none is.
pub fn locate_all(p: &mut crate::code_rule_dsl::logic::program_image::manager::Pipeline<'_>) -> Result<Vec<String>, Vec<String>> {
    let mut moved: Vec<String> = Vec::new();
    let mut located: Vec<String> = Vec::new();
    loop {
        let mut locate = Locate::default();
        p.program(&mut locate);
        located.extend(locate.located);
        if locate.blocked.is_empty() {
            return Ok(located);
        }
        let fresh: Vec<String> = locate.blocked.into_iter().filter(|r| !moved.contains(r)).collect();
        if fresh.is_empty() {
            moved.sort();
            return Err(moved);
        }
        moved.extend(fresh.iter().cloned());
        p.program(&mut MoveCrossFile(fresh));
    }
}

/// A round of `locate_all`'s move: the relations a round of `Locate` left, their rows naming no document,
/// qualified `CROSS_FILE` (decision 0290). The split then holds them cross-file, with their file-local readers, and the
/// program is located again (`preparation::run`).
pub struct MoveCrossFile(pub Vec<String>);

impl ProgramPass for MoveCrossFile {
    const NAME: &'static str = "move the relations naming no document to the cross-file part";
    fn settings(&self) -> Vec<u8> {
        self.0.join("\n").into_bytes()
    }
    fn run(&mut self, _cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        let words: Vec<(&str, &str)> = self.0.iter().flat_map(|r| [(r.as_str(), super::CROSS_FILE), (r.as_str(), super::MOVED_CROSS_FILE)]).collect();
        w.on()?.fields().add_qualifiers(&words);
        Ok(())
    }
}

/// How a located rule names its document.
#[derive(Clone, Copy)]
enum Doc {
    /// A document column it binds.
    Var(Slot),
    /// One it leaves unnamed: body literal `.0`'s column `.1`, which then names it.
    Unnamed(usize, usize),
    /// Through a node it binds: by `docof` (`true`) or `node`.
    Through(Slot, bool),
    /// Through a file's path it binds or writes out (decision 0665): body literal `.0`'s column `.1`, by
    /// `doc(d, p, …)` of `.2` columns.
    ByPath(usize, usize, usize),
    /// Through a node it leaves unnamed (decision 0680): body literal `.0`'s node column `.1`, `_`, named and read by
    /// `docof` (`.2`) or `node`.
    Wild(usize, usize, bool),
    /// Through a relation of values it reads (decision 0680): body literal `.0`, read from that relation's located
    /// twin at the document.
    Twin(usize),
}

/// A rule of the located program, in order: kept, made from a target's rule, or a target read from its located form.
enum Item<'v> {
    Keep(RuleRef<'v>),
    Made { rule: RuleRef<'v>, at: usize, doc: Doc },
    ReadAt { rel: String, at: usize, width: usize },
    /// A target its file-local rules also read, keyed by a file's path in column `col` (decision 0665): it stays, and
    /// `at` is derived beside it, `r_at(d, …) :- r(…), doc(d, p, …)` of `doc_width` columns; the cross-file part
    /// reads `r_at(_, …)` for it.
    Relay { rel: String, at: usize, width: usize, col: usize, doc_width: usize },
}

impl ProgramPass for Locate {
    const NAME: &'static str = "locate the chunks' rows";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        let v = cx.v();
        let d = cx.get::<FieldsData>()?;
        let f = Fields::over(v, &d);
        let split = cx.get::<FileLocal>()?;
        let (file_local, constants) = split.found();
        let locations = &split.locations;
        let super::on_version::Crossing { exports, .. } = super::on_version::crossing(v, f, &file_local);
        let rules: Vec<RuleRef<'_>> = v.rules().collect();
        let head = |r: &RuleRef<'_>| v.rel_name(r.head().rel());
        let local: BTreeSet<&str> = rules.iter().map(head).filter(|h| file_local.contains(*h) || constants.contains(*h)).collect();
        let targets: BTreeSet<String> = exports
            .iter()
            .map(String::as_str)
            .chain(f.outputs().filter(|o| file_local.contains(*o) && !f.qualified(o, crate::code_rule_dsl::logic::plan::fields::PART_OUT)))
            .filter(|r| local.contains(r) && locations.get(*r).is_none_or(|v| v.is_empty()))
            .map(str::to_owned)
            .collect();
        let docof = f.is_input("docof");
        // `doc`'s width, where it is an input: a path key's row names its document through it (decision 0665).
        let doc_width = f.columns("doc").filter(|_| f.is_input("doc")).map(|c| c.len());
        let path_keys = |rel: &str| -> rerail_blocks::vector::Vector<usize> {
            match super::path_key_column(rel) {
                Some(c) if !f.derives(rel) => rerail_blocks::vector![c],
                _ => split.path_keys.get(rel).cloned().unwrap_or_default(),
            }
        };
        let node_declared = f.declared("node");
        let node_typed = |rel: &str, c: usize| f.column_type(rel, c) == Some("node");
        let doc_type = f.column_type("node", 0).unwrap_or("int");
        // How many rules of a file-local head read each relation (not their own): a target read so is blocked.
        let reads = |r: &RuleRef<'_>| -> BTreeSet<&str> { r.body().filter_map(|l| l.atom()).map(|a| v.rel_name(a.rel())).filter(|a| *a != head(r)).collect() };
        let mut readers: HashMap<&str, usize> = HashMap::new();
        for r in rules.iter().filter(|r| local.contains(head(r))) {
            for a in reads(r) {
                *readers.entry(a).or_default() += 1;
            }
        }
        let mut items: Vec<Item<'_>> = rules.iter().map(|&r| Item::Keep(r)).collect();
        let mut ats: Vec<(String, Vec<(String, String)>)> = Vec::new();
        let mut names: HashSet<String> = f.decls().iter().map(|d| v.text(d.name()).to_owned()).collect();
        // The relayed targets, by name: the cross-file part's reads of each are made reads of its `at`.
        let mut relayed: HashMap<String, usize> = HashMap::new();
        let outputs: BTreeSet<&str> = f.outputs().collect();
        // The derived relations' location columns that hold documents, not nodes (decision 0680): each rule fills the
        // column from a document column of a positive read, an input's (`document_column`) or another such. The
        // greatest such set.
        let mut doc_cols: BTreeSet<(&str, usize)> = rules
            .iter()
            .flat_map(|r| {
                let rel = head(r);
                locations.get(rel).into_iter().flatten().copied().filter(move |&c| f.derives(rel) && f.column_type(rel, c).is_some_and(|t| t != "node")).map(move |c| (rel, c))
            })
            .collect();
        loop {
            let fills = |r: &RuleRef<'_>, c: usize, cols: &BTreeSet<(&str, usize)>| {
                let Some(TermRef::Var(x)) = r.head().args().nth(c) else { return false };
                r.body().any(|l| match l.view() {
                    L::Atom(a) if !a.neg() => {
                        let at = v.rel_name(a.rel());
                        a.args().enumerate().any(|(k, t)| t.var() == Some(x) && (super::document_column(at) == Some(k) || cols.contains(&(at, k))))
                    }
                    _ => false,
                })
            };
            let bad: Vec<(&str, usize)> = doc_cols.iter().copied().filter(|&(rel, c)| !rules.iter().filter(|r| head(r) == rel).all(|r| fills(r, c, &doc_cols))).collect();
            if bad.is_empty() {
                break;
            }
            for x in bad {
                doc_cols.remove(&x);
            }
        }
        // How a rule of a located relation names its document, where `twin_ok` holds the relations of values it may
        // read it through.
        let find_doc = |rule: RuleRef<'_>, twin_ok: &BTreeSet<String>| -> Option<Doc> {
            let positive = || {
                rule.body().enumerate().filter_map(|(i, l)| match l.view() {
                    L::Atom(a) if !a.neg() => Some((i, a)),
                    _ => None,
                })
            };
            // A relation's document column: an input's by its name, or a derived relation's location column that holds
            // documents (`doc_cols`: `pyl_rest(d, l, j, r)`, decision 0680).
            let document = |rel: &str| super::document_column(rel).or_else(|| locations.get(rel).and_then(|cs| cs.iter().copied().find(|&c| doc_cols.contains(&(rel, c)))));
            let named = positive().find_map(|(_, a)| match document(v.rel_name(a.rel())).and_then(|c| a.args().nth(c)) {
                Some(TermRef::Var(s)) => Some(s),
                _ => None,
            });
            if let Some(s) = named {
                return Some(Doc::Var(s));
            }
            if let Some((i, c)) = positive().find_map(|(i, a)| document(v.rel_name(a.rel())).filter(|&c| matches!(a.args().nth(c), Some(TermRef::Wild))).map(|c| (i, c))) {
                return Some(Doc::Unnamed(i, c));
            }
            let nodes = docof || node_declared;
            // A node that places its read's row: a node column of an input, or of a derived relation located by the
            // split. A node a relation of values holds, one with no location column (`spot(s)`, a place of any path),
            // is no document of its row: its row's is its located form's (`Doc::Twin`, decision 0680).
            let places = |rel: &str, c: usize| node_typed(rel, c) && (!f.derives(rel) || locations.get(rel).is_some_and(|cs| !cs.is_empty()));
            if nodes && let Some(n) = positive().find_map(|(_, a)| a.args().enumerate().find_map(|(c, t)| t.var().filter(|_| places(v.rel_name(a.rel()), c)))) {
                return Some(Doc::Through(n, docof));
            }
            if let Some(w) = doc_width
                && let Some((i, c)) = positive().find_map(|(i, a)| path_keys(v.rel_name(a.rel())).into_iter().find(|&c| matches!(a.args().nth(c), Some(TermRef::Var(_) | TermRef::Str(_)))).map(|c| (i, c)))
            {
                return Some(Doc::ByPath(i, c, w));
            }
            // A node left unnamed: each row of its relation is its node's document's (decision 0680).
            if nodes && let Some((i, c)) = positive().find_map(|(i, a)| a.args().enumerate().find(|(c, t)| matches!(t, TermRef::Wild) && node_typed(v.rel_name(a.rel()), *c)).map(|(c, _)| (i, c))) {
                return Some(Doc::Wild(i, c, docof));
            }
            positive().find_map(|(i, a)| twin_ok.contains(v.rel_name(a.rel())).then_some(Doc::Twin(i)))
        };
        // The relations of values a located rule may read its document through (decision 0680): file-local, derived,
        // with no location column (each row one document's), each rule of which finds its document. The greatest such.
        let mut by_head: HashMap<&str, Vec<RuleRef<'_>>> = HashMap::new();
        for &r in &rules {
            by_head.entry(head(&r)).or_default().push(r);
        }
        let mut twin_ok: BTreeSet<String> = by_head
            .keys()
            .copied()
            .filter(|q| file_local.contains(*q) && f.derives(q) && !f.lattice(q) && f.columns(q).is_some() && locations.get(*q).is_none_or(|c| c.is_empty()))
            .map(str::to_owned)
            .collect();
        loop {
            let bad: Vec<String> = twin_ok.iter().filter(|q| by_head[q.as_str()].iter().any(|&r| find_doc(r, &twin_ok).is_none())).cloned().collect();
            if bad.is_empty() {
                break;
            }
            for q in bad {
                twin_ok.remove(&q);
            }
        }
        // The relation a rule's twin read names, where its document is found so.
        let twin_read = |rule: RuleRef<'_>, doc: Doc| -> Option<String> {
            match doc {
                Doc::Twin(i) => rule.body().nth(i).and_then(|l| l.atom()).map(|a| v.rel_name(a.rel()).to_owned()),
                _ => None,
            }
        };
        let alloc = |rel: &str, names: &mut HashSet<String>, ats: &mut Vec<(String, Vec<(String, String)>)>, cols: &[(&str, &str)]| -> usize {
            let mut at = format!("{rel}_at");
            while names.contains(&at) {
                at.push('_');
            }
            let mut at_cols = vec![("d".to_owned(), doc_type.to_owned())];
            at_cols.extend(cols.iter().map(|(n, t)| ((*n).to_owned(), (*t).to_owned())));
            names.insert(at.clone());
            ats.push((at, at_cols));
            ats.len() - 1
        };
        let mut at_of: HashMap<String, usize> = HashMap::new();
        let mut wanted: Vec<String> = Vec::new();
        for rel in &targets {
            let read_locally = readers.get(rel.as_str()).is_some_and(|&n| n > 0) || f.qualified(rel, crate::code_rule_dsl::logic::plan::fields::PART_LOCAL_READ);
            // Read locally but keyed by a path: each row's document is its path's (decision 0665).
            let key = split.path_keys.get(rel.as_str()).and_then(|c| c.first().copied()).filter(|_| read_locally && !f.lattice(rel));
            // A relay made in an earlier round (`r_at…(d, …) :- r(…), doc(…)`) is not made again.
            let key = key.filter(|_| !rules.iter().any(|r| head(r).starts_with(&format!("{rel}_at")) && reads(r).contains(rel.as_str()) && reads(r).contains("doc")));
            if let (Some(col), Some(doc_width), Some(cols)) = (key, doc_width, f.columns(rel)) {
                let mut at = format!("{rel}_at");
                while names.contains(&at) {
                    at.push('_');
                }
                let mut at_cols = vec![("d".to_owned(), doc_type.to_owned())];
                at_cols.extend(cols.iter().map(|(n, t)| ((*n).to_owned(), (*t).to_owned())));
                names.insert(at.clone());
                let k = ats.len();
                ats.push((at, at_cols));
                relayed.insert(rel.clone(), k);
                at_of.insert(rel.clone(), k);
                items.push(Item::Relay { rel: rel.clone(), at: k, width: cols.len(), col, doc_width });
                self.located.push(rel.clone());
                continue;
            }
            let Some(cols) = f.columns(rel).filter(|_| !f.lattice(rel) && !(read_locally && outputs.contains(rel.as_str()))) else {
                self.blocked.push(rel.clone());
                continue;
            };
            let of_rel: Vec<RuleRef<'_>> = items.iter().filter_map(|i| if let Item::Keep(r) = i { Some(*r) } else { None }).filter(|r| head(r) == rel).collect();
            let Some(made) = of_rel.iter().map(|&rule| find_doc(rule, &twin_ok).map(|doc| (rule, doc))).collect::<Option<Vec<(RuleRef<'_>, Doc)>>>() else {
                self.blocked.push(rel.clone());
                continue;
            };
            let k = alloc(rel, &mut names, &mut ats, &cols);
            at_of.insert(rel.clone(), k);
            wanted.extend(made.iter().filter_map(|&(rule, doc)| twin_read(rule, doc)));
            if read_locally {
                // Read by file-local rules too, its rows of values (decision 0680): kept, its located twin made beside
                // it, and the cross-file part's readers read the twin, as a relay's.
                items.extend(made.into_iter().map(|(rule, doc)| Item::Made { rule, at: k, doc }));
                relayed.insert(rel.clone(), k);
                self.located.push(rel.clone());
                continue;
            }
            for r in &of_rel {
                for a in reads(r) {
                    if let Some(n) = readers.get_mut(a) {
                        *n -= 1;
                    }
                }
            }
            items.retain(|i| !matches!(i, Item::Keep(r) if head(r) == rel));
            items.extend(made.into_iter().map(|(rule, doc)| Item::Made { rule, at: k, doc }));
            items.push(Item::ReadAt { rel: rel.clone(), at: k, width: cols.len() });
            self.located.push(rel.clone());
        }
        // The located twins the located rules read their documents through (decision 0680), each made beside its
        // relation from its rules, as a target's located form is; and those their rules read through, in turn.
        while let Some(q) = wanted.pop() {
            if at_of.contains_key(&q) {
                continue;
            }
            let cols = f.columns(&q).expect("a twin's columns");
            let k = alloc(&q, &mut names, &mut ats, &cols);
            at_of.insert(q.clone(), k);
            for &rule in by_head.get(q.as_str()).into_iter().flatten() {
                let doc = find_doc(rule, &twin_ok).expect("a twin's rule finds its document");
                wanted.extend(twin_read(rule, doc));
                items.push(Item::Made { rule, at: k, doc });
            }
            self.located.push(q);
        }
        // A twin's own rules stay (decision 0680), read or not: the split reads its family's rows by them.
        if self.located.is_empty() {
            return Ok(());
        }
        let input = w.input().clone();
        let b = w.derive()?;
        for (at, cols) in &ats {
            let cols: Vec<(&str, &str)> = cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
            b.fields().declare(at, &cols);
        }
        // Each located form says what it is (decision 0680).
        let forms: Vec<(&str, &str)> = ats.iter().map(|(at, _)| (at.as_str(), super::LOCATED_FORM)).collect();
        b.fields().add_qualifiers(&forms);
        // What the cross part reads back from a located form stays there (decision 0665).
        let read_back: Vec<(&str, &str)> = items.iter().filter_map(|i| if let Item::ReadAt { rel, .. } = i { Some((rel.as_str(), super::READ_LOCATED)) } else { None }).collect();
        b.fields().add_qualifiers(&read_back);
        const D: &str = "__located_d";
        // A node a located rule left unnamed (`Doc::Wild`).
        const N: &str = "__located_n";
        for item in items {
            match item {
                Item::Keep(r) if !local.contains(head(&r)) && r.body().filter_map(|l| l.atom()).any(|a| relayed.contains_key(v.rel_name(a.rel()))) => {
                    // A cross-file rule reading a relayed target reads its located form.
                    let args: Vec<T<'_>> = r.head().args().map(|t| t_of(&input, r, t)).collect();
                    let mut rb = b.rule().source(r.id()).head(head(&r), &args);
                    for l in r.body() {
                        rb = match l.view() {
                            L::Atom(a) if relayed.contains_key(v.rel_name(a.rel())) => {
                                let mut args = vec![T::Wild];
                                args.extend(a.args().map(|t| t_of(&input, r, t)));
                                rb.atom_with(ats[relayed[v.rel_name(a.rel())]].0.as_str(), &args, a.neg(), a.gated())
                            }
                            _ => rb.lit(&input, r, l),
                        };
                    }
                    rb.finish()?;
                }
                Item::Keep(r) => {
                    b.keep(&input, r);
                }
                Item::Relay { rel, at, width, col, doc_width } => {
                    let cols: Vec<String> = (0..width).map(|i| format!("__c{i}")).collect();
                    let vars: Vec<T<'_>> = cols.iter().map(|c| T::Var(c.as_str())).collect();
                    let mut args = vec![T::Var(D)];
                    args.extend(vars.iter().cloned());
                    let mut doc = vec![T::Var(D), vars[col].clone()];
                    doc.extend((2..doc_width).map(|_| T::Wild));
                    b.rule().head(ats[at].0.as_str(), &args).atom(rel.as_str(), &vars).atom("doc", &doc).finish()?;
                }
                Item::Made { rule, at, doc } => {
                    let mut args: Vec<T<'_>> = vec![match doc {
                        Doc::Var(s) => T::VarAtom(rule.var(s)),
                        Doc::Unnamed(..) | Doc::Through(..) | Doc::ByPath(..) | Doc::Wild(..) | Doc::Twin(_) => T::Var(D),
                    }];
                    args.extend(rule.head().args().map(|t| t_of(&input, rule, t)));
                    let mut rb = b.rule().source(rule.id()).head(ats[at].0.as_str(), &args);
                    for (i, l) in rule.body().enumerate() {
                        rb = match (doc, l.view()) {
                            (Doc::Unnamed(j, c), L::Atom(a)) if j == i => {
                                let args: Vec<T<'_>> = a.args().enumerate().map(|(k, t)| if k == c { T::Var(D) } else { t_of(&input, rule, t) }).collect();
                                rb.atom_with(a.rel(), &args, a.neg(), a.gated())
                            }
                            (Doc::Wild(j, c, _), L::Atom(a)) if j == i => {
                                let args: Vec<T<'_>> = a.args().enumerate().map(|(k, t)| if k == c { T::Var(N) } else { t_of(&input, rule, t) }).collect();
                                rb.atom_with(a.rel(), &args, a.neg(), a.gated())
                            }
                            (Doc::Twin(j), L::Atom(a)) if j == i => {
                                let mut args = vec![T::Var(D)];
                                args.extend(a.args().map(|t| t_of(&input, rule, t)));
                                rb.atom_with(ats[at_of[v.rel_name(a.rel())]].0.as_str(), &args, a.neg(), a.gated())
                            }
                            _ => rb.lit(&input, rule, l),
                        };
                    }
                    if let Doc::ByPath(i, c, width) = doc {
                        let path = rule.body().nth(i).and_then(|l| l.atom()).and_then(|a| a.args().nth(c)).expect("the path key's term");
                        let mut args = vec![T::Var(D), t_of(&input, rule, path)];
                        args.extend((2..width).map(|_| T::Wild));
                        rb = rb.atom("doc", &args);
                    }
                    if let Doc::Wild(_, _, docof) = doc {
                        rb = if docof { rb.atom("docof", &[T::Var(N), T::Var(D)]) } else { rb.atom("node", &[T::Var(D), T::Var(N), T::Wild, T::Wild, T::Wild]) };
                    }
                    if let Doc::Through(n, docof) = doc {
                        let n = T::VarAtom(rule.var(n));
                        rb = if docof { rb.atom("docof", &[n, T::Var(D)]) } else { rb.atom("node", &[T::Var(D), n, T::Wild, T::Wild, T::Wild]) };
                    }
                    rb.finish()?;
                }
                Item::ReadAt { rel, at, width } => {
                    let cols: Vec<String> = (0..width).map(|i| format!("__c{i}")).collect();
                    let vars: Vec<T<'_>> = cols.iter().map(|c| T::Var(c.as_str())).collect();
                    let mut args = vec![T::Wild];
                    args.extend(vars.iter().cloned());
                    b.rule().head(rel.as_str(), &vars).atom(ats[at].0.as_str(), &args).finish()?;
                }
            }
        }
        Ok(())
    }
}

// ---- rules a pass makes from its input's, written into a derivation (`FactorizeExports`, `PushLocalJoins`) ----

/// A body item: a literal of the source rule, or an atom made (`rel`, args); a negated atom made; a count of a
/// relation's rows into a variable (`var = count : { rel(_, …) }`, its width); a comparison made.
#[derive(Clone)]
pub(super) enum BodyItem<'v> {
    Orig(LitRef<'v>),
    New(String, Vec<NOwned<'v>>),
    Not(String, Vec<NOwned<'v>>),
    Count { var: String, rel: String, width: usize },
    Cmp(NOwned<'v>, crate::code_rule_dsl::logic::program_image::CmpOp, NOwned<'v>),
}

/// A term of a rule being made: one of its source rule's, a variable by name, a number, or a call made.
#[derive(Clone)]
pub(super) enum NOwned<'v> {
    T(TermRef<'v>),
    V(String),
    Int(i64),
    Call(String, Vec<NOwned<'v>>),
}

/// A rule of the round's program: its source rule, its head (the source's, or one made), its body.
#[derive(Clone)]
pub(super) struct Made<'v> {
    pub(super) src: RuleRef<'v>,
    /// `None`: the source's own head.
    pub(super) head: Option<(String, Vec<NOwned<'v>>)>,
    pub(super) body: Vec<BodyItem<'v>>,
    /// Unchanged from its source (kept as it is).
    pub(super) kept: bool,
}

fn t_from<'b>(input: &'b Version, src: RuleRef<'b>, n: &'b NOwned<'b>) -> T<'b> {
    match n {
        NOwned::T(t) => t_of(input, src, *t),
        NOwned::V(s) => T::Var(s.as_str()),
        NOwned::Int(i) => T::Int(*i),
        NOwned::Call(f, args) => T::Call(f.as_str(), args.iter().map(|a| t_from(input, src, a)).collect()),
    }
}

/// The round's program written: the declarations it states and drops, its rules, its keys' qualifiers.
pub(super) struct Round<'v> {
    pub(super) decls: Vec<(String, Vec<(String, String)>)>,
    pub(super) undeclared: Vec<String>,
    pub(super) rules: Vec<Made<'v>>,
    pub(super) no_inline: Vec<String>,
    /// Relations that choose an evaluation, not rows (`file_local::STRATEGY`).
    pub(super) strategy: Vec<String>,
    /// Relations it makes nullary (decision 0488's pair caches' guards).
    pub(super) nullary: Vec<String>,
}

pub(super) fn write(b: &mut Builder<'_>, input: &Version, round: &Round<'_>) -> Result<(), String> {
    write_onto(b, input, round, false, true)
}

/// `write`, into a derivation of `input` that is itself a derivation (`first` false): its rules' sources are its
/// own rules' sources, as `product_demand::Program::derive` records them.
pub(super) fn write_derived(b: &mut Builder<'_>, input: &Version, round: &Round<'_>, first: bool) -> Result<(), String> {
    write_onto(b, input, round, false, first)
}

/// A round's program as an overlay of `input` holding only what the round changes (the rules it rewrites or drops
/// removed, the ones it makes added), and its split, carried from `input`'s across that edit: a round costs its
/// change, where a derivation of the whole program split it whole.
pub(super) fn candidate(input: &std::sync::Arc<Version>, round: &Round<'_>) -> Result<(std::sync::Arc<Version>, std::sync::Arc<FileLocal>), String> {
    let mut b = Builder::on(input);
    write_onto(&mut b, input, round, true, true)?;
    let candidate = b.finish_overlay()?;
    let split = crate::code_rule_dsl::logic::program_image::manager::carried::<FileLocal>(input, &candidate)?;
    Ok((candidate, split))
}

/// `write`, into a derivation of `input` (`onto` false: every rule written, in order), or onto an edit of it (`onto`:
/// the rules kept are there already, and the others are removed).
fn write_onto(b: &mut Builder<'_>, input: &Version, round: &Round<'_>, onto: bool, first: bool) -> Result<(), String> {
    if onto {
        let kept: HashSet<u32> = round.rules.iter().filter(|m| m.kept).map(|m| m.src.id().0).collect();
        let gone: Vec<_> = input.rules().map(|r| r.id()).filter(|id| !kept.contains(&id.0)).collect();
        for id in gone {
            b.remove(id);
        }
    }
    for (rel, cols) in &round.decls {
        let cols: Vec<(&str, &str)> = cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
        b.fields().declare(rel, &cols);
    }
    for rel in &round.undeclared {
        b.fields().undeclare(rel);
    }
    for m in &round.rules {
        if m.kept {
            if !onto && first {
                b.keep(input, m.src);
            } else if !onto {
                b.copy_as_is(input, m.src);
            }
            continue;
        }
        let source = if first { Some(m.src.id()) } else { m.src.source() };
        let mut rb = b.rule();
        if let Some(id) = source {
            rb = rb.source(id);
        }
        rb = match &m.head {
            None => rb.head_from(input, m.src, m.src.head()),
            Some((rel, args)) => {
                let args: Vec<T<'_>> = args.iter().map(|n| t_from(input, m.src, n)).collect();
                rb.head(rel.as_str(), &args)
            }
        };
        for it in &m.body {
            rb = match it {
                BodyItem::Orig(l) => rb.lit(input, m.src, *l),
                BodyItem::New(rel, args) => {
                    let args: Vec<T<'_>> = args.iter().map(|n| t_from(input, m.src, n)).collect();
                    rb.atom(rel.as_str(), &args)
                }
                BodyItem::Not(rel, args) => {
                    let args: Vec<T<'_>> = args.iter().map(|n| t_from(input, m.src, n)).collect();
                    rb.not(rel.as_str(), &args)
                }
                BodyItem::Count { var, rel, width } => {
                    let wild = vec![T::Wild; *width];
                    rb.agg(T::Var(var.as_str()), &crate::code_rule_dsl::logic::front::ast::AggOp::Count, None, rel.as_str(), &wild)
                }
                BodyItem::Cmp(l, op, r) => rb.cmp(t_from(input, m.src, l), *op, t_from(input, m.src, r)),
            };
        }
        rb.finish()?;
    }
    if !round.nullary.is_empty() {
        b.fields().add_nullary(&round.nullary);
    }
    if !round.no_inline.is_empty() {
        let words: Vec<(&str, &str)> = round.no_inline.iter().map(|k| (k.as_str(), "no_inline")).collect();
        b.fields().add_qualifiers(&words);
    }
    if !round.strategy.is_empty() {
        let words: Vec<(&str, &str)> = round.strategy.iter().map(|k| (k.as_str(), super::STRATEGY)).collect();
        b.fields().add_qualifiers(&words);
    }
    Ok(())
}

/// A variable of `rule` by its name (the passes read variables by name, so their sets and searches run in a fixed
/// order).
pub(super) fn name(v: &Version, rule: RuleRef<'_>, s: crate::code_rule_dsl::logic::program_image::Slot) -> String {
    v.text(rule.var(s)).to_owned()
}

pub(super) fn lit_vars(v: &Version, rule: RuleRef<'_>, l: LitRef<'_>) -> BTreeSet<String> {
    body::lit_vars(l).iter().map(|s| name(v, rule, s)).collect()
}

pub(super) fn term_vars(v: &Version, rule: RuleRef<'_>, t: TermRef<'_>) -> BTreeSet<String> {
    let mut out = body::VarSet::new();
    body::term_vars(t, &mut out);
    out.iter().map(|s| name(v, rule, s)).collect()
}

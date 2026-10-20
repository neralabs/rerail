//! Shared literal facts (`docs/sql-set-composition-design.md`, "Shared literal facts"): many
//! clauses of a generated policy set test the same capture-free sub-rule with only a regex changed
//! (a call's callee name `^count$`, `^delete$`, …; a decorator's text). Each such sub-rule is a
//! *member* of a *family*: the sub-rules alike but for their regexes (the *holes*), in one dialect.
//! A family is one per-batch stage, a *fact*, whose rows are `(lit, file, node)`: the nodes each
//! member holds at, its member number in `lit`. One walk computes every member's rows, and each
//! clause reads its own member's (`Op::InFact`), or walks only its member's nodes (`Need::fact`).
//!
//! A member's rule is a *spine* (`Spine`) of `all`, `any` and relations down to its holes, with
//! literal-free tests on the way (`Filter`: kinds, or a stage of the literal-free sub-rule). The
//! fact evaluates the spine top-down over frames of nodes, from every node of the rule's root
//! kinds in the stage's files to the nodes each hole tests, and its verdicts bottom-up over
//! *member frames* `(m, k)`: member `m` holds at the row `k` of a node frame. A hole is the
//! `text_match` terminal over a list of regexes (`Terminal::Regexes`), which returns each key with
//! every member whose regex at that hole matches its text. The rule is capture-free, so a relation
//! holds for a member where some candidate holds for it: set semantics, exact. A member's rows are
//! those its rule holds at, so a clause reading them reads the rule's verdicts.

use super::emit::{self, Emitted, FILES, Reads, TEMP, Writes};
use super::lower::Stage;
use super::plan::NodeSet;
use super::terminals::Terminal;
use crate::code_rule_dsl::language::GrammarDialect;
use regex::Regex;
use std::collections::HashMap;
use std::fmt::Write;

/// A spine's step from a node to its candidates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Axis {
    /// `has`, `stopBy: neighbor`: every child.
    Child,
    /// `has`, `stopBy: end`: every proper descendant.
    Descendant,
    /// `inside`, `stopBy: neighbor`: the parent.
    Parent,
    /// `inside`, `stopBy: end`: every proper ancestor.
    Ancestor,
    /// A `has` with a `field`, `stopBy: neighbor`: the first child in the field.
    FieldChild(u16),
    /// An `inside` with a `field`, `stopBy: neighbor`: the parent, where the node is its first
    /// child in the field.
    FieldParent(u16),
}

/// A literal-free test at a node of a spine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Filter {
    /// The node's kind is one of these.
    Kinds(Vec<u16>),
    /// The node is in the set: the stage of a capture-free sub-rule with no hole.
    Set(NodeSet),
}

/// A family's rule: its holes, and the operators down to them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Spine {
    /// A regex, by its index among the rule's holes in walk order.
    Hole(usize),
    /// `all`: its literal-free terms, tested first, and its terms with holes, all of which hold.
    All {
        filters: Vec<Filter>,
        items: Vec<Spine>,
    },
    /// `any` of terms with holes, one of which holds.
    Any(Vec<Spine>),
    /// A relation over a sub-rule with holes; `kinds`: the sub-rule's root kinds, where known.
    Walk {
        axis: Axis,
        kinds: Option<Vec<u16>>,
        sub: Box<Spine>,
    },
}

/// A family: the stage of its fact, its rule, and its members, each the regexes of its holes.
struct Family {
    stage: usize,
    roots: Vec<u16>,
    spine: Spine,
    members: Vec<Vec<Regex>>,
    index: HashMap<Vec<String>, usize>,
}

/// The families of a program, by their rule (dialect, root kinds and spine).
#[derive(Default)]
pub(super) struct Facts {
    families: Vec<Family>,
    index: HashMap<String, usize>,
}

impl Facts {
    /// The fact stage and member number of the rule `spine` (of root kinds `roots`) with the
    /// regexes `holes`, in `dialect`: a new member of its family, or of a new family whose stage
    /// is the next of `stages`.
    pub(super) fn member(
        &mut self,
        stages: &mut Vec<Stage>,
        dialect: GrammarDialect,
        roots: Vec<u16>,
        spine: Spine,
        holes: Vec<Regex>,
    ) -> (usize, usize) {
        let key = format!("{dialect}:{roots:?}:{spine:?}");
        let family = match self.index.get(&key) {
            Some(&family) => family,
            None => {
                let stage = stages.len();
                stages.push(Stage {
                    dialect,
                    sql: Emitted::default(),
                    three: false,
                    fact: true,
                });
                self.index.insert(key, self.families.len());
                self.families.push(Family {
                    stage,
                    roots,
                    spine,
                    members: Vec::new(),
                    index: HashMap::new(),
                });
                self.families.len() - 1
            }
        };
        let family = &mut self.families[family];
        let patterns: Vec<String> = holes.iter().map(|regex| regex.as_str().to_owned()).collect();
        let next = family.members.len();
        let member = *family.index.entry(patterns).or_insert(next);
        if member == next {
            family.members.push(holes);
        }
        (family.stage, member)
    }

    /// The families' and members' counts, to go back to (`truncate`).
    pub(super) fn mark(&self) -> Vec<usize> {
        self.families
            .iter()
            .map(|family| family.members.len())
            .collect()
    }

    /// Back to the families and members of `mark`, where a clause's lowering failed: its new
    /// stages are gone (`stages`, the stages' count before it).
    pub(super) fn truncate(&mut self, mark: &[usize], stages: usize) {
        self.families.truncate(mark.len());
        self.index.retain(|_, family| *family < mark.len());
        for (family, &members) in self.families.iter_mut().zip(mark) {
            family.members.truncate(members);
            family.index.retain(|_, member| *member < members);
        }
        debug_assert!(self.families.iter().all(|family| family.stage < stages));
    }

    /// Every fact stage's index.
    pub(super) fn stages(&self) -> impl Iterator<Item = usize> + '_ {
        self.families.iter().map(|family| family.stage)
    }

    /// Whether member `member` of the fact `stage` is one.
    pub(super) fn contains(&self, stage: usize, member: usize) -> bool {
        self.families
            .iter()
            .any(|family| family.stage == stage && member < family.members.len())
    }

    /// Each family's statements, into its stage, once every member is known: a hole's regexes
    /// are a terminal of `terminals`.
    pub(super) fn emit(&self, terminals: &mut Vec<Terminal>, stages: &mut [Stage]) {
        for family in &self.families {
            let mut holes: Vec<Vec<(Regex, Vec<u32>)>> = Vec::new();
            for (member, regexes) in family.members.iter().enumerate() {
                for (hole, regex) in regexes.iter().enumerate() {
                    if holes.len() <= hole {
                        holes.push(Vec::new());
                    }
                    let patterns = &mut holes[hole];
                    match patterns
                        .iter_mut()
                        .find(|(pattern, _)| pattern.as_str() == regex.as_str())
                    {
                        Some((_, members)) => members.push(member as u32),
                        None => patterns.push((regex.clone(), vec![member as u32])),
                    }
                }
            }
            let terms: Vec<usize> = holes
                .into_iter()
                .map(|patterns| {
                    terminals.push(Terminal::regexes(patterns));
                    terminals.len() - 1
                })
                .collect();
            let mut writer = Writer {
                out: Emitted::default(),
                next: 0,
                terms: &terms,
            };
            stages[family.stage].sql = writer.fact(&family.roots, &family.spine);
        }
    }
}

/// Whether member `member` of the fact `stage` holds at node `node` of file `file` (SQL
/// expressions), as an SQL condition.
pub(super) fn holds(stage: usize, member: usize, file: &str, node: &str) -> String {
    // Mutant 23: a clause reads the next member's rows.
    let member = if super::mutant(23) { member + 1 } else { member };
    format!(
        "EXISTS (SELECT 1 FROM {} st WHERE st.lit = {member} AND st.file = {file} AND st.node = {node})",
        emit::stage_table(stage)
    )
}

/// A family's statements as they are written.
struct Writer<'t> {
    out: Emitted,
    next: usize,
    /// Each hole's `text_match` terminal.
    terms: &'t [usize],
}

impl Writer<'_> {
    /// The fact's statements: the last selects `(lit, file, node)`.
    fn fact(&mut self, roots: &[u16], spine: &Spine) -> Emitted {
        let root = self.nodes(
            &format!(
                "SELECT NULL, n.file, n.id, NULL FROM node n WHERE n.file IN ({FILES}) AND n.kind IN ({})",
                list(roots)
            ),
            Reads::always(),
        );
        self.out.root = Some(root.clone());
        let members = self.spine(spine, &root);
        self.out.push(
            format!(
                "SELECT DISTINCT z.m, f.file, f.node FROM {members} z JOIN {root} f ON f.k = z.k"
            ),
            Reads::join(&[&members]).writes(Writes::None),
        );
        std::mem::take(&mut self.out)
    }

    fn name(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{TEMP}{prefix}{}", self.next)
    }

    /// A node frame `(k, file, node, pk)` filled by `select`: `pk` is the row of the frame it
    /// walked from.
    fn nodes(&mut self, select: &str, reads: Reads) -> String {
        self.frame("y", "k INTEGER PRIMARY KEY, file, node, pk", "", select, reads)
    }

    /// A member frame `(m, k)` filled by `select` (with CTEs `with`): member `m` holds at row `k`
    /// of a node frame.
    fn members(&mut self, with: &str, select: &str, reads: Reads) -> String {
        self.frame("z", "m, k", with, select, reads)
    }

    fn frame(
        &mut self,
        prefix: &str,
        columns: &str,
        with: &str,
        select: &str,
        reads: Reads,
    ) -> String {
        let name = self.name(prefix);
        self.out.push(
            format!("CREATE TEMP TABLE {name}({columns})"),
            Default::default(),
        );
        self.out.push(
            format!("{with}INSERT INTO {name} {select}"),
            reads.writes(Writes::Insert(name.clone())),
        );
        name
    }

    /// The members `spine` holds for at the rows of the node frame `input`.
    fn spine(&mut self, spine: &Spine, input: &str) -> String {
        match spine {
            // The terminal is read once, over the frame's keys (a `MATERIALIZED` CTE: inlined,
            // the join would pass it one key per row).
            Spine::Hole(hole) => {
                let term = self.terms[*hole];
                self.members(
                    &format!(
                        "WITH t AS MATERIALIZED (SELECT ek, idx FROM text_match WHERE term = {term} \
                         AND ek IN (SELECT file * 4294967296 + node FROM {input})) "
                    ),
                    &format!(
                        "SELECT t.idx, f.k FROM {input} f JOIN t ON t.ek = f.file * 4294967296 + f.node"
                    ),
                    Reads::join(&[input]),
                )
            }
            Spine::All { filters, items } => {
                let mut current = input.to_owned();
                for filter in filters {
                    current = self.filter(filter, &current);
                }
                let found: Vec<String> = items
                    .iter()
                    .map(|item| self.spine(item, &current))
                    .collect();
                self.combine(&found, " INTERSECT ", Reads::join)
            }
            Spine::Any(items) => {
                let found: Vec<String> = items.iter().map(|item| self.spine(item, input)).collect();
                self.combine(&found, " UNION ", Reads::union)
            }
            Spine::Walk { axis, kinds, sub } => {
                let candidates = self.walk(*axis, kinds.as_deref(), input);
                let found = self.spine(sub, &candidates);
                self.members(
                    "",
                    &format!(
                        "SELECT DISTINCT z.m, c.pk FROM {found} z JOIN {candidates} c ON c.k = z.k"
                    ),
                    Reads::join(&[&found]),
                )
            }
        }
    }

    /// One member frame of `found`'s rows set-combined by `operator`, or the one frame there is.
    fn combine(
        &mut self,
        found: &[String],
        operator: &str,
        reads: fn(&[&str]) -> Reads,
    ) -> String {
        if let [one] = found {
            return one.clone();
        }
        let select = found
            .iter()
            .map(|frame| format!("SELECT m, k FROM {frame}"))
            .collect::<Vec<_>>()
            .join(operator);
        let inputs: Vec<&str> = found.iter().map(String::as_str).collect();
        self.members("", &select, reads(&inputs))
    }

    /// The rows of `input` that pass `filter`, with their `k`.
    fn filter(&mut self, filter: &Filter, input: &str) -> String {
        match filter {
            Filter::Kinds(kinds) => self.nodes(
                &format!(
                    "SELECT f.k, f.file, f.node, f.pk FROM {input} f JOIN node n ON n.file = f.file AND n.id = f.node \
                     WHERE n.kind IN ({})",
                    list(kinds)
                ),
                Reads::join(&[input]),
            ),
            Filter::Set(set) => {
                let table = emit::stage_table(set.stage);
                let reads = match emit::set_inputs(*set) {
                    Some(_) => Reads::join(&[input, &table]),
                    None => Reads::join(&[input]),
                };
                self.nodes(
                    &format!(
                        "SELECT f.k, f.file, f.node, f.pk FROM {input} f WHERE {}",
                        emit::in_set(*set, "f.file", "f.node")
                    ),
                    reads,
                )
            }
        }
    }

    /// The candidates of `axis` from each row of `input` (of `kinds`, where known), `pk` the row.
    fn walk(&mut self, axis: Axis, kinds: Option<&[u16]>, input: &str) -> String {
        let c = match kinds {
            None => String::new(),
            Some(kinds) => format!(" AND c.kind IN ({})", list(kinds)),
        };
        let at = "JOIN node fn ON fn.file = f.file AND fn.id = f.node";
        let from = match axis {
            Axis::Child => format!("JOIN node c ON c.file = f.file AND c.parent = f.node{c}"),
            Axis::Descendant => format!(
                "{at} JOIN node c ON c.file = f.file AND c.id > f.node AND c.id <= fn.\"last\"{c}"
            ),
            Axis::Parent => format!("{at} JOIN node c ON c.file = f.file AND c.id = fn.parent{c}"),
            Axis::Ancestor => format!(
                "JOIN node c ON c.file = f.file AND c.id < f.node AND c.\"last\" >= f.node{c}"
            ),
            Axis::FieldChild(field) => format!(
                "JOIN node c ON c.file = f.file AND c.parent = f.node{c} WHERE c.field = {field} AND c.ffirst = 1"
            ),
            Axis::FieldParent(field) => format!(
                "{at} JOIN node c ON c.file = f.file AND c.id = fn.parent{c} WHERE fn.field = {field} AND fn.ffirst = 1"
            ),
        };
        self.nodes(
            &format!("SELECT NULL, c.file, c.id, f.k FROM {input} f {from}"),
            Reads::join(&[input]),
        )
    }
}

fn list(items: &[u16]) -> String {
    let mut out = String::new();
    for (at, item) in items.iter().enumerate() {
        if at > 0 {
            out.push_str(", ");
        }
        let _ = write!(out, "{item}");
    }
    out
}

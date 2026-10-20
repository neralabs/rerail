//! The recursive reads a unit's evaluation needs: those made from rows the matcher evaluates.
//!
//! A set evaluation tests every candidate of a relation, where the matcher stops at the first
//! match in walk order (`recursion`'s proof). A body that walks back to its own key, as a function
//! calling itself does where the walk descends from its definition to its calls, reads the key it
//! evaluates through a candidate past the first match: the matcher never evaluates it there, but
//! the read graph has a cycle. So a pass records, of the reads its body makes, only those from the
//! rows the matcher evaluates, and the read graph `exec::acyclic` checks holds those alone.
//!
//! # k-spaces
//!
//! Every operator keeps its input rows' `k` (`plan`'s contract: an output row is an input row's),
//! so the frames of a body fall into k-spaces: those of one frame that numbers its rows afresh
//! (`k` NULL) and of every frame derived from it. A fresh frame derives each row from a row of
//! its input's space, its parent, recorded in the frame: a relation's, a count's or
//! `redefines.every`'s candidates and an `nthChild` probe's siblings carry `pk`, the parent's `k`;
//! a keyed operator's distinct keys (`keyed`) come from every input row with the key's columns.
//! A unit's demanded keys (`u`) are the root of its body's spaces: a row is a key the pass
//! evaluates. A statement's root frame (`r`, every root of the clause) is the root of its own.
//!
//! # Needed rows
//!
//! A row is needed where the matcher, evaluating the row's key (or the clause at its root),
//! reaches it:
//! - a key, a root: always (which keys the check starts from is below);
//! - a first-match search's candidate: where its parent is, and no candidate before it in walk
//!   order (`ord`) matched: true, or, in a strict search, unknown; a non-strict one passes over
//!   unknown candidates, so only true ones end it. A candidate past that first one is cut. Equal
//!   orders are kept, and so are a strict search's candidates past an unknown stop: either way
//!   more rows are needed, never fewer;
//! - a count's, `redefines.every`'s or an `nthChild` probe's candidate: where its parent is (the
//!   matcher tests them all), and so a first match over a walk of one candidate (`Walk::single`);
//! - a keyed operator's key: where one of the input rows it stands for is.
//!
//! Every other operator decides a row from its own row alone: `all` passes the rows its earlier
//! terms kept, `any` gives each branch the rows no earlier branch decided, as the matcher tests
//! them. After a unit's body (and after the statement's operator, for its own rows) each needed
//! table is computed from its parent's, in creation order, and every recursive reference is
//! recorded, from the site's row's key to the key it demands, only where that row is needed:
//! into the reads of the referring unit's component (`e{scc}`), or, from the statement's own
//! rows, into its start table (`ex`).
//! Which tables it computes is static; a space under no first-match search has every row needed
//! and no table. A space whose rows' derivation is not recorded (a fresh frame this module does
//! not know of keeps no space, and every frame under it none either) has its reads recorded
//! whole: a missing lineage only adds reads, so it only adds refusals.
//!
//! # Why the needed reads decide
//!
//! The keys the check holds are those the recorded reads reach from the statement's own needed
//! rows (`exec::acyclic`): each demanded by a needed row of the clause, or of a key already held,
//! whatever component it is in. A key no needed row of a held key or of the clause reads, the
//! matcher never evaluates; its answer reaches no output.
//!
//! The pass's answers `A` satisfy `A = T(A)`, and the reads among the held keys form an acyclic
//! graph `N`. By induction over an order of the held keys where each comes after every key it
//! reads in `N`: at a key, the needed rows read keys before it, whose answers are the matcher's,
//! so each needed row's verdict and bindings are the matcher's (by induction over the body's
//! operators, each a function of its own row). A first-match search's first matched candidate
//! is then the matcher's too: every candidate before it is needed and false (or unknown in a
//! non-strict search), as the matcher finds it. Its verdict and bindings read that candidate
//! alone, so the cut candidates, and whatever they read, decide nothing: the key's answer is
//! the matcher's, which reads only needed rows, so it terminates within `N`'s depth. The clause's
//! own needed rows read held keys alone, so its answers are the matcher's the same way.
//!
//! The semi-naive loop marks a key dirty where a key it read changed (`Emitter::recursion_loop`),
//! now over the needed reads: a key whose needed reads keep their answers evaluates its needed
//! rows alike, so it finds the same first matches and the same answer again, whatever the cut
//! rows would read. Its cut rows may demand keys no needed row reads: those are evaluated too,
//! but not held, and their answers decide nothing. A key outside the held ones whose loop does
//! not settle still runs the loop to its pass limit, which refuses the batch: that only refuses
//! more.

use super::{Emitter, Reads};
use crate::code_rule_dsl::sql::plan::Walk;

/// How a k-space's rows derive from its parent's (the module doc's needed rows).
#[derive(Clone)]
pub(super) enum Derive {
    /// A unit's demanded keys: every row is needed.
    Keys,
    /// Each row from its parent row `pk`: needed where the parent is.
    Probe,
    /// A first-match search's candidates: needed where the parent is and no earlier candidate
    /// is in `matched`, the sub-rule's rows (true or unknown); `open` counts the true ones only.
    First { matched: Option<String>, open: bool },
    /// A keyed operator's keys of `input`'s rows (`columns`): needed where one of its rows is.
    Keyed { input: String, columns: Vec<String> },
}

/// A k-space: the fresh frame `frame` and the frames that keep its rows.
pub(super) struct Space {
    parent: Option<usize>,
    /// The unit body (`Member::id`) it lies in.
    member: usize,
    frame: String,
    derive: Derive,
    /// Its needed rows' table, once emitted: `None` inside where every row is needed.
    needed: Option<Option<String>>,
}

/// A unit's body being emitted: its keys' space, and its recursive reads from other spaces,
/// recorded after the body (`Emitter::member_end`).
pub(super) struct Member {
    id: usize,
    root: usize,
    reads: Vec<Read>,
}

/// A recursive read into the table `reads` from the rows of `space`'s frame `input` (`f`), joined
/// with the unit's demand table `demand`: `head` up to `FROM … f`, `tail` the joins after it.
struct Read {
    space: usize,
    reads: String,
    input: String,
    demand: String,
    head: String,
    tail: String,
}

impl Emitter<'_> {
    /// Records that `to` keeps `from`'s rows' `k`: it lies in `from`'s space.
    pub(super) fn same_space(&mut self, from: &str, to: &str) {
        if from != to
            && let Some(&space) = self.space_of.get(from)
        {
            self.space_of.entry(to.to_owned()).or_insert(space);
        }
    }

    /// Records that `frame` numbers its rows afresh from `input`'s rows, as `derive` says, where
    /// `input` lies in a space of the unit body being emitted.
    pub(super) fn open_space(&mut self, frame: &str, input: &str, derive: Derive) {
        let (Some(&parent), Some(member)) = (self.space_of.get(input), self.members.last()) else {
            return;
        };
        if self.spaces[parent].member != member.id {
            return;
        }
        let member = member.id;
        self.space_of.insert(frame.to_owned(), self.spaces.len());
        self.spaces.push(Space {
            parent: Some(parent),
            member,
            frame: frame.to_owned(),
            derive,
            needed: None,
        });
    }

    /// Records that `candidates` (a `Probe` space) are a first-match search's over `walk`, which
    /// stops at its first candidate in `matched` (`first_matched`); `open`, its first true one.
    pub(super) fn first_match(&mut self, candidates: &str, walk: Walk, open: bool) {
        if walk.single() {
            return;
        }
        if let Some(&space) = self.space_of.get(candidates)
            && matches!(self.spaces[space].derive, Derive::Probe)
        {
            self.spaces[space].derive = Derive::First {
                matched: None,
                open,
            };
        }
    }

    /// Records the sub-rule's rows of the first-match search over `candidates`.
    pub(super) fn first_matched(&mut self, candidates: &str, matched: &str) {
        if let Some(&space) = self.space_of.get(candidates)
            && let Derive::First { matched: rows, .. } = &mut self.spaces[space].derive
        {
            *rows = Some(matched.to_owned());
        }
    }

    /// Starts the body of a unit over its demanded keys `keys`.
    pub(super) fn member_start(&mut self, keys: &str) {
        let id = self.next_member;
        self.next_member += 1;
        let root = self.spaces.len();
        self.space_of.insert(keys.to_owned(), root);
        self.spaces.push(Space {
            parent: None,
            member: id,
            frame: keys.to_owned(),
            derive: Derive::Keys,
            needed: Some(None),
        });
        self.members.push(Member {
            id,
            root,
            reads: Vec::new(),
        });
    }

    /// A recursive read into `reads` from the rows `f` of `input` joined with `demand`, `head` up
    /// to `FROM {input} f` and `tail` the joins after it: recorded now where every row of `input`
    /// is needed (a key's row, or a frame in no known space), else after the body, from its
    /// needed rows.
    pub(super) fn record_read(
        &mut self,
        reads: &str,
        input: &str,
        demand: &str,
        head: String,
        tail: String,
    ) {
        if let Some(member) = self.members.last_mut()
            && let Some(&space) = self.space_of.get(input)
            && self.spaces[space].member == member.id
            && space != member.root
        {
            member.reads.push(Read {
                space,
                reads: reads.to_owned(),
                input: input.to_owned(),
                demand: demand.to_owned(),
                head,
                tail,
            });
            return;
        }
        self.insert(
            reads,
            format!("{head}{tail}"),
            Reads::join(&[input, demand]),
        );
    }

    /// Ends a unit's body: its deferred reads, each from the needed rows of its space.
    pub(super) fn member_end(&mut self) {
        let member = self.members.pop().expect("a unit body being emitted");
        for read in member.reads {
            let needed = self.needed(read.space);
            let mut inputs = vec![read.input.as_str(), read.demand.as_str()];
            let filter = needed.as_ref().map_or(String::new(), |table| {
                inputs.push(table);
                format!(" WHERE f.k IN (SELECT k FROM {table})")
            });
            let reads = Reads::join(&inputs);
            self.insert(
                &read.reads,
                format!("{}{}{filter}", read.head, read.tail),
                reads,
            );
        }
    }

    /// The table of the `k` of `space`'s needed rows, emitted on first use (its parent's first);
    /// `None` where every row is needed.
    fn needed(&mut self, space: usize) -> Option<String> {
        if let Some(needed) = &self.spaces[space].needed {
            return needed.clone();
        }
        let parent = self.spaces[space]
            .parent
            .expect("a unit's keys are all needed");
        let above = self.needed(parent);
        let frame = self.spaces[space].frame.clone();
        // The rows whose parent row `alias` is needed.
        let under = |alias: &str| {
            above
                .as_ref()
                .map(|table| format!("{alias} IN (SELECT k FROM {table})"))
        };
        // The select, and the frames it reads (each output row stems from a row of each).
        let mut inputs = vec![frame.clone()];
        inputs.extend(above.clone());
        let select = match self.spaces[space].derive.clone() {
            Derive::Keys => unreachable!("a unit's keys are a root"),
            Derive::Probe => {
                under("g.pk").map(|under| format!("SELECT g.k FROM {frame} g WHERE {under}"))
            }
            Derive::Keyed { input, columns } => under("f.k").map(|under| {
                let matched: String = columns
                    .iter()
                    .map(|column| format!(" AND q.{column} IS f.{column}"))
                    .collect();
                let select = format!(
                    "SELECT DISTINCT q.k FROM {input} f CROSS JOIN {frame} q \
                     ON q.file = f.file AND q.node = f.node{matched} WHERE {under}"
                );
                inputs.push(input);
                select
            }),
            // The first candidate per parent row in `matched`, then the candidates up to it.
            Derive::First {
                matched: Some(matched),
                open,
            } => {
                let mut counted: Vec<String> = under("g.pk").into_iter().collect();
                if open {
                    counted.push("s.unk = 0".to_owned());
                }
                let counted = if counted.is_empty() {
                    String::new()
                } else {
                    format!(" WHERE {}", counted.join(" AND "))
                };
                let mut grouped: Vec<&str> = inputs.iter().map(String::as_str).collect();
                grouped.push(&matched);
                let first = self.grouped(
                    "o",
                    &format!(
                        "SELECT g.pk, MIN(g.ord) FROM {matched} s JOIN {frame} g ON g.k = s.k{counted} GROUP BY g.pk"
                    ),
                    Reads::join(&grouped),
                );
                let under = under("g.pk").map_or(String::new(), |under| format!(" AND {under}"));
                Some(format!(
                    "SELECT g.k FROM {frame} g LEFT JOIN {first} w ON w.pk = g.pk \
                     WHERE (w.pk IS NULL OR g.ord <= w.o){under}"
                ))
            }
            // Not reached: every search records its sub-rule's rows. Were one not to, every
            // row would count as needed, which only adds reads.
            Derive::First { matched: None, .. } => {
                debug_assert!(false, "a first-match search without its sub-rule's rows");
                None
            }
        };
        let needed = select.map(|select| {
            // `m`: `n{unit}` is a unit's fresh answers (`Emitter::recursion_loop`).
            let name = self.name("m");
            self.create(&name, "k INTEGER PRIMARY KEY");
            let inputs: Vec<&str> = inputs.iter().map(String::as_str).collect();
            self.insert(
                &name,
                format!("INSERT INTO {name} {select}"),
                Reads::join(&inputs),
            );
            name
        });
        self.spaces[space].needed = Some(needed.clone());
        needed
    }
}

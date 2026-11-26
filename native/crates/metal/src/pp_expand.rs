//! Macro expansion on the device (decision 0448), `pp_expand.uglykl`. Built so far: the setup's
//! definitions (S1, S2, S4), read from the tapes in place.

use crate::Error;
use crate::pp_arena::{Arena, Buf, Lay, Scan, Sort};
use crate::pp_eval::{Front, TO_ALL, TO_DEFS, TO_VIS, Unit, name_key, on_unit, preempted_again};

/// A definition (`PpDef` in the UKL): the unit's nodes of its name, its `params` node (0: none)
/// and its body's first leaf; the leaves of each; its flags; its first parameter slot and item.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PpDef {
    pub name: u32,
    pub params: u32,
    pub n_params: u32,
    pub body: u32,
    pub n_body: u32,
    pub flags: u32,
    pub pfirst: u32,
    pub ifirst: u32,
}

/// `PpDef::flags`.
pub const D_DEFINE: u32 = 1;
pub const D_FNLIKE: u32 = 2;
pub const D_VARIADIC: u32 = 4;
pub const D_BAD: u32 = 8;
/// An item's parameter where it is none, and its flags above it.
pub const NO_PARAM: u32 = 0xffff;
pub const I_PASTE: u32 = 1 << 16;
pub const I_SPACE: u32 = 1 << 17;
/// A body item that is `#` (or `%:`): what a stringizing reads, the tape not read again.
pub const I_HASH: u32 = 1 << 18;

/// The setup's definitions on the device: one a directive row, each parameter list's leaves'
/// keys (a parameter's at its leaf, 0 elsewhere), and each body leaf's item.
pub struct Defs {
    pub defs: Buf,
    pcount: Buf,
    bcount: Buf,
    pfirst: Buf,
    ifirst: Buf,
    pscan: Scan,
    bscan: Scan,
    pub pkeys: Buf,
    pub items: Buf,
    /// Each body leaf's name's rows in the sorted rows (`pd_item_rows`), two words a leaf.
    irows: Buf,
    /// Each body leaf's place in a `__VA_OPT__` (`VA_*` in `pp_expand.uglykl`; decision 0527 §2).
    pub vai: Buf,
    /// Set where a store's room ran out (decision 0448 §2): the unit runs again at the next rung.
    pub room: Buf,
    cap: u32,
}

/// The rungs of the setup's stores: rung `k` holds `nodes >> (RUNGS - 1 - k)` leaves, so the last
/// holds every node of the unit, the most there can be.
pub const RUNGS: u32 = 4;

impl Defs {
    /// Room for `unit`'s rows, and at rung `rung` for their parameters' and bodies' leaves: each
    /// a node, so the last rung's, the unit's nodes, bounds them.
    pub fn new(arena: &Arena, unit: &Unit, rung: u32) -> Result<Defs, Error> {
        let rows = unit.dir_bound;
        let nodes = (unit.node_bound >> (RUNGS - 1 - rung.min(RUNGS - 1))).max(256);
        let words = |k: usize| arena.zeroed::<u32>(k.max(1));
        Ok(Defs {
            defs: arena.zeroed::<PpDef>(rows)?,
            pcount: words(rows)?,
            bcount: words(rows)?,
            pfirst: words(rows)?,
            ifirst: words(rows)?,
            pscan: Scan::new(arena)?,
            bscan: Scan::new(arena)?,
            pkeys: arena.zeroed::<u64>(nodes)?,
            items: words(nodes)?,
            irows: words(2 * nodes)?,
            vai: words(nodes)?,
            room: words(1)?,
            cap: nodes as u32,
        })
    }

    /// The definitions' slots of the layout: S1 (`pd_defs`, a thread a row), the scans of the
    /// counts, S2 (`pd_params`, a thread a parameter list's leaf), S4 (`pd_items`, a thread a
    /// body leaf) and the `__VA_OPT__`s (`pd_va_opt`, a thread a body leaf), as phases of
    /// the unit's program (`pp_grid.uglykl`).
    pub fn lay(&self, l: &mut Lay) {
        let va = name_key(b"__VA_ARGS__");
        let vo = name_key(b"__VA_OPT__");
        l.at("DEFS", &self.defs).at("PCOUNT", &self.pcount).at("BCOUNT", &self.bcount).at("PFIRST", &self.pfirst).at("IFIRST", &self.ifirst);
        l.at("PTOTAL", &self.pscan.total).at("ITOTAL", &self.bscan.total).at("PKEYS", &self.pkeys).at("ITEMS", &self.items).at("IROWS", &self.irows).at("VAI", &self.vai).at("ROOM", &self.room);
        l.word("CAP", self.cap).word("VA_LO", va as u32).word("VA_HI", (va >> 32) as u32).word("VO_LO", vo as u32).word("VO_HI", (vo >> 32) as u32);
    }
}

/// The setup's definitions, read back for tests.
pub struct DefsRead {
    pub defs: Vec<PpDef>,
    pub pkeys: Vec<u64>,
    pub items: Vec<u32>,
}

/// The front passes and the definitions over `unit`, in one command buffer, read back: from
/// rung 0, and at the next where a store's room ran out.
pub fn definitions(unit: &Unit) -> Result<(crate::pp_eval::Evaluated, DefsRead), Error> {
    definitions_from(unit, 0)
}

/// `definitions` from rung `rung`.
pub fn definitions_from(unit: &Unit, rung: u32) -> Result<(crate::pp_eval::Evaluated, DefsRead), Error> {
    preempted_again(|| definitions_at(unit, rung))
}

fn definitions_at(unit: &Unit, rung: u32) -> Result<(crate::pp_eval::Evaluated, DefsRead), Error> {
    let arena = Arena::new();
    let front = Front::new(&arena, unit, false)?;
    let defs = Defs::new(&arena, unit, rung)?;
    arena.lease()?;
    let mut l = Lay::new();
    front.lay(&mut l, TO_DEFS);
    defs.lay(&mut l);
    on_unit(&arena, unit, &front, &l)?;
    // SAFETY: the device finished writing `room`.
    if unsafe { *defs.room.host().cast::<u32>() } != 0 {
        if rung + 1 >= RUNGS {
            return Err(crate::pp_arena::capacity(concat!(file!(), ":", line!())));
        }
        drop((front, defs));
        return definitions_from(unit, rung + 1);
    }
    let ev = front.read()?;
    let rows = ev.rows.len();
    // SAFETY: the device finished; each buffer holds at least the counts read.
    let (pt, bt) = unsafe { (*defs.pscan.total.host().cast::<u32>() as usize, *defs.bscan.total.host().cast::<u32>() as usize) };
    let read = unsafe {
        DefsRead {
            defs: std::slice::from_raw_parts(defs.defs.host().cast::<PpDef>(), rows).to_vec(),
            pkeys: std::slice::from_raw_parts(defs.pkeys.host().cast::<u64>(), pt).to_vec(),
            items: std::slice::from_raw_parts(defs.items.host().cast::<u32>(), bt).to_vec(),
        }
    };
    Ok((ev, read))
}

/// An instance of a file in the unit's layout (`PpInst` in the UKL), as P1 lays it out on the
/// device (decision 0467 §2): its file, its first directive and its first stretch in the layout's
/// words.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PpInst {
    pub file: u32,
    pub dir_first: u32,
    pub gap_first: u32,
}

/// An instance of the include tree (`PpNode` in the UKL, decision 0467 §2): its file, its parent
/// (`NO_PARENT` for a root) and the parent's directive that entered it (a root's rank: 0 the
/// predefines, 1 the main file, the next instance), its depth, and its unit in a batch.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PpNode {
    pub file: u32,
    pub parent: u32,
    pub pdir: u32,
    pub depth: u32,
    pub unit: u32,
}

/// P1's answer on the device, read back for tests: the instances (the roots given, then those
/// the device made), whether each is live, each one's first directive word, and each directive's
/// and stretch's slot (§2) and activity (§4), in the layout's words.
#[derive(Debug)]
pub struct P1Out {
    pub nodes: Vec<PpNode>,
    pub live: Vec<u32>,
    pub dir_first: Vec<u32>,
    pub dir_slots: Vec<u32>,
    pub gap_slots: Vec<u32>,
    pub dir_active: Vec<u32>,
    pub gap_active: Vec<u32>,
    /// P1's refusals and the round's requests: with either, no lookups.
    pub flags: u32,
    pub requests: Vec<Request>,
    /// The expansion's refusals (`X_*`) in the tests' lines (decision 0467 §4).
    pub refused: u32,
}

/// A root's parent.
pub const NO_PARENT: u32 = u32::MAX;

/// A file table row's file where the search found none.
pub const NOT_FOUND: u32 = u32::MAX;

/// An include the file table has no row for (decision 0467 §1): its key (unit-free; the table's
/// row takes it), the including file, its name, and whether it is `<…>` and an `#include_next`
/// (the main file's is an `#include`). The host answers it and nothing else.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Request {
    pub key: u64,
    /// The batch's unit that made it, and its file there (the unit's file index).
    pub unit: u32,
    pub file: u32,
    /// The name, as the device spelled it (phase 2: a literal's bytes, or an expansion's).
    pub name: Vec<u8>,
    pub angle: bool,
    pub next: bool,
}

/// P1's refusals (`pp_p1.uglykl`'s `P1_*`): the cases a later step of decision 0467 builds.
pub const P1_ROOM: u32 = 1;
pub const P1_EMBED: u32 = 4;
pub const P1_ITERATIONS: u32 = 16;
/// `defined` made by a macro's expansion, or read inside a call's arguments, in an `#if`.
pub const P1_IF_DEFINED: u32 = 32;
/// An `#if` past the evaluator's stacks, or a constant past its bytes.
pub const P1_IF_BIG: u32 = 64;

/// The `#if` operators' kinds (`pp_if.uglykl`'s `OPK_*`): the `__has_*` tests by their names,
/// and C++'s alternative tokens (`OPK_ALT` + the operator's code, `EO_*`).
pub const OPK_HAS_INCLUDE: u32 = 1;
pub const OPK_HAS_INCLUDE_NEXT: u32 = 2;
pub const OPK_HAS_EMBED: u32 = 3;
pub const OPK_FEATURE: u32 = 4;
/// A test the configuration answers, 1 where it does not (`__is_identifier`).
pub const OPK_IS_IDENTIFIER: u32 = 5;
pub const OPK_ALT: u32 = 64;
/// The evaluator's operators an alternative token can be (`EO_*`); `EO_OTHER` none it reads.
pub const EO_OROR: u32 = 1;
pub const EO_ANDAND: u32 = 2;
pub const EO_OR: u32 = 3;
pub const EO_XOR: u32 = 4;
pub const EO_AND: u32 = 5;
pub const EO_NE: u32 = 7;
pub const EO_TILDE: u32 = 20;
pub const EO_BANG: u32 = 21;
pub const EO_OTHER: u32 = 27;

/// A `__has_*` test's row of the configuration's answers: the test's name's key and its
/// operand's (its tokens' spellings together) made one.
pub fn feature_key(test: &str, operand: &str) -> u64 {
    crate::pp_eval::key_mix(name_key(test.as_bytes()), name_key(operand.as_bytes()))
}

/// A unit as P1 starts it, and what the host knows from the rounds before: P1's input. The device
/// grows its include tree (decision 0467 §5), lays it out (§2) and decides its conditionals (§4).
pub struct Layout {
    /// The roots: each unit's predefines (`pdir` 0), then its main file (1).
    pub nodes: Vec<PpNode>,
    /// The file table (§1): by (unit, the include's key as a request gave it), the file the host
    /// found for it (`NOT_FOUND`: none).
    pub table: Vec<(u32, u64, u32)>,
    /// By file: the search entry the host found it by (`NOT_FOUND`: its includer's directory, an
    /// absolute name, a root).
    pub found_by: Vec<u32>,
    /// The configuration's include depth.
    pub max_depth: u32,
    /// The `#if` operators by their names' keys (`OPK_*`): the dialect's `__has_*` tests and
    /// alternative tokens.
    pub if_ops: Vec<(u64, u32)>,
    /// The configuration's answers to the `__has_*` tests, by `feature_key`.
    pub features: Vec<(u64, u64)>,
    /// `true` is 1 in `#if`.
    pub true_is_one: bool,
    /// The built-in macros' names' keys: rows at slot 0.
    pub builtins: Vec<u64>,
    /// The dialect's options as the paste relexer reads them (`relex_options`).
    pub relex_options: u32,
    /// Each file's `__FILE__`: the string literal of its presumed path (its `"` and `\` escaped),
    /// by the unit's file index; then `__DATE__`'s, `__TIME__`'s and `__TIMESTAMP__`'s.
    pub file_literals: Vec<Vec<u8>>,
    /// Each file's `__FILE_NAME__`: its presumed path's last component's literal.
    pub name_literals: Vec<Vec<u8>>,
    /// A round started from the last round's tree (decision 0613 §3): `nodes` holds it whole, each
    /// instance's entered there (`entered`, by node) and its groups' activity (`groups`, each
    /// instance's in its order). Empty: a round from the roots.
    pub entered: Vec<u32>,
    pub groups: Vec<u32>,
}

/// What a round's P1 settled at, for the next round to start from (`Layout::entered`).
#[derive(Clone, Debug, Default)]
pub struct Tree {
    pub nodes: Vec<PpNode>,
    pub entered: Vec<u32>,
    pub groups: Vec<u32>,
}

impl Layout {
    /// The layout of a batch of units, `parts` each a unit's layout and its files: its files and
    /// roots after the earlier units'. The units are of one dialect (their built-ins and options
    /// the first's) and one configuration.
    pub fn join(parts: &[(&Layout, usize)]) -> Option<Layout> {
        let first = parts.first()?.0;
        let mut lay = Layout {
            nodes: Vec::new(),
            table: Vec::new(),
            found_by: Vec::new(),
            max_depth: first.max_depth,
            if_ops: first.if_ops.clone(),
            features: first.features.clone(),
            true_is_one: first.true_is_one,
            builtins: first.builtins.clone(),
            relex_options: first.relex_options,
            file_literals: Vec::new(),
            name_literals: Vec::new(),
            entered: Vec::new(),
            groups: Vec::new(),
        };
        let mut files = 0u32;
        for (u, &(l, n_files)) in parts.iter().enumerate() {
            assert!(
                l.builtins == first.builtins && l.relex_options == first.relex_options && l.max_depth == first.max_depth && l.if_ops == first.if_ops && l.features == first.features && l.true_is_one == first.true_is_one,
                "a batch of one dialect and configuration"
            );
            let (u, i0) = (u as u32, lay.nodes.len() as u32);
            assert!(l.groups.is_empty(), "a batch joined of rounds from the roots");
            lay.nodes.extend(l.nodes.iter().map(|n| PpNode { file: n.file + files, parent: if n.parent == NO_PARENT { NO_PARENT } else { n.parent + i0 }, pdir: n.pdir, depth: n.depth, unit: u }));
            lay.table.extend(l.table.iter().map(|&(_, k, f)| (u, k, if f == NOT_FOUND { f } else { f + files })));
            // copy: not block data (the files' search entries, configuration input, joined on the host)
            lay.found_by.extend_from_slice(&l.found_by[..n_files]);
            lay.file_literals.extend(l.file_literals[..n_files].iter().cloned());
            lay.name_literals.extend(l.name_literals[..n_files].iter().cloned());
            files += n_files as u32;
        }
        lay.file_literals.extend(first.file_literals[first.file_literals.len() - 3..].iter().cloned());
        Some(lay)
    }

    /// The batch's units.
    pub fn units(&self) -> u32 {
        self.nodes.iter().map(|n| n.unit + 1).max().unwrap_or(1)
    }
}

/// The unit-free key's row for unit `u` (`pp_p1.uglykl`'s `p1_inc_find`: `key_mix(key, u + 1)`).
fn unit_key(key: u64, u: u32) -> u64 {
    let h = (key ^ (u64::from(u) + 1)).wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^ (h >> 32)
}

/// P1's rooms, by what each counts: the directive words, the stretches, the groups, an `#if`
/// top's tokens, the text's, the tree's nodes, the requests' name bytes, the requests.
pub const NEED_DIRS: usize = 0;
pub const NEED_GAPS: usize = 1;
pub const NEED_GROUPS: usize = 2;
pub const NEED_IF_TOP: usize = 3;
pub const NEED_TEXT_TOP: usize = 4;
pub const NEED_NODES: usize = 5;
pub const NEED_NAMES: usize = 6;
pub const NEED_REQS: usize = 7;
pub const NEEDS: usize = 8;

/// P1's rungs.
pub const P1_RUNGS: u32 = 8;

/// The text's rows, `once` marks and `__COUNTER__` uses room at the expansion's rung `rung`
/// (decision 0485).
pub fn text_rows(rung: u32) -> usize {
    1024 << rung
}

/// P1's stores' room: the instances, the directive words, the stretch and group words, and the
/// active text's tokens. The first run's fits a unit whose files are each entered twice; a run
/// refused for room says what each room needed (`P1_NEED`), and the next is sized from that
/// (`grown`): only the rooms that ran out grow, to twice their need.
#[derive(Clone, Copy, Debug)]
pub struct Caps {
    pub nodes: usize,
    pub dirs: usize,
    pub gaps: usize,
    pub top: usize,
    /// The requests a round makes, and their names' bytes: few (a round's includes not yet
    /// in the file table), so their own rooms, not the directive words'.
    pub reqs: usize,
    pub names: usize,
    /// The runs refused for room so far (P1's rungs, bounded by `P1_RUNGS`).
    pub rung: u32,
}

impl Caps {
    pub fn first(unit: &Unit, lay: &Layout) -> Caps {
        Caps::of(unit, lay, 0)
    }

    /// Every room at `2 << rung` entries a file.
    pub fn of(unit: &Unit, lay: &Layout, rung: u32) -> Caps {
        let k = 2usize << rung.min(P1_RUNGS - 1);
        let nodes = lay.nodes.len() + unit.len() * k + 2;
        let dirs = unit.dir_bound * k + 16;
        Caps { nodes, dirs, gaps: dirs + nodes, top: unit.node_bound * k + 16, reqs: dirs / 8 + 64, names: dirs * 8 + 4096, rung }
    }

    /// The rooms of a run after one whose P1 used `need` (its high water), on a unit whose first
    /// rooms were `was` and are now `now`: twice what it used, and for the files added since
    /// their first rooms' share at twice the use the last run measured over its first rooms.
    /// The first rooms are a bound by the tapes' nodes (a directive is two at least, a top token
    /// one), far past the use where most nodes are directives (Boost's headers: 126 top tokens
    /// used of 169k). A room past them grows from its count (`grown`).
    pub fn from_use(need: [u32; NEEDS], was: Caps, now: Caps, rung: u32) -> Caps {
        let n = |k: usize| need[k] as usize;
        // The added files' first rooms (`a - b`) at the measured use (`used` of `of`), twice.
        let more = |a: usize, b: usize| a.saturating_sub(b);
        let share = |a: usize, b: usize, used: usize| (more(a, b) as u128 * (2 * used).min(b.max(1)) as u128 / b.max(1) as u128) as usize;
        let nodes = 2 * n(NEED_NODES) + share(now.nodes, was.nodes, n(NEED_NODES)) + 16;
        let mut dirs = 2 * n(NEED_DIRS) + share(now.dirs, was.dirs, n(NEED_DIRS)) + 16;
        if 2 * n(NEED_GROUPS) > dirs + nodes {
            dirs = 2 * n(NEED_GROUPS);
        }
        let gaps = (2 * n(NEED_GAPS) + share(now.gaps, was.gaps, n(NEED_GAPS)) + 16).max(dirs + nodes);
        // The top tokens: a run that made requests stopped at them, before the text (the corpus:
        // 9 units of 80 refused at 4,096, needing up to 44k), so its use says nothing of it, and
        // the text's room is the first.
        let t = n(NEED_IF_TOP).max(n(NEED_TEXT_TOP));
        let top = if n(NEED_REQS) > 0 { now.top.max(2 * t + 4096) } else { 2 * t + share(now.top, was.top, t) + 4096 };
        let reqs = 2 * n(NEED_REQS) + more(now.reqs, was.reqs) + 64;
        let names = 2 * n(NEED_NAMES) + more(now.names, was.names) + 4096;
        Caps { nodes, dirs, gaps, top, reqs, names, rung }
    }

    /// The next run's rooms after one refused for room that needed `need`: each room past its
    /// cap at twice its need, the others as they were. A room that names no count (the requests'
    /// scratch) grows them all a rung.
    pub fn grown(self, unit: &Unit, lay: &Layout, need: [u32; NEEDS]) -> Caps {
        let n = |k: usize| need[k] as usize;
        if need[NEED_REQS] == u32::MAX {
            return Caps::of(unit, lay, self.rung + 1);
        }
        let mut c = self;
        c.rung += 1;
        let past = |want: usize, cap: usize| if want > cap { want * 2 } else { cap };
        c.nodes = past(n(NEED_NODES), c.nodes);
        c.dirs = past(n(NEED_DIRS), c.dirs);
        c.reqs = past(n(NEED_REQS), c.reqs);
        c.names = past(n(NEED_NAMES), c.names);
        c.top = past(n(NEED_IF_TOP).max(n(NEED_TEXT_TOP)), c.top);
        // The stretch words are the directives' and the instances' at least.
        c.gaps = past(n(NEED_GAPS), c.gaps).max(c.dirs + c.nodes);
        // The groups: `dirs + nodes`.
        if n(NEED_GROUPS) > c.dirs + c.nodes {
            c.dirs = n(NEED_GROUPS) * 2;
            c.gaps = c.gaps.max(c.dirs + c.nodes);
        }
        if c.nodes == self.nodes && c.dirs == self.dirs && c.gaps == self.gaps && c.top == self.top && c.reqs == self.reqs && c.names == self.names {
            return Caps::of(unit, lay, self.rung + 1);
        }
        c
    }
}

/// The expansion's rooms: its tokens, output, and the scratch's bytes and tokens (past the
/// literals and the `#line` rows' room). The first run's are from the top level's tokens; a run
/// refused for room says what each used (`Exp::room`), and the next is sized from that (`grown`):
/// only the rooms that ran out grow, by twice what they lacked at least.
#[derive(Clone, Copy, Debug)]
pub struct XCaps {
    pub tok: usize,
    pub out: usize,
    pub sb: usize,
    pub st: usize,
    /// The runs refused for room so far: the counters' and the text facts' rooms, and the
    /// `#line` rows', double with it.
    pub rung: u32,
}

impl XCaps {
    /// Every room at rung `rung` over a top level of `top` tokens.
    ///
    /// The shares are from the corpus's most use over its top level's tokens (decision 0486
    /// §8): tokens 0.39, the output 0.64, scratch bytes 0.34; a room past them grows from its own
    /// count.
    pub fn at(top: usize, rung: u32) -> XCaps {
        let tok = (top + 4096) << (rung + 1);
        XCaps { tok, out: top + tok / 4 + 4096, sb: ((64usize << 10) << rung).max(tok / 2), st: tok / 4 + 256, rung }
    }

    fn max(self, a: XCaps) -> XCaps {
        XCaps { tok: self.tok.max(a.tok), out: self.out.max(a.out), sb: self.sb.max(a.sb), st: self.st.max(a.st), rung: self.rung.max(a.rung) }
    }

    /// At least the first run's rooms of a top level of `top` tokens (P1's grew).
    pub fn fit(self, top: usize) -> XCaps {
        let a = XCaps::at(top, 0);
        XCaps { rung: self.rung, ..self.max(a) }
    }

    /// The next run's rooms after one refused for room that used `room` (`Exp::room`): each
    /// room that ran out grows by twice its lack, at least fourfold (its count stopped where it
    /// ran out), and the others double where their use is past their room's half. A room `room` does not name (the counters) grows them all a rung.
    pub fn grown(self, top: usize, room: [(u32, u32); ROOMS]) -> XCaps {
        // A run refused for room stops where the first room ran out: the others' counts are
        // lower bounds, so a room more than half used grows with it, to twice its use.
        let out = room.iter().any(|&(used, cap)| used > cap);
        let up = |(used, cap): (u32, u32), now: usize| {
            if used > cap {
                // Its count stopped where it ran out: what it needs is past it, unknown.
                (now + 2 * (used - cap) as usize).max(4 * now)
            } else if out && used as usize * 2 > cap as usize {
                2 * now
            } else {
                now
            }
        };
        let mut c = self;
        c.rung += 1;
        let [tok, sb, st, out] = room;
        c.tok = up(tok, c.tok);
        c.sb = up(sb, c.sb);
        c.st = up(st, c.st);
        c.out = up(out, c.out);
        if c.tok == self.tok && c.sb == self.sb && c.st == self.st && c.out == self.out {
            let a = XCaps::at(top, self.rung + 1);
            return XCaps { rung: a.rung, ..self.fit(top) }.max(a);
        }
        c
    }
}

/// The rooms a unit's runs settled at: P1's, the expansion's and the definitions' rung. They
/// carry to the unit's next run: a round's unit holds the last round's, so its rooms are a floor
/// (decision 0486 §6), and the expansion's run starts from the last round's.
#[derive(Clone, Copy, Debug)]
pub struct Sizes {
    pub p1: Caps,
    pub x: XCaps,
    pub defs: u32,
    /// What the last run's P1 used (`NEED_*`, its rooms' high water), and the first rooms of the
    /// unit it ran on: the next round's are sized from them (decision 0486 §8).
    pub used: Option<([u32; NEEDS], Caps)>,
}

impl Sizes {
    pub fn first(unit: &Unit, lay: &Layout) -> Sizes {
        let p1 = Caps::first(unit, lay);
        Sizes { p1, x: XCaps::at(p1.top, 0), defs: 0, used: None }
    }

    /// These rooms for `unit` (laid out as `lay`): from what the last run used where it says,
    /// else at least its first.
    pub fn fit(self, unit: &Unit, lay: &Layout) -> Sizes {
        let (a, b) = (self.p1, Caps::first(unit, lay));
        let p1 = match self.used {
            Some((need, was)) => Caps::from_use(need, was, b, a.rung),
            None => Caps { nodes: a.nodes.max(b.nodes), dirs: a.dirs.max(b.dirs), gaps: a.gaps.max(b.gaps), top: a.top.max(b.top), reqs: a.reqs.max(b.reqs), names: a.names.max(b.names), rung: a.rung },
        };
        Sizes { p1, x: self.x.fit(p1.top), defs: self.defs, used: None }
    }
}

/// What a lookup answers where it is no definition's row (`pp_expand.uglykl`'s `L_*`).
pub const L_NOT_NAME: u32 = 0xffff_ffff;
pub const L_NONE: u32 = 0xffff_fffe;
pub const L_BUILTIN: u32 = 0xffff_fffd;
/// The built-ins the device expands to one token, and those that are plain names in text
/// (`__has_include`, `__has_include_next`, `__has_embed`); a definition's row is below `L_IDENT`.
pub const L_FILE: u32 = 0xffff_fffc;
pub const L_LINE: u32 = 0xffff_fffb;
pub const L_DATE: u32 = 0xffff_fffa;
pub const L_TIME: u32 = 0xffff_fff9;
pub const L_TIMESTAMP: u32 = 0xffff_fff8;
pub const L_FILE_NAME: u32 = 0xffff_fff7;
pub const L_BASE_FILE: u32 = 0xffff_fff6;
pub const L_INCLUDE_LEVEL: u32 = 0xffff_fff5;
pub const L_COUNTER: u32 = 0xffff_fff4;
/// A test the configuration answers, function-like in text.
pub const L_FEATURE: u32 = 0xffff_fff3;
pub const L_PRAGMA: u32 = 0xffff_fff2;
pub const L_IDENT: u32 = 0xffff_fff1;

/// A built-in's lookup value, by its name.
fn builtin_value(key: u64) -> u32 {
    let named: [(&[u8], u32); 13] = [
        (b"__LINE__", L_LINE),
        (b"__FILE__", L_FILE),
        (b"__DATE__", L_DATE),
        (b"__TIME__", L_TIME),
        (b"__TIMESTAMP__", L_TIMESTAMP),
        (b"__FILE_NAME__", L_FILE_NAME),
        (b"__BASE_FILE__", L_BASE_FILE),
        (b"__INCLUDE_LEVEL__", L_INCLUDE_LEVEL),
        (b"__COUNTER__", L_COUNTER),
        (b"_Pragma", L_PRAGMA),
        (b"__has_include", L_IDENT),
        (b"__has_include_next", L_IDENT),
        (b"__has_embed", L_IDENT),
    ];
    named.iter().find(|(n, _)| name_key(n) == key).map_or(L_BUILTIN, |&(_, v)| v)
}

/// S6–S8 and the lookups (R4): the text map, the visibility rows and their sort, the active
/// stretches, and each stretch token's meaning where it stands.
pub struct Vis {
    // The include tree, grown and laid out on the device (decision 0467 §2, §5): the roots and
    // the instances made, the tree's depth, the file table, the files' search entries, the depth's
    // bound, the given `#if` values; each instance's liveness; by directive word, the instance
    // made for it and its file, the one it enters, and its found file; each file's first
    // `#pragma once` or `#import`; the new instances' flags and places; the requests; the
    // refusals; each unit's first stretch in the unit's order.
    nodes: Buf,
    depth: Buf,
    n_ft: u32,
    ft_keys: Buf,
    ft_files: Buf,
    found_by: Buf,
    unit_file: Buf,
    /// The same, on the host: a request's unit and its file there.
    unit_files: Vec<u32>,
    max_depth: u32,
    live: Buf,
    // The tests' lines (decision 0467 §4): by directive word, its tokens, its first node, its
    // first top token, its value; by top token, its node and its mark; the keys of `defined`
    // and `true`, and whether `true` is 1; the operators' table and the configuration's answers.
    if_count: Buf,
    if_first: Buf,
    if_kind: Buf,
    if_inst: Buf,
    // `#line`'s rows (decision 0467 §6): each word's offset, key, scans' values and heads, the
    // latest rows before each word and each stretch, and the rows' scratch room.
    ldelta: Buf,
    lkey: Buf,
    lval: Buf,
    lvaln: Buf,
    lhead: Buf,
    latest: Buf,
    latestn: Buf,
    iline: Buf,
    ilinen: Buf,
    sline: Buf,
    // The stretches' runs (a top call's scope in the text): starts, their scan, each run's end.
    rstart: Buf,
    runx: Buf,
    run_scan: Scan,
    run_end: Buf,
    slinen: Buf,
    lctl: Buf,
    // The requests' names: the device's bytes, their top and room.
    rnames: Buf,
    rntop: Buf,
    rn_cap: u32,
    rq_cap: u32,
    if_at: Buf,
    if_scan: Scan,
    if_nodes: Buf,
    if_ops: Buf,
    if_val: Buf,
    if_keys: Buf,
    true_is_one: u32,
    n_ifop: u32,
    op_keys: Buf,
    op_kinds: Buf,
    n_feat: u32,
    feat_keys: Buf,
    feat_vals: Buf,
    made: Buf,
    made_file: Buf,
    enter: Buf,
    cand: Buf,
    once: Buf,
    newf: Buf,
    newat: Buf,
    nnew: Buf,
    reqs: Buf,
    nreq: Buf,
    flags: Buf,
    // What each of P1's rooms needed where it ran out (`NEED_*`), for the next run's sizes.
    need: Buf,
    pub(crate) ufirst: Buf,
    // The active text's tokens' room.
    tc: u32,
    insts: Buf,
    n_insts: Buf,
    dslot: Buf,
    dactive: Buf,
    n_dirs: Buf,
    gslot: Buf,
    floors: Buf,
    n_units: u32,
    p1_ndirs: Buf,
    p1_ngaps: Buf,
    p1_dfirst: Buf,
    p1_gfirst: Buf,
    p1_size: Buf,
    p1_csize: Buf,
    p1_gc: Buf,
    p1_gheads: Buf,
    p1_gshift: Buf,
    p1_usize: Buf,
    p1_utotal: Buf,
    p1_base: Buf,
    // P1's conditionals (decision 0467 §4): each instance's group count and first group word,
    // the groups (the state an iteration reads), each chain's taken arm, the jumping's two pairs
    // of buffers, the rounds' and iterations' flags, and the given `#if` values.
    nq: usize,
    p1_gcount: Buf,
    p1_grpf: Buf,
    p1_ngrp: Buf,
    p1_ga: Buf,
    p1_seede: Buf,
    p1_seedg: Buf,
    n_seed: u32,
    p1_taken: Buf,
    p1_own: Buf,
    p1_par: Buf,
    p1_own2: Buf,
    p1_par2: Buf,
    p1_jchanged: Buf,
    p1_ichanged: Buf,
    gactive: Buf,
    pub(crate) n_gaps: Buf,
    n_builtin: u32,
    ni: usize,
    nd: usize,
    ng: usize,
    // The rows: a directive of the layout each, after the built-ins'.
    rflag: Buf,
    rat: Buf,
    rscan: Scan,
    pub(crate) sort_keys: Buf,
    pub(crate) name_keys: Buf,
    pub(crate) values: Buf,
    // Push and pop rows (decision 0467 §3): each row's type, the active pushes and pops, each
    // sorted row's matching push and the row its value comes from, and the jumping's rounds.
    rtypes: Buf,
    n_stack: Buf,
    stk_match: Buf,
    stk_src: Buf,
    stk_changed: Buf,
    pub(crate) n_rows: Buf,
    pub(crate) sort: Sort,
    // The stretches: a word each, and a lookup a text token.
    gcounts: Buf,
    pub(crate) gfirsts: Buf,
    /// The stretches in the unit's order: their keys (slots), sorted with their places, and
    /// each one's slot in that order.
    gkeys: Buf,
    gsort: Sort,
    pub(crate) sslot: Buf,
    sinst: Buf,
    pub(crate) gat: Buf,
    pub(crate) gscan: Scan,
    pub lookups: Buf,
    /// Each top token's node (`pe_lookups`): the loop's reads of the top level, without a search.
    topnode: Buf,
    /// Each top token's name's rows (`pe_lookups`, `p1_if_tokens`; `IROWS` a body item's), two words a token.
    trows: Buf,
    pub bad: Buf,
}


/// The pointer-jumping rounds a pop's value takes at most (2^32 pops' chains).
const STACK_ROUNDS: usize = 32;
/// The jumping rounds over the groups' nesting (2^20 deep), and P1's iterations, at most.
const P1_JUMPS: usize = 20;
const P1_ITERS: usize = 64;

const NO_NODE: [PpNode; 1] = [PpNode { file: 0, parent: NO_PARENT, pdir: 0, depth: 0, unit: 0 }];

impl Vis {
    pub fn new(arena: &Arena, unit: &Unit, lay: &Layout, caps: Caps, x_rung: u32) -> Result<Vis, Error> {
        let words = |v: &[u32]| arena.from_slice(if v.is_empty() { &[0] } else { v });
        let zeroed = |k: usize| arena.zeroed::<u32>(k.max(1));
        let top = caps.top;
        let (nd, ng, nb, ni) = (caps.dirs, caps.gaps, lay.builtins.len(), caps.nodes);
        // Group words: a file has a group more than its arms, so at most one more than its directives.
        let nq = nd + ni;
        // Past the directives' and the built-ins' rows, the text's (decision 0485 §2).
        let rows = nd + nb + text_rows(x_rung);
        // The roots, then the room for the instances the device makes.
        let mut nodes = lay.nodes.clone();
        nodes.resize(ni, PpNode::default());
        // The file table and the `#if` values, sorted by their keys.
        let mut table: Vec<(u64, u32)> = lay.table.iter().map(|&(u, k, f)| (unit_key(k, u), f)).collect();
        table.sort_unstable();
        let mut feats = lay.features.clone();
        feats.sort_unstable();
        let mut found_by = lay.found_by.clone();
        found_by.resize(unit.len().max(1), NOT_FOUND);
        // Each unit's first file: its predefines root's.
        let unit_files: Vec<u32> = (0..lay.units()).map(|u| lay.nodes.iter().find(|n| n.unit == u && n.pdir == 0).map_or(0, |n| n.file)).collect();
        // The built-ins' rows, at slot 0: the configuration's, written before the passes.
        let mut sk = vec![0u64; rows.max(1)];
        let mut nk = vec![0u64; rows.max(1)];
        let mut vals = vec![0u32; rows.max(1)];
        for (k, &key) in lay.builtins.iter().enumerate() {
            sk[k] = key & 0xffff_ffff_0000_0000;
            nk[k] = key;
            // The configuration's tests, by the dialect's operators.
            let feature = lay.if_ops.iter().any(|&(k, kind)| k == key && (kind == OPK_FEATURE || kind == OPK_IS_IDENTIFIER));
            vals[k] = if feature { L_FEATURE } else { builtin_value(key) };
        }
        Ok(Vis {
            nodes: arena.from_slice(if nodes.is_empty() { &NO_NODE } else { &nodes })?,
            depth: zeroed(1)?,
            insts: arena.zeroed::<PpInst>(ni.max(1))?,
            n_insts: words(&[lay.nodes.len() as u32])?,
            n_ft: table.len() as u32,
            ft_keys: arena.from_slice(&if table.is_empty() { vec![0u64] } else { table.iter().map(|t| t.0).collect() })?,
            ft_files: words(&if table.is_empty() { vec![0] } else { table.iter().map(|t| t.1).collect() })?,
            found_by: words(&found_by)?,
            // Each unit's first file: its predefines root's.
            unit_file: words(&unit_files)?,
            unit_files,
            max_depth: lay.max_depth,
            if_count: zeroed(nd)?,
            if_first: zeroed(nd)?,
            if_kind: zeroed(nd)?,
            if_inst: zeroed(nd)?,
            ldelta: zeroed(nd)?,
            lkey: arena.zeroed::<u64>(nd.max(1))?,
            lval: zeroed(nd)?,
            lvaln: zeroed(nd)?,
            lhead: zeroed(nd)?,
            latest: zeroed(nd)?,
            latestn: zeroed(nd)?,
            iline: zeroed(nd)?,
            ilinen: zeroed(nd)?,
            sline: zeroed(ng)?,
            rstart: zeroed(ng)?,
            runx: zeroed(ng)?,
            run_scan: Scan::new(arena)?,
            run_end: zeroed(ng)?,
            slinen: zeroed(ng)?,
            lctl: zeroed(4)?,
            rnames: arena.zeroed::<u8>(caps.names.max(1))?,
            rntop: zeroed(1)?,
            rn_cap: caps.names as u32,
            rq_cap: caps.reqs as u32,
            if_at: zeroed(nd)?,
            if_scan: Scan::new(arena)?,
            if_nodes: zeroed(top)?,
            if_ops: zeroed(top)?,
            if_val: zeroed(nd)?,
            if_keys: arena.from_slice(&[name_key(b"defined"), name_key(b"true")])?,
            true_is_one: u32::from(lay.true_is_one),
            n_ifop: lay.if_ops.len() as u32,
            op_keys: arena.from_slice(&if lay.if_ops.is_empty() { vec![0u64] } else { lay.if_ops.iter().map(|t| t.0).collect() })?,
            op_kinds: words(&if lay.if_ops.is_empty() { vec![0] } else { lay.if_ops.iter().map(|t| t.1).collect() })?,
            n_feat: feats.len() as u32,
            feat_keys: arena.from_slice(&if feats.is_empty() { vec![0u64] } else { feats.iter().map(|t| t.0).collect() })?,
            feat_vals: arena.from_slice(&if feats.is_empty() { vec![0u64] } else { feats.iter().map(|t| t.1).collect() })?,
            live: zeroed(ni)?,
            made: zeroed(nd)?,
            made_file: zeroed(nd)?,
            enter: zeroed(nd)?,
            cand: zeroed(nd)?,
            once: zeroed(unit.len())?,
            newf: zeroed(nd)?,
            newat: zeroed(nd)?,
            nnew: zeroed(1)?,
            reqs: arena.zeroed::<[u32; 6]>(caps.reqs.max(1))?,
            nreq: zeroed(1)?,
            flags: zeroed(1)?,
            need: zeroed(NEEDS)?,
            ufirst: zeroed(lay.units() as usize)?,
            dslot: zeroed(nd)?,
            dactive: zeroed(nd)?,
            n_dirs: zeroed(1)?,
            gslot: zeroed(ng)?,
            floors: zeroed(lay.units() as usize)?,
            n_units: lay.units(),
            p1_ndirs: zeroed(ni)?,
            p1_ngaps: zeroed(ni)?,
            p1_dfirst: zeroed(ni)?,
            p1_gfirst: zeroed(ni)?,
            p1_size: zeroed(ni)?,
            p1_csize: zeroed(nd)?,
            p1_gc: zeroed(ng)?,
            p1_gheads: zeroed(ng)?,
            p1_gshift: zeroed(ng)?,
            p1_usize: zeroed(lay.units() as usize)?,
            p1_utotal: zeroed(1)?,
            p1_base: zeroed(ni)?,
            gactive: zeroed(ng)?,
            nq,
            p1_gcount: zeroed(ni)?,
            p1_grpf: zeroed(ni)?,
            p1_ngrp: zeroed(1)?,
            p1_ga: zeroed(nq)?,
            p1_seede: words(&{
                let mut e = lay.entered.clone();
                e.resize(ni.max(1), 0);
                e
            })?,
            p1_seedg: words(&if lay.groups.is_empty() { vec![0] } else { lay.groups.clone() })?,
            n_seed: lay.groups.len() as u32,
            p1_taken: zeroed(nq)?,
            p1_own: zeroed(nq)?,
            p1_par: zeroed(nq)?,
            p1_own2: zeroed(nq)?,
            p1_par2: zeroed(nq)?,
            p1_jchanged: zeroed(P1_JUMPS)?,
            p1_ichanged: zeroed(P1_ITERS)?,
            n_gaps: zeroed(1)?,
            n_builtin: nb as u32,
            ni,
            nd,
            ng,
            rflag: zeroed(nd)?,
            rat: zeroed(nd)?,
            rscan: Scan::new(arena)?,
            sort_keys: arena.from_slice(&sk)?,
            name_keys: arena.from_slice(&nk)?,
            values: arena.from_slice(&vals)?,
            n_rows: zeroed(1)?,
            rtypes: zeroed(rows)?,
            n_stack: zeroed(1)?,
            stk_match: zeroed(rows)?,
            stk_src: zeroed(rows)?,
            stk_changed: zeroed(STACK_ROUNDS)?,
            sort: Sort::new(arena, rows)?,
            gcounts: zeroed(ng)?,
            gfirsts: zeroed(ng)?,
            gkeys: arena.zeroed::<u64>(ng.max(1))?,
            gsort: Sort::new(arena, ng)?,
            sslot: zeroed(ng)?,
            sinst: zeroed(ng)?,
            gat: zeroed(ng)?,
            gscan: Scan::new(arena)?,
            lookups: zeroed(top)?,
            topnode: zeroed(top)?,
            trows: zeroed(2 * top)?,
            tc: top as u32,
            bad: zeroed(1)?,
        })
    }

    /// The visibility's slots of the layout (the text map is the front's): the rows (a flag and
    /// a scan, then a sort by name), the stretches
    /// (a sort of their keys, their counts and a scan) and the lookups, as phases of the unit's program.
    pub fn lay(&self, l: &mut Lay) {
        l.at("INSTS", &self.insts).at("N_INSTS", &self.n_insts).at("DSLOT", &self.dslot).at("DACTIVE", &self.dactive).at("N_DIRS", &self.n_dirs);
        l.at("GSLOT", &self.gslot).at("GACTIVE", &self.gactive).at("N_GAPS", &self.n_gaps).at("FLOORS", &self.floors).word("N_UNITS", self.n_units);
        l.word("N_BUILTIN", self.n_builtin).word("ND", self.nd as u32).word("NG", self.ng as u32);
        l.at("P1_NODES", &self.nodes).at("P1_DEPTHN", &self.depth).at("P1_NDIRS", &self.p1_ndirs).at("P1_NGAPS", &self.p1_ngaps);
        l.word("N_FT", self.n_ft).at("P1_FTKEYS", &self.ft_keys).at("P1_FTFILES", &self.ft_files).at("P1_FOUNDBY", &self.found_by).at("P1_UFILE", &self.unit_file).word("P1_MAXDEPTH", self.max_depth);
        l.at("P1_LIVE", &self.live);
        l.at("IF_COUNT", &self.if_count).at("IF_FIRST", &self.if_first).at("IF_KIND", &self.if_kind).at("IF_INST", &self.if_inst)
            .at("LDELTA", &self.ldelta).at("LKEY", &self.lkey).at("LVAL", &self.lval).at("LVALN", &self.lvaln).at("LHEAD", &self.lhead).at("LATEST", &self.latest).at("LATESTN", &self.latestn)
            .at("ILINE", &self.iline).at("ILINEN", &self.ilinen).at("SLINE", &self.sline).at("RSTART", &self.rstart).at("RUNX", &self.runx).at("RUNTOT", &self.run_scan.total).at("RUN_END", &self.run_end).at("SLINEN", &self.slinen).at("LCTL", &self.lctl).at("P1_RNAMES", &self.rnames).at("P1_RNTOP", &self.rntop).word("RNC", self.rn_cap).word("RQC", self.rq_cap).at("IF_AT", &self.if_at).at("IF_TOTAL", &self.if_scan.total).at("IF_NODES", &self.if_nodes);
        l.at("IF_OPS", &self.if_ops).at("IF_VAL", &self.if_val).at("IF_KEYS", &self.if_keys).word("IF_TRUE", self.true_is_one).word("N_IFOP", self.n_ifop);
        l.at("IF_OPKEYS", &self.op_keys).at("IF_OPKINDS", &self.op_kinds).word("N_FEAT", self.n_feat).at("IF_FEATKEYS", &self.feat_keys).at("IF_FEATVALS", &self.feat_vals);
        l.at("P1_MADE", &self.made).at("P1_MADEF", &self.made_file).at("P1_ENTER", &self.enter).at("P1_CAND", &self.cand).at("P1_ONCE", &self.once);
        l.at("P1_NEWF", &self.newf).at("P1_NEWAT", &self.newat).at("P1_NNEW", &self.nnew).at("P1_REQS", &self.reqs).at("P1_NREQ", &self.nreq);
        l.at("P1_FLAGS", &self.flags).at("P1_NEED", &self.need).at("P1_UFIRST", &self.ufirst);
        l.word("NC", self.ni as u32).word("DC", self.nd as u32).word("GC", self.ng as u32).word("TC", self.tc);
        l.at("P1_DFIRST", &self.p1_dfirst).at("P1_GFIRST", &self.p1_gfirst).at("P1_SIZE", &self.p1_size).at("P1_CSIZE", &self.p1_csize);
        l.at("P1_GC", &self.p1_gc).at("P1_GHEADS", &self.p1_gheads).at("P1_GSHIFT", &self.p1_gshift).at("P1_USIZE", &self.p1_usize);
        l.at("P1_UTOTAL", &self.p1_utotal).at("P1_BASE", &self.p1_base);
        l.word("NQ", self.nq as u32).at("P1_GCOUNT", &self.p1_gcount).at("P1_GRPF", &self.p1_grpf).at("P1_NGRP", &self.p1_ngrp).at("P1_GA", &self.p1_ga).at("P1_SEEDE", &self.p1_seede).at("P1_SEEDG", &self.p1_seedg).word("P1_NSEED", self.n_seed);
        l.at("P1_TAKEN", &self.p1_taken).at("P1_OWN", &self.p1_own).at("P1_PAR", &self.p1_par).at("P1_OWN2", &self.p1_own2).at("P1_PAR2", &self.p1_par2);
        l.at("P1_JCHANGED", &self.p1_jchanged).at("P1_ICHANGED", &self.p1_ichanged);
        l.word("P1_JUMPS", P1_JUMPS as u32).word("P1_ITERS", P1_ITERS as u32);
        l.at("RFLAG", &self.rflag).at("RAT", &self.rat).at("RTOTAL", &self.rscan.total);
        l.at("SORT_KEYS", &self.sort_keys).at("NAME_KEYS", &self.name_keys).at("VALUES", &self.values).at("N_ROWS", &self.n_rows);
        l.at("RTYPES", &self.rtypes).at("N_STACK", &self.n_stack).at("STK_MATCH", &self.stk_match).at("STK_SRC", &self.stk_src).at("STK_CHANGED", &self.stk_changed).word("STK_ROUNDS", STACK_ROUNDS as u32);
        l.at("SORT_KB", &self.sort.keys_b).at("SORT_VA", &self.sort.vals_a).at("SORT_VB", &self.sort.vals_b).at("SORT_HIST", &self.sort.hist);
        l.at("GCOUNTS", &self.gcounts).at("GFIRSTS", &self.gfirsts).at("GKEYS", &self.gkeys);
        l.at("GSORT_KB", &self.gsort.keys_b).at("GSORT_VA", &self.gsort.vals_a).at("GSORT_VB", &self.gsort.vals_b).at("GSORT_HIST", &self.gsort.hist);
        l.at("SSLOT", &self.sslot).at("SINST", &self.sinst).at("GAT", &self.gat).at("GTOTAL", &self.gscan.total).at("LOOKUPS", &self.lookups).at("TOPNODE", &self.topnode).at("TROWS", &self.trows).at("VBAD", &self.bad);
    }

    /// The layout P1 made on the device (decision 0467 §2), read back for tests: each directive's
    /// slot and each stretch's, in the layout's words.
    pub fn p1(&self) -> P1Out {
        // SAFETY: the device finished; each store holds at least its count's words.
        let words = |b: &Buf, n: usize| unsafe { std::slice::from_raw_parts(b.host().cast::<u32>(), n).to_vec() };
        let (ni, nd, ng) = (self.word(&self.n_insts) as usize, self.word(&self.n_dirs) as usize, self.word(&self.n_gaps) as usize);
        // SAFETY: as above; `nodes` holds `ni` instances.
        let nodes = unsafe { std::slice::from_raw_parts(self.nodes.host().cast::<PpNode>(), ni).to_vec() };
        P1Out {
            nodes,
            live: words(&self.live, ni),
            dir_first: words(&self.p1_dfirst, ni),
            dir_slots: words(&self.dslot, nd),
            gap_slots: words(&self.gslot, ng),
            dir_active: words(&self.dactive, nd),
            gap_active: words(&self.gactive, ng),
            flags: 0,
            requests: Vec::new(),
            refused: 0,
        }
    }

    /// The tree P1 settled at (`Tree`): its instances, each one entered where it is live, the groups'
    /// activity.
    pub fn tree(&self) -> Tree {
        let (ni, nq) = (self.word(&self.n_insts) as usize, self.word(&self.p1_ngrp) as usize);
        // SAFETY: the device finished; the stores hold `ni` instances and `nq` group words.
        let words = |b: &Buf, n: usize| unsafe { std::slice::from_raw_parts(b.host().cast::<u32>(), n).to_vec() };
        // SAFETY: as above.
        let nodes = unsafe { std::slice::from_raw_parts(self.nodes.host().cast::<PpNode>(), ni).to_vec() };
        // copy: not block data (P1's settled state, the next round's input: decision 0613 §3)
        Tree { nodes, entered: words(&self.live, ni), groups: words(&self.p1_ga, nq) }
    }

    fn word(&self, b: &Buf) -> u32 {
        // SAFETY: the device finished; the store holds a word at least.
        unsafe { *b.host().cast::<u32>() }
    }

    /// `RERAIL_PP_ARENA_LOG`: what ran out of room at P1's rung `rung`, against its caps.
    fn log_room(&self, unit: &Unit, c: Caps) {
        if std::env::var_os("RERAIL_PP_ARENA_LOG").is_some() {
            eprintln!("p1 room at rung {}: need {:?}; caps nodes {} dirs {} gaps {} top {}; unit nodes {} dirs {} node_bound {}", c.rung, self.p1_need(), c.nodes, c.dirs, c.gaps, c.top, unit.len(), unit.dir_bound, unit.node_bound);
        }
    }

    /// What P1's rooms needed (`NEED_*`), after the command buffer is done.
    pub fn p1_need(&self) -> [u32; NEEDS] {
        // SAFETY: the device finished; `need` holds `NEEDS` words.
        unsafe { std::ptr::read(self.need.host().cast::<[u32; NEEDS]>()) }
    }

    /// P1's refusals (`P1_*`), after the command buffer is done.
    pub fn p1_flags(&self) -> u32 {
        self.word(&self.flags)
    }

    /// The round's requests (decision 0467 §1), each once.
    pub fn requests(&self) -> Vec<Request> {
        let n = (self.word(&self.nreq) as usize).min(self.rq_cap as usize);
        // SAFETY: the device finished; `reqs` holds `rq_cap` requests, `n` of them written.
        let raw = unsafe { std::slice::from_raw_parts(self.reqs.host().cast::<[u32; 6]>(), n) };
        let mut out: Vec<Request> = raw
            .iter()
            .map(|q| {
                // copy: not block data (a request's name, the host's search key: decision 0467 §1)
                // SAFETY: the device finished; a request's name lies within the names' room.
                let (at, n) = ((q[3] as usize).min(self.rn_cap as usize), q[4] as usize);
                let name = unsafe { std::slice::from_raw_parts(self.rnames.host().add(at), n.min(self.rn_cap as usize - at)).to_vec() };
                let unit = self.unit_files.partition_point(|&f| f <= q[2]).saturating_sub(1);
                Request { key: u64::from(q[0]) | (u64::from(q[1]) << 32), unit: unit as u32, file: q[2] - self.unit_files[unit], name, angle: q[5] & 1 != 0, next: q[5] & 2 != 0 }
            })
            .collect();
        // A key is unit-free: one request a key and unit.
        out.sort_unstable_by_key(|q| (q.unit, q.key, q.file));
        out.dedup_by_key(|q| (q.unit, q.key));
        out
    }

    /// The lookups, read back for tests (after the command buffer is done).
    pub fn read(&self) -> Result<Vec<u32>, Error> {
        // SAFETY: the device finished; `lookups` holds the scan's total of words.
        unsafe {
            if *self.bad.host().cast::<u32>() != 0 {
                return Err(crate::pp_arena::capacity(concat!(file!(), ":", line!())));
            }
            let n = *self.gscan.total.host().cast::<u32>() as usize;
            Ok(std::slice::from_raw_parts(self.lookups.host().cast::<u32>(), n).to_vec())
        }
    }
}

/// Why a unit is refused (`pp_expand.uglykl`'s `X_*`). `X_ROOM` reruns it at the next rung.
pub const X_BUILTIN: u32 = 1;
pub const X_PAST_END: u32 = 4;
pub const X_PASTE: u32 = 8;
pub const X_DEPTH: u32 = 16;
pub const X_ROOM: u32 = 32;
pub const X_QUEUE: u32 = 64;

/// A piece's source kind (decision 0520 §1: `P_KIND`'s bits of its first column), its length.
const P_TOP: u32 = 0;
const P_BODY: u32 = 1 << 30;
const P_KIND: u32 = 3 << 30;
/// A piece of a source in the tokens' store (`pp_loop.uglykl`'s `P_SPAN`), the top text's (`LP_TOPSRC`).
const P_SPAN: u32 = 3 << 30;
const LP_TOPSRC: u32 = 0x3fff_ffff;
const PN_LEN: u32 = 0xff_ffff;
pub const X_HASH: u32 = 128;
pub const X_ACROSS: u32 = 256;
pub const X_VA_OPT: u32 = 512;
pub const X_RELEX: u32 = 1024;
pub const X_SPLICE: u32 = 2048;
/// `#` or `##` of `__COUNTER__`'s value (decision 0485 §3).
pub const X_COUNTER_SPELLED: u32 = 8192;
/// A `_Pragma` that would make a row (`once`, `push_macro`, `pop_macro`), or whose operand is a
/// macro's.
pub const X_PRAGMA: u32 = 16384;
/// A piece's steps past the loop's bound (`LP_WORK`): not room, so no rung is larger for it.
pub const X_WORK: u32 = 32768;

/// The expansion's tries for room. A refused run stops where its first room ran out, so each try
/// uncovers about one room's need (seven rooms); the budget's lease bounds the sizes.
pub const EXP_RUNGS: u32 = 10;
/// A string literal's tape kind.
const K_LITERAL: u32 = 30;

/// The expansion's stores (0448 §2), placed before the command buffer, at a rung:
/// - by top token (`top`): its stretch head, depth change and depth (the loop's work arrays: a
///   piece's end, its join, whether it was joined), its state, its marks and the pieces' starts,
///   the overruns' places, its pieces' counts and places in the output;
/// - the arena: the tokens' columns (decision 0520), the scratch tokens;
/// - the output (pieces), and the control words (the arena's tops, the refusals).
pub struct Exp {
    out_cap: u32,
    words: Buf,
    /// The scratch tokens (`#`'s and `##`'s): their bytes, and each one's (start, end, kind).
    /// `st_cap` (start, end, kind) words, then `sb_cap` bytes and slack.
    scratch: Buf,
    st_cap: usize,
    theads: Buf,
    delta: Buf,
    depth: Buf,
    state: Buf,
    oflag: Buf,
    v: Buf,
    obase: Buf,
    /// Each top token's place in its piece's output (`lp_lex`; `lp_obase` makes it `obase`'s).
    lob: Buf,
    /// Each piece's last output entry (`lp_flush`), for its joins' lengths (`lp_extend`).
    plast: Buf,
    /// The pieces' starts (`lp_starts`), and the overruns' scan and places (`lp_overruns`).
    xflag: Buf,
    xat: Buf,
    xscan: Scan,
    xpos: Buf,
    vscan: Scan,
    /// The output's pieces (decision 0520 §3): each top token's count, their scan.
    opv: Buf,
    opbase: Buf,
    opscan: Scan,
    /// The tokens' columns, `tok_cap` words each: the source, the length and flags, the start and
    /// the origin (decision 0520).
    tokens: Buf,
    /// The loop's queue and its pieces' words (`LPQ`, decision 0613 §4), for tops of `lpq_top` tokens.
    lpq: Buf,
    lpq_top: u32,
    /// The expansions' rows (`XROWS`, decision 0613 §6: `lp_roll_rows`), 3 words each, `xrows_cap` of them.
    xrows: Buf,
    xrows_cap: u32,
    pub ctl: Buf,
    // `__COUNTER__`'s uses and their room.
    cnt: Buf,
    // The text pass's facts (decision 0485): its rows (sort key, name key, type), its `once`
    // marks (file, slot), the `__COUNTER__` uses' places (the text's, then the lines'), and their
    // counts and keys.
    tr_sk: Buf,
    tr_nk: Buf,
    tr_ty: Buf,
    to_f: Buf,
    to_s: Buf,
    cu_slot: Buf,
    cu_sub: Buf,
    cu_unit: Buf,
    trn: Buf,
    lread: Buf,
    cnt_cap: u32,
    lb_cap: u32,
    pub out: Buf,
    /// The `__COUNTER__` uses' ranking (`px_cv_*`): a sort of the uses of both passes, and its
    /// segmented scans' rows (`SC_RANKING`), `sc_cap` (twice `cnt_cap`) each.
    sc_tok: [Buf; 6],
    sc_ckeys: Buf,
    sc_sort: Sort,
    sc_cap: u32,
}

/// The `__COUNTER__` ranking's rows (`Exp::sc_tok`), by slot.
const SC_RANKING: [&str; 6] = ["SC_DEPTH", "SC_CLOSE", "SC_CEND", "SC_REACH", "SC_LIVE", "SC_RANK"];

impl Exp {
    pub fn new(arena: &Arena, unit: &Unit, lay: &Layout, x: XCaps, caps: Caps) -> Result<Exp, Error> {
        let zeroed = |k: usize| arena.zeroed::<u32>(k.max(1));
        let top = caps.top;
        let (rung, tok, out) = (x.rung, x.tok, x.out);
        if tok >= u32::MAX as usize / 4 {
            return Err(crate::pp_arena::capacity(concat!(file!(), ":", line!())));
        }
        // The scratch: bytes and tokens, in one buffer: the tokens (an
        // even number, so the bytes after them are 8-byte aligned), then the bytes. The first of
        // each are the files' `__FILE__` literals, the configuration's three, then the files'
        // `__FILE_NAME__` literals, placed here.
        let nf = 2 * unit.len() + 3;
        let lit = |f: usize| -> &[u8] {
            let l = if f < unit.len() + 3 { lay.file_literals.get(f) } else { lay.name_literals.get(f - unit.len() - 3) };
            l.map_or(b"\"\"", Vec::as_slice)
        };
        let lit_bytes: usize = (0..nf).map(|f| lit(f).len()).sum();
        // Past them, the `#line` rows' room: two tokens a directive word, and their bytes.
        let (lb_cap, dirs) = ((16usize << 10) << rung, caps.dirs);
        let (sb, st) = (x.sb + lit_bytes + lb_cap, (x.st + nf + 2 * dirs).next_multiple_of(2));
        let mut scratch = vec![0u8; st * 12 + sb + 16];
        let mut at = 0usize;
        for f in 0..nf {
            let lit = lit(f);
            scratch[st * 12 + at..st * 12 + at + lit.len()].copy_from_slice(lit);
            let tok3 = [at as u32, (at + lit.len()) as u32, K_LITERAL];
            for (k, w) in tok3.iter().enumerate() {
                scratch[f * 12 + 4 * k..f * 12 + 4 * k + 4].copy_from_slice(&w.to_le_bytes());
            }
            at += lit.len();
        }
        // The control words: the tokens' front (0), the refusals (2), the scratch's bytes' and
        // tokens' fronts (4, 5) and the literals' (6, 7), each top's high water over the passes
        // (8 the tokens', 11 and 12 the scratch's, 14 the output's), the givings back at the
        // substitutions (16), and the counters' ranking's need (24).
        let cnt_cap = text_rows(rung);
        let mut ctl = [0u32; 32];
        ctl[4] = lit_bytes as u32;
        ctl[5] = nf as u32;
        let words = [unit.len() as u32, tok as u32, 0, 0, out as u32, 0, 0, 0, sb as u32, st as u32, lay.relex_options, 0, 0, 0];
        let sc = 2 * cnt_cap;
        Ok(Exp {
            out_cap: out as u32,
            words: arena.from_slice(&words)?,
            // Slack past the last byte: a name's key reads 8 bytes at a time.
            scratch: arena.from_slice(&scratch)?,
            st_cap: st,
            theads: zeroed(top)?,
            delta: zeroed(top)?,
            depth: zeroed(top)?,
            state: zeroed(top)?,
            oflag: zeroed(top)?,
            v: zeroed(top)?,
            obase: zeroed(top)?,
            lob: zeroed(top)?,
            plast: zeroed(top)?,
            xflag: zeroed(top)?,
            xat: zeroed(top)?,
            xscan: Scan::new(arena)?,
            xpos: zeroed(top)?,
            vscan: Scan::new(arena)?,
            opv: zeroed(top)?,
            opbase: zeroed(top)?,
            opscan: Scan::new(arena)?,
            tokens: zeroed(tok * 5)?,
            // The queue's slots (`lpq_cap`: the pieces', the requests' and their ends', a request's record
            // `RQ_ENTRIES` of the tokens' rows), then the pieces' three words.
            lpq: zeroed(16 + 3 * top + 64 + tok / 3 + 5 * top)?,
            xrows: zeroed(3 * tok)?,
            xrows_cap: tok as u32,
            lpq_top: top as u32,
            ctl: arena.from_slice(&ctl)?,
            cnt: zeroed(2 + 2 * cnt_cap)?,
            tr_sk: arena.zeroed::<u64>(cnt_cap)?,
            tr_nk: arena.zeroed::<u64>(cnt_cap)?,
            tr_ty: zeroed(cnt_cap)?,
            to_f: zeroed(cnt_cap)?,
            to_s: zeroed(cnt_cap)?,
            cu_slot: zeroed(2 * cnt_cap)?,
            cu_sub: zeroed(2 * cnt_cap)?,
            cu_unit: zeroed(2 * cnt_cap)?,
            trn: zeroed(32)?,
            lread: zeroed(dirs)?,
            cnt_cap: cnt_cap as u32,
            lb_cap: lb_cap as u32,
            // The output's pieces: five columns, `out` words each (decision 0520 §3), their order and
            // the loop's chunks (`lp_flush`).
            out: zeroed(9 * out)?,
            sc_tok: [(); 6].map(|()| zeroed(sc)).into_iter().collect::<Result<Vec<_>, _>>()?.try_into().map_err(|_| crate::pp_arena::failure(concat!(file!(), ":", line!())))?,
            sc_ckeys: arena.zeroed::<u64>(sc)?,
            sc_sort: Sort::new(arena, sc)?,
            sc_cap: sc as u32,
        })
    }

    /// The expansion's slots of the layout: the loop's (`lp_loop`: the pieces, their counts and
    /// places, the output), the arena's, the text pass's facts and the counters' ranking.
    pub fn lay(&self, l: &mut Lay) {
        l.at("THEADS", &self.theads).at("DELTA", &self.delta).at("DEPTH", &self.depth).at("STATE", &self.state);
        l.at("OFLAG", &self.oflag).at("CTL", &self.ctl).at("XFLAG", &self.xflag).at("XAT", &self.xat).at("XTOTAL", &self.xscan.total);
        l.at("XPOS", &self.xpos).at("V", &self.v).at("OBASE", &self.obase).at("LOB", &self.lob).at("PLAST", &self.plast).at("VTOTAL", &self.vscan.total);
        l.at("OPV", &self.opv).at("OPBASE", &self.opbase).at("OPTOTAL", &self.opscan.total);
        l.at("OUT", &self.out).at("TOKENS", &self.tokens).at("WORDS", &self.words).at("SCRATCH", &self.scratch);
        l.at("LPQ", &self.lpq).word("LPQ_TOP", self.lpq_top);
        l.at("XROWS", &self.xrows).word("XROWS_CAP", self.xrows_cap);
        l.word("OUT_CAP", self.out_cap);
        l.at("CNT", &self.cnt).word("CNT_CAP", self.cnt_cap).word("LB_CAP", self.lb_cap);
        l.at("TR_SK", &self.tr_sk).at("TR_NK", &self.tr_nk).at("TR_TY", &self.tr_ty).at("TO_F", &self.to_f).at("TO_S", &self.to_s);
        l.at("CU_SLOT", &self.cu_slot).at("CU_SUB", &self.cu_sub).at("CU_UNIT", &self.cu_unit).at("TRN", &self.trn).at("LREAD", &self.lread);
        for (name, b) in SC_RANKING.iter().zip(&self.sc_tok) {
            l.at(name, b);
        }
        l.at("SC_CKEYS", &self.sc_ckeys);
        l.at("SC_KB", &self.sc_sort.keys_b).at("SC_VA", &self.sc_sort.vals_a).at("SC_VB", &self.sc_sort.vals_b).at("SC_HIST", &self.sc_sort.hist).word("SC_CAP", self.sc_cap);
    }

    /// The scratch tokens' bytes and (start, end, kind) (after the command buffer).
    pub fn scratch(&self) -> (Vec<u8>, Vec<[u32; 3]>) {
        // SAFETY: the device finished; the tops are within the capacities where no room ran out.
        unsafe {
            let ctl = self.ctl.host().cast::<u32>();
            let (nb, nt) = (*ctl.add(4) as usize, *ctl.add(5) as usize);
            let bytes = self.scratch.host().add(self.st_cap * 12);
            let nb = nb.min(self.scratch.byte_length() - self.st_cap * 12);
            let nt = nt.min(self.st_cap);
            (std::slice::from_raw_parts(bytes, nb).to_vec(), std::slice::from_raw_parts(self.scratch.host().cast::<[u32; 3]>(), nt).to_vec())
        }
    }

    /// The refusals, and the output's refs where there are none (after the command buffer).
    /// Each store's use and room (`ROOM_STORES`'s order): its top (past the room where it ran
    /// out: a top only grows) and its capacity.
    pub fn gives(&self) -> u32 {
        // SAFETY: the device finished; `ctl` 16 counts the givings back (decision 0486 §10).
        unsafe { *self.ctl.host().cast::<u32>().add(16) }
    }

    pub fn room(&self) -> [(u32, u32); ROOMS] {
        // SAFETY: the device finished; `ctl` and `words` are the control and capacity words.
        unsafe {
            let ctl = self.ctl.host().cast::<u32>();
            let w = self.words.host().cast::<u32>();
            let [a, b, c] = [(0, 1, 8), (4, 8, 11), (5, 9, 12)].map(|(t, c, h)| ((*ctl.add(t)).max(*ctl.add(h)), *w.add(c)));
            // The output: the pieces' total of the text and of each top before (the lines'), as
            // `pp_check` and the loop compare it with the room.
            [a, b, c, ((*self.opscan.total.host().cast::<u32>()).max(*ctl.add(14)), self.out_cap)]
        }
    }

    /// Each unit's first output token: its first stretch's first top token (the stretches'
    /// scan), that token's piece's place (the pieces' scan); then the output's end.
    /// Where each unit of the batch begins in the output, from its first stretch in the unit's
    /// order (`P1_UFIRST`, the device's), and the output's end.
    fn unit_out(&self, vis: &Vis, units: u32, n_out: usize) -> Vec<u32> {
        // SAFETY: the device finished; the scans hold `n_gaps` and `nt` words, their totals after,
        // and `ufirst` a word a unit.
        unsafe {
            let n_gaps = *vis.n_gaps.host().cast::<u32>();
            let nt = *vis.gscan.total.host().cast::<u32>();
            let gat = vis.gat.host().cast::<u32>();
            let obase = self.obase.host().cast::<u32>();
            let first = vis.ufirst.host().cast::<u32>();
            let mut at: Vec<u32> = (0..units as usize)
                .map(|u| {
                    let p = *first.add(u);
                    let t = if p < n_gaps { *gat.add(p as usize) } else { nt };
                    if t < nt { *obase.add(t as usize) } else { n_out as u32 }
                })
                .collect();
            at.push(n_out as u32);
            at
        }
    }

    /// The refusals (`X_*`), after the command buffer.
    pub fn refusals(&self) -> u32 {
        // SAFETY: the device finished; `ctl` holds the control words.
        unsafe { *self.ctl.host().cast::<u32>().add(2) }
    }

    /// The text's output pieces (`OPTOTAL`).
    pub fn pieces(&self) -> u32 {
        // SAFETY: the device finished; a scan's total is a word.
        unsafe { *self.opscan.total.host().cast::<u32>() }
    }

    /// The longest piece's steps in the run (`TRN[16]`, decision 0588).
    /// The expansions' rows (decision 0613 §6), numbered from 1 in their order here: each one's definition's name,
    /// its parent (0 the text) and its root call's name (`Expanded::expansions`).
    pub fn expansions(&self) -> Vec<[u32; 3]> {
        // SAFETY: the device finished; `ctl` holds 32 words and `xrows` 3 a row for its `xrows_cap` rows.
        let (n, rows) = unsafe { (*self.ctl.host().cast::<u32>().add(26) as usize, std::slice::from_raw_parts(self.xrows.host().cast::<u32>(), 3 * self.xrows_cap as usize)) };
        let n = n.min(self.xrows_cap as usize);
        let mut id = vec![0u32; n + 1];
        let mut out = Vec::new();
        for g in 0..n {
            if rows[3 * g] != 0 {
                out.push([rows[3 * g] - 1, rows[3 * g + 1], rows[3 * g + 2]]);
                id[g + 1] = out.len() as u32;
            }
        }
        for r in &mut out {
            r[1] = id.get(r[1] as usize).copied().unwrap_or(0);
        }
        out
    }

    /// The requests posted (`LPQ` 5).
    pub fn posted(&self) -> u32 {
        // SAFETY: the device finished; `lpq` holds the queue's control words.
        unsafe { *self.lpq.host().cast::<u32>().add(5) }
    }

    /// The rows the roll-up walked (`CTL` 26, `lp_roll_rows`).
    pub fn log_rows(&self) -> u32 {
        // SAFETY: the device finished; `ctl` holds 32 words.
        unsafe { *self.ctl.host().cast::<u32>().add(26) }
    }

    pub fn steps(&self) -> u32 {
        // SAFETY: the device finished; `trn` holds 32 words.
        unsafe { *self.trn.host().cast::<u32>().add(16) }
    }

    /// The memo's hits and the results it kept (`TRN[28]`, `TRN[29]`, decision 0613 §2).
    pub fn memo(&self) -> (u32, u32) {
        // SAFETY: the device finished; `trn` holds 32 words.
        unsafe { (*self.trn.host().cast::<u32>().add(28), *self.trn.host().cast::<u32>().add(29)) }
    }

    /// The last text pass's pieces (`TRN[13]`, decision 0588).
    pub fn loop_pieces(&self) -> u32 {
        // SAFETY: the device finished; `trn` holds 32 words.
        unsafe { *self.trn.host().cast::<u32>().add(13) }
    }

    /// P1's iterations and the text passes the run made (`TRN[11]`, `TRN[10]`).
    pub fn passes(&self) -> (u32, u32) {
        // SAFETY: the device finished; `trn` holds 32 words.
        unsafe { (*self.trn.host().cast::<u32>().add(11), *self.trn.host().cast::<u32>().add(10)) }
    }

    /// The refusals of the directives' lines alone: those before the last text pass (the text's
    /// own are the expansion's to report).
    pub fn line_refusals(&self) -> u32 {
        // SAFETY: the device finished; `trn` holds the facts' counts, [9] the lines' refusals.
        unsafe { *self.trn.host().cast::<u32>().add(9) }
    }

    /// The output's refs, made from its pieces (decision 0520 §3) for a caller that asks for
    /// tokens: a top level's run each top token's node (the text's stretches, `top_ref`), a
    /// definition's body item its ref, a produced token its own.
    ///
    /// Every word it follows is checked against its store: an output whose pieces do not add up
    /// to its count, or name a node, gap or definition past their stores, is the device's failure
    /// (`BackendFailure`), never a read past a buffer.
    pub fn read(&self, front: &crate::pp_eval::Front, vis: &Vis, defs: &Defs) -> Result<(u32, Vec<u32>), Error> {
        fn words(b: &Buf) -> &[u32] {
            // SAFETY: the device finished; a store holds `bytes()` bytes of `u32` words.
            unsafe { std::slice::from_raw_parts(b.host().cast::<u32>(), b.byte_length() / 4) }
        }
        let refused = *words(&self.ctl).get(2).ok_or_else(|| crate::pp_arena::failure(concat!(file!(), ":", line!())))?;
        if refused != 0 {
            return Ok((refused, Vec::new()));
        }
        let n = *words(&self.vscan.total).first().ok_or_else(|| crate::pp_arena::failure(concat!(file!(), ":", line!())))? as usize;
        let cap = self.out_cap as usize;
        let np = (*words(&self.opscan.total).first().ok_or_else(|| crate::pp_arena::failure(concat!(file!(), ":", line!())))? as usize).min(cap);
        let o = words(&self.out).get(..8 * cap).ok_or_else(|| crate::pp_arena::failure(concat!(file!(), ":", line!())))?;
        let n_gaps = *words(&vis.n_gaps).first().ok_or_else(|| crate::pp_arena::failure(concat!(file!(), ":", line!())))? as usize;
        let gat = words(&vis.gat).get(..n_gaps).ok_or_else(|| crate::pp_arena::failure(concat!(file!(), ":", line!())))?;
        let firsts = words(&vis.gfirsts);
        let nodes = words(&front.text_nodes);
        // SAFETY: as `words`, of `PpDef` records.
        let d = unsafe { std::slice::from_raw_parts(defs.defs.host().cast::<PpDef>(), defs.defs.byte_length() / std::mem::size_of::<PpDef>()) };
        let mut out = Vec::with_capacity(n);
        let show = std::env::var_os("RERAIL_PP_PIECES").is_some();
        if show {
            // The roll-up's words, a top token each (the first 24): what the order and places are made from.
            let col = |b: &Buf| words(b).iter().take(24).map(|w| w.to_string()).collect::<Vec<_>>().join(" ");
            eprintln!("roll: n {n} np {np}\n  xflag {}\n  v {}\n  opv {}\n  obase {}\n  opbase {}\n  theads {}", col(&self.xflag), col(&self.v), col(&self.opv), col(&self.obase), col(&self.opbase), col(&self.theads));
            let c = |k: usize| o[k * cap..k * cap + 16].iter().map(|w| w.to_string()).collect::<Vec<_>>().join(" ");
            eprintln!("  out src {}\n  out len {}\n  out st {}\n  out place {}\n  order {}\n  chunks {}", c(0), c(1), c(2), c(3), c(5), c(6));
        }
        // The tokens' store: its columns (`Ar`'s), the leaves of the requests' results among its entries.
        let tw = words(&self.tokens);
        let tc = tw.len() / 5;
        for i in 0..np {
            // The loop's order of its pieces (`lp_roll_place`): an output piece of its own, or (`cap` past its
            // entry's) a leaf of a request's result in the tokens' store (decision 0613 §6), read in place.
            let e = o[5 * cap + i] as usize;
            let (src, len, st) = if e < cap {
                (o[e], o[cap + e] & PN_LEN, o[2 * cap + e])
            } else {
                let x = e - cap;
                if x >= tc {
                    return Err(crate::pp_arena::failure(concat!(file!(), ":", line!())));
                }
                // A hit's leaves of the text are its own call's tokens, its `(` this far past the first's (the
                // order's eighth column, `lp_rq_post`).
                let (src, fl, st) = (tw[x], tw[tc + x], tw[2 * tc + x]);
                let (src, len) = if src & P_KIND == P_SPAN {
                    let sub = src & !P_KIND;
                    (if sub == LP_TOPSRC { P_TOP } else { P_BODY | sub }, fl >> 4)
                } else {
                    (src, 1)
                };
                (src, len, if src & P_KIND == P_TOP { st.wrapping_add(o[7 * cap + i]) } else { st })
            };
            if show {
                eprintln!("piece {i}: kind {} def {} start {st} len {len} at {}", src >> 30, src & !P_KIND, o[3 * cap + e]);
            }
            if out.len() + len as usize > n {
                if std::env::var_os("RERAIL_PP_WHY").is_some() {
                    eprintln!("pp: the output's piece {i} of {np} (room {cap}) passes its count {n}");
                }
                return Err(crate::pp_arena::failure(concat!(file!(), ":", line!())));
            }
            for k in st..st.checked_add(len).ok_or_else(|| crate::pp_arena::failure(concat!(file!(), ":", line!())))? {
                out.push(match src & P_KIND {
                    P_TOP => {
                        let j = gat.partition_point(|&g| g <= k).max(1) - 1;
                        let at = firsts.get(j).ok_or_else(|| crate::pp_arena::failure(concat!(file!(), ":", line!())))?.checked_add(k.wrapping_sub(*gat.get(j).ok_or_else(|| crate::pp_arena::failure(concat!(file!(), ":", line!())))?)).ok_or_else(|| crate::pp_arena::failure(concat!(file!(), ":", line!())))?;
                        *nodes.get(at as usize).ok_or_else(|| crate::pp_arena::failure(concat!(file!(), ":", line!())))?
                    }
                    P_BODY => d.get((src & !P_KIND) as usize).ok_or_else(|| crate::pp_arena::failure(concat!(file!(), ":", line!())))?.body + k,
                    _ => k,
                });
            }
        }
        if out.len() != n {
            if std::env::var_os("RERAIL_PP_WHY").is_some() {
                eprintln!("pp: the output's {np} pieces (room {cap}) hold {} tokens, its count {n}", out.len());
            }
            return Err(crate::pp_arena::failure(concat!(file!(), ":", line!())));
        }
        Ok((0, out))
    }
}

/// A unit expanded on the device: its output's tokens (refs to its tapes' nodes, in order), or
/// why it is refused (`X_*` bits), and the rung it ran at.
pub struct Expanded {
    pub ev: crate::pp_eval::Evaluated,
    pub refused: u32,
    pub out: Vec<u32>,
    pub rung: u32,
    /// A command's launches: `2 * PP_LAUNCHES` for every unit, `pp_run` and `pp_rebase` (decision 0653).
    pub launches: usize,
    /// The command buffer's time on the GPU (its last run's), in seconds.
    pub gpu_seconds: f64,
    /// Where each unit of the batch begins in `out`, and its end.
    pub unit_out: Vec<u32>,
    /// The scratch tokens (a ref with `SCRATCH` set): their bytes, and each one's (start, end,
    /// kind).
    pub scratch: Vec<u8>,
    pub stoks: Vec<[u32; 3]>,
    /// The arena's use and room at the rung it ran at, store by store (`ROOM_STORES`).
    pub room: [(u32, u32); ROOMS],
    /// The text's output pieces (`OPTOTAL`): the output's room holds them in the loop's chunks
    /// (`lp_flush`), so its use is the chunks'.
    pub pieces: u32,
    /// P1's refusals (`P1_*`), and the round's requests (decision 0467 §1): with either, the
    /// round ended before the expansion, and `out` is empty.
    pub p1_flags: u32,
    pub requests: Vec<Request>,
    /// The rooms the run settled at, and its P1 iterations and text passes (measurements).
    pub caps: Caps,
    pub xcaps: XCaps,
    pub passes: (u32, u32),
    /// The givings back at the loop's substitutions (decision 0486 §10), every top's.
    pub gives: u32,
    /// The longest piece's steps (`TRN[16]`, decision 0588): a measurement, and the tests' bound on a chain.
    pub steps: u32,
    /// The memo's hits and the results it kept (`TRN[28]`, `TRN[29]`, decision 0613 §2), every pass's.
    pub memo: (u32, u32),
    /// A round that made requests: the tree its P1 settled at, the next round's start (`Layout::entered`).
    pub tree: Option<Tree>,
    pub def_rung: u32,
    /// The unit's expansions (decision 0613 §6, the sink's `Expansion` rows), in the roll-up's order: each one's
    /// definition (its name's top token), its parent (a row's number from 1, 0 the text), and the top token its
    /// root call's name is.
    pub expansions: Vec<[u32; 3]>,
    /// The rows the roll-up walked (`CTL` 26: each run's log, an occurrence's rows each time it is read): the
    /// logs' use of the tokens' store, as the output's is the output's.
    pub log_rows: u32,
    /// The requests posted (`LPQ` 5): the calls read in a run of their own (decision 0613 §4).
    pub posted: u32,
    /// The command it ran in (`next_run`): a batch's units ran in one, and its GPU time is that command's.
    pub run: u64,
}

/// A command's number, the process's next (`Expanded::run`).
pub fn next_run() -> u64 {
    static RUNS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// The arena's stores, as `Expanded::room` lists them.
pub const ROOM_STORES: [&str; ROOMS] = ["tokens", "scratch bytes", "scratch tokens", "output"];
/// The expansion's rooms a run reports (`Exp::room`).
pub const ROOMS: usize = 4;

/// A ref to a scratch token, not a tape node.
pub const SCRATCH: u32 = 0x8000_0000;

/// The front passes, the definitions, the lookups and the expansion over `unit` laid out as
/// `lay`, in one command buffer; from rung `rung`, and at the next where a store's room ran out.
pub fn expand_from(unit: &Unit, lay: &Layout, rung: u32) -> Result<Expanded, Error> {
    let mut sizes = Sizes::first(unit, lay);
    sizes.x = XCaps::at(sizes.p1.top, rung);
    expand_sized(unit, lay, &mut sizes)
}

/// `expand_from` from the rooms `sizes` (a unit's last run's), which it leaves at the rooms this
/// run settled at. Each room grows a bounded number of times a call. A run the system stopped runs
/// again (`pp_eval::preempted_again`).
pub fn expand_sized(unit: &Unit, lay: &Layout, sizes: &mut Sizes) -> Result<Expanded, Error> {
    preempted_again(|| expand_sized_once(unit, lay, sizes))
}

fn expand_sized_once(unit: &Unit, lay: &Layout, sizes: &mut Sizes) -> Result<Expanded, Error> {
    let mut r = Runs::new(unit, lay, sizes);
    loop {
        let arena = Arena::new();
        let m = match r.make(&arena) {
            Ok(m) => m,
            Err(done) => return *done,
        };
        match arena.lease() {
            Ok(()) => {}
            Err(Error::BackendCapacity) if r.short.is_some() => return Ok(r.short.take().unwrap()),
            Err(e) => return Err(e),
        }
        let (launches, gpu_seconds) = on_unit(&arena, unit, &m.front, &m.l)?;
        if let Some(done) = r.after(m, launches, gpu_seconds, next_run(), sizes) {
            return done;
        }
    }
}

/// A batch's units expanded together (decisions 0588 §6, 0653): each round of runs is one program run
/// (`pp_eval::on_program`) over the units still running, each in stores of its own in one arena, their programs on
/// one queue, and a unit that runs again (a room grown) does so in the next. Each unit's answer, by its index, from
/// the rooms `sizes` (its last round's), which it leaves at the rooms each settled at. `Err(BackendCapacity)` for the
/// batch where a round's arena is past the budget, or its documents are where a batch cannot read them together: the
/// caller runs it in halves.
pub fn expand_batch(units: &[(&Unit, &Layout)], sizes: &mut [Sizes]) -> Result<Vec<Result<Expanded, Error>>, Error> {
    let n = units.len();
    let Some(&(first, _)) = units.first() else { return Ok(Vec::new()) };
    // Every unit's views from the first unit's slab, the one the command binds.
    let members: Vec<Unit> = units.iter().map(|(u, _)| u.rebased(first)).collect::<Result<_, _>>()?;
    let mut resident: Vec<std::sync::Arc<crate::slabs::Slab>> = Vec::new();
    for m in &members {
        for s in &m.slabs {
            if !resident.iter().any(|r| r.id == s.id) {
                resident.push(std::sync::Arc::clone(s));
            }
        }
    }
    let mut runs: Vec<Runs<'_>> = members.iter().zip(units).zip(sizes.iter()).map(|((m, (_, l)), s)| Runs::new(m, l, s)).collect();
    let mut out: Vec<Option<Result<Expanded, Error>>> = (0..n).map(|_| None).collect();
    let mut stopped = 0;
    let (run, mut gpu) = (next_run(), 0f64);
    loop {
        let arena = Arena::new();
        let mut made: Vec<(usize, Made)> = Vec::new();
        for k in 0..n {
            if out[k].is_some() {
                continue;
            }
            match runs[k].make(&arena) {
                Ok(m) => made.push((k, m)),
                Err(done) => out[k] = Some(*done),
            }
        }
        if made.is_empty() {
            break;
        }
        let tiles: Vec<u32> = made.iter().map(|(_, m)| m.l.get("TMAX")).collect();
        let queue = crate::pp_eval::Queue::new(&arena, &tiles)?;
        arena.lease()?;
        let ran = {
            let refs: Vec<(&Front, &Lay)> = made.iter().map(|(_, m)| (&m.front, &m.l)).collect();
            let data: &dyn crate::pp_arena::Bound = match members[0].data() {
                Some(b) => b,
                None => &made[0].1.front.files,
            };
            crate::pp_eval::on_program(&arena, &queue, &refs, data, &resident)
        };
        match ran {
            Ok((launches, gpu_seconds)) => {
                gpu += gpu_seconds;
                // `RERAIL_PP_TRACE`: every unit's start and end on the device's clock, from the round's first start:
                // the latest ends.
                if std::env::var_os("RERAIL_PP_TRACE").is_some() {
                    let spans: Vec<(usize, u64, u64, usize)> = made
                        .iter()
                        .filter_map(|(k, m)| {
                            let t = m.front.trace();
                            Some((*k, t.first()?.1, t.last()?.1, t.len()))
                        })
                        .collect();
                    let t0 = spans.iter().map(|s| s.1).min().unwrap_or(0);
                    let mut by_end = spans.clone();
                    by_end.sort_by_key(|s| std::cmp::Reverse(s.2));
                    eprintln!("UNITS {} units; the latest ends (unit: start..end us, phases, nodes):", spans.len());
                    for (k, a, b, p) in by_end.iter().take(6) {
                        eprintln!("UNITS   unit {k}: {:.1}..{:.1}, {p} phases, nodes {}", (a - t0) as f64 / 1e3, (b - t0) as f64 / 1e3, members[*k].node_bound);
                    }
                }
                for (k, m) in made {
                    if let Some(done) = runs[k].after(m, launches, gpu_seconds, run, &mut sizes[k]) {
                        out[k] = Some(done);
                    }
                }
            }
            // The system stopped the command (`pp_eval::preempted_again`'s rule): the round again.
            Err(Error::BackendPreempted) if stopped + 1 < crate::pp_eval::PREEMPTED_RUNS => stopped += 1,
            Err(Error::BackendPreempted) => return Err(crate::pp_arena::failure(concat!(file!(), ":", line!(), ": stopped by the system each run"))),
            Err(e) => return Err(e),
        }
    }
    // Each answer's GPU time is the batch's: every command of the call (`run`).
    Ok(out
        .into_iter()
        .map(|r| {
            let mut x = r.unwrap_or_else(|| Err(crate::pp_arena::failure(concat!(file!(), ":", line!()))))?;
            (x.gpu_seconds, x.run) = (gpu, run);
            Ok(x)
        })
        .collect())
}

/// One unit's runs toward its expansion (`expand_sized_once`, `expand_batch`): its rooms, the rooms'
/// tries so far, and the last run's answer where it refused for the expansion's room (it stands
/// where the next run's room is past the device's: the serial pass takes the unit).
struct Runs<'u> {
    unit: &'u Unit,
    lay: &'u Layout,
    def_rung: u32,
    p1: Caps,
    x: XCaps,
    p1_tries: u32,
    x_tries: u32,
    short: Option<Expanded>,
}

/// A unit's stores for one run, in its arena, and their layout.
struct Made {
    front: Front,
    defs: Defs,
    vis: Vis,
    exp: Exp,
    l: Lay,
}

impl<'u> Runs<'u> {
    fn new(unit: &'u Unit, lay: &'u Layout, sizes: &Sizes) -> Runs<'u> {
        Runs { unit, lay, def_rung: sizes.defs, p1: sizes.p1, x: sizes.x, p1_tries: 0, x_tries: 0, short: None }
    }

    /// The next run's stores in `arena`; or the unit's answer, where they are past the device's room.
    fn make(&mut self, arena: &Arena) -> Result<Made, Box<Result<Expanded, Error>>> {
        let (unit, lay) = (self.unit, self.lay);
        self.x = self.x.fit(self.p1.top);
        let rung = self.x.rung;
        let made = (|| -> Result<_, Error> {
            let front = Front::new(arena, unit, false)?;
            let defs = Defs::new(arena, unit, self.def_rung)?;
            let vis = Vis::new(arena, unit, lay, self.p1, rung)?;
            let exp = Exp::new(arena, unit, lay, self.x, self.p1)?;
            Ok((front, defs, vis, exp))
        })();
        let (front, defs, vis, exp) = match made {
            Ok(m) => m,
            Err(Error::BackendCapacity) if self.short.is_some() => return Err(Box::new(Ok(self.short.take().unwrap()))),
            Err(e) => return Err(Box::new(Err(e))),
        };
        let mut l = Lay::new();
        front.lay(&mut l, TO_ALL);
        defs.lay(&mut l);
        vis.lay(&mut l);
        exp.lay(&mut l);
        // A phase's tiles at most: the unit's nodes, or its expansion's tokens, whose room each refused run grows.
        l.word("TMAX", crate::pp_arena::tiles_for(unit.node_bound.max(self.x.tok)));
        arena_log(arena, &l, format_args!("expansion at rung {rung}, p1 rung {}, defs rung {}", self.p1.rung, self.def_rung));
        Ok(Made { front, defs, vis, exp, l })
    }

    /// The run `m` read (its command's `launches` and GPU time): the unit's answer, or `None` where it runs again (a
    /// room grown). `sizes` is left at the rooms it ran at.
    fn after(&mut self, m: Made, launches: usize, gpu_seconds: f64, run: u64, sizes: &mut Sizes) -> Option<Result<Expanded, Error>> {
        let (unit, lay) = (self.unit, self.lay);
        let Made { front, defs, vis, exp, .. } = m;
        let (p1, x, def_rung) = (self.p1, self.x, self.def_rung);
        let rung = x.rung;
        if std::env::var_os("RERAIL_PP_LOOP_LOG").is_some() {
            // SAFETY: the device finished; `trn` holds 32 words, 16 to 19 the longest piece's, 20 to 23 the slowest's.
            let w = |k: usize| unsafe { *exp.trn.host().cast::<u32>().add(k) };
            // SAFETY: as `w`: the queue's control words (`LPQ`, 16), 5 to 7 the requests posted, the pieces read
            // again and the requests' steps.
            let q = |k: usize| unsafe { *exp.lpq.host().cast::<u32>().add(k) };
            eprintln!("pp loop: {} pieces; the longest {} steps, top tokens {}..{}, {} tokens out; the slowest {} us, {} steps, top tokens {}..{} (nodes {}); all pieces {} us, {} steps; memo {} hits, {} kept; requests {}, {} steps, pieces read again {}; arguments taken {}, read by their caller {}, read twice {}; tokens {:?} rung {} top {}", exp.loop_pieces(), w(16), w(17), w(18), w(19), f64::from(w(20)) / 1e3, w(22), w(21), w(23), unit.node_bound, w(24), w(25), w(28), w(29), q(5), q(7), q(6), q(8), q(9), q(10), exp.room()[0], x.rung, p1.top);
        }
        *sizes = Sizes { p1, x, defs: def_rung, used: Some((vis.p1_need(), Caps::first(unit, lay))) };
        let expanded = |refused: u32, p1_flags: u32, requests: Vec<Request>, out: Vec<u32>, unit_out: Vec<u32>, scratch: Vec<u8>, stoks: Vec<[u32; 3]>| -> Result<Expanded, Error> {
            Ok(Expanded { ev: front.read()?, refused, p1_flags, requests, out, rung, launches, gpu_seconds, unit_out, scratch, stoks, room: exp.room(), pieces: exp.pieces(), caps: p1, xcaps: x, passes: exp.passes(), posted: exp.posted(), gives: exp.gives(), steps: exp.steps(), memo: exp.memo(), tree: None, def_rung, expansions: exp.expansions(), log_rows: exp.log_rows(), run })
        };
        // SAFETY: the device finished writing `room`.
        let why = |room: &str| {
            if std::env::var_os("RERAIL_PP_WHY").is_some() {
                eprintln!("pp: again, larger: {room} (nodes {})", unit.node_bound);
            }
        };
        if unsafe { *defs.room.host().cast::<u32>() } != 0 {
            why("the definitions");
            self.def_rung += 1;
            if self.def_rung >= RUNGS {
                return Some(Err(crate::pp_arena::capacity(concat!(file!(), ":", line!()))));
            }
            return None;
        }
        // P1 ended the round: room (a rung up), requests (the host's to answer), or a refusal.
        let p1_flags = vis.p1_flags();
        if p1_flags == P1_ROOM && self.p1_tries + 1 < P1_RUNGS {
            why("P1");
            vis.log_room(unit, p1);
            self.p1 = p1.grown(unit, lay, vis.p1_need());
            self.p1_tries += 1;
            return None;
        }
        // The lines' room (their tokens are the expansion's stores): what they decided is not to
        // be read, so before the requests and P1's refusals.
        let lines = exp.line_refusals();
        if lines & X_ROOM != 0 && self.x_tries + 1 < EXP_RUNGS {
            why("the lines' expansion");
            self.x = x.grown(p1.top, exp.room());
            self.x_tries += 1;
            return None;
        }
        let requests = vis.requests();
        if p1_flags != 0 || !requests.is_empty() {
            let tree = (p1_flags == 0).then(|| vis.tree());
            return Some(expanded(0, p1_flags, requests, Vec::new(), Vec::new(), Vec::new(), Vec::new()).map(|mut e| {
                e.tree = tree;
                e
            }));
        }
        if let Err(e) = vis.read() {
            return Some(Err(e));
        }
        let (refused, out) = match exp.read(&front, &vis, &defs) {
            Ok(r) => r,
            Err(e) => return Some(Err(e)),
        };
        // A run out of room runs again larger whatever else it refused: past the room that ran
        // out, its items read what was never written (a list cut short reads as a call past its
        // end), so its other refusals are the larger run's to give.
        if refused & X_ROOM != 0 && self.x_tries + 1 < EXP_RUNGS {
            why(&format!("the expansion: {:?} (tok, sb, st, out: used, room)", exp.room()));
            if std::env::var_os("RERAIL_PP_ARENA_LOG").is_some() {
                eprintln!("expansion room at rung {rung}: (used, cap) {:?} {:?}", ROOM_STORES, exp.room());
            }
            self.x = x.grown(p1.top, exp.room());
            self.x_tries += 1;
            match expanded(refused, 0, Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()) {
                Ok(mut e) => {
                    e.xcaps = self.x;
                    self.short = Some(e);
                }
                Err(e) => return Some(Err(e)),
            }
            return None;
        }
        if std::env::var_os("RERAIL_PP_ARENA_LOG").is_some() {
            eprintln!("settled: p1 need {:?} against nodes {} dirs {} gaps {} top {} reqs {} names {}; expansion {:?}", vis.p1_need(), p1.nodes, p1.dirs, p1.gaps, p1.top, p1.reqs, p1.names, exp.room());
        }
        let (scratch, stoks) = exp.scratch();
        let unit_out = if refused != 0 { Vec::new() } else { exp.unit_out(&vis, lay.units(), out.len()) };
        Some(expanded(refused, 0, Vec::new(), out, unit_out, scratch, stoks))
    }
}

/// `expand_from` rung 0.
pub fn expand(unit: &Unit, lay: &Layout) -> Result<Expanded, Error> {
    expand_from(unit, lay, 0)
}

/// `RERAIL_PP_ARENA_LOG`: an arena's bytes, and its largest stores.
fn arena_log(arena: &Arena, l: &Lay, what: std::fmt::Arguments) {
    if std::env::var_os("RERAIL_PP_ARENA_LOG").is_some() {
        let big: Vec<String> = l.largest(if std::env::var_os("RERAIL_PP_ARENA_ALL").is_some() { 400 } else { 12 }).iter().map(|(n, b)| format!("{n} {:.1}", *b as f64 / 1048576.0)).collect();
        eprintln!("arena {:.1} MiB, {what}: {}", arena.bytes() as f64 / 1048576.0, big.join(", "));
    }
}

/// The front passes, the definitions and the lookups over `unit` laid out as `lay`, in one
/// command buffer: each active stretch token's meaning (`L_*`, or the directive row of its
/// definition), in the layout's order; and P1's answer (`Vis::p1`).
pub fn lookups(unit: &Unit, lay: &Layout) -> Result<(crate::pp_eval::Evaluated, Vec<u32>, P1Out), Error> {
    lookups_sized(unit, lay, &mut Sizes::first(unit, lay))
}

/// `lookups` from the rooms `sizes` (the unit's last round's), which it leaves at the rooms this
/// run settled at.
pub fn lookups_sized(unit: &Unit, lay: &Layout, sizes: &mut Sizes) -> Result<(crate::pp_eval::Evaluated, Vec<u32>, P1Out), Error> {
    preempted_again(|| lookups_at(unit, lay, sizes))
}

fn lookups_at(unit: &Unit, lay: &Layout, sizes: &mut Sizes) -> Result<(crate::pp_eval::Evaluated, Vec<u32>, P1Out), Error> {
    let Sizes { p1, x, defs: rung, .. } = *sizes;
    let (mut rung, mut p1, mut x) = (rung, p1, x);
    let (mut p1_tries, mut x_tries) = (0, 0);
    loop {
        x = x.fit(p1.top);
        let x_rung = x.rung;
        let arena = Arena::new();
        let front = Front::new(&arena, unit, false)?;
        let defs = Defs::new(&arena, unit, rung)?;
        let vis = Vis::new(&arena, unit, lay, p1, x_rung)?;
        // The expansion's stores: the tests' lines are expanded in P1 (decision 0467 §4).
        let exp = Exp::new(&arena, unit, lay, x, p1)?;
        let mut l = Lay::new();
        front.lay(&mut l, TO_VIS);
        defs.lay(&mut l);
        vis.lay(&mut l);
        exp.lay(&mut l);
        arena_log(&arena, &l, format_args!("lookups at rung {x_rung}, p1 rung {}, defs rung {rung}", p1.rung));
        arena.lease()?;
        on_unit(&arena, unit, &front, &l)?;
        *sizes = Sizes { p1, x, defs: rung, used: Some((vis.p1_need(), Caps::first(unit, lay))) };
        // SAFETY: the device finished writing `room`.
        if unsafe { *defs.room.host().cast::<u32>() } != 0 {
            rung += 1;
            if rung >= RUNGS {
                return Err(crate::pp_arena::capacity(concat!(file!(), ":", line!())));
            }
            continue;
        }
        let mut out = vis.p1();
        out.flags = vis.p1_flags();
        if out.flags == P1_ROOM && p1_tries + 1 < P1_RUNGS {
            vis.log_room(unit, p1);
            p1 = p1.grown(unit, lay, vis.p1_need());
            p1_tries += 1;
            continue;
        }
        out.refused = exp.line_refusals();
        if out.refused & X_ROOM != 0 && x_tries + 1 < EXP_RUNGS {
            x = x.grown(p1.top, exp.room());
            x_tries += 1;
            continue;
        }
        out.requests = vis.requests();
        if out.flags != 0 || !out.requests.is_empty() {
            return Ok((front.read()?, Vec::new(), out));
        }
        return Ok((front.read()?, vis.read()?, out));
    }
}

/// The dialect's options as the relexer reads them (`pp_expand.uglykl`'s `O_*`): C++, `::`, digit
/// separators, raw strings.
pub fn relex_options(cpp: bool, colons: bool, digit_separators: bool, raw_strings: bool) -> u32 {
    u32::from(cpp) | u32::from(colons) << 1 | u32::from(digit_separators) << 2 | u32::from(raw_strings) << 3
}

/// The relexer's answer where a spelling is no one token, and where the device refuses it.
pub const RELEX_NONE: u32 = 0;
pub const RELEX_REFUSED: u32 = 0xffff;

/// For tests: the relexer's answer for each spelling (a tape kind, `RELEX_NONE` or
/// `RELEX_REFUSED`), under `options`.
pub fn relex(texts: &[Vec<u8>], options: u32) -> Result<Vec<u32>, Error> {
    let mut bytes = Vec::new();
    let mut offsets = vec![0u32];
    for t in texts {
        bytes.extend_from_slice(t);
        offsets.push(bytes.len() as u32);
    }
    bytes.extend_from_slice(&[0; 16]);
    let arena = Arena::new();
    let (b, o) = (arena.from_slice(&bytes)?, arena.from_slice(&offsets)?);
    let out = arena.zeroed::<u32>(texts.len())?;
    arena.lease()?;
    let n = texts.len() as u32;
    let args = crate::pp_arena::ProbeArgs([n, options, 0, 0]);
    crate::pp_eval::run(|enc| {
        enc.run("pd_relex_test", &[("bytes", &b), ("offsets", &o), ("out", &out)], Some(args), u64::from(n).div_ceil(crate::pp_arena::tg()));
        Ok(())
    })?;
    // SAFETY: the device finished writing `out`.
    Ok(unsafe { std::slice::from_raw_parts(out.host().cast::<u32>(), texts.len()).to_vec() })
}

/// For measurements: the GPU time of a run over `unit` that ends after stage `last` (`TO_FRONT`, `TO_DEFS`,
/// `TO_VIS`; `TO_ALL` with the expansion; `TO_ALL + 1` the whole setup without it), or before its phase `stop` (0:
/// none), in seconds.
pub fn stage_seconds(unit: &Unit, lay: &Layout, last: u32, stop: u32) -> Result<f64, Error> {
    let caps = Caps::first(unit, lay);
    stage_run(unit, lay, (last, stop, 0), (caps, XCaps::at(caps.top, 0), 0))
}

/// `stage_seconds` at the rooms the run `x` settled at, with P1's iterations bounded by `iters`
/// (`P1_ITERS` where 0).
pub fn stage_seconds_as(unit: &Unit, lay: &Layout, last: u32, stop: u32, iters: u32, x: &Expanded) -> Result<f64, Error> {
    stage_run(unit, lay, (last, stop, iters), (x.caps, x.xcaps, x.def_rung))
}

/// A run over `unit` at the rooms given, laid out to its stage `last` and `stop`, P1's iterations bounded by `iters`
/// where not 0: its front, to read its program's words (`Front::trace`), and its GPU time.
fn stage_front(unit: &Unit, lay: &Layout, (last, stop, iters): (u32, u32, u32), (caps, xcaps, def_rung): (Caps, XCaps, u32)) -> Result<(Front, f64), Error> {
    let arena = Arena::new();
    let front = Front::new(&arena, unit, false)?;
    let defs = Defs::new(&arena, unit, def_rung)?;
    let vis = Vis::new(&arena, unit, lay, caps, xcaps.rung)?;
    let exp = Exp::new(&arena, unit, lay, xcaps, caps)?;
    arena.lease()?;
    let mut l = Lay::new();
    front.lay(&mut l, last);
    defs.lay(&mut l);
    vis.lay(&mut l);
    exp.lay(&mut l);
    l.word("TMAX", crate::pp_arena::tiles_for(unit.node_bound.max(xcaps.tok)));
    l.word("STOP", stop);
    if iters != 0 {
        l.word("P1_ITERS", iters);
    }
    let seconds = on_unit(&arena, unit, &front, &l)?.1;
    Ok((front, seconds))
}

fn stage_run(unit: &Unit, lay: &Layout, run: (u32, u32, u32), rooms: (Caps, XCaps, u32)) -> Result<f64, Error> {
    Ok(stage_front(unit, lay, run, rooms)?.1)
}

/// The phases a run over `unit` passes, each its source line in the evaluator (a traced run, `STOP` at
/// `PP_TRACE_ON`), at the rooms its run `x` settled at: decision 0588's count of the expansion's phases.
pub fn phase_lines(unit: &Unit, lay: &Layout, x: &Expanded) -> Result<Vec<u32>, Error> {
    let (front, _) = stage_front(unit, lay, (TO_ALL, u32::MAX, 0), (x.caps, x.xcaps, x.def_rung))?;
    Ok(front.trace().into_iter().map(|(line, _)| line).collect())
}

/// One unit's answer from a batch (`rounds::run_many`, decision 0486 §7): the batch's expansion it ran in, its files
/// there (`x.ev.bases`' and the refs' files), and its output (`x.out`'s range). Where the unit was
/// refused, it ran alone, and `x.refused` says why.
pub struct Part {
    pub x: std::rc::Rc<Expanded>,
    pub files: std::ops::Range<usize>,
    pub out: std::ops::Range<usize>,
}

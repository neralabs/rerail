//! Tier 2's analysis (docs/parser-generators/15-analysis-and-plans.md §3, 16 §4): every value
//! of a block program, tier 1's ops and the values its rows imply, with a class, a monoid and a
//! depth; the scans by depth; the candidates; the passes over the bytes. The plan (phase 4) and
//! the generators read it. Nothing here runs a file.
//!
//! - **Tier 1:** `toggle`, `escaped` and `fill` carry a state bit each. They merge into one scan,
//!   S1, a function of those bits; its candidates are their values at a block's first byte. A
//!   `prev` of a value S1 decides reads the block before's last bits: a halo, an output of S1.
//! - **Tier 2:** the rows' conditions are scans over the tokens. `after K` and `before K` carry
//!   one bit (`last 1`), `parent K` reads the stack (`dyck`), the first token is an `or`, a
//!   node's place an `add`. A scan's depth is one more than its inputs'.
//! - **Fusing:** a summary that reads an earlier scan's carry through a bit at the block's edge
//!   stores a candidate (S1's state) or a fix (any other bit), and is made in the pass before.
//!   One that reads a wide carry inside the block, as `parent` reads the stack, waits for a
//!   pass over the bytes after that scan.

use super::block::{BlockOp, BlockProgram, Fact, RowCond, RowEnd, RowStart};
use std::fmt::Write as _;

/// Who runs the plan: Metal (scans over entries) or the CPU (one forward pass).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum Target {
    Gpu,
    Cpu,
}

/// Which of a row's rules a target uses: its `when`, or its `also`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum Rule {
    When,
    Also,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum Class {
    Local,
    Forward,
    Backward,
    /// The last bits of the block before, of a value S1 decides: an output of S1.
    Halo,
}

/// A carried value's monoid (`block_monoid`), with its width.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum Monoid {
    /// A function of `inputs` state bits to those and `outputs - inputs` halo bits.
    Fn { inputs: u8, outputs: u8 },
    /// One state bit, by parity (`toggle`) or by a borrow (`escaped`, `fill`): parts of S1.
    Xor,
    Borrow,
    HaloBits(u8),
    Last(u8),
    Or,
    Add(u8),
    Dyck { kinds: u8, max: u8, pair: bool },
    /// One `last` per level, for matching a close to its open.
    PerLevel { bits: u8, levels: u8 },
}

impl Monoid {
    fn text(&self) -> String {
        match self {
            Monoid::Fn { inputs, outputs } => format!("fn {inputs}->{outputs} ({} bits)", (1u32 << inputs) * u32::from(*outputs)),
            Monoid::Xor => "xor".into(),
            Monoid::Borrow => "fn".into(),
            Monoid::HaloBits(1) => "1 bit".into(),
            Monoid::HaloBits(k) => format!("{k} bits"),
            Monoid::Last(b) => format!("last {b}"),
            Monoid::Or => "or".into(),
            Monoid::Add(b) => format!("add {b}"),
            Monoid::Dyck { kinds, max, pair } => format!("dyck {kinds} max {max}{}", if *pair { " pair" } else { "" }),
            Monoid::PerLevel { bits, levels } => format!("last {bits} x {levels}"),
        }
    }

    /// The scan's name for its monoid, in the scan list: the stack by its stored width.
    fn scan_text(&self) -> String {
        match self {
            Monoid::Dyck { .. } => "dyck 41".into(),
            other => other.text(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct Value {
    pub(super) name: String,
    pub(super) class: Class,
    pub(super) monoid: Option<Monoid>,
    pub(super) depth: u8,
    /// The scan it belongs to, or a note.
    pub(super) scan: String,
    /// It reads a wide carry (the stack) inside the block.
    wide: bool,
    /// The scans' values it reads through a bit at the block's edge: fixes, not candidates.
    edges: Vec<String>,
}

/// The scans of one depth: one scan of the product of their monoids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct Scan {
    pub(super) name: String,
    pub(super) depth: u8,
    pub(super) forward: Vec<Monoid>,
    pub(super) backward: Vec<Monoid>,
    /// Its summaries are stored per candidate.
    pub(super) cand: bool,
    pub(super) fixes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct Dataflow {
    pub(super) target: Target,
    pub(super) rule: Rule,
    /// Tier 1's ops: all, local, given S1, carried, halos.
    pub(super) tier1: [usize; 5],
    pub(super) values: Vec<Value>,
    pub(super) scans: Vec<Scan>,
    /// S1's state bits; the candidates are `1 << state_bits`.
    pub(super) state_bits: u8,
    pub(super) byte_passes: u8,
    /// The most nodes that start at one byte, and the bit planes that count them.
    pub(super) nodes_at_a_byte: u8,
}

/// The bytes one entry covers: a SIMD group of 32 lanes of 64 bytes.
pub(super) const ENTRY_BYTES: u32 = 2048;

/// A target's dataflow, under the rule it takes: Metal the one with fewer passes over the bytes
/// (`when` on a tie); the CPU the one with nothing backward.
pub(in crate::code_rule_dsl) fn analyse(p: &BlockProgram, target: Target) -> Result<Dataflow, String> {
    let when = analyse_for(p, Rule::When, target)?;
    let has_also = p.nodes.iter().any(|r| r.also.is_some());
    match target {
        Target::Gpu if has_also => {
            let also = analyse_for(p, Rule::Also, target)?;
            Ok(if also.byte_passes < when.byte_passes { also } else { when })
        }
        Target::Cpu if when.values.iter().any(|v| v.class == Class::Backward) => {
            let also = analyse_for(p, Rule::Also, target)?;
            if also.values.iter().any(|v| v.class == Class::Backward) {
                return Err("no rule of the program runs forward".into());
            }
            Ok(also)
        }
        _ => Ok(when),
    }
}

/// The dataflow on Metal under a given rule.
pub(in crate::code_rule_dsl) fn analyse_rule(p: &BlockProgram, rule: Rule) -> Result<Dataflow, String> {
    analyse_for(p, rule, Target::Gpu)
}

/// A kind's name: its keyword's text, if one gives it.
fn kind_name(p: &BlockProgram, k: u16) -> String {
    match p.keywords.iter().find(|w| w.kind == k) {
        Some(w) if w.word.len() == 1 => format!("'{}'", w.word[0] as char),
        Some(w) => format!("\"{}\"", String::from_utf8_lossy(&w.word)),
        None => format!("k{k}"),
    }
}

fn kind_names(p: &BlockProgram, ks: &[u16]) -> String {
    ks.iter().map(|&k| kind_name(p, k)).collect::<Vec<_>>().join(" ")
}

/// `m` holds only bits of `s`: it is `s`, or an `and` of something with it.
fn within_mask(p: &BlockProgram, m: u16, s: u16) -> bool {
    m == s
        || match p.ops[usize::from(m)] {
            BlockOp::And(a, b) => within_mask(p, a, s) || within_mask(p, b, s),
            BlockOp::AndNot(a, _) => within_mask(p, a, s),
            _ => false,
        }
}

fn analyse_for(p: &BlockProgram, rule: Rule, target: Target) -> Result<Dataflow, String> {
    // --- Tier 1 ---
    let n = p.ops.len();
    let carried = |op: &BlockOp| matches!(op, BlockOp::Toggle(_) | BlockOp::Escaped(_) | BlockOp::Fill(..));
    let mut given = vec![false; n];
    for (i, op) in p.ops.iter().enumerate() {
        given[i] = carried(op) || op.operands().iter().any(|&a| given[usize::from(a)]);
    }
    // A `prev` of a value S1 decides, other than a carry whose top bit is its carry: a halo.
    let halo = |i: usize| match p.ops[i] {
        BlockOp::Prev(a, _) => given[usize::from(a)] && !matches!(p.ops[usize::from(a)], BlockOp::Toggle(_) | BlockOp::Fill(..)),
        _ => false,
    };
    let state_bits = p.ops.iter().filter(|op| carried(op)).count();
    if state_bits > 2 {
        return Err(format!("{state_bits} tier-1 carries: more than four candidates"));
    }
    let state_bits = state_bits as u8;
    let halo_bits: u8 = (0..n).filter(|&i| halo(i)).map(|i| if let BlockOp::Prev(_, k) = p.ops[i] { k } else { 0 }).sum();
    let ncarried = p.ops.iter().filter(|op| carried(op)).count();
    let nhalos = (0..n).filter(|&i| halo(i)).count();
    let nlocal = (0..n).filter(|&i| !given[i]).count();
    let tier1 = [n, nlocal, n - nlocal - ncarried - nhalos, ncarried, nhalos];
    let depth1 = |v: u16| u8::from(given[usize::from(v)]);

    let mut values: Vec<Value> = Vec::new();
    let value = |name: String, class, monoid: Option<Monoid>, depth: u8, scan: &str| Value { name, class, monoid, depth, scan: scan.to_owned(), wide: false, edges: Vec::new() };
    for (i, op) in p.ops.iter().enumerate() {
        let m = match op {
            BlockOp::Toggle(_) => Monoid::Xor,
            BlockOp::Escaped(_) | BlockOp::Fill(..) => Monoid::Borrow,
            BlockOp::Prev(_, k) if halo(i) => Monoid::HaloBits(*k),
            _ => continue,
        };
        let class = if matches!(m, Monoid::HaloBits(_)) { Class::Halo } else { Class::Forward };
        // A carry by its mnemonic; a halo by the value it reads.
        let text = op.text();
        let name = if class == Class::Halo { text.split(',').next().unwrap_or_default() } else { text.split(' ').next().unwrap_or_default() };
        values.push(value(format!("%{i} {name}"), class, Some(m), 1, "S1"));
    }

    // --- Tier 2: the tokens and their kinds ---
    let leaves: Vec<usize> = (0..p.nodes.len()).filter(|&r| p.nodes[r].leaf).collect();
    let start_depth = |s: RowStart| match s {
        RowStart::At(v) => depth1(v),
        _ => 0,
    };
    let tokens = leaves.iter().map(|&r| start_depth(p.nodes[r].start)).max().unwrap_or(0);
    values.push(value("tokens".into(), Class::Local, None, tokens, &format!("rows {}", leaves.iter().map(|r| r.to_string()).collect::<Vec<_>>().join(" "))));
    let one: Vec<String> = p.keywords.iter().filter(|k| k.word.len() == 1).map(|k| format!("'{}'", k.word[0] as char)).collect();
    let text = p.keywords.iter().filter(|k| k.word.len() > 1).count();
    values.push(value("kinds".into(), Class::Local, None, tokens, &format!("byte {}; text {text}", one.join(" "))));
    let scanned = tokens + 1;

    // The rows' conditions, under the rule. A row's start reads these values.
    let cond = |r: usize| -> RowCond {
        let row = &p.nodes[r];
        match (&row.also, rule) {
            (Some(c), Rule::Also) => c.clone(),
            _ => RowCond { after: row.after.clone(), before: row.before.clone(), parent: row.parent },
        }
    };
    let mut start_reads: Vec<Vec<String>> = vec![Vec::new(); p.nodes.len()];
    let mut start_wide = vec![false; p.nodes.len()];
    for r in 0..p.nodes.len() {
        let c = cond(r);
        if !c.before.is_empty() {
            let name = format!("r{r} before {}", kind_names(p, &c.before));
            values.push(value(name, Class::Backward, Some(Monoid::Last(1)), scanned, ""));
            // The bit, carried back from the first child's end to the row's start.
            values.push(value(format!("r{r} key"), Class::Backward, Some(Monoid::Last(1)), scanned, ""));
            start_reads[r].push(format!("r{r} key"));
        }
        if !c.after.is_empty() {
            let name = format!("r{r} after {}", kind_names(p, &c.after));
            values.push(value(name.clone(), Class::Forward, Some(Monoid::Last(1)), scanned, ""));
            start_reads[r].push(name);
        }
        if let Some(k) = c.parent {
            start_wide[r] = true;
            let mut v = value(format!("r{r} parent {}", kind_name(p, k)), Class::Local, None, 0, "given the stack");
            v.wide = true;
            values.push(v);
        }
    }
    // The code facts' `after`, once per set of kinds.
    let mut afters: Vec<Vec<u16>> = Vec::new();
    for c in &p.codes {
        if let Fact::After(ks) = &c.when {
            if !afters.contains(ks) {
                afters.push(ks.clone());
                values.push(value(format!("after {}", kind_names(p, ks)), Class::Forward, Some(Monoid::Last(1)), scanned, ""));
            }
        }
    }
    let first = p.nodes.iter().any(|r| r.start == RowStart::FirstToken);
    if first {
        values.push(value("first token".into(), Class::Forward, Some(Monoid::Or), scanned, ""));
        for r in 0..p.nodes.len() {
            if p.nodes[r].start == RowStart::FirstToken {
                start_reads[r].push("first token".into());
            }
        }
    }
    // An offset in the entry; stored with its present bit.
    let entry_bits = 32 - (ENTRY_BYTES - 1).leading_zeros() as u8;
    values.push(value("last token start".into(), Class::Forward, Some(Monoid::Last(entry_bits)), scanned, ""));

    // The stack: the nested rows' opens and closes, and the pair bit of a row that ends with a
    // child (its start opens it). Its depth: one more than its events'.
    let nested: Vec<usize> = (0..p.nodes.len()).filter(|&r| p.nodes[r].nested.is_some()).collect();
    let with: Vec<usize> = (0..p.nodes.len()).filter(|&r| matches!(p.nodes[r].end, RowEnd::With(_))).collect();
    let find = |values: &[Value], name: &str| values.iter().find(|v| v.name == name).map_or(0, |v| v.depth);
    let mut stack_depth = 0;
    if !nested.is_empty() {
        let max = nested.iter().filter_map(|&r| p.nodes[r].nested).min().unwrap_or(63);
        let mut events = tokens;
        let mut edges = Vec::new();
        for &r in &with {
            events = events.max(start_depth(p.nodes[r].start));
            for name in &start_reads[r] {
                events = events.max(find(&values, name));
                edges.push(name.clone());
            }
        }
        stack_depth = events + 1;
        let kinds = (usize::BITS - (nested.len().max(2) - 1).leading_zeros()) as u8;
        let mut v = value("stack".into(), Class::Forward, Some(Monoid::Dyck { kinds: kinds.max(1) * 2, max, pair: !with.is_empty() }), stack_depth, "");
        v.edges = edges;
        values.push(v);
    }
    for v in values.iter_mut() {
        if v.wide {
            v.depth = stack_depth;
        }
    }

    // Nodes per byte: every row's start; the most that start at one byte, from which leaf rows'
    // tokens each row's starts lie in.
    let mut count_depth = tokens;
    let mut count_wide = false;
    let mut count_edges: Vec<String> = Vec::new();
    for r in 0..p.nodes.len() {
        count_depth = count_depth.max(start_depth(p.nodes[r].start));
        for name in &start_reads[r] {
            let d = find(&values, name);
            count_depth = count_depth.max(d);
            if !count_edges.contains(name) {
                count_edges.push(name.clone());
            }
        }
        if start_wide[r] {
            count_wide = true;
            count_depth = count_depth.max(stack_depth);
        }
    }
    let at_a_byte = leaves
        .iter()
        .map(|&l| {
            let leaf_kinds: Vec<u16> = p.keywords.iter().filter(|k| usize::from(k.node) == l).map(|k| k.kind).collect();
            let RowStart::At(s) = p.nodes[l].start else { return 1 };
            1 + p
                .nodes
                .iter()
                .filter(|row| !row.leaf)
                .filter(|row| match row.start {
                    RowStart::FirstToken => true,
                    RowStart::Kind(k) => leaf_kinds.contains(&k),
                    RowStart::At(m) => within_mask(p, m, s),
                })
                .count()
        })
        .max()
        .unwrap_or(1) as u8;
    let planes = 8 - at_a_byte.leading_zeros() as u8;
    values.push(value("nodes per byte".into(), Class::Local, None, count_depth, &format!("{at_a_byte} at most: {planes} planes")));
    // Up to `at_a_byte` per byte of an entry, 0 included.
    let node_bits = 32 - (ENTRY_BYTES * u32::from(at_a_byte)).leading_zeros() as u8;
    let ord_depth = count_depth + 1;
    let mut ord = value("ord".into(), Class::Forward, Some(Monoid::Add(node_bits)), ord_depth, "");
    ord.edges = count_edges;
    if halo_bits > 0 {
        ord.edges.push("halos".into());
    }
    values.push(ord);
    // Matching closes to opens: a `last` per level of each row that a close ends.
    let levels: u32 = p
        .nodes
        .iter()
        .filter(|row| !row.leaf && row.start != RowStart::FirstToken)
        .map(|row| match (row.nested, &row.end) {
            (Some(_), _) => 0,
            (None, RowEnd::With(_)) => u32::from(nested.iter().filter_map(|&r| p.nodes[r].nested).min().unwrap_or(1)),
            _ => 1,
        })
        .sum::<u32>()
        + nested.iter().filter_map(|&r| p.nodes[r].nested).min().map_or(0, u32::from);
    values.push(value("open ords".into(), Class::Forward, Some(Monoid::PerLevel { bits: 32, levels: levels as u8 }), ord_depth.max(stack_depth) + 1, "pass B"));

    // --- Scans by depth (Metal) ---
    let mut scans: Vec<Scan> = Vec::new();
    let s1 = Scan { name: "S1".into(), depth: 1, forward: vec![Monoid::Fn { inputs: state_bits, outputs: state_bits + halo_bits }], backward: Vec::new(), cand: false, fixes: Vec::new() };
    scans.push(s1);
    let top = values.iter().filter(|v| v.monoid.is_some() && v.scan != "pass B" && v.depth >= 2).map(|v| v.depth).max().unwrap_or(1);
    for d in 2..=top {
        let at: Vec<&Value> = values.iter().filter(|v| v.depth == d && v.monoid.is_some() && v.scan != "pass B").collect();
        if at.is_empty() {
            continue;
        }
        let name = format!("S{}", scans.len() + 1);
        let mut fixes: Vec<String> = Vec::new();
        for v in &at {
            for e in &v.edges {
                if !fixes.contains(e) && (e == "halos" || find(&values, e) < d) {
                    fixes.push(e.clone());
                }
            }
        }
        // Fixes in the order the values are listed, halos last.
        let mut ordered: Vec<String> = values.iter().map(|v| v.name.clone()).filter(|n| fixes.contains(n)).collect();
        if fixes.iter().any(|f| f == "halos") {
            ordered.push("halos".into());
        }
        scans.push(Scan {
            name,
            depth: d,
            forward: at.iter().filter(|v| v.class == Class::Forward).filter_map(|v| v.monoid.clone()).collect(),
            backward: at.iter().filter(|v| v.class == Class::Backward).filter_map(|v| v.monoid.clone()).collect(),
            cand: state_bits > 0,
            fixes: ordered,
        });
    }
    // Each value's scan, by its depth.
    for v in values.iter_mut() {
        if v.scan.is_empty() {
            v.scan = scans.iter().find(|s| s.depth == v.depth).map_or(String::new(), |s| s.name.clone());
        }
    }
    let byte_passes = match target {
        Target::Cpu => 1,
        Target::Gpu => 2 + u8::from(count_wide),
    };
    Ok(Dataflow { target, rule, tier1, values, scans, state_bits, byte_passes, nodes_at_a_byte: at_a_byte })
}

impl Dataflow {
    /// The dataflow as text, as the tests pin it.
    pub(in crate::code_rule_dsl) fn text(&self) -> String {
        let mut out = String::new();
        let target = match self.target {
            Target::Gpu => "gpu",
            Target::Cpu => "cpu",
        };
        let rule = match self.rule {
            Rule::When => "when",
            Rule::Also => "also",
        };
        let _ = writeln!(out, "dataflow {target}, rule {rule}");
        let [all, local, given, carried, halos] = self.tier1;
        let _ = writeln!(out, "  tier 1: {all} ops: {local} local, {given} given S1, {carried} carried, {halos} halos");
        let _ = writeln!(out, "  {:<22}{:<10}{:<21}{:<7}scan", "value", "class", "monoid", "depth");
        for v in &self.values {
            let class = match v.class {
                Class::Local => "local",
                Class::Forward => "forward",
                Class::Backward => "backward",
                Class::Halo => "halo",
            };
            let monoid = v.monoid.as_ref().map(Monoid::text).unwrap_or_default();
            let _ = writeln!(out, "  {:<22}{:<10}{:<21}{:<7}{}", v.name, class, monoid, v.depth, v.scan);
        }
        if self.target == Target::Gpu {
            let _ = writeln!(out, "  scans");
            for s in &self.scans {
                let list = |ms: &[Monoid]| ms.iter().map(Monoid::scan_text).collect::<Vec<_>>().join(", ");
                let mut parts = Vec::new();
                if !s.backward.is_empty() {
                    parts.push(format!("backward {}", list(&s.backward)));
                }
                if !s.forward.is_empty() {
                    parts.push(format!("forward {}", list(&s.forward)));
                }
                let mut line = format!("  {}  depth {}  {}", s.name, s.depth, parts.join("; "));
                if s.cand {
                    line.push_str("  cand S1");
                }
                if !s.fixes.is_empty() {
                    let _ = write!(line, ", fix {}", s.fixes.join(", "));
                }
                let _ = writeln!(out, "{line}");
            }
            let _ = writeln!(out, "  candidates {}, from S1's {} state bits", 1u32 << self.state_bits, self.state_bits);
        }
        let _ = writeln!(out, "  byte passes {}", self.byte_passes);
        out
    }
}

// --- The plan (docs/parser-generators/17-tier-2-tdd-plan.md §0, §4) ------------------------

/// Where a store lives: a barrier on threadgroup memory orders only it; one on device memory
/// orders both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum Mem {
    Threadgroup,
    Device,
}

/// Memory the steps share. Only threadgroup stores count against the budget.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct Store {
    pub(super) name: String,
    pub(super) mem: Mem,
    /// Threadgroup bytes, both buffers if `double`.
    pub(super) bytes: u32,
    /// Two buffers, which writes take in turn: a write waits only for a barrier after the reads of
    /// the buffer it takes. The generated code keeps the same parity.
    pub(super) double: bool,
    pub(super) note: String,
}

/// A step of a plan. Every thread of the threadgroup runs every step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum Step {
    /// A loop over tiles or rounds. `uniform`: every thread runs the same iterations.
    Loop { what: String, uniform: bool, by: String, body: Vec<Step> },
    /// Work, with the stores it reads and writes; `simd`: it uses SIMD-group operations.
    Work { what: String, simd: bool, reads: Vec<String>, writes: Vec<String> },
    Barrier(Mem),
}

/// A field of a candidate's words: bits `at..at + bits`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct Field {
    pub(super) name: String,
    pub(super) at: u32,
    pub(super) bits: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum Matching {
    Barrier,
    Scan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct Plan {
    pub(super) target: Target,
    pub(super) rule: Rule,
    pub(super) matching: Matching,
    pub(super) flow: Dataflow,
    pub(super) candidates: u32,
    /// S1's word: its table's bits and the identity's.
    pub(super) s1_bits: u32,
    pub(super) fields: Vec<Field>,
    pub(super) cand_words: u32,
    pub(super) stores: Vec<Store>,
    pub(super) steps: Vec<Step>,
    pub(super) budget: u32,
}

/// Threads per threadgroup, lanes per SIMD group, bytes per lane.
pub(super) const THREADS: u32 = 1024;
pub(super) const SIMD: u32 = 32;
pub(super) const LANE: u32 = 64;
/// The threadgroup memory a plan may use: well under 32 KB, so several threadgroups fit a core
/// and hide memory latency.
pub(super) const BUDGET: u32 = 10240;
/// The most words of each entry staged at once, before pass A writes them out.
const STAGE_WORDS: u32 = 16;
/// The most bytes a one-step exchange of every candidate may take, both buffers.
const EXCHANGE_BYTES: u32 = 4096;

/// A target's plan.
pub(in crate::code_rule_dsl) fn plan(p: &BlockProgram, target: Target) -> Result<Plan, String> {
    let flow = analyse(p, target)?;
    let matching = if p.nodes.iter().any(|r| r.match_scan) { Matching::Scan } else { Matching::Barrier };
    if target == Target::Cpu {
        let steps = vec![Step::Work {
            what: "pass over the bytes, one thread: tier 1, then tier 2, every scan a running carry".into(),
            simd: false,
            reads: vec!["bytes".into()],
            writes: vec!["tape".into()],
        }];
        let stores = vec![device("bytes"), device("tape")];
        return Ok(Plan { target, rule: flow.rule, matching, flow, candidates: 1, s1_bits: 0, fields: Vec::new(), cand_words: 0, stores, steps, budget: BUDGET });
    }
    let candidates = 1u32 << flow.state_bits;
    let s1 = flow.scans.first().and_then(|s| s.forward.first().cloned());
    let s1_bits = match s1 {
        Some(Monoid::Fn { inputs, outputs }) => (1u32 << inputs) * u32::from(outputs) + 1,
        _ => 0,
    };
    // A candidate's fields: the later scans' values, in the dataflow's order; then the fixes and
    // the error; then, matching by a scan, the open ords. A field wider than a word starts at
    // one; no narrower field crosses one.
    let mut wanted: Vec<(String, u32)> = Vec::new();
    for v in &flow.values {
        if v.scan == "S1" || v.scan == "pass B" {
            continue;
        }
        let Some(m) = &v.monoid else { continue };
        let bits = match m {
            Monoid::Last(b) => u32::from(*b) + 1,
            Monoid::Or => 1,
            Monoid::Add(b) => u32::from(*b),
            Monoid::Dyck { .. } => 41,
            _ => continue,
        };
        wanted.push((v.name.clone(), bits));
    }
    let fixes = fix_bits(&flow);
    if fixes > 0 {
        wanted.push(("fix".into(), fixes));
    }
    wanted.push(("error".into(), 1));
    let levels = flow.values.iter().find_map(|v| match v.monoid {
        Some(Monoid::PerLevel { levels, .. }) => Some(u32::from(levels)),
        _ => None,
    });
    if matching == Matching::Scan {
        wanted.push(("open ords".into(), 32 * levels.unwrap_or(0)));
    }
    let mut fields = Vec::new();
    let mut at = 0u32;
    for (name, bits) in wanted {
        if bits > 32 || at / 32 != (at + bits - 1) / 32 {
            at = at.div_ceil(32) * 32;
        }
        fields.push(Field { name, at, bits });
        at += bits;
    }
    let cand_words = at.div_ceil(32);
    let entry_words = 1 + candidates * cand_words;
    let levels = levels.unwrap_or(0);

    // Threadgroup stores: staging and the exchange in two buffers, the opens in one; the largest
    // made single while over the budget.
    let stage = entry_words.min(STAGE_WORDS);
    let slices = entry_words.div_ceil(stage);
    let one_step = 2 * SIMD * entry_words * 4 <= EXCHANGE_BYTES;
    let mut tg: Vec<(String, u32, bool, String)> = vec![("staging".into(), SIMD * stage * 4, true, format!("32 entries x {stage} words"))];
    if one_step {
        tg.push(("exchange".into(), SIMD * entry_words * 4, true, format!("32 SIMD totals x {entry_words} words")));
    } else {
        tg.push(("exchange s1".into(), SIMD * 4, true, "32 SIMD totals x 1 word".into()));
        tg.push(("exchange".into(), SIMD * cand_words * 4, true, format!("32 SIMD totals x {cand_words} words, one candidate")));
    }
    if matching == Matching::Barrier {
        tg.push(("opens".into(), SIMD * levels * 4, false, format!("32 SIMD groups x {levels} levels x 4 bytes")));
    }
    let total = |tg: &[(String, u32, bool, String)]| tg.iter().map(|t| t.1 * if t.2 { 2 } else { 1 }).sum::<u32>();
    while total(&tg) > BUDGET {
        let Some(big) = (0..tg.len()).filter(|&i| tg[i].2).max_by_key(|&i| tg[i].1) else { break };
        tg[big].2 = false;
    }
    let mut stores = vec![device("bytes"), device("entries"), device("prefixes"), device("tape")];
    for (name, bytes, double, note) in tg {
        let n = if double { 2 } else { 1 };
        stores.push(Store { name, mem: Mem::Threadgroup, bytes: bytes * n, double, note: if double { format!("2 x {note}") } else { note } });
    }

    let w = |what: &str, simd: bool, reads: &[&str], writes: &[&str]| Step::Work {
        what: what.to_owned(),
        simd,
        reads: reads.iter().map(|s| (*s).to_owned()).collect(),
        writes: writes.iter().map(|s| (*s).to_owned()).collect(),
    };
    let tile = format!("tiles of {} bytes", THREADS * LANE);
    let x = format!("x{candidates}");
    let scans = flow.scans.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
    let later = scans[1..].join(", ");
    let all = scans.join(", ");
    // Pass A: summaries, staged, then written out as each array's 32 words.
    let mut a = vec![w(&format!("lanes: tier 1 {x}, summaries {x}"), false, &["bytes"], &[])];
    for k in 0..slices {
        let what = if k == 0 { format!("simd: S1, halos, summaries {x}") } else { format!("stage words {}..{}", k * stage, ((k + 1) * stage).min(entry_words)) };
        a.push(w(&what, true, &[], &["staging"]));
        a.push(Step::Barrier(Mem::Threadgroup));
        let words = stage.min(entry_words - k * stage);
        a.push(w(&format!("flush: {words} arrays x 32 words"), false, &["staging"], &["entries"]));
    }
    let scan = if one_step {
        vec![
            w(&format!("simd: {all} over 32 entries"), true, &["entries"], &["exchange"]),
            Step::Barrier(Mem::Threadgroup),
            w(&format!("simd: {all} over 32 totals"), true, &["exchange"], &["prefixes"]),
        ]
    } else {
        vec![
            w("simd: S1 over 32 entries", true, &["entries"], &["exchange s1"]),
            Step::Barrier(Mem::Threadgroup),
            w(&format!("simd: S1 over 32 totals; {later} over 32 entries, picked"), true, &["exchange s1", "entries"], &["exchange"]),
            Step::Barrier(Mem::Threadgroup),
            w(&format!("simd: {later} over 32 totals"), true, &["exchange"], &["prefixes"]),
        ]
    };
    let planes = 8 - flow.nodes_at_a_byte.leading_zeros();
    let mut b = vec![
        w(&format!("B1 lanes: tier 1, tier-2 masks, {planes} planes"), true, &["bytes", "prefixes"], &[]),
        w("B2 outputs: the nodes spread over each SIMD group", true, &[], &["tape"]),
    ];
    match matching {
        Matching::Barrier => {
            b.push(w("B3 lanes: opens and closes in order", true, &[], &["tape", "opens"]));
            b.push(Step::Barrier(Mem::Threadgroup));
            b.push(w("B3: closes of earlier lanes' opens", true, &["opens"], &["tape"]));
        }
        Matching::Scan => b.push(w("B3 lanes: opens and closes in order, the opens from the scans", true, &[], &["tape"])),
    }
    let c = vec![w("off, narrowing, codes", false, &["tape"], &["tape"])];
    let mut plan = Plan { target, rule: flow.rule, matching, flow, candidates, s1_bits, fields, cand_words, stores, steps: Vec::new(), budget: BUDGET };
    let rounds = format!("rounds of {THREADS}");
    let mut steps = Vec::new();
    for (what, by, body) in [("pass A over the bytes", tile.clone(), a), ("scan over the entries", rounds.clone(), scan), ("pass B over the bytes", tile, b), ("pass C over the tape", rounds, c)] {
        let mut lp = Step::Loop { what: what.into(), uniform: true, by, body };
        // A barrier at the body's end, if a store is rewritten while another thread may still
        // read it from the iteration before.
        if plan.check(std::slice::from_ref(&lp)).is_err() {
            if let Step::Loop { body, .. } = &mut lp {
                body.push(Step::Barrier(Mem::Threadgroup));
            }
        }
        if !steps.is_empty() {
            steps.push(Step::Barrier(Mem::Device));
        }
        steps.push(lp);
    }
    plan.steps = steps;
    Ok(plan)
}

fn device(name: &str) -> Store {
    Store { name: name.into(), mem: Mem::Device, bytes: 0, double: false, note: String::new() }
}

/// The fixes' bits: one per edge bit other than the first token's (which the `or` field gives),
/// the halos, and with halos a second copy of each backward field that reads the tokens.
fn fix_bits(flow: &Dataflow) -> u32 {
    let mut bits = 0;
    let mut halos = false;
    for s in &flow.scans[1..] {
        for f in &s.fixes {
            match f.as_str() {
                "first token" => {}
                "halos" => halos = true,
                _ => bits += 1,
            }
        }
    }
    if halos {
        let halo_bits: u32 = flow.values.iter().filter_map(|v| if let Some(Monoid::HaloBits(k)) = v.monoid { Some(u32::from(k)) } else { None }).sum();
        bits += halo_bits;
        // `before` reads the next token: a token at bit 0 that a halo cancels changes it.
        bits += flow.values.iter().filter(|v| v.class == Class::Backward && v.name.contains(" before ")).map(|_| 2).sum::<u32>();
    }
    bits
}

impl Plan {
    pub(in crate::code_rule_dsl) fn entry_words(&self) -> u32 {
        if self.target == Target::Cpu { 0 } else { 1 + self.candidates * self.cand_words }
    }

    pub(in crate::code_rule_dsl) fn threadgroup_bytes(&self) -> u32 {
        self.stores.iter().filter(|s| s.mem == Mem::Threadgroup).map(|s| s.bytes).sum()
    }

    fn store(&self, name: &str) -> Option<&Store> {
        self.stores.iter().find(|s| s.name == name)
    }

    /// The plan's barriers, in order.
    pub(in crate::code_rule_dsl) fn barriers(&self) -> usize {
        fn count(steps: &[Step]) -> usize {
            steps
                .iter()
                .map(|s| match s {
                    Step::Barrier(_) => 1,
                    Step::Loop { body, .. } => count(body),
                    Step::Work { .. } => 0,
                })
                .sum()
        }
        count(&self.steps)
    }

    /// The plan with its `k`th barrier removed: for the verifier's tests.
    pub(in crate::code_rule_dsl) fn without_barrier(&self, k: usize) -> Plan {
        fn cut(steps: &mut Vec<Step>, k: &mut usize) -> bool {
            let mut i = 0;
            while i < steps.len() {
                match &mut steps[i] {
                    Step::Barrier(_) if *k == 0 => {
                        steps.remove(i);
                        return true;
                    }
                    Step::Barrier(_) => *k -= 1,
                    Step::Loop { body, .. } => {
                        if cut(body, k) {
                            return true;
                        }
                    }
                    Step::Work { .. } => {}
                }
                i += 1;
            }
            false
        }
        let mut p = self.clone();
        cut(&mut p.steps, &mut { k });
        p
    }

    /// The verifier: the budget; SIMD operations and barriers only in uniform loops; and every
    /// shared store read after a barrier that follows another thread's write, and rewritten after
    /// a barrier that follows another thread's read.
    pub(in crate::code_rule_dsl) fn verify(&self) -> Result<(), String> {
        if self.threadgroup_bytes() > self.budget {
            return Err(format!("{} bytes of threadgroup memory, over the budget of {}", self.threadgroup_bytes(), self.budget));
        }
        self.check(&self.steps)
    }

    fn check(&self, steps: &[Step]) -> Result<(), String> {
        fn ragged(steps: &[Step]) -> bool {
            steps.iter().any(|s| match s {
                Step::Barrier(_) => true,
                Step::Work { simd, .. } => *simd,
                Step::Loop { body, .. } => ragged(body),
            })
        }
        fn uniform(steps: &[Step]) -> Result<(), String> {
            for s in steps {
                if let Step::Loop { what, uniform: u, body, .. } = s {
                    if !*u && ragged(body) {
                        return Err(format!("{what}: a SIMD operation or a barrier in a loop that is not uniform"));
                    }
                    uniform(body)?;
                }
            }
            Ok(())
        }
        uniform(steps)?;
        let mut state = Hazards::default();
        self.walk(steps, &mut state)
    }

    /// Walks the steps, each loop's body three times, so that a double store's two buffers both
    /// meet the iteration before.
    fn walk(&self, steps: &[Step], h: &mut Hazards) -> Result<(), String> {
        for s in steps {
            match s {
                Step::Barrier(m) => h.barrier(*m, |name| self.store(name).map_or(Mem::Device, |s| s.mem)),
                Step::Loop { body, .. } => {
                    for _ in 0..3 {
                        self.walk(body, h)?;
                    }
                }
                Step::Work { what, reads, writes, .. } => {
                    for r in reads {
                        if let Some(w) = h.written.get(r) {
                            if w != what {
                                return Err(format!("`{what}` reads {r}, written by `{w}`, with no barrier between"));
                            }
                        }
                        let buffer = h.buffer(r, false);
                        h.read.insert((r.clone(), buffer), what.clone());
                    }
                    for x in writes {
                        let double = self.store(x).is_some_and(|s| s.double);
                        let buffer = h.buffer(x, double);
                        if let Some(r) = h.read.get(&(x.clone(), buffer)) {
                            if r != what {
                                return Err(format!("`{what}` rewrites {x} while `{r}` may still read it: no barrier between"));
                            }
                        }
                        h.written.insert(x.clone(), what.clone());
                    }
                }
            }
        }
        Ok(())
    }


    pub(in crate::code_rule_dsl) fn text(&self) -> String {
        let mut out = String::new();
        if self.target == Target::Cpu {
            let rule = if self.rule == Rule::When { "when" } else { "also" };
            let _ = writeln!(out, "plan cpu, rule {rule}");
        } else {
            let matching = if self.matching == Matching::Barrier { "barrier" } else { "scan" };
            let _ = writeln!(out, "plan gpu, match {matching}");
            let _ = writeln!(out, "  threads {THREADS}, simd {SIMD}, lane {LANE} bytes, entry {ENTRY_BYTES} bytes, tile {} bytes", THREADS * LANE);
            let words = self.entry_words();
            let _ = writeln!(out, "  entry: {words} words, {} bytes, in device memory as {words} arrays, candidate-major", words * 4);
            if let Some(Monoid::Fn { inputs, outputs }) = self.flow.scans.first().and_then(|s| s.forward.first()) {
                let _ = writeln!(out, "    word 0: s1: fn {inputs}->{outputs} ({} bits), identity 1", self.s1_bits - 1);
            }
            let ranges: Vec<String> = (0..self.candidates).map(|c| format!("{}-{}", 1 + c * self.cand_words, (c + 1) * self.cand_words)).collect();
            let _ = writeln!(out, "    candidate words {}:", ranges.join(", "));
            let fields: Vec<String> = self.fields.iter().map(|f| format!("{} {}..{}", f.name, f.at, f.at + f.bits)).collect();
            let _ = writeln!(out, "      {}", fields.join(", "));
            let _ = writeln!(out, "  threadgroup memory: {} of {} bytes", self.threadgroup_bytes(), self.budget);
            for s in self.stores.iter().filter(|s| s.mem == Mem::Threadgroup) {
                let _ = writeln!(out, "    {} {}: {}", s.name, s.bytes, s.note);
            }
        }
        let _ = writeln!(out, "  steps");
        fn print_steps(out: &mut String, steps: &[Step], indent: usize) {
            let pad = " ".repeat(indent);
            for s in steps {
                match s {
                    Step::Loop { what, uniform, by, body } => {
                        let _ = writeln!(out, "{pad}{what}, {}, by {by}", if *uniform { "uniform" } else { "ragged" });
                        print_steps(out, body, indent + 2);
                    }
                    Step::Work { what, reads, writes, .. } => {
                        let mut line = format!("{pad}{what}");
                        if !reads.is_empty() {
                            let _ = write!(line, "; reads {}", reads.join(", "));
                        }
                        if !writes.is_empty() {
                            let _ = write!(line, "; writes {}", writes.join(", "));
                        }
                        let _ = writeln!(out, "{line}");
                    }
                    Step::Barrier(m) => {
                        let _ = writeln!(out, "{pad}barrier {}", if *m == Mem::Threadgroup { "threadgroup" } else { "device" });
                    }
                }
            }
        }
        print_steps(&mut out, &self.steps, 4);
        if self.target == Target::Gpu {
            let per = |i: usize| match &self.steps.get(i) {
                Some(Step::Loop { body, .. }) => body.iter().filter(|s| matches!(s, Step::Barrier(_))).count(),
                _ => 0,
            };
            let between = self.steps.iter().filter(|s| matches!(s, Step::Barrier(_))).count();
            let _ = writeln!(out, "  barriers: pass A {} per tile, scan {} per round, pass B {} per tile, {between} between passes", per(0), per(2), per(4));
        }
        out
    }
}

/// The verifier's state: each store's writer and readers since its last barrier. A double
/// store's writes take its two buffers in turn, and a read reads the buffer last written; so a
/// write waits only for a barrier after the reads of the buffer it takes.
#[derive(Default)]
struct Hazards {
    written: std::collections::HashMap<String, String>,
    read: std::collections::HashMap<(String, u32), String>,
    writes: std::collections::HashMap<String, u32>,
}

impl Hazards {
    /// The buffer a write takes (`write`), or a read reads.
    fn buffer(&mut self, store: &str, write: bool) -> u32 {
        let n = self.writes.entry(store.to_owned()).or_insert(0);
        if write {
            *n += 1;
            *n % 2
        } else {
            *n % 2
        }
    }

    fn barrier(&mut self, m: Mem, mem: impl Fn(&str) -> Mem) {
        let keep = |name: &String| m == Mem::Threadgroup && mem(name) == Mem::Device;
        self.written.retain(|k, _| keep(k));
        self.read.retain(|k, _| keep(&k.0));
    }
}

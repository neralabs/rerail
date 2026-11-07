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
    let entry_bits = 32 - (ENTRY_BYTES - 1).leading_zeros() as u8 + 1;
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

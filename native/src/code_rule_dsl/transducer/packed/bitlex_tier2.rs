//! The Bitlex compiler's last pass (decision 0053): tier 2's state machine (`grammar`) lowered
//! to the transducer's words, as simdjson's stage 2 is written: per state, take a token, switch
//! on its first byte, do the arm's straight-line work, jump to the next state.
//!
//! - **A state** is `TOKEN; SWITCH Tn`. A nested level's state that may end and takes no token
//!   closes its node and returns at once.
//! - **The switch** maps each byte to an arm: the token row whose tokens can begin with it
//!   (`first_bytes`), and a keyword of the row by its bytes (a switch per byte of the keyword,
//!   then one `WORD`). Where two rows can begin with one byte, the narrower takes it and tier 1
//!   gets a guard: the other row starting with it sends the file to tree-sitter.
//! - **An arm** is its row's `CHECK` (a token with a check, not a keyword), its acts (`CLOSE`,
//!   `OPEN`, `LEAF`, `NODE_FIELD`, `NODE_SUPER`), its `PUSH` and its `GOTO`.
//! - **The end:** the root closes and accepts at the end of the stream; a nested level closes
//!   its node and returns to switch on the token again.

use super::block::{BlockOp, ClassTable};
use super::grammar::{Act, ArmRow, StateRow};
use super::stream::{ByteSwitch, REFUSE};
use std::collections::HashMap;

/// A set of bytes, bit `b` for byte `b`.
pub(super) type ByteSet = [u64; 4];

fn has(s: &ByteSet, b: u8) -> bool {
    s[usize::from(b / 64)] >> (b % 64) & 1 == 1
}

fn count(s: &ByteSet) -> u32 {
    s.iter().map(|w| w.count_ones()).sum()
}

/// A token row, as tier 2 reads it.
pub(super) struct Row {
    /// Its tokens' kind; 0 when every token is a keyword.
    pub(super) kind: u16,
    /// Its tokens are one byte (the start is the last byte).
    pub(super) one_byte: bool,
    /// The bytes a token of the row can begin with.
    pub(super) first: ByteSet,
    /// Its keywords: a token whose text is the word has the kind.
    pub(super) keywords: Vec<(Vec<u8>, u16)>,
    /// Its check (an index of the block program's checks) and the bytes a token passing it can
    /// begin with.
    pub(super) check: Option<(u16, ByteSet)>,
}

impl Row {
    /// Its tokens' ends are the stream's last bytes (`SPAN m=1`): not one byte, and not all keywords,
    /// whose words give their lengths.
    pub(super) fn takes_last(&self) -> bool {
        !self.one_byte && self.kind != 0
    }
}

/// The bytes a value's set bits can be at, at delay `d` (bit `i` is byte `i - d`): each byte for
/// which the value's operations can give a bit, with every carried value (and every byte but
/// the one asked about) free. Sound: a byte left out never has a bit. With too many free values,
/// every byte.
pub(super) fn first_bytes(ops: &[BlockOp], tables: &[ClassTable], v: u16, d: u8) -> ByteSet {
    // The free values: (operation, lag) pairs the value reads, other than the byte at lag `d`.
    let mut atoms: HashMap<(u16, u32), usize> = HashMap::new();
    let mut seen: HashMap<(u16, u32), ()> = HashMap::new();
    let mut work = vec![(v, 0u32)];
    while let Some((at, lag)) = work.pop() {
        if seen.insert((at, lag), ()).is_some() {
            continue;
        }
        match ops[usize::from(at)] {
            BlockOp::Eq(_) | BlockOp::Class { .. } if lag == u32::from(d) => {}
            BlockOp::Not(a) => work.push((a, lag)),
            BlockOp::And(a, b) | BlockOp::Or(a, b) | BlockOp::Xor(a, b) | BlockOp::AndNot(a, b) => work.extend([(a, lag), (b, lag)]),
            BlockOp::Prev(a, k) => work.push((a, lag + u32::from(k))),
            BlockOp::Const(0) | BlockOp::Const(u64::MAX) => {}
            _ => {
                let n = atoms.len();
                atoms.entry((at, lag)).or_insert(n);
            }
        }
    }
    if atoms.len() > 12 {
        return [u64::MAX; 4];
    }
    fn eval(ops: &[BlockOp], tables: &[ClassTable], at: u16, lag: u32, d: u32, b: u8, atoms: &HashMap<(u16, u32), usize>, set: u32) -> bool {
        let e = |a: u16, l: u32| eval(ops, tables, a, l, d, b, atoms, set);
        if let Some(&k) = atoms.get(&(at, lag)) {
            return set >> k & 1 == 1;
        }
        match ops[usize::from(at)] {
            BlockOp::Eq(c) => c == b,
            BlockOp::Class { table, bits } => tables[usize::from(table)].member(b, bits),
            BlockOp::Not(a) => !e(a, lag),
            BlockOp::And(a, c) => e(a, lag) && e(c, lag),
            BlockOp::Or(a, c) => e(a, lag) || e(c, lag),
            BlockOp::Xor(a, c) => e(a, lag) != e(c, lag),
            BlockOp::AndNot(a, c) => e(a, lag) && !e(c, lag),
            BlockOp::Prev(a, k) => e(a, lag + u32::from(k)),
            BlockOp::Const(c) => c != 0,
            _ => unreachable!("every other operation is free"),
        }
    }
    let mut out = [0u64; 4];
    for b in 0..=255u8 {
        if (0..1u32 << atoms.len()).any(|set| eval(ops, tables, v, 0, u32::from(d), b, &atoms, set)) {
            out[usize::from(b / 64)] |= 1 << (b % 64);
        }
    }
    out
}

/// An operand as `rir` prints it.
#[derive(Clone, Copy)]
enum Arg {
    None,
    Num(u16),
    Kind(u16),
    Field(u16),
    Str(u16),
    Table(u16),
    /// A state's entry, known once every state is laid out.
    State(u16),
}

/// Where an arm's token ends: a length, or the stream's next last byte.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Span {
    Len(u16),
    Last,
}

/// What an arm tests before its acts.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Pre {
    None,
    Word(Vec<u8>),
    Check(u16),
}

/// Tier 2 as words: their `rir` text (strings, switches, words), and the guards tier 1 adds
/// (a token row, and the bytes it must not begin with).
pub(super) struct Lowered {
    pub(super) text: String,
    pub(super) guards: Vec<(usize, ByteSet)>,
}

struct Emit<'a> {
    states: &'a [StateRow],
    arms: &'a [ArmRow],
    rows: &'a [Row],
    root: u16,
    words: Vec<(&'static str, u8, Arg)>,
    switches: Vec<ByteSwitch>,
    strings: Vec<Vec<u8>>,
    entry: Vec<u16>,
    /// Each arm's code, once per test before it.
    arm_code: HashMap<(usize, Span, Pre), u16>,
    guards: HashMap<usize, ByteSet>,
}

/// One candidate of a byte: an arm, the row its token comes from, and the keyword it is (none:
/// the row's own kind).
#[derive(Clone)]
struct Cand {
    arm: usize,
    row: usize,
    word: Option<Vec<u8>>,
}

impl Emit<'_> {
    fn pc(&self) -> u16 {
        self.words.len() as u16
    }

    fn push(&mut self, op: &'static str, mode: u8, arg: Arg) {
        self.words.push((op, mode, arg));
    }

    fn string(&mut self, s: &[u8]) -> u16 {
        match self.strings.iter().position(|x| x == s) {
            Some(i) => i as u16,
            None => {
                self.strings.push(s.to_vec());
                (self.strings.len() - 1) as u16
            }
        }
    }

    /// An arm's code: its token's end, its test, acts, push and jump.
    fn arm(&mut self, arm: usize, span: Span, pre: Pre) -> u16 {
        if let Some(&pc) = self.arm_code.get(&(arm, span, pre.clone())) {
            return pc;
        }
        let at = self.pc();
        self.arm_code.insert((arm, span, pre.clone()), at);
        match span {
            Span::Len(n) => self.push("SPAN", 0, Arg::Num(n)),
            Span::Last => self.push("SPAN", 1, Arg::None),
        }
        match &pre {
            Pre::None => {}
            Pre::Word(w) => {
                let s = self.string(w);
                self.push("WORD", 0, Arg::Str(s));
            }
            Pre::Check(c) => self.push("CHECK", 0, Arg::Num(*c)),
        }
        let a = &self.arms[arm];
        let (token, to, ret) = (a.token, a.to, a.push);
        for act in a.acts.clone() {
            let (supers, field) = match act {
                Act::Close => {
                    self.push("CLOSE", 0, Arg::None);
                    continue;
                }
                Act::Open { kind, field, supers } => {
                    self.push("OPEN", 0, Arg::Kind(kind));
                    (supers, field)
                }
                Act::Leaf { field, supers } => {
                    self.push("LEAF", 0, Arg::Kind(token));
                    (supers, field)
                }
            };
            if field != 0 {
                self.push("NODE_FIELD", 0, Arg::Field(field));
            }
            for s in supers {
                self.push("NODE_SUPER", 0, Arg::Kind(s));
            }
        }
        if let Some(p) = ret {
            self.push("PUSH", 0, Arg::State(p));
        }
        self.push("GOTO", 0, Arg::State(to));
        at
    }

    /// The target for the candidates of one byte at `depth` of their keywords.
    fn target(&mut self, b: u8, depth: usize, cands: Vec<Cand>) -> Result<u16, String> {
        let row = &self.rows[cands[0].row];
        let plain: Vec<&Cand> = cands.iter().filter(|c| c.word.is_none()).collect();
        let words: Vec<&Cand> = cands.iter().filter(|c| c.word.is_some()).collect();
        if words.is_empty() {
            let pre = row.check.map_or(Pre::None, |(c, _)| Pre::Check(c));
            let span = if row.one_byte { Span::Len(1) } else { Span::Last };
            return Ok(self.arm(plain[0].arm, span, pre));
        }
        if !plain.is_empty() {
            // A keyword and the row's own kind on one byte: the keyword's `WORD` refuses the
            // others, so the row's check must refuse every token beginning with the byte.
            if words.len() > 1 || row.check.is_none_or(|(_, first)| has(&first, b)) {
                return Err(format!("keywords sharing a byte with the row's own tokens, which the check does not refuse: not compiled yet (byte {b:#04x})"));
            }
        }
        if words.len() == 1 {
            let w = words[0].word.clone().expect("a keyword");
            // A row of keywords only: the word gives the length. Else the token ends at its last byte.
            let span = if row.takes_last() { Span::Last } else { Span::Len(w.len() as u16) };
            // The switches have read every byte of the word, and its length is its row's: no `WORD`.
            let pre = if depth + 1 >= w.len() && !row.takes_last() { Pre::None } else { Pre::Word(w) };
            return Ok(self.arm(words[0].arm, span, pre));
        }
        // Keywords sharing their bytes up to `depth`: a switch on the next.
        let next = depth + 1;
        if words.iter().any(|c| c.word.as_ref().expect("a keyword").len() <= next) {
            return Err("a keyword that is a prefix of another: not compiled yet".into());
        }
        let t = self.switches.len();
        self.switches.push(ByteSwitch::refusing(next as u8));
        let mut by: Vec<(u8, Vec<Cand>)> = Vec::new();
        for c in words {
            let x = c.word.as_ref().expect("a keyword")[next];
            match by.iter_mut().find(|e| e.0 == x) {
                Some(e) => e.1.push(c.clone()),
                None => by.push((x, vec![c.clone()])),
            }
        }
        for (x, cs) in by {
            let pc = self.target(x, next, cs)?;
            self.switches[t].targets[usize::from(x)] = pc;
        }
        let pc = self.pc();
        self.push("SWITCH", 0, Arg::Table(t as u16));
        Ok(pc)
    }

    fn state(&mut self, s: usize) -> Result<(), String> {
        let st = self.states[s].clone();
        let mine: Vec<usize> = (0..self.arms.len()).filter(|&a| usize::from(self.arms[a].from) == s).collect();
        let nested = st.level != self.root;
        self.entry[s] = self.pc();
        if nested && mine.is_empty() {
            let Some(c) = st.end else { return Err(format!("state {s} takes no token and may not end")) };
            for _ in 0..=c {
                self.push("CLOSE", 1, Arg::None);
            }
            self.push("RETURN", 0, Arg::None);
            return Ok(());
        }
        self.push("TOKEN", 0, Arg::None);
        let t = self.switches.len();
        self.switches.push(ByteSwitch::refusing(0));
        self.push("SWITCH", 0, Arg::Table(t as u16));
        // The end: the root closes and accepts; a nested level closes its node and returns to
        // switch on the token again (at the stream's end, or on a token no arm takes).
        let end = match st.end {
            None => REFUSE,
            Some(c) => {
                let pc = self.pc();
                for _ in 0..c + u8::from(nested) {
                    self.push("CLOSE", 0, Arg::None);
                }
                self.push(if nested { "RETURN" } else { "ACCEPT" }, u8::from(nested), Arg::None);
                pc
            }
        };
        self.switches[t].end = end;
        let default = if nested { end } else { REFUSE };
        // Each byte's candidates: the arms whose token can begin with it.
        let mut cands: Vec<Vec<Cand>> = vec![Vec::new(); 256];
        for &a in &mine {
            let kind = self.arms[a].token;
            for (r, row) in self.rows.iter().enumerate() {
                if row.kind == kind && row.kind != 0 {
                    for b in 0..=255u8 {
                        if has(&row.first, b) {
                            cands[usize::from(b)].push(Cand { arm: a, row: r, word: None });
                        }
                    }
                }
                for (w, k) in &row.keywords {
                    if *k == kind && has(&row.first, w[0]) {
                        cands[usize::from(w[0])].push(Cand { arm: a, row: r, word: Some(w.clone()) });
                    }
                }
            }
        }
        for b in 0..=255u8 {
            let mut cs = std::mem::take(&mut cands[usize::from(b)]);
            if cs.is_empty() {
                self.switches[t].targets[usize::from(b)] = default;
                continue;
            }
            // Two rows on one byte: the narrower takes it; the other gets a guard.
            let mut rows: Vec<usize> = cs.iter().map(|c| c.row).collect();
            rows.sort_unstable();
            rows.dedup();
            if rows.len() > 1 {
                let narrow = *rows.iter().min_by_key(|&&r| count(&self.rows[r].first)).expect("rows");
                if rows.iter().filter(|&&r| count(&self.rows[r].first) == count(&self.rows[narrow].first)).count() > 1 {
                    return Err(format!("two token rows as narrow as each other begin with byte {b:#04x}"));
                }
                for &r in rows.iter().filter(|&&r| r != narrow) {
                    let g = self.guards.entry(r).or_insert([0; 4]);
                    g[usize::from(b / 64)] |= 1 << (b % 64);
                }
                cs.retain(|c| c.row == narrow);
            }
            let pc = self.target(b, 0, cs)?;
            self.switches[t].targets[usize::from(b)] = pc;
        }
        Ok(())
    }
}

/// Tier 2's words from the state machine and the token rows.
pub(super) fn lower(states: &[StateRow], arms: &[ArmRow], rows: &[Row]) -> Result<Lowered, String> {
    let mut e = Emit {
        states,
        arms,
        rows,
        root: states.first().ok_or("no states")?.level,
        words: Vec::new(),
        switches: Vec::new(),
        strings: Vec::new(),
        entry: vec![0; states.len()],
        arm_code: HashMap::new(),
        guards: HashMap::new(),
    };
    for s in 0..states.len() {
        e.state(s)?;
    }
    // A return lands on a state's `TOKEN`.
    for a in arms {
        if let Some(p) = a.push {
            let p = usize::from(p);
            if e.words[usize::from(e.entry[p])].0 != "TOKEN" {
                return Err(format!("a call returns to state {p}, which takes no token: not compiled yet"));
            }
        }
    }
    let mut text = String::new();
    use std::fmt::Write as _;
    for (at, s) in e.strings.iter().enumerate() {
        let _ = writeln!(text, "  string {at} {}", super::rir::quote(s));
    }
    for (at, s) in e.switches.iter().enumerate() {
        super::rir::switch_rows(&mut text, at, s);
    }
    for (pc, (op, mode, arg)) in e.words.iter().enumerate() {
        let _ = write!(text, "  {pc:04} {op}");
        if *mode != 0 {
            let _ = write!(text, " m={mode}");
        }
        let _ = match *arg {
            Arg::None => write!(text, " 0"),
            Arg::Num(n) => write!(text, " {n}"),
            Arg::Kind(k) | Arg::Field(k) => write!(text, " #{k}"),
            Arg::Str(s) => write!(text, " s{s}"),
            Arg::Table(t) => write!(text, " T{t}"),
            Arg::State(s) => write!(text, " @{:04}", e.entry[usize::from(s)]),
        };
        text.push('\n');
    }
    let mut guards: Vec<(usize, ByteSet)> = e.guards.into_iter().collect();
    guards.sort_unstable();
    Ok(Lowered { text, guards })
}

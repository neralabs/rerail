//! The Bitlex compiler's last pass (decision 0087): tier 2's state machine (`grammar`) lowered
//! to the transducer's words, as simdjson's stage 2 is written: per state, take a token, switch
//! on its first byte, do the arm's straight-line work, jump to the next state.
//!
//! - **A state** is `TOKEN; SWITCH Tn`. A nested level's state that may end and takes no token
//!   closes its node and returns at once.
//! - **The switch** maps each byte to an arm: the token row whose tokens can begin with it
//!   (`first_bytes`), and a keyword of the row by its bytes (a switch per byte of the keyword,
//!   then one `WORD`). Where two rows can begin with one byte, tier 1's start mask of each tells
//!   them apart: `ROW m` takes the next word (its `GOTO`) when the token starts at a bit of mask
//!   `m` (docs/parser-generators/24-rr-and-bitlex-plan.md, A15).
//! - **An arm** is its acts (`CLOSE`, `OPEN`, `LEAF`, `NODE_FIELD`, `NODE_SUPER`), its `PUSH` and its `GOTO`.
//! - **The end:** the root closes and accepts at the end of the stream; a nested level closes
//!   its node and returns to switch on the token again.

use super::grammar::{Act, ArmRow, StateRow};
use super::stream::{ByteSwitch, REFUSE};
use std::collections::HashMap;

/// A set of bytes, bit `b` for byte `b`.
pub(super) type ByteSet = [u64; 4];

fn has(s: &ByteSet, b: u8) -> bool {
    s[usize::from(b / 64)] >> (b % 64) & 1 == 1
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
    /// Its soft keywords: a token whose text is the word has the kind where an arm takes it, and
    /// the row's own elsewhere.
    pub(super) soft: Vec<(Vec<u8>, u16)>,
    /// With a check, the bytes a token passing it can begin with. The check itself is tier 1's
    /// (masks joining the fallback); tier 2 uses the bytes only to tell rows apart.
    pub(super) check: Option<ByteSet>,
    /// Its start mask, the program's mask of that index, when another row can begin with one
    /// of its bytes: `ROW` tests it.
    pub(super) mask: Option<u16>,
}

impl Row {
    /// Its tokens' ends are the stream's last bytes (`SPAN m=1`): every row but one of one-byte
    /// tokens. A row of keywords only is no exception: its token runs to its last byte whatever
    /// the word, and `truee` is not `true` (the word's bytes are compared over the whole token).
    pub(super) fn takes_last(&self) -> bool {
        !self.one_byte
    }
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
    /// A word's place.
    Pc(u16),
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
}

/// Tier 2 as words: their `rir` text (strings, switches, words).
pub(super) struct Lowered {
    pub(super) text: String,
    /// The rows `ROW` tests, in order: those that need a start mask.
    pub(super) tested: Vec<usize>,
}

/// A mark (a token of no width) as tier 2 tests it: its kind, the mask of its bits at token
/// starts, and whether it is hidden (skipped where no arm takes it).
#[derive(Clone, Copy)]
pub(super) struct Mark {
    pub(super) kind: u16,
    pub(super) mask: u16,
    pub(super) hidden: bool,
    /// The file's end has it, after the last token.
    pub(super) end: bool,
}

struct Emit<'a> {
    states: &'a [StateRow],
    arms: &'a [ArmRow],
    rows: &'a [Row],
    marks: &'a [Mark],
    /// The rows each state can meet (`bitlex_compile::row_modes`).
    meets: &'a [Vec<bool>],
    /// A first lowering, to find the rows `ROW` tests: their masks are not given yet.
    survey: bool,
    tested: std::collections::BTreeSet<usize>,
    root: u16,
    words: Vec<(&'static str, u8, Arg)>,
    switches: Vec<ByteSwitch>,
    strings: Vec<Vec<u8>>,
    entry: Vec<u16>,
    /// Each arm's code, once per test before it.
    arm_code: HashMap<(usize, Span, Pre), u16>,
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
            Span::Last => self.push("SPAN", 1, Arg::Num(LASTS)),
        }
        match &pre {
            Pre::None => {}
            Pre::Word(w) => {
                let s = self.string(w);
                self.push("WORD", 0, Arg::Str(s));
            }
        }
        self.body(arm);
        at
    }

    /// An arm's acts, push and jump, here.
    fn body(&mut self, arm: usize) -> u16 {
        let at = self.pc();
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
                Act::Extra => {
                    self.push("LEAF", 1, Arg::Kind(token));
                    continue;
                }
                Act::Retag { kind } => {
                    self.push("RETAG", 0, Arg::Kind(kind));
                    continue;
                }
                Act::Merge { kind } => {
                    self.push("MERGE", 0, Arg::Kind(kind));
                    continue;
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
        // The row's keywords beginning with the byte that no arm here takes: a token that is one
        // has the keyword's kind, not the row's, so the row's own arm must not take it.
        let missing: Vec<Vec<u8>> = row
            .keywords
            .iter()
            .map(|(w, _)| w.clone())
            .filter(|w| depth == 0 && w[0] == b && !words.iter().any(|c| c.word.as_ref() == Some(w)))
            .collect();
        let check_refuses = row.check.is_some_and(|first| !has(&first, b));
        // A token of a checked row beginning with a byte its check does not take, and no keyword:
        // refused (tier 1's check masks pass such tokens to tier 2).
        if words.is_empty() && check_refuses {
            return Ok(REFUSE);
        }
        if words.is_empty() && (missing.is_empty() || plain.is_empty() || check_refuses) {
            let span = if row.one_byte { Span::Len(1) } else { Span::Last };
            return Ok(self.arm(plain[0].arm, span, Pre::None));
        }
        if !plain.is_empty() && (words.len() > 1 || !check_refuses) {
            // Keywords and the row's own kind on one byte, which the row's check does not tell
            // apart: the token's end, then a test per keyword (`WORD m=1`, a jump to the
            // keyword's arm where the token is the word), then the row's own arm.
            if row.one_byte {
                return Err(format!("one-byte tokens sharing byte {b:#04x} with keywords: not compiled yet"));
            }
            let at = self.pc();
            self.push("SPAN", 1, Arg::Num(LASTS));
            let mut jumps = Vec::new();
            for c in &words {
                let w = self.string(c.word.as_ref().expect("a keyword"));
                self.push("WORD", 1, Arg::Str(w));
                jumps.push((self.pc(), Some(c.arm)));
                self.push("GOTO", 0, Arg::Pc(0));
            }
            // A keyword no arm here takes refuses the file.
            for w in &missing {
                let w = self.string(w);
                self.push("WORD", 1, Arg::Str(w));
                jumps.push((self.pc(), None));
                self.push("GOTO", 0, Arg::Pc(0));
            }
            self.body(plain[0].arm);
            let mut stop = None;
            for (jump, arm) in jumps {
                let to = match arm {
                    Some(arm) => self.body(arm),
                    None => *stop.get_or_insert_with(|| {
                        let pc = self.pc();
                        self.push("STOP", 0, Arg::None);
                        pc
                    }),
                };
                self.words[usize::from(jump)].2 = Arg::Pc(to);
            }
            return Ok(at);
        }
        if words.len() == 1 {
            let w = words[0].word.clone().expect("a keyword");
            // A one-byte row: the word is its byte. Else the token ends at its last byte, and the
            // word is compared with all of it.
            let span = if row.takes_last() { Span::Last } else { Span::Len(w.len() as u16) };
            // The switches have read every byte of the word, and its length is its row's: no `WORD`.
            let pre = if depth + 1 >= w.len() && !row.takes_last() { Pre::None } else { Pre::Word(w) };
            return Ok(self.arm(words[0].arm, span, pre));
        }
        // Keywords sharing their bytes up to `depth`: a switch on the next.
        let next = depth + 1;
        if words.iter().any(|c| c.word.as_ref().expect("a keyword").len() <= next) {
            // One ends here, a prefix of another (`:` and `:-`). A row whose token ends at its
            // last byte knows its length from tier 1: the end, then a test per keyword (each
            // compares the whole token), then the row's own arm, or a refusal.
            if !row.takes_last() {
                return Err("a keyword that is a prefix of another, in a row of one-byte tokens".into());
            }
            let prefix = words[0].word.clone().expect("a keyword")[..next].to_vec();
            let missing: Vec<Vec<u8>> = row
                .keywords
                .iter()
                .map(|(w, _)| w.clone())
                .filter(|w| w.starts_with(&prefix) && !words.iter().any(|c| c.word.as_ref() == Some(w)))
                .collect();
            let at = self.pc();
            self.push("SPAN", 1, Arg::Num(LASTS));
            let mut jumps = Vec::new();
            for c in &words {
                let w = self.string(c.word.as_ref().expect("a keyword"));
                self.push("WORD", 1, Arg::Str(w));
                jumps.push((self.pc(), Some(c.arm)));
                self.push("GOTO", 0, Arg::Pc(0));
            }
            for w in &missing {
                let w = self.string(w);
                self.push("WORD", 1, Arg::Str(w));
                jumps.push((self.pc(), None));
                self.push("GOTO", 0, Arg::Pc(0));
            }
            let stop = match plain.first() {
                Some(p) => {
                    self.body(p.arm);
                    None
                }
                None => {
                    let pc = self.pc();
                    self.push("STOP", 0, Arg::None);
                    Some(pc)
                }
            };
            let mut stop = stop;
            for (jump, arm) in jumps {
                let to = match arm {
                    Some(arm) => self.body(arm),
                    None => *stop.get_or_insert_with(|| {
                        let pc = self.pc();
                        self.push("STOP", 0, Arg::None);
                        pc
                    }),
                };
                self.words[usize::from(jump)].2 = Arg::Pc(to);
            }
            return Ok(at);
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
        self.push("TOKEN", 0, Arg::Num(STARTS));
        // The marks, in order: each the state takes, a test and a jump to its arm; each not
        // hidden that it does not take, a test that refuses.
        let mut mark_arms = Vec::new();
        for (k, m) in self.marks.iter().enumerate() {
            let arm = mine.iter().copied().find(|&a| self.arms[a].token == m.kind);
            if arm.is_none() && m.hidden {
                continue;
            }
            // Its order, and 32 when the end has it.
            self.push("MARK", k as u8 | if m.end { 32 } else { 0 }, Arg::Num(m.mask));
            match arm {
                Some(a) => {
                    mark_arms.push((self.pc(), a));
                    self.push("GOTO", 0, Arg::Pc(0));
                }
                None => self.push("STOP", 0, Arg::None),
            }
        }
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
                for (w, k) in row.keywords.iter().chain(&row.soft) {
                    if *k == kind && has(&row.first, w[0]) {
                        cands[usize::from(w[0])].push(Cand { arm: a, row: r, word: Some(w.clone()) });
                    }
                }
            }
        }
        for b in 0..=255u8 {
            // An arm whose row the state cannot meet is dead (an extra's in a string, say).
            let mut cs = std::mem::take(&mut cands[usize::from(b)]);
            cs.retain(|c| self.meets[s][c.row]);
            if cs.is_empty() {
                self.switches[t].targets[usize::from(b)] = default;
                continue;
            }
            // The rows the state can meet that begin with the byte (`meets`): when more than the
            // one it takes, a test of each one it takes (`ROW`, its start mask) and its jump; the
            // last untested when the state takes them all. Any other goes to the default.
            let mut taken: Vec<usize> = cs.iter().map(|c| c.row).collect();
            taken.sort_unstable();
            taken.dedup();
            let met: Vec<usize> = (0..self.rows.len()).filter(|&r| has(&self.rows[r].first, b) && self.meets[s][r]).collect();
            if met.len() > 1 {
                let all = met.iter().all(|r| taken.contains(r));
                let at = self.pc();
                let mut jumps = Vec::new();
                for (i, &r) in taken.iter().enumerate() {
                    if !(all && i + 1 == taken.len()) {
                        self.tested.insert(r);
                        let m = match self.rows[r].mask {
                            Some(m) => m,
                            None if self.survey => 0,
                            None => return Err(format!("row {r} shares byte {b:#04x} and has no start mask")),
                        };
                        self.push("ROW", 0, Arg::Num(m));
                    }
                    jumps.push((self.pc(), r));
                    self.push("GOTO", 0, Arg::Pc(0));
                }
                if !all {
                    if default == REFUSE {
                        self.push("STOP", 0, Arg::None);
                    } else {
                        self.push("GOTO", 0, Arg::Pc(default));
                    }
                }
                for (jump, r) in jumps {
                    let these: Vec<Cand> = cs.iter().filter(|c| c.row == r).cloned().collect();
                    let to = self.target(b, 0, these)?;
                    self.words[usize::from(jump)] = if to == REFUSE { ("STOP", 0, Arg::None) } else { ("GOTO", 0, Arg::Pc(to)) };
                }
                self.switches[t].targets[usize::from(b)] = at;
                continue;
            }
            let pc = self.target(b, 0, cs)?;
            self.switches[t].targets[usize::from(b)] = pc;
        }
        // The marks' arms: their acts, no bytes taken (the next state switches on this token).
        for (jump, a) in mark_arms {
            let to = self.body(a);
            self.words[usize::from(jump)].2 = Arg::Pc(to);
        }
        Ok(())
    }
}

/// Tier 2's words from the state machine and the token rows.
/// Tier 1's streams the words read: the extract of every token's first byte (`TOKEN`), and the
/// mask of the last byte of each token whose arm reads it (`SPAN m=1`).
pub(super) const STARTS: u16 = 0;
pub(super) const LASTS: u16 = 0;

/// The words, after `empties` words `EMPTY t` (`t` in `0..empties`): tier 1's fallback totals,
/// each of which must be zero before the first token is taken. With a `whole` root, an `OPEN` of
/// the root follows them: no token is taken yet, so it opens at byte 0.
#[allow(clippy::too_many_arguments)]
pub(super) fn lower(states: &[StateRow], arms: &[ArmRow], rows: &[Row], marks: &[Mark], meets: &[Vec<bool>], empties: u16, whole: bool, survey: bool) -> Result<Lowered, String> {
    let mut e = Emit {
        states,
        arms,
        rows,
        marks,
        meets,
        survey,
        tested: Default::default(),
        root: states.first().ok_or("no states")?.level,
        words: Vec::new(),
        switches: Vec::new(),
        strings: Vec::new(),
        entry: vec![0; states.len()],
        arm_code: HashMap::new(),
    };
    for t in 0..empties {
        e.push("EMPTY", 0, Arg::Num(t));
    }
    if whole {
        let root = e.root;
        e.push("OPEN", 0, Arg::Kind(root));
    }
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
            Arg::Pc(p) => write!(text, " @{p:04}"),
        };
        text.push('\n');
    }
    Ok(Lowered { text, tested: e.tested.into_iter().collect() })
}

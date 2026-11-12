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

use crate::grammar::{Act, ArmRow, StateRow};
use crate::stream_ir::{ByteSwitch, REFUSE};
use std::collections::HashMap;

/// A set of bytes, bit `b` for byte `b`.
pub type ByteSet = [u64; 4];

fn has(s: &ByteSet, b: u8) -> bool {
    s[usize::from(b / 64)] >> (b % 64) & 1 == 1
}

/// A token row, as tier 2 reads it.
pub struct Row {
    /// Its tokens' kind; 0 when every token is a keyword.
    pub kind: u16,
    /// Its tokens are one byte (the start is the last byte).
    pub one_byte: bool,
    /// The bytes a token of the row can begin with.
    pub first: ByteSet,
    /// Its keywords: a token whose text is the word has the kind.
    pub keywords: Vec<(Vec<u8>, u16)>,
    /// Its soft keywords: a token whose text is the word has the kind where an arm takes it, and
    /// the row's own elsewhere.
    pub soft: Vec<(Vec<u8>, u16)>,
    /// With a check, the bytes a token passing it can begin with. The check itself is tier 1's
    /// (masks joining the fallback); tier 2 uses the bytes only to tell rows apart.
    pub check: Option<ByteSet>,
    /// Its start mask, the program's mask of that index, when another row can begin with one
    /// of its bytes: `ROW` tests it.
    pub mask: Option<u16>,
}

impl Row {
    /// Its tokens' ends are the stream's last bytes (`SPAN m=1`): every row but one of one-byte
    /// tokens. A row of keywords only is no exception: its token runs to its last byte whatever
    /// the word, and `truee` is not `true` (the word's bytes are compared over the whole token).
    pub fn takes_last(&self) -> bool {
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
    /// Where a call returns to a state: its entry, or for a state that takes no token, its
    /// return entry (`Emit::returns`).
    Return(u16),
    /// A state's `SWITCH`: a call's way into its callee's start (`ArmRow::call`), to switch on
    /// the token the caller has, where the caller's state has tested each mark the callee's does
    /// (`Emit::mark_tests`).
    Again(u16),
    /// A state's entry past its `TOKEN`: a call's way into its callee's start, where the
    /// callee's level tests a mark its caller's does not (`Arg::Again`).
    Past(u16),
    /// Where a mark's arm goes: a state's entry past its `TOKEN` (the token is not taken, so
    /// the state switches on it), or for a state that takes no token, its entry after a mark
    /// (`Emit::marked`), which closes at the last token taken (`CLOSE m=0`): a mark takes no
    /// bytes, so its token's end is no node's.
    Marked(u16),
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
pub struct Lowered {
    pub text: String,
    /// The switches and strings, as in `text`: tier 1's records are made from them (a byte's
    /// kind, the words).
    pub switches: Vec<ByteSwitch>,
    pub strings: Vec<Vec<u8>>,
    /// The rows `ROW` tests, in order: those that need a start mask.
    pub tested: Vec<usize>,
    /// A function a rule (`lower`'s `rules`): each rule's first place (`Words::rules`); else
    /// empty.
    pub rules: Vec<u16>,
    /// The marks of one word: (order, string), rising (`Words::mark_words`).
    pub mark_words: Vec<(u8, u16)>,
}

/// A mark (a token of no width) as tier 2 tests it: its kind, the mask of its bits at token
/// starts, and whether it is hidden (skipped where no arm takes it).
#[derive(Clone)]
pub struct Mark {
    pub kind: u16,
    pub mask: u16,
    pub hidden: bool,
    /// The file's end has it, after the last token.
    pub end: bool,
    /// It holds only at a token whose text is this word (`Words::mark_words`): its mask is the
    /// rest of it.
    pub word: Option<Vec<u8>>,
}

struct Emit<'a> {
    states: &'a [StateRow],
    arms: &'a [ArmRow],
    rows: &'a [Row],
    marks: &'a [Mark],
    /// The mask of any mark at a token (`TOKEN m=1+k`), when there are marks.
    any_mark: Option<u16>,
    /// The rows each state can meet (`bitlex_compile::row_modes`).
    meets: &'a [Vec<bool>],
    /// A first lowering, to find the rows `ROW` tests: their masks are not given yet.
    survey: bool,
    tested: std::collections::BTreeSet<usize>,
    root: u16,
    words: Vec<(&'static str, u8, Arg)>,
    /// Each word's rule (`StateRow::rule`): the state's it is lowered for, or a call's callee's.
    rule_of: Vec<u16>,
    /// The rule being lowered.
    rule: u16,
    /// A function a rule: the words after a call's `PUSH` are its callee's (`body`).
    by_rules: bool,
    switches: Vec<ByteSwitch>,
    strings: Vec<Vec<u8>>,
    entry: Vec<u16>,
    /// Each state's `SWITCH` (a call's way in, `Arg::Again`).
    switch_at: Vec<u16>,
    /// Each state's entry after a mark (`Arg::Marked`): its entry, but for a nested state that
    /// takes no token.
    marked: Vec<u16>,
    /// Each arm's code, once per test before it.
    arm_code: HashMap<(usize, Span, Pre), u16>,
    /// The marks of one word, by order: their strings (`Lowered::mark_words`).
    mark_words: std::collections::BTreeMap<u8, u16>,
    /// Where the state being lowered goes on a token none of its arms takes: its level's end
    /// (the caller then switches on the token), or `REFUSE`.
    default: u16,
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
        self.rule_of.push(self.rule);
    }

    /// A call's `PUSH`, and with a function a rule (`by_rules`), its `GOTO` to the words after
    /// it, which are its callee's: they run in its frame (its flags, its word, its own calls).
    /// `callee`: the state whose rule that is.
    fn call(&mut self, ret: u16, callee: u16) {
        self.push("PUSH", 0, Arg::Return(ret));
        if self.by_rules {
            let next = self.pc() + 1;
            self.push("GOTO", 0, Arg::Pc(next));
            self.rule = self.states[usize::from(callee)].rule;
        }
    }

    /// Where a mark's arm goes in state `s` (`Arg::Marked`).
    fn after_mark(&self, s: u16) -> u16 {
        let at = self.marked[usize::from(s)];
        if self.words.get(usize::from(at)).is_some_and(|w| w.0 == "TOKEN") { at + 1 } else { at }
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
        // The arm's guards before its token is taken: a guard that holds goes on as if the
        // state had no arm on the token, which is then still the next one's (`SPAN` takes a
        // token for good, so every word knows whether its token is taken).
        self.guards(arm);
        // A call takes no token (`ArmRow::call`): the callee's start switches on it.
        match span {
            _ if self.arms[arm].call => {}
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
        self.acts(arm);
        at
    }

    /// Call `i`'s flags, set or clear (`FLAG m=0, 1`), after its `PUSH`.
    fn flags_of(&mut self, flags: &[(u8, u16, bool)], i: usize) {
        for &(call, flag, set) in flags {
            if usize::from(call) == i {
                self.push("FLAG", u8::from(!set), Arg::Num(flag));
            }
        }
    }

    /// An arm that takes its token's bytes to its record's end, entered by falling to it: its
    /// code here, or a jump to where it is already.
    fn here(&mut self, arm: usize) {
        let pc = self.pc();
        let at = self.arm(arm, Span::Last, Pre::None);
        if at != pc {
            self.push("GOTO", 0, Arg::Pc(at));
        }
    }

    /// An arm's guards, acts, push and jump, here: an arm that takes no token.
    fn body(&mut self, arm: usize) -> u16 {
        let at = self.pc();
        self.guards(arm);
        self.acts(arm);
        at
    }

    /// An arm's guards (`unless F`): where the flag is set, the state goes on as if it had no
    /// arm on the token: its end, or a refusal.
    fn guards(&mut self, arm: usize) {
        for act in self.arms[arm].acts.clone() {
            if let Act::Unless { flag } = act {
                self.push("FLAG", 2, Arg::Num(flag));
                if self.default == REFUSE {
                    self.push("STOP", 0, Arg::None);
                } else {
                    self.push("GOTO", 0, Arg::Pc(self.default));
                }
            }
        }
    }

    /// An arm's acts, push and jump, here.
    fn acts(&mut self, arm: usize) {
        let a = &self.arms[arm];
        let (token, to, ret) = (a.token, a.to, a.push.clone());
        // A remembered token is the word of the level it is taken at: after the calls the arm
        // makes, which keep their caller's (`PUSH`), so the innermost level's.
        let mut remember = None;
        // Each call's flags (`with F R`), after its `PUSH`: (call, flag, set).
        let mut flags: Vec<(u8, u16, bool)> = Vec::new();
        let a = &self.arms[arm];
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
                Act::Leaf { field, supers, kind } => {
                    self.push("LEAF", 0, Arg::Kind(if kind != 0 { kind } else { token }));
                    (supers, field)
                }
                Act::Extra => {
                    self.push("LEAF", 1, Arg::Kind(token));
                    continue;
                }
                // Mode 3: a token the grammar implies, no node.
                Act::Settle => {
                    self.push("LEAF", 3, Arg::Kind(token));
                    continue;
                }
                Act::ExtraNow => {
                    self.push("LEAF", 2, Arg::Kind(token));
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
                // Mode 1: the open node's two leaves, one.
                Act::Fold { kind } => {
                    self.push("MERGE", 1, Arg::Kind(kind));
                    continue;
                }
                // Mode 3: the node made last at this level, and the nodes inside it, read.
                Act::Reread { reading } => {
                    self.push("RETAG", 3, Arg::Num(reading));
                    continue;
                }
                Act::Remember { kind } => {
                    remember = Some(kind);
                    continue;
                }
                Act::Flag { flag, set, call } => {
                    flags.push((call, flag, set));
                    continue;
                }
                Act::Unless { .. } => continue,
                Act::Relabel { field, moved, inner } => {
                    // Mode 2: the node made last at this level; mode 3: the node its wrap holds
                    // first.
                    if inner != 0 {
                        self.push("NODE_FIELD", 3, Arg::Field(inner));
                    }
                    if field != 0 {
                        self.push("NODE_FIELD", 2, Arg::Field(field));
                    }
                    if moved != 0 {
                        self.push("RETAG", 2, Arg::Kind(moved));
                    }
                    continue;
                }
                Act::Wrap { kind, field, moved } => {
                    // The wrap takes the moved node's field; then the moved node gets its own.
                    self.push("WRAP", 0, Arg::Kind(kind));
                    if field != 0 {
                        self.push("NODE_FIELD", 1, Arg::Field(field));
                    }
                    if moved != 0 {
                        self.push("RETAG", 1, Arg::Kind(moved));
                    }
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
        // The calls, outermost first. When the innermost call's level ends at its first token:
        // that level's closes, at this token's end, and on to the state it returns to; that call
        // is not made.
        let st = &self.states[usize::from(to)];
        let ends_at_once = !self.arms.iter().any(|b| b.from == to);
        // Call `i`'s callee is the rule of the call made inside it, or of the state the last
        // goes to.
        let caller = self.rule;
        match (ret.split_last(), st.end) {
            (Some((&p, outer)), Some(c)) if ends_at_once => {
                for (i, &o) in outer.iter().enumerate() {
                    let callee = outer.get(i + 1).copied().unwrap_or(p);
                    self.call(o, callee);
                    self.flags_of(&flags, i);
                }
                for _ in 0..c + u8::from(st.own) {
                    self.push("CLOSE", 1, Arg::None);
                }
                self.push("GOTO", 0, Arg::State(p));
            }
            _ => {
                for (i, &p) in ret.iter().enumerate() {
                    let callee = ret.get(i + 1).copied().unwrap_or(to);
                    self.call(p, callee);
                    self.flags_of(&flags, i);
                }
                if let Some(k) = remember {
                    self.push("REMEMBER", 0, Arg::Kind(k));
                }
                let marked = self.marks.iter().any(|m| m.kind == token);
                let from = usize::from(self.arms[arm].from);
                let again = if self.mark_tests(usize::from(to)).iter().all(|k| self.mark_tests(from).contains(k)) { Arg::Again(to) } else { Arg::Past(to) };
                self.push("GOTO", 0, if self.arms[arm].call { again } else if marked { Arg::Marked(to) } else { Arg::State(to) });
            }
        }
        self.rule = caller;
    }

    /// The target for the candidates of one byte at `depth` of their keywords.
    /// A state's switch `t` at byte `b`, where one row the state can meet begins with it: each
    /// keyword an arm takes, a target of the switch's own (`ByteSwitch::words`), which takes the
    /// token; each keyword of the row's no arm here takes, the state's end (the row reserves it).
    /// The byte's target is then the row's own arm, with no keyword tested: tier 1's record
    /// names the word, as a lexer gives a keyword its kind.
    fn by_words(&mut self, t: usize, b: u8, cands: Vec<Cand>) -> Result<u16, String> {
        let row = &self.rows[cands[0].row];
        let (one_byte, takes_last) = (row.one_byte, row.takes_last());
        let check_refuses = row.check.is_some_and(|first| !has(&first, b));
        let missing: Vec<Vec<u8>> = row.keywords.iter().map(|(w, _)| w.clone()).filter(|w| w[0] == b && !cands.iter().any(|c| c.word.as_ref() == Some(w))).collect();
        let plain = cands.iter().find(|c| c.word.is_none()).map(|c| c.arm);
        for w in missing {
            let s = self.string(&w);
            let to = self.default;
            self.name_word(t, s, to)?;
        }
        for c in cands.iter().filter(|c| c.word.is_some()) {
            let w = c.word.clone().expect("a keyword");
            let span = if takes_last { Span::Last } else { Span::Len(w.len() as u16) };
            let s = self.string(&w);
            let to = self.arm(c.arm, span, Pre::None);
            self.name_word(t, s, to)?;
        }
        // A token that is no keyword here: refused where the row's check does not take the byte
        // (tier 1 passes such tokens on), else the row's own arm, else the state's end.
        Ok(match plain {
            _ if check_refuses => REFUSE,
            Some(a) => self.arm(a, if one_byte { Span::Len(1) } else { Span::Last }, Pre::None),
            None => self.default,
        })
    }

    /// Switch `t` names word `s` with target `to`, kept in order.
    fn name_word(&mut self, t: usize, s: u16, to: u16) -> Result<(), String> {
        let words = &mut self.switches[t].words;
        match words.binary_search_by_key(&s, |&(x, _)| x) {
            Ok(i) if words[i].1 != to => Err(format!("word {s} with two targets in one switch")),
            Ok(_) => Ok(()),
            Err(i) => {
                words.insert(i, (s, to));
                Ok(())
            }
        }
    }

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
            // apart: a test per keyword (`WORD m=1`, a jump to the keyword's arm where the token
            // is the word: the record names it), then the row's own arm. Each arm takes the
            // token; one that is no arm's is left for the state's end.
            if row.one_byte {
                return Err(format!("one-byte tokens sharing byte {b:#04x} with keywords: not compiled yet"));
            }
            let at = self.pc();
            let mut jumps = Vec::new();
            for c in &words {
                let w = self.string(c.word.as_ref().expect("a keyword"));
                self.push("WORD", 1, Arg::Str(w));
                jumps.push((self.pc(), Some(c.arm)));
                self.push("GOTO", 0, Arg::Pc(0));
            }
            // A keyword no arm here takes: the state's end, or a refusal.
            for w in &missing {
                let w = self.string(w);
                self.push("WORD", 1, Arg::Str(w));
                jumps.push((self.pc(), None));
                self.push("GOTO", 0, Arg::Pc(0));
            }
            self.here(plain[0].arm);
            let mut stop = (self.default != REFUSE).then_some(self.default);
            for (jump, arm) in jumps {
                let to = match arm {
                    Some(arm) => self.arm(arm, Span::Last, Pre::None),
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
            let pre = if depth + 1 >= w.len() && !row.takes_last() { Pre::None } else { Pre::Word(w.clone()) };
            if pre == Pre::None || self.default == REFUSE {
                return Ok(self.arm(words[0].arm, span, pre));
            }
            // Where the state may end, a token that is not the word is its caller's: a test (the
            // record names the word), the arm where the token is the word, else the end, with
            // the token not taken.
            let at = self.pc();
            let s = self.string(&w);
            self.push("WORD", 1, Arg::Str(s));
            let jump = self.pc();
            self.push("GOTO", 0, Arg::Pc(0));
            self.push("GOTO", 0, Arg::Pc(self.default));
            let to = self.arm(words[0].arm, span, Pre::None);
            self.words[usize::from(jump)].2 = Arg::Pc(to);
            return Ok(at);
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
            // No word and no plain arm: the state's end, or a refusal.
            let stop = match plain.first() {
                Some(p) => {
                    self.here(p.arm);
                    (self.default != REFUSE).then_some(self.default)
                }
                None if self.default != REFUSE => {
                    let pc = self.pc();
                    self.push("GOTO", 0, Arg::Pc(self.default));
                    Some(pc)
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
                    Some(arm) => self.arm(arm, Span::Last, Pre::None),
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
        // A byte no word here has: the state's end, or a refusal.
        if self.default != REFUSE {
            self.switches[t].targets = vec![self.default; 256];
        }
        for (x, cs) in by {
            let pc = self.target(x, next, cs)?;
            self.switches[t].targets[usize::from(x)] = pc;
        }
        let pc = self.pc();
        self.push("SWITCH", 0, Arg::Table(t as u16));
        Ok(pc)
    }

    /// A state's mark tests (`state`): each a `MARK` and its arm's jump, or a refusal.
    fn mark_tests_here(&mut self, tests: &[(usize, Option<usize>)], mark_arms: &mut Vec<(u16, usize)>) {
        for &(k, arm) in tests {
            if let Some(w) = self.marks[k].word.clone() {
                let s = self.string(&w);
                self.mark_words.insert(k as u8, s);
            }
            let m = &self.marks[k];
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
    }

    /// The marks state `s` tests after its `TOKEN`, in order (`state`): each it takes, and each
    /// not hidden or that its level refuses.
    fn mark_tests(&self, s: usize) -> Vec<usize> {
        let st = &self.states[s];
        (0..self.marks.len())
            .filter(|&k| {
                let m = &self.marks[k];
                let takes = self.arms.iter().any(|a| usize::from(a.from) == s && a.token == m.kind);
                takes || !m.hidden || st.refuses.contains(&m.kind)
            })
            .collect()
    }

    fn state(&mut self, s: usize) -> Result<(), String> {
        let st = self.states[s].clone();
        self.rule = st.rule;
        let mine: Vec<usize> = (0..self.arms.len()).filter(|&a| usize::from(self.arms[a].from) == s).collect();
        let nested = st.level != self.root;
        self.entry[s] = self.pc();
        self.marked[s] = self.pc();
        if nested && mine.is_empty() {
            let Some(c) = st.end else { return Err(format!("state {s} takes no token and may not end")) };
            // A nested choice's level has no node of its own to close. Its nodes end at the
            // token taken (`CLOSE m=1`); after a mark's arm, which took none, at the last token
            // taken (`m=0`).
            let marks: Vec<u16> = self.marks.iter().map(|m| m.kind).collect();
            let after_mark = self.arms.iter().any(|a| usize::from(a.to) == s && !a.call && marks.contains(&a.token));
            for mode in [1, 0].into_iter().take(1 + usize::from(after_mark)) {
                if mode == 0 {
                    self.marked[s] = self.pc();
                }
                for _ in 0..c + u8::from(st.own) {
                    self.push("CLOSE", mode, Arg::None);
                }
                // After a mark, the token is not taken: the caller switches on it again
                // (`RETURN m=1`), as a return lands past `TOKEN` only where none was taken.
                self.push("RETURN", u8::from(mode == 0), Arg::None);
            }
            return Ok(());
        }
        // `m=1+k`: mask `k` holds where any mark does; `TOKEN` sets the marks live there (and
        // at the stream's end) and none elsewhere, so a state's mark tests fail on a register.
        self.push("TOKEN", self.any_mark.map_or(0, |k| k as u8 + 1), Arg::Num(STARTS));
        // The marks, in order: each the state takes, a test and a jump to its arm; each not
        // hidden that it does not take, and each its level refuses, a test that refuses.
        let tests: Vec<(usize, Option<usize>)> = (0..self.marks.len())
            .filter_map(|k| {
                let m = &self.marks[k];
                let arm = mine.iter().copied().find(|&a| self.arms[a].token == m.kind);
                (arm.is_some() || !m.hidden || st.refuses.contains(&m.kind)).then_some((k, arm))
            })
            .collect();
        let mut mark_arms = Vec::new();
        self.mark_tests_here(&tests, &mut mark_arms);
        let t = self.switches.len();
        self.switches.push(ByteSwitch::refusing(0));
        self.switch_at[s] = self.pc();
        self.push("SWITCH", 0, Arg::Table(t as u16));
        // The end: the root closes and accepts; a nested level closes its node and returns to
        // switch on the token again (at the stream's end, or on a token no arm takes).
        let end = match st.end {
            None => REFUSE,
            Some(c) => {
                let pc = self.pc();
                for _ in 0..c + u8::from(nested && st.own) {
                    self.push("CLOSE", 0, Arg::None);
                }
                self.push(if nested { "RETURN" } else { "ACCEPT" }, u8::from(nested), Arg::None);
                pc
            }
        };
        self.switches[t].end = end;
        let default = if nested { end } else { REFUSE };
        self.default = default;
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
            let pc = self.by_words(t, b, cs)?;
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
/// Tier 1's records the words read: the extract of every token's first byte (`TOKEN`); `SPAN
/// m=1` takes the record's end (operand 0).
pub const STARTS: u16 = 0;
pub const LASTS: u16 = 0;

/// The words, after `empties` words `EMPTY t` (`t` in `0..empties`): tier 1's fallback totals,
/// each of which must be zero before the first token is taken. With a `whole` root, an `OPEN` of
/// the root follows them: no token is taken yet, so it opens at byte 0.
#[allow(clippy::too_many_arguments)]
///
/// `rules`: a function a rule (`grammar::build_shaped`). The words are laid out rule by rule,
/// each rule's first place in `Lowered::rules`, and words are shared within a rule only.
pub fn lower(states: &[StateRow], arms: &[ArmRow], rows: &[Row], marks: &[Mark], any_mark: Option<u16>, meets: &[Vec<bool>], empties: u16, whole: bool, survey: bool, rules: bool) -> Result<Lowered, String> {
    let mut e = Emit {
        rule_of: Vec::new(),
        rule: 0,
        by_rules: rules,
        states,
        arms,
        rows,
        marks,
        any_mark,
        meets,
        survey,
        tested: Default::default(),
        default: REFUSE,
        root: states.first().ok_or("no states")?.level,
        words: Vec::new(),
        switches: Vec::new(),
        strings: Vec::new(),
        entry: vec![0; states.len()],
        switch_at: vec![REFUSE; states.len()],
        marked: vec![0; states.len()],
        arm_code: HashMap::new(),
        mark_words: Default::default(),
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
    // A return lands on a state's `TOKEN` (`RETURN m=0`, the token taken) or its `SWITCH`
    // (`m=1`, to switch on it again). A state that takes no token, which closes at once, gets a
    // return entry of that shape: `TOKEN`, a `SWITCH` whose every way is the end, and the end,
    // its closes at the end of the token before and a return to switch on this one again.
    let mut returns: HashMap<u16, u16> = HashMap::new();
    for p in arms.iter().flat_map(|a| a.push.iter().copied()) {
        if e.words[usize::from(e.entry[usize::from(p)])].0 == "TOKEN" || returns.contains_key(&p) {
            continue;
        }
        let st = &states[usize::from(p)];
        let Some(c) = st.end else { return Err(format!("state {p} takes no token and may not end")) };
        e.rule = st.rule;
        let at = e.pc();
        let any = e.any_mark.map_or(0, |k| k as u8 + 1);
        e.push("TOKEN", any, Arg::Num(STARTS));
        let t = e.switches.len();
        e.push("SWITCH", 0, Arg::Table(t as u16));
        let end = e.pc();
        e.switches.push(ByteSwitch { offset: 0, targets: vec![end; 256], end, words: Vec::new() });
        for _ in 0..c + u8::from(st.own) {
            e.push("CLOSE", 0, Arg::None);
        }
        e.push("RETURN", 1, Arg::None);
        returns.insert(p, at);
    }
    // Every target as a word's place, then the words that behave alike shared (`merge_words`).
    let place = |a: Arg| match a {
        Arg::State(s) => Arg::Pc(e.entry[usize::from(s)]),
        Arg::Again(s) => Arg::Pc(e.switch_at[usize::from(s)]),
        Arg::Past(s) => Arg::Pc(e.entry[usize::from(s)] + 1),
        Arg::Marked(s) => Arg::Pc(e.after_mark(s)),
        Arg::Return(s) => Arg::Pc(returns.get(&s).copied().unwrap_or(e.entry[usize::from(s)])),
        a => a,
    };
    let mut placed: Vec<(&'static str, u8, Arg)> = e.words.iter().map(|&(op, mode, arg)| (op, mode, place(arg))).collect();
    // A function a rule: the words rule by rule (in order within each), every place renumbered.
    // A word goes on by position only within its rule (each run the emitter makes is one rule's
    // and ends in a jump), so the runs move whole.
    let mut rule_of = e.rule_of.clone();
    if rules {
        let mut order: Vec<usize> = (0..placed.len()).collect();
        order.sort_by_key(|&pc| rule_of[pc]);
        let mut new_at = vec![0u16; placed.len()];
        for (k, &pc) in order.iter().enumerate() {
            new_at[pc] = k as u16;
        }
        let to = |t: u16| if t == REFUSE || usize::from(t) >= new_at.len() { t } else { new_at[usize::from(t)] };
        placed = order.iter().map(|&pc| match placed[pc] { (op, mode, Arg::Pc(t)) => (op, mode, Arg::Pc(to(t))), w => w }).collect();
        rule_of = order.iter().map(|&pc| rule_of[pc]).collect();
        for sw in &mut e.switches {
            for t in sw.targets.iter_mut().chain([&mut sw.end]).chain(sw.words.iter_mut().map(|(_, t)| t)) {
                *t = to(*t);
            }
        }
    }
    let (words, switches, kept_rules) = merge_words(&placed, &e.switches, rules.then_some(&rule_of[..]));
    let (words, switches, kept_rules) = lay_out(&words, &switches, &kept_rules);
    e.words = words;
    e.switches = switches;
    // Each rule's first place.
    let firsts: Vec<u16> = if rules { (0..kept_rules.len()).filter(|&pc| pc == 0 || kept_rules[pc] != kept_rules[pc - 1]).map(|pc| pc as u16).collect() } else { Vec::new() };
    if rules && kept_rules.windows(2).any(|w| w[0] > w[1]) {
        return Err("a function a rule: the rules' words are not laid out in order".into());
    }
    let mut text = String::new();
    use std::fmt::Write as _;
    for (at, s) in e.strings.iter().enumerate() {
        let _ = writeln!(text, "  string {at} {}", crate::rir_text::quote(s));
    }
    for (at, s) in e.switches.iter().enumerate() {
        crate::rir_text::switch_rows(&mut text, at, s);
    }
    let mark_words: Vec<(u8, u16)> = e.mark_words.iter().map(|(&k, &s)| (k, s)).collect();
    crate::rir_text::mark_word_rows(&mut text, &mark_words);
    crate::rir_text::rule_rows(&mut text, &firsts);
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
            Arg::Again(s) => write!(text, " @{:04}", e.switch_at[usize::from(s)]),
            Arg::Past(s) => write!(text, " @{:04}", e.entry[usize::from(s)] + 1),
            Arg::Marked(s) => write!(text, " @{:04}", e.after_mark(s)),
            Arg::Return(s) => write!(text, " @{:04}", returns.get(&s).copied().unwrap_or(e.entry[usize::from(s)])),
            Arg::Pc(p) => write!(text, " @{p:04}"),
        };
        text.push('\n');
    }
    if let Some(at) = std::env::var_os("RR_LOWERED_TEXT") {
        let _ = std::fs::write(at, &text);
    }
    Ok(Lowered { text, switches: e.switches, strings: e.strings, tested: e.tested.into_iter().collect(), rules: firsts, mark_words })
}

/// The words laid out as compiled code is, with no jump a layout can spare (the lean parser's
/// `match` arms run on into their code):
///
/// - a place a jump reaches that only jumps on is passed: every target goes to where the jumps
///   end, within its rule;
/// - a jump to a lone `STOP`, or within a rule to a `RETURN`, is that word (not after a `MARK`
///   or `ROW`, whose way is a `GOTO` or `STOP`);
/// - a run of words entered by one jump alone is placed after that jump, and a jump to the
///   next place goes.
///
/// A run is words that go on by position: a word and the next it falls to, and a test's two
/// ways. Runs move whole, and a rule's runs stay its own, its first run first.
fn lay_out(words: &[(&'static str, u8, Arg)], switches: &[ByteSwitch], rules: &[u16]) -> (Vec<(&'static str, u8, Arg)>, Vec<ByteSwitch>, Vec<u16>) {
    let n = words.len();
    if n == 0 {
        return (words.to_vec(), switches.to_vec(), rules.to_vec());
    }
    let rule = |pc: usize| rules.get(pc).copied().unwrap_or(0);
    let by_place: Vec<usize> = (0..n).map(|pc| ways(words, switches, pc).1).collect();
    // `glued[pc]`: the word at `pc + 1` is reached from `pc`'s run by position.
    let glued: Vec<bool> = (0..n).map(|pc| pc + 1 < n && (by_place[pc] >= 1 || (pc > 0 && by_place[pc - 1] == 2))).collect();
    let starts_run = |pc: usize| pc == 0 || !glued[pc - 1];
    let mut words: Vec<(&'static str, u8, Arg)> = words.to_vec();
    let mut switches: Vec<ByteSwitch> = switches.to_vec();
    // Each switch's rule: its word's.
    let mut table_rule: HashMap<u16, u16> = HashMap::new();
    for (pc, w) in words.iter().enumerate() {
        if let ("SWITCH", _, Arg::Table(t)) = *w {
            table_rule.insert(t, rule(pc));
        }
    }
    // Where a jump to `t` ends, within rule `r`: past places that are a lone `GOTO`.
    let lone_goto = |words: &[(&'static str, u8, Arg)], t: usize| words[t].0 == "GOTO" && starts_run(t) && !glued[t];
    let dest = |words: &[(&'static str, u8, Arg)], t: u16, r: u16| -> u16 {
        let mut t = t;
        for _ in 0..n {
            if t == REFUSE || usize::from(t) >= n || rule(usize::from(t)) != r || !lone_goto(words, usize::from(t)) {
                break;
            }
            let Arg::Pc(next) = words[usize::from(t)].2 else { break };
            if next == t || (usize::from(next) < n && rule(usize::from(next)) != r) {
                break;
            }
            t = next;
        }
        t
    };
    // 1. Targets past jumps.
    for pc in 0..n {
        if let (op, mode, Arg::Pc(t)) = words[pc] {
            // A call's callee is another rule's first place: kept as it is.
            if op == "GOTO" && pc > 0 && words[pc - 1].0 == "PUSH" {
                continue;
            }
            let d = dest(&words, t, rule(pc));
            words[pc] = (op, mode, Arg::Pc(d));
        }
    }
    for (t, sw) in switches.iter_mut().enumerate() {
        let r = table_rule.get(&(t as u16)).copied().unwrap_or(0);
        let w = &words;
        for x in sw.targets.iter_mut().chain([&mut sw.end]).chain(sw.words.iter_mut().map(|(_, x)| x)) {
            *x = dest(w, *x, r);
        }
    }
    // 2. A jump to a lone `STOP`, or to a `RETURN` of its rule, is that word.
    for pc in 0..n {
        let ("GOTO", _, Arg::Pc(t)) = words[pc] else { continue };
        if (pc > 0 && words[pc - 1].0 == "PUSH") || usize::from(t) >= n {
            continue;
        }
        let after_mark = pc > 0 && matches!(words[pc - 1].0, "MARK" | "ROW");
        let (op, mode, arg) = words[usize::from(t)];
        let copy = match op {
            "STOP" => true,
            "RETURN" => rule(usize::from(t)) == rule(pc) && !after_mark,
            _ => false,
        };
        if copy {
            words[pc] = (op, mode, arg);
        }
    }
    // The runs, and the references into each from the runs that are reached.
    let mut run_of = vec![0usize; n];
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for pc in 0..n {
        if starts_run(pc) {
            runs.push((pc, pc));
        }
        let last = runs.len() - 1;
        runs[last].1 = pc;
        run_of[pc] = last;
    }
    let targets_of = |words: &[(&'static str, u8, Arg)], switches: &[ByteSwitch], pc: usize| -> Vec<u16> {
        match words[pc] {
            (_, _, Arg::Pc(t)) => vec![t],
            ("SWITCH", _, Arg::Table(t)) => switches[usize::from(t)].all_targets().collect(),
            _ => Vec::new(),
        }
    };
    // Kept from the start, and from each rule's first run (a call enters there).
    let mut reached = vec![false; runs.len()];
    let mut work: Vec<usize> = vec![0];
    for pc in 1..n {
        if rule(pc) != rule(pc - 1) {
            work.push(run_of[pc]);
        }
    }
    let firsts: std::collections::HashSet<usize> = work.iter().copied().collect();
    let mut refs = vec![0usize; runs.len()];
    let mut ref_from_goto_end = vec![None::<usize>; runs.len()];
    while let Some(r) = work.pop() {
        if reached[r] {
            continue;
        }
        reached[r] = true;
        let (a, b) = runs[r];
        for pc in a..=b {
            for t in targets_of(&words, &switches, pc) {
                if t == REFUSE || usize::from(t) >= n {
                    continue;
                }
                let tr = run_of[usize::from(t)];
                // Into a run's middle, or a call's landing (its next place too): never moved.
                let whole = runs[tr].0 == usize::from(t) && words[pc].0 != "PUSH";
                refs[tr] += if whole { 1 } else { 2 };
                if whole && pc == b && words[pc].0 == "GOTO" && !(pc > 0 && words[pc - 1].0 == "PUSH") {
                    ref_from_goto_end[tr] = Some(r);
                }
                work.push(tr);
            }
        }
    }
    // 3. The order: each rule's runs as they were, a run entered by one jump alone after it.
    let movable = |r: usize| reached[r] && refs[r] == 1 && !firsts.contains(&r) && r != 0;
    let mut placed = vec![false; runs.len()];
    let mut order: Vec<usize> = Vec::new();
    for r in 0..runs.len() {
        if placed[r] || !reached[r] || (movable(r) && ref_from_goto_end[r].is_some_and(|from| rule(runs[from].0) == rule(runs[r].0))) {
            continue;
        }
        let mut at = r;
        loop {
            placed[at] = true;
            order.push(at);
            let end = runs[at].1;
            let ("GOTO", _, Arg::Pc(t)) = words[end] else { break };
            if usize::from(t) >= n || (end > 0 && words[end - 1].0 == "PUSH") {
                break;
            }
            let next = run_of[usize::from(t)];
            if placed[next] || !movable(next) || ref_from_goto_end[next] != Some(at) || rule(runs[next].0) != rule(end) {
                break;
            }
            at = next;
        }
    }
    // A run whose one way in was never placed (its jump's run unreached): placed where it was.
    for r in 0..runs.len() {
        if reached[r] && !placed[r] {
            placed[r] = true;
            order.push(r);
        }
    }
    order.sort_by_key(|&r| rule(runs[r].0));
    // 4. The places, numbered: a run's last `GOTO` to the run after it goes.
    let drops: Vec<bool> = order
        .iter()
        .enumerate()
        .map(|(i, &r)| {
            let end = runs[r].1;
            let call = end > 0 && words[end - 1].0 == "PUSH";
            matches!(words[end], ("GOTO", _, Arg::Pc(t)) if !call && order.get(i + 1).is_some_and(|&q| runs[q].0 == usize::from(t)))
        })
        .collect();
    let mut new_at = vec![REFUSE; n];
    let mut k = 0u16;
    for (i, &r) in order.iter().enumerate() {
        let (a, b) = runs[r];
        for pc in a..=b {
            new_at[pc] = k;
            if !(pc == b && drops[i]) {
                k += 1;
            }
        }
    }
    let to = |t: u16| if t == REFUSE || usize::from(t) >= n { REFUSE } else { new_at[usize::from(t)] };
    let mut out = Vec::with_capacity(usize::from(k));
    let mut out_rules = Vec::with_capacity(usize::from(k));
    for (i, &r) in order.iter().enumerate() {
        let (a, b) = runs[r];
        for pc in a..=b {
            if pc == b && drops[i] {
                continue;
            }
            let (op, mode, arg) = words[pc];
            out.push((op, mode, if let Arg::Pc(t) = arg { Arg::Pc(to(t)) } else { arg }));
            out_rules.push(rule(pc));
        }
    }
    for sw in &mut switches {
        for x in sw.targets.iter_mut().chain([&mut sw.end]).chain(sw.words.iter_mut().map(|(_, x)| x)) {
            *x = to(*x);
        }
    }
    (out, switches, out_rules)
}

/// A word's operand without its target, for the first partition (`merge_words`).
fn operand_key(arg: Arg) -> (u8, u16) {
    match arg {
        Arg::None => (0, 0),
        Arg::Num(n) => (1, n),
        Arg::Kind(k) => (2, k),
        Arg::Field(f) => (3, f),
        Arg::Str(s) => (4, s),
        Arg::Table(_) | Arg::State(_) | Arg::Again(_) | Arg::Past(_) | Arg::Marked(_) | Arg::Return(_) | Arg::Pc(_) => (5, 0),
    }
}

/// How a word goes on, as `stream::validate` reads it: its targets, and the places after it it
/// goes to by position (the next, and for a test the one after).
fn ways(words: &[(&'static str, u8, Arg)], switches: &[ByteSwitch], pc: usize) -> (Vec<u16>, usize) {
    let (op, mode, arg) = words[pc];
    let target = |a: Arg| match a {
        Arg::Pc(t) => t,
        _ => REFUSE,
    };
    match op {
        "STOP" | "ACCEPT" | "RETURN" => (vec![], 0),
        "GOTO" => (vec![target(arg)], 0),
        "SWITCH" => {
            let Arg::Table(t) = arg else { return (vec![], 0) };
            let s = &switches[usize::from(t)];
            (s.all_targets().collect(), 0)
        }
        // A call's landings: the state (`RETURN m=0`) and the word after it (`m=1`).
        "PUSH" => {
            let p = target(arg);
            (vec![p, p.wrapping_add(1)], 1)
        }
        "WORD" if mode != 0 => (vec![], 2),
        "FLAG" if mode >= 2 => (vec![], 2),
        "MARK" | "ROW" => (vec![], 2),
        _ => (vec![], 1),
    }
}

/// The words with those that behave alike shared: words are alike when their operation and
/// operands are, and so are the words they go to, by target and by position (the coarsest such
/// partition, found by refining one by operation until it holds: cycles included). One word of
/// each kind is kept; a place a kept word reaches by position that holds another of its kind
/// becomes a `GOTO` to it, and words nothing reaches go. The places of a state a call returns to
/// stay whole (`TOKEN`, its `MARK`s and their ways, `SWITCH`: `stream::validate`'s shape).
///
/// `rules`: each word's rule (a function a rule): words of two rules are never alike, but a
/// call goes to any rule's words alike with its callee's (the same partition with no rules):
/// rules that are copies, whole or from the place a call enters, are one function, as the
/// lean parser's rules call one helper. No word goes to another rule but by a call. The kept
/// words' rules come back with them.
fn merge_words(words: &[(&'static str, u8, Arg)], switches: &[ByteSwitch], rules: Option<&[u16]>) -> (Vec<(&'static str, u8, Arg)>, Vec<ByteSwitch>, Vec<u16>) {
    let n = words.len();
    const NONE: u32 = u32::MAX;
    let class_of = |class: &[u32], t: u16| if t == REFUSE || usize::from(t) >= n { NONE } else { class[usize::from(t)] };
    // The first partition: operation, mode, operand (a switch's offset too), and the rule.
    let is_call = |pc: usize| pc > 0 && words[pc].0 == "GOTO" && words[pc - 1].0 == "PUSH";
    let partition = |rules: Option<&[u16]>| -> (Vec<u32>, usize) {
        let mut class: Vec<u32> = {
            let mut ids: HashMap<(&str, u8, (u8, u16), u8, u16, Vec<u16>), u32> = HashMap::new();
            words
                .iter()
                .enumerate()
                .map(|(pc, &(op, mode, arg))| {
                    // A switch's offset and the words it names (their targets are its ways).
                    let (offset, named) = if let Arg::Table(t) = arg { (switches[usize::from(t)].offset, switches[usize::from(t)].words.iter().map(|&(w, _)| w).collect()) } else { (0, Vec::new()) };
                    let next = ids.len() as u32;
                    let rule = rules.map_or(0, |r| r[pc]);
                    // A call's `GOTO` is never another `GOTO`'s kind: it goes to its callee.
                    let op = if is_call(pc) { "GOTO (a call's)" } else { op };
                    *ids.entry((op, mode, operand_key(arg), offset, rule, named)).or_insert(next)
                })
                .collect()
        };
        let mut count = class.iter().copied().max().map_or(0, |m| m as usize + 1);
        loop {
            let mut ids: HashMap<Vec<u32>, u32> = HashMap::new();
            let next: Vec<u32> = (0..n)
                .map(|pc| {
                    let (targets, by_place) = ways(words, switches, pc);
                    let mut key = Vec::with_capacity(1 + targets.len() + by_place);
                    key.push(class[pc]);
                    key.extend(targets.iter().map(|&t| class_of(&class, t)));
                    key.extend((1..=by_place).map(|d| if pc + d < n { class[pc + d] } else { NONE }));
                    let id = ids.len() as u32;
                    *ids.entry(key).or_insert(id)
                })
                .collect();
            let refined = ids.len();
            class = next;
            if refined == count {
                break;
            }
            count = refined;
        }
        (class, count)
    };
    let (class, count) = partition(rules);
    let mut rep = vec![usize::MAX; count];
    for pc in (0..n).rev() {
        rep[class[pc] as usize] = pc;
    }
    // A call's callee: the first place alike with it in any rule (its own partition's first is
    // that place, the first of its kind there too).
    let callee_of: Vec<u16> = {
        let (across, count) = if rules.is_some() { partition(None) } else { (class.clone(), count) };
        let mut first = vec![usize::MAX; count];
        for pc in (0..n).rev() {
            first[across[pc] as usize] = pc;
        }
        (0..n).map(|t| first[across[t] as usize] as u16).collect()
    };
    // What each place holds: nothing, its own word, or a `GOTO` to its kind's kept word.
    #[derive(Clone, Copy, PartialEq)]
    enum Hold {
        Gone,
        Own,
        Stub(usize),
    }
    let mut hold = vec![Hold::Gone; n];
    // A call's landing for each `PUSH`: its state's kept word where the word after is alike.
    let mut landing = vec![0u16; n];
    let mut todo: Vec<(usize, bool)> = vec![(0, false)];
    while let Some((pc, own)) = todo.pop() {
        let r = rep[class[pc] as usize];
        if !own && r != pc {
            if hold[pc] == Hold::Gone {
                hold[pc] = Hold::Stub(r);
                todo.push((r, false));
            }
            continue;
        }
        if hold[pc] == Hold::Own {
            continue;
        }
        hold[pc] = Hold::Own;
        let (op, _, arg) = words[pc];
        let (targets, by_place) = ways(words, switches, pc);
        if op == "PUSH" {
            let Arg::Pc(p) = arg else { continue };
            let (p, r) = (usize::from(p), rep[class[usize::from(p)] as usize]);
            let to = if r + 1 < n && p + 1 < n && class[r + 1] == class[p + 1] { r } else { p };
            landing[pc] = to as u16;
            todo.push((to, true));
            todo.push((to + 1, false));
        } else {
            for t in targets {
                if t != REFUSE && usize::from(t) < n {
                    let t = if is_call(pc) { callee_of[usize::from(t)] } else { t };
                    todo.push((rep[class[usize::from(t)] as usize], true));
                }
            }
        }
        // A state's places by position stay its own: after `TOKEN` its `MARK` or `SWITCH`, after
        // a `MARK` its way, then the next `MARK` or the `SWITCH`; after a `PUSH` its call's
        // `GOTO` (only a `PUSH` reaches it, so it goes to its callee as a call).
        for d in 1..=by_place {
            if pc + d < n {
                let whole = matches!((op, d), ("TOKEN", 1) | ("MARK", 2) | ("PUSH", 1));
                todo.push((pc + d, whole));
            }
        }
    }
    // The kept places in order, renumbered: places kept stay next to each other.
    let mut new_at = vec![REFUSE; n];
    let mut k = 0u16;
    for pc in 0..n {
        if hold[pc] != Hold::Gone {
            new_at[pc] = k;
            k += 1;
        }
    }
    let to = |t: u16| if t == REFUSE || usize::from(t) >= n { REFUSE } else { new_at[rep[class[usize::from(t)] as usize]] };
    let mut tables: HashMap<u16, u16> = HashMap::new();
    let mut out_switches = Vec::new();
    let mut out = Vec::with_capacity(usize::from(k));
    let mut out_rules = Vec::with_capacity(usize::from(k));
    for pc in 0..n {
        if hold[pc] != Hold::Gone {
            out_rules.push(rules.map_or(0, |r| r[pc]));
        }
        match hold[pc] {
            Hold::Gone => {}
            Hold::Stub(r) => out.push(("GOTO", 0, Arg::Pc(new_at[r]))),
            Hold::Own => {
                let (op, mode, arg) = words[pc];
                let arg = match (op, arg) {
                    ("PUSH", Arg::Pc(_)) => Arg::Pc(new_at[usize::from(landing[pc])]),
                    ("SWITCH", Arg::Table(t)) => {
                        let at = *tables.entry(t).or_insert_with(|| {
                            let s = &switches[usize::from(t)];
                            out_switches.push(ByteSwitch { offset: s.offset, targets: s.targets.iter().map(|&x| to(x)).collect(), end: to(s.end), words: s.words.iter().map(|&(w, x)| (w, to(x))).collect() });
                            (out_switches.len() - 1) as u16
                        });
                        Arg::Table(at)
                    }
                    (_, Arg::Pc(t)) if is_call(pc) && usize::from(t) < n => Arg::Pc(to(callee_of[usize::from(t)])),
                    (_, Arg::Pc(t)) => Arg::Pc(to(t)),
                    (_, a) => a,
                };
                out.push((op, mode, arg));
            }
        }
    }
    (out, out_switches, out_rules)
}

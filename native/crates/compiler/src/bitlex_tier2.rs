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
use crate::program::{MARK_KEEP, push_flag};
use crate::stream_ir::{ByteSwitch, WORD_KIND};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

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
    /// Its kind is its own (`records::Retag`, a reader's or a part's), at its one byte of
    /// `first`, which no token begins with: its keywords are met at that byte, not at their
    /// first letters.
    pub moved: bool,
}

impl Row {
    /// Whether tier 2 reads the two rows alike: one kind, its first bytes, its words, its end and
    /// its check (decision 0509: the rows of one kind in each reader's region, a string's text).
    fn alike(&self, o: &Row) -> bool {
        self.kind == o.kind && self.one_byte == o.one_byte && self.first == o.first && self.keywords == o.keywords && self.soft == o.soft && self.check == o.check && self.moved == o.moved
    }

    /// Its tokens' ends are the stream's last bytes (`SPAN m=1`): every row but one of one-byte
    /// tokens. A row of keywords only is no exception: its token runs to its last byte whatever
    /// the word, and `truee` is not `true` (the word's bytes are compared over the whole token).
    pub fn takes_last(&self) -> bool {
        !self.one_byte
    }
}

/// A word's place while the words are made: wider than the program's `u16` (`Lowered`), as the
/// words before `merge_words` shares those alike are several times the program's; the program is
/// refused past `u16`'s places once they are laid out (`too_big`).
type Place = u32;

/// No place: refused (`stream_ir::REFUSE` in the program).
const REFUSE: Place = Place::MAX;

/// A `ByteSwitch` while the words are made, its targets places (`Place`).
#[derive(Clone)]
struct Switch {
    offset: u8,
    targets: Vec<Place>,
    end: Place,
    words: Vec<(u16, Place)>,
}

impl Switch {
    fn refusing(offset: u8) -> Self {
        Switch { offset, targets: vec![REFUSE; 256], end: REFUSE, words: Vec::new() }
    }

    /// Every place the switch goes: its bytes', its end, its words'.
    fn all_targets(&self) -> impl Iterator<Item = Place> + '_ {
        self.targets.iter().copied().chain([self.end]).chain(self.words.iter().map(|&(_, t)| t))
    }

    /// The program's switch: every place a `u16` (`too_big` has checked them).
    fn narrow(&self) -> ByteSwitch {
        let n = |t: Place| if t == REFUSE { crate::stream_ir::REFUSE } else { t as u16 };
        ByteSwitch { offset: self.offset, targets: self.targets.iter().map(|&t| n(t)).collect(), end: n(self.end), words: self.words.iter().map(|&(w, t)| (w, n(t))).collect() }
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
    /// (`Emit::tests_of`).
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
    Pc(Place),
}

/// Where an arm's token ends: a length, or the stream's next last byte.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Span {
    Len(u16),
    Last,
}

/// What a call's arm does: its acts, the state it goes to, the returns it pushes, what it tests
/// before (`Pre`), where a guard that holds goes (the state's `default`), and the rule its words
/// are (the caller's: a function's words are its own).
type CallCode = (Vec<Act>, u16, Vec<u16>, Pre, Place, u16);

/// What an arm tests before its acts.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Pre {
    None,
    Word(Vec<u8>),
}

/// Tier 2 as words: their `rir` text (strings, switches, words).
/// One element of a token mark's run, as `Lowered::token_marks` holds it: its offset, its kinds'
/// names, its words (strings), whether it is a `~`, whether the file's end takes it, and
/// whether it is a run of its choices (`program::TokenElem`), a run's depths' sides: each its
/// counter, whether it opens, its kinds' names and its words (`program::Nest`), and a run's
/// `ending`'s kinds and words, and the marks it holds with or without (`with M`, `without M`:
/// each its name and whether with).
pub type TokenElemRow = (i8, Vec<String>, Vec<u16>, bool, bool, bool, Vec<NestRow>, Option<(Vec<String>, Vec<u16>)>, Vec<(String, bool)>);

/// One side of a run's depth, as `TokenElemRow` holds it: counter, opens, kinds, words.
pub type NestRow = (u8, bool, Vec<String>, Vec<u16>);

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
    /// Each of `rules`' rule: its index (`StateRow::rule`).
    pub rule_ids: Vec<u16>,
    /// The marks of one word: (order, string), rising (`Words::mark_words`).
    pub mark_words: Vec<(u8, u16)>,
    /// The token marks (`Mark::tokens`), rising by order: each element's offset, its kinds'
    /// names and its words (strings). The kinds become the records' (`Words::token_marks`).
    /// Each element: its offset, kinds, words, whether it is a `~` of them, whether the file's
    /// end takes it (`::bitlex::syntax::TokenElem`). The flag: the mark refuses (`refusing`).
    pub token_marks: Vec<(u8, bool, bool, Vec<TokenElemRow>)>,
    /// The `drops` and `loses` questions (`Words::gap_patterns`), in the order `SETTLE` names
    /// them: each entry's pattern, whether it is an `unless`, and its elements (`GapElemRow`).
    pub gap_patterns: Vec<(u8, bool, Vec<GapElemRow>)>,
}

/// One element of a gap pattern, as `Lowered::gap_patterns` holds it: its offset from the token,
/// its kinds' names, its words (strings), whether it is a `~` of them, the marks it holds with or
/// without (each its name and whether with), and the bytes it may start with (`starting`).
pub type GapElemRow = (i8, Vec<String>, Vec<u16>, bool, Vec<(String, bool)>, Vec<u8>, bool);

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
    /// The bytes a token it holds at can begin with (`bitlex_prove::never` of its mask and the
    /// other rows' starts).
    pub first: ByteSet,
    /// The words a token it holds at is one of, where it holds at no other (its first element's
    /// words, or proved of its mask): a state tests it at those words alone, as the lean parser
    /// asks `is_word` where it reads the token.
    pub words: Option<Vec<Vec<u8>>>,
    /// A token mark (decision 0129): the token before (none: any), then the run from the mark's.
    pub tokens: Option<(Vec<::bitlex::syntax::TokenElem>, Vec<Vec<::bitlex::syntax::TokenElem>>)>,
    /// `before`, `unless before` (decision 0416): the tokens from the one a gap mark is before,
    /// each with whether it is an `unless`, tested by tier 2 as a token mark's run is.
    pub nexts: Vec<(bool, Vec<Vec<::bitlex::syntax::TokenElem>>)>,
    /// `unless after`: the words of the grammar token before that make it none
    /// (`records::Records::unless`); the records' words name them.
    pub unless: Vec<Vec<u8>>,
    /// `, settles`: its mask from each bit to the next grammar token's start, which its arms
    /// settle the pending extras before (`SETTLE`).
    pub settle: Option<u16>,
    /// `, passes M`: the marks after it that taking it passes too, a bit each by order
    /// (`Words::mark_passes`).
    pub passes: u64,
    /// `, drops …`: its questions' first entry (`Words::gap_patterns`), at whose tokens its
    /// arms drop the extra pending last (`SETTLE m=1`).
    pub drop: Option<u16>,
    /// `, loses …`: its questions' first entry, at whose tokens each test of the mark gives the
    /// extra pending last no width, at the token (`SETTLE m=2`), as the lean lexer's `asi`
    /// loses it.
    pub lose: Option<u16>,
    /// The questions themselves (`GapPattern`), whose rows `lower` writes (`Lowered::gap_patterns`).
    pub drops: Vec<::bitlex::syntax::GapPattern>,
    pub loses: Vec<::bitlex::syntax::GapPattern>,
    /// `, refuses …`: its questions' first entry, at whose tokens each test of the mark refuses
    /// the file (`SETTLE m=19`), and the questions.
    pub refuse: Option<u16>,
    pub refuses: Vec<::bitlex::syntax::GapPattern>,
    /// `, refuses taken …`: its questions' first entry, asked in each arm that takes the mark
    /// (`SETTLE m=19`, where it holds), and the questions.
    pub refuse_taken: Option<u16>,
    pub refuses_taken: Vec<::bitlex::syntax::GapPattern>,
    /// With `lose` or `refuse`: the flag set where a level is at the mark's level (decision 0451), the lean
    /// parser's `stmt`. A call sets it where the state it returns to takes the mark, keeps its
    /// caller's where that state ends at it, and clears it elsewhere (`lose_flags`).
    pub lose_flag: u16,
    /// `, ends`: hidden, and a level that may end where it holds and does not take it ends
    /// there, its caller to take it.
    pub ends: bool,
}

/// What a state does at a mark it tests: its arm, a refusal, or its level's end (`Mark::ends`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum AtMark {
    Arm(usize),
    Refuse,
    End,
}

struct Emit<'a> {
    states: &'a [StateRow],
    arms: &'a [ArmRow],
    rows: &'a [Row],
    marks: &'a [Mark],
    /// The mask of any mark at a token (`TOKEN m=1+k`), when there are marks.
    any_mark: Option<u16>,
    /// The extras taken in the token step (`TOKEN_EXTRAS`): every `TOKEN` but the root's first,
    /// where an extra opens the root (`grammar::with_extras`).
    token_extras: bool,
    whole: bool,
    /// The counts of tokens (`COUNT`): each one's ups' and downs' kinds.
    counts: Vec<(Vec<u16>, Vec<u16>)>,
    /// The local checks (`CHECK`): each one's token kinds, and the mark whose refusing states
    /// alone check it.
    checks: Vec<(Vec<u16>, Option<u16>)>,
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
    switches: Vec<Switch>,
    strings: Vec<Vec<u8>>,
    entry: Vec<Place>,
    /// Each state's `SWITCH` (a call's way in, `Arg::Again`).
    switch_at: Vec<Place>,
    /// Each state's entry after a mark (`Arg::Marked`): its entry, but for a nested state that
    /// takes no token.
    marked: Vec<Place>,
    /// Each state's mark tests (`tests_of`), and whether every call into it may enter it at its
    /// `SWITCH` (`at_switch`): found once, before the states are laid out.
    tests_of: Vec<Vec<usize>>,
    at_switch: Vec<bool>,
    /// Each state whose arms are one call (`Emit::direct_call`): no switch, the call made at once,
    /// as the lean parser calls `unary()` without peeking first (decision 0492).
    direct: Vec<bool>,
    /// Each arm's code, once per test before it.
    arm_code: HashMap<(usize, Span, Pre), Place>,
    /// A call's code, by what it does (`CallCode`): a call takes no token, so the arms of one
    /// call, one per token its callee may begin with, are one code.
    call_code: HashMap<CallCode, Place>,
    /// The `ROW` tests at a byte, by their rule, rows, whether the last is untested, the
    /// default and each row's target: a chain shared by the bytes alike (a number's ten digits).
    row_chains: HashMap<(u16, Vec<usize>, bool, Place, Vec<Place>), Place>,
    /// The marks of one word, by order: their strings (`Lowered::mark_words`).
    mark_words: std::collections::BTreeMap<u8, u16>,
    /// Each keyword's leaf kind, by its string (`Words::word_kinds`): a leaf of a keyword's
    /// own kind takes it from the token's word, so keywords' arms differing only in it are one.
    word_kinds: std::collections::BTreeMap<u16, u16>,
    /// Where the state being lowered goes on a token none of its arms takes: its level's end
    /// (the caller then switches on the token), or `REFUSE`.
    default: Place,
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
    /// The next word's place.
    fn pc(&self) -> Place {
        self.words.len() as Place
    }

    fn push(&mut self, op: &'static str, mode: u8, arg: Arg) {
        self.words.push((op, mode, arg));
        self.rule_of.push(self.rule);
    }

    /// A call's `PUSH`, and with a function a rule (`by_rules`), its `GOTO` to the words after
    /// it, which are its callee's: they run in its frame (its flags, its word, its own calls).
    /// `callee`: the state whose rule that is.
    /// `flags`: the arm's flags (`flags_of`), `i` the call's: with a function a rule, one flag
    /// the call runs with is its `PUSH`'s (`push_flag`), as a parameter, so the callee is entered
    /// at its one entry; else they are the first words after it.
    fn call(&mut self, ret: u16, callee: u16, flags: &[(u8, u16, bool)], i: usize) {
        let mine: Vec<(u16, bool)> = flags.iter().filter(|f| usize::from(f.0) == i).map(|f| (f.1, f.2)).collect();
        let on_push = self.by_rules && mine.len() == 1;
        let mode = if on_push { push_flag(mine[0].0, mine[0].1) } else { 0 };
        self.push("PUSH", mode, Arg::Return(ret));
        if self.by_rules {
            let next = self.pc() + 1;
            self.push("GOTO", 0, Arg::Pc(next));
            self.rule = self.states[usize::from(callee)].rule;
        }
        if !on_push {
            self.flags_of(flags, i);
        }
    }

    /// Where a mark's arm goes in state `s` (`Arg::Marked`).
    fn after_mark(&self, s: u16) -> Place {
        let at = self.marked[usize::from(s)];
        if self.words.get(at as usize).is_some_and(|w| w.0 == "TOKEN") { at + 1 } else { at }
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

    /// The string of the one keyword whose kind `kind` is, where only a token of that word has
    /// it (no row's own kind): a leaf of the kind then takes it from the token's word, as the
    /// lean parser's `Kind::DotDecl + (d - W_DECL)`.
    fn word_kind(&mut self, kind: u16) -> Option<u16> {
        if self.rows.iter().any(|r| r.kind == kind) {
            return None;
        }
        let mut words = self.rows.iter().flat_map(|r| r.keywords.iter().chain(&r.soft)).filter(|&&(_, k)| k == kind).map(|(w, _)| w.clone());
        let w = words.next()?;
        if words.any(|x| x != w) {
            return None;
        }
        Some(self.string(&w))
    }

    /// A call's field, set on the node it made where it returns to state `st`
    /// (`StateRow::returned_field`): on the node made last at the level, where it has none.
    fn returned_field(&mut self, st: &StateRow) {
        if st.returned_field != 0 {
            self.push("NODE_FIELD", 4, Arg::Field(st.returned_field));
        }
    }

    /// An arm's code: its token's end, its test, acts, push and jump.
    fn arm(&mut self, arm: usize, span: Span, pre: Pre) -> Place {
        if let Some(&pc) = self.arm_code.get(&(arm, span, pre.clone())) {
            return pc;
        }
        let a = &self.arms[arm];
        let call: Option<CallCode> = a.call.then(|| (a.acts.clone(), a.to, a.push.clone(), pre.clone(), self.default, self.rule));
        if let Some(&pc) = call.as_ref().and_then(|k| self.call_code.get(k)) {
            self.arm_code.insert((arm, span, pre.clone()), pc);
            return pc;
        }
        let at = self.pc();
        self.arm_code.insert((arm, span, pre.clone()), at);
        if let Some(k) = call {
            self.call_code.insert(k, at);
        }
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
        // A token a count counts, taken here: once a token, as only an arm takes one.
        if !self.arms[arm].call {
            let token = self.arms[arm].token;
            for i in 0..self.counts.len() {
                if self.counts[i].0.contains(&token) {
                    self.push("COUNT", 0, Arg::Num(i as u16));
                } else if self.counts[i].1.contains(&token) {
                    self.push("COUNT", 1, Arg::Num(i as u16));
                }
            }
            let refuses = &self.states[usize::from(self.arms[arm].from)].refuses;
            for i in 0..self.checks.len() {
                if self.checks[i].0.contains(&token) && self.checks[i].1.is_none_or(|m| refuses.contains(&m)) {
                    self.push("CHECK", 0, Arg::Num(i as u16));
                }
            }
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
    fn body(&mut self, arm: usize) -> Place {
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
        // `refuses taken`: an arm that takes the mark asks first.
        if let Some(r) = self.marks.iter().find(|m| m.kind == token).and_then(|m| m.refuse_taken) {
            self.push("SETTLE", crate::program::SETTLE_REFUSE, Arg::Num(r));
        }
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
                    let kind = if kind != 0 { kind } else { token };
                    let kind = match self.word_kind(kind) {
                        Some(s) => {
                            self.word_kinds.insert(s, kind);
                            WORD_KIND
                        }
                        None => kind,
                    };
                    self.push("LEAF", 0, Arg::Kind(kind));
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
                // A settling mark's place: the pending extras before its bit settled.
                Act::SettleMark => {
                    let m = self.marks.iter().find(|m| m.kind == token).and_then(|m| m.settle).expect("a settling mark's mask");
                    self.push("SETTLE", 0, Arg::Num(m));
                    // `drops`: the extra pending last, where its mask holds at the token.
                    if let Some(d) = self.marks.iter().filter(|m| m.kind == token).find_map(|m| m.drop) {
                        self.push("SETTLE", 1, Arg::Num(d));
                    }
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
        self.lose_flags(&ret, &mut flags);
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
                    self.call(o, callee, &flags, i);
                }
                for _ in 0..c + u8::from(st.own) {
                    self.push("CLOSE", 1, Arg::None);
                }
                self.push("GOTO", 0, Arg::State(p));
            }
            _ => {
                for (i, &p) in ret.iter().enumerate() {
                    let callee = ret.get(i + 1).copied().unwrap_or(to);
                    self.call(p, callee, &flags, i);
                }
                if let Some(k) = remember {
                    // A keyword's: its kind from the token's word, as a keyword's leaf's.
                    let k = match self.word_kind(k) {
                        Some(s) => {
                            self.word_kinds.insert(s, k);
                            WORD_KIND
                        }
                        None => k,
                    };
                    self.push("REMEMBER", 0, Arg::Kind(k));
                }
                let marked = self.marks.iter().any(|m| m.kind == token);
                // One way into a callee, as a function has one entry: its `SWITCH` where every
                // call's caller has tested each mark it tests and none is on a mark; else past
                // its `TOKEN`, where it tests its marks (a mark its caller tested and did not
                // take, it passes; one the caller took to call it, it takes).
                let again = if self.enters_at_switch(usize::from(to)) { Arg::Again(to) } else { Arg::Past(to) };
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
    fn by_words(&mut self, t: usize, b: u8, cands: Vec<Cand>) -> Result<Place, String> {
        let row = &self.rows[cands[0].row];
        let (one_byte, takes_last) = (row.one_byte, row.takes_last());
        let check_refuses = row.check.is_some_and(|first| !has(&first, b));
        // A row of its own kind (`Row::moved`): a keyword read as it keeps the row's kind, so it
        // reserves none.
        let moved = row.moved;
        let missing: Vec<Vec<u8>> = row.keywords.iter().map(|(w, _)| w.clone()).filter(|w| !moved && w[0] == b && !cands.iter().any(|c| c.word.as_ref() == Some(w))).collect();
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

    /// `by_words` over rows `met` that begin with byte `b`, all but at most one of keywords only:
    /// each row's keywords named (its arm where one takes it, else the state's default), and a
    /// token that is no keyword the other row's.
    fn by_words_of(&mut self, t: usize, b: u8, met: &[usize], cands: Vec<Cand>) -> Result<Place, String> {
        let mut plain_row: Option<usize> = None;
        for &r in met {
            let row = &self.rows[r];
            if row.kind != 0 {
                plain_row = Some(r);
            }
            let takes_last = row.takes_last();
            // A row of its own kind (`Row::moved`) reserves no keyword (`by_words`).
            let keywords: Vec<Vec<u8>> = row.keywords.iter().filter(|(w, _)| !row.moved && w[0] == b).map(|(w, _)| w.clone()).collect();
            let span_of = |w: &[u8]| if takes_last { Span::Last } else { Span::Len(w.len() as u16) };
            for w in &keywords {
                let s = self.string(w);
                let to = match cands.iter().find(|c| c.row == r && c.word.as_ref() == Some(w)) {
                    Some(c) => self.arm(c.arm, span_of(w), Pre::None),
                    None => self.default,
                };
                self.name_word(t, s, to)?;
            }
            for c in cands.iter().filter(|c| c.row == r && c.word.is_some() && !keywords.iter().any(|k| Some(k) == c.word.as_ref())) {
                let w = c.word.clone().expect("a soft keyword");
                let s = self.string(&w);
                let to = self.arm(c.arm, span_of(&w), Pre::None);
                self.name_word(t, s, to)?;
            }
        }
        let Some(p) = plain_row else { return Ok(self.default) };
        let row = &self.rows[p];
        let (one_byte, check_refuses) = (row.one_byte, row.check.is_some_and(|first| !has(&first, b)));
        Ok(match cands.iter().find(|c| c.row == p && c.word.is_none()) {
            _ if check_refuses => REFUSE,
            Some(c) => self.arm(c.arm, if one_byte { Span::Len(1) } else { Span::Last }, Pre::None),
            None => self.default,
        })
    }

    /// Switch `t` names word `s` with target `to`, kept in order.
    fn name_word(&mut self, t: usize, s: u16, to: Place) -> Result<(), String> {
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

    fn target(&mut self, b: u8, depth: usize, cands: Vec<Cand>) -> Result<Place, String> {
        let row = &self.rows[cands[0].row];
        let plain: Vec<&Cand> = cands.iter().filter(|c| c.word.is_none()).collect();
        let words: Vec<&Cand> = cands.iter().filter(|c| c.word.is_some()).collect();
        // The row's keywords beginning with the byte that no arm here takes: a token that is one
        // has the keyword's kind, not the row's, so the row's own arm must not take it.
        let missing: Vec<Vec<u8>> = row
            .keywords
            .iter()
            .map(|(w, _)| w.clone())
            .filter(|w| depth == 0 && !row.moved && w[0] == b && !words.iter().any(|c| c.word.as_ref() == Some(w)))
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
                self.words[jump as usize].2 = Arg::Pc(to);
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
            self.words[jump as usize].2 = Arg::Pc(to);
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
                self.words[jump as usize].2 = Arg::Pc(to);
            }
            return Ok(at);
        }
        let t = self.switches.len();
        self.switches.push(Switch::refusing(next as u8));
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
    fn mark_tests_here(&mut self, tests: &[(usize, AtMark)], mark_arms: &mut Vec<(Place, usize)>, to_end: &mut Vec<Place>) {
        for &(k, at) in tests {
            let arm = match at {
                AtMark::Arm(a) => Some(a),
                _ => None,
            };
            if let Some(w) = self.marks[k].word.clone() {
                let s = self.string(&w);
                self.mark_words.insert(k as u8, s);
            }
            let m = &self.marks[k];
            // Its order, and 32 when the end has it.
            // An arm that calls: the mark kept, for the callee, which tests it again and takes it.
            // At a level's end, kept: the caller takes it.
            let keep = arm.is_some_and(|a| self.arms[a].call) || at == AtMark::End;
            // `loses`: asked here, where its mask holds (and so the mark does not), the extra
            // before the token has no width.
            // At the mark's level (it takes the mark): always; where the level may end at it,
            // where the flag says a caller's level is the mark's; never where it refuses.
            // `refuses`: asked first, the same way.
            let refuse = crate::program::SETTLE_REFUSE;
            match (m.refuse, at) {
                (Some(r), AtMark::Arm(_)) => self.push("SETTLE", refuse, Arg::Num(r)),
                (Some(r), AtMark::End) => self.push("SETTLE", refuse + 1 + m.lose_flag as u8, Arg::Num(r)),
                _ => {}
            }
            let m = &self.marks[k];
            match (m.lose, at) {
                (Some(l), AtMark::Arm(_)) => self.push("SETTLE", 2, Arg::Num(l)),
                (Some(l), AtMark::End) => self.push("SETTLE", 3 + m.lose_flag as u8, Arg::Num(l)),
                _ => {}
            }
            let m = &self.marks[k];
            self.push("MARK", k as u8 | if m.end { crate::program::MARK_END } else { 0 } | if keep { MARK_KEEP } else { 0 }, Arg::Num(m.mask));
            match at {
                AtMark::Arm(a) => {
                    mark_arms.push((self.pc(), a));
                    self.push("GOTO", 0, Arg::Pc(0));
                }
                AtMark::End => {
                    to_end.push(self.pc());
                    self.push("GOTO", 0, Arg::Pc(0));
                }
                AtMark::Refuse => self.push("STOP", 0, Arg::None),
            }
        }
    }

    /// Whether every call into state `s` may enter it at its `SWITCH` (`Arg::Again`).
    fn enters_at_switch(&self, s: usize) -> bool {
        self.at_switch[s]
    }


    /// Whether state `s` only calls, so its switch decides nothing its callee's does not: every
    /// arm is the same call (its acts, callee and returns), the state may not end, the callee's
    /// start has an arm on each token it has and on no other, meets the same rows, and tests each
    /// mark the state tests to the same end (a mark arm the callee takes; a refusal it makes).
    /// Then a token the state refuses, the callee refuses, as the lean parser's `unary()` refuses
    /// what no operand starts with.
    fn direct_call(&self, s: usize) -> bool {
        let mine: Vec<&ArmRow> = self.arms.iter().filter(|a| usize::from(a.from) == s).collect();
        let Some(first) = mine.first() else { return false };
        let to = usize::from(first.to);
        if !self.by_rules || self.states[s].end.is_some() || to == s || !mine.iter().all(|a| a.call && a.to == first.to && a.push == first.push && a.acts == first.acts) {
            return false;
        }
        let mut here: Vec<u16> = mine.iter().map(|a| a.token).collect();
        let mut there: Vec<u16> = self.arms.iter().filter(|a| usize::from(a.from) == to).map(|a| a.token).collect();
        here.sort_unstable();
        there.sort_unstable();
        if here != there || self.meets[s] != self.meets[to] {
            return false;
        }
        let callee = self.tests_at(to);
        self.tests_at(s).iter().all(|&(k, at)| {
            let theirs = callee.iter().find(|t| t.0 == k).map(|t| t.1);
            match at {
                AtMark::Arm(_) => matches!(theirs, Some(AtMark::Arm(_))),
                AtMark::Refuse => theirs == Some(AtMark::Refuse),
                AtMark::End => false,
            }
        })
    }

    /// State `s`'s mark tests, in order: each mark it takes, a test and its arm; each not hidden
    /// that it does not take, and each its level refuses, a refusal; a mark that ends levels,
    /// or one not hidden, where this nested level may end and takes none, its end.
    /// Entered only by taking a mark, at the token it was taken at: the level has made the
    /// gap's decision there, so a hidden mark that ends levels does not end it, as the lean
    /// parser asks `asi` once at a line break.
    fn tests_at(&self, s: usize) -> Vec<(usize, AtMark)> {
        let st = &self.states[s];
        let nested = st.level != self.root;
        let mine: Vec<usize> = (0..self.arms.len()).filter(|&a| usize::from(self.arms[a].from) == s).collect();
        let mark_kinds: Vec<u16> = self.marks.iter().map(|m| m.kind).collect();
        let into: Vec<&ArmRow> = self.arms.iter().filter(|a| usize::from(a.to) == s).collect();
        let returned_to = self.arms.iter().any(|a| a.push.iter().any(|&p| usize::from(p) == s));
        let after_mark_only = !into.is_empty() && !returned_to && into.iter().all(|a| !a.call && mark_kinds.contains(&a.token));
        (0..self.marks.len())
            .filter_map(|k| {
                let m = &self.marks[k];
                let arm = mine.iter().copied().find(|&a| self.arms[a].token == m.kind);
                if refusing(m, self.arms) {
                    return None;
                }
                // A mark not hidden, as one that ends levels, ends a nested level that may end
                // here: its caller takes it or refuses the file (the reference's `past_marks`).
                // Elsewhere, it refuses the file; so does a mark the level refuses.
                match arm {
                    Some(a) => Some((k, AtMark::Arm(a))),
                    None if st.refuses.contains(&m.kind) => Some((k, AtMark::Refuse)),
                    None if (m.ends && !after_mark_only || !m.hidden || st.follows.contains(&m.kind)) && nested && st.end.is_some() => Some((k, AtMark::End)),
                    None if !m.hidden => Some((k, AtMark::Refuse)),
                    None => None,
                }
            })
            .collect()
    }

    /// The flags of the marks that lose (`Mark::lose_flag`) each call of an arm runs with, call
    /// `i` returning to state `ret[i]`: set where that state takes the mark (the level it returns
    /// to is the mark's), kept where it ends at it (its caller decides), else cleared.
    fn lose_flags(&self, ret: &[u16], flags: &mut Vec<(u8, u16, bool)>) {
        for (k, m) in self.marks.iter().enumerate().filter(|(_, m)| m.lose.is_some() || m.refuse.is_some()) {
            for (i, &p) in ret.iter().enumerate() {
                match self.tests_at(usize::from(p)).iter().find(|t| t.0 == k).map(|t| t.1) {
                    Some(AtMark::Arm(_)) => flags.push((i as u8, m.lose_flag, true)),
                    Some(AtMark::End) => {}
                    _ => flags.push((i as u8, m.lose_flag, false)),
                }
            }
        }
    }

    fn state(&mut self, s: usize) -> Result<(), String> {
        let st = self.states[s].clone();
        self.rule = st.rule;
        let mine: Vec<usize> = (0..self.arms.len()).filter(|&a| usize::from(self.arms[a].from) == s).collect();
        let nested = st.level != self.root;
        self.entry[s] = self.pc();
        self.marked[s] = self.pc();
        if nested && mine.is_empty() {
            let Some(c) = st.end else { return Err(format!("state {s} takes no token and may not end: {st:?}")) };
            // A nested choice's level has no node of its own to close. Its nodes end at the
            // token taken (`CLOSE m=1`); after a mark's arm, which took none, at the last token
            // taken (`m=0`).
            let marks: Vec<u16> = self.marks.iter().map(|m| m.kind).collect();
            let after_mark = self.arms.iter().any(|a| usize::from(a.to) == s && !a.call && marks.contains(&a.token));
            self.returned_field(&st);
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
        let extras = if self.token_extras && (s != 0 || self.whole) { crate::program::TOKEN_EXTRAS } else { 0 };
        self.push("TOKEN", self.any_mark.map_or(0, |k| k as u8 + 1) | extras, Arg::Num(STARTS));
        // Past the `TOKEN`, where both returns land (`m=0` at it, `m=1` past it).
        self.returned_field(&st);
        if self.direct[s] {
            // The call at once: no switch, no mark test (`direct_call`).
            self.switch_at[s] = self.pc();
            self.default = REFUSE;
            let jump = self.pc();
            self.push("GOTO", 0, Arg::Pc(0));
            let at = self.arm(mine[0], Span::Last, Pre::None);
            self.words[jump as usize].2 = Arg::Pc(at);
            return Ok(());
        }
        // The marks, in order: each the state takes, a test and a jump to its arm; each not
        // hidden that it does not take, and each its level refuses, a test that refuses. A mark
        // that ends levels (`Mark::ends`), or one not hidden, where this nested level may end
        // and takes none: a test that ends it.
        // Entered only by taking a mark, at the token it was taken at: the level has made the
        // gap's decision there, so a hidden mark that ends levels does not end it, as the lean
        // parser asks `asi` once at a line break.
        // A hidden mark only some of the level's uses have after them (`StateRow::split`):
        // where the level may end, it may hold at no token the state takes. There ending at it
        // and skipping it are the same (the token is no arm's: the level ends, and its caller
        // takes the mark or skips it); at a token the state takes, one call can not end after
        // some uses and read on after others.
        if nested && st.end.is_some() && !st.split.is_empty() {
            for m in self.marks.iter().filter(|m| st.split.contains(&m.kind) && !mine.iter().any(|&a| self.arms[a].token == m.kind)) {
                let taken = (0..=255u8).find(|&b| {
                    has(&m.first, b)
                        && mine.iter().any(|&a| {
                            let kind = self.arms[a].token;
                            self.rows.iter().any(|row| (row.kind == kind && row.kind != 0 && has(&row.first, b)) || row.keywords.iter().chain(&row.soft).any(|(w, k)| *k == kind && w[0] == b))
                        })
                });
                if let Some(b) = taken {
                    return Err(format!(
                        "state {s}: a call whose uses differ in the hidden mark of kind {} after them may end here and take a token it may hold at (byte {b:#04x}): one call can not end at it after some uses and read on after others; make the rule `nested`, or give its uses one follow",
                        m.kind
                    ));
                }
            }
        }
        let tests = self.tests_at(s);
        // A refusal tested on its token's bytes only (`Mark::first`), in the switch's arms for
        // them, as the lean parser looks for a mark where it reads its token: where every test
        // after it refuses too, or is of a mark that never holds at a token it holds at (their
        // first bytes apart): refusals commute, and so do two tests never both met at a token;
        // a jump to a mark's arm, or a level's end, must stay after one that may hold with it.
        // Not a mark the end has (the switch's end is not a byte's).
        // A level's end of a mark of words (`Mark::words`) moves the same way, into the arms of
        // its words, as the lean parser stops a loop at the token it reads (`i != type_stop` at a
        // `.`): a token no arm takes ends the level anyway (the switch's default). It moves past
        // tests of marks apart from it only: a mark's arm, an end or a refusal that holds with it
        // must still come first or after as before.
        let apart = |a: usize, b: usize| {
            let (x, y) = (&self.marks[a], &self.marks[b]);
            (0..4).all(|i| x.first[i] & y.first[i] == 0) || matches!((&x.words, &y.words), (Some(p), Some(q)) if p.iter().all(|w| !q.contains(w)))
        };
        let moved: Vec<(usize, AtMark)> = (0..tests.len())
            .filter(|&i| {
                let (k, at) = tests[i];
                let later = &tests[i + 1..];
                !self.marks[k].end
                    && match at {
                        AtMark::Refuse => later.iter().all(|&(j, a)| a == AtMark::Refuse || apart(k, j)),
                        AtMark::End => nested && self.marks[k].words.is_some() && later.iter().all(|&(j, _)| apart(k, j)),
                        AtMark::Arm(_) => false,
                    }
            })
            .map(|i| tests[i])
            .collect();
        let tests: Vec<(usize, AtMark)> = tests.into_iter().filter(|t| !moved.contains(t)).collect();
        let mut mark_arms = Vec::new();
        let mut to_end = Vec::new();
        self.mark_tests_here(&tests, &mut mark_arms, &mut to_end);
        let t = self.switches.len();
        self.switches.push(Switch::refusing(0));
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
        for jump in to_end {
            self.words[jump as usize].2 = Arg::Pc(end);
        }
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
                let own = (0..=255u8).find(|&b| row.moved && has(&row.first, b));
                for (w, k) in row.keywords.iter().chain(&row.soft) {
                    let at = if has(&row.first, w[0]) { Some(w[0]) } else { own };
                    if let Some(at) = at.filter(|_| *k == kind) {
                        cands[usize::from(at)].push(Cand { arm: a, row: r, word: Some(w.clone()) });
                    }
                }
            }
        }
        for b in 0..=255u8 {
            // An arm whose row the state cannot meet is dead (an extra's in a string, say).
            let mut cs = std::mem::take(&mut cands[usize::from(b)]);
            cs.retain(|c| self.meets[s][c.row]);
            // A part's own byte (`Row::moved`) where the state meets none of its rows: no token
            // of that kind comes here (it is read only inside its region), so none is taken.
            let rows_at: Vec<usize> = (0..self.rows.len()).filter(|&r| has(&self.rows[r].first, b)).collect();
            let unmet_part = !rows_at.is_empty() && rows_at.iter().all(|&r| self.rows[r].moved && !self.meets[s][r]);
            if cs.is_empty() {
                self.switches[t].targets[usize::from(b)] = if unmet_part { REFUSE } else { default };
                continue;
            }
            // The rows the state can meet that begin with the byte (`meets`): when more than the
            // one it takes, a test of each one it takes (`ROW`, its start mask) and its jump; the
            // last untested when the state takes them all. Any other goes to the default.
            let mut taken: Vec<usize> = cs.iter().map(|c| c.row).collect();
            taken.sort_unstable();
            taken.dedup();
            let met: Vec<usize> = (0..self.rows.len()).filter(|&r| has(&self.rows[r].first, b) && self.meets[s][r]).collect();
            // Rows tier 2 reads alike (`Row::alike`, one kind's in several regions): one row, its
            // arms taken together, as the lean lexer's one `T` a kind.
            if met.len() > 1 && met.iter().all(|&r| self.rows[r].alike(&self.rows[met[0]]) && cs.iter().any(|c| c.row == r)) {
                let mut one: Vec<Cand> = Vec::new();
                for mut c in cs {
                    c.row = met[0];
                    if !one.iter().any(|o| o.arm == c.arm && o.word == c.word) {
                        one.push(c);
                    }
                }
                // As a row tested would be (`target`): a moved row's words its own switch's.
                let pc = self.target(b, 0, one)?;
                self.switches[t].targets[usize::from(b)] = pc;
                continue;
            }
            // Rows of keywords only beside at most one other: the record's word tells them apart,
            // as the lean parser's `peek()` gives the token's kind (a `.` or a `.5`).
            if met.len() > 1 && met.iter().filter(|&&r| self.rows[r].kind != 0).count() <= 1 {
                let pc = self.by_words_of(t, b, &met, cs)?;
                self.switches[t].targets[usize::from(b)] = pc;
                continue;
            }
            if met.len() > 1 {
                let all = met.iter().all(|r| taken.contains(r));
                let mut tos = Vec::new();
                for &r in &taken {
                    let these: Vec<Cand> = cs.iter().filter(|c| c.row == r).cloned().collect();
                    tos.push(self.target(b, 0, these)?);
                }
                // Every row it can meet there it takes, to one place: no test tells them apart. So
                // too where the rows it takes go where the others do.
                if all && tos.iter().all(|&t| t == tos[0]) || !all && tos.iter().all(|&t| t == default) {
                    self.switches[t].targets[usize::from(b)] = if all { tos[0] } else { default };
                    continue;
                }
                let key = (self.rule, taken.clone(), all, default, tos.clone());
                if let Some(&at) = self.row_chains.get(&key) {
                    self.switches[t].targets[usize::from(b)] = at;
                    continue;
                }
                let at = self.pc();
                for (i, (&r, &to)) in taken.iter().zip(&tos).enumerate() {
                    if !(all && i + 1 == taken.len()) {
                        self.tested.insert(r);
                        let m = match self.rows[r].mask {
                            Some(m) => m,
                            None if self.survey => 0,
                            None => return Err(format!("row {r} shares byte {b:#04x} and has no start mask")),
                        };
                        self.push("ROW", 0, Arg::Num(m));
                    }
                    if to == REFUSE {
                        self.push("STOP", 0, Arg::None);
                    } else {
                        self.push("GOTO", 0, Arg::Pc(to));
                    }
                }
                if !all {
                    if default == REFUSE {
                        self.push("STOP", 0, Arg::None);
                    } else {
                        self.push("GOTO", 0, Arg::Pc(default));
                    }
                }
                self.row_chains.insert(key, at);
                self.switches[t].targets[usize::from(b)] = at;
                continue;
            }
            let pc = self.by_words(t, b, cs)?;
            self.switches[t].targets[usize::from(b)] = pc;
        }
        // The moved refusals: each target of a byte one of them can begin a token with gets
        // them first, once a target and set of marks.
        // A mark of words is tested at its words: a word of more than one byte by its entry in
        // the switch (named here where it is not yet, to the target its byte gives), one of one
        // byte at that byte. Any other at the bytes it may begin with (`Mark::first`).
        if !moved.is_empty() {
            let long = |e: &Self, k: usize| e.marks[k].words.as_ref().map(|ws| ws.iter().filter(|w| w.len() > 1).cloned().collect::<Vec<_>>());
            let mut named: Vec<u16> = Vec::new();
            for &(k, _) in &moved {
                for w in long(self, k).unwrap_or_default() {
                    let s = self.string(&w);
                    if self.switches[t].words.binary_search_by_key(&s, |&(x, _)| x).is_err() {
                        let to = self.switches[t].targets[usize::from(w[0])];
                        self.name_word(t, s, to)?;
                        named.push(s);
                    }
                }
            }
            let mut made: HashMap<(Place, Vec<(usize, AtMark)>), Place> = HashMap::default();
            let mut ends: Vec<Place> = Vec::new();
            // The tests at a byte's target (`word` None) or a word's (`Some`).
            let mut wrap = |e: &mut Self, b: u8, word: Option<&[u8]>, old: Place| -> Place {
                let these: Vec<(usize, AtMark)> = moved
                    .iter()
                    .copied()
                    .filter(|&(k, at)| {
                        let m = &e.marks[k];
                        let here = match (&m.words, word) {
                            (Some(ws), Some(w)) => ws.iter().any(|x| x.as_slice() == w),
                            (Some(ws), None) => ws.iter().any(|x| x.len() == 1 && x[0] == b),
                            (None, _) => has(&m.first, b),
                        };
                        here && !(at == AtMark::End && old == end)
                    })
                    .collect();
                if old == REFUSE || these.is_empty() {
                    return old;
                }
                if let Some(&pc) = made.get(&(old, these.clone())) {
                    return pc;
                }
                let at = e.pc();
                e.mark_tests_here(&these, &mut Vec::new(), &mut ends);
                e.push("GOTO", 0, Arg::Pc(old));
                made.insert((old, these), at);
                at
            };
            for b in 0..=255u8 {
                let old = self.switches[t].targets[usize::from(b)];
                let new = wrap(self, b, None, old);
                self.switches[t].targets[usize::from(b)] = new;
            }
            for i in 0..self.switches[t].words.len() {
                let (w, old) = self.switches[t].words[i];
                let text = self.strings[usize::from(w)].clone();
                let new = wrap(self, text[0], Some(&text), old);
                self.switches[t].words[i].1 = new;
            }
            for jump in ends {
                self.words[jump as usize].2 = Arg::Pc(end);
            }
            // A word named here for its tests that has none: its byte's target is its own.
            let (targets, strings) = (&self.switches[t].targets, &self.strings);
            let keep: Vec<(u16, Place)> = self.switches[t].words.iter().copied().filter(|&(w, to)| !(named.contains(&w) && to == targets[usize::from(strings[usize::from(w)][0])])).collect();
            self.switches[t].words = keep;
        }
        // The marks' arms: their acts, no bytes taken (the next state switches on this token).
        for (jump, a) in mark_arms {
            let to = self.body(a);
            self.words[jump as usize].2 = Arg::Pc(to);
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
/// A token mark the grammar takes nowhere, not hidden: wherever it holds the file is refused,
/// at every level. `TOKEN` tests it, once a token (`program::TokenMark::refuses`), and no
/// state does: a test of it in each state would be two words a state.
fn refusing(m: &Mark, arms: &[ArmRow]) -> bool {
    (m.tokens.is_some() || matches!(m.nexts.as_slice(), [(false, _)])) && !m.hidden && !m.end && arms.iter().all(|a| a.token != m.kind)
}

/// The marks each state tests after its `TOKEN`, in order: each it takes, and each not hidden
/// or that its level refuses (no refusing token mark: `TOKEN` tests those).
fn tests_of(states: &[StateRow], arms: &[ArmRow], marks: &[Mark]) -> Vec<Vec<usize>> {
    let mut takes: Vec<Vec<u16>> = vec![Vec::new(); states.len()];
    for a in arms {
        takes[usize::from(a.from)].push(a.token);
    }
    states
        .iter()
        .enumerate()
        .map(|(s, st)| (0..marks.len()).filter(|&k| !refusing(&marks[k], arms) && (takes[s].contains(&marks[k].kind) || !marks[k].hidden || st.refuses.contains(&marks[k].kind) || (marks[k].ends || st.follows.contains(&marks[k].kind)) && st.end.is_some())).collect())
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub fn lower(states: &[StateRow], arms: &[ArmRow], rows: &[Row], marks: &[Mark], any_mark: Option<u16>, meets: &[Vec<bool>], empties: u16, whole: bool, survey: bool, rules: bool, token_extras: bool, counts: &[(Vec<u16>, Vec<u16>)], checks: &[(Vec<u16>, Option<u16>)]) -> Result<Lowered, String> {
    let mut e = Emit {
        counts: counts.to_vec(),
        checks: checks.to_vec(),
        token_extras,
        whole,
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
        tests_of: tests_of(states, arms, marks),
        at_switch: Vec::new(),
        direct: Vec::new(),
        arm_code: HashMap::default(),
        call_code: HashMap::default(),
        row_chains: HashMap::default(),
        mark_words: Default::default(),
        word_kinds: Default::default(),
    };
    // Every call into a state enters at its `SWITCH` where its caller has tested each mark it
    // tests and the call is on no mark; else past its `TOKEN`, where it tests its marks.
    e.at_switch = vec![true; states.len()];
    for a in arms.iter().filter(|a| a.call) {
        let (to, from) = (usize::from(a.to), usize::from(a.from));
        if marks.iter().any(|m| m.kind == a.token) || !e.tests_of[to].iter().all(|k| e.tests_of[from].contains(k)) {
            e.at_switch[to] = false;
        }
    }
    // A state that only calls: the call at once, its callee entered past its `TOKEN`, where it
    // tests its marks, as the caller tested none.
    e.direct = (0..states.len()).map(|s| e.direct_call(s)).collect();
    for s in (0..states.len()).filter(|&s| e.direct[s]) {
        let to = arms.iter().find(|a| usize::from(a.from) == s).map(|a| usize::from(a.to)).expect("an arm");
        e.at_switch[to] = false;
    }
    // The keywords' strings first, in the order of their kinds, as the lean parser numbers its
    // words: a keyword's leaf kind is then its word plus a constant over each run of kinds
    // (`word_kind`), one add, not a compare tree.
    let mut keywords: Vec<(u16, Vec<u8>)> = rows.iter().flat_map(|r| r.keywords.iter().chain(&r.soft)).map(|(w, k)| (*k, w.clone())).collect();
    keywords.sort();
    for (_, w) in &keywords {
        e.string(w);
    }
    // The token marks' words, and their elements by offset.
    let mut token_marks = Vec::new();
    for (k, m) in marks.iter().enumerate() {
        // A token mark's run; a gap mark's `before`s and `unless before`s (decision 0416), each
        // an entry of the mark's order.
        let none = Vec::new();
        let patterns: Vec<(bool, &Vec<::bitlex::syntax::TokenElem>, &Vec<Vec<::bitlex::syntax::TokenElem>>)> = match &m.tokens {
            // A token mark's `unless before`s after its run (`Mark::nexts`).
            Some((before, seq)) => std::iter::once((false, before, seq)).chain(m.nexts.iter().map(|(unless, seq)| (*unless, &none, seq))).collect(),
            None => m.nexts.iter().map(|(unless, seq)| (*unless, &none, seq)).collect(),
        };
        for (unless, before, seq) in patterns {
            let mut elems = Vec::new();
            let at = std::iter::once((-1i8, before)).filter(|(_, b)| !b.is_empty()).chain(seq.iter().enumerate().map(|(i, x)| (i as i8, x)));
            for (offset, choices) in at {
                let (mut kinds, mut words, mut end, mut without) = (Vec::new(), Vec::new(), false, Vec::new());
                let none_nest = Vec::new();
                let none_ending = Vec::new();
                let (not, run, choices, nests, ending) = match choices.as_slice() {
                    [::bitlex::syntax::TokenElem::Not(inner)] => (true, false, inner.as_slice(), &none_nest, &none_ending),
                    // A run of tokens none of the choices.
                    [::bitlex::syntax::TokenElem::Many(inner, nests, ending)] => match inner.as_slice() {
                        [::bitlex::syntax::TokenElem::Not(none)] => (true, true, none.as_slice(), nests, ending),
                        _ => (false, true, inner.as_slice(), nests, ending),
                    },
                    c => (false, false, c, &none_nest, &none_ending),
                };
                // The run's `ending` (Q342).
                let ending_row = if ending.is_empty() {
                    None
                } else {
                    let (mut ks, mut ws) = (Vec::new(), Vec::new());
                    for c in ending {
                        match c {
                            ::bitlex::syntax::TokenElem::Kind(name) => ks.push(name.clone()),
                            ::bitlex::syntax::TokenElem::Word(w) => ws.push(e.string(w)),
                            _ => return Err("a run's `ending` of kinds and words".into()),
                        }
                    }
                    Some((ks, ws))
                };
                // The run's depths (Q339): each side its counter's.
                let mut nest_rows = Vec::new();
                for (k, n) in nests.iter().enumerate() {
                    for (open, side) in [(true, &n.open), (false, &n.close)] {
                        let (mut ks, mut ws) = (Vec::new(), Vec::new());
                        for c in side {
                            match c {
                                ::bitlex::syntax::TokenElem::Kind(name) => ks.push(name.clone()),
                                ::bitlex::syntax::TokenElem::Word(w) => ws.push(e.string(w)),
                                _ => return Err("a depth's side of kinds and words".into()),
                            }
                        }
                        nest_rows.push((k as u8, open, ks, ws));
                    }
                }
                for c in choices {
                    match c {
                        ::bitlex::syntax::TokenElem::Kind(name) => kinds.push(name.clone()),
                        ::bitlex::syntax::TokenElem::Word(w) => words.push(e.string(w)),
                        ::bitlex::syntax::TokenElem::End => end = true,
                        ::bitlex::syntax::TokenElem::Without(m) => without.push((m.clone(), false)),
                        ::bitlex::syntax::TokenElem::With(m) => without.push((m.clone(), true)),
                        ::bitlex::syntax::TokenElem::Not(_) | ::bitlex::syntax::TokenElem::Many(..) => return Err("a `~` or a run inside a `~` or a run".into()),
                        ::bitlex::syntax::TokenElem::Starting(_) => return Err("`starting` in a mark's `drops` or `loses` only".into()),
                    }
                }
                // The token before, `~`: it holds with no token before too (`end` there).
                let end = end || (offset == -1 && not);
                elems.push((offset, kinds, words, not, end, run, nest_rows, ending_row, without));
            }
            // A refusing one's word: its bit in the record is the word's alone (no state tests
            // it, so no `MARK` names the word).
            if let (true, Some(w)) = (refusing(m, arms), &m.word) {
                let s = e.string(w);
                e.mark_words.insert(k as u8, s);
            }
            token_marks.push((k as u8, refusing(m, arms), unless, elems));
        }
    }
    for w in marks.iter().flat_map(|m| m.unless.iter()) {
        e.string(w);
    }
    // The `drops` and `loses` questions, a pattern each (`bitlex_masks` numbers their entries
    // in this order), their elements at offsets from the token: `after`'s before it, the last
    // at -1; `before`'s from it.
    let mut gap_patterns = Vec::new();
    for (p, qs) in marks.iter().flat_map(|m| [&m.drops, &m.loses, &m.refuses, &m.refuses_taken]).filter(|q| !q.is_empty()).enumerate() {
        for q in qs.iter() {
            // `since`: `after`'s from the grammar token before (0).
            let a = q.after.len() as i8;
            let at = q.after.iter().enumerate().map(|(i, x)| if q.since { (i as i8, true, x) } else { (i as i8 - a, false, x) }).chain(q.before.iter().enumerate().map(|(i, x)| (i as i8, false, x)));
            let mut elems = Vec::new();
            for (offset, since, choices) in at {
                let (not, choices) = match choices.as_slice() {
                    [::bitlex::syntax::TokenElem::Not(inner)] => (true, inner.as_slice()),
                    c => (false, c),
                };
                let (mut kinds, mut words, mut marks_named, mut first) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
                for c in choices {
                    match c {
                        ::bitlex::syntax::TokenElem::Kind(name) => kinds.push(name.clone()),
                        ::bitlex::syntax::TokenElem::Word(w) => words.push(e.string(w)),
                        ::bitlex::syntax::TokenElem::With(m) => marks_named.push((m.clone(), true)),
                        ::bitlex::syntax::TokenElem::Without(m) => marks_named.push((m.clone(), false)),
                        ::bitlex::syntax::TokenElem::Starting(b) => first.extend_from_slice(b),
                        _ => return Err("a gap pattern's element of kinds, words, `with`, `without` and `starting`".into()),
                    }
                }
                elems.push((offset, kinds, words, not, marks_named, first, since));
            }
            gap_patterns.push((p as u8, q.unless, elems));
        }
    }
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
    // What made the words so big, before they are shared (`merge_words`): told with a program
    // past `u16`'s places, which is refused once laid out, or on stderr at any size with
    // `RERAIL_TIER2_SIZES` (as a grammar's author compares two).
    let sizes = |e: &Emit| -> String {
        let mut by_rule: std::collections::BTreeMap<usize, usize> = std::collections::BTreeMap::new();
        for r in &e.rule_of {
            *by_rule.entry(usize::from(*r)).or_default() += 1;
        }
        // (words, the rule's level's kind, its states)
        let mut top: Vec<(usize, u16, usize)> = by_rule.into_iter().map(|(r, n)| (n, states.iter().find(|s| usize::from(s.rule) == r).map_or(0, |s| s.level), states.iter().filter(|s| usize::from(s.rule) == r).count())).collect();
        top.sort_by_key(|t| std::cmp::Reverse(t.0));
        let mut ops: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
        for w in &e.words {
            *ops.entry(w.0).or_default() += 1;
        }
        let mut ops: Vec<_> = ops.into_iter().collect();
        ops.sort_by_key(|o| std::cmp::Reverse(o.1));
        format!("{} words unshared, {} states, {} arms; by opcode {:?}; words, level kind and states by rule: {:?}", e.words.len(), states.len(), arms.len(), &ops[..ops.len().min(10)], &top[..top.len().min(12)])
    };
    // A return lands on a state's `TOKEN` (`RETURN m=0`, the token taken) or its `SWITCH`
    // (`m=1`, to switch on it again). A state that takes no token, which closes at once, gets a
    // return entry of that shape: `TOKEN`, a `SWITCH` whose every way is the end, and the end,
    // its closes at the end of the token before and a return to switch on this one again.
    let mut returns: HashMap<u16, Place> = HashMap::default();
    for p in arms.iter().flat_map(|a| a.push.iter().copied()) {
        if e.words[e.entry[usize::from(p)] as usize].0 == "TOKEN" || returns.contains_key(&p) {
            continue;
        }
        let st = &states[usize::from(p)];
        let Some(c) = st.end else {
            let callers: Vec<&ArmRow> = arms.iter().filter(|a| a.push.contains(&p)).take(3).collect();
            return Err(format!("state {p} takes no token and may not end: {st:?}; returned to by {callers:?}"));
        };
        e.rule = st.rule;
        let at = e.pc();
        let any = e.any_mark.map_or(0, |k| k as u8 + 1) | if e.token_extras { crate::program::TOKEN_EXTRAS } else { 0 };
        e.push("TOKEN", any, Arg::Num(STARTS));
        e.returned_field(st);
        let t = e.switches.len();
        e.push("SWITCH", 0, Arg::Table(t as u16));
        let end = e.pc();
        e.switches.push(Switch { offset: 0, targets: vec![end; 256], end, words: Vec::new() });
        for _ in 0..c + u8::from(st.own) {
            e.push("CLOSE", 0, Arg::None);
        }
        e.push("RETURN", 1, Arg::None);
        returns.insert(p, at);
    }
    let unshared = sizes(&e);
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
        let mut new_at: Vec<Place> = vec![0; placed.len()];
        for (k, &pc) in order.iter().enumerate() {
            new_at[pc] = k as Place;
        }
        let to = |t: Place| if t == REFUSE || t as usize >= new_at.len() { t } else { new_at[t as usize] };
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
    // A word's place is a `u16` in the program.
    if words.len() >= usize::from(u16::MAX) - 1 {
        return Err(format!("tier 2's program is past {} words: {} words; {unshared}", u16::MAX - 2, words.len()));
    }
    if std::env::var_os("RERAIL_TIER2_SIZES").is_some() {
        eprintln!("tier 2: {} words; {unshared}", words.len());
    }
    e.words = words;
    let switches: Vec<ByteSwitch> = switches.iter().map(Switch::narrow).collect();
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
    for (at, s) in switches.iter().enumerate() {
        crate::rir_text::switch_rows(&mut text, at, s);
    }
    let mark_words: Vec<(u8, u16)> = e.mark_words.iter().map(|(&k, &s)| (k, s)).collect();
    crate::rir_text::mark_word_rows(&mut text, &mark_words);
    let mark_passes: Vec<(u8, u64)> = marks.iter().enumerate().filter(|(_, m)| m.passes != 0).map(|(k, m)| (k as u8, m.passes)).collect();
    crate::rir_text::mark_pass_rows(&mut text, &mark_passes);
    let word_kinds: Vec<(u16, u16)> = e.word_kinds.iter().map(|(&s, &k)| (s, k)).collect();
    crate::rir_text::word_kind_rows(&mut text, &word_kinds);
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
    let rule_ids = firsts.iter().map(|&pc| kept_rules[usize::from(pc)]).collect();
    Ok(Lowered { text, switches, strings: e.strings, tested: e.tested.into_iter().collect(), rules: firsts, rule_ids, mark_words, token_marks, gap_patterns })
}

/// The words laid out as compiled code is, with no jump a layout can spare (the lean parser's
/// `match` arms run on into their code):
///
/// - a place a jump reaches that only jumps on is passed: every target goes to where the jumps
///   end, within its rule;
/// - a jump to a lone `STOP` or a `RETURN` (in any rule: the same return) is that word (not
///   after a `MARK` or `ROW`, whose way is a `GOTO` or `STOP`);
/// - a run of words entered by one jump alone is placed after that jump, and a jump to the
///   next place goes.
///
/// A run is words that go on by position: a word and the next it falls to, and a test's two
/// ways. Runs move whole, and a rule's runs stay its own, its first run first.
fn lay_out(words: &[(&'static str, u8, Arg)], switches: &[Switch], rules: &[u16]) -> (Vec<(&'static str, u8, Arg)>, Vec<Switch>, Vec<u16>) {
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
    let mut switches: Vec<Switch> = switches.to_vec();
    // Each switch's rule: its word's.
    let mut table_rule: HashMap<u16, u16> = HashMap::default();
    for (pc, w) in words.iter().enumerate() {
        if let ("SWITCH", _, Arg::Table(t)) = *w {
            table_rule.insert(t, rule(pc));
        }
    }
    // Where a jump to `t` ends, within rule `r`: past places that are a lone `GOTO`.
    let lone_goto = |words: &[(&'static str, u8, Arg)], t: usize| words[t].0 == "GOTO" && starts_run(t) && !glued[t];
    let dest = |words: &[(&'static str, u8, Arg)], t: Place, r: u16| -> Place {
        let mut t = t;
        for _ in 0..n {
            if t == REFUSE || (t as usize) >= n || rule(t as usize) != r || !lone_goto(words, t as usize) {
                break;
            }
            let Arg::Pc(next) = words[t as usize].2 else { break };
            if next == t || ((next as usize) < n && rule(next as usize) != r) {
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
        if (pc > 0 && words[pc - 1].0 == "PUSH") || (t as usize) >= n {
            continue;
        }
        let after_mark = pc > 0 && matches!(words[pc - 1].0, "MARK" | "ROW");
        let (op, mode, arg) = words[t as usize];
        let copy = match op {
            "STOP" => true,
            "RETURN" => !after_mark,
            _ => false,
        };
        if copy {
            words[pc] = (op, mode, arg);
        }
    }
    // The runs, and the references into each from the runs that are reached.
    let mut run_of = vec![0usize; n];
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (pc, of) in run_of.iter_mut().enumerate() {
        if starts_run(pc) {
            runs.push((pc, pc));
        }
        let last = runs.len() - 1;
        runs[last].1 = pc;
        *of = last;
    }
    let targets_of = |words: &[(&'static str, u8, Arg)], switches: &[Switch], pc: usize| -> Vec<Place> {
        match words[pc] {
            (_, _, Arg::Pc(t)) => vec![t],
            ("SWITCH", _, Arg::Table(t)) => switches[t as usize].all_targets().collect(),
            _ => Vec::new(),
        }
    };
    // Kept from the start, and from each rule's first run (a call enters there).
    let mut reached = vec![false; runs.len()];
    let mut work: Vec<usize> = vec![0];
    for (pc, &r) in run_of.iter().enumerate().skip(1) {
        if rule(pc) != rule(pc - 1) {
            work.push(r);
        }
    }
    let firsts: HashSet<usize> = work.iter().copied().collect();
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
                if t == REFUSE || (t as usize) >= n {
                    continue;
                }
                let tr = run_of[t as usize];
                // Into a run's middle, or a call's landing (its next place too): never moved.
                let whole = runs[tr].0 == (t as usize) && words[pc].0 != "PUSH";
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
            if (t as usize) >= n || (end > 0 && words[end - 1].0 == "PUSH") {
                break;
            }
            let next = run_of[t as usize];
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
            matches!(words[end], ("GOTO", _, Arg::Pc(t)) if !call && order.get(i + 1).is_some_and(|&q| runs[q].0 == (t as usize)))
        })
        .collect();
    let mut new_at = vec![REFUSE; n];
    let mut k: Place = 0;
    for (i, &r) in order.iter().enumerate() {
        let (a, b) = runs[r];
        for (pc, at) in (a..).zip(&mut new_at[a..=b]) {
            *at = k;
            if !(pc == b && drops[i]) {
                k += 1;
            }
        }
    }
    let to = |t: Place| if t == REFUSE || (t as usize) >= n { REFUSE } else { new_at[t as usize] };
    let mut out = Vec::with_capacity(k as usize);
    let mut out_rules = Vec::with_capacity(k as usize);
    for (i, &r) in order.iter().enumerate() {
        let (a, b) = runs[r];
        for (pc, &(op, mode, arg)) in (a..).zip(&words[a..=b]) {
            if pc == b && drops[i] {
                continue;
            }
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
fn ways(words: &[(&'static str, u8, Arg)], switches: &[Switch], pc: usize) -> (Vec<Place>, usize) {
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
            let s = &switches[t as usize];
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
/// `rules`: each word's rule (a function a rule). Words are alike within a rule alone, so each
/// rule keeps its own code, as each of the lean parser's methods does, and a function is
/// entered by calls alone (`stream::validate_rules`). A call's landing stays its caller's (the
/// call returns into its own function). The kept words' rules come back with them.
fn merge_words(words: &[(&'static str, u8, Arg)], switches: &[Switch], rules: Option<&[u16]>) -> (Vec<(&'static str, u8, Arg)>, Vec<Switch>, Vec<u16>) {
    let n = words.len();
    const NONE: u32 = u32::MAX;
    let class_of = |class: &[u32], t: Place| if t == REFUSE || (t as usize) >= n { NONE } else { class[t as usize] };
    // The first partition: operation, mode, operand (a switch's offset too), and the rule.
    let is_call = |pc: usize| pc > 0 && words[pc].0 == "GOTO" && words[pc - 1].0 == "PUSH";
    let partition = |by_rule: bool| -> (Vec<u32>, usize) {
        let mut class: Vec<u32> = {
            let mut ids: HashMap<(&str, u8, (u8, u16), u8, Vec<u16>, u16), u32> = HashMap::default();
            words
                .iter()
                .enumerate()
                .map(|(pc, &(op, mode, arg))| {
                    // A switch's offset and the words it names (their targets are its ways).
                    let (offset, named) = if let Arg::Table(t) = arg { (switches[t as usize].offset, switches[t as usize].words.iter().map(|&(w, _)| w).collect()) } else { (0, Vec::new()) };
                    let next = ids.len() as u32;
                    // A call's `GOTO` is never another `GOTO`'s kind: it goes to its callee.
                    let op = if is_call(pc) { "GOTO (a call's)" } else { op };
                    let rule = if by_rule { rules.map_or(0, |r| r[pc]) } else { 0 };
                    *ids.entry((op, mode, operand_key(arg), offset, named, rule)).or_insert(next)
                })
                .collect()
        };
        let mut count = class.iter().copied().max().map_or(0, |m| m as usize + 1);
        loop {
            let mut ids: HashMap<Vec<u32>, u32> = HashMap::default();
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
    // A rule's words are its own, as each of the lean parser's methods is: no rule goes into
    // another's code, so each function has one entry.
    let (class, count) = partition(rules.is_some());
    let mut rep = vec![usize::MAX; count];
    for pc in (0..n).rev() {
        rep[class[pc] as usize] = pc;
    }
    // What each place holds: nothing, its own word, or a `GOTO` to its kind's kept word.
    #[derive(Clone, Copy, PartialEq)]
    enum Hold {
        Gone,
        Own,
        Stub(usize),
    }
    let mut hold = vec![Hold::Gone; n];
    // A call's landing for each `PUSH`: its state's kept word where the word after is alike.
    let mut landing: Vec<Place> = vec![0; n];
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
            let (p, r) = (p as usize, rep[class[p as usize] as usize]);
            // In the caller's rule: the call returns into its own function.
            let same_rule = rules.is_none_or(|q| q[r] == q[pc]);
            let to = if same_rule && r + 1 < n && p + 1 < n && class[r + 1] == class[p + 1] { r } else { p };
            landing[pc] = to as Place;
            todo.push((to, true));
            todo.push((to + 1, false));
        } else {
            for t in targets {
                if t != REFUSE && (t as usize) < n {
                    todo.push((rep[class[t as usize] as usize], true));
                }
            }
        }
        // A state's places by position stay its own: after `TOKEN` its `MARK` or `SWITCH` (past
        // a call's field, `NODE_FIELD m=4`), after a `MARK` its way, then the next `MARK` or the
        // `SWITCH`; after a `PUSH` its call's `GOTO` (only a `PUSH` reaches it, so it goes to its
        // callee as a call).
        let field_after_token = op == "NODE_FIELD" && words[pc].1 == 4 && pc > 0 && words[pc - 1].0 == "TOKEN";
        for d in 1..=by_place {
            if pc + d < n {
                let whole = matches!((op, d), ("TOKEN", 1) | ("MARK", 2) | ("PUSH", 1)) || (field_after_token && d == 1);
                todo.push((pc + d, whole));
            }
        }
    }
    // The kept places in order, renumbered: places kept stay next to each other.
    let mut new_at = vec![REFUSE; n];
    let mut k: Place = 0;
    for pc in 0..n {
        if hold[pc] != Hold::Gone {
            new_at[pc] = k;
            k += 1;
        }
    }
    let to = |t: Place| if t == REFUSE || (t as usize) >= n { REFUSE } else { new_at[rep[class[t as usize] as usize]] };
    let mut tables: HashMap<u16, u16> = HashMap::default();
    let mut out_switches = Vec::new();
    let mut out = Vec::with_capacity(k as usize);
    let mut out_rules = Vec::with_capacity(k as usize);
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
                    ("PUSH", Arg::Pc(_)) => Arg::Pc(new_at[landing[pc] as usize]),
                    ("SWITCH", Arg::Table(t)) => {
                        let at = *tables.entry(t).or_insert_with(|| {
                            let s = &switches[t as usize];
                            out_switches.push(Switch { offset: s.offset, targets: s.targets.iter().map(|&x| to(x)).collect(), end: to(s.end), words: s.words.iter().map(|&(w, x)| (w, to(x))).collect() });
                            (out_switches.len() - 1) as u16
                        });
                        Arg::Table(at)
                    }
                    (_, Arg::Pc(t)) => Arg::Pc(to(t)),
                    (_, a) => a,
                };
                out.push((op, mode, arg));
            }
        }
    }
    (out, out_switches, out_rules)
}

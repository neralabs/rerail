//! The transducer's word program: its opcodes, a word's encoding (`word`, `split`), and the
//! words with the tables they index (`Words`): strings, labels, longs, tier 1's block programs
//! and tier 2's switches and readings. The engine's `Program` holds one, with the matcher's
//! predicates, patterns and directives beside it.

use crate::block::BlockProgram;
use crate::stream_ir::{ByteSwitch, Readings};

pub const STOP: u8 = 0x00;
pub const KIND: u8 = 0x01;
pub const FIELD: u8 = 0x02;
pub const FLAGS: u8 = 0x03;
pub const TEXT: u8 = 0x04;
pub const SIZE: u8 = 0x05;
pub const ASSERT: u8 = 0x06;
pub const FIRST: u8 = 0x07;
pub const LAST: u8 = 0x08;
pub const ABSENT_FIELD: u8 = 0x09;
pub const SUPERTYPE: u8 = 0x0a;
pub const CHILD: u8 = 0x10;
pub const NEXT: u8 = 0x11;
pub const PARENT: u8 = 0x12;
pub const DESCEND: u8 = 0x13;
pub const FOLLOWING: u8 = 0x14;
pub const SAVE_CURSOR: u8 = 0x15;
pub const RESTORE_CURSOR: u8 = 0x16;
pub const CAPTURE: u8 = 0x20;
pub const REPEAT_BEGIN: u8 = 0x21;
pub const REPEAT_BUMP: u8 = 0x22;
pub const REPEAT_END: u8 = 0x23;
pub const FORK: u8 = 0x30;
pub const GOTO: u8 = 0x31;
pub const EMIT: u8 = 0x40;
// Tier 2's words, over the token stream (decision 0087, `stream`).
pub const TOKEN: u8 = 0x50;
pub const SWITCH: u8 = 0x51;
/// `WORD` in `rir`.
pub const WORD_IS: u8 = 0x52;
// 0x53 was `CHECK`, a token's check run per token: a check is tier 1's masks (doc 18 §4).
pub const OPEN: u8 = 0x54;
pub const CLOSE: u8 = 0x55;
pub const LEAF: u8 = 0x56;
pub const NODE_FIELD: u8 = 0x57;
pub const NODE_SUPER: u8 = 0x58;
pub const PUSH: u8 = 0x59;
pub const RETURN: u8 = 0x5a;
pub const ACCEPT: u8 = 0x5b;
pub const SPAN: u8 = 0x5c;
/// Refuses unless one of tier 1's totals is zero: the fallback.
pub const EMPTY: u8 = 0x5d;
/// A mark (a token of no width) before the token: `m & MARK_ORDER` its order, the operand its
/// mask.
pub const MARK: u8 = 0x5e;
/// `MARK`'s mode bits for the mark's order: 64 marks.
pub const MARK_ORDER: u8 = 63;
/// `MARK`'s mode bit: the file's end has the mark.
pub const MARK_END: u8 = 64;
/// `MARK`'s mode bit: on the mark, its arm, the mark kept for the callee that arm calls.
pub const MARK_KEEP: u8 = 128;
/// `TOKEN`'s mode bit: the extras taken in the token step (`Words::extras`), as the lean parser
/// places comments where it passes them: a token that is an extra is its leaf, deferred, and
/// the next token is taken.
pub const TOKEN_EXTRAS: u8 = 128;

/// A `PUSH`'s mode: the flag its call runs with (`flag`, set or cleared), as a parameter: the
/// caller's flags saved first and given back where the call returns. 0: none.
pub const fn push_flag(flag: u16, set: bool) -> u8 {
    1 + ((flag as u8) << 1 | if set { 0 } else { 1 })
}

/// A `PUSH` mode's flag (`push_flag`): the flag and whether it is set.
pub const fn push_flag_of(mode: u8) -> Option<(u16, bool)> {
    if mode == 0 { None } else { Some((((mode - 1) >> 1) as u16, (mode - 1) & 1 == 0)) }
}
/// Runs the next word when the token starts at a bit of mask `operand`, else skips it: which of
/// two rows beginning with one byte the token is.
pub const ROW: u8 = 0x5f;
/// The innermost open node becomes a node of kind `operand`: its kind decided by this token.
pub const RETAG: u8 = 0x60;
/// The token's leaf joins the grammar's leaf before it (the pending extras between are its text,
/// dropped): one leaf of kind `operand`, to this token's end.
pub const MERGE: u8 = 0x61;
/// A node of kind `operand` around the node last made at this level, open: it takes that node's
/// field and start; `NODE_FIELD m=1` and `RETAG m=1` then act on the moved node.
pub const WRAP: u8 = 0x62;
/// The token just taken is its level's word, of kind `operand` (`remember C`): a kind
/// `stream_ir::REMEMBERED` names it (`RETAG m=1, 2`, a reading's way) until the next. A call
/// (`PUSH`) keeps its caller's word, given back where it returns; the called level starts
/// with it.
pub const REMEMBER: u8 = 0x63;
/// The level's flag `operand` (`with F R`, `unless F A`): `m=0` set, `m=1` clear, after a
/// `PUSH` (the called level's; the caller's come back where it returns); `m=2` a test, which
/// runs the next word where the flag is set and skips it where it is clear.
pub const FLAG: u8 = 0x64;
/// A count of tokens, the file refused past its most (`Words::counts`, operand its index): `m=0`
/// one more (the token is an up's, as `{`), `m=1` one less where above 0 (a down's, `}`).
/// Tier 1's `count(up, down) max M` where its ups and downs are tokens' starts, at the arms
/// that take them, as the lean parser counts a depth as it parses.
pub const COUNT: u8 = 0x65;
/// A local check at the token taken (`Words::local_checks`, operand its index): refused where
/// the check holds at a bit of its mask among the token's bytes, as the lean lexer checks a
/// string's `\u` escapes where it notes one.
pub const CHECK: u8 = 0x66;
/// The pending extras before the first whose code has mark `operand`'s bit (the marks of its
/// own gap, `records::Records::own`) are no longer pending: the nodes closed next keep them, as
/// a leaf taken after them would. A settling mark's arm (`mark … , settles`): the extras before its
/// line break are the statement's.
/// `m=1` (`mark … , drops …`): where gap pattern `operand` holds at the token
/// (`Words::gap_patterns`, its first entry), the extra pending last is dropped: no node of the
/// tape (decision 0416, the lean lexer's swallowed comment). `m=2`, and `m=3+F` where flag `F`
/// is set (`mark … , loses …`): where gap pattern `operand` holds, the extra pending last is
/// lost: of no width, after the token (decision 0451, the lean lexer's `asi`). `m=19`, and
/// `m=20+F` where flag `F` is set (`mark … , refuses …`): where gap pattern `operand` holds, the
/// file is refused (what the lean lexer's `asi` reads there the grammar does not say yet).
/// `m=36`, and `m=37+F` where flag `F` is set (`mark … , loses since …`): where gap pattern
/// `operand` holds, the first extra pending is of no width at the start of the one after it, in
/// its place (the lean lexer's `asi` reads a block comment through to another comment, loses its
/// bytes there, and is asked again after that one).
pub const SETTLE: u8 = 0x84;
/// `SETTLE`'s first refusing mode (`m=19`; `m=20+F` with a flag).
pub const SETTLE_REFUSE: u8 = 19;
/// `SETTLE`'s first moving mode (`m=36`; `m=37+F` with a flag).
pub const SETTLE_MOVE: u8 = 36;

// The preprocessing words (decision 0348, `pp_words`): tier 2's words over a stack of token
// sources (a file's tape, a range of the TU's arena), with a token register `T`, a macro register
// `D`, the macro table and a stack of call frames. A test runs the next word where it holds and
// skips it where not.
/// `T` ← the next token, unexpanded: an ended source left (its macro enabled again), a stop
/// source's end read as `EOF` with `STOP`.
pub const READ: u8 = 0x67;
/// A test: `T` ← the directive line's next token (the file source on top); not where it ended.
pub const LINE: u8 = 0x68;
/// A test of `T`: `m=0` its kind is the operand; `m=1` it is spelled string `operand`; `m=3` it is
/// a directive's `#` (a line's first token, read from a file).
pub const IS: u8 = 0x69;
/// A test of the next token, unread (through ended sources, never past a stop): spelled string
/// `operand`.
pub const PEEK: u8 = 0x6a;
/// A test: `D` ← the macro `T` names, unless `T` is painted.
pub const LOOKUP: u8 = 0x6b;
/// A test of `D`: `m=0` disabled (being expanded); `m=1` function-like.
pub const MACRO: u8 = 0x6c;
/// `T` painted: never expanded again.
pub const PAINT: u8 = 0x6d;
/// `T` onto the output tape.
pub const PUT: u8 = 0x6e;
/// A call frame for `D`, named by `T` (a function-like macro's first argument opened).
pub const FRAME: u8 = 0x6f;
/// The top frame's arguments: `m=0` `T` appended; `m=1` a `(`, appended; `m=2` a test, a `)`:
/// the call's end at depth 0, else appended; `m=3` a `,`: a new argument at depth 0 (not the
/// variadic one's), else appended; `m=4` a test, the call's end: the arguments' count right, else
/// the frame dropped and `T` its name, painted; `m=5` the frame dropped (no `)`).
pub const ARG: u8 = 0x70;
/// `m=0` a test: the top frame's next argument to expand before substitution, pushed as a stop
/// source with a level opened for its tokens; `m=1`: that level closed, the argument's expansion.
pub const PRE: u8 = 0x71;
/// `T` onto the open level.
pub const KEEP: u8 = 0x72;
/// A `#define`'s line read into the macro table, a token a turn (`pp_words::DefState`; a malformed
/// one left): `m=0`, a test: its name read (else the line passed); `m=1`, a test: `(` right after
/// it, function-like; `m=2`, a test: a parameter list's token read, the list going on; `m=3`, a
/// test: the list closed (else malformed or unterminated: no definition); `m=4` the body begun;
/// `m=5`, a test: the body's next token read and marked (a parameter, `__VA_OPT__`, `##`); `m=6`
/// the walk of its argument uses begun; `m=7`, a test: one body token walked (`Def::pre`'s
/// order); `m=8` the definition entered.
pub const DEFINE: u8 = 0x74;
/// An `#undef`'s line read: its macro taken out of the table.
pub const UNDEF: u8 = 0x75;
/// The directive line's tokens left unread.
pub const SKIPLINE: u8 = 0x76;
/// The file source to the next directive of the chain of the conditional whose `#` it read last
/// (mode (a)'s `c_cond_next`, `pp_words::File::chains`): a group not taken passed unread. No
/// next: the file's end.
pub const SKIP: u8 = 0x77;
/// The file's conditional stack: `m=0` a group opened, not taken; `m=1` opened, taken; `m=2` a
/// test, a group is open in this file; `m=3` a test, its chain was taken or had its `#else`;
/// `m=4` its chain taken; `m=5` its `#else` seen; `m=6` closed.
pub const COND: u8 = 0x78;
/// `m=0`: an `#if`'s line (the rest of it) pushed as a stop source, a level opened for its
/// expansion, `defined` an operator; `m=1`: the level closed, its tokens the expression `EXPR`
/// reads; `m=2`: as `m=0`, `defined` no operator (an `#include`'s line).
pub const IF: u8 = 0x79;
/// A test: `T` is `defined` (not painted), its operand (`X` or `( X )`) read unexpanded, and
/// `T` the number `1` or `0`.
pub const DEFINED: u8 = 0x7a;
/// An `#include`, `#include_next` or `#import` (`T` its word): `m=0`, a test: its header name
/// was read by mode (a) (`pp_words::File::includes`), its file looked up, its line passed; `m=1`,
/// a test: the level `IF m=2` opened closed, a header name made of its tokens, and its file
/// found by name (`File::named`; one not there is wanted, `Expanded::wanted`); `m=2` the file
/// entered, unless not found, once only, or too deep.
pub const INCLUDE: u8 = 0x7b;
/// A `#pragma`'s line, unexpanded: `once`, `push_macro("X")` and `pop_macro("X")` carried out.
pub const PRAGMA: u8 = 0x7c;
/// A built-in macro `D` (`pp_words::BUILTINS`), `T` its name: `m=0` `T` its value (one with no
/// operand); `m=1`, a test: an operator here (`__has_include` and `__has_embed` only in `#if`),
/// its operand's level opened; `m=2`, a test: `T` is the operand's `)` (else kept); `m=3`, a
/// test: the operand is to be expanded (`_Pragma`'s, a header name that is neither `"…"` nor
/// `<…>`), pushed as a stop source, else `T` its value; `m=4`, a test: the expanded operand's
/// level closed, and `T` its value (else the pragma carried out: no token).
pub const BUILTIN: u8 = 0x7d;
/// `#line` and GNU's line marker: `m=0` `T` the marker's number; `m=1` the level `IF m=2` opened
/// closed, the file's presumed line (and name) set from its tokens.
pub const LINEDIR: u8 = 0x7e;
/// `#embed` (`T` its word): `m=0`, a test: its name's tokens pushed to be expanded (a
/// level), else its parameters' (the name read by mode (a) or spelled); `m=1` the name's level
/// closed, the name made, the parameters pushed; `m=2` the parameters' level closed, its tokens
/// left in place; `m=3`, a test: a parameter's name read (and its argument's `(`); `m=4`, a test:
/// an argument's token stepped; `m=5`, a test: the argument closed, and the parameter `limit` (its
/// expression `EXPR`'s; else `prefix`, `suffix` or `if_empty` kept); `m=6` `limit` its value;
/// `m=7`, a test: the file found, its first tokens pushed (`prefix`, or `if_empty` where it has no
/// bytes), and it has bytes; `m=8`, a test: a byte pushed, a number (after a `,`); `m=9` `suffix`
/// pushed; `m=10` the tokens pushed a source.
pub const EMBED: u8 = 0x7f;
/// The top frame's macro substituted into the arena, a body token a turn (`pp_words::Sub`):
/// `m=0` begun (the expansion recorded); `m=1`, a test: the body's next token taken (`B`);
/// `m=2`, a test: `B` is `#` before a parameter, its argument the range to stringize; `m=3`, a
/// test: `B` is `#` before `__VA_OPT__`, its content opened, to be stringized; `m=4`, a test:
/// `B` is `__VA_OPT__`, its content opened (or a placemarker); `m=5` `B` substituted: a
/// parameter's argument (raw beside `##`, else expanded; GNU's comma elision), or `B` itself;
/// `m=6`, a test: the content closed by `m=7` is to be stringized, its pastes begun; `m=7`, a
/// test: a content ended, the body's walk after it resumed; `m=8` the content's placemarkers
/// dropped, the range to stringize; `m=9` the output's pastes begun; `m=10` its placemarkers
/// dropped, the frame dropped, the output pushed as a source that disables the macro.
pub const SUB: u8 = 0x80;
/// `#`'s literal (C17 §6.10.3.2), a token a turn: `m=0` begun; `m=1`, a test: the range's next
/// token's spelling appended (a blank where white space was, `"` and `\` escaped in literals);
/// `m=2` the literal appended to the output; `m=3` the range replaced by the literal.
pub const STR: u8 = 0x81;
/// `m=0`, a test: one step of the range's pastes (`##` joins its neighbours into one token, or
/// leaves them two); where none is left, the range ends at its last token written.
pub const GLUE: u8 = 0x82;
/// `#if`'s expression a step at a time (`pp_expr`'s precedence climbing as rules, its values on a
/// stack, `pp_words::Ex`): `m=0`, a test: an operand's token taken (else an error, the value 0);
/// `m=1`, a test: it is a prefix operator, pushed; `m=2` the prefix applied; `m=3`, a test: it is
/// `(`; `m=4` its `)` read (else an error); `m=5` its value pushed (a primary); `m=6`, a test: `,`
/// read, the left value dropped; `m=7` a binary operand's least precedence 1; `m=8`, a test: a
/// binary operator of that precedence or more read; `m=9` it applied; `m=10` the least dropped;
/// `m=11`, a test: `?` read; `m=12` its `:` read (else an error); `m=13` the arm chosen; `m=14`, a
/// test: the expression whole, with no error, and true.
pub const EXPR: u8 = 0x83;

pub const NAMED: u8 = 1;
pub const GREEDY: u8 = 1;
pub const EXTRA: u8 = 2;
pub const MISSING: u8 = 4;
pub const ERROR: u8 = 8;
pub const SIZE_LONG: u8 = 0x80;

/// A word: its opcode, its mode and its operand, in one `u32`.
pub const fn word(opcode: u8, mode: u8, operand: u16) -> u32 {
    opcode as u32 | ((mode as u32) << 8) | ((operand as u32) << 16)
}

/// A word's opcode, mode and operand.
pub fn split(word: u32) -> (u8, u8, u16) {
    (word as u8, (word >> 8) as u8, (word >> 16) as u16)
}

/// The grammar's names for a program's numbers (`name rule R "N"`, `name kind #K "N"`, `name
/// field F "N"`, `name token K "N"`, `name mark K "N"`): each rule's (by its place among
/// `Words::rules`), each node kind's and field's (rising by number; no word reads them), each record kind's (the token kinds tier 1's records give), and each mark's
/// in the records' code (`Records::marks`, by its bit).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Names {
    pub rules: Vec<(u16, String)>,
    pub kinds: Vec<(u16, String)>,
    pub fields: Vec<(u16, String)>,
    pub tokens: Vec<(u8, String)>,
    pub marks: Vec<(u8, String)>,
}

/// A program's words and the tables they index.
#[derive(Clone, Default)]
pub struct Words {
    pub words: rerail_blocks::vector::Vector<u32>,
    pub strings: Vec<Vec<u8>>,
    pub labels: rerail_blocks::vector::Vector<u16>,
    pub longs: rerail_blocks::vector::Vector<u64>,
    pub tree_sitter_query: bool,
    /// Set mode (t1 spec-transducer §1, list step 14): the program's consumer reads its output
    /// as a set of (task, label) keys, so an executor may stop a label at its first emit in a
    /// task and drop a path whose reachable labels have all fired (`set_mode::Reach`). Its
    /// captures are then one witness's, not every path's.
    pub set_mode: bool,
    /// Block programs, the side programs over source bytes (`block`), as value programs are
    /// the side programs over a node's text.
    pub block_programs: Vec<BlockProgram>,
    /// Tier 2's switch tables (`SWITCH`, decision 0087).
    pub switches: Vec<ByteSwitch>,
    /// Tier 2's readings (`RETAG m=3`) and provisional fields.
    pub readings: Readings,
    /// Tier 2's rules, when it has them (`rule N @P`): each rule's first place, from 0 up; a rule
    /// is the places from its first to the next rule's. Each rule is a function (a grammar
    /// rule's), its calls (`PUSH`) native calls: only a call's `GOTO`, the word after its
    /// `PUSH`, goes to another rule (`stream::validate_rules`). Empty: one state machine.
    pub rules: Vec<u16>,
    /// The grammar's names (`name …` rows), for code that reads as the grammar does: none changes
    /// what the program does.
    pub names: Names,
    /// Tier 2's marks that hold only at a token of one word (`markword K sW`): mark `K` (its
    /// order) holds where its mask does and the token's record names string `W`, as a parser
    /// tests the next token's word. Rising by mark.
    pub mark_words: Vec<(u8, u16)>,
    /// The marks each mark passes too where it is taken, past those before it (`markpasses K
    /// M`, a mark's `, passes`): mark `K`'s, a bit each by order. Rising by mark.
    pub mark_passes: Vec<(u8, u64)>,
    /// Each keyword's node kind (`wordkind sW #K`): a token whose record names string `W` makes a
    /// leaf of kind `K` where a `LEAF` names `stream_ir::WORD_KIND`. Rising by string.
    pub word_kinds: Vec<(u16, u16)>,
    /// The extras a `TOKEN` takes (`m` with `TOKEN_EXTRAS`; `tokenextra bB #K`): a token whose
    /// record's code has the first entry's mark bit `B` (`records::MARK_SHIFT + B`) is an extra,
    /// of the first entry's kind `K`, or of a later entry's where it has that entry's bit too
    /// (`stream::extra_kind`: a kind each, after the first). Rising by bit.
    pub extras: Vec<(u8, u16)>,
    /// The token marks (decision 0129; `tokenmark K oO kK sW …`): mark `K` holds where its
    /// record's bit does and its tokens follow, each element one record (the extras between
    /// skipped). Rising by mark.
    pub token_marks: Vec<TokenMark>,
    /// The questions a mark's `drops` and `loses` ask (decision 0451; `gappattern P oO kK …`):
    /// entries of one pattern share its order `P`, which `SETTLE` names by its first entry
    /// (`patterns_hold`). Each element is a record as the lexer wrote it, extras included, at
    /// its offset from the token (`gap_pattern_holds`): the lean lexer's `asi`, which reads on
    /// past a comment after a line break. Rising by pattern.
    pub gap_patterns: Vec<TokenMark>,
    /// The counts of tokens (`COUNT`; `tokencount N max M`): each one's most.
    pub counts: Vec<u16>,
    /// Checks of a few bytes at each bit of a sparse mask (`localcheck`): tier 1's escape or
    /// error `A & L`, with `A` a mask tier 2 reads and `L` local (byte tests at fixed offsets);
    /// at `ACCEPT`, the file is refused where `L` holds at a bit of `A`. As the lean lexer checks
    /// a string's `\u` escapes where it notes one. Or a check over a token's own bytes
    /// (`LocalCheck::token`): a number's syntax, as simdjson's stage 2 parses a number.
    pub local_checks: Vec<LocalCheck>,
    /// The preprocessing words' tables (`pp_words`), when the program has them.
    pub pp: PpTables,
}

impl Words {
    /// The marks done at a token where mark `order` is taken: it, the marks before it, and those
    /// it passes (`mark_passes`).
    pub fn taken(&self, order: u8) -> u64 {
        let passes = self.mark_passes.iter().find(|p| p.0 == order).map_or(0, |p| p.1);
        u64::MAX >> (63 - u32::from(order & crate::program::MARK_ORDER)) | passes
    }
}

/// The preprocessing words' tables: what a paste (`##`) may make, which a dialect sets.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PpTables {
    /// The punctuators (`ppop sN`): two punctuators paste into one where their bytes spell one.
    pub ops: Vec<u16>,
    /// The punctuators C++ has and C does not (`ppop sN cpp`): one where the run is C++'s
    /// (`pp_words::Env::cpp`).
    pub cpp_ops: Vec<u16>,
    /// In C++, a literal pastes with an identifier after it into one literal, its suffix
    /// (`ppsuffix`, user-defined literals).
    pub suffix: bool,
    /// In C++, `#if` reads the alternative tokens as operators and `true` as 1 (`ppcpp`).
    pub cpp: bool,
}

/// A local check (`Words::local_checks`): at each byte `j` of mask `mask`, `ops`' last value at
/// stream place `j + delay` (`LocalOp`, each op's operands earlier ones); 1 refuses.
///
/// A token's check (`token`, decision 0398): `ops` over the token's own bytes, place 0 its first
/// byte, as simdjson's stage 2 reads a number (`token_value`); `mask` unused. With a `delay`
/// (decision 0399), the frame runs that many bytes past the token, and the test at place `i`
/// is the last op at `i + delay`: 1 at any of places `delay` to `n + delay` refuses. A
/// `within k` there is a byte of the file: place `i - k` is in the frame and the file. A check
/// `check KIND = G` compiles to one: its byte machine as a `fill` per self-loop and logic, run
/// word-parallel, 64 places a word, its carries passed from word to word on a longer token. No
/// loop over bytes.
///
/// A token's check may read `back` bytes before the token (decision 0428): the frame then
/// begins that many bytes earlier, the token's first byte at place `back`. Before the file every
/// op's operand is 0 (no byte, no token), as tier 1's `prev` reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct LocalCheck {
    pub mask: u16,
    pub delay: u8,
    pub ops: Vec<LocalOp>,
    pub token: bool,
    pub back: u8,
}

/// A local check's operation, at a stream place `i` (tier 1's meaning: `block::BlockOp`): a
/// byte's test reads byte `i` (0 outside the file), `Prev` the operand at `i - k` (0 before the
/// stream).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LocalOp {
    /// The byte is in the set (256 bits).
    Bytes([u64; 4]),
    /// Bit `i % 64` of the pattern.
    Const(u64),
    /// `d <= i < len + d`.
    Within(u8),
    Not(u16),
    And(u16, u16),
    Or(u16, u16),
    Xor(u16, u16),
    AndNot(u16, u16),
    Prev(u16, u8),
    /// A token's check only (`LocalCheck::token`): `fill(s, e)`, `block::BlockOp::Fill`'s, from
    /// the token's first byte: `f[i] = s[i] | (f[i - 1] & !e[i])`, `f[-1] = 0`.
    Fill(u16, u16),
    /// A token's check only: 1 at the token's first byte.
    Start,
    /// A token's check only: 1 at the token's last byte.
    Last,
    /// A token's check only (decision 0428): 1 where a token before this one has its last
    /// byte, as tier 2's records hold them; 0 at the token and after it.
    Ended,
}

impl LocalOp {
    /// The ops it reads.
    pub fn operands(&self) -> Vec<u16> {
        match *self {
            LocalOp::Not(a) | LocalOp::Prev(a, _) => vec![a],
            LocalOp::And(a, b) | LocalOp::Or(a, b) | LocalOp::Xor(a, b) | LocalOp::AndNot(a, b) | LocalOp::Fill(a, b) => vec![a, b],
            LocalOp::Bytes(_) | LocalOp::Const(_) | LocalOp::Within(_) | LocalOp::Start | LocalOp::Last | LocalOp::Ended => vec![],
        }
    }

    /// An op of a token's check only.
    pub fn token_only(&self) -> bool {
        matches!(self, LocalOp::Fill(..) | LocalOp::Start | LocalOp::Last | LocalOp::Ended)
    }
}

/// A token's check (`LocalCheck::token`) at the token of `bytes[start..end]`: whether it
/// refuses. The frame is `back` bytes before the token, its bytes and `delay` more, a byte past
/// the file 0. `ended(j)`: whether a token before this one has its last byte at `j`.
pub fn token_value(c: &LocalCheck, bytes: &[u8], start: usize, end: usize, ended: &dyn Fn(usize) -> bool) -> bool {
    let (ops, back) = (&c.ops, usize::from(c.back));
    let (n, d) = (end - start, usize::from(c.delay));
    let m = back + n + d;
    // The file's index of place `i`; none before the file.
    let file = |i: usize| (start + i).checked_sub(back);
    let byte = |i: usize| file(i).map(|j| bytes.get(j).copied().unwrap_or(0));
    let mut v: Vec<Vec<bool>> = Vec::with_capacity(ops.len());
    for op in ops {
        let at = |a: u16, i: usize| v[usize::from(a)][i];
        let row: Vec<bool> = match *op {
            LocalOp::Bytes(s) => (0..m).map(|i| byte(i).is_some_and(|b| s[usize::from(b) / 64] >> (b % 64) & 1 == 1)).collect(),
            LocalOp::Const(p) => (0..m).map(|i| p >> (i % 64) & 1 == 1).collect(),
            LocalOp::Within(k) => (0..m).map(|i| i >= usize::from(k) && file(i - usize::from(k)).is_some_and(|j| j < bytes.len())).collect(),
            LocalOp::Ended => (0..m).map(|i| i < back && file(i).is_some_and(ended)).collect(),
            LocalOp::Not(a) => (0..m).map(|i| !at(a, i)).collect(),
            LocalOp::And(a, b) => (0..m).map(|i| at(a, i) && at(b, i)).collect(),
            LocalOp::Or(a, b) => (0..m).map(|i| at(a, i) || at(b, i)).collect(),
            LocalOp::Xor(a, b) => (0..m).map(|i| at(a, i) != at(b, i)).collect(),
            LocalOp::AndNot(a, b) => (0..m).map(|i| at(a, i) && !at(b, i)).collect(),
            LocalOp::Prev(a, k) => (0..m).map(|i| i >= usize::from(k) && at(a, i - usize::from(k))).collect(),
            LocalOp::Fill(a, b) => {
                let mut f = vec![false; m];
                for i in 0..m {
                    f[i] = at(a, i) || (i > 0 && f[i - 1] && !at(b, i));
                }
                f
            }
            LocalOp::Start => (0..m).map(|i| i == back).collect(),
            LocalOp::Last => (0..m).map(|i| i + 1 == back + n).collect(),
        };
        v.push(row);
    }
    v.last().is_some_and(|r| r[back + d..].iter().any(|&b| b))
}

/// A local check's value at stream place `i` over `bytes` (`LocalCheck`), each op's once.
pub fn local_value(ops: &[LocalOp], bytes: &[u8], i: i64) -> bool {
    fn at(ops: &[LocalOp], k: usize, bytes: &[u8], i: i64) -> bool {
        let byte = |i: i64| if i >= 0 && (i as usize) < bytes.len() { bytes[i as usize] } else { 0 };
        match ops[k] {
            LocalOp::Bytes(s) => {
                let b = byte(i);
                s[usize::from(b) / 64] >> (b % 64) & 1 == 1
            }
            LocalOp::Const(p) => i >= 0 && p >> (i % 64) & 1 == 1,
            LocalOp::Within(d) => i >= i64::from(d) && i < bytes.len() as i64 + i64::from(d),
            LocalOp::Not(a) => !at(ops, usize::from(a), bytes, i),
            LocalOp::And(a, b) => at(ops, usize::from(a), bytes, i) && at(ops, usize::from(b), bytes, i),
            LocalOp::Or(a, b) => at(ops, usize::from(a), bytes, i) || at(ops, usize::from(b), bytes, i),
            LocalOp::Xor(a, b) => at(ops, usize::from(a), bytes, i) != at(ops, usize::from(b), bytes, i),
            LocalOp::AndNot(a, b) => at(ops, usize::from(a), bytes, i) && !at(ops, usize::from(b), bytes, i),
            LocalOp::Prev(a, k) => i >= i64::from(k) && at(ops, usize::from(a), bytes, i - i64::from(k)),
            // A token's check's alone (`stream::validate_local_checks`).
            LocalOp::Fill(..) | LocalOp::Start | LocalOp::Last | LocalOp::Ended => unreachable!("a token's check's op in a local check"),
        }
    }
    !ops.is_empty() && at(ops, ops.len() - 1, bytes, i)
}

/// A token mark (`Words::token_marks`): its order, and its elements.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct TokenMark {
    pub order: u8,
    pub elems: Vec<TokenElem>,
    /// A mark the grammar takes nowhere: where it holds, the file is refused. `TOKEN` tests it
    /// at each token its record's bit names, once (no state tests it).
    pub refuses: bool,
    /// An `unless before` (decision 0416): the mark holds where it does not. A mark's entries
    /// share its order (`patterns_hold`).
    pub unless: bool,
}

/// Whether the token marks of one order hold (`Words::token_marks` from `first`, the entries
/// of `first`'s order): none of its `unless` ones, and one of the others where it has any.
/// `holds` tests one entry.
pub fn patterns_hold(marks: &[TokenMark], first: usize, holds: &mut dyn FnMut(&TokenMark) -> bool) -> bool {
    let order = marks[first].order;
    let these = marks[first..].iter().take_while(|m| m.order == order);
    let (mut some, mut any) = (false, false);
    for m in these {
        let h = holds(m);
        if m.unless {
            if h {
                return false;
            }
        } else {
            some = true;
            any |= h;
        }
    }
    !some || any
}

/// One token of a token mark: its offset from the mark's token (-1 the one before), and what
/// the token's record may be: a kind of `kinds` (bit `k`: the records' kind `k`), or a word of
/// `words` (strings).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct TokenElem {
    pub offset: i8,
    pub kinds: u64,
    pub words: Vec<u16>,
    /// The kinds' hard keywords (strings): a record of one of the kinds whose word is one of
    /// these is that keyword's token, of the keyword's own kind (as Bitlex's evaluator has it),
    /// not one of the kinds. `simple_identifier "<"` holds at `foo <` but not at `fun <`.
    pub except: Vec<u16>,
    /// A token none of the kinds and words (never the file's end).
    pub not: bool,
    /// The file's end takes it too: no token left (the last element only).
    pub end: bool,
    /// A run (decision 0416): any number of records the element takes, then the elements after
    /// it from any of them on (one run at most, after offset 0, never last).
    pub run: bool,
    /// A run's depths (Q339, Bitlex's `nest`): each side of one, the run's records it takes go
    /// one deeper (`open`) or one out. The elements after the run are tried only where every
    /// depth is 0; a closer where its depth is 0 ends the run.
    pub nests: Vec<Nest>,
    /// A run's `ending` (Q342): the elements after it are tried only where its last record (the
    /// one before it, where it took none) is one this test takes.
    pub ending: Option<Box<TokenElem>>,
    /// The marks (bits of the records' marks) whose bit the record must not hold (`without M`),
    /// as the lean parser asks `!peek_at(j).nl`: the element holds only where the code has none
    /// of them.
    pub without: u32,
    /// The marks whose bit the record must hold (`with M`), as the lean parser asks
    /// `peek_at(j).nl`: the element holds only where the code has all of them.
    pub with: u32,
    /// The bytes the record's token may begin with (`starting "B"`, a gap pattern's only): a
    /// set of 256 bits, none for no test. The lean lexer's `asi` reads the byte after a comment.
    pub first: [u64; 4],
    /// A gap pattern's `since` (decision 0451): `offset` counts from the grammar token before
    /// the token (0; none before the file's first), and the record must be before the token.
    /// The first comment after a line break, however many follow it, where the lean lexer's
    /// `asi` decides.
    pub since: bool,
    /// The marks (bits of the records' marks) that take a record as one of the kinds: a kind
    /// whose first byte another token's starts too has its rows' starts as a mark bit of the
    /// code, read as tier 2's `ROW` reads a row's start, and `kinds` holds only the classes of
    /// the bytes no other token starts.
    pub rows: u32,
}

/// One side of a run's depth (`TokenElem::nests`): its counter (0 to 3), whether it opens, and
/// the records it takes (`test`'s kinds and words; never a `~`, an `end` or a run).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Nest {
    pub counter: u8,
    pub open: bool,
    pub test: TokenElem,
}

/// The depths a run counts (`TokenElem::nests`): 4 at most.
pub const NESTS: usize = 4;

/// A run's depths after it takes a record (`code`): `None` where a closer's depth is 0, which
/// ends the run (Q339).
pub fn nest_step(nests: &[Nest], depth: [u32; NESTS], code: u32) -> Option<[u32; NESTS]> {
    let mut d = depth;
    for n in nests.iter().filter(|n| n.test.takes(code)) {
        let k = usize::from(n.counter);
        if n.open {
            d[k] += 1;
        } else if d[k] == 0 {
            return None;
        } else {
            d[k] -= 1;
        }
    }
    Some(d)
}

impl TokenElem {
    /// Whether a record's code is one of the element's.
    pub fn takes(&self, code: u32) -> bool {
        let kind = code & 0xff;
        let word = crate::records::word_of(code);
        let keyword = word > 0 && self.except.contains(&((word - 1) as u16));
        let row = code >> crate::records::MARK_SHIFT & self.rows != 0;
        ((kind < 64 && self.kinds >> kind & 1 == 1 || row) && !keyword) || (word > 0 && self.words.contains(&((word - 1) as u16)))
    }

    /// Whether the record's first byte is one the element takes (`TokenElem::first`).
    pub fn first_holds(&self, byte: u8) -> bool {
        self.first == [0; 4] || self.first[usize::from(byte / 64)] >> (byte % 64) & 1 == 1
    }

    /// Whether the element holds at a record's code (`None`: no record left, the file's end).
    pub fn holds(&self, code: Option<u32>) -> bool {
        match code {
            None => self.end,
            Some(c) => self.takes(c) != self.not && c >> crate::records::MARK_SHIFT & self.without == 0 && c >> crate::records::MARK_SHIFT & self.with == self.with,
        }
    }
}

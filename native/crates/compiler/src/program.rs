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
/// A mark (a token of no width) before the token: `m=k` its order, the operand its mask.
pub const MARK: u8 = 0x5e;
/// `MARK`'s mode bit: on the mark, its arm, the mark kept for the callee that arm calls.
pub const MARK_KEEP: u8 = 64;
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

/// A program's words and the tables they index.
#[derive(Clone, Default)]
pub struct Words {
    pub words: Vec<u32>,
    pub strings: Vec<Vec<u8>>,
    pub labels: Vec<u16>,
    pub longs: Vec<u64>,
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
    /// Tier 2's marks that hold only at a token of one word (`markword K sW`): mark `K` (its
    /// order) holds where its mask does and the token's record names string `W`, as a parser
    /// tests the next token's word. Rising by mark.
    pub mark_words: Vec<(u8, u16)>,
    /// Each keyword's node kind (`wordkind sW #K`): a token whose record names string `W` makes a
    /// leaf of kind `K` where a `LEAF` names `stream_ir::WORD_KIND`. Rising by string.
    pub word_kinds: Vec<(u16, u16)>,
    /// The extras a `TOKEN` takes (`m` with `TOKEN_EXTRAS`; `tokenextra bB #K`): a token whose
    /// record's code has mark bit `B` (`records::MARK_SHIFT + B`) is an extra of kind `K`. Rising
    /// by bit.
    pub extras: Vec<(u8, u16)>,
    /// The token marks (decision 0129; `tokenmark K oO kK sW …`): mark `K` holds where its
    /// record's bit does and its tokens follow, each element one record (the extras between
    /// skipped). Rising by mark.
    pub token_marks: Vec<TokenMark>,
    /// The counts of tokens (`COUNT`; `tokencount N max M`): each one's most.
    pub counts: Vec<u16>,
}

/// A token mark (`Words::token_marks`): its order, and its elements.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct TokenMark {
    pub order: u8,
    pub elems: Vec<TokenElem>,
}

/// One token of a token mark: its offset from the mark's token (-1 the one before), and what
/// the token's record may be: a kind of `kinds` (bit `k`: the records' kind `k`), or a word of
/// `words` (strings).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct TokenElem {
    pub offset: i8,
    pub kinds: u64,
    pub words: Vec<u16>,
}

impl TokenElem {
    /// Whether a record's code is one of the element's.
    pub fn takes(&self, code: u32) -> bool {
        let kind = code & 0xff;
        let word = crate::records::word_of(code);
        (kind < 64 && self.kinds >> kind & 1 == 1) || (word > 0 && self.words.contains(&((word - 1) as u16)))
    }
}

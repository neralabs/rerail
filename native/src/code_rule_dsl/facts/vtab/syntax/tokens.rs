//! core.rerail's token order (`first_token`, `last_token`, `token_before`, `token_after`), read off the rank index
//! (decision 0353 step 3): no row is made.
//!
//! The lexemes (`Item::Lexeme`: ESLint's token store) are the numbered tokens and the numbered nodes that are tape
//! leaves with text and no comment. They do not overlap, so in tape order their starts and ends only grow, and each
//! read is a rank and a select:
//! - a place's first token is the first lexeme at or after it on the tape, where its subtree holds one;
//! - its last token is the last lexeme before its subtree's end;
//! - the token before it is the last lexeme before it;
//! - the token after it is the first lexeme past its subtree.
//!
//! A place is a numbered node or token. Each relation's rows are the places that have its answer, marked by the walk
//! (`Marks::token_order`: `Leads`, `Follows`, `Precedes`), in tape order.

use super::ranks::Item;
use super::{NONE, Projected};

/// The relations read here, where a program reads one its projection marks the token order.
pub const VIEWS: [&str; 4] = ["first_token", "last_token", "token_before", "token_after"];

/// Which of the four.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TokRel {
    First,
    Last,
    Before,
    After,
}

impl TokRel {
    pub fn named(name: &str) -> Option<Self> {
        Some(match name {
            "first_token" => TokRel::First,
            "last_token" => TokRel::Last,
            "token_before" => TokRel::Before,
            "token_after" => TokRel::After,
            _ => return None,
        })
    }
    /// The places that have an answer: the relation's rows.
    pub fn rows(self) -> Item {
        match self {
            TokRel::First | TokRel::Last => Item::Leads,
            TokRel::Before => Item::Follows,
            TokRel::After => Item::Precedes,
        }
    }
    /// The columns, as core.rerail declares them.
    pub fn columns(self) -> &'static [&'static str] {
        match self {
            TokRel::First | TokRel::Last => &["n", "t"],
            TokRel::Before | TokRel::After => &["x", "t"],
        }
    }
}

impl Projected {
    /// Place `local`'s tape node: a node's, or (past the nodes) a token's.
    #[inline]
    pub fn place_node(&self, local: u32) -> u32 {
        if local < self.len() { self.node(local) } else { self.ranks.select(Item::Token, local - self.len()).unwrap_or(NONE) }
    }
    /// Tape node `x`'s place (a numbered node or token), `NONE` for none.
    #[inline]
    pub fn place_of(&self, x: u32) -> u32 {
        if self.ranks.is(Item::Node, x) {
            self.ranks.rank(Item::Node, x)
        } else if self.ranks.is(Item::Token, x) {
            self.len() + self.ranks.rank(Item::Token, x)
        } else {
            NONE
        }
    }
    /// The id of `rel`'s answer at place `local`, `-1` where it has none.
    pub fn token_of(&self, rel: TokRel, local: u32) -> i64 {
        let x = self.place_node(local);
        if x == NONE || !self.ranks.is(rel.rows(), x) {
            return -1;
        }
        let r = &self.ranks;
        let k = match rel {
            TokRel::First => r.rank(Item::Lexeme, x),
            TokRel::Last => r.rank_to(Item::Lexeme, self.facts.subtree_end(x)) - 1,
            TokRel::Before => r.rank(Item::Lexeme, x) - 1,
            TokRel::After => r.rank_to(Item::Lexeme, self.facts.subtree_end(x)),
        };
        r.select(Item::Lexeme, k).map(|t| self.place_of(t)).filter(|&l| l != NONE).map_or(-1, |l| self.global(l))
    }
    /// Row `i` of `rel`'s rows: its place and tape node.
    pub fn token_row(&self, rel: TokRel, i: u32) -> (u32, u32) {
        let x = self.ranks.select(rel.rows(), i).expect("a token order row");
        (self.place_of(x), x)
    }
    /// The row of place `local` among `rel`'s rows, if it has one.
    pub fn token_row_of(&self, rel: TokRel, local: u32) -> Option<u32> {
        let x = self.place_node(local);
        (x != NONE).then(|| self.ranks.local(rel.rows(), x)).flatten()
    }
    /// `rel`'s row count.
    pub fn token_rows(&self, rel: TokRel) -> u32 {
        self.ranks.count(rel.rows())
    }
}

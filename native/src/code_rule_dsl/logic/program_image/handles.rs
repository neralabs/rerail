//! Handles on a checked image's rules (decision 0173 §2): a rule, its head and body, each literal, each term, read in
//! place by id. A handle is two words (an image and a node), copied freely; nothing is decoded until asked.
//!
//! ```text
//! for rule in version.rules() {
//!     let head = rule.head();                       // AtomRef: rel(), args(), neg()
//!     for lit in rule.body() {                      // LitRef: node(), offset(), view()
//!         match lit.view() {
//!             Lit::Atom(a) => { for t in a.args() { if let TermRef::Var(s) = t { /* rule.var(s) */ } } }
//!             Lit::Cmp { l, op, r } => {}
//!             _ => {}
//!         }
//!     }
//! }
//! ```
//!
//! Names are ids: `AtomId` (text by `Version::text`), `RelId` (`Version::rel_name`), `Slot` (a rule's variable,
//! `RuleRef::var`). Handles rely on `Image::open`'s check of every rule's nodes, so reading never fails.
use super::{AtomId, Image, RelId, RuleId, Slot, kind};

/// A comparison's operator.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CmpOp {
    pub const ALL: [CmpOp; 6] = [CmpOp::Eq, CmpOp::Ne, CmpOp::Lt, CmpOp::Le, CmpOp::Gt, CmpOp::Ge];
    pub fn as_str(self) -> &'static str {
        ["=", "!=", "<", "<=", ">", ">="][self as usize]
    }
    pub fn of(op: &str) -> Option<CmpOp> {
        CmpOp::ALL.into_iter().find(|o| o.as_str() == op)
    }
}

/// A rule of an image, with its global id.
#[derive(Clone, Copy)]
pub struct RuleRef<'v> {
    im: &'v Image,
    k: u32,
    id: RuleId,
}

impl<'v> RuleRef<'v> {
    pub(crate) fn new(im: &'v Image, k: u32, id: RuleId) -> RuleRef<'v> {
        RuleRef { im, k, id }
    }
    pub fn id(self) -> RuleId {
        self.id
    }
    /// The hash of the rule's contents (its texts, not its ids): equal rules of any two images hash the same.
    pub fn hash(self) -> u64 {
        self.im.rule_hash(self.k)
    }
    pub(crate) fn image(self) -> &'v Image {
        self.im
    }
    pub(crate) fn local(self) -> u32 {
        self.k
    }
    fn at(self) -> u32 {
        self.im.rule_node(self.k)
    }
    /// The rule's nodes: its first and one past its last.
    pub(crate) fn nodes(self) -> std::ops::Range<u32> {
        let at = self.at();
        at..at + self.im.node(at).1
    }
    pub fn head(self) -> AtomRef<'v> {
        AtomRef { im: self.im, at: self.at() + 1 }
    }
    /// The body's literals, in order.
    pub fn body(self) -> Lits<'v> {
        let at = self.at();
        let head = at + 1;
        Lits { im: self.im, at: head + self.im.node(head).1, end: at + self.im.node(at).1, rule: at }
    }
    /// The atom naming each of the rule's variables, by slot.
    pub fn vars(self) -> impl ExactSizeIterator<Item = AtomId> + 'v {
        self.im.rule_vars(self.k).iter().map(|&a| AtomId(a))
    }
    /// The atom naming variable `s`.
    pub fn var(self, s: Slot) -> AtomId {
        AtomId(self.im.rule_vars(self.k)[s.index()])
    }
    pub fn var_count(self) -> usize {
        self.im.rule_vars(self.k).len()
    }
    /// The relations the body reads, positive or negated, in order (an aggregate's atom included).
    pub fn reads(self) -> impl Iterator<Item = AtomRef<'v>> + 'v {
        self.body().filter_map(|l| l.atom())
    }
}

/// An atom: a relation and its arguments.
#[derive(Clone, Copy)]
pub struct AtomRef<'v> {
    im: &'v Image,
    at: u32,
}

impl<'v> AtomRef<'v> {
    pub(crate) fn new(im: &'v Image, at: u32) -> AtomRef<'v> {
        AtomRef { im, at }
    }
    pub fn rel(self) -> RelId {
        RelId(self.im.node(self.at).2)
    }
    pub fn neg(self) -> bool {
        self.im.node(self.at).0 & kind::NEG != 0
    }
    pub fn gated(self) -> bool {
        self.im.node(self.at).0 & kind::GATED != 0
    }
    pub fn args(self) -> Terms<'v> {
        Terms { im: self.im, at: self.at + 1, end: self.at + self.im.node(self.at).1 }
    }
    pub fn arity(self) -> usize {
        self.args().count()
    }
    /// The atom's node in its image.
    pub fn node(self) -> u32 {
        self.at
    }
}

/// A term.
#[derive(Clone, Copy)]
pub enum TermRef<'v> {
    Var(Slot),
    Wild,
    Top,
    Int(i64),
    Str(AtomId),
    Call(AtomId, Terms<'v>),
}

impl TermRef<'_> {
    pub fn var(self) -> Option<Slot> {
        if let TermRef::Var(s) = self { Some(s) } else { None }
    }
}

fn term(im: &Image, at: u32) -> TermRef<'_> {
    let (k, size, val) = im.node(at);
    match k & kind::MASK {
        kind::VAR => TermRef::Var(Slot(val)),
        kind::WILD => TermRef::Wild,
        kind::TOP => TermRef::Top,
        kind::INT if k & kind::BIG != 0 => TermRef::Int(im.big(val)),
        kind::INT => TermRef::Int(val as i64),
        kind::STR => TermRef::Str(AtomId(val)),
        _ => TermRef::Call(AtomId(val), Terms { im, at: at + 1, end: at + size }),
    }
}

/// Sibling terms, in order.
#[derive(Clone, Copy)]
pub struct Terms<'v> {
    im: &'v Image,
    at: u32,
    end: u32,
}

impl<'v> Iterator for Terms<'v> {
    type Item = TermRef<'v>;
    fn next(&mut self) -> Option<TermRef<'v>> {
        if self.at >= self.end {
            return None;
        }
        let t = term(self.im, self.at);
        self.at += self.im.node(self.at).1;
        Some(t)
    }
}

/// An aggregate's operator.
#[derive(Clone, Copy)]
pub enum AggRef<'v> {
    Count,
    Min,
    Max,
    Sum,
    Product,
    /// A join of strings, ordered by `by`'s variables, separated by `sep`.
    Join { by: Slots<'v>, sep: AtomId },
}

/// Variables, in order.
#[derive(Clone, Copy)]
pub struct Slots<'v> {
    im: &'v Image,
    at: u32,
    end: u32,
}

impl Iterator for Slots<'_> {
    type Item = Slot;
    fn next(&mut self) -> Option<Slot> {
        (self.at < self.end).then(|| {
            self.at += 1;
            Slot(self.im.node(self.at - 1).2)
        })
    }
}

/// An `ast` literal's captures: (capture name, its variable).
#[derive(Clone, Copy)]
pub struct Captures<'v> {
    im: &'v Image,
    at: u32,
    end: u32,
}

impl Iterator for Captures<'_> {
    type Item = (AtomId, Slot);
    fn next(&mut self) -> Option<(AtomId, Slot)> {
        (self.at < self.end).then(|| {
            self.at += 2;
            (AtomId(self.im.node(self.at - 2).2), Slot(self.im.node(self.at - 1).2))
        })
    }
}

/// A body literal, viewed.
#[derive(Clone, Copy)]
pub enum Lit<'v> {
    Atom(AtomRef<'v>),
    Cmp { l: TermRef<'v>, op: CmpOp, r: TermRef<'v> },
    Agg { var: Slot, op: AggRef<'v>, over: Option<Slot>, atom: AtomRef<'v> },
    Nearest { n: Slot, a: Slot, kindset: AtomId },
    Ast { document: Slot, node: Slot, matcher: AtomId, captures: Captures<'v> },
}

/// A body literal: where it is, and what it is (`view`).
#[derive(Clone, Copy)]
pub struct LitRef<'v> {
    im: &'v Image,
    at: u32,
    rule: u32,
}

impl<'v> LitRef<'v> {
    /// The literal's node in its image.
    pub fn node(self) -> u32 {
        self.at
    }
    /// The literal's node from its rule's first: the same in every image the rule is copied to.
    pub fn offset(self) -> u32 {
        self.at - self.rule
    }
    /// The atom this literal reads: its own, or its aggregate's.
    pub fn atom(self) -> Option<AtomRef<'v>> {
        match self.view() {
            Lit::Atom(a) | Lit::Agg { atom: a, .. } => Some(a),
            _ => None,
        }
    }
    pub fn view(self) -> Lit<'v> {
        let im = self.im;
        let (k, size, val) = im.node(self.at);
        let end = self.at + size;
        match k & kind::MASK {
            kind::ATOM => Lit::Atom(AtomRef { im, at: self.at }),
            kind::CMP => {
                let l = self.at + 1;
                let r = l + im.node(l).1;
                Lit::Cmp { l: term(im, l), op: CmpOp::ALL[val as usize], r: term(im, r) }
            }
            kind::AGG => {
                let mut p = self.at + 1;
                let over = (im.node(p).0 & kind::MASK == kind::VAR).then(|| Slot(im.node(p).2));
                p += 1;
                let op = match k >> 5 {
                    0 => AggRef::Count,
                    1 => AggRef::Min,
                    2 => AggRef::Max,
                    3 => AggRef::Sum,
                    4 => AggRef::Product,
                    _ => {
                        let start = p;
                        while im.node(p).0 & kind::MASK == kind::VAR {
                            p += 1;
                        }
                        let sep = AtomId(im.node(p).2);
                        let by = Slots { im, at: start, end: p };
                        p += 1;
                        AggRef::Join { by, sep }
                    }
                };
                Lit::Agg { var: Slot(val), op, over, atom: AtomRef { im, at: p } }
            }
            kind::NEAREST => Lit::Nearest { n: Slot(im.node(self.at + 1).2), a: Slot(im.node(self.at + 2).2), kindset: AtomId(val) },
            _ => Lit::Ast { document: Slot(im.node(self.at + 1).2), node: Slot(im.node(self.at + 2).2), matcher: AtomId(val), captures: Captures { im, at: self.at + 3, end } },
        }
    }
}

/// A rule's body literals, in order.
#[derive(Clone, Copy)]
pub struct Lits<'v> {
    im: &'v Image,
    at: u32,
    end: u32,
    rule: u32,
}

impl<'v> Iterator for Lits<'v> {
    type Item = LitRef<'v>;
    fn next(&mut self) -> Option<LitRef<'v>> {
        if self.at >= self.end {
            return None;
        }
        let l = LitRef { im: self.im, at: self.at, rule: self.rule };
        self.at += self.im.node(self.at).1;
        Some(l)
    }
}

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

/// The ids of a union's part in the union (decision 0639 §4): the union's atom and relation of each of the part's,
/// by the part's id. A handle on a part's rule maps each id it reads through it; any other handle has none.
#[derive(Clone, Debug, Default)]
pub struct IdMap {
    pub atoms: Vec<u32>,
    pub rels: Vec<u32>,
}

/// Atom id `a` of an image, in its version's ids.
#[inline]
fn atom_in(map: Option<&IdMap>, a: u32) -> AtomId {
    AtomId(map.map_or(a, |m| m.atoms[a as usize]))
}

/// Relation id `r` of an image, in its version's ids.
#[inline]
fn rel_in(map: Option<&IdMap>, r: u32) -> RelId {
    RelId(map.map_or(r, |m| m.rels[r as usize]))
}

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
    map: Option<&'v IdMap>,
}

impl<'v> RuleRef<'v> {
    pub fn new(im: &'v Image, k: u32, id: RuleId) -> RuleRef<'v> {
        RuleRef { im, k, id, map: None }
    }
    /// The rule as a union's (`Version::Union`): its id the union's, its ids mapped by `map`.
    pub fn in_union(self, id: RuleId, map: &'v IdMap) -> RuleRef<'v> {
        RuleRef { im: self.im, k: self.k, id, map: Some(map) }
    }
    /// The map its ids are read through: a union's part's, else none.
    pub fn map(self) -> Option<&'v IdMap> {
        self.map
    }
    /// An atom id of its image's nodes (`image`), in its version's ids.
    pub fn atom_of(self, a: u32) -> AtomId {
        atom_in(self.map, a)
    }
    /// A relation id of its image's nodes, in its version's ids.
    pub fn rel_of(self, r: u32) -> RelId {
        rel_in(self.map, r)
    }
    pub fn id(self) -> RuleId {
        self.id
    }
    /// The hash of the rule's contents (its texts, not its ids): equal rules of any two images hash the same.
    pub fn hash(self) -> u64 {
        self.im.rule_hash(self.k)
    }
    /// The rule of a derivation's input this rule came from (§9), if it came from one.
    pub fn source(self) -> Option<RuleId> {
        self.im.source(self.k)
    }
    pub fn image(self) -> &'v Image {
        self.im
    }
    pub fn local(self) -> u32 {
        self.k
    }
    fn at(self) -> u32 {
        self.im.rule_node(self.k)
    }
    /// The rule's nodes: its first and one past its last.
    pub fn nodes(self) -> std::ops::Range<u32> {
        let at = self.at();
        at..at + self.im.node(at).1
    }
    pub fn head(self) -> AtomRef<'v> {
        AtomRef { im: self.im, at: self.at() + 1, map: self.map }
    }
    /// The body's literals, in order.
    pub fn body(self) -> Lits<'v> {
        let at = self.at();
        let head = at + 1;
        Lits { im: self.im, at: head + self.im.node(head).1, end: at + self.im.node(at).1, rule: at, map: self.map }
    }
    /// The atom naming each of the rule's variables, by slot.
    pub fn vars(self) -> impl ExactSizeIterator<Item = AtomId> + 'v {
        let map = self.map;
        self.im.rule_vars(self.k).iter().map(move |&a| atom_in(map, a))
    }
    /// The atom naming variable `s`.
    pub fn var(self, s: Slot) -> AtomId {
        atom_in(self.map, self.im.rule_vars(self.k)[s.index()])
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
    map: Option<&'v IdMap>,
}

impl<'v> AtomRef<'v> {
    pub fn new(im: &'v Image, at: u32) -> AtomRef<'v> {
        AtomRef { im, at, map: None }
    }
    /// The atom at node `at` of a rule read through `map` (`RuleRef::map`).
    pub fn mapped(im: &'v Image, at: u32, map: Option<&'v IdMap>) -> AtomRef<'v> {
        AtomRef { im, at, map }
    }
    /// The map its ids are read through.
    pub fn map(self) -> Option<&'v IdMap> {
        self.map
    }
    pub fn rel(self) -> RelId {
        rel_in(self.map, self.im.node(self.at).2)
    }
    #[allow(clippy::should_implement_trait, reason = "whether the literal is negated, not `ops::Neg`")]
    pub fn neg(self) -> bool {
        self.im.node(self.at).0 & kind::NEG != 0
    }
    pub fn gated(self) -> bool {
        self.im.node(self.at).0 & kind::GATED != 0
    }
    pub fn args(self) -> Terms<'v> {
        Terms { im: self.im, at: self.at + 1, end: self.at + self.im.node(self.at).1, map: self.map }
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
    /// A branded atom (decision 0270); at run time its value, `atom`.
    Branded(crate::code_rule_dsl::logic::front::ast::Branded),
}

impl TermRef<'_> {
    pub fn var(self) -> Option<Slot> {
        if let TermRef::Var(s) = self { Some(s) } else { None }
    }
}

pub fn term(im: &Image, at: u32) -> TermRef<'_> {
    term_in(im, at, None)
}

/// The term at node `at` of `im`, its ids read through `map` (`RuleRef::map`).
pub fn term_in<'v>(im: &'v Image, at: u32, map: Option<&'v IdMap>) -> TermRef<'v> {
    let (k, size, val) = im.node(at);
    match k & kind::MASK {
        kind::VAR => TermRef::Var(Slot(val)),
        kind::WILD => TermRef::Wild,
        kind::TOP => TermRef::Top,
        kind::INT if k & kind::BIG != 0 => TermRef::Int(im.big(val)),
        kind::INT => TermRef::Int(val as i64),
        kind::STR => TermRef::Str(atom_in(map, val)),
        kind::BRANDED => TermRef::Branded(super::brand_of(k >> 5 & 3, atom_in(map, val).0)),
        _ => TermRef::Call(atom_in(map, val), Terms { im, at: at + 1, end: at + size, map }),
    }
}

/// Sibling terms, in order.
#[derive(Clone, Copy)]
pub struct Terms<'v> {
    im: &'v Image,
    at: u32,
    end: u32,
    map: Option<&'v IdMap>,
}

impl<'v> Iterator for Terms<'v> {
    type Item = TermRef<'v>;
    fn next(&mut self) -> Option<TermRef<'v>> {
        if self.at >= self.end {
            return None;
        }
        let t = term_in(self.im, self.at, self.map);
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
    /// The extreme on a bound's side (`AggOp::Bounded`, decision 0511).
    Bounded { max: bool, bound: Slot, strict: bool },
}

impl AggRef<'_> {
    /// A bounded aggregate's bound: a variable it reads from outside its body (decision 0511).
    pub fn bound(&self) -> Option<Slot> {
        match self {
            AggRef::Bounded { bound, .. } => Some(*bound),
            _ => None,
        }
    }
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
    map: Option<&'v IdMap>,
}

impl Iterator for Captures<'_> {
    type Item = (AtomId, Slot);
    fn next(&mut self) -> Option<(AtomId, Slot)> {
        (self.at < self.end).then(|| {
            self.at += 2;
            (atom_in(self.map, self.im.node(self.at - 2).2), Slot(self.im.node(self.at - 1).2))
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
    map: Option<&'v IdMap>,
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
        let map = self.map;
        let (k, size, val) = im.node(self.at);
        let end = self.at + size;
        match k & kind::MASK {
            kind::ATOM => Lit::Atom(AtomRef { im, at: self.at, map }),
            kind::CMP => {
                let l = self.at + 1;
                let r = l + im.node(l).1;
                Lit::Cmp { l: term_in(im, l, map), op: CmpOp::ALL[val as usize], r: term_in(im, r, map) }
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
                    6 => {
                        // The bound's variable, then its flags: 1 a max, 2 strict.
                        let bound = Slot(im.node(p).2);
                        let flags = im.node(p + 1).2;
                        p += 2;
                        AggRef::Bounded { max: flags & 1 != 0, bound, strict: flags & 2 != 0 }
                    }
                    _ => {
                        let start = p;
                        while im.node(p).0 & kind::MASK == kind::VAR {
                            p += 1;
                        }
                        let sep = atom_in(map, im.node(p).2);
                        let by = Slots { im, at: start, end: p };
                        p += 1;
                        AggRef::Join { by, sep }
                    }
                };
                Lit::Agg { var: Slot(val), op, over, atom: AtomRef { im, at: p, map } }
            }
            kind::NEAREST => Lit::Nearest { n: Slot(im.node(self.at + 1).2), a: Slot(im.node(self.at + 2).2), kindset: atom_in(map, val) },
            _ => Lit::Ast { document: Slot(im.node(self.at + 1).2), node: Slot(im.node(self.at + 2).2), matcher: atom_in(map, val), captures: Captures { im, at: self.at + 3, end, map } },
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
    map: Option<&'v IdMap>,
}

impl<'v> Iterator for Lits<'v> {
    type Item = LitRef<'v>;
    fn next(&mut self) -> Option<LitRef<'v>> {
        if self.at >= self.end {
            return None;
        }
        let l = LitRef { im: self.im, at: self.at, rule: self.rule, map: self.map };
        self.at += self.im.node(self.at).1;
        Some(l)
    }
}

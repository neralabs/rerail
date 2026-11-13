//! A program's text read straight off its tape image into the program's parts (decision 0162): the lean parser's
//! image (`rerail_rr_syntax::tape`) walked in place (`rerail_rr_syntax::tape_format::View`), each node read from its columns
//! once, with no nodes decoded, no tree indexed and no statements made between. A name is copied once, into the
//! part that holds it; a string once, unescaped only when it holds a `\`.
//!
//! Each list (arguments, a body, columns, a row) is gathered in the walk's own stack and taken from the run's
//! `Lists` (decision 0172): a list the run has already met is that list, and makes nothing.
//!
//! It reads what `program_text::print` writes, and refuses the rest as `read` over the statements did.

use crate::code_rule_dsl::logic::front::{AggOp, Atom, List, Lists, Literal, Name, Rule, Stacks, Term, Value};
use std::cell::RefCell;
use rerail_rr_syntax::tape::{ERROR, Field, Kind};
use rerail_rr_syntax::tape_format::{CODE_ERROR, CODE_EXTRA, KIND_BITS, View};
use std::collections::BTreeMap;

/// The variables the parser reads as words.
pub const VAR_WORDS: [&str; 10] = ["top", "count", "min", "max", "sum", "product", "mean", "join", "by", "sep"];

/// The relations named as this text's literals.
pub const REL_WORDS: [&str; 2] = ["nearest", "ast"];

/// Each relation's rows (`program_facts::Facts`).
pub type Facts = BTreeMap<String, Vec<List<Value>>>;

/// `name` with its escape (`program_text::escape`) undone.
pub fn unescape(name: &str, words: &[&str]) -> String {
    unescaped(name, words).to_owned()
}

/// `unescape`, borrowed: the name, or the name less its last `_`.
#[inline]
pub fn unescaped<'s>(name: &'s str, words: &[&str]) -> &'s str {
    match name.strip_suffix('_') {
        Some(stripped) if words.contains(&name.trim_end_matches('_')) => stripped,
        _ => name,
    }
}

/// A `number` node's value: its digits, after a `-` and the spaces after it (`- 1` is one token).
fn number(t: &str) -> Option<i64> {
    let (neg, digits) = match t.strip_prefix('-') {
        Some(rest) => (true, rest.trim_start()),
        None => (false, t),
    };
    if digits.is_empty() || digits.len() > 19 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // At most 19 digits, so the magnitude fits a u64; then it must fit an i64 (the lexer reads it as one).
    let m = digits.bytes().fold(0u64, |m, b| m * 10 + u64::from(b - b'0'));
    let m = i64::try_from(m).ok()?;
    Some(if neg { -m } else { m })
}

fn bad(what: impl std::fmt::Display) -> String {
    format!("program text: {what}")
}

/// The parts a text holds, as read: the rest of the program is made from `about` (`program_facts::program`).
#[derive(Default)]
pub struct Parts {
    pub decls: BTreeMap<Name, List<(Name, Name)>>,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub facts: Facts,
    pub rules: Vec<Rule>,
    pub about: Facts,
}

#[derive(Clone, Copy, PartialEq)]
enum Part {
    Facts,
    Rules,
    About,
}

const fn k(kind: Kind) -> u16 {
    kind as u16
}

const fn f(field: Field) -> u16 {
    field as u16
}

struct Walk<'a> {
    src: &'a str,
    view: View<'a>,
    /// The names met in this text, by their text: each interned once a text (`Name::new` takes the process's
    /// table's lock), then cloned by a count.
    names: RefCell<rustc_hash::FxHashMap<&'a str, Name>>,
    /// The run's distinct lists.
    lists: &'a Lists,
    /// Argument lists and declarations' columns met in this text, by their text: the same text is the same list,
    /// taken again without walking it.
    arguments: RefCell<rustc_hash::FxHashMap<&'a str, List<Term>>>,
    declared: RefCell<rustc_hash::FxHashMap<&'a str, List<(Name, Name)>>>,
    /// Where each kind of list is gathered before it is taken from `lists`: one stack each, nested lists above.
    terms: RefCell<Vec<Term>>,
    literals: RefCell<Vec<Literal>>,
    columns: RefCell<Vec<(Name, Name)>>,
    values: RefCell<Vec<Value>>,
}

/// `f` of each of `items`, gathered on `stack` above what it holds and taken as one list by `take`; the stack is
/// left as it was found.
fn gather<T, I: Iterator<Item = usize>>(stack: &RefCell<Vec<T>>, items: I, f: impl Fn(usize) -> Result<T, String>, take: impl FnOnce(&[T]) -> List<T>) -> Result<List<T>, String> {
    let mark = stack.borrow().len();
    let mut made = Ok(());
    for c in items {
        match f(c) {
            Ok(t) => stack.borrow_mut().push(t),
            Err(e) => {
                made = Err(e);
                break;
            }
        }
    }
    let list = made.map(|()| take(&stack.borrow()[mark..]));
    stack.borrow_mut().truncate(mark);
    list
}

impl<'a> Walk<'a> {
    #[inline(always)]
    fn kind(&self, n: usize) -> u16 {
        let key = self.view.key(n);
        if key >> 16 & u32::from(CODE_ERROR) != 0 { ERROR } else { (key & ((1 << KIND_BITS) - 1)) as u16 }
    }

    #[inline(always)]
    fn field(&self, n: usize) -> u16 {
        (self.view.key(n) >> KIND_BITS & 0x3f) as u16
    }

    #[inline(always)]
    fn text(&self, n: usize) -> &'a str {
        self.src.get(self.view.start(n) as usize..self.view.end(n) as usize).unwrap_or("")
    }

    /// `n`'s children, comments left out.
    #[inline(always)]
    fn children(&self, n: usize) -> impl Iterator<Item = usize> + '_ {
        self.view.children(n).filter(|&c| self.view.key(c) >> 16 & u32::from(CODE_EXTRA) == 0)
    }

    /// `n`'s children of `field`, in order.
    #[inline(always)]
    fn all(&self, n: usize, field: Field) -> impl Iterator<Item = usize> + '_ {
        self.children(n).filter(move |&c| self.field(c) == f(field))
    }

    fn get(&self, n: usize, field: Field) -> Option<usize> {
        self.children(n).find(|&c| self.field(c) == f(field))
    }

    fn one(&self, n: usize, field: Field) -> Result<usize, String> {
        self.get(n, field).ok_or_else(|| bad(format!("a {} with no {field:?}", rerail_rr_syntax::tape::KIND_NAMES.get(usize::from(self.kind(n))).unwrap_or(&"?"))))
    }

    fn name(&self, n: usize, field: Field) -> Result<&'a str, String> {
        Ok(self.text(self.one(n, field)?))
    }

    /// An anonymous child written `text`.
    fn has_token(&self, n: usize, text: &str) -> bool {
        self.children(n).any(|c| self.kind(c) >= rerail_rr_syntax::tape::FIRST_ANONYMOUS && self.kind(c) != ERROR && self.text(c) == text)
    }

    /// `text` as a name: from this text's table, or interned and kept there.
    #[inline]
    fn intern(&self, text: &'a str) -> Name {
        if let Some(name) = self.names.borrow().get(text) {
            return name.clone();
        }
        let name = Name::new(text);
        self.names.borrow_mut().insert(text, name.clone());
        name
    }

    /// A string node's value as a name: its text's span when it holds no escape, so no string is made.
    fn string_name(&self, n: usize) -> Result<Name, String> {
        if self.kind(n) != k(Kind::String) {
            return Err(bad(format!("{:?} where a string goes", self.text(n))));
        }
        let t = self.text(n);
        let body = t.strip_prefix('"').and_then(|t| t.strip_suffix('"')).ok_or_else(|| bad(format!("a string {t:?}")))?;
        if body.contains('\\') { rerail_rr_syntax::syntax::unescape(body).map(|s| Name::new(&s)).map_err(bad) } else { Ok(self.intern(body)) }
    }

    /// A variable's name, its escape undone, interned.
    fn var_name(&self, v: &'a str) -> Name {
        self.intern(unescaped(v, &VAR_WORDS))
    }

    fn string(&self, n: usize) -> Result<String, String> {
        if self.kind(n) != k(Kind::String) {
            return Err(bad(format!("{:?} where a string goes", self.text(n))));
        }
        let t = self.text(n);
        let body = t.strip_prefix('"').and_then(|t| t.strip_suffix('"')).ok_or_else(|| bad(format!("a string {t:?}")))?;
        if body.contains('\\') { rerail_rr_syntax::syntax::unescape(body).map_err(bad) } else { Ok(body.to_owned()) }
    }

    fn var(&self, n: usize) -> Result<String, String> {
        match self.kind(n) {
            x if x == k(Kind::Variable) && self.text(n) != "top" => Ok(unescape(self.text(n), &VAR_WORDS)),
            _ => Err(bad(format!("{:?} where a variable goes", self.text(n)))),
        }
    }

    fn int(&self, n: usize) -> Result<i64, String> {
        match self.kind(n) {
            x if x == k(Kind::Number) => number(self.text(n)).ok_or_else(|| bad(format!("a number {:?}", self.text(n)))),
            x if x == k(Kind::ParenthesizedExpression) => self.int(self.one(n, Field::Expression)?),
            x if x == k(Kind::UnaryExpression) => 0i64.checked_sub(self.int(self.one(n, Field::Operand)?)?).ok_or_else(|| bad("a number out of range")),
            x if x == k(Kind::BinaryExpression) && self.name(n, Field::Operator)? == "-" => {
                self.int(self.one(n, Field::Left)?)?.checked_sub(self.int(self.one(n, Field::Right)?)?).ok_or_else(|| bad("a number out of range"))
            }
            _ => Err(bad(format!("{:?} where a number goes", self.text(n)))),
        }
    }

    fn value(&self, n: usize) -> Result<Value, String> {
        if self.kind(n) == k(Kind::String) { self.string_name(n).map(Value::Str) } else { self.int(n).map(Value::Int) }
    }

    fn term(&self, n: usize) -> Result<Term, String> {
        Ok(match self.kind(n) {
            x if x == k(Kind::Variable) => match self.text(n) {
                "top" => Term::Top,
                v => Term::Var(self.var_name(v)),
            },
            x if x == k(Kind::Wildcard) => Term::Wild,
            x if x == k(Kind::String) => Term::Str(self.string_name(n)?),
            x if x == k(Kind::FunctorCall) => {
                let args = match self.get(n, Field::Arguments) {
                    Some(a) => self.terms(a)?,
                    None => List::new(),
                };
                Term::Call(self.intern(self.name(n, Field::Function)?), args)
            }
            _ => Term::Int(self.int(n)?),
        })
    }

    /// An argument list's terms: this text's list of the same text, else the run's.
    fn terms(&self, list: usize) -> Result<List<Term>, String> {
        let text = self.text(list);
        if let Some(found) = self.arguments.borrow().get(text) {
            return Ok(found.clone());
        }
        let made = gather(&self.terms, self.all(list, Field::Argument), |c| self.term(c), |t| self.lists.terms(t))?;
        self.arguments.borrow_mut().insert(text, made.clone());
        Ok(made)
    }

    /// An atom (or a head): its relation and its arguments.
    fn atom(&self, n: usize, neg: bool) -> Result<Atom, String> {
        let (mut rel, mut args) = (None, None);
        for c in self.children(n) {
            match self.field(c) {
                x if x == f(Field::Relation) => rel = Some(self.text(c)),
                x if x == f(Field::Arguments) => args = Some(self.terms(c)?),
                x if x == f(Field::Modal) => return Err(bad("a modal head")),
                _ => {}
            }
        }
        let rel = rel.ok_or_else(|| bad("an atom with no relation"))?;
        Ok(Atom { rel: self.intern(unescaped(rel, &REL_WORDS)), args: args.ok_or_else(|| bad("an atom with no arguments"))?, neg, gated: false })
    }

    /// A body: one conjunction.
    fn conjunction(&self, b: usize) -> Result<usize, String> {
        if self.kind(b) != k(Kind::Conjunction) {
            return Err(bad("a body of other than one conjunction"));
        }
        Ok(b)
    }

    fn literal(&self, l: usize) -> Result<Literal, String> {
        Ok(match self.kind(l) {
            x if x == k(Kind::Atom) => {
                let rel = self.name(l, Field::Relation)?;
                if rel == "nearest" || rel == "ast" {
                    let list = self.one(l, Field::Arguments)?;
                    // Rare (a few hundred in a run's programs): the arguments' nodes collected.
                    let args: Vec<usize> = self.all(list, Field::Argument).collect();
                    let args = args.as_slice();
                    if rel == "nearest" {
                        let [n, a, set] = args else { return Err(bad("nearest of other than three arguments")) };
                        Literal::Nearest { n: self.var(*n)?, a: self.var(*a)?, kindset: self.string(*set)? }
                    } else {
                        let [document, node, matcher, captures @ ..] = args else { return Err(bad("ast of under three arguments")) };
                        if captures.len() % 2 != 0 {
                            return Err(bad("an ast capture without its variable"));
                        }
                        let captures = captures.chunks(2).map(|c| Ok((self.string(c[0])?, self.var(c[1])?))).collect::<Result<_, String>>()?;
                        Literal::Ast { document: self.var(*document)?, node: self.var(*node)?, matcher: self.string(*matcher)?, captures }
                    }
                } else {
                    Literal::Atom(self.atom(l, false)?)
                }
            }
            x if x == k(Kind::Negation) => Literal::Atom(self.negated(l)?),
            x if x == k(Kind::Comparison) => {
                let (left, right) = (self.one(l, Field::Left)?, self.one(l, Field::Right)?);
                let op = match self.name(l, Field::Operator)? {
                    "=" => "=",
                    "!=" => "!=",
                    "<" => "<",
                    "<=" => "<=",
                    ">" => ">",
                    ">=" => ">=",
                    other => return Err(bad(format!("comparison {other:?}"))),
                };
                if op == "=" && self.kind(left) == k(Kind::Variable) && self.kind(right) == k(Kind::Aggregate) {
                    self.aggregate(self.text(left), right)?
                } else {
                    Literal::Cmp { l: self.term(left)?, op, r: self.term(right)? }
                }
            }
            _ => return Err(bad(format!("literal {:?}", self.text(l)))),
        })
    }

    /// A negation's atom, each `!` turning it.
    fn negated(&self, l: usize) -> Result<Atom, String> {
        let operand = self.one(l, Field::Operand)?;
        match self.kind(operand) {
            x if x == k(Kind::Atom) => self.atom(operand, true),
            x if x == k(Kind::Negation) => self.negated(operand).map(|a| Atom { neg: !a.neg, ..a }),
            _ => Err(bad(format!("a negated {:?}", self.text(operand)))),
        }
    }

    /// `v = op over by (k, …) sep "s" : { atom }`.
    fn aggregate(&self, v: &str, n: usize) -> Result<Literal, String> {
        let (mut op, mut over, mut by, mut sep, mut body) = ("count", None, Vec::new(), None, None);
        for c in self.children(n) {
            match self.field(c) {
                x if x == f(Field::Operator) => op = self.text(c),
                x if x == f(Field::Value) => over = Some(c),
                x if x == f(Field::Key) => by.push(self.var(c)?),
                x if x == f(Field::Separator) => sep = Some(self.string(c)?),
                x if x == f(Field::Body) => body = Some(c),
                _ => {}
            }
        }
        let body = self.conjunction(body.ok_or_else(|| bad("an aggregate with no body"))?)?;
        let mut lits = self.all(body, Field::Literal);
        let (Some(l), None) = (lits.next(), lits.next()) else { return Err(bad("an aggregate over other than one atom")) };
        let atom = match self.kind(l) {
            x if x == k(Kind::Atom) => self.atom(l, false)?,
            x if x == k(Kind::Negation) => self.negated(l)?,
            _ => return Err(bad("an aggregate over other than one atom")),
        };
        let over = match over {
            Some(o) if self.kind(o) != k(Kind::Wildcard) => Some(self.var(o)?),
            _ => None,
        };
        let op = match op {
            "count" => AggOp::Count,
            "min" => AggOp::Min,
            "max" => AggOp::Max,
            "sum" => AggOp::Sum,
            "product" => AggOp::Product,
            "join" => AggOp::Join { by, sep: sep.ok_or_else(|| bad("a join without its separator"))? },
            other => return Err(bad(format!("aggregate {other:?}"))),
        };
        Ok(Literal::Agg { var: unescape(v, &VAR_WORDS), op, over, atom })
    }

    /// A declaration's columns, each its name and type: this text's list of the same text, else the run's.
    fn columns(&self, s: usize) -> Result<List<(Name, Name)>, String> {
        let list = self.one(s, Field::Columns)?;
        let text = self.text(list);
        if let Some(found) = self.declared.borrow().get(text) {
            return Ok(found.clone());
        }
        let made = gather(&self.columns, self.all(list, Field::Column), |c| {
                let ty = self.one(c, Field::Type)?;
                let name = self.name(ty, Field::Name)?;
                let ty: Name = match self.get(ty, Field::Argument) {
                    Some(a) => format!("{name}<{}>", self.text(a)).into(),
                    None if self.has_token(ty, "<") => format!("{name}<>").into(),
                    None => self.intern(name),
                };
                Ok((self.intern(self.name(c, Field::Name)?), ty))
            }, |c| self.lists.columns(c))?;
        self.declared.borrow_mut().insert(text, made.clone());
        Ok(made)
    }

    fn statement(&self, s: usize, part: &mut Part, out: &mut Parts, decls: &mut Vec<(Name, List<(Name, Name)>)>) -> Result<(), String> {
        match self.kind(s) {
            x if x == k(Kind::Setting) => match self.name(s, Field::Name)? {
                "rules" => *part = Part::Rules,
                "about" => *part = Part::About,
                other => return Err(bad(format!("setting .{other}"))),
            },
            x if x == k(Kind::Declaration) => {
                decls.push((self.intern(unescaped(self.name(s, Field::Name)?, &REL_WORDS)), self.columns(s)?));
            }
            x if x == k(Kind::IoDirective) => {
                let into = if self.has_token(s, ".output") { &mut out.outputs } else { &mut out.inputs };
                for r in self.children(s).filter(|&c| self.field(c) == f(Field::Relation)) {
                    into.push(unescape(self.name(r, Field::Name)?, &REL_WORDS));
                }
            }
            x if x == k(Kind::Clause) => {
                let (mut head, mut body) = (None, None);
                for c in self.children(s) {
                    match self.field(c) {
                        x if x == f(Field::Head) && head.is_none() => head = Some(c),
                        x if x == f(Field::Body) => body = Some(c),
                        x if x == f(Field::Head) || x == f(Field::Edit) => return Err(bad(format!("clause {:?}", self.text(s)))),
                        _ => {}
                    }
                }
                let head = head.ok_or_else(|| bad("a clause with no head"))?;
                if *part == Part::Rules {
                    let lits = match body {
                        Some(b) => gather(&self.literals, self.all(self.conjunction(b)?, Field::Literal), |l| self.literal(l), |l| self.lists.literals(l))?,
                        None => List::new(),
                    };
                    out.rules.push(Rule { head: self.atom(head, false)?, body: lits });
                } else if body.is_none() {
                    let mut rel = None;
                    let mut row = List::new();
                    for c in self.children(head) {
                        match self.field(c) {
                            x if x == f(Field::Relation) => rel = Some(self.text(c)),
                            x if x == f(Field::Arguments) => row = gather(&self.values, self.all(c, Field::Argument), |a| self.value(a), |v| self.lists.values(v))?,
                            x if x == f(Field::Modal) => return Err(bad("a modal head")),
                            _ => {}
                        }
                    }
                    let rel = unescaped(rel.ok_or_else(|| bad("a fact with no relation"))?, &REL_WORDS);
                    let into = if *part == Part::About { &mut out.about } else { &mut out.facts };
                    // A relation's rows are written together, and the relations in order (`print`): a row is most often
                    // the last relation's, found without a search. Else the map is searched, and a key made once.
                    match into.last_entry() {
                        Some(mut last) if last.key() == rel => last.get_mut().push(row),
                        _ => match into.get_mut(rel) {
                            Some(rows) => rows.push(row),
                            None => {
                                into.insert(rel.to_owned(), vec![row]);
                            }
                        },
                    }
                } else {
                    return Err(bad(format!("a rule before .rules: {:?}", self.text(s))));
                }
            }
            _ => return Err(bad(format!("statement {:?}", self.text(s)))),
        }
        Ok(())
    }
}

/// The parts of `src`, read off its image `image` (`rerail_rr_syntax::tape(src)`), its lists taken from `lists`.
pub fn parts(src: &str, image: &[u8], lists: &Lists) -> Result<Parts, String> {
    let view = View::new(image)?;
    let stacks = lists.take_stacks();
    // Room for what a text of this many nodes meets: in the run's programs, about a distinct name every 40 nodes, an
    // argument list every 90 and a declaration's columns every 700.
    let nodes = view.len();
    let walk = Walk {
        src,
        view,
        names: RefCell::new(rustc_hash::FxHashMap::with_capacity_and_hasher(nodes / 40, Default::default())),
        lists,
        arguments: RefCell::new(rustc_hash::FxHashMap::with_capacity_and_hasher(nodes / 90, Default::default())),
        declared: RefCell::new(rustc_hash::FxHashMap::with_capacity_and_hasher(nodes / 700, Default::default())),
        terms: RefCell::new(stacks.terms),
        literals: RefCell::new(stacks.literals),
        columns: RefCell::new(stacks.columns),
        values: RefCell::new(stacks.values),
    };
    let mut out = Parts::default();
    let mut decls = Vec::new();
    let read = walk_statements(&walk, &mut out, &mut decls);
    // Declarations gathered and the map built at once (`BTreeMap::from_iter`: full nodes, the last of a name kept).
    out.decls = decls.into_iter().collect();
    let Walk { terms, literals, columns, values, .. } = walk;
    lists.give_stacks(Stacks { terms: terms.into_inner(), literals: literals.into_inner(), columns: columns.into_inner(), values: values.into_inner() });
    read.map(|()| out)
}

fn walk_statements(walk: &Walk<'_>, out: &mut Parts, decls: &mut Vec<(Name, List<(Name, Name)>)>) -> Result<(), String> {
    if walk.view.is_empty() {
        return Ok(());
    }
    let mut part = Part::Facts;
    out.rules.reserve(walk.children(0).count());
    for s in walk.children(0) {
        if walk.kind(s) == ERROR {
            return Err(bad(format!("a statement that does not parse: {:?}", walk.text(s))));
        }
        walk.statement(s, &mut part, out, decls)?;
    }
    Ok(())
}

//! Moved from `front.rs` by the crate split (tools/cratify/moves.tsv).

use rerail_blocks::vector::Vector;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use crate::code_rule_dsl::logic::front::name::Name;

/// What an atom names (decision 0270): a name is the store's (interned, numbered as the store met it); a kind or a
/// field is the grammar's (`kind_atom`, `field_atom`), the same in every store.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Class {
    Name,
    Kind,
    Field,
}

/// An atom with what it names (decision 0270). Its value at run time is `atom`, as an integer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Branded {
    pub class: Class,
    pub atom: u32,
}

impl Branded {
    pub fn name(atom: u32) -> Branded {
        Branded { class: Class::Name, atom }
    }
    /// Its value as the run holds it.
    pub fn value(self) -> i64 {
        i64::from(self.atom)
    }
    /// Its printed tag (`#n12`, `#k1034`, `#f3`), which the rr parser reads back.
    pub fn tag(self) -> char {
        match self.class {
            Class::Name => 'n',
            Class::Kind => 'k',
            Class::Field => 'f',
        }
    }
    pub fn of_tag(tag: char, atom: u32) -> Option<Branded> {
        let class = match tag {
            'n' => Class::Name,
            'k' => Class::Kind,
            'f' => Class::Field,
            _ => return None,
        };
        Some(Branded { class, atom })
    }
}

impl fmt::Display for Branded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}{}", self.tag(), self.atom)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AggOp {
    Count,
    Min,
    Max,
    /// `sum x : { … }`: the sum of `x` over the body's rows, 0 when there are none.
    Sum,
    /// `product x : {…}`: the product of `x` over the rows, 1 over no rows, saturating at i64's range (a count of
    /// combinations, as pytest's stacked parametrize marks multiply).
    Product,
    /// `join x by (k, …) sep "s" : { … }`: the distinct `(k, …, x)` rows' `x` texts ordered by `k, …` then `x`
    /// (symbols by code point, numbers by value), separated by `sep`; `""` when there are none.
    Join { by: Vec<String>, sep: String },
    /// `max t : { … }` with `t < r` (or `<=`), `min` with `t > r` (or `>=`), `r` bound outside the body (decision
    /// 0511): the extreme of the body's values on `bound`'s side of it, no row when there are none. The body is the
    /// group's values alone, read once for every probe.
    Bounded { max: bool, bound: String, strict: bool },
}

impl AggOp {
    /// A bounded aggregate's bound: a variable it reads from outside its body (decision 0511).
    pub fn bound(&self) -> Option<&String> {
        match self {
            AggOp::Bounded { bound, .. } => Some(bound),
            _ => None,
        }
    }

    /// The comparison a bounded aggregate's value passes: `t op bound`.
    pub fn bound_cmp(max: bool, strict: bool) -> &'static str {
        match (max, strict) {
            (true, true) => "<",
            (true, false) => "<=",
            (false, true) => ">",
            (false, false) => ">=",
        }
    }
}

/// A relation's lattice column: the last, after `keys` key columns.
#[derive(Clone, Debug, PartialEq)]
pub struct LatticeDecl {
    pub keys: usize,
    /// The value column's type, which names the lattice (`flat`, `min`, `max`) unless `merge` does.
    pub ty: String,
    pub merge: Option<String>,
    pub widen: Option<u32>,
}

impl LatticeDecl {
    /// The lattice's name for `logic::native` (`declare`d when `merge` names a native join).
    pub fn lattice(&self) -> &str {
        self.merge.as_deref().unwrap_or(&self.ty)
    }

    pub fn bounded(&self) -> bool {
        self.widen.is_some() || (self.merge.is_none() && self.ty == "flat")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Value {
    Int(i64),
    Str(Name),
    /// A string a fact held, lowered to its atom (decision 0270).
    Branded(Branded),
}

/// A join shape directive written after a rule (`.shape posarg after pv`, `.shape retparam bound f`): in the rules of
/// `head`, the literal of `lit` is joined only once `wait` holds. A rule's driving delta is never held back. The
/// planner searches within it, so a directive narrows the search; it does not fix an order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shape {
    pub head: String,
    pub lit: String,
    pub wait: ShapeWait,
    /// As written, for the profile.
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShapeWait {
    /// Once a literal of this relation is joined.
    After(String),
    /// Once these variables are bound.
    Bound(Vec<String>),
}

impl Shape {
    /// `B after A` or `B bound x, y` (the directive's text), on the rules of `head`.
    pub fn parse(head: &str, text: &str) -> Result<Shape, String> {
        let words: Vec<&str> = text.split(|c: char| c.is_whitespace() || c == ',').filter(|w| !w.is_empty()).collect();
        let wait = match words.as_slice() {
            [_, "after", a] => ShapeWait::After((*a).to_owned()),
            [_, "bound", vars @ ..] if !vars.is_empty() => ShapeWait::Bound(vars.iter().map(|v| v.trim_start_matches('$').to_owned()).collect()),
            _ => return Err(format!("`.shape {text}`: write `.shape <relation> after <relation>` or `.shape <relation> bound <variable>, …`")),
        };
        Ok(Shape { head: head.to_owned(), lit: words[0].to_owned(), wait, text: text.trim().to_owned() })
    }
}

/// How many rows a probe on a fan-out's columns returns (`.layout r fanout(p -> c, few)`, decision 0361): one, few
/// (16 or fewer), many, or a measured number (`~4`), which is kept for the estimate but plans by its class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Fan {
    One,
    Few,
    Many,
    About(u32),
}

// A fan-out and a column's values are their own records in a section (`plan::shapes`, decision 0569 §4).
rerail_section::plain_archive!(Fan, Values);

/// The most a few-fan-out probe returns (decision 0361: "few (16 or fewer)").
pub const FEW: u32 = 16;

impl Fan {
    /// The class a plan is keyed by: a measured number is the class it falls in.
    pub fn class(self) -> Fan {
        match self {
            Fan::About(n) if n <= 1 => Fan::One,
            Fan::About(n) if n <= FEW => Fan::Few,
            Fan::About(_) => Fan::Many,
            f => f,
        }
    }
}

/// A relation's size (`.layout r size(…)`, decision 0361): at most `n` rows per value of a column (`per n`, `<= 2 per
/// n`), one per document or per kind, or small (a table of the rules' own).
#[derive(Clone, Debug, PartialEq, Eq, Hash, rerail_section::Archive)]
#[archive(derive(Debug, PartialEq), decode)]
pub enum Size {
    Per {
        most: u32,
        #[archive(flat)]
        col: usize,
    },
    PerDocument,
    PerKind,
    Small,
}

/// How many distinct values a column holds (`values(c, …)`, decision 0361 item 5), where no key, size or fan-out
/// says: the planner's estimate of the column, so its statistics need not be counted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Values {
    /// The documents: one value a document (`node`'s `d`).
    Documents,
    /// The grammar's kinds (`node`'s `kind`).
    Kinds,
    /// Numbered within each document (a document's line or visit index): about a document's rows.
    WithinDocument,
    /// About one a row, without a key's guarantee (a node's start offset).
    Many,
    /// At most about N (a grammar's fields).
    About(u32),
    /// About one every N rows (interned source text).
    OneIn(u32),
}

/// The native operator that makes a relation, where its rules' form does not show it (`.layout r by …`, decision
/// 0361 item 4): an override a profiled run reports once the facts find it alone.
#[derive(Clone, Debug, PartialEq, Eq, Hash, rerail_section::Archive)]
#[archive(derive(Debug, PartialEq), decode)]
pub enum Algo {
    Reach,
    /// A walk along a tree relation.
    Walk(String),
    /// A running aggregate along a column ordered within a group.
    Running {
        #[archive(flat)]
        col: usize,
        #[archive(flat)]
        within: Vector<usize>,
    },
}

/// One fact a relation's rows hold (`.layout`, decision 0361), its columns by place. A fact is about the rows, not
/// a plan, so it holds for every form made from the relation's rules (their deltas', maintenance's and hiding's).
#[derive(Clone, Debug, PartialEq, Eq, Hash, rerail_section::Archive)]
#[archive(derive(Debug, PartialEq), decode)]
pub enum LayoutFact {
    /// The columns are a key: a probe on them returns at most one row.
    Key(#[archive(flat)] Vector<usize>),
    /// A probe on `from` returns `fan` rows of `to`.
    Fanout {
        #[archive(flat)]
        from: Vector<usize>,
        #[archive(flat)]
        to: Vector<usize>,
        fan: Fan,
    },
    Size(Size),
    /// Every column that holds a node holds one of one document: no row crosses documents.
    Local,
    /// `col` is ordered within each group of `within`; dense: numbered 0, 1, 2, … in each.
    Order {
        #[archive(flat)]
        col: usize,
        #[archive(flat)]
        within: Vector<usize>,
        dense: bool,
    },
    /// `from -> to` is a forest: each `to` has at most one `from`, and no cycle.
    Tree {
        #[archive(flat)]
        from: usize,
        #[archive(flat)]
        to: usize,
    },
    /// Column `col` holds `values` distinct values.
    Values {
        #[archive(flat)]
        col: usize,
        values: Values,
    },
    By(Algo),
}

/// A `.layout` line's facts, read: `text` is the line after the relation's name, `col` names a column's place (a
/// declared name, or `$k` for the k-th, as `LayoutFact::show` writes them).
pub fn layout_facts(text: &str, col: &dyn Fn(&str) -> Result<usize, String>) -> Result<Vec<LayoutFact>, String> {
    // Words, with `(`, `)`, `,` and `->` apart.
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let punct = match c {
            '(' | ')' | ',' => Some(c.to_string()),
            '-' if chars.peek() == Some(&'>') => {
                chars.next();
                Some("->".to_owned())
            }
            c if c.is_whitespace() => Some(String::new()),
            _ => None,
        };
        match punct {
            Some(p) => {
                if !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                }
                if !p.is_empty() {
                    words.push(p);
                }
            }
            None => cur.push(c),
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    let mut at = 0;
    let mut facts = Vec::new();
    // The words inside the parentheses that follow the fact's name.
    let args = |at: &mut usize, name: &str| -> Result<Vec<String>, String> {
        if words.get(*at).map(String::as_str) != Some("(") {
            return Err(format!("`{name}` needs its columns in parentheses"));
        }
        let start = *at + 1;
        let end = words[start..].iter().position(|w| w == ")").map(|k| start + k).ok_or_else(|| format!("`{name}(` is not closed"))?;
        *at = end + 1;
        Ok(words[start..end].to_vec())
    };
    let cols = |ws: &[String]| -> Result<Vector<usize>, String> {
        let names: Vec<&String> = ws.iter().filter(|w| *w != ",").collect();
        if names.is_empty() {
            return Err("no column named".to_owned());
        }
        names.into_iter().map(|w| col(w)).collect()
    };
    let one = |ws: &[String]| -> Result<usize, String> {
        match cols(ws)?.as_slice() {
            [c] => Ok(*c),
            _ => Err(format!("one column, not `{}`", ws.join(" "))),
        }
    };
    let number = |w: &str| w.parse::<u32>().map_err(|_| format!("`{w}`: a number"));
    while at < words.len() {
        let name = words[at].clone();
        at += 1;
        let fact = match name.as_str() {
            "key" => LayoutFact::Key(cols(&args(&mut at, "key")?)?),
            "local" => LayoutFact::Local,
            "fanout" => {
                let a = args(&mut at, "fanout")?;
                let arrow = a.iter().position(|w| w == "->").ok_or("`fanout(from -> to, class)`")?;
                let comma = a.iter().rposition(|w| w == ",").filter(|&k| k > arrow).ok_or("`fanout(from -> to, class)`: no class")?;
                let fan = match a[comma + 1..].iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
                    ["one"] => Fan::One,
                    ["few"] => Fan::Few,
                    ["many"] => Fan::Many,
                    [n] if n.starts_with('~') => Fan::About(number(&n[1..])?),
                    other => return Err(format!("`fanout`: a class is one, few, many or ~N, not `{}`", other.join(" "))),
                };
                LayoutFact::Fanout { from: cols(&a[..arrow])?, to: cols(&a[arrow + 1..comma])?, fan }
            }
            "size" => {
                let a = args(&mut at, "size")?;
                let a: Vec<&str> = a.iter().map(String::as_str).collect();
                LayoutFact::Size(match a.as_slice() {
                    ["small"] => Size::Small,
                    ["per", "document"] => Size::PerDocument,
                    ["per", "kind"] => Size::PerKind,
                    ["per", c] => Size::Per { most: 1, col: col(c)? },
                    ["<=", n, "per", c] => Size::Per { most: number(n)?, col: col(c)? },
                    _ => return Err(format!("`size({})`: write `per <column>`, `<= N per <column>`, `per document`, `per kind` or `small`", a.join(" "))),
                })
            }
            "order" => {
                let a = args(&mut at, "order")?;
                let inn = a.iter().position(|w| w == "in").ok_or("`order(column [dense] in group)`")?;
                let (head, within) = (&a[..inn], &a[inn + 1..]);
                let dense = head.last().map(String::as_str) == Some("dense");
                let head = if dense { &head[..head.len() - 1] } else { head };
                LayoutFact::Order { col: one(head)?, within: cols(within)?, dense }
            }
            "tree" => {
                let a = args(&mut at, "tree")?;
                let arrow = a.iter().position(|w| w == "->").ok_or("`tree(parent -> child)`")?;
                LayoutFact::Tree { from: one(&a[..arrow])?, to: one(&a[arrow + 1..])? }
            }
            "values" => {
                let a = args(&mut at, "values")?;
                let comma = a.iter().position(|w| w == ",").ok_or("`values(column, class)`")?;
                let class: Vec<&str> = a[comma + 1..].iter().map(String::as_str).collect();
                let values = match class.as_slice() {
                    ["documents"] => Values::Documents,
                    ["kinds"] => Values::Kinds,
                    ["within", "document"] => Values::WithinDocument,
                    ["many"] => Values::Many,
                    [n] if n.starts_with('~') => Values::About(number(&n[1..])?),
                    ["rows", "/", n] | ["rows/", n] => Values::OneIn(number(n)?),
                    [r] if r.starts_with("rows/") => Values::OneIn(number(&r[5..])?),
                    _ => return Err(format!("`values`: a class is documents, kinds, within document, many, ~N or rows / N, not `{}`", class.join(" "))),
                };
                LayoutFact::Values { col: one(&a[..comma])?, values }
            }
            "by" => {
                let algo = words.get(at).cloned().ok_or("`by` names an algorithm: reach, walk(r) or running(c in g)")?;
                at += 1;
                LayoutFact::By(match algo.as_str() {
                    "reach" => Algo::Reach,
                    "walk" => match args(&mut at, "walk")?.as_slice() {
                        [r] => Algo::Walk(r.clone()),
                        _ => return Err("`by walk(relation)`".to_owned()),
                    },
                    "running" => {
                        let a = args(&mut at, "running")?;
                        let inn = a.iter().position(|w| w == "in").ok_or("`by running(column in group)`")?;
                        Algo::Running { col: one(&a[..inn])?, within: cols(&a[inn + 1..])? }
                    }
                    other => return Err(format!("`by {other}`: an algorithm is reach, walk(r) or running(c in g)")),
                })
            }
            other => return Err(format!("`{other}`: a layout fact is key, fanout, size, local, order, tree, values or by")),
        };
        facts.push(fact);
    }
    if facts.is_empty() {
        return Err("no fact".to_owned());
    }
    Ok(facts)
}

impl LayoutFact {
    /// The fact as `layout_facts` reads it back, its columns named by `name` (`$k` keeps their places).
    pub fn show(&self, name: &dyn Fn(usize) -> String) -> String {
        let list = |cs: &[usize]| cs.iter().map(|&c| name(c)).collect::<Vec<_>>().join(", ");
        match self {
            LayoutFact::Key(cs) => format!("key({})", list(cs)),
            LayoutFact::Fanout { from, to, fan } => {
                let fan = match fan {
                    Fan::One => "one".to_owned(),
                    Fan::Few => "few".to_owned(),
                    Fan::Many => "many".to_owned(),
                    Fan::About(n) => format!("~{n}"),
                };
                format!("fanout({} -> {}, {fan})", list(from), list(to))
            }
            LayoutFact::Size(Size::Small) => "size(small)".to_owned(),
            LayoutFact::Size(Size::PerDocument) => "size(per document)".to_owned(),
            LayoutFact::Size(Size::PerKind) => "size(per kind)".to_owned(),
            LayoutFact::Size(Size::Per { most: 1, col }) => format!("size(per {})", name(*col)),
            LayoutFact::Size(Size::Per { most, col }) => format!("size(<= {most} per {})", name(*col)),
            LayoutFact::Local => "local".to_owned(),
            LayoutFact::Order { col, within, dense } => format!("order({}{} in {})", name(*col), if *dense { " dense" } else { "" }, list(within)),
            LayoutFact::Tree { from, to } => format!("tree({} -> {})", name(*from), name(*to)),
            LayoutFact::Values { col, values } => format!(
                "values({}, {})",
                name(*col),
                match values {
                    Values::Documents => "documents".to_owned(),
                    Values::Kinds => "kinds".to_owned(),
                    Values::WithinDocument => "within document".to_owned(),
                    Values::Many => "many".to_owned(),
                    Values::About(n) => format!("~{n}"),
                    Values::OneIn(n) => format!("rows / {n}"),
                }
            ),
            LayoutFact::By(Algo::Reach) => "by reach".to_owned(),
            LayoutFact::By(Algo::Walk(r)) => format!("by walk({r})"),
            LayoutFact::By(Algo::Running { col, within }) => format!("by running({} in {})", name(*col), list(within)),
        }
    }

    /// The fact with its columns by place (`$k`), as the program's facts and its image keep it.
    pub fn placed(&self) -> String {
        self.show(&|c| format!("${c}"))
    }

    /// A placed fact (`placed`) read back.
    pub fn of_placed(text: &str) -> Result<LayoutFact, String> {
        match layout_facts(text, &placed_col)?.as_slice() {
            [f] => Ok(f.clone()),
            _ => Err(format!("`{text}`: one layout fact")),
        }
    }

    /// The relations the fact names besides its own (`by walk(r)`), which `combine` renames.
    pub fn rename(&mut self, to: &dyn Fn(&str) -> String) {
        if let LayoutFact::By(Algo::Walk(r)) = self {
            *r = to(r);
        }
    }
}

/// A column named by place, `$k`.
pub fn placed_col(w: &str) -> Result<usize, String> {
    w.strip_prefix('$').and_then(|k| k.parse().ok()).ok_or_else(|| format!("`{w}`: a column by place, `$k`"))
}

/// Adds `facts` to a relation's layout: each fact once, in the order first written.
pub fn add_layout(to: &mut Vec<LayoutFact>, facts: impl IntoIterator<Item = LayoutFact>) {
    for f in facts {
        if !to.contains(&f) {
            to.push(f);
        }
    }
}

/// Each rule's readers in the request record (`asked_by`, the `asked_by` field), by its place among the rules whose
/// heads are `heads`, in order: `None` for a rule of no request relation, or of one whose rules no longer match the
/// record (then its own relation, which keeps it wherever its relation is needed).
pub fn readers_of_heads(asked_by: &BTreeMap<String, Vec<BTreeSet<String>>>, heads: &[&str]) -> Vec<Option<BTreeSet<String>>> {
    if asked_by.is_empty() {
        return vec![None; heads.len()];
    }
    let mut at: BTreeMap<&str, usize> = BTreeMap::new();
    let counts: BTreeMap<&str, usize> = asked_by.keys().map(|req| (req.as_str(), heads.iter().filter(|h| **h == req.as_str()).count())).collect();
    heads
        .iter()
        .map(|&req| {
            let readers = asked_by.get(req)?;
            let k = at.entry(req).or_default();
            *k += 1;
            Some(if counts[req] == readers.len() { readers[*k - 1].clone() } else { BTreeSet::from([req.to_owned()]) })
        })
        .collect()
}

/// The request record's step for a copy with head `head` of the rule at `from`, before it is appended to the rules
/// whose heads are `heads`: a copy under another name is no request rule, but reads what its source reads, so where
/// its source is a reader, so is it (a guest's copy of a reader, read in its place, asks what the reader asked); and a
/// copy of a request rule serves its source's readers.
pub fn record_copy(asked_by: &mut BTreeMap<String, Vec<BTreeSet<String>>>, heads: &[&str], from: usize, head: &str) {
    if let Some(&src) = heads.get(from)
        && src != head
    {
        for readers in asked_by.values_mut().flatten().filter(|by| by.contains(src)) {
            readers.insert(head.to_owned());
        }
    }
    if asked_by.contains_key(head) {
        let serves = readers_of_heads(asked_by, heads).get(from).cloned().flatten().filter(|_| heads[from] == head).unwrap_or_else(|| BTreeSet::from([head.to_owned()]));
        let rules = heads.iter().filter(|h| **h == head).count();
        let readers = asked_by.get_mut(head).expect("recorded");
        if rules == readers.len() {
            readers.push(serves);
        }
    }
}

/// The `.decl` qualifiers that choose what a relation keeps (decision 0099), as Soufflé spells them.
pub const QUALIFIERS: &[&str] = &["inline", "no_inline", "magic", "no_magic", "reach", "no_reach", "sorted"];

/// The request record after a pass that writes `rules` (each a head and the places of the old rules it came from), the
/// old rules' readers `old` (`readers_of_heads`, taken before the pass took the rules apart): a request rule serves
/// the readers of the rules it came from, together, and one that came from none (or from a rule the record did not
/// cover) its own relation, which keeps it wherever its relation is needed. A pass that rewrites, splits, merges or
/// drops request rules states their sources here, and the record holds through it.
pub fn carried_record<'r>(asked_by: &BTreeMap<String, Vec<BTreeSet<String>>>, old: &[Option<BTreeSet<String>>], rules: impl IntoIterator<Item = (&'r str, &'r [usize])>) -> BTreeMap<String, Vec<BTreeSet<String>>> {
    let mut asked: BTreeMap<String, Vec<BTreeSet<String>>> = BTreeMap::new();
    for (req, from) in rules {
        if !asked_by.contains_key(req) {
            continue;
        }
        let mut readers: BTreeSet<String> = BTreeSet::new();
        for &i in from {
            match old.get(i) {
                Some(Some(by)) => readers.extend(by.iter().cloned()),
                _ => {
                    readers.insert(req.to_owned());
                }
            }
        }
        if readers.is_empty() {
            readers.insert(req.to_owned());
        }
        asked.entry(req.to_owned()).or_default().push(readers);
    }
    asked
}

/// A `.scope` entry taken from a policy option (`.scope dirs option("applicationRoots")`), as `doc_scopes` holds it
/// until the run's options are known: `rr-option:<form>:<name>`, `form` one of `dirs`, `include`, `exclude`. As a glob
/// it holds no path; `resolve_scopes` adds the option's values beside it.
pub const OPTION_GLOB: &str = "rr-option:";

/// `.global r, …`: relations of a policy that a combined run shares, as it shares a library's, when every policy that
/// has one defines it alike (decision 0091). `.global library r, …` names an included file's relations (`rr::read_file`
/// writes it): shared where alike, else each policy's own.
pub const GLOBAL: &str = "global";

/// `.global`'s first word for an included file's relations.
pub const LIBRARY: &str = "library";

/// A program's `.global` relations: (named by the policy, an included file's).
pub fn globals(settings: &[(String, String)]) -> (BTreeSet<String>, BTreeSet<String>) {
    let (mut named, mut library) = (BTreeSet::new(), BTreeSet::new());
    for (_, text) in settings.iter().filter(|(k, _)| k == GLOBAL) {
        let (to, rest) = match text.trim().strip_prefix(LIBRARY).filter(|r| r.starts_with(char::is_whitespace)) {
            Some(rest) => (&mut library, rest),
            None => (&mut named, text.as_str()),
        };
        to.extend(rest.split(|c: char| c == ',' || c.is_whitespace()).filter(|w| !w.is_empty()).map(str::to_owned));
    }
    (named, library)
}

/// The documents out of a policy's scope (`.scope`'s `dirs`, `include`, `exclude`), as lowering derives them:
/// `rr_scope_not(d)`. In a combined run, the guards on the policy's reads negate it (decision 0091).
pub const SCOPE_NOT: &str = "rr_scope_not";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SpanIn {
    Source,
    Line,
    Comment,
    /// Nodes of this kind.
    Kind(String),
    /// The children of nodes of this kind: a node a rule reaches as such a child (`child(p, n, _), kind(p, "block")`),
    /// whose own kinds it does not name.
    ChildOf(String),
    /// The spans group `.1` of pattern `.0` captured.
    Capture(String, String),
}

/// A relation made natively from relations of the program (the program's `natives` field).
#[derive(Clone, Debug, PartialEq)]
pub enum Native {
    /// `dfs_first(r, hit, first, edge, target)`; with `roots`, `dfs_first_memo` (roots in the order of their
    /// second column, one memo shared).
    DfsFirst { edge: String, target: String, roots: Option<String> },
    /// `match_all(re, s)` over symbols: `(s, g…)`, a row per non-overlapping match of `pattern` in each `s` of
    /// `input`'s one column whose `groups` all take part, each group's text (`logic::native::match_all`).
    MatchAll { input: String, pattern: String, groups: Vec<String>, positions: bool },
    /// A text test (`@op(s, patterns) = 1`) over a column the program derives: the rows of `input` whose `column`
    /// passes it, each distinct string tested by the run's vocabulary with every other test at once (decision 0037),
    /// once a run (`preparation::text::TextRelations`). A row is there as its input row is and its string passes.
    TextTest { input: String, column: usize, op: String, patterns: Vec<String> },
    /// `components(d, c, key, edge, own)`: each document's strongly connected component of `edge(d, e)` (its
    /// smallest document) and the component's Merkle key over `own(d, digest)` (`logic::native::components`).
    Components { edge: String, own: String },
    /// `r(x, y) :- seed(x, y). r(x, z) :- r(x, y), edge(y, z).`: each start's nodes reached over `edge` from its seeds,
    /// held as the reach index (decision 0119, `logic::native::reach`), not as rows.
    Reach { seed: String, edge: String },
    /// `s(x, q) :- r(x, e), map(e, q).` over a `Reach` relation `r`: each start's `map` values over what it reaches,
    /// held the same way.
    ReachThrough { reach: String, map: String },
    /// `r(g…, q…, n)`: for each distinct tuple of `query(g…, q…, kx)` but its bound, `n` distinct `(g…, c…)` tuples of
    /// `counted(g…, c…, ky)` have `ky op kx` (`logic::native::rank`), its first `group` columns the group. A count
    /// below a bound made without the pairs (`rank_form`, decision 0361 step 7).
    Rank { query: String, counted: String, group: usize, op: String },
    Python { kind: String, input: String, targets: Option<String> },
    /// Conservative call candidates over demanded documents; exact semantic operators remain separate.
    PythonCandidates { input: String, targets: String, context: String, member: String },
}

impl Native {
    /// Rewrite producer inputs with the same relation mapping as the rest of its program.
    pub fn rename_reads(&mut self, rename: impl Fn(&str) -> String) {
        match self {
            Native::DfsFirst { edge, target, roots } => {
                *edge = rename(edge);
                *target = rename(target);
                if let Some(roots) = roots { *roots = rename(roots); }
            }
            Native::MatchAll { input, .. } | Native::TextTest { input, .. } => *input = rename(input),
            Native::PythonCandidates {input,targets,..} => { *input=rename(input); *targets=rename(targets); },
            Native::Python {input,targets,..} => { *input=rename(input); if let Some(t)=targets {*t=rename(t);} },
            Native::Components { edge, own } => {
                *edge = rename(edge);
                *own = rename(own);
            }
            Native::Reach { seed, edge } => {
                *seed = rename(seed);
                *edge = rename(edge);
            }
            Native::ReachThrough { reach, map } => {
                *reach = rename(reach);
                *map = rename(map);
            }
            Native::Rank { query, counted, .. } => {
                *query = rename(query);
                *counted = rename(counted);
            }
        }
    }
    /// The relations it reads.
    pub fn reads(&self) -> Vec<&str> {
        match self {
            Native::DfsFirst { edge, target, roots } => [Some(edge.as_str()), Some(target.as_str()), roots.as_deref()].into_iter().flatten().collect(),
            Native::MatchAll { input, .. } | Native::TextTest { input, .. } => vec![input.as_str()],
            Native::PythonCandidates {input,targets,..} => vec![input.as_str(),targets.as_str()],
            Native::Python {input,targets,..} => std::iter::once(input.as_str()).chain(targets.as_deref()).collect(),
            Native::Components { edge, own } => vec![edge.as_str(), own.as_str()],
            Native::Reach { seed, edge } => vec![seed.as_str(), edge.as_str()],
            Native::ReachThrough { reach, map } => vec![reach.as_str(), map.as_str()],
            Native::Rank { query, counted, .. } => vec![query.as_str(), counted.as_str()],
        }
    }
}

/// The prefix of the relations the engine names itself (helpers, axes, places): a rule file may not declare one
/// (`rr::parse_file`), so a rule's relation never collides with them.
pub const RESERVED: &str = "rr_";

/// The prefix of a tree axis's relation: `rr_inside_<set>(n, a)`, a an ancestor of n whose kind is in the set
/// (decision 0036).
pub const AXIS: &str = "rr_inside_";

/// The prefix of the descendant axis's relation: `rr_has_<set>(n, a)`, n of a kind in the set and a any strict
/// ancestor of n (`core.rerail`'s `has(a, n, set)`: the set tests the descendant).
pub const DESCENDANTS: &str = "rr_has_";

/// The prefix of a `nearest(n, a, set)` literal's relation: `rr_nearest_<set>(n, a)` (`plan::lower`).
pub const NEAREST: &str = "rr_nearest_";

/// The layout of a relation the transducer makes from a kind set (an axis's, or `nearest`'s), `(n, a)` rows, wherever
/// it is declared (decision 0361 step 5): what the planner sizes it by with the program. `nearest` holds one
/// ancestor a node; `inside` the few of a set above it; `has` every ancestor of a node of the set, about a tree's
/// depth. An ancestor column holds a share of the rows' values (a few nodes of the set hold many below them). `None`
/// for another relation.
pub fn made_from_a_kind_set(rel: &str) -> Option<Vec<LayoutFact>> {
    let ancestors = LayoutFact::Values { col: 1, values: Values::OneIn(4) };
    if rel.starts_with(NEAREST) {
        Some(vec![LayoutFact::Key(rerail_blocks::vector![0]), LayoutFact::Local, ancestors])
    } else if rel.starts_with(AXIS) {
        Some(vec![LayoutFact::Fanout { from: rerail_blocks::vector![0], to: rerail_blocks::vector![1], fan: Fan::About(2) }, LayoutFact::Local, ancestors])
    } else if rel.starts_with(DESCENDANTS) {
        Some(vec![LayoutFact::Fanout { from: rerail_blocks::vector![0], to: rerail_blocks::vector![1], fan: Fan::About(8) }, LayoutFact::Local, ancestors])
    } else {
        None
    }
}

/// Whether `rel` is a tree axis's relation, which the transducer makes: `(n, a)` rows.
pub fn is_axis(rel: &str) -> bool {
    rel.starts_with(AXIS) || rel.starts_with(DESCENDANTS)
}

/// Whether the transducer makes `rel`: a tree axis's relation or a `nearest` one, `(n, a)` rows.
pub fn is_transduced(rel: &str) -> bool {
    is_axis(rel) || rel.starts_with(NEAREST)
}

/// A tree axis relation's set, and whether it is `has`'s (`DESCENDANTS`: the set tested on its first column, the
/// descendant) rather than `inside`'s (`AXIS`: on its second, the ancestor).
pub fn axis(rel: &str) -> Option<(&str, bool)> {
    rel.strip_prefix(DESCENDANTS).map(|set| (set, true)).or_else(|| rel.strip_prefix(AXIS).map(|set| (set, false)))
}

/// A kind set's one member meaning every kind of the dialect: core.rerail's `within(n, a)` is the tree axis over it
/// (`rr::lower::core`); lowering for a dialect makes it the dialect's kinds (`plan::lower`).
pub const EVERY_KIND: &str = "*";

/// What `magic_adorn` did (T's `magic_report`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub adornments: Vec<String>,
    pub magic_rules: usize,
    pub full: Vec<String>,
    pub extras_kept: usize,
    pub gates: usize,
    pub full_for_negation: Vec<String>,
}

/// A gated negation's gate: `done` copies `src` (the negated relation's demand) once `quiet` is quiet, after the
/// gates in `deps`; `src` is `None` for a unit gate (`done_N(0)`).
#[derive(Clone, Debug, PartialEq)]
pub struct Gate {
    pub src: Option<String>,
    pub rel: String,
    pub level: usize,
    pub quiet: Vec<String>,
    pub deps: Vec<String>,
}

impl Value {
    /// The integer the run holds for it: an integer's, or a branded atom's (decision 0270).
    pub fn integer(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Branded(a) => Some(a.value()),
            Value::Str(_) => None,
        }
    }
}

/// A value by its name's address (`Shallow`).
impl crate::code_rule_dsl::logic::front::name::Shallow for Value {
    fn shallow_hash(&self, h: &mut rustc_hash::FxHasher) {
        use std::hash::Hasher;
        match self {
            Value::Int(i) => {
                h.write_u8(0);
                h.write_i64(*i);
            }
            Value::Str(s) => {
                h.write_u8(1);
                h.write_usize(s.address());
            }
            Value::Branded(b) => {
                h.write_u8(2);
                h.write_u8(b.class as u8);
                h.write_u32(b.atom);
            }
        }
    }
    fn shallow_eq(&self, other: &Value) -> bool {
        self == other
    }
}

/// The parser's punctuation, longest first where one begins another.
pub const PUNCT: [&str; 22] = [":-", "!=", "<=", ">=", "(", ")", ",", ".", ";", ":", "=", "<", ">", "!", "{", "}", "|", "@", "*", "[", "]", "-"];

/// The parser's own spelling of the operator `op`, if it is one (a comparison read back from facts or the store).
pub fn operator(op: &str) -> Option<&'static str> {
    PUNCT.iter().copied().find(|p| *p == op)
}

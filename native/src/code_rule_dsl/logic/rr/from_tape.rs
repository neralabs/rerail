//! A `.rr` file's statements, read off its tape (decision 0062). The tape is the lean parser's image
//! (`rerail_rr_syntax::tape`, decision 0140) and what the content store keeps; lowering reads the statements this
//! module gives, so a rule file's program is a function of its stored tape. The image is read where it is
//! (`tape_format::View`): each node's kind, field and span from its columns, its children by their subtree sizes,
//! with no node decoded ahead and no index of children made.
//!
//! Every statement kind and term kind of decision 0061 maps back to the form `lower` takes (`syntax::Item`,
//! `syntax::Lit`, `syntax::Term`). Texts come from the source by the nodes' spans: a string is unescaped as the lexer
//! does (`super::string_value`), a backtick literal loses its fences (`super::code_body`).

use super::Error;
use super::syntax::{Code, Fix, Head, Item, Lines, Lit, Statement, Suppress, Term};
use super::tape::{ERROR, Field, Kind};
use rerail_rr_syntax::tape_format::{CODE_EXTRA, View};

/// A `.rr` tape read in place: its source and its image's columns.
pub struct Tape<'a> {
    src: &'a str,
    lines: Lines,
    v: View<'a>,
}

/// An operator's text, as the parser holds it.
fn op(text: &str) -> Result<&'static str, Error> {
    const OPS: [&str; 11] = ["=", "!=", "<", "<=", ">", ">=", "+", "-", "*", "/", "%"];
    OPS.iter().find(|o| **o == text).copied().ok_or_else(|| Error::Syntax(format!("tape: unknown operator {text:?}")))
}

fn bad<T>(what: &str) -> Result<T, Error> {
    Err(Error::Syntax(format!("tape: {what}")))
}

impl<'a> Tape<'a> {
    /// `image` (the lean parser's tape of `src`) over `src`.
    pub fn new(src: &'a str, image: &'a [u8]) -> Result<Self, Error> {
        Ok(Self { src, lines: Lines::of(src), v: View::new(image).map_err(|e| Error::Syntax(format!("tape: {e}")))? })
    }

    fn kind(&self, n: u32) -> u16 {
        super::tape::kind_field(self.v.key(n as usize)).0
    }

    fn field(&self, n: u32) -> u16 {
        super::tape::kind_field(self.v.key(n as usize)).1
    }

    fn extra(&self, n: u32) -> bool {
        self.v.key(n as usize) >> 16 & u32::from(CODE_EXTRA) != 0
    }

    fn is(&self, n: u32, kind: Kind) -> bool {
        self.kind(n) == kind as u16
    }

    fn end(&self, n: u32) -> usize {
        self.v.end(n as usize) as usize
    }

    fn text(&self, n: u32) -> &'a str {
        &self.src[self.v.start(n as usize) as usize..self.end(n)]
    }

    fn line(&self, n: u32) -> usize {
        self.lines.line(self.v.start(n as usize) as usize)
    }

    /// `n`'s children in order, extras (comments) left out.
    fn kids(&self, n: u32) -> impl Iterator<Item = u32> + '_ {
        self.v.children(n as usize).map(|c| c as u32).filter(move |&c| !self.extra(c))
    }

    /// Whether a node of the tape is an ERROR: a statement that does not parse.
    fn has_error(&self) -> bool {
        (0..self.v.len() as u32).any(|i| self.kind(i) == ERROR)
    }

    fn all(&self, n: u32, field: Field) -> impl Iterator<Item = u32> + '_ {
        self.kids(n).filter(move |&c| self.field(c) == field as u16)
    }

    fn get(&self, n: u32, field: Field) -> Option<u32> {
        self.all(n, field).next()
    }

    fn one(&self, n: u32, field: Field) -> Result<u32, Error> {
        self.get(n, field).map_or_else(|| bad(&format!("{} has no {field:?}", super::tape::KIND_NAMES.get(usize::from(self.kind(n))).unwrap_or(&"?"))), Ok)
    }

    fn name(&self, n: u32, field: Field) -> Result<String, Error> {
        Ok(self.text(self.one(n, field)?).to_owned())
    }

    /// An anonymous child written `text`.
    fn has_token(&self, n: u32, text: &str) -> bool {
        self.kids(n).any(|c| self.kind(c) >= super::tape::FIRST_ANONYMOUS && self.text(c) == text)
    }

    fn string(&self, n: u32) -> Result<String, Error> {
        super::string_value(self.text(n)).map_or_else(|| bad("a string that does not read"), Ok)
    }

    /// A column named in braces: an identifier, or the braces' text as written (`raw_text`), trimmed as the parser does.
    fn brace_column(&self, n: u32) -> String {
        self.text(n).trim_start_matches('{').trim_end_matches('}').trim().to_owned()
    }

    fn statements(&self) -> Result<Vec<Statement>, Error> {
        self.kids(0).map(|s| Ok((self.statement(s)?, self.line(s)))).collect()
    }

    fn columns(&self, n: u32) -> Result<Vec<(String, String)>, Error> {
        let list = self.one(n, Field::Columns)?;
        self.all(list, Field::Column).map(|c| Ok((self.name(c, Field::Name)?, self.ty(self.one(c, Field::Type)?)?))).collect()
    }

    fn ty(&self, n: u32) -> Result<String, Error> {
        let name = self.name(n, Field::Name)?;
        // `T<a>`: the argument says it (its `<` and `>` are implied); `T<>` stores them.
        if let Some(a) = self.get(n, Field::Argument) {
            return Ok(format!("{name}<{}>", self.text(a)));
        }
        Ok(if self.has_token(n, "<") { format!("{name}<>") } else { name })
    }

    fn statement(&self, s: u32) -> Result<Item, Error> {
        if self.kind(s) == ERROR {
            return bad("a statement that does not parse");
        }
        Ok(match self.kind(s) {
            k if k == Kind::Include as u16 => Item::Include(self.string(self.one(s, Field::Path)?)?),
            k if k == Kind::Declaration as u16 => {
                // The words after the columns, as `rest_of_line` reads them.
                let list = self.one(s, Field::Columns)?;
                let after = self.kids(s).skip_while(|&c| c != list).skip(1);
                let tail = after
                    .map(|c| {
                        Ok(match self.kind(c) {
                            k if k == Kind::String as u16 => self.string(c)?,
                            k if k == Kind::Number as u16 => self.text(c).parse::<i64>().map_or_else(|_| self.text(c).to_owned(), |n| n.to_string()),
                            _ => self.text(c).to_owned(),
                        })
                    })
                    .collect::<Result<_, Error>>()?;
                Item::Decl { name: self.name(s, Field::Name)?, cols: self.columns(s)?, tail }
            }
            k if k == Kind::EditDeclaration as u16 => Item::Edit { name: self.name(s, Field::Name)? },
            k if k == Kind::FunctorDeclaration as u16 => Item::Functor { name: self.name(s, Field::Name)? },
            k if k == Kind::TypeDeclaration as u16 => Item::Type { name: self.name(s, Field::Name)? },
            k if k == Kind::IoDirective as u16 => {
                let output = self.has_token(s, ".output");
                let (mut names, mut params) = (Vec::new(), Vec::new());
                for r in self.all(s, Field::Relation) {
                    let name = self.name(r, Field::Name)?;
                    // `r(…)`: the parameters' text, from the byte after the name's `(` to its `)`. The parentheses
                    // around parameters are implied; `r()` stores them.
                    let open = self.end(self.one(r, Field::Name)?);
                    if let Some(p) = self.get(r, Field::Parameters) {
                        let from = self.src[open..].find('(').map_or(open, |k| open + k + 1);
                        let to = self.src[..self.end(r)].rfind(')').unwrap_or(self.end(p));
                        params.push((name.clone(), self.src[from..to].to_owned()));
                    } else if self.has_token(r, "(") {
                        params.push((name.clone(), String::new()));
                    }
                    names.push(name);
                }
                Item::Io { output, names, params }
            }
            k if k == Kind::LatticeDeclaration as u16 => {
                let ops = self
                    .all(s, Field::Operation)
                    .map(|o| {
                        let value = self.one(o, Field::Value)?;
                        let term = if self.is(value, Kind::FunctorCall) && self.get(value, Field::Arguments).is_none() {
                            Term::Call { name: self.name(value, Field::Function)?, functor: true, args: Vec::new() }
                        } else {
                            self.term(value)?
                        };
                        Ok((self.name(o, Field::Name)?, term))
                    })
                    .collect::<Result<_, Error>>()?;
                Item::Lattice { name: self.name(s, Field::Name)?, base: self.get(s, Field::Base).map_or_else(String::new, |b| self.text(b).to_owned()), ops }
            }
            k if k == Kind::Pragma as u16 => Item::Pragma(self.all(s, Field::Argument).map(|a| self.string(a)).collect::<Result<_, _>>()?),
            k if k == Kind::ViolationDeclaration as u16 => {
                let (mut code, mut code_in, mut tag) = (None, None, None);
                for c in self.all(s, Field::Code) {
                    if self.is(c, Kind::CodeColumn) {
                        code = Some(Code::Column(self.brace_column(self.one(c, Field::Column)?)));
                        code_in = self.get(c, Field::Codes).map(|x| self.string(x)).transpose()?;
                    } else {
                        code = Some(Code::Constant(self.string(c)?));
                    }
                }
                if let Some(t) = self.all(s, Field::Tag).last() {
                    tag = Some(self.brace_column(self.one(t, Field::Column)?));
                }
                let fixes = self
                    .all(s, Field::Fix)
                    .map(|f| {
                        Ok(Fix {
                            name: self.name(f, Field::Relation)?,
                            safe: self.has_token(f, "safe"),
                            title: self.string(self.one(f, Field::Description)?)?,
                        })
                    })
                    .collect::<Result<_, Error>>()?;
                let suppress = self
                    .all(s, Field::Suppress)
                    .map(|l| {
                        Ok(Suppress {
                            tool: self.name(l, Field::Tool)?,
                            rules: self.all(l, Field::Rule).map(|r| self.string(r)).collect::<Result<_, _>>()?,
                            over: self.get(l, Field::Over).map(|o| self.brace_column(o)),
                        })
                    })
                    .collect::<Result<_, Error>>()?;
                Item::Violation {
                    name: self.name(s, Field::Name)?,
                    cols: self.columns(s)?,
                    code,
                    code_in,
                    tag,
                    message: self.string(self.one(s, Field::Message)?)?,
                    fixes,
                    suppress,
                }
            }
            k if k == Kind::PolicyDocument as u16 => {
                let text = self.text(self.one(s, Field::Document)?);
                Item::Policy(serde_json::from_str(text).map_err(|e| Error::Syntax(format!("line {}: bad .policy document: {e}", self.line(s))))?)
            }
            k if k == Kind::TestBlock as u16 => Item::Test {
                name: self.name(s, Field::Name)?,
                exactly: self.get(s, Field::Modifier).is_some(),
                uses: self.all(s, Field::Argument).map(|c| self.text(c).to_owned()).collect(),
                items: self.all(s, Field::Body).map(|c| Ok((self.statement(c)?, self.line(c)))).collect::<Result<_, Error>>()?,
            },
            k if k == Kind::FixturesBlock as u16 => Item::Fixtures {
                name: self.name(s, Field::Name)?,
                items: self.all(s, Field::Body).map(|c| Ok((self.statement(c)?, self.line(c)))).collect::<Result<_, Error>>()?,
            },
            k if k == Kind::Setting as u16 => {
                let name = self.one(s, Field::Name)?;
                let first = self.kids(s).skip_while(|&c| c != name).nth(1);
                let end = self.end(s);
                let text = first.map_or("", |f| &self.src[self.v.start(f as usize) as usize..end]);
                Item::Setting { key: self.text(name).to_owned(), text: text.to_owned() }
            }
            k if k == Kind::Clause as u16 => Item::Clause {
                heads: self.all(s, Field::Head).map(|h| self.head(h)).collect::<Result<_, _>>()?,
                body: self.get(s, Field::Body).map_or_else(|| Ok(Vec::new()), |b| self.body(b))?,
                edits: self.all(s, Field::Edit).map(|e| self.term(e)).collect::<Result<_, _>>()?,
            },
            _ => return bad("an unknown statement"),
        })
    }

    fn head(&self, h: u32) -> Result<Head, Error> {
        Ok(Head {
            modal: self.get(h, Field::Modal).map(|m| self.text(m).to_owned()),
            rel: self.name(h, Field::Relation)?,
            args: self.arguments(self.one(h, Field::Arguments)?)?,
        })
    }

    fn arguments(&self, list: u32) -> Result<Vec<Term>, Error> {
        self.all(list, Field::Argument).map(|a| self.term(a)).collect()
    }

    /// A body or a group's: its alternatives, each a conjunction.
    fn alternatives(&self, b: u32) -> Result<Vec<Vec<Lit>>, Error> {
        if self.is(b, Kind::Disjunction) {
            self.all(b, Field::Alternative).map(|c| self.conjunction(c)).collect()
        } else {
            Ok(vec![self.conjunction(b)?])
        }
    }

    /// A clause's or an aggregate's body: one conjunction, or a group of the alternatives.
    fn body(&self, b: u32) -> Result<Vec<Lit>, Error> {
        let alts = self.alternatives(b)?;
        Ok(if alts.len() == 1 { alts.into_iter().next().expect("one") } else { vec![Lit::Group { alts, neg: false }] })
    }

    fn conjunction(&self, c: u32) -> Result<Vec<Lit>, Error> {
        self.all(c, Field::Literal).map(|l| self.literal(l)).collect()
    }

    fn literal(&self, l: u32) -> Result<Lit, Error> {
        Ok(match self.kind(l) {
            k if k == Kind::Atom as u16 => Lit::Atom { rel: self.name(l, Field::Relation)?, args: self.arguments(self.one(l, Field::Arguments)?)?, neg: false },
            k if k == Kind::Negation as u16 => match self.literal(self.one(l, Field::Operand)?)? {
                Lit::Atom { rel, args, neg } => Lit::Atom { rel, args, neg: !neg },
                Lit::Pattern { subject, pattern, selector, neg } => Lit::Pattern { subject, pattern, selector, neg: !neg },
                Lit::Group { alts, neg } => Lit::Group { alts, neg: !neg },
                Lit::Cmp { .. } => return bad("a negated comparison"),
            },
            k if k == Kind::Comparison as u16 => Lit::Cmp {
                l: self.term(self.one(l, Field::Left)?)?,
                op: op(self.text(self.one(l, Field::Operator)?))?,
                r: self.term(self.one(l, Field::Right)?)?,
            },
            k if k == Kind::PatternMatch as u16 => Lit::Pattern {
                subject: self.term(self.one(l, Field::Subject)?)?,
                pattern: super::code_body(self.text(self.one(l, Field::Pattern)?)).to_owned(),
                selector: self.get(l, Field::Selector).map(|s| self.string(s)).transpose()?,
                neg: false,
            },
            k if k == Kind::Group as u16 => Lit::Group { alts: self.alternatives(self.one(l, Field::Body)?)?, neg: false },
            _ => return bad("an unknown literal"),
        })
    }

    fn term(&self, t: u32) -> Result<Term, Error> {
        let compact = || self.text(t).split_whitespace().collect::<String>();
        Ok(match self.kind(t) {
            k if k == Kind::Variable as u16 => Term::Var(self.text(t).to_owned()),
            k if k == Kind::Wildcard as u16 => Term::Wild,
            k if k == Kind::Number as u16 => Term::Num(compact().parse().map_err(|_| Error::Syntax(format!("tape: bad number {:?}", self.text(t))))?),
            k if k == Kind::Float as u16 => Term::Float(compact()),
            k if k == Kind::String as u16 => Term::Str(self.string(t)?),
            k if k == Kind::CodeLiteral as u16 => Term::Tick(super::code_body(self.text(t)).to_owned()),
            k if k == Kind::JsonObject as u16 => Term::Json(self.text(t).to_owned()),
            k if k == Kind::Call as u16 || k == Kind::FunctorCall as u16 => Term::Call {
                name: self.name(t, Field::Function)?,
                functor: k == Kind::FunctorCall as u16,
                args: self.get(t, Field::Arguments).map_or_else(|| Ok(Vec::new()), |a| self.arguments(a))?,
            },
            k if k == Kind::BinaryExpression as u16 => Term::Bin {
                op: op(self.text(self.one(t, Field::Operator)?))?,
                l: Box::new(self.term(self.one(t, Field::Left)?)?),
                r: Box::new(self.term(self.one(t, Field::Right)?)?),
            },
            k if k == Kind::UnaryExpression as u16 => Term::Bin { op: "-", l: Box::new(Term::Num(0)), r: Box::new(self.term(self.one(t, Field::Operand)?)?) },
            k if k == Kind::ParenthesizedExpression as u16 => self.term(self.one(t, Field::Expression)?)?,
            k if k == Kind::Aggregate as u16 => Term::Agg {
                // `count` is implied: the aggregate with a body alone.
                op: self.get(t, Field::Operator).map_or("count", |o| self.text(o)).to_owned(),
                over: self.get(t, Field::Value).map(|v| self.term(v).map(Box::new)).transpose()?,
                by: self.all(t, Field::Key).map(|k| self.term(k)).collect::<Result<_, _>>()?,
                sep: self.get(t, Field::Separator).map(|s| self.string(s)).transpose()?,
                body: self.body(self.one(t, Field::Body)?)?,
            },
            k if k == Kind::NamedArgument as u16 => Term::Named(self.name(t, Field::Name)?, Box::new(self.term(self.one(t, Field::Value)?)?)),
            _ => return bad("an unknown term"),
        })
    }
}

/// The statements of `src`, read off its tape (`rerail_rr_syntax::tape`). A tape with an ERROR gives the error
/// running the file gives (`super::syntax_error`).
pub fn statements(src: &str) -> Result<Vec<Statement>, Error> {
    let image = rerail_rr_syntax::tape(src);
    let tape = Tape::new(src, &image)?;
    if tape.has_error() {
        let (_, message) = super::syntax_error(src).unwrap_or((0, "a statement that does not parse".to_owned()));
        return Err(Error::Syntax(message));
    }
    if tape.v.is_empty() {
        return Ok(Vec::new());
    }
    tape.statements()
}

//! PHP variables (tree-sitter-php 0.24.2, PHP-only grammar), as PHP 8.5 scopes them: a
//! function, method or closure has its own variables and sees no other (a closure only those its
//! `use` clause copies in, an arrow function every variable of where it is written, by value);
//! blocks are no scopes; the file's top level is the global scope. A class body holds no
//! variables but is a scope, so a method's scope knows it is one.
//!
//! Each read gets what reaches it: the bindings (assignments, parameters, `foreach` targets,
//! `catch` variables, `global` and `static` declarations, by-reference arguments of PHP's
//! builtins) that some path from the scope's start reaches it with, and whether the variable is
//! defined there on every path, on none, or on some. The paths are PHP's structured control flow:
//! branches, loops (to a fixpoint), `break`/`continue`, `return`, `throw`, `exit`, `switch`
//! fallthrough, `match`, `try`/`catch`/`finally`, short-circuiting `&&`, `||`, `??`, `?:`, and
//! `isset`/`empty` conditions. Types are not read: a path PHPStan proves infeasible from types
//! (an always-iterable `foreach`, a condition always true) is still a path here, so a variable
//! defined on some paths is `Maybe`, which PHPStan may report either way. What PHP's dynamic
//! symbol table can change (`extract`, `eval`, `include`, `$$name = ...`, `goto`, a call that may
//! take a variable by reference) makes the answer `Unknown` rather than a guess.
//!
//! The oracles are PHPStan 2.2.16's `variable.undefined` and Psalm 6.19.1's unused-variable
//! reports (`fixtures/php/scope/php`).

use super::php_names::{self, PhpNames, Resolved};
use super::{Binding, BuildError, Builder, Form, NodeRef, ScopeKind, Site, VALUE};
use crate::code_rule_dsl::value::{Context as Decoding, php as php_value};
use ast_grep_core::{Doc, Node};
use foldhash::{HashMap, HashMapExt};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

mod blade;
mod by_reference;

/// Whether a variable is defined where it is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Defined {
    /// On every path.
    Yes,
    /// On no path.
    No,
    /// On some paths.
    Maybe,
    /// PHP's dynamic symbol table, a reference or an unknown callee may decide it.
    Unknown,
}

/// How a read may meet an undefined variable without an ordinary warning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Quiet {
    /// An ordinary read: an undefined variable warns, and PHPStan reports it.
    No,
    /// `isset(...)`, `empty(...)`, the left side of `??` and `??=`, a subscript or reference that
    /// creates the variable (`$a[] = 1`, `&$a`): silent, and PHPStan reports nothing.
    Silent,
    /// An argument a callee may take by reference: whether PHP warns, and whether PHPStan reports
    /// an undefined one, depends on the callee (PHPStan's stubs tell an out-parameter such as
    /// `preg_match`'s `$matches` from an in-out one such as `array_push`'s).
    ByReference,
}

/// What reaches one read of a variable.
#[derive(Clone, Debug)]
pub(crate) struct Read {
    /// The variable, `$` included.
    pub(crate) name: Box<str>,
    /// The node read: the variable, or the string of a `compact` argument.
    pub(crate) node: NodeRef,
    /// The scope the read is made in.
    pub(crate) scope: u32,
    /// The bindings some path reaches the read with: their scope and index among that scope's
    /// bindings of the name.
    pub(crate) bindings: Box<[(u32, usize)]>,
    pub(crate) defined: Defined,
    /// The variable may hold a value none of `bindings` gave it: a reference to it was taken, it
    /// is `global` or `static`, or a callee may have taken it by reference.
    pub(crate) aliased: bool,
    pub(crate) quiet: Quiet,
    /// A superglobal (`$_GET`, `$GLOBALS`, ...), which every scope sees.
    pub(crate) superglobal: bool,
    /// A read that changes the value rather than reads it for another use: the binding the
    /// change makes carries its use back to the bindings this read reaches.
    pub(crate) modify: Option<Modify>,
}

/// How a read changes its variable's value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Modify {
    /// The base of a subscript assignment (`$a[] = 1`, `$a[k] .= v;`).
    Element,
    /// A compound assignment or increment whose value is discarded (`$a .= v;`, `$i++;`): as
    /// Psalm reads it, a value only such an update of the variable reads is used only if the
    /// updated value is.
    Update,
}

/// What reaches a binding where it is made: the bindings of its variable some path reaches the
/// binding's site with (the ones it overwrites), whether some path reaches it with the variable
/// undefined, and whether PHP's dynamic symbol table may have bound the variable on some path, so
/// that `bindings` may be incomplete.
#[derive(Clone, Debug, Default)]
pub(crate) struct Reach {
    pub(crate) bindings: Box<[(u32, usize)]>,
    /// Read by the oracle tests, which record where a binding meets its variable undefined.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) undefined: bool,
    pub(crate) unknown: bool,
}

/// Whether a binding writes, through `global` or a closure's by-reference `use`, a variable of
/// the global scope or of the scope the closure is written in: bits of the paths that reach it.
const OUTER: u8 = 1;
/// Some path reaches the binding with its variable the scope's own.
const LOCAL: u8 = 2;

/// How a binding's form decides whether it writes an outer variable.
#[derive(Clone, Copy, PartialEq)]
enum Outer {
    /// It writes the variable it finds (an assignment writes through a reference).
    Keep,
    /// It makes the variable an outer one (`global $x`, `use (&$x)`, `$x = &$GLOBALS['x']`).
    Shared,
    /// It makes the variable the scope's own (a parameter, `static`, `=&` of another variable).
    Local,
    /// It may do either (`$x = &$GLOBALS[$name]`).
    Unknown,
}

/// What the flow found in a file: each read, and the scopes whose variables a construct reads or
/// writes by a name the source does not spell.
#[derive(Default)]
pub(crate) struct Flow {
    /// By the read's start byte.
    reads: HashMap<usize, Read>,
    /// Scopes where every variable may be read (`get_defined_vars()`, `compact` of a name that is
    /// not a literal, `$$name`, `eval`, `include`).
    pub(crate) reads_all: BTreeSet<u32>,
    /// Bindings of a variable that is a reference (a by-reference parameter or `use`, `global`,
    /// `static`, `&$x`, a by-reference `foreach` value): what they assign may be read elsewhere.
    pub(crate) aliased: BTreeSet<BindingKey>,
    /// Bindings some path carries to a `goto`: its label, where the flow knows nothing, may read
    /// them.
    pub(crate) jumped: BTreeSet<BindingKey>,
    /// Bindings some path carries to an `unset` of their variable, which ends their value's life
    /// there: `$x = new D(); unset($x); echo 'after';` runs `D::__destruct` before the echo, so
    /// the unset may be the binding's use (Psalm 6.19.1 counts it as one).
    pub(crate) unset: BTreeSet<BindingKey>,
    /// Each binding a subscript assignment or a self-update makes, with the bindings of the value
    /// it changed: a read of the new one reads what they assigned too.
    pub(crate) modifies: BTreeMap<BindingKey, Vec<BindingKey>>,
    /// What reaches each binding where it is made, by the start of its name node.
    before: HashMap<u32, Reach>,
    /// The bindings (by the start of their name node) some path carries to their scope's normal
    /// end: a `return`, or the end of its body (of the file, for the global scope).
    ends: BTreeSet<u32>,
    /// Scopes whose end a dynamic construct (`extract`, `include`, `goto`, ...) may reach with
    /// other bindings: their bindings' liveness is not known.
    open_ends: BTreeSet<u32>,
    /// `OUTER` and `LOCAL` bits of each binding, by the start of its name node.
    outer: HashMap<u32, u8>,
    /// Variable nodes that name a variable without reading or binding it (a closure's
    /// by-reference `use` in the scope it is written in, `unset($x)`): their scope and name.
    mentions: Vec<(u32, Box<str>, NodeRef)>,
    /// The scopes of arrow functions, which share by value the variables they do not bind.
    arrows: BTreeSet<u32>,
    /// The file's class, function and constant names, resolved: the flow reads them to tell PHP's
    /// builtins apart, and the file's other name checks may share them. The table's cost leaves
    /// theirs (`PhpNames::cost`) out.
    pub(crate) names: Arc<PhpNames>,
}

/// A binding by its scope, variable and index among the scope's bindings of the variable.
pub(crate) type BindingKey = (u32, Box<str>, usize);

impl Flow {
    pub(crate) fn read(&self, start: usize) -> Option<&Read> {
        self.reads.get(&start)
    }

    pub(crate) fn reads(&self) -> impl Iterator<Item = &Read> {
        self.reads.values()
    }

    /// What reaches the binding whose name node starts at `start`.
    pub(crate) fn reach(&self, start: u32) -> Option<&Reach> {
        self.before.get(&start)
    }

    /// Whether the binding of `scope` whose name node starts at `start` may be its variable's
    /// value where the scope ends normally; `None` where that is not known.
    pub(crate) fn latest(&self, scope: u32, start: u32) -> Option<bool> {
        (!self.open_ends.contains(&scope)).then(|| self.ends.contains(&start))
    }

    /// Whether the binding whose name node starts at `start` writes an outer variable: on every
    /// path that reaches it, on none, or (`None`) on some.
    pub(crate) fn outer(&self, start: u32) -> Option<bool> {
        match self.outer.get(&start).copied().unwrap_or(LOCAL) {
            OUTER => Some(true),
            bits if bits & OUTER != 0 => None,
            _ => Some(false),
        }
    }

    pub(crate) fn mentions(&self) -> impl Iterator<Item = &(u32, Box<str>, NodeRef)> {
        self.mentions.iter()
    }

    /// The scope whose variable `name` is where `scope` names it: an arrow function that binds
    /// no `name` shares its enclosing scope's.
    pub(crate) fn variable_scope(&self, table: &super::ScopeTable, scope: u32, name: &str) -> u32 {
        let mut scope = scope;
        while self.arrows.contains(&scope)
            && !table.scopes[scope as usize].bindings.contains_key(name)
            && let Some(parent) = table.scopes[scope as usize].parent
        {
            scope = parent;
        }
        scope
    }
}

/// PHP's superglobals, which every scope sees without `global`.
const SUPERGLOBALS: &[&str] = &[
    "$GLOBALS",
    "$_SERVER",
    "$_GET",
    "$_POST",
    "$_FILES",
    "$_COOKIE",
    "$_SESSION",
    "$_REQUEST",
    "$_ENV",
];

/// Iterations of a loop body before the flow gives the loop's variables up as unknown.
const MAX_LOOP_ITERATIONS: usize = 32;

/// One variable's state on the paths that reach a point.
#[derive(Clone, Debug, Default, PartialEq)]
struct Var {
    /// Bindings (ids into `Visitor::bindings`) some path reaches with.
    defs: BTreeSet<u32>,
    /// Some path reaches without a binding.
    undefined: bool,
    /// A dynamic construct may have defined it on some path.
    unknown: bool,
    aliased: bool,
    /// `OUTER` where some path reaches with it an outer variable, `LOCAL` where some path reaches
    /// with it the scope's own.
    outer: u8,
}

impl Var {
    fn undefined() -> Self {
        Self {
            undefined: true,
            outer: LOCAL,
            ..Self::default()
        }
    }

    fn join(&mut self, other: &Var) {
        self.defs.extend(other.defs.iter().copied());
        self.undefined |= other.undefined;
        self.unknown |= other.unknown;
        self.aliased |= other.aliased;
        self.outer |= other.outer;
    }
}

/// The variables on the paths that reach a point; a variable it lacks is undefined on all of them.
/// `dynamic`: `extract`, `eval`, `include` or `$$name = ...` ran on some path, so any variable
/// may be defined.
#[derive(Clone, Debug, Default, PartialEq)]
struct State {
    vars: BTreeMap<Box<str>, Var>,
    dynamic: bool,
}

/// `None` where no path reaches.
type Flowing = Option<State>;

fn join(left: Flowing, right: Flowing) -> Flowing {
    match (left, right) {
        (None, other) | (other, None) => other,
        (Some(mut left), Some(right)) => {
            for (name, var) in &mut left.vars {
                match right.vars.get(name) {
                    Some(other) => var.join(other),
                    None => {
                        var.undefined = true;
                        var.outer |= LOCAL;
                    }
                }
            }
            for (name, var) in right.vars {
                left.vars.entry(name).or_insert_with(|| {
                    let mut var = var;
                    var.undefined = true;
                    var.outer |= LOCAL;
                    var
                });
            }
            left.dynamic |= right.dynamic;
            Some(left)
        }
    }
}

fn join_into(target: &mut Flowing, state: Flowing) {
    *target = join(target.take(), state);
}

/// What a loop or `switch` collects from `break` and `continue`.
#[derive(Default)]
struct Frame {
    breaks: Flowing,
    continues: Flowing,
}

/// What `break`, `continue` and `return` carry: each frame's breaks and continues, by frame, and
/// the returns.
type Exits = (Vec<(Flowing, Flowing)>, Flowing);

/// How a function-like scope starts.
#[derive(Clone, Copy, PartialEq)]
enum Context {
    /// The file's top level: included files and the caller of `include` may define anything.
    Top,
    /// A function, method or closure, and whether `$this` is defined in it (`Some(false)`: it
    /// is not, as in a static method; `None`: it is not known, as in a closure outside a class
    /// that may be bound later).
    Function { this: Option<bool> },
}

type Visit<T = Flowing> = Result<T, BuildError>;

pub(crate) fn visit_program<D: Doc>(builder: &mut Builder, root: &Node<'_, D>) -> Visit<()> {
    visit(builder, root, false)
}

/// A Blade template's embedded program (`blade::view`), whose markers stand for the variables
/// Blade's directives define.
pub(crate) fn visit_blade_program<D: Doc>(builder: &mut Builder, root: &Node<'_, D>) -> Visit<()> {
    visit(builder, root, true)
}

fn visit<D: Doc>(builder: &mut Builder, root: &Node<'_, D>, blade: bool) -> Visit<()> {
    // Which callee is PHP's builtin is name resolution's answer: the names the caller built, or
    // ones built here. Either way the table's cost leaves theirs out (`PhpNames::cost` is charged
    // where they are built), though the build here spends the same cap.
    let names = match builder.php_names.take() {
        Some(names) => names,
        None => {
            let names = PhpNames::build(
                root.clone(),
                builder.cap.saturating_sub(builder.table.cost),
                builder.deadline,
            )?;
            builder.cap = builder.cap.saturating_sub(names.cost);
            Arc::new(names)
        }
    };
    let mut visitor = Visitor {
        builder,
        bindings: Vec::new(),
        sites: HashMap::new(),
        scopes: HashMap::new(),
        flow: Flow::default(),
        before: HashMap::new(),
        frames: Vec::new(),
        throws: Vec::new(),
        returns: None,
        context: Context::Top,
        class: false,
        names: Arc::clone(&names),
        scope: 0,
        blade,
    };
    let state = visitor.statements(named(root), Some(State::default()))?;
    let returns = visitor.returns.take();
    visitor.end(0, join(state, returns));
    let mut flow = std::mem::take(&mut visitor.flow);
    flow.names = names;
    // What reaches each binding, by the name nodes of the bindings.
    flow.before = std::mem::take(&mut visitor.before)
        .into_iter()
        .map(|(id, (defs, undefined, unknown))| {
            let reach = Reach {
                bindings: defs
                    .iter()
                    .map(|def| {
                        let (scope, _, index, _) = &visitor.bindings[*def as usize];
                        (*scope, *index)
                    })
                    .collect(),
                undefined,
                unknown,
            };
            (visitor.bindings[id as usize].3, reach)
        })
        .collect();
    visitor.builder.table.php = Some(Box::new(flow));
    Ok(())
}

fn named<'r, D: Doc>(node: &Node<'r, D>) -> impl Iterator<Item = Node<'r, D>> {
    let children: Vec<_> = node
        .children()
        .filter(|child| child.is_named() && child.kind() != "comment")
        .collect();
    children.into_iter()
}

/// A variable node's name, `$` included; `None` for `$$x` and `${...}`.
fn variable<D: Doc>(node: &Node<'_, D>) -> Option<String> {
    (node.kind() == "variable_name").then(|| node.text().to_string())
}

/// The starts of the reads of `name` whose value `value` only computes with (arithmetic, bitwise
/// and string operators, parentheses): in `$a = $a + 1`, what the right side reads of `$a` only
/// makes the new `$a`, as Psalm 6.19.1's variable-use graph links it (`BinaryOpAnalyzer::addDataFlow`).
fn self_reads<D: Doc>(value: &Node<'_, D>, name: &str) -> Vec<usize> {
    const COMPUTING: &[&str] = &[
        "+", "-", "*", "/", "%", "**", ".", "&", "|", "^", "<<", ">>",
    ];
    match value.kind().as_ref() {
        "variable_name" if value.text() == name => vec![value.range().start],
        "parenthesized_expression" => named(value)
            .flat_map(|inner| self_reads(&inner, name))
            .collect(),
        "binary_expression"
            if value
                .field("operator")
                .is_some_and(|operator| COMPUTING.contains(&operator.text().as_ref())) =>
        {
            ["left", "right"]
                .into_iter()
                .filter_map(|field| value.field(field))
                .flat_map(|operand| self_reads(&operand, name))
                .collect()
        }
        _ => Vec::new(),
    }
}

struct Visitor<'b, 'd> {
    builder: &'b mut Builder<'d>,
    /// Every binding the flow made: its scope, name, index among the scope's bindings of it, and
    /// the start of its name node.
    bindings: Vec<(u32, Box<str>, usize, u32)>,
    /// The binding made at a name node, so a loop body visited again binds nothing twice.
    sites: HashMap<(usize, usize), u32>,
    /// The scope opened at a function-like or class node, for the same reason.
    scopes: HashMap<(usize, usize), u32>,
    flow: Flow,
    /// What reaches each binding (by id) on any visit: the bindings, whether the variable may be
    /// undefined, whether a dynamic construct may have bound it.
    before: HashMap<u32, (BTreeSet<u32>, bool, bool)>,
    frames: Vec<Frame>,
    /// For each enclosing `try` body: the states a throw may leave it with.
    throws: Vec<Flowing>,
    /// The states the current function's `return` statements leave it with.
    returns: Flowing,
    context: Context,
    /// Inside a class body (a method or a closure in one).
    class: bool,
    /// The file's class, function and constant names, resolved.
    names: Arc<PhpNames>,
    /// The scope variables are bound in.
    scope: u32,
    /// Reading a Blade template's program: its empty statements over directives are markers.
    blade: bool,
}

impl Visitor<'_, '_> {
    // -----------------------------------------------------------------------------------------
    // Bindings and reads
    // -----------------------------------------------------------------------------------------

    /// Binds the variable `name_node` names, made by `site` in `form`, and makes it the variable's
    /// only binding on this path. `outer` says whether the binding writes an outer variable.
    #[allow(clippy::too_many_arguments)]
    fn bind<D: Doc>(
        &mut self,
        name_node: &Node<'_, D>,
        form: Form,
        site: Option<&Node<'_, D>>,
        value: Option<&Node<'_, D>>,
        state: &mut Flowing,
        aliased: bool,
        outer: Outer,
    ) {
        let Some(name) = variable(name_node) else {
            // `$$name = ...` may bind any variable.
            if let Some(state) = state {
                state.dynamic = true;
            }
            return;
        };
        if SUPERGLOBALS.contains(&name.as_str()) || name == "$this" {
            return;
        }
        let range = name_node.range();
        let id = match self.sites.get(&(range.start, range.end)) {
            Some(id) => *id,
            None => {
                let mut binding = Binding::new(
                    range.start,
                    VALUE,
                    Site {
                        form,
                        node: site.map(NodeRef::of),
                        value: value.map(NodeRef::of),
                        annotation: None,
                    },
                );
                binding.name = Some(NodeRef::of(name_node));
                let scope = self.scope;
                let index = self.builder.table.scopes[scope as usize]
                    .bindings
                    .get(name.as_str())
                    .map_or(0, Vec::len);
                self.builder.bind(scope, &name, binding);
                let id = self.bindings.len() as u32;
                self.bindings
                    .push((scope, name.clone().into(), index, range.start as u32));
                self.sites.insert((range.start, range.end), id);
                id
            }
        };
        self.assign(id, name, range.start, state, aliased, outer);
    }

    /// Makes binding `id` (made at `start`) the variable `name`'s only binding on this path,
    /// recording what it overwrites and whether it writes an outer variable.
    fn assign(
        &mut self,
        id: u32,
        name: String,
        start: usize,
        state: &mut Flowing,
        aliased: bool,
        outer: Outer,
    ) {
        let Some(state) = state else {
            return;
        };
        let found = state.vars.get(name.as_str());
        // What this binding overwrites.
        let reach = self.before.entry(id).or_default();
        match found {
            Some(var) => {
                reach.0.extend(var.defs.iter().copied());
                reach.1 |= var.undefined;
                reach.2 |= var.unknown || (var.undefined && state.dynamic);
            }
            None => {
                reach.1 = true;
                reach.2 |= state.dynamic;
            }
        }
        // The global scope's variables are its own; elsewhere an assignment writes the
        // variable it finds, which `global`, a closure's by-reference `use` or a dynamic
        // construct may have made an outer one.
        let bits = match (self.context, outer) {
            (Context::Top, _) | (_, Outer::Local) => LOCAL,
            (_, Outer::Shared) => OUTER,
            (_, Outer::Unknown) => OUTER | LOCAL,
            (_, Outer::Keep) if state.dynamic => OUTER | LOCAL,
            (_, Outer::Keep) => found.map_or(LOCAL, |var| var.outer),
        };
        *self.flow.outer.entry(start as u32).or_default() |= bits;
        // A variable that is a reference stays one when assigned, also one a by-reference `use`
        // or `=&` made from nothing: `$f = function () use (&$x) { echo $x; }; $x = 1; $f();`
        // prints `1`.
        let aliased = aliased || found.is_some_and(|var| var.aliased);
        if aliased {
            let (scope, name, index, _) = &self.bindings[id as usize];
            self.flow.aliased.insert((*scope, name.clone(), *index));
        }
        state.vars.insert(
            name.into(),
            Var {
                defs: BTreeSet::from([id]),
                undefined: false,
                unknown: false,
                aliased,
                outer: bits,
            },
        );
    }

    /// Records which bindings of `scope` reach its normal end (`state`, `None` where none does).
    fn end(&mut self, scope: u32, state: Flowing) {
        let Some(state) = state else {
            return;
        };
        if state.dynamic {
            self.flow.open_ends.insert(scope);
        }
        for var in state.vars.values() {
            for def in &var.defs {
                let (owner, _, _, start) = &self.bindings[*def as usize];
                if *owner == scope {
                    self.flow.ends.insert(*start);
                }
            }
        }
    }

    /// A variable node that names a variable of the current scope without reading or binding it.
    fn mention<D: Doc>(&mut self, node: &Node<'_, D>) {
        if let Some(name) = variable(node) {
            self.flow
                .mentions
                .push((self.builder.current, name.into(), NodeRef::of(node)));
        }
    }

    /// Records the bindings that reach `var` where it becomes a reference (`&$x`, a closure's
    /// by-reference `use`) as aliased: what they assigned may be read through the other name.
    fn alias_reaching(flow: &mut Flow, bindings: &[(u32, Box<str>, usize, u32)], var: &Var) {
        for def in &var.defs {
            let (scope, name, index, _) = &bindings[*def as usize];
            flow.aliased.insert((*scope, name.clone(), *index));
        }
    }

    /// Records a read of the variable `node` names under `state`.
    fn read<D: Doc>(&mut self, node: &Node<'_, D>, state: &Flowing, quiet: Quiet) {
        let Some(name) = variable(node) else {
            return;
        };
        let range = node.range();
        self.builder.reference(range.clone(), VALUE);
        let superglobal = SUPERGLOBALS.contains(&name.as_str());
        let scope = self.builder.current;
        let at = NodeRef::of(node);
        let read = match state {
            _ if superglobal => Read {
                name: name.as_str().into(),
                node: at,
                scope,
                bindings: Box::new([]),
                defined: Defined::Yes,
                aliased: true,
                quiet,
                superglobal,
                modify: None,
            },
            None => Read {
                name: name.as_str().into(),
                node: at,
                scope,
                bindings: Box::new([]),
                defined: Defined::Unknown,
                aliased: false,
                quiet,
                superglobal,
                modify: None,
            },
            Some(state) => {
                let var = state.vars.get(name.as_str()).cloned();
                let this = name == "$this";
                let var = var.unwrap_or_else(Var::undefined);
                let bindings: Box<[(u32, usize)]> = var
                    .defs
                    .iter()
                    .map(|id| {
                        let (scope, _, index, _) = &self.bindings[*id as usize];
                        (*scope, *index)
                    })
                    .collect();
                let defined = self.definedness(&name, &var, state, this);
                Read {
                    name: name.as_str().into(),
                    node: at,
                    scope,
                    bindings,
                    defined,
                    aliased: var.aliased,
                    quiet,
                    superglobal,
                    modify: None,
                }
            }
        };
        self.flow.reads.insert(range.start, read);
    }

    /// Whether `var`, the variable `name` (`$this` where `this`), is defined at a read in `state`.
    /// A variable no path of the file's top level defines may come from an including file; in a
    /// Blade template's top level one some path leaves unset may still come from the view's data,
    /// so either is unknown there, never undefined or maybe. PHP's HTTP stream wrapper defines
    /// `$http_response_header` in the scope of a builtin call that opens a URL
    /// (`file_get_contents`, `fopen`, `file`, `copy`, ...), which this flow does not model.
    fn definedness(&self, name: &str, var: &Var, state: &State, this: bool) -> Defined {
        if var.unknown || (var.undefined && state.dynamic) {
            Defined::Unknown
        } else if this && var.defs.is_empty() {
            match self.context {
                Context::Function { this: Some(true) } => Defined::Yes,
                Context::Function { this: Some(false) } => Defined::No,
                _ => Defined::Unknown,
            }
        } else if !var.undefined {
            Defined::Yes
        } else {
            match self.context {
                _ if var.defs.is_empty() && name == "$http_response_header" => Defined::Unknown,
                Context::Top if var.defs.is_empty() || self.blade => Defined::Unknown,
                Context::Function { .. } if var.defs.is_empty() => Defined::No,
                _ => Defined::Maybe,
            }
        }
    }

    /// Marks every variable of the state possibly changed, as a construct that may define or
    /// overwrite any of them does (`extract`, `$$name = ...`).
    fn dynamic(state: &mut Flowing) {
        if let Some(state) = state {
            state.dynamic = true;
            for var in state.vars.values_mut() {
                var.aliased = true;
            }
        }
    }

    /// As `dynamic`, for code that may also unset any of them (`eval`, `include`, what a `goto`
    /// may come from): no variable's definedness is known after it.
    fn arbitrary(state: &mut Flowing) {
        Self::dynamic(state);
        if let Some(state) = state {
            for var in state.vars.values_mut() {
                var.unknown = true;
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Statements
    // -----------------------------------------------------------------------------------------

    fn statements<'r, D: Doc + 'r>(
        &mut self,
        nodes: impl Iterator<Item = Node<'r, D>>,
        mut state: Flowing,
    ) -> Visit {
        for node in nodes {
            state = self.statement(&node, state)?;
        }
        Ok(state)
    }

    fn may_throw(&mut self, state: &Flowing) {
        if let Some(throws) = self.throws.last_mut() {
            join_into(throws, state.clone());
        }
    }

    fn statement<D: Doc>(&mut self, node: &Node<'_, D>, state: Flowing) -> Visit {
        self.builder.enter()?;
        self.may_throw(&state);
        let state = self.statement_inner(node, state)?;
        self.may_throw(&state);
        self.builder.leave();
        Ok(state)
    }

    fn statement_inner<D: Doc>(&mut self, node: &Node<'_, D>, mut state: Flowing) -> Visit {
        match node.kind().as_ref() {
            "compound_statement" | "colon_block" | "declare_statement" => {
                self.statements(named(node), state)
            }
            "namespace_definition" => match node.field("body") {
                Some(body) => self.statements(named(&body), state),
                None => Ok(state),
            },
            "expression_statement" | "echo_statement" | "const_declaration" => {
                for child in named(node) {
                    state = self.expression(&child, state)?;
                }
                Ok(state)
            }
            "return_statement" => {
                for child in named(node) {
                    state = self.expression(&child, state)?;
                }
                // A `finally` around it runs first (`try_statement`).
                join_into(&mut self.returns, state);
                Ok(None)
            }
            "exit_statement" => {
                for child in named(node) {
                    state = self.expression(&child, state)?;
                }
                Ok(None)
            }
            "break_statement" | "continue_statement" => {
                let levels = named(node)
                    .next()
                    .and_then(|level| level.text().parse::<usize>().ok())
                    .unwrap_or(1)
                    .max(1);
                if let Some(index) = self.frames.len().checked_sub(levels) {
                    let frame = &mut self.frames[index];
                    // `continue` in a `switch` acts as `break`; a frame's continues are its loop's.
                    if node.kind() == "break_statement" {
                        join_into(&mut frame.breaks, state);
                    } else {
                        join_into(&mut frame.continues, state);
                    }
                }
                Ok(None)
            }
            "if_statement" => self.if_statement(node, state),
            "while_statement" => self.while_statement(node, state),
            "do_statement" => self.do_statement(node, state),
            "for_statement" => self.for_statement(node, state),
            "foreach_statement" => self.foreach_statement(node, state),
            "switch_statement" => self.switch_statement(node, state),
            "try_statement" => self.try_statement(node, state),
            "unset_statement" => {
                for child in named(node) {
                    match variable(&child) {
                        Some(name) => {
                            self.builder.reference(child.range(), VALUE);
                            self.mention(&child);
                            if let Some(state) = &mut state
                                && let Some(var) = state.vars.remove(name.as_str())
                            {
                                for def in &var.defs {
                                    let (scope, name, index, _) = &self.bindings[*def as usize];
                                    self.flow.unset.insert((*scope, name.clone(), *index));
                                }
                            }
                        }
                        None => state = self.unset_base(&child, state)?,
                    }
                }
                Ok(state)
            }
            "global_declaration" => {
                for child in named(node) {
                    self.bind(
                        &child,
                        Form::Global,
                        Some(node),
                        None,
                        &mut state,
                        true,
                        Outer::Shared,
                    );
                }
                Ok(state)
            }
            "function_static_declaration" => {
                for declaration in named(node) {
                    if let Some(value) = declaration.field("value") {
                        state = self.expression(&value, state)?;
                    }
                    if let Some(name) = declaration.field("name") {
                        let value = declaration.field("value");
                        self.bind(
                            &name,
                            Form::Static,
                            Some(&declaration),
                            value.as_ref(),
                            &mut state,
                            true,
                            Outer::Local,
                        );
                    }
                }
                Ok(state)
            }
            // A label may be reached from any `goto`, after anything it ran: nothing is known
            // after it, so what reaches the `goto` may be read there.
            "goto_statement" => {
                if let Some(state) = &state {
                    for var in state.vars.values() {
                        for def in &var.defs {
                            let (scope, name, index, _) = &self.bindings[*def as usize];
                            self.flow.jumped.insert((*scope, name.clone(), *index));
                        }
                    }
                }
                Ok(None)
            }
            "named_label_statement" => {
                let mut state = state.or_else(|| Some(State::default()));
                Self::arbitrary(&mut state);
                Ok(state)
            }
            "function_definition" => {
                self.function(node, Context::Function { this: Some(false) })?;
                Ok(state)
            }
            "class_declaration"
            | "interface_declaration"
            | "trait_declaration"
            | "enum_declaration" => {
                self.class(node)?;
                Ok(state)
            }
            "empty_statement" if self.blade => Ok(self.blade_marker(node, state)),
            "empty_statement" | "namespace_use_declaration" | "php_tag" | "text_interpolation" => {
                Ok(state)
            }
            _ => self.expression(node, state),
        }
    }

    fn if_statement<D: Doc>(&mut self, node: &Node<'_, D>, state: Flowing) -> Visit {
        let (mut taken, mut rest) = match node.field("condition") {
            Some(condition) => self.condition(&condition, state)?,
            None => (state.clone(), state),
        };
        let mut out = match node.field("body") {
            Some(body) => self.statement(&body, taken)?,
            None => taken,
        };
        let mut else_seen = false;
        for clause in named(node) {
            match clause.kind().as_ref() {
                "else_if_clause" => {
                    (taken, rest) = match clause.field("condition") {
                        Some(condition) => self.condition(&condition, rest)?,
                        None => (rest.clone(), rest),
                    };
                    let body = match clause.field("body") {
                        Some(body) => self.statement(&body, taken)?,
                        None => taken,
                    };
                    join_into(&mut out, body);
                }
                "else_clause" => {
                    else_seen = true;
                    let body = match clause.field("body") {
                        Some(body) => self.statement(&body, rest.take())?,
                        None => rest.take(),
                    };
                    join_into(&mut out, body);
                }
                _ => {}
            }
        }
        if !else_seen {
            join_into(&mut out, rest);
        }
        Ok(out)
    }

    /// Runs a loop to its fixpoint: `body` takes the state at the top of an iteration and gives
    /// the state at its end; it runs under a frame that collects `break` and `continue`.
    fn fixpoint(
        &mut self,
        entry: Flowing,
        mut body: impl FnMut(&mut Self, Flowing) -> Visit<(Flowing, Flowing)>,
    ) -> Visit {
        let mut top = entry.clone();
        for _ in 0..MAX_LOOP_ITERATIONS {
            self.frames.push(Frame::default());
            let (end, exit) = body(self, top.clone())?;
            let frame = self.frames.pop().unwrap_or_default();
            let next = join(join(entry.clone(), end), frame.continues);
            if next == top {
                return Ok(join(exit, frame.breaks));
            }
            top = next;
        }
        // No fixpoint within the bound: nothing is known after the loop.
        let mut state = join(entry, top);
        Self::arbitrary(&mut state);
        Ok(state)
    }

    fn while_statement<D: Doc>(&mut self, node: &Node<'_, D>, state: Flowing) -> Visit {
        let condition = node.field("condition");
        let body = node.field("body");
        self.fixpoint(state, |this, top| {
            let (taken, exit) = match &condition {
                Some(condition) => this.condition(condition, top)?,
                None => (top.clone(), top),
            };
            let end = match &body {
                Some(body) => this.statement(body, taken)?,
                None => taken,
            };
            Ok((end, exit))
        })
    }

    fn do_statement<D: Doc>(&mut self, node: &Node<'_, D>, state: Flowing) -> Visit {
        let condition = node.field("condition");
        let body = node.field("body");
        // The body runs before the condition; `continue` goes to the condition.
        self.fixpoint(state, |this, top| {
            let end = match &body {
                Some(body) => this.statement(body, top)?,
                None => top,
            };
            let continues = this
                .frames
                .last_mut()
                .and_then(|frame| frame.continues.take());
            let before = join(end, continues);
            match &condition {
                Some(condition) => this.condition(condition, before),
                None => Ok((before.clone(), before)),
            }
        })
    }

    fn for_statement<D: Doc>(&mut self, node: &Node<'_, D>, mut state: Flowing) -> Visit {
        if let Some(initialize) = node.field("initialize") {
            state = self.expression(&initialize, state)?;
        }
        let condition = node.field("condition");
        let update = node.field("update");
        let body = node.field("body");
        self.fixpoint(state, |this, top| {
            let (taken, exit) = match &condition {
                Some(condition) => this.condition(condition, top)?,
                None => (top, None),
            };
            let mut end = match &body {
                Some(body) => this.statement(body, taken)?,
                None => taken,
            };
            let continues = this
                .frames
                .last_mut()
                .and_then(|frame| frame.continues.take());
            end = join(end, continues);
            if let Some(update) = &update {
                end = this.expression(update, end)?;
            }
            Ok((end, exit))
        })
    }

    fn foreach_statement<D: Doc>(&mut self, node: &Node<'_, D>, state: Flowing) -> Visit {
        let parts: Vec<_> = named(node).collect();
        let body = node.field("body");
        let mut state = state;
        let (subject, target) = match parts.as_slice() {
            [subject, target, ..] => (Some(subject.clone()), Some(target.clone())),
            _ => (None, None),
        };
        if let Some(subject) = &subject {
            state = self.expression(subject, state)?;
        }
        self.fixpoint(state, |this, top| {
            let mut bound = top.clone();
            if let Some(target) = &target {
                bound = this.foreach_target(target, node, bound)?;
            }
            let end = match &body {
                Some(body) => this.statement(body, bound)?,
                None => bound,
            };
            // The loop may end before any iteration or after any.
            Ok((end, top))
        })
    }

    fn foreach_target<D: Doc>(
        &mut self,
        target: &Node<'_, D>,
        foreach: &Node<'_, D>,
        mut state: Flowing,
    ) -> Visit {
        match target.kind().as_ref() {
            "pair" => {
                let parts: Vec<_> = named(target).collect();
                for part in parts {
                    state = self.foreach_target(&part, foreach, state)?;
                }
                Ok(state)
            }
            "by_ref" => {
                for part in named(target) {
                    if variable(&part).is_some() {
                        self.bind(
                            &part,
                            Form::For,
                            Some(foreach),
                            None,
                            &mut state,
                            true,
                            Outer::Local,
                        );
                    }
                }
                Ok(state)
            }
            "list_literal" | "array_creation_expression" => {
                self.destructure(target, Form::For, foreach, None, state)
            }
            "variable_name" => {
                self.bind(
                    target,
                    Form::For,
                    Some(foreach),
                    None,
                    &mut state,
                    false,
                    Outer::Keep,
                );
                Ok(state)
            }
            _ => self.expression(target, state),
        }
    }

    fn switch_statement<D: Doc>(&mut self, node: &Node<'_, D>, state: Flowing) -> Visit {
        let state = match node.field("condition") {
            Some(condition) => self.expression(&condition, state)?,
            None => state,
        };
        // PHP compares the cases in order until one matches, so a case is entered with every
        // earlier case's value evaluated, and `default` (wherever it is written) and the exit
        // without a match with all of them.
        let cases: Vec<_> = node
            .field("body")
            .map(|block| {
                named(&block)
                    .filter(|case| {
                        matches!(case.kind().as_ref(), "case_statement" | "default_statement")
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut unmatched = state;
        let mut entries = Vec::with_capacity(cases.len());
        for case in &cases {
            match case.field("value") {
                Some(value) if case.kind() == "case_statement" => {
                    unmatched = self.expression(&value, unmatched)?;
                    entries.push(Some(unmatched.clone()));
                }
                _ => entries.push(None),
            }
        }
        self.frames.push(Frame::default());
        let mut fall: Flowing = None;
        let has_default = cases.iter().any(|case| case.kind() == "default_statement");
        for (case, entry) in cases.iter().zip(entries) {
            let entry = join(entry.unwrap_or_else(|| unmatched.clone()), fall.take());
            let statements: Vec<_> = named(case)
                .filter(|child| {
                    case.field("value")
                        .is_none_or(|value| value.range() != child.range())
                })
                .collect();
            fall = self.statements(statements.into_iter(), entry)?;
        }
        let frame = self.frames.pop().unwrap_or_default();
        let mut state = join(join(fall, frame.breaks), frame.continues);
        if !has_default {
            state = join(state, unmatched);
        }
        Ok(state)
    }

    /// Takes what `break`, `continue` and `return` have carried so far (each frame's breaks and
    /// continues, and the returns), leaving none.
    fn take_exits(&mut self) -> Exits {
        let frames = self
            .frames
            .iter_mut()
            .map(|frame| (frame.breaks.take(), frame.continues.take()))
            .collect();
        (frames, self.returns.take())
    }

    /// Puts back what `take_exits` took, giving what was carried since.
    fn restore_exits(&mut self, (frames, returns): Exits) -> Exits {
        let carried = self.take_exits();
        for (frame, (breaks, continues)) in self.frames.iter_mut().zip(frames) {
            frame.breaks = breaks;
            frame.continues = continues;
        }
        self.returns = returns;
        carried
    }

    fn try_statement<D: Doc>(&mut self, node: &Node<'_, D>, state: Flowing) -> Visit {
        let finally = named(node)
            .find(|clause| clause.kind() == "finally_clause")
            .and_then(|clause| clause.field("body"));
        // A `finally` runs before a `break`, `continue` or `return` of the body or a catch goes
        // where it goes: what they carry is collected apart from what was carried before.
        let before = finally.as_ref().map(|_| self.take_exits());
        self.throws.push(state.clone());
        let end = match node.field("body") {
            Some(body) => self.statement(&body, state)?,
            None => state,
        };
        let mut thrown = self.throws.pop().flatten();
        // So it does before a throw out of a catch goes on.
        if finally.is_some() {
            self.throws.push(None);
        }
        let mut out = end;

        for clause in named(node) {
            if clause.kind() != "catch_clause" {
                continue;
            }
            let mut entry = thrown.clone();
            if let Some(name) = clause.field("name") {
                // PHP assigns the exception through a reference the variable is.
                self.bind(
                    &name,
                    Form::Except,
                    Some(&clause),
                    None,
                    &mut entry,
                    false,
                    Outer::Keep,
                );
            }
            let body = match clause.field("body") {
                Some(body) => self.statement(&body, entry)?,
                None => entry,
            };
            join_into(&mut out, body);
        }
        let (Some(body), Some(before)) = (finally, before) else {
            return Ok(out);
        };
        join_into(&mut thrown, self.throws.pop().flatten());
        let (frames, returns) = self.restore_exits(before);
        // The `finally` runs on each way out: after the body or a catch, on to where each
        // `break`, `continue` and `return` goes, and on a throw, which it passes on (its
        // statements may throw). Each way is visited apart, then all of them at once, last, so
        // its reads are what any way reaches them with.
        let throwing = thrown.is_some();
        let mut every = thrown;
        join_into(&mut every, out.clone());
        let after = match out {
            Some(_) => self.statement(&body, out)?,
            None => None,
        };
        for (index, (breaks, continues)) in frames.into_iter().enumerate() {
            if breaks.is_some() {
                join_into(&mut every, breaks.clone());
                let ran = self.statement(&body, breaks)?;
                if let Some(frame) = self.frames.get_mut(index) {
                    join_into(&mut frame.breaks, ran);
                }
            }
            if continues.is_some() {
                join_into(&mut every, continues.clone());
                let ran = self.statement(&body, continues)?;
                if let Some(frame) = self.frames.get_mut(index) {
                    join_into(&mut frame.continues, ran);
                }
            }
        }
        if returns.is_some() {
            join_into(&mut every, returns.clone());
            let ran = self.statement(&body, returns)?;
            join_into(&mut self.returns, ran);
        }
        if every.is_some() {
            let ran = self.statement(&body, every)?;
            // After a throw it passes the exception on, to an enclosing `try`'s catches.
            if throwing {
                self.may_throw(&ran);
            }
        }
        Ok(after)
    }

    // -----------------------------------------------------------------------------------------
    // Conditions
    // -----------------------------------------------------------------------------------------

    /// The states where `node` is truthy and where it is falsy.
    fn condition<D: Doc>(
        &mut self,
        node: &Node<'_, D>,
        state: Flowing,
    ) -> Visit<(Flowing, Flowing)> {
        match node.kind().as_ref() {
            "parenthesized_expression" => match named(node).next() {
                Some(inner) => self.condition(&inner, state),
                None => Ok((state.clone(), state)),
            },
            "unary_op_expression"
                if node
                    .field("operator")
                    .is_some_and(|operator| operator.text() == "!") =>
            {
                match node.field("argument").or_else(|| named(node).last()) {
                    Some(operand) => {
                        let (truthy, falsy) = self.condition(&operand, state)?;
                        Ok((falsy, truthy))
                    }
                    None => Ok((state.clone(), state)),
                }
            }
            "binary_expression" => {
                let operator = node
                    .field("operator")
                    .map(|operator| operator.text().to_ascii_lowercase())
                    .unwrap_or_default();
                let (Some(left), Some(right)) = (node.field("left"), node.field("right")) else {
                    let state = self.expression(node, state)?;
                    return Ok((state.clone(), state));
                };
                match operator.as_str() {
                    "&&" | "and" => {
                        let (left_true, left_false) = self.condition(&left, state)?;
                        let (right_true, right_false) = self.condition(&right, left_true)?;
                        Ok((right_true, join(left_false, right_false)))
                    }
                    "||" | "or" => {
                        let (left_true, left_false) = self.condition(&left, state)?;
                        let (right_true, right_false) = self.condition(&right, left_false)?;
                        Ok((join(left_true, right_true), right_false))
                    }
                    _ => {
                        let state = self.expression(node, state)?;
                        Ok((state.clone(), state))
                    }
                }
            }
            "function_call_expression" if Self::callee(node).as_deref() == Some("isset") => {
                let state = self.expression(node, state)?;
                // Where `isset` holds, each variable it names that some path binds is defined. One
                // no path binds stays undefined there, as PHPStan reads the branch.
                let mut truthy = state.clone();
                for argument in Self::argument_values(node) {
                    if let (Some(name), Some(truthy)) = (variable(&argument), &mut truthy)
                        && let Some(var) = truthy.vars.get_mut(name.as_str())
                        && (!var.defs.is_empty() || var.unknown)
                    {
                        var.undefined = false;
                    }
                }
                Ok((truthy, state))
            }
            "function_call_expression" if Self::callee(node).as_deref() == Some("empty") => {
                let state = self.expression(node, state)?;
                let mut falsy = state.clone();
                if let [argument] = Self::argument_values(node).as_slice()
                    && let (Some(name), Some(falsy)) = (variable(argument), &mut falsy)
                    && let Some(var) = falsy.vars.get_mut(name.as_str())
                    && (!var.defs.is_empty() || var.unknown)
                {
                    var.undefined = false;
                }
                Ok((state, falsy))
            }
            _ => {
                let state = self.expression(node, state)?;
                Ok((state.clone(), state))
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Expressions
    // -----------------------------------------------------------------------------------------

    /// The lowercased name a call's function is written as, without a leading `\`.
    fn callee<D: Doc>(node: &Node<'_, D>) -> Option<String> {
        let function = node.field("function")?;
        match function.kind().as_ref() {
            "name" => Some(function.text().to_ascii_lowercase()),
            "qualified_name"
                if function.text().starts_with('\\') && !function.text()[1..].contains('\\') =>
            {
                Some(function.text()[1..].to_ascii_lowercase())
            }
            _ => None,
        }
    }

    /// The lowercased name of the PHP builtin a call calls: the constructs `isset`, `empty`,
    /// `eval`, `exit` and `die`, or a function name resolution resolves to a function PHP 8.5.11
    /// defines. `None` for any other callee.
    fn builtin<D: Doc>(&self, node: &Node<'_, D>) -> Option<String> {
        let written = Self::callee(node)?;
        if matches!(
            written.as_str(),
            "isset" | "empty" | "eval" | "exit" | "die"
        ) {
            return Some(written);
        }
        let function = node.field("function")?;
        match &self.names.reference(&function.range())?.resolved {
            Resolved::Name(name) => {
                let name = name.to_ascii_lowercase();
                php_names::builtin_function(&name).then_some(name)
            }
            _ => None,
        }
    }

    /// A call's argument expressions, in order (spreads and named arguments included as their
    /// values).
    fn argument_values<'r, D: Doc>(node: &Node<'r, D>) -> Vec<Node<'r, D>> {
        let Some(arguments) = node.field("arguments") else {
            return Vec::new();
        };
        named(&arguments)
            .filter(|argument| argument.kind() == "argument")
            .filter_map(|argument| {
                let name = argument.field("name");
                named(&argument).find(|part| {
                    name.as_ref()
                        .is_none_or(|name| name.range() != part.range())
                        && part.kind() != "reference_modifier"
                })
            })
            .collect()
    }

    fn expressions<'r, D: Doc + 'r>(
        &mut self,
        nodes: impl Iterator<Item = Node<'r, D>>,
        mut state: Flowing,
    ) -> Visit {
        for node in nodes {
            state = self.expression(&node, state)?;
        }
        Ok(state)
    }

    fn expression<D: Doc>(&mut self, node: &Node<'_, D>, state: Flowing) -> Visit {
        self.builder.enter()?;
        let state = self.expression_inner(node, state)?;
        self.builder.leave();
        Ok(state)
    }

    fn expression_inner<D: Doc>(&mut self, node: &Node<'_, D>, mut state: Flowing) -> Visit {
        match node.kind().as_ref() {
            "variable_name" => {
                self.read(node, &state, Quiet::No);
                Ok(state)
            }
            // `A::$b` names a static property, not a variable; `A::$$b` reads `$b`.
            "scoped_property_access_expression" => {
                if let Some(scope) = node.field("scope") {
                    state = self.expression(&scope, state)?;
                }
                match node.field("name") {
                    Some(name) if name.kind() != "variable_name" => self.expression(&name, state),
                    _ => Ok(state),
                }
            }
            "dynamic_variable_name" => {
                // `$$name` reads a variable the source does not spell.
                state = self.expressions(named(node), state)?;
                self.flow.reads_all.insert(self.scope);
                Ok(state)
            }
            "assignment_expression" | "reference_assignment_expression" => {
                let reference = node.kind() == "reference_assignment_expression";
                let (Some(left), Some(right)) = (node.field("left"), node.field("right")) else {
                    return self.expressions(named(node), state);
                };
                if reference && variable(&right).is_some() {
                    // `&$b` creates `$b` if it is undefined; both now name one value.
                    self.read(&right, &state, Quiet::Silent);
                    if let (Some(name), Some(state)) = (variable(&right), &mut state) {
                        let var = state.vars.entry(name.into()).or_insert_with(Var::undefined);
                        var.unknown |= var.undefined;
                        var.aliased = true;
                        // What reaches `$b` may be read through the reference.
                        Self::alias_reaching(&mut self.flow, &self.bindings, var);
                    }
                } else {
                    state = self.expression(&right, state)?;
                }
                let outer = if reference {
                    Self::reference_outer(&left, &right)
                } else {
                    Outer::Keep
                };
                let carried = match variable(&left) {
                    Some(name) if !reference && Self::discarded(node) => self_reads(&right, &name),
                    _ => Vec::new(),
                };
                state = self.target(&left, node, Some(&right), reference, outer, state)?;
                self.carry(&left, &carried);
                Ok(state)
            }
            "augmented_assignment_expression" => {
                let (Some(left), Some(right)) = (node.field("left"), node.field("right")) else {
                    return self.expressions(named(node), state);
                };
                let coalesce = node
                    .field("operator")
                    .is_some_and(|operator| operator.text() == "??=");
                if coalesce {
                    // `$a ??= v` assigns only where `$a` is null or undefined; either way `$a` is
                    // defined after it.
                    // `$a[k] ??= v` reads `$a[k]` before it may assign it; its base is bound anew
                    // once, there.
                    let before = match variable(&left) {
                        Some(_) => {
                            self.read(&left, &state, Quiet::Silent);
                            state
                        }
                        None => {
                            let state = self.target_base(&left, state)?;
                            self.read_element(&left);
                            state
                        }
                    };
                    let assigned = self.expression(&right, before.clone())?;
                    let assigned = match variable(&left) {
                        Some(_) => {
                            self.target(&left, node, Some(&right), false, Outer::Keep, assigned)?
                        }
                        None => assigned,
                    };
                    let mut after = join(assigned, before);
                    if let (Some(name), Some(after)) = (variable(&left), &mut after)
                        && let Some(var) = after.vars.get_mut(name.as_str())
                    {
                        var.undefined = false;
                    }
                    return Ok(after);
                }
                // A discarded update (`$a .= v;`, `$a[k] .= v;`) changes the variable: its old
                // value is used where the new one is. Otherwise its value is read.
                let discarded = Self::discarded(node);
                let previous = Self::defs(&state, &left);
                match variable(&left) {
                    Some(_) => self.read(&left, &state, Quiet::No),
                    None => {
                        state = self.target_base(&left, state)?;
                        if !discarded {
                            self.read_element(&left);
                        }
                    }
                }
                state = self.expression(&right, state)?;
                if let Some(name) = variable(&left) {
                    self.bind(
                        &left,
                        Form::Augmented,
                        Some(node),
                        None,
                        &mut state,
                        false,
                        Outer::Keep,
                    );
                    if discarded {
                        self.changed(&left, previous, Modify::Update);
                        // `$a .= $a . 'x';` reads `$a` on its right only to make the new `$a`.
                        self.carry(&left, &self_reads(&right, &name));
                    }
                }
                Ok(state)
            }
            "update_expression" => {
                let Some(argument) = node.field("argument") else {
                    return self.expressions(named(node), state);
                };
                let discarded = Self::discarded(node);
                match variable(&argument) {
                    Some(_) => {
                        let previous = Self::defs(&state, &argument);
                        self.read(&argument, &state, Quiet::No);
                        self.bind(
                            &argument,
                            Form::Augmented,
                            Some(node),
                            None,
                            &mut state,
                            false,
                            Outer::Keep,
                        );
                        if discarded {
                            self.changed(&argument, previous, Modify::Update);
                        }
                        Ok(state)
                    }
                    None => {
                        let state = self.target_base(&argument, state)?;
                        if !discarded {
                            self.read_element(&argument);
                        }
                        Ok(state)
                    }
                }
            }
            "binary_expression" => {
                let operator = node
                    .field("operator")
                    .map(|operator| operator.text().to_ascii_lowercase())
                    .unwrap_or_default();
                let (Some(left), Some(right)) = (node.field("left"), node.field("right")) else {
                    return self.expressions(named(node), state);
                };
                match operator.as_str() {
                    "&&" | "and" | "||" | "or" => {
                        let (truthy, falsy) = self.condition(node, state)?;
                        Ok(join(truthy, falsy))
                    }
                    "??" => {
                        let left_state = self.coalesce_left(&left, state)?;
                        let right_state = self.expression(&right, left_state.clone())?;
                        Ok(join(left_state, right_state))
                    }
                    _ => {
                        state = self.expression(&left, state)?;
                        self.expression(&right, state)
                    }
                }
            }
            "conditional_expression" => {
                let Some(condition) = node.field("condition") else {
                    return self.expressions(named(node), state);
                };
                let (truthy, falsy) = self.condition(&condition, state)?;
                let body = match node.field("body") {
                    Some(body) => self.expression(&body, truthy)?,
                    None => truthy,
                };
                let alternative = match node.field("alternative") {
                    Some(alternative) => self.expression(&alternative, falsy)?,
                    None => falsy,
                };
                Ok(join(body, alternative))
            }
            "match_expression" => {
                if let Some(condition) = node.field("condition") {
                    state = self.expression(&condition, state)?;
                }
                let mut out = None;
                if let Some(block) = node.field("body") {
                    for arm in named(&block) {
                        if let Some(conditions) = arm.field("conditional_expressions") {
                            state = self.expression(&conditions, state)?;
                        }
                        if let Some(value) = arm.field("return_expression") {
                            let arm_state = self.expression(&value, state.clone())?;
                            join_into(&mut out, arm_state);
                        }
                    }
                }
                // Without a default arm, no arm matching throws: only the arms' states leave.
                Ok(out)
            }
            "function_call_expression" => self.call(node, state),
            "member_call_expression" | "nullsafe_member_call_expression" => {
                self.method_call_chain(node, state)
            }
            "scoped_call_expression" | "object_creation_expression" => {
                for child in named(node) {
                    if child.kind() != "arguments" {
                        state = self.expression(&child, state)?;
                    }
                }
                self.arguments(node, None, state)
            }
            "include_expression"
            | "include_once_expression"
            | "require_expression"
            | "require_once_expression" => {
                state = self.expressions(named(node), state)?;
                // The included file runs in this scope: it may read, define and unset anything.
                self.flow.reads_all.insert(self.scope);
                Self::arbitrary(&mut state);
                Ok(state)
            }
            "throw_expression" => {
                state = self.expressions(named(node), state)?;
                self.may_throw(&state);
                Ok(None)
            }
            "anonymous_function" => {
                self.closure(node, &mut state)?;
                Ok(state)
            }
            "arrow_function" => {
                self.arrow(node, &state)?;
                Ok(state)
            }
            "class_declaration" | "anonymous_class" => {
                for child in named(node) {
                    if child.kind() == "arguments" {
                        state = self.arguments(node, None, state)?;
                    }
                }
                self.class(node)?;
                Ok(state)
            }
            "encapsed_string" | "heredoc" | "heredoc_body" | "string" | "nowdoc" => {
                // tree-sitter-php 0.24.2 reads a heredoc's text after an `<?php` in it as PHP (a
                // `/* ... */` there becomes a comment, hiding the `$x` PHP interpolates in it):
                // a heredoc with a part no heredoc has may read any variable.
                if node.kind() == "heredoc"
                    && node.children().any(|part| {
                        part.is_named()
                            && !matches!(
                                part.kind().as_ref(),
                                "heredoc_start" | "heredoc_body" | "heredoc_end"
                            )
                    })
                {
                    self.flow.reads_all.insert(self.scope);
                }
                self.expressions(named(node), state)
            }
            _ => self.expressions(named(node), state),
        }
    }

    /// The left side of `??`: a plain variable is read silently; a longer chain reads its base
    /// silently too, and its other parts (indexes, arguments) as usual.
    fn coalesce_left<D: Doc>(&mut self, node: &Node<'_, D>, state: Flowing) -> Visit {
        match node.kind().as_ref() {
            "variable_name" => {
                self.read(node, &state, Quiet::Silent);
                Ok(state)
            }
            "subscript_expression"
            | "member_access_expression"
            | "nullsafe_member_access_expression" => {
                let parts: Vec<_> = named(node).collect();
                let mut state = state;
                for (index, part) in parts.iter().enumerate() {
                    state = if index == 0 {
                        self.coalesce_left(part, state)?
                    } else {
                        self.expression(part, state)?
                    };
                }
                Ok(state)
            }
            _ => self.expression(node, state),
        }
    }

    /// A chain of method calls (`$a->b(x)->c(y)`, a fluent builder): PHP evaluates the innermost
    /// object, then each call's name and arguments and runs it, from the inside out. The chain
    /// is walked in a loop, so a long one costs a visited node per call but no recursion.
    fn method_call_chain<D: Doc>(&mut self, node: &Node<'_, D>, mut state: Flowing) -> Visit {
        let mut chain = vec![node.clone()];
        while let Some(object) = chain.last().and_then(|call| call.field("object"))
            && matches!(
                object.kind().as_ref(),
                "member_call_expression" | "nullsafe_member_call_expression"
            )
        {
            chain.push(object);
        }
        for (index, call) in chain.iter().rev().enumerate() {
            if index > 0 {
                self.builder.enter()?;
                self.builder.leave();
            }
            let object = call.field("object").map(|object| object.range());
            for child in named(call) {
                let evaluated = index > 0 && object.as_ref() == Some(&child.range());
                if child.kind() != "arguments" && !evaluated {
                    state = self.expression(&child, state)?;
                }
            }
            state = self.arguments(call, None, state)?;
        }
        Ok(state)
    }

    fn call<D: Doc>(&mut self, node: &Node<'_, D>, mut state: Flowing) -> Visit {
        let builtin = self.builtin(node);
        if let Some(function) = node.field("function")
            && !matches!(function.kind().as_ref(), "name" | "qualified_name")
        {
            state = self.expression(&function, state)?;
        }
        match builtin.as_deref() {
            Some("isset") | Some("empty") => {
                for argument in Self::argument_values(node) {
                    match variable(&argument) {
                        Some(_) => self.read(&argument, &state, Quiet::Silent),
                        None => state = self.coalesce_left(&argument, state)?,
                    }
                }
                Ok(state)
            }
            Some("compact") => {
                for argument in Self::argument_values(node) {
                    let literal = matches!(argument.kind().as_ref(), "string" | "encapsed_string")
                        && named(&argument).all(|part| part.kind() == "string_content");
                    if literal {
                        // `compact('a')` reads `$a`: a read at the string, which PHPStan reports
                        // as it does any undefined read.
                        let text = argument.text();
                        let name = format!("${}", &text[1..text.len().saturating_sub(1)]);
                        self.read_named(&argument, &name, &state);
                    } else {
                        state = self.expression(&argument, state)?;
                        self.flow.reads_all.insert(self.scope);
                    }
                }
                Ok(state)
            }
            // `func_get_args()` reads the parameters' current values.
            Some("get_defined_vars") | Some("func_get_args") | Some("func_get_arg") => {
                self.flow.reads_all.insert(self.scope);
                self.arguments(node, None, state)
            }
            Some("extract") => {
                state = self.arguments(node, None, state)?;
                Self::dynamic(&mut state);
                Ok(state)
            }
            // The code `eval` runs in this scope may read, define and unset any variable (`php -r
            // 'function f() { $x = 1; eval("echo \$x;"); } f();'` prints `1`).
            Some("eval") => {
                state = self.arguments(node, None, state)?;
                self.flow.reads_all.insert(self.scope);
                Self::arbitrary(&mut state);
                Ok(state)
            }
            Some("parse_str") if Self::argument_values(node).len() < 2 => {
                state = self.arguments(node, None, state)?;
                Self::dynamic(&mut state);
                Ok(state)
            }
            Some("exit") | Some("die") => {
                self.arguments(node, None, state)?;
                Ok(None)
            }
            // Only PHP's builtins are known here; any other function may take a variable by
            // reference.
            Some(name) => {
                let callee = by_reference::BY_REFERENCE
                    .binary_search_by(|(function, _, _)| (*function).cmp(name))
                    .map_or((&[][..], &[][..]), |index| {
                        let (_, parameters, positions) = by_reference::BY_REFERENCE[index];
                        (parameters, positions)
                    });
                self.arguments(node, Some(callee), state)
            }
            None => self.arguments(node, None, state),
        }
    }

    /// A call's arguments: `callee` holds a known callee's parameter names, in order, and the
    /// positions of those it takes by reference (a variadic one as `-1 - p`); `None` where the
    /// callee is not known, so a variable argument may be taken by reference and so defined or
    /// changed. The callee then runs, and may throw with every argument evaluated: a closure
    /// argument's by-reference `use` is already shared, a by-reference argument already bound.
    fn arguments<D: Doc>(
        &mut self,
        node: &Node<'_, D>,
        callee: Option<(&[&str], &[i8])>,
        mut state: Flowing,
    ) -> Visit {
        let Some(arguments) = node
            .field("arguments")
            .or_else(|| named(node).find(|child| child.kind() == "arguments"))
        else {
            self.may_throw(&state);
            return Ok(state);
        };
        let mut position = 0usize;
        for argument in named(&arguments) {
            if argument.kind() != "argument" {
                state = self.expression(&argument, state)?;
                continue;
            }
            let name = argument.field("name");
            let Some(value) = named(&argument).find(|part| {
                name.as_ref()
                    .is_none_or(|name| name.range() != part.range())
                    && part.kind() != "reference_modifier"
            }) else {
                continue;
            };
            // Whether the callee takes the argument by reference; `None` where it may.
            let referenced = match callee {
                None => None,
                // A spread's value is no variable.
                Some(_) if value.kind() == "variadic_unpacking" => Some(false),
                Some((parameters, positions)) => match &name {
                    // A named argument is the parameter's it names. A name the callee lacks
                    // (PHP throws, or a variadic parameter collects it) says nothing.
                    Some(name) => match parameters.iter().position(|p| *p == name.text()) {
                        Some(at) => Self::by_reference_at(positions, at, true),
                        None if positions.is_empty() => Some(false),
                        None => None,
                    },
                    None => {
                        position += 1;
                        Self::by_reference_at(positions, position - 1, false)
                    }
                },
            };
            if variable(&value).is_none() || referenced == Some(false) {
                state = self.expression(&value, state)?;
                continue;
            }
            self.read(&value, &state, Quiet::ByReference);
            if referenced == Some(true) {
                // A builtin that takes it by reference defines it.
                self.bind(
                    &value,
                    Form::Assignment,
                    Some(node),
                    None,
                    &mut state,
                    true,
                    Outer::Keep,
                );
            } else if let (Some(name), Some(state)) = (variable(&value), &mut state) {
                // An unknown callee may: a variable not defined on every path is unknown after
                // the call, and one that is may be changed.
                let var = state.vars.entry(name.into()).or_insert_with(Var::undefined);
                var.unknown |= var.undefined;
                var.aliased = true;
            }
        }
        self.may_throw(&state);
        Ok(state)
    }

    /// Whether a callee whose by-reference parameters are at `positions` takes its parameter at
    /// `at` by reference; `named`: an argument names it, which says nothing where that is the
    /// variadic one (`None`).
    fn by_reference_at(positions: &[i8], at: usize, named: bool) -> Option<bool> {
        if positions
            .iter()
            .any(|&p| usize::try_from(p).is_ok_and(|p| p == at))
        {
            return Some(true);
        }
        let variadic = positions
            .iter()
            .any(|&p| p < 0 && at >= usize::try_from(-1 - i16::from(p)).unwrap_or(0));
        match (variadic, named) {
            (false, _) => Some(false),
            (true, false) => Some(true),
            (true, true) => None,
        }
    }

    /// A read of the variable a `compact` string names, at the string.
    fn read_named<D: Doc>(&mut self, node: &Node<'_, D>, name: &str, state: &Flowing) {
        let range = node.range();
        let scope = self.builder.current;
        let read = match state {
            Some(state) => {
                let var = state.vars.get(name).cloned().unwrap_or_else(Var::undefined);
                Read {
                    name: name.into(),
                    node: NodeRef::of(node),
                    scope,
                    bindings: var
                        .defs
                        .iter()
                        .map(|id| {
                            let (scope, _, index, _) = &self.bindings[*id as usize];
                            (*scope, *index)
                        })
                        .collect(),
                    defined: self.definedness(name, &var, state, false),
                    aliased: var.aliased,
                    quiet: Quiet::No,
                    superglobal: false,
                    modify: None,
                }
            }
            None => Read {
                name: name.into(),
                node: NodeRef::of(node),
                scope,
                bindings: Box::new([]),
                defined: Defined::Unknown,
                aliased: false,
                quiet: Quiet::No,
                superglobal: false,
                modify: None,
            },
        };
        self.flow.reads.insert(range.start, read);
    }

    /// An assignment target: a variable is bound; a list is destructured; a subscript or property
    /// reads its base (and a subscript creates an undefined array base silently).
    fn target<D: Doc>(
        &mut self,
        node: &Node<'_, D>,
        site: &Node<'_, D>,
        value: Option<&Node<'_, D>>,
        reference: bool,
        outer: Outer,
        mut state: Flowing,
    ) -> Visit {
        match node.kind().as_ref() {
            "variable_name" => {
                let value = if reference { None } else { value };
                self.bind(
                    node,
                    Form::Assignment,
                    Some(site),
                    value,
                    &mut state,
                    reference,
                    outer,
                );
                Ok(state)
            }
            "list_literal" | "array_creation_expression" => {
                let source = if reference { None } else { value };
                self.destructure(node, Form::Assignment, site, source, state)
            }
            "dynamic_variable_name" => {
                state = self.expressions(named(node), state)?;
                Self::dynamic(&mut state);
                Ok(state)
            }
            // PHP casts the assignment (`(string) $x = f()` is `(string) ($x = f())`), where
            // tree-sitter-php 0.24.2 reads the cast into the target.
            "cast_expression" => match node.field("value") {
                Some(operand) => self.target(&operand, site, value, reference, outer, state),
                None => Ok(state),
            },
            _ => self.target_base(node, state),
        }
    }

    /// Whether `$x = &right` makes `$x` an outer variable: `&$GLOBALS['x']` is `global $x`; a
    /// `$GLOBALS` element whose key is not a literal may be; anything else is not.
    fn reference_outer<D: Doc>(left: &Node<'_, D>, right: &Node<'_, D>) -> Outer {
        let parts: Vec<_> = match right.kind().as_ref() {
            "subscript_expression" => named(right).collect(),
            _ => return Outer::Local,
        };
        let ([base, key], Some(name)) = (parts.as_slice(), variable(left)) else {
            return Outer::Local;
        };
        if base.kind() != "variable_name" || base.text() != "$GLOBALS" {
            return Outer::Local;
        }
        let literal = matches!(key.kind().as_ref(), "string" | "encapsed_string")
            && named(key).all(|part| part.kind() == "string_content");
        if !literal {
            return Outer::Unknown;
        }
        let text = key.text();
        if text.get(1..text.len().saturating_sub(1)) == Some(&name[1..]) {
            Outer::Shared
        } else {
            Outer::Local
        }
    }

    /// A discarded `$a = $a + v` (or the right side of `$a .= $a . v`) reads `$a` (at `reads`)
    /// only to make the binding at `target`: those reads change the value rather than use it, and
    /// whatever reads the new binding reads what they read (the chain `changed` records). Psalm
    /// 6.19.1 reports `$a = 5; $a = $a + 1;` both unused: its variable-use graph reaches no use
    /// from either.
    fn carry<D: Doc>(&mut self, target: &Node<'_, D>, reads: &[usize]) {
        if reads.is_empty() {
            return;
        }
        let Some(&id) = self.sites.get(&(target.range().start, target.range().end)) else {
            return;
        };
        let (scope, name, index, _) = self.bindings[id as usize].clone();
        let mut previous = Vec::new();
        for start in reads {
            let Some(read) = self.flow.reads.get_mut(start) else {
                continue;
            };
            read.modify = Some(Modify::Update);
            for &(read_scope, read_index) in read.bindings.iter() {
                let key = (read_scope, read.name.clone(), read_index);
                if (read_scope, read_index) != (scope, index) && !previous.contains(&key) {
                    previous.push(key);
                }
            }
        }
        let chain = self.flow.modifies.entry((scope, name, index)).or_default();
        for key in previous {
            if !chain.contains(&key) {
                chain.push(key);
            }
        }
    }

    /// `$a[k] = v`, `$a[] = v`, `$o->p = v`: the base is read (a subscript base silently, as PHP
    /// creates an undefined array) and a subscripted variable is bound anew, since its value
    /// changes.
    fn target_base<D: Doc>(&mut self, node: &Node<'_, D>, mut state: Flowing) -> Visit {
        match node.kind().as_ref() {
            "subscript_expression" => {
                let parts: Vec<_> = named(node).collect();
                for part in parts.iter().skip(1) {
                    state = self.expression(part, state)?;
                }
                let Some(base) = parts.first() else {
                    return Ok(state);
                };
                if variable(base).is_none() {
                    return self.target_base(base, state);
                }
                // PHP creates an undefined array silently; the variable's value changes, so it is
                // bound anew, and whatever reads the new binding reads what it was made from.
                self.read(base, &state, Quiet::Silent);
                let previous = Self::defs(&state, base);
                self.bind(
                    base,
                    Form::Assignment,
                    Some(node),
                    None,
                    &mut state,
                    false,
                    Outer::Keep,
                );
                self.changed(base, previous, Modify::Element);
                Ok(state)
            }
            _ => self.expression(node, state),
        }
    }

    /// The bindings of the variable `node` names that reach it under `state`.
    fn defs<D: Doc>(state: &Flowing, node: &Node<'_, D>) -> Vec<u32> {
        match (state, variable(node)) {
            (Some(state), Some(name)) => state
                .vars
                .get(name.as_str())
                .map(|var| var.defs.iter().copied().collect())
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// Marks the read of the variable `node` names as a change of its value (`modify`) and
    /// records the binding made at `node` as made from `previous`: a read of the new binding
    /// reads what they assigned too.
    fn changed<D: Doc>(&mut self, node: &Node<'_, D>, previous: Vec<u32>, modify: Modify) {
        if let Some(read) = self.flow.reads.get_mut(&node.range().start) {
            read.modify = Some(modify);
        }
        if let Some(id) = self.sites.get(&(node.range().start, node.range().end)) {
            let (scope, name, index, _) = self.bindings[*id as usize].clone();
            let previous: Vec<BindingKey> = previous
                .into_iter()
                .filter(|previous| previous != id)
                .map(|previous| {
                    let (scope, name, index, _) = &self.bindings[previous as usize];
                    (*scope, name.clone(), *index)
                })
                .collect();
            // A loop's body, or `$a[k] ??= v`, reaches the site more than once: keep every
            // binding each visit changed.
            let chain = self.flow.modifies.entry((scope, name, index)).or_default();
            for key in previous {
                if !chain.contains(&key) {
                    chain.push(key);
                }
            }
        }
    }

    /// Whether an assignment's or increment's value is discarded: it stands as a statement or as
    /// a `for` clause.
    fn discarded<D: Doc>(node: &Node<'_, D>) -> bool {
        node.parent().is_some_and(|parent| {
            matches!(
                parent.kind().as_ref(),
                "expression_statement" | "for_statement" | "sequence_expression"
            )
        })
    }

    /// `$a[k] .= v`, `$a[k]++`, `$a[k] ??= v` read the element they change: the subscripted
    /// variable's read (`target_base`) uses its value, rather than only changing it.
    fn read_element<D: Doc>(&mut self, node: &Node<'_, D>) {
        if node.kind() != "subscript_expression" {
            return;
        }
        let mut base = node.clone();
        while base.kind() == "subscript_expression" {
            let Some(inner) = named(&base).next() else {
                return;
            };
            base = inner;
        }
        if let Some(read) = self.flow.reads.get_mut(&base.range().start) {
            read.modify = None;
        }
    }

    /// `unset($a[k])`: its keys and base are read as `$a[k] = v` reads them, and a defined array
    /// changes, so its variable is bound anew; but PHP defines no variable here, so on a path
    /// where it is undefined it stays undefined.
    fn unset_base<D: Doc>(&mut self, node: &Node<'_, D>, state: Flowing) -> Visit {
        let mut base = node.clone();
        while base.kind() == "subscript_expression" {
            let Some(inner) = named(&base).next() else {
                break;
            };
            base = inner;
        }
        let name = variable(&base);
        let before = match (&state, &name) {
            (Some(state), Some(name)) => Some(state.vars.get(name.as_str()).cloned()),
            _ => None,
        };
        let mut state = self.target_base(node, state)?;
        // Where the variable is undefined the binding is never made: it overwrites no undefined
        // variable.
        if let Some(id) = self.sites.get(&(base.range().start, base.range().end))
            && let Some(reach) = self.before.get_mut(id)
        {
            reach.1 = false;
        }
        if let (Some(before), Some(name), Some(state)) = (before, name, &mut state) {
            match before {
                None => {
                    state.vars.remove(name.as_str());
                }
                Some(before) => {
                    if let Some(var) = state.vars.get_mut(name.as_str()) {
                        if before.defs.is_empty() {
                            var.defs.clear();
                        }
                        var.undefined = before.undefined;
                        var.unknown = before.unknown;
                    }
                }
            }
        }
        Ok(state)
    }

    /// Destructures a `list(...)` or `[...]` target: keys are read, each target bound. Where the
    /// assigned `source` is an array literal whose keys are known, each target's value is the
    /// element its key (written, or its position) selects, as PHP destructures.
    fn destructure<'n, 'r, D: Doc>(
        &mut self,
        node: &Node<'n, D>,
        form: Form,
        site: &Node<'_, D>,
        source: Option<&Node<'r, D>>,
        mut state: Flowing,
    ) -> Visit {
        let slots = Self::slots(node);
        let values = self.elements(&slots, source);
        for ((key, target), value) in slots.into_iter().zip(values) {
            let Some(target) = target else {
                continue;
            };
            if let Some(key) = key {
                state = self.expression(&key, state)?;
            }
            state = match target.kind().as_ref() {
                "variable_name" => {
                    self.bind(
                        &target,
                        form,
                        Some(site),
                        value.as_ref(),
                        &mut state,
                        false,
                        Outer::Keep,
                    );
                    state
                }
                "by_ref" => {
                    for part in named(&target) {
                        if variable(&part).is_some() {
                            self.bind(
                                &part,
                                form,
                                Some(site),
                                None,
                                &mut state,
                                true,
                                Outer::Local,
                            );
                        }
                    }
                    state
                }
                "list_literal" | "array_creation_expression" => {
                    self.destructure(&target, form, site, value.as_ref(), state)?
                }
                _ => self.target(&target, site, None, false, Outer::Keep, state)?,
            };
        }
        Ok(state)
    }

    /// A destructuring's slots in order, each its key and its target (both none in an empty
    /// slot, `[, $b]`): an array literal's elements, or a `list_literal`'s comma-separated parts,
    /// whose `=>` follows a key.
    #[allow(clippy::type_complexity)]
    fn slots<'r, D: Doc>(node: &Node<'r, D>) -> Vec<(Option<Node<'r, D>>, Option<Node<'r, D>>)> {
        if node.kind() == "array_creation_expression" {
            return named(node)
                .map(|element| {
                    let parts: Vec<_> = named(&element).collect();
                    match parts.as_slice() {
                        [key, target] => (Some(key.clone()), Some(target.clone())),
                        [target] => (None, Some(target.clone())),
                        _ => (None, None),
                    }
                })
                .collect();
        }
        let mut slots = Vec::new();
        let (mut key, mut target) = (None, None);
        for child in node.children() {
            match child.kind().as_ref() {
                "," => slots.push((key.take(), target.take())),
                "=>" => key = target.take(),
                "comment" => {}
                _ if child.is_named() => target = Some(child),
                _ => {}
            }
        }
        if key.is_some() || target.is_some() {
            slots.push((key, target));
        }
        slots
    }

    /// The element of `source`, an array literal, each slot's key selects: its written key as PHP
    /// coerces it, or its position. None where `source` is no array literal, a key is not known,
    /// or the literal has no such key (PHP gives `null` with a warning).
    #[allow(clippy::type_complexity)]
    fn elements<'n, 'r, D: Doc>(
        &mut self,
        slots: &[(Option<Node<'n, D>>, Option<Node<'n, D>>)],
        source: Option<&Node<'r, D>>,
    ) -> Vec<Option<Node<'r, D>>> {
        let mut values = vec![None; slots.len()];
        let Some(source) = source.filter(|source| source.kind() == "array_creation_expression")
        else {
            return values;
        };
        let mut spent = 0usize;
        let mut spend = || {
            spent += 1;
            true
        };
        let mut context = Decoding::new(&mut spend);
        if let Some(entries) = php_value::array_entries(source, &mut context) {
            for (position, ((key, _), value)) in slots.iter().zip(values.iter_mut()).enumerate() {
                let key = match key {
                    Some(key) => match php_value::key(key, &mut context) {
                        Some(key) => key,
                        None => continue,
                    },
                    None => php_value::Key::Int(position as i64),
                };
                *value = entries
                    .iter()
                    .find(|(own, _)| *own == key)
                    .map(|(_, element)| element.clone());
            }
        }
        self.builder.table.cost += spent;
        values
    }

    // -----------------------------------------------------------------------------------------
    // Scopes
    // -----------------------------------------------------------------------------------------

    /// Enters the scope a node opens (the same one on a second visit) and makes it current.
    fn enter_scope<D: Doc>(&mut self, node: &Node<'_, D>, kind: ScopeKind) -> (u32, u32) {
        let range = node.range();
        let outer = self.builder.current;
        let scope = match self.scopes.get(&(range.start, range.end)) {
            Some(scope) => {
                self.builder.current = *scope;
                *scope
            }
            None => {
                let scope = self.builder.push(kind, range.clone());
                self.scopes.insert((range.start, range.end), scope);
                scope
            }
        };
        (outer, scope)
    }

    fn class<D: Doc>(&mut self, node: &Node<'_, D>) -> Visit<()> {
        let (outer, _) = self.enter_scope(node, ScopeKind::Class);
        let class = std::mem::replace(&mut self.class, true);
        let body = node.field("body").or_else(|| {
            named(node).find(|child| {
                matches!(
                    child.kind().as_ref(),
                    "declaration_list" | "enum_declaration_list"
                )
            })
        });
        if let Some(body) = body {
            for member in named(&body) {
                // Property defaults, constants and enum cases are constant expressions: they read
                // no variables, and a property's `$name` is none.
                if member.kind() == "method_declaration" {
                    let is_static = named(&member).any(|child| child.kind() == "static_modifier");
                    self.function(
                        &member,
                        Context::Function {
                            this: Some(!is_static),
                        },
                    )?;
                }
            }
        }
        self.class = class;
        self.builder.current = outer;
        Ok(())
    }

    /// Visits a function, method or closure body in a scope of its own: its parameters, and a
    /// closure's `use` variables, are bound where it starts.
    fn function<D: Doc>(&mut self, node: &Node<'_, D>, context: Context) -> Visit<()> {
        let kind = match node.kind().as_ref() {
            "anonymous_function" => ScopeKind::Lambda,
            _ => ScopeKind::Function,
        };
        let (outer, scope) = self.enter_scope(node, kind);
        let saved_scope = std::mem::replace(&mut self.scope, scope);
        let saved_context = std::mem::replace(&mut self.context, context);
        let saved_frames = std::mem::take(&mut self.frames);
        let saved_throws = std::mem::take(&mut self.throws);
        let saved_returns = self.returns.take();
        let mut state = Some(State::default());
        if let Some(parameters) = node.field("parameters") {
            state = self.parameters(&parameters, state)?;
        }
        {
            // A closure's `use` variables are its own, copied (or referenced) at its creation.
            if let Some(clause) =
                named(node).find(|child| child.kind() == "anonymous_function_use_clause")
            {
                for part in named(&clause) {
                    let (variable_node, by_reference) = match part.kind().as_ref() {
                        "by_ref" => match named(&part).next() {
                            Some(inner) => (inner, true),
                            None => continue,
                        },
                        _ => (part.clone(), false),
                    };
                    if variable(&variable_node).is_some() {
                        self.bind(
                            &variable_node,
                            Form::Parameter,
                            Some(&clause),
                            None,
                            &mut state,
                            by_reference,
                            if by_reference {
                                Outer::Shared
                            } else {
                                Outer::Local
                            },
                        );
                    }
                }
            }
        }
        let end = match node.field("body") {
            Some(body) => self.statement(&body, state)?,
            None => state,
        };
        let returns = std::mem::replace(&mut self.returns, saved_returns);
        self.end(scope, join(end, returns));
        self.frames = saved_frames;
        self.throws = saved_throws;
        self.context = saved_context;
        self.scope = saved_scope;
        self.builder.current = outer;
        Ok(())
    }

    fn parameters<D: Doc>(&mut self, node: &Node<'_, D>, mut state: Flowing) -> Visit {
        for parameter in named(node) {
            if let Some(default) = parameter.field("default_value") {
                // A default is a constant expression: its reads leave the parameters' state as is.
                self.expression(&default, state.clone())?;
            }
            let name = parameter.field("name");
            let (name, by_reference) = match name {
                Some(name) if name.kind() == "by_ref" => match named(&name).next() {
                    Some(inner) => (inner, true),
                    None => continue,
                },
                Some(name) => (name, parameter.field("reference_modifier").is_some()),
                None => continue,
            };
            self.bind(
                &name,
                Form::Parameter,
                Some(&parameter),
                None,
                &mut state,
                by_reference,
                Outer::Local,
            );
        }
        Ok(state)
    }

    fn closure<D: Doc>(&mut self, node: &Node<'_, D>, state: &mut Flowing) -> Visit<()> {
        // The `use` clause runs where the closure is created: a by-value variable is read there,
        // a by-reference one is created there if undefined.
        if let Some(clause) =
            named(node).find(|child| child.kind() == "anonymous_function_use_clause")
        {
            for part in named(&clause) {
                match part.kind().as_ref() {
                    "by_ref" => {
                        if let Some(inner) = named(&part).next() {
                            // The closure's variable is a reference to this one: what it holds
                            // now is what the closure reads when it runs (silently: `&` creates
                            // an undefined one), and what reaches here may be read or changed
                            // through it.
                            self.read(&inner, state, Quiet::Silent);
                            self.mention(&inner);
                            if let (Some(name), Some(state)) = (variable(&inner), state.as_mut()) {
                                let var =
                                    state.vars.entry(name.into()).or_insert_with(Var::undefined);
                                var.undefined = false;
                                var.aliased = true;
                                // The closure may read what reaches here through its variable.
                                Self::alias_reaching(&mut self.flow, &self.bindings, var);
                            }
                        }
                    }
                    _ => {
                        self.read(&part, state, Quiet::No);
                    }
                }
            }
        }
        let is_static = node.field("static_modifier").is_some();
        // A closure has its creator's `$this`, unless static; outside a method's `$this`,
        // `Closure::bind` or `->call()` may give it one.
        let this = match (is_static, self.context) {
            (true, _) => Some(false),
            (false, Context::Function { this: Some(true) }) => Some(true),
            _ => None,
        };
        self.function(node, Context::Function { this })
    }

    /// An arrow function sees, by value, every variable of where it is written.
    fn arrow<D: Doc>(&mut self, node: &Node<'_, D>, state: &Flowing) -> Visit<()> {
        let (outer, scope) = self.enter_scope(node, ScopeKind::Lambda);
        self.flow.arrows.insert(scope);
        let saved_scope = std::mem::replace(&mut self.scope, scope);
        let is_static = node.field("static_modifier").is_some();
        let saved_context = self.context;
        if is_static {
            self.context = Context::Function { this: Some(false) };
        }
        // The arrow function's variables are copies: writing one writes no outer variable.
        let mut inner = state.clone();
        if let Some(inner) = &mut inner {
            for var in inner.vars.values_mut() {
                var.outer = LOCAL;
            }
        }
        if let Some(parameters) = node.field("parameters") {
            inner = self.parameters(&parameters, inner)?;
        }
        let end = match node.field("body") {
            Some(body) => self.expression(&body, inner)?,
            None => inner,
        };
        self.end(scope, end);
        self.context = saved_context;
        self.scope = saved_scope;
        self.builder.current = outer;
        Ok(())
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod flow_tests;

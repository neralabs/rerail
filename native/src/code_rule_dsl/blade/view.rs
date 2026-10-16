//! The PHP a Blade template embeds, read as one PHP program in the order the template runs it: the
//! program `illuminate/view` 13.34.0's `BladeCompiler` compiles the template to, without the
//! bookkeeping Blade's runtime adds (`$__env`, `e()`, output buffering).
//!
//! The program is written out as PHP text and parsed with tree-sitter-php's PHP-only grammar: a
//! document of its own (`embed`), whose text maps back to the template bytes it came from. The PHP
//! a template spells (an echo's expression, a directive's arguments, a `@php` block, a
//! component's bound attribute) is copied byte for byte, so its nodes stand for exactly those
//! template bytes. The PHP Blade wraps it in is written around it, each token anchored to the
//! Blade token it stands for:
//!
//! | Template | Program |
//! |---|---|
//! | `{{ e }}`, `{!! e !!}`, `<?= e ?>` | `echo e;` |
//! | `@php ... @endphp`, `<?php ... ?>`, `@php(e)` | the statements; `e;` |
//! | `@if(c)` `@elseif(c)` `@else` `@endif` | `if (c):` `elseif (c):` `else:` `endif;` |
//! | `@unless(c)`, `@isset(a)`, `@empty(a)` | `if (!(c)):`, `if (isset(a)):`, `if (empty(a)):` |
//! | `@foreach(a as b)` / `@forelse(a as b) ... @empty ... @endforelse` | `foreach (a as b): ... endforeach;` / the same, then `if (...): ... endif;` |
//! | `@for(...)`, `@while(c)` | `for (...):`, `while (c):` |
//! | `@switch(x)` `@case(v)` `@break` `@default` | `switch (x):` `case (v):` `break;` `default:` |
//! | `@break`, `@continue`, `@break(c)`, `@break(2)` | `break;`, `continue;`, `if (c) break;`, `break 2;` |
//! | `@use('A\B', 'C')` | `use A\B as C;` |
//! | `@unset(a)` | `unset(a);` |
//! | `@inject('name', s)` | a marker defining `$name`, then a call of the directive: `(s)` |
//! | any other directive `BladeCompiler` compiles, `@name(args)` | a call of the directive: `(args)` |
//! | a conditional directive Blade decides at run time (`@auth`, `@can(...)`, `@error(...)`, `@once`, `@pushIf(...)`) | `if` with that call as its condition, or none |
//! | `<x-component :attr="e" :$name>` | `e;`, `$name;` |
//!
//! The call a directive is has no `function` child: Blade's runtime is not a PHP function the
//! template names. A condition Blade decides from its runtime alone (`@auth`, `@once`) is no node
//! at all. Blade's implicit variables are empty statements over the directive that defines them
//! (`blade::marker`): `@foreach` and `@forelse` define `$loop` (Blade assigns it before every
//! iteration and after the loop), `@error` `$message` and `@session`/`@context` `$value` inside
//! their body, `@inject('name', ...)` `$name`; `@props` and `@aware` define `$attributes` and any
//! variable; `@include` and its variants, `@each`, `@extends` and `@component` pass every variable
//! on.
//!
//! What Blade does not compile is not read: `@verbatim` blocks, `{{-- --}}` comments, escaped
//! `@{{ }}` echoes, `@@` directives and their arguments, and the arguments of directives
//! `BladeCompiler` 13.34.0 has no `compile*` method for (an application's `Blade::directive` or
//! `Blade::if`, a package's such as Livewire's), which Forte's default registry reads as text
//! (`<div>@icon('x')</div>`). Such a directive with arguments is a marker that may read and assign
//! every variable, since the application compiles it to PHP the template does not show; the body
//! of a pair of them (`@name(...) ... @endname`) is read as the template's.
//! A directive's arguments run to the `)` Blade finds with PHP's tokenizer, so one in a comment,
//! string or heredoc does not end them.
//!
//! The view walks the template's Forte tree (`blade::forte::tree`): Forte pairs a block's
//! directives (`directive_block`: its start, branches and end, each directive's body its children)
//! and lexes Blade wherever Blade's compiler reads it, `<script>` and `<style>` text and attribute
//! names and values included. What Forte leaves as text (a directive its registry does not know,
//! an HTML comment's text) is read as Blade's compiler reads it (`loose`). A control directive
//! Forte leaves unpaired (in an attribute's value) is not read: the view reads no half of a
//! structure.

use crate::code_rule_dsl::blade::forte::ParseError;
use crate::code_rule_dsl::blade::forte::tree::{BladeNode, BladeTree, parse_tree};
use crate::code_rule_dsl::embed::{Program, segments::SegmentMap};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::php_literal::literal_end;
use ast_grep_core::source::SgNode;
use ast_grep_language::LanguageExt;
use std::ops::Range;
#[cfg(test)]
use tree_sitter::Node as TsNode;
use tree_sitter::{Parser, Tree};

/// The embedded PHP program and its map back to the template.
pub(crate) struct View {
    text: String,
    segments: SegmentMap,
    tree: Tree,
    /// The template's syntax errors, sorted: Forte's, but where Forte read a directive's arguments
    /// Blade does not (`Emitter::stray_arguments`), those of the template Blade reads there
    /// instead.
    syntax_errors: Vec<Range<usize>>,
}

/// Directives Blade compiles to control flow (a condition, a loop, a `@php` block, a guard):
/// read only as the grammar's structures, never as text.
const CONTROL: &[&str] = &[
    "if",
    "elseif",
    "unless",
    "isset",
    "empty",
    "auth",
    "elseAuth",
    "guest",
    "elseGuest",
    "can",
    "cannot",
    "canany",
    "elsecan",
    "elsecannot",
    "elsecanany",
    "env",
    "production",
    "error",
    "session",
    "context",
    "hasSection",
    "sectionMissing",
    "switch",
    "case",
    "break",
    "continue",
    "foreach",
    "forelse",
    "for",
    "while",
    "php",
    "once",
    "pushOnce",
    "prependOnce",
    "pushIf",
    "elsePushIf",
];

/// Directives that pass every variable of the template on (`get_defined_vars()`).
const PASSES_VARIABLES: &[&str] = &[
    "include",
    "includeIf",
    "includeWhen",
    "includeUnless",
    "includeFirst",
    "each",
    "extends",
    "extendsFirst",
    "component",
];

/// The directives `BladeCompiler` 13.34.0 compiles: one per `compile*` method, which it finds by
/// name as PHP does, whatever the case. Any other `@name(...)` is the application's (a
/// `Blade::directive`, a package's such as Livewire's) or text; the template does not say which
/// PHP, if any, it compiles to, so the view reads none.
const DIRECTIVES: &[&str] = &[
    "append",
    "auth",
    "aware",
    "bool",
    "break",
    "can",
    "canany",
    "cannot",
    "case",
    "checked",
    "choice",
    "class",
    "classComponentOpening",
    "comments",
    "component",
    "componentFirst",
    "componentTags",
    "context",
    "continue",
    "csrf",
    "dd",
    "default",
    "disabled",
    "dump",
    "each",
    "echos",
    "else",
    "elseAuth",
    "elseGuest",
    "elsePush",
    "elsePushIf",
    "elsecan",
    "elsecanany",
    "elsecannot",
    "elseif",
    "empty",
    "endAuth",
    "endComponent",
    "endComponentClass",
    "endComponentFirst",
    "endEmpty",
    "endEnv",
    "endGuest",
    "endIsset",
    "endOnce",
    "endProduction",
    "endPushIf",
    "endSlot",
    "endSwitch",
    "endcan",
    "endcanany",
    "endcannot",
    "endcontext",
    "enderror",
    "endfor",
    "endforeach",
    "endforelse",
    "endfragment",
    "endif",
    "endlang",
    "endprepend",
    "endprependOnce",
    "endpush",
    "endpushOnce",
    "endsection",
    "endsession",
    "endunless",
    "endwhile",
    "env",
    "error",
    "escapedEchos",
    "extends",
    "extendsFirst",
    "extensions",
    "fonts",
    "for",
    "foreach",
    "forelse",
    "fragment",
    "guest",
    "hasSection",
    "hasStack",
    "if",
    "include",
    "includeFirst",
    "includeIf",
    "includeIsolated",
    "includeUnless",
    "includeWhen",
    "inject",
    "isset",
    "js",
    "json",
    "lang",
    "method",
    "once",
    "overwrite",
    "parent",
    "php",
    "prepend",
    "prependOnce",
    "production",
    "props",
    "push",
    "pushIf",
    "pushOnce",
    "rawEchos",
    "readonly",
    "regularEchos",
    "required",
    "section",
    "sectionMissing",
    "selected",
    "session",
    "show",
    "slot",
    "stack",
    "statement",
    "statements",
    "stop",
    "string",
    "style",
    "switch",
    "unless",
    "unset",
    "use",
    "vite",
    "viteReactRefresh",
    "while",
    "yield",
];

/// Whether Blade compiles `@name`.
pub(crate) fn compiles(name: &str) -> bool {
    DIRECTIVES
        .iter()
        .any(|directive| directive.eq_ignore_ascii_case(name))
}

/// A template as Blade reads it (`View::read`).
pub(crate) struct Read {
    /// Its syntax errors: Forte's, except in the arguments Forte read after a line break, which
    /// Blade reads as text: there, that text's (`syntax_errors`).
    pub(crate) syntax_errors: Vec<Range<usize>>,
    /// The program it embeds; `None` where it embeds none.
    pub(crate) view: Option<View>,
}

/// Why a template could not be read (`View::read`).
#[derive(Debug)]
pub(crate) enum ReadError {
    /// The budget interrupted it (polled before each sub-template is parsed, `Emitter::template`).
    Interrupted,
    /// Forte stopped at a limit on a template the template holds: nothing about its syntax.
    Parser(ParseError),
}

impl View {
    /// The template `tree` as Blade reads it: its syntax errors, and the program it embeds.
    pub(crate) fn read(
        tree: &BladeTree,
        template: &str,
        budget: &dyn Fn() -> bool,
    ) -> Result<Read, ReadError> {
        let mut emitter = Emitter {
            template,
            out: String::new(),
            segments: SegmentMap::new(),
            loosened: 0,
            consumed: 0,
            loops: 0,
            switches: 0,
            origin: 0,
            depth: 0,
            budget,
            interrupted: false,
            failed: None,
            strays: Vec::new(),
            reporting: true,
        };
        emitter.body(tree.root());
        if let Some(error) = emitter.failed {
            return Err(ReadError::Parser(error));
        }
        if emitter.interrupted {
            return Err(ReadError::Interrupted);
        }
        let syntax_errors = syntax_errors(tree.error_ranges(), &emitter.strays);
        if emitter.segments.is_empty() {
            return Ok(Read {
                syntax_errors,
                view: None,
            });
        }
        let mut parser = Parser::new();
        let program = parser
            .set_language(&GrammarDialect::Php.get_ts_language())
            .ok()
            .and_then(|()| parser.parse(&emitter.out, None));
        let view = program.map(|program| {
            let mut segments = emitter.segments;
            segments.index();
            Self {
                text: emitter.out,
                segments,
                tree: program,
                syntax_errors: syntax_errors.clone(),
            }
        });
        Ok(Read {
            syntax_errors,
            view,
        })
    }

    /// The view as the template's embedded program (`compiled-view`): PHP, read through the
    /// template's syntax errors.
    pub(crate) fn into_program(self) -> Program {
        Program {
            dialect: GrammarDialect::Php,
            tree: self.tree,
            text: self.text,
            segments: self.segments,
            host_errors: self.syntax_errors,
        }
    }

    #[cfg(test)]
    pub(crate) fn root(&self) -> TsNode<'_> {
        self.tree.root_node()
    }

    /// The program's text.
    #[cfg(test)]
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// Where the program's text stands in the template.
    #[cfg(test)]
    pub(crate) fn segments(&self) -> &SegmentMap {
        &self.segments
    }

    /// The template's syntax errors as Blade reads them.
    #[cfg(test)]
    pub(crate) fn syntax_errors(&self) -> &[Range<usize>] {
        &self.syntax_errors
    }

    /// The template ranges the program copies byte for byte, in program order.
    #[cfg(test)]
    pub(crate) fn copied(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.segments.copied()
    }
}

struct Emitter<'t> {
    template: &'t str,
    out: String,
    segments: SegmentMap,
    /// Where the text `loose` has read ends: text is read once.
    loosened: usize,
    /// Where the last directive's arguments, as Blade reads them, end: a node Forte placed before
    /// it (Forte ended the arguments at an earlier `)`) is part of them.
    consumed: usize,
    /// The loops and switches being read: `@break` and `@continue` in them are PHP's.
    loops: usize,
    switches: usize,
    /// Where the tree being read starts in the template: a sub-template's (`Emitter::template`).
    origin: usize,
    /// The sub-templates being read, one inside the other (`Emitter::template`).
    depth: usize,
    /// Polled before each sub-template is parsed; once it holds, `interrupted` is set and nothing
    /// more is read.
    budget: &'t dyn Fn() -> bool,
    interrupted: bool,
    /// Forte stopped at a limit on a sub-template; once set, nothing more is read.
    failed: Option<ParseError>,
    /// The `stray_arguments` read as the template, each with that template's syntax errors.
    strays: Vec<(Range<usize>, Vec<Range<usize>>)>,
    /// Whether the tree being read reports its syntax errors: the template's, and a stray
    /// arguments' template read where they are reported; not a `template` Forte read as text.
    reporting: bool,
}

/// A template's syntax errors as Blade reads it: Forte's (`forte`) and those of each stray
/// arguments' template (`strays`), but none inside a stray read deeper than where it was found
/// (Forte's errors there are about arguments Blade does not read). Sorted, without repeats.
fn syntax_errors(
    forte: &[Range<usize>],
    strays: &[(Range<usize>, Vec<Range<usize>>)],
) -> Vec<Range<usize>> {
    let inside = |error: &Range<usize>, range: &Range<usize>| {
        range.start <= error.start && error.end <= range.end
    };
    let read_over = |error: &Range<usize>, found_in: Option<usize>| {
        strays.iter().enumerate().any(|(index, (range, _))| {
            Some(index) != found_in
                && found_in.is_none_or(|outer| inside(range, &strays[outer].0))
                && inside(error, range)
        })
    };
    let mut errors = forte
        .iter()
        .filter(|error| !read_over(error, None))
        .cloned()
        .chain(strays.iter().enumerate().flat_map(|(index, (_, errors))| {
            errors
                .iter()
                .filter(move |error| !read_over(error, Some(index)))
                .cloned()
        }))
        .collect::<Vec<_>>();
    errors.sort_by_key(|range| (range.start, range.end));
    errors.dedup();
    errors
}

/// How deeply sub-templates may nest (`Emitter::template`). Past it, a range is read as text
/// (`Emitter::loose`): each level re-lexes its range, so the bound keeps the work linear in the
/// template and the recursion off the stack. Real templates nest two at most (a `@php` block in a
/// `<script type="text/x-template">`).
const TEMPLATE_DEPTH: usize = 4;

/// An empty template range at `at`.
fn at(at: usize) -> Range<usize> {
    at..at
}

/// One parenthesis around a directive's arguments, by its template byte.
#[derive(Clone, Copy)]
struct Paren(usize);

impl Paren {
    fn start_byte(self) -> usize {
        self.0
    }

    fn end_byte(self) -> usize {
        self.0 + 1
    }

    fn byte_range(self) -> Range<usize> {
        self.0..self.0 + 1
    }
}

/// The arguments Forte gave a directive, by template byte (`Emitter::forte_arguments`).
struct ForteArguments {
    /// Where the directive's name ends.
    name: usize,
    /// Forte's `(`.
    open: usize,
    /// Forte's `)`, if it found one.
    close: Option<usize>,
    /// Where Forte's arguments end.
    end: usize,
}

/// A condition Blade decides at run time from what its directive passes its runtime.
enum Condition {
    /// `(c)`: the template's own condition.
    Plain,
    /// `(!(c))`.
    Not,
    /// `(isset(a))`, `(empty(a))`: the construct names the directive's own bytes.
    Construct,
    /// A call of the directive (its `@name` at these template bytes) with its arguments, or
    /// nothing where it has none.
    Runtime(Range<usize>),
}

/// How Blade compiles a block (`directive_block`), by its start directive's name.
enum Block {
    /// `if` with a condition, `elseif`/`else` branches: `@if`, `@auth`, `@can`, `@pushIf`, ...
    Conditional,
    /// `@foreach`, `@forelse`, `@for`, `@while`.
    Loop,
    Switch,
    /// A body Blade's runtime may skip: `@once`, `@pushOnce`, `@prependOnce`.
    Guarded,
    /// Directives around a body Blade compiles as the template's (`@section`, `@push`, ...).
    Plain,
}

impl Block {
    fn of(name: &str) -> Self {
        const CONDITIONAL: &[&str] = &[
            "if",
            "unless",
            "isset",
            "empty",
            "auth",
            "guest",
            "can",
            "cannot",
            "canany",
            "env",
            "production",
            "error",
            "session",
            "context",
            "hasSection",
            "sectionMissing",
            "hasStack",
            "pushIf",
        ];
        let is = |names: &[&str]| names.iter().any(|known| known.eq_ignore_ascii_case(name));
        if is(CONDITIONAL) {
            Self::Conditional
        } else if is(&["foreach", "forelse", "for", "while"]) {
            Self::Loop
        } else if name.eq_ignore_ascii_case("switch") {
            Self::Switch
        } else if is(&["once", "pushOnce", "prependOnce"]) {
            Self::Guarded
        } else {
            Self::Plain
        }
    }
}

/// A directive's `@name` leaf, if it has one (`@endphp` closing a `@php` block has none).
fn name_leaf<'r>(directive: &BladeNode<'r>) -> Option<BladeNode<'r>> {
    directive.field("name")
}

/// The directives of a block other than its start and end: its branches, in order.
fn branches<'r>(block: &BladeNode<'r>) -> Vec<BladeNode<'r>> {
    block
        .children()
        .filter(|child| child.field_name().is_none() && child.kind() == "directive")
        .collect()
}

/// One directive of a structure and what it governs: its own children (a block Forte paired), or
/// the siblings up to the structure's next directive (one Forte left unpaired, in an attribute's
/// value).
struct Arm<'r> {
    directive: BladeNode<'r>,
    siblings: Option<Vec<BladeNode<'r>>>,
}

/// A control structure: its start, its branches (`@else...`, `@empty`, ...) and where it closes.
struct Parts<'r> {
    start: Arm<'r>,
    branches: Vec<Arm<'r>>,
    /// Its end directive, or where it ends unclosed.
    close: Range<usize>,
}

/// A directive's lowercase name, without its `@`.
fn directive_name(directive: &BladeNode<'_>) -> Option<String> {
    let leaf = name_leaf(directive)?;
    Some(String::from_utf8_lossy(leaf.bytes().get(1..)?).to_ascii_lowercase())
}

/// Whether `name` is a directive Blade compiles to the start of a control structure.
fn opens(name: &str) -> bool {
    !matches!(Block::of(name), Block::Plain) && compiles(name)
}

/// Whether `name` closes the structure `opener` opened (`@endif` an `@if` or `@hasSection`).
fn closes(opener: &str, name: &str) -> bool {
    match opener {
        "if" | "hassection" | "sectionmissing" | "hasstack" => name == "endif",
        _ => name.strip_prefix("end") == Some(opener),
    }
}

/// Whether `name` is a branch of the structure `opener` opened.
fn branches_of(opener: &str, name: &str) -> bool {
    match Block::of(opener) {
        Block::Conditional if opener == "pushif" => name == "elsepushif",
        Block::Conditional => {
            name == "else" || name.starts_with("elseif") || {
                matches!(
                    name,
                    "elseauth" | "elseguest" | "elsecan" | "elsecannot" | "elsecanany"
                )
            }
        }
        Block::Loop => opener == "forelse" && name == "empty",
        _ => false,
    }
}

/// The control structures Forte left unpaired among `nodes` (an attribute's value holds its
/// directives side by side), as Blade's compiler pairs them: each opener with its branches and its
/// end, by index. An opener left unclosed, or closed past by an outer structure's end, is none.
fn flat_structures(nodes: &[BladeNode<'_>]) -> Vec<(usize, Vec<usize>, usize)> {
    let mut open: Vec<(usize, String, Vec<usize>)> = Vec::new();
    let mut found = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        if node.kind() != "directive" {
            continue;
        }
        let Some(name) = directive_name(node) else {
            continue;
        };
        if opens(&name) {
            open.push((index, name, Vec::new()));
        } else if let Some(at) = open
            .iter()
            .rposition(|(_, opener, _)| closes(opener, &name))
        {
            let (start, _, branches) = open.drain(at..).next().expect("an open structure");
            found.push((start, branches, index));
        } else if let Some((_, opener, branches)) = open.last_mut()
            && branches_of(opener, &name)
        {
            branches.push(index);
        }
    }
    found.sort_unstable_by_key(|(start, _, _)| *start);
    found
}

/// Whether a tag is a component Blade compiles (`<x-...>`, `<x:...>`).
fn is_component(tag: &BladeNode<'_>) -> bool {
    tag.field("name").is_some_and(|name| {
        let name = name.bytes();
        name.starts_with(b"x-") || name.starts_with(b"x:")
    })
}

impl<'t> Emitter<'t> {
    fn text(&self, range: Range<usize>) -> &'t str {
        &self.template[range]
    }

    fn push(&mut self, text: &str, anchor: Range<usize>, copied: bool, omit: bool) {
        let at = self.out.len();
        self.out.push_str(text);
        self.segments.push(at, self.out.len(), anchor, copied, omit);
    }

    /// Writes PHP Blade wraps the template's PHP in, standing for the template bytes `anchor`.
    fn emit(&mut self, text: &str, anchor: Range<usize>) {
        self.push(text, anchor, false, false);
    }

    /// Writes PHP the grammar needs but the view does not keep (a callee or condition Blade's
    /// runtime supplies).
    fn placeholder(&mut self, text: &str, anchor: Range<usize>) {
        self.push(text, anchor, false, true);
    }

    /// Copies template bytes as PHP.
    fn copy(&mut self, range: Range<usize>) {
        if range.is_empty() {
            return;
        }
        let text = &self.template[range.clone()];
        self.push(text, range, true, false);
    }

    /// An implicit-variable marker over a directive (`blade::marker`).
    fn marker(&mut self, anchor: Range<usize>) {
        self.emit(";", anchor);
    }

    /// An application's directive with arguments (`@name(...)` Blade itself does not compile) at
    /// template bytes `directive`: its arguments are no PHP the template shows, but what the
    /// application compiles it to may read or assign any variable, so a marker over the directive
    /// does (`blade::marker`).
    fn application_directive(&mut self, directive: Range<usize>) {
        let text = &self.template[directive.clone()];
        let start = directive.start + (text.len() - text.trim_start().len());
        self.marker(start..directive.end);
    }

    /// A node's children in order.
    fn body(&mut self, node: BladeNode<'_>) {
        let children = node.children().collect::<Vec<_>>();
        self.sequence(&children);
    }

    /// A directive's body: its children after its name and arguments, after what Forte read as
    /// its arguments where Blade reads none (`read_stray`).
    fn directive_body(&mut self, directive: BladeNode<'_>) {
        self.read_stray(&directive);
        let children = directive
            .children()
            .filter(|child| child.is_named() && !matches!(&*child.kind(), "directive_name" | "php"))
            .collect::<Vec<_>>();
        self.sequence(&children);
    }

    /// Sibling nodes in order, a control structure Forte left unpaired among them read as Blade
    /// pairs it (`flat_structures`).
    fn sequence(&mut self, nodes: &[BladeNode<'_>]) {
        let structures = flat_structures(nodes);
        let mut structures = structures.into_iter().peekable();
        let mut index = 0;
        while index < nodes.len() {
            let Some((start, branches, end)) = structures.next_if(|(start, _, _)| *start == index)
            else {
                self.node(nodes[index]);
                index += 1;
                continue;
            };
            // Structures nested in this one are read as its arms' siblings.
            while structures.next_if(|(nested, _, _)| *nested < end).is_some() {}
            let mut bounds = branches.clone();
            bounds.push(end);
            let arm = |at: usize, until: usize| Arm {
                directive: nodes[at],
                siblings: Some(nodes[at + 1..until].to_vec()),
            };
            let parts = Parts {
                start: arm(start, bounds[0]),
                branches: branches
                    .iter()
                    .zip(&bounds[1..])
                    .map(|(&at, &until)| arm(at, until))
                    .collect(),
                close: self.closing(&nodes[end]),
            };
            if self.span(&nodes[start]).start >= self.consumed {
                self.structure(parts);
                self.read_stray(&nodes[end]);
            }
            index = end + 1;
        }
    }

    /// What an arm governs: its directive's children, or its siblings.
    fn arm_body(&mut self, arm: &Arm<'_>) {
        match &arm.siblings {
            None => self.directive_body(arm.directive),
            Some(siblings) => {
                self.directive_body(arm.directive);
                self.sequence(siblings);
            }
        }
    }

    /// A directive outside the structure's own syntax (`directive`), then what its arm governs.
    fn arm(&mut self, arm: &Arm<'_>) {
        self.directive(arm.directive);
        if let Some(siblings) = &arm.siblings {
            self.sequence(siblings);
        }
    }

    /// The template bytes a node of the tree being read spans, widened to whole characters:
    /// Forte lexes bytes and can end a text node inside a multi-byte character (`<a é@b)` splits
    /// `é` in an attribute name), and the view slices the template as `str`. The bytes it widens
    /// over are no syntax, which is ASCII.
    fn span(&self, node: &BladeNode<'_>) -> Range<usize> {
        let range = node.range();
        self.template.floor_char_boundary(range.start + self.origin)
            ..self.template.ceil_char_boundary(range.end + self.origin)
    }

    /// Reads `range` of the template as a template of its own: what Forte left unread there
    /// (text in `<script>` and `<style>`, which Blade's compiler reads as any text; what Forte's
    /// recovery swallowed into a `@php` block or a directive's arguments). Its syntax errors are
    /// not the template's: Forte reported what it found there.
    fn template(&mut self, range: Range<usize>) {
        let reporting = std::mem::replace(&mut self.reporting, false);
        self.sub_template(range);
        self.reporting = reporting;
    }

    /// Reads `range` as a template of its own (`template`); its syntax errors, by template byte,
    /// `None` where it was not parsed. Past `TEMPLATE_DEPTH` nested sub-templates, the range is
    /// read as text.
    fn sub_template(&mut self, range: Range<usize>) -> Option<Vec<Range<usize>>> {
        if range.is_empty() || self.interrupted || self.failed.is_some() {
            return None;
        }
        if (self.budget)() {
            self.interrupted = true;
            return None;
        }
        if self.depth >= TEMPLATE_DEPTH {
            self.loose(range);
            return None;
        }
        let tree = match parse_tree(&self.template.as_bytes()[range.clone()]) {
            Ok(tree) => tree,
            Err(error) => {
                self.failed = Some(error);
                return None;
            }
        };
        let origin = std::mem::replace(&mut self.origin, range.start);
        self.depth += 1;
        self.body(tree.root());
        self.depth -= 1;
        self.origin = origin;
        Some(
            tree.error_ranges()
                .iter()
                .map(|error| error.start + range.start..error.end + range.start)
                .collect(),
        )
    }

    fn node(&mut self, node: BladeNode<'_>) {
        let range = self.span(&node);
        // Text is read from `loosened` on; any other node Blade read as a directive's arguments
        // is not read again.
        if range.start < self.consumed && node.kind() != "text" {
            return;
        }
        match &*node.kind() {
            "echo" | "raw_echo" | "triple_echo" => self.echo_node(node),
            "php_block" => self.php_block(node),
            "php_tag" => self.php_tag(node),
            "directive" => self.directive(node),
            "directive_block" => self.block(node),
            "element" | "component" | "slot" => self.tag(node),
            // Blade compiles what text holds that Forte leaves as text: an application's
            // directive, an echo in an HTML comment.
            "text" | "comment_text" => self.loose(range),
            "verbatim" | "blade_comment" | "escape" | "doctype" => {}
            _ => self.body(node),
        }
    }

    /// `{{ e }}`, `{!! e !!}`: an echo Blade compiles once it is closed.
    fn echo_node(&mut self, node: BladeNode<'_>) {
        let children = node.children().collect::<Vec<_>>();
        let (Some(open), Some(close)) = (children.first(), children.last()) else {
            return;
        };
        let closed = matches!(&*close.kind(), "}}" | "!!}" | "}}}");
        let open = self.span(open);
        if !closed {
            return;
        }
        // `@{{ ... }}` is Blade's escape for a literal echo, which Forte reads as an echo in a
        // `<script>`'s raw text: Blade writes it as text, after compiling the directives it holds.
        if self.template[..open.start].ends_with('@') {
            return self.loose(open.end..self.span(&node).end);
        }
        if let Some(php) = node.field("expression") {
            let (php, close) = (self.span(&php), self.span(close));
            self.echo(open, php, close);
        }
    }

    /// `@php ... @endphp`. Blade's `@php` runs to the first `@endphp` after it wherever it stands,
    /// which Forte's lexer misses in a `<script>`'s raw text (it then reads the rest of the
    /// template as the block): there the view ends the block at that `@endphp` and reads what
    /// follows as the template.
    fn php_block(&mut self, node: BladeNode<'_>) {
        let range = self.span(&node);
        let closed = node
            .children()
            .last()
            .filter(|last| last.kind() == "@endphp");
        if let Some(close) = closed {
            if let Some(php) = node.field("code") {
                let (php, close) = (self.span(&php), self.span(&close));
                self.statements(php, close);
            }
            return;
        }
        let open = range.start + "@php".len();
        let Some(end) = self.template[open..range.end].find("@endphp") else {
            if let Some(php) = node.field("code") {
                let php = self.span(&php);
                self.statements(php, at(range.end));
            }
            return;
        };
        let end = open + end;
        let text = &self.template[open..end];
        let php = open + (text.len() - text.trim_start().len())
            ..end - (text.len() - text.trim_end().len());
        if !php.is_empty() {
            self.statements(php, end..end + "@endphp".len());
        }
        self.loosened = self.loosened.max(end + "@endphp".len());
        self.template(end + "@endphp".len()..range.end);
    }

    /// `<?php ... ?>`, `<?= e ?>`. A tag's statements are all its bytes between `<?php` and
    /// `?>`, as PHP reads them (a `#` comment runs to the `?>`).
    fn php_tag(&mut self, node: BladeNode<'_>) {
        let Some(php) = node.field("code") else {
            return;
        };
        let php = self.span(&php);
        let children = node.children().collect::<Vec<_>>();
        let Some(open) = children.first().map(|first| self.span(first)) else {
            return;
        };
        let close = children
            .last()
            .filter(|last| last.kind() == "?>")
            .map_or(at(self.span(&node).end), |last| self.span(last));
        if self.text(open.clone()) == "<?=" {
            self.echo(open, php, close);
        } else {
            self.statements(open.end..close.start, close);
        }
    }

    fn echo(&mut self, open: Range<usize>, php: Range<usize>, close: Range<usize>) {
        if self.template[php.clone()].trim().is_empty() {
            return;
        }
        self.emit("echo ", open);
        self.copy(php);
        self.emit("\n;", close);
    }

    /// Statements, then the `;` and line end PHP's `?>` stands for: the `;` where the template
    /// leaves the last statement open, the line end always, as `?>` also ends a line comment.
    fn statements(&mut self, php: Range<usize>, close: Range<usize>) {
        let ended = ends_statement(&self.template[php.clone()]);
        self.copy(php);
        self.emit(if ended { "\n" } else { "\n;" }, close);
    }

    /// Where Forte's arguments of a directive open (its `(`), close (its `)`, if any) and end,
    /// and where the directive's name ends.
    fn forte_arguments(&self, directive: &BladeNode<'_>) -> Option<ForteArguments> {
        let name = self.span(&name_leaf(directive)?).end;
        let (mut open, mut close, mut end) = (None, None, 0);
        for child in directive.children() {
            match &*child.kind() {
                "(" => open = Some(self.span(&child).start),
                ")" => close = Some(self.span(&child).start),
                "php" if child.field_name() == Some("arguments") => {}
                _ => continue,
            }
            end = end.max(self.span(&child).end);
        }
        Some(ForteArguments {
            name,
            open: open?,
            close,
            end,
        })
    }

    /// Whether Blade reads the `(` at `open` as the arguments of the directive whose name ends at
    /// `name`: only spaces and tabs between them (`[ \t]*` in `compileStatements`), where Forte's
    /// lexer also skips line breaks.
    fn blade_arguments(&self, name: usize, open: usize) -> bool {
        self.template.as_bytes()[name..open]
            .iter()
            .all(|byte| matches!(byte, b' ' | b'\t'))
    }

    /// The `(` and `)` around a directive's arguments, as Blade reads them: from the `(` Forte
    /// found after its name (and any spaces or tabs) to the `)` that balances it (`balanced`),
    /// wherever Forte ended them (`@section('a', __('b'))`); without one, to Forte's `)`. Then
    /// where Forte's arguments end.
    fn parens(&self, directive: &BladeNode<'_>) -> Option<(Paren, Paren, usize)> {
        let arguments = self.forte_arguments(directive)?;
        if !self.blade_arguments(arguments.name, arguments.open) {
            return None;
        }
        let open = arguments.open;
        let close = match balanced(&self.template[open..]) {
            Some(close) => open + close,
            None => arguments.close?,
        };
        Some((Paren(open), Paren(close), arguments.end))
    }

    /// Forte's arguments of a directive Blade reads without them (a line break before the `(`,
    /// `@endforeach\n(...)`): template text after the directive, through the whitespace after
    /// them up to the directive's next child or end (unclosed arguments end before trailing
    /// whitespace, and Forte reports them at the end of the template).
    fn stray_arguments(&self, directive: &BladeNode<'_>) -> Option<Range<usize>> {
        let arguments = self.forte_arguments(directive)?;
        if self.blade_arguments(arguments.name, arguments.open) {
            return None;
        }
        let next = directive
            .children()
            .map(|child| self.span(&child).start)
            .filter(|start| *start >= arguments.end)
            .min()
            .unwrap_or_else(|| self.span(directive).end);
        let end = if self.template.as_bytes()[arguments.end..next]
            .iter()
            .all(u8::is_ascii_whitespace)
        {
            next
        } else {
            arguments.end
        };
        Some(arguments.open..end)
    }

    /// Reads a directive's `stray_arguments` as the template.
    fn read_stray(&mut self, directive: &BladeNode<'_>) {
        if let Some(stray) = self.stray_arguments(directive) {
            let end = stray.end;
            // Where the template's errors are reported (`reporting`), the errors of the template
            // Blade reads here replace Forte's (`syntax_errors`).
            if let Some(errors) = self.sub_template(stray.clone())
                && self.reporting
            {
                self.strays.push((stray, errors));
            }
            self.loosened = self.loosened.max(end);
        }
    }

    /// The bytes a structure's closing directive spans as Blade reads it: without its
    /// `stray_arguments`.
    fn closing(&self, end: &BladeNode<'_>) -> Range<usize> {
        let span = self.span(end);
        match (self.stray_arguments(end), name_leaf(end)) {
            (Some(_), Some(name)) => span.start..self.span(&name).end,
            _ => span,
        }
    }

    /// Marks a directive's arguments read through `close`: Forte's nodes before it are part of
    /// them. What Forte read as arguments past it (`forte_end`) is the template's.
    fn consume(&mut self, close: Paren, forte_end: usize) {
        self.consumed = self.consumed.max(close.end_byte());
        self.loosened = self.loosened.max(close.end_byte());
        if forte_end > close.end_byte() {
            self.template(close.end_byte()..forte_end);
            self.loosened = self.loosened.max(forte_end);
        }
    }

    /// A directive outside the blocks Blade compiles to control flow: one with arguments, `@break`
    /// and `@continue` in a loop, `@case` and `@default` in a switch; then its body.
    fn directive(&mut self, directive: BladeNode<'_>) {
        let Some(name_leaf) = name_leaf(&directive) else {
            return self.directive_body(directive);
        };
        let name_range = self.span(&name_leaf);
        let name = &self.template[name_range.start + 1..name_range.end];
        let head = self.parens(&directive);
        let parens = head.map(|(open, close, _)| (open, close));
        let is = |known: &str| name.eq_ignore_ascii_case(known);
        if self.loops > 0 && (is("break") || is("continue")) {
            self.loop_operator(name_range, parens);
        } else if self.switches > 0 && is("case") {
            if let Some((open, close)) = parens {
                self.emit("case", name_range);
                self.emit(" (", open.byte_range());
                self.copy(open.end_byte()..close.start_byte());
                self.emit(")", close.byte_range());
                self.emit(":\n", at(close.end_byte()));
            }
        } else if self.switches > 0 && is("default") {
            self.emit("default", name_range.clone());
            self.emit(":\n", at(name_range.end));
        } else if let Some((open, close)) = parens {
            if is("php") {
                // `@php (...) @endphp` is a block whose PHP is all between them, its
                // parentheses too (Blade's `storePhpBlocks`).
                let block = self.template[close.end_byte()..]
                    .trim_start()
                    .starts_with("@endphp");
                if block {
                    self.copy(open.start_byte()..close.end_byte());
                } else {
                    self.copy(open.end_byte()..close.start_byte());
                }
                self.emit("\n;", at(close.start_byte()));
            } else if !CONTROL.iter().any(|control| is(control)) {
                self.directive_at(name_range, open, close);
            }
        }
        if let Some((_, close, end)) = head {
            self.consume(close, end);
        }
        self.directive_body(directive);
    }

    /// A block Forte paired (`directive_block`), compiled as its start directive says.
    fn block(&mut self, block: BladeNode<'_>) {
        let Some(start) = block.field("start") else {
            return self.body(block);
        };
        let Some(name) = directive_name(&start) else {
            return self.body(block);
        };
        let parts = Parts {
            start: Arm {
                directive: start,
                siblings: None,
            },
            branches: branches(&block)
                .into_iter()
                .map(|directive| Arm {
                    directive,
                    siblings: None,
                })
                .collect(),
            close: block
                .field("end")
                .map_or(at(self.span(&block).end), |end| self.closing(&end)),
        };
        if !compiles(&name) {
            // A pair Blade does not compile (an application's `Blade::if`, a package's) is text
            // around a body Blade compiles as the template's.
            let name_range = self.span(&name_leaf(&start).expect("a named directive"));
            if let Some((_, close, end)) = self.parens(&start) {
                self.application_directive(name_range);
                self.consume(close, end);
            }
            self.directive_body(start);
            for branch in &parts.branches {
                self.arm(branch);
            }
            if let Some(end) = block.field("end") {
                self.read_stray(&end);
            }
            return;
        }
        if let Block::Plain = Block::of(&name) {
            self.directive(start);
            for branch in &parts.branches {
                self.arm(branch);
            }
            if let Some(end) = block.field("end") {
                self.directive(end);
            }
            return;
        }
        self.structure(parts);
        if let Some(end) = block.field("end") {
            self.read_stray(&end);
        }
    }

    /// A control structure Blade compiles, as its start directive says.
    fn structure(&mut self, parts: Parts<'_>) {
        let start = parts.start.directive;
        let (Some(name), Some(leaf)) = (directive_name(&start), name_leaf(&start)) else {
            return;
        };
        let name_range = self.span(&leaf);
        match Block::of(&name) {
            Block::Conditional => self.conditional(parts, name_range, &name),
            Block::Loop => self.repeat(parts, name_range, &name),
            Block::Switch => self.switch(parts, name_range),
            Block::Guarded => self.guarded(parts, name_range),
            Block::Plain => {}
        }
    }

    /// A condition from a directive's arguments, after `if`/`elseif` was written over
    /// `directive` (its `@name`); then the arguments are read.
    fn condition(
        &mut self,
        directive: Range<usize>,
        head: Option<(Paren, Paren, usize)>,
        condition: Condition,
    ) {
        match (condition, head) {
            (Condition::Plain, Some((open, close, _))) => {
                self.emit("(", open.byte_range());
                self.copy(open.end_byte()..close.start_byte());
                self.emit(")", close.byte_range());
            }
            (Condition::Not, Some((open, close, _))) => {
                self.emit("(", open.byte_range());
                self.emit("!(", at(open.end_byte()));
                self.copy(open.end_byte()..close.start_byte());
                self.emit(")", at(close.start_byte()));
                self.emit(")", close.byte_range());
            }
            (Condition::Construct, Some((open, close, _))) => {
                // `@isset(` is `if (isset(`: the name is the directive's own bytes.
                self.emit("(", at(directive.start + 1));
                self.emit(
                    &self.template[directive.start + 1..directive.end],
                    directive.start + 1..directive.end,
                );
                self.emit("(", open.byte_range());
                self.copy(open.end_byte()..close.start_byte());
                self.emit(")", close.byte_range());
                self.emit(")", at(close.end_byte()));
            }
            (Condition::Runtime(name), Some((open, close, _))) => {
                self.emit("(", at(name.start));
                self.placeholder("_", name);
                self.emit("(", open.byte_range());
                self.copy(open.end_byte()..close.start_byte());
                self.emit(")", close.byte_range());
                self.emit(")", at(close.end_byte()));
            }
            (_, None) => self.placeholder("(0)", at(directive.end)),
        }
        let end = head.map_or(directive.end, |(_, close, _)| close.end_byte());
        self.emit(":\n", at(end));
        if let Some((_, close, end)) = head {
            self.consume(close, end);
        }
    }

    fn conditional(&mut self, parts: Parts<'_>, name_range: Range<usize>, name: &str) {
        // Blade's variables of the branch: defined in it, forgotten after it.
        let variable = ["error", "session", "context"].contains(&name);
        if variable && parts.branches.is_empty() {
            self.marker(name_range.clone());
        }
        self.emit("if ", name_range.clone());
        let condition = match name {
            "if" => Condition::Plain,
            "unless" => Condition::Not,
            "isset" | "empty" => Condition::Construct,
            _ => Condition::Runtime(name_range.clone()),
        };
        let head = self.parens(&parts.start.directive);
        self.condition(name_range, head, condition);
        self.arm_body(&parts.start);
        for branch in &parts.branches {
            self.else_clause(branch);
        }
        self.emit("endif;\n", parts.close.clone());
        if variable {
            self.marker(parts.close);
        }
    }

    /// A branch of a conditional (`@elseif(c)`, `@else`, `@elsecan(...)`) and its body.
    fn else_clause(&mut self, branch: &Arm<'_>) {
        let Some(name_leaf) = name_leaf(&branch.directive) else {
            return self.arm_body(branch);
        };
        let name_range = self.span(&name_leaf);
        let name = &self.template[name_range.start + 1..name_range.end];
        let head = self.parens(&branch.directive);
        if !compiles(name) {
            // `@else` of a `Blade::if` (`@elsefeature(...)`) Blade does not compile: text.
            if let Some((_, close, end)) = head {
                self.application_directive(name_range);
                self.consume(close, end);
            }
        } else if name.eq_ignore_ascii_case("else") {
            self.emit("else", name_range.clone());
            self.emit(":\n", at(name_range.end));
        } else {
            let condition = if name.eq_ignore_ascii_case("elseif") {
                Condition::Plain
            } else {
                Condition::Runtime(name_range.clone())
            };
            self.emit("elseif ", name_range.clone());
            self.condition(name_range, head, condition);
        }
        self.arm_body(branch);
    }

    /// A body Blade's runtime may skip (`@once`, `@pushOnce`, `@prependOnce`).
    fn guarded(&mut self, parts: Parts<'_>, name_range: Range<usize>) {
        let head = self.parens(&parts.start.directive);
        self.emit("if ", name_range.clone());
        self.condition(name_range.clone(), head, Condition::Runtime(name_range));
        self.arm_body(&parts.start);
        for branch in &parts.branches {
            self.arm(branch);
        }
        self.emit("endif;\n", parts.close);
    }

    fn repeat(&mut self, parts: Parts<'_>, name_range: Range<usize>, name: &str) {
        let Some((open, close, end)) = self.parens(&parts.start.directive) else {
            self.arm_body(&parts.start);
            for branch in &parts.branches {
                self.arm(branch);
            }
            return;
        };
        let keyword = match name {
            "foreach" | "forelse" => "foreach",
            "for" => "for",
            _ => "while",
        };
        let each = keyword == "foreach";
        if each {
            self.marker(name_range.clone());
        }
        self.emit(keyword, name_range);
        self.emit(" (", open.byte_range());
        self.copy(open.end_byte()..close.start_byte());
        self.emit(")", close.byte_range());
        self.emit(":\n", at(close.end_byte()));
        self.loops += 1;
        self.consume(close, end);
        self.arm_body(&parts.start);
        let mut looping = true;
        for branch in &parts.branches {
            let empty = looping
                && name == "forelse"
                && directive_name(&branch.directive).as_deref() == Some("empty")
                && self.parens(&branch.directive).is_none();
            if let Some(leaf) = name_leaf(&branch.directive).filter(|_| empty) {
                let range = self.span(&leaf);
                self.loops -= 1;
                looping = false;
                self.emit("endforeach;\n", range.clone());
                self.marker(range.clone());
                self.emit("if ", range.clone());
                self.placeholder("(0)", at(range.end));
                self.emit(":\n", at(range.end));
                self.arm_body(branch);
            } else {
                self.arm(branch);
            }
        }
        if !looping {
            self.emit("endif;\n", parts.close);
            return;
        }
        self.loops -= 1;
        let ending = match keyword {
            "foreach" => "endforeach;\n",
            "for" => "endfor;\n",
            _ => "endwhile;\n",
        };
        self.emit(ending, parts.close.clone());
        if each {
            self.marker(parts.close);
        }
    }

    /// `@break`, `@continue`, `@break(c)` (`if (c) break;`) and `@break(2)` (`break 2;`), the
    /// directive's `@name` at template bytes `directive`.
    fn loop_operator(&mut self, directive: Range<usize>, parens: Option<(Paren, Paren)>) {
        let keyword = self.template[directive.start + 1..directive.end].to_ascii_lowercase();
        let Some((open, close)) = parens else {
            self.emit(&keyword, directive.clone());
            self.emit(";\n", at(directive.end));
            return;
        };
        let arguments = open.end_byte()..close.start_byte();
        let argument = self.template[arguments.clone()].trim();
        if !argument.is_empty() && argument.bytes().all(|byte| byte.is_ascii_digit()) {
            self.emit(&keyword, directive);
            self.emit(" ", open.byte_range());
            self.copy(arguments);
            self.emit(";\n", close.byte_range());
            return;
        }
        self.emit("if ", directive);
        self.emit("(", open.byte_range());
        self.copy(arguments);
        self.emit(")", close.byte_range());
        self.emit(&format!(" {keyword};\n"), at(close.end_byte()));
    }

    fn switch(&mut self, parts: Parts<'_>, name_range: Range<usize>) {
        let Some((open, close, end)) = self.parens(&parts.start.directive) else {
            self.arm_body(&parts.start);
            for branch in &parts.branches {
                self.arm(branch);
            }
            return;
        };
        self.emit("switch", name_range);
        self.emit(" (", open.byte_range());
        self.copy(open.end_byte()..close.start_byte());
        self.emit(")", close.byte_range());
        self.emit(":\n", at(close.end_byte()));
        // `break` and `continue` end a switch as they end a loop.
        self.switches += 1;
        self.loops += 1;
        self.consume(close, end);
        self.arm_body(&parts.start);
        for branch in &parts.branches {
            self.arm(branch);
        }
        self.loops -= 1;
        self.switches -= 1;
        self.emit("endswitch;\n", parts.close);
    }

    /// The directive at template bytes `directive` (its `@` and name), with arguments.
    fn directive_at(&mut self, directive: Range<usize>, open: Paren, close: Paren) {
        let template = self.template;
        let arguments = open.end_byte()..close.start_byte();
        let name = &template[directive.clone()].trim_start()[1..];
        let sign = directive.end - name.len() - 1;
        // Blade's directive pattern (`\B@`): one a word byte precedes (`x_@json(...)`, which
        // Forte reads as a directive) is text, which may hold echoes.
        if sign > 0 && word(template.as_bytes()[sign - 1]) {
            return self.loose(open.start_byte()..close.end_byte());
        }
        if !compiles(name) {
            return self.application_directive(directive);
        }
        match name {
            "use" => return self.use_statement(directive, arguments, close),
            "unset" => {
                // `unset(args);`
                self.emit("unset", directive);
                self.emit("(", open.byte_range());
                self.copy(arguments);
                self.emit(")", close.byte_range());
                self.emit(";\n", at(close.end_byte()));
                return;
            }
            "inject" => {
                // `@inject('name', service)` is `$name = app(service);`: the marker spans the
                // directive and the variable's name (`@inject('name`), and the service, after the
                // first comma, is the call's PHP.
                let text = &template[arguments.clone()];
                let first = text.split(',').next().unwrap_or_default();
                let name = first.trim_end().trim_end_matches(['\'', '"']);
                self.marker(directive.start..arguments.start + name.len());
                let service = text
                    .find(',')
                    .map_or(arguments.end, |comma| arguments.start + comma + 1);
                return self.call(
                    directive,
                    open.byte_range(),
                    close.byte_range(),
                    service..arguments.end,
                );
            }
            "props" | "aware" => self.marker(directive.clone()),
            _ if PASSES_VARIABLES.contains(&name) => self.marker(directive.clone()),
            _ => {}
        }
        self.call(directive, open.byte_range(), close.byte_range(), arguments);
    }

    /// `@name(args)` as a call with no callee spanning the directive, its arguments the template's
    /// `arguments`.
    fn call(
        &mut self,
        directive: Range<usize>,
        open: Range<usize>,
        close: Range<usize>,
        arguments: Range<usize>,
    ) {
        self.placeholder("_", directive);
        self.emit("(", open);
        self.copy(arguments);
        self.emit(")", close.clone());
        self.emit(";\n", at(close.end));
    }

    /// `@use('A\B', 'C')`: `use A\B as C;`, the name and alias read as Blade reads them (quotes,
    /// parentheses and spaces around them dropped; a `{...}` group keeps its commas).
    fn use_statement(&mut self, directive: Range<usize>, arguments: Range<usize>, close: Paren) {
        let trim = |range: Range<usize>| -> Range<usize> {
            let text = &self.template[range.clone()];
            let leading = text.len()
                - text
                    .trim_start_matches([' ', '\'', '"', '(', ')', '\t', '\n', '\r'])
                    .len();
            let kept =
                text[leading..].trim_end_matches([' ', '\'', '"', '(', ')', '\t', '\n', '\r']);
            range.start + leading..range.start + leading + kept.len()
        };
        let text = &self.template[arguments.clone()];
        let (path, alias) = match text.find(',').filter(|_| !text.contains('{')) {
            Some(comma) => (
                trim(arguments.start..arguments.start + comma),
                Some(trim(arguments.start + comma + 1..arguments.end)),
            ),
            None => (trim(arguments), None),
        };
        if path.is_empty() {
            return;
        }
        self.emit("use ", directive);
        self.copy(path);
        if let Some(alias) = alias.filter(|alias| !alias.is_empty()) {
            self.emit(" as ", at(alias.start));
            self.copy(alias);
        }
        self.emit(";\n", at(close.end_byte()));
    }

    /// An element's attributes and content; a component's (`<x-...>`) bound attributes
    /// (`:name="php"`, `:$name`) are PHP. The text of `<script>` and `<style>`, which Forte reads
    /// as raw text (Blade's own constructs aside), is read as a template of its own where it holds
    /// a component tag, as Blade's compiler reads any text (a `<script type="text/x-template">`).
    fn tag(&mut self, node: BladeNode<'_>) {
        let component = is_component(&node);
        let raw = node.kind() == "element"
            && node.field("name").is_some_and(|name| {
                let name = name.bytes();
                name.eq_ignore_ascii_case(b"script") || name.eq_ignore_ascii_case(b"style")
            });
        let children = node.children().collect::<Vec<_>>();
        // The content: between the start tag's `>` and the closing tag's `<`.
        let open = children.iter().position(|child| child.kind() == ">");
        let close = children
            .iter()
            .position(|child| child.field_name() == Some("closing_name"))
            .map(|name| name.saturating_sub(2))
            .unwrap_or(children.len());
        let content = open.filter(|&open| raw && open < close).and_then(|open| {
            let start = self.span(&children[open]).end;
            let end = children
                .get(close)
                .map_or(self.span(&node).end, |child| self.span(child).start);
            let text = &self.template[start..end.max(start)];
            (text.contains("<x-") || text.contains("<x:")).then_some((open + 1..close, start..end))
        });
        // A component's attributes by name, the last of a name winning (Blade reads them into
        // an array): an earlier one of the same name is not compiled.
        let keys = children
            .iter()
            .map(|child| {
                let name = child
                    .field("name")
                    .filter(|_| child.kind() == "attribute")?;
                let name = name.bytes();
                let mut name = name
                    .strip_prefix(b":")
                    .filter(|rest| !rest.starts_with(b":"))
                    .unwrap_or(name);
                while let Some(rest) = name.strip_prefix(b"$") {
                    name = rest;
                }
                Some(name.to_vec())
            })
            .collect::<Vec<_>>();
        for (index, child) in children.iter().enumerate() {
            if let Some((children, range)) = &content
                && children.contains(&index)
            {
                if index == children.start {
                    self.template(range.clone());
                    self.loosened = self.loosened.max(range.end);
                }
                continue;
            }
            if component && child.kind() == "attribute" {
                if keys[index].is_some() && keys[index + 1..].contains(&keys[index]) {
                    continue;
                }
                if let Some(bound) = self.bound(child) {
                    self.copy(bound.clone());
                    self.emit("\n;", at(bound.end));
                    continue;
                }
            }
            self.node(*child);
        }
    }

    /// The PHP a component attribute binds: `:name="php"`'s value, `:$name`'s variable.
    fn bound(&self, attribute: &BladeNode<'_>) -> Option<Range<usize>> {
        let name = self.span(&attribute.field("name")?);
        let text = self.text(name.clone());
        if !text.starts_with(':') || text.starts_with("::") {
            return None;
        }
        let value = attribute.field("value").filter(|value| {
            let bytes = value.bytes();
            bytes.len() >= 2 && matches!(bytes[0], b'"' | b'\'') && bytes.last() == Some(&bytes[0])
        });
        match value {
            Some(value) => {
                let range = self.span(&value);
                Some(range.start + 1..range.end - 1)
            }
            None if attribute.field("value").is_none() && text.starts_with(":$") => {
                Some(name.start + 1..name.end)
            }
            None => None,
        }
    }

    /// Echoes, and directives with arguments that compile to a statement, in text Forte did not
    /// read as Blade (a directive its registry does not know, an HTML comment's text), found as
    /// Blade's compiler finds them. A directive Blade compiles to control flow there is not read:
    /// the view reads no half of a structure.
    fn loose(&mut self, range: Range<usize>) {
        let range = range.start.max(self.loosened)..range.end;
        if range.is_empty() {
            return;
        }
        self.loosened = range.end;
        let text = &self.template[range.clone()];
        let bytes = text.as_bytes();
        let mut index = 0;
        // Where the last escaped directive (`@@name(...)`) ends: a directive inside it is text.
        let mut escaped_until = 0;
        // `@@` and an escaped directive can step past the range's end (the second `@` is the next
        // node's): the text ends there.
        while let Some(found) = text.get(index..).and_then(|rest| rest.find(['{', '@'])) {
            index += found;
            let rest = &text[index..];
            if rest.starts_with('@') {
                let at = range.start + index;
                // Blade's directive pattern (`\B@`): an `@` no word byte precedes.
                let after_word = at > 0 && word(self.template.as_bytes()[at - 1]);
                if !after_word && let Some(end) = escaped_directive(self.template, at, range.end) {
                    // Blade writes it as text, `@name(...)`; the echoes in it are still echoes.
                    escaped_until = escaped_until.max(end - range.start);
                    index += 2;
                } else if index < escaped_until {
                    index += 1;
                } else {
                    index = self.loose_directive(range.clone(), index);
                }
                continue;
            }
            let (open, close) = if rest.starts_with("{{--") {
                // A comment runs to its `--}}`.
                index += rest.find("--}}").map_or(rest.len(), |end| end + 4);
                continue;
            } else if rest.starts_with("{!!") {
                ("{!!", "!!}")
            } else if rest.starts_with("{{") {
                ("{{", "}}")
            } else {
                index += 1;
                continue;
            };
            let escaped = if index > 0 {
                bytes[index - 1] == b'@'
            } else {
                self.template[..range.start].ends_with('@')
            };
            if escaped {
                // Blade writes `@{{ ... }}` as text, after compiling the directives it holds.
                index += open.len();
                continue;
            }
            let Some(end) = rest[open.len()..].find(close) else {
                break;
            };
            let start = range.start + index;
            let php = start + open.len()..start + open.len() + end;
            let after = php.end + close.len();
            self.echo(start..start + open.len(), php.clone(), php.end..after);
            index = after - range.start;
        }
    }

    /// The directive at `range.start + index` (its `@`) in loose text, read when Blade compiles it
    /// to a statement (`\B@name(...)`, its arguments balanced as `balanced` reads them). Returns
    /// the index after it.
    fn loose_directive(&mut self, range: Range<usize>, index: usize) -> usize {
        let start = range.start + index;
        let bytes = self.template.as_bytes();
        if start > 0 && (word(bytes[start - 1]) || bytes[start - 1] == b'@') {
            return index + 1;
        }
        if bytes.get(start + 1) == Some(&b'@') {
            // `@@name` is Blade's escape for the text `@name`.
            return index + 2;
        }
        let name = self.template[start + 1..range.end]
            .bytes()
            .take_while(|byte| word(*byte))
            .count();
        let spaces = self.template[start + 1 + name..range.end]
            .bytes()
            .take_while(|byte| *byte == b' ' || *byte == b'\t')
            .count();
        let open = start + 1 + name + spaces;
        let directive = &self.template[start + 1..start + 1 + name];
        if name == 0
            || bytes.get(open) != Some(&b'(')
            || CONTROL
                .iter()
                .any(|control| control.eq_ignore_ascii_case(directive))
        {
            return index + 1 + name;
        }
        let Some(close) = balanced(&self.template[open..range.end]).map(|close| open + close)
        else {
            return index + 1 + name;
        };
        self.directive_at(start..start + 1 + name, Paren(open), Paren(close));
        close + 1 - range.start
    }
}

/// Whether PCRE reads `byte` as a word byte (`\w`), as Blade's directive pattern does.
fn word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Where the escaped directive at `template[start]` ends, `None` where none is there: `@@name`
/// and, after spaces or tabs, its balanced `(...)`, all of which Blade writes as text.
fn escaped_directive(template: &str, start: usize, limit: usize) -> Option<usize> {
    let bytes = &template.as_bytes()[start..limit];
    let name = bytes
        .get(2..)?
        .iter()
        .take_while(|byte| word(**byte))
        .count();
    if !bytes.starts_with(b"@@") || name == 0 {
        return None;
    }
    let after = 2 + name;
    let open = after
        + bytes[after..]
            .iter()
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .count();
    if bytes.get(open) == Some(&b'(')
        && let Some(close) = balanced(&template[start + open..limit])
    {
        return Some(start + open + close + 1);
    }
    Some(start + after)
}

/// The offset of the `)` closing the `(` at `text[0]`, as `BladeCompiler` finds it: the first `)`
/// PHP's tokenizer reads as code that balances it (`hasEvenNumberOfParentheses`), so one in a
/// comment, a string, a heredoc or a nowdoc does not.
fn balanced(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (index, byte) in code(text) {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

/// Whether the last token of the PHP `text`, comments aside, is a `;`.
fn ends_statement(text: &str) -> bool {
    code(text)
        .filter(|(_, byte)| !byte.is_ascii_whitespace())
        .last()
        .is_some_and(|(_, byte)| byte == b';')
}

/// Each byte of PHP code in `text`, with its offset, as PHP's tokenizer reads it: outside its
/// comments and string literals (quoted, heredoc, nowdoc), each skipped whole.
fn code(text: &str) -> impl Iterator<Item = (usize, u8)> + '_ {
    let bytes = text.as_bytes();
    let mut at = 0;
    std::iter::from_fn(move || {
        while at < bytes.len() {
            if let Some(end) = literal_end(bytes, at, 0) {
                at = end;
                continue;
            }
            at += 1;
            return Some((at - 1, bytes[at - 1]));
        }
        None
    })
}

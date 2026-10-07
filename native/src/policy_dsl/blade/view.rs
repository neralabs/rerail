//! The PHP a Blade template embeds, read as one PHP program in the order the template runs it: the
//! program `illuminate/view` 13.34.0's `BladeCompiler` compiles the template to, without the
//! bookkeeping Blade's runtime adds (`$__env`, `e()`, output buffering).
//!
//! The program is written out as PHP text and parsed with tree-sitter-php's PHP-only grammar; each
//! of its nodes maps back to the template bytes it came from. The PHP a template spells (an echo's
//! expression, a directive's arguments, a `@php` block, a component's bound attribute) is copied
//! byte for byte, so its nodes map exactly onto the template. The PHP Blade wraps it in is
//! written around it, each token anchored to the Blade token it stands for:
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
//! `Blade::if`, a package's such as Livewire's), even where the grammar's error recovery reads one
//! as a block (`<div>@icon('x')</div>`). Such a directive with arguments is a marker that may read
//! and assign every variable, since the application compiles it to PHP the template does not
//! show; the body of a pair of them (`@name(...) ... @endname`) is read as the template's.
//! A directive's arguments run to the `)` Blade finds with PHP's tokenizer, so one in a comment,
//! string or heredoc does not end them.
//! Blade's compiler does not read HTML, so the view reads what it compiles wherever it stands: the
//! text of `<script>` and `<style>`, which the grammar does not parse (their tags' attributes it
//! does), is parsed as a template of its own (a `<script type="text/x-template">` holds directives
//! and components); echoes in an attribute's name or value, or in text the grammar's error
//! recovery left unread, are found as Blade's compiler finds them.

use crate::policy_dsl::language::GrammarDialect;
use ast_grep_language::LanguageExt;
use std::ops::Range;
use tree_sitter::{Node as TsNode, Parser, Tree};

/// One stretch of the program's text and the template bytes it stands for.
#[derive(Clone, Debug)]
struct Segment {
    /// Where it starts in the program.
    at: usize,
    end: usize,
    anchor: Range<usize>,
    /// Copied from the template: each byte maps to the byte at the same offset of `anchor`.
    copied: bool,
    /// A placeholder the grammar needs and the view does not keep (`Emitter::placeholder`).
    omit: bool,
}

/// The embedded PHP program and its map back to the template.
pub(crate) struct View {
    segments: Vec<Segment>,
    tree: Tree,
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

impl View {
    /// The program `root`'s Blade tree embeds, `None` where it embeds no PHP.
    pub(crate) fn build(root: TsNode<'_>, template: &str) -> Option<Self> {
        let mut emitter = Emitter {
            template,
            out: String::new(),
            segments: Vec::new(),
            loosened: 0,
        };
        emitter.children(root);
        if emitter.segments.is_empty() {
            return None;
        }
        let mut parser = Parser::new();
        parser
            .set_language(&GrammarDialect::Php.get_ts_language())
            .ok()?;
        let tree = parser.parse(&emitter.out, None)?;
        Some(Self {
            segments: emitter.segments,
            tree,
        })
    }

    pub(crate) fn root(&self) -> TsNode<'_> {
        self.tree.root_node()
    }

    /// The segment holding program byte `at` (the last one at the end of the program).
    fn segment(&self, at: usize) -> &Segment {
        let index = self
            .segments
            .partition_point(|segment| segment.end <= at)
            .min(self.segments.len() - 1);
        &self.segments[index]
    }

    /// The template position a node starting at program byte `at` starts at.
    fn start(&self, at: usize) -> usize {
        let segment = self.segment(at);
        if segment.copied {
            segment.anchor.start + (at.clamp(segment.at, segment.end) - segment.at)
        } else {
            segment.anchor.start
        }
    }

    /// The template position a node ending at program byte `at` ends at.
    fn end(&self, at: usize) -> usize {
        let segment = self.segment(at.saturating_sub(1));
        if segment.copied {
            segment.anchor.start + (at.clamp(segment.at, segment.end) - segment.at)
        } else {
            segment.anchor.end
        }
    }

    /// The template bytes a node of the program stands for.
    pub(crate) fn span(&self, node: TsNode<'_>) -> Range<usize> {
        let range = node.byte_range();
        if range.is_empty() {
            let at = self.start(range.start);
            return at..at;
        }
        let start = self.start(range.start);
        start..self.end(range.end).max(start)
    }

    /// Whether a node is part of a placeholder the view does not keep.
    pub(crate) fn omitted(&self, node: TsNode<'_>) -> bool {
        let range = node.byte_range();
        !range.is_empty() && {
            let segment = self.segment(range.start);
            segment.omit && range.end <= segment.end
        }
    }

    /// The template ranges the program copies byte for byte, in program order.
    #[cfg(test)]
    pub(crate) fn copied(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.segments
            .iter()
            .filter(|segment| segment.copied)
            .map(|segment| segment.anchor.clone())
    }
}

struct Emitter<'t> {
    template: &'t str,
    out: String,
    segments: Vec<Segment>,
    /// Where the text `loose` has read ends: a run of attributes is read once, from its first.
    loosened: usize,
}

/// An empty template range at `at`.
fn at(at: usize) -> Range<usize> {
    at..at
}

fn children(node: TsNode<'_>) -> Vec<TsNode<'_>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).collect()
}

/// Whether an attribute holds only text: no echo or directive the grammar read in it.
fn plain(node: TsNode<'_>) -> bool {
    children(node).into_iter().all(|kid| {
        !kid.is_named()
            || matches!(kid.kind(), "attribute_name" | "attribute_value")
            || (kid.kind() == "quoted_attribute_value" && plain(kid))
    })
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

/// A condition Blade decides at run time from what its directive passes its runtime.
enum Condition<'n> {
    /// `(c)`: the template's own condition.
    Plain,
    /// `(!(c))`.
    Not,
    /// `(isset(a))`, `(empty(a))`: the construct names the directive's own bytes.
    Construct,
    /// A call of the directive with its arguments, or nothing where it has none.
    Runtime(TsNode<'n>),
}

impl Emitter<'_> {
    /// The `(` and `)` around the arguments of the directive `kids[from - 1]`, and the index of the
    /// first kid after the `)`. As Blade reads them, the arguments open with a `(` after the
    /// directive and any spaces or tabs, and run to the `)` that balances it (`balanced`),
    /// wherever the grammar ends them (`@section('a', __('b'))`, which it may read as an error);
    /// without one, to the grammar's first `)`.
    fn parens(&self, kids: &[TsNode<'_>], from: usize) -> Option<(Paren, Paren, usize)> {
        let directive = kids.get(from.checked_sub(1)?)?;
        let first = kids.get(from)?;
        let open = directive.end_byte()
            + self.template[directive.end_byte()..]
                .bytes()
                .take_while(|byte| *byte == b' ' || *byte == b'\t')
                .count();
        if first.start_byte() != open || self.template.as_bytes().get(open) != Some(&b'(') {
            return None;
        }
        let close = match balanced(&self.template[open..]) {
            Some(close) => open + close,
            None => kids[from + 1..]
                .iter()
                .find(|kid| !kid.is_named() && kid.kind() == ")")?
                .start_byte(),
        };
        let next = (from + 1..kids.len())
            .find(|&index| kids[index].start_byte() > close)
            .unwrap_or(kids.len());
        Some((Paren(open), Paren(close), next))
    }
}

impl<'t> Emitter<'t> {
    fn text(&self, node: TsNode<'_>) -> &'t str {
        &self.template[node.byte_range()]
    }

    fn push(&mut self, text: &str, anchor: Range<usize>, copied: bool, omit: bool) {
        let at = self.out.len();
        self.out.push_str(text);
        self.segments.push(Segment {
            at,
            end: self.out.len(),
            anchor,
            copied,
            omit,
        });
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

    fn children(&mut self, node: TsNode<'_>) {
        let kids = children(node);
        self.sequence(&kids, 0);
    }

    /// Nodes in order, a directive with arguments read as one.
    fn sequence(&mut self, kids: &[TsNode<'_>], from: usize) {
        self.sequence_within(kids, from, None);
    }

    /// Nodes in order, a directive with arguments read as one. Within an error the grammar
    /// recovered from (`within`), the bytes no named node holds are text Blade still compiles.
    fn sequence_within(&mut self, kids: &[TsNode<'_>], from: usize, within: Option<Range<usize>>) {
        let mut cursor = within.as_ref().map(|within| within.start);
        let mut index = from;
        while index < kids.len() {
            let kid = kids[index];
            if let Some(at) = cursor {
                // A token, or PHP the grammar read outside the echo or directive it recovered
                // from: text, which may hold that echo whole.
                if !kid.is_named() || matches!(kid.kind(), "php_only" | "parameter") {
                    index += 1;
                    continue;
                }
                self.loose(at..kid.start_byte().max(at));
            }
            if matches!(kid.kind(), "directive" | "directive_start")
                && let Some((open, close, next)) = self.parens(kids, index + 1)
            {
                self.directive(kid, open, close);
                cursor = cursor.map(|at| at.max(close.end_byte()));
                index = next;
                continue;
            }
            self.node(kid);
            cursor = cursor.map(|at| at.max(kid.end_byte()));
            index += 1;
        }
        if let (Some(at), Some(within)) = (cursor, within) {
            self.loose(at..within.end.max(at));
        }
    }

    fn node(&mut self, node: TsNode<'_>) {
        match node.kind() {
            "php_statement" => self.php_statement(node),
            "conditional" => self.conditional(node),
            "loop" => self.repeat(node),
            "switch" => self.switch(node),
            "once" => self.guarded(node),
            "stack" | "livewire" => {
                let kids = children(node);
                let name = kids.first().map(|first| self.text(*first));
                match name {
                    Some("@pushOnce" | "@prependOnce" | "@pushIf") => self.guarded(node),
                    _ => self.sequence(&kids, 0),
                }
            }
            // An element's children in order, a directive with arguments read as one.
            "element" => self.children(node),
            "start_tag" | "self_closing_tag" => self.tag(node),
            // The tags' attributes as any tag's; the text between them as Blade reads it.
            "script_element" | "style_element" => {
                for kid in children(node) {
                    if kid.kind() == "raw_text" {
                        self.raw_text(kid.byte_range());
                    } else {
                        self.node(kid);
                    }
                }
            }
            "ERROR" => {
                let kids = children(node);
                self.sequence_within(&kids, 0, Some(node.byte_range()));
            }
            // Text holds no echo the grammar read, but Blade compiles any it holds: one in an
            // attribute's name (`x-on:{{ $event }}`) or an HTML comment.
            "attribute" if plain(node) => self.plain_attributes(node),
            "text" | "attribute_name" | "attribute_value" => self.loose(node.byte_range()),
            "comment" if self.text(node).starts_with("<!--") => self.loose(node.byte_range()),
            "verbatim" | "comment" | "keyword" | "doctype" | "entity" | "raw_text"
            | "directive_end" | "tag_name" => {}
            _ => self.children(node),
        }
    }

    fn php_statement(&mut self, node: TsNode<'_>) {
        let kids = children(node);
        let Some(&first) = kids.first() else {
            return;
        };
        let php = kids.iter().find(|kid| kid.kind() == "php_only").copied();
        let last = *kids.last().expect("a first child");
        match first.kind() {
            "{{" | "{!!" => {
                // `@{{ ... }}` is Blade's escape for a literal echo.
                if self.template[..first.start_byte()].ends_with('@') {
                    return;
                }
                if let Some(php) = php {
                    self.echo(first.byte_range(), php.byte_range(), last.byte_range());
                }
            }
            "php_tag" => {
                if let Some(php) = php {
                    if self.text(first) == "<?=" {
                        self.echo(first.byte_range(), php.byte_range(), last.byte_range());
                    } else {
                        self.statements(php.byte_range(), last.byte_range());
                    }
                }
            }
            "directive_start" => {
                if let Some(php) = php {
                    self.statements(php.byte_range(), last.byte_range());
                }
            }
            "directive" => {
                if let Some((open, close, _)) = self.parens(&kids, 1) {
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
                }
            }
            _ => {}
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

    /// A condition from a directive's arguments `open..close`, after `if`/`elseif` was written
    /// over `directive`.
    fn condition(
        &mut self,
        directive: TsNode<'_>,
        parens: Option<(Paren, Paren)>,
        condition: Condition<'_>,
    ) {
        let directive = directive.byte_range();
        match (condition, parens) {
            (Condition::Plain, Some((open, close))) => {
                self.emit("(", open.byte_range());
                self.copy(open.end_byte()..close.start_byte());
                self.emit(")", close.byte_range());
            }
            (Condition::Not, Some((open, close))) => {
                self.emit("(", open.byte_range());
                self.emit("!(", at(open.end_byte()));
                self.copy(open.end_byte()..close.start_byte());
                self.emit(")", at(close.start_byte()));
                self.emit(")", close.byte_range());
            }
            (Condition::Construct, Some((open, close))) => {
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
            (Condition::Runtime(name), Some((open, close))) => {
                self.emit("(", at(name.start_byte()));
                self.placeholder("_", name.byte_range());
                self.emit("(", open.byte_range());
                self.copy(open.end_byte()..close.start_byte());
                self.emit(")", close.byte_range());
                self.emit(")", at(close.end_byte()));
            }
            (_, None) => self.placeholder("(0)", at(directive.end)),
        }
        let end = parens.map_or(directive.end, |(_, close)| close.end_byte());
        self.emit(":\n", at(end));
    }

    fn conditional(&mut self, node: TsNode<'_>) {
        let kids = children(node);
        let Some(&start) = kids.first() else {
            return;
        };
        let name = self.text(start).trim().trim_start_matches('@').to_owned();
        if !compiles(&name) {
            // A pair Blade does not compile (an application's `Blade::if`, a package's) is text
            // around a body Blade compiles as the template's.
            return self.unknown_block(&kids);
        }
        let head = self.parens(&kids, 1);
        let body = head.map_or(1, |(_, _, next)| next);
        let end = kids
            .last()
            .filter(|kid| kid.kind() == "directive_end")
            .copied();
        let stop = if end.is_some() {
            kids.len() - 1
        } else {
            kids.len()
        };
        let has_else = kids[body..]
            .iter()
            .any(|kid| kid.kind() == "conditional_keyword");
        // Blade's variables of the branch: defined in it, forgotten after it.
        let variable = match name.as_str() {
            "error" | "session" | "context" => Some(()),
            _ => None,
        };
        if variable.is_some() && !has_else {
            self.marker(start.byte_range());
        }
        self.emit("if ", start.byte_range());
        let condition = match name.as_str() {
            "if" => Condition::Plain,
            "unless" => Condition::Not,
            "isset" | "empty" => Condition::Construct,
            _ => Condition::Runtime(start),
        };
        self.condition(start, head.map(|(open, close, _)| (open, close)), condition);
        let mut index = body;
        while index < stop {
            let kid = kids[index];
            if kid.kind() == "conditional_keyword" {
                self.else_clause(kid);
            } else if matches!(kid.kind(), "directive" | "directive_start")
                && let Some((open, close, next)) = self.parens(&kids, index + 1)
            {
                self.directive(kid, open, close);
                index = next;
                continue;
            } else {
                self.node(kid);
            }
            index += 1;
        }
        let close = end.map_or(at(node.end_byte()), |end| end.byte_range());
        self.emit("endif;\n", close.clone());
        if variable.is_some() {
            self.marker(close);
        }
    }

    fn else_clause(&mut self, node: TsNode<'_>) {
        let kids = children(node);
        let Some(&directive) = kids.first() else {
            return;
        };
        // `@else` of a `Blade::if` (`@elsefeature(...)`) Blade does not compile: text.
        if !compiles(self.text(directive).trim_start().trim_start_matches('@')) {
            if self.parens(&kids, 1).is_some() {
                self.application_directive(directive.byte_range());
            }
            return;
        }
        let head = self.parens(&kids, 1).map(|(open, close, _)| (open, close));
        if self.text(directive) == "@else" {
            self.emit("else", directive.byte_range());
            self.emit(":\n", at(directive.end_byte()));
            return;
        }
        self.emit("elseif ", directive.byte_range());
        let condition = if self.text(directive) == "@elseif" {
            Condition::Plain
        } else {
            Condition::Runtime(directive)
        };
        self.condition(directive, head, condition);
    }

    /// A container whose body Blade may skip at run time (`@once`, `@pushOnce`, `@pushIf`).
    fn guarded(&mut self, node: TsNode<'_>) {
        let kids = children(node);
        let Some(&start) = kids.first() else {
            return;
        };
        if !compiles(self.text(start).trim().trim_start_matches('@')) {
            return self.unknown_block(&kids);
        }
        let head = self.parens(&kids, 1);
        self.emit("if ", start.byte_range());
        self.condition(
            start,
            head.map(|(open, close, _)| (open, close)),
            Condition::Runtime(start),
        );
        let body = head.map_or(1, |(_, _, next)| next);
        let end = kids
            .last()
            .filter(|kid| kid.kind() == "directive_end")
            .copied();
        let stop = if end.is_some() {
            kids.len() - 1
        } else {
            kids.len()
        };
        self.sequence(&kids[..stop], body);
        let close = end.map_or(at(node.end_byte()), |end| end.byte_range());
        self.emit("endif;\n", close);
    }

    fn repeat(&mut self, node: TsNode<'_>) {
        let kids = children(node);
        let Some(&start) = kids.first() else {
            return;
        };
        let name = self.text(start).trim().trim_start_matches('@').to_owned();
        if !compiles(&name) {
            return self.unknown_block(&kids);
        }
        let Some((open, close, body)) = self.parens(&kids, 1) else {
            self.sequence(&kids, 1);
            return;
        };
        let keyword = match name.as_str() {
            "foreach" | "forelse" => "foreach",
            "for" => "for",
            _ => "while",
        };
        let each = keyword == "foreach";
        if each {
            self.marker(start.byte_range());
        }
        self.emit(keyword, start.byte_range());
        self.emit(" (", open.byte_range());
        self.copy(open.end_byte()..close.start_byte());
        self.emit(")", close.byte_range());
        self.emit(":\n", at(close.end_byte()));
        let end = kids
            .last()
            .filter(|kid| kid.kind() == "directive_end")
            .copied();
        let stop = if end.is_some() {
            kids.len() - 1
        } else {
            kids.len()
        };
        let mut looping = true;
        let mut index = body;
        while index < stop {
            let kid = kids[index];
            if kid.kind() == "directive" {
                let operator = self.parens(&kids, index + 1);
                match self.text(kid) {
                    "@empty" if looping && name == "forelse" => {
                        self.emit("endforeach;\n", kid.byte_range());
                        self.marker(kid.byte_range());
                        self.emit("if ", kid.byte_range());
                        self.placeholder("(0)", at(kid.end_byte()));
                        self.emit(":\n", at(kid.end_byte()));
                        looping = false;
                        index += 1;
                        continue;
                    }
                    "@break" | "@continue" => {
                        self.loop_operator(kid, operator.map(|(open, close, _)| (open, close)));
                        index = operator.map_or(index + 1, |(_, _, next)| next);
                        continue;
                    }
                    _ => {}
                }
                if let Some((open, close, next)) = operator {
                    self.directive(kid, open, close);
                    index = next;
                    continue;
                }
            }
            self.node(kid);
            index += 1;
        }
        let close = end.map_or(at(node.end_byte()), |end| end.byte_range());
        if !looping {
            self.emit("endif;\n", close);
            return;
        }
        let ending = match keyword {
            "foreach" => "endforeach;\n",
            "for" => "endfor;\n",
            _ => "endwhile;\n",
        };
        self.emit(ending, close.clone());
        if each {
            self.marker(close);
        }
    }

    /// `@break`, `@continue`, `@break(c)` (`if (c) break;`) and `@break(2)` (`break 2;`).
    fn loop_operator(&mut self, directive: TsNode<'_>, parens: Option<(Paren, Paren)>) {
        let keyword = &self.text(directive)[1..];
        let Some((open, close)) = parens else {
            self.emit(keyword, directive.byte_range());
            self.emit(";\n", at(directive.end_byte()));
            return;
        };
        let arguments = open.end_byte()..close.start_byte();
        let argument = self.template[arguments.clone()].trim();
        if !argument.is_empty() && argument.bytes().all(|byte| byte.is_ascii_digit()) {
            self.emit(keyword, directive.byte_range());
            self.emit(" ", open.byte_range());
            self.copy(arguments);
            self.emit(";\n", close.byte_range());
            return;
        }
        self.emit("if ", directive.byte_range());
        self.emit("(", open.byte_range());
        self.copy(arguments);
        self.emit(")", close.byte_range());
        self.emit(&format!(" {keyword};\n"), at(close.end_byte()));
    }

    fn switch(&mut self, node: TsNode<'_>) {
        let kids = children(node);
        let Some(&start) = kids.first() else {
            return;
        };
        if !compiles(self.text(start).trim().trim_start_matches('@')) {
            return self.unknown_block(&kids);
        }
        let Some((open, close, body)) = self.parens(&kids, 1) else {
            self.sequence(&kids, 1);
            return;
        };
        self.emit("switch", start.byte_range());
        self.emit(" (", open.byte_range());
        self.copy(open.end_byte()..close.start_byte());
        self.emit(")", close.byte_range());
        self.emit(":\n", at(close.end_byte()));
        let end = kids
            .last()
            .filter(|kid| kid.kind() == "directive_end")
            .copied();
        let stop = if end.is_some() {
            kids.len() - 1
        } else {
            kids.len()
        };
        let mut index = body;
        while index < stop {
            let kid = kids[index];
            if kid.kind() == "directive" {
                let operator = self.parens(&kids, index + 1);
                match (self.text(kid), operator) {
                    ("@case", Some((open, close, next))) => {
                        self.emit("case", kid.byte_range());
                        self.emit(" (", open.byte_range());
                        self.copy(open.end_byte()..close.start_byte());
                        self.emit(")", close.byte_range());
                        self.emit(":\n", at(close.end_byte()));
                        index = next;
                        continue;
                    }
                    ("@default", _) => {
                        self.emit("default", kid.byte_range());
                        self.emit(":\n", at(kid.end_byte()));
                        index += 1;
                        continue;
                    }
                    ("@break" | "@continue", _) => {
                        self.loop_operator(kid, operator.map(|(open, close, _)| (open, close)));
                        index = operator.map_or(index + 1, |(_, _, next)| next);
                        continue;
                    }
                    (_, Some((open, close, next))) => {
                        self.directive(kid, open, close);
                        index = next;
                        continue;
                    }
                    _ => {}
                }
            }
            self.node(kid);
            index += 1;
        }
        let close = end.map_or(at(node.end_byte()), |end| end.byte_range());
        self.emit("endswitch;\n", close);
    }

    /// A directive with arguments outside Blade's control structures.
    fn directive(&mut self, directive: TsNode<'_>, open: Paren, close: Paren) {
        self.directive_at(directive.byte_range(), open, close);
    }

    /// The directive at template bytes `directive` (its `@` and name), with arguments.
    fn directive_at(&mut self, directive: Range<usize>, open: Paren, close: Paren) {
        let template = self.template;
        let arguments = open.end_byte()..close.start_byte();
        // A directive the grammar recovered from an error may hold the whitespace before it.
        let name = &template[directive.clone()].trim_start()[1..];
        let sign = directive.end - name.len() - 1;
        // Blade's directive pattern (`\B@`): one a word byte precedes (`x_@json(...)`, which the
        // grammar reads as a directive) is text, which may hold echoes.
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

    /// A tag's attributes; a component's (`<x-...>`) bound ones (`:name="php"`, `:$name`) are PHP.
    fn tag(&mut self, node: TsNode<'_>) {
        let kids = children(node);
        let component = kids
            .iter()
            .find(|kid| kid.kind() == "tag_name")
            .is_some_and(|name| {
                let name = self.text(*name);
                name.starts_with("x-") || name.starts_with("x:")
            });
        // A component's attributes by name, the last of a name winning (Blade reads them into
        // an array): an earlier one of the same name is not compiled.
        let template = self.template;
        let key = |attribute: TsNode<'_>| -> Option<&'t str> {
            let name = children(attribute)
                .into_iter()
                .find(|kid| kid.kind() == "attribute_name")?;
            let name = &template[name.byte_range()];
            let name = name
                .strip_prefix(':')
                .filter(|rest| !rest.starts_with(':'))
                .unwrap_or(name);
            Some(name.trim_start_matches('$'))
        };
        let mut index = 0;
        while index < kids.len() {
            let kid = kids[index];
            if component
                && kid.kind() == "attribute"
                && let Some(name) = key(kid)
                && kids[index + 1..]
                    .iter()
                    .any(|later| later.kind() == "attribute" && key(*later) == Some(name))
            {
                index += 1;
                continue;
            }
            if component
                && kid.kind() == "attribute"
                && let Some(bound) = self.bound(kid)
            {
                self.copy(bound.clone());
                self.emit("\n;", at(bound.end));
            } else if kid.kind() == "directive"
                && let Some((open, close, next)) = self.parens(&kids, index + 1)
            {
                self.directive(kid, open, close);
                index = next;
                continue;
            } else {
                self.node(kid);
            }
            index += 1;
        }
    }

    /// An attribute that holds only text, read with the attributes after it that hold only text,
    /// and the name of the one after them: the grammar may split an echo across them
    /// (`wire:model.{{ $modifier }}="search"`, `x-on:{{ $event }}.window="@js($id)"`).
    fn plain_attributes(&mut self, attribute: TsNode<'_>) {
        let mut end = attribute.end_byte();
        let mut next = attribute.next_sibling();
        while let Some(sibling) = next {
            if sibling.kind() != "attribute" || self.text(sibling).starts_with(':') {
                break;
            }
            if !plain(sibling) {
                if let Some(name) = children(sibling)
                    .into_iter()
                    .find(|kid| kid.kind() == "attribute_name")
                {
                    end = name.end_byte();
                }
                break;
            }
            end = sibling.end_byte();
            next = sibling.next_sibling();
        }
        self.loose(attribute.start_byte()..end);
    }

    /// The PHP a component attribute binds: `:name="php"`'s value, `:$name`'s variable.
    fn bound(&self, attribute: TsNode<'_>) -> Option<Range<usize>> {
        let kids = children(attribute);
        let name = kids.iter().find(|kid| kid.kind() == "attribute_name")?;
        let text = self.text(*name);
        if !text.starts_with(':') || text.starts_with("::") {
            return None;
        }
        match kids
            .iter()
            .find(|kid| kid.kind() == "quoted_attribute_value")
        {
            Some(value) => {
                let range = value.byte_range();
                (range.len() >= 2).then(|| range.start + 1..range.end - 1)
            }
            None if text.starts_with(":$") => Some(name.start_byte() + 1..name.end_byte()),
            None => None,
        }
    }

    /// A block the grammar read around a directive `BladeCompiler` does not compile (its error
    /// recovery's, or a package's block such as `@capture ... @endcapture`, an application's
    /// `Blade::if`): with arguments, the directive is an application's (`application_directive`);
    /// what follows them is the template's.
    fn unknown_block(&mut self, kids: &[TsNode<'_>]) {
        let head = self.parens(kids, 1);
        if head.is_some() {
            self.application_directive(kids[0].byte_range());
        }
        let body = head.map_or(1, |(_, _, next)| next);
        self.sequence(kids, body);
    }

    /// `<script>` and `<style>` text, which Blade's compiler reads as it reads any template text:
    /// parsed as a template of its own over those bytes.
    fn raw_text(&mut self, range: Range<usize>) {
        let point = |at: usize| {
            let before = &self.template[..at];
            let row = before.matches('\n').count();
            let column = before.len() - before.rfind('\n').map_or(0, |newline| newline + 1);
            tree_sitter::Point { row, column }
        };
        let ranges = [tree_sitter::Range {
            start_byte: range.start,
            end_byte: range.end,
            start_point: point(range.start),
            end_point: point(range.end),
        }];
        let mut parser = Parser::new();
        if range.is_empty()
            || parser
                .set_language(&GrammarDialect::Blade.get_ts_language())
                .is_err()
            || parser.set_included_ranges(&ranges).is_err()
        {
            return;
        }
        if let Some(tree) = parser.parse(self.template, None) {
            self.children(tree.root_node());
        }
    }

    /// Echoes, and directives with arguments that compile to a statement, in text the grammar did
    /// not read as Blade (an attribute's name, an HTML comment, or what its error recovery left),
    /// found as Blade's compiler finds them. A directive Blade compiles to control flow there is
    /// not read: the view reads no half of a structure.
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
        while let Some(found) = text[index..].find(['{', '@']) {
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
            let Some(end) = rest[open.len()..].find(close) else {
                break;
            };
            let start = range.start + index;
            let php = start + open.len()..start + open.len() + end;
            let after = php.end + close.len();
            if !escaped {
                self.echo(start..start + open.len(), php.clone(), php.end..after);
            }
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

/// How deeply string interpolations may nest (`"{$a["{$b}"]}"`) before the scan takes the rest of
/// the text as the innermost literal: a bound on the recursion template text drives.
const NESTING: usize = 32;

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

/// Where the comment or string literal at `bytes[at]` ends, `None` where none starts there. One
/// PHP would find unterminated runs to the end of `bytes`.
fn literal_end(bytes: &[u8], at: usize, nesting: usize) -> Option<usize> {
    let end = match &bytes[at..] {
        [quote @ (b'\'' | b'"' | b'`'), ..] => quoted_end(bytes, at + 1, *quote, nesting),
        [b'/', b'*', ..] => bytes[at + 2..]
            .windows(2)
            .position(|pair| pair == b"*/")
            .map_or(bytes.len(), |end| at + 2 + end + 2),
        [b'/', b'/', ..] => line_end(bytes, at + 2),
        // `#[` opens an attribute.
        [b'#', b'[', ..] => return None,
        [b'#', ..] => line_end(bytes, at + 1),
        [b'<', b'<', b'<', ..] => return doc_end(bytes, at, nesting),
        _ => return None,
    };
    Some(end)
}

/// Where a `//` or `#` comment whose text starts at `from` ends: at its line's end.
fn line_end(bytes: &[u8], from: usize) -> usize {
    bytes[from..]
        .iter()
        .position(|byte| matches!(byte, b'\n' | b'\r'))
        .map_or(bytes.len(), |end| from + end)
}

/// Where a `close`-quoted string whose text starts at `from` ends: past `\` escapes and, in one
/// that interpolates (`"`, `` ` ``), past the code of each `{$...}` and `${...}`.
fn quoted_end(bytes: &[u8], from: usize, close: u8, nesting: usize) -> usize {
    let mut at = from;
    while at < bytes.len() {
        if bytes[at] == b'\\' {
            at += 2;
        } else if bytes[at] == close {
            return at + 1;
        } else if close != b'\''
            && let Some(end) = interpolation_end(bytes, at, nesting)
        {
            at = end;
        } else {
            at += 1;
        }
    }
    bytes.len()
}

/// Where the interpolation (`{$...}`, `${...}`) at `bytes[at]` ends, `None` where none starts
/// there: its code runs to the `}` balancing its `{`.
fn interpolation_end(bytes: &[u8], at: usize, nesting: usize) -> Option<usize> {
    let from = match bytes.get(at..at + 2)? {
        b"{$" => at + 1,
        b"${" => at + 2,
        _ => return None,
    };
    if nesting == NESTING {
        return Some(bytes.len());
    }
    let mut depth = 1usize;
    let mut index = from;
    while index < bytes.len() {
        if let Some(end) = literal_end(bytes, index, nesting + 1) {
            index = end;
            continue;
        }
        match bytes[index] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            _ => {}
        }
        index += 1;
    }
    Some(bytes.len())
}

/// Whether `byte` may be in a heredoc's label.
fn label_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

/// Where the heredoc or nowdoc whose `<<<` is at `bytes[at]` ends, `None` where `<<<` opens none:
/// after its label, closing the first line whose text, past spaces and tabs, is the label and no
/// further label byte (PHP 7.3's closing marker, which code may follow). A heredoc's body
/// interpolates; a nowdoc's (`<<<'LABEL'`) does not.
fn doc_end(bytes: &[u8], at: usize, nesting: usize) -> Option<usize> {
    let blank = |from: usize| {
        from + bytes[from..]
            .iter()
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .count()
    };
    let mut index = blank(at + 3);
    let quote = match bytes.get(index) {
        Some(&quote @ (b'\'' | b'"')) => {
            index += 1;
            Some(quote)
        }
        _ => None,
    };
    let start = index;
    index += bytes[index..]
        .iter()
        .take_while(|byte| label_byte(**byte))
        .count();
    let label = &bytes[start..index];
    if label.first().is_none_or(u8::is_ascii_digit) {
        return None;
    }
    if let Some(quote) = quote {
        if bytes.get(index) != Some(&quote) {
            return None;
        }
        index += 1;
    }
    index += match bytes.get(index..) {
        Some([b'\r', b'\n', ..]) => 2,
        Some([b'\n' | b'\r', ..]) => 1,
        _ => return None,
    };
    let interpolates = quote != Some(b'\'');
    let mut line = true;
    while index < bytes.len() {
        if line {
            line = false;
            let indent = blank(index);
            let after = indent + label.len();
            if bytes.get(indent..after) == Some(label)
                && !bytes.get(after).copied().is_some_and(label_byte)
            {
                return Some(after);
            }
        }
        if bytes[index] == b'\n' {
            line = true;
            index += 1;
        } else if !interpolates {
            index += 1;
        } else if bytes[index] == b'\\' && bytes.get(index + 1) != Some(&b'\n') {
            // `\` escapes in a heredoc, but not the line end.
            index += 2;
        } else if let Some(end) = interpolation_end(bytes, index, nesting) {
            index = end;
        } else {
            index += 1;
        }
    }
    Some(bytes.len())
}

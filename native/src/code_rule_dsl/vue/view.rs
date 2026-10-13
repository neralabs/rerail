//! The JavaScript or TypeScript a Vue single-file component runs, read as one program: its
//! `<script>` and `<script setup>` blocks as @vue/compiler-sfc 3.5.43 compiles them, and the
//! expressions of its template in the scopes compiler-dom 3.5.43 gives them when it compiles the
//! template to a render function, without the render function's own code (`_ctx.` and `$setup.`
//! prefixes, `_toDisplayString`, `_createVNode`, `_renderList`, `_withCtx`).
//!
//! The program is written out as text in the grammar of the script's `lang`, the one of
//! `<script setup>` before the one of `<script>` (`ts`: TypeScript, `tsx`: TSX, none, `js` or
//! `jsx`: JavaScript; a file without a script: JavaScript; any other `lang`, such as `coffee`: no
//! program), and parsed with tree-sitter; each of its nodes maps back to the `.vue` bytes it came
//! from (`View::span`). Script content and template expressions are copied byte for byte, so their
//! nodes map exactly onto the file. The syntax around them is synthetic, each token anchored to
//! the Vue construct it stands for. The blocks are read in source order:
//!
//! | Vue | Program | Span of the synthetic syntax |
//! |---|---|---|
//! | `<script>`, `<script setup>` without `src` | `;` then the content, then `;` | the `;` marker: the start tag (rules tell blocks apart by it); the closing `;`, which ends the content's last statement where it has no `;` of its own: the empty range after the content's last non-blank byte |
//! | `<template>` in HTML (no `lang` other than `html`, no `src`) | `{ ... }` holding the template's view | the template element |
//! | `{{ E }}` | `(E);` | the interpolation |
//! | a directive with a value: `:x="E"`, `v-bind:x`, `v-bind="E"`, `v-show`, `v-html`, `v-text`, `v-model`, `v-memo`, `v-on="{...}"`, a custom `v-foo` | `(E);` | the directive attribute |
//! | `:x`, `v-bind:x`, `.x` without a value (Vue 3.4's same-name shorthand) | `(x);` (the argument camelized: copied, or written over the argument where camelizing changes it) | the directive attribute |
//! | a dynamic argument `v-x:[A]`, `:[A]`, `@[A]`, `#[A]` | `(A);` before the directive's own statement | the `[A]` |
//! | `@x="E"`, `v-on:x` whose value is a member expression or a function (compiler-core's `isMemberExpression`, `isFnExpression`) | `(E);` | the directive attribute |
//! | `@x="S"` whose value is anything else: inline statements | `(() => { S });` | the directive attribute |
//! | `v-if="C"`, then sibling `v-else-if="C2"` and `v-else` elements (only whitespace and comments between) | `if (C) { E1 } else if (C2) { E2 } else { E3 }` | the `if` statement: the first element's start to the last one's end; each block: its element; each condition's parentheses: its directive |
//! | `v-for="ALIAS in SRC"` (or `of`) on element E | `(SRC), (ALIAS) => { E };` | the statement, sequence, `(SRC)` and arrow: the element E |
//! | `v-slot="P"`, `v-slot:name="P"`, `#name="P"`, Vue 2's `slot-scope="P"` and `scope="P"` on element E | `(P) => { children of E };` | the statement and arrow: the element E |
//! | a component tag (`<MyComp>`, `<Foo.Bar>`) | `Tag;` (the tag name copied) | the tag name |
//! | text, static attributes, comments, `v-pre` elements | nothing | |
//!
//! Inside an element the view reads its component tag, then its directives in attribute order,
//! then its children. A `v-for` holds all of its element (its tag, directives and children: Vue
//! evaluates the element's own bindings, `:key` among them, once per item, with the alias in
//! scope); a slot's arrow holds only the element's children (its props belong to the parent's
//! render); a `v-if` holds the element with its `v-for`, because Vue 3 gives `v-if` priority over
//! `v-for` on one element (compiler-core's `transformIf` runs before `transformFor`; Vue 2 did the
//! opposite), so the condition cannot read the alias. The `v-for` statement's shape, an
//! `expression_statement` holding a `sequence_expression` of a parenthesized expression and an
//! arrow, never comes from template code, which is always parenthesized. The alias is copied
//! with its parentheses where it has them (`(item, index)`), so the arrow then starts at the
//! alias; otherwise (`item`, `{ a, b }`, `item, index`) synthetic parentheses anchored at the
//! element wrap it, as compiler-core's `parseFor` reads it.
//!
//! An inline handler's arrow declares nothing: Vue compiles it to `$event => (S)`, but no byte of
//! the file declares `$event`, so the view leaves Vue's implicit variable an unresolved read rather
//! than a binding no rule could report at. A condition-less `v-if` or `v-else-if` reads `true`, as
//! compiler-core substitutes for the missing expression; a `v-else-if` with no `v-if` before it
//! is a plain directive (compiler-core reads its condition, then reports it and renders the
//! element unconditionally). A `v-pre` element is not compiled at all, its directives before and
//! after the `v-pre` included. A `<script>` or `<style>` inside the template is no element Vue
//! renders (`X_IGNORED_SIDE_EFFECT_TAG`). A tag is a component as compiler-core's `isComponent`
//! decides with compiler-dom's options: not a native HTML, SVG or MathML tag, not `<slot>`,
//! `<template>`, `<component>` or a built-in (`Teleport`, `Suspense`, `KeepAlive`,
//! `BaseTransition`, `Transition`, `TransitionGroup`); the view reads it where it is a JavaScript
//! identifier or a dotted chain of them (a kebab-case tag names no binding the program can show).
//! A tag or same-name shorthand that is a reserved word (`:class`) reads nothing: Vue reads it as
//! a property of the component instance (`_ctx.class`), which no binding can be.
//!
//! compiler-core decodes character references in attribute values and interpolations before it
//! reads them as expressions (`v-if="a &gt; 0"` is `a > 0`): the view writes each reference of a
//! template expression as the character it stands for, a synthetic token over the reference.
//! What the view does not read: tree-sitter-vue (ce8011a) reads two value-less shorthands in a
//! row (`:a :b`, `@a :b`) as one attribute, of which the view reads the first.

use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::vue::sfc;
use crate::code_rule_dsl::vue::template::{self, Arg, Directive, Parse};
use ast_grep_language::LanguageExt;
use std::borrow::Cow;
use std::ops::Range;
use tree_sitter::{Node, Parser, Tree};

/// The foundation moves this map to embed::SegmentMap; Vue's view adopts it on merge.
/// One stretch of the program's text and the `.vue` bytes it stands for.
#[derive(Clone, Debug)]
struct Segment {
    /// Where it starts in the program.
    at: usize,
    /// Where it ends in the program.
    end: usize,
    /// The `.vue` bytes it stands for.
    anchor: Range<usize>,
    /// Copied from the file: each byte maps to the byte at the same offset of `anchor`.
    copied: bool,
}

/// The program a Vue single-file component embeds and its map back to the file.
pub(crate) struct View {
    /// The program's text in order, never empty.
    segments: Vec<Segment>,
    tree: Tree,
    dialect: GrammarDialect,
    #[cfg(test)]
    text: String,
}

impl View {
    /// The program the SFC whose tree-sitter-vue tree is `root` embeds; `None` where its script
    /// is in a language other than JavaScript or TypeScript, or nothing it holds is read.
    pub(crate) fn build(root: Node<'_>, source: &str) -> Option<Self> {
        let blocks = sfc::blocks(root, source);
        let dialect = sfc::script_dialect(&blocks)?;
        let language = dialect.get_ts_language();
        let mut handlers = Parser::new();
        handlers.set_language(&language).ok()?;
        let mut emitter = Emitter {
            source,
            out: String::new(),
            segments: Vec::new(),
            handlers,
        };
        for block in &blocks {
            match block.name {
                "script" if block.src.is_none() => emitter.script(block),
                "template"
                    if block.src.is_none() && block.lang.is_none_or(|lang| lang == "html") =>
                {
                    emitter.template(block.node);
                }
                _ => {}
            }
        }
        if emitter.segments.is_empty() {
            return None;
        }
        let mut parser = Parser::new();
        parser.set_language(&language).ok()?;
        let tree = parser.parse(&emitter.out, None)?;
        Some(Self {
            segments: emitter.segments,
            tree,
            dialect,
            #[cfg(test)]
            text: emitter.out,
        })
    }

    /// The grammar the program is written in.
    pub(crate) fn dialect(&self) -> GrammarDialect {
        self.dialect
    }

    /// The program's root node.
    pub(crate) fn root(&self) -> Node<'_> {
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

    /// The file position a node starting at program byte `at` starts at.
    fn start(&self, at: usize) -> usize {
        let segment = self.segment(at);
        if segment.copied {
            segment.anchor.start + (at.clamp(segment.at, segment.end) - segment.at)
        } else {
            segment.anchor.start
        }
    }

    /// The file position a node ending at program byte `at` ends at.
    fn end(&self, at: usize) -> usize {
        let segment = self.segment(at.saturating_sub(1));
        if segment.copied {
            segment.anchor.start + (at.clamp(segment.at, segment.end) - segment.at)
        } else {
            segment.anchor.end
        }
    }

    /// The `.vue` bytes a node of the program stands for.
    pub(crate) fn span(&self, node: Node<'_>) -> Range<usize> {
        let range = node.byte_range();
        if range.is_empty() {
            let at = self.start(range.start);
            return at..at;
        }
        let start = self.start(range.start);
        start..self.end(range.end).max(start)
    }

    /// Whether a node is part of a placeholder the view does not keep: never, since every token
    /// the view writes stands for script, a template expression or the Vue syntax around one.
    pub(crate) fn omitted(&self, _node: Node<'_>) -> bool {
        false
    }

    /// The file ranges the program copies byte for byte, in program order.
    #[cfg(test)]
    pub(crate) fn copied(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.segments
            .iter()
            .filter(|segment| segment.copied)
            .map(|segment| segment.anchor.clone())
    }

    /// The program's text.
    #[cfg(test)]
    pub(crate) fn text(&self) -> &str {
        &self.text
    }
}

/// Native tags of compiler-dom 3.5.43's `isNativeTag` (@vue/shared's `HTML_TAGS`, `SVG_TAGS` and
/// `MATH_TAGS`) that are JavaScript identifiers, sorted bytewise for binary search.
#[rustfmt::skip]
const NATIVE_TAGS: &[&str] = &[
    "a", "abbr", "address", "animate", "animateMotion", "animateTransform", "annotation", "area",
    "article", "aside", "audio", "b", "base", "bdi", "bdo", "blockquote", "body", "br", "button",
    "canvas", "caption", "circle", "cite", "clipPath", "code", "col", "colgroup", "data",
    "datalist", "dd", "defs", "del", "desc", "details", "dfn", "dialog", "discard", "div", "dl",
    "dt", "ellipse", "em", "embed", "feBlend", "feColorMatrix", "feComponentTransfer",
    "feComposite", "feConvolveMatrix", "feDiffuseLighting", "feDisplacementMap", "feDistantLight",
    "feDropShadow", "feFlood", "feFuncA", "feFuncB", "feFuncG", "feFuncR", "feGaussianBlur",
    "feImage", "feMerge", "feMergeNode", "feMorphology", "feOffset", "fePointLight",
    "feSpecularLighting", "feSpotLight", "feTile", "feTurbulence", "fieldset", "figcaption",
    "figure", "filter", "footer", "foreignObject", "form", "g", "h1", "h2", "h3", "h4", "h5", "h6",
    "hatch", "hatchpath", "head", "header", "hgroup", "hr", "html", "i", "iframe", "image", "img",
    "input", "ins", "kbd", "label", "legend", "li", "line", "linearGradient", "link", "maction",
    "main", "maligngroup", "malignmark", "map", "mark", "marker", "mask", "math", "menclose",
    "menu", "merror", "mesh", "meshgradient", "meshpatch", "meshrow", "meta", "metadata", "meter",
    "mfenced", "mfrac", "mfraction", "mglyph", "mi", "mlabeledtr", "mlongdiv", "mmultiscripts",
    "mn", "mo", "mover", "mpadded", "mpath", "mphantom", "mprescripts", "mroot", "mrow", "ms",
    "mscarries", "mscarry", "msgroup", "msline", "mspace", "msqrt", "msrow", "mstack", "mstyle",
    "msub", "msubsup", "msup", "mtable", "mtd", "mtext", "mtr", "munder", "munderover", "nav",
    "none", "noscript", "object", "ol", "optgroup", "option", "output", "p", "param", "path",
    "pattern", "picture", "polygon", "polyline", "pre", "progress", "q", "radialGradient", "rect",
    "rp", "rt", "ruby", "s", "samp", "script", "section", "select", "semantics", "set", "small",
    "solidcolor", "source", "span", "stop", "strong", "style", "sub", "summary", "sup", "svg",
    "switch", "symbol", "table", "tbody", "td", "template", "text", "textPath", "textarea", "tfoot",
    "th", "thead", "time", "title", "tr", "track", "tspan", "u", "ul", "unknown", "use", "var",
    "video", "view", "wbr",
];

/// Tags compiler-core 3.5.43 compiles itself rather than as a component of the program: `<slot>`
/// outlets, `<component>` (dynamic, `isComponentTag`), its core components (`isCoreComponent`)
/// and compiler-dom's built-ins (`isBuiltInComponent`), in their identifier spellings.
const BUILT_IN_TAGS: &[&str] = &[
    "slot",
    "component",
    "Component",
    "Teleport",
    "teleport",
    "Suspense",
    "suspense",
    "KeepAlive",
    "BaseTransition",
    "Transition",
    "transition",
    "TransitionGroup",
];

/// ECMAScript's reserved words (strict mode and modules included): no binding has one as its
/// name, so a tag or same-name shorthand spelling one reads none.
const RESERVED_WORDS: &[&str] = &[
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "let",
    "new",
    "null",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

/// Whether `text` is a JavaScript identifier name.
fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|first| first == '_' || first == '$' || first.is_alphabetic())
        && chars.all(|rest| rest == '_' || rest == '$' || rest.is_alphanumeric())
}

/// Whether `text` is an identifier a program can read as a variable: not a reserved word.
fn is_variable(text: &str) -> bool {
    is_identifier(text) && !RESERVED_WORDS.contains(&text)
}

/// Whether the view reads a tag as a component: Vue compiles it to one, and the tag is a
/// variable or a member chain on one (a namespaced component, `<Form.Item>`).
fn is_component(tag: &str) -> bool {
    let mut parts = tag.split('.');
    NATIVE_TAGS.binary_search(&tag).is_err()
        && !BUILT_IN_TAGS.contains(&tag)
        && parts.next().is_some_and(is_variable)
        && parts.all(is_identifier)
}

/// `range` of `source` without its surrounding whitespace, `None` where nothing else is left.
fn trimmed(source: &str, range: Range<usize>) -> Option<Range<usize>> {
    let text = &source[range.clone()];
    let start = range.start + (text.len() - text.trim_start().len());
    let end = range.end - (text.len() - text.trim_end().len());
    (start < end).then_some(start..end)
}

/// The character an HTML character reference at the start of `text` stands for, and the
/// reference's length: the named references an expression spells its operators and quotes with
/// (`&lt;`, `&gt;`, `&amp;`, `&quot;`, `&apos;`, `&nbsp;`) and numeric ones (`&#60;`, `&#x3C;`),
/// each ended by its `;`. compiler-core 3.5.43's tokenizer decodes them in attribute values, and
/// its `decodeHTML` in interpolations, before it reads either as an expression.
fn character_reference(text: &str) -> Option<(char, usize)> {
    /// Longer than any reference the view decodes (`&#x10FFFF;`).
    const LONGEST: usize = 10;
    let semicolon = text.bytes().take(LONGEST).position(|byte| byte == b';')?;
    let name = text.get(1..semicolon).filter(|_| text.starts_with('&'))?;
    let char = match name {
        "lt" => '<',
        "gt" => '>',
        "amp" => '&',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        _ => {
            let number = name.strip_prefix('#')?;
            let code = match number.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => number.parse().ok()?,
            };
            char::from_u32(code)?
        }
    };
    Some((char, semicolon + 1))
}

/// `text` with its character references (`character_reference`) decoded, as Vue reads a
/// template expression.
fn decoded(text: &str) -> Cow<'_, str> {
    if !text.contains('&') {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let (char, length) = character_reference(rest).unwrap_or(('&', 1));
        out.push(char);
        rest = &rest[length..];
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// @vue/shared's `camelize`: each `-` and the character after it become that character in upper
/// case.
fn camelize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(char) = chars.next() {
        match chars.peek() {
            Some(&next) if char == '-' && (next.is_ascii_alphanumeric() || next == '_') => {
                out.push(next.to_ascii_uppercase());
                chars.next();
            }
            _ => out.push(char),
        }
    }
    out
}

/// Which branch of a `v-if` chain a directive makes its element.
#[derive(Clone, Copy, PartialEq)]
enum Branch {
    If,
    ElseIf,
    Else,
}

/// An element of the template, its start tag read once.
struct Element<'t> {
    node: Node<'t>,
    /// The tag name, where the tag is a component the view reads (`Tag;`).
    component: Option<Node<'t>>,
    /// Its directives in attribute order.
    directives: Vec<Directive<'t>>,
    /// Its first `v-if`, `v-else-if` or `v-else`, by index into `directives`.
    branch: Option<(Branch, usize)>,
    /// Its first `v-for`, by index into `directives`.
    for_loop: Option<usize>,
    /// Its first `v-slot`, `slot-scope` or `scope`, by index into `directives`.
    slot: Option<usize>,
}

impl<'t> Element<'t> {
    /// The element `node` (an `element` or `template_element`), `None` for a `v-pre` one, which
    /// Vue does not compile.
    fn read(node: Node<'t>, source: &'t str) -> Option<Self> {
        let mut element = Element {
            node,
            component: None,
            directives: Vec::new(),
            branch: None,
            for_loop: None,
            slot: None,
        };
        let mut cursor = node.walk();
        let Some(start) = node
            .named_children(&mut cursor)
            .find(|child| matches!(child.kind(), "start_tag" | "self_closing_tag"))
        else {
            return Some(element);
        };
        let mut cursor = start.walk();
        for attribute in start.named_children(&mut cursor) {
            if attribute.kind() == "tag_name" {
                if is_component(&source[attribute.byte_range()]) {
                    element.component = Some(attribute);
                }
                continue;
            }
            let Some(directive) = Directive::read(attribute, source) else {
                continue;
            };
            let index = element.directives.len();
            let branch = match directive.name {
                "pre" => return None,
                "if" => Some(Branch::If),
                "else-if" => Some(Branch::ElseIf),
                "else" => Some(Branch::Else),
                _ => None,
            };
            if let Some(branch) = branch {
                element.branch = element.branch.or(Some((branch, index)));
            } else if directive.name == "for" {
                element.for_loop = element.for_loop.or(Some(index));
            } else if matches!(directive.name, "slot" | "slot-scope" | "scope") {
                element.slot = element.slot.or(Some(index));
            }
            element.directives.push(directive);
        }
        Some(element)
    }
}

/// A child of the template or of an element, as the view reads it.
enum Child<'t> {
    /// `{{ E }}` and the range of `E`.
    Interpolation(Node<'t>, Range<usize>),
    /// An element Vue compiles (no `v-pre`).
    Element(Element<'t>),
    /// A comment or whitespace: a `v-if` chain continues across it.
    Gap,
    /// Anything else (text, an empty interpolation, a `v-pre` element, a tag): nothing the view
    /// reads, and the end of a `v-if` chain.
    Other,
}

impl<'t> Child<'t> {
    /// How the view reads the template node `node`.
    fn read(node: Node<'t>, source: &'t str) -> Self {
        match node.kind() {
            "interpolation" => node
                .named_child(0)
                .and_then(|expression| trimmed(source, expression.byte_range()))
                .map_or(Child::Other, |expression| {
                    Child::Interpolation(node, expression)
                }),
            "element" | "template_element" => {
                Element::read(node, source).map_or(Child::Other, Child::Element)
            }
            "comment" => Child::Gap,
            "text" if source[node.byte_range()].trim().is_empty() => Child::Gap,
            _ => Child::Other,
        }
    }
}

/// Writes the program and its segments.
struct Emitter<'t> {
    source: &'t str,
    out: String,
    segments: Vec<Segment>,
    /// A parser in the program's grammar, for `template::handler_with`.
    handlers: Parser,
}

impl<'t> Emitter<'t> {
    /// Appends `text`, standing for the file bytes `anchor` (byte for byte where `copied`); empty
    /// text writes no segment.
    fn push(&mut self, text: &str, anchor: Range<usize>, copied: bool) {
        if text.is_empty() {
            return;
        }
        let at = self.out.len();
        self.out.push_str(text);
        self.segments.push(Segment {
            at,
            end: self.out.len(),
            anchor,
            copied,
        });
    }

    /// Writes syntax Vue's compiler wraps the file's code in, standing for the bytes `anchor`.
    fn emit(&mut self, text: &str, anchor: Range<usize>) {
        self.push(text, anchor, false);
    }

    /// Copies file bytes as program text.
    fn copy(&mut self, range: Range<usize>) {
        let source = self.source;
        self.push(&source[range.clone()], range, true);
    }

    /// Copies a template expression, each character reference (`character_reference`) written
    /// as the character Vue reads, standing for the reference.
    fn code(&mut self, range: Range<usize>) {
        let source = self.source;
        let mut from = range.start;
        for (offset, _) in source[range.clone()].match_indices('&') {
            let at = range.start + offset;
            if at < from {
                continue;
            }
            let Some((char, length)) = character_reference(&source[at..range.end]) else {
                continue;
            };
            self.copy(from..at);
            self.emit(char.encode_utf8(&mut [0; 4]), at..at + length);
            from = at + length;
        }
        self.copy(from..range.end);
    }

    /// A `<script>` block: a `;` marker over its start tag, its content, then a `;` so that no
    /// statement runs on into what follows. The closing `;` may end the content's last statement,
    /// so it stands for the empty range after the content's last non-blank byte.
    fn script(&mut self, block: &sfc::Block<'_>) {
        let marker = if self.out.is_empty() { ";" } else { "\n;" };
        self.emit(marker, block.node.start_byte()..block.content.start);
        self.copy(block.content.clone());
        let content = &self.source[block.content.clone()];
        let end = block.content.start + content.trim_end().len();
        self.emit("\n;", end..end);
    }

    /// The `<template>` block `node`: `{ ... }` over it.
    fn template(&mut self, node: Node<'t>) {
        let open = if self.out.is_empty() { "{" } else { "\n{" };
        self.emit(open, node.byte_range());
        self.children(node);
        self.emit(" }", node.byte_range());
    }

    /// The children of `parent` in order, each `v-if` chain read as one statement.
    fn children(&mut self, parent: Node<'t>) {
        let source = self.source;
        let mut cursor = parent.walk();
        let children: Vec<Child<'t>> = parent
            .named_children(&mut cursor)
            .map(|node| Child::read(node, source))
            .collect();
        let mut index = 0;
        while index < children.len() {
            match &children[index] {
                Child::Interpolation(node, expression) => {
                    self.expression(expression.clone(), node.byte_range());
                }
                Child::Element(element) => match element.branch {
                    Some((Branch::If, _)) => {
                        index = self.chain(&children, index);
                        continue;
                    }
                    _ => self.element(element, false),
                },
                Child::Gap | Child::Other => {}
            }
            index += 1;
        }
    }

    /// The `v-if` chain starting at `children[first]`, a `v-if` element, and its `v-else-if` and
    /// `v-else` siblings; returns the index after its last branch.
    fn chain(&mut self, children: &[Child<'t>], first: usize) -> usize {
        let mut next = first + 1;
        let mut index = first;
        let mut opening = true;
        while let Some(child) = children.get(index) {
            index += 1;
            let element = match child {
                Child::Gap => continue,
                Child::Element(element) => element,
                _ => break,
            };
            let Some((branch, directive)) = element.branch else {
                break;
            };
            if opening != (branch == Branch::If) {
                break;
            }
            let anchor = element.node.byte_range();
            match branch {
                Branch::If => self.emit(" if ", anchor.clone()),
                Branch::ElseIf => self.emit(" else if ", anchor.clone()),
                Branch::Else => self.emit(" else", anchor.clone()),
            }
            if branch != Branch::Else {
                self.condition(&element.directives[directive]);
            }
            self.emit(" {", anchor.clone());
            self.element(element, true);
            self.emit(" }", anchor);
            opening = false;
            next = index;
            if branch == Branch::Else {
                break;
            }
        }
        next
    }

    /// A branch's condition in parentheses over its directive: `true` where it has none.
    fn condition(&mut self, directive: &Directive<'t>) {
        let anchor = directive.node.byte_range();
        self.emit("(", anchor.clone());
        match directive
            .value
            .clone()
            .and_then(|value| trimmed(self.source, value))
        {
            Some(condition) => self.code(condition),
            None => self.emit("true", anchor.clone()),
        }
        self.emit(")", anchor);
    }

    /// An element in its `v-for`, if it has one Vue can parse; `chained` where a `v-if` chain
    /// has read its branch directive.
    fn element(&mut self, element: &Element<'t>, chained: bool) {
        let source = self.source;
        let parsed = element.for_loop.and_then(|index| {
            let value = trimmed(source, element.directives[index].value.clone()?)?;
            template::parse_for(source, value)
        });
        let Some(parsed) = parsed else {
            self.contents(element, chained);
            return;
        };
        let anchor = element.node.byte_range();
        self.emit(" (", anchor.clone());
        if let Some(iterable) = trimmed(source, parsed.source.clone()) {
            self.code(iterable);
        }
        self.emit("), ", anchor.clone());
        let alias = trimmed(source, parsed.alias.clone());
        if parsed.parenthesized
            && let Some(alias) = alias
        {
            self.code(alias);
        } else {
            self.emit("(", anchor.clone());
            if let Some(alias) = alias {
                self.code(alias);
            }
            self.emit(")", anchor.clone());
        }
        self.emit(" => {", anchor.clone());
        self.contents(element, chained);
        self.emit(" };", anchor);
    }

    /// An element's component tag, its directives, then its children, in its slot's arrow if it
    /// has slot parameters.
    fn contents(&mut self, element: &Element<'t>, chained: bool) {
        if let Some(tag) = element.component {
            let anchor = tag.byte_range();
            self.emit(" ", anchor.clone());
            self.copy(anchor.clone());
            self.emit(";", anchor);
        }
        let read = element.branch.filter(|_| chained).map(|(_, index)| index);
        for (index, directive) in element.directives.iter().enumerate() {
            if Some(index) != read {
                self.directive(directive);
            }
        }
        let source = self.source;
        let parameters = element.slot.and_then(|index| {
            let value = element.directives[index].value.clone()?;
            trimmed(source, value)
        });
        let Some(parameters) = parameters else {
            self.children(element.node);
            return;
        };
        let anchor = element.node.byte_range();
        self.emit(" (", anchor.clone());
        self.code(parameters);
        self.emit(") => {", anchor.clone());
        self.children(element.node);
        self.emit(" };", anchor);
    }

    /// `(E);` over `anchor`.
    fn expression(&mut self, expression: Range<usize>, anchor: Range<usize>) {
        self.emit(" (", anchor.clone());
        self.code(expression);
        self.emit(");", anchor);
    }

    /// A directive's dynamic argument, then its value, where Vue reads it as an expression.
    fn directive(&mut self, directive: &Directive<'t>) {
        let source = self.source;
        if let Arg::Dynamic(inner) = &directive.arg
            && let Some(argument) = trimmed(source, inner.clone())
        {
            let mut cursor = directive.node.walk();
            let anchor = directive
                .node
                .named_children(&mut cursor)
                .find(|child| child.kind() == "dynamic_directive_value")
                .map_or_else(|| directive.node.byte_range(), |node| node.byte_range());
            self.expression(argument, anchor);
        }
        // Their values are the structure around their element, which `element` and `contents`
        // write.
        if matches!(directive.name, "for" | "slot" | "slot-scope" | "scope") {
            return;
        }
        let anchor = directive.node.byte_range();
        let value = directive
            .value
            .clone()
            .and_then(|value| trimmed(source, value));
        match (value, &directive.arg) {
            (Some(value), Arg::Static(_) | Arg::Dynamic(_))
                if directive.name == "on"
                    && matches!(
                        template::handler_with(
                            &mut self.handlers,
                            &decoded(&source[value.clone()])
                        ),
                        Parse::Statements
                    ) =>
            {
                self.emit(" (() => { ", anchor.clone());
                self.code(value);
                self.emit(" });", anchor);
            }
            (Some(value), _) => self.expression(value, anchor),
            (None, Arg::Static(argument)) if directive.name == "bind" => {
                self.same_name(argument.clone(), anchor);
            }
            (None, _) => {}
        }
    }

    /// Vue 3.4's same-name shorthand, `:x` for `:x="x"`: compiler-core's `transformVBindShorthand`
    /// reads the camelized argument where it starts as an identifier does.
    fn same_name(&mut self, argument: Range<usize>, anchor: Range<usize>) {
        let text = &self.source[argument.clone()];
        let name = camelize(text);
        if !is_variable(&name) {
            return;
        }
        self.emit(" (", anchor.clone());
        if name == text {
            self.copy(argument);
        } else {
            self.emit(&name, argument);
        }
        self.emit(");", anchor);
    }
}

#[cfg(test)]
mod tests {
    use super::{NATIVE_TAGS, View};
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::vue::grammar;
    use crate::code_rule_dsl::vue::template::Directive;
    use std::ops::Range;
    use std::path::{Path, PathBuf};
    use tree_sitter::{Node, Parser, Tree};

    fn parse(source: &str) -> Tree {
        let mut parser = Parser::new();
        parser.set_language(&grammar()).expect("Vue grammar");
        parser.parse(source, None).expect("Vue tree")
    }

    /// Every named node under `node`, in document order.
    fn named<'a>(node: Node<'a>, out: &mut Vec<Node<'a>>) {
        out.push(node);
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            named(child, out);
        }
    }

    /// Checks that each copied segment's program text is the file's text it stands for.
    fn check_copies(view: &View, source: &str) -> Result<(), String> {
        for segment in view.segments.iter().filter(|segment| segment.copied) {
            let program = &view.text[segment.at..segment.end];
            let file = &source[segment.anchor.clone()];
            if program != file {
                return Err(format!("copied {program:?} stands for {file:?}"));
            }
        }
        Ok(())
    }

    /// The `v-for` and directive spans the view must give statements: each `v-for` element's
    /// range, and each directive's range that reads its value as one expression statement.
    fn template_spans(
        node: Node<'_>,
        source: &str,
        loops: &mut Vec<Range<usize>>,
        directives: &mut Vec<Range<usize>>,
    ) {
        if matches!(node.kind(), "element" | "template_element") {
            let mut cursor = node.walk();
            let start = node
                .named_children(&mut cursor)
                .find(|child| matches!(child.kind(), "start_tag" | "self_closing_tag"));
            let read: Vec<Directive<'_>> = start
                .map(|start| {
                    let mut cursor = start.walk();
                    start
                        .named_children(&mut cursor)
                        .filter_map(|attribute| Directive::read(attribute, source))
                        .collect()
                })
                .unwrap_or_default();
            if read.iter().any(|directive| directive.name == "pre") {
                return;
            }
            for directive in &read {
                let has_value = directive
                    .value
                    .clone()
                    .is_some_and(|value| !source[value].trim().is_empty());
                match directive.name {
                    "for" if has_value => loops.push(node.byte_range()),
                    "for" | "slot" | "slot-scope" | "scope" | "if" | "else-if" | "else" => {}
                    _ if has_value => directives.push(directive.node.byte_range()),
                    _ => {}
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            template_spans(child, source, loops, directives);
        }
    }

    /// The view of `source`, after checking what every view keeps: a program without syntax
    /// errors, copies that match the file, identifiers whose span is their text, a `v-for`
    /// statement over each `v-for` element and a statement over each expression directive.
    fn view(source: &str) -> View {
        checked(source, true)
    }

    /// `view`, checking identifiers only where `identifiers` (the view writes a camelized
    /// same-name shorthand's identifier itself).
    fn checked(source: &str, identifiers: bool) -> View {
        let tree = parse(source);
        assert!(!tree.root_node().has_error(), "Vue tree of {source:?}");
        let view = View::build(tree.root_node(), source).expect("a program");
        let text = view.text();
        assert!(!view.root().has_error(), "syntax error in {text:?}");
        check_copies(&view, source).expect("copies");
        let mut nodes = Vec::new();
        named(view.root(), &mut nodes);
        let mut statements = Vec::new();
        let mut loops = Vec::new();
        for node in nodes {
            assert!(!view.omitted(node), "the view omits nothing");
            if identifiers && node.kind().ends_with("identifier") {
                assert_eq!(
                    &source[view.span(node)],
                    &text[node.byte_range()],
                    "identifier in {text:?}"
                );
            }
            if node.kind() != "expression_statement" {
                continue;
            }
            statements.push(view.span(node));
            let mut cursor = node.walk();
            let kids: Vec<_> = node.named_children(&mut cursor).collect();
            if let [sequence] = kids.as_slice()
                && sequence.kind() == "sequence_expression"
            {
                let mut cursor = sequence.walk();
                let parts: Vec<_> = sequence
                    .named_children(&mut cursor)
                    .map(|part| part.kind())
                    .collect();
                assert_eq!(
                    parts,
                    ["parenthesized_expression", "arrow_function"],
                    "{text:?}"
                );
                loops.push(view.span(node));
            }
        }
        let mut expected_loops = Vec::new();
        let mut directives = Vec::new();
        template_spans(
            tree.root_node(),
            source,
            &mut expected_loops,
            &mut directives,
        );
        assert_eq!(loops, expected_loops, "v-for statements of {text:?}");
        for directive in directives {
            assert!(
                statements.contains(&directive),
                "no statement over {:?} in {text:?}",
                &source[directive]
            );
        }
        view
    }

    #[test]
    fn native_tags_are_sorted() {
        assert!(NATIVE_TAGS.windows(2).all(|pair| pair[0] < pair[1]));
    }

    /// The spans of the program's nodes of `kind`, in document order.
    fn spans(view: &View, kind: &str) -> Vec<Range<usize>> {
        let mut nodes = Vec::new();
        named(view.root(), &mut nodes);
        nodes
            .into_iter()
            .filter(|node| node.kind() == kind)
            .map(|node| view.span(node))
            .collect()
    }

    /// The byte range of the first occurrence of `text` in `source`.
    fn find(source: &str, text: &str) -> Range<usize> {
        let start = source.find(text).expect("text in source");
        start..start + text.len()
    }

    #[test]
    fn interpolation() {
        let source = "<template><p>{{ message }}</p></template>";
        let view = view(source);
        assert_eq!(view.text(), "{ (message); }");
        assert_eq!(view.dialect(), GrammarDialect::JavaScript);
        assert_eq!(spans(&view, "statement_block"), vec![0..source.len()]);
        assert_eq!(
            spans(&view, "expression_statement"),
            [find(source, "{{ message }}")]
        );
    }

    #[test]
    fn bindings_and_handlers() {
        let source =
            r#"<template><input :a="x" @click="go" @input="n++" v-on="{ a: b }"></template>"#;
        let view = view(source);
        assert_eq!(view.text(), "{ (x); (go); (() => { n++ }); ({ a: b }); }");
        assert_eq!(
            spans(&view, "arrow_function"),
            [find(source, r#"@input="n++""#)]
        );
    }

    #[test]
    fn handler_object_and_statements() {
        // `v-on` without an argument is an object of handlers, never statements; with one, a
        // call is statements and a function or member is the handler itself. `$event` is no
        // parameter of an inline handler's arrow.
        let source = r#"<template><a v-on="go()" @x="go($event)" @y="(e) => go(e)" @z="a.b" @w="list.map(f => f)"></a></template>"#;
        let view = view(source);
        assert_eq!(
            view.text(),
            "{ (go()); (() => { go($event) }); ((e) => go(e)); (a.b); (() => { list.map(f => f) }); }"
        );
    }

    #[test]
    fn same_name_shorthand() {
        // tree-sitter-vue reads `:a :b` without values as one attribute, so they are apart here.
        let source = r#"<template><a :href class="c" :aria-label class :class></a></template>"#;
        let view = checked(source, false);
        assert_eq!(view.text(), "{ (href); (ariaLabel); }");
        let synthetic = spans(&view, "identifier")[1].clone();
        assert_eq!(&source[synthetic], "aria-label");
    }

    #[test]
    fn character_references() {
        let source =
            r#"<template><p v-if="a &gt; 0 &amp;&amp; b">{{ c &lt; d &#38; e }}</p></template>"#;
        let view = view(source);
        assert_eq!(view.text(), "{ if (a > 0 && b) { (c < d & e); } }");
    }

    #[test]
    fn if_chain_across_a_comment() {
        let source = "<template>\n  <p v-if=\"a\">1</p>\n  <!-- c -->\n  <p v-else-if=\"b\">2</p>\n  <p v-else>3</p>\n  <p v-else>4</p>\n</template>";
        let view = view(source);
        assert_eq!(view.text(), "{ if (a) { } else if (b) { } else { } }");
        let first = find(source, "<p v-if");
        let last = find(source, "<p v-else>3</p>");
        assert_eq!(
            spans(&view, "if_statement"),
            [
                first.start..last.end,
                find(source, "<p v-else-if").start..last.end
            ]
        );
        assert_eq!(
            spans(&view, "parenthesized_expression"),
            [
                find(source, r#"v-if="a""#),
                find(source, r#"v-else-if="b""#)
            ]
        );
        assert_eq!(
            spans(&view, "statement_block")[1..],
            [
                find(source, "<p v-if=\"a\">1</p>"),
                find(source, "<p v-else-if=\"b\">2</p>"),
                last,
            ]
        );
    }

    #[test]
    fn text_ends_an_if_chain() {
        let source =
            r#"<template><p v-if="a"></p> text <p v-else-if="b"></p><p v-else></p></template>"#;
        assert_eq!(view(source).text(), "{ if (a) { } (b); }");
    }

    #[test]
    fn condition_less_if_reads_true() {
        let source = r#"<template><p v-if></p><p v-else-if=" "></p></template>"#;
        assert_eq!(view(source).text(), "{ if (true) { } else if (true) { } }");
    }

    #[test]
    fn for_aliases() {
        let source = r#"<template><li v-for="item in items">{{ item }}</li><li v-for="(item, index) of items" :key="index">{{ item }}</li><li v-for="{ id, name } in rows">{{ id }}{{ name }}</li><li v-for="value, key in object"></li></template>"#;
        let view = view(source);
        assert_eq!(
            view.text(),
            "{ (items), (item) => { (item); }; (items), (item, index) => { (index); (item); }; \
             (rows), ({ id, name }) => { (id); (name); }; (object), (value, key) => { }; }"
        );
        assert_eq!(
            spans(&view, "formal_parameters")[1],
            find(source, "(item, index)")
        );
    }

    #[test]
    fn if_wraps_for_on_one_element() {
        let source =
            r#"<template><li v-for="item in items" v-if="show">{{ item }}</li></template>"#;
        let view = view(source);
        assert_eq!(
            view.text(),
            "{ if (show) { (items), (item) => { (item); }; } }"
        );
        let element = find(source, "<li v-for");
        assert_eq!(
            spans(&view, "if_statement"),
            vec![element.start..source.len() - "</template>".len()]
        );
    }

    #[test]
    fn for_in_a_chain_branch() {
        let source = r#"<template><Row v-if="ok" v-for="item in items" :item>{{ item }}</Row><p v-else>{{ none }}</p></template>"#;
        let view = view(source);
        assert_eq!(
            view.text(),
            "{ if (ok) { (items), (item) => { Row; (item); (item); }; } else { (none); } }"
        );
    }

    #[test]
    fn pre_in_a_chain() {
        let source = r#"<template><p v-if="a"></p><p v-else v-pre>{{ b }}</p><p v-else-if="c"></p></template>"#;
        assert_eq!(view(source).text(), "{ if (a) { } (c); }");
    }

    #[test]
    fn component_slot() {
        let source = r#"<template><Comp v-slot="{ item }">{{ item }}</Comp></template>"#;
        let view = view(source);
        assert_eq!(view.text(), "{ Comp; ({ item }) => { (item); }; }");
        let element = find(source, r#"<Comp v-slot="{ item }">{{ item }}</Comp>"#);
        assert_eq!(
            spans(&view, "arrow_function"),
            std::slice::from_ref(&element)
        );
        assert_eq!(
            spans(&view, "expression_statement"),
            [find(source, "Comp"), element, find(source, "{{ item }}")]
        );
    }

    #[test]
    fn template_slot() {
        let source = r#"<template><Table><template #row="r">{{ r }}</template></Table></template>"#;
        let view = view(source);
        assert_eq!(view.text(), "{ Table; (r) => { (r); }; }");
        assert_eq!(
            spans(&view, "arrow_function"),
            [find(source, r#"<template #row="r">{{ r }}</template>"#)]
        );
    }

    #[test]
    fn dynamic_argument() {
        let source = r#"<template><a :[k]="v"></a><Comp><template #[name]="p">{{ p }}</template></Comp></template>"#;
        let view = view(source);
        assert_eq!(view.text(), "{ (k); (v); Comp; (name); (p) => { (p); }; }");
        let statements = spans(&view, "expression_statement");
        assert_eq!(statements[0], find(source, "[k]"));
        assert_eq!(statements[1], find(source, r#":[k]="v""#));
        assert_eq!(statements[3], find(source, "[name]"));
    }

    #[test]
    fn pre_subtree() {
        let source = r#"<template><div v-pre :a="x">{{ y }}<p :b="z"></p></div>{{ w }}</template>"#;
        let view = view(source);
        assert_eq!(view.text(), "{ (w); }");
        let pre = find(source, r#"<div v-pre :a="x">{{ y }}<p :b="z"></p></div>"#);
        assert!(
            view.copied()
                .all(|copy| copy.start >= pre.end || copy.end <= pre.start)
        );
    }

    #[test]
    fn template_first_with_two_scripts() {
        let source = "<template><p>{{ a + b }}</p></template>\n<script>\nexport const a = 1\n</script>\n<script setup lang=\"ts\">\nconst b: number = 2\n</script>\n<style>p { color: red }</style>\n";
        let view = view(source);
        assert_eq!(view.dialect(), GrammarDialect::TypeScript);
        assert_eq!(
            view.text(),
            "{ (a + b); }\n;\nexport const a = 1\n\n;\n;\nconst b: number = 2\n\n;"
        );
        // The closing `;` ends the last statement where it has none, at its last byte.
        assert_eq!(
            spans(&view, "empty_statement"),
            [
                find(source, "<script>"),
                find(source, "<script setup lang=\"ts\">")
            ]
        );
        assert_eq!(
            spans(&view, "lexical_declaration"),
            [
                find(source, "const a = 1"),
                find(source, "const b: number = 2")
            ]
        );
    }

    #[test]
    fn script_dialects() {
        let coffee = "<template><p>{{ a }}</p></template><script lang=\"coffee\">a = 1</script>";
        assert!(View::build(parse(coffee).root_node(), coffee).is_none());
        let template_only = "<template><p>{{ a }}</p></template>";
        assert_eq!(view(template_only).dialect(), GrammarDialect::JavaScript);
        let style_only = "<style>p {}</style>";
        assert!(View::build(parse(style_only).root_node(), style_only).is_none());
    }

    /// The `.vue` files under `directory`, recursively.
    fn vue_files(directory: &Path, out: &mut Vec<PathBuf>) {
        let entries = std::fs::read_dir(directory)
            .unwrap_or_else(|error| panic!("{}: {error}", directory.display()));
        for entry in entries {
            let path = entry.expect("directory entry").path();
            if path.is_dir() {
                vue_files(&path, out);
            } else if path.extension().is_some_and(|extension| extension == "vue") {
                out.push(path);
            }
        }
    }

    /// Every Vue fixture: copies match the file, and the program parses wherever the template
    /// does, but for a few files whose template expressions are no ECMAScript (listed).
    #[test]
    fn corpus() {
        let fixtures = Path::new(crate::code_rule_dsl::fixtures_dir!());
        let mut files = Vec::new();
        vue_files(&fixtures.join("vue/language/view/fixtures"), &mut files);
        vue_files(
            &fixtures.join("../packs/vue/upstream/eslint-plugin-vue/fixtures"),
            &mut files,
        );
        files.sort();
        let mut built = 0;
        let mut broken = Vec::new();
        for path in &files {
            let source = std::fs::read_to_string(path).expect("fixture");
            let tree = parse(&source);
            let Some(view) = View::build(tree.root_node(), &source) else {
                continue;
            };
            built += 1;
            if let Err(error) = check_copies(&view, &source) {
                panic!("{}: {error}", path.display());
            }
            if view.root().has_error() && !tree.root_node().has_error() {
                broken.push(
                    path.strip_prefix(fixtures)
                        .unwrap_or(path)
                        .display()
                        .to_string(),
                );
            }
        }
        eprintln!(
            "{} files, {built} programs, {} with errors only in the program:\n{}",
            files.len(),
            broken.len(),
            broken.join("\n")
        );
        assert!(files.len() > 700, "{} files", files.len());
        assert!(broken.len() * 50 <= files.len(), "{broken:#?}");
    }
}

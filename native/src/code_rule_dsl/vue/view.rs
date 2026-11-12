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
//! from (`embed::segments::SegmentMap::file_ranges`). Script content and template expressions are copied byte for byte, so their
//! nodes map exactly onto the file. The syntax around them is synthetic, each token anchored to
//! the Vue construct it stands for. The two scripts share the program's module scope, as
//! vue-eslint-parser 10.4.1 reads them (it parses their bodies as one program for ESLint), not as
//! Vue's compiler runs them (`<script setup>`'s declarations in `setup()`): a `<script>` function
//! reading a name `<script setup>` declares resolves to it. The blocks are read in source order:
//!
//! | Vue | Program | Span of the synthetic syntax |
//! |---|---|---|
//! | `<script>`, `<script setup>` without `src` | `;` then the content, then `;` | the `;` marker: the start tag (rules tell blocks apart by it); the closing `;`, which ends the content's last statement where it has no `;` of its own: the empty range after the content's last non-blank byte |
//! | `<template>` in HTML (no `lang` other than `html`, no `src`) | `{ ... }` holding the template's view | the template element |
//! | `{{ E }}` | `(E);` | the interpolation |
//! | a directive with a value: `:x="E"`, `v-bind:x`, `v-bind="E"`, `v-show`, `v-html`, `v-text`, `v-model`, `v-memo`, `v-on="{...}"`, a custom `v-foo` | `(E);` | the directive attribute |
//! | `:x`, `v-bind:x`, `.x` without a value (Vue 3.4's same-name shorthand) | `(x);` (the argument camelized: copied, or written over the argument where camelizing changes it, whose span a tape keeps: Vue's `Adapter::name` reads `aria-label` as `ariaLabel`) | the directive attribute |
//! | a dynamic argument `v-x:[A]`, `:[A]`, `@[A]`, `#[A]` | `(A);` before the directive's own statement | the `[A]` |
//! | `@x="E"`, `v-on:x` whose value is a member expression or a function (compiler-core's `isMemberExpression`, `isFnExpression`) | `(E);` | the directive attribute |
//! | `@x="S"` whose value is anything else: inline statements | `(() => { S });` | the directive attribute |
//! | `v-if="C"`, then sibling `v-else-if="C2"` and `v-else` elements (only whitespace and comments between) | `if (C) { E1 } else if (C2) { E2 } else { E3 }` | each branch's `if`, `else if`, `else` and block: its element (so the statement stands for its branches' elements, not what lies between them); each condition's parentheses: its directive |
//! | `v-for="ALIAS in SRC"` (or `of`) on element E | `(SRC), (ALIAS) => { E };` | the statement, sequence, `(SRC)` and arrow: the element E; the alias's parentheses: empty, at E's start and after the alias (so the parameters stand for the alias alone) |
//! | `v-slot="P"`, `v-slot:name="P"`, `#name="P"`, Vue 2's `slot-scope="P"` and `scope="P"` on element E | `(P) => { children of E };` | the statement and arrow: the element E |
//! | a component tag (`<MyComp>`, `<Foo.Bar>`) | `Tag;`, `Foo.Bar;` (the tag name copied: tree-sitter-vue ends `tag_name` at a `.` and reads the rest as `.` shorthands, which the view reads as the tag, as vue-eslint-parser does) | the tag name |
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
//! arrow, never comes from template code, which is always parenthesized. The alias is written in
//! synthetic parentheses, as compiler-core's `parseFor` reads it (its own parentheses,
//! `(item, index)`, stripped), which stand for no bytes: the opening one at the element's start,
//! so the arrow holds the element's tag and the directives before the `v-for` too (a tape keeps
//! a node inside its parent's span), and the closing one after the alias, so neither overlaps a
//! parameter (a byte-positioned sibling walk from a parameter starts at the first sibling
//! ending after the parameter's start).
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
//! row (`:a :b`, `@a :b`) as one attribute, of which the view reads the first; and a dotted end
//! tag (`</ElTable.TableColumn>`) with an `ERROR` in it, a syntax error in the file (a
//! self-closing dotted tag parses).

use crate::code_rule_dsl::embed::{Program, segments::SegmentMap};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::vue::sfc;
use crate::code_rule_dsl::vue::template::{self, Arg, Directive, Parse};
use ast_grep_language::LanguageExt;
use std::borrow::Cow;
use std::ops::Range;
use tree_sitter::{Node, Parser, Tree};

/// The program a Vue single-file component embeds and its map back to the file.
pub struct View {
    /// The program's text, never empty.
    pub text: String,
    pub segments: SegmentMap,
    tree: Tree,
    dialect: GrammarDialect,
}

impl View {
    /// The program the SFC whose tree-sitter-vue tree is `root` embeds; `None` where its script
    /// is in a language other than JavaScript or TypeScript, or nothing it holds is read.
    pub fn build(root: Node<'_>, source: &str) -> Option<Self> {
        let blocks = sfc::blocks(root, source);
        let dialect = sfc::script_dialect(&blocks)?;
        let language = dialect.get_ts_language();
        let mut handlers = Parser::new();
        handlers.set_language(&language).ok()?;
        let mut emitter = Emitter {
            source,
            out: String::new(),
            segments: SegmentMap::new(),
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
        let mut segments = emitter.segments;
        segments.index();
        Some(Self {
            text: emitter.out,
            segments,
            tree,
            dialect,
        })
    }

    /// The view as the component's embedded program (`script-block`), built from what the
    /// component's recovered tree holds, as vue-eslint-parser recovers from a template's syntax
    /// errors and ESLint reads the scripts: the component's errors are not the program's.
    pub(crate) fn into_program(self) -> Program {
        Program {
            dialect: self.dialect,
            tree: self.tree,
            text: self.text,
            segments: self.segments,
            host_errors: Vec::new(),
        }
    }

    /// The grammar the program is written in.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn dialect(&self) -> GrammarDialect {
        self.dialect
    }

    /// The program's root node.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn root(&self) -> Node<'_> {
        self.tree.root_node()
    }

    /// The program's text.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Where the program's text stands in the file. Its placeholders the tape leaves out are the
    /// parameters it writes in an empty `v-for` alias slot (`(,a)`); every other token the view
    /// writes stands for script, a template expression or the Vue syntax around one.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn segments(&self) -> &SegmentMap {
        &self.segments
    }

    /// A node's extent in the program: from the first of its leaves standing for file bytes to
    /// the last; a node none of whose leaves does, from its first leaf to its last
    /// (`tape::node_extents`).
    #[cfg(any(test, feature = "test-hooks"))]
    fn extent(&self, node: Node<'_>) -> Range<usize> {
        let mut leaves = Vec::new();
        let mut stack = vec![node];
        while let Some(at) = stack.pop() {
            if at.child_count() == 0 {
                leaves.push(at.byte_range());
            }
            let mut cursor = at.walk();
            let children = at.children(&mut cursor).collect::<Vec<_>>();
            stack.extend(children.into_iter().rev());
        }
        let mut standing = leaves
            .iter()
            .filter(|leaf| self.segments.stands_for((*leaf).clone()));
        match (standing.next(), standing.next_back()) {
            (Some(first), Some(last)) => first.start..last.end,
            (Some(only), None) => only.clone(),
            _ => leaves[0].start..leaves[leaves.len() - 1].end,
        }
    }

    /// The ranges of `source`, the `.vue` file, a node of the program stands for, in file order,
    /// as its tape's document places them (`tape::TapeDoc`'s `Origin`).
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn ranges(&self, node: Node<'_>, source: &str) -> Vec<Range<usize>> {
        self.segments
            .file_ranges(self.extent(node))
            .bridged(source)
            .iter()
            .cloned()
            .collect()
    }

    /// The bytes of `source` a node of the program reports: its primary range
    /// (`FileRanges::primary`).
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn span(&self, node: Node<'_>, source: &str) -> Range<usize> {
        self.segments
            .file_ranges(self.extent(node))
            .bridged(source)
            .primary()
    }

    /// The file ranges the program copies byte for byte, in program order.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn copied(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.segments.copied()
    }
}

/// Native tags of compiler-dom 3.5.43's `isNativeTag` (@vue/shared's `HTML_TAGS`, `SVG_TAGS` and
/// `MATH_TAGS`) that are JavaScript identifiers, sorted bytewise for binary search.
#[rustfmt::skip]
pub const NATIVE_TAGS: &[&str] = &[
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

/// `range` of `source` without its surrounding whitespace, `None` where nothing else is left or
/// `range` is no range of `source`.
fn trimmed(source: &str, range: Range<usize>) -> Option<Range<usize>> {
    let text = source.get(range.clone())?;
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
pub(crate) fn camelize(text: &str) -> String {
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
    /// The tag name's bytes (`template::tag_name`), where the tag is a component the view reads
    /// (`Tag;`, `Ns.Tag;`).
    component: Option<Range<usize>>,
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
                let tag = template::tag_name(attribute);
                if is_component(&source[tag.clone()]) {
                    element.component = Some(tag);
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
    segments: SegmentMap,
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
        self.segments
            .push(at, self.out.len(), anchor, copied, false);
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

    /// The inside of a parenthesized `v-for` alias (`item, index` of `(item, index)`), copied,
    /// with a placeholder the tape leaves out in each empty slot before the last (`(,a,b)`,
    /// `(a,,b)`, `(a,b,,)`): compiler-core's `parseFor` leaves them unset and its codegen pads
    /// them (`createForLoopParams`), so the program parses and its parameters are the file's
    /// aliases alone.
    fn parameters(&mut self, inner: Range<usize>) {
        let source = self.source;
        let text = &source[inner.clone()];
        let mut depth = 0usize;
        let mut slot = inner.start;
        let mut from = inner.start;
        for (offset, byte) in text.bytes().enumerate() {
            let at = inner.start + offset;
            match byte {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth = depth.saturating_sub(1),
                b',' if depth == 0 => {
                    if source[slot..at].trim().is_empty() {
                        self.code(from..at);
                        self.placeholder(at);
                        from = at;
                    }
                    slot = at + 1;
                }
                _ => {}
            }
        }
        self.code(from..inner.end);
    }

    /// A parameter the program needs and the file does not write, at file byte `at`: the tape
    /// leaves it out (`View::omitted`).
    fn placeholder(&mut self, at: usize) {
        let start = self.out.len();
        self.out.push('_');
        self.segments
            .push(start, self.out.len(), at..at, false, true);
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
        // The alias's parentheses stand for no bytes: the opening one at the element's start, so
        // that the arrow holds the whole element, and the closing one after the alias. Neither
        // overlaps a parameter, so a byte-positioned sibling walk (`next_all`) from a parameter
        // starts at it.
        let start = anchor.start;
        self.emit("(", start..start);
        let end = match trimmed(source, parsed.alias.clone()) {
            Some(alias) if parsed.parenthesized => {
                self.parameters(alias.start + 1..alias.end - 1);
                alias.end
            }
            Some(alias) => {
                self.code(alias.clone());
                alias.end
            }
            None => start,
        };
        self.emit(")", end..end);
        self.emit(" => {", anchor.clone());
        self.contents(element, chained);
        self.emit(" };", anchor);
    }

    /// An element's component tag, its directives, then its children, in its slot's arrow if it
    /// has slot parameters.
    fn contents(&mut self, element: &Element<'t>, chained: bool) {
        if let Some(anchor) = element.component.clone() {
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

// cratify: `tests` is rerail-engine's (it mounts it)

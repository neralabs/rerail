//! A `.rr` file's tree, as its tape holds it (decision 0061): the kinds and fields an `rr` tape numbers
//! (`language::KindCatalog` reads `KIND_NAMES`, `FIRST_ANONYMOUS` and `FIELD_NAMES`), and the tree `syntax::Parser`
//! builds while it reads the statements, flattened in preorder with the comments merged in (`nodes`).
//!
//! Ids start at 1: the named kinds, then the anonymous tokens from `FIRST_ANONYMOUS`. ERROR is every grammar's
//! (`u16::MAX`). Adding a kind or a field changes the grammar build id (`build.rs` hashes this file), so stored tapes
//! are made again.

macro_rules! kinds {
    ($($kind:ident = $name:literal,)*) => {
        /// A node kind; its id is its discriminant.
        #[repr(u16)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Kind {
            _Unknown = 0,
            $($kind,)*
        }

        /// Kind names, by id (0: no kind).
        pub const KIND_NAMES: &[&str] = &["", $($name,)*];
    };
}

kinds! {
    SourceFile = "source_file",
    Include = "include",
    Declaration = "declaration",
    EditDeclaration = "edit_declaration",
    FunctorDeclaration = "functor_declaration",
    TypeDeclaration = "type_declaration",
    IoDirective = "io_directive",
    IoRelation = "io_relation",
    LatticeDeclaration = "lattice_declaration",
    LatticeOperation = "lattice_operation",
    Pragma = "pragma",
    ViolationDeclaration = "violation_declaration",
    CodeColumn = "code_column",
    TagColumn = "tag_column",
    FixLine = "fix_line",
    SuppressLine = "suppress_line",
    PolicyDocument = "policy_document",
    Setting = "setting",
    Clause = "clause",
    Head = "head",
    Conjunction = "conjunction",
    Disjunction = "disjunction",
    Atom = "atom",
    Negation = "negation",
    Comparison = "comparison",
    PatternMatch = "pattern_match",
    Group = "group",
    Call = "call",
    FunctorCall = "functor_call",
    BinaryExpression = "binary_expression",
    UnaryExpression = "unary_expression",
    ParenthesizedExpression = "parenthesized_expression",
    Aggregate = "aggregate",
    NamedArgument = "named_argument",
    ArgumentList = "argument_list",
    ColumnList = "column_list",
    Column = "column",
    Type = "type",
    // Named leaves.
    Identifier = "identifier",
    Variable = "variable",
    Wildcard = "wildcard",
    Number = "number",
    Float = "float",
    String = "string",
    CodeLiteral = "code_literal",
    JsonObject = "json_object",
    RawText = "raw_text",
    Comment = "comment",
    // Anonymous tokens (the first is `FIRST_ANONYMOUS`): the lexer's punctuation, in `syntax::PUNCT`'s order.
    Turnstile = ":-",
    Arrow = "=>",
    NotEqual = "!=",
    LessEqual = "<=",
    GreaterEqual = ">=",
    Subtype = "<:",
    RightArrow = "->",
    OpenParen = "(",
    CloseParen = ")",
    Comma = ",",
    Dot = ".",
    Semicolon = ";",
    Colon = ":",
    Equal = "=",
    Less = "<",
    Greater = ">",
    Bang = "!",
    OpenBrace = "{",
    CloseBrace = "}",
    Bar = "|",
    At = "@",
    Star = "*",
    OpenBracket = "[",
    CloseBracket = "]",
    Minus = "-",
    Plus = "+",
    Slash = "/",
    Percent = "%",
    Tilde = "~",
    Caret = "^",
    Ampersand = "&",
    // Directive keywords, each one token with its `.` or `#`.
    HashInclude = "#include",
    DotDecl = ".decl",
    DotEdit = ".edit",
    DotFunctor = ".functor",
    DotType = ".type",
    DotInput = ".input",
    DotOutput = ".output",
    DotLattice = ".lattice",
    DotPragma = ".pragma",
    DotViolation = ".violation",
    DotPolicy = ".policy",
    // The words the grammar reserves where they appear.
    Must = "must",
    May = "may",
    CodeWord = "code",
    In = "in",
    Tag = "tag",
    Fix = "fix",
    Safe = "safe",
    Suppress = "suppress",
    Over = "over",
    Options = "options",
    By = "by",
    Sep = "sep",
    Count = "count",
    Min = "min",
    Max = "max",
    Sum = "sum",
    Mean = "mean",
    Join = "join",
}

/// Kinds from this id on are anonymous.
pub const FIRST_ANONYMOUS: u16 = Kind::Turnstile as u16;

/// The kind id a tree gives ERROR, as every grammar does.
pub const ERROR: u16 = u16::MAX;

macro_rules! fields {
    ($($field:ident = $name:literal,)*) => {
        /// A field; its id is its discriminant.
        #[repr(u16)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Field {
            None = 0,
            $($field,)*
        }

        /// Field names, by id (0: no field).
        pub const FIELD_NAMES: &[&str] = &["", $($name,)*];
    };
}

fields! {
    Path = "path",
    Name = "name",
    Columns = "columns",
    Modifier = "modifier",
    ReturnType = "return_type",
    Definition = "definition",
    Relation = "relation",
    Parameters = "parameters",
    Base = "base",
    Operation = "operation",
    Value = "value",
    Argument = "argument",
    Code = "code",
    Tag = "tag",
    Message = "message",
    Fix = "fix",
    Suppress = "suppress",
    Options = "options",
    Column = "column",
    Codes = "codes",
    Description = "description",
    Tool = "tool",
    Rule = "rule",
    Over = "over",
    Document = "document",
    Head = "head",
    Body = "body",
    Edit = "edit",
    Modal = "modal",
    Arguments = "arguments",
    Literal = "literal",
    Alternative = "alternative",
    Operand = "operand",
    Left = "left",
    Operator = "operator",
    Right = "right",
    Subject = "subject",
    Pattern = "pattern",
    Function = "function",
    Expression = "expression",
    Key = "key",
    Separator = "separator",
    Type = "type",
}

/// The anonymous kind of a token written `text` (punctuation, a directive keyword, a reserved word), if it is one.
pub fn anonymous(text: &str) -> Option<u16> {
    KIND_NAMES[usize::from(FIRST_ANONYMOUS)..].iter().position(|name| *name == text).map(|at| at as u16 + FIRST_ANONYMOUS)
}

/// The directive keyword `.word` names, as one anonymous token; `None` for a setting (`.mode`, `.scope`, …), whose
/// word is its name.
pub fn directive(word: &str) -> Option<u16> {
    matches!(word, "decl" | "edit" | "functor" | "type" | "input" | "output" | "lattice" | "pragma" | "violation" | "policy")
        .then(|| anonymous(&format!(".{word}")))
        .flatten()
}

/// One node of the tree a parse builds: a kind (`ERROR` for a statement that did not parse), the field it holds in
/// its parent, its byte span, and its children in source order.
#[derive(Clone, Debug)]
pub struct Built {
    pub kind: u16,
    pub field: u16,
    pub start: usize,
    pub end: usize,
    pub children: Vec<u32>,
    /// A token's leaf (a `json_object`, a `raw_text` …): its text is its own. Not a node opened
    /// for children, which may have none (`()`, its tokens implied).
    pub leaf: bool,
}

/// The tree a `syntax::Parser` builds: node 0 is the `source_file`, and `stack` the nodes open while it parses.
#[derive(Clone, Debug)]
pub struct Tree {
    pub nodes: Vec<Built>,
    pub stack: Vec<u32>,
}

/// Where a parse can be put back to (`Parser::mark`): the open nodes, the top one's children and the nodes made.
#[derive(Clone, Copy)]
pub struct Mark {
    pub tok: usize,
    stack: usize,
    children: usize,
    nodes: usize,
}

impl Tree {
    pub fn new(len: usize) -> Self {
        Self { nodes: vec![Built { kind: Kind::SourceFile as u16, field: 0, start: 0, end: len, children: Vec::new(), leaf: false }], stack: vec![0] }
    }

    fn top(&mut self) -> &mut Built {
        let top = *self.stack.last().expect("source_file stays open");
        &mut self.nodes[top as usize]
    }

    /// A node of `kind` from `start`, a child of the open node; it stays open when `open`.
    pub fn push(&mut self, kind: u16, start: usize, end: usize, open: bool) -> u32 {
        let id = self.nodes.len() as u32;
        self.nodes.push(Built { kind, field: 0, start, end, children: Vec::new(), leaf: !open });
        self.top().children.push(id);
        if open {
            self.stack.push(id);
        }
        id
    }

    /// Closes the open node at `end`.
    pub fn close(&mut self, end: usize) {
        let id = self.stack.pop().expect("an open node");
        debug_assert!(id != 0, "source_file is never closed");
        let node = &mut self.nodes[id as usize];
        node.end = end.max(node.start);
    }

    /// The open node's child count: where a later `wrap` starts.
    pub fn here(&self) -> usize {
        self.nodes[*self.stack.last().expect("open") as usize].children.len()
    }

    /// Opens a node of `kind` around the open node's children from `from` on (an operator found after its left
    /// operand: `a - b`, `x <= y`, `n ~ `…``). It takes the field the first of them held.
    pub fn wrap(&mut self, from: usize, kind: u16) -> u32 {
        let id = self.nodes.len() as u32;
        let moved: Vec<u32> = self.top().children.split_off(from);
        let first = moved.first().map(|&c| self.nodes[c as usize].clone());
        let (start, field) = first.map_or((0, 0), |f| (f.start, f.field));
        if let Some(&c) = moved.first() {
            self.nodes[c as usize].field = 0;
        }
        self.nodes.push(Built { kind, field, start, end: start, children: moved, leaf: false });
        self.top().children.push(id);
        self.stack.push(id);
        id
    }

    /// The open node's last child.
    pub fn last(&self) -> Option<u32> {
        self.nodes[*self.stack.last().expect("open") as usize].children.last().copied()
    }

    pub fn mark(&self, tok: usize) -> Mark {
        Mark { tok, stack: self.stack.len(), children: self.here(), nodes: self.nodes.len() }
    }

    /// Puts the tree back to `mark`: what was made since is dropped.
    pub fn reset(&mut self, mark: Mark) {
        self.stack.truncate(mark.stack);
        self.top().children.truncate(mark.children);
        self.nodes.truncate(mark.nodes);
    }
}

/// A node of a `.rr` tape, in preorder: its kind (`ERROR` for a bad statement), field, span and parent, and whether
/// it is an extra (a comment).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Node {
    pub kind: u16,
    pub field: u16,
    pub start: u32,
    pub end: u32,
    pub parent: Option<u32>,
    pub extra: bool,
}

impl Node {
    pub fn is_error(&self) -> bool {
        self.kind == ERROR
    }
}

/// A row of a `.rr` tape, as the parsers write it: a node's kind and field, its bytes, the rows
/// in its subtree (its own counted), its parent's row (`NO_PARENT` for the root), and 1 for an
/// extra (a comment). In preorder. The lean parser writes these rows as it parses; the compiled
/// parser's tier 2 writes the same 24 bytes (`rerail_compiler::native_cpu_stream::NodeRow`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub kind: u16,
    pub field: u16,
    pub start: u32,
    pub end: u32,
    pub size: u32,
    pub parent: u32,
    pub extra: u8,
    pub pad: [u8; 3],
}

/// The root's parent in a row.
pub const NO_PARENT: u32 = u32::MAX;

/// The reference parser's nodes (`crate::tape`) as rows: each node's size from the last node
/// back. For comparing with the parsers that write rows.
pub fn rows(nodes: &[Node]) -> Vec<Row> {
    let mut rows: Vec<Row> = nodes.iter().map(|n| Row { kind: n.kind, field: n.field, start: n.start, end: n.end, size: 1, parent: n.parent.unwrap_or(NO_PARENT), extra: u8::from(n.extra), pad: [0; 3] }).collect();
    for (i, n) in nodes.iter().enumerate().rev() {
        if let Some(p) = n.parent {
            rows[p as usize].size += rows[i].size;
        }
    }
    rows
}

/// `tree` in preorder, each comment of `comments` a child of the innermost node whose span holds it, at its place in
/// source order. A comment inside a leaf (a `json_object`, a `raw_text`) is part of the leaf's text.
pub fn flatten(tree: &Tree, comments: &[(usize, usize)]) -> Vec<Node> {
    struct Walk<'t> {
        tree: &'t Tree,
        comments: &'t [(usize, usize)],
        next: usize,
        out: Vec<Node>,
    }
    impl Walk<'_> {
        fn comments_before(&mut self, parent: u32, before: usize) {
            while let Some(&(start, end)) = self.comments.get(self.next) {
                if start >= before {
                    break;
                }
                self.next += 1;
                self.out.push(Node { kind: Kind::Comment as u16, field: 0, start: start as u32, end: end as u32, parent: Some(parent), extra: true });
            }
        }

        fn node(&mut self, id: u32, parent: Option<u32>) {
            let built = &self.tree.nodes[id as usize];
            let at = self.out.len() as u32;
            self.out.push(Node { kind: built.kind, field: built.field, start: built.start as u32, end: built.end as u32, parent, extra: false });
            if built.leaf {
                // A leaf's text is its own: a comment inside it is not a node.
                while self.comments.get(self.next).is_some_and(|&(start, _)| start < built.end) {
                    self.next += 1;
                }
                return;
            }
            for &child in &built.children {
                self.comments_before(at, self.tree.nodes[child as usize].start);
                self.node(child, Some(at));
            }
            self.comments_before(at, built.end);
        }
    }
    let mut walk = Walk { tree, comments, next: 0, out: Vec::with_capacity(tree.nodes.len() + comments.len()) };
    walk.node(0, None);
    // Comments after the last statement belong to the file.
    walk.comments_before(0, usize::MAX);
    walk.out
}

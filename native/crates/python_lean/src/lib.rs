//! A lean Python parser for the tape alone: tree-sitter-python's tape, as the engine encodes its
//! tree (named-only, decision 0140's writer), written as the parse goes. It stands in for
//! tree-sitter-python where a file parses; a file it cannot read is left to tree-sitter
//! (`Refusal`), which recovers from errors.
//!
//! - **The lexer** (`lex`) is tree-sitter-python's lexer and external scanner in one pass: the
//!   tokens, the layout tokens (NEWLINE, INDENT, DEDENT) where the scanner gives them, the string
//!   pieces, and the extras (comments, line continuations) in a list of their own. Runs of bytes
//!   (a name, blanks, a comment, a string's text) are found 64 bytes a step (`block`).
//! - **The parser** (`parse`) is recursive descent over the tokens. Each node is written in
//!   preorder as it opens, with its code (kind, field, supertypes); a node found after its first
//!   child (an operator after its left operand) is put before it (`Writer::insert`). Only the
//!   tokens tree-sitter's tape stores are written (format 15: the ones its grammar does not imply).
//! - **Extras** are placed as tree-sitter places them: in the lowest node holding the tokens on
//!   either side, the layout tokens counted. A comment is written when the parse next opens a
//!   node or reads a token, at the node then open.
//!
//! A node's key is the parser's own (`key`): a kind from `KINDS`, a field from `FIELDS`, a set of
//! supertypes from `CLASSES`, and the extra flag. The engine maps each to its grammar's ids by name.

mod block;
mod lex;
mod parse;

pub use lex::Refusal;

/// Node kinds, as the tape names them: tree-sitter-python's, named and anonymous.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum K {
    Module,
    Comment,
    LineContinuation,
    Identifier,
    Integer,
    Float,
    True,
    False,
    None,
    Ellipsis,
    String,
    StringStart,
    StringContent,
    StringEnd,
    EscapeSequence,
    EscapeInterpolation,
    Interpolation,
    FormatSpecifier,
    FormatExpression,
    TypeConversion,
    ConcatenatedString,
    ExpressionStatement,
    Assignment,
    AugmentedAssignment,
    PatternList,
    ExpressionList,
    ImportStatement,
    ImportFromStatement,
    FutureImportStatement,
    AliasedImport,
    DottedName,
    RelativeImport,
    ImportPrefix,
    WildcardImport,
    PrintStatement,
    Chevron,
    AssertStatement,
    ReturnStatement,
    DeleteStatement,
    RaiseStatement,
    PassStatement,
    BreakStatement,
    ContinueStatement,
    GlobalStatement,
    NonlocalStatement,
    ExecStatement,
    TypeAliasStatement,
    IfStatement,
    ElifClause,
    ElseClause,
    ForStatement,
    WhileStatement,
    TryStatement,
    ExceptClause,
    FinallyClause,
    WithStatement,
    WithClause,
    WithItem,
    FunctionDefinition,
    Parameters,
    LambdaParameters,
    ClassDefinition,
    DecoratedDefinition,
    Decorator,
    Block,
    MatchStatement,
    CaseClause,
    CasePattern,
    ArgumentList,
    KeywordArgument,
    ListSplat,
    DictionarySplat,
    ParenthesizedListSplat,
    TypedParameter,
    DefaultParameter,
    TypedDefaultParameter,
    ListSplatPattern,
    DictionarySplatPattern,
    KeywordSeparator,
    PositionalSeparator,
    TuplePattern,
    ListPattern,
    Type,
    SplatType,
    GenericType,
    UnionType,
    ConstrainedType,
    MemberType,
    TypeParameter,
    TypeDefault,
    NotOperator,
    BooleanOperator,
    BinaryOperator,
    UnaryOperator,
    ComparisonOperator,
    Lambda,
    ConditionalExpression,
    NamedExpression,
    AsPattern,
    AsPatternTarget,
    Await,
    Yield,
    Attribute,
    Subscript,
    Slice,
    Call,
    List,
    Set,
    Tuple,
    Dictionary,
    Pair,
    ListComprehension,
    DictionaryComprehension,
    SetComprehension,
    GeneratorExpression,
    ForInClause,
    IfClause,
    ParenthesizedExpression,
    UnionPattern,
    DictPattern,
    KeywordPattern,
    SplatPattern,
    ClassPattern,
    ComplexPattern,
    // Anonymous: the tokens a tape stores, and the two-word operators.
    NotIn,
    IsNot,
    TComma,
    TRParen,
    TRBracket,
    TRBrace,
    TLParen,
    TDot,
    TSemi,
    TColon,
    TEq,
    TStar,
    TPow,
    TSlash,
    TMinus,
    TPlus,
    TTilde,
    TPercent,
    TAmp,
    TPipe,
    TCaret,
    TLShift,
    TRShift,
    TDSlash,
    TAt,
    TLt,
    TGt,
    TLe,
    TGe,
    TEqEq,
    TNe,
    TLtGt,
    TIn,
    TIs,
    TNot,
    TAnd,
    TOr,
    TPlusEq,
    TMinusEq,
    TStarEq,
    TSlashEq,
    TAtEq,
    TDSlashEq,
    TPercentEq,
    TPowEq,
    TRShiftEq,
    TLShiftEq,
    TAmpEq,
    TCaretEq,
    TPipeEq,
    TDef,
    TFor,
    TAsync,
    TWith,
    TExcept,
    TImport,
    TFrom,
    TFuture,
    TYield,
    TBackslash,
}

/// Each kind's name in tree-sitter-python and whether it is named, in `K`'s order.
pub const KINDS: &[(&str, bool)] = &[
    ("module", true),
    ("comment", true),
    ("line_continuation", true),
    ("identifier", true),
    ("integer", true),
    ("float", true),
    ("true", true),
    ("false", true),
    ("none", true),
    ("ellipsis", true),
    ("string", true),
    ("string_start", true),
    ("string_content", true),
    ("string_end", true),
    ("escape_sequence", true),
    ("escape_interpolation", true),
    ("interpolation", true),
    ("format_specifier", true),
    ("format_expression", true),
    ("type_conversion", true),
    ("concatenated_string", true),
    ("expression_statement", true),
    ("assignment", true),
    ("augmented_assignment", true),
    ("pattern_list", true),
    ("expression_list", true),
    ("import_statement", true),
    ("import_from_statement", true),
    ("future_import_statement", true),
    ("aliased_import", true),
    ("dotted_name", true),
    ("relative_import", true),
    ("import_prefix", true),
    ("wildcard_import", true),
    ("print_statement", true),
    ("chevron", true),
    ("assert_statement", true),
    ("return_statement", true),
    ("delete_statement", true),
    ("raise_statement", true),
    ("pass_statement", true),
    ("break_statement", true),
    ("continue_statement", true),
    ("global_statement", true),
    ("nonlocal_statement", true),
    ("exec_statement", true),
    ("type_alias_statement", true),
    ("if_statement", true),
    ("elif_clause", true),
    ("else_clause", true),
    ("for_statement", true),
    ("while_statement", true),
    ("try_statement", true),
    ("except_clause", true),
    ("finally_clause", true),
    ("with_statement", true),
    ("with_clause", true),
    ("with_item", true),
    ("function_definition", true),
    ("parameters", true),
    ("lambda_parameters", true),
    ("class_definition", true),
    ("decorated_definition", true),
    ("decorator", true),
    ("block", true),
    ("match_statement", true),
    ("case_clause", true),
    ("case_pattern", true),
    ("argument_list", true),
    ("keyword_argument", true),
    ("list_splat", true),
    ("dictionary_splat", true),
    ("parenthesized_list_splat", true),
    ("typed_parameter", true),
    ("default_parameter", true),
    ("typed_default_parameter", true),
    ("list_splat_pattern", true),
    ("dictionary_splat_pattern", true),
    ("keyword_separator", true),
    ("positional_separator", true),
    ("tuple_pattern", true),
    ("list_pattern", true),
    ("type", true),
    ("splat_type", true),
    ("generic_type", true),
    ("union_type", true),
    ("constrained_type", true),
    ("member_type", true),
    ("type_parameter", true),
    ("type_default", true),
    ("not_operator", true),
    ("boolean_operator", true),
    ("binary_operator", true),
    ("unary_operator", true),
    ("comparison_operator", true),
    ("lambda", true),
    ("conditional_expression", true),
    ("named_expression", true),
    ("as_pattern", true),
    ("as_pattern_target", true),
    ("await", true),
    ("yield", true),
    ("attribute", true),
    ("subscript", true),
    ("slice", true),
    ("call", true),
    ("list", true),
    ("set", true),
    ("tuple", true),
    ("dictionary", true),
    ("pair", true),
    ("list_comprehension", true),
    ("dictionary_comprehension", true),
    ("set_comprehension", true),
    ("generator_expression", true),
    ("for_in_clause", true),
    ("if_clause", true),
    ("parenthesized_expression", true),
    ("union_pattern", true),
    ("dict_pattern", true),
    ("keyword_pattern", true),
    ("splat_pattern", true),
    ("class_pattern", true),
    ("complex_pattern", true),
    ("not in", false),
    ("is not", false),
    (",", false),
    (")", false),
    ("]", false),
    ("}", false),
    ("(", false),
    (".", false),
    (";", false),
    (":", false),
    ("=", false),
    ("*", false),
    ("**", false),
    ("/", false),
    ("-", false),
    ("+", false),
    ("~", false),
    ("%", false),
    ("&", false),
    ("|", false),
    ("^", false),
    ("<<", false),
    (">>", false),
    ("//", false),
    ("@", false),
    ("<", false),
    (">", false),
    ("<=", false),
    (">=", false),
    ("==", false),
    ("!=", false),
    ("<>", false),
    ("in", false),
    ("is", false),
    ("not", false),
    ("and", false),
    ("or", false),
    ("+=", false),
    ("-=", false),
    ("*=", false),
    ("/=", false),
    ("@=", false),
    ("//=", false),
    ("%=", false),
    ("**=", false),
    (">>=", false),
    ("<<=", false),
    ("&=", false),
    ("^=", false),
    ("|=", false),
    ("def", false),
    ("for", false),
    ("async", false),
    ("with", false),
    ("except", false),
    ("import", false),
    ("from", false),
    ("__future__", false),
    ("yield", false),
    ("\\", false),
];

/// Fields, as the tape names them; 0 is no field.
pub const FIELDS: &[&str] = &[
    "",
    "alias",
    "alternative",
    "argument",
    "arguments",
    "attribute",
    "body",
    "cause",
    "code",
    "condition",
    "consequence",
    "definition",
    "expression",
    "format_specifier",
    "function",
    "guard",
    "key",
    "left",
    "module_name",
    "name",
    "object",
    "operator",
    "operators",
    "parameters",
    "return_type",
    "right",
    "subject",
    "subscript",
    "superclasses",
    "type",
    "type_conversion",
    "type_parameters",
    "value",
];

/// Field ids, in `FIELDS`' order.
#[allow(non_upper_case_globals)]
pub mod f {
    pub const NONE: u8 = 0;
    pub const ALIAS: u8 = 1;
    pub const ALTERNATIVE: u8 = 2;
    pub const ARGUMENT: u8 = 3;
    pub const ARGUMENTS: u8 = 4;
    pub const ATTRIBUTE: u8 = 5;
    pub const BODY: u8 = 6;
    pub const CAUSE: u8 = 7;
    pub const CODE: u8 = 8;
    pub const CONDITION: u8 = 9;
    pub const CONSEQUENCE: u8 = 10;
    pub const DEFINITION: u8 = 11;
    pub const EXPRESSION: u8 = 12;
    pub const FORMAT_SPECIFIER: u8 = 13;
    pub const FUNCTION: u8 = 14;
    pub const GUARD: u8 = 15;
    pub const KEY: u8 = 16;
    pub const LEFT: u8 = 17;
    pub const MODULE_NAME: u8 = 18;
    pub const NAME: u8 = 19;
    pub const OBJECT: u8 = 20;
    pub const OPERATOR: u8 = 21;
    pub const OPERATORS: u8 = 22;
    pub const PARAMETERS: u8 = 23;
    pub const RETURN_TYPE: u8 = 24;
    pub const RIGHT: u8 = 25;
    pub const SUBJECT: u8 = 26;
    pub const SUBSCRIPT: u8 = 27;
    pub const SUPERCLASSES: u8 = 28;
    pub const TYPE: u8 = 29;
    pub const TYPE_CONVERSION: u8 = 30;
    pub const TYPE_PARAMETERS: u8 = 31;
    pub const VALUE: u8 = 32;
}

/// The supertype sets a node may be in, by where it stands: the hidden supertype rules between it
/// and its parent, as tree-sitter's cursor reports them.
pub const CLASSES: &[&[&str]] = &[&[], &["expression", "primary_expression"], &["expression"], &["primary_expression"], &["pattern"], &["parameter"]];

/// A node's key in the writer: its kind, field, supertype set (`CLASSES`) and the extra flag.
#[inline(always)]
pub const fn key(kind: K, field: u8, class: u8, extra: bool) -> u32 {
    kind as u32 | (field as u32) << 8 | (class as u32) << 14 | (extra as u32) << 17
}

/// A key's kind, field, supertype set and extra flag (`key`'s inverse).
pub const fn unkey(key: u32) -> (u8, u8, u8, bool) {
    ((key & 0xff) as u8, ((key >> 8) & 0x3f) as u8, ((key >> 14) & 0x7) as u8, key >> 17 & 1 != 0)
}

/// A file's tape as the parser writes it: the columns at their widths (`rerail_tape_format::Columns`),
/// each code id's key.
pub use rerail_tape_format::Columns as Tape;

/// The tape of `src`, a Python file: tree-sitter-python's, as the engine encodes it. `Err` where the
/// parser leaves the file to tree-sitter (a syntax error, a construct it does not read).
pub fn tape(src: &[u8]) -> Result<Tape, Refusal> {
    if src.len() >= u32::MAX as usize / 2 {
        return Err(Refusal::TooLarge);
    }
    let lexed = lex::lex(src)?;
    parse::parse(src, &lexed)
}

/// How many tokens and extras `src` lexes to (the lexer alone, for a measurement).
pub fn token_count(src: &[u8]) -> Result<usize, Refusal> {
    let lexed = lex::lex(src)?;
    Ok(lexed.toks.len() + lexed.extras.len())
}

/// The tokens `src` lexes to, as text (for the tests): each token's kind and span, and the extras.
pub fn tokens_text(src: &[u8]) -> Result<String, Refusal> {
    let lexed = lex::lex(src)?;
    Ok(lexed.text(src))
}

/// A tape as text, as the engine's `python_tape_plain` writes tree-sitter's: a node a line,
/// indented by depth, its kind, field, flags, supertypes and span.
pub fn tape_text(tape: &Tape) -> String {
    use rerail_tape_format::TAIL_FROM_START;
    let cell = |column: &[u8], width: u8, i: usize| -> u32 {
        let w = usize::from(width & 0x7f);
        let b = &column[i * w..i * w + w];
        if w == 2 { u32::from(u16::from_le_bytes([b[0], b[1]])) } else { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) }
    };
    let n = tape.code.len() / 2;
    let start = |i: usize| {
        let b = i / rerail_tape_format::CHECKPOINT_STRIDE;
        u32::from_le_bytes(tape.base[4 * b..4 * b + 4].try_into().expect("a base")) + cell(&tape.off, tape.widths[1], i)
    };
    let mut out = String::new();
    let mut open: Vec<usize> = Vec::new();
    for i in 0..n {
        while open.last().is_some_and(|&end| end <= i) {
            open.pop();
        }
        let code = u16::from_le_bytes([tape.code[2 * i], tape.code[2 * i + 1]]);
        let (kind, field, class, extra) = unkey(tape.keys[usize::from(code)]);
        let size = cell(&tape.size, tape.widths[0], i) as usize;
        let s = start(i);
        let from = if tape.widths[2] & TAIL_FROM_START != 0 { s } else { start(i + size - 1) };
        let end = from + cell(&tape.tail, tape.widths[2], i);
        let classes = CLASSES[usize::from(class)];
        out.push_str(&format!(
            "{}{}{}{}{} [{s}..{end}]\n",
            "  ".repeat(open.len()),
            KINDS[usize::from(kind)].0,
            if field == 0 { String::new() } else { format!(" {}:", FIELDS[usize::from(field)]) },
            if extra { " flags=1" } else { "" },
            if classes.is_empty() { String::new() } else { format!(" {{{}}}", classes.join(",")) },
        ));
        open.push(i + size);
    }
    out
}

#[cfg(test)]
mod oracle;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tables_agree() {
        assert_eq!(KINDS.len(), K::TBackslash as usize + 1);
        assert_eq!(FIELDS.len(), f::VALUE as usize + 1);
        assert!(KINDS.len() < 256 && FIELDS.len() <= 64 && CLASSES.len() <= 8);
    }
}

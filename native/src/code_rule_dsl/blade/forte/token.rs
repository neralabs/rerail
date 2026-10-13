//! Forte's tokens (`Lexer\Tokens\TokenType`, `Internal\TokenRecord`), lexer states
//! (`Lexer\State`) and lexer errors (`Lexer\LexerError`, `Lexer\ErrorReason`).

/// A token type, numbered as Forte numbers it (the oracle records the numbers).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TokenType {
    Text = 0,
    LessThan = 1,
    GreaterThan = 2,
    Slash = 3,
    Equals = 4,
    TagName = 5,
    AttributeName = 6,
    AttributeValue = 7,
    Quote = 8,
    Whitespace = 9,
    CommentStart = 10,
    CommentEnd = 11,
    DoctypeStart = 12,
    Doctype = 13,
    DoctypeEnd = 14,
    AtSign = 15,
    EchoStart = 16,
    EchoEnd = 17,
    EchoContent = 18,
    RawEchoStart = 19,
    RawEchoEnd = 20,
    TripleEchoStart = 21,
    TripleEchoEnd = 22,
    Directive = 23,
    DirectiveArgs = 24,
    VerbatimStart = 25,
    VerbatimEnd = 26,
    JsxAttributeValue = 27,
    JsxShorthandAttribute = 28,
    TsxGenericType = 29,
    BogusComment = 30,
    ConditionalCommentStart = 31,
    ConditionalCommentEnd = 32,
    PhpBlockStart = 33,
    PhpBlockEnd = 34,
    PhpBlock = 35,
    PhpTagStart = 36,
    PhpTagEnd = 37,
    PhpContent = 38,
    BladeCommentStart = 39,
    BladeCommentEnd = 40,
    SyntheticClose = 41,
    /// `::name`.
    EscapedAttribute = 42,
    /// `:name`.
    BoundAttribute = 43,
    /// `:$name`.
    ShorthandAttribute = 44,
    CdataStart = 45,
    CdataEnd = 46,
    DeclStart = 47,
    DeclEnd = 48,
    PiStart = 49,
    PiEnd = 50,
}

/// A token: its type and byte range in the template (`TokenRecord`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Token {
    pub(crate) kind: TokenType,
    pub(crate) start: u32,
    pub(crate) end: u32,
}

/// A lexer state (`Lexer\State`), numbered as Forte numbers it; Forte's `TagOpen` (1), `Verbatim`
/// (15) and `PhpBlock` (16) are left out, since no error Forte logs carries them.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    Data = 0,
    TagName = 2,
    BeforeAttrName = 3,
    AttrName = 4,
    AfterAttrName = 5,
    BeforeAttrValue = 6,
    AttrValueQuoted = 7,
    AttrValueUnquoted = 8,
    EchoContent = 9,
    RawEchoContent = 10,
    TripleEchoContent = 11,
    RawText = 12,
    Comment = 13,
    BladeComment = 14,
    /// No error the port logs carries it; the oracle's records name it.
    #[cfg(test)]
    PhpTag = 17,
}

impl State {
    /// Every state, for reading a name back.
    #[cfg(test)]
    pub(crate) const ALL: [State; 15] = [
        State::Data,
        State::TagName,
        State::BeforeAttrName,
        State::AttrName,
        State::AfterAttrName,
        State::BeforeAttrValue,
        State::AttrValueQuoted,
        State::AttrValueUnquoted,
        State::EchoContent,
        State::RawEchoContent,
        State::TripleEchoContent,
        State::RawText,
        State::Comment,
        State::BladeComment,
        State::PhpTag,
    ];

    /// The case name (`State::label`).
    #[cfg(test)]
    pub(crate) fn name(self) -> &'static str {
        use State::*;
        match self {
            Data => "Data",
            TagName => "TagName",
            BeforeAttrName => "BeforeAttrName",
            AttrName => "AttrName",
            AfterAttrName => "AfterAttrName",
            BeforeAttrValue => "BeforeAttrValue",
            AttrValueQuoted => "AttrValueQuoted",
            AttrValueUnquoted => "AttrValueUnquoted",
            EchoContent => "EchoContent",
            RawEchoContent => "RawEchoContent",
            TripleEchoContent => "TripleEchoContent",
            RawText => "RawText",
            Comment => "Comment",
            BladeComment => "BladeComment",
            PhpTag => "PhpTag",
        }
    }
}

/// Why the lexer logged an error (`Lexer\ErrorReason`), numbered as Forte numbers it. Forte
/// declares six more cases (nested echoes, unclosed string, invalid escape, unimplemented state)
/// and never logs them.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ErrorReason {
    UnexpectedEof = 3,
    UnclosedComment = 5,
    ConstructCollision = 8,
    PhpCloseTagInComment = 9,
}

impl ErrorReason {
    /// Every reason, for reading a name back.
    #[cfg(test)]
    pub(crate) const ALL: [ErrorReason; 4] = [
        ErrorReason::UnexpectedEof,
        ErrorReason::UnclosedComment,
        ErrorReason::ConstructCollision,
        ErrorReason::PhpCloseTagInComment,
    ];

    /// The case name (`ErrorReason::label`, and `Diagnostic::$code`).
    #[cfg(test)]
    pub(crate) fn name(self) -> &'static str {
        use ErrorReason::*;
        match self {
            UnexpectedEof => "UnexpectedEof",
            UnclosedComment => "UnclosedComment",
            ConstructCollision => "ConstructCollision",
            PhpCloseTagInComment => "PhpCloseTagInComment",
        }
    }
}

/// An error the lexer logged (`Lexer\LexerError`): Forte's diagnostics, each an error spanning
/// the byte at `offset` (`Diagnostic::fromLexerError`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LexerError {
    pub(crate) state: State,
    pub(crate) reason: ErrorReason,
    pub(crate) offset: u32,
}

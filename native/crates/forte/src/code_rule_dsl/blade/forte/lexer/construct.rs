//! `Lexer/ConstructDetector.php` and `Lexer/Construct.php`.
use super::super::scan::is_alnum;
use super::Lexer;

/// A Blade construct an echo runs into (`Construct`). Its `name()` label is unused by Forte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Construct {
    Echo,
    RawEcho,
    TripleEcho,
    BladeComment,
    Directive,
    PhpTag,
}

// `CONSTRUCT_NONE` and `CONSTRUCT_ALL_ECHOS` are unused by the lexer.
pub(super) const CONSTRUCT_ECHO: u8 = 1 << 0;
pub(super) const CONSTRUCT_RAW_ECHO: u8 = 1 << 1;
pub(super) const CONSTRUCT_TRIPLE_ECHO: u8 = 1 << 2;
pub(super) const CONSTRUCT_BLADE_COMMENT: u8 = 1 << 3;
pub(super) const CONSTRUCT_DIRECTIVE: u8 = 1 << 4;
pub(super) const CONSTRUCT_PHP_TAG: u8 = 1 << 5;
pub(super) const CONSTRUCT_ALL: u8 = CONSTRUCT_ECHO
    | CONSTRUCT_RAW_ECHO
    | CONSTRUCT_TRIPLE_ECHO
    | CONSTRUCT_BLADE_COMMENT
    | CONSTRUCT_DIRECTIVE
    | CONSTRUCT_PHP_TAG;

impl Lexer<'_> {
    /// `canStartDirective`: `@` followed by an alphanumeric or `_`.
    fn can_start_directive(&self) -> bool {
        match self.peek_ahead(1) {
            None => false,
            Some(next) => is_alnum(next) || next == b'_',
        }
    }

    /// `detectConstruct`.
    pub(super) fn detect_construct(&self, flags: u8) -> Option<Construct> {
        let peek = self.peek();
        if flags & CONSTRUCT_TRIPLE_ECHO != 0
            && peek == Some(b'{')
            && self.peek_ahead(1) == Some(b'{')
            && self.peek_ahead(2) == Some(b'{')
        {
            return Some(Construct::TripleEcho);
        }

        if flags & CONSTRUCT_BLADE_COMMENT != 0
            && peek == Some(b'{')
            && self.peek_ahead(1) == Some(b'{')
            && self.peek_ahead(2) == Some(b'-')
            && self.peek_ahead(3) == Some(b'-')
        {
            return Some(Construct::BladeComment);
        }

        if flags & CONSTRUCT_RAW_ECHO != 0
            && peek == Some(b'{')
            && self.peek_ahead(1) == Some(b'!')
            && self.peek_ahead(2) == Some(b'!')
        {
            return Some(Construct::RawEcho);
        }

        if flags & CONSTRUCT_ECHO != 0 && peek == Some(b'{') && self.peek_ahead(1) == Some(b'{') {
            return Some(Construct::Echo);
        }

        if flags & CONSTRUCT_DIRECTIVE != 0 && peek == Some(b'@') && self.can_start_directive() {
            return Some(Construct::Directive);
        }

        if flags & CONSTRUCT_PHP_TAG != 0 && peek == Some(b'<') && self.peek_ahead(1) == Some(b'?')
        {
            return Some(Construct::PhpTag);
        }

        None
    }
}

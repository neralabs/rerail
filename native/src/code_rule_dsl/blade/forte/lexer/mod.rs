//! Forte's lexer: `Lexer/Lexer.php`, `Lexer/ConstructDetector.php`, `Lexer/Construct.php` and the
//! traits of `Lexer/Concerns/*.php`, ported method by method. Each PHP trait is one file holding
//! one `impl Lexer` block (`Concerns/DataScanner.php` is `data.rs`, `Concerns/ByteHelpers.php` is
//! `bytes.rs`, and so on); the methods keep their PHP names in snake_case and their control flow.
//!
//! Left out: Forte's extension hooks (`ExtensionStack`, `LexerExtension`, `LexerContext`,
//! attribute extensions), which are no-ops with no extension registered, and the public API only
//! extensions call (`advance`, `matchesAt`, `skipPhpString`, `skipPhpComment`, `consume`, ...).
//!
//! PHP's `$source[$i]` past the end reads `""`, which no comparison matches; `at` returns `None`
//! there. Positions are `usize` byte offsets, as PHP's ints are, and never go below zero.
//!
//! Bounds: the main loop spends one step per iteration, so no state cycle that makes no
//! progress runs unbounded; the lookahead scans that restore their position when they fail (JSX
//! attribute values, TSX generics, bogus comments) spend the distance they scanned, so retrying
//! them from every offset is bounded. Every other loop moves forward only. Nothing recurses.

mod attribute;
mod blade_comment;
mod bytes;
mod comment;
mod construct;
mod data;
mod directive;
mod doctype;
mod echo;
mod html;
mod jsx;
mod php;
mod rawtext;

use super::directives::Directives;
use super::scan;
use super::token::{ErrorReason, LexerError, State, Token, TokenType};
use super::{Budget, Exhausted};

/// What the lexer produces (`Lexer::tokenizeCompact`).
pub(crate) struct Lexed {
    pub(crate) tokens: Vec<Token>,
    pub(crate) errors: Vec<LexerError>,
}

/// `(new Lexer($source, $directives))->tokenizeCompact()`.
pub(crate) fn tokenize(
    source: &[u8],
    directives: &Directives,
    budget: &mut Budget,
) -> Result<Lexed, Exhausted> {
    let mut lexer = Lexer::new(source, directives, budget);
    lexer.scan()?;
    Ok(Lexed {
        tokens: lexer.tokens,
        errors: lexer.errors,
    })
}

/// `Forte\Lexer\Lexer`'s state.
struct Lexer<'a> {
    source: &'a [u8],
    pos: usize,
    len: usize,
    state: State,
    return_state: State,
    tokens: Vec<Token>,
    errors: Vec<LexerError>,
    directives: &'a Directives,
    verbatim: bool,
    verbatim_return_state: Option<State>,
    php_block: bool,
    php_tag: bool,
    continued_tag_name: bool,
    /// Always `''` or one of `HtmlScanner::SPECIAL_TEXT_ELEMENTS`.
    rawtext_tag_name: &'static [u8],
    current_tag_name: Vec<u8>,
    is_closing_tag: bool,
    in_xml_declaration: bool,
    budget: &'a mut Budget,
}

impl<'a> Lexer<'a> {
    /// `__construct`.
    fn new(source: &'a [u8], directives: &'a Directives, budget: &'a mut Budget) -> Self {
        Self {
            source,
            pos: 0,
            len: source.len(),
            state: State::Data,
            return_state: State::Data,
            tokens: Vec::with_capacity(source.len() / 8 + 4),
            errors: Vec::new(),
            directives,
            verbatim: false,
            verbatim_return_state: None,
            php_block: false,
            php_tag: false,
            continued_tag_name: false,
            rawtext_tag_name: b"",
            current_tag_name: Vec::new(),
            is_closing_tag: false,
            in_xml_declaration: false,
            budget,
        }
    }

    /// `scan`.
    fn scan(&mut self) -> Result<(), Exhausted> {
        while self.pos < self.len {
            self.budget.spend(1)?;
            // `$this->extensions->tryTokenize(...)`: no extensions are registered.
            match self.state {
                State::Data => self.scan_data()?,
                State::RawText => self.scan_rawtext(),
                State::BladeComment => self.scan_blade_comment_content(),
                State::Comment => self.scan_comment(),
                State::TagName => self.scan_tag_name()?,
                State::BeforeAttrName => self.scan_before_attr_name()?,
                State::AttrName => self.scan_attr_name(),
                State::AfterAttrName => self.scan_after_attr_name(),
                State::BeforeAttrValue => self.scan_before_attr_value()?,
                State::AttrValueQuoted => self.scan_attr_value_quoted(),
                State::AttrValueUnquoted => self.scan_attr_value_unquoted(),
                // Unhandled state
                _ => break,
            }
        }

        if matches!(
            self.state,
            State::TagName
                | State::BeforeAttrName
                | State::AttrName
                | State::AfterAttrName
                | State::BeforeAttrValue
                | State::AttrValueQuoted
                | State::AttrValueUnquoted
        ) {
            self.emit_token(TokenType::SyntheticClose, self.pos, self.pos);
        }
        Ok(())
    }

    /// `logError`.
    fn log_error(&mut self, state: State, reason: ErrorReason, offset: usize) {
        self.errors.push(LexerError {
            state,
            reason,
            offset: offset as u32,
        });
    }

    /// `setPhpBlock`.
    fn set_php_block(&mut self, php_block: bool) {
        self.php_block = php_block;
    }

    /// `setVerbatim`.
    fn set_verbatim(&mut self, verbatim: bool) {
        self.verbatim = verbatim;
    }

    /// `emitToken`.
    #[inline]
    fn emit_token(&mut self, kind: TokenType, start: usize, end: usize) {
        self.tokens.push(Token {
            kind,
            start: start as u32,
            end: end as u32,
        });
    }

    /// `skipLineCommentWithWarnings`: logs a notice for each `?>` in the line comment.
    fn skip_line_comment_with_warnings(&mut self) {
        let detected =
            scan::skip_line_comment_detecting(self.source, &mut self.pos, self.len, &[b"?>"]);
        for offset in detected {
            self.log_error(self.state, ErrorReason::PhpCloseTagInComment, offset);
        }
    }

    /// `skipBalancedParens`.
    fn skip_balanced_parens(&mut self) -> usize {
        if self.pos >= self.len || self.source[self.pos] != b'(' {
            return self.pos;
        }

        self.pos += 1; // Skip opening (
        let mut depth = 1usize;

        while depth > 0 && self.pos < self.len {
            let byte = self.source[self.pos];

            if byte == b'(' {
                depth += 1;
                self.pos += 1;
            } else if byte == b')' {
                depth -= 1;
                self.pos += 1;
            } else if byte == b'\'' || byte == b'"' {
                self.pos += 1;
                scan::skip_quoted_string(self.source, &mut self.pos, self.len, byte);
            } else if byte == b'/' && self.pos + 1 < self.len {
                let next = self.source[self.pos + 1];
                if next == b'/' {
                    self.pos += 2;
                    self.skip_line_comment_with_warnings();
                } else if next == b'*' {
                    self.pos += 2;
                    scan::skip_block_comment(self.source, &mut self.pos, self.len);
                } else {
                    self.pos += 1;
                }
            } else {
                self.pos += 1;
            }
        }

        self.pos
    }
}

//! Forte's `Parser\Concerns\ProcessesConditionalPairingDirectives`: `@section` and `@lang`,
//! which open blocks only when used the block way (`@section('name')` … `@endsection`, `@lang`
//! … `@endlang`) and stand alone otherwise (`@section('name', 'content')`).

use super::super::ParseError;
use super::super::directives::helper::{
    args_start_with_array, check_directive_args, count_directive_args, find_matching_terminator,
};
use super::super::directives::token_index::{DirectiveTokenIndex, token_text};
use super::super::token::TokenType;
use super::TreeBuilder;
use std::borrow::Cow;

impl<'a> TreeBuilder<'a, '_> {
    /// `shouldPairDirective`.
    fn should_pair_directive(
        &mut self,
        directive_name: &[u8],
        start_pos: usize,
        token_count: usize,
    ) -> Result<bool, ParseError> {
        match self.directives.get_pairing_strategy(directive_name) {
            Some("lang_style") => {
                self.should_pair_lang_style(directive_name, start_pos, token_count)
            }
            Some("section_style") => {
                self.should_pair_section_style(directive_name, start_pos, token_count)
            }
            _ => Ok(false),
        }
    }

    /// `shouldPairLangStyle`: no arguments, or an array, and a terminator follows.
    fn should_pair_lang_style(
        &mut self,
        directive_name: &[u8],
        start_pos: usize,
        token_count: usize,
    ) -> Result<bool, ParseError> {
        let args_info = check_directive_args(
            self.tokens,
            self.source,
            start_pos + 1,
            start_pos + token_count,
        );
        // If no args or array args (second char is '['), check for terminator.
        if !args_info.has_args || args_start_with_array(args_info.args_content.unwrap_or(b"")) {
            // Look ahead for @endlang.
            return self.has_matching_terminator(directive_name, start_pos + token_count);
        }
        Ok(false)
    }

    /// `shouldPairSectionStyle`: fewer than two arguments, and a terminator follows.
    fn should_pair_section_style(
        &mut self,
        directive_name: &[u8],
        start_pos: usize,
        token_count: usize,
    ) -> Result<bool, ParseError> {
        let mut arg_count = 0;
        for i in start_pos..(start_pos + token_count).min(self.tokens.len()) {
            let token = self.tokens[i];
            if token.kind == TokenType::DirectiveArgs {
                let content = token_text(self.source, token.start, token.end);
                arg_count = count_directive_args(content);
                break;
            }
        }
        if arg_count >= 2 {
            return Ok(false);
        }
        self.has_matching_terminator(directive_name, start_pos + token_count)
    }

    /// `DirectiveHelper::findMatchingTerminator(...) !== null` with the builder's index.
    fn has_matching_terminator(
        &mut self,
        directive_name: &[u8],
        start_idx: usize,
    ) -> Result<bool, ParseError> {
        let (tokens, source, directives) = (self.tokens, self.source, self.directives);
        let index = self
            .directive_index
            .get_or_insert_with(|| DirectiveTokenIndex::new(tokens, source));
        Ok(
            find_matching_terminator(directive_name, start_idx, directives, index, self.budget)?
                .is_some(),
        )
    }

    /// `processConditionalPairingDirective`.
    pub(super) fn process_conditional_pairing_directive(
        &mut self,
        directive_name: Cow<'a, [u8]>,
        start_pos: usize,
        token_count: usize,
        args_content: Option<&'a [u8]>,
    ) -> Result<(), ParseError> {
        if self.should_pair_directive(&directive_name, start_pos, token_count)? {
            return self.open_paired_directive(
                directive_name,
                start_pos,
                token_count,
                args_content,
                None,
            );
        }
        self.create_standalone_directive(&directive_name, start_pos, token_count, args_content);
        Ok(())
    }
}

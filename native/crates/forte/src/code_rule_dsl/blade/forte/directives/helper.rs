//! Forte's `Parser\Directives\DirectiveHelper`, the methods the tree builder calls, and
//! `Support\StringUtilities::unwrapParentheses`, which `countDirectiveArgs` calls. The tree
//! builder always passes its `DirectiveTokenIndex`, so `findMatchingTerminator` is ported as its
//! indexed branch.

use super::super::scan::substr;
use super::super::token::{Token, TokenType};
use super::super::{Budget, Exhausted};
use super::Directives;
use super::arguments::count_arguments;
use super::token_index::{DirectiveTokenIndex, token_text};
use std::borrow::Cow;

/// `StringInterner::lower` without a copy when the name is lowercase already.
pub(crate) fn lower_cow(name: &[u8]) -> Cow<'_, [u8]> {
    if name.iter().any(u8::is_ascii_uppercase) {
        Cow::Owned(name.to_ascii_lowercase())
    } else {
        Cow::Borrowed(name)
    }
}

/// `extractDirectiveNameFromToken`: the token's text without a leading `@`, lowercase.
pub(crate) fn extract_directive_name_from_token<'s>(
    token: &Token,
    source: &'s [u8],
) -> Cow<'s, [u8]> {
    let text = token_text(source, token.start, token.end);
    let text = text.strip_prefix(b"@").unwrap_or(text);
    lower_cow(text)
}

/// What `checkDirectiveArgs` returns.
pub(crate) struct ArgsInfo<'s> {
    pub(crate) has_args: bool,
    pub(crate) args_content: Option<&'s [u8]>,
    pub(crate) consumed: usize,
}

/// `checkDirectiveArgs`: the `DirectiveArgs` token at `start_idx`, or after one whitespace token.
pub(crate) fn check_directive_args<'s>(
    tokens: &[Token],
    source: &'s [u8],
    start_idx: usize,
    end_idx: usize,
) -> ArgsInfo<'s> {
    let mut consumed = 0;
    let mut has_args = false;
    let mut args_content = None;
    let mut check_idx = start_idx;
    let kind = |idx: usize| tokens.get(idx).map(|token| token.kind);
    if check_idx < end_idx
        && kind(check_idx) == Some(TokenType::Whitespace)
        && check_idx + 1 < end_idx
        && kind(check_idx + 1) == Some(TokenType::DirectiveArgs)
    {
        consumed += 1;
        check_idx += 1;
    }
    if check_idx < end_idx && kind(check_idx) == Some(TokenType::DirectiveArgs) {
        has_args = true;
        let token = &tokens[check_idx];
        args_content = Some(substr(
            source,
            token.start as usize,
            token.end.saturating_sub(token.start) as usize,
        ));
        consumed += 1;
    }
    ArgsInfo {
        has_args,
        args_content,
        consumed,
    }
}

/// `findMatchingTerminator` with the index: the directive's terminator at nesting zero after
/// `start_idx`.
pub(crate) fn find_matching_terminator(
    directive_name: &[u8],
    start_idx: usize,
    directives: &Directives,
    index: &mut DirectiveTokenIndex,
    budget: &mut Budget,
) -> Result<Option<usize>, Exhausted> {
    let terminators = get_terminator_names(directive_name, directives);
    index.find_matching_terminator(directive_name, start_idx, &*terminators, None, 0, budget)
}

/// `getTerminatorNames`: the directive's terminators, else `end<name>`.
pub(crate) fn get_terminator_names<'d>(
    directive_name: &[u8],
    directives: &'d Directives,
) -> Cow<'d, [Box<[u8]>]> {
    if let Some(directive) = directives.get_directive(directive_name)
        && !directive.terminators.is_empty()
    {
        return Cow::Borrowed(&directive.terminators);
    }
    Cow::Owned(vec![
        [b"end".as_slice(), directive_name]
            .concat()
            .into_boxed_slice(),
    ])
}

/// `countDirectiveArgs`.
pub(crate) fn count_directive_args(args_content: &[u8]) -> usize {
    if args_content.is_empty() {
        return 0;
    }
    count_arguments(unwrap_parentheses(args_content))
}

/// `argsStartWithArray`: the second byte is `[`.
pub(crate) fn args_start_with_array(args_content: &[u8]) -> bool {
    args_content.get(1) == Some(&b'[')
}

/// `StringUtilities::unwrapParentheses`: strips outer `(`…`)` pairs. PHP's `mb_substr($value, 1,
/// -1)` drops the first and last characters, both ASCII here, and replaces each invalid UTF-8
/// sequence with `?`; ASCII bytes survive, and `countArguments` treats `?` as it treats any
/// non-ASCII byte, so dropping the two bytes counts the same arguments.
pub(crate) fn unwrap_parentheses(mut value: &[u8]) -> &[u8] {
    while value.len() >= 2 && value.first() == Some(&b'(') && value.last() == Some(&b')') {
        value = &value[1..value.len() - 1];
    }
    value
}

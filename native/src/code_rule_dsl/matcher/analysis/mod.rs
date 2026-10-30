//! What a matcher's terms say before any file is read: whether it reads `calls` only positively,
//! and the text a file or node must contain for it to match there.

mod polarity;
mod required_text;

pub(crate) use polarity::{calls_positive_only, reads_import_targets};
pub(in crate::code_rule_dsl) use required_text::RequiredText;

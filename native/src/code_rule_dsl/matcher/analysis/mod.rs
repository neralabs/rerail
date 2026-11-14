//! What a matcher's terms say before any file is read: whether it reads `calls` only positively,
//! and the text a file or node must contain for it to match there.

pub mod polarity;
pub mod required_text;

pub use polarity::{calls_positive_only, reads_import_targets};
pub use required_text::RequiredText;

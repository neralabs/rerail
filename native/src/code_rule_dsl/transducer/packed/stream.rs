//! Tier 2's instructions (decision 0087): the transducer's words over tier 1's token stream, as
//! simdjson's stage 2 runs over its structural indexes. A program is a state machine: each
//! state takes a token (`TOKEN`), switches on its byte (`SWITCH`), writes nodes, and jumps to the
//! next state (`GOTO`). Its verifier refuses a cycle with no `TOKEN`, so its one loop is over the
//! stream. The switch tables, the verifier and the interpreter are the compiler crate's (`rerail_compiler::stream`); `native_ir` and
//! Cranelift lower the same words.


pub(in crate::code_rule_dsl) use rerail_compiler::stream::*;

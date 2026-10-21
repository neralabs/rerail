//! Whole-call relational compilation experiments. No addon switch or production alternate path.
//! The adapter retains the existing semantic lowerings, but replaces physical frame steps with
//! immutable typed relation references. Unsupported stateful constructs fail explicitly.
mod adapter;
mod buffers;
mod composition;
mod crossfile;
mod direct;
mod each;
mod graph;
mod homogeneous;
mod inputs;
mod integer_keys;
mod lex;
mod recursive;
mod run;
mod terminal_set;
mod tests;
mod workload;
